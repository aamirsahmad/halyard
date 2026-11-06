//! Messages exchanged between the UI and the backend thread, and the
//! storage-level mutations derived from Signal protocol content.

use std::path::PathBuf;

use uuid::Uuid;

use crate::ids::{ConversationId, GroupKey};
use crate::model::{
    AccountInfo, Attachment, Contact, Conversation, Message, MessageStatus, Millis, Quote,
};

/// Commands sent from the UI to the backend.
#[derive(Clone, Debug, PartialEq)]
pub enum BackendCommand {
    /// Start linking this client as a secondary device.
    Link { device_name: String },
    SendText {
        conversation: ConversationId,
        body: String,
        quote: Option<Quote>,
        attachments: Vec<PathBuf>,
    },
    /// Retry a failed outgoing message.
    Resend { conversation: ConversationId, sent_at: Millis },
    React {
        conversation: ConversationId,
        target_author: Uuid,
        target_sent_at: Millis,
        emoji: String,
        remove: bool,
    },
    DeleteForEveryone { conversation: ConversationId, sent_at: Millis },
    /// The user has seen all messages in this conversation.
    MarkRead { conversation: ConversationId },
    Typing { conversation: ConversationId, started: bool },
    /// Ask the primary device for a fresh contact list.
    SyncContacts,
    /// Unlink this device and wipe local data.
    Unlink,
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountState {
    /// Nothing stored yet; the UI should offer to link.
    Unlinked,
    /// Linking in progress; `url` is the provisioning URL encoded in the QR code.
    Linking { url: Option<String> },
    Registered(AccountInfo),
    LinkFailed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    /// Connected, draining the server queue.
    Syncing,
    Online,
    /// Lost connection, will retry after a delay.
    Offline { reason: String, retry_in_secs: u64 },
}

/// Events sent from the backend to the UI.
#[derive(Clone, Debug, PartialEq)]
pub enum BackendEvent {
    Account(AccountState),
    Connection(ConnectionState),
    ConversationUpdated(Conversation),
    MessageUpserted(Message),
    Typing { conversation: ConversationId, who: Uuid, started: bool },
    ContactsChanged,
    /// A user-facing, non-fatal error.
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptKind {
    Delivered,
    Read,
    Viewed,
}

impl ReceiptKind {
    pub fn status(self) -> MessageStatus {
        match self {
            ReceiptKind::Delivered => MessageStatus::Delivered,
            ReceiptKind::Read | ReceiptKind::Viewed => MessageStatus::Read,
        }
    }
}

/// A message about to be stored.
#[derive(Clone, Debug, PartialEq)]
pub struct NewMessage {
    pub conversation: ConversationId,
    pub author: Uuid,
    pub sent_at: Millis,
    pub received_at: Millis,
    pub outgoing: bool,
    pub body: String,
    pub attachments: Vec<Attachment>,
    pub quote: Option<Quote>,
    pub status: MessageStatus,
    pub read: bool,
    pub expires_in_secs: Option<u32>,
}

/// Storage mutations; the backend interprets protocol content into these and
/// the [`crate::db::Database`] applies them.
#[derive(Clone, Debug, PartialEq)]
pub enum Mutation {
    UpsertContact(Contact),
    UpsertGroup { key: GroupKey, title: String, members: Vec<Uuid> },
    InsertMessage(NewMessage),
    Edit {
        conversation: ConversationId,
        author: Uuid,
        target_sent_at: Millis,
        body: String,
        edited_at: Millis,
    },
    DeleteForEveryone { conversation: ConversationId, author: Uuid, target_sent_at: Millis },
    React {
        conversation: ConversationId,
        target_author: Uuid,
        target_sent_at: Millis,
        reactor: Uuid,
        emoji: String,
        remove: bool,
        at: Millis,
    },
    /// Delivery/read receipts from `from` for our outgoing messages.
    Receipt { from: Uuid, kind: ReceiptKind, sent_ats: Vec<Millis> },
    /// Local outcome of sending a message.
    SendStatus { conversation: ConversationId, sent_at: Millis, status: MessageStatus },
    /// Another of our devices read a message.
    ReadSync { author: Uuid, sent_at: Millis },
    MarkConversationRead { conversation: ConversationId },
    /// An attachment of a stored message finished downloading.
    AttachmentDownloaded { message_id: i64, index: usize, local_path: String },
}

impl Mutation {
    /// The conversation affected, when known up-front.
    pub fn conversation(&self) -> Option<ConversationId> {
        match self {
            Mutation::InsertMessage(m) => Some(m.conversation),
            Mutation::Edit { conversation, .. }
            | Mutation::DeleteForEveryone { conversation, .. }
            | Mutation::React { conversation, .. }
            | Mutation::SendStatus { conversation, .. }
            | Mutation::MarkConversationRead { conversation } => Some(*conversation),
            Mutation::UpsertGroup { key, .. } => Some(ConversationId::Group(*key)),
            Mutation::UpsertContact(c) => Some(ConversationId::Direct(c.uuid)),
            Mutation::Receipt { .. }
            | Mutation::ReadSync { .. }
            | Mutation::AttachmentDownloaded { .. } => None,
        }
    }
}

/// Result of applying a mutation.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Changes {
    pub conversations: Vec<Conversation>,
    pub messages: Vec<Message>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.conversations.is_empty() && self.messages.is_empty()
    }

    pub fn extend(&mut self, other: Changes) {
        for c in other.conversations {
            self.conversations.retain(|x| x.id != c.id);
            self.conversations.push(c);
        }
        for m in other.messages {
            self.messages.retain(|x| x.id != m.id);
            self.messages.push(m);
        }
    }

    pub fn into_events(self) -> Vec<BackendEvent> {
        let mut ev: Vec<BackendEvent> =
            self.messages.into_iter().map(BackendEvent::MessageUpserted).collect();
        ev.extend(self.conversations.into_iter().map(BackendEvent::ConversationUpdated));
        ev
    }
}
