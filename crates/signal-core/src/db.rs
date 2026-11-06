//! Local, encrypted (SQLCipher) message database.
//!
//! The Signal protocol state itself (keys, sessions) lives in presage's store;
//! this database is the client-side index of conversations and messages that
//! drives the UI, similar to Signal Desktop's `db.sqlite`.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use crate::ids::ConversationId;
use crate::model::{
    Contact, Conversation, Message, MessageKind, MessageStatus, Millis, Reaction, Revision,
    now_millis,
};
use crate::protocol::{Changes, Mutation, NewMessage};

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization: {0}")]
    Json(#[from] serde_json::Error),
    #[error("database is encrypted with a different key or corrupted")]
    BadKey,
    #[error("database schema version {0} is newer than supported ({SCHEMA_VERSION})")]
    TooNew(i64),
}

pub type Result<T> = std::result::Result<T, DbError>;

pub const SCHEMA_VERSION: i64 = 2;

const MIGRATIONS: &[&str] = &[
    // v1
    r#"
    CREATE TABLE contacts (
        uuid TEXT PRIMARY KEY NOT NULL,
        name TEXT,
        profile_name TEXT,
        phone TEXT
    );
    CREATE TABLE conversations (
        id TEXT PRIMARY KEY NOT NULL,
        title TEXT NOT NULL,
        last_activity INTEGER NOT NULL DEFAULT 0,
        last_preview TEXT NOT NULL DEFAULT '',
        last_preview_outgoing INTEGER NOT NULL DEFAULT 0,
        unread_count INTEGER NOT NULL DEFAULT 0,
        pinned INTEGER NOT NULL DEFAULT 0,
        archived INTEGER NOT NULL DEFAULT 0,
        muted INTEGER NOT NULL DEFAULT 0,
        draft TEXT,
        members TEXT NOT NULL DEFAULT '[]'
    );
    CREATE TABLE messages (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
        author TEXT NOT NULL,
        sent_at INTEGER NOT NULL,
        received_at INTEGER NOT NULL,
        outgoing INTEGER NOT NULL,
        body TEXT NOT NULL DEFAULT '',
        attachments TEXT NOT NULL DEFAULT '[]',
        quote TEXT,
        reactions TEXT NOT NULL DEFAULT '[]',
        status INTEGER NOT NULL,
        read INTEGER NOT NULL DEFAULT 0,
        edited_at INTEGER,
        deleted INTEGER NOT NULL DEFAULT 0,
        expires_in INTEGER,
        UNIQUE(conversation_id, author, sent_at)
    );
    CREATE INDEX messages_by_conversation ON messages(conversation_id, sent_at, id);
    CREATE INDEX messages_by_sent_at ON messages(sent_at);
    "#,
    // v2: mute expiry, mark unread, disappearing messages, blocking, message
    // requests, notices, formatting and edit history
    r#"
    ALTER TABLE conversations ADD COLUMN muted_until INTEGER NOT NULL DEFAULT 0;
    UPDATE conversations SET muted_until = 9223372036854775807 WHERE muted = 1;
    ALTER TABLE conversations ADD COLUMN marked_unread INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE conversations ADD COLUMN expire_timer INTEGER;
    ALTER TABLE conversations ADD COLUMN blocked INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE conversations ADD COLUMN accepted INTEGER NOT NULL DEFAULT 1;
    ALTER TABLE messages ADD COLUMN expires_at INTEGER;
    ALTER TABLE messages ADD COLUMN kind TEXT NOT NULL DEFAULT '"normal"';
    ALTER TABLE messages ADD COLUMN body_ranges TEXT NOT NULL DEFAULT '[]';
    ALTER TABLE messages ADD COLUMN edit_history TEXT NOT NULL DEFAULT '[]';
    CREATE INDEX messages_by_expiry ON messages(expires_at) WHERE expires_at IS NOT NULL;
    "#,
];

pub struct Database {
    conn: Connection,
}

const MESSAGE_COLUMNS: &str = "id, conversation_id, author, sent_at, received_at, outgoing, body, \
     attachments, quote, reactions, status, read, edited_at, deleted, expires_in, expires_at, kind, \
     body_ranges, edit_history";

const CONVERSATION_COLUMNS: &str = "id, title, last_activity, last_preview, last_preview_outgoing, \
     unread_count, pinned, archived, muted_until, draft, members, marked_unread, expire_timer, \
     blocked, accepted";

fn bool_col(row: &Row, idx: usize) -> rusqlite::Result<bool> {
    Ok(row.get::<_, i64>(idx)? != 0)
}

fn parse_uuid(s: String) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(&s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn parse_conv_id(s: String) -> rusqlite::Result<ConversationId> {
    s.parse::<ConversationId>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn json_col<T: serde::de::DeserializeOwned>(row: &Row, idx: usize) -> rusqlite::Result<T> {
    let s: String = row.get(idx)?;
    serde_json::from_str(&s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn message_from_row(row: &Row) -> rusqlite::Result<Message> {
    let quote: Option<String> = row.get(8)?;
    Ok(Message {
        id: row.get(0)?,
        conversation: parse_conv_id(row.get(1)?)?,
        author: parse_uuid(row.get(2)?)?,
        sent_at: row.get(3)?,
        received_at: row.get(4)?,
        outgoing: bool_col(row, 5)?,
        body: row.get(6)?,
        attachments: json_col(row, 7)?,
        quote: quote.and_then(|q| serde_json::from_str(&q).ok()),
        reactions: json_col(row, 9)?,
        status: MessageStatus::from_i64(row.get(10)?),
        read: bool_col(row, 11)?,
        edited_at: row.get(12)?,
        deleted: bool_col(row, 13)?,
        expires_in_secs: row.get(14)?,
        expires_at: row.get(15)?,
        kind: json_col(row, 16)?,
        body_ranges: json_col(row, 17)?,
        edit_history: json_col(row, 18)?,
    })
}

fn conversation_from_row(row: &Row) -> rusqlite::Result<Conversation> {
    let muted_until = Some(row.get::<_, i64>(8)?).filter(|t| *t > 0);
    Ok(Conversation {
        id: parse_conv_id(row.get(0)?)?,
        title: row.get(1)?,
        last_activity: row.get(2)?,
        last_preview: row.get(3)?,
        last_preview_outgoing: bool_col(row, 4)?,
        unread_count: row.get(5)?,
        pinned: bool_col(row, 6)?,
        archived: bool_col(row, 7)?,
        muted: muted_until.is_some_and(|until| until > crate::model::now_millis()),
        muted_until,
        draft: row.get(9)?,
        members: json_col(row, 10)?,
        marked_unread: bool_col(row, 11)?,
        expire_timer: row.get::<_, Option<u32>>(12)?.filter(|t| *t > 0),
        blocked: bool_col(row, 13)?,
        accepted: bool_col(row, 14)?,
    })
}

/// Escapes `%`, `_` and `\` for use in a `LIKE ... ESCAPE '\'` pattern.
fn like_pattern(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 2);
    out.push('%');
    for c in query.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

impl Database {
    /// Opens (creating if needed) the database at `path`. When `key` is set the
    /// file is encrypted with SQLCipher.
    pub fn open(path: impl AsRef<Path>, key: Option<&str>) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn, key)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, None)
    }

    fn init(conn: Connection, key: Option<&str>) -> Result<Self> {
        if let Some(key) = key {
            conn.pragma_update(None, "key", key)?;
        }
        // Touching the schema validates the key.
        if conn
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| {
                r.get::<_, i64>(0)
            })
            .is_err()
        {
            return Err(DbError::BadKey);
        }
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // WAL lets the UI read while the backend writes. In-memory dbs ignore it.
        let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let mut db = Database { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    fn migrate(&mut self) -> Result<()> {
        let current = self.schema_version()?;
        if current > SCHEMA_VERSION {
            return Err(DbError::TooNew(current));
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
            tracing::info!(version = i + 1, "applied database migration");
        }
        Ok(())
    }

    // ---------------------------------------------------------------- queries

    pub fn conversation(&self, id: &ConversationId) -> Result<Option<Conversation>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE id = ?1"),
                [id.to_string()],
                conversation_from_row,
            )
            .optional()?)
    }

    /// All conversations that have activity (or a draft), most recent first,
    /// pinned conversations on top.
    pub fn conversations(&self) -> Result<Vec<Conversation>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CONVERSATION_COLUMNS} FROM conversations \
             WHERE last_activity > 0 OR draft IS NOT NULL OR pinned = 1 \
             ORDER BY pinned DESC, last_activity DESC, title ASC"
        ))?;
        let rows = stmt.query_map([], conversation_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every known conversation (including contacts never messaged), by title.
    /// Used for the "new message" contact picker.
    pub fn all_conversations(&self) -> Result<Vec<Conversation>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CONVERSATION_COLUMNS} FROM conversations ORDER BY title COLLATE NOCASE ASC"
        ))?;
        let rows = stmt.query_map([], conversation_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn message(&self, id: i64) -> Result<Option<Message>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"),
                [id],
                message_from_row,
            )
            .optional()?)
    }

    pub fn message_by_key(
        &self,
        conversation: &ConversationId,
        author: &Uuid,
        sent_at: Millis,
    ) -> Result<Option<Message>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {MESSAGE_COLUMNS} FROM messages \
                     WHERE conversation_id = ?1 AND author = ?2 AND sent_at = ?3"
                ),
                params![conversation.to_string(), author.to_string(), sent_at],
                message_from_row,
            )
            .optional()?)
    }

    /// Up to `limit` messages before `before` (exclusive: `(sent_at, id)`),
    /// returned in ascending order. `None` loads the newest page.
    pub fn messages(
        &self,
        conversation: &ConversationId,
        before: Option<(Millis, i64)>,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let (sent, id) = before.unwrap_or((i64::MAX, i64::MAX));
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages \
             WHERE conversation_id = ?1 AND (sent_at < ?2 OR (sent_at = ?2 AND id < ?3)) \
             ORDER BY sent_at DESC, id DESC LIMIT ?4"
        ))?;
        let rows = stmt.query_map(
            params![conversation.to_string(), sent, id, limit as i64],
            message_from_row,
        )?;
        let mut out: Vec<Message> = rows.collect::<rusqlite::Result<_>>()?;
        out.reverse();
        Ok(out)
    }

    /// Search within one conversation, newest first.
    pub fn search_in_conversation(
        &self,
        conversation: &ConversationId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages \
             WHERE conversation_id = ?1 AND deleted = 0 AND body LIKE ?2 ESCAPE '\\' \
             ORDER BY sent_at DESC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(
            params![conversation.to_string(), like_pattern(query), limit as i64],
            message_from_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Case-insensitive substring search over message bodies, newest first.
    pub fn search_messages(&self, query: &str, limit: usize) -> Result<Vec<Message>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages \
             WHERE deleted = 0 AND body LIKE ?1 ESCAPE '\\' \
             ORDER BY sent_at DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![like_pattern(query), limit as i64], message_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn contact(&self, uuid: &Uuid) -> Result<Option<Contact>> {
        Ok(self
            .conn
            .query_row(
                "SELECT uuid, name, profile_name, phone FROM contacts WHERE uuid = ?1",
                [uuid.to_string()],
                |r| {
                    Ok(Contact {
                        uuid: parse_uuid(r.get(0)?)?,
                        name: r.get(1)?,
                        profile_name: r.get(2)?,
                        phone: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn contacts(&self) -> Result<Vec<Contact>> {
        let mut stmt = self
            .conn
            .prepare("SELECT uuid, name, profile_name, phone FROM contacts")?;
        let rows = stmt.query_map([], |r| {
            Ok(Contact {
                uuid: parse_uuid(r.get(0)?)?,
                name: r.get(1)?,
                profile_name: r.get(2)?,
                phone: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Display name for an author uuid (falls back to "Unknown contact").
    pub fn display_name(&self, uuid: &Uuid) -> Result<String> {
        Ok(self
            .contact(uuid)?
            .map(|c| c.display_name())
            .unwrap_or_else(|| "Unknown contact".to_string()))
    }

    /// Unread incoming messages of a conversation (for read receipts).
    pub fn unread_messages(&self, conversation: &ConversationId) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages \
             WHERE conversation_id = ?1 AND outgoing = 0 AND read = 0 ORDER BY sent_at"
        ))?;
        let rows = stmt.query_map([conversation.to_string()], message_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ----------------------------------------------------- local UI settings

    pub fn set_draft(&mut self, id: &ConversationId, draft: Option<&str>) -> Result<()> {
        let draft = draft.map(str::trim_end).filter(|d| !d.trim().is_empty());
        self.conn.execute(
            "UPDATE conversations SET draft = ?2 WHERE id = ?1",
            params![id.to_string(), draft],
        )?;
        Ok(())
    }

    pub fn set_pinned(
        &mut self,
        id: &ConversationId,
        pinned: bool,
    ) -> Result<Option<Conversation>> {
        self.set_flag(id, "pinned", pinned)
    }

    pub fn set_archived(
        &mut self,
        id: &ConversationId,
        archived: bool,
    ) -> Result<Option<Conversation>> {
        self.set_flag(id, "archived", archived)
    }

    /// Mutes forever (`true`) or unmutes; see [`Mutation::SetMuted`] for timed mutes.
    pub fn set_muted(&mut self, id: &ConversationId, muted: bool) -> Result<Option<Conversation>> {
        let until = if muted { Some(i64::MAX) } else { None };
        let ch = self.apply(Mutation::SetMuted {
            conversation: *id,
            until,
        })?;
        Ok(ch
            .conversations
            .into_iter()
            .next()
            .or(self.conversation(id)?))
    }

    fn set_flag(
        &mut self,
        id: &ConversationId,
        column: &'static str,
        value: bool,
    ) -> Result<Option<Conversation>> {
        self.conn.execute(
            &format!("UPDATE conversations SET {column} = ?2 WHERE id = ?1"),
            params![id.to_string(), value as i64],
        )?;
        self.conversation(id)
    }

    /// Ensures a conversation row exists (e.g. when composing to a contact).
    pub fn ensure_conversation(&mut self, id: &ConversationId) -> Result<Conversation> {
        if let Some(c) = self.conversation(id)? {
            return Ok(c);
        }
        let title = match id {
            ConversationId::Direct(u) => self.display_name(u)?,
            ConversationId::Group(_) => "Unknown group".to_string(),
        };
        self.conn.execute(
            "INSERT OR IGNORE INTO conversations (id, title) VALUES (?1, ?2)",
            params![id.to_string(), title],
        )?;
        Ok(self.conversation(id)?.expect("conversation just inserted"))
    }

    /// Deletes all local data (used when unlinking).
    pub fn wipe(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM messages; DELETE FROM conversations; DELETE FROM contacts;",
        )?;
        Ok(())
    }

    // -------------------------------------------------------------- mutations

    /// Applies a mutation atomically and returns the changed rows.
    pub fn apply(&mut self, mutation: Mutation) -> Result<Changes> {
        let tx = self.conn.transaction()?;
        let changes = {
            let mut w = Writer { conn: &tx };
            w.apply(mutation)?
        };
        tx.commit()?;
        Ok(changes)
    }

    pub fn apply_all(&mut self, mutations: impl IntoIterator<Item = Mutation>) -> Result<Changes> {
        let mut all = Changes::default();
        for m in mutations {
            all.extend(self.apply(m)?);
        }
        Ok(all)
    }
}

/// Mutation logic operating inside a transaction.
struct Writer<'a> {
    conn: &'a Connection,
}

impl Writer<'_> {
    fn conversation(&self, id: &ConversationId) -> Result<Option<Conversation>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE id = ?1"),
                [id.to_string()],
                conversation_from_row,
            )
            .optional()?)
    }

    fn message_by_key(
        &self,
        conv: &ConversationId,
        author: &Uuid,
        sent_at: Millis,
    ) -> Result<Option<Message>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {MESSAGE_COLUMNS} FROM messages \
                     WHERE conversation_id = ?1 AND author = ?2 AND sent_at = ?3"
                ),
                params![conv.to_string(), author.to_string(), sent_at],
                message_from_row,
            )
            .optional()?)
    }

    fn message(&self, id: i64) -> Result<Message> {
        Ok(self.conn.query_row(
            &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"),
            [id],
            message_from_row,
        )?)
    }

    fn contact_name(&self, uuid: &Uuid) -> Result<String> {
        let c: Option<Contact> = self
            .conn
            .query_row(
                "SELECT uuid, name, profile_name, phone FROM contacts WHERE uuid = ?1",
                [uuid.to_string()],
                |r| {
                    Ok(Contact {
                        uuid: parse_uuid(r.get(0)?)?,
                        name: r.get(1)?,
                        profile_name: r.get(2)?,
                        phone: r.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(c.map(|c| c.display_name())
            .unwrap_or_else(|| "Unknown contact".into()))
    }

    fn ensure_conversation(&self, id: &ConversationId) -> Result<()> {
        let title = match id {
            ConversationId::Direct(u) => self.contact_name(u)?,
            ConversationId::Group(_) => "Unknown group".to_string(),
        };
        self.conn.execute(
            "INSERT OR IGNORE INTO conversations (id, title) VALUES (?1, ?2)",
            params![id.to_string(), title],
        )?;
        Ok(())
    }

    /// Recomputes last activity, preview and unread count from messages.
    fn refresh_summary(&self, id: &ConversationId) -> Result<Conversation> {
        let key = id.to_string();
        let last: Option<Message> = self
            .conn
            .query_row(
                &format!(
                    "SELECT {MESSAGE_COLUMNS} FROM messages WHERE conversation_id = ?1 \
                     ORDER BY sent_at DESC, id DESC LIMIT 1"
                ),
                [&key],
                message_from_row,
            )
            .optional()?;
        let unread: i64 = self.conn.query_row(
            "SELECT count(*) FROM messages WHERE conversation_id = ?1 AND outgoing = 0 AND read = 0",
            [&key],
            |r| r.get(0),
        )?;
        let (activity, preview, outgoing) = match &last {
            Some(m) => (m.sent_at, self.preview_of(m)?, m.outgoing),
            None => (0, String::new(), false),
        };
        self.conn.execute(
            "UPDATE conversations SET last_activity = ?2, last_preview = ?3, \
             last_preview_outgoing = ?4, unread_count = ?5 WHERE id = ?1",
            params![key, activity, preview, outgoing as i64, unread],
        )?;
        Ok(self.conversation(id)?.expect("conversation exists"))
    }

    fn apply(&mut self, mutation: Mutation) -> Result<Changes> {
        let mut changes = Changes::default();
        match mutation {
            Mutation::UpsertContact(c) => {
                self.conn.execute(
                    "INSERT INTO contacts (uuid, name, profile_name, phone) VALUES (?1, ?2, ?3, ?4) \
                     ON CONFLICT(uuid) DO UPDATE SET \
                       name = COALESCE(excluded.name, name), \
                       profile_name = COALESCE(excluded.profile_name, profile_name), \
                       phone = COALESCE(excluded.phone, phone)",
                    params![c.uuid.to_string(), c.name, c.profile_name, c.phone],
                )?;
                let id = ConversationId::Direct(c.uuid);
                let title = self.contact_name(&c.uuid)?;
                self.conn.execute(
                    "INSERT INTO conversations (id, title) VALUES (?1, ?2) \
                     ON CONFLICT(id) DO UPDATE SET title = excluded.title",
                    params![id.to_string(), title],
                )?;
                changes.conversations.extend(self.conversation(&id)?);
            }
            Mutation::UpsertGroup {
                key,
                title,
                members,
            } => {
                let id = ConversationId::Group(key);
                let title = if title.trim().is_empty() {
                    "Unnamed group".to_string()
                } else {
                    title
                };
                self.conn.execute(
                    "INSERT INTO conversations (id, title, members) VALUES (?1, ?2, ?3) \
                     ON CONFLICT(id) DO UPDATE SET title = excluded.title, members = excluded.members",
                    params![id.to_string(), title, serde_json::to_string(&members)?],
                )?;
                changes.conversations.extend(self.conversation(&id)?);
            }
            Mutation::InsertMessage(m) => {
                let existed = self.conversation(&m.conversation)?.is_some();
                self.ensure_conversation(&m.conversation)?;
                let conv = self.conversation(&m.conversation)?.expect("ensured");
                if !m.outgoing && conv.blocked {
                    return Ok(changes); // drop messages from blocked chats
                }
                if m.outgoing {
                    self.set_column(&m.conversation, "accepted", 1)?;
                } else if !existed
                    && !m.conversation.is_group()
                    && !self.is_known_contact(&m.author)?
                {
                    // first contact from a stranger: a message request
                    self.set_column(&m.conversation, "accepted", 0)?;
                }
                // a message carrying a different timer updates the chat's timer
                if m.kind == MessageKind::Normal && m.expires_in_secs != conv.expire_timer {
                    self.conn.execute(
                        "UPDATE conversations SET expire_timer = ?2 WHERE id = ?1",
                        params![m.conversation.to_string(), m.expires_in_secs],
                    )?;
                }
                if let Some(id) = self.insert_message(&m)? {
                    changes.messages.push(self.message(id)?);
                    changes
                        .conversations
                        .push(self.refresh_summary(&m.conversation)?);
                }
            }
            Mutation::Edit {
                conversation,
                author,
                target_sent_at,
                body,
                edited_at,
            } => {
                if let Some(msg) = self.message_by_key(&conversation, &author, target_sent_at)?
                    && !msg.deleted
                {
                    let mut history = msg.edit_history.clone();
                    history.push(Revision {
                        body: msg.body.clone(),
                        at: msg.edited_at.unwrap_or(msg.sent_at),
                    });
                    self.conn.execute(
                        "UPDATE messages SET body = ?2, edited_at = ?3, edit_history = ?4 WHERE id = ?1",
                        params![msg.id, body, edited_at, serde_json::to_string(&history)?],
                    )?;
                    changes.messages.push(self.message(msg.id)?);
                    changes
                        .conversations
                        .push(self.refresh_summary(&conversation)?);
                }
            }
            Mutation::DeleteForEveryone {
                conversation,
                author,
                target_sent_at,
            } => {
                if let Some(msg) = self.message_by_key(&conversation, &author, target_sent_at)? {
                    self.conn.execute(
                        "UPDATE messages SET body = '', attachments = '[]', quote = NULL, \
                         reactions = '[]', deleted = 1, read = 1 WHERE id = ?1",
                        [msg.id],
                    )?;
                    changes.messages.push(self.message(msg.id)?);
                    changes
                        .conversations
                        .push(self.refresh_summary(&conversation)?);
                }
            }
            Mutation::React {
                conversation,
                target_author,
                target_sent_at,
                reactor,
                emoji,
                remove,
                at,
            } => {
                if let Some(mut msg) =
                    self.message_by_key(&conversation, &target_author, target_sent_at)?
                {
                    // one reaction per reactor, like Signal
                    msg.reactions.retain(|r| r.reactor != reactor);
                    if !remove {
                        msg.reactions.push(Reaction { reactor, emoji, at });
                    }
                    self.conn.execute(
                        "UPDATE messages SET reactions = ?2 WHERE id = ?1",
                        params![msg.id, serde_json::to_string(&msg.reactions)?],
                    )?;
                    changes.messages.push(self.message(msg.id)?);
                }
            }
            Mutation::Receipt {
                from,
                kind,
                sent_ats,
            } => {
                for sent_at in sent_ats {
                    let mut stmt = self.conn.prepare(&format!(
                        "SELECT {MESSAGE_COLUMNS} FROM messages WHERE outgoing = 1 AND sent_at = ?1"
                    ))?;
                    let msgs: Vec<Message> = stmt
                        .query_map([sent_at], message_from_row)?
                        .collect::<rusqlite::Result<_>>()?;
                    for msg in msgs {
                        // receipts for a direct chat must come from that contact
                        if let ConversationId::Direct(peer) = msg.conversation
                            && peer != from
                        {
                            continue;
                        }
                        let next = msg.status.advance(kind.status());
                        if next != msg.status {
                            self.conn.execute(
                                "UPDATE messages SET status = ?2 WHERE id = ?1",
                                params![msg.id, next as i64],
                            )?;
                            changes.messages.push(self.message(msg.id)?);
                        }
                    }
                }
            }
            Mutation::SendStatus {
                conversation,
                sent_at,
                status,
            } => {
                let mut stmt = self.conn.prepare(&format!(
                    "SELECT {MESSAGE_COLUMNS} FROM messages \
                     WHERE conversation_id = ?1 AND outgoing = 1 AND sent_at = ?2"
                ))?;
                let msgs: Vec<Message> = stmt
                    .query_map(params![conversation.to_string(), sent_at], message_from_row)?
                    .collect::<rusqlite::Result<_>>()?;
                for msg in msgs {
                    let next = msg.status.advance(status);
                    if next != msg.status {
                        self.conn.execute(
                            "UPDATE messages SET status = ?2 WHERE id = ?1",
                            params![msg.id, next as i64],
                        )?;
                        changes.messages.push(self.message(msg.id)?);
                    }
                }
            }
            Mutation::ReadSync { author, sent_at } => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, conversation_id FROM messages \
                     WHERE author = ?1 AND sent_at = ?2 AND outgoing = 0 AND read = 0",
                )?;
                let rows: Vec<(i64, String)> = stmt
                    .query_map(params![author.to_string(), sent_at], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })?
                    .collect::<rusqlite::Result<_>>()?;
                for (id, conv) in rows {
                    let conv = parse_conv_id(conv)?;
                    // Like Signal, reading a message implies reading everything before it.
                    self.conn.execute(
                        "UPDATE messages SET read = 1 WHERE conversation_id = ?1 AND outgoing = 0 \
                         AND read = 0 AND sent_at <= ?2",
                        params![conv.to_string(), sent_at],
                    )?;
                    self.start_timers(&conv, now_millis())?;
                    changes.messages.push(self.message(id)?);
                    changes.conversations.push(self.refresh_summary(&conv)?);
                }
            }
            Mutation::MarkConversationRead { conversation } => {
                let n = self.conn.execute(
                    "UPDATE messages SET read = 1 WHERE conversation_id = ?1 AND outgoing = 0 AND read = 0",
                    [conversation.to_string()],
                )?;
                let timers = self.start_timers(&conversation, now_millis())?;
                changes.messages.extend(timers);
                let was_marked = self
                    .conversation(&conversation)?
                    .is_some_and(|c| c.marked_unread);
                if was_marked {
                    self.set_column(&conversation, "marked_unread", 0)?;
                }
                if n > 0
                    || was_marked
                    || self
                        .conversation(&conversation)?
                        .is_some_and(|c| c.unread_count > 0)
                {
                    changes
                        .conversations
                        .push(self.refresh_summary(&conversation)?);
                }
            }
            Mutation::AttachmentDownloaded {
                message_id,
                index,
                local_path,
            } => {
                let Ok(mut msg) = self.message(message_id) else {
                    return Ok(changes);
                };
                if let Some(att) = msg.attachments.get_mut(index) {
                    att.local_path = Some(local_path);
                    self.conn.execute(
                        "UPDATE messages SET attachments = ?2 WHERE id = ?1",
                        params![message_id, serde_json::to_string(&msg.attachments)?],
                    )?;
                    changes.messages.push(self.message(message_id)?);
                }
            }
            Mutation::SetExpireTimer {
                conversation,
                secs,
                author,
                outgoing,
                at,
            } => {
                let secs = secs.filter(|s| *s > 0);
                self.ensure_conversation(&conversation)?;
                self.conn.execute(
                    "UPDATE conversations SET expire_timer = ?2 WHERE id = ?1",
                    params![conversation.to_string(), secs],
                )?;
                let mut notice = NewMessage::text(conversation, author, at, outgoing, "");
                notice.read = true;
                notice.kind = MessageKind::TimerChanged { secs };
                if let Some(id) = self.insert_message(&notice)? {
                    changes.messages.push(self.message(id)?);
                }
                changes
                    .conversations
                    .push(self.refresh_summary(&conversation)?);
            }
            Mutation::StartExpiryTimers { conversation, now } => {
                let started = self.start_timers(&conversation, now)?;
                changes.messages.extend(started);
            }
            Mutation::ExpireDue { now } => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, conversation_id FROM messages WHERE expires_at IS NOT NULL AND expires_at <= ?1",
                )?;
                let due: Vec<(i64, String)> = stmt
                    .query_map([now], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                let mut touched = Vec::new();
                for (id, conv) in due {
                    let conv = parse_conv_id(conv)?;
                    changes.removed_files.extend(self.attachment_files(id)?);
                    self.conn
                        .execute("DELETE FROM messages WHERE id = ?1", [id])?;
                    changes.removed_messages.push((conv, id));
                    if !touched.contains(&conv) {
                        touched.push(conv);
                    }
                }
                for conv in touched {
                    changes.conversations.push(self.refresh_summary(&conv)?);
                }
            }
            Mutation::DeleteForMe { message_id } => {
                if let Ok(msg) = self.message(message_id) {
                    changes
                        .removed_files
                        .extend(self.attachment_files(message_id)?);
                    self.conn
                        .execute("DELETE FROM messages WHERE id = ?1", [message_id])?;
                    changes
                        .removed_messages
                        .push((msg.conversation, message_id));
                    changes
                        .conversations
                        .push(self.refresh_summary(&msg.conversation)?);
                }
            }
            Mutation::DeleteConversation { conversation } => {
                let key = conversation.to_string();
                let ids: Vec<i64> = self
                    .conn
                    .prepare("SELECT id FROM messages WHERE conversation_id = ?1")?
                    .query_map([&key], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                for id in ids {
                    changes.removed_files.extend(self.attachment_files(id)?);
                }
                self.conn
                    .execute("DELETE FROM messages WHERE conversation_id = ?1", [&key])?;
                // keep the row (contact/group metadata) but clear its state
                self.conn.execute(
                    "UPDATE conversations SET last_activity = 0, last_preview = '', unread_count = 0, \
                     pinned = 0, archived = 0, marked_unread = 0, draft = NULL WHERE id = ?1",
                    [&key],
                )?;
                changes.removed_conversations.push(conversation);
            }
            Mutation::SetMuted {
                conversation,
                until,
            } => {
                self.conn.execute(
                    "UPDATE conversations SET muted_until = ?2 WHERE id = ?1",
                    params![conversation.to_string(), until.unwrap_or(0)],
                )?;
                changes
                    .conversations
                    .extend(self.conversation(&conversation)?);
            }
            Mutation::SetMarkedUnread {
                conversation,
                unread,
            } => {
                self.set_column(&conversation, "marked_unread", unread as i64)?;
                changes
                    .conversations
                    .extend(self.conversation(&conversation)?);
            }
            Mutation::SetBlocked {
                conversation,
                blocked,
            } => {
                self.ensure_conversation(&conversation)?;
                self.set_column(&conversation, "blocked", blocked as i64)?;
                if !blocked {
                    self.set_column(&conversation, "accepted", 1)?;
                }
                changes
                    .conversations
                    .extend(self.conversation(&conversation)?);
            }
            Mutation::Accept { conversation } => {
                self.set_column(&conversation, "accepted", 1)?;
                self.set_column(&conversation, "blocked", 0)?;
                changes
                    .conversations
                    .extend(self.conversation(&conversation)?);
            }
        }
        Ok(changes)
    }

    /// List preview of a message with author and mention names resolved.
    fn preview_of(&self, m: &Message) -> Result<String> {
        if let MessageKind::TimerChanged { secs } = &m.kind {
            let who = if m.outgoing {
                None
            } else {
                Some(self.contact_name(&m.author)?)
            };
            return Ok(crate::model::timer_notice(who.as_deref(), *secs));
        }
        if m.deleted || m.body_ranges.is_empty() {
            return Ok(m.preview());
        }
        let rich = crate::richtext::layout(&m.body, &m.body_ranges, |u| {
            self.contact_name(u)
                .unwrap_or_else(|_| "Unknown contact".into())
        });
        let mut resolved = m.clone();
        resolved.body = rich.text;
        resolved.body_ranges.clear();
        Ok(resolved.preview())
    }

    /// Local paths of a message's attachments.
    fn attachment_files(&self, id: i64) -> Result<Vec<String>> {
        Ok(self
            .message(id)
            .map(|m| {
                m.attachments
                    .into_iter()
                    .filter_map(|a| a.local_path)
                    .collect()
            })
            .unwrap_or_default())
    }

    fn set_column(&self, id: &ConversationId, column: &'static str, value: i64) -> Result<()> {
        self.conn.execute(
            &format!("UPDATE conversations SET {column} = ?2 WHERE id = ?1"),
            params![id.to_string(), value],
        )?;
        Ok(())
    }

    /// A contact synced from the phone's address book (has a name).
    fn is_known_contact(&self, uuid: &Uuid) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM contacts WHERE uuid = ?1 AND name IS NOT NULL",
                [uuid.to_string()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Starts the disappearing timer of read incoming messages that have one.
    fn start_timers(&self, conversation: &ConversationId, now: Millis) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, expires_in FROM messages WHERE conversation_id = ?1 AND read = 1 \
             AND expires_in > 0 AND expires_at IS NULL",
        )?;
        let rows: Vec<(i64, i64)> = stmt
            .query_map([conversation.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for (id, secs) in rows {
            self.conn.execute(
                "UPDATE messages SET expires_at = ?2 WHERE id = ?1",
                params![id, now + secs * 1000],
            )?;
            out.push(self.message(id)?);
        }
        Ok(out)
    }

    fn insert_message(&self, m: &NewMessage) -> Result<Option<i64>> {
        let quote = m.quote.as_ref().map(serde_json::to_string).transpose()?;
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO messages (conversation_id, author, sent_at, received_at, outgoing, \
             body, attachments, quote, status, read, expires_in, expires_at, kind, body_ranges) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                m.conversation.to_string(),
                m.author.to_string(),
                m.sent_at,
                m.received_at,
                m.outgoing as i64,
                m.body,
                serde_json::to_string(&m.attachments)?,
                quote,
                m.status as i64,
                (m.read || m.outgoing) as i64,
                m.expires_in_secs.filter(|s| *s > 0),
                // outgoing messages start their timer when sent
                m.expires_in_secs
                    .filter(|s| *s > 0 && m.outgoing)
                    .map(|s| m.sent_at + s as i64 * 1000),
                serde_json::to_string(&m.kind)?,
                serde_json::to_string(&m.body_ranges)?,
            ],
        )?;
        Ok((n == 1).then(|| self.conn.last_insert_rowid()))
    }
}

#[cfg(test)]
mod tests;
