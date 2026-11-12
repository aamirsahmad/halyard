//! Integration tests of the real (presage) backend thread.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use signal_backend::{BackendConfig, DataPaths, EventSink, SignalServers};
use signal_core::{AccountState, BackendCommand, BackendEvent};

fn start(dir: &std::path::Path) -> (signal_backend::BackendHandle, Arc<Mutex<Vec<BackendEvent>>>) {
    let paths = DataPaths::new(dir);
    let key = paths.load_or_create_db_key().unwrap();
    let events: Arc<Mutex<Vec<BackendEvent>>> = Arc::default();
    let sink: EventSink = {
        let events = events.clone();
        Arc::new(move |e| events.lock().unwrap().push(e))
    };
    let handle = signal_backend::spawn(
        BackendConfig {
            paths,
            db_key: key,
            servers: SignalServers::Production,
        },
        sink,
    )
    .unwrap();
    (handle, events)
}

fn wait_for(
    events: &Arc<Mutex<Vec<BackendEvent>>>,
    timeout: Duration,
    pred: impl Fn(&BackendEvent) -> bool,
) -> Option<BackendEvent> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(e) = events.lock().unwrap().iter().find(|e| pred(e)) {
            return Some(e.clone());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

#[test]
fn fresh_install_starts_unlinked_and_creates_encrypted_stores() {
    let dir = tempfile::tempdir().unwrap();
    let (handle, events) = start(dir.path());
    let ev = wait_for(&events, Duration::from_secs(20), |e| {
        matches!(e, BackendEvent::Account(_))
    });
    assert_eq!(ev, Some(BackendEvent::Account(AccountState::Unlinked)));
    // sending while unlinked is rejected gracefully
    handle.send(BackendCommand::SendText {
        conversation: signal_core::ConversationId::Direct(uuid::Uuid::nil()),
        body: "hi".into(),
        quote: None,
        attachments: vec![],
        ranges: vec![],
    });
    let err = wait_for(&events, Duration::from_secs(5), |e| {
        matches!(e, BackendEvent::Error(_))
    });
    assert!(matches!(err, Some(BackendEvent::Error(m)) if m.contains("Not linked")));
    handle.shutdown();

    let paths = DataPaths::new(dir.path());
    for db in [paths.protocol_db(), paths.app_db()] {
        let bytes = std::fs::read(&db).unwrap();
        assert!(
            !bytes.starts_with(b"SQLite format 3"),
            "{} must be encrypted",
            db.display()
        );
    }
}

/// Talks to Signal's production provisioning service. Run with
/// `cargo test -p signal-backend --test worker -- --ignored`.
#[test]
#[ignore = "requires network access to Signal servers"]
fn linking_produces_a_provisioning_url() {
    let dir = tempfile::tempdir().unwrap();
    let (handle, events) = start(dir.path());
    wait_for(&events, Duration::from_secs(20), |e| {
        matches!(e, BackendEvent::Account(_))
    })
    .unwrap();
    handle.send(BackendCommand::Link {
        device_name: "halyard test".into(),
    });
    let ev = wait_for(&events, Duration::from_secs(60), |e| {
        matches!(
            e,
            BackendEvent::Account(AccountState::Linking { url: Some(_) })
                | BackendEvent::Account(AccountState::LinkFailed(_))
        )
    });
    handle.shutdown();
    match ev {
        Some(BackendEvent::Account(AccountState::Linking { url: Some(url) })) => {
            assert!(url.starts_with("sgnl://linkdevice?"), "{url}");
            assert!(url.contains("uuid=") && url.contains("pub_key="), "{url}");
        }
        other => panic!("expected a provisioning url, got {other:?}"),
    }
}
