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
}

impl Message {
    /// Text shown in the conversation list preview.
    pub fn preview(&self) -> String {
        if self.deleted {
            return "This message was deleted.".into();
        }
        let body = self.body.trim();
        if !body.is_empty() {
            return body.lines().next().unwrap_or_default().to_string();
        }
        match self.attachments.first() {
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
    pub muted: bool,
    pub draft: Option<String>,
    pub members: Vec<Uuid>,
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
            draft: None,
            members: Vec::new(),
        }
    }

    pub fn is_group(&self) -> bool {
        self.id.is_group()
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
        }
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
