//! Persistent storage manager for internal DTN services (`chat`, `files`).
//!
//! Backed by an embedded SQLite database (`services.db`) and raw payload files directory
//! stored in `<data_dir>/internal_services/`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hardy_async::CancellationToken;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tracing::{debug, error, info, warn};

/// Chat message record stored in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    /// Bundle ID / unique key
    pub id: String,
    /// Normalized conversation peer node EID (e.g. "dtn://f4jxq/")
    pub conversation_peer: String,
    /// Transmitting endpoint ID (e.g. "dtn://f4jxq/chat")
    pub sender_eid: String,
    /// Destination endpoint ID (e.g. "dtn://stata/chat")
    pub recipient_eid: String,
    /// Message body (UTF-8 Markdown)
    pub content: String,
    /// Direction: "inbound" | "outbound"
    pub direction: String,
    /// Status: "queued" | "sent" | "received"
    pub status: String,
    /// Whether the message has been read by the operator
    pub is_read: bool,
    /// Whether the message is pinned/protected against lifetime expiration
    #[serde(default)]
    pub is_sticky: bool,
    /// Creation timestamp in milliseconds since epoch
    pub timestamp: i64,
    /// Expiration deadline timestamp in milliseconds (0 if none)
    #[serde(default)]
    pub expiry_timestamp: i64,
    /// RFC 3339 formatted creation date string
    pub created_at: String,
}

/// Conversation summary item for conversations list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationSummary {
    /// Normalized peer EID (e.g. "dtn://f4jxq/")
    pub peer: String,
    /// Callsign extracted from peer if available (e.g. "F4JXQ")
    pub callsign: Option<String>,
    /// Last message content snippet
    pub last_message: String,
    /// Last message direction
    pub last_direction: String,
    /// Last message status
    pub last_status: String,
    /// Last message timestamp in milliseconds
    pub last_timestamp: i64,
    /// Last message RFC 3339 formatted date
    pub last_created_at: String,
    /// Number of unread inbound messages in this conversation
    pub unread_count: u32,
    /// Total number of messages in this conversation
    pub total_messages: u32,
}

/// Metadata record for a bundle received by the `files` service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileBundleRecord {
    /// Bundle ID / unique key
    pub id: String,
    /// Source endpoint ID (e.g. "dtn://f4jxq/files")
    pub source_eid: String,
    /// Destination endpoint ID (e.g. "dtn://stata/files")
    pub destination_eid: String,
    /// Creation timestamp in milliseconds since epoch
    pub creation_timestamp: i64,
    /// Bundle lifetime in seconds
    pub lifetime_secs: u64,
    /// Expiration timestamp in milliseconds since epoch
    pub expiry_timestamp: i64,
    /// Payload size in bytes
    pub size: u64,
    /// Absolute path to the raw payload file on disk
    pub payload_path: String,
    /// Detected MIME type (e.g. "text/markdown", "image/png", "application/octet-stream")
    pub mime_type: String,
    /// Whether the file bundle is pinned/sticky against expiration
    #[serde(default)]
    pub is_sticky: bool,
    /// Whether the bundle has been viewed by the operator
    #[serde(default)]
    pub is_read: bool,
    /// RFC 3339 formatted creation date
    pub created_at: String,
}

/// Normalizes any peer string into canonical node EID.
/// E.g. "f4jxq" -> "dtn://f4jxq/"
pub fn normalize_peer_node(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "dtn:none".to_string();
    }

    if let Some(stripped) = trimmed.strip_prefix("ipn:") {
        let parts: Vec<&str> = stripped.split('.').collect();
        if let Some(node) = parts.first() {
            return format!("ipn:{node}.0");
        }
        return trimmed.to_string();
    }

    let without_prefix = if let Some(stripped) = trimmed.strip_prefix("dtn://") {
        stripped
    } else if let Some(stripped) = trimmed.strip_prefix("dtn:") {
        stripped
    } else {
        trimmed
    };

    let node_part = without_prefix.split('/').next().unwrap_or(without_prefix);
    let node_clean = node_part.trim().to_lowercase();
    format!("dtn://{node_clean}/")
}

/// Normalizes a target into a chat service endpoint according to chat rules:
/// - If starting with "dtn://", checks if there is a service component.
///   If no service, appends "/chat".
///   If service is present, preserves it as-is without modification.
/// - If not starting with "dtn://", prepends "dtn://" and appends "/chat".
/// - IPN scheme (e.g. "ipn:1.1") is preserved as-is.
pub fn normalize_chat_target(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "dtn:none".to_string();
    }

    if trimmed.starts_with("ipn:") {
        return trimmed.to_string();
    }

    if let Some(stripped) = trimmed.strip_prefix("dtn://") {
        let parts: Vec<&str> = stripped
            .split('/')
            .filter(|p| !p.trim().is_empty())
            .collect();
        if parts.is_empty() {
            "dtn://none/chat".to_string()
        } else if parts.len() == 1 {
            // Node name only, no service component: append "/chat"
            format!("dtn://{}/chat", parts[0].to_lowercase())
        } else {
            // Has node name and service component: preserve both, do not modify!
            let node = parts[0].to_lowercase();
            let service = parts[1..].join("/");
            format!("dtn://{node}/{service}")
        }
    } else {
        // Does not start with "dtn://": prepend "dtn://" and append "/chat"
        let clean = trimmed.trim_matches('/').to_lowercase();
        format!("dtn://{clean}/chat")
    }
}

/// Normalizes a target into a chat service endpoint.
/// Alias to [`normalize_chat_target`].
pub fn normalize_chat_endpoint(input: &str) -> String {
    normalize_chat_target(input)
}

/// Normalizes a target into a files service endpoint.
/// E.g. "dtn://f4jxq/" -> "dtn://f4jxq/files"
pub fn normalize_files_endpoint(input: &str) -> String {
    let peer = normalize_peer_node(input);
    if peer.starts_with("ipn:") {
        if let Some(stripped) = peer.strip_suffix(".0") {
            return format!("{stripped}.2");
        }
        return peer;
    }
    format!("{peer}files")
}

/// Extracts a readable callsign from a node EID if possible.
pub fn extract_callsign(peer_eid: &str) -> Option<String> {
    if let Some(stripped) = peer_eid.strip_prefix("dtn://") {
        let node = stripped
            .trim_end_matches('/')
            .split('/')
            .next()
            .unwrap_or("");
        let callsign = node.split('-').next().unwrap_or(node).to_uppercase();
        if !callsign.is_empty() && callsign.chars().all(|c| c.is_alphanumeric()) {
            return Some(callsign);
        }
    }
    None
}

/// Persistent storage manager for station internal services.
#[derive(Clone)]
pub struct InternalStorage {
    conn: Arc<Mutex<Connection>>,
    files_dir: PathBuf,
    _ephemeral_dir: Option<Arc<tempfile::TempDir>>,
}

impl InternalStorage {
    /// Constructs a persistent storage in `<base_dir>/internal_services/`.
    pub fn new_disk(base_dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        let internal_dir = base_dir.as_ref().join("internal_services");
        let files_dir = internal_dir.join("files");

        std::fs::create_dir_all(&internal_dir)?;
        std::fs::create_dir_all(&files_dir)?;

        let db_path = internal_dir.join("services.db");
        let conn = Connection::open(&db_path)?;
        Self::init_tables(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            files_dir,
            _ephemeral_dir: None,
        })
    }

    /// Constructs an in-memory storage with temporary files directory.
    pub fn new_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init_tables(&conn)?;

        let temp_dir = tempfile::Builder::new()
            .prefix("dtn-ham-internal-")
            .tempdir()?;
        let files_dir = temp_dir.path().join("files");
        std::fs::create_dir_all(&files_dir)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            files_dir,
            _ephemeral_dir: Some(Arc::new(temp_dir)),
        })
    }

    /// Accessor for the files storage payload directory.
    #[must_use]
    pub fn files_dir(&self) -> &Path {
        &self.files_dir
    }

    fn init_tables(conn: &Connection) -> anyhow::Result<()> {
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;

            CREATE TABLE IF NOT EXISTS chat_conversations (
                peer TEXT PRIMARY KEY,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS chat_messages (
                id TEXT PRIMARY KEY,
                conversation_peer TEXT NOT NULL,
                sender_eid TEXT NOT NULL,
                recipient_eid TEXT NOT NULL,
                content TEXT NOT NULL,
                direction TEXT NOT NULL CHECK(direction IN ('inbound', 'outbound')),
                status TEXT NOT NULL CHECK(status IN ('queued', 'sent', 'received')),
                is_read INTEGER NOT NULL DEFAULT 0,
                is_sticky INTEGER NOT NULL DEFAULT 0,
                timestamp INTEGER NOT NULL,
                expiry_timestamp INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_chat_peer ON chat_messages(conversation_peer, timestamp);
            CREATE INDEX IF NOT EXISTS idx_chat_expiry ON chat_messages(is_sticky, expiry_timestamp);

            INSERT OR IGNORE INTO chat_conversations (peer, created_at)
            SELECT DISTINCT conversation_peer, created_at FROM chat_messages;

            CREATE TABLE IF NOT EXISTS files_bundles (
                id TEXT PRIMARY KEY,
                source_eid TEXT NOT NULL,
                destination_eid TEXT NOT NULL,
                creation_timestamp INTEGER NOT NULL,
                lifetime_secs INTEGER NOT NULL,
                expiry_timestamp INTEGER NOT NULL,
                size INTEGER NOT NULL,
                payload_path TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                is_sticky INTEGER NOT NULL DEFAULT 0,
                is_read INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_files_created ON files_bundles(creation_timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_files_expiry ON files_bundles(is_sticky, expiry_timestamp);
            "#,
        )?;

        let user_version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if user_version < 1 {
            conn.execute_batch(
                r#"
                -- Migration: convert DTN BPv7 epoch (relative to 2000-01-01) timestamps to Unix epoch (1970-01-01)
                -- 946684800000 ms is the exact offset between 1970-01-01 and 2000-01-01.
                UPDATE chat_messages
                SET timestamp = timestamp + 946684800000,
                    expiry_timestamp = expiry_timestamp + 946684800000
                WHERE timestamp > 0 AND timestamp < 1000000000000;

                -- Migration: fix legacy outbound chat messages that had 24h (86400s) lifetime instead of 1h (3600s)
                UPDATE chat_messages
                SET expiry_timestamp = timestamp + 3600000
                WHERE is_sticky = 0 AND (expiry_timestamp - timestamp) > 3600000;

                UPDATE files_bundles
                SET creation_timestamp = creation_timestamp + 946684800000,
                    expiry_timestamp = expiry_timestamp + 946684800000
                WHERE creation_timestamp > 0 AND creation_timestamp < 1000000000000;

                PRAGMA user_version = 1;
                "#,
            )?;
        }
        Ok(())
    }

    // =========================================================================
    // Chat Operations
    // =========================================================================

    /// Inserts or replaces a chat message.
    pub fn insert_chat_message(&self, msg: &ChatMessage) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;

        // Ensure the conversation peer is recorded in chat_conversations
        conn.execute(
            "INSERT OR IGNORE INTO chat_conversations (peer, created_at) VALUES (?1, ?2)",
            params![msg.conversation_peer, msg.created_at],
        )?;

        conn.execute(
            r#"
            INSERT OR REPLACE INTO chat_messages (
                id, conversation_peer, sender_eid, recipient_eid, content,
                direction, status, is_read, is_sticky, timestamp, expiry_timestamp, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                msg.id,
                msg.conversation_peer,
                msg.sender_eid,
                msg.recipient_eid,
                msg.content,
                msg.direction,
                msg.status,
                if msg.is_read { 1 } else { 0 },
                if msg.is_sticky { 1 } else { 0 },
                msg.timestamp,
                msg.expiry_timestamp,
                msg.created_at,
            ],
        )?;
        Ok(())
    }

    /// Alias for insert_chat_message for backwards compatibility.
    pub fn insert_message(&self, msg: &ChatMessage) -> anyhow::Result<()> {
        self.insert_chat_message(msg)
    }

    /// Explicitly creates and persists a conversation thread even before any message is sent.
    pub fn create_conversation(&self, peer: &str) -> anyhow::Result<ConversationSummary> {
        let canonical = normalize_chat_target(peer);
        let now = OffsetDateTime::now_utc();
        let created_at = now
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| now.to_string());

        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;

        conn.execute(
            "INSERT OR IGNORE INTO chat_conversations (peer, created_at) VALUES (?1, ?2)",
            params![canonical, created_at],
        )?;

        drop(conn);
        self.get_conversation_summary(&canonical)
    }

    /// Returns a single conversation summary for a peer.
    pub fn get_conversation_summary(&self, peer: &str) -> anyhow::Result<ConversationSummary> {
        let canonical = normalize_chat_target(peer);
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;

        let mut stmt = conn.prepare(
            r#"
            SELECT
                c.peer,
                COALESCE(m.content, '') as content,
                COALESCE(m.direction, 'outbound') as direction,
                COALESCE(m.status, 'queued') as status,
                COALESCE(m.timestamp, 0) as timestamp,
                COALESCE(m.created_at, c.created_at) as created_at,
                (SELECT COUNT(*) FROM chat_messages m2 WHERE m2.conversation_peer = c.peer AND m2.is_read = 0 AND m2.direction = 'inbound') as unread,
                (SELECT COUNT(*) FROM chat_messages m3 WHERE m3.conversation_peer = c.peer) as total
            FROM chat_conversations c
            LEFT JOIN chat_messages m ON m.rowid = (
                SELECT MAX(rowid) FROM chat_messages WHERE conversation_peer = c.peer
            )
            WHERE c.peer = ?1
            "#,
        )?;

        let mut rows = stmt.query_map([&canonical], |row| {
            let peer: String = row.get(0)?;
            let last_message: String = row.get(1)?;
            let last_direction: String = row.get(2)?;
            let last_status: String = row.get(3)?;
            let last_timestamp: i64 = row.get(4)?;
            let last_created_at: String = row.get(5)?;
            let unread_count: u32 = row.get(6)?;
            let total_messages: u32 = row.get(7)?;

            Ok(ConversationSummary {
                peer: peer.clone(),
                callsign: Some(peer),
                last_message,
                last_direction,
                last_status,
                last_timestamp,
                last_created_at,
                unread_count,
                total_messages,
            })
        })?;

        if let Some(r) = rows.next() {
            Ok(r?)
        } else {
            let now = OffsetDateTime::now_utc();
            let created_at = now
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| now.to_string());
            Ok(ConversationSummary {
                peer: canonical.clone(),
                callsign: Some(canonical.clone()),
                last_message: String::new(),
                last_direction: "outbound".to_string(),
                last_status: "queued".to_string(),
                last_timestamp: 0,
                last_created_at: created_at,
                unread_count: 0,
                total_messages: 0,
            })
        }
    }

    /// Lists all conversation threads with summaries.
    pub fn list_conversations(&self) -> anyhow::Result<Vec<ConversationSummary>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;

        let mut stmt = conn.prepare(
            r#"
            SELECT
                c.peer,
                COALESCE(m.content, '') as content,
                COALESCE(m.direction, 'outbound') as direction,
                COALESCE(m.status, 'queued') as status,
                COALESCE(m.timestamp, 0) as timestamp,
                COALESCE(m.created_at, c.created_at) as created_at,
                (SELECT COUNT(*) FROM chat_messages m2 WHERE m2.conversation_peer = c.peer AND m2.is_read = 0 AND m2.direction = 'inbound') as unread,
                (SELECT COUNT(*) FROM chat_messages m3 WHERE m3.conversation_peer = c.peer) as total
            FROM chat_conversations c
            LEFT JOIN chat_messages m ON m.rowid = (
                SELECT MAX(rowid) FROM chat_messages WHERE conversation_peer = c.peer
            )
            ORDER BY COALESCE(m.timestamp, 0) DESC, c.created_at DESC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            let peer: String = row.get(0)?;
            let last_message: String = row.get(1)?;
            let last_direction: String = row.get(2)?;
            let last_status: String = row.get(3)?;
            let last_timestamp: i64 = row.get(4)?;
            let last_created_at: String = row.get(5)?;
            let unread_count: u32 = row.get(6)?;
            let total_messages: u32 = row.get(7)?;

            Ok(ConversationSummary {
                peer: peer.clone(),
                callsign: Some(peer),
                last_message,
                last_direction,
                last_status,
                last_timestamp,
                last_created_at,
                unread_count,
                total_messages,
            })
        })?;

        let mut convs = Vec::new();
        for r in rows {
            convs.push(r?);
        }
        Ok(convs)
    }

    /// Gets all messages for a specific conversation peer.
    pub fn get_conversation_messages(&self, peer: &str) -> anyhow::Result<Vec<ChatMessage>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
        let norm = normalize_chat_target(peer);

        let mut stmt = conn.prepare(
            r#"
            SELECT id, conversation_peer, sender_eid, recipient_eid, content,
                   direction, status, is_read, is_sticky, timestamp, expiry_timestamp, created_at
            FROM chat_messages
            WHERE conversation_peer = ?1
            ORDER BY timestamp ASC
            "#,
        )?;

        let rows = stmt.query_map([norm], |row| {
            Ok(ChatMessage {
                id: row.get(0)?,
                conversation_peer: row.get(1)?,
                sender_eid: row.get(2)?,
                recipient_eid: row.get(3)?,
                content: row.get(4)?,
                direction: row.get(5)?,
                status: row.get(6)?,
                is_read: row.get::<_, i64>(7)? != 0,
                is_sticky: row.get::<_, i64>(8)? != 0,
                timestamp: row.get(9)?,
                expiry_timestamp: row.get(10)?,
                created_at: row.get(11)?,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Marks all inbound messages from a peer as read.
    pub fn mark_conversation_read(&self, peer: &str) -> anyhow::Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
        let norm = normalize_chat_target(peer);
        let count = conn.execute(
            "UPDATE chat_messages SET is_read = 1 WHERE conversation_peer = ?1 AND is_read = 0",
            params![norm],
        )?;
        Ok(count)
    }

    /// Toggles the sticky retention flag for a chat message.
    pub fn toggle_chat_sticky(&self, id: &str) -> anyhow::Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
        let current: Option<i64> = conn
            .query_row(
                "SELECT is_sticky FROM chat_messages WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .ok();

        let Some(val) = current else {
            anyhow::bail!("Chat message not found");
        };

        let next = if val == 0 { 1 } else { 0 };
        conn.execute(
            "UPDATE chat_messages SET is_sticky = ?1 WHERE id = ?2",
            params![next, id],
        )?;
        Ok(next != 0)
    }

    // =========================================================================
    // Files Operations
    // =========================================================================

    /// Inserts or replaces a file bundle record.
    pub fn insert_file_bundle(&self, bundle: &FileBundleRecord) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
        conn.execute(
            r#"
            INSERT OR REPLACE INTO files_bundles (
                id, source_eid, destination_eid, creation_timestamp, lifetime_secs,
                expiry_timestamp, size, payload_path, mime_type, is_sticky, is_read, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                bundle.id,
                bundle.source_eid,
                bundle.destination_eid,
                bundle.creation_timestamp,
                bundle.lifetime_secs as i64,
                bundle.expiry_timestamp,
                bundle.size as i64,
                bundle.payload_path,
                bundle.mime_type,
                if bundle.is_sticky { 1 } else { 0 },
                if bundle.is_read { 1 } else { 0 },
                bundle.created_at,
            ],
        )?;
        Ok(())
    }

    /// Lists all file bundle records ordered by newest first.
    pub fn list_file_bundles(&self) -> anyhow::Result<Vec<FileBundleRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, source_eid, destination_eid, creation_timestamp, lifetime_secs,
                   expiry_timestamp, size, payload_path, mime_type, is_sticky, is_read, created_at
            FROM files_bundles
            ORDER BY creation_timestamp DESC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(FileBundleRecord {
                id: row.get(0)?,
                source_eid: row.get(1)?,
                destination_eid: row.get(2)?,
                creation_timestamp: row.get(3)?,
                lifetime_secs: row.get::<_, i64>(4)? as u64,
                expiry_timestamp: row.get(5)?,
                size: row.get::<_, i64>(6)? as u64,
                payload_path: row.get(7)?,
                mime_type: row.get(8)?,
                is_sticky: row.get::<_, i64>(9)? != 0,
                is_read: row.get::<_, i64>(10)? != 0,
                created_at: row.get(11)?,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Gets a file bundle record by its ID.
    pub fn get_file_bundle(&self, id: &str) -> anyhow::Result<Option<FileBundleRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, source_eid, destination_eid, creation_timestamp, lifetime_secs,
                   expiry_timestamp, size, payload_path, mime_type, is_sticky, is_read, created_at
            FROM files_bundles
            WHERE id = ?1
            "#,
        )?;

        let mut rows = stmt.query_map(params![id], |row| {
            Ok(FileBundleRecord {
                id: row.get(0)?,
                source_eid: row.get(1)?,
                destination_eid: row.get(2)?,
                creation_timestamp: row.get(3)?,
                lifetime_secs: row.get::<_, i64>(4)? as u64,
                expiry_timestamp: row.get(5)?,
                size: row.get::<_, i64>(6)? as u64,
                payload_path: row.get(7)?,
                mime_type: row.get(8)?,
                is_sticky: row.get::<_, i64>(9)? != 0,
                is_read: row.get::<_, i64>(10)? != 0,
                created_at: row.get(11)?,
            })
        })?;

        if let Some(r) = rows.next() {
            Ok(Some(r?))
        } else {
            Ok(None)
        }
    }

    /// Toggles the sticky retention flag for a file bundle.
    pub fn toggle_file_sticky(&self, id: &str) -> anyhow::Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
        let current: Option<i64> = conn
            .query_row(
                "SELECT is_sticky FROM files_bundles WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .ok();

        let Some(val) = current else {
            anyhow::bail!("File bundle not found");
        };

        let next = if val == 0 { 1 } else { 0 };
        conn.execute(
            "UPDATE files_bundles SET is_sticky = ?1 WHERE id = ?2",
            params![next, id],
        )?;
        Ok(next != 0)
    }

    /// Marks a file bundle as read.
    pub fn mark_file_read(&self, id: &str) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
        conn.execute(
            "UPDATE files_bundles SET is_read = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// Deletes a file bundle and removes its payload file from disk.
    pub fn delete_file_bundle(&self, id: &str) -> anyhow::Result<bool> {
        let record = self.get_file_bundle(id)?;
        let Some(rec) = record else {
            return Ok(false);
        };

        // Attempt payload deletion; ignore if already absent
        let path = Path::new(&rec.payload_path);
        if path.exists()
            && let Err(e) = std::fs::remove_file(path)
        {
            warn!(
                "Failed to remove payload file {} during deletion: {}",
                rec.payload_path, e
            );
        }

        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
        let count = conn.execute("DELETE FROM files_bundles WHERE id = ?1", params![id])?;
        Ok(count > 0)
    }

    /// Returns the number of unread file bundles.
    pub fn count_unread_files(&self) -> anyhow::Result<u32> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM files_bundles WHERE is_read = 0",
            [],
            |r| r.get(0),
        )?;
        Ok(count as u32)
    }

    // =========================================================================
    // Expiration & Cyclic Purge
    // =========================================================================

    /// Computes the next earliest expiration timestamp across non-sticky items.
    /// Returns `None` if no items are scheduled for expiration.
    pub fn get_next_expiration_deadline(&self) -> anyhow::Result<Option<i64>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;

        let min_files: Option<i64> = conn
            .query_row(
                "SELECT MIN(expiry_timestamp) FROM files_bundles WHERE is_sticky = 0 AND expiry_timestamp > 0",
                [],
                |r| r.get(0),
            )
            .ok()
            .flatten();

        let min_chat: Option<i64> = conn
            .query_row(
                "SELECT MIN(expiry_timestamp) FROM chat_messages WHERE is_sticky = 0 AND expiry_timestamp > 0",
                [],
                |r| r.get(0),
            )
            .ok()
            .flatten();

        let next = match (min_files, min_chat) {
            (Some(f), Some(c)) => Some(f.min(c)),
            (Some(f), None) => Some(f),
            (None, Some(c)) => Some(c),
            (None, None) => None,
        };

        Ok(next)
    }

    /// Purges all non-sticky bundles/messages whose `expiry_timestamp <= now_ms`.
    /// Returns the total number of items deleted.
    pub fn purge_expired(&self, now_ms: i64) -> anyhow::Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;

        // 1. Find and remove expired file bundles
        let mut files_stmt = conn.prepare(
            "SELECT id, payload_path FROM files_bundles WHERE is_sticky = 0 AND expiry_timestamp > 0 AND expiry_timestamp <= ?1",
        )?;
        let expired_files: Vec<(String, String)> = files_stmt
            .query_map(params![now_ms], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        for (id, path) in &expired_files {
            let p = Path::new(path);
            if p.exists()
                && let Err(e) = std::fs::remove_file(p)
            {
                warn!(
                    "Expiration: failed to delete payload file {} for bundle {}: {}",
                    path, id, e
                );
            }
        }

        let purged_files = conn.execute(
            "DELETE FROM files_bundles WHERE is_sticky = 0 AND expiry_timestamp > 0 AND expiry_timestamp <= ?1",
            params![now_ms],
        )?;

        // 2. Purge expired chat messages
        let purged_chat = conn.execute(
            "DELETE FROM chat_messages WHERE is_sticky = 0 AND expiry_timestamp > 0 AND expiry_timestamp <= ?1",
            params![now_ms],
        )?;

        let total = purged_files + purged_chat;
        if total > 0 {
            info!(
                "Expiration reaper: purged {} expired items (files: {}, chat: {})",
                total, purged_files, purged_chat
            );
        }

        Ok(total)
    }
}

/// Spawns the background expiration reaper task that sleeps until the next earliest deadline.
pub fn spawn_expiration_reaper(
    storage: InternalStorage,
    service_tracker: crate::service_tracker::ServiceTracker,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        debug!("Internal services expiration reaper task started");
        loop {
            // Check current time in milliseconds
            let now_ms = time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
            let now_ms = now_ms as i64;

            // Purge anything already expired
            match storage.purge_expired(now_ms) {
                Ok(purged) if purged > 0 => {
                    service_tracker.broadcast_bundles_update();
                }
                Err(e) => {
                    error!("Error running expiration purge: {}", e);
                }
                _ => {}
            }

            // Determine sleep duration until next deadline
            let sleep_duration = match storage.get_next_expiration_deadline() {
                Ok(Some(deadline)) => {
                    if deadline <= now_ms {
                        // Expired item present; minimal backoff before purging
                        Duration::from_millis(50)
                    } else {
                        let diff_ms = (deadline - now_ms) as u64;
                        // Cap sleep between 200ms and 1 hour
                        Duration::from_millis(diff_ms.clamp(200, 3600 * 1000))
                    }
                }
                Ok(None) => {
                    // No expiration scheduled; sleep for 10 minutes or until cancelled
                    Duration::from_secs(600)
                }
                Err(e) => {
                    error!("Failed to fetch next expiration deadline: {}", e);
                    Duration::from_secs(30)
                }
            };

            tokio::select! {
                _ = cancel.cancelled() => {
                    debug!("Expiration reaper task cancelled");
                    break;
                }
                _ = tokio::time::sleep(sleep_duration) => {}
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_internal_storage_crud_and_sticky() {
        let storage = InternalStorage::new_in_memory().unwrap();

        // 1. Files CRUD
        let record = FileBundleRecord {
            id: "bundle-file-1".to_string(),
            source_eid: "dtn://f4jxq/files".to_string(),
            destination_eid: "dtn://stata/files".to_string(),
            creation_timestamp: 1000,
            lifetime_secs: 3600,
            expiry_timestamp: 3601000,
            size: 42,
            payload_path: "/tmp/fake_payload.bin".to_string(),
            mime_type: "text/markdown".to_string(),
            is_sticky: false,
            is_read: false,
            created_at: "2026-11-05T00:00:00Z".to_string(),
        };

        storage.insert_file_bundle(&record).unwrap();
        assert_eq!(storage.count_unread_files().unwrap(), 1);

        let list = storage.list_file_bundles().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "bundle-file-1");
        assert!(!list[0].is_sticky);

        // Toggle sticky
        let is_sticky = storage.toggle_file_sticky("bundle-file-1").unwrap();
        assert!(is_sticky);
        let updated = storage.get_file_bundle("bundle-file-1").unwrap().unwrap();
        assert!(updated.is_sticky);

        // Mark read
        storage.mark_file_read("bundle-file-1").unwrap();
        assert_eq!(storage.count_unread_files().unwrap(), 0);

        // Delete
        let deleted = storage.delete_file_bundle("bundle-file-1").unwrap();
        assert!(deleted);
        assert!(storage.get_file_bundle("bundle-file-1").unwrap().is_none());
    }

    #[test]
    fn test_expiration_reaper_purging() {
        let storage = InternalStorage::new_in_memory().unwrap();

        // Expired non-sticky bundle
        let f1 = FileBundleRecord {
            id: "f1".to_string(),
            source_eid: "dtn://f4jxq/files".to_string(),
            destination_eid: "dtn://stata/files".to_string(),
            creation_timestamp: 1000,
            lifetime_secs: 10,
            expiry_timestamp: 1000 + 10 * 1000, // 11000
            size: 10,
            payload_path: "/tmp/non_existent.bin".to_string(),
            mime_type: "text/plain".to_string(),
            is_sticky: false,
            is_read: false,
            created_at: "2026-11-05T00:00:00Z".to_string(),
        };

        // Expired but STICKY bundle (must NOT be deleted)
        let f2 = FileBundleRecord {
            id: "f2".to_string(),
            source_eid: "dtn://f4jxq/files".to_string(),
            destination_eid: "dtn://stata/files".to_string(),
            creation_timestamp: 1000,
            lifetime_secs: 10,
            expiry_timestamp: 1000 + 10 * 1000,
            size: 10,
            payload_path: "/tmp/non_existent2.bin".to_string(),
            mime_type: "text/plain".to_string(),
            is_sticky: true,
            is_read: false,
            created_at: "2026-11-05T00:00:00Z".to_string(),
        };

        // Future non-sticky bundle (expiry = 50000)
        let f3 = FileBundleRecord {
            id: "f3".to_string(),
            source_eid: "dtn://f4jxq/files".to_string(),
            destination_eid: "dtn://stata/files".to_string(),
            creation_timestamp: 1000,
            lifetime_secs: 49,
            expiry_timestamp: 50000,
            size: 10,
            payload_path: "/tmp/non_existent3.bin".to_string(),
            mime_type: "text/plain".to_string(),
            is_sticky: false,
            is_read: false,
            created_at: "2026-11-05T00:00:00Z".to_string(),
        };

        storage.insert_file_bundle(&f1).unwrap();
        storage.insert_file_bundle(&f2).unwrap();
        storage.insert_file_bundle(&f3).unwrap();

        // Earliest deadline should be 11000 (from f1; f2 is sticky so excluded)
        assert_eq!(storage.get_next_expiration_deadline().unwrap(), Some(11000));

        // Purge at t=20000: only f1 should be purged
        let purged = storage.purge_expired(20000).unwrap();
        assert_eq!(purged, 1);

        assert!(storage.get_file_bundle("f1").unwrap().is_none());
        assert!(storage.get_file_bundle("f2").unwrap().is_some()); // sticky kept!
        assert!(storage.get_file_bundle("f3").unwrap().is_some()); // future kept!

        // Next deadline is now 50000 (from f3)
        assert_eq!(storage.get_next_expiration_deadline().unwrap(), Some(50000));
    }

    #[test]
    fn test_degraded_cases() {
        let storage = InternalStorage::new_in_memory().unwrap();

        // 1. Non-existent ID operations should return safe defaults or not panic
        assert_eq!(storage.get_file_bundle("ghost").unwrap(), None);
        assert!(!storage.delete_file_bundle("ghost").unwrap());
        assert!(storage.toggle_file_sticky("ghost").is_err());
        assert!(storage.toggle_chat_sticky("ghost").is_err());

        // 2. Missing payload file during deletion
        let rec = FileBundleRecord {
            id: "missing-file-test".to_string(),
            source_eid: "dtn://f4jxq/files".to_string(),
            destination_eid: "dtn://stata/files".to_string(),
            creation_timestamp: 1000,
            lifetime_secs: 100,
            expiry_timestamp: 101000,
            size: 5,
            payload_path: "/path/that/does/not/exist/ever.bin".to_string(),
            mime_type: "application/octet-stream".to_string(),
            is_sticky: false,
            is_read: false,
            created_at: "2026-11-05T00:00:00Z".to_string(),
        };
        storage.insert_file_bundle(&rec).unwrap();
        // Deletion must succeed even if physical file is missing
        assert!(storage.delete_file_bundle("missing-file-test").unwrap());

        // 3. Purging missing file does not fail the purge query
        storage.insert_file_bundle(&rec).unwrap();
        let purged = storage.purge_expired(200000).unwrap();
        assert_eq!(purged, 1);
    }

    #[test]
    fn test_dtn_to_unix_epoch_migration_and_persistence() {
        let storage = InternalStorage::new_in_memory().unwrap();

        // Simulate a legacy message stored with DTN BPv7 epoch (relative to 2000-01-01)
        let dtn_ts: i64 = 847_178_744_614;
        let dtn_expiry: i64 = dtn_ts + 86_400_000; // 24h lifetime
        let legacy_msg = ChatMessage {
            id: "legacy-msg-1".to_string(),
            conversation_peer: "dtn://f4jxq/chat".to_string(),
            sender_eid: "dtn://stata/chat".to_string(),
            recipient_eid: "dtn://f4jxq/chat".to_string(),
            content: "Hello legacy!".to_string(),
            direction: "outbound".to_string(),
            status: "sent".to_string(),
            is_read: true,
            is_sticky: false,
            timestamp: dtn_ts,
            expiry_timestamp: dtn_expiry,
            created_at: "2026-11-05T07:25:44Z".to_string(),
        };

        storage.insert_chat_message(&legacy_msg).unwrap();

        // Re-run init_tables() to simulate server reboot / migration
        {
            let conn = storage.conn.lock().unwrap();
            conn.execute("PRAGMA user_version = 0", []).unwrap();
            InternalStorage::init_tables(&conn).unwrap();
        }

        // Check that timestamp & expiry_timestamp were migrated to Unix epoch (offset +946684800000)
        // and that legacy 24h messages are aligned to 1h lifetime (3600000 ms)
        let msgs = storage
            .get_conversation_messages("dtn://f4jxq/chat")
            .unwrap();
        assert_eq!(msgs.len(), 1);
        let expected_unix_ts = dtn_ts + 946_684_800_000;
        let expected_unix_expiry = expected_unix_ts + 3_600_000;
        assert_eq!(msgs[0].timestamp, expected_unix_ts);
        assert_eq!(msgs[0].expiry_timestamp, expected_unix_expiry);

        // At current time (which is less than expected_unix_expiry), purge_expired must NOT delete it
        let current_now_ms = expected_unix_ts + 10_000;
        let purged = storage.purge_expired(current_now_ms).unwrap();
        assert_eq!(purged, 0);

        let msgs_after = storage
            .get_conversation_messages("dtn://f4jxq/chat")
            .unwrap();
        assert_eq!(msgs_after.len(), 1);

        // Once expiry passes, purge_expired deletes it
        let future_now_ms = expected_unix_expiry + 1000;
        let purged = storage.purge_expired(future_now_ms).unwrap();
        assert_eq!(purged, 1);

        let msgs_expired = storage
            .get_conversation_messages("dtn://f4jxq/chat")
            .unwrap();
        assert_eq!(msgs_expired.len(), 0);
    }

    #[test]
    fn test_legacy_24h_outbound_chat_migration() {
        let storage = InternalStorage::new_in_memory().unwrap();

        let now_ms = 1_793_863_544_614_i64; // Unix epoch ms
        let old_24h_expiry = now_ms + 86_400_000; // 24h

        let outbound_msg = ChatMessage {
            id: "outbound-legacy-24h".to_string(),
            conversation_peer: "dtn://stata/chat".to_string(),
            sender_eid: "dtn://station/chat".to_string(),
            recipient_eid: "dtn://stata/chat".to_string(),
            content: "Outbound message from yesterday".to_string(),
            direction: "outbound".to_string(),
            status: "sent".to_string(),
            is_read: true,
            is_sticky: false,
            timestamp: now_ms,
            expiry_timestamp: old_24h_expiry,
            created_at: "2026-11-05T08:00:00Z".to_string(),
        };

        // Sticky message should NOT be modified even if > 3600s
        let sticky_msg = ChatMessage {
            id: "outbound-sticky".to_string(),
            conversation_peer: "dtn://stata/chat".to_string(),
            sender_eid: "dtn://station/chat".to_string(),
            recipient_eid: "dtn://stata/chat".to_string(),
            content: "Sticky message".to_string(),
            direction: "outbound".to_string(),
            status: "sent".to_string(),
            is_read: true,
            is_sticky: true,
            timestamp: now_ms,
            expiry_timestamp: old_24h_expiry,
            created_at: "2026-11-05T08:00:00Z".to_string(),
        };

        storage.insert_chat_message(&outbound_msg).unwrap();
        storage.insert_chat_message(&sticky_msg).unwrap();

        // Run migration via init_tables with user_version = 0
        {
            let conn = storage.conn.lock().unwrap();
            conn.execute("PRAGMA user_version = 0", []).unwrap();
            InternalStorage::init_tables(&conn).unwrap();
        }

        let msgs = storage
            .get_conversation_messages("dtn://stata/chat")
            .unwrap();
        let normal = msgs.iter().find(|m| m.id == "outbound-legacy-24h").unwrap();
        let sticky = msgs.iter().find(|m| m.id == "outbound-sticky").unwrap();

        // The non-sticky message should have been clamped to timestamp + 3600s (3600000ms)
        assert_eq!(normal.expiry_timestamp, now_ms + 3_600_000);
        // The sticky message should have kept its original expiry
        assert_eq!(sticky.expiry_timestamp, old_24h_expiry);

        // Advance time to 1 hour and 1 second after message creation:
        // the normal message must be purged, while sticky remains!
        let purge_time = now_ms + 3_601_000;
        let purged = storage.purge_expired(purge_time).unwrap();
        assert_eq!(purged, 1);

        let remaining = storage
            .get_conversation_messages("dtn://stata/chat")
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "outbound-sticky");

        // Now test that modern messages with 72h or 1 week (604800s) lifetime
        // are PRESERVED and NOT clamped on subsequent server restarts (user_version >= 1)
        let week_secs = 604_800_i64; // 1 week
        let week_msg = ChatMessage {
            id: "outbound-1week".to_string(),
            conversation_peer: "dtn://stata/chat".to_string(),
            sender_eid: "dtn://station/chat".to_string(),
            recipient_eid: "dtn://stata/chat".to_string(),
            content: "Message for the whole week".to_string(),
            direction: "outbound".to_string(),
            status: "sent".to_string(),
            is_read: true,
            is_sticky: false,
            timestamp: now_ms,
            expiry_timestamp: now_ms + (week_secs * 1000),
            created_at: "2026-11-05T08:00:00Z".to_string(),
        };
        storage.insert_chat_message(&week_msg).unwrap();

        // Server restarts and re-runs init_tables (user_version is already 1)
        {
            let conn = storage.conn.lock().unwrap();
            InternalStorage::init_tables(&conn).unwrap();
        }

        let week_fetched = storage
            .get_conversation_messages("dtn://stata/chat")
            .unwrap();
        let week_found = week_fetched
            .iter()
            .find(|m| m.id == "outbound-1week")
            .unwrap();
        assert_eq!(
            week_found.expiry_timestamp,
            now_ms + (week_secs * 1000),
            "1-week message expiry must remain unchanged across server restarts"
        );
    }
}
