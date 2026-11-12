//! Domain model shared by the backend and the UI.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ids::ConversationId;

/// Milliseconds since the UNIX epoch (Signal's native timestamp unit).
pub type Millis = i64;

pub fn now_millis() -> Millis {
    chrono::Utc::now().timestamp_millis()
}

/// Delivery state of an outgoing message. Ordered: a status can only move
/// forward (`Sending < Sent < Delivered < Read`) except to `Failed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MessageStatus {
    Sending = 0,
    Sent = 1,
    Delivered = 2,
    Read = 3,
    Failed = 4,
    /// Incoming messages carry no delivery status.
    Received = 5,
}

impl MessageStatus {
    pub fn from_i64(v: i64) -> Self {
        match v {
            0 => Self::Sending,
            1 => Self::Sent,
            2 => Self::Delivered,
            3 => Self::Read,
            4 => Self::Failed,
            _ => Self::Received,
        }
    }

    /// Returns the status after applying `next`, never regressing a delivered
    /// message back to an earlier state.
    pub fn advance(self, next: MessageStatus) -> MessageStatus {
        use MessageStatus::*;
        match (self, next) {
            (Received, _) => Received,
            (_, Failed) => {
                if self >= Sent && self != Failed {
                    self
                } else {
                    Failed
                }
            }
            (Failed, Sent | Delivered | Read) => next,
            (cur, next) if next > cur && next != Received => next,
            (cur, _) => cur,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Attachment {
    pub content_type: String,
    pub file_name: Option<String>,
    pub size: u64,
    /// Local path once downloaded (or the source path for outgoing files).
    pub local_path: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Opaque, serialized pointer used to (re)download the attachment.
    #[serde(default)]
    pub pointer: Option<Vec<u8>>,
    /// View-once media: can be opened a single time.
    #[serde(default)]
    pub view_once: bool,
    /// A view-once attachment that has been opened (and deleted locally).
    #[serde(default)]
    pub viewed: bool,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        self.content_type.starts_with("image/")
    }

    pub fn display_name(&self) -> String {
        self.file_name.clone().unwrap_or_else(|| {
            let ext = self.content_type.rsplit('/').next().unwrap_or("bin");
            format!("attachment.{ext}")
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    pub author: Uuid,
    pub sent_at: Millis,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub reactor: Uuid,
    pub emoji: String,
    pub at: Millis,
}

/// Formatting or mention over a UTF-16 range of a message body (Signal's
/// `BodyRange`). Mentions cover a U+FFFC placeholder in the body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyRange {
    pub start: u32,
    pub length: u32,
    pub kind: RangeKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeKind {
    Bold,
    Italic,
    Strikethrough,
    Monospace,
    Spoiler,
    Mention(Uuid),
}

/// File-name prefix of recorded voice notes; such attachments are sent with
/// the voice-note flag so other clients show them as voice messages.
pub const VOICE_NOTE_PREFIX: &str = "voice-message-";

/// Quote text used for replies to stories.
pub const STORY_REPLY_QUOTE: &str = "Story";

/// Placeholder character Signal puts in the body where a mention goes.
pub const MENTION_PLACEHOLDER: char = '\u{FFFC}';

/// What a timeline entry is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    #[default]
    Normal,
    /// "X set the disappearing message timer to …" notice.
    TimerChanged { secs: Option<u32> },
    /// Group membership notice; the message author is who made the change.
    GroupMember {
        change: MemberChange,
        member: Uuid,
        /// The member is this account ("You left", "removed you").
        member_is_me: bool,
    },
}

/// A change to a group's membership.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberChange {
    Added,
    Left,
    Removed,
}

/// Text of a membership notice. Names are `None` for "you".
pub fn member_notice(actor: Option<&str>, member: Option<&str>, change: MemberChange) -> String {
    match change {
        MemberChange::Left => match member {
            Some(m) => format!("{m} left the group."),
            None => "You left the group.".into(),
        },
        MemberChange::Added => match (actor, member) {
            (None, Some(m)) => format!("You added {m}."),
            (Some(a), None) => format!("{a} added you."),
            (Some(a), Some(m)) => format!("{a} added {m}."),
            (None, None) => "You joined the group.".into(),
        },
        MemberChange::Removed => match (actor, member) {
            (None, Some(m)) => format!("You removed {m}."),
            (Some(a), None) => format!("{a} removed you."),
            (Some(a), Some(m)) => format!("{a} removed {m}."),
            (None, None) => "You left the group.".into(),
        },
    }
}

/// Stories disappear 24 hours after they were posted.
pub const STORY_LIFETIME_MS: Millis = 24 * 60 * 60 * 1000;

/// A story posted by a contact (text on a background, or a photo/video).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Story {
    pub id: i64,
    pub author: Uuid,
    pub sent_at: Millis,
    pub text: Option<String>,
    /// Background colour (0xAARRGGBB) of a text story; a gradient's start colour.
    pub background: Option<u32>,
    /// Text colour (0xAARRGGBB) of a text story.
    pub foreground: Option<u32>,
    pub attachment: Option<Attachment>,
    pub viewed: bool,
}

/// A link preview card attached to a message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPreview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
}

impl LinkPreview {
    /// Host shown under the title, like Signal ("example.com").
    pub fn domain(&self) -> String {
        let rest = self.url.split("://").nth(1).unwrap_or(&self.url);
        let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        host.trim_start_matches("www.").to_string()
    }
}

/// A previous version of an edited message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    pub body: String,
    pub at: Millis,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Local row id.
    pub id: i64,
    pub conversation: ConversationId,
    pub author: Uuid,
    /// Signal's sender timestamp; together with `author` it identifies a message.
    pub sent_at: Millis,
    pub received_at: Millis,
    pub outgoing: bool,
    pub body: String,
    pub attachments: Vec<Attachment>,
    pub quote: Option<Quote>,
    pub reactions: Vec<Reaction>,
    pub status: MessageStatus,
    pub read: bool,
    pub edited_at: Option<Millis>,
    pub deleted: bool,
    pub expires_in_secs: Option<u32>,
    /// When the message disappears (timer started).
    pub expires_at: Option<Millis>,
    pub kind: MessageKind,
    pub body_ranges: Vec<BodyRange>,
    /// Earlier versions, oldest first.
    pub edit_history: Vec<Revision>,
    pub link_preview: Option<LinkPreview>,
}

impl Message {
    pub fn is_notice(&self) -> bool {
        self.kind != MessageKind::Normal
    }

    /// Text shown in the conversation list preview.
    pub fn preview(&self) -> String {
        if self.deleted {
            return "This message was deleted.".into();
        }
        if let MessageKind::TimerChanged { secs } = &self.kind {
            return timer_notice(None, *secs);
        }
        if let MessageKind::GroupMember { change, .. } = &self.kind {
            return member_notice(None, None, *change);
        }
        let body = self.body.replace(MENTION_PLACEHOLDER, "@");
        let body = body.trim();
        if !body.is_empty() {
            return body.lines().next().unwrap_or_default().to_string();
        }
        match self.attachments.first() {
            Some(a) if a.view_once => "📷 View-once media".into(),
            Some(a) if a.is_image() => "📷 Photo".into(),
            Some(a) if a.content_type.starts_with("video/") => "🎥 Video".into(),
            Some(a) if a.content_type.starts_with("audio/") => "🎤 Voice message".into(),
            Some(a) => format!("📎 {}", a.display_name()),
            None => String::new(),
        }
    }

    /// Signal only allows editing your own messages for 24 hours.
    pub const EDIT_WINDOW_MS: Millis = 24 * 60 * 60 * 1000;

    /// Whether the user may edit this message at `now`.
    pub fn editable(&self, now: Millis) -> bool {
        self.outgoing
            && !self.is_notice()
            && !self.deleted
            && !self.body.trim().is_empty()
            && self.status != MessageStatus::Failed
            && now - self.sent_at < Self::EDIT_WINDOW_MS
    }

    /// Reactions grouped by emoji, in order of first appearance, with counts.
    pub fn reaction_summary(&self) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = Vec::new();
        for r in &self.reactions {
            match out.iter_mut().find(|(e, _)| *e == r.emoji) {
                Some((_, n)) => *n += 1,
                None => out.push((r.emoji.clone(), 1)),
            }
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub uuid: Uuid,
    pub name: Option<String>,
    pub profile_name: Option<String>,
    pub phone: Option<String>,
}

impl Contact {
    pub fn display_name(&self) -> String {
        [&self.name, &self.profile_name, &self.phone]
            .into_iter()
            .flatten()
            .map(|s| s.trim())
            .find(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "Unknown contact".to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: ConversationId,
    pub title: String,
    pub last_activity: Millis,
    pub last_preview: String,
    pub last_preview_outgoing: bool,
    pub unread_count: u32,
    pub pinned: bool,
    pub archived: bool,
    /// Muted right now (derived from `muted_until` when loaded).
    pub muted: bool,
    /// Muted until this time; `Some(Millis::MAX)` means forever.
    pub muted_until: Option<Millis>,
    pub draft: Option<String>,
    pub members: Vec<Uuid>,
    /// Manually marked as unread.
    pub marked_unread: bool,
    /// Disappearing-message timer.
    pub expire_timer: Option<u32>,
    pub blocked: bool,
    /// False while this is a message request from an unknown sender.
    pub accepted: bool,
    /// Local (encrypted) file of the contact's or group's photo.
    pub avatar: Option<String>,
    /// Contact's "about" text or group description.
    pub about: Option<String>,
    /// This account is no longer a member of the group.
    pub left: bool,
}

impl Conversation {
    pub fn new(id: ConversationId, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            last_activity: 0,
            last_preview: String::new(),
            last_preview_outgoing: false,
            unread_count: 0,
            pinned: false,
            archived: false,
            muted: false,
            muted_until: None,
            draft: None,
            members: Vec::new(),
            marked_unread: false,
            expire_timer: None,
            blocked: false,
            accepted: true,
            avatar: None,
            about: None,
            left: false,
        }
    }

    pub fn is_group(&self) -> bool {
        self.id.is_group()
    }

    /// Whether the conversation shows as unread in the list.
    pub fn has_unread(&self) -> bool {
        self.unread_count > 0 || self.marked_unread
    }
}

/// The account this client is linked to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountInfo {
    pub aci: Uuid,
    pub phone_number: Option<String>,
    pub device_id: u32,
    pub device_name: Option<String>,
}

/// Human label for a disappearing-message duration, like Signal's.
pub fn timer_label(secs: u32) -> String {
    const UNITS: [(u32, &str, &str); 5] = [
        (7 * 86_400, "week", "weeks"),
        (86_400, "day", "days"),
        (3_600, "hour", "hours"),
        (60, "minute", "minutes"),
        (1, "second", "seconds"),
    ];
    for (unit, one, many) in UNITS {
        if secs >= unit && secs.is_multiple_of(unit) {
            let n = secs / unit;
            return format!("{n} {}", if n == 1 { one } else { many });
        }
    }
    format!("{secs} seconds")
}

/// Short countdown ("5m", "2h", "3d", "1w") for a remaining duration.
pub fn timer_short(remaining_ms: Millis) -> String {
    let s = (remaining_ms.max(0) + 999) / 1000;
    match s {
        0..=59 => format!("{s}s"),
        60..=3_599 => format!("{}m", s / 60),
        3_600..=86_399 => format!("{}h", s / 3_600),
        86_400..=604_799 => format!("{}d", s / 86_400),
        _ => format!("{}w", s / 604_800),
    }
}

/// Text of a timer-change notice; `who` is `None` for "You".
pub fn timer_notice(who: Option<&str>, secs: Option<u32>) -> String {
    let who = who.unwrap_or("You");
    match secs.filter(|s| *s > 0) {
        Some(s) => format!(
            "{who} set the disappearing message timer to {}.",
            timer_label(s)
        ),
        None => format!("{who} disabled disappearing messages."),
    }
}

/// The disappearing-message presets Signal offers.
pub const TIMER_PRESETS: [u32; 6] = [30, 5 * 60, 3_600, 8 * 3_600, 86_400, 7 * 86_400];

/// Mute durations offered in the UI, in milliseconds (`None` = always).
pub const MUTE_PRESETS: [(&str, Option<Millis>); 5] = [
    ("For 1 hour", Some(3_600_000)),
    ("For 8 hours", Some(8 * 3_600_000)),
    ("For 1 day", Some(86_400_000)),
    ("For 1 week", Some(7 * 86_400_000)),
    ("Always", None),
];

#[cfg(test)]
mod tests {
    use super::*;
    use MessageStatus::*;

    #[test]
    fn status_never_regresses() {
        assert_eq!(Sending.advance(Sent), Sent);
        assert_eq!(Sent.advance(Delivered), Delivered);
        assert_eq!(Delivered.advance(Sent), Delivered);
        assert_eq!(Read.advance(Delivered), Read);
        assert_eq!(Sending.advance(Read), Read);
    }

    #[test]
    fn status_failure_rules() {
        assert_eq!(Sending.advance(Failed), Failed);
        // a message already acknowledged by the server can't fail anymore
        assert_eq!(Delivered.advance(Failed), Delivered);
        // a successful retry recovers from failure
        assert_eq!(Failed.advance(Sent), Sent);
        assert_eq!(Received.advance(Read), Received);
        assert_eq!(Sent.advance(Received), Sent);
    }

    #[test]
    fn status_from_i64_roundtrip() {
        for s in [Sending, Sent, Delivered, Read, Failed, Received] {
            assert_eq!(MessageStatus::from_i64(s as i64), s);
        }
    }

    fn msg(body: &str, attachments: Vec<Attachment>) -> Message {
        Message {
            id: 1,
            conversation: ConversationId::Direct(Uuid::nil()),
            author: Uuid::nil(),
            sent_at: 1,
            received_at: 1,
            outgoing: false,
            body: body.into(),
            attachments,
            quote: None,
            reactions: vec![],
            status: Received,
            read: false,
            edited_at: None,
            deleted: false,
            expires_in_secs: None,
            expires_at: None,
            kind: MessageKind::Normal,
            body_ranges: vec![],
            edit_history: vec![],
            link_preview: None,
        }
    }

    #[test]
    fn link_preview_domain() {
        let p = |u: &str| {
            LinkPreview {
                url: u.into(),
                title: None,
                description: None,
            }
            .domain()
        };
        assert_eq!(p("https://www.signal.org/download?x=1"), "signal.org");
        assert_eq!(p("http://a.b"), "a.b");
    }

    #[test]
    fn timer_labels() {
        assert_eq!(timer_label(30), "30 seconds");
        assert_eq!(timer_label(60), "1 minute");
        assert_eq!(timer_label(8 * 3600), "8 hours");
        assert_eq!(timer_label(86_400), "1 day");
        assert_eq!(timer_label(604_800), "1 week");
        assert_eq!(timer_label(90), "90 seconds");
        assert_eq!(timer_short(4_000), "4s");
        assert_eq!(timer_short(5 * 60_000), "5m");
        assert_eq!(timer_short(3 * 86_400_000), "3d");
        assert_eq!(
            timer_notice(Some("Alice"), Some(3600)),
            "Alice set the disappearing message timer to 1 hour."
        );
        assert_eq!(
            timer_notice(None, None),
            "You disabled disappearing messages."
        );
    }

    #[test]
    fn preview_replaces_mentions_and_notices() {
        let mut m = msg("hi \u{FFFC}!", vec![]);
        assert_eq!(m.preview(), "hi @!");
        m.kind = MessageKind::TimerChanged { secs: Some(60) };
        assert!(m.preview().contains("1 minute"));
        m.outgoing = true;
        assert!(!m.editable(2), "notices can't be edited");
    }

    #[test]
    fn preview_uses_first_line() {
        assert_eq!(msg("  hello\nworld ", vec![]).preview(), "hello");
    }

    #[test]
    fn preview_for_attachments() {
        let img = Attachment {
            content_type: "image/png".into(),
            ..Default::default()
        };
        assert_eq!(msg("", vec![img]).preview(), "📷 Photo");
        let file = Attachment {
            content_type: "application/pdf".into(),
            file_name: Some("doc.pdf".into()),
            ..Default::default()
        };
        assert_eq!(msg("", vec![file]).preview(), "📎 doc.pdf");
        let anon = Attachment {
            content_type: "application/zip".into(),
            ..Default::default()
        };
        assert_eq!(anon.display_name(), "attachment.zip");
    }

    #[test]
    fn preview_for_deleted() {
        let mut m = msg("secret", vec![]);
        m.deleted = true;
        assert_eq!(m.preview(), "This message was deleted.");
    }

    #[test]
    fn edit_window() {
        let mut m = msg("hi", vec![]);
        assert!(!m.editable(2), "incoming messages can't be edited");
        m.outgoing = true;
        m.status = Sent;
        assert!(m.editable(2));
        assert!(!m.editable(1 + Message::EDIT_WINDOW_MS));
        m.body = " ".into();
        assert!(
            !m.editable(2),
            "attachment-only messages have no text to edit"
        );
        m.body = "x".into();
        m.deleted = true;
        assert!(!m.editable(2));
    }

    #[test]
    fn reaction_summary_groups() {
        let mut m = msg("hi", vec![]);
        for (i, e) in ["👍", "❤️", "👍"].iter().enumerate() {
            m.reactions.push(Reaction {
                reactor: Uuid::from_u128(i as u128),
                emoji: e.to_string(),
                at: 0,
            });
        }
        assert_eq!(
            m.reaction_summary(),
            vec![("👍".into(), 2), ("❤️".into(), 1)]
        );
    }

    #[test]
    fn contact_display_name_fallbacks() {
        let mut c = Contact {
            uuid: Uuid::nil(),
            name: None,
            profile_name: None,
            phone: None,
        };
        assert_eq!(c.display_name(), "Unknown contact");
        c.phone = Some("+15550001".into());
        assert_eq!(c.display_name(), "+15550001");
        c.profile_name = Some("  ".into());
        assert_eq!(c.display_name(), "+15550001");
        c.profile_name = Some("Prof".into());
        assert_eq!(c.display_name(), "Prof");
        c.name = Some("Alice".into());
        assert_eq!(c.display_name(), "Alice");
    }
}
