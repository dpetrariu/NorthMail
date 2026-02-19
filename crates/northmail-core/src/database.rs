//! Database storage using SQLite

use crate::{CoreError, CoreResult};
use sqlx::{sqlite::{SqliteConnectOptions, SqlitePoolOptions}, Pool, Row, Sqlite};
use std::path::Path;
use tracing::{debug, info, warn};

/// Prepare a user query for FTS5 search with prefix matching
/// Transforms "jenni smith" → "jenni* smith*" for partial word matching
fn prepare_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|word| {
            // Escape special FTS5 characters and add wildcard
            let escaped = word
                .replace('"', "\"\"")
                .replace('*', "")
                .replace(':', " ");
            if escaped.is_empty() {
                String::new()
            } else {
                format!("{}*", escaped)
            }
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Database folder record
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DbFolder {
    pub id: i64,
    pub account_id: String,
    pub name: String,
    pub full_path: String,
    pub folder_type: String,
    pub uidvalidity: Option<i64>,
    pub uid_next: Option<i64>,
    pub message_count: Option<i64>,
    pub unread_count: Option<i64>,
}

/// Attachment metadata from database
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AttachmentMetadata {
    pub id: i64,
    pub message_id: i64,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub content_id: Option<String>,
    pub is_inline: bool,
    pub data: Option<Vec<u8>>,
}

/// Attachment info for saving (without id)
#[derive(Debug, Clone)]
pub struct AttachmentInfo {
    pub filename: String,
    pub mime_type: String,
    pub size: usize,
    pub content_id: Option<String>,
    pub is_inline: bool,
    pub data: Vec<u8>,
}

/// Database message record
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DbMessage {
    pub id: i64,
    pub folder_id: i64,
    pub uid: i64,
    pub message_id: Option<String>,
    pub subject: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub date_sent: Option<String>,
    /// Unix timestamp for proper date sorting
    pub date_epoch: Option<i64>,
    pub snippet: Option<String>,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
    pub size: i64,
    pub maildir_path: Option<String>,
    /// Cached plain text body
    pub body_text: Option<String>,
    /// Cached HTML body
    pub body_html: Option<String>,
}

/// Filter parameters for message queries
#[derive(Debug, Clone, Default)]
pub struct MessageFilter {
    pub unread_only: bool,
    pub starred_only: bool,
    pub has_attachments: bool,
    pub from_contains: String,
    pub date_after: Option<i64>,
    pub date_before: Option<i64>,
}

impl MessageFilter {
    pub fn is_active(&self) -> bool {
        self.unread_only
            || self.starred_only
            || self.has_attachments
            || !self.from_contains.is_empty()
            || self.date_after.is_some()
            || self.date_before.is_some()
    }

    /// Build WHERE clause fragments and return the conditions + a closure to bind params
    fn build_conditions(&self) -> Vec<String> {
        let mut conditions = Vec::new();
        if self.unread_only {
            conditions.push("m.is_read = 0".to_string());
        }
        if self.starred_only {
            conditions.push("m.is_starred = 1".to_string());
        }
        if self.has_attachments {
            conditions.push("m.has_attachments = 1".to_string());
        }
        if !self.from_contains.is_empty() {
            conditions.push("(m.from_name LIKE ? OR m.from_address LIKE ?)".to_string());
        }
        if self.date_after.is_some() {
            conditions.push("m.date_epoch >= ?".to_string());
        }
        if self.date_before.is_some() {
            conditions.push("m.date_epoch <= ?".to_string());
        }
        conditions
    }
}

/// Database connection pool
pub struct Database {
    pool: Pool<Sqlite>,
}

impl Database {
    /// Open or create a database at the given path
    pub async fn open(path: impl AsRef<Path>) -> CoreResult<Self> {
        let path = path.as_ref();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        info!("Opening database at {}", path.display());

        let connect_options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(30));

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(connect_options)
            .await?;

        let db = Self { pool };

        db.initialize().await?;

        Ok(db)
    }

    /// Open an in-memory database (for testing)
    pub async fn open_memory() -> CoreResult<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;

        let db = Self { pool };
        db.initialize().await?;

        Ok(db)
    }

    /// Initialize the database schema
    async fn initialize(&self) -> CoreResult<()> {
        debug!("Initializing database schema");

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS accounts (
                id TEXT PRIMARY KEY,
                email_address TEXT NOT NULL,
                display_name TEXT,
                provider TEXT NOT NULL,
                auth_method TEXT NOT NULL,
                config_json TEXT NOT NULL,
                created_at TEXT DEFAULT (datetime('now')),
                updated_at TEXT DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                full_path TEXT NOT NULL,
                folder_type TEXT NOT NULL DEFAULT 'other',
                uidvalidity INTEGER,
                uid_next INTEGER,
                message_count INTEGER DEFAULT 0,
                unread_count INTEGER DEFAULT 0,
                created_at TEXT DEFAULT (datetime('now')),
                updated_at TEXT DEFAULT (datetime('now')),
                UNIQUE(account_id, full_path)
            );

            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                uid INTEGER NOT NULL,
                message_id TEXT,
                subject TEXT,
                from_address TEXT,
                from_name TEXT,
                to_addresses TEXT,
                cc_addresses TEXT,
                date_sent TEXT,
                date_epoch INTEGER,
                snippet TEXT,
                is_read INTEGER DEFAULT 0,
                is_starred INTEGER DEFAULT 0,
                has_attachments INTEGER DEFAULT 0,
                size INTEGER DEFAULT 0,
                maildir_path TEXT,
                body_text TEXT,
                body_html TEXT,
                created_at TEXT DEFAULT (datetime('now')),
                updated_at TEXT DEFAULT (datetime('now')),
                UNIQUE(folder_id, uid)
            );

            CREATE INDEX IF NOT EXISTS idx_messages_folder ON messages(folder_id);
            CREATE INDEX IF NOT EXISTS idx_messages_date ON messages(date_epoch DESC);
            CREATE INDEX IF NOT EXISTS idx_messages_message_id ON messages(message_id);
            CREATE INDEX IF NOT EXISTS idx_folders_account ON folders(account_id);

            -- Full-text search for messages
            CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                subject,
                from_address,
                from_name,
                snippet,
                content=messages,
                content_rowid=id
            );

            -- Triggers to keep FTS in sync
            CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
                INSERT INTO messages_fts(rowid, subject, from_address, from_name, snippet)
                VALUES (new.id, new.subject, new.from_address, new.from_name, new.snippet);
            END;

            CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, subject, from_address, from_name, snippet)
                VALUES ('delete', old.id, old.subject, old.from_address, old.from_name, old.snippet);
            END;

            CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, subject, from_address, from_name, snippet)
                VALUES ('delete', old.id, old.subject, old.from_address, old.from_name, old.snippet);
                INSERT INTO messages_fts(rowid, subject, from_address, from_name, snippet)
                VALUES (new.id, new.subject, new.from_address, new.from_name, new.snippet);
            END;

            -- Attachment metadata cache (data fetched from IMAP on demand)
            CREATE TABLE IF NOT EXISTS attachments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                filename TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                size INTEGER DEFAULT 0,
                content_id TEXT,
                is_inline INTEGER DEFAULT 0,
                UNIQUE(message_id, filename)
            );

            CREATE INDEX IF NOT EXISTS idx_attachments_message ON attachments(message_id);
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Migration: Add body columns if they don't exist (for existing databases)
        self.migrate_add_body_columns().await?;

        // Migration: Add date_epoch column if it doesn't exist
        self.migrate_add_date_epoch().await?;

        // Migration: Add cc_addresses column if it doesn't exist
        self.migrate_add_cc_addresses().await?;

        // Migration: Add graph_folder_id and graph_message_id columns
        self.migrate_add_graph_ids().await?;

        // Migration: Rebuild FTS index to ensure all messages are indexed
        self.migrate_rebuild_fts().await?;

        info!("Database schema initialized");
        Ok(())
    }

    /// Add body_text and body_html columns if they don't exist
    async fn migrate_add_body_columns(&self) -> CoreResult<()> {
        // Check if columns exist by trying to select them
        let result = sqlx::query("SELECT body_text FROM messages LIMIT 1")
            .fetch_optional(&self.pool)
            .await;

        if result.is_err() {
            // Columns don't exist, add them
            debug!("Migrating database: adding body_text and body_html columns");
            if let Err(e) = sqlx::query("ALTER TABLE messages ADD COLUMN body_text TEXT")
                .execute(&self.pool)
                .await
            {
                if !e.to_string().contains("duplicate column") {
                    warn!("Migration error adding body_text column: {}", e);
                }
            }
            if let Err(e) = sqlx::query("ALTER TABLE messages ADD COLUMN body_html TEXT")
                .execute(&self.pool)
                .await
            {
                if !e.to_string().contains("duplicate column") {
                    warn!("Migration error adding body_html column: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Add date_epoch column if it doesn't exist
    async fn migrate_add_date_epoch(&self) -> CoreResult<()> {
        let result = sqlx::query("SELECT date_epoch FROM messages LIMIT 1")
            .fetch_optional(&self.pool)
            .await;

        if result.is_err() {
            debug!("Migrating database: adding date_epoch column");
            if let Err(e) = sqlx::query("ALTER TABLE messages ADD COLUMN date_epoch INTEGER")
                .execute(&self.pool)
                .await
            {
                if !e.to_string().contains("duplicate column") {
                    warn!("Migration error adding date_epoch column: {}", e);
                }
            }
            // Create index for sorting
            if let Err(e) = sqlx::query("CREATE INDEX IF NOT EXISTS idx_messages_date_epoch ON messages(date_epoch DESC)")
                .execute(&self.pool)
                .await
            {
                warn!("Migration error creating date_epoch index: {}", e);
            }
        }

        Ok(())
    }

    /// Add cc_addresses column if it doesn't exist
    async fn migrate_add_cc_addresses(&self) -> CoreResult<()> {
        let result = sqlx::query("SELECT cc_addresses FROM messages LIMIT 1")
            .fetch_optional(&self.pool)
            .await;

        if result.is_err() {
            debug!("Migrating database: adding cc_addresses column");
            if let Err(e) = sqlx::query("ALTER TABLE messages ADD COLUMN cc_addresses TEXT")
                .execute(&self.pool)
                .await
            {
                if !e.to_string().contains("duplicate column") {
                    warn!("Migration error adding cc_addresses column: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Add graph_folder_id and graph_message_id columns for Microsoft Graph API support
    async fn migrate_add_graph_ids(&self) -> CoreResult<()> {
        // Check if graph_folder_id column exists on folders
        let result = sqlx::query("SELECT graph_folder_id FROM folders LIMIT 1")
            .fetch_optional(&self.pool)
            .await;

        if result.is_err() {
            debug!("Migrating database: adding graph_folder_id column to folders");
            if let Err(e) = sqlx::query("ALTER TABLE folders ADD COLUMN graph_folder_id TEXT")
                .execute(&self.pool)
                .await
            {
                if !e.to_string().contains("duplicate column") {
                    warn!("Migration error adding graph_folder_id column: {}", e);
                }
            }
        }

        // Check if graph_message_id column exists on messages
        let result = sqlx::query("SELECT graph_message_id FROM messages LIMIT 1")
            .fetch_optional(&self.pool)
            .await;

        if result.is_err() {
            debug!("Migrating database: adding graph_message_id column to messages");
            if let Err(e) = sqlx::query("ALTER TABLE messages ADD COLUMN graph_message_id TEXT")
                .execute(&self.pool)
                .await
            {
                if !e.to_string().contains("duplicate column") {
                    warn!("Migration error adding graph_message_id column: {}", e);
                }
            }
            // Create index for graph_message_id lookups
            if let Err(e) = sqlx::query("CREATE INDEX IF NOT EXISTS idx_messages_graph_id ON messages(graph_message_id)")
                .execute(&self.pool)
                .await
            {
                warn!("Migration error creating graph_message_id index: {}", e);
            }
        }

        // Migration: add data BLOB column to attachments table
        if let Err(e) = sqlx::query("ALTER TABLE attachments ADD COLUMN data BLOB")
            .execute(&self.pool)
            .await
        {
            if !e.to_string().contains("duplicate column") {
                warn!("Migration error adding data column to attachments: {}", e);
            }
        }

        Ok(())
    }

    /// Rebuild FTS index to ensure all messages are indexed
    /// This is needed because messages inserted before the FTS table existed won't be in the index
    async fn migrate_rebuild_fts(&self) -> CoreResult<()> {
        // Check if there are messages not in FTS by comparing counts
        let msg_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);

        let fts_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages_fts")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);

        if msg_count > fts_count {
            info!(
                "FTS index incomplete ({} messages, {} indexed). Rebuilding...",
                msg_count, fts_count
            );

            // Rebuild FTS index by re-inserting all messages
            // First delete all FTS entries
            if let Err(e) = sqlx::query("DELETE FROM messages_fts")
                .execute(&self.pool)
                .await
            {
                warn!("Failed to clear FTS index: {}", e);
            }

            // Then repopulate from messages table
            sqlx::query(
                r#"
                INSERT INTO messages_fts(rowid, subject, from_address, from_name, snippet)
                SELECT id, subject, from_address, from_name, snippet FROM messages
                "#,
            )
            .execute(&self.pool)
            .await?;

            info!("FTS index rebuilt with {} messages", msg_count);
        }

        Ok(())
    }

    /// Insert or update an account
    pub async fn upsert_account(&self, account: &crate::Account) -> CoreResult<()> {
        let auth_method = serde_json::to_string(&account.auth_method)
            .map_err(|e| CoreError::DatabaseError(e.to_string()))?;
        let config_json = serde_json::to_string(&account.config)
            .map_err(|e| CoreError::DatabaseError(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO accounts (id, email_address, display_name, provider, auth_method, config_json)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                email_address = excluded.email_address,
                display_name = excluded.display_name,
                provider = excluded.provider,
                auth_method = excluded.auth_method,
                config_json = excluded.config_json,
                updated_at = datetime('now')
            "#,
        )
        .bind(&account.id)
        .bind(&account.email)
        .bind(&account.display_name)
        .bind(&account.provider)
        .bind(&auth_method)
        .bind(&config_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get all accounts
    pub async fn get_accounts(&self) -> CoreResult<Vec<crate::Account>> {
        #[derive(sqlx::FromRow)]
        struct AccountRow {
            id: String,
            email_address: String,
            display_name: Option<String>,
            provider: String,
            auth_method: String,
            config_json: String,
        }

        let rows: Vec<AccountRow> =
            sqlx::query_as("SELECT id, email_address, display_name, provider, auth_method, config_json FROM accounts")
                .fetch_all(&self.pool)
                .await?;

        let mut accounts = Vec::new();
        for row in rows {
            let auth_method: northmail_auth::AuthMethod = serde_json::from_str(&row.auth_method)
                .map_err(|e| CoreError::DatabaseError(e.to_string()))?;
            let config: crate::AccountConfig = serde_json::from_str(&row.config_json)
                .map_err(|e| CoreError::DatabaseError(e.to_string()))?;

            accounts.push(crate::Account {
                id: row.id,
                email: row.email_address,
                display_name: row.display_name,
                provider: row.provider,
                auth_method,
                config,
            });
        }

        Ok(accounts)
    }

    /// Delete an account
    pub async fn delete_account(&self, account_id: &str) -> CoreResult<()> {
        sqlx::query("DELETE FROM accounts WHERE id = ?")
            .bind(account_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Insert or update a folder
    pub async fn upsert_folder(
        &self,
        account_id: &str,
        name: &str,
        full_path: &str,
        folder_type: &str,
    ) -> CoreResult<i64> {
        self.upsert_folder_with_counts(account_id, name, full_path, folder_type, None, None)
            .await
    }

    /// Insert or update a folder with message/unread counts
    pub async fn upsert_folder_with_counts(
        &self,
        account_id: &str,
        name: &str,
        full_path: &str,
        folder_type: &str,
        message_count: Option<i64>,
        unread_count: Option<i64>,
    ) -> CoreResult<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO folders (account_id, name, full_path, folder_type, message_count, unread_count)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(account_id, full_path) DO UPDATE SET
                name = excluded.name,
                folder_type = excluded.folder_type,
                message_count = COALESCE(excluded.message_count, folders.message_count),
                unread_count = COALESCE(excluded.unread_count, folders.unread_count),
                updated_at = datetime('now')
            RETURNING id
            "#,
        )
        .bind(account_id)
        .bind(name)
        .bind(full_path)
        .bind(folder_type)
        .bind(message_count)
        .bind(unread_count)
        .fetch_one(&self.pool)
        .await?;

        Ok(result.get::<i64, _>("id"))
    }

    /// Get folders for an account
    pub async fn get_folders(&self, account_id: &str) -> CoreResult<Vec<DbFolder>> {
        let folders = sqlx::query_as::<_, DbFolder>(
            "SELECT id, account_id, name, full_path, folder_type, uidvalidity, uid_next, message_count, unread_count FROM folders WHERE account_id = ? ORDER BY folder_type, name",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(folders)
    }

    /// Update folder sync state
    pub async fn update_folder_sync(
        &self,
        folder_id: i64,
        uidvalidity: i64,
        uid_next: i64,
        message_count: i64,
        unread_count: i64,
    ) -> CoreResult<()> {
        sqlx::query(
            r#"
            UPDATE folders SET
                uidvalidity = ?,
                uid_next = ?,
                message_count = ?,
                unread_count = ?,
                updated_at = datetime('now')
            WHERE id = ?
            "#,
        )
        .bind(uidvalidity)
        .bind(uid_next)
        .bind(message_count)
        .bind(unread_count)
        .bind(folder_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Upsert a folder with Graph API folder ID
    pub async fn upsert_folder_graph(
        &self,
        account_id: &str,
        name: &str,
        full_path: &str,
        folder_type: &str,
        message_count: Option<i64>,
        unread_count: Option<i64>,
        graph_folder_id: &str,
    ) -> CoreResult<i64> {
        let id = self
            .upsert_folder_with_counts(
                account_id,
                name,
                full_path,
                folder_type,
                message_count,
                unread_count,
            )
            .await?;

        // Update the graph_folder_id
        sqlx::query("UPDATE folders SET graph_folder_id = ? WHERE id = ?")
            .bind(graph_folder_id)
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(id)
    }

    /// Get graph_message_id for a message by folder_id and uid
    pub async fn get_graph_message_id(&self, folder_id: i64, uid: i64) -> CoreResult<Option<String>> {
        let result = sqlx::query_scalar::<_, Option<String>>(
            "SELECT graph_message_id FROM messages WHERE folder_id = ? AND uid = ?",
        )
        .bind(folder_id)
        .bind(uid)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.flatten())
    }

    /// Get graph_folder_id for a folder by its DB id
    pub async fn get_graph_folder_id(&self, folder_id: i64) -> CoreResult<Option<String>> {
        let result = sqlx::query_scalar::<_, Option<String>>(
            "SELECT graph_folder_id FROM folders WHERE id = ?",
        )
        .bind(folder_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.flatten())
    }

    /// Upsert a message with a Graph message ID
    pub async fn upsert_message_graph(
        &self,
        folder_id: i64,
        msg: &DbMessage,
        graph_message_id: &str,
    ) -> CoreResult<i64> {
        let id = self.upsert_message(folder_id, msg).await?;

        sqlx::query("UPDATE messages SET graph_message_id = ? WHERE id = ?")
            .bind(graph_message_id)
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(id)
    }

    /// Batch upsert messages with Graph message IDs
    pub async fn upsert_messages_batch_graph(
        &self,
        folder_id: i64,
        messages: &[(DbMessage, String)], // (message, graph_message_id)
    ) -> CoreResult<usize> {
        let mut count = 0;

        // Process in chunks to avoid holding write lock too long
        for chunk in messages.chunks(50) {
            let mut tx = self.pool.begin().await?;

            for (msg, graph_id) in chunk {
                let result = sqlx::query(
                    r#"
                    INSERT INTO messages (
                        folder_id, uid, message_id, subject, from_address, from_name,
                        to_addresses, cc_addresses, date_sent, date_epoch, snippet, is_read, is_starred,
                        has_attachments, size, maildir_path, graph_message_id
                    )
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(folder_id, uid) DO UPDATE SET
                        message_id = excluded.message_id,
                        subject = excluded.subject,
                        from_address = excluded.from_address,
                        from_name = excluded.from_name,
                        to_addresses = excluded.to_addresses,
                        cc_addresses = excluded.cc_addresses,
                        date_sent = excluded.date_sent,
                        date_epoch = excluded.date_epoch,
                        snippet = excluded.snippet,
                        is_read = excluded.is_read,
                        is_starred = excluded.is_starred,
                        has_attachments = excluded.has_attachments,
                        size = excluded.size,
                        maildir_path = excluded.maildir_path,
                        graph_message_id = excluded.graph_message_id,
                        updated_at = datetime('now')
                    "#,
                )
                .bind(folder_id)
                .bind(msg.uid)
                .bind(&msg.message_id)
                .bind(&msg.subject)
                .bind(&msg.from_address)
                .bind(&msg.from_name)
                .bind(&msg.to_addresses)
                .bind(&msg.cc_addresses)
                .bind(&msg.date_sent)
                .bind(msg.date_epoch)
                .bind(&msg.snippet)
                .bind(msg.is_read)
                .bind(msg.is_starred)
                .bind(msg.has_attachments)
                .bind(msg.size)
                .bind(&msg.maildir_path)
                .bind(graph_id)
                .execute(&mut *tx)
                .await;

                match result {
                    Ok(_) => count += 1,
                    Err(e) => {
                        tracing::warn!("Failed to upsert graph message uid={}: {}", msg.uid, e);
                    }
                }
            }

            tx.commit().await?;
        }

        Ok(count)
    }

    /// Look up the Graph message ID for a message by its UID hash
    pub async fn get_graph_message_id_by_uid(&self, uid: i64) -> CoreResult<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT graph_message_id FROM messages WHERE uid = ? AND graph_message_id IS NOT NULL LIMIT 1",
        )
        .bind(uid)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.0))
    }

    /// Look up the Graph message ID for a message by account, folder, and UID
    pub async fn get_graph_message_id_for_folder_uid(
        &self,
        account_id: &str,
        folder_path: &str,
        uid: i64,
    ) -> CoreResult<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            r#"
            SELECT m.graph_message_id FROM messages m
            JOIN folders f ON m.folder_id = f.id
            WHERE f.account_id = ? AND f.full_path = ? AND m.uid = ?
            AND m.graph_message_id IS NOT NULL
            LIMIT 1
            "#,
        )
        .bind(account_id)
        .bind(folder_path)
        .bind(uid)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.0))
    }

    /// Insert or update messages in a batch (wrapped in a transaction for performance)
    pub async fn upsert_messages_batch(
        &self,
        folder_id: i64,
        messages: &[DbMessage],
    ) -> CoreResult<usize> {
        let mut count = 0;

        // Process in chunks to avoid holding write lock too long
        for chunk in messages.chunks(50) {
            let mut tx = self.pool.begin().await?;

            for msg in chunk {
                let result = sqlx::query(
                    r#"
                    INSERT INTO messages (
                        folder_id, uid, message_id, subject, from_address, from_name,
                        to_addresses, cc_addresses, date_sent, date_epoch, snippet, is_read, is_starred,
                        has_attachments, size, maildir_path
                    )
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(folder_id, uid) DO UPDATE SET
                        message_id = excluded.message_id,
                        subject = excluded.subject,
                        from_address = excluded.from_address,
                        from_name = excluded.from_name,
                        to_addresses = excluded.to_addresses,
                        cc_addresses = excluded.cc_addresses,
                        date_sent = excluded.date_sent,
                        date_epoch = excluded.date_epoch,
                        snippet = excluded.snippet,
                        is_read = excluded.is_read,
                        is_starred = excluded.is_starred,
                        has_attachments = excluded.has_attachments,
                        size = excluded.size,
                        maildir_path = excluded.maildir_path,
                        updated_at = datetime('now')
                    "#,
                )
                .bind(folder_id)
                .bind(msg.uid)
                .bind(&msg.message_id)
                .bind(&msg.subject)
                .bind(&msg.from_address)
                .bind(&msg.from_name)
                .bind(&msg.to_addresses)
                .bind(&msg.cc_addresses)
                .bind(&msg.date_sent)
                .bind(msg.date_epoch)
                .bind(&msg.snippet)
                .bind(msg.is_read)
                .bind(msg.is_starred)
                .bind(msg.has_attachments)
                .bind(msg.size)
                .bind(&msg.maildir_path)
                .execute(&mut *tx)
                .await;

                match result {
                    Ok(_) => count += 1,
                    Err(e) => {
                        tracing::warn!("Failed to upsert message uid={}: {}", msg.uid, e);
                    }
                }
            }

            tx.commit().await?;
        }

        Ok(count)
    }

    /// Insert or update a message
    pub async fn upsert_message(&self, folder_id: i64, msg: &DbMessage) -> CoreResult<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO messages (
                folder_id, uid, message_id, subject, from_address, from_name,
                to_addresses, cc_addresses, date_sent, date_epoch, snippet, is_read, is_starred,
                has_attachments, size, maildir_path
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(folder_id, uid) DO UPDATE SET
                message_id = excluded.message_id,
                subject = excluded.subject,
                from_address = excluded.from_address,
                from_name = excluded.from_name,
                to_addresses = excluded.to_addresses,
                cc_addresses = excluded.cc_addresses,
                date_sent = excluded.date_sent,
                date_epoch = excluded.date_epoch,
                snippet = excluded.snippet,
                is_read = excluded.is_read,
                is_starred = excluded.is_starred,
                has_attachments = excluded.has_attachments,
                size = excluded.size,
                maildir_path = excluded.maildir_path,
                updated_at = datetime('now')
            RETURNING id
            "#,
        )
        .bind(folder_id)
        .bind(msg.uid)
        .bind(&msg.message_id)
        .bind(&msg.subject)
        .bind(&msg.from_address)
        .bind(&msg.from_name)
        .bind(&msg.to_addresses)
        .bind(&msg.cc_addresses)
        .bind(&msg.date_sent)
        .bind(msg.date_epoch)
        .bind(&msg.snippet)
        .bind(msg.is_read)
        .bind(msg.is_starred)
        .bind(msg.has_attachments)
        .bind(msg.size)
        .bind(&msg.maildir_path)
        .fetch_one(&self.pool)
        .await?;

        Ok(result.get::<i64, _>("id"))
    }

    /// Get messages for a folder
    pub async fn get_messages(
        &self,
        folder_id: i64,
        limit: i64,
        offset: i64,
    ) -> CoreResult<Vec<DbMessage>> {
        let messages = sqlx::query_as::<_, DbMessage>(
            r#"
            SELECT id, folder_id, uid, message_id, subject, from_address, from_name,
                   to_addresses, cc_addresses, date_sent, date_epoch, snippet, is_read, is_starred,
                   has_attachments, size, maildir_path, body_text, body_html
            FROM messages
            WHERE folder_id = ?
            ORDER BY date_epoch DESC, uid DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(folder_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(messages)
    }

    /// Get message body by folder and UID
    pub async fn get_message_body(
        &self,
        folder_id: i64,
        uid: i64,
    ) -> CoreResult<Option<(Option<String>, Option<String>)>> {
        let result = sqlx::query(
            "SELECT body_text, body_html FROM messages WHERE folder_id = ? AND uid = ?",
        )
        .bind(folder_id)
        .bind(uid)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(|row| {
            (
                row.get::<Option<String>, _>("body_text"),
                row.get::<Option<String>, _>("body_html"),
            )
        }))
    }

    /// Save message body
    pub async fn save_message_body(
        &self,
        folder_id: i64,
        uid: i64,
        body_text: Option<&str>,
        body_html: Option<&str>,
    ) -> CoreResult<()> {
        sqlx::query(
            r#"
            UPDATE messages
            SET body_text = ?, body_html = ?, updated_at = datetime('now')
            WHERE folder_id = ? AND uid = ?
            "#,
        )
        .bind(body_text)
        .bind(body_html)
        .bind(folder_id)
        .bind(uid)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get attachment metadata for a message
    pub async fn get_message_attachments(
        &self,
        folder_id: i64,
        uid: i64,
    ) -> CoreResult<Vec<AttachmentMetadata>> {
        // First get the message id
        let msg_id: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM messages WHERE folder_id = ? AND uid = ?",
        )
        .bind(folder_id)
        .bind(uid)
        .fetch_optional(&self.pool)
        .await?;

        let Some((message_id,)) = msg_id else {
            return Ok(Vec::new());
        };

        let attachments = sqlx::query_as::<_, AttachmentMetadata>(
            "SELECT id, message_id, filename, mime_type, size, content_id, is_inline, data FROM attachments WHERE message_id = ?",
        )
        .bind(message_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(attachments)
    }

    /// Save attachment metadata for a message (replaces existing)
    pub async fn save_message_attachments(
        &self,
        folder_id: i64,
        uid: i64,
        attachments: &[AttachmentInfo],
    ) -> CoreResult<()> {
        // First get the message id
        let msg_id: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM messages WHERE folder_id = ? AND uid = ?",
        )
        .bind(folder_id)
        .bind(uid)
        .fetch_optional(&self.pool)
        .await?;

        let Some((message_id,)) = msg_id else {
            return Ok(());
        };

        // Delete existing attachments
        sqlx::query("DELETE FROM attachments WHERE message_id = ?")
            .bind(message_id)
            .execute(&self.pool)
            .await?;

        // Insert new attachments
        for att in attachments {
            sqlx::query(
                r#"
                INSERT INTO attachments (message_id, filename, mime_type, size, content_id, is_inline, data)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(message_id)
            .bind(&att.filename)
            .bind(&att.mime_type)
            .bind(att.size as i64)
            .bind(&att.content_id)
            .bind(att.is_inline)
            .bind(if att.data.is_empty() { None } else { Some(&att.data) })
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Get message UIDs that need body prefetch (no cached body, within last N days)
    /// Returns (uid, is_unread) pairs, prioritizing unread messages
    pub async fn get_messages_needing_body_prefetch(
        &self,
        folder_id: i64,
        days: i64,
        limit: i64,
    ) -> CoreResult<Vec<(i64, bool)>> {
        let cutoff_epoch = chrono::Utc::now().timestamp() - (days * 24 * 60 * 60);

        let results: Vec<(i64, bool)> = sqlx::query_as(
            r#"
            SELECT uid, is_read = 0 as is_unread
            FROM messages
            WHERE folder_id = ?
              AND (body_text IS NULL AND body_html IS NULL)
              AND (date_epoch IS NULL OR date_epoch >= ?)
            ORDER BY is_read ASC, date_epoch DESC
            LIMIT ?
            "#,
        )
        .bind(folder_id)
        .bind(cutoff_epoch)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(results)
    }

    /// Search messages using FTS
    pub async fn search_messages(&self, query: &str, limit: i64) -> CoreResult<Vec<DbMessage>> {
        let fts_query = prepare_fts_query(query);
        debug!("FTS search: '{}' -> '{}'", query, fts_query);

        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let messages = sqlx::query_as::<_, DbMessage>(
            r#"
            SELECT m.id, m.folder_id, m.uid, m.message_id, m.subject, m.from_address,
                   m.from_name, m.to_addresses, m.cc_addresses, m.date_sent, m.date_epoch, m.snippet,
                   m.is_read, m.is_starred, m.has_attachments, m.size, m.maildir_path,
                   m.body_text, m.body_html
            FROM messages m
            JOIN messages_fts fts ON m.id = fts.rowid
            WHERE messages_fts MATCH ?
            ORDER BY rank
            LIMIT ?
            "#,
        )
        .bind(&fts_query)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(messages)
    }

    /// Search messages using FTS within a specific folder
    pub async fn search_messages_in_folder(
        &self,
        folder_id: i64,
        query: &str,
        limit: i64,
    ) -> CoreResult<Vec<DbMessage>> {
        let fts_query = prepare_fts_query(query);
        debug!("FTS folder search: '{}' -> '{}' (folder_id={})", query, fts_query, folder_id);

        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let messages = sqlx::query_as::<_, DbMessage>(
            r#"
            SELECT m.id, m.folder_id, m.uid, m.message_id, m.subject, m.from_address,
                   m.from_name, m.to_addresses, m.cc_addresses, m.date_sent, m.date_epoch, m.snippet,
                   m.is_read, m.is_starred, m.has_attachments, m.size, m.maildir_path,
                   m.body_text, m.body_html
            FROM messages m
            JOIN messages_fts fts ON m.id = fts.rowid
            WHERE messages_fts MATCH ? AND m.folder_id = ?
            ORDER BY m.date_epoch DESC
            LIMIT ?
            "#,
        )
        .bind(&fts_query)
        .bind(folder_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(messages)
    }

    /// Update message read status
    pub async fn set_message_read(&self, message_id: i64, is_read: bool) -> CoreResult<()> {
        sqlx::query("UPDATE messages SET is_read = ?, updated_at = datetime('now') WHERE id = ?")
            .bind(is_read)
            .bind(message_id)
            .execute(&self.pool)
            .await?;

        // Update the folder's unread_count
        let delta: i64 = if is_read { -1 } else { 1 };
        sqlx::query(
            r#"
            UPDATE folders SET unread_count = MAX(0, COALESCE(unread_count, 0) + ?)
            WHERE id = (SELECT folder_id FROM messages WHERE id = ?)
            "#,
        )
        .bind(delta)
        .bind(message_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Update read status by folder_id + UID (for Graph messages where DB id may be 0)
    pub async fn set_message_read_by_uid(&self, folder_id: i64, uid: i64, is_read: bool) -> CoreResult<()> {
        sqlx::query("UPDATE messages SET is_read = ?, updated_at = datetime('now') WHERE folder_id = ? AND uid = ?")
            .bind(is_read)
            .bind(folder_id)
            .bind(uid)
            .execute(&self.pool)
            .await?;

        // Update the folder's unread_count
        let delta: i64 = if is_read { -1 } else { 1 };
        sqlx::query(
            "UPDATE folders SET unread_count = MAX(0, COALESCE(unread_count, 0) + ?) WHERE id = ?",
        )
        .bind(delta)
        .bind(folder_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Update message starred status
    pub async fn set_message_starred(&self, message_id: i64, is_starred: bool) -> CoreResult<()> {
        sqlx::query(
            "UPDATE messages SET is_starred = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(is_starred)
        .bind(message_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Update message has_attachments flag (corrected after body parsing)
    pub async fn set_message_has_attachments_by_uid(
        &self,
        folder_id: i64,
        uid: i64,
        has_attachments: bool,
    ) -> CoreResult<()> {
        sqlx::query(
            "UPDATE messages SET has_attachments = ?, updated_at = datetime('now') WHERE folder_id = ? AND uid = ?",
        )
        .bind(has_attachments)
        .bind(folder_id)
        .bind(uid)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Get the has_attachments flag for a message
    pub async fn get_message_has_attachments(
        &self,
        folder_id: i64,
        uid: i64,
    ) -> CoreResult<bool> {
        let result: Option<(bool,)> = sqlx::query_as(
            "SELECT has_attachments FROM messages WHERE folder_id = ? AND uid = ?",
        )
        .bind(folder_id)
        .bind(uid)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(|(v,)| v).unwrap_or(false))
    }

    /// Delete a single message by ID
    pub async fn delete_message(&self, message_id: i64) -> CoreResult<()> {
        sqlx::query("DELETE FROM messages WHERE id = ?")
            .bind(message_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Delete a single message by folder_id and IMAP UID
    /// More reliable than delete_message() since the UID is always known from IMAP
    pub async fn delete_message_by_uid(&self, folder_id: i64, uid: i64) -> CoreResult<()> {
        // Check if message is unread before deleting, to update folder count
        let is_unread: bool = sqlx::query_scalar(
            "SELECT CASE WHEN is_read = 0 THEN 1 ELSE 0 END FROM messages WHERE folder_id = ? AND uid = ?",
        )
        .bind(folder_id)
        .bind(uid)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(false);

        sqlx::query("DELETE FROM messages WHERE folder_id = ? AND uid = ?")
            .bind(folder_id)
            .bind(uid)
            .execute(&self.pool)
            .await?;

        // Decrement unread count if the deleted message was unread
        if is_unread {
            sqlx::query(
                "UPDATE folders SET unread_count = MAX(0, COALESCE(unread_count, 0) - 1) WHERE id = ?",
            )
            .bind(folder_id)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Delete messages by UID (for sync)
    pub async fn delete_messages_not_in_uids(
        &self,
        folder_id: i64,
        uids: &[i64],
    ) -> CoreResult<u64> {
        if uids.is_empty() {
            let result = sqlx::query("DELETE FROM messages WHERE folder_id = ?")
                .bind(folder_id)
                .execute(&self.pool)
                .await?;
            return Ok(result.rows_affected());
        }

        // Use a temp table within a single connection to avoid SQLite's variable limit
        let mut conn = self.pool.acquire().await?;

        sqlx::query("CREATE TEMP TABLE IF NOT EXISTS _valid_uids (uid INTEGER PRIMARY KEY)")
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM _valid_uids")
            .execute(&mut *conn)
            .await?;

        // Insert UIDs in chunks of 500
        for chunk in uids.chunks(500) {
            let placeholders: String = chunk.iter().map(|_| "(?)").collect::<Vec<_>>().join(",");
            let query = format!("INSERT OR IGNORE INTO _valid_uids (uid) VALUES {}", placeholders);
            let mut q = sqlx::query(&query);
            for uid in chunk {
                q = q.bind(uid);
            }
            q.execute(&mut *conn).await?;
        }

        let result = sqlx::query(
            "DELETE FROM messages WHERE folder_id = ? AND uid NOT IN (SELECT uid FROM _valid_uids)"
        )
        .bind(folder_id)
        .execute(&mut *conn)
        .await?;

        sqlx::query("DELETE FROM _valid_uids").execute(&mut *conn).await?;

        Ok(result.rows_affected())
    }

    /// Get a folder by account ID and path
    pub async fn get_folder_by_path(
        &self,
        account_id: &str,
        folder_path: &str,
    ) -> CoreResult<Option<DbFolder>> {
        let folder = sqlx::query_as::<_, DbFolder>(
            r#"
            SELECT id, account_id, name, full_path, folder_type, uidvalidity,
                   uid_next, message_count, unread_count
            FROM folders
            WHERE account_id = ? AND full_path = ?
            "#,
        )
        .bind(account_id)
        .bind(folder_path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(folder)
    }

    /// Get folder ID by path, creating the folder if it doesn't exist
    pub async fn get_or_create_folder_id(
        &self,
        account_id: &str,
        folder_path: &str,
    ) -> CoreResult<i64> {
        // Try to find existing folder
        if let Some(folder) = self.get_folder_by_path(account_id, folder_path).await? {
            return Ok(folder.id);
        }

        // Create new folder entry
        let folder_name = folder_path.rsplit('/').next().unwrap_or(folder_path);
        let folder_type = Self::guess_folder_type(folder_path);
        self.upsert_folder(account_id, folder_name, folder_path, &folder_type)
            .await
    }

    /// Delete a folder and all its messages from the database
    pub async fn delete_folder_by_path(
        &self,
        account_id: &str,
        full_path: &str,
    ) -> CoreResult<()> {
        // Delete the folder and all child folders (messages cascade via FK)
        sqlx::query(
            "DELETE FROM folders WHERE account_id = ? AND (full_path = ? OR full_path LIKE ? OR full_path LIKE ?)",
        )
        .bind(account_id)
        .bind(full_path)
        .bind(format!("{}/%", full_path))
        .bind(format!("{}.%", full_path))
        .execute(&self.pool)
        .await?;

        debug!("Deleted folder {} (and children) for account {}", full_path, account_id);
        Ok(())
    }

    /// Delete all messages in a folder (by account_id and folder path), keeping the folder itself
    pub async fn delete_messages_in_folder(
        &self,
        account_id: &str,
        folder_path: &str,
    ) -> CoreResult<u64> {
        let result = sqlx::query(
            r#"
            DELETE FROM messages WHERE folder_id IN (
                SELECT id FROM folders WHERE account_id = ? AND full_path = ?
            )
            "#,
        )
        .bind(account_id)
        .bind(folder_path)
        .execute(&self.pool)
        .await?;

        // Reset folder counts to 0
        sqlx::query(
            "UPDATE folders SET message_count = 0, unread_count = 0 WHERE account_id = ? AND full_path = ?",
        )
        .bind(account_id)
        .bind(folder_path)
        .execute(&self.pool)
        .await?;

        debug!("Deleted {} messages in folder {} for account {}", result.rows_affected(), folder_path, account_id);
        Ok(result.rows_affected())
    }

    /// Check if a message is unread (by folder_id + uid)
    pub async fn is_message_unread(&self, folder_id: i64, uid: i64) -> CoreResult<bool> {
        let is_unread: bool = sqlx::query_scalar(
            "SELECT CASE WHEN is_read = 0 THEN 1 ELSE 0 END FROM messages WHERE folder_id = ? AND uid = ?",
        )
        .bind(folder_id)
        .bind(uid)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(false);
        Ok(is_unread)
    }

    /// Increment unread count for a folder (used when moving an unread message into a folder)
    pub async fn increment_folder_unread(
        &self,
        account_id: &str,
        folder_path: &str,
    ) -> CoreResult<()> {
        sqlx::query(
            "UPDATE folders SET unread_count = COALESCE(unread_count, 0) + 1 WHERE account_id = ? AND full_path = ?",
        )
        .bind(account_id)
        .bind(folder_path)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete folders not in the given set of paths (cleanup stale folders after sync)
    pub async fn delete_stale_folders(
        &self,
        account_id: &str,
        valid_paths: &[String],
    ) -> CoreResult<u64> {
        if valid_paths.is_empty() {
            return Ok(0);
        }
        // Build a query with placeholders for the valid paths
        let placeholders: Vec<&str> = valid_paths.iter().map(|_| "?").collect();
        let query = format!(
            "DELETE FROM folders WHERE account_id = ? AND full_path NOT IN ({})",
            placeholders.join(", ")
        );
        let mut q = sqlx::query(&query).bind(account_id);
        for path in valid_paths {
            q = q.bind(path);
        }
        let result = q.execute(&self.pool).await?;
        Ok(result.rows_affected())
    }

    /// Rename a folder path in the database, updating child folder paths too
    pub async fn rename_folder_path(
        &self,
        account_id: &str,
        old_path: &str,
        new_path: &str,
    ) -> CoreResult<()> {
        let new_name = new_path.rsplit('/').next()
            .or_else(|| new_path.rsplit('.').next())
            .unwrap_or(new_path);

        // Update the folder itself
        sqlx::query(
            "UPDATE folders SET full_path = ?, name = ? WHERE account_id = ? AND full_path = ?",
        )
        .bind(new_path)
        .bind(new_name)
        .bind(account_id)
        .bind(old_path)
        .execute(&self.pool)
        .await?;

        // Update child folders whose paths start with old_path + delimiter
        // Try both "/" and "." delimiters
        for delim in &["/", "."] {
            let old_prefix = format!("{}{}", old_path, delim);
            let new_prefix = format!("{}{}", new_path, delim);

            // Find child folders
            let children: Vec<(i64, String)> = sqlx::query_as(
                "SELECT id, full_path FROM folders WHERE account_id = ? AND full_path LIKE ?",
            )
            .bind(account_id)
            .bind(format!("{}%", old_prefix))
            .fetch_all(&self.pool)
            .await?;

            for (child_id, child_path) in children {
                let updated_path = format!("{}{}", new_prefix, &child_path[old_prefix.len()..]);
                let child_name = updated_path.rsplit(*delim).next().unwrap_or(&updated_path);
                sqlx::query("UPDATE folders SET full_path = ?, name = ? WHERE id = ?")
                    .bind(&updated_path)
                    .bind(child_name)
                    .bind(child_id)
                    .execute(&self.pool)
                    .await?;
            }
        }

        debug!(
            "Renamed folder {} -> {} for account {}",
            old_path, new_path, account_id
        );
        Ok(())
    }

    /// Get the graph_folder_id for a folder identified by account_id and full_path
    pub async fn get_graph_folder_id_by_path(
        &self,
        account_id: &str,
        folder_path: &str,
    ) -> CoreResult<Option<String>> {
        let result = sqlx::query_scalar::<_, Option<String>>(
            "SELECT graph_folder_id FROM folders WHERE account_id = ? AND full_path = ?",
        )
        .bind(account_id)
        .bind(folder_path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.flatten())
    }

    /// Get all message UIDs in a folder (for sync comparison)
    pub async fn get_message_uids(&self, folder_id: i64) -> CoreResult<Vec<i64>> {
        let rows = sqlx::query("SELECT uid FROM messages WHERE folder_id = ?")
            .bind(folder_id)
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.iter().map(|r| r.get::<i64, _>("uid")).collect())
    }

    /// Get message count for a folder
    pub async fn get_message_count(&self, folder_id: i64) -> CoreResult<i64> {
        let row = sqlx::query("SELECT COUNT(*) as count FROM messages WHERE folder_id = ?")
            .bind(folder_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(row.get::<i64, _>("count"))
    }

    /// Guess folder type from path
    fn guess_folder_type(folder_path: &str) -> String {
        let path_lower = folder_path.to_lowercase();
        if path_lower == "inbox" {
            "inbox".to_string()
        } else if path_lower.contains("sent") {
            "sent".to_string()
        } else if path_lower.contains("draft") {
            "drafts".to_string()
        } else if path_lower.contains("trash") || path_lower.contains("deleted") {
            "trash".to_string()
        } else if path_lower.contains("spam") || path_lower.contains("junk") {
            "spam".to_string()
        } else if path_lower.contains("archive") {
            "archive".to_string()
        } else {
            "other".to_string()
        }
    }

    /// Get total message count for an account (across all folders)
    pub async fn get_account_message_count(&self, account_id: &str) -> CoreResult<i64> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) as count FROM messages m
            INNER JOIN folders f ON m.folder_id = f.id
            WHERE f.account_id = ?
            "#,
        )
        .bind(account_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get::<i64, _>("count"))
    }

    /// Get total cached body count for an account
    pub async fn get_account_body_count(&self, account_id: &str) -> CoreResult<i64> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) as count FROM messages m
            INNER JOIN folders f ON m.folder_id = f.id
            WHERE f.account_id = ? AND (m.body_text IS NOT NULL OR m.body_html IS NOT NULL)
            "#,
        )
        .bind(account_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get::<i64, _>("count"))
    }

    /// Get inbox message count for a specific account
    pub async fn get_inbox_message_count_for_account(&self, account_id: &str) -> CoreResult<i64> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) as count FROM messages m
            INNER JOIN folders f ON m.folder_id = f.id
            WHERE f.account_id = ? AND f.folder_type = 'inbox'
            "#,
        )
        .bind(account_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get::<i64, _>("count"))
    }

    /// Get the latest inbox message for a specific account (for notifications)
    pub async fn get_latest_inbox_message(&self, account_id: &str) -> CoreResult<Option<DbMessage>> {
        let message = sqlx::query_as::<_, DbMessage>(
            r#"
            SELECT m.id, m.folder_id, m.uid, m.message_id, m.subject, m.from_address,
                   m.from_name, m.to_addresses, m.cc_addresses, m.date_sent, m.date_epoch, m.snippet,
                   m.is_read, m.is_starred, m.has_attachments, m.size, m.maildir_path,
                   m.body_text, m.body_html
            FROM messages m
            JOIN folders f ON m.folder_id = f.id
            WHERE f.account_id = ? AND f.folder_type = 'inbox'
            ORDER BY m.date_epoch DESC, m.uid DESC
            LIMIT 1
            "#,
        )
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(message)
    }

    /// Get total unread count across all accounts (for window badge)
    pub async fn get_total_unread_count(&self) -> CoreResult<i64> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) as count FROM messages m
            INNER JOIN folders f ON m.folder_id = f.id
            WHERE f.folder_type = 'inbox' AND m.is_read = 0
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get::<i64, _>("count"))
    }

    /// Clear all cached data for an account
    pub async fn clear_account_cache(&self, account_id: &str) -> CoreResult<()> {
        // Delete messages first (foreign key constraint)
        sqlx::query(
            r#"
            DELETE FROM messages WHERE folder_id IN (
                SELECT id FROM folders WHERE account_id = ?
            )
            "#,
        )
        .bind(account_id)
        .execute(&self.pool)
        .await?;

        // Delete folders
        sqlx::query("DELETE FROM folders WHERE account_id = ?")
            .bind(account_id)
            .execute(&self.pool)
            .await?;

        info!("Cleared cache for account {}", account_id);
        Ok(())
    }

    /// Get messages across all inbox folders (for unified inbox)
    pub async fn get_inbox_messages(
        &self,
        limit: i64,
        offset: i64,
    ) -> CoreResult<Vec<DbMessage>> {
        let messages = sqlx::query_as::<_, DbMessage>(
            r#"
            SELECT m.id, m.folder_id, m.uid, m.message_id, m.subject, m.from_address,
                   m.from_name, m.to_addresses, m.cc_addresses, m.date_sent, m.date_epoch, m.snippet,
                   m.is_read, m.is_starred, m.has_attachments, m.size, m.maildir_path,
                   m.body_text, m.body_html
            FROM messages m
            JOIN folders f ON m.folder_id = f.id
            WHERE f.folder_type = 'inbox'
            ORDER BY m.date_epoch DESC, m.uid DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(messages)
    }

    /// Get total message count across all inbox folders
    pub async fn get_inbox_message_count(&self) -> CoreResult<i64> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) as count FROM messages m
            JOIN folders f ON m.folder_id = f.id
            WHERE f.folder_type = 'inbox'
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get::<i64, _>("count"))
    }

    /// Search messages using FTS scoped to inbox folders (for unified inbox)
    pub async fn search_inbox_messages(
        &self,
        query: &str,
        limit: i64,
    ) -> CoreResult<Vec<DbMessage>> {
        let fts_query = prepare_fts_query(query);
        debug!("FTS inbox search: '{}' -> '{}'", query, fts_query);

        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let messages = sqlx::query_as::<_, DbMessage>(
            r#"
            SELECT m.id, m.folder_id, m.uid, m.message_id, m.subject, m.from_address,
                   m.from_name, m.to_addresses, m.cc_addresses, m.date_sent, m.date_epoch, m.snippet,
                   m.is_read, m.is_starred, m.has_attachments, m.size, m.maildir_path,
                   m.body_text, m.body_html
            FROM messages m
            JOIN messages_fts fts ON m.id = fts.rowid
            JOIN folders f ON m.folder_id = f.id
            WHERE messages_fts MATCH ? AND f.folder_type = 'inbox'
            ORDER BY m.date_epoch DESC
            LIMIT ?
            "#,
        )
        .bind(&fts_query)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(messages)
    }

    /// Get a folder by its ID (for resolving folder→account mapping in unified inbox)
    pub async fn get_folder_by_id(&self, folder_id: i64) -> CoreResult<Option<DbFolder>> {
        let folder = sqlx::query_as::<_, DbFolder>(
            r#"
            SELECT id, account_id, name, full_path, folder_type, uidvalidity,
                   uid_next, message_count, unread_count
            FROM folders
            WHERE id = ?
            "#,
        )
        .bind(folder_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(folder)
    }

    /// Get messages for a folder with filters applied
    pub async fn get_messages_filtered(
        &self,
        folder_id: i64,
        limit: i64,
        offset: i64,
        filter: &MessageFilter,
    ) -> CoreResult<Vec<DbMessage>> {
        let mut conditions = vec!["m.folder_id = ?".to_string()];
        conditions.extend(filter.build_conditions());
        let where_clause = conditions.join(" AND ");
        let query_str = format!(
            r#"SELECT m.id, m.folder_id, m.uid, m.message_id, m.subject, m.from_address,
                   m.from_name, m.to_addresses, m.cc_addresses, m.date_sent, m.date_epoch, m.snippet,
                   m.is_read, m.is_starred, m.has_attachments, m.size, m.maildir_path,
                   m.body_text, m.body_html
            FROM messages m
            WHERE {}
            ORDER BY m.date_epoch DESC, m.uid DESC
            LIMIT ? OFFSET ?"#,
            where_clause
        );
        let mut query = sqlx::query_as::<_, DbMessage>(&query_str).bind(folder_id);
        if !filter.from_contains.is_empty() {
            let pattern = format!("%{}%", filter.from_contains);
            query = query.bind(pattern.clone()).bind(pattern);
        }
        if let Some(after) = filter.date_after {
            query = query.bind(after);
        }
        if let Some(before) = filter.date_before {
            query = query.bind(before);
        }
        let messages = query.bind(limit).bind(offset).fetch_all(&self.pool).await?;
        Ok(messages)
    }

    /// Get message count for a folder with filters applied
    pub async fn get_messages_filtered_count(
        &self,
        folder_id: i64,
        filter: &MessageFilter,
    ) -> CoreResult<i64> {
        let mut conditions = vec!["m.folder_id = ?".to_string()];
        conditions.extend(filter.build_conditions());
        let where_clause = conditions.join(" AND ");
        let query_str = format!(
            "SELECT COUNT(*) as count FROM messages m WHERE {}",
            where_clause
        );
        let mut query = sqlx::query(&query_str).bind(folder_id);
        if !filter.from_contains.is_empty() {
            let pattern = format!("%{}%", filter.from_contains);
            query = query.bind(pattern.clone()).bind(pattern);
        }
        if let Some(after) = filter.date_after {
            query = query.bind(after);
        }
        if let Some(before) = filter.date_before {
            query = query.bind(before);
        }
        let row = query.fetch_one(&self.pool).await?;
        Ok(row.get::<i64, _>("count"))
    }

    /// Get messages across all inbox folders with filters applied
    pub async fn get_inbox_messages_filtered(
        &self,
        limit: i64,
        offset: i64,
        filter: &MessageFilter,
    ) -> CoreResult<Vec<DbMessage>> {
        let mut conditions = vec!["f.folder_type = 'inbox'".to_string()];
        conditions.extend(filter.build_conditions());
        let where_clause = conditions.join(" AND ");
        let query_str = format!(
            r#"SELECT m.id, m.folder_id, m.uid, m.message_id, m.subject, m.from_address,
                   m.from_name, m.to_addresses, m.cc_addresses, m.date_sent, m.date_epoch, m.snippet,
                   m.is_read, m.is_starred, m.has_attachments, m.size, m.maildir_path,
                   m.body_text, m.body_html
            FROM messages m
            JOIN folders f ON m.folder_id = f.id
            WHERE {}
            ORDER BY m.date_epoch DESC, m.uid DESC
            LIMIT ? OFFSET ?"#,
            where_clause
        );
        let mut query = sqlx::query_as::<_, DbMessage>(&query_str);
        if !filter.from_contains.is_empty() {
            let pattern = format!("%{}%", filter.from_contains);
            query = query.bind(pattern.clone()).bind(pattern);
        }
        if let Some(after) = filter.date_after {
            query = query.bind(after);
        }
        if let Some(before) = filter.date_before {
            query = query.bind(before);
        }
        let messages = query.bind(limit).bind(offset).fetch_all(&self.pool).await?;
        Ok(messages)
    }

    /// Get message count across all inbox folders with filters applied
    pub async fn get_inbox_messages_filtered_count(
        &self,
        filter: &MessageFilter,
    ) -> CoreResult<i64> {
        let mut conditions = vec!["f.folder_type = 'inbox'".to_string()];
        conditions.extend(filter.build_conditions());
        let where_clause = conditions.join(" AND ");
        let query_str = format!(
            r#"SELECT COUNT(*) as count FROM messages m
            JOIN folders f ON m.folder_id = f.id
            WHERE {}"#,
            where_clause
        );
        let mut query = sqlx::query(&query_str);
        if !filter.from_contains.is_empty() {
            let pattern = format!("%{}%", filter.from_contains);
            query = query.bind(pattern.clone()).bind(pattern);
        }
        if let Some(after) = filter.date_after {
            query = query.bind(after);
        }
        if let Some(before) = filter.date_before {
            query = query.bind(before);
        }
        let row = query.fetch_one(&self.pool).await?;
        Ok(row.get::<i64, _>("count"))
    }

    /// Get the Drafts folder path for an account
    pub async fn get_drafts_folder(&self, account_id: &str) -> CoreResult<Option<String>> {
        let row = sqlx::query(
            "SELECT full_path FROM folders WHERE account_id = ? AND folder_type = 'drafts' LIMIT 1",
        )
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.get::<String, _>("full_path")))
    }

    /// Get the trash folder path for an account
    pub async fn get_trash_folder(&self, account_id: &str) -> CoreResult<Option<String>> {
        let row = sqlx::query(
            "SELECT full_path FROM folders WHERE account_id = ? AND folder_type = 'trash' LIMIT 1",
        )
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.get::<String, _>("full_path")))
    }

    /// Get the archive folder path for an account
    pub async fn get_archive_folder(&self, account_id: &str) -> CoreResult<Option<String>> {
        let row = sqlx::query(
            "SELECT full_path FROM folders WHERE account_id = ? AND folder_type = 'archive' LIMIT 1",
        )
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.get::<String, _>("full_path")))
    }

    /// Get the minimum UID in a folder (for resume sync)
    pub async fn get_min_uid(&self, folder_id: i64) -> CoreResult<Option<u32>> {
        let row = sqlx::query("SELECT MIN(uid) as min_uid FROM messages WHERE folder_id = ?")
            .bind(folder_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(row.get::<Option<i64>, _>("min_uid").map(|v| v as u32))
    }

    /// Get the folder_id for a message by its database ID
    pub async fn get_message_folder_id(&self, message_id: i64) -> CoreResult<Option<i64>> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT folder_id FROM messages WHERE id = ?"
        )
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(folder_id,)| folder_id))
    }

    /// Batch update is_read and is_starred flags by UID within a transaction
    pub async fn batch_update_flags(
        &self,
        folder_id: i64,
        flags: &[(u32, bool, bool)],
    ) -> CoreResult<usize> {
        if flags.is_empty() {
            return Ok(0);
        }

        let mut tx = self.pool.begin().await?;
        let mut count = 0;

        for &(uid, is_read, is_starred) in flags {
            let result = sqlx::query(
                "UPDATE messages SET is_read = ?, is_starred = ?, updated_at = datetime('now') WHERE folder_id = ? AND uid = ?",
            )
            .bind(is_read)
            .bind(is_starred)
            .bind(folder_id)
            .bind(uid as i64)
            .execute(&mut *tx)
            .await;

            match result {
                Ok(r) => {
                    if r.rows_affected() > 0 {
                        count += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to update flags for uid={}: {}", uid, e);
                }
            }
        }

        tx.commit().await?;
        Ok(count)
    }

    /// Clear all cached data
    pub async fn clear_all_cache(&self) -> CoreResult<()> {
        sqlx::query("DELETE FROM messages")
            .execute(&self.pool)
            .await?;

        sqlx::query("DELETE FROM folders")
            .execute(&self.pool)
            .await?;

        info!("Cleared all cache");
        Ok(())
    }
}
