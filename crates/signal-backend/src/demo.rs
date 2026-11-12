//! An offline backend with sample conversations. It implements the same
//! command/event contract as the real backend, so the UI can be developed,
//! demoed and tested without a Signal account (`--demo`).

use std::time::Duration;

use signal_core::{
    AccountInfo, AccountState, BackendCommand, BackendEvent, ConnectionState, Contact,
    ConversationId, Database, GroupKey, MessageKind, MessageStatus, Millis, Mutation, NewMessage,
    ReceiptKind, now_millis,
};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::worker::{BackendHandle, EventSink, next_timestamp};

pub const DEMO_ME: Uuid = Uuid::from_u128(0x5157_0000_0000_0000_0000_0000_0000_0001);
pub const DEMO_ALICE: Uuid = Uuid::from_u128(0x5157_0000_0000_0000_0000_0000_0000_00a1);
pub const DEMO_BOB: Uuid = Uuid::from_u128(0x5157_0000_0000_0000_0000_0000_0000_00b0);
pub const DEMO_CAROL: Uuid = Uuid::from_u128(0x5157_0000_0000_0000_0000_0000_0000_00c0);
pub const DEMO_DAVE: Uuid = Uuid::from_u128(0x5157_0000_0000_0000_0000_0000_0000_00d0);
pub const DEMO_STRANGER: Uuid = Uuid::from_u128(0x5157_0000_0000_0000_0000_0000_0000_00e0);
pub const DEMO_GROUP: GroupKey = GroupKey([0x42; 32]);

/// Sample data. Applied only when the database has no conversations.
pub fn seed(db: &mut Database, now: Millis) -> signal_core::db::Result<()> {
    if !db.all_conversations()?.is_empty() {
        return Ok(());
    }
    let contacts = [
        (DEMO_ALICE, "Alice Johnson", "+1 555 0101"),
        (DEMO_BOB, "Bob Smith", "+1 555 0102"),
        (DEMO_CAROL, "Carol Diaz", "+1 555 0103"),
        (DEMO_DAVE, "Dave", "+1 555 0104"),
        (DEMO_ME, "Sam Taylor", "+1 555 0100"),
    ];
    for (uuid, name, phone) in contacts {
        db.apply(Mutation::UpsertContact(Contact {
            uuid,
            name: Some(name.into()),
            profile_name: None,
            phone: Some(phone.into()),
        }))?;
    }
    db.apply(Mutation::UpsertGroup {
        key: DEMO_GROUP,
        title: "Weekend Hiking 🥾".into(),
        members: vec![DEMO_ME, DEMO_ALICE, DEMO_BOB, DEMO_CAROL],
    })?;
    let min = 60_000;
    let day = 24 * 60 * min;
    let alice = ConversationId::Direct(DEMO_ALICE);
    let bob = ConversationId::Direct(DEMO_BOB);
    let carol = ConversationId::Direct(DEMO_CAROL);
    let group = ConversationId::Group(DEMO_GROUP);
    let note = ConversationId::Direct(DEMO_ME);
    let script: Vec<(ConversationId, Uuid, Millis, &str, bool)> = vec![
        (
            bob,
            DEMO_BOB,
            now - 3 * day,
            "Did you get a chance to look at the proposal?",
            true,
        ),
        (
            bob,
            DEMO_ME,
            now - 3 * day + 4 * min,
            "Yes! Left some comments, looks great overall",
            true,
        ),
        (bob, DEMO_BOB, now - 3 * day + 6 * min, "Thanks 🙏", true),
        (
            note,
            DEMO_ME,
            now - 4 * day,
            "Groceries: oat milk, coffee beans, basil",
            true,
        ),
        (
            note,
            DEMO_ME,
            now - 4 * day + min,
            "Book dentist appointment",
            true,
        ),
        (
            group,
            DEMO_ALICE,
            now - day - 90 * min,
            "Who's in for Saturday? Weather looks perfect ☀️",
            true,
        ),
        (
            group,
            DEMO_CAROL,
            now - day - 80 * min,
            "Count me in!",
            true,
        ),
        (
            group,
            DEMO_ME,
            now - day - 70 * min,
            "Me too. Meet at the trailhead at 8?",
            true,
        ),
        (
            group,
            DEMO_BOB,
            now - 50 * min,
            "8 works. I'll bring snacks",
            false,
        ),
        (
            group,
            DEMO_ALICE,
            now - 49 * min,
            "Perfect, see you all there",
            false,
        ),
        (
            carol,
            DEMO_CAROL,
            now - 2 * day,
            "Here's the recipe I mentioned",
            true,
        ),
        (
            carol,
            DEMO_ME,
            now - 2 * day + 2 * min,
            "Amazing, thank you!",
            true,
        ),
        (
            alice,
            DEMO_ALICE,
            now - 20 * min,
            "Hey! Are we still on for coffee tomorrow?",
            true,
        ),
        (
            alice,
            DEMO_ME,
            now - 18 * min,
            "Absolutely. 10am at the usual place?",
            true,
        ),
        (alice, DEMO_ALICE, now - 17 * min, "Sounds good", true),
        (
            alice,
            DEMO_ALICE,
            now - 16 * min,
            "I'll grab us a table by the window",
            false,
        ),
    ];
    for (conv, author, ts, body, read) in script {
        let outgoing = author == DEMO_ME;
        db.apply(Mutation::InsertMessage(NewMessage {
            conversation: conv,
            author,
            sent_at: ts,
            received_at: ts,
            outgoing,
            body: body.into(),
            attachments: vec![],
            quote: None,
            status: if outgoing {
                MessageStatus::Read
            } else {
                MessageStatus::Received
            },
            read,
            expires_in_secs: None,
            kind: MessageKind::Normal,
            body_ranges: vec![],
            link_preview: None,
        }))?;
    }
    db.apply(Mutation::React {
        conversation: group,
        target_author: DEMO_ME,
        target_sent_at: now - day - 70 * min,
        reactor: DEMO_ALICE,
        emoji: "👍".into(),
        remove: false,
        at: now - day - 69 * min,
    })?;
    // formatting, a mention and a link, like Signal's rich messages
    let mut formatted = NewMessage::text(
        carol,
        DEMO_CAROL,
        now - 2 * day + 3 * min,
        false,
        "It's the *best* soup: https://example.com/recipes/tomato-basil",
    );
    formatted.read = true;
    formatted.body_ranges = vec![
        signal_core::BodyRange {
            start: 9,
            length: 4,
            kind: signal_core::RangeKind::Bold,
        },
        signal_core::BodyRange {
            start: 5,
            length: 3,
            kind: signal_core::RangeKind::Italic,
        },
    ];
    formatted.body = "It's the best soup: https://example.com/recipes/tomato-basil".into();
    formatted.link_preview = Some(signal_core::LinkPreview {
        url: "https://example.com/recipes/tomato-basil".into(),
        title: Some("Tomato basil soup".into()),
        description: Some(
            "A 20-minute weeknight soup with roasted tomatoes and fresh basil.".into(),
        ),
    });
    db.apply(Mutation::InsertMessage(formatted))?;
    let mut mention = NewMessage::text(
        group,
        DEMO_CAROL,
        now - 48 * min,
        false,
        "\u{FFFC} can you bring the map?",
    );
    mention.body_ranges = vec![signal_core::BodyRange {
        start: 0,
        length: 1,
        kind: signal_core::RangeKind::Mention(DEMO_ME),
    }];
    db.apply(Mutation::InsertMessage(mention))?;
    // a disappearing-messages notice in Bob's chat
    db.apply(Mutation::SetExpireTimer {
        conversation: bob,
        secs: Some(7 * 86_400),
        author: DEMO_BOB,
        outgoing: false,
        at: now - 3 * day + 10 * min,
    })?;
    // a message request from someone not in your contacts
    let stranger = ConversationId::Direct(DEMO_STRANGER);
    db.apply(Mutation::InsertMessage(NewMessage::text(
        stranger,
        DEMO_STRANGER,
        now - 5 * min,
        false,
        "Hi! We met at the conference — is this the right number?",
    )))?;
    // stories from contacts (they expire after 24 hours)
    db.apply(Mutation::InsertStory(signal_core::NewStory {
        author: DEMO_ALICE,
        sent_at: now - 2 * 60 * min,
        text: Some("Fresh snow on the trail ❄️".into()),
        background: Some(0xFF2C6BED),
        foreground: Some(0xFFFFFFFF),
        attachment: None,
    }))?;
    db.apply(Mutation::InsertStory(signal_core::NewStory {
        author: DEMO_ALICE,
        sent_at: now - 90 * min,
        text: Some("Who's bringing hot chocolate?".into()),
        background: Some(0xFFB8057C),
        foreground: Some(0xFFFFFFFF),
        attachment: None,
    }))?;
    db.apply(Mutation::InsertStory(signal_core::NewStory {
        author: DEMO_CAROL,
        sent_at: now - 5 * 60 * min,
        text: Some("New recipe blog post is up!".into()),
        background: Some(0xFF067906),
        foreground: Some(0xFFFFFFFF),
        attachment: None,
    }))?;
    db.set_pinned(&group, true)?;
    Ok(())
}

pub fn demo_account() -> AccountInfo {
    AccountInfo {
        aci: DEMO_ME,
        phone_number: Some("+1 555 0100".into()),
        device_id: 2,
        device_name: Some("Signal GPUI (demo)".into()),
    }
}

/// A stable, fake 60-digit safety number for demo contacts.
pub fn demo_safety_number(contact: &Uuid) -> String {
    let mut digits = String::with_capacity(60);
    let mut x = contact.as_u128() ^ DEMO_ME.as_u128();
    while digits.len() < 60 {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        digits.push(char::from(b'0' + ((x >> 64) % 10) as u8));
    }
    digits
}

/// Starts the demo backend using the database at `db_path`.
pub fn spawn_demo(
    db_path: std::path::PathBuf,
    db_key: Option<String>,
    events: EventSink,
    reply_delay: Duration,
) -> std::io::Result<BackendHandle> {
    let (tx, mut rx) = mpsc::unbounded_channel::<BackendCommand>();
    let thread = std::thread::Builder::new().name("signal-demo-backend".into()).spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        rt.block_on(async move {
            let mut db = match Database::open(&db_path, db_key.as_deref()) {
                Ok(db) => db,
                Err(e) => {
                    events(BackendEvent::Error(format!("Failed to open demo database: {e}")));
                    return;
                }
            };
            if let Err(e) = seed(&mut db, now_millis()) {
                events(BackendEvent::Error(format!("Failed to seed demo data: {e}")));
            }
            events(BackendEvent::Account(AccountState::Registered(demo_account())));
            events(BackendEvent::Connection(ConnectionState::Online));
            let (later_tx, mut later_rx) = mpsc::unbounded_channel::<Mutation>();
            let mut last_ts = 0;
            let mut sweep = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = sweep.tick() => {
                        if let Ok(ch) = db.apply(Mutation::ExpireDue { now: now_millis() }) {
                            ch.into_events().into_iter().for_each(|e| events(e));
                        }
                    }
                    cmd = rx.recv() => {
                        let Some(cmd) = cmd else { break };
                        if cmd == BackendCommand::Shutdown { break; }
                        last_ts = next_timestamp(last_ts, now_millis());
                        for m in demo_reaction(cmd, last_ts, &db, &later_tx, reply_delay, &events) {
                            match db.apply(m) {
                                Ok(ch) => ch.into_events().into_iter().for_each(|e| events(e)),
                                Err(e) => events(BackendEvent::Error(e.to_string())),
                            }
                        }
                    }
                    Some(m) = later_rx.recv() => {
                        if let Mutation::InsertMessage(n) = &m {
                            events(BackendEvent::Typing { conversation: n.conversation, who: n.author, started: false });
                        }
                        match db.apply(m) {
                            Ok(ch) => ch.into_events().into_iter().for_each(|e| events(e)),
                            Err(e) => events(BackendEvent::Error(e.to_string())),
                        }
                    }
                }
            }
        });
    })?;
    Ok(BackendHandle::from_parts(tx, thread))
}

/// Computes the immediate mutations for a command and schedules simulated
/// remote activity (receipts, replies).
fn demo_reaction(
    cmd: BackendCommand,
    ts: Millis,
    db: &Database,
    later: &mpsc::UnboundedSender<Mutation>,
    delay: Duration,
    events: &EventSink,
) -> Vec<Mutation> {
    let schedule = |after: Duration, m: Mutation| {
        let later = later.clone();
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            let _ = later.send(m);
        });
    };
    match cmd {
        BackendCommand::LeaveGroup { group } => vec![Mutation::GroupMember {
            conversation: ConversationId::Group(group),
            actor: DEMO_ME,
            member: DEMO_ME,
            change: signal_core::MemberChange::Left,
            outgoing: true,
            member_is_me: true,
            at: ts,
        }],
        BackendCommand::Forward {
            targets,
            body,
            attachments,
        } => targets
            .into_iter()
            .flat_map(|conversation| {
                let cmd = BackendCommand::SendText {
                    conversation,
                    body: body.clone(),
                    quote: None,
                    attachments: attachments.clone(),
                    ranges: vec![],
                };
                demo_reaction(cmd, ts, db, later, delay, events)
            })
            .collect(),
        BackendCommand::ReplyToStory {
            author,
            story_sent_at,
            body,
        } => {
            let mut m = NewMessage::text(ConversationId::Direct(author), DEMO_ME, ts, true, body);
            m.quote = Some(signal_core::Quote {
                author,
                sent_at: story_sent_at,
                text: signal_core::STORY_REPLY_QUOTE.into(),
            });
            vec![Mutation::InsertMessage(m)]
        }
        BackendCommand::SetExpireTimer { conversation, secs } => vec![Mutation::SetExpireTimer {
            conversation,
            secs,
            author: DEMO_ME,
            outgoing: true,
            at: ts,
        }],
        BackendCommand::SendText {
            conversation,
            body,
            quote,
            attachments,
            ranges,
        } => {
            let timer = db
                .conversation(&conversation)
                .ok()
                .flatten()
                .and_then(|c| c.expire_timer);
            let atts = attachments
                .iter()
                .map(|p| signal_core::Attachment {
                    content_type: mime_guess::from_path(p)
                        .first_or_octet_stream()
                        .essence_str()
                        .into(),
                    file_name: p.file_name().map(|n| n.to_string_lossy().into_owned()),
                    size: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
                    local_path: Some(p.to_string_lossy().into_owned()),
                    ..Default::default()
                })
                .collect();
            let peer = match conversation {
                ConversationId::Direct(u) => u,
                ConversationId::Group(_) => DEMO_ALICE,
            };
            schedule(
                delay / 4,
                Mutation::SendStatus {
                    conversation,
                    sent_at: ts,
                    status: MessageStatus::Sent,
                },
            );
            schedule(
                delay / 2,
                Mutation::Receipt {
                    from: peer,
                    kind: ReceiptKind::Delivered,
                    sent_ats: vec![ts],
                },
            );
            schedule(
                delay,
                Mutation::Receipt {
                    from: peer,
                    kind: ReceiptKind::Read,
                    sent_ats: vec![ts],
                },
            );
            // simulated reply
            events(BackendEvent::Typing {
                conversation,
                who: peer,
                started: true,
            });
            let reply = format!("You said: “{}”", body.trim());
            schedule(
                delay * 2,
                Mutation::InsertMessage(NewMessage {
                    conversation,
                    author: peer,
                    sent_at: ts + delay.as_millis() as Millis * 2,
                    received_at: ts + delay.as_millis() as Millis * 2,
                    outgoing: false,
                    body: reply,
                    attachments: vec![],
                    quote: None,
                    status: MessageStatus::Received,
                    read: false,
                    expires_in_secs: timer,
                    kind: MessageKind::Normal,
                    body_ranges: vec![],
                    link_preview: None,
                }),
            );
            vec![Mutation::InsertMessage(NewMessage {
                conversation,
                author: DEMO_ME,
                sent_at: ts,
                received_at: ts,
                outgoing: true,
                body: body.trim_end().to_string(),
                attachments: atts,
                quote,
                status: MessageStatus::Sending,
                read: true,
                expires_in_secs: timer,
                kind: MessageKind::Normal,
                body_ranges: ranges,
                link_preview: None,
            })]
        }
        BackendCommand::Resend {
            conversation,
            sent_at,
        } => {
            vec![Mutation::SendStatus {
                conversation,
                sent_at,
                status: MessageStatus::Sent,
            }]
        }
        BackendCommand::React {
            conversation,
            target_author,
            target_sent_at,
            emoji,
            remove,
        } => {
            vec![Mutation::React {
                conversation,
                target_author,
                target_sent_at,
                reactor: DEMO_ME,
                emoji,
                remove,
                at: ts,
            }]
        }
        BackendCommand::DeleteForEveryone {
            conversation,
            sent_at,
        } => {
            vec![Mutation::DeleteForEveryone {
                conversation,
                author: DEMO_ME,
                target_sent_at: sent_at,
            }]
        }
        BackendCommand::Edit {
            conversation,
            target_sent_at,
            body,
        } => vec![Mutation::Edit {
            conversation,
            author: DEMO_ME,
            target_sent_at,
            body,
            edited_at: ts,
        }],
        BackendCommand::MarkRead { conversation } => {
            vec![Mutation::MarkConversationRead { conversation }]
        }
        BackendCommand::Unlink => {
            let _ = db; // demo data is kept; nothing to unlink
            events(BackendEvent::Error(
                "Unlinking is not available in demo mode".into(),
            ));
            vec![]
        }
        BackendCommand::RequestSafetyNumber { contact } => {
            events(BackendEvent::SafetyNumber {
                contact,
                number: Some(demo_safety_number(&contact)),
            });
            vec![]
        }
        BackendCommand::UpdateProfile {
            given_name,
            family_name,
            about,
        } => {
            let name = match family_name.filter(|f| !f.trim().is_empty()) {
                Some(f) => format!("{given_name} {f}"),
                None => given_name,
            };
            vec![
                Mutation::UpsertContact(Contact {
                    uuid: DEMO_ME,
                    name: None,
                    profile_name: Some(name),
                    phone: None,
                }),
                Mutation::SetAbout {
                    conversation: ConversationId::Direct(DEMO_ME),
                    about,
                },
            ]
        }
        BackendCommand::Link { .. }
        | BackendCommand::FetchAvatar { .. }
        | BackendCommand::SetReadReceipts(_)
        | BackendCommand::Typing { .. }
        | BackendCommand::SyncContacts
        | BackendCommand::Shutdown => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn seed_is_idempotent() {
        let mut db = Database::open_in_memory().unwrap();
        seed(&mut db, 1_000_000_000_000).unwrap();
        let n = db.all_conversations().unwrap().len();
        assert_eq!(n, 7); // 4 contacts + group + Note to Self + a message request
        seed(&mut db, 1_000_000_000_000).unwrap();
        assert_eq!(db.all_conversations().unwrap().len(), n);
        let list = db.conversations().unwrap();
        assert_eq!(
            list[0].id,
            ConversationId::Group(DEMO_GROUP),
            "pinned first"
        );
        assert_eq!(list[0].unread_count, 3);
    }

    #[test]
    fn demo_backend_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let events: Arc<Mutex<Vec<BackendEvent>>> = Arc::default();
        let sink = {
            let events = events.clone();
            Arc::new(move |e| events.lock().unwrap().push(e)) as EventSink
        };
        let handle = spawn_demo(
            dir.path().join("demo.db"),
            None,
            sink,
            Duration::from_millis(40),
        )
        .unwrap();
        let alice = ConversationId::Direct(DEMO_ALICE);
        handle.send(BackendCommand::SendText {
            conversation: alice,
            body: "ping".into(),
            quote: None,
            attachments: vec![],
            ranges: vec![],
        });
        // wait for the simulated reply
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let done = events.lock().unwrap().iter().any(|e| {
                matches!(e, BackendEvent::MessageUpserted(m) if !m.outgoing && m.body.contains("ping"))
            });
            if done || std::time::Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        handle.shutdown();
        let events = events.lock().unwrap();
        assert!(matches!(
            events[0],
            BackendEvent::Account(AccountState::Registered(_))
        ));
        let statuses: Vec<MessageStatus> = events
            .iter()
            .filter_map(|e| match e {
                BackendEvent::MessageUpserted(m) if m.outgoing && m.body == "ping" => {
                    Some(m.status)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            statuses,
            vec![
                MessageStatus::Sending,
                MessageStatus::Sent,
                MessageStatus::Delivered,
                MessageStatus::Read
            ]
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, BackendEvent::Typing { started: true, .. }))
        );
        assert!(events.iter().any(
            |e| matches!(e, BackendEvent::MessageUpserted(m) if m.body == "You said: “ping”")
        ));
    }
}
