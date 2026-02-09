//! Database storage using SQLite

use crate::{CoreError, CoreResult};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};
use std::path::Path;
use tracing::{debug, info};

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

        let db_url = format!("sqlite:{}?mode=rwc", path.display());
        info!("Opening database at {}", path.display());

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_url)
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
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Migration: Add body columns if they don't exist (for existing databases)
        self.migrate_add_body_columns().await?;

        // Migration: Add date_epoch column if it doesn't exist
        self.migrate_add_date_epoch().await?;

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
            sqlx::query("ALTER TABLE messages ADD COLUMN body_text TEXT")
                .execute(&self.pool)
                .await
                .ok(); // Ignore error if column already exists
            sqlx::query("ALTER TABLE messages ADD COLUMN body_html TEXT")
                .execute(&self.pool)
                .await
                .ok();
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
            sqlx::query("ALTER TABLE messages ADD COLUMN date_epoch INTEGER")
                .execute(&self.pool)
                .await
                .ok();
            // Create index for sorting
            sqlx::query("CREATE INDEX IF NOT EXISTS idx_messages_date_epoch ON messages(date_epoch DESC)")
                .execute(&self.pool)
                .await
                .ok();
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
        let result = sqlx::query(
            r#"
            INSERT INTO folders (account_id, name, full_path, folder_type)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(account_id, full_path) DO UPDATE SET
                name = excluded.name,
                folder_type = excluded.folder_type,
                updated_at = datetime('now')
            RETURNING id
            "#,
        )
        .bind(account_id)
        .bind(name)
        .bind(full_path)
        .bind(folder_type)
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

    /// Insert or update messages in a batch (wrapped in a transaction for performance)
    pub async fn upsert_messages_batch(
        &self,
        folder_id: i64,
        messages: &[DbMessage],
    ) -> CoreResult<usize> {
        let mut tx = self.pool.begin().await?;
        let mut count = 0;

        for msg in messages {
            let result = sqlx::query(
                r#"
                INSERT INTO messages (
                    folder_id, uid, message_id, subject, from_address, from_name,
                    to_addresses, date_sent, date_epoch, snippet, is_read, is_starred,
                    has_attachments, size, maildir_path
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(folder_id, uid) DO UPDATE SET
                    message_id = excluded.message_id,
                    subject = excluded.subject,
                    from_address = excluded.from_address,
                    from_name = excluded.from_name,
                    to_addresses = excluded.to_addresses,
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
        Ok(count)
    }

    /// Insert or update a message
    pub async fn upsert_message(&self, folder_id: i64, msg: &DbMessage) -> CoreResult<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO messages (
                folder_id, uid, message_id, subject, from_address, from_name,
                to_addresses, date_sent, date_epoch, snippet, is_read, is_starred,
                has_attachments, size, maildir_path
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(folder_id, uid) DO UPDATE SET
                message_id = excluded.message_id,
                subject = excluded.subject,
                from_address = excluded.from_address,
                from_name = excluded.from_name,
                to_addresses = excluded.to_addresses,
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
                   to_addresses, date_sent, date_epoch, snippet, is_read, is_starred,
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

    /// Search messages using FTS
    pub async fn search_messages(&self, query: &str, limit: i64) -> CoreResult<Vec<DbMessage>> {
        let messages = sqlx::query_as::<_, DbMessage>(
            r#"
            SELECT m.id, m.folder_id, m.uid, m.message_id, m.subject, m.from_address,
                   m.from_name, m.to_addresses, m.date_sent, m.date_epoch, m.snippet,
                   m.is_read, m.is_starred, m.has_attachments, m.size, m.maildir_path,
                   m.body_text, m.body_html
            FROM messages m
            JOIN messages_fts fts ON m.id = fts.rowid
            WHERE messages_fts MATCH ?
            ORDER BY rank
            LIMIT ?
            "#,
        )
        .bind(query)
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

        // SQLite doesn't support arrays, so we build the IN clause
        let placeholders: String = uids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let query = format!(
            "DELETE FROM messages WHERE folder_id = ? AND uid NOT IN ({})",
            placeholders
        );

        let mut q = sqlx::query(&query).bind(folder_id);
        for uid in uids {
            q = q.bind(uid);
        }

        let result = q.execute(&self.pool).await?;
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
