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
        /// Mentions (and formatting) over `body`, in UTF-16 offsets.
        ranges: Vec<BodyRange>,
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
    /// Reply privately to a contact's story (text or an emoji reaction).
    ReplyToStory {
        author: Uuid,
        story_sent_at: Millis,
        body: String,
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
    /// Fetch the photo (and about text) of a conversation if not known yet.
    FetchAvatar {
        conversation: ConversationId,
    },
    /// Update your own profile.
    UpdateProfile {
        given_name: String,
        family_name: Option<String>,
        about: Option<String>,
    },
    /// Whether to send read receipts (a privacy preference).
    SetReadReceipts(bool),
    /// Ask the primary device for a fresh contact list.
    SyncContacts,
    /// Post a story to "My Story" (everyone you chat with): text on a
    /// coloured background, or a photo/video.
    PostStory {
        text: Option<String>,
        /// Background colour (0xAARRGGBB) of a text story.
        background: Option<u32>,
        attachment: Option<PathBuf>,
    },
    /// Tell your other devices (the phone records it in the storage service)
    /// how a message request was answered.
    MessageRequestResponse {
        conversation: ConversationId,
        response: RequestResponse,
    },
    /// The blocked list changed locally: sync it to your other devices.
    SyncBlockedList,
    /// Leave a group (removes this account from it on the server).
    LeaveGroup {
        group: GroupKey,
    },
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
    StoriesChanged,
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

#[derive(Clone, Debug, PartialEq)]
pub struct NewStory {
    pub author: Uuid,
    pub sent_at: Millis,
    pub text: Option<String>,
    pub background: Option<u32>,
    pub foreground: Option<u32>,
    pub attachment: Option<Attachment>,
    /// Our own stories are stored as already viewed.
    pub viewed: bool,
}

/// How a message request was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestResponse {
    Accept,
    Delete,
    Block,
    BlockAndDelete,
}

/// A chat's state as recorded in Signal's storage service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteChatState {
    pub conversation: ConversationId,
    /// "Whitelisted": you accepted the chat (or started it).
    pub accepted: bool,
    pub blocked: bool,
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
    /// A story to store (ignored if already known).
    InsertStory(NewStory),
    MarkStoryViewed {
        id: i64,
    },
    /// A view-once attachment was opened: forget the media.
    ViewOnceOpened {
        message_id: i64,
    },
    /// Photo of a conversation (contact or group) was stored at `path`.
    SetAvatar {
        conversation: ConversationId,
        path: Option<String>,
    },
    /// "About" text of a contact or description of a group.
    SetAbout {
        conversation: ConversationId,
        about: Option<String>,
    },
    /// These accounts came from the phone's contact sync: never show their
    /// chats as message requests.
    ContactsSynced {
        uuids: Vec<Uuid>,
    },
    StoryAttachmentDownloaded {
        id: i64,
        local_path: String,
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
    /// The complete blocked list, synced from your phone: everything listed
    /// is blocked, everything else unblocked.
    BlockedList {
        contacts: Vec<Uuid>,
        groups: Vec<GroupKey>,
    },
    /// Message-request and block state from Signal's storage service (what
    /// the phone decided). Authoritative for the chats it lists.
    RemoteChatStates {
        states: Vec<RemoteChatState>,
    },
    /// A member joined, left or was removed; adds a notice to the timeline
    /// and marks the group as left when the member is this account.
    GroupMember {
        conversation: ConversationId,
        actor: Uuid,
        member: Uuid,
        change: crate::model::MemberChange,
        /// `actor` is this account.
        outgoing: bool,
        /// `member` is this account.
        member_is_me: bool,
        at: Millis,
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
            | Mutation::GroupMember { conversation, .. }
            | Mutation::Accept { conversation }
            | Mutation::SetAvatar { conversation, .. }
            | Mutation::SetAbout { conversation, .. } => Some(*conversation),
            Mutation::UpsertGroup { key, .. } => Some(ConversationId::Group(*key)),
            Mutation::UpsertContact(c) => Some(ConversationId::Direct(c.uuid)),
            Mutation::Receipt { .. }
            | Mutation::ReadSync { .. }
            | Mutation::AttachmentDownloaded { .. }
            | Mutation::ExpireDue { .. }
            | Mutation::InsertStory(_)
            | Mutation::RemoteChatStates { .. }
            | Mutation::BlockedList { .. }
            | Mutation::MarkStoryViewed { .. }
            | Mutation::ContactsSynced { .. }
            | Mutation::ViewOnceOpened { .. }
            | Mutation::StoryAttachmentDownloaded { .. }
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
    /// Stories were added, viewed or expired.
    pub stories_changed: bool,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.conversations.is_empty()
            && self.messages.is_empty()
            && self.removed_messages.is_empty()
            && self.removed_conversations.is_empty()
            && !self.stories_changed
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
        self.stories_changed |= other.stories_changed;
        for c in other.removed_conversations {
            self.conversations.retain(|x| x.id != c);
            self.removed_conversations.push(c);
        }
    }

    pub fn into_events(self) -> Vec<BackendEvent> {
        let stories = self.stories_changed;
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
        if stories {
            ev.push(BackendEvent::StoriesChanged);
        }
        ev
    }
}
