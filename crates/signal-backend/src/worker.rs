//! The backend actor. Runs on its own OS thread with a single-threaded tokio
//! runtime + `LocalSet` (presage's `Manager` is `!Send`). All database writes
//! coming from the network happen here, sequentially; slow network operations
//! run as local tasks that report back through an internal channel.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::oneshot;
use presage::Manager;
use presage::libsignal_service::configuration::SignalServers;
use presage::libsignal_service::content::ContentBody;
use presage::libsignal_service::proto::{
    DataMessage, EditMessage, GroupContextV2, ReceiptMessage, SyncMessage, TypingMessage,
    data_message, receipt_message, sync_message, typing_message,
};
use presage::libsignal_service::protocol::{Aci, ServiceId};
use presage::libsignal_service::sender::AttachmentSpec;
use presage::libsignal_service::zkgroup::groups::{GroupMasterKey, GroupSecretParams};
use presage::manager::Registered;
use presage::model::identity::OnNewIdentity;
use presage::model::messages::Received;
use presage::store::{ContentsStore, StateStore, Store};
use presage_store_sqlite::SqliteStore;
use signal_core::{
    AccountInfo, AccountState, Attachment, BackendCommand, BackendEvent, Changes, ConnectionState,
    Contact, ConversationId, Database, GroupKey, MessageKind, MessageStatus, Millis, Mutation,
    NewMessage, Quote, now_millis,
};
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, LocalSet, spawn_local};
use uuid::Uuid;

use crate::interpret::{self, Ctx, Output};
use crate::paths::DataPaths;

pub type EventSink = Arc<dyn Fn(BackendEvent) + Send + Sync>;

type Mgr = Manager<SqliteStore, Registered>;

#[derive(Clone, Debug)]
pub struct BackendConfig {
    pub paths: DataPaths,
    /// Key used to encrypt both databases.
    pub db_key: String,
    pub servers: SignalServers,
}

/// Handle to the backend thread. Dropping it shuts the backend down.
pub struct BackendHandle {
    tx: mpsc::UnboundedSender<BackendCommand>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl BackendHandle {
    pub(crate) fn from_parts(
        tx: mpsc::UnboundedSender<BackendCommand>,
        thread: std::thread::JoinHandle<()>,
    ) -> Self {
        Self {
            tx,
            thread: Some(thread),
        }
    }

    pub fn send(&self, cmd: BackendCommand) {
        if self.tx.send(cmd).is_err() {
            tracing::error!("backend thread is gone");
        }
    }

    /// Requests shutdown and waits for the thread to finish.
    pub fn shutdown(mut self) {
        let _ = self.tx.send(BackendCommand::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for BackendHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(BackendCommand::Shutdown);
    }
}

/// Starts the backend thread.
pub fn spawn(config: BackendConfig, events: EventSink) -> std::io::Result<BackendHandle> {
    let (tx, rx) = mpsc::unbounded_channel();
    let thread = std::thread::Builder::new()
        .name("signal-backend".into())
        // libsignal's post-quantum code uses large stack frames
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    events(BackendEvent::Error(format!("failed to start runtime: {e}")));
                    return;
                }
            };
            let local = LocalSet::new();
            local.block_on(&rt, async move {
                match Worker::open(config, events.clone()).await {
                    Ok(worker) => worker.run(rx).await,
                    Err(e) => {
                        tracing::error!(%e, "backend failed to start");
                        events(BackendEvent::Error(format!(
                            "Failed to open local storage: {e}"
                        )));
                    }
                }
            });
        })?;
    Ok(BackendHandle {
        tx,
        thread: Some(thread),
    })
}

/// Results of background tasks, delivered back to the actor loop.
enum Internal {
    LinkUrl(String),
    Linked(Result<Box<Mgr>, String>),
    Received(Received),
    Connection(ConnectionState),
    SendResult {
        conversation: ConversationId,
        sent_at: Millis,
        ok: bool,
        error: Option<String>,
    },
    AttachmentDownloaded {
        message_id: i64,
        index: usize,
        path: Option<PathBuf>,
    },
    Profile {
        uuid: Uuid,
        name: Option<String>,
    },
    Error(String),
    StoryAttachmentDownloaded {
        story_id: i64,
        path: Option<PathBuf>,
    },
    Avatar {
        conversation: ConversationId,
        bytes: Option<Vec<u8>>,
        about: Option<String>,
    },
}

struct Worker {
    config: BackendConfig,
    store: SqliteStore,
    db: Database,
    events: EventSink,
    manager: Option<Mgr>,
    me: Option<Uuid>,
    group_ids: HashMap<[u8; 32], GroupKey>,
    receive_task: Option<JoinHandle<()>>,
    link_task: Option<JoinHandle<()>>,
    internal_tx: mpsc::UnboundedSender<Internal>,
    internal_rx: Option<mpsc::UnboundedReceiver<Internal>>,
    last_sent_at: Millis,
    profile_requested: HashSet<Uuid>,
    read_receipts: bool,
    avatar_requested: HashSet<ConversationId>,
}

/// Derives the public group identifier (as used in typing messages) from a master key.
pub fn group_identifier(key: &GroupKey) -> [u8; 32] {
    GroupSecretParams::derive_from_master_key(GroupMasterKey::new(key.0)).get_group_identifier()
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        "attachment".into()
    } else {
        cleaned.chars().take(100).collect()
    }
}

/// Picks a unique, monotonically increasing send timestamp.
pub fn next_timestamp(last: Millis, now: Millis) -> Millis {
    if now > last { now } else { last + 1 }
}

impl Worker {
    async fn open(config: BackendConfig, events: EventSink) -> anyhow::Result<Self> {
        config.paths.ensure()?;
        let store = SqliteStore::open_with_passphrase(
            &config.paths.protocol_db().to_string_lossy(),
            Some(&config.db_key),
            // like Signal Desktop: accept changed safety numbers (and mark them in the UI later)
            OnNewIdentity::Trust,
        )
        .await?;
        let db = Database::open(config.paths.app_db(), Some(&config.db_key))?;
        let (internal_tx, internal_rx) = mpsc::unbounded_channel();
        Ok(Self {
            config,
            store,
            db,
            events,
            manager: None,
            me: None,
            group_ids: HashMap::new(),
            receive_task: None,
            link_task: None,
            internal_tx,
            internal_rx: Some(internal_rx),
            last_sent_at: 0,
            profile_requested: HashSet::new(),
            read_receipts: true,
            avatar_requested: HashSet::new(),
        })
    }

    fn emit(&self, ev: BackendEvent) {
        (self.events)(ev);
    }

    fn emit_changes(&self, mut changes: Changes) {
        // only ever delete our own (encrypted) attachment files, never user originals
        let store = self.config.paths.attachments_dir();
        for f in std::mem::take(&mut changes.removed_files) {
            let path = PathBuf::from(f);
            if path.starts_with(&store) {
                let _ = std::fs::remove_file(path);
            }
        }
        for ev in changes.into_events() {
            self.emit(ev);
        }
    }

    fn error(&self, context: &str, e: impl std::fmt::Display) {
        tracing::error!(%e, "{context}");
        self.emit(BackendEvent::Error(format!("{context}: {e}")));
    }

    fn apply(&mut self, m: Mutation) -> Changes {
        match self.db.apply(m) {
            Ok(ch) => ch,
            Err(e) => {
                self.error("Database error", e);
                Changes::default()
            }
        }
    }

    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<BackendCommand>) {
        let mut internal_rx = self.internal_rx.take().expect("internal rx");
        self.startup().await;
        let mut sweep = tokio::time::interval(Duration::from_secs(1));
        sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = sweep.tick() => {
                    let ch = self.apply(Mutation::ExpireDue { now: now_millis() });
                    if !ch.is_empty() {
                        self.emit_changes(ch);
                    }
                }
                cmd = rx.recv() => match cmd {
                    None | Some(BackendCommand::Shutdown) => break,
                    Some(cmd) => self.handle_command(cmd).await,
                },
                Some(msg) = internal_rx.recv() => self.handle_internal(msg).await,
            }
        }
        if let Some(t) = self.receive_task.take() {
            t.abort();
        }
        tracing::info!("backend stopped");
    }

    async fn startup(&mut self) {
        if !self.store.is_registered().await {
            self.emit(BackendEvent::Account(AccountState::Unlinked));
            return;
        }
        match Manager::load_registered(self.store.clone()).await {
            Ok(manager) => self.on_registered(manager, false).await,
            Err(e) => {
                tracing::error!(%e, "failed to load registration");
                self.emit(BackendEvent::Account(AccountState::LinkFailed(format!(
                    "This device is no longer linked ({e}). Please link it again."
                ))));
            }
        }
    }

    async fn on_registered(&mut self, mut manager: Mgr, fresh_link: bool) {
        let data = manager.registration_data().clone();
        let info = AccountInfo {
            aci: data.service_ids.aci,
            phone_number: Some(data.phone_number.to_string()),
            device_id: u32::from(manager.device_id()),
            device_name: data.device_name.clone(),
        };
        self.me = Some(info.aci);
        if fresh_link && let Err(e) = manager.request_contacts().await {
            tracing::warn!(%e, "failed to request contacts sync");
        }
        self.manager = Some(manager);
        self.sync_contacts_and_groups().await;
        self.emit(BackendEvent::Account(AccountState::Registered(info)));
        self.start_receiving();
    }

    fn start_receiving(&mut self) {
        if let Some(t) = self.receive_task.take() {
            t.abort();
        }
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let tx = self.internal_tx.clone();
        self.receive_task = Some(spawn_local(receive_loop(manager, tx)));
    }

    async fn sync_contacts_and_groups(&mut self) {
        let mut mutations = Vec::new();
        match self.store.contacts().await {
            Ok(iter) => {
                for c in iter.flatten() {
                    mutations.push(Mutation::UpsertContact(Contact {
                        uuid: c.uuid,
                        name: Some(c.name).filter(|n| !n.trim().is_empty()),
                        profile_name: None,
                        phone: c.phone_number.map(|p| p.to_string()),
                    }));
                }
            }
            Err(e) => tracing::warn!(%e, "failed to read contacts"),
        }
        let synced: Vec<Uuid> = mutations
            .iter()
            .filter_map(|m| match m {
                Mutation::UpsertContact(c) => Some(c.uuid),
                _ => None,
            })
            .collect();
        mutations.push(Mutation::ContactsSynced { uuids: synced });
        match self.store.groups().await {
            Ok(iter) => {
                for (key, g) in iter.flatten() {
                    let gk = GroupKey(key);
                    self.group_ids.insert(group_identifier(&gk), gk);
                    mutations.push(Mutation::UpsertGroup {
                        key: gk,
                        title: g.title,
                        members: g.members.iter().map(|m| m.aci.into()).collect(),
                    });
                }
            }
            Err(e) => tracing::warn!(%e, "failed to read groups"),
        }
        match self.db.apply_all(mutations) {
            Ok(ch) => self.emit_changes(ch),
            Err(e) => self.error("Failed to save contacts", e),
        }
        self.emit(BackendEvent::ContactsChanged);
        // photos for the chats in the list (most recent first) and yourself
        let mut wanted: Vec<ConversationId> = self
            .db
            .conversations()
            .unwrap_or_default()
            .into_iter()
            .take(200)
            .map(|c| c.id)
            .collect();
        if let Some(me) = self.me {
            wanted.insert(0, ConversationId::Direct(me));
        }
        self.fetch_avatars(wanted);
    }

    /// Syncs a group's title and members from presage's store (which presage
    /// keeps up to date from group changes), emitting only real changes.
    async fn ensure_group(&mut self, key: GroupKey) {
        self.group_ids.entry(group_identifier(&key)).or_insert(key);
        let Ok(Some(g)) = self.store.group(key.0).await else {
            return;
        };
        let members: Vec<Uuid> = g.members.iter().map(|m| m.aci.into()).collect();
        // same normalisation as the database applies
        let title = if g.title.trim().is_empty() {
            "Unnamed group".to_string()
        } else {
            g.title
        };
        let current = self
            .db
            .conversation(&ConversationId::Group(key))
            .ok()
            .flatten();
        if current.is_some_and(|c| c.title == title && c.members == members) {
            return;
        }
        let ch = self.apply(Mutation::UpsertGroup {
            key,
            title,
            members,
        });
        self.emit_changes(ch);
    }

    /// Fetches the profile name of a sender we have no name for.
    async fn ensure_profile(&mut self, uuid: Uuid) {
        if Some(uuid) == self.me || !self.profile_requested.insert(uuid) {
            return;
        }
        if matches!(self.db.contact(&uuid), Ok(Some(c)) if c.name.is_some() || c.profile_name.is_some())
        {
            return;
        }
        let Some(mut manager) = self.manager.clone() else {
            return;
        };
        let sid = ServiceId::Aci(Aci::from(uuid));
        let Ok(Some(key)) = self.store.profile_key(&sid).await else {
            return;
        };
        let tx = self.internal_tx.clone();
        spawn_local(async move {
            let name = match manager.retrieve_profile_by_uuid(Aci::from(uuid), key).await {
                Ok(p) => p.name.map(|n| match n.family_name {
                    Some(f) if !f.is_empty() => format!("{} {f}", n.given_name),
                    _ => n.given_name,
                }),
                Err(e) => {
                    tracing::debug!(%e, "profile fetch failed");
                    None
                }
            };
            let _ = tx.send(Internal::Profile { uuid, name });
        });
    }

    async fn handle_internal(&mut self, msg: Internal) {
        match msg {
            Internal::LinkUrl(url) => self.emit(BackendEvent::Account(AccountState::Linking {
                url: Some(url),
            })),
            Internal::Linked(Ok(manager)) => {
                self.link_task = None;
                self.on_registered(*manager, true).await;
            }
            Internal::Linked(Err(e)) => {
                self.link_task = None;
                self.emit(BackendEvent::Account(AccountState::LinkFailed(e)));
            }
            Internal::Connection(c) => self.emit(BackendEvent::Connection(c)),
            Internal::Received(r) => self.handle_received(r).await,
            Internal::SendResult {
                conversation,
                sent_at,
                ok,
                error,
            } => {
                let status = if ok {
                    MessageStatus::Sent
                } else {
                    MessageStatus::Failed
                };
                let ch = self.apply(Mutation::SendStatus {
                    conversation,
                    sent_at,
                    status,
                });
                self.emit_changes(ch);
                if let Some(e) = error {
                    self.emit(BackendEvent::Error(format!("Message not sent: {e}")));
                }
            }
            Internal::AttachmentDownloaded {
                message_id,
                index,
                path,
            } => {
                if let Some(path) = path {
                    let ch = self.apply(Mutation::AttachmentDownloaded {
                        message_id,
                        index,
                        local_path: path.to_string_lossy().into_owned(),
                    });
                    self.emit_changes(ch);
                }
            }
            Internal::Error(e) => self.emit(BackendEvent::Error(e)),
            Internal::Avatar {
                conversation,
                bytes,
                about,
            } => {
                let mut changes = Changes::default();
                if let Some(bytes) = bytes.filter(|b| !b.is_empty()) {
                    let store = self.config.paths.attachment_store(&self.config.db_key);
                    let name = format!(
                        "avatar-{}-{}",
                        sanitize_file_name(&conversation.to_string()),
                        now_millis()
                    );
                    match store.write(&name, &bytes) {
                        Ok(path) => changes.extend(self.apply(Mutation::SetAvatar {
                            conversation,
                            path: Some(path.to_string_lossy().into_owned()),
                        })),
                        Err(e) => tracing::warn!(%e, "failed to save avatar"),
                    }
                }
                if about.is_some() {
                    changes.extend(self.apply(Mutation::SetAbout {
                        conversation,
                        about,
                    }));
                }
                self.emit_changes(changes);
            }
            Internal::StoryAttachmentDownloaded { story_id, path } => {
                if let Some(path) = path {
                    let ch = self.apply(Mutation::StoryAttachmentDownloaded {
                        id: story_id,
                        local_path: path.to_string_lossy().into_owned(),
                    });
                    self.emit_changes(ch);
                }
            }
            Internal::Profile { uuid, name } => {
                if let Some(name) = name.filter(|n| !n.trim().is_empty()) {
                    let ch = self.apply(Mutation::UpsertContact(Contact {
                        uuid,
                        name: None,
                        profile_name: Some(name),
                        phone: None,
                    }));
                    self.emit_changes(ch);
                }
            }
        }
    }

    async fn handle_received(&mut self, received: Received) {
        match received {
            Received::QueueEmpty => self.emit(BackendEvent::Connection(ConnectionState::Online)),
            Received::Contacts => self.sync_contacts_and_groups().await,
            Received::DecryptionError(sender) => {
                tracing::warn!(sender = %sender.service_id_string(), "failed to decrypt message");
            }
            Received::Content(content) => {
                let Some(me) = self.me else { return };
                let groups = self.group_ids.clone();
                let resolve = move |id: &[u8]| -> Option<GroupKey> {
                    let id: [u8; 32] = id.try_into().ok()?;
                    groups.get(&id).copied()
                };
                let outputs = interpret::interpret(
                    &content,
                    &Ctx {
                        me,
                        received_at: now_millis(),
                        group_by_id: &resolve,
                    },
                );
                for out in outputs {
                    match out {
                        Output::GroupSeen(key) => self.ensure_group(key).await,
                        Output::Typing {
                            conversation,
                            who,
                            started,
                        } => self.emit(BackendEvent::Typing {
                            conversation,
                            who,
                            started,
                        }),
                        Output::Mutation(m) => {
                            let author = match &m {
                                Mutation::InsertMessage(n) if !n.outgoing => Some(n.author),
                                _ => None,
                            };
                            let is_story = matches!(m, Mutation::InsertStory(_));
                            let changes = self.apply(m);
                            if is_story && changes.stories_changed {
                                self.download_story_attachments();
                            }
                            for msg in &changes.messages {
                                if !msg.outgoing {
                                    self.download_attachments(msg.id, &msg.attachments);
                                }
                            }
                            self.emit_changes(changes);
                            if let Some(author) = author {
                                self.ensure_profile(author).await;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Fetches photos (and about texts) of conversations one after another.
    fn fetch_avatars(&mut self, conversations: Vec<ConversationId>) {
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let todo: Vec<ConversationId> = conversations
            .into_iter()
            .filter(|c| self.avatar_requested.insert(*c))
            .collect();
        if todo.is_empty() {
            return;
        }
        let store = self.store.clone();
        let tx = self.internal_tx.clone();
        spawn_local(async move {
            let mut manager = manager;
            for conversation in todo {
                let (bytes, about) = match conversation {
                    ConversationId::Direct(uuid) => {
                        let sid = ServiceId::Aci(Aci::from(uuid));
                        match store.profile_key(&sid).await {
                            Ok(Some(key)) => {
                                let about = manager
                                    .retrieve_profile_by_uuid(Aci::from(uuid), key)
                                    .await
                                    .ok()
                                    .and_then(|p| p.about);
                                let bytes = manager
                                    .retrieve_profile_avatar_by_uuid(uuid, key)
                                    .await
                                    .ok()
                                    .flatten();
                                (bytes, about)
                            }
                            _ => (None, None),
                        }
                    }
                    ConversationId::Group(key) => {
                        let group = store.group(key.0).await.ok().flatten();
                        let about = group.as_ref().and_then(|g| g.description.clone());
                        let bytes = match group.filter(|g| !g.avatar.is_empty()) {
                            Some(g) => manager
                                .retrieve_group_avatar(GroupContextV2 {
                                    master_key: Some(key.0.to_vec()),
                                    revision: Some(g.revision),
                                    group_change: None,
                                })
                                .await
                                .ok()
                                .flatten(),
                            None => None,
                        };
                        (bytes, about)
                    }
                };
                if (bytes.is_some() || about.is_some())
                    && tx
                        .send(Internal::Avatar {
                            conversation,
                            bytes,
                            about,
                        })
                        .is_err()
                {
                    return;
                }
            }
        });
    }

    /// Downloads attachments of newly received stories.
    fn download_story_attachments(&self) {
        let Some(manager) = self.manager.clone() else {
            return;
        };
        for story in self.db.stories().unwrap_or_default() {
            let Some(att) = story.attachment.filter(|a| a.local_path.is_none()) else {
                continue;
            };
            let Some(pointer) = interpret::pointer_from_attachment(&att) else {
                continue;
            };
            let (manager, tx) = (manager.clone(), self.internal_tx.clone());
            let store = self.config.paths.attachment_store(&self.config.db_key);
            let file_name = format!(
                "story-{}-{}",
                story.id,
                sanitize_file_name(&att.display_name())
            );
            let story_id = story.id;
            spawn_local(async move {
                let path = match manager.get_attachment(&pointer).await {
                    Ok(bytes) => {
                        tokio::task::spawn_blocking(move || store.write(&file_name, &bytes))
                            .await
                            .ok()
                            .and_then(Result::ok)
                    }
                    Err(e) => {
                        tracing::warn!(%e, "failed to download story");
                        None
                    }
                };
                let _ = tx.send(Internal::StoryAttachmentDownloaded { story_id, path });
            });
        }
    }

    fn download_attachments(&self, message_id: i64, attachments: &[Attachment]) {
        let Some(manager) = self.manager.clone() else {
            return;
        };
        for (index, att) in attachments.iter().enumerate() {
            if att.local_path.is_some() {
                continue;
            }
            let Some(pointer) = interpret::pointer_from_attachment(att) else {
                continue;
            };
            let manager = manager.clone();
            let tx = self.internal_tx.clone();
            let store = self.config.paths.attachment_store(&self.config.db_key);
            let file_name = format!(
                "{message_id}-{index}-{}",
                sanitize_file_name(&att.display_name())
            );
            spawn_local(async move {
                let path = match manager.get_attachment(&pointer).await {
                    Ok(bytes) => {
                        let written =
                            tokio::task::spawn_blocking(move || store.write(&file_name, &bytes))
                                .await;
                        match written {
                            Ok(Ok(path)) => Some(path),
                            Ok(Err(e)) => {
                                tracing::error!(%e, "failed to save attachment");
                                None
                            }
                            Err(e) => {
                                tracing::error!(%e, "attachment writer panicked");
                                None
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(%e, "failed to download attachment");
                        None
                    }
                };
                let _ = tx.send(Internal::AttachmentDownloaded {
                    message_id,
                    index,
                    path,
                });
            });
        }
    }

    async fn handle_command(&mut self, cmd: BackendCommand) {
        match cmd {
            BackendCommand::Link { device_name } => self.start_linking(device_name),
            BackendCommand::SetExpireTimer { conversation, secs } => {
                let Some(me) = self.me else { return };
                let at = self.next_ts();
                let ch = self.apply(Mutation::SetExpireTimer {
                    conversation,
                    secs,
                    author: me,
                    outgoing: true,
                    at,
                });
                self.emit_changes(ch);
                let dm = DataMessage {
                    timestamp: Some(at as u64),
                    flags: Some(data_message::Flags::ExpirationTimerUpdate as u32),
                    expire_timer: Some(secs.unwrap_or(0)),
                    ..Default::default()
                };
                self.spawn_send(conversation, dm, at, false);
            }
            BackendCommand::ReplyToStory {
                author,
                story_sent_at,
                body,
            } => {
                let Some(me) = self.me else { return };
                let conversation = ConversationId::Direct(author);
                let at = self.next_ts();
                let mut m = NewMessage::text(conversation, me, at, true, body.clone());
                m.status = MessageStatus::Sending;
                m.quote = Some(Quote {
                    author,
                    sent_at: story_sent_at,
                    text: signal_core::STORY_REPLY_QUOTE.into(),
                });
                let ch = self.apply(Mutation::InsertMessage(m));
                self.emit_changes(ch);
                let Some(manager) = self.manager.clone() else {
                    return;
                };
                let tx = self.internal_tx.clone();
                let dm = DataMessage {
                    body: Some(body),
                    timestamp: Some(at as u64),
                    story_context: Some(data_message::StoryContext {
                        author_aci: Some(author.to_string()),
                        sent_timestamp: Some(story_sent_at as u64),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                spawn_local(async move {
                    let result = deliver(manager, conversation, dm, at).await;
                    let _ = tx.send(Internal::SendResult {
                        conversation,
                        sent_at: at,
                        ok: result.is_ok(),
                        error: result.err(),
                    });
                });
            }
            BackendCommand::Forward {
                targets,
                body,
                attachments,
            } => {
                for target in targets {
                    self.send_text(target, body.clone(), None, attachments.clone(), None);
                }
            }
            BackendCommand::SendText {
                conversation,
                body,
                quote,
                attachments,
            } => self.send_text(conversation, body, quote, attachments, None),
            BackendCommand::Resend {
                conversation,
                sent_at,
            } => {
                let Some(me) = self.me else { return };
                if let Ok(Some(msg)) = self.db.message_by_key(&conversation, &me, sent_at) {
                    let paths = msg
                        .attachments
                        .iter()
                        .filter_map(|a| a.local_path.as_ref().map(PathBuf::from))
                        .collect();
                    self.send_text(conversation, msg.body, msg.quote, paths, Some(sent_at));
                }
            }
            BackendCommand::React {
                conversation,
                target_author,
                target_sent_at,
                emoji,
                remove,
            } => {
                let Some(me) = self.me else { return };
                let at = self.next_ts();
                let ch = self.apply(Mutation::React {
                    conversation,
                    target_author,
                    target_sent_at,
                    reactor: me,
                    emoji: emoji.clone(),
                    remove,
                    at,
                });
                self.emit_changes(ch);
                let dm = DataMessage {
                    timestamp: Some(at as u64),
                    reaction: Some(data_message::Reaction {
                        emoji: Some(emoji),
                        remove: Some(remove),
                        target_author_aci: Some(target_author.to_string()),
                        target_sent_timestamp: Some(target_sent_at as u64),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                self.spawn_send(conversation, dm, at, false);
            }
            BackendCommand::DeleteForEveryone {
                conversation,
                sent_at,
            } => {
                let Some(me) = self.me else { return };
                let ch = self.apply(Mutation::DeleteForEveryone {
                    conversation,
                    author: me,
                    target_sent_at: sent_at,
                });
                self.emit_changes(ch);
                let at = self.next_ts();
                let dm = DataMessage {
                    timestamp: Some(at as u64),
                    delete: Some(data_message::Delete {
                        target_sent_timestamp: Some(sent_at as u64),
                    }),
                    ..Default::default()
                };
                self.spawn_send(conversation, dm, at, false);
            }
            BackendCommand::Edit {
                conversation,
                target_sent_at,
                body,
            } => {
                let Some(me) = self.me else { return };
                let body = body.trim_end().to_string();
                if body.trim().is_empty() {
                    return;
                }
                let at = self.next_ts();
                let ch = self.apply(Mutation::Edit {
                    conversation,
                    author: me,
                    target_sent_at,
                    body: body.clone(),
                    edited_at: at,
                });
                self.emit_changes(ch);
                let Some(manager) = self.manager.clone() else {
                    return;
                };
                let tx = self.internal_tx.clone();
                spawn_local(async move {
                    let dm = DataMessage {
                        body: Some(body),
                        timestamp: Some(at as u64),
                        ..Default::default()
                    };
                    if let Err(e) =
                        deliver_edit(manager, conversation, target_sent_at, dm, at).await
                    {
                        tracing::warn!(%e, "failed to send edit");
                        let _ = tx.send(Internal::Error(format!("Edit not sent: {e}")));
                    }
                });
            }
            BackendCommand::MarkRead { conversation } => self.mark_read(conversation),
            BackendCommand::SetReadReceipts(on) => self.read_receipts = on,
            BackendCommand::FetchAvatar { conversation } => self.fetch_avatars(vec![conversation]),
            BackendCommand::UpdateProfile {
                given_name,
                family_name,
                about,
            } => {
                let (Some(mut manager), Some(me)) = (self.manager.clone(), self.me) else {
                    return;
                };
                let tx = self.internal_tx.clone();
                spawn_local(async move {
                    let name = presage::libsignal_service::profile_name::ProfileName {
                        given_name: given_name.clone(),
                        family_name: family_name.clone().filter(|f| !f.trim().is_empty()),
                    };
                    match manager.update_profile(name, about.clone(), None).await {
                        Ok(()) => {
                            let _ = tx.send(Internal::Profile {
                                uuid: me,
                                name: Some(match family_name.filter(|f| !f.trim().is_empty()) {
                                    Some(f) => format!("{given_name} {f}"),
                                    None => given_name,
                                }),
                            });
                            let _ = tx.send(Internal::Avatar {
                                conversation: ConversationId::Direct(me),
                                bytes: None,
                                about: Some(about.unwrap_or_default()),
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(Internal::Error(format!(
                                "Couldn't update your profile: {e}"
                            )));
                        }
                    }
                });
            }
            BackendCommand::RequestSafetyNumber { contact } => {
                let number = match self.me {
                    Some(me) => {
                        use presage::libsignal_service::session_store::SessionStoreExt as _;
                        self.store
                            .aci_protocol_store()
                            .compute_safety_number(
                                &ServiceId::Aci(Aci::from(me)),
                                &ServiceId::Aci(Aci::from(contact)),
                            )
                            .await
                            .map_err(|e| tracing::info!(%e, "no safety number yet"))
                            .ok()
                    }
                    None => None,
                };
                self.emit(BackendEvent::SafetyNumber { contact, number });
            }
            BackendCommand::Typing {
                conversation,
                started,
            } => self.send_typing(conversation, started),
            BackendCommand::SyncContacts => {
                if let Some(mut m) = self.manager.clone() {
                    spawn_local(async move {
                        if let Err(e) = m.request_contacts().await {
                            tracing::warn!(%e, "contact sync request failed");
                        }
                    });
                }
            }
            BackendCommand::Unlink => self.unlink().await,
            BackendCommand::Shutdown => {}
        }
    }

    fn next_ts(&mut self) -> Millis {
        self.last_sent_at = next_timestamp(self.last_sent_at, now_millis());
        self.last_sent_at
    }

    fn start_linking(&mut self, device_name: String) {
        if self.manager.is_some() || self.link_task.is_some() {
            return;
        }
        self.emit(BackendEvent::Account(AccountState::Linking { url: None }));
        let store = self.store.clone();
        let servers = self.config.servers;
        let tx = self.internal_tx.clone();
        self.link_task = Some(spawn_local(async move {
            let (url_tx, url_rx) = oneshot::channel::<url::Url>();
            let url_fwd = {
                let tx = tx.clone();
                async move {
                    if let Ok(url) = url_rx.await {
                        let _ = tx.send(Internal::LinkUrl(url.to_string()));
                    }
                }
            };
            let (result, ()) = futures::join!(
                Manager::link_secondary_device(store, servers, device_name, url_tx),
                url_fwd
            );
            let _ = tx.send(Internal::Linked(
                result.map(Box::new).map_err(|e| e.to_string()),
            ));
        }));
    }

    fn send_text(
        &mut self,
        conversation: ConversationId,
        body: String,
        quote: Option<Quote>,
        attachment_paths: Vec<PathBuf>,
        resend_ts: Option<Millis>,
    ) {
        let Some(me) = self.me else {
            self.emit(BackendEvent::Error("Not linked".into()));
            return;
        };
        let body = body.trim_end().to_string();
        if body.trim().is_empty() && attachment_paths.is_empty() {
            return;
        }
        let timer = self
            .db
            .conversation(&conversation)
            .ok()
            .flatten()
            .and_then(|c| c.expire_timer);
        let sent_at = match resend_ts {
            Some(ts) => ts,
            None => {
                let ts = self.next_ts();
                let attachments = attachment_paths
                    .iter()
                    .map(|p| local_attachment(p))
                    .collect();
                let ch = self.apply(Mutation::InsertMessage(NewMessage {
                    conversation,
                    author: me,
                    sent_at: ts,
                    received_at: ts,
                    outgoing: true,
                    body: body.clone(),
                    attachments,
                    quote: quote.clone(),
                    status: MessageStatus::Sending,
                    read: true,
                    expires_in_secs: timer,
                    kind: MessageKind::Normal,
                    body_ranges: vec![],
                    link_preview: None,
                }));
                self.emit_changes(ch);
                ts
            }
        };
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let tx = self.internal_tx.clone();
        spawn_local(async move {
            let result = async {
                let pointers = upload(&manager, &attachment_paths).await?;
                let dm = DataMessage {
                    body: Some(body).filter(|b| !b.is_empty()),
                    timestamp: Some(sent_at as u64),
                    expire_timer: Some(timer.unwrap_or(0)),
                    attachments: pointers,
                    quote: quote.map(|q| data_message::Quote {
                        id: Some(q.sent_at as u64),
                        author_aci: Some(q.author.to_string()),
                        text: Some(q.text),
                        r#type: Some(data_message::quote::Type::Normal as i32),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                deliver(manager, conversation, dm, sent_at).await
            }
            .await;
            let _ = tx.send(Internal::SendResult {
                conversation,
                sent_at,
                ok: result.is_ok(),
                error: result.err(),
            });
        });
    }

    /// Sends a control message (reaction, deletion) in the background.
    fn spawn_send(&self, conversation: ConversationId, dm: DataMessage, at: Millis, report: bool) {
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let tx = self.internal_tx.clone();
        spawn_local(async move {
            if let Err(e) = deliver(manager, conversation, dm, at).await {
                tracing::warn!(%e, "failed to send control message");
                if report {
                    let _ = tx.send(Internal::SendResult {
                        conversation,
                        sent_at: at,
                        ok: false,
                        error: Some(e),
                    });
                }
            }
        });
    }

    fn mark_read(&mut self, conversation: ConversationId) {
        let unread = self.db.unread_messages(&conversation).unwrap_or_default();
        let ch = self.apply(Mutation::MarkConversationRead { conversation });
        self.emit_changes(ch);
        if unread.is_empty() {
            return;
        }
        let (Some(manager), Some(me)) = (self.manager.clone(), self.me) else {
            return;
        };
        // no receipts while the user hasn't accepted a message request
        let accepted = self
            .db
            .conversation(&conversation)
            .ok()
            .flatten()
            .is_none_or(|c| c.accepted);
        let send_receipts = self.read_receipts && accepted;
        let mut by_author: HashMap<Uuid, Vec<u64>> = HashMap::new();
        for m in &unread {
            by_author
                .entry(m.author)
                .or_default()
                .push(m.sent_at as u64);
        }
        let now = self.next_ts() as u64;
        spawn_local(async move {
            let mut manager = manager;
            for (author, timestamps) in by_author.iter().filter(|_| send_receipts) {
                let receipt = ReceiptMessage {
                    r#type: Some(receipt_message::Type::Read as i32),
                    timestamp: timestamps.clone(),
                };
                if let Err(e) = manager
                    .send_message(
                        ServiceId::Aci(Aci::from(*author)),
                        ContentBody::ReceiptMessage(receipt),
                        now,
                    )
                    .await
                {
                    tracing::debug!(%e, "failed to send read receipt");
                }
            }
            // let our other devices know too
            let sync = SyncMessage {
                read: by_author
                    .iter()
                    .flat_map(|(a, ts)| {
                        ts.iter().map(move |t| sync_message::Read {
                            sender_aci: Some(a.to_string()),
                            timestamp: Some(*t),
                            ..Default::default()
                        })
                    })
                    .collect(),
                ..Default::default()
            };
            if let Err(e) = manager
                .send_message(
                    ServiceId::Aci(Aci::from(me)),
                    ContentBody::SynchronizeMessage(sync),
                    now,
                )
                .await
            {
                tracing::debug!(%e, "failed to send read sync");
            }
        });
    }

    fn send_typing(&mut self, conversation: ConversationId, started: bool) {
        let Some(mut manager) = self.manager.clone() else {
            return;
        };
        let ts = now_millis() as u64;
        let action = if started {
            typing_message::Action::Started
        } else {
            typing_message::Action::Stopped
        };
        spawn_local(async move {
            let result = match conversation {
                ConversationId::Direct(uuid) => {
                    let msg = TypingMessage {
                        timestamp: Some(ts),
                        action: Some(action as i32),
                        group_id: None,
                    };
                    manager
                        .send_message(
                            ServiceId::Aci(Aci::from(uuid)),
                            ContentBody::TypingMessage(msg),
                            ts,
                        )
                        .await
                }
                ConversationId::Group(key) => {
                    let msg = TypingMessage {
                        timestamp: Some(ts),
                        action: Some(action as i32),
                        group_id: Some(group_identifier(&key).to_vec()),
                    };
                    manager
                        .send_message_to_group(&key.0, ContentBody::TypingMessage(msg), ts)
                        .await
                }
            };
            if let Err(e) = result {
                tracing::debug!(%e, "failed to send typing indicator");
            }
        });
    }

    async fn unlink(&mut self) {
        if let Some(t) = self.receive_task.take() {
            t.abort();
        }
        if let Some(t) = self.link_task.take() {
            t.abort();
        }
        if let Some(manager) = self.manager.take()
            && let Err(e) = manager.unlink_self().await
        {
            tracing::warn!(%e, "failed to unlink on the server; wiping local data anyway");
        }
        if let Err(e) = self.store.clear().await {
            self.error("Failed to clear protocol store", e);
        }
        if let Err(e) = self.db.wipe() {
            self.error("Failed to clear messages", e);
        }
        let _ = std::fs::remove_dir_all(self.config.paths.attachments_dir());
        let _ = std::fs::remove_dir_all(self.config.paths.open_cache_dir());
        let _ = self.config.paths.ensure();
        self.me = None;
        self.group_ids.clear();
        self.profile_requested.clear();
        self.emit(BackendEvent::Connection(ConnectionState::Disconnected));
        self.emit(BackendEvent::Account(AccountState::Unlinked));
    }
}

fn local_attachment(path: &Path) -> Attachment {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or_default();
    Attachment {
        content_type: mime_guess::from_path(path)
            .first_or_octet_stream()
            .essence_str()
            .to_string(),
        file_name: path.file_name().map(|n| n.to_string_lossy().into_owned()),
        size,
        local_path: Some(path.to_string_lossy().into_owned()),
        width: None,
        height: None,
        pointer: None,
    }
}

/// Maximum attachment size accepted by Signal servers (100 MiB).
pub const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;

async fn upload(
    manager: &Mgr,
    paths: &[PathBuf],
) -> Result<Vec<presage::libsignal_service::proto::AttachmentPointer>, String> {
    let mut specs = Vec::with_capacity(paths.len());
    for path in paths {
        let meta = tokio::fs::metadata(path)
            .await
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.len() > MAX_ATTACHMENT_BYTES {
            return Err(format!("{} is larger than 100 MB", path.display()));
        }
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let a = local_attachment(path);
        let voice_note = a
            .file_name
            .as_deref()
            .is_some_and(|n| n.starts_with(signal_core::VOICE_NOTE_PREFIX));
        specs.push((
            AttachmentSpec {
                content_type: a.content_type,
                length: bytes.len(),
                file_name: a.file_name,
                preview: None,
                voice_note: voice_note.then_some(true),
                borderless: None,
                width: None,
                height: None,
                caption: None,
                blur_hash: None,
            },
            bytes,
        ));
    }
    let results = manager
        .upload_attachments(specs)
        .await
        .map_err(|e| e.to_string())?;
    results
        .into_iter()
        .map(|r| r.map_err(|e| format!("upload failed: {e}")))
        .collect()
}

async fn deliver(
    mut manager: Mgr,
    conversation: ConversationId,
    mut dm: DataMessage,
    ts: Millis,
) -> Result<(), String> {
    let ts = ts as u64;
    match conversation {
        ConversationId::Direct(uuid) => manager
            .send_message(ServiceId::Aci(Aci::from(uuid)), dm, ts)
            .await
            .map_err(|e| e.to_string()),
        ConversationId::Group(key) => {
            let group_revision = match manager.store().group(key.0).await {
                Ok(Some(g)) => g.revision,
                _ => 0,
            };
            dm.group_v2 = Some(GroupContextV2 {
                master_key: Some(key.0.to_vec()),
                revision: Some(group_revision),
                group_change: None,
            });
            manager
                .send_message_to_group(&key.0, dm, ts)
                .await
                .map_err(|e| e.to_string())
        }
    }
}

async fn group_context(manager: &Mgr, key: &GroupKey) -> GroupContextV2 {
    let revision = match manager.store().group(key.0).await {
        Ok(Some(g)) => g.revision,
        _ => 0,
    };
    GroupContextV2 {
        master_key: Some(key.0.to_vec()),
        revision: Some(revision),
        group_change: None,
    }
}

async fn deliver_edit(
    mut manager: Mgr,
    conversation: ConversationId,
    target_sent_at: Millis,
    mut dm: DataMessage,
    ts: Millis,
) -> Result<(), String> {
    if let ConversationId::Group(key) = &conversation {
        dm.group_v2 = Some(group_context(&manager, key).await);
    }
    let edit = ContentBody::EditMessage(EditMessage {
        target_sent_timestamp: Some(target_sent_at as u64),
        data_message: Some(dm),
    });
    let ts = ts as u64;
    match conversation {
        ConversationId::Direct(uuid) => {
            manager
                .send_message(ServiceId::Aci(Aci::from(uuid)), edit, ts)
                .await
        }
        ConversationId::Group(key) => manager.send_message_to_group(&key.0, edit, ts).await,
    }
    .map_err(|e| e.to_string())
}

/// Keeps a websocket open, reconnecting with exponential backoff.
async fn receive_loop(mut manager: Mgr, tx: mpsc::UnboundedSender<Internal>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let _ = tx.send(Internal::Connection(ConnectionState::Connecting));
        match manager.receive_messages().await {
            Ok(stream) => {
                let _ = tx.send(Internal::Connection(ConnectionState::Syncing));
                futures::pin_mut!(stream);
                while let Some(item) = stream.next().await {
                    if matches!(item, Received::QueueEmpty) {
                        backoff = Duration::from_secs(1);
                    }
                    if tx.send(Internal::Received(item)).is_err() {
                        return;
                    }
                }
                tracing::info!("message stream closed");
                let _ = tx.send(Internal::Connection(ConnectionState::Offline {
                    reason: "Connection closed".into(),
                    retry_in_secs: backoff.as_secs(),
                }));
            }
            Err(e) => {
                tracing::warn!(%e, "failed to open message stream");
                let _ = tx.send(Internal::Connection(ConnectionState::Offline {
                    reason: e.to_string(),
                    retry_in_secs: backoff.as_secs(),
                }));
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_strictly_increasing() {
        assert_eq!(next_timestamp(0, 100), 100);
        assert_eq!(next_timestamp(100, 100), 101);
        assert_eq!(next_timestamp(150, 100), 151);
    }

    #[test]
    fn sanitizes_attachment_names() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_file_name("..."), "attachment");
        assert_eq!(sanitize_file_name("photo 1.jpg"), "photo_1.jpg");
        assert_eq!(sanitize_file_name(&"a".repeat(300)).len(), 100);
    }

    #[test]
    fn group_identifier_is_stable() {
        let k = GroupKey([5; 32]);
        assert_eq!(group_identifier(&k), group_identifier(&k));
        assert_ne!(group_identifier(&k), group_identifier(&GroupKey([6; 32])));
        assert_ne!(group_identifier(&k), k.0);
    }

    #[test]
    fn local_attachment_guesses_mime() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("pic.png");
        std::fs::write(&p, [0u8; 10]).unwrap();
        let a = local_attachment(&p);
        assert_eq!(a.content_type, "image/png");
        assert_eq!(a.size, 10);
        assert_eq!(a.file_name.as_deref(), Some("pic.png"));
    }
}
