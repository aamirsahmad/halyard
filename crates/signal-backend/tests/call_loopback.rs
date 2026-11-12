//! A real RingRTC voice call between two engines in this process.
//!
//! All signaling goes through Halyard's `CallMessage` encoding (as it would
//! over Signal), and media connects over the local network with no relay.
//! It needs audio devices, so it does not run by default:
//!
//!     cargo test -p signal-backend --test call_loopback -- --ignored --nocapture

use std::time::{Duration, Instant};

use presage::libsignal_service::prelude::ProtobufMessage;
use presage::libsignal_service::proto::CallMessage;
use signal_backend::calls::{CallEngine, CallEvent, EndReason, StateChange};
use tokio::sync::mpsc;
use uuid::Uuid;

const ALICE: Uuid = Uuid::from_u128(0xa11ce);
const BOB: Uuid = Uuid::from_u128(0xb0b);
const ALICE_KEY: [u8; 32] = [0xaa; 32];
const BOB_KEY: [u8; 32] = [0xbb; 32];
const DEVICE: u32 = 1;

struct Side {
    me: Uuid,
    key: [u8; 32],
    engine: CallEngine,
    rx: mpsc::UnboundedReceiver<CallEvent>,
    states: Vec<StateChange>,
}

impl Side {
    fn new(me: Uuid, key: [u8; 32]) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            me,
            key,
            engine: CallEngine::new(tx).expect("call engine"),
            rx,
            states: Vec::new(),
        }
    }
}

/// Handles one side's events; returns signaling to deliver to the other side.
fn pump(side: &mut Side) -> Vec<CallMessage> {
    let mut out = Vec::new();
    while let Ok(ev) = side.rx.try_recv() {
        match ev {
            CallEvent::Signal {
                call_id, message, ..
            } => {
                let message = *message;
                // serialize like the network would
                let bytes = message.encode_to_vec();
                out.push(CallMessage::decode(bytes.as_slice()).expect("decode"));
                side.engine.message_sent(call_id, true);
            }
            CallEvent::State { call_id, state, .. } => {
                eprintln!(
                    "{}: {state:?}",
                    if side.me == ALICE { "alice" } else { "bob" }
                );
                if matches!(state, StateChange::Incoming { .. } | StateChange::Outgoing) {
                    side.engine.proceed(call_id, &[], false);
                }
                side.states.push(state);
            }
            CallEvent::RemoteAudio { .. } => {}
        }
    }
    out
}

fn deliver(to: &mut Side, from: &Side, messages: Vec<CallMessage>) {
    for m in messages {
        to.engine.receive(
            from.me,
            DEVICE,
            DEVICE,
            (&from.key, &to.key),
            Duration::ZERO,
            &m,
        );
    }
}

fn run_until(
    alice: &mut Side,
    bob: &mut Side,
    limit: Duration,
    done: impl Fn(&Side, &Side) -> bool,
) {
    let start = Instant::now();
    while !done(alice, bob) {
        assert!(
            start.elapsed() < limit,
            "timed out: alice {:?}, bob {:?}",
            alice.states,
            bob.states
        );
        let to_bob = pump(alice);
        deliver(bob, alice, to_bob);
        let to_alice = pump(bob);
        deliver(alice, bob, to_alice);
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Bob answers the ringing call.
fn answer(alice: &Side, bob: &mut Side) {
    let call_id = alice.engine.current.as_ref().expect("alice's call").call_id;
    bob.engine.current = Some(signal_backend::calls::Current {
        call_id,
        peer: ALICE,
        outgoing: false,
        video_offered: false,
        phase: signal_core::CallPhase::IncomingRinging,
        accepted: false,
        muted: false,
        remote_muted: false,
        started_at: 0,
        connected_at: None,
    });
    bob.engine.accept();
}

#[test]
#[ignore = "needs audio devices; run with --ignored"]
fn voice_call_connects_and_hangs_up() {
    let mut alice = Side::new(ALICE, ALICE_KEY);
    let mut bob = Side::new(BOB, BOB_KEY);

    alice.engine.start(BOB, DEVICE, 0);
    run_until(&mut alice, &mut bob, Duration::from_secs(20), |a, b| {
        a.states.contains(&StateChange::Ringing)
            && b.states
                .iter()
                .any(|s| matches!(s, StateChange::Incoming { video: false }))
    });

    answer(&alice, &mut bob);
    run_until(&mut alice, &mut bob, Duration::from_secs(20), |a, b| {
        a.states.contains(&StateChange::Connected) && b.states.contains(&StateChange::Connected)
    });

    // talk for a moment, then alice hangs up
    std::thread::sleep(Duration::from_secs(2));
    alice.engine.hang_up();
    run_until(&mut alice, &mut bob, Duration::from_secs(20), |a, b| {
        a.states
            .contains(&StateChange::Ended(EndReason::LocalHangup))
            && b.states
                .contains(&StateChange::Ended(EndReason::RemoteHangup))
    });
}

#[test]
#[ignore = "needs audio devices; run with --ignored"]
fn mismatched_identity_keys_do_not_connect() {
    // If the identity keys differ between the sides, the call's keys differ
    // and media must not connect. This guards the key handling.
    let mut alice = Side::new(ALICE, ALICE_KEY);
    let mut bob = Side::new(BOB, BOB_KEY);
    bob.key = [0xcc; 32]; // bob thinks his key is something alice doesn't
    alice.engine.start(BOB, DEVICE, 0);
    run_until(&mut alice, &mut bob, Duration::from_secs(20), |_, b| {
        b.states
            .iter()
            .any(|s| matches!(s, StateChange::Incoming { .. }))
    });
    answer(&alice, &mut bob);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        let to_bob = pump(&mut alice);
        deliver(&mut bob, &alice, to_bob);
        let to_alice = pump(&mut bob);
        deliver(&mut alice, &bob, to_alice);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !alice.states.contains(&StateChange::Connected),
        "connected despite different keys: {:?}",
        alice.states
    );
    alice.engine.hang_up();
}
