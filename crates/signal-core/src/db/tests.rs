use super::*;
use crate::ids::GroupKey;
use crate::model::{Attachment, MessageKind, Quote};
use crate::protocol::ReceiptKind;

const ME: Uuid = Uuid::from_u128(0xAAAA);
const ALICE: Uuid = Uuid::from_u128(0xA11CE);
const BOB: Uuid = Uuid::from_u128(0xB0B);

fn alice() -> ConversationId {
    ConversationId::Direct(ALICE)
}

fn incoming(conv: ConversationId, author: Uuid, sent_at: Millis, body: &str) -> Mutation {
    Mutation::InsertMessage(NewMessage {
        conversation: conv,
        author,
        sent_at,
        received_at: sent_at + 5,
        outgoing: false,
        body: body.into(),
        attachments: vec![],
        quote: None,
        status: MessageStatus::Received,
        read: false,
        expires_in_secs: None,
        kind: MessageKind::Normal,
        body_ranges: vec![],
        link_preview: None,
    })
}

fn outgoing(conv: ConversationId, sent_at: Millis, body: &str) -> Mutation {
    Mutation::InsertMessage(NewMessage {
        conversation: conv,
        author: ME,
        sent_at,
        received_at: sent_at,
        outgoing: true,
        body: body.into(),
        attachments: vec![],
        quote: None,
        status: MessageStatus::Sending,
        read: true,
        expires_in_secs: None,
        kind: MessageKind::Normal,
        body_ranges: vec![],
        link_preview: None,
    })
}

fn db() -> Database {
    Database::open_in_memory().unwrap()
}

#[test]
fn migrates_to_latest() {
    let db = db();
    assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
}

#[test]
fn reopening_is_idempotent_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    {
        let mut db = Database::open(&path, None).unwrap();
        db.apply(incoming(alice(), ALICE, 10, "hi")).unwrap();
    }
    let db = Database::open(&path, None).unwrap();
    assert_eq!(db.messages(&alice(), None, 10).unwrap().len(), 1);
}

#[test]
fn encrypted_database_requires_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("enc.db");
    {
        let mut db = Database::open(&path, Some("correct horse")).unwrap();
        db.apply(incoming(alice(), ALICE, 10, "secret")).unwrap();
    }
    assert!(matches!(
        Database::open(&path, Some("wrong")),
        Err(DbError::BadKey)
    ));
    assert!(matches!(Database::open(&path, None), Err(DbError::BadKey)));
    let db = Database::open(&path, Some("correct horse")).unwrap();
    assert_eq!(db.messages(&alice(), None, 10).unwrap()[0].body, "secret");
    // plaintext must not appear on disk
    drop(db);
    let raw = std::fs::read(&path).unwrap();
    assert!(!raw.windows(6).any(|w| w == b"secret"));
}

#[test]
fn insert_creates_conversation_with_contact_title_and_unread() {
    let mut db = db();
    db.apply(Mutation::UpsertContact(Contact {
        uuid: ALICE,
        name: Some("Alice".into()),
        profile_name: None,
        phone: None,
    }))
    .unwrap();
    let ch = db
        .apply(incoming(alice(), ALICE, 100, "hello\nsecond line"))
        .unwrap();
    assert_eq!(ch.messages.len(), 1);
    let conv = &ch.conversations[0];
    assert_eq!(conv.title, "Alice");
    assert_eq!(conv.unread_count, 1);
    assert_eq!(conv.last_preview, "hello");
    assert_eq!(conv.last_activity, 100);
    assert!(!conv.last_preview_outgoing);
}

#[test]
fn unknown_sender_gets_placeholder_title_then_renamed_by_contact_sync() {
    let mut db = db();
    db.apply(incoming(alice(), ALICE, 100, "hey")).unwrap();
    assert_eq!(
        db.conversation(&alice()).unwrap().unwrap().title,
        "Unknown contact"
    );
    let ch = db
        .apply(Mutation::UpsertContact(Contact {
            uuid: ALICE,
            name: None,
            profile_name: Some("Ally".into()),
            phone: Some("+1555".into()),
        }))
        .unwrap();
    assert_eq!(ch.conversations[0].title, "Ally");
    // a later partial update keeps known fields
    db.apply(Mutation::UpsertContact(Contact {
        uuid: ALICE,
        name: None,
        profile_name: None,
        phone: None,
    }))
    .unwrap();
    assert_eq!(
        db.contact(&ALICE).unwrap().unwrap().phone.as_deref(),
        Some("+1555")
    );
}

#[test]
fn duplicate_messages_are_ignored() {
    let mut db = db();
    db.apply(incoming(alice(), ALICE, 100, "a")).unwrap();
    let ch = db.apply(incoming(alice(), ALICE, 100, "a")).unwrap();
    assert!(ch.is_empty());
    assert_eq!(db.messages(&alice(), None, 10).unwrap().len(), 1);
}

#[test]
fn pagination_returns_ascending_pages() {
    let mut db = db();
    for i in 1..=25 {
        db.apply(incoming(alice(), ALICE, i * 10, &format!("m{i}")))
            .unwrap();
    }
    let page1 = db.messages(&alice(), None, 10).unwrap();
    assert_eq!(page1.first().unwrap().body, "m16");
    assert_eq!(page1.last().unwrap().body, "m25");
    let first = &page1[0];
    let page2 = db
        .messages(&alice(), Some((first.sent_at, first.id)), 10)
        .unwrap();
    assert_eq!(page2.first().unwrap().body, "m6");
    assert_eq!(page2.last().unwrap().body, "m15");
    let page3 = db
        .messages(&alice(), Some((page2[0].sent_at, page2[0].id)), 10)
        .unwrap();
    assert_eq!(page3.len(), 5);
}

#[test]
fn conversations_sorted_pinned_then_recent() {
    let mut db = db();
    let bob = ConversationId::Direct(BOB);
    let group = ConversationId::Group(GroupKey([1; 32]));
    db.apply(Mutation::UpsertGroup {
        key: GroupKey([1; 32]),
        title: "Team".into(),
        members: vec![ALICE, BOB],
    })
    .unwrap();
    db.apply(incoming(alice(), ALICE, 100, "a")).unwrap();
    db.apply(incoming(bob, BOB, 200, "b")).unwrap();
    db.apply(incoming(group, ALICE, 50, "g")).unwrap();
    let order: Vec<_> = db
        .conversations()
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(order, vec![bob, alice(), group]);
    db.set_pinned(&group, true).unwrap();
    let order: Vec<_> = db
        .conversations()
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(order, vec![group, bob, alice()]);
    let g = db.conversation(&group).unwrap().unwrap();
    assert_eq!(g.members, vec![ALICE, BOB]);
    assert_eq!(g.title, "Team");
}

#[test]
fn empty_conversations_hidden_from_list_but_in_picker() {
    let mut db = db();
    db.apply(Mutation::UpsertContact(Contact {
        uuid: BOB,
        name: Some("Bob".into()),
        profile_name: None,
        phone: None,
    }))
    .unwrap();
    assert!(db.conversations().unwrap().is_empty());
    assert_eq!(db.all_conversations().unwrap().len(), 1);
    db.set_draft(&ConversationId::Direct(BOB), Some("draft"))
        .unwrap();
    assert_eq!(db.conversations().unwrap().len(), 1);
    db.set_draft(&ConversationId::Direct(BOB), Some("   "))
        .unwrap();
    assert!(db.conversations().unwrap().is_empty());
}

#[test]
fn receipts_advance_outgoing_status_only_for_matching_peer() {
    let mut db = db();
    db.apply(outgoing(alice(), 500, "yo")).unwrap();
    db.apply(Mutation::SendStatus {
        conversation: alice(),
        sent_at: 500,
        status: MessageStatus::Sent,
    })
    .unwrap();
    // receipt from someone else is ignored for a direct chat
    let ch = db
        .apply(Mutation::Receipt {
            from: BOB,
            kind: ReceiptKind::Read,
            sent_ats: vec![500],
        })
        .unwrap();
    assert!(ch.messages.is_empty());
    let ch = db
        .apply(Mutation::Receipt {
            from: ALICE,
            kind: ReceiptKind::Read,
            sent_ats: vec![500],
        })
        .unwrap();
    assert_eq!(ch.messages[0].status, MessageStatus::Read);
    // late delivery receipt does not regress
    let ch = db
        .apply(Mutation::Receipt {
            from: ALICE,
            kind: ReceiptKind::Delivered,
            sent_ats: vec![500],
        })
        .unwrap();
    assert!(ch.messages.is_empty());
    let m = &db.messages(&alice(), None, 1).unwrap()[0];
    assert_eq!(m.status, MessageStatus::Read);
}

#[test]
fn send_failure_marks_failed_and_outgoing_preview() {
    let mut db = db();
    let ch = db.apply(outgoing(alice(), 500, "yo")).unwrap();
    assert!(ch.conversations[0].last_preview_outgoing);
    assert_eq!(ch.conversations[0].unread_count, 0);
    let ch = db
        .apply(Mutation::SendStatus {
            conversation: alice(),
            sent_at: 500,
            status: MessageStatus::Failed,
        })
        .unwrap();
    assert_eq!(ch.messages[0].status, MessageStatus::Failed);
}

#[test]
fn reactions_replace_and_remove() {
    let mut db = db();
    db.apply(incoming(alice(), ALICE, 100, "a")).unwrap();
    let react = |emoji: &str, reactor, remove| Mutation::React {
        conversation: alice(),
        target_author: ALICE,
        target_sent_at: 100,
        reactor,
        emoji: emoji.into(),
        remove,
        at: 1,
    };
    db.apply(react("👍", ME, false)).unwrap();
    db.apply(react("❤️", ME, false)).unwrap();
    let ch = db.apply(react("😂", ALICE, false)).unwrap();
    let m = &ch.messages[0];
    assert_eq!(m.reactions.len(), 2);
    assert_eq!(
        m.reaction_summary(),
        vec![("❤️".into(), 1), ("😂".into(), 1)]
    );
    let ch = db.apply(react("❤️", ME, true)).unwrap();
    assert_eq!(ch.messages[0].reactions.len(), 1);
    // reaction to an unknown message is a no-op
    let ch = db
        .apply(Mutation::React {
            conversation: alice(),
            target_author: ALICE,
            target_sent_at: 999,
            reactor: ME,
            emoji: "👍".into(),
            remove: false,
            at: 1,
        })
        .unwrap();
    assert!(ch.is_empty());
}

#[test]
fn edits_and_deletes() {
    let mut db = db();
    db.apply(incoming(alice(), ALICE, 100, "typo")).unwrap();
    let ch = db
        .apply(Mutation::Edit {
            conversation: alice(),
            author: ALICE,
            target_sent_at: 100,
            body: "fixed".into(),
            edited_at: 150,
        })
        .unwrap();
    assert_eq!(ch.messages[0].body, "fixed");
    assert_eq!(ch.messages[0].edited_at, Some(150));
    assert_eq!(ch.conversations[0].last_preview, "fixed");
    // an edit cannot be forged by another author
    let ch = db
        .apply(Mutation::Edit {
            conversation: alice(),
            author: BOB,
            target_sent_at: 100,
            body: "evil".into(),
            edited_at: 160,
        })
        .unwrap();
    assert!(ch.is_empty());
    let ch = db
        .apply(Mutation::DeleteForEveryone {
            conversation: alice(),
            author: ALICE,
            target_sent_at: 100,
        })
        .unwrap();
    assert!(ch.messages[0].deleted);
    assert_eq!(ch.messages[0].body, "");
    assert_eq!(
        ch.conversations[0].last_preview,
        "This message was deleted."
    );
    assert_eq!(ch.conversations[0].unread_count, 0);
    // deleted messages can't be edited back
    let ch = db
        .apply(Mutation::Edit {
            conversation: alice(),
            author: ALICE,
            target_sent_at: 100,
            body: "x".into(),
            edited_at: 170,
        })
        .unwrap();
    assert!(ch.is_empty());
}

#[test]
fn read_tracking() {
    let mut db = db();
    for i in 1..=3 {
        db.apply(incoming(alice(), ALICE, i * 100, "m")).unwrap();
    }
    assert_eq!(db.unread_messages(&alice()).unwrap().len(), 3);
    // read sync of message 2 marks 1 and 2 as read
    let ch = db
        .apply(Mutation::ReadSync {
            author: ALICE,
            sent_at: 200,
        })
        .unwrap();
    assert_eq!(ch.conversations[0].unread_count, 1);
    let ch = db
        .apply(Mutation::MarkConversationRead {
            conversation: alice(),
        })
        .unwrap();
    assert_eq!(ch.conversations[0].unread_count, 0);
    // nothing left to mark: no change event
    let ch = db
        .apply(Mutation::MarkConversationRead {
            conversation: alice(),
        })
        .unwrap();
    assert!(ch.is_empty());
}

#[test]
fn search_escapes_like_wildcards() {
    let mut db = db();
    db.apply(incoming(alice(), ALICE, 1, "100% sure")).unwrap();
    db.apply(incoming(alice(), ALICE, 2, "1000 things"))
        .unwrap();
    db.apply(incoming(alice(), ALICE, 3, "snake_case")).unwrap();
    db.apply(incoming(alice(), ALICE, 4, "snakeXcase")).unwrap();
    assert_eq!(db.search_messages("0%", 10).unwrap().len(), 1);
    assert_eq!(db.search_messages("e_c", 10).unwrap().len(), 1);
    assert_eq!(db.search_messages("SURE", 10).unwrap().len(), 1);
    assert!(db.search_messages("  ", 10).unwrap().is_empty());
}

#[test]
fn attachments_and_quotes_roundtrip() {
    let mut db = db();
    let att = Attachment {
        content_type: "image/jpeg".into(),
        file_name: Some("a.jpg".into()),
        size: 1234,
        local_path: Some("/tmp/a.jpg".into()),
        width: Some(10),
        height: Some(20),
        pointer: Some(vec![1, 2, 3]),
    };
    let quote = Quote {
        author: ALICE,
        sent_at: 5,
        text: "orig".into(),
    };
    db.apply(Mutation::InsertMessage(NewMessage {
        conversation: alice(),
        author: ME,
        sent_at: 10,
        received_at: 10,
        outgoing: true,
        body: String::new(),
        attachments: vec![att.clone()],
        quote: Some(quote.clone()),
        status: MessageStatus::Sending,
        read: true,
        expires_in_secs: Some(3600),
        kind: MessageKind::Normal,
        body_ranges: vec![],
        link_preview: None,
    }))
    .unwrap();
    let m = db.message_by_key(&alice(), &ME, 10).unwrap().unwrap();
    assert_eq!(m.attachments, vec![att]);
    assert_eq!(m.quote, Some(quote));
    assert_eq!(m.expires_in_secs, Some(3600));
    assert_eq!(
        db.conversation(&alice()).unwrap().unwrap().last_preview,
        "📷 Photo"
    );
}

#[test]
fn flags_and_wipe() {
    let mut db = db();
    db.apply(incoming(alice(), ALICE, 1, "x")).unwrap();
    assert!(db.set_muted(&alice(), true).unwrap().unwrap().muted);
    assert!(db.set_archived(&alice(), true).unwrap().unwrap().archived);
    let c = db
        .ensure_conversation(&ConversationId::Group(GroupKey([2; 32])))
        .unwrap();
    assert_eq!(c.title, "Unknown group");
    db.wipe().unwrap();
    assert!(db.all_conversations().unwrap().is_empty());
    assert!(db.contacts().unwrap().is_empty());
}

#[test]
fn apply_all_merges_changes() {
    let mut db = db();
    let ch = db
        .apply_all([
            incoming(alice(), ALICE, 1, "a"),
            incoming(alice(), ALICE, 2, "b"),
        ])
        .unwrap();
    assert_eq!(ch.messages.len(), 2);
    assert_eq!(
        ch.conversations.len(),
        1,
        "conversation updates are deduplicated"
    );
    assert_eq!(ch.conversations[0].unread_count, 2);
    assert_eq!(ch.into_events().len(), 3);
}

#[test]
fn attachment_download_updates_path() {
    let mut db = db();
    let ch = db
        .apply(Mutation::InsertMessage(NewMessage {
            conversation: alice(),
            author: ALICE,
            sent_at: 10,
            received_at: 10,
            outgoing: false,
            body: String::new(),
            attachments: vec![Attachment {
                content_type: "image/png".into(),
                ..Default::default()
            }],
            quote: None,
            status: MessageStatus::Received,
            read: false,
            expires_in_secs: None,
            kind: MessageKind::Normal,
            body_ranges: vec![],
            link_preview: None,
        }))
        .unwrap();
    let id = ch.messages[0].id;
    let ch = db
        .apply(Mutation::AttachmentDownloaded {
            message_id: id,
            index: 0,
            local_path: "/x.png".into(),
        })
        .unwrap();
    assert_eq!(
        ch.messages[0].attachments[0].local_path.as_deref(),
        Some("/x.png")
    );
    // out of range index / unknown message are no-ops
    assert!(
        db.apply(Mutation::AttachmentDownloaded {
            message_id: id,
            index: 3,
            local_path: "y".into()
        })
        .unwrap()
        .is_empty()
    );
    assert!(
        db.apply(Mutation::AttachmentDownloaded {
            message_id: 999,
            index: 0,
            local_path: "y".into()
        })
        .unwrap()
        .is_empty()
    );
}

#[test]
fn migrates_v1_database_preserving_mutes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v1.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute(
            "INSERT INTO conversations (id, title, last_activity, muted) VALUES (?1, 'A', 5, 1)",
            [alice().to_string()],
        )
        .unwrap();
    }
    let db = Database::open(&path, None).unwrap();
    assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
    let c = db.conversation(&alice()).unwrap().unwrap();
    assert!(c.muted);
    assert_eq!(c.muted_until, Some(i64::MAX));
    assert!(c.accepted && !c.blocked && !c.marked_unread);
}

#[test]
fn disappearing_messages_lifecycle() {
    let mut db = db();
    let ch = db
        .apply(Mutation::SetExpireTimer {
            conversation: alice(),
            secs: Some(60),
            author: ME,
            outgoing: true,
            at: 1_000,
        })
        .unwrap();
    assert_eq!(ch.conversations[0].expire_timer, Some(60));
    let notice = &ch.messages[0];
    assert_eq!(notice.kind, MessageKind::TimerChanged { secs: Some(60) });
    assert!(notice.read && notice.expires_at.is_none());
    assert_eq!(ch.conversations[0].unread_count, 0, "notices aren't unread");

    // outgoing: timer starts at send time
    let mut out = NewMessage::text(alice(), ME, 2_000, true, "bye");
    out.expires_in_secs = Some(60);
    let ch = db.apply(Mutation::InsertMessage(out)).unwrap();
    assert_eq!(ch.messages[0].expires_at, Some(62_000));

    // incoming: timer starts when read
    let mut inc = NewMessage::text(alice(), ALICE, 3_000, false, "hi");
    inc.expires_in_secs = Some(60);
    let ch = db.apply(Mutation::InsertMessage(inc)).unwrap();
    assert_eq!(ch.messages[0].expires_at, None);
    let ch = db
        .apply(Mutation::MarkConversationRead {
            conversation: alice(),
        })
        .unwrap();
    let started = ch.messages.iter().find(|m| m.body == "hi").unwrap();
    assert!(started.expires_at.is_some());

    // sweep at 62s removes only the outgoing one
    let ch = db.apply(Mutation::ExpireDue { now: 62_000 }).unwrap();
    assert_eq!(ch.removed_messages.len(), 1);
    assert_eq!(ch.conversations.len(), 1);
    let left: Vec<String> = db
        .messages(&alice(), None, 10)
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect();
    assert_eq!(left, vec!["".to_string(), "hi".to_string()]);
    assert!(
        ch.clone()
            .into_events()
            .iter()
            .any(|e| matches!(e, crate::BackendEvent::MessageRemoved { .. }))
    );

    // a message without a timer turns the chat's timer off (it was disabled elsewhere)
    db.apply(Mutation::InsertMessage(NewMessage::text(
        alice(),
        ALICE,
        4_000,
        false,
        "plain",
    )))
    .unwrap();
    assert_eq!(
        db.conversation(&alice()).unwrap().unwrap().expire_timer,
        None
    );
}

#[test]
fn delete_for_me_and_delete_chat() {
    let mut db = db();
    db.apply(incoming(alice(), ALICE, 1, "a")).unwrap();
    let ch = db.apply(incoming(alice(), ALICE, 2, "b")).unwrap();
    let id = ch.messages[0].id;
    let ch = db.apply(Mutation::DeleteForMe { message_id: id }).unwrap();
    assert_eq!(ch.removed_messages, vec![(alice(), id)]);
    assert_eq!(ch.conversations[0].last_preview, "a");
    assert!(
        db.apply(Mutation::DeleteForMe { message_id: 999 })
            .unwrap()
            .is_empty()
    );

    db.set_pinned(&alice(), true).unwrap();
    let ch = db
        .apply(Mutation::DeleteConversation {
            conversation: alice(),
        })
        .unwrap();
    assert_eq!(ch.removed_conversations, vec![alice()]);
    assert!(db.messages(&alice(), None, 10).unwrap().is_empty());
    assert!(db.conversations().unwrap().is_empty(), "no longer listed");
    assert!(
        db.conversation(&alice()).unwrap().is_some(),
        "contact metadata kept"
    );
}

#[test]
fn timed_mute_and_marked_unread() {
    let mut db = db();
    db.apply(incoming(alice(), ALICE, 1, "a")).unwrap();
    let future = crate::model::now_millis() + 60_000;
    let ch = db
        .apply(Mutation::SetMuted {
            conversation: alice(),
            until: Some(future),
        })
        .unwrap();
    assert!(ch.conversations[0].muted);
    let ch = db
        .apply(Mutation::SetMuted {
            conversation: alice(),
            until: Some(10),
        })
        .unwrap();
    assert!(!ch.conversations[0].muted, "expired mute");
    let ch = db
        .apply(Mutation::SetMuted {
            conversation: alice(),
            until: None,
        })
        .unwrap();
    assert_eq!(ch.conversations[0].muted_until, None);

    db.apply(Mutation::MarkConversationRead {
        conversation: alice(),
    })
    .unwrap();
    let ch = db
        .apply(Mutation::SetMarkedUnread {
            conversation: alice(),
            unread: true,
        })
        .unwrap();
    assert!(ch.conversations[0].has_unread());
    let ch = db
        .apply(Mutation::MarkConversationRead {
            conversation: alice(),
        })
        .unwrap();
    assert!(
        !ch.conversations[0].marked_unread,
        "opening clears the mark"
    );
}

#[test]
fn message_requests_and_blocking() {
    let mut db = db();
    // stranger: request
    let ch = db.apply(incoming(alice(), ALICE, 1, "hey")).unwrap();
    assert!(!ch.conversations[0].accepted);
    // synced contact: accepted
    db.apply(Mutation::UpsertContact(Contact {
        uuid: BOB,
        name: Some("Bob".into()),
        profile_name: None,
        phone: None,
    }))
    .unwrap();
    let ch = db
        .apply(incoming(ConversationId::Direct(BOB), BOB, 1, "yo"))
        .unwrap();
    assert!(ch.conversations[0].accepted);
    // replying accepts
    let ch = db.apply(outgoing(alice(), 5, "hello stranger")).unwrap();
    assert!(ch.conversations[0].accepted);

    let ch = db
        .apply(Mutation::SetBlocked {
            conversation: alice(),
            blocked: true,
        })
        .unwrap();
    assert!(ch.conversations[0].blocked);
    assert!(
        db.apply(incoming(alice(), ALICE, 10, "spam"))
            .unwrap()
            .is_empty(),
        "blocked: dropped"
    );
    let ch = db
        .apply(Mutation::Accept {
            conversation: alice(),
        })
        .unwrap();
    assert!(!ch.conversations[0].blocked && ch.conversations[0].accepted);
}

#[test]
fn edits_keep_history() {
    let mut db = db();
    db.apply(incoming(alice(), ALICE, 100, "v1")).unwrap();
    for (body, at) in [("v2", 150), ("v3", 160)] {
        db.apply(Mutation::Edit {
            conversation: alice(),
            author: ALICE,
            target_sent_at: 100,
            body: body.into(),
            edited_at: at,
        })
        .unwrap();
    }
    let m = db.message_by_key(&alice(), &ALICE, 100).unwrap().unwrap();
    assert_eq!(m.body, "v3");
    let history: Vec<(&str, i64)> = m
        .edit_history
        .iter()
        .map(|r| (r.body.as_str(), r.at))
        .collect();
    assert_eq!(history, vec![("v1", 100), ("v2", 150)]);
}

#[test]
fn body_ranges_roundtrip() {
    use crate::model::{BodyRange, RangeKind};
    let mut db = db();
    db.apply(Mutation::UpsertContact(Contact {
        uuid: BOB,
        name: Some("Bob".into()),
        profile_name: None,
        phone: None,
    }))
    .unwrap();
    let mut m = NewMessage::text(alice(), ALICE, 1, false, "hi \u{FFFC} bold");
    m.body_ranges = vec![
        BodyRange {
            start: 3,
            length: 1,
            kind: RangeKind::Mention(BOB),
        },
        BodyRange {
            start: 5,
            length: 4,
            kind: RangeKind::Bold,
        },
    ];
    let ch = db.apply(Mutation::InsertMessage(m.clone())).unwrap();
    assert_eq!(ch.messages[0].body_ranges, m.body_ranges);
    assert_eq!(ch.conversations[0].last_preview, "hi @Bob bold");
}

#[test]
fn search_within_a_conversation() {
    let mut db = db();
    db.apply(incoming(alice(), ALICE, 1, "coffee at ten"))
        .unwrap();
    db.apply(incoming(
        ConversationId::Direct(BOB),
        BOB,
        2,
        "coffee later?",
    ))
    .unwrap();
    let found = db.search_in_conversation(&alice(), "COFFEE", 10).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].body, "coffee at ten");
    assert!(
        db.search_in_conversation(&alice(), " ", 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn timer_notice_preview_names_the_author() {
    let mut db = db();
    db.apply(Mutation::UpsertContact(Contact {
        uuid: ALICE,
        name: Some("Alice".into()),
        profile_name: None,
        phone: None,
    }))
    .unwrap();
    let ch = db
        .apply(Mutation::SetExpireTimer {
            conversation: alice(),
            secs: Some(86_400),
            author: ALICE,
            outgoing: false,
            at: 5,
        })
        .unwrap();
    assert_eq!(
        ch.conversations[0].last_preview,
        "Alice set the disappearing message timer to 1 day."
    );
}

#[test]
fn stories_insert_view_expire() {
    use crate::protocol::NewStory;
    let mut db = db();
    let now = 10 * crate::model::STORY_LIFETIME_MS;
    let story = |at| NewStory {
        author: ALICE,
        sent_at: at,
        text: Some("hello".into()),
        background: Some(0xFF2C6BED),
        foreground: None,
        attachment: None,
    };
    assert!(
        db.apply(Mutation::InsertStory(story(now - 1000)))
            .unwrap()
            .stories_changed
    );
    assert!(
        !db.apply(Mutation::InsertStory(story(now - 1000)))
            .unwrap()
            .stories_changed,
        "dedup"
    );
    db.apply(Mutation::InsertStory(story(
        now - crate::model::STORY_LIFETIME_MS - 5,
    )))
    .unwrap();
    let all = db.stories().unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].background, Some(0xFF2C6BED));
    let ch = db
        .apply(Mutation::MarkStoryViewed { id: all[0].id })
        .unwrap();
    assert!(
        ch.clone()
            .into_events()
            .contains(&crate::BackendEvent::StoriesChanged)
    );
    let ch = db.apply(Mutation::ExpireDue { now }).unwrap();
    assert!(ch.stories_changed);
    let left = db.stories().unwrap();
    assert_eq!(left.len(), 1);
    assert!(left[0].viewed);
}
