//! The root view: owns the UI state, reacts to backend events and dispatches
//! user intents as [`BackendCommand`]s.

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::message_scroller::MessageScrollerState;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, notification::Notification};
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, PathPromptOptions, Subscription,
    Window,
};
use signal_backend::BackendHandle;
use signal_core::format::local_day;
use signal_core::state::Effects;
use signal_core::timeline::{TimelineItem, build_timeline};
use signal_core::{
    AccountState, BackendCommand, BackendEvent, ChatState, Conversation, ConversationId, Database,
    Message, Quote, now_millis,
};
use uuid::Uuid;

use crate::theme::{Appearance, apply_appearance};

/// Number of messages loaded per page.
pub const PAGE_SIZE: usize = 60;
/// Quick reactions offered in the UI, like Signal Desktop's defaults.
pub const QUICK_REACTIONS: [&str; 6] = ["❤️", "👍", "👎", "😂", "😮", "😢"];
/// How long a "typing" indicator we send stays valid without new keystrokes.
const TYPING_IDLE: Duration = Duration::from_secs(3);
const TYPING_REFRESH: Duration = Duration::from_secs(10);

/// Destination for UI commands (the backend, or a recorder in tests).
pub trait CommandSink: 'static {
    fn send(&self, cmd: BackendCommand);
}

impl CommandSink for BackendHandle {
    fn send(&self, cmd: BackendCommand) {
        BackendHandle::send(self, cmd)
    }
}

/// Records commands; used by tests and as a no-op sink.
#[derive(Default, Clone)]
pub struct RecordingSink {
    commands: Rc<std::cell::RefCell<Vec<BackendCommand>>>,
    notifications: Rc<std::cell::RefCell<Vec<(String, String)>>>,
}

impl CommandSink for RecordingSink {
    fn send(&self, cmd: BackendCommand) {
        self.commands.borrow_mut().push(cmd);
    }
}

impl RecordingSink {
    pub fn take(&self) -> Vec<BackendCommand> {
        std::mem::take(&mut self.commands.borrow_mut())
    }

    /// Notifications raised so far, as `(title, body)`.
    pub fn take_notifications(&self) -> Vec<(String, String)> {
        std::mem::take(&mut self.notifications.borrow_mut())
    }
}

/// Whether to raise OS notifications.
pub trait Notifier: 'static {
    fn notify(&self, title: &str, body: &str);
}

/// Desktop notifications through the platform service.
pub struct DesktopNotifier;

impl Notifier for DesktopNotifier {
    fn notify(&self, title: &str, body: &str) {
        let (title, body) = (title.to_string(), body.to_string());
        std::thread::spawn(move || {
            if let Err(e) = notify_rust::Notification::new()
                .summary(&title)
                .body(&body)
                .appname("Signal")
                .show()
            {
                tracing::warn!(%e, "failed to show notification");
            }
        });
    }
}

impl Notifier for RecordingSink {
    fn notify(&self, title: &str, body: &str) {
        self.notifications
            .borrow_mut()
            .push((title.to_string(), body.to_string()));
    }
}

/// Snapshot of what the timeline renders; shared with row renderers.
pub struct TimelineData {
    pub items: Vec<TimelineItem>,
    pub messages: Vec<Message>,
    pub names: Rc<HashMap<Uuid, String>>,
    pub me: Option<Uuid>,
    pub is_group: bool,
    pub attachments: Rc<crate::attachments::AttachmentAccess>,
    pub audio: Rc<crate::audio::AudioPlayer>,
}

/// What changed in the open conversation's messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineChange {
    Reset,
    Prepend,
    Update,
}

/// Left-pane mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListMode {
    Chats,
    Archived,
    /// Choosing a contact to start a new chat.
    NewChat,
    Stories,
}

pub struct SignalApp {
    pub(crate) state: ChatState,
    pub(crate) db: Database,
    pub(crate) commands: Rc<dyn CommandSink>,
    pub(crate) notifier: Rc<dyn Notifier>,
    pub(crate) names: Rc<HashMap<Uuid, String>>,
    pub(crate) search: Entity<InputState>,
    pub(crate) composer: Entity<TextareaState>,
    pub(crate) device_name: Entity<InputState>,
    pub(crate) scroller: Entity<MessageScrollerState>,
    pub(crate) timeline: Rc<TimelineData>,
    pub(crate) replying_to: Option<Message>,
    pub(crate) editing: Option<Message>,
    pub(crate) conv_search: Entity<InputState>,
    pub(crate) conv_search_open: bool,
    pub(crate) conv_search_results: Vec<Message>,
    /// First unread message (id, count) when the chat was opened.
    pub(crate) unread_anchor: Option<(i64, usize)>,
    pub(crate) pending_attachments: Vec<PathBuf>,
    pub(crate) list_mode: ListMode,
    pub(crate) contacts: Vec<Conversation>,
    pub(crate) message_results: Vec<Message>,
    pub(crate) loading_history: bool,
    pub(crate) prefs: crate::settings::Preferences,
    typing_sent: Option<(ConversationId, Instant)>,
    typing_generation: u64,
    focus_override: Option<bool>,
    settings_path: Option<PathBuf>,
    pub(crate) attachments: Rc<crate::attachments::AttachmentAccess>,
    pub(crate) audio: Rc<crate::audio::AudioPlayer>,
    pub(crate) stories: Vec<signal_core::Story>,
    pub(crate) story_reply: Entity<InputState>,
    audio_was_active: bool,
    pub(crate) composer_emoji: Entity<crate::views::emoji_picker::EmojiPicker>,
    pub(crate) reaction_picker: Entity<crate::views::emoji_picker::EmojiPicker>,
    pub(crate) reaction_target: Rc<std::cell::Cell<Option<i64>>>,
    pub(crate) focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl SignalApp {
    pub fn new(
        db: Database,
        commands: Rc<dyn CommandSink>,
        notifier: Rc<dyn Notifier>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let conv_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search in chat"));
        let story_reply = cx.new(|cx| InputState::new(window, cx).placeholder("Reply…"));
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .submit_on_enter(true)
                .placeholder("Message")
        });
        let device_name = cx.new(|cx| {
            let host = hostname();
            InputState::new(window, cx)
                .placeholder("Device name")
                .default_value(format!("Signal GPUI on {host}"))
        });
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let weak = cx.entity().downgrade();
        let composer_emoji = {
            let weak = weak.clone();
            cx.new(|cx| {
                crate::views::emoji_picker::EmojiPicker::new(
                    "composer-emoji",
                    Rc::new(move |emoji, window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.composer
                                .update(cx, |c, cx| c.insert(emoji, window, cx));
                        });
                    }),
                    window,
                    cx,
                )
            })
        };
        let reaction_target: Rc<std::cell::Cell<Option<i64>>> = Rc::default();
        let reaction_picker = {
            let target = reaction_target.clone();
            cx.new(|cx| {
                crate::views::emoji_picker::EmojiPicker::new(
                    "reaction-emoji",
                    Rc::new(move |emoji, window, cx| {
                        if let Some(id) = target.take() {
                            let _ = weak.update(cx, |this, cx| this.react(id, emoji, cx));
                        }
                        window.close_dialog(cx);
                    }),
                    window,
                    cx,
                )
            })
        };

        let subscriptions = vec![
            cx.subscribe_in(&search, window, |this, state, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    let q = state.read(cx).value().to_string();
                    this.set_search(q, cx);
                }
            }),
            cx.subscribe_in(
                &conv_search,
                window,
                |this, state, ev: &InputEvent, _, cx| {
                    if matches!(ev, InputEvent::Change) {
                        let q = state.read(cx).value().to_string();
                        this.conv_search_results = match this.state.selected() {
                            Some(id) if q.trim().chars().count() >= 2 => this
                                .db
                                .search_in_conversation(id, &q, 50)
                                .unwrap_or_default(),
                            _ => Vec::new(),
                        };
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &composer,
                window,
                |this, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::PressEnter { shift, secondary } => {
                        if *secondary || (this.prefs.enter_sends && !*shift) {
                            this.send_composed(window, cx)
                        }
                    }
                    InputEvent::Change => this.on_composer_changed(window, cx),
                    InputEvent::Blur => this.save_draft(cx),
                    _ => {}
                },
            ),
            cx.observe_window_activation(window, |this, window, cx| {
                this.state.window_active = this.window_is_active(window);
                if this.state.window_active {
                    this.mark_selected_read();
                }
                cx.notify();
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                if this.prefs.appearance == Appearance::System {
                    apply_appearance(Appearance::System, Some(window), cx);
                    crate::theme::apply_zoom(this.prefs.zoom, window, cx);
                    cx.notify();
                }
            }),
        ];

        let mut this = Self {
            state: ChatState::new(),
            db,
            commands,
            notifier,
            names: Rc::default(),
            search,
            composer,
            device_name,
            scroller,
            timeline: Rc::new(TimelineData {
                items: vec![],
                messages: vec![],
                names: Rc::default(),
                me: None,
                is_group: false,
                attachments: Rc::default(),
                audio: Rc::default(),
            }),
            replying_to: None,
            editing: None,
            conv_search,
            conv_search_open: false,
            conv_search_results: Vec::new(),
            unread_anchor: None,
            pending_attachments: Vec::new(),
            list_mode: ListMode::Chats,
            contacts: Vec::new(),
            message_results: Vec::new(),
            loading_history: false,
            prefs: crate::settings::Preferences::default(),
            typing_sent: None,
            typing_generation: 0,
            focus_override: None,
            settings_path: None,
            attachments: Rc::default(),
            audio: Rc::default(),
            stories: Vec::new(),
            story_reply,
            audio_was_active: false,
            composer_emoji,
            reaction_picker,
            reaction_target,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.state.window_active = this.window_is_active(window);
        this.reload_names();
        // repaint audio progress and disappearing-message countdowns
        cx.spawn_in(window, async move |this, cx| {
            let mut tick: u32 = 0;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                tick = tick.wrapping_add(1);
                let alive = this.update_in(cx, |this, window, cx| {
                    if let Some(e) = this.audio.take_error() {
                        window.push_notification(Notification::error(e), cx);
                    }
                    let audio = this.audio.is_active();
                    let countdown = tick.is_multiple_of(4)
                        && this.state.messages().iter().any(|m| m.expires_at.is_some());
                    if audio || countdown || this.audio_was_active {
                        cx.notify();
                    }
                    this.audio_was_active = audio;
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        this
    }

    // ------------------------------------------------------------- accessors

    /// Overrides OS window-focus detection, which decides between marking
    /// messages read and raising notifications. `None` uses the OS state.
    pub fn set_focus_override(&mut self, active: Option<bool>) {
        self.focus_override = active;
    }

    fn window_is_active(&self, window: &Window) -> bool {
        self.focus_override
            .unwrap_or_else(|| window.is_window_active())
    }

    pub fn state(&self) -> &ChatState {
        &self.state
    }

    pub fn composer(&self) -> &Entity<TextareaState> {
        &self.composer
    }

    pub fn audio_status(&self) -> crate::audio::PlaybackStatus {
        self.audio.status()
    }

    /// Ids of the current in-conversation search results.
    pub fn conv_search_result_ids(&self) -> Vec<i64> {
        self.conv_search_results.iter().map(|m| m.id).collect()
    }

    pub fn editing(&self) -> Option<&Message> {
        self.editing.as_ref()
    }

    pub fn replying_to(&self) -> Option<&Message> {
        self.replying_to.as_ref()
    }

    pub fn pending_attachments(&self) -> &[PathBuf] {
        &self.pending_attachments
    }

    pub fn list_mode(&self) -> ListMode {
        self.list_mode
    }

    pub fn timeline(&self) -> &TimelineData {
        &self.timeline
    }

    pub fn display_name(&self, uuid: &Uuid) -> String {
        if Some(*uuid) == self.me() {
            return "You".into();
        }
        self.names
            .get(uuid)
            .cloned()
            .unwrap_or_else(|| "Unknown contact".into())
    }

    pub fn me(&self) -> Option<Uuid> {
        match &self.state.account {
            AccountState::Registered(info) => Some(info.aci),
            _ => None,
        }
    }

    fn reload_names(&mut self) {
        let names = self
            .db
            .contacts()
            .unwrap_or_default()
            .into_iter()
            .map(|c| (c.uuid, c.display_name()))
            .collect();
        self.names = Rc::new(names);
    }

    /// "Note to Self" title for our own conversation, like Signal.
    pub const NOTE_TO_SELF: &'static str = "Note to Self";

    pub fn is_note_to_self(&self, id: &ConversationId) -> bool {
        self.me()
            .is_some_and(|me| *id == ConversationId::Direct(me))
    }

    fn normalize(&self, mut conv: Conversation) -> Conversation {
        if self.is_note_to_self(&conv.id) {
            conv.title = Self::NOTE_TO_SELF.to_string();
        }
        conv
    }

    fn reload_conversations(&mut self) {
        match self.db.conversations() {
            Ok(list) => {
                let list = list.into_iter().map(|c| self.normalize(c)).collect();
                self.state.set_conversations(list)
            }
            Err(e) => tracing::error!(%e, "failed to load conversations"),
        }
    }

    // ---------------------------------------------------------------- events

    /// Applies backend events from `events` until the channel closes.
    pub fn listen(
        &mut self,
        mut events: futures::channel::mpsc::UnboundedReceiver<BackendEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use futures::StreamExt as _;
        cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = events.next().await {
                if this
                    .update_in(cx, |this, window, cx| this.handle_event(event, window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Applies a backend event to the UI.
    pub fn handle_event(
        &mut self,
        event: BackendEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut reload_names = false;
        match &event {
            BackendEvent::Account(AccountState::Registered(_)) => {
                reload_names = true;
            }
            BackendEvent::ContactsChanged => reload_names = true,
            BackendEvent::StoriesChanged => self.reload_stories(),
            BackendEvent::SafetyNumber { contact, number } => {
                self.show_safety_number(*contact, number.clone(), window, cx);
            }
            BackendEvent::ConversationUpdated(c) if !c.is_group() => reload_names = true,
            BackendEvent::Error(e) => {
                window.push_notification(Notification::error(e.clone()), cx);
            }
            _ => {}
        }
        self.state.window_active = self.window_is_active(window);
        let registered_now = matches!(&event, BackendEvent::Account(AccountState::Registered(_)));
        let names = self.names.clone();
        let me = self.me();
        let event = match event {
            BackendEvent::ConversationUpdated(c) => {
                BackendEvent::ConversationUpdated(self.normalize(c))
            }
            other => other,
        };
        let fx: Effects = self.state.apply(event, |u| {
            if Some(*u) == me {
                "You".into()
            } else {
                names.get(u).cloned().unwrap_or_else(|| "Unknown".into())
            }
        });
        if reload_names {
            self.reload_names();
        }
        if registered_now {
            self.reload_conversations();
            self.reload_stories();
            if self.list_mode == ListMode::NewChat {
                self.load_contacts();
            }
        }
        if let Some(conv) = fx.mark_read {
            self.commands
                .send(BackendCommand::MarkRead { conversation: conv });
        }
        if let Some(n) = fx.notify
            && self.prefs.notifications
        {
            let (title, body) = self.prefs.notification_content.redact(&n.title, &n.body);
            self.notifier.notify(&title, &body);
        }
        if fx.timeline_changed {
            self.rebuild_timeline(TimelineChange::Update, cx);
        }
        self.update_title(window);
        cx.notify();
    }

    fn update_title(&self, window: &mut Window) {
        let unread = self.state.total_unread();
        let title = if unread > 0 {
            format!("Signal ({unread})")
        } else {
            "Signal".to_string()
        };
        window.set_window_title(&title);
    }

    // ---------------------------------------------------------- conversations

    pub fn open_conversation(
        &mut self,
        id: ConversationId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.selected() == Some(&id) {
            return;
        }
        self.save_draft(cx);
        self.stop_typing();
        let conv = match self.db.ensure_conversation(&id) {
            Ok(c) => c,
            Err(e) => {
                window.push_notification(
                    Notification::error(format!("Failed to open chat: {e}")),
                    cx,
                );
                return;
            }
        };
        let messages = self.db.messages(&id, None, PAGE_SIZE).unwrap_or_default();
        let has_more = messages.len() == PAGE_SIZE;
        let draft = conv.draft.clone().unwrap_or_default();
        let unread = conv.unread_count;
        let conv = self.normalize(conv);
        let first_unread = messages
            .iter()
            .find(|m| !m.outgoing && !m.read && !m.is_notice())
            .map(|m| m.id);
        self.unread_anchor = first_unread.map(|id| (id, unread as usize));
        self.state.open(conv, messages, has_more);
        self.conv_search_open = false;
        self.conv_search_results.clear();
        self.replying_to = None;
        self.editing = None;
        self.pending_attachments.clear();
        if self.list_mode == ListMode::NewChat {
            self.list_mode = ListMode::Chats;
            self.clear_search(window, cx);
        }
        self.composer
            .update(cx, |c, cx| c.set_value(draft, window, cx));
        self.composer.read(cx).focus_handle(cx).focus(window, cx);
        self.state.window_active = self.window_is_active(window);
        if unread > 0 && self.state.window_active {
            self.commands
                .send(BackendCommand::MarkRead { conversation: id });
        }
        self.rebuild_timeline(TimelineChange::Reset, cx);
        cx.notify();
    }

    pub fn toggle_conversation_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.conv_search_open = !self.conv_search_open;
        self.conv_search_results.clear();
        self.conv_search
            .update(cx, |s, cx| s.set_value("", window, cx));
        if self.conv_search_open {
            self.conv_search.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// Jumps to a search result and closes the search bar.
    pub fn jump_to_result(
        &mut self,
        message: Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.conv_search_open = false;
        self.conv_search_results.clear();
        self.scroll_to_message(message.author, message.sent_at, window, cx);
        cx.notify();
    }

    /// Plays or pauses an audio attachment.
    pub fn toggle_audio(
        &mut self,
        message_id: i64,
        index: usize,
        path: String,
        cx: &mut Context<Self>,
    ) {
        let access = self.attachments.clone();
        self.audio
            .toggle((message_id, index), || access.bytes(&path).ok());
        cx.notify();
    }

    pub fn close_conversation(&mut self, cx: &mut Context<Self>) {
        self.audio.stop();
        self.conv_search_open = false;
        self.conv_search_results.clear();
        self.unread_anchor = None;
        self.save_draft(cx);
        self.stop_typing();
        self.state.close();
        self.replying_to = None;
        self.editing = None;
        self.pending_attachments.clear();
        self.rebuild_timeline(TimelineChange::Reset, cx);
        cx.notify();
    }

    /// Moves the selection up or down the visible conversation list.
    pub fn select_relative(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<ConversationId> = self
            .state
            .visible_conversations()
            .iter()
            .map(|c| c.id)
            .collect();
        if ids.is_empty() {
            return;
        }
        let current = self
            .state
            .selected()
            .and_then(|s| ids.iter().position(|i| i == s));
        let next = match current {
            Some(i) => (i as isize + delta).clamp(0, ids.len() as isize - 1) as usize,
            None => 0,
        };
        self.open_conversation(ids[next], window, cx);
    }

    fn mark_selected_read(&mut self) {
        if let Some(c) = self.state.selected_conversation()
            && c.unread_count > 0
        {
            self.commands
                .send(BackendCommand::MarkRead { conversation: c.id });
        }
    }

    pub fn load_older(&mut self, cx: &mut Context<Self>) {
        if self.loading_history || !self.state.has_more_history {
            return;
        }
        let (Some(id), Some(cursor)) =
            (self.state.selected().copied(), self.state.history_cursor())
        else {
            return;
        };
        self.loading_history = true;
        let older = self
            .db
            .messages(&id, Some(cursor), PAGE_SIZE)
            .unwrap_or_default();
        let has_more = older.len() == PAGE_SIZE;
        self.state.prepend_history(older, has_more);
        self.loading_history = false;
        self.rebuild_timeline(TimelineChange::Prepend, cx);
        cx.notify();
    }

    pub(crate) fn rebuild_timeline(&mut self, change: TimelineChange, cx: &mut Context<Self>) {
        let today = local_day(now_millis());
        let mut items = build_timeline(self.state.messages(), today, local_day);
        let marker = self.unread_anchor.and_then(|(id, n)| {
            signal_core::timeline::insert_unread_marker(&mut items, self.state.messages(), id, n)
        });
        let old_len = self.timeline.items.len();
        let new_len = items.len();
        self.timeline = Rc::new(TimelineData {
            items,
            messages: self.state.messages().to_vec(),
            names: self.names.clone(),
            me: self.me(),
            is_group: self.state.selected().is_some_and(|c| c.is_group()),
            attachments: self.attachments.clone(),
            audio: self.audio.clone(),
        });
        self.scroller.update(cx, |s, cx| match change {
            TimelineChange::Reset => {
                s.reset(new_len, cx);
                // open at the first unread message, like Signal
                if let Some(ix) = marker {
                    s.scroll_to_item(ix, cx);
                }
            }
            TimelineChange::Prepend if new_len >= old_len => {
                s.prepend(new_len - old_len, cx);
                s.remeasure(cx);
            }
            TimelineChange::Update if new_len > old_len && old_len == s.item_count() => {
                let following = s.is_following_tail() || !s.is_scrolled_up();
                s.append(new_len - old_len, cx);
                s.remeasure(cx);
                if following {
                    s.scroll_to_end(cx);
                }
            }
            TimelineChange::Update | TimelineChange::Prepend if new_len == s.item_count() => {
                s.remeasure(cx)
            }
            _ => s.reset(new_len, cx),
        });
    }

    /// Scrolls the timeline to the message (e.g. the original of a quote),
    /// loading older history if needed.
    pub fn scroll_to_message(
        &mut self,
        author: Uuid,
        sent_at: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for _ in 0..50 {
            if let Some(m) = self
                .state
                .messages()
                .iter()
                .find(|m| m.author == author && m.sent_at == sent_at)
            {
                let id = m.id;
                if let Some(ix) = signal_core::timeline::index_of_message(
                    &self.timeline.items,
                    &self.timeline.messages,
                    id,
                ) {
                    self.scroller.update(cx, |s, cx| {
                        s.scroll_to_item(ix, cx);
                    });
                }
                return;
            }
            if !self.state.has_more_history {
                break;
            }
            self.load_older(cx);
        }
        window.push_notification(Notification::info("Original message not found"), cx);
    }

    pub fn toggle_pinned(&mut self, id: ConversationId, cx: &mut Context<Self>) {
        let pinned = self.state.conversation(&id).is_some_and(|c| c.pinned);
        if let Ok(Some(c)) = self.db.set_pinned(&id, !pinned) {
            self.state.upsert_conversation(c);
        }
        cx.notify();
    }

    pub fn toggle_muted(&mut self, id: ConversationId, cx: &mut Context<Self>) {
        let muted = self.state.conversation(&id).is_some_and(|c| c.muted);
        if let Ok(Some(c)) = self.db.set_muted(&id, !muted) {
            self.state.upsert_conversation(c);
        }
        cx.notify();
    }

    pub fn toggle_archived(&mut self, id: ConversationId, cx: &mut Context<Self>) {
        let archived = self.state.conversation(&id).is_some_and(|c| c.archived);
        if let Ok(Some(c)) = self.db.set_archived(&id, !archived) {
            self.state.upsert_conversation(c);
        }
        if !archived && self.state.selected() == Some(&id) {
            self.close_conversation(cx);
        }
        cx.notify();
    }

    pub fn set_list_mode(&mut self, mode: ListMode, window: &mut Window, cx: &mut Context<Self>) {
        self.list_mode = mode;
        self.state.show_archived = mode == ListMode::Archived;
        if mode == ListMode::Stories {
            self.reload_stories();
        }
        if mode == ListMode::NewChat {
            self.load_contacts();
            self.search.read(cx).focus_handle(cx).focus(window, cx);
        }
        self.clear_search(window, cx);
        cx.notify();
    }

    fn load_contacts(&mut self) {
        let me = self.me();
        let mut contacts: Vec<Conversation> = self
            .db
            .all_conversations()
            .unwrap_or_default()
            .into_iter()
            .filter(|c| me.is_none() || c.id.direct_uuid() != me)
            .collect();
        // Note to Self is always offered first
        if let Some(me) = me {
            let id = ConversationId::Direct(me);
            let note = self
                .db
                .conversation(&id)
                .ok()
                .flatten()
                .unwrap_or_else(|| Conversation::new(id, ""));
            contacts.insert(0, self.normalize(note));
        }
        self.contacts = contacts;
    }

    /// Contacts matching the search query, for the new-chat picker.
    pub fn filtered_contacts(&self) -> Vec<&Conversation> {
        let q = self.state.filter().trim().to_lowercase();
        self.contacts
            .iter()
            .filter(|c| q.is_empty() || c.title.to_lowercase().contains(&q))
            .collect()
    }

    // ---------------------------------------------------------------- search

    pub(crate) fn set_search(&mut self, query: String, cx: &mut Context<Self>) {
        self.message_results =
            if query.trim().chars().count() >= 2 && self.list_mode != ListMode::NewChat {
                self.db.search_messages(&query, 50).unwrap_or_default()
            } else {
                Vec::new()
            };
        self.state.set_filter(query);
        cx.notify();
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        self.set_search(String::new(), cx);
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.read(cx).focus_handle(cx).focus(window, cx);
    }

    // -------------------------------------------------------------- composing

    pub fn send_composed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conversation) = self.state.selected().copied() else {
            return;
        };
        let body = self.composer.read(cx).value().to_string();
        if let Some(target) = self.editing.take() {
            if !body.trim().is_empty() && body.trim() != target.body.trim() {
                self.commands.send(BackendCommand::Edit {
                    conversation,
                    target_sent_at: target.sent_at,
                    body: body.trim().to_string(),
                });
            }
            self.composer
                .update(cx, |c, cx| c.set_value("", window, cx));
            cx.notify();
            return;
        }
        if body.trim().is_empty() && self.pending_attachments.is_empty() {
            return;
        }
        let quote = self.replying_to.take().map(|m| Quote {
            author: m.author,
            sent_at: m.sent_at,
            text: m.preview(),
        });
        self.commands.send(BackendCommand::SendText {
            conversation,
            body: body.trim().to_string(),
            quote,
            attachments: std::mem::take(&mut self.pending_attachments),
        });
        self.typing_sent = None;
        self.typing_generation += 1;
        self.composer
            .update(cx, |c, cx| c.set_value("", window, cx));
        let _ = self.db.set_draft(&conversation, None);
        self.scroller.update(cx, |s, cx| s.scroll_to_end(cx));
        cx.notify();
    }

    fn on_composer_changed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(conversation) = self.state.selected().copied() else {
            return;
        };
        let empty = self.composer.read(cx).value().trim().is_empty();
        if self.is_note_to_self(&conversation) || !self.prefs.typing_indicators {
            return; // nobody to show it to, or the user turned typing indicators off
        }
        if empty {
            self.stop_typing();
            return;
        }
        let fresh = match self.typing_sent {
            Some((c, at)) => c != conversation || at.elapsed() > TYPING_REFRESH,
            None => true,
        };
        if fresh {
            self.typing_sent = Some((conversation, Instant::now()));
            self.commands.send(BackendCommand::Typing {
                conversation,
                started: true,
            });
        }
        // stop after a pause in typing
        self.typing_generation += 1;
        let generation = self.typing_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TYPING_IDLE).await;
            let _ = this.update(cx, |this, _| {
                if this.typing_generation == generation {
                    this.stop_typing();
                }
            });
        })
        .detach();
    }

    fn stop_typing(&mut self) {
        if let Some((conversation, _)) = self.typing_sent.take() {
            self.commands.send(BackendCommand::Typing {
                conversation,
                started: false,
            });
        }
    }

    pub(crate) fn save_draft(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.state.selected().copied() else {
            return;
        };
        let text = self.composer.read(cx).value().to_string();
        let draft = Some(text.as_str()).filter(|t| !t.trim().is_empty());
        if let Err(e) = self.db.set_draft(&id, draft) {
            tracing::warn!(%e, "failed to save draft");
        }
        if let Ok(Some(c)) = self.db.conversation(&id) {
            self.state.upsert_conversation(c);
        }
    }

    pub fn reply_to(&mut self, message_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.replying_to = self.state.message(message_id).cloned();
        self.composer.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// Puts the composer in edit mode for one of our own recent messages.
    pub fn start_edit(&mut self, message_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.state.message(message_id).cloned() else {
            return;
        };
        if !msg.editable(signal_core::now_millis()) {
            return;
        }
        self.replying_to = None;
        let body = msg.body.clone();
        self.editing = Some(msg);
        self.composer
            .update(cx, |c, cx| c.set_value(body, window, cx));
        self.composer.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.take().is_some() {
            self.composer
                .update(cx, |c, cx| c.set_value("", window, cx));
        }
        cx.notify();
    }

    pub fn cancel_reply(&mut self, cx: &mut Context<Self>) {
        self.replying_to = None;
        cx.notify();
    }

    pub fn react(&mut self, message_id: i64, emoji: &str, cx: &mut Context<Self>) {
        let (Some(msg), Some(me)) = (self.state.message(message_id), self.me()) else {
            return;
        };
        let remove = msg
            .reactions
            .iter()
            .any(|r| r.reactor == me && r.emoji == emoji);
        self.commands.send(BackendCommand::React {
            conversation: msg.conversation,
            target_author: msg.author,
            target_sent_at: msg.sent_at,
            emoji: emoji.to_string(),
            remove,
        });
        cx.notify();
    }

    pub fn delete_for_everyone(&mut self, message_id: i64, cx: &mut Context<Self>) {
        if let Some(msg) = self.state.message(message_id)
            && msg.outgoing
            && !msg.deleted
        {
            self.commands.send(BackendCommand::DeleteForEveryone {
                conversation: msg.conversation,
                sent_at: msg.sent_at,
            });
        }
        cx.notify();
    }

    pub fn resend(&mut self, message_id: i64, cx: &mut Context<Self>) {
        if let Some(msg) = self.state.message(message_id) {
            self.commands.send(BackendCommand::Resend {
                conversation: msg.conversation,
                sent_at: msg.sent_at,
            });
        }
        cx.notify();
    }

    pub fn copy_text(&mut self, message_id: i64, cx: &mut Context<Self>) {
        if let Some(msg) = self.state.message(message_id) {
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(msg.body.clone()));
        }
    }

    pub fn attach_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update(cx, |this, cx| this.add_attachments(paths, cx));
            }
        })
        .detach();
    }

    pub fn add_attachments(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        for p in paths {
            if p.is_file() && !self.pending_attachments.contains(&p) {
                self.pending_attachments.push(p);
            }
        }
        cx.notify();
    }

    pub fn remove_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.pending_attachments.len() {
            self.pending_attachments.remove(index);
        }
        cx.notify();
    }

    // ---------------------------------------------------------------- account

    pub fn start_linking(&mut self, cx: &mut Context<Self>) {
        let name = self.device_name.read(cx).value().trim().to_string();
        let device_name = if name.is_empty() {
            format!("Signal GPUI on {}", hostname())
        } else {
            name
        };
        self.commands.send(BackendCommand::Link { device_name });
        self.state.account = AccountState::Linking { url: None };
        cx.notify();
    }

    pub fn confirm_unlink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let commands = self.commands.clone();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let commands = commands.clone();
            dialog
                .title("Unlink this device?")
                .description(
                    "All messages on this computer will be deleted. Your phone and other devices are not affected.",
                )
                .ok_text("Unlink")
                .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                .show_cancel(true)
                .on_ok(move |_, _, _| {
                    commands.send(BackendCommand::Unlink);
                    true
                })
        });
    }

    /// People in a conversation as `(uuid, display name)`, "You" first, then
    /// by name. For a direct chat this is just the other person.
    pub fn members(&self, id: &ConversationId) -> Vec<(Uuid, String)> {
        let me = self.me();
        let uuids: Vec<Uuid> = match id {
            ConversationId::Direct(u) => vec![*u],
            ConversationId::Group(_) => self
                .state
                .conversation(id)
                .map(|c| c.members.clone())
                .or_else(|| self.db.conversation(id).ok().flatten().map(|c| c.members))
                .unwrap_or_default(),
        };
        let mut out: Vec<(Uuid, String)> = uuids
            .into_iter()
            .map(|u| (u, self.display_name(&u)))
            .collect();
        out.sort_by(|a, b| {
            (Some(b.0) == me)
                .cmp(&(Some(a.0) == me))
                .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
        });
        out.dedup_by_key(|m| m.0);
        out
    }

    /// Opens the conversation details panel (group members / contact info).
    pub fn show_details(
        &mut self,
        id: ConversationId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = self
            .state
            .conversation(&id)
            .map(|c| c.title.clone())
            .unwrap_or_else(|| "Chat details".into());
        let members = self.members(&id);
        let phones: HashMap<Uuid, String> = self
            .db
            .contacts()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|c| c.phone.map(|p| (c.uuid, p)))
            .collect();
        let me = self.me();
        let palette = crate::theme::Palette::current(cx);
        window.open_dialog(cx, move |dialog, _, _| {
            crate::views::details::details_dialog(
                dialog, &title, id, &members, &phones, me, &palette,
            )
        });
    }

    /// Uses `access` to read (encrypted) attachment files.
    pub fn set_attachment_access(&mut self, access: crate::attachments::AttachmentAccess) {
        self.attachments = Rc::new(access);
    }

    /// Loads (and from now on saves) UI preferences at `path`.
    pub fn use_settings_file(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prefs = crate::settings::Preferences::load(&path);
        self.settings_path = Some(path);
        self.apply_prefs(prefs, window, cx);
    }

    pub fn prefs(&self) -> &crate::settings::Preferences {
        &self.prefs
    }

    /// Applies (and persists) new preferences.
    pub fn apply_prefs(
        &mut self,
        prefs: crate::settings::Preferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        apply_appearance(prefs.appearance, Some(window), cx);
        crate::theme::apply_zoom(prefs.zoom, window, cx);
        let enter_sends = prefs.enter_sends;
        self.composer
            .update(cx, |c, cx| c.set_submit_on_enter(enter_sends, cx));
        if prefs.read_receipts != self.prefs.read_receipts || self.settings_path.is_none() {
            self.commands
                .send(BackendCommand::SetReadReceipts(prefs.read_receipts));
        }
        if !prefs.typing_indicators {
            self.stop_typing();
        }
        self.prefs = prefs;
        if let Some(path) = &self.settings_path
            && let Err(e) = self.prefs.save(path)
        {
            tracing::warn!(%e, "failed to save settings");
        }
        cx.notify();
    }

    pub fn update_prefs(
        &mut self,
        f: impl FnOnce(&mut crate::settings::Preferences),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut prefs = self.prefs.clone();
        f(&mut prefs);
        self.apply_prefs(prefs, window, cx);
    }

    /// Zooms in (`dir > 0`), out (`dir < 0`) or back to 100% (`0`).
    pub fn zoom(&mut self, dir: i32, window: &mut Window, cx: &mut Context<Self>) {
        let z = if dir == 0 {
            1.0
        } else {
            crate::settings::step_zoom(self.prefs.zoom, dir)
        };
        self.update_prefs(|p| p.zoom = z, window, cx);
    }

    pub fn set_appearance(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_prefs(|p| p.appearance = appearance, window, cx);
    }

    pub fn is_dark(&self, cx: &gpui_kit::App) -> bool {
        cx.theme().mode.is_dark()
    }
}

impl Focusable for SignalApp {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn hostname() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| {
                    String::from_utf8(o.stdout)
                        .ok()
                        .map(|s| s.trim().trim_end_matches(".local").to_string())
                })
        })
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "Desktop".into())
}
