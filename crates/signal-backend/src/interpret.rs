//! Translates decrypted Signal protocol [`Content`] into storage
//! [`Mutation`]s and transient UI signals. Pure: no I/O, fully unit-tested.

use presage::libsignal_service::content::{Content, ContentBody};
use presage::libsignal_service::prelude::ProtobufMessage;
use presage::libsignal_service::proto::{
    AttachmentPointer, BodyRange, DataMessage, EditMessage, ReceiptMessage, SyncMessage,
    TypingMessage, body_range, data_message, receipt_message, sync_message, typing_message,
};
use presage::libsignal_service::protocol::ServiceId;
use presage::store::Thread;
use signal_core::{
    Attachment, ConversationId, GroupKey, MessageKind, MessageStatus, Millis, Mutation, NewMessage,
    Quote, RangeKind, ReceiptKind,
};
use uuid::Uuid;

/// Inputs needed to interpret content that aren't part of it.
pub struct Ctx<'a> {
    /// Our own ACI.
    pub me: Uuid,
    pub received_at: Millis,
    /// Resolves a group *identifier* (as found in typing messages) to its master key.
    pub group_by_id: &'a dyn Fn(&[u8]) -> Option<GroupKey>,
}

// Short-lived values produced per envelope; boxing would only add allocations.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    Mutation(Mutation),
    Typing {
        conversation: ConversationId,
        who: Uuid,
        started: bool,
    },
    /// A message referenced this group; make sure its metadata is known.
    GroupSeen(GroupKey),
}

/// Parses an ACI given either as a string or as 16 raw bytes.
pub fn parse_aci(text: Option<&str>, binary: Option<&[u8]>) -> Option<Uuid> {
    if let Some(b) = binary
        && let Some(sid) = ServiceId::parse_from_service_id_binary(b)
    {
        return Some(sid.raw_uuid());
    }
    text.and_then(ServiceId::parse_from_service_id_string)
        .map(|s| s.raw_uuid())
}

pub fn conversation_of(thread: &Thread) -> ConversationId {
    match thread {
        Thread::Contact(sid) => ConversationId::Direct(sid.raw_uuid()),
        Thread::Group(key) => ConversationId::Group(GroupKey(*key)),
    }
}

pub fn attachment_from_pointer(p: &AttachmentPointer) -> Attachment {
    Attachment {
        content_type: p
            .content_type
            .clone()
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "application/octet-stream".into()),
        file_name: p.file_name.clone().filter(|f| !f.is_empty()),
        size: p.size.unwrap_or_default() as u64,
        local_path: None,
        width: p.width.filter(|w| *w > 0),
        height: p.height.filter(|h| *h > 0),
        pointer: Some(p.encode_to_vec()),
    }
}

pub fn pointer_from_attachment(a: &Attachment) -> Option<AttachmentPointer> {
    a.pointer
        .as_deref()
        .and_then(|b| AttachmentPointer::decode(b).ok())
}

fn quote_from_proto(q: &data_message::Quote) -> Option<Quote> {
    Some(Quote {
        author: parse_aci(q.author_aci.as_deref(), q.author_aci_binary.as_deref())?,
        sent_at: q.id? as Millis,
        text: q.text.clone().unwrap_or_default(),
    })
}

/// Converts a data message into mutations.
fn data_message(
    dm: &DataMessage,
    conversation: ConversationId,
    author: Uuid,
    outgoing: bool,
    fallback_ts: Millis,
    ctx: &Ctx,
    out: &mut Vec<Output>,
) {
    if let ConversationId::Group(key) = conversation {
        out.push(Output::GroupSeen(key));
    }
    let sent_at = dm.timestamp.map(|t| t as Millis).unwrap_or(fallback_ts);

    if let Some(r) = &dm.reaction {
        let target = parse_aci(
            r.target_author_aci.as_deref(),
            r.target_author_aci_binary.as_deref(),
        );
        if let (Some(target_author), Some(ts), Some(emoji)) =
            (target, r.target_sent_timestamp, &r.emoji)
        {
            out.push(Output::Mutation(Mutation::React {
                conversation,
                target_author,
                target_sent_at: ts as Millis,
                reactor: author,
                emoji: emoji.clone(),
                remove: r.remove.unwrap_or(false),
                at: sent_at,
            }));
        }
        return;
    }

    if let Some(data_message::Delete {
        target_sent_timestamp: Some(ts),
    }) = &dm.delete
    {
        out.push(Output::Mutation(Mutation::DeleteForEveryone {
            conversation,
            author,
            target_sent_at: *ts as Millis,
        }));
        return;
    }

    if dm.flags.unwrap_or(0) & data_message::Flags::ExpirationTimerUpdate as u32 != 0 {
        out.push(Output::Mutation(Mutation::SetExpireTimer {
            conversation,
            secs: dm.expire_timer.filter(|t| *t > 0),
            author,
            outgoing,
            at: sent_at,
        }));
        return;
    }

    let mut body = dm.body.clone().unwrap_or_default();
    let mut attachments: Vec<Attachment> =
        dm.attachments.iter().map(attachment_from_pointer).collect();
    // stickers are shown as images
    if let Some(sticker) = &dm.sticker
        && let Some(data) = &sticker.data
    {
        let mut a = attachment_from_pointer(data);
        if a.content_type == "application/octet-stream" {
            a.content_type = "image/webp".into();
        }
        a.file_name = Some("sticker.webp".into());
        attachments.push(a);
    }
    // shared contact cards
    for c in &dm.contact {
        let name = c
            .name
            .as_ref()
            .and_then(|n| {
                let parts: Vec<&str> = [n.given_name.as_deref(), n.family_name.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect();
                (!parts.is_empty()).then(|| parts.join(" "))
            })
            .or_else(|| c.organization.clone())
            .unwrap_or_else(|| "Contact".into());
        let phone = c.number.iter().find_map(|p| p.value.clone());
        let card = match phone {
            Some(p) => format!("👤 {name} · {p}"),
            None => format!("👤 {name}"),
        };
        body = if body.is_empty() {
            card
        } else {
            format!("{body}\n{card}")
        };
    }
    let unsupported = dm.poll_create.is_some() || dm.payment.is_some() || dm.gift_badge.is_some();
    if body.trim().is_empty() && attachments.is_empty() {
        if unsupported {
            body = "⚠️ This message can't be displayed on this device yet.".into();
        } else {
            // profile key updates, group updates, …
            return;
        }
    }
    let link_preview = dm.preview.iter().find_map(|p| {
        let url = p.url.clone().filter(|u| !u.is_empty())?;
        Some(signal_core::LinkPreview {
            url,
            title: p.title.clone().filter(|t| !t.is_empty()),
            description: p.description.clone().filter(|t| !t.is_empty()),
        })
    });
    out.push(Output::Mutation(Mutation::InsertMessage(NewMessage {
        conversation,
        author,
        sent_at,
        received_at: ctx.received_at,
        outgoing,
        body,
        attachments,
        quote: dm.quote.as_ref().and_then(quote_from_proto),
        status: if outgoing {
            MessageStatus::Sent
        } else {
            MessageStatus::Received
        },
        read: outgoing,
        expires_in_secs: dm.expire_timer.filter(|t| *t > 0),
        kind: MessageKind::Normal,
        body_ranges: dm
            .body_ranges
            .iter()
            .filter_map(body_range_from_proto)
            .collect(),
        link_preview,
    })));
}

/// Converts a protocol body range (style or mention) into the domain model.
pub fn body_range_from_proto(r: &BodyRange) -> Option<signal_core::BodyRange> {
    use body_range::{AssociatedValue, Style};
    let kind = match r.associated_value.as_ref()? {
        AssociatedValue::Style(s) => match Style::try_from(*s).ok()? {
            Style::Bold => RangeKind::Bold,
            Style::Italic => RangeKind::Italic,
            Style::Spoiler => RangeKind::Spoiler,
            Style::Strikethrough => RangeKind::Strikethrough,
            Style::Monospace => RangeKind::Monospace,
            Style::None => return None,
        },
        AssociatedValue::MentionAci(aci) => RangeKind::Mention(parse_aci(Some(aci), None)?),
        AssociatedValue::MentionAciBinary(b) => RangeKind::Mention(parse_aci(None, Some(b))?),
    };
    Some(signal_core::BodyRange {
        start: r.start?,
        length: r.length?,
        kind,
    })
}

fn edit_message(
    edit: &EditMessage,
    conversation: ConversationId,
    author: Uuid,
    out: &mut Vec<Output>,
) {
    let (Some(target), Some(dm)) = (edit.target_sent_timestamp, &edit.data_message) else {
        return;
    };
    out.push(Output::Mutation(Mutation::Edit {
        conversation,
        author,
        target_sent_at: target as Millis,
        body: dm.body.clone().unwrap_or_default(),
        edited_at: dm
            .timestamp
            .map(|t| t as Millis)
            .unwrap_or(target as Millis),
    }));
}

/// Interprets a decrypted envelope.
pub fn interpret(content: &Content, ctx: &Ctx) -> Vec<Output> {
    let mut out = Vec::new();
    let sender = content.metadata.sender.raw_uuid();
    let fallback_ts = content.metadata.client_timestamp.timestamp_millis();
    let thread = match Thread::try_from(content) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(%e, "dropping content without a valid thread");
            return out;
        }
    };
    let conversation = conversation_of(&thread);

    match &content.body {
        ContentBody::DataMessage(dm) => data_message(
            dm,
            conversation,
            sender,
            sender == ctx.me,
            fallback_ts,
            ctx,
            &mut out,
        ),
        ContentBody::EditMessage(edit) => edit_message(edit, conversation, sender, &mut out),
        ContentBody::SynchronizeMessage(SyncMessage {
            content: Some(sync),
            read,
            ..
        }) => {
            if let sync_message::Content::Sent(sent) = sync {
                if let Some(dm) = &sent.message {
                    let ts = sent.timestamp.map(|t| t as Millis).unwrap_or(fallback_ts);
                    data_message(dm, conversation, ctx.me, true, ts, ctx, &mut out);
                }
                if let Some(edit) = &sent.edit_message {
                    edit_message(edit, conversation, ctx.me, &mut out);
                }
            }
            read_sync(read, &mut out);
        }
        ContentBody::SynchronizeMessage(SyncMessage {
            content: None,
            read,
            ..
        }) => read_sync(read, &mut out),
        ContentBody::ReceiptMessage(ReceiptMessage { r#type, timestamp }) => {
            let kind = match r#type.and_then(|t| receipt_message::Type::try_from(t).ok()) {
                Some(receipt_message::Type::Delivery) | None => ReceiptKind::Delivered,
                Some(receipt_message::Type::Read) => ReceiptKind::Read,
                Some(receipt_message::Type::Viewed) => ReceiptKind::Viewed,
            };
            if !timestamp.is_empty() {
                out.push(Output::Mutation(Mutation::Receipt {
                    from: sender,
                    kind,
                    sent_ats: timestamp.iter().map(|t| *t as Millis).collect(),
                }));
            }
        }
        ContentBody::TypingMessage(TypingMessage {
            action, group_id, ..
        }) if sender != ctx.me => {
            let conversation = match group_id.as_deref() {
                Some(id) if !id.is_empty() => match (ctx.group_by_id)(id) {
                    Some(key) => ConversationId::Group(key),
                    None => return out,
                },
                _ => ConversationId::Direct(sender),
            };
            let started = action.and_then(|a| typing_message::Action::try_from(a).ok())
                == Some(typing_message::Action::Started);
            out.push(Output::Typing {
                conversation,
                who: sender,
                started,
            });
        }
        // calls are out of scope for now; stories, null messages, etc. are ignored
        _ => {}
    }
    out
}

fn read_sync(read: &[sync_message::Read], out: &mut Vec<Output>) {
    for r in read {
        if let (Some(author), Some(ts)) = (
            parse_aci(r.sender_aci.as_deref(), r.sender_aci_binary.as_deref()),
            r.timestamp,
        ) {
            out.push(Output::Mutation(Mutation::ReadSync {
                author,
                sent_at: ts as Millis,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presage::libsignal_service::content::Metadata;
    use presage::libsignal_service::proto::GroupContextV2;
    use presage::libsignal_service::protocol::Aci;

    const ME: Uuid = Uuid::from_u128(0x1111);
    const ALICE: Uuid = Uuid::from_u128(0xA11CE);
    const BOB: Uuid = Uuid::from_u128(0xB0B);
    const GROUP: [u8; 32] = [9; 32];

    fn sid(u: Uuid) -> ServiceId {
        Aci::from(u).into()
    }

    fn content(sender: Uuid, body: impl Into<ContentBody>) -> Content {
        let now = chrono::DateTime::from_timestamp_millis(1_700_000_000_000).unwrap();
        Content {
            metadata: Metadata {
                sender: sid(sender),
                sender_device: 1.try_into().unwrap(),
                destination: sid(ME),
                server_guid: None,
                client_timestamp: now,
                server_timestamp: now,
                needs_receipt: false,
                unidentified_sender: false,
                was_plaintext: false,
                pni_verified: None,
            },
            body: body.into(),
        }
    }

    fn no_groups(_: &[u8]) -> Option<GroupKey> {
        None
    }

    fn run(c: &Content) -> Vec<Output> {
        interpret(
            c,
            &Ctx {
                me: ME,
                received_at: 42,
                group_by_id: &no_groups,
            },
        )
    }

    fn only_mutation(out: Vec<Output>) -> Mutation {
        let muts: Vec<Mutation> = out
            .into_iter()
            .filter_map(|o| match o {
                Output::Mutation(m) => Some(m),
                _ => None,
            })
            .collect();
        assert_eq!(muts.len(), 1, "{muts:?}");
        muts.into_iter().next().unwrap()
    }

    fn text(ts: u64, body: &str) -> DataMessage {
        DataMessage {
            body: Some(body.into()),
            timestamp: Some(ts),
            ..Default::default()
        }
    }

    #[test]
    fn incoming_direct_text() {
        let m = only_mutation(run(&content(ALICE, text(100, "hello"))));
        let Mutation::InsertMessage(m) = m else {
            panic!()
        };
        assert_eq!(m.conversation, ConversationId::Direct(ALICE));
        assert_eq!(m.author, ALICE);
        assert_eq!(m.sent_at, 100);
        assert_eq!(m.received_at, 42);
        assert!(!m.outgoing && !m.read);
        assert_eq!(m.status, MessageStatus::Received);
    }

    #[test]
    fn incoming_group_text_with_quote_and_timer() {
        let mut dm = text(100, "re");
        dm.group_v2 = Some(GroupContextV2 {
            master_key: Some(GROUP.to_vec()),
            ..Default::default()
        });
        dm.expire_timer = Some(3600);
        dm.quote = Some(data_message::Quote {
            id: Some(50),
            author_aci: Some(BOB.to_string()),
            text: Some("orig".into()),
            ..Default::default()
        });
        let out = run(&content(ALICE, dm));
        assert!(out.contains(&Output::GroupSeen(GroupKey(GROUP))));
        let Mutation::InsertMessage(m) = only_mutation(out) else {
            panic!()
        };
        assert_eq!(m.conversation, ConversationId::Group(GroupKey(GROUP)));
        assert_eq!(
            m.quote,
            Some(Quote {
                author: BOB,
                sent_at: 50,
                text: "orig".into()
            })
        );
        assert_eq!(m.expires_in_secs, Some(3600));
    }

    #[test]
    fn attachments_keep_pointer() {
        let mut dm = text(100, "");
        dm.attachments.push(AttachmentPointer {
            content_type: Some("image/png".into()),
            file_name: Some("cat.png".into()),
            size: Some(2048),
            width: Some(10),
            height: Some(0),
            key: Some(vec![1; 64]),
            ..Default::default()
        });
        let Mutation::InsertMessage(m) = only_mutation(run(&content(ALICE, dm))) else {
            panic!()
        };
        let a = &m.attachments[0];
        assert_eq!(a.content_type, "image/png");
        assert_eq!(a.size, 2048);
        assert_eq!((a.width, a.height), (Some(10), None));
        let p = pointer_from_attachment(a).unwrap();
        assert_eq!(p.key, Some(vec![1; 64]));
        assert_eq!(p.file_name.as_deref(), Some("cat.png"));
    }

    #[test]
    fn empty_and_control_messages_are_ignored() {
        let dm = DataMessage {
            profile_key: Some(vec![1; 32]),
            timestamp: Some(1),
            ..Default::default()
        };
        assert!(run(&content(ALICE, dm)).is_empty());
        assert!(run(&content(ALICE, text(1, "   "))).is_empty());
    }

    #[test]
    fn reaction_and_removal() {
        let dm = DataMessage {
            timestamp: Some(200),
            reaction: Some(data_message::Reaction {
                emoji: Some("👍".into()),
                remove: Some(true),
                target_author_aci_binary: Some(ME.as_bytes().to_vec()),
                target_sent_timestamp: Some(100),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            only_mutation(run(&content(ALICE, dm))),
            Mutation::React {
                conversation: ConversationId::Direct(ALICE),
                target_author: ME,
                target_sent_at: 100,
                reactor: ALICE,
                emoji: "👍".into(),
                remove: true,
                at: 200,
            }
        );
    }

    #[test]
    fn delete_for_everyone() {
        let dm = DataMessage {
            timestamp: Some(200),
            delete: Some(data_message::Delete {
                target_sent_timestamp: Some(100),
            }),
            ..Default::default()
        };
        assert_eq!(
            only_mutation(run(&content(ALICE, dm))),
            Mutation::DeleteForEveryone {
                conversation: ConversationId::Direct(ALICE),
                author: ALICE,
                target_sent_at: 100
            }
        );
    }

    #[test]
    fn edit_message() {
        let edit = EditMessage {
            target_sent_timestamp: Some(100),
            data_message: Some(text(150, "new")),
        };
        assert_eq!(
            only_mutation(run(&content(ALICE, edit))),
            Mutation::Edit {
                conversation: ConversationId::Direct(ALICE),
                author: ALICE,
                target_sent_at: 100,
                body: "new".into(),
                edited_at: 150
            }
        );
    }

    #[test]
    fn sync_sent_from_other_device_is_outgoing() {
        let sync = SyncMessage {
            content: Some(sync_message::Content::Sent(sync_message::Sent {
                destination_service_id: Some(BOB.to_string()),
                timestamp: Some(300),
                message: Some(text(300, "from phone")),
                ..Default::default()
            })),
            ..Default::default()
        };
        let Mutation::InsertMessage(m) = only_mutation(run(&content(ME, sync))) else {
            panic!()
        };
        assert_eq!(m.conversation, ConversationId::Direct(BOB));
        assert_eq!(m.author, ME);
        assert!(m.outgoing && m.read);
        assert_eq!(m.status, MessageStatus::Sent);
    }

    #[test]
    fn read_sync() {
        let sync = SyncMessage {
            read: vec![
                sync_message::Read {
                    sender_aci: Some(ALICE.to_string()),
                    timestamp: Some(10),
                    ..Default::default()
                },
                sync_message::Read {
                    sender_aci: None,
                    timestamp: Some(11),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            only_mutation(run(&content(ME, sync))),
            Mutation::ReadSync {
                author: ALICE,
                sent_at: 10
            }
        );
    }

    #[test]
    fn receipts() {
        let r = ReceiptMessage {
            r#type: Some(receipt_message::Type::Read as i32),
            timestamp: vec![1, 2],
        };
        assert_eq!(
            only_mutation(run(&content(ALICE, r))),
            Mutation::Receipt {
                from: ALICE,
                kind: ReceiptKind::Read,
                sent_ats: vec![1, 2]
            }
        );
        let r = ReceiptMessage {
            r#type: None,
            timestamp: vec![1],
        };
        assert!(matches!(
            only_mutation(run(&content(ALICE, r))),
            Mutation::Receipt {
                kind: ReceiptKind::Delivered,
                ..
            }
        ));
        assert!(
            run(&content(
                ALICE,
                ReceiptMessage {
                    r#type: None,
                    timestamp: vec![]
                }
            ))
            .is_empty()
        );
    }

    #[test]
    fn typing_direct_and_group() {
        let t = TypingMessage {
            action: Some(typing_message::Action::Started as i32),
            ..Default::default()
        };
        assert_eq!(
            run(&content(ALICE, t)),
            vec![Output::Typing {
                conversation: ConversationId::Direct(ALICE),
                who: ALICE,
                started: true
            }]
        );
        let t = TypingMessage {
            action: Some(typing_message::Action::Stopped as i32),
            group_id: Some(vec![7; 32]),
            ..Default::default()
        };
        let resolve = |id: &[u8]| (id == [7; 32]).then_some(GroupKey(GROUP));
        let out = interpret(
            &content(ALICE, t.clone()),
            &Ctx {
                me: ME,
                received_at: 0,
                group_by_id: &resolve,
            },
        );
        assert_eq!(
            out,
            vec![Output::Typing {
                conversation: ConversationId::Group(GroupKey(GROUP)),
                who: ALICE,
                started: false
            }]
        );
        // unknown group: dropped
        assert!(run(&content(ALICE, t)).is_empty());
        // our own typing (from another device) is ignored
        let t = TypingMessage {
            action: Some(0),
            ..Default::default()
        };
        assert!(run(&content(ME, t)).is_empty());
    }

    #[test]
    fn stickers_contacts_previews_and_unsupported() {
        let dm = DataMessage {
            timestamp: Some(1),
            sticker: Some(data_message::Sticker {
                data: Some(AttachmentPointer {
                    key: Some(vec![1; 64]),
                    ..Default::default()
                }),
                emoji: Some("😀".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let Mutation::InsertMessage(m) = only_mutation(run(&content(ALICE, dm))) else {
            panic!()
        };
        assert_eq!(m.attachments[0].content_type, "image/webp");

        let dm = DataMessage {
            timestamp: Some(2),
            contact: vec![data_message::Contact {
                name: Some(data_message::contact::Name {
                    given_name: Some("Dana".into()),
                    family_name: Some("Lee".into()),
                    ..Default::default()
                }),
                number: vec![data_message::contact::Phone {
                    value: Some("+1 555 0199".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let Mutation::InsertMessage(m) = only_mutation(run(&content(ALICE, dm))) else {
            panic!()
        };
        assert_eq!(m.body, "👤 Dana Lee · +1 555 0199");

        let mut dm = text(3, "look https://signal.org");
        dm.preview = vec![presage::libsignal_service::proto::Preview {
            url: Some("https://signal.org".into()),
            title: Some("Signal".into()),
            description: Some("Speak freely".into()),
            ..Default::default()
        }];
        let Mutation::InsertMessage(m) = only_mutation(run(&content(ALICE, dm))) else {
            panic!()
        };
        let p = m.link_preview.unwrap();
        assert_eq!(
            (p.url.as_str(), p.title.as_deref()),
            ("https://signal.org", Some("Signal"))
        );

        let dm = DataMessage {
            timestamp: Some(4),
            payment: Some(Default::default()),
            ..Default::default()
        };
        let Mutation::InsertMessage(m) = only_mutation(run(&content(ALICE, dm))) else {
            panic!()
        };
        assert!(m.body.contains("can't be displayed"));
    }

    #[test]
    fn timer_update_becomes_notice() {
        let dm = DataMessage {
            timestamp: Some(500),
            flags: Some(data_message::Flags::ExpirationTimerUpdate as u32),
            expire_timer: Some(3600),
            ..Default::default()
        };
        assert_eq!(
            only_mutation(run(&content(ALICE, dm))),
            Mutation::SetExpireTimer {
                conversation: ConversationId::Direct(ALICE),
                secs: Some(3600),
                author: ALICE,
                outgoing: false,
                at: 500
            }
        );
        let dm = DataMessage {
            timestamp: Some(600),
            flags: Some(2),
            expire_timer: Some(0),
            ..Default::default()
        };
        assert!(matches!(
            only_mutation(run(&content(ALICE, dm))),
            Mutation::SetExpireTimer { secs: None, .. }
        ));
    }

    #[test]
    fn body_ranges_are_kept() {
        let mut dm = text(100, "hi \u{FFFC} there");
        dm.body_ranges = vec![
            BodyRange {
                start: Some(3),
                length: Some(1),
                associated_value: Some(body_range::AssociatedValue::MentionAci(BOB.to_string())),
            },
            BodyRange {
                start: Some(5),
                length: Some(5),
                associated_value: Some(body_range::AssociatedValue::Style(
                    body_range::Style::Bold as i32,
                )),
            },
            // malformed ranges are dropped
            BodyRange {
                start: None,
                length: Some(1),
                associated_value: Some(body_range::AssociatedValue::Style(1)),
            },
            BodyRange {
                start: Some(0),
                length: Some(1),
                associated_value: Some(body_range::AssociatedValue::Style(0)),
            },
        ];
        let Mutation::InsertMessage(m) = only_mutation(run(&content(ALICE, dm))) else {
            panic!()
        };
        assert_eq!(
            m.body_ranges,
            vec![
                signal_core::BodyRange {
                    start: 3,
                    length: 1,
                    kind: RangeKind::Mention(BOB)
                },
                signal_core::BodyRange {
                    start: 5,
                    length: 5,
                    kind: RangeKind::Bold
                },
            ]
        );
    }

    #[test]
    fn parse_aci_variants() {
        assert_eq!(parse_aci(Some(&ALICE.to_string()), None), Some(ALICE));
        assert_eq!(parse_aci(None, Some(ALICE.as_bytes())), Some(ALICE));
        assert_eq!(parse_aci(Some("garbage"), Some(&[1, 2])), None);
        assert_eq!(parse_aci(None, None), None);
    }
}
