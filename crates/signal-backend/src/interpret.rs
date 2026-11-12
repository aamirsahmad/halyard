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
        view_once: false,
        viewed: false,
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

/// A "My Story" post: text on a coloured background, or a photo/video.
pub fn story_message(
    text: Option<String>,
    background: Option<u32>,
    file: Option<AttachmentPointer>,
    profile_key: Vec<u8>,
) -> presage::libsignal_service::proto::StoryMessage {
    use presage::libsignal_service::proto::{
        StoryMessage, TextAttachment, story_message::Attachment as StoryAttachment,
        text_attachment::Background,
    };
    let attachment = match file {
        Some(pointer) => StoryAttachment::FileAttachment(pointer),
        None => StoryAttachment::TextAttachment(TextAttachment {
            text,
            text_style: Some(
                presage::libsignal_service::proto::text_attachment::Style::Regular as i32,
            ),
            text_foreground_color: Some(0xFFFF_FFFF),
            background: Some(Background::Color(background.unwrap_or(STORY_COLORS[0]))),
            ..Default::default()
        }),
    };
    StoryMessage {
        profile_key: Some(profile_key),
        attachment: Some(attachment),
        allows_replies: Some(true),
        ..Default::default()
    }
}

/// Background colours offered for text stories (0xAARRGGBB), like Signal's.
pub const STORY_COLORS: [u32; 6] = [
    0xFF2C_6BED, // ultramarine
    0xFF68_8BD4,
    0xFF8B_6CC8,
    0xFFC9_4B76,
    0xFFE0_7A3E,
    0xFF3E_9D6A,
];

/// Block-list and message-request syncs from your phone.
fn sync_settings(sync: &sync_message::Content, ctx: &Ctx, out: &mut Vec<Output>) {
    match sync {
        sync_message::Content::Blocked(b) => {
            let mut contacts: Vec<Uuid> = b
                .acis
                .iter()
                .filter_map(|a| parse_aci(Some(a), None))
                .chain(
                    b.acis_binary
                        .iter()
                        .filter_map(|a| parse_aci(None, Some(a))),
                )
                .collect();
            contacts.sort();
            contacts.dedup();
            let groups = b
                .group_ids
                .iter()
                .filter_map(|id| (ctx.group_by_id)(id))
                .collect();
            out.push(Output::Mutation(Mutation::BlockedList { contacts, groups }));
        }
        sync_message::Content::MessageRequestResponse(r) => {
            use sync_message::message_request_response::Type;
            let conversation = match (
                parse_aci(r.thread_aci.as_deref(), r.thread_aci_binary.as_deref()),
                r.group_id.as_deref(),
            ) {
                (Some(u), _) => ConversationId::Direct(u),
                (None, Some(id)) => match (ctx.group_by_id)(id) {
                    Some(key) => ConversationId::Group(key),
                    None => return,
                },
                _ => return,
            };
            let block = |blocked| {
                Output::Mutation(Mutation::SetBlocked {
                    conversation,
                    blocked,
                })
            };
            let delete = || Output::Mutation(Mutation::DeleteConversation { conversation });
            match r.r#type.and_then(|t| Type::try_from(t).ok()) {
                Some(Type::Accept) => out.push(Output::Mutation(Mutation::Accept { conversation })),
                Some(Type::Delete) => out.push(delete()),
                Some(Type::Block | Type::BlockAndSpam) => out.push(block(true)),
                Some(Type::BlockAndDelete) => {
                    out.push(block(true));
                    out.push(delete());
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// The sync telling your other devices how a message request was answered.
pub fn request_response_sync(
    conversation: ConversationId,
    response: signal_core::RequestResponse,
) -> SyncMessage {
    use sync_message::message_request_response::Type;
    let r#type = match response {
        signal_core::RequestResponse::Accept => Type::Accept,
        signal_core::RequestResponse::Delete => Type::Delete,
        signal_core::RequestResponse::Block => Type::Block,
        signal_core::RequestResponse::BlockAndDelete => Type::BlockAndDelete,
    };
    let mut r = sync_message::MessageRequestResponse {
        r#type: Some(r#type as i32),
        ..Default::default()
    };
    match conversation {
        ConversationId::Direct(u) => {
            r.thread_aci = Some(u.to_string());
            r.thread_aci_binary = Some(u.as_bytes().to_vec());
        }
        ConversationId::Group(key) => {
            r.group_id = Some(crate::worker::group_identifier(&key).to_vec())
        }
    }
    SyncMessage {
        content: Some(sync_message::Content::MessageRequestResponse(r)),
        ..Default::default()
    }
}

/// The full blocked list, as Signal clients sync it.
pub fn blocked_sync(contacts: &[Uuid], groups: &[GroupKey]) -> SyncMessage {
    SyncMessage {
        content: Some(sync_message::Content::Blocked(sync_message::Blocked {
            numbers: vec![],
            acis: contacts.iter().map(Uuid::to_string).collect(),
            acis_binary: contacts.iter().map(|u| u.as_bytes().to_vec()).collect(),
            group_ids: groups
                .iter()
                .map(|g| crate::worker::group_identifier(g).to_vec())
                .collect(),
        })),
        ..Default::default()
    }
}

/// "My Story" distribution list id.
pub const MY_STORY_ID: &str = "00000000-0000-0000-0000-000000000000";

/// The sent-transcript of a story, so your other devices show it too.
pub fn story_sent_sync(
    story: presage::libsignal_service::proto::StoryMessage,
    recipients: &[Uuid],
    ts: Millis,
) -> SyncMessage {
    SyncMessage {
        content: Some(sync_message::Content::Sent(sync_message::Sent {
            timestamp: Some(ts as u64),
            story_message: Some(story),
            story_message_recipients: recipients
                .iter()
                .map(|u| sync_message::sent::StoryMessageRecipient {
                    destination_service_id: Some(u.to_string()),
                    destination_service_id_binary: Some(u.as_bytes().to_vec()),
                    distribution_list_ids: vec![MY_STORY_ID.into()],
                    is_allowed_to_reply: Some(true),
                })
                .collect(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// Membership notices for a signed, encrypted group change.
pub fn group_change_mutations(key: GroupKey, change: &[u8], me: Uuid, at: Millis) -> Vec<Mutation> {
    use presage::libsignal_service::groups_v2::{GroupChange, GroupOperations};
    use presage::libsignal_service::zkgroup::groups::{GroupMasterKey, GroupSecretParams};
    let params = GroupSecretParams::derive_from_master_key(GroupMasterKey::new(key.0));
    let decoded = presage::libsignal_service::proto::GroupChange::decode(change)
        .map_err(|e| e.to_string())
        .and_then(|c| {
            GroupOperations::new(params)
                .decrypt_group_change(c)
                .map_err(|e| e.to_string())
        });
    let changes = match decoded {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(%e, "undecodable group change");
            return Vec::new();
        }
    };
    let actor: Uuid = changes.editor.into();
    changes
        .changes
        .into_iter()
        .filter_map(|c| {
            let (member, change): (Uuid, _) = match c {
                GroupChange::NewMember(m) => (m.aci.into(), signal_core::MemberChange::Added),
                GroupChange::DeleteMember(aci) => {
                    let member: Uuid = aci.into();
                    let change = if member == actor {
                        signal_core::MemberChange::Left
                    } else {
                        signal_core::MemberChange::Removed
                    };
                    (member, change)
                }
                _ => return None,
            };
            Some(Mutation::GroupMember {
                conversation: ConversationId::Group(key),
                actor,
                member,
                change,
                outgoing: actor == me,
                member_is_me: member == me,
                at,
            })
        })
        .collect()
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

    // signed group changes ride along on group messages: show who joined/left
    if let ConversationId::Group(key) = conversation
        && let Some(change) = dm.group_v2.as_ref().and_then(|g| g.group_change.as_deref())
        && !change.is_empty()
    {
        out.extend(
            group_change_mutations(key, change, ctx.me, sent_at)
                .into_iter()
                .map(Output::Mutation),
        );
    }

    // replies and reactions to a story arrive as data messages with a story context
    if let Some(sc) = &dm.story_context
        && let Some(story_author) =
            parse_aci(sc.author_aci.as_deref(), sc.author_aci_binary.as_deref())
    {
        let body = dm
            .reaction
            .as_ref()
            .and_then(|r| r.emoji.clone())
            .or_else(|| dm.body.clone())
            .unwrap_or_default();
        if !body.trim().is_empty() {
            let mut m = NewMessage::text(conversation, author, sent_at, outgoing, body);
            m.received_at = ctx.received_at;
            m.quote = Some(Quote {
                author: story_author,
                sent_at: sc.sent_timestamp.unwrap_or_default() as Millis,
                text: signal_core::STORY_REPLY_QUOTE.into(),
            });
            out.push(Output::Mutation(Mutation::InsertMessage(m)));
        }
        return;
    }

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
    if dm.is_view_once == Some(true) {
        for a in &mut attachments {
            a.view_once = true;
        }
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

/// Converts a domain body range into the protocol form.
pub fn body_range_to_proto(r: &signal_core::BodyRange) -> BodyRange {
    use body_range::{AssociatedValue, Style};
    let value = match &r.kind {
        RangeKind::Mention(u) => AssociatedValue::MentionAci(u.to_string()),
        RangeKind::Bold => AssociatedValue::Style(Style::Bold as i32),
        RangeKind::Italic => AssociatedValue::Style(Style::Italic as i32),
        RangeKind::Spoiler => AssociatedValue::Style(Style::Spoiler as i32),
        RangeKind::Strikethrough => AssociatedValue::Style(Style::Strikethrough as i32),
        RangeKind::Monospace => AssociatedValue::Style(Style::Monospace as i32),
    };
    BodyRange {
        start: Some(r.start),
        length: Some(r.length),
        associated_value: Some(value),
    }
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
            sync_settings(sync, ctx, &mut out);
            if let sync_message::Content::Sent(sent) = sync {
                if let Some(story) = &sent.story_message {
                    let ts = sent.timestamp.map(|t| t as Millis).unwrap_or(fallback_ts);
                    if let Some(mut new) = story_of(story, ctx.me, ts) {
                        new.viewed = true;
                        out.push(Output::Mutation(Mutation::InsertStory(new)));
                    }
                }
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
        ContentBody::StoryMessage(story) if sender != ctx.me => {
            if let Some(new) = story_of(story, sender, fallback_ts) {
                out.push(Output::Mutation(Mutation::InsertStory(new)));
            }
        }
        // calls are out of scope for now; null messages, etc. are ignored
        _ => {}
    }
    out
}

/// A story as stored locally (unviewed); `None` without content.
fn story_of(
    story: &presage::libsignal_service::proto::StoryMessage,
    author: Uuid,
    sent_at: Millis,
) -> Option<signal_core::NewStory> {
    use presage::libsignal_service::proto::story_message::Attachment as StoryAttachment;
    use presage::libsignal_service::proto::text_attachment::Background;
    let mut new = signal_core::NewStory {
        viewed: false,
        author,
        sent_at,
        text: None,
        background: None,
        foreground: None,
        attachment: None,
    };
    match story.attachment.as_ref()? {
        StoryAttachment::TextAttachment(t) => {
            new.text = t.text.clone();
            new.foreground = t.text_foreground_color;
            new.background = match &t.background {
                Some(Background::Color(c)) => Some(*c),
                Some(Background::Gradient(g)) => g.colors.first().copied().or(g.start_color),
                None => None,
            };
        }
        StoryAttachment::FileAttachment(p) => new.attachment = Some(attachment_from_pointer(p)),
    }
    Some(new)
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
    fn story_replies_and_reactions() {
        let mut dm = text(10, "love this!");
        dm.story_context = Some(data_message::StoryContext {
            author_aci: Some(ME.to_string()),
            sent_timestamp: Some(5),
            ..Default::default()
        });
        let Mutation::InsertMessage(m) = only_mutation(run(&content(ALICE, dm.clone()))) else {
            panic!()
        };
        assert_eq!(m.body, "love this!");
        let q = m.quote.unwrap();
        assert_eq!((q.author, q.sent_at, q.text.as_str()), (ME, 5, "Story"));
        dm.body = None;
        dm.reaction = Some(data_message::Reaction {
            emoji: Some("🔥".into()),
            ..Default::default()
        });
        let Mutation::InsertMessage(m) = only_mutation(run(&content(ALICE, dm))) else {
            panic!()
        };
        assert_eq!(m.body, "🔥");
    }

    #[test]
    fn stories_from_contacts() {
        use presage::libsignal_service::proto::{
            StoryMessage, TextAttachment, story_message, text_attachment,
        };
        let story = StoryMessage {
            attachment: Some(story_message::Attachment::TextAttachment(TextAttachment {
                text: Some("Hiking today!".into()),
                text_foreground_color: Some(0xFFFFFFFF),
                background: Some(text_attachment::Background::Color(0xFF2C6BED)),
                ..Default::default()
            })),
            ..Default::default()
        };
        let Mutation::InsertStory(s) = only_mutation(run(&content(
            ALICE,
            ContentBody::StoryMessage(story.clone()),
        ))) else {
            panic!()
        };
        assert_eq!(s.text.as_deref(), Some("Hiking today!"));
        assert_eq!(s.background, Some(0xFF2C6BED));
        assert_eq!(s.author, ALICE);
        // our own stories (synced from the phone) aren't shown as incoming
        assert!(run(&content(ME, ContentBody::StoryMessage(story))).is_empty());
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
    fn body_ranges_roundtrip_through_proto() {
        for kind in [
            RangeKind::Mention(BOB),
            RangeKind::Bold,
            RangeKind::Spoiler,
            RangeKind::Monospace,
        ] {
            let r = signal_core::BodyRange {
                start: 2,
                length: 3,
                kind,
            };
            assert_eq!(body_range_from_proto(&body_range_to_proto(&r)), Some(r));
        }
    }

    #[test]
    fn view_once_flag() {
        let mut dm = text(1, "");
        dm.is_view_once = Some(true);
        dm.attachments.push(AttachmentPointer {
            content_type: Some("image/jpeg".into()),
            ..Default::default()
        });
        let Mutation::InsertMessage(m) = only_mutation(run(&content(ALICE, dm))) else {
            panic!()
        };
        assert!(m.attachments[0].view_once && !m.attachments[0].viewed);
    }

    #[test]
    fn parse_aci_variants() {
        assert_eq!(parse_aci(Some(&ALICE.to_string()), None), Some(ALICE));
        assert_eq!(parse_aci(None, Some(ALICE.as_bytes())), Some(ALICE));
        assert_eq!(parse_aci(Some("garbage"), Some(&[1, 2])), None);
        assert_eq!(parse_aci(None, None), None);
    }

    /// An encrypted group change made by `editor` deleting `deleted`.
    fn delete_member_change(editor: Uuid, deleted: Uuid) -> Vec<u8> {
        use presage::libsignal_service::proto::{GroupChange, group_change};
        use presage::libsignal_service::zkgroup::{
            self,
            groups::{GroupMasterKey, GroupSecretParams},
        };
        let params = GroupSecretParams::derive_from_master_key(GroupMasterKey::new(GROUP));
        let enc = |u: Uuid| zkgroup::serialize(&params.encrypt_service_id(sid(u)));
        GroupChange {
            actions: group_change::Actions {
                source_user_id: enc(editor),
                group_id: params.get_group_identifier().to_vec(),
                version: 4,
                delete_members: vec![group_change::actions::DeleteMemberAction {
                    deleted_user_id: enc(deleted),
                }],
                ..Default::default()
            }
            .encode_to_vec(),
            ..Default::default()
        }
        .encode_to_vec()
    }

    fn group_update(change: Vec<u8>) -> DataMessage {
        DataMessage {
            timestamp: Some(500),
            group_v2: Some(GroupContextV2 {
                master_key: Some(GROUP.to_vec()),
                revision: Some(4),
                group_change: Some(change),
            }),
            ..Default::default()
        }
    }

    fn member_mutations(out: Vec<Output>) -> Vec<Mutation> {
        out.into_iter()
            .filter_map(|o| match o {
                Output::Mutation(m @ Mutation::GroupMember { .. }) => Some(m),
                Output::Mutation(other) => panic!("unexpected {other:?}"),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn group_change_member_left() {
        let out = run(&content(BOB, group_update(delete_member_change(BOB, BOB))));
        assert_eq!(
            member_mutations(out),
            vec![Mutation::GroupMember {
                conversation: ConversationId::Group(GroupKey(GROUP)),
                actor: BOB,
                member: BOB,
                change: signal_core::MemberChange::Left,
                outgoing: false,
                member_is_me: false,
                at: 500,
            }]
        );
    }

    #[test]
    fn group_change_removed_me() {
        let out = run(&content(
            ALICE,
            group_update(delete_member_change(ALICE, ME)),
        ));
        let mutations = member_mutations(out);
        let [
            Mutation::GroupMember {
                change,
                member_is_me,
                outgoing,
                ..
            },
        ] = mutations.as_slice()
        else {
            panic!()
        };
        assert_eq!(*change, signal_core::MemberChange::Removed);
        assert!(*member_is_me && !*outgoing);
    }

    #[test]
    fn garbage_group_change_is_ignored() {
        let out = run(&content(BOB, group_update(vec![1, 2, 3])));
        assert!(member_mutations(out).is_empty());
    }

    #[test]
    fn text_story_round_trips_through_the_receiver() {
        let story = story_message(
            Some("hi all".into()),
            Some(STORY_COLORS[2]),
            None,
            vec![7; 32],
        );
        assert_eq!(story.allows_replies, Some(true));
        let out = run(&content(ALICE, ContentBody::StoryMessage(story)));
        let Some(Output::Mutation(Mutation::InsertStory(s))) = out.into_iter().next() else {
            panic!("expected a story")
        };
        assert_eq!(s.text.as_deref(), Some("hi all"));
        assert_eq!(s.background, Some(STORY_COLORS[2]));
        assert!(!s.viewed);
    }

    fn run_sync(sync: SyncMessage) -> Vec<Output> {
        let key = GroupKey(GROUP);
        let id = crate::worker::group_identifier(&key);
        let resolve = move |g: &[u8]| (g == id).then_some(key);
        interpret(
            &content(ME, ContentBody::SynchronizeMessage(sync)),
            &Ctx {
                me: ME,
                received_at: 42,
                group_by_id: &resolve,
            },
        )
    }

    fn mutations(out: Vec<Output>) -> Vec<Mutation> {
        out.into_iter()
            .filter_map(|o| match o {
                Output::Mutation(m) => Some(m),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn blocked_list_round_trips() {
        let sync = blocked_sync(&[ALICE, BOB], &[GroupKey(GROUP)]);
        assert_eq!(
            mutations(run_sync(sync)),
            vec![Mutation::BlockedList {
                contacts: vec![BOB, ALICE], // sorted, deduplicated
                groups: vec![GroupKey(GROUP)],
            }]
        );
    }

    #[test]
    fn message_request_responses_round_trip() {
        use signal_core::RequestResponse;
        let bob = ConversationId::Direct(BOB);
        let group = ConversationId::Group(GroupKey(GROUP));
        assert_eq!(
            mutations(run_sync(request_response_sync(
                bob,
                RequestResponse::Accept
            ))),
            vec![Mutation::Accept { conversation: bob }]
        );
        assert_eq!(
            mutations(run_sync(request_response_sync(
                group,
                RequestResponse::BlockAndDelete
            ))),
            vec![
                Mutation::SetBlocked {
                    conversation: group,
                    blocked: true
                },
                Mutation::DeleteConversation {
                    conversation: group
                },
            ]
        );
        assert_eq!(
            mutations(run_sync(request_response_sync(
                bob,
                RequestResponse::Delete
            ))),
            vec![Mutation::DeleteConversation { conversation: bob }]
        );
    }

    #[test]
    fn story_posted_on_another_device_shows_in_my_stories() {
        let story = story_message(Some("from my phone".into()), None, None, vec![1; 32]);
        let sync = story_sent_sync(story, &[ALICE], 777);
        let SyncMessage {
            content: Some(sync_message::Content::Sent(sent)),
            ..
        } = &sync
        else {
            panic!()
        };
        assert_eq!(
            sent.story_message_recipients[0].distribution_list_ids,
            vec![MY_STORY_ID.to_string()]
        );
        let Ok([Mutation::InsertStory(s)]) = <[Mutation; 1]>::try_from(mutations(run_sync(sync)))
        else {
            panic!("expected one story")
        };
        assert_eq!(s.author, ME);
        assert_eq!(s.sent_at, 777);
        assert_eq!(s.text.as_deref(), Some("from my phone"));
        assert!(s.viewed);
    }
}
