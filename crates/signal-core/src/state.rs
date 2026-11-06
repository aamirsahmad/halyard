//! In-memory view state of the chat UI and the reducer applying backend events.

use std::collections::{BTreeSet, HashMap};

use uuid::Uuid;

use crate::ids::ConversationId;
use crate::model::{Conversation, Message};
use crate::protocol::{AccountState, BackendEvent, ConnectionState};

/// Side effects the UI should perform after applying an event.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Effects {
    /// The open conversation received new incoming messages while visible.
    pub mark_read: Option<ConversationId>,
    /// Show a desktop notification.
    pub notify: Option<Notification>,
    /// The open conversation's timeline changed (scroll/re-render).
    pub timeline_changed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub conversation: ConversationId,
    pub title: String,
    pub body: String,
}

#[derive(Debug)]
pub struct ChatState {
    pub account: AccountState,
    pub connection: ConnectionState,
    conversations: Vec<Conversation>,
    selected: Option<ConversationId>,
    filter: String,
    pub show_archived: bool,
    messages: Vec<Message>,
    pub has_more_history: bool,
    typing: HashMap<ConversationId, BTreeSet<Uuid>>,
    /// Whether the app window is focused (affects read marking/notifications).
    pub window_active: bool,
    pub last_error: Option<String>,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            account: AccountState::Unlinked,
            connection: ConnectionState::Disconnected,
            conversations: Vec::new(),
            selected: None,
            filter: String::new(),
            show_archived: false,
            messages: Vec::new(),
            has_more_history: false,
            typing: HashMap::new(),
            window_active: true,
            last_error: None,
        }
    }
}

fn sort_key(c: &Conversation) -> (std::cmp::Reverse<bool>, std::cmp::Reverse<i64>) {
    (
        std::cmp::Reverse(c.pinned),
        std::cmp::Reverse(c.last_activity),
    )
}

impl ChatState {
    pub fn new() -> Self {
        Self::default()
    }

    // ---------------------------------------------------------- conversations

    pub fn set_conversations(&mut self, mut list: Vec<Conversation>) {
        list.sort_by_key(sort_key);
        self.conversations = list;
    }

    pub fn conversations(&self) -> &[Conversation] {
        &self.conversations
    }

    pub fn conversation(&self, id: &ConversationId) -> Option<&Conversation> {
        self.conversations.iter().find(|c| c.id == *id)
    }

    pub fn upsert_conversation(&mut self, conv: Conversation) {
        self.conversations.retain(|c| c.id != conv.id);
        let listed = conv.last_activity > 0 || conv.draft.is_some() || conv.pinned;
        if listed || self.selected == Some(conv.id) {
            let pos = self
                .conversations
                .partition_point(|c| sort_key(c) <= sort_key(&conv));
            self.conversations.insert(pos, conv);
        }
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn set_filter(&mut self, filter: impl Into<String>) {
        self.filter = filter.into();
    }

    /// Conversations shown in the left pane, honouring the search filter and
    /// the archived view.
    pub fn visible_conversations(&self) -> Vec<&Conversation> {
        let needle = self.filter.trim().to_lowercase();
        self.conversations
            .iter()
            .filter(|c| {
                if needle.is_empty() {
                    c.archived == self.show_archived
                } else {
                    // searching covers archived chats too
                    c.title.to_lowercase().contains(&needle)
                }
            })
            .collect()
    }

    pub fn archived_count(&self) -> usize {
        self.conversations.iter().filter(|c| c.archived).count()
    }

    /// Total unread messages in non-muted conversations (dock badge).
    pub fn total_unread(&self) -> u32 {
        self.conversations
            .iter()
            .filter(|c| !c.muted)
            .map(|c| c.unread_count)
            .sum()
    }

    // --------------------------------------------------------------- timeline

    pub fn selected(&self) -> Option<&ConversationId> {
        self.selected.as_ref()
    }

    pub fn selected_conversation(&self) -> Option<&Conversation> {
        self.selected.as_ref().and_then(|id| self.conversation(id))
    }

    /// Opens a conversation with its newest page of messages.
    pub fn open(&mut self, conv: Conversation, messages: Vec<Message>, has_more: bool) {
        self.selected = Some(conv.id);
        if self.conversation(&conv.id).is_none() {
            // composing to a contact without history: show it at the top
            self.conversations.insert(0, conv);
        }
        self.messages = messages;
        self.has_more_history = has_more;
    }

    pub fn close(&mut self) {
        self.selected = None;
        self.messages.clear();
        self.has_more_history = false;
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Cursor for loading the page before the oldest loaded message.
    pub fn history_cursor(&self) -> Option<(i64, i64)> {
        self.messages.first().map(|m| (m.sent_at, m.id))
    }

    pub fn prepend_history(&mut self, mut older: Vec<Message>, has_more: bool) {
        older.retain(|m| !self.messages.iter().any(|x| x.id == m.id));
        older.append(&mut self.messages);
        self.messages = older;
        self.has_more_history = has_more;
    }

    /// Inserts or replaces a message of the open conversation.
    /// Returns whether the open timeline changed.
    pub fn upsert_message(&mut self, msg: Message) -> bool {
        if self.selected != Some(msg.conversation) {
            return false;
        }
        if let Some(existing) = self.messages.iter_mut().find(|m| m.id == msg.id) {
            *existing = msg;
            return true;
        }
        let key = |m: &Message| (m.sent_at, m.id);
        let pos = self.messages.partition_point(|m| key(m) <= key(&msg));
        self.messages.insert(pos, msg);
        true
    }

    pub fn message(&self, id: i64) -> Option<&Message> {
        self.messages.iter().find(|m| m.id == id)
    }

    // ----------------------------------------------------------------- typing

    pub fn typing_in(&self, id: &ConversationId) -> Vec<Uuid> {
        self.typing
            .get(id)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }

    // ----------------------------------------------------------------- events

    /// Applies a backend event. `author_name` resolves display names for
    /// notifications in groups.
    pub fn apply(&mut self, event: BackendEvent, author_name: impl Fn(&Uuid) -> String) -> Effects {
        let mut fx = Effects::default();
        match event {
            BackendEvent::Account(state) => {
                if !matches!(state, AccountState::Registered(_)) {
                    self.conversations.clear();
                    self.close();
                }
                self.account = state;
            }
            BackendEvent::Connection(c) => self.connection = c,
            BackendEvent::ConversationUpdated(conv) => self.upsert_conversation(conv),
            BackendEvent::MessageUpserted(msg) => {
                let fresh_incoming = !msg.outgoing && !msg.read && !msg.deleted;
                let conv_id = msg.conversation;
                // a new message from someone stops their typing indicator
                if let Some(set) = self.typing.get_mut(&conv_id) {
                    set.remove(&msg.author);
                }
                let open = self.selected == Some(conv_id);
                fx.timeline_changed = self.upsert_message(msg.clone());
                if fresh_incoming {
                    if open && self.window_active {
                        fx.mark_read = Some(conv_id);
                    } else if !self.conversation(&conv_id).is_some_and(|c| c.muted) {
                        let conv_title = self
                            .conversation(&conv_id)
                            .map(|c| c.title.clone())
                            .unwrap_or_else(|| author_name(&msg.author));
                        let (title, body) = if conv_id.is_group() {
                            (
                                conv_title,
                                format!("{}: {}", author_name(&msg.author), msg.preview()),
                            )
                        } else {
                            (conv_title, msg.preview())
                        };
                        fx.notify = Some(Notification {
                            conversation: conv_id,
                            title,
                            body,
                        });
                    }
                }
            }
            BackendEvent::Typing {
                conversation,
                who,
                started,
            } => {
                let set = self.typing.entry(conversation).or_default();
                if started {
                    set.insert(who);
                } else {
                    set.remove(&who);
                }
                fx.timeline_changed = self.selected == Some(conversation);
            }
            BackendEvent::MessageRemoved { conversation, id } => {
                if self.selected == Some(conversation) {
                    let before = self.messages.len();
                    self.messages.retain(|m| m.id != id);
                    fx.timeline_changed = self.messages.len() != before;
                }
            }
            BackendEvent::ConversationRemoved(id) => {
                self.conversations.retain(|c| c.id != id);
                self.typing.remove(&id);
                if self.selected == Some(id) {
                    self.close();
                    fx.timeline_changed = true;
                }
            }
            BackendEvent::ContactsChanged | BackendEvent::SafetyNumber { .. } => {}
            BackendEvent::Error(e) => self.last_error = Some(e),
        }
        fx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::GroupKey;
    use crate::model::{AccountInfo, MessageKind, MessageStatus};

    const ALICE: Uuid = Uuid::from_u128(1);
    const BOB: Uuid = Uuid::from_u128(2);

    fn conv(id: ConversationId, title: &str, activity: i64) -> Conversation {
        let mut c = Conversation::new(id, title);
        c.last_activity = activity;
        c
    }

    fn msg(id: i64, conv: ConversationId, author: Uuid, sent_at: i64) -> Message {
        Message {
            id,
            conversation: conv,
            author,
            sent_at,
            received_at: sent_at,
            outgoing: false,
            body: format!("msg {id}"),
            attachments: vec![],
            quote: None,
            reactions: vec![],
            status: MessageStatus::Received,
            read: false,
            edited_at: None,
            deleted: false,
            expires_in_secs: None,
            expires_at: None,
            kind: MessageKind::Normal,
            body_ranges: vec![],
            edit_history: vec![],
        }
    }

    fn name(u: &Uuid) -> String {
        if *u == ALICE {
            "Alice".into()
        } else {
            "Bob".into()
        }
    }

    fn a() -> ConversationId {
        ConversationId::Direct(ALICE)
    }
    fn b() -> ConversationId {
        ConversationId::Direct(BOB)
    }
    fn g() -> ConversationId {
        ConversationId::Group(GroupKey([3; 32]))
    }

    fn ids(s: &ChatState) -> Vec<ConversationId> {
        s.visible_conversations().iter().map(|c| c.id).collect()
    }

    #[test]
    fn conversation_ordering_and_upsert() {
        let mut s = ChatState::new();
        s.set_conversations(vec![conv(a(), "Alice", 10), conv(b(), "Bob", 20)]);
        assert_eq!(ids(&s), vec![b(), a()]);
        s.upsert_conversation(conv(a(), "Alice", 30));
        assert_eq!(ids(&s), vec![a(), b()]);
        let mut pinned = conv(g(), "Group", 1);
        pinned.pinned = true;
        s.upsert_conversation(pinned);
        assert_eq!(ids(&s), vec![g(), a(), b()]);
        // conversations without activity aren't listed
        s.upsert_conversation(conv(ConversationId::Direct(Uuid::from_u128(9)), "X", 0));
        assert_eq!(s.conversations().len(), 3);
    }

    #[test]
    fn filter_and_archive() {
        let mut s = ChatState::new();
        let mut arch = conv(g(), "Book club", 5);
        arch.archived = true;
        s.set_conversations(vec![conv(a(), "Alice", 10), conv(b(), "Bob", 20), arch]);
        assert_eq!(ids(&s), vec![b(), a()]);
        assert_eq!(s.archived_count(), 1);
        s.set_filter("b");
        assert_eq!(ids(&s), vec![b(), g()], "search includes archived");
        s.set_filter("");
        s.show_archived = true;
        assert_eq!(ids(&s), vec![g()]);
    }

    #[test]
    fn unread_total_ignores_muted() {
        let mut s = ChatState::new();
        let mut c1 = conv(a(), "Alice", 1);
        c1.unread_count = 3;
        let mut c2 = conv(b(), "Bob", 2);
        c2.unread_count = 4;
        c2.muted = true;
        s.set_conversations(vec![c1, c2]);
        assert_eq!(s.total_unread(), 3);
    }

    #[test]
    fn timeline_upserts_in_order_and_history() {
        let mut s = ChatState::new();
        s.open(
            conv(a(), "Alice", 10),
            vec![msg(5, a(), ALICE, 50), msg(7, a(), ALICE, 70)],
            true,
        );
        assert!(s.upsert_message(msg(6, a(), ALICE, 60)));
        assert!(
            !s.upsert_message(msg(8, b(), BOB, 60)),
            "other conversations ignored"
        );
        let mut edited = msg(6, a(), ALICE, 60);
        edited.body = "edited".into();
        s.upsert_message(edited);
        let ids: Vec<i64> = s.messages().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![5, 6, 7]);
        assert_eq!(s.message(6).unwrap().body, "edited");
        assert_eq!(s.history_cursor(), Some((50, 5)));
        s.prepend_history(vec![msg(3, a(), ALICE, 30), msg(5, a(), ALICE, 50)], false);
        let ids: Vec<i64> = s.messages().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![3, 5, 6, 7]);
        assert!(!s.has_more_history);
        s.close();
        assert!(s.messages().is_empty());
    }

    #[test]
    fn opening_unlisted_conversation_shows_it() {
        let mut s = ChatState::new();
        s.open(conv(a(), "Alice", 0), vec![], false);
        assert_eq!(s.conversations().len(), 1);
        assert_eq!(s.selected_conversation().unwrap().title, "Alice");
        // and updates to it are kept while it is selected
        s.upsert_conversation(conv(a(), "Alice B", 0));
        assert_eq!(s.selected_conversation().unwrap().title, "Alice B");
    }

    #[test]
    fn incoming_in_open_conversation_marks_read() {
        let mut s = ChatState::new();
        s.set_conversations(vec![conv(a(), "Alice", 1)]);
        s.open(conv(a(), "Alice", 1), vec![], false);
        let fx = s.apply(BackendEvent::MessageUpserted(msg(1, a(), ALICE, 5)), name);
        assert_eq!(fx.mark_read, Some(a()));
        assert!(fx.notify.is_none());
        assert!(fx.timeline_changed);
    }

    #[test]
    fn incoming_when_unfocused_notifies() {
        let mut s = ChatState::new();
        s.set_conversations(vec![conv(a(), "Alice", 1), conv(g(), "Team", 1)]);
        s.open(conv(a(), "Alice", 1), vec![], false);
        s.window_active = false;
        let fx = s.apply(BackendEvent::MessageUpserted(msg(1, a(), ALICE, 5)), name);
        assert!(fx.mark_read.is_none());
        assert_eq!(fx.notify.unwrap().title, "Alice");
        let fx = s.apply(BackendEvent::MessageUpserted(msg(2, g(), BOB, 5)), name);
        let n = fx.notify.unwrap();
        assert_eq!(n.title, "Team");
        assert_eq!(n.body, "Bob: msg 2");
    }

    #[test]
    fn muted_and_outgoing_do_not_notify() {
        let mut s = ChatState::new();
        let mut c = conv(a(), "Alice", 1);
        c.muted = true;
        s.set_conversations(vec![c, conv(b(), "Bob", 1)]);
        assert!(
            s.apply(BackendEvent::MessageUpserted(msg(1, a(), ALICE, 5)), name)
                .notify
                .is_none()
        );
        let mut out = msg(2, b(), Uuid::from_u128(99), 5);
        out.outgoing = true;
        assert!(
            s.apply(BackendEvent::MessageUpserted(out), name)
                .notify
                .is_none()
        );
    }

    #[test]
    fn typing_indicator_lifecycle() {
        let mut s = ChatState::new();
        s.open(conv(a(), "Alice", 1), vec![], false);
        let fx = s.apply(
            BackendEvent::Typing {
                conversation: a(),
                who: ALICE,
                started: true,
            },
            name,
        );
        assert!(fx.timeline_changed);
        assert_eq!(s.typing_in(&a()), vec![ALICE]);
        s.apply(BackendEvent::MessageUpserted(msg(1, a(), ALICE, 5)), name);
        assert!(s.typing_in(&a()).is_empty(), "message clears typing");
        s.apply(
            BackendEvent::Typing {
                conversation: a(),
                who: ALICE,
                started: true,
            },
            name,
        );
        s.apply(
            BackendEvent::Typing {
                conversation: a(),
                who: ALICE,
                started: false,
            },
            name,
        );
        assert!(s.typing_in(&a()).is_empty());
    }

    #[test]
    fn removals() {
        let mut s = ChatState::new();
        s.set_conversations(vec![conv(a(), "Alice", 1), conv(b(), "Bob", 2)]);
        s.open(
            conv(a(), "Alice", 1),
            vec![msg(1, a(), ALICE, 1), msg(2, a(), ALICE, 2)],
            false,
        );
        let fx = s.apply(
            BackendEvent::MessageRemoved {
                conversation: a(),
                id: 1,
            },
            name,
        );
        assert!(fx.timeline_changed);
        assert_eq!(s.messages().len(), 1);
        let fx = s.apply(
            BackendEvent::MessageRemoved {
                conversation: b(),
                id: 9,
            },
            name,
        );
        assert!(!fx.timeline_changed);
        let fx = s.apply(BackendEvent::ConversationRemoved(a()), name);
        assert!(fx.timeline_changed);
        assert!(s.selected().is_none());
        assert_eq!(s.conversations().len(), 1);
    }

    #[test]
    fn account_changes_reset_state() {
        let mut s = ChatState::new();
        s.set_conversations(vec![conv(a(), "Alice", 1)]);
        s.open(conv(a(), "Alice", 1), vec![msg(1, a(), ALICE, 1)], false);
        let info = AccountInfo {
            aci: ALICE,
            phone_number: None,
            device_id: 2,
            device_name: None,
        };
        s.apply(BackendEvent::Account(AccountState::Registered(info)), name);
        assert_eq!(s.conversations().len(), 1);
        s.apply(BackendEvent::Account(AccountState::Unlinked), name);
        assert!(s.conversations().is_empty());
        assert!(s.selected().is_none());
        s.apply(BackendEvent::Error("boom".into()), name);
        assert_eq!(s.last_error.as_deref(), Some("boom"));
        s.apply(BackendEvent::Connection(ConnectionState::Online), name);
        assert_eq!(s.connection, ConnectionState::Online);
    }
}
