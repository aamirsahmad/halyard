//! 1:1 voice calls: the incoming-call bar and the call screen. Sizes and
//! colours come from Signal Desktop's `IncomingCallBar.scss`,
//! `CallingButton.scss` and `_variables.scss`.

use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window,
    div, rgb, svg,
};
use signal_core::{BackendCommand, CallInfo, CallPhase, ConversationId, now_millis};
use uuid::Uuid;

use crate::app::SignalApp;
use crate::assets::signal_icons;
use crate::views::avatar::Avatar;
use crate::views::rem_px;

const GRAY_15: u32 = 0xDEDEDE;
const GRAY_25: u32 = 0xB9B9B9;
const GRAY_45: u32 = 0x848484;
const GRAY_60: u32 = 0x5E5E5E;
const GRAY_75: u32 = 0x3B3B3B;
const GRAY_90: u32 = 0x1B1B1B;
const GREEN: u32 = 0x4CAF50;
const RED: u32 = 0xF44336;

/// How long the call screen shows why a call ended.
pub const ENDED_SCREEN: Duration = Duration::from_millis(1500);

/// Status text under the name on the call screen.
pub fn call_status(info: &CallInfo, now: i64) -> String {
    let status = match &info.phase {
        CallPhase::IncomingRinging => "Signal voice call".to_string(),
        CallPhase::Calling => "Calling…".into(),
        CallPhase::OutgoingRinging => "Ringing…".into(),
        CallPhase::Connecting => "Connecting…".into(),
        CallPhase::Connected { since } => {
            let secs = ((now - since).max(0) / 1000) as u64;
            let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
            if h > 0 {
                format!("{h}:{m:02}:{s:02}")
            } else {
                format!("{m:02}:{s:02}")
            }
        }
        CallPhase::Ended { reason } => reason.clone(),
    };
    if info.remote_muted && matches!(info.phase, CallPhase::Connected { .. }) {
        format!("{status} · Microphone off")
    } else {
        status
    }
}

/// A round calling button with a centred icon.
fn round_button(
    id: &'static str,
    icon: &'static str,
    size: f32,
    icon_size: f32,
    bg: Hsla,
    fg: Hsla,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rem_px(size))
        .rounded_full()
        .bg(bg)
        .cursor_pointer()
        .hover(|s| s.opacity(0.85))
        .child(svg().path(icon).size(rem_px(icon_size)).text_color(fg))
}

impl SignalApp {
    pub fn start_call(&mut self, contact: Uuid, cx: &mut Context<Self>) {
        if self.state.call.as_ref().is_some_and(|c| !c.ended()) {
            return;
        }
        self.commands.send(BackendCommand::StartCall { contact });
        cx.notify();
    }

    pub fn accept_call(&mut self, cx: &mut Context<Self>) {
        self.commands.send(BackendCommand::AcceptCall);
        cx.notify();
    }

    pub fn hang_up(&mut self, cx: &mut Context<Self>) {
        self.commands.send(BackendCommand::HangUp);
        cx.notify();
    }

    pub fn toggle_call_mute(&mut self, cx: &mut Context<Self>) {
        if let Some(call) = &self.state.call {
            self.commands
                .send(BackendCommand::SetCallMuted(!call.muted));
        }
        cx.notify();
    }

    /// Chooses the ringtone and plays one round of it.
    pub fn set_ringtone(
        &mut self,
        tone: crate::settings::Ringtone,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_prefs(|p| p.ringtone = tone, window, cx);
        // don't interrupt a call that is ringing now
        if self.state.call.is_none() {
            self.notifier
                .call_sound(crate::audio::CallSound::Preview(tone));
        }
    }

    /// Whether the header may offer a voice call in this chat.
    pub fn can_call(&self, id: &ConversationId) -> bool {
        let Some(contact) = id.direct_uuid() else {
            return false;
        };
        Some(contact) != self.me() && self.state.conversation(id).is_none_or(|c| !c.blocked)
    }

    /// Keeps the call timer moving while a call is shown.
    pub(crate) fn tick_call(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.call_ticker.is_some() || self.state.call.is_none() {
            return;
        }
        self.call_ticker = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        cx.notify();
                        let keep = this.state.call.is_some();
                        if !keep {
                            this.call_ticker = None;
                        }
                        keep
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }

    /// Shows why the call ended for a moment, then closes the call screen.
    pub(crate) fn clear_ended_call(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.call_clear = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(ENDED_SCREEN).await;
            let _ = this.update(cx, |this, cx| {
                if this.state.call.as_ref().is_some_and(|c| c.ended()) {
                    this.state.call = None;
                }
                this.call_clear = None;
                cx.notify();
            });
        }));
    }

    fn call_title(&self, contact: &Uuid) -> String {
        self.state
            .conversation(&ConversationId::Direct(*contact))
            .map(|c| c.title.clone())
            .unwrap_or_else(|| self.display_name(contact))
    }

    /// The incoming-call bar or the call screen, drawn over the app.
    pub(crate) fn render_call(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let info = self.state.call.as_ref()?;
        Some(if info.phase == CallPhase::IncomingRinging {
            self.render_incoming_bar(info, cx)
        } else {
            self.render_call_screen(info, cx)
        })
    }

    fn render_incoming_bar(&self, info: &CallInfo, cx: &mut Context<Self>) -> AnyElement {
        let id = ConversationId::Direct(info.contact);
        let title = self.call_title(&info.contact);
        let kind = if info.video_offered {
            "Signal video call · answer with voice"
        } else {
            "Signal voice call"
        };
        div()
            .absolute()
            .top(rem_px(22.))
            .left_0()
            .right_0()
            .px_4()
            .flex()
            .justify_center()
            .child(
                div()
                    .id("incoming-call")
                    .flex()
                    .items_center()
                    .w_full()
                    .max_w(rem_px(600.))
                    .h(rem_px(70.))
                    .rounded(rem_px(8.))
                    .bg(rgb(GRAY_75))
                    .shadow_lg()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .ml(rem_px(16.))
                            .child(
                                Avatar::new(id.color_seed(), title.clone())
                                    .photo(self.photo_for(&id))
                                    .size(rem_px(52.)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .ml(rem_px(12.))
                                    .text_color(rgb(0xFFFFFF))
                                    .child(
                                        div()
                                            .id("incoming-call-name")
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(title)
                                            .test_support(),
                                    )
                                    .child(div().text_size(rem_px(13.)).child(kind)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .mr(rem_px(16.))
                            .child(
                                round_button(
                                    "call-decline",
                                    signal_icons::PHONE_DOWN,
                                    40.,
                                    24.,
                                    rgb(RED).into(),
                                    rgb(0xFFFFFF).into(),
                                )
                                .mx(rem_px(12.))
                                .on_click(cx.listener(|this, _, _, cx| this.hang_up(cx)))
                                .test_support(),
                            )
                            .child(
                                round_button(
                                    "call-accept",
                                    signal_icons::PHONE_FILL,
                                    40.,
                                    24.,
                                    rgb(GREEN).into(),
                                    rgb(0xFFFFFF).into(),
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.accept_call(cx)))
                                .test_support(),
                            ),
                    )
                    .test_support(),
            )
            .into_any_element()
    }

    fn render_call_screen(&self, info: &CallInfo, cx: &mut Context<Self>) -> AnyElement {
        let id = ConversationId::Direct(info.contact);
        let title = self.call_title(&info.contact);
        let ended = info.ended();
        let status = call_status(info, now_millis());
        let (mute_icon, mute_bg, mute_fg, mute_tip) = if info.muted {
            (signal_icons::MIC_OFF, GRAY_15, GRAY_90, "Unmute")
        } else {
            (signal_icons::MIC, GRAY_60, GRAY_15, "Mute")
        };
        div()
            .id("call-screen")
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .bg(rgb(GRAY_90))
            .text_color(rgb(0xFFFFFF))
            // swallow clicks so the app underneath stays inert
            .on_click(|_, _, _| {})
            .child(
                div()
                    .flex()
                    .flex_1()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(
                        Avatar::new(id.color_seed(), title.clone())
                            .photo(self.photo_for(&id))
                            .size(rem_px(112.)),
                    )
                    .child(
                        div()
                            .id("call-name")
                            .mt_2()
                            .text_size(rem_px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title)
                            .test_support(),
                    )
                    .child(
                        div()
                            .id("call-status")
                            .text_size(rem_px(14.))
                            .text_color(rgb(GRAY_25))
                            .child(status)
                            .test_support(),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rem_px(24.))
                    .pb(rem_px(40.))
                    .when(!ended, |d| {
                        d.child(
                            round_button(
                                "call-mute",
                                mute_icon,
                                56.,
                                24.,
                                rgb(mute_bg).into(),
                                rgb(mute_fg).into(),
                            )
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(mute_tip)
                                    .build(window, cx)
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_call_mute(cx)))
                            .test_support(),
                        )
                        .child(
                            round_button(
                                "call-hangup",
                                signal_icons::PHONE_DOWN,
                                56.,
                                24.,
                                rgb(RED).into(),
                                rgb(0xFFFFFF).into(),
                            )
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new("End call")
                                    .build(window, cx)
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.hang_up(cx)))
                            .test_support(),
                        )
                    })
                    .when(ended, |d| {
                        d.child(
                            round_button(
                                "call-ended",
                                signal_icons::PHONE_DOWN,
                                56.,
                                24.,
                                rgb(GRAY_45).into(),
                                rgb(0xFFFFFF).into(),
                            )
                            .test_support(),
                        )
                    }),
            )
            .test_support()
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(phase: CallPhase) -> CallInfo {
        CallInfo {
            contact: Uuid::nil(),
            outgoing: true,
            video_offered: false,
            phase,
            muted: false,
            remote_muted: false,
        }
    }

    #[test]
    fn status_text() {
        assert_eq!(call_status(&info(CallPhase::Calling), 0), "Calling…");
        assert_eq!(
            call_status(&info(CallPhase::OutgoingRinging), 0),
            "Ringing…"
        );
        assert_eq!(
            call_status(&info(CallPhase::Connected { since: 0 }), 65_000),
            "01:05"
        );
        assert_eq!(
            call_status(&info(CallPhase::Connected { since: 0 }), 3_725_000),
            "1:02:05"
        );
        let mut muted = info(CallPhase::Connected { since: 0 });
        muted.remote_muted = true;
        assert_eq!(call_status(&muted, 1_000), "00:01 · Microphone off");
        assert_eq!(
            call_status(
                &info(CallPhase::Ended {
                    reason: "Busy".into()
                }),
                0
            ),
            "Busy"
        );
    }
}
