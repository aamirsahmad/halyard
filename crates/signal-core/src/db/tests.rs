use super::*;
use crate::ids::GroupKey;
use crate::model::{Attachment, Quote};
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
    assert!(matches!(Database::open(&path, Some("wrong")), Err(DbError::BadKey)));
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
    let ch = db.apply(incoming(alice(), ALICE, 100, "hello\nsecond line")).unwrap();
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
    assert_eq!(db.conversation(&alice()).unwrap().unwrap().title, "Unknown contact");
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
    db.apply(Mutation::UpsertContact(Contact { uuid: ALICE, name: None, profile_name: None, phone: None }))
        .unwrap();
    assert_eq!(db.contact(&ALICE).unwrap().unwrap().phone.as_deref(), Some("+1555"));
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
        db.apply(incoming(alice(), ALICE, i * 10, &format!("m{i}"))).unwrap();
    }
    let page1 = db.messages(&alice(), None, 10).unwrap();
    assert_eq!(page1.first().unwrap().body, "m16");
    assert_eq!(page1.last().unwrap().body, "m25");
    let first = &page1[0];
    let page2 = db.messages(&alice(), Some((first.sent_at, first.id)), 10).unwrap();
    assert_eq!(page2.first().unwrap().body, "m6");
    assert_eq!(page2.last().unwrap().body, "m15");
    let page3 = db.messages(&alice(), Some((page2[0].sent_at, page2[0].id)), 10).unwrap();
    assert_eq!(page3.len(), 5);
}

#[test]
fn conversations_sorted_pinned_then_recent() {
    let mut db = db();
    let bob = ConversationId::Direct(BOB);
    let group = ConversationId::Group(GroupKey([1; 32]));
    db.apply(Mutation::UpsertGroup { key: GroupKey([1; 32]), title: "Team".into(), members: vec![ALICE, BOB] })
        .unwrap();
    db.apply(incoming(alice(), ALICE, 100, "a")).unwrap();
    db.apply(incoming(bob, BOB, 200, "b")).unwrap();
    db.apply(incoming(group, ALICE, 50, "g")).unwrap();
    let order: Vec<_> = db.conversations().unwrap().into_iter().map(|c| c.id).collect();
    assert_eq!(order, vec![bob, alice(), group]);
    db.set_pinned(&group, true).unwrap();
    let order: Vec<_> = db.conversations().unwrap().into_iter().map(|c| c.id).collect();
    assert_eq!(order, vec![group, bob, alice()]);
    let g = db.conversation(&group).unwrap().unwrap();
    assert_eq!(g.members, vec![ALICE, BOB]);
    assert_eq!(g.title, "Team");
}

#[test]
fn empty_conversations_hidden_from_list_but_in_picker() {
    let mut db = db();
    db.apply(Mutation::UpsertContact(Contact { uuid: BOB, name: Some("Bob".into()), profile_name: None, phone: None }))
        .unwrap();
    assert!(db.conversations().unwrap().is_empty());
    assert_eq!(db.all_conversations().unwrap().len(), 1);
    db.set_draft(&ConversationId::Direct(BOB), Some("draft")).unwrap();
    assert_eq!(db.conversations().unwrap().len(), 1);
    db.set_draft(&ConversationId::Direct(BOB), Some("   ")).unwrap();
    assert!(db.conversations().unwrap().is_empty());
}

#[test]
fn receipts_advance_outgoing_status_only_for_matching_peer() {
    let mut db = db();
    db.apply(outgoing(alice(), 500, "yo")).unwrap();
    db.apply(Mutation::SendStatus { conversation: alice(), sent_at: 500, status: MessageStatus::Sent }).unwrap();
    // receipt from someone else is ignored for a direct chat
    let ch = db.apply(Mutation::Receipt { from: BOB, kind: ReceiptKind::Read, sent_ats: vec![500] }).unwrap();
    assert!(ch.messages.is_empty());
    let ch = db.apply(Mutation::Receipt { from: ALICE, kind: ReceiptKind::Read, sent_ats: vec![500] }).unwrap();
    assert_eq!(ch.messages[0].status, MessageStatus::Read);
    // late delivery receipt does not regress
    let ch = db.apply(Mutation::Receipt { from: ALICE, kind: ReceiptKind::Delivered, sent_ats: vec![500] }).unwrap();
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
    let ch = db.apply(Mutation::SendStatus { conversation: alice(), sent_at: 500, status: MessageStatus::Failed }).unwrap();
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
    assert_eq!(m.reaction_summary(), vec![("❤️".into(), 1), ("😂".into(), 1)]);
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
        .apply(Mutation::Edit { conversation: alice(), author: ALICE, target_sent_at: 100, body: "fixed".into(), edited_at: 150 })
        .unwrap();
    assert_eq!(ch.messages[0].body, "fixed");
    assert_eq!(ch.messages[0].edited_at, Some(150));
    assert_eq!(ch.conversations[0].last_preview, "fixed");
    // an edit cannot be forged by another author
    let ch = db
        .apply(Mutation::Edit { conversation: alice(), author: BOB, target_sent_at: 100, body: "evil".into(), edited_at: 160 })
        .unwrap();
    assert!(ch.is_empty());
    let ch = db.apply(Mutation::DeleteForEveryone { conversation: alice(), author: ALICE, target_sent_at: 100 }).unwrap();
    assert!(ch.messages[0].deleted);
    assert_eq!(ch.messages[0].body, "");
    assert_eq!(ch.conversations[0].last_preview, "This message was deleted.");
    assert_eq!(ch.conversations[0].unread_count, 0);
    // deleted messages can't be edited back
    let ch = db
        .apply(Mutation::Edit { conversation: alice(), author: ALICE, target_sent_at: 100, body: "x".into(), edited_at: 170 })
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
    let ch = db.apply(Mutation::ReadSync { author: ALICE, sent_at: 200 }).unwrap();
    assert_eq!(ch.conversations[0].unread_count, 1);
    let ch = db.apply(Mutation::MarkConversationRead { conversation: alice() }).unwrap();
    assert_eq!(ch.conversations[0].unread_count, 0);
    // nothing left to mark: no change event
    let ch = db.apply(Mutation::MarkConversationRead { conversation: alice() }).unwrap();
    assert!(ch.is_empty());
}

#[test]
fn search_escapes_like_wildcards() {
    let mut db = db();
    db.apply(incoming(alice(), ALICE, 1, "100% sure")).unwrap();
    db.apply(incoming(alice(), ALICE, 2, "1000 things")).unwrap();
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
    let quote = Quote { author: ALICE, sent_at: 5, text: "orig".into() };
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
    }))
    .unwrap();
    let m = db.message_by_key(&alice(), &ME, 10).unwrap().unwrap();
    assert_eq!(m.attachments, vec![att]);
    assert_eq!(m.quote, Some(quote));
    assert_eq!(m.expires_in_secs, Some(3600));
    assert_eq!(db.conversation(&alice()).unwrap().unwrap().last_preview, "📷 Photo");
}

#[test]
fn flags_and_wipe() {
    let mut db = db();
    db.apply(incoming(alice(), ALICE, 1, "x")).unwrap();
    assert!(db.set_muted(&alice(), true).unwrap().unwrap().muted);
    assert!(db.set_archived(&alice(), true).unwrap().unwrap().archived);
    let c = db.ensure_conversation(&ConversationId::Group(GroupKey([2; 32]))).unwrap();
    assert_eq!(c.title, "Unknown group");
    db.wipe().unwrap();
    assert!(db.all_conversations().unwrap().is_empty());
    assert!(db.contacts().unwrap().is_empty());
}

#[test]
fn apply_all_merges_changes() {
    let mut db = db();
    let ch = db
        .apply_all([incoming(alice(), ALICE, 1, "a"), incoming(alice(), ALICE, 2, "b")])
        .unwrap();
    assert_eq!(ch.messages.len(), 2);
    assert_eq!(ch.conversations.len(), 1, "conversation updates are deduplicated");
    assert_eq!(ch.conversations[0].unread_count, 2);
    assert_eq!(ch.into_events().len(), 3);
}
