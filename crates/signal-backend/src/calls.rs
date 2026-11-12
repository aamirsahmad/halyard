//! 1:1 voice calls with RingRTC, the WebRTC library Signal's own apps use.
//!
//! RingRTC owns the call: it makes the offer and answer, gathers ICE
//! candidates, encrypts media and plays and records audio. Halyard carries
//! RingRTC's signaling over Signal messages (`CallMessage`), gives it the
//! TURN relays from the chat server, and shows the call.
//!
//! RingRTC calls back on its own threads. [`Bridge`] turns those callbacks
//! into [`CallEvent`]s for the backend thread, which owns the [`CallEngine`].

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use presage::libsignal_service::configuration::Endpoint;
use presage::libsignal_service::proto::{CallMessage, call_message};
use presage::libsignal_service::push_service::{HttpAuthOverride, PushService};
use ringrtc::common::{CallConfig, CallEndReason, CallId, CallMediaType, DeviceId};
use ringrtc::core::call_manager::CallManager;
use ringrtc::core::{group_call, signaling};
use ringrtc::lite::{http, sfu::UserId};
use ringrtc::native::{
    CallState, CallStateHandler, GroupUpdate, GroupUpdateHandler, NativeCallContext,
    NativePlatform, SignalingSender,
};
use ringrtc::webrtc::media::{AudioTrack, VideoFrame, VideoSink, VideoTrack};
use ringrtc::webrtc::peer_connection::AudioLevel;
use ringrtc::webrtc::peer_connection_factory::{self as pcf, IceServer, PeerConnectionFactory};
use ringrtc::webrtc::peer_connection_observer::NetworkRoute;
use serde::Deserialize;
use signal_core::{CallInfo, CallOutcome, CallPhase, Millis};
use tokio::sync::mpsc;
use uuid::Uuid;

// ------------------------------------------------------------------ signaling

/// Converts RingRTC signaling to the `CallMessage` that carries it.
pub fn to_proto(
    call_id: u64,
    receiver_device: Option<DeviceId>,
    message: &signaling::Message,
) -> CallMessage {
    let mut proto = CallMessage {
        destination_device_id: receiver_device,
        ..Default::default()
    };
    match message {
        signaling::Message::Offer(offer) => {
            let typ = match offer.call_media_type {
                CallMediaType::Audio => call_message::offer::Type::OfferAudioCall,
                CallMediaType::Video => call_message::offer::Type::OfferVideoCall,
            };
            proto.offer = Some(call_message::Offer {
                id: Some(call_id),
                r#type: Some(typ as i32),
                opaque: Some(offer.opaque.clone()),
            });
        }
        signaling::Message::Answer(answer) => {
            proto.answer = Some(call_message::Answer {
                id: Some(call_id),
                opaque: Some(answer.opaque.clone()),
            });
        }
        signaling::Message::Ice(ice) => {
            proto.ice_update = ice
                .candidates
                .iter()
                .map(|c| call_message::IceUpdate {
                    id: Some(call_id),
                    opaque: Some(c.opaque.clone()),
                })
                .collect();
        }
        signaling::Message::Hangup(hangup) => {
            let (typ, device) = hangup.to_type_and_device_id();
            proto.hangup = Some(call_message::Hangup {
                id: Some(call_id),
                r#type: Some(typ as i32),
                device_id: Some(device.unwrap_or(0)),
            });
        }
        signaling::Message::Busy => {
            proto.busy = Some(call_message::Busy { id: Some(call_id) });
        }
    }
    proto
}

/// One signaling message from a `CallMessage`.
pub struct Incoming {
    pub call_id: u64,
    pub message: signaling::Message,
}

/// Converts a received `CallMessage` to RingRTC signaling. Invalid parts are
/// dropped, as Signal Desktop does.
pub fn from_proto(proto: &CallMessage) -> Vec<Incoming> {
    let mut out = Vec::new();
    if let Some(call_message::Offer {
        id: Some(id),
        r#type,
        opaque: Some(opaque),
    }) = &proto.offer
    {
        let media = if *r#type == Some(call_message::offer::Type::OfferVideoCall as i32) {
            CallMediaType::Video
        } else {
            CallMediaType::Audio
        };
        match signaling::Offer::new(media, opaque.clone()) {
            Ok(offer) => out.push(Incoming {
                call_id: *id,
                message: signaling::Message::Offer(offer),
            }),
            Err(e) => tracing::warn!(%e, "invalid call offer"),
        }
    }
    if let Some(call_message::Answer {
        id: Some(id),
        opaque: Some(opaque),
    }) = &proto.answer
    {
        match signaling::Answer::new(opaque.clone()) {
            Ok(answer) => out.push(Incoming {
                call_id: *id,
                message: signaling::Message::Answer(answer),
            }),
            Err(e) => tracing::warn!(%e, "invalid call answer"),
        }
    }
    // RingRTC takes candidates per call; a message normally has one call ID
    let mut ice: Vec<(u64, Vec<signaling::IceCandidate>)> = Vec::new();
    for update in &proto.ice_update {
        let (Some(id), Some(opaque)) = (update.id, &update.opaque) else {
            continue;
        };
        let candidate = signaling::IceCandidate::new(opaque.clone());
        match ice.iter_mut().find(|(i, _)| *i == id) {
            Some((_, list)) => list.push(candidate),
            None => ice.push((id, vec![candidate])),
        }
    }
    out.extend(ice.into_iter().map(|(call_id, candidates)| Incoming {
        call_id,
        message: signaling::Message::Ice(signaling::Ice { candidates }),
    }));
    if let Some(call_message::Hangup {
        id: Some(id),
        r#type,
        device_id,
    }) = &proto.hangup
    {
        let typ = r#type
            .and_then(signaling::HangupType::from_i32)
            .unwrap_or(signaling::HangupType::Normal);
        out.push(Incoming {
            call_id: *id,
            message: signaling::Message::Hangup(signaling::Hangup::from_type_and_device_id(
                typ,
                device_id.unwrap_or(0),
            )),
        });
    }
    if let Some(call_message::Busy { id: Some(id) }) = &proto.busy {
        out.push(Incoming {
            call_id: *id,
            message: signaling::Message::Busy,
        });
    }
    out
}

/// A call offer, as the `NeedPermission` hangup we answer it with when the
/// caller may not ring us (blocked, or an unanswered message request).
pub fn need_permission(call_id: u64, our_device: DeviceId) -> CallMessage {
    to_proto(
        call_id,
        None,
        &signaling::Message::Hangup(signaling::Hangup::NeedPermission(Some(our_device))),
    )
}

/// Whether a `CallMessage` starts a call (and so may ring).
pub fn offer_of(proto: &CallMessage) -> Option<(u64, bool)> {
    let offer = proto.offer.as_ref()?;
    let video = offer.r#type == Some(call_message::offer::Type::OfferVideoCall as i32);
    Some((offer.id?, video))
}

// --------------------------------------------------------------- TURN relays

#[derive(Debug, Deserialize)]
struct RelaysResponse {
    #[serde(default)]
    relays: Vec<Relay>,
}

/// A group of TURN servers from `GET /v2/calling/relays`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Relay {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub urls_with_ips: Vec<String>,
    #[serde(default)]
    pub hostname: Option<String>,
}

/// Parses the relay response.
pub fn parse_relays(body: &[u8]) -> Result<Vec<Relay>, String> {
    serde_json::from_slice::<RelaysResponse>(body)
        .map(|r| r.relays)
        .map_err(|e| e.to_string())
}

/// Gets the TURN relays for a call.
pub async fn fetch_relays(service: &PushService) -> Result<Vec<Relay>, String> {
    let response = service
        .request(
            reqwest::Method::GET,
            Endpoint::service("/v2/calling/relays"),
            HttpAuthOverride::NoOverride,
        )
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let body = response.bytes().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("relay request failed ({status})"));
    }
    parse_relays(&body)
}

/// RingRTC ICE servers for the relays, like Signal Desktop: the IP addresses
/// first (with the hostname for TLS), then the host names.
fn ice_servers(relays: &[Relay]) -> Vec<IceServer> {
    let mut out = Vec::new();
    for r in relays {
        if !r.urls_with_ips.is_empty() {
            out.push(IceServer::new(
                r.username.clone(),
                r.password.clone(),
                r.hostname.clone().unwrap_or_default(),
                r.urls_with_ips.clone(),
            ));
        }
        if !r.urls.is_empty() {
            out.push(IceServer::new(
                r.username.clone(),
                r.password.clone(),
                String::new(),
                r.urls.clone(),
            ));
        }
    }
    if out.is_empty() {
        out.push(IceServer::none());
    }
    out
}

// --------------------------------------------------------------------- bridge

/// What RingRTC reports, in a form the backend thread handles.
#[derive(Debug)]
pub enum CallEvent {
    /// Send this signaling to `peer` (it names the receiving device, if any).
    Signal {
        peer: Uuid,
        call_id: u64,
        message: Box<CallMessage>,
    },
    State {
        peer: Uuid,
        call_id: u64,
        state: StateChange,
    },
    RemoteAudio {
        peer: Uuid,
        enabled: bool,
    },
}

/// RingRTC's [`CallState`], without the parts Halyard doesn't use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateChange {
    Incoming {
        video: bool,
    },
    Outgoing,
    Ringing,
    Connected,
    Connecting,
    Ended(EndReason),
    /// RingRTC refused an offer (too old, or glare).
    Rejected,
    Concluded,
}

/// Why a call ended, reduced to what call history and the UI need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    LocalHangup,
    RemoteHangup,
    NeedPermission,
    AcceptedElsewhere,
    DeclinedElsewhere,
    BusyElsewhere,
    Busy,
    Timeout,
    Failed,
}

impl From<&CallEndReason> for EndReason {
    fn from(r: &CallEndReason) -> Self {
        match r {
            CallEndReason::LocalHangup | CallEndReason::AppDroppedCall => Self::LocalHangup,
            CallEndReason::RemoteHangup
            | CallEndReason::RemoteGlare
            | CallEndReason::RemoteReCall => Self::RemoteHangup,
            CallEndReason::RemoteHangupNeedPermission => Self::NeedPermission,
            CallEndReason::RemoteHangupAccepted => Self::AcceptedElsewhere,
            CallEndReason::RemoteHangupDeclined => Self::DeclinedElsewhere,
            CallEndReason::RemoteHangupBusy => Self::BusyElsewhere,
            CallEndReason::RemoteBusy => Self::Busy,
            CallEndReason::Timeout => Self::Timeout,
            _ => Self::Failed,
        }
    }
}

impl From<&CallState> for StateChange {
    fn from(s: &CallState) -> Self {
        match s {
            CallState::Incoming(media) => Self::Incoming {
                video: *media == CallMediaType::Video,
            },
            CallState::Outgoing(_) => Self::Outgoing,
            CallState::Ringing => Self::Ringing,
            CallState::Connected => Self::Connected,
            CallState::Connecting => Self::Connecting,
            CallState::Ended(reason, _) => Self::Ended(reason.into()),
            CallState::Rejected(_) => Self::Rejected,
            CallState::Concluded => Self::Concluded,
        }
    }
}

/// RingRTC's callbacks. Sends each one to the backend thread.
#[derive(Clone)]
pub struct Bridge {
    tx: mpsc::UnboundedSender<CallEvent>,
}

fn peer_uuid(id: &str) -> ringrtc::common::Result<Uuid> {
    Uuid::parse_str(id).map_err(|e| anyhow::anyhow!("bad peer id {id}: {e}"))
}

impl SignalingSender for Bridge {
    fn send_signaling(
        &self,
        recipient_id: &str,
        call_id: CallId,
        receiver_device_id: Option<DeviceId>,
        message: signaling::Message,
    ) -> ringrtc::common::Result<()> {
        let call_id = call_id.as_u64();
        let _ = self.tx.send(CallEvent::Signal {
            peer: peer_uuid(recipient_id)?,
            call_id,
            message: Box::new(to_proto(call_id, receiver_device_id, &message)),
        });
        Ok(())
    }

    // Group calls only.
    fn send_call_message(
        &self,
        _recipient_id: UserId,
        _message: Vec<u8>,
        _urgency: group_call::SignalingMessageUrgency,
    ) -> ringrtc::common::Result<()> {
        Ok(())
    }

    fn send_call_message_to_group(
        &self,
        _group_id: group_call::GroupId,
        _message: Vec<u8>,
        _urgency: group_call::SignalingMessageUrgency,
        _recipients_override: HashSet<UserId>,
    ) -> ringrtc::common::Result<()> {
        Ok(())
    }

    fn send_call_message_to_adhoc_group(
        &self,
        _message: Vec<u8>,
        _urgency: group_call::SignalingMessageUrgency,
        _expiration: u64,
        _recipients_to_endorsements: HashMap<UserId, Vec<u8>>,
    ) -> ringrtc::common::Result<()> {
        Ok(())
    }
}

impl CallStateHandler for Bridge {
    fn handle_call_state(
        &self,
        remote_peer_id: &str,
        call_id: CallId,
        state: CallState,
    ) -> ringrtc::common::Result<()> {
        tracing::info!(peer = remote_peer_id, call = %call_id, ?state, "call state");
        let _ = self.tx.send(CallEvent::State {
            peer: peer_uuid(remote_peer_id)?,
            call_id: call_id.as_u64(),
            state: StateChange::from(&state),
        });
        Ok(())
    }

    fn handle_remote_audio_state(
        &self,
        remote_peer_id: &str,
        enabled: bool,
    ) -> ringrtc::common::Result<()> {
        let _ = self.tx.send(CallEvent::RemoteAudio {
            peer: peer_uuid(remote_peer_id)?,
            enabled,
        });
        Ok(())
    }

    fn handle_remote_video_state(&self, _: &str, _: bool) -> ringrtc::common::Result<()> {
        Ok(())
    }

    fn handle_remote_sharing_screen(&self, _: &str, _: bool) -> ringrtc::common::Result<()> {
        Ok(())
    }

    fn handle_network_route(
        &self,
        remote_peer_id: &str,
        route: NetworkRoute,
    ) -> ringrtc::common::Result<()> {
        tracing::debug!(peer = remote_peer_id, ?route, "call network route");
        Ok(())
    }

    fn handle_audio_levels(
        &self,
        _: &str,
        _: AudioLevel,
        _: AudioLevel,
    ) -> ringrtc::common::Result<()> {
        Ok(())
    }

    fn handle_low_bandwidth_for_video(&self, _: &str, _: bool) -> ringrtc::common::Result<()> {
        Ok(())
    }
}

impl GroupUpdateHandler for Bridge {
    fn handle_group_update(&self, _update: GroupUpdate) -> ringrtc::common::Result<()> {
        Ok(())
    }
}

impl http::Delegate for Bridge {
    // Only group calls make HTTP requests.
    fn send_request(&self, _request_id: u32, _request: http::Request) {}
}

/// Halyard has no video; incoming frames are dropped.
#[derive(Clone)]
struct NoVideo;

impl VideoSink for NoVideo {
    fn on_video_frame(&self, _demux_id: u32, _frame: VideoFrame) {}
    fn box_clone(&self) -> Box<dyn VideoSink> {
        Box::new(self.clone())
    }
}

// --------------------------------------------------------------------- engine

/// The call in progress (there is at most one).
#[derive(Clone, Debug)]
pub struct Current {
    pub call_id: u64,
    pub peer: Uuid,
    pub outgoing: bool,
    pub video_offered: bool,
    pub phase: CallPhase,
    pub accepted: bool,
    pub muted: bool,
    pub remote_muted: bool,
    pub started_at: Millis,
    pub connected_at: Option<Millis>,
}

impl Current {
    pub fn info(&self) -> CallInfo {
        CallInfo {
            contact: self.peer,
            outgoing: self.outgoing,
            video_offered: self.video_offered,
            phase: self.phase.clone(),
            muted: self.muted,
            remote_muted: self.remote_muted,
        }
    }
}

/// Call history entry for a call that ended.
pub fn outcome(outgoing: bool, connected: bool, accepted: bool, reason: EndReason) -> CallOutcome {
    if connected {
        return CallOutcome::Answered;
    }
    match (outgoing, reason) {
        (false, EndReason::AcceptedElsewhere) => CallOutcome::Answered,
        (false, EndReason::DeclinedElsewhere) => CallOutcome::Declined,
        (false, EndReason::LocalHangup) => CallOutcome::Declined,
        // accepted here, but media never connected
        (false, _) if accepted => CallOutcome::Missed,
        (false, _) => CallOutcome::Missed,
        (true, EndReason::DeclinedElsewhere | EndReason::RemoteHangup) => CallOutcome::Declined,
        (true, _) => CallOutcome::Unanswered,
    }
}

/// Short text for the call screen when a call ends.
pub fn end_text(reason: EndReason, connected: bool) -> &'static str {
    match reason {
        EndReason::LocalHangup | EndReason::RemoteHangup if connected => "Call ended",
        EndReason::LocalHangup => "Call ended",
        EndReason::RemoteHangup => "Call declined",
        EndReason::NeedPermission => "Call not accepted",
        EndReason::AcceptedElsewhere => "Answered on another device",
        EndReason::DeclinedElsewhere => "Declined on another device",
        EndReason::BusyElsewhere | EndReason::Busy => "Busy",
        EndReason::Timeout => "No answer",
        EndReason::Failed => "Call failed",
    }
}

/// RingRTC's call manager, with the media objects every call shares.
pub struct CallEngine {
    manager: CallManager<NativePlatform>,
    factory: PeerConnectionFactory,
    audio_track: AudioTrack,
    video_track: VideoTrack,
    devices_ready: bool,
    pub current: Option<Current>,
}

impl CallEngine {
    pub fn new(events: mpsc::UnboundedSender<CallEvent>) -> anyhow::Result<Self> {
        let bridge = Bridge { tx: events };
        let factory = PeerConnectionFactory::new(&pcf::AudioConfig::default(), false, "", None)?;
        let audio_track = factory.create_outgoing_audio_track()?;
        audio_track.set_enabled(false);
        let video_source = factory.create_outgoing_video_source()?;
        let video_track = factory.create_outgoing_video_track(&video_source)?;
        video_track.set_enabled(false);
        let platform = NativePlatform::new(
            factory.clone(),
            Box::new(bridge.clone()),
            // the backend tells RingRTC when each message is sent
            false,
            Box::new(bridge.clone()),
            Box::new(bridge.clone()),
        );
        let manager = CallManager::new(platform, http::DelegatingClient::new(bridge))?;
        Ok(Self {
            manager,
            factory,
            audio_track,
            video_track,
            devices_ready: false,
            current: None,
        })
    }

    /// Gives RingRTC a received `CallMessage`. `keys` are the sender's and
    /// our 32-byte identity keys; `age` is how long the server held it.
    pub fn receive(
        &mut self,
        peer: Uuid,
        sender_device: DeviceId,
        our_device: DeviceId,
        keys: (&[u8], &[u8]),
        age: Duration,
        message: &CallMessage,
    ) {
        let (sender_key, our_key) = keys;
        let cm = &mut self.manager;
        for incoming in from_proto(message) {
            let id = CallId::new(incoming.call_id);
            let peer_id = peer.to_string();
            match incoming.message {
                signaling::Message::Offer(offer) => cm.received_offer(
                    peer_id,
                    id,
                    signaling::ReceivedOffer {
                        offer,
                        age,
                        sender_device_id: sender_device,
                        receiver_device_id: our_device,
                        sender_identity_key: sender_key.to_vec(),
                        receiver_identity_key: our_key.to_vec(),
                    },
                ),
                signaling::Message::Answer(answer) => cm.received_answer(
                    peer_id,
                    id,
                    signaling::ReceivedAnswer {
                        answer,
                        sender_device_id: sender_device,
                        sender_identity_key: sender_key.to_vec(),
                        receiver_identity_key: our_key.to_vec(),
                    },
                ),
                signaling::Message::Ice(ice) => cm.received_ice(
                    peer_id,
                    id,
                    signaling::ReceivedIce {
                        ice,
                        sender_device_id: sender_device,
                    },
                ),
                signaling::Message::Hangup(hangup) => cm.received_hangup(
                    peer_id,
                    id,
                    signaling::ReceivedHangup {
                        hangup,
                        sender_device_id: sender_device,
                    },
                ),
                signaling::Message::Busy => cm.received_busy(
                    peer_id,
                    id,
                    signaling::ReceivedBusy {
                        sender_device_id: sender_device,
                    },
                ),
            }
        }
    }

    pub fn busy(&self) -> bool {
        self.current.is_some()
    }

    pub fn start(&mut self, peer: Uuid, our_device: DeviceId, now: Millis) -> u64 {
        let call_id = CallId::random();
        self.current = Some(Current {
            call_id: call_id.as_u64(),
            peer,
            outgoing: true,
            video_offered: false,
            phase: CallPhase::Calling,
            accepted: true,
            muted: false,
            remote_muted: false,
            started_at: now,
            connected_at: None,
        });
        self.manager.create_outgoing_call(
            peer.to_string(),
            call_id,
            CallMediaType::Audio,
            our_device,
        );
        call_id.as_u64()
    }

    /// Lets RingRTC set up the connection for a new call.
    pub fn proceed(&mut self, call_id: u64, relays: &[Relay], hide_ip: bool) {
        if !self.devices_ready {
            match ringrtc::bin::utils::audio::set_default_audio_devices(&mut self.factory) {
                Ok(()) => self.devices_ready = true,
                Err(e) => tracing::warn!(%e, "no audio devices for calls"),
            }
        }
        let context = NativeCallContext::new(
            hide_ip,
            ice_servers(relays),
            self.audio_track.clone(),
            self.video_track.clone(),
            Box::new(NoVideo),
        );
        self.manager
            .proceed(CallId::new(call_id), context, CallConfig::default(), None);
    }

    pub fn accept(&mut self) {
        let Some(c) = &mut self.current else { return };
        if c.outgoing || c.accepted {
            return;
        }
        c.accepted = true;
        c.phase = CallPhase::Connecting;
        let id = CallId::new(c.call_id);
        self.manager.accept_call(id);
    }

    pub fn hang_up(&mut self) {
        self.manager.hangup();
    }

    /// Sends audio only while connected and not muted.
    pub fn update_audio(&mut self) {
        let send = self
            .current
            .as_ref()
            .is_some_and(|c| matches!(c.phase, CallPhase::Connected { .. }) && !c.muted);
        self.audio_track.set_enabled(send);
    }

    pub fn set_muted(&mut self, muted: bool) {
        let Some(c) = &mut self.current else { return };
        c.muted = muted;
        self.update_audio();
        if let Ok(mut connection) = self.manager.active_connection() {
            let _ = connection.update_sender_status(signaling::SenderStatus {
                audio_enabled: Some(!muted),
                ..Default::default()
            });
        }
    }

    pub fn message_sent(&mut self, call_id: u64, ok: bool) {
        if ok {
            self.manager.message_sent(CallId::new(call_id));
        } else {
            self.manager.message_send_failure(CallId::new(call_id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hangup_round_trips() {
        let msg = signaling::Message::Hangup(signaling::Hangup::AcceptedOnAnotherDevice(3));
        let proto = to_proto(42, Some(2), &msg);
        assert_eq!(proto.destination_device_id, Some(2));
        let back = from_proto(&proto);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].call_id, 42);
        match &back[0].message {
            signaling::Message::Hangup(h) => {
                assert_eq!(*h, signaling::Hangup::AcceptedOnAnotherDevice(3))
            }
            _ => panic!("not a hangup"),
        }
    }

    #[test]
    fn ice_candidates_are_grouped_per_call() {
        let msg = signaling::Message::Ice(signaling::Ice {
            candidates: vec![
                signaling::IceCandidate::new(vec![1]),
                signaling::IceCandidate::new(vec![2]),
            ],
        });
        let proto = to_proto(7, None, &msg);
        assert_eq!(proto.ice_update.len(), 2);
        let back = from_proto(&proto);
        assert_eq!(back.len(), 1);
        match &back[0].message {
            signaling::Message::Ice(ice) => {
                let all: Vec<_> = ice.candidates.iter().map(|c| c.opaque.clone()).collect();
                assert_eq!(all, vec![vec![1], vec![2]]);
            }
            _ => panic!("not ice"),
        }
    }

    #[test]
    fn busy_round_trips_and_offer_is_detected() {
        let proto = to_proto(9, None, &signaling::Message::Busy);
        assert!(matches!(
            from_proto(&proto)[0].message,
            signaling::Message::Busy
        ));
        assert_eq!(offer_of(&proto), None);
        let offer = CallMessage {
            offer: Some(call_message::Offer {
                id: Some(5),
                r#type: Some(call_message::offer::Type::OfferVideoCall as i32),
                opaque: Some(vec![]),
            }),
            ..Default::default()
        };
        assert_eq!(offer_of(&offer), Some((5, true)));
    }

    #[test]
    fn need_permission_names_our_device() {
        let proto = need_permission(11, 4);
        let h = proto.hangup.unwrap();
        assert_eq!(h.id, Some(11));
        assert_eq!(h.r#type, Some(signaling::HangupType::NeedPermission as i32));
        assert_eq!(h.device_id, Some(4));
    }

    #[test]
    fn parses_relays() {
        let body = br#"{"relays":[{"username":"u","password":"p","urls":["turn:turn.example:80"],
            "urlsWithIps":["turn:1.2.3.4:80"],"hostname":"turn.example","ttl":86400}]}"#;
        let relays = parse_relays(body).unwrap();
        assert_eq!(relays.len(), 1);
        assert_eq!(relays[0].urls_with_ips, vec!["turn:1.2.3.4:80"]);
        assert_eq!(relays[0].hostname.as_deref(), Some("turn.example"));
        assert_eq!(parse_relays(b"{}").unwrap(), vec![]);
    }

    #[test]
    fn call_history_outcomes() {
        use EndReason::*;
        assert_eq!(
            outcome(false, true, true, RemoteHangup),
            CallOutcome::Answered
        );
        assert_eq!(
            outcome(false, false, false, LocalHangup),
            CallOutcome::Declined
        );
        assert_eq!(
            outcome(false, false, false, RemoteHangup),
            CallOutcome::Missed
        );
        assert_eq!(outcome(false, false, false, Timeout), CallOutcome::Missed);
        assert_eq!(
            outcome(false, false, false, AcceptedElsewhere),
            CallOutcome::Answered
        );
        assert_eq!(
            outcome(false, false, false, DeclinedElsewhere),
            CallOutcome::Declined
        );
        assert_eq!(outcome(true, false, true, Timeout), CallOutcome::Unanswered);
        assert_eq!(
            outcome(true, false, true, LocalHangup),
            CallOutcome::Unanswered
        );
        assert_eq!(
            outcome(true, false, true, RemoteHangup),
            CallOutcome::Declined
        );
        assert_eq!(
            outcome(true, true, true, LocalHangup),
            CallOutcome::Answered
        );
    }
}
