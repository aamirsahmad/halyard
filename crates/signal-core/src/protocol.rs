//! Messages exchanged between the UI and the backend thread, and the
//! storage-level mutations derived from Signal protocol content.

use std::path::PathBuf;

use uuid::Uuid;

use crate::ids::{ConversationId, GroupKey};
use crate::model::{
    AccountInfo, Attachment, BodyRange, Contact, Conversation, LinkPreview, Message, MessageKind,
    MessageStatus, Millis, Quote,
};

/// Commands sent from the UI to the backend.
#[derive(Clone, Debug, PartialEq)]
pub enum BackendCommand {
    /// Start linking this client as a secondary device.
    Link {
        device_name: String,
    },
    SendText {
        conversation: ConversationId,
        body: String,
        quote: Option<Quote>,
        attachments: Vec<PathBuf>,
    },
    /// Retry a failed outgoing message.
    Resend {
        conversation: ConversationId,
        sent_at: Millis,
    },
    React {
        conversation: ConversationId,
        target_author: Uuid,
        target_sent_at: Millis,
        emoji: String,
        remove: bool,
    },
    DeleteForEveryone {
        conversation: ConversationId,
        sent_at: Millis,
    },
    /// Replace the text of one of our own messages.
    Edit {
        conversation: ConversationId,
        target_sent_at: Millis,
        body: String,
    },
    /// The user has seen all messages in this conversation.
    MarkRead {
        conversation: ConversationId,
    },
    /// Change the disappearing-message timer (`None` turns it off).
    SetExpireTimer {
        conversation: ConversationId,
        secs: Option<u32>,
    },
    /// Forward a message's text and attachments to other conversations.
    Forward {
        targets: Vec<ConversationId>,
        body: String,
        attachments: Vec<PathBuf>,
    },
    Typing {
        conversation: ConversationId,
        started: bool,
    },
    /// Compute the safety number with a contact (answered by
    /// [`BackendEvent::SafetyNumber`]).
    RequestSafetyNumber {
        contact: Uuid,
    },
    /// Whether to send read receipts (a privacy preference).
    SetReadReceipts(bool),
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
    Linking {
        url: Option<String>,
    },
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
    Offline {
        reason: String,
        retry_in_secs: u64,
    },
}

/// Events sent from the backend to the UI.
#[derive(Clone, Debug, PartialEq)]
pub enum BackendEvent {
    Account(AccountState),
    Connection(ConnectionState),
    ConversationUpdated(Conversation),
    /// A conversation was deleted locally.
    ConversationRemoved(ConversationId),
    MessageUpserted(Message),
    /// A message was removed (deleted for me, or disappeared).
    MessageRemoved {
        conversation: ConversationId,
        id: i64,
    },
    Typing {
        conversation: ConversationId,
        who: Uuid,
        started: bool,
    },
    ContactsChanged,
    /// Safety number (60 digits) with a contact; `None` if no session yet.
    SafetyNumber {
        contact: Uuid,
        number: Option<String>,
    },
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
    pub kind: MessageKind,
    pub body_ranges: Vec<BodyRange>,
    pub link_preview: Option<LinkPreview>,
}

impl NewMessage {
    /// A plain text message; fill in the rest with struct update syntax.
    pub fn text(
        conversation: ConversationId,
        author: Uuid,
        sent_at: Millis,
        outgoing: bool,
        body: impl Into<String>,
    ) -> Self {
        Self {
            conversation,
            author,
            sent_at,
            received_at: sent_at,
            outgoing,
            body: body.into(),
            attachments: Vec::new(),
            quote: None,
            status: if outgoing {
                MessageStatus::Sent
            } else {
                MessageStatus::Received
            },
            read: outgoing,
            expires_in_secs: None,
            kind: MessageKind::Normal,
            body_ranges: Vec::new(),
            link_preview: None,
        }
    }
}

/// Storage mutations; the backend interprets protocol content into these and
/// the [`crate::db::Database`] applies them.
#[derive(Clone, Debug, PartialEq)]
pub enum Mutation {
    UpsertContact(Contact),
    UpsertGroup {
        key: GroupKey,
        title: String,
        members: Vec<Uuid>,
    },
    InsertMessage(NewMessage),
    Edit {
        conversation: ConversationId,
        author: Uuid,
        target_sent_at: Millis,
        body: String,
        edited_at: Millis,
    },
    DeleteForEveryone {
        conversation: ConversationId,
        author: Uuid,
        target_sent_at: Millis,
    },
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
    Receipt {
        from: Uuid,
        kind: ReceiptKind,
        sent_ats: Vec<Millis>,
    },
    /// Local outcome of sending a message.
    SendStatus {
        conversation: ConversationId,
        sent_at: Millis,
        status: MessageStatus,
    },
    /// Another of our devices read a message.
    ReadSync {
        author: Uuid,
        sent_at: Millis,
    },
    MarkConversationRead {
        conversation: ConversationId,
    },
    /// An attachment of a stored message finished downloading.
    AttachmentDownloaded {
        message_id: i64,
        index: usize,
        local_path: String,
    },
    /// Disappearing-message timer changed by `author` at `at`; records a notice.
    SetExpireTimer {
        conversation: ConversationId,
        secs: Option<u32>,
        author: Uuid,
        outgoing: bool,
        at: Millis,
    },
    /// Remove messages whose disappearing timer ran out by `now`.
    ExpireDue {
        now: Millis,
    },
    /// Start the disappearing timer of read incoming messages (Signal starts
    /// it when a message is read).
    StartExpiryTimers {
        conversation: ConversationId,
        now: Millis,
    },
    DeleteForMe {
        message_id: i64,
    },
    DeleteConversation {
        conversation: ConversationId,
    },
    SetMuted {
        conversation: ConversationId,
        until: Option<Millis>,
    },
    SetMarkedUnread {
        conversation: ConversationId,
        unread: bool,
    },
    SetBlocked {
        conversation: ConversationId,
        blocked: bool,
    },
    Accept {
        conversation: ConversationId,
    },
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
            | Mutation::MarkConversationRead { conversation }
            | Mutation::SetExpireTimer { conversation, .. }
            | Mutation::StartExpiryTimers { conversation, .. }
            | Mutation::DeleteConversation { conversation }
            | Mutation::SetMuted { conversation, .. }
            | Mutation::SetMarkedUnread { conversation, .. }
            | Mutation::SetBlocked { conversation, .. }
            | Mutation::Accept { conversation } => Some(*conversation),
            Mutation::UpsertGroup { key, .. } => Some(ConversationId::Group(*key)),
            Mutation::UpsertContact(c) => Some(ConversationId::Direct(c.uuid)),
            Mutation::Receipt { .. }
            | Mutation::ReadSync { .. }
            | Mutation::AttachmentDownloaded { .. }
            | Mutation::ExpireDue { .. }
            | Mutation::DeleteForMe { .. } => None,
        }
    }
}

/// Result of applying a mutation.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Changes {
    pub conversations: Vec<Conversation>,
    pub messages: Vec<Message>,
    pub removed_messages: Vec<(ConversationId, i64)>,
    pub removed_conversations: Vec<ConversationId>,
    /// Local attachment files of removed messages (for the caller to delete).
    pub removed_files: Vec<String>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.conversations.is_empty()
            && self.messages.is_empty()
            && self.removed_messages.is_empty()
            && self.removed_conversations.is_empty()
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
        for r in other.removed_messages {
            self.messages.retain(|x| x.id != r.1);
            self.removed_messages.push(r);
        }
        self.removed_files.extend(other.removed_files);
        for c in other.removed_conversations {
            self.conversations.retain(|x| x.id != c);
            self.removed_conversations.push(c);
        }
    }

    pub fn into_events(self) -> Vec<BackendEvent> {
        let mut ev: Vec<BackendEvent> = self
            .removed_messages
            .into_iter()
            .map(|(conversation, id)| BackendEvent::MessageRemoved { conversation, id })
            .collect();
        ev.extend(self.messages.into_iter().map(BackendEvent::MessageUpserted));
        ev.extend(
            self.removed_conversations
                .into_iter()
                .map(BackendEvent::ConversationRemoved),
        );
        ev.extend(
            self.conversations
                .into_iter()
                .map(BackendEvent::ConversationUpdated),
        );
        ev
    }
}
