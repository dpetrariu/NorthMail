//! Main application setup

use crate::imap_pool::{ImapCommand, ImapCredentials, ImapPool, ImapResponse};
use crate::widgets::MessageInfo;
use crate::window::NorthMailWindow;
use base64::Engine;
use gtk4::{gio, glib, prelude::*, subclass::prelude::*};
use libadwaita as adw;
use libadwaita::prelude::*;
use northmail_auth::AuthManager;
use northmail_imap::{ImapClient, SimpleImapClient};
use tracing::{debug, error, info, warn};

const APP_ID: &str = "org.northmail.NorthMail";

/// Map a DB folder_type string to a GTK icon name
fn folder_type_to_icon(folder_type: &str) -> &'static str {
    match folder_type {
        "inbox" => "mail-inbox-symbolic",
        "sent" => "mail-send-symbolic",
        "drafts" => "document-edit-symbolic",
        "trash" => "user-trash-symbolic",
        "spam" => "mail-mark-junk-symbolic",
        "archive" => "mail-read-symbolic",
        _ => "folder-symbolic",
    }
}

/// Sort priority for known folder types (lower = higher in sidebar)
fn folder_type_sort_key(folder_type: &str) -> u8 {
    match folder_type {
        "inbox" => 0,
        "sent" => 1,
        "drafts" => 2,
        "trash" => 3,
        "spam" => 4,
        "archive" => 5,
        _ => 10,
    }
}

/// Decode MIME encoded-word headers (RFC 2047)
/// Handles =?charset?encoding?text?= format
fn decode_mime_header(input: &str) -> String {
    let mut result = String::new();
    let mut remaining = input;

    while !remaining.is_empty() {
        if let Some(start) = remaining.find("=?") {
            // Add text before encoded word
            result.push_str(&remaining[..start]);
            remaining = &remaining[start..];

            // Try to parse encoded word
            if let Some(decoded) = try_decode_encoded_word(remaining) {
                result.push_str(&decoded.0);
                remaining = decoded.1;
            } else {
                // Not valid encoded word, add the =? and continue
                result.push_str("=?");
                remaining = &remaining[2..];
            }
        } else {
            result.push_str(remaining);
            break;
        }
    }

    result
}

/// Try to decode an encoded word starting at the beginning of input
/// Returns (decoded_text, remaining_input) on success
fn try_decode_encoded_word(input: &str) -> Option<(String, &str)> {
    // Format: =?charset?encoding?encoded_text?=
    if !input.starts_with("=?") {
        return None;
    }

    let rest = &input[2..];
    let parts: Vec<&str> = rest.splitn(4, '?').collect();
    if parts.len() < 3 {
        return None;
    }

    let charset = parts[0].to_uppercase();
    let encoding = parts[1].to_uppercase();
    let encoded_text = parts[2];

    // Check if there's actually a ?= after the encoded text
    let full_pattern = format!("=?{}?{}?{}?=", parts[0], parts[1], encoded_text);
    if !input.starts_with(&full_pattern) {
        return None;
    }

    // Decode the bytes first
    let bytes = match encoding.as_str() {
        "B" => {
            // Base64 encoding
            base64::prelude::BASE64_STANDARD
                .decode(encoded_text)
                .ok()
        }
        "Q" => {
            // Quoted-printable encoding
            Some(decode_quoted_printable_bytes(encoded_text))
        }
        _ => None,
    }?;

    // Convert bytes to string using the specified charset
    let text = decode_charset(&charset, &bytes)?;

    let consumed = full_pattern.len();
    // Skip any whitespace between encoded words
    let remaining = input[consumed..].trim_start();
    Some((text, remaining))
}

/// Decode quoted-printable encoding for headers to bytes
fn decode_quoted_printable_bytes(input: &str) -> Vec<u8> {
    let mut result = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '_' => result.push(b' '), // Underscore = space in headers
            '=' => {
                // =XX hex encoding
                let hex: String = chars.by_ref().take(2).collect();
                if hex.len() == 2 {
                    if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                        result.push(byte);
                    } else {
                        result.push(b'=');
                        result.extend(hex.as_bytes());
                    }
                } else {
                    result.push(b'=');
                    result.extend(hex.as_bytes());
                }
            }
            _ => {
                // ASCII character
                if c.is_ascii() {
                    result.push(c as u8);
                }
            }
        }
    }

    result
}

/// Decode bytes using the specified charset
fn decode_charset(charset: &str, bytes: &[u8]) -> Option<String> {
    match charset {
        "UTF-8" | "UTF8" => String::from_utf8(bytes.to_vec()).ok(),
        "ISO-8859-1" | "LATIN1" | "LATIN-1" => {
            // ISO-8859-1 is a 1:1 mapping to Unicode code points 0-255
            Some(bytes.iter().map(|&b| b as char).collect())
        }
        "ISO-8859-15" | "LATIN9" | "LATIN-9" => {
            // ISO-8859-15 is similar to ISO-8859-1 with some differences
            Some(bytes.iter().map(|&b| {
                match b {
                    0xA4 => '€',
                    0xA6 => 'Š',
                    0xA8 => 'š',
                    0xB4 => 'Ž',
                    0xB8 => 'ž',
                    0xBC => 'Œ',
                    0xBD => 'œ',
                    0xBE => 'Ÿ',
                    _ => b as char,
                }
            }).collect())
        }
        "WINDOWS-1252" | "CP1252" => {
            // Windows-1252 has extra characters in 0x80-0x9F range
            Some(bytes.iter().map(|&b| {
                match b {
                    0x80 => '€', 0x82 => '‚', 0x83 => 'ƒ', 0x84 => '„',
                    0x85 => '…', 0x86 => '†', 0x87 => '‡', 0x88 => 'ˆ',
                    0x89 => '‰', 0x8A => 'Š', 0x8B => '‹', 0x8C => 'Œ',
                    0x8E => 'Ž', 0x91 => '\u{2018}', 0x92 => '\u{2019}', 0x93 => '"',
                    0x94 => '"', 0x95 => '•', 0x96 => '–', 0x97 => '—',
                    0x98 => '˜', 0x99 => '™', 0x9A => 'š', 0x9B => '›',
                    0x9C => 'œ', 0x9E => 'ž', 0x9F => 'Ÿ',
                    _ => b as char,
                }
            }).collect())
        }
        _ => {
            // Try UTF-8 as fallback, or lossy conversion
            String::from_utf8(bytes.to_vec())
                .ok()
                .or_else(|| Some(String::from_utf8_lossy(bytes).into_owned()))
        }
    }
}

/// App state that persists across sessions
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct AppState {
    /// Last selected folder (account_id, folder_path)
    last_folder: Option<(String, String)>,
    /// Whether unified inbox was selected
    unified_inbox: bool,
}

impl AppState {
    fn config_path() -> std::path::PathBuf {
        let config_dir = glib::user_config_dir().join("northmail");
        std::fs::create_dir_all(&config_dir).ok();
        config_dir.join("state.json")
    }

    fn load() -> Self {
        std::fs::read_to_string(Self::config_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            std::fs::write(Self::config_path(), json).ok();
        }
    }
}

/// State for progressive message loading
#[derive(Clone, Default)]
pub struct FolderLoadState {
    /// Account ID
    pub account_id: String,
    /// Folder path
    pub folder_path: String,
    /// Total message count in folder
    pub total_count: u32,
    /// Lowest sequence number we've fetched (for loading more older messages)
    pub lowest_seq: u32,
    /// How many messages to fetch per batch
    pub batch_size: u32,
}

/// Events for streaming message fetches
enum FetchEvent {
    FolderInfo { total_count: u32 },
    /// Messages to display in UI
    Messages(Vec<MessageInfo>),
    /// Messages for background sync (save to DB only, don't update UI)
    BackgroundMessages(Vec<MessageInfo>),
    /// Prefetched body for a message (uid, raw_body)
    BodyPrefetched { uid: u32, body: String },
    /// Initial batch done, background sync continues
    InitialBatchDone { lowest_seq: u32 },
    /// Full sync complete (all messages fetched)
    FullSyncDone { total_synced: u32 },
    /// Progress update during background sync
    SyncProgress { synced: u32, total: u32 },
    Error(String),
}

/// Convert IMAP FolderType to the DB string representation
fn folder_type_to_db_string(ft: &northmail_imap::FolderType) -> String {
    match ft {
        northmail_imap::FolderType::Inbox => "inbox",
        northmail_imap::FolderType::Sent => "sent",
        northmail_imap::FolderType::Drafts => "drafts",
        northmail_imap::FolderType::Trash => "trash",
        northmail_imap::FolderType::Spam => "spam",
        northmail_imap::FolderType::Archive => "archive",
        northmail_imap::FolderType::Other => "other",
    }
    .to_string()
}

/// Result of an account sync (folder list + inbox message count)
struct SyncResult {
    inbox_count: usize,
    /// (name, full_path, folder_type_str, message_count, unseen_count) for each folder
    folders: Vec<SyncedFolder>,
}

/// Folder info from IMAP LIST + STATUS
struct SyncedFolder {
    name: String,
    full_path: String,
    folder_type: String,
    message_count: u32,
    unseen_count: u32,
}

/// Parsed email body
#[derive(Debug, Clone, Default)]
pub struct ParsedEmailBody {
    pub text: Option<String>,
    pub html: Option<String>,
}

mod imp {
    use super::*;
    use libadwaita::subclass::prelude::*;
    use std::cell::{OnceCell, RefCell};
    use std::sync::Arc;
    use tracing::info;

    #[derive(Default)]
    pub struct NorthMailApplication {
        pub window: OnceCell<NorthMailWindow>,
        pub accounts: RefCell<Vec<northmail_auth::GoaAccount>>,
        pub(super) state: RefCell<AppState>,
        /// Current folder loading state for "load more"
        pub(super) folder_load_state: RefCell<Option<FolderLoadState>>,
        /// Database connection for message caching
        pub(super) database: OnceCell<Arc<northmail_core::Database>>,
        /// Generation counter for folder fetches - increments each time a folder is selected
        /// Used to detect and ignore stale fetch results
        pub(super) fetch_generation: std::cell::Cell<u64>,
        /// IMAP connection pool for reusing connections
        pub(super) imap_pool: OnceCell<Arc<ImapPool>>,
        /// Current cache pagination offset (how many messages already loaded from cache)
        pub(super) cache_offset: std::cell::Cell<i64>,
        /// Current folder ID in the database (for cache-based pagination)
        pub(super) cache_folder_id: std::cell::Cell<i64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NorthMailApplication {
        const NAME: &'static str = "NorthMailApplication";
        type Type = super::NorthMailApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for NorthMailApplication {}

    impl ApplicationImpl for NorthMailApplication {
        fn activate(&self) {
            let app = self.obj();
            info!("Application activating");

            // Create or present the window
            let window = self.window.get_or_init(|| {
                let win = NorthMailWindow::new(&app);
                win.present();
                win
            });

            window.present();

            // Load accounts on startup
            app.load_accounts();
        }

        fn startup(&self) {
            self.parent_startup();
            info!("Application starting up");

            let app = self.obj();
            app.setup_actions();
        }
    }

    impl GtkApplicationImpl for NorthMailApplication {}
    impl AdwApplicationImpl for NorthMailApplication {}
}

glib::wrapper! {
    pub struct NorthMailApplication(ObjectSubclass<imp::NorthMailApplication>)
        @extends adw::Application, gtk4::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl NorthMailApplication {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", APP_ID)
            .property("flags", gio::ApplicationFlags::FLAGS_NONE)
            .property("resource-base-path", "/org/northmail/NorthMail")
            .build()
    }

    /// Initialize the database for message caching
    /// Runs in a separate thread with tokio runtime since sqlx requires tokio
    async fn init_database(&self) -> Result<(), String> {
        let data_dir = glib::user_data_dir().join("northmail");
        let db_path = data_dir.join("mail.db");

        info!("Initializing database at {:?}", db_path);

        // sqlx requires tokio runtime, so we run in a separate thread
        let (sender, receiver) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
                northmail_core::Database::open(&db_path).await
            });
            let _ = sender.send(result);
        });

        // Wait for the result with a timeout
        let timeout = std::time::Duration::from_secs(5);
        match receiver.recv_timeout(timeout) {
            Ok(Ok(db)) => {
                if self
                    .imp()
                    .database
                    .set(std::sync::Arc::new(db))
                    .is_err()
                {
                    warn!("Database already initialized");
                }
                info!("Database initialized successfully");
                Ok(())
            }
            Ok(Err(e)) => {
                error!("Failed to initialize database: {}", e);
                Err(format!("Database error: {}", e))
            }
            Err(_) => {
                error!("Database initialization timed out");
                Err("Database initialization timed out".to_string())
            }
        }
    }

    /// Get the database if available
    fn database(&self) -> Option<&std::sync::Arc<northmail_core::Database>> {
        self.imp().database.get()
    }

    /// Get the database (public, for use from window.rs)
    pub fn database_ref(&self) -> Option<&std::sync::Arc<northmail_core::Database>> {
        self.imp().database.get()
    }

    /// Get the current cache folder ID
    pub fn cache_folder_id(&self) -> i64 {
        self.imp().cache_folder_id.get()
    }

    /// Set the cache offset
    pub fn set_cache_offset(&self, offset: i64) {
        self.imp().cache_offset.set(offset);
    }

    /// Save GOA accounts to database for foreign key relationships
    fn save_accounts_to_db(&self, accounts: &[northmail_auth::GoaAccount]) {
        let Some(db) = self.database() else {
            return;
        };

        let db = db.clone();
        let accounts: Vec<northmail_auth::GoaAccount> = accounts.to_vec();

        // Run in background thread with tokio
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                for account in &accounts {
                    // Convert GOA account to core Account
                    let config = if account.provider_type == "google" {
                        northmail_core::AccountConfig::gmail()
                    } else if account.provider_type == "windows_live" || account.provider_type == "microsoft" {
                        northmail_core::AccountConfig::outlook()
                    } else {
                        northmail_core::AccountConfig {
                            imap_host: account.imap_host.clone().unwrap_or_default(),
                            imap_port: 993,
                            smtp_host: account.smtp_host.clone().unwrap_or_default(),
                            smtp_port: 587,
                        }
                    };

                    let core_account = northmail_core::Account {
                        id: account.id.clone(),
                        email: account.email.clone(),
                        display_name: Some(account.provider_name.clone()),
                        provider: account.provider_type.clone(),
                        auth_method: northmail_auth::AuthMethod::Goa {
                            account_id: account.id.clone(),
                        },
                        config,
                    };

                    if let Err(e) = db.upsert_account(&core_account).await {
                        warn!("Failed to save account {} to database: {}", account.email, e);
                    } else {
                        debug!("Saved account {} to database", account.email);
                    }
                }
                info!("Saved {} accounts to database", accounts.len());
            });
        });
    }

    /// Get or create the IMAP connection pool
    fn imap_pool(&self) -> std::sync::Arc<ImapPool> {
        self.imp()
            .imap_pool
            .get_or_init(|| {
                info!("Initializing IMAP connection pool");
                std::sync::Arc::new(ImapPool::new())
            })
            .clone()
    }

    /// Load accounts from GOA on startup
    fn load_accounts(&self) {
        let app = self.clone();

        glib::spawn_future_local(async move {
            // Initialize database first
            if let Err(e) = app.init_database().await {
                warn!("Database initialization failed: {}", e);
                // Continue without caching
            }

            match AuthManager::new().await {
                Ok(auth_manager) => {
                    if auth_manager.is_goa_available() {
                        match auth_manager.list_goa_accounts().await {
                            Ok(accounts) => {
                                if accounts.is_empty() {
                                    info!("No GOA mail accounts found");
                                } else {
                                    info!("Found {} GOA mail accounts", accounts.len());
                                    for account in &accounts {
                                        info!(
                                            "  - {} ({}) [{}]",
                                            account.email, account.provider_name, account.provider_type
                                        );
                                    }
                                    // Store accounts for later use
                                    app.imp().accounts.replace(accounts.clone());
                                    app.update_sidebar_with_accounts(&accounts);

                                    // Save accounts to database for foreign key relationships
                                    app.save_accounts_to_db(&accounts);

                                    // Restore last selected folder or select unified inbox
                                    app.restore_last_folder();

                                    // Start background sync for all supported accounts
                                    app.sync_all_accounts();
                                }
                            }
                            Err(e) => {
                                warn!("Failed to list GOA accounts: {}", e);
                            }
                        }
                    } else {
                        info!("GOA not available");
                    }
                }
                Err(e) => {
                    error!("Failed to create auth manager: {}", e);
                }
            }
        });
    }

    /// Check if an account is Google (Gmail)
    fn is_google_account(account: &northmail_auth::GoaAccount) -> bool {
        account.provider_type == "google"
    }

    /// Check if an account is Microsoft (Outlook/Hotmail)
    fn is_microsoft_account(account: &northmail_auth::GoaAccount) -> bool {
        account.provider_type == "windows_live" || account.provider_type == "microsoft"
    }

    /// Check if an account supports OAuth2 (Gmail, Microsoft, etc.)
    fn is_oauth2_account(account: &northmail_auth::GoaAccount) -> bool {
        account.auth_type == northmail_auth::GoaAuthType::OAuth2
    }

    /// Check if an account uses password-based auth (iCloud, generic IMAP)
    fn is_password_account(account: &northmail_auth::GoaAccount) -> bool {
        account.auth_type == northmail_auth::GoaAuthType::Password
    }

    /// Check if an account is supported
    fn is_supported_account(account: &northmail_auth::GoaAccount) -> bool {
        Self::is_google_account(account) || Self::is_microsoft_account(account) || Self::is_password_account(account)
    }

    /// Restore last selected folder on startup
    fn restore_last_folder(&self) {
        // Load saved state
        let state = AppState::load();
        self.imp().state.replace(state.clone());

        let accounts = self.imp().accounts.borrow().clone();
        if accounts.is_empty() {
            return;
        }

        if state.unified_inbox {
            info!("Restoring unified inbox view");
            self.fetch_unified_inbox();
            // Highlight the unified inbox row in the sidebar
            if let Some(window) = self.active_window() {
                if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                    if let Some(sidebar) = win.folder_sidebar() {
                        sidebar.select_unified_inbox();
                    }
                }
            }
        } else if let Some((account_id, folder_path)) = state.last_folder {
            // Restore last folder if account still exists
            if accounts.iter().any(|a| a.id == account_id) {
                info!("Restoring last folder: {}/{}", account_id, folder_path);
                self.fetch_folder(&account_id, &folder_path);
            } else {
                // Account no longer exists, select first account's inbox
                info!("Last account not found, selecting first inbox");
                if let Some(account) = accounts.iter().find(|a| Self::is_supported_account(a)) {
                    self.fetch_folder(&account.id, "INBOX");
                }
            }
        } else {
            // First launch - select first account's inbox (or unified when implemented)
            info!("First launch - selecting first account inbox");
            if let Some(account) = accounts.iter().find(|a| Self::is_supported_account(a)) {
                self.fetch_folder(&account.id, "INBOX");
            }
        }
    }

    /// Sync all accounts in the background
    fn sync_all_accounts(&self) {
        let app = self.clone();
        let accounts = self.imp().accounts.borrow().clone();

        // Filter to only supported accounts
        let supported_accounts: Vec<_> = accounts
            .iter()
            .filter(|a| Self::is_supported_account(a))
            .cloned()
            .collect();

        if supported_accounts.is_empty() {
            info!("No supported accounts to sync");
            return;
        }

        // Show simple sync status (no progress bar for background sync)
        if let Some(window) = self.active_window() {
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                if let Some(sidebar) = win.folder_sidebar() {
                    sidebar.show_simple_sync_status("Checking mail...");
                }
            }
        }

        glib::spawn_future_local(async move {
            for account in supported_accounts.iter() {
                // Update sync status - show just email (spinner indicates syncing)
                app.update_simple_sync_status(&account.email);

                // Sync this account's inbox
                app.sync_account_inbox(&account.id).await;

                // Refresh sidebar after each account so counts appear progressively
                app.refresh_sidebar_folders();
            }

            // Show completion briefly
            app.update_simple_sync_status("Up to date");

            // Hide sync status after a short delay
            glib::timeout_future(std::time::Duration::from_secs(2)).await;
            app.hide_sync_status();
        });
    }

    /// Update sync status with simple display (no progress bar)
    fn update_simple_sync_status(&self, message: &str) {
        if let Some(window) = self.active_window() {
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                if let Some(sidebar) = win.folder_sidebar() {
                    sidebar.show_simple_sync_status(message);
                }
            }
        }
    }

    /// Update sync status with full display (with progress bar)
    fn update_sync_status(&self, message: &str) {
        if let Some(window) = self.active_window() {
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                if let Some(sidebar) = win.folder_sidebar() {
                    sidebar.show_sync_status(message);
                }
            }
        }
    }

    fn hide_sync_status(&self) {
        if let Some(window) = self.active_window() {
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                if let Some(sidebar) = win.folder_sidebar() {
                    sidebar.hide_sync_status();
                }
            }
        }
    }

    fn update_sync_progress(&self, fraction: f64) {
        if let Some(window) = self.active_window() {
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                if let Some(sidebar) = win.folder_sidebar() {
                    sidebar.set_sync_progress(fraction);
                }
            }
        }
    }

    fn update_sync_detail(&self, detail: &str) {
        if let Some(window) = self.active_window() {
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                if let Some(sidebar) = win.folder_sidebar() {
                    sidebar.set_sync_detail(detail);
                }
            }
        }
    }

    /// Sync a single account's inbox in the background
    async fn sync_account_inbox(&self, account_id: &str) {
        let accounts = self.imp().accounts.borrow().clone();
        let account = match accounts.iter().find(|a| a.id == account_id) {
            Some(a) => a.clone(),
            None => {
                error!("Account not found for sync: {}", account_id);
                return;
            }
        };

        // Only sync supported accounts
        if !Self::is_supported_account(&account) {
            debug!("Skipping unsupported account: {}", account.email);
            return;
        }

        info!("Syncing inbox for {}", account.email);

        // Load cached folders from DB to skip list_folders() when possible
        let cached_folders: Option<Vec<(String, String, String)>> = if let Some(db) = self.database() {
            let db = db.clone();
            let acct_id = account_id.to_string();
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async {
                    db.get_folders(&acct_id).await
                });
                let _ = sender.send(result);
            });
            match receiver.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(Ok(folders)) if !folders.is_empty() => {
                    let cached: Vec<(String, String, String)> = folders
                        .iter()
                        .map(|f| (f.full_path.clone(), f.name.clone(), f.folder_type.clone()))
                        .collect();
                    info!("Using {} cached folders for {}, skipping list_folders()", cached.len(), account.email);
                    Some(cached)
                }
                _ => None,
            }
        } else {
            None
        };

        let sync_result: Option<SyncResult> = match AuthManager::new().await {
            Ok(auth_manager) => {
                if Self::is_google_account(&account) {
                    match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                        Ok((email, access_token)) => {
                            debug!("Got OAuth2 token for {}", email);
                            match Self::fetch_inbox_google_async(email, access_token, cached_folders.clone()).await {
                                Ok(sr) => {
                                    info!("Synced {} messages for {}", sr.inbox_count, account.email);
                                    Some(sr)
                                }
                                Err(e) => {
                                    warn!("Failed to sync {}: {}", account.email, e);
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to get OAuth2 token for {}: {}", account.email, e);
                            None
                        }
                    }
                } else if Self::is_microsoft_account(&account) {
                    match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                        Ok((email, access_token)) => {
                            debug!("Got OAuth2 token for {}", email);
                            match Self::fetch_inbox_microsoft_async(email, access_token, cached_folders.clone()).await {
                                Ok(sr) => {
                                    info!("Synced {} messages for {}", sr.inbox_count, account.email);
                                    Some(sr)
                                }
                                Err(e) => {
                                    warn!("Failed to sync {}: {}", account.email, e);
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to get OAuth2 token for {}: {}", account.email, e);
                            None
                        }
                    }
                } else if Self::is_password_account(&account) {
                    let username = account.imap_username.clone().unwrap_or(account.email.clone());
                    let host = account.imap_host.clone().unwrap_or_else(|| "imap.mail.me.com".to_string());

                    match auth_manager.get_goa_password(&account.id).await {
                        Ok(password) => {
                            debug!("Got password for {}", username);
                            match Self::fetch_inbox_password_async(host, username, password, cached_folders.clone()).await {
                                Ok(sr) => {
                                    info!("Synced {} messages for {}", sr.inbox_count, account.email);
                                    Some(sr)
                                }
                                Err(e) => {
                                    warn!("Failed to sync {}: {}", account.email, e);
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to get password for {}: {}", account.email, e);
                            None
                        }
                    }
                } else {
                    None
                }
            }
            Err(e) => {
                warn!("Failed to create auth manager: {}", e);
                None
            }
        };

        // Save synced folders to database
        if let Some(sr) = sync_result {
            if !sr.folders.is_empty() {
                if let Some(db) = self.database() {
                    let db = db.clone();
                    let acct_id = account_id.to_string();
                    let folder_count = sr.folders.len();
                    let folders = sr.folders;

                    let (sender, receiver) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        let result = rt.block_on(async {
                            for f in &folders {
                                if let Err(e) = db
                                    .upsert_folder_with_counts(
                                        &acct_id,
                                        &f.name,
                                        &f.full_path,
                                        &f.folder_type,
                                        Some(f.message_count as i64),
                                        Some(f.unseen_count as i64),
                                    )
                                    .await
                                {
                                    warn!("Failed to upsert folder {}: {}", f.full_path, e);
                                }
                            }
                        });
                        let _ = sender.send(result);
                    });

                    // Wait for DB writes (short timeout)
                    let timeout = std::time::Duration::from_secs(3);
                    match receiver.recv_timeout(timeout) {
                        Ok(_) => {
                            info!("Saved {} folders for {}", folder_count, account.email);
                        }
                        Err(_) => {
                            warn!("Timed out saving folders for {}", account.email);
                        }
                    }
                }
            }
        }
    }

    /// Fetch inbox messages asynchronously for Google (Gmail)
    /// If cached_folders is Some, skip list_folders() and use cached folder paths for STATUS.
    async fn fetch_inbox_google_async(
        email: String,
        access_token: String,
        cached_folders: Option<Vec<(String, String, String)>>,
    ) -> Result<SyncResult, String> {
        let (sender, receiver) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let result = async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();

                match client.connect_gmail(&email, &access_token).await {
                    Ok(_) => {
                        debug!("IMAP connected for {}", email);

                        // Get folder list: use cache or fetch from IMAP
                        let folder_entries: Vec<(String, String, String)> = if let Some(cached) = cached_folders {
                            debug!("Using {} cached folders, skipping LIST", cached.len());
                            cached
                        } else {
                            match client.list_folders().await {
                                Ok(folder_list) => {
                                    folder_list.into_iter().map(|f| {
                                        (f.full_path, f.name, folder_type_to_db_string(&f.folder_type))
                                    }).collect()
                                }
                                Err(e) => {
                                    warn!("Failed to list folders: {}", e);
                                    Vec::new()
                                }
                            }
                        };

                        // Batch STATUS for all folders (pipelined)
                        let folder_paths: Vec<&str> = folder_entries.iter().map(|(p, _, _)| p.as_str()).collect();
                        let status_results = client
                            .batch_folder_status(&folder_paths)
                            .await
                            .unwrap_or_default();

                        // Build SyncedFolder list and extract inbox count
                        let mut folders = Vec::new();
                        let mut inbox_count: usize = 0;
                        for (path, msg_count, unseen) in &status_results {
                            let (_, name, ft) = folder_entries.iter()
                                .find(|(p, _, _)| p == path)
                                .cloned()
                                .unwrap_or_else(|| (path.clone(), path.clone(), "other".to_string()));
                            if path.eq_ignore_ascii_case("INBOX") {
                                inbox_count = *msg_count as usize;
                            }
                            folders.push(SyncedFolder {
                                name,
                                full_path: path.clone(),
                                folder_type: ft,
                                message_count: *msg_count,
                                unseen_count: *unseen,
                            });
                        }

                        let _ = client.logout().await;
                        Ok(SyncResult { inbox_count, folders })
                    }
                    Err(e) => Err(format!("Auth failed: {}", e)),
                }
            });

            let _ = sender.send(result);
        });

        Self::poll_result_channel(receiver).await
    }

    /// Fetch inbox messages asynchronously for Microsoft (Outlook/Hotmail)
    /// If cached_folders is Some, skip list_folders() and use cached folder paths for STATUS.
    async fn fetch_inbox_microsoft_async(
        email: String,
        access_token: String,
        cached_folders: Option<Vec<(String, String, String)>>,
    ) -> Result<SyncResult, String> {
        let (sender, receiver) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let result = async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();

                match client.connect_outlook(&email, &access_token).await {
                    Ok(_) => {
                        debug!("IMAP connected for {}", email);

                        // Get folder list: use cache or fetch from IMAP
                        let folder_entries: Vec<(String, String, String)> = if let Some(cached) = cached_folders {
                            debug!("Using {} cached folders, skipping LIST", cached.len());
                            cached
                        } else {
                            match client.list_folders().await {
                                Ok(folder_list) => {
                                    folder_list.into_iter().map(|f| {
                                        (f.full_path, f.name, folder_type_to_db_string(&f.folder_type))
                                    }).collect()
                                }
                                Err(e) => {
                                    warn!("Failed to list folders: {}", e);
                                    Vec::new()
                                }
                            }
                        };

                        // Batch STATUS for all folders (pipelined)
                        let folder_paths: Vec<&str> = folder_entries.iter().map(|(p, _, _)| p.as_str()).collect();
                        let status_results = client
                            .batch_folder_status(&folder_paths)
                            .await
                            .unwrap_or_default();

                        // Build SyncedFolder list and extract inbox count
                        let mut folders = Vec::new();
                        let mut inbox_count: usize = 0;
                        for (path, msg_count, unseen) in &status_results {
                            let (_, name, ft) = folder_entries.iter()
                                .find(|(p, _, _)| p == path)
                                .cloned()
                                .unwrap_or_else(|| (path.clone(), path.clone(), "other".to_string()));
                            if path.eq_ignore_ascii_case("INBOX") {
                                inbox_count = *msg_count as usize;
                            }
                            folders.push(SyncedFolder {
                                name,
                                full_path: path.clone(),
                                folder_type: ft,
                                message_count: *msg_count,
                                unseen_count: *unseen,
                            });
                        }

                        let _ = client.logout().await;
                        Ok(SyncResult { inbox_count, folders })
                    }
                    Err(e) => Err(format!("Auth failed: {}", e)),
                }
            });

            let _ = sender.send(result);
        });

        Self::poll_result_channel(receiver).await
    }

    /// Poll a result channel
    async fn poll_result_channel<T>(receiver: std::sync::mpsc::Receiver<Result<T, String>>) -> Result<T, String> {
        loop {
            match receiver.try_recv() {
                Ok(result) => return result,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::timeout_future(std::time::Duration::from_millis(50)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err("Thread panicked".to_string());
                }
            }
        }
    }

    /// Fetch inbox messages asynchronously using password auth (for iCloud, generic IMAP)
    /// If cached_folders is Some, skip list_folders() and use cached folder paths for STATUS.
    /// No pipelining available (async-imap doesn't expose raw stream), but we skip list_folders()
    /// when cached and get inbox count from STATUS instead of select_folder().
    async fn fetch_inbox_password_async(
        host: String,
        username: String,
        password: String,
        cached_folders: Option<Vec<(String, String, String)>>,
    ) -> Result<SyncResult, String> {
        let (sender, receiver) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let result = async_std::task::block_on(async {
                let mut client = ImapClient::new(&host, 993);

                match client.authenticate_login(&username, &password).await {
                    Ok(_) => {
                        debug!("IMAP connected for {}", username);

                        // Get folder list: use cache or fetch from IMAP
                        let folder_entries: Vec<(String, String, String)> = if let Some(cached) = cached_folders {
                            debug!("Using {} cached folders, skipping LIST", cached.len());
                            cached
                        } else {
                            match client.list_folders().await {
                                Ok(folder_list) => {
                                    folder_list.into_iter().map(|f| {
                                        (f.full_path, f.name, folder_type_to_db_string(&f.folder_type))
                                    }).collect()
                                }
                                Err(e) => {
                                    warn!("Failed to list folders: {}", e);
                                    Vec::new()
                                }
                            }
                        };

                        // Get STATUS for each folder (no pipelining with async-imap)
                        let mut folders = Vec::new();
                        let mut inbox_count: usize = 0;
                        for (full_path, name, ft) in &folder_entries {
                            let (msg_count, unseen) = client
                                .folder_status(full_path)
                                .await
                                .unwrap_or((0, 0));
                            if full_path.eq_ignore_ascii_case("INBOX") {
                                inbox_count = msg_count as usize;
                            }
                            folders.push(SyncedFolder {
                                name: name.clone(),
                                full_path: full_path.clone(),
                                folder_type: ft.clone(),
                                message_count: msg_count,
                                unseen_count: unseen,
                            });
                        }

                        let _ = client.logout().await;
                        Ok(SyncResult { inbox_count, folders })
                    }
                    Err(e) => Err(format!("Auth failed: {}", e)),
                }
            });

            let _ = sender.send(result);
        });

        Self::poll_result_channel(receiver).await
    }

    /// Build sidebar folder list for an account from the database cache.
    /// Returns a Vec<FolderInfo> from cached folders, or a fallback with just INBOX.
    fn build_sidebar_folders(
        db_folders: &[northmail_core::models::DbFolder],
    ) -> Vec<crate::widgets::FolderInfo> {
        if db_folders.is_empty() {
            // Fallback: show just INBOX until real folders are synced
            return vec![crate::widgets::FolderInfo {
                name: "Inbox".to_string(),
                full_path: "INBOX".to_string(),
                icon_name: "mail-inbox-symbolic".to_string(),
                unread_count: Some(0),
                is_header: false,
            }];
        }

        let mut folders: Vec<crate::widgets::FolderInfo> = db_folders
            .iter()
            // Skip INBOX since it's shown as the top-level account row
            .filter(|f| f.folder_type != "inbox")
            .map(|f| crate::widgets::FolderInfo {
                name: f.name.clone(),
                full_path: f.full_path.clone(),
                icon_name: folder_type_to_icon(&f.folder_type).to_string(),
                unread_count: f.unread_count.map(|c| c as u32),
                is_header: false,
            })
            .collect();

        // Sort: known types first by priority, then alphabetical for "other" folders
        folders.sort_by(|a, b| {
            let type_a = db_folders
                .iter()
                .find(|f| f.full_path == a.full_path)
                .map(|f| f.folder_type.as_str())
                .unwrap_or("other");
            let type_b = db_folders
                .iter()
                .find(|f| f.full_path == b.full_path)
                .map(|f| f.folder_type.as_str())
                .unwrap_or("other");

            let key_a = folder_type_sort_key(type_a);
            let key_b = folder_type_sort_key(type_b);

            key_a.cmp(&key_b).then_with(|| a.name.cmp(&b.name))
        });

        folders
    }

    /// Load cached folders for all accounts from the database (blocking, runs tokio in thread)
    fn load_cached_folders_for_accounts(
        db: &std::sync::Arc<northmail_core::Database>,
        accounts: &[northmail_auth::GoaAccount],
    ) -> std::collections::HashMap<String, Vec<northmail_core::models::DbFolder>> {
        let db = db.clone();
        let account_ids: Vec<String> = accounts.iter().map(|a| a.id.clone()).collect();

        let (sender, receiver) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
                let mut map = std::collections::HashMap::new();
                for account_id in &account_ids {
                    match db.get_folders(account_id).await {
                        Ok(folders) => {
                            map.insert(account_id.clone(), folders);
                        }
                        Err(e) => {
                            warn!("Failed to load cached folders for {}: {}", account_id, e);
                        }
                    }
                }
                map
            });
            let _ = sender.send(result);
        });

        let timeout = std::time::Duration::from_secs(2);
        receiver.recv_timeout(timeout).unwrap_or_default()
    }

    /// Update sidebar with accounts from GOA
    fn update_sidebar_with_accounts(&self, accounts: &[northmail_auth::GoaAccount]) {
        if let Some(window) = self.active_window() {
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                // Update the message view to show "Select a folder" instead of welcome
                win.show_main_view();

                if let Some(sidebar) = win.folder_sidebar() {
                    // Connect folder selection to message fetching
                    let app = self.clone();
                    sidebar.connect_folder_selected(
                        move |_sidebar, account_id, folder_path, is_unified| {
                            info!(
                                "Folder selected: account={}, folder={}, unified={}",
                                account_id, folder_path, is_unified
                            );

                            // Save state for next launch
                            {
                                let mut state = app.imp().state.borrow_mut();
                                state.unified_inbox = is_unified;
                                if !is_unified {
                                    state.last_folder = Some((account_id.to_string(), folder_path.to_string()));
                                } else {
                                    state.last_folder = None;
                                }
                                state.save();
                            }

                            if is_unified {
                                app.fetch_unified_inbox();
                            } else {
                                app.fetch_folder(account_id, folder_path);
                            }
                        },
                    );

                    // Load cached folders from database
                    let cached_folders_map = self.database()
                        .map(|db| Self::load_cached_folders_for_accounts(db, accounts))
                        .unwrap_or_default();

                    let account_folders: Vec<crate::widgets::AccountFolders> = accounts
                        .iter()
                        .map(|account| {
                            let is_supported = Self::is_supported_account(account);
                            let email_display = if is_supported {
                                account.email.clone()
                            } else {
                                format!("{} (unsupported)", account.email)
                            };

                            let db_folders = cached_folders_map
                                .get(&account.id)
                                .map(|v: &Vec<northmail_core::models::DbFolder>| v.as_slice())
                                .unwrap_or(&[]);

                            let inbox_unread = db_folders
                                .iter()
                                .find(|f| f.folder_type == "inbox")
                                .and_then(|f| f.unread_count)
                                .map(|c| c as u32);

                            crate::widgets::AccountFolders {
                                id: account.id.clone(),
                                email: email_display,
                                inbox_unread,
                                folders: Self::build_sidebar_folders(db_folders),
                            }
                        })
                        .collect();

                    sidebar.set_accounts(account_folders);
                }
            }
        }
    }

    /// Refresh sidebar folder list from database (without re-connecting signal handlers)
    fn refresh_sidebar_folders(&self) {
        let accounts = self.imp().accounts.borrow().clone();
        if accounts.is_empty() {
            return;
        }

        let cached_folders_map = self
            .database()
            .map(|db| Self::load_cached_folders_for_accounts(db, &accounts))
            .unwrap_or_default();

        let account_folders: Vec<crate::widgets::AccountFolders> = accounts
            .iter()
            .map(|account| {
                let is_supported = Self::is_supported_account(account);
                let email_display = if is_supported {
                    account.email.clone()
                } else {
                    format!("{} (unsupported)", account.email)
                };

                let db_folders = cached_folders_map
                    .get(&account.id)
                    .map(|v: &Vec<northmail_core::models::DbFolder>| v.as_slice())
                    .unwrap_or(&[]);

                let inbox_unread = db_folders
                    .iter()
                    .find(|f| f.folder_type == "inbox")
                    .and_then(|f| f.unread_count)
                    .map(|c| c as u32);

                crate::widgets::AccountFolders {
                    id: account.id.clone(),
                    email: email_display,
                    inbox_unread,
                    folders: Self::build_sidebar_folders(db_folders),
                }
            })
            .collect();

        if let Some(window) = self.active_window() {
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                if let Some(sidebar) = win.folder_sidebar() {
                    sidebar.set_accounts(account_folders);
                }
            }
        }
    }

    /// Load cached messages for a folder from the database
    /// Runs database operations in a tokio runtime since sqlx requires it
    async fn load_cached_messages(
        &self,
        account_id: &str,
        folder_path: &str,
    ) -> Option<(i64, Vec<MessageInfo>)> {
        let db = self.database()?.clone();
        let account_id_str = account_id.to_string();
        let folder_path_str = folder_path.to_string();

        // Run database operations in a thread with tokio runtime
        let (sender, receiver) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
                // Get or create folder ID
                let folder_id = db
                    .get_or_create_folder_id(&account_id_str, &folder_path_str)
                    .await?;

                // Load cached messages
                let messages = db.get_messages(folder_id, 100, 0).await?;
                Ok::<_, northmail_core::CoreError>((folder_id, messages))
            });
            let _ = sender.send(result);
        });

        // Wait for result with timeout
        let timeout = std::time::Duration::from_secs(2);
        match receiver.recv_timeout(timeout) {
            Ok(Ok((folder_id, messages))) => {
                if messages.is_empty() {
                    info!("📭 Cache MISS: No cached messages for {}/{}", account_id, folder_path);
                    None
                } else {
                    info!(
                        "📬 Cache HIT: Loaded {} cached messages for {}/{}",
                        messages.len(),
                        account_id,
                        folder_path
                    );
                    let message_infos: Vec<MessageInfo> =
                        messages.iter().map(MessageInfo::from).collect();
                    Some((folder_id, message_infos))
                }
            }
            Ok(Err(e)) => {
                warn!("Failed to load cached messages: {}", e);
                None
            }
            Err(_) => {
                warn!("Cache load timed out");
                None
            }
        }
    }

    /// Check if cache has more messages beyond what's loaded
    fn check_cache_has_more(&self, folder_id: i64, loaded_count: i64) -> bool {
        let db = match self.database() {
            Some(db) => db.clone(),
            None => return false,
        };

        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = if folder_id == -1 {
                rt.block_on(db.get_inbox_message_count())
            } else {
                rt.block_on(db.get_message_count(folder_id))
            };
            let _ = sender.send(result);
        });

        match receiver.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(Ok(total)) => {
                debug!("Cache has {} total messages, loaded {}", total, loaded_count);
                total > loaded_count
            }
            _ => false,
        }
    }

    /// Load more messages from the SQLite cache (pagination)
    fn load_more_from_cache(&self) {
        let folder_id = self.imp().cache_folder_id.get();
        let offset = self.imp().cache_offset.get();
        let batch_size: i64 = 50;

        if folder_id == 0 {
            warn!("No cache folder ID for load more");
            return;
        }

        let db = match self.database() {
            Some(db) => db.clone(),
            None => {
                warn!("No database for cache load more");
                return;
            }
        };

        // Read current filter state from the message list
        let filter = if let Some(window) = self.active_window() {
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                win.message_list().map(|ml| ml.get_message_filter())
            } else {
                None
            }
        } else {
            None
        };
        let filter = filter.unwrap_or_default();

        let app = self.clone();

        glib::spawn_future_local(async move {
            let (sender, receiver) = std::sync::mpsc::channel();
            let f = filter.clone();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async {
                    let (messages, total) = if f.is_active() {
                        let msgs = if folder_id == -1 {
                            db.get_inbox_messages_filtered(batch_size, offset, &f).await?
                        } else {
                            db.get_messages_filtered(folder_id, batch_size, offset, &f).await?
                        };
                        let count = if folder_id == -1 {
                            db.get_inbox_messages_filtered_count(&f).await?
                        } else {
                            db.get_messages_filtered_count(folder_id, &f).await?
                        };
                        (msgs, count)
                    } else {
                        let msgs = if folder_id == -1 {
                            db.get_inbox_messages(batch_size, offset).await?
                        } else {
                            db.get_messages(folder_id, batch_size, offset).await?
                        };
                        let count = if folder_id == -1 {
                            db.get_inbox_message_count().await?
                        } else {
                            db.get_message_count(folder_id).await?
                        };
                        (msgs, count)
                    };
                    Ok::<_, northmail_core::CoreError>((messages, total))
                });
                let _ = sender.send(result);
            });

            // Poll for result
            let result = loop {
                match receiver.try_recv() {
                    Ok(result) => break Some(result),
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        glib::timeout_future(std::time::Duration::from_millis(10)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break None,
                }
            };

            match result {
                Some(Ok((messages, total))) => {
                    let loaded = messages.len() as i64;
                    let new_offset = offset + loaded;
                    info!("📄 Cache page: loaded {} more messages (offset {} -> {})", loaded, offset, new_offset);

                    app.imp().cache_offset.set(new_offset);

                    let message_infos: Vec<MessageInfo> =
                        messages.iter().map(MessageInfo::from).collect();

                    if let Some(window) = app.active_window() {
                        if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                            if let Some(message_list) = win.message_list() {
                                message_list.append_messages(message_infos);
                                message_list.set_can_load_more(new_offset < total);
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    error!("Failed to load more from cache: {}", e);
                    if let Some(window) = app.active_window() {
                        if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                            if let Some(message_list) = win.message_list() {
                                message_list.finish_loading_more();
                            }
                        }
                    }
                }
                None => {
                    warn!("Cache load more channel disconnected");
                }
            }
        });
    }

    /// Save messages to the database cache
    /// Runs in background thread with tokio runtime (fire-and-forget)
    fn save_messages_to_cache(
        &self,
        account_id: &str,
        folder_path: &str,
        messages: &[MessageInfo],
    ) {
        let Some(db) = self.database() else {
            return;
        };

        let db = db.clone();
        let account_id = account_id.to_string();
        let folder_path = folder_path.to_string();
        let messages: Vec<MessageInfo> = messages.to_vec();

        // Run in background thread - fire and forget
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                // Get or create folder ID
                let folder_id = match db.get_or_create_folder_id(&account_id, &folder_path).await {
                    Ok(id) => id,
                    Err(e) => {
                        warn!("Failed to get folder ID for caching: {}", e);
                        return;
                    }
                };

                // Build batch of DbMessages
                let db_messages: Vec<northmail_core::models::DbMessage> = messages
                    .iter()
                    .map(|msg| {
                        northmail_core::models::DbMessage {
                            id: 0,
                            folder_id,
                            uid: msg.uid as i64,
                            message_id: None,
                            subject: Some(msg.subject.clone()),
                            from_address: Some(msg.from.clone()),
                            from_name: None,
                            to_addresses: None,
                            date_sent: Some(msg.date.clone()),
                            date_epoch: msg.date_epoch,
                            snippet: msg.snippet.clone(),
                            is_read: msg.is_read,
                            is_starred: msg.is_starred,
                            has_attachments: msg.has_attachments,
                            size: 0,
                            maildir_path: None,
                            body_text: None,
                            body_html: None,
                        }
                    })
                    .collect();

                // Batch insert in a single transaction (much faster than individual inserts)
                match db.upsert_messages_batch(folder_id, &db_messages).await {
                    Ok(saved_count) => {
                        info!(
                            "💾 Cache SAVE: Saved {}/{} messages for {}/{}",
                            saved_count,
                            messages.len(),
                            account_id,
                            folder_path
                        );
                    }
                    Err(e) => {
                        warn!(
                            "Failed to batch save messages for {}/{}: {}",
                            account_id, folder_path, e
                        );
                    }
                }
            });
        });
    }

    /// Fetch messages for a folder (with progressive loading)
    pub fn fetch_folder(&self, account_id: &str, folder_path: &str) {
        let account_id = account_id.to_string();
        let folder_path = folder_path.to_string();
        let app = self.clone();

        // Find the account
        let accounts = self.imp().accounts.borrow().clone();
        let account = match accounts.iter().find(|a| a.id == account_id) {
            Some(a) => a.clone(),
            None => {
                error!("Account not found: {}", account_id);
                self.show_error("Account not found");
                return;
            }
        };

        // Check if it's a supported account
        if !Self::is_supported_account(&account) {
            self.show_error(&format!(
                "{} accounts are not yet supported",
                account.provider_name
            ));
            return;
        }

        // IMPORTANT: Set current folder state IMMEDIATELY before any async work
        // This prevents race conditions with is_current_folder() checks
        {
            let mut state = self.imp().state.borrow_mut();
            state.last_folder = Some((account_id.clone(), folder_path.clone()));
            state.unified_inbox = false;
        }
        // Save state to disk
        self.imp().state.borrow().save();

        // Highlight the selected folder in the sidebar
        if let Some(window) = self.active_window() {
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                if let Some(sidebar) = win.folder_sidebar() {
                    sidebar.select_folder(&account_id, &folder_path);
                }
            }
        }

        let account_email = account.email.clone();
        let account_id_clone = account.id.clone();
        let is_google = Self::is_google_account(&account);
        let is_microsoft = Self::is_microsoft_account(&account);
        let imap_host = account.imap_host.clone();
        let imap_username = account.imap_username.clone();

        // Clear previous load state and cache pagination
        self.imp().folder_load_state.replace(None);
        self.imp().cache_offset.set(0);
        self.imp().cache_folder_id.set(0);

        // Increment fetch generation to detect stale results
        let generation = self.imp().fetch_generation.get() + 1;
        self.imp().fetch_generation.set(generation);

        glib::spawn_future_local(async move {
            info!("Fetching messages for {}/{}", account_email, folder_path);

            // Phase 1: Try to load from cache first (instant display)
            let has_cache = if let Some((folder_id, cached_messages)) = app
                .load_cached_messages(&account_id, &folder_path)
                .await
            {
                let loaded_count = cached_messages.len() as i64;
                info!(
                    "Displaying {} cached messages for {}/{}",
                    loaded_count,
                    account_email,
                    folder_path
                );

                // Track cache pagination state
                app.imp().cache_offset.set(loaded_count);
                app.imp().cache_folder_id.set(folder_id);

                // Set folder_load_state immediately so message body fetching works
                // This enables clicking on messages while background sync happens
                app.imp().folder_load_state.replace(Some(FolderLoadState {
                    account_id: account_id.clone(),
                    folder_path: folder_path.clone(),
                    total_count: loaded_count as u32,
                    lowest_seq: 1, // Will be updated by IMAP sync
                    batch_size: 50,
                }));

                // Display cached messages immediately
                if let Some(window) = app.active_window() {
                    if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                        win.restore_message_list();
                        if let Some(message_list) = win.message_list() {
                            message_list.set_messages(cached_messages);

                            // Wire up "load more" from cache
                            let app_clone = app.clone();
                            message_list.connect_load_more(move || {
                                app_clone.load_more_from_cache();
                            });

                            // Check if there are more messages in cache
                            let has_more = app.check_cache_has_more(folder_id, loaded_count);
                            message_list.set_can_load_more(has_more);
                        }
                    }
                }

                // Show simple sync status for background update
                app.update_simple_sync_status("Checking for updates...");
                true
            } else {
                // No cache - show full loading state with detailed status
                if let Some(window) = app.active_window() {
                    if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                        win.show_loading_with_status("Connecting...", None);
                    }
                }
                false
            };

            // Phase 2: Fetch from IMAP (updates cache and UI)
            debug!(
                "Starting IMAP sync for {}/{} (has_cache: {})",
                account_email, folder_path, has_cache
            );

            match AuthManager::new().await {
                Ok(auth_manager) => {
                    if is_google {
                        // Google OAuth2 auth
                        match auth_manager
                            .get_xoauth2_token_for_goa(&account_id_clone)
                            .await
                        {
                            Ok((email, access_token)) => {
                                debug!("Got OAuth2 token for {}", email);

                                let folder_path_clone = folder_path.clone();
                                let result =
                                    Self::fetch_folder_streaming_oauth2(account_id_clone.clone(), email, access_token, folder_path_clone, has_cache, generation, &app)
                                        .await;

                                if let Err(e) = result {
                                    error!("Failed to fetch messages: {}", e);
                                    app.show_error(&format!("Failed to fetch messages: {}", e));
                                }
                            }
                            Err(e) => {
                                error!("Failed to get OAuth2 token: {}", e);
                                app.show_error(&format!("Authentication failed: {}", e));
                            }
                        }
                    } else if is_microsoft {
                        // Microsoft OAuth2 auth
                        match auth_manager
                            .get_xoauth2_token_for_goa(&account_id_clone)
                            .await
                        {
                            Ok((email, access_token)) => {
                                debug!("Got OAuth2 token for {}", email);

                                let folder_path_clone = folder_path.clone();
                                let result =
                                    Self::fetch_folder_streaming_microsoft(account_id_clone.clone(), email, access_token, folder_path_clone, has_cache, generation, &app)
                                        .await;

                                if let Err(e) = result {
                                    error!("Failed to fetch messages: {}", e);
                                    app.show_error(&format!("Failed to fetch messages: {}", e));
                                }
                            }
                            Err(e) => {
                                error!("Failed to get OAuth2 token: {}", e);
                                app.show_error(&format!("Authentication failed: {}", e));
                            }
                        }
                    } else {
                        // Password auth (iCloud, generic IMAP)
                        let username = imap_username.unwrap_or(account_email.clone());
                        let host = imap_host.unwrap_or_else(|| "imap.mail.me.com".to_string());

                        match auth_manager.get_goa_password(&account_id_clone).await {
                            Ok(password) => {
                                debug!("Got password for {}", username);

                                let folder_path_clone = folder_path.clone();
                                let result =
                                    Self::fetch_folder_streaming_password(account_id_clone.clone(), host, username, password, folder_path_clone, has_cache, generation, &app)
                                        .await;

                                if let Err(e) = result {
                                    error!("Failed to fetch messages: {}", e);
                                    app.show_error(&format!("Failed to fetch messages: {}", e));
                                }
                            }
                            Err(e) => {
                                error!("Failed to get password: {}", e);
                                app.show_error(&format!("Authentication failed: {}", e));
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to create auth manager: {}", e);
                    app.show_error(&format!("Failed to authenticate: {}", e));
                }
            }
        });
    }

    /// Load more messages for the current folder
    pub fn load_more_messages(&self) {
        let load_state = self.imp().folder_load_state.borrow().clone();
        let Some(state) = load_state else {
            warn!("No folder load state for load more");
            return;
        };

        if state.lowest_seq <= 1 {
            info!("No more messages to load");
            return;
        }

        let app = self.clone();
        let accounts = self.imp().accounts.borrow().clone();
        let account = match accounts.iter().find(|a| a.id == state.account_id) {
            Some(a) => a.clone(),
            None => {
                error!("Account not found for load more");
                return;
            }
        };

        let is_google = Self::is_google_account(&account);
        let is_microsoft = Self::is_microsoft_account(&account);
        let imap_host = account.imap_host.clone();
        let imap_username = account.imap_username.clone();
        let account_id = account.id.clone();

        glib::spawn_future_local(async move {
            info!("Loading more messages for {}", state.folder_path);

            match AuthManager::new().await {
                Ok(auth_manager) => {
                    if is_google {
                        match auth_manager.get_xoauth2_token_for_goa(&account_id).await {
                            Ok((email, access_token)) => {
                                let result = Self::load_more_google(email, access_token, state, &app).await;
                                if let Err(e) = result {
                                    error!("Failed to load more: {}", e);
                                }
                            }
                            Err(e) => error!("Failed to get token for load more: {}", e),
                        }
                    } else if is_microsoft {
                        match auth_manager.get_xoauth2_token_for_goa(&account_id).await {
                            Ok((email, access_token)) => {
                                let result = Self::load_more_microsoft(email, access_token, state, &app).await;
                                if let Err(e) = result {
                                    error!("Failed to load more: {}", e);
                                }
                            }
                            Err(e) => error!("Failed to get token for load more: {}", e),
                        }
                    } else {
                        let username = imap_username.unwrap_or(account.email.clone());
                        let host = imap_host.unwrap_or_else(|| "imap.mail.me.com".to_string());

                        match auth_manager.get_goa_password(&account_id).await {
                            Ok(password) => {
                                let result = Self::load_more_password(host, username, password, state, &app).await;
                                if let Err(e) = result {
                                    error!("Failed to load more: {}", e);
                                }
                            }
                            Err(e) => error!("Failed to get password for load more: {}", e),
                        }
                    }
                }
                Err(e) => error!("Failed to create auth manager: {}", e),
            }
        });
    }

    /// Fetch folder with streaming updates for Google (Gmail)
    async fn fetch_folder_streaming_oauth2(
        account_id: String,
        email: String,
        access_token: String,
        folder_path: String,
        has_cache: bool,
        generation: u64,
        app: &NorthMailApplication,
    ) -> Result<(), String> {
        let (sender, receiver) = std::sync::mpsc::channel::<FetchEvent>();
        let folder_path_clone = folder_path.clone();

        std::thread::spawn(move || {
            async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();

                match client.connect_gmail(&email, &access_token).await {
                    Ok(_) => {
                        Self::fetch_streaming(&mut client, &folder_path_clone, &sender, true).await;
                    }
                    Err(e) => {
                        let _ = sender.send(FetchEvent::Error(format!("Authentication failed: {}", e)));
                    }
                }
            });
        });

        Self::handle_fetch_events(receiver, &account_id, &folder_path, has_cache, generation, app).await
    }

    /// Fetch folder with streaming updates for Microsoft (Outlook/Hotmail)
    async fn fetch_folder_streaming_microsoft(
        account_id: String,
        email: String,
        access_token: String,
        folder_path: String,
        has_cache: bool,
        generation: u64,
        app: &NorthMailApplication,
    ) -> Result<(), String> {
        let (sender, receiver) = std::sync::mpsc::channel::<FetchEvent>();
        let folder_path_clone = folder_path.clone();

        std::thread::spawn(move || {
            async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();

                match client.connect_outlook(&email, &access_token).await {
                    Ok(_) => {
                        Self::fetch_streaming(&mut client, &folder_path_clone, &sender, true).await;
                    }
                    Err(e) => {
                        let _ = sender.send(FetchEvent::Error(format!("Authentication failed: {}", e)));
                    }
                }
            });
        });

        Self::handle_fetch_events(receiver, &account_id, &folder_path, has_cache, generation, app).await
    }

    /// Fetch folder with streaming updates using password auth
    async fn fetch_folder_streaming_password(
        account_id: String,
        host: String,
        username: String,
        password: String,
        folder_path: String,
        has_cache: bool,
        generation: u64,
        app: &NorthMailApplication,
    ) -> Result<(), String> {
        let (sender, receiver) = std::sync::mpsc::channel::<FetchEvent>();
        let folder_path_clone = folder_path.clone();

        std::thread::spawn(move || {
            async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();

                match client.connect_login(&host, 993, &username, &password).await {
                    Ok(_) => {
                        Self::fetch_streaming(&mut client, &folder_path_clone, &sender, true).await;
                    }
                    Err(e) => {
                        let _ = sender.send(FetchEvent::Error(format!("Authentication failed: {}", e)));
                    }
                }
            });
        });

        Self::handle_fetch_events(receiver, &account_id, &folder_path, has_cache, generation, app).await
    }

    /// Common streaming fetch using SimpleImapClient
    /// Fetches initial batch for display, then continues syncing ALL messages in background
    async fn fetch_streaming(
        client: &mut SimpleImapClient,
        folder_path: &str,
        sender: &std::sync::mpsc::Sender<FetchEvent>,
        _is_initial: bool,
    ) {
        match client.select(folder_path).await {
            Ok(folder_info) => {
                let count = folder_info.message_count.unwrap_or(0);
                let _ = sender.send(FetchEvent::FolderInfo { total_count: count });

                if count == 0 {
                    let _ = sender.send(FetchEvent::InitialBatchDone { lowest_seq: 0 });
                    let _ = sender.send(FetchEvent::FullSyncDone { total_synced: 0 });
                    let _ = client.logout().await;
                    return;
                }

                // Phase 1: Fetch initial batch for immediate display
                const INITIAL_BATCH: u32 = 50;
                const PREFETCH_BODIES: usize = 5;

                let initial_end = count;
                let initial_start = if count > INITIAL_BATCH { count - INITIAL_BATCH + 1 } else { 1 };

                let range = format!("{}:{}", initial_start, initial_end);
                match client.fetch_headers(&range).await {
                    Ok(headers) => {
                        let messages = Self::headers_to_message_info(&headers);

                        // Prefetch bodies for first N messages
                        let uids_to_prefetch: Vec<u32> = messages
                            .iter()
                            .take(PREFETCH_BODIES)
                            .map(|m| m.uid)
                            .collect();

                        // Send messages for UI display
                        let _ = sender.send(FetchEvent::Messages(messages));

                        // Prefetch bodies
                        for uid in uids_to_prefetch {
                            if let Ok(body) = client.fetch_body(uid).await {
                                let _ = sender.send(FetchEvent::BodyPrefetched { uid, body });
                            }
                        }

                        // Signal initial batch done - UI can now be interactive
                        let _ = sender.send(FetchEvent::InitialBatchDone { lowest_seq: initial_start });
                    }
                    Err(e) => {
                        let _ = sender.send(FetchEvent::Error(format!("Fetch failed: {}", e)));
                        let _ = client.logout().await;
                        return;
                    }
                }

                // Phase 2: Background sync - fetch ALL remaining messages
                // This runs after initial display, user can interact while this continues
                if initial_start > 1 {
                    let mut synced = INITIAL_BATCH.min(count);
                    let mut current_end = initial_start - 1;
                    const BACKGROUND_BATCH: u32 = 500; // Larger batches for background sync

                    tracing::info!(
                        "Starting background sync: {} more messages to fetch",
                        current_end
                    );

                    while current_end > 0 {
                        let batch_start = if current_end > BACKGROUND_BATCH {
                            current_end - BACKGROUND_BATCH + 1
                        } else {
                            1
                        };

                        let range = format!("{}:{}", batch_start, current_end);
                        match client.fetch_headers(&range).await {
                            Ok(headers) => {
                                let messages = Self::headers_to_message_info(&headers);
                                let batch_count = messages.len() as u32;
                                synced += batch_count;

                                // Send as background messages (saved to DB, not displayed)
                                let _ = sender.send(FetchEvent::BackgroundMessages(messages));
                                let _ = sender.send(FetchEvent::SyncProgress {
                                    synced,
                                    total: count,
                                });

                                current_end = batch_start - 1;
                            }
                            Err(e) => {
                                tracing::warn!("Background sync batch failed: {}", e);
                                // Continue with next batch on error
                                current_end = if current_end > BACKGROUND_BATCH {
                                    current_end - BACKGROUND_BATCH
                                } else {
                                    0
                                };
                            }
                        }
                    }

                    tracing::info!("Background sync complete: {} messages synced", synced);
                    let _ = sender.send(FetchEvent::FullSyncDone { total_synced: synced });
                } else {
                    let _ = sender.send(FetchEvent::FullSyncDone { total_synced: count });
                }

                let _ = client.logout().await;
            }
            Err(e) => {
                let _ = client.logout().await;
                let _ = sender.send(FetchEvent::Error(format!("Failed to select folder: {}", e)));
            }
        }
    }

    /// Check if we're currently viewing the specified folder
    fn is_current_folder(&self, account_id: &str, folder_path: &str) -> bool {
        let state = self.imp().state.borrow();
        if let Some((current_account, current_folder)) = &state.last_folder {
            current_account == account_id && current_folder == folder_path
        } else {
            false
        }
    }

    /// Check if the given generation is still current (no new folder was selected)
    fn is_current_generation(&self, generation: u64) -> bool {
        self.imp().fetch_generation.get() == generation
    }

    /// Handle streaming fetch events
    async fn handle_fetch_events(
        receiver: std::sync::mpsc::Receiver<FetchEvent>,
        account_id: &str,
        folder_path: &str,
        has_cache: bool,
        generation: u64,
        app: &NorthMailApplication,
    ) -> Result<(), String> {
        let mut total_count = 0u32;
        let mut loaded_count = 0u32;
        let mut first_batch = true;
        #[allow(unused_assignments)]
        let mut lowest_seq = 0u32;
        // Track all UIDs seen during sync for cache cleanup
        let mut synced_uids: Vec<i64> = Vec::new();

        loop {
            // Check if this fetch is still valid (user hasn't switched folders)
            let is_stale = !app.is_current_generation(generation);

            match receiver.try_recv() {
                Ok(event) => match event {
                    FetchEvent::FolderInfo { total_count: count } => {
                        total_count = count;
                        info!("Folder has {} messages", total_count);

                        // Skip UI updates if stale
                        if is_stale {
                            debug!("Generation changed, skipping UI update for {}/{}", account_id, folder_path);
                            continue;
                        }

                        if has_cache {
                            // Cache is displayed - use simple sidebar indicator for background sync
                            app.update_simple_sync_status("Syncing...");
                        } else {
                            // No cache - update the loading status in message list area
                            if let Some(window) = app.active_window() {
                                if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                    win.update_loading_status(
                                        "Loading messages...",
                                        Some(&format!("0 of {}", total_count))
                                    );
                                }
                            }
                        }

                        // DON'T restore message_list here - wait until we have actual messages
                        // Otherwise we'd briefly show stale content before messages arrive
                        // The loading spinner should keep showing until Messages event
                    }
                    FetchEvent::Messages(messages) => {
                        loaded_count += messages.len() as u32;
                        info!("Received batch of {} messages ({}/{})", messages.len(), loaded_count, total_count);

                        // Track UIDs for cache cleanup
                        synced_uids.extend(messages.iter().map(|m| m.uid as i64));

                        // Always save to cache, even if viewing different folder
                        app.save_messages_to_cache(account_id, folder_path, &messages);

                        // Skip UI updates if stale
                        if is_stale {
                            debug!("Generation changed, skipping message UI update for {}/{}", account_id, folder_path);
                            continue;
                        }

                        // Update progress - in message list area for no-cache, sidebar for cache
                        if total_count > 0 {
                            if has_cache {
                                // Sidebar progress for background sync
                                // Don't show progress bar, just keep simple status
                            } else {
                                // Message list area progress for initial load
                                if let Some(window) = app.active_window() {
                                    if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                        win.update_loading_status(
                                            "Loading messages...",
                                            Some(&format!("{} of {}", loaded_count, total_count))
                                        );
                                    }
                                }
                            }
                        }

                        // Set folder_load_state on first batch so body fetching works
                        // immediately (don't wait for InitialBatchDone)
                        if first_batch {
                            app.imp().folder_load_state.replace(Some(FolderLoadState {
                                account_id: account_id.to_string(),
                                folder_path: folder_path.to_string(),
                                total_count,
                                lowest_seq: 1, // Will be updated by InitialBatchDone
                                batch_size: 50,
                            }));
                        }

                        // Always update UI with IMAP messages - they're fresher than cache
                        if let Some(window) = app.active_window() {
                            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                if first_batch && !has_cache {
                                    // Restore message list now that we have actual messages
                                    // (removes loading spinner and adds message_list widget)
                                    win.restore_message_list();
                                }
                                if let Some(message_list) = win.message_list() {
                                    // Always replace with first batch from IMAP - it has the latest messages
                                    if first_batch {
                                        info!("Replacing message list with {} fresh messages from IMAP", messages.len());
                                        let msg_count = messages.len() as i64;
                                        message_list.set_messages(messages);

                                        // Wire up cache-based "load more" for IMAP path too
                                        // (after IMAP saves to cache, pagination pulls from SQLite)
                                        let app_for_load_more = app.clone();
                                        message_list.connect_load_more(move || {
                                            app_for_load_more.load_more_from_cache();
                                        });
                                        app.imp().cache_offset.set(msg_count);

                                        first_batch = false;
                                    } else {
                                        message_list.append_messages(messages);
                                    }
                                }
                            }
                        }
                    }
                    FetchEvent::BodyPrefetched { uid, body } => {
                        // Parse and cache the prefetched body
                        let parsed = Self::parse_email_body(&body);

                        // Always cache, even if stale (useful for next time)
                        if let Some(db) = app.imp().database.get() {
                            Self::save_body_to_cache(db, account_id, folder_path, uid, &parsed);
                        }

                        debug!(
                            "📥 Prefetched body for message {} ({} text, {} html)",
                            uid,
                            parsed.text.as_ref().map(|t| t.len()).unwrap_or(0),
                            parsed.html.as_ref().map(|h| h.len()).unwrap_or(0)
                        );
                    }
                    FetchEvent::BackgroundMessages(messages) => {
                        // Track UIDs for cache cleanup
                        synced_uids.extend(messages.iter().map(|m| m.uid as i64));
                        // Background sync - just save to cache, don't update UI
                        app.save_messages_to_cache(account_id, folder_path, &messages);
                        // No UI update needed - this is background work
                    }
                    FetchEvent::SyncProgress { synced, total } => {
                        // Update sync progress in sidebar (non-intrusive)
                        if !is_stale {
                            app.update_simple_sync_status(&format!("Syncing {}/{}...", synced, total));
                        }
                    }
                    FetchEvent::InitialBatchDone { lowest_seq: seq } => {
                        lowest_seq = seq;
                        info!("Initial batch complete for {}/{}, lowest_seq={}", account_id, folder_path, lowest_seq);

                        // Skip UI updates if stale
                        if is_stale {
                            debug!("Generation changed, skipping InitialBatchDone UI update");
                            continue; // Don't return - keep processing background sync events
                        }

                        // If no messages were received and we didn't have cache,
                        // we need to restore message list and show empty state
                        if first_batch && !has_cache {
                            if let Some(window) = app.active_window() {
                                if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                    win.restore_message_list();
                                    if let Some(message_list) = win.message_list() {
                                        message_list.set_messages(vec![]);
                                    }
                                }
                            }
                        }

                        // Store state for "load more" (though background sync will get everything)
                        app.imp().folder_load_state.replace(Some(FolderLoadState {
                            account_id: account_id.to_string(),
                            folder_path: folder_path.to_string(),
                            total_count,
                            lowest_seq,
                            batch_size: 50,
                        }));

                        // Update cache folder ID so load-more-from-cache works
                        if let Some(db) = app.database() {
                            let db = db.clone();
                            let aid = account_id.to_string();
                            let fp = folder_path.to_string();
                            let (sender, receiver) = std::sync::mpsc::channel();
                            std::thread::spawn(move || {
                                let rt = tokio::runtime::Runtime::new().unwrap();
                                let result = rt.block_on(db.get_or_create_folder_id(&aid, &fp));
                                let _ = sender.send(result);
                            });
                            if let Ok(Ok(fid)) = receiver.recv_timeout(std::time::Duration::from_secs(1)) {
                                app.imp().cache_folder_id.set(fid);
                            }
                        }

                        // Enable "load more" from cache since IMAP has been saving to DB
                        let cache_folder_id = app.imp().cache_folder_id.get();
                        let cache_offset = app.imp().cache_offset.get();
                        if cache_folder_id > 0 {
                            let has_more = app.check_cache_has_more(cache_folder_id, cache_offset);
                            if let Some(window) = app.active_window() {
                                if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                    if let Some(message_list) = win.message_list() {
                                        message_list.set_can_load_more(has_more);
                                    }
                                }
                            }
                        }

                        // Update status to show background sync is continuing
                        if lowest_seq > 1 {
                            app.update_simple_sync_status("Syncing older messages...");
                        }
                        // Don't return - keep processing background sync events
                    }
                    FetchEvent::FullSyncDone { total_synced } => {
                        info!("Full sync complete for {}/{}: {} messages (tracked {} UIDs)", account_id, folder_path, total_synced, synced_uids.len());

                        // Clean up stale messages from cache that no longer exist on server
                        if !synced_uids.is_empty() {
                            if let Some(db) = app.database() {
                                let db = db.clone();
                                let aid = account_id.to_string();
                                let fp = folder_path.to_string();
                                let uids = synced_uids.clone();
                                std::thread::spawn(move || {
                                    let rt = tokio::runtime::Runtime::new().unwrap();
                                    rt.block_on(async {
                                        if let Ok(folder_id) = db.get_or_create_folder_id(&aid, &fp).await {
                                            match db.delete_messages_not_in_uids(folder_id, &uids).await {
                                                Ok(deleted) => {
                                                    if deleted > 0 {
                                                        info!("🧹 Cache cleanup: removed {} stale messages from {}/{}", deleted, aid, fp);
                                                    }
                                                }
                                                Err(e) => {
                                                    warn!("Failed to clean up stale messages: {}", e);
                                                }
                                            }
                                        }
                                    });
                                });
                            }
                        }

                        // Hide sync indicator
                        app.hide_sync_status();

                        // Update folder load state - no more messages to load
                        app.imp().folder_load_state.replace(Some(FolderLoadState {
                            account_id: account_id.to_string(),
                            folder_path: folder_path.to_string(),
                            total_count,
                            lowest_seq: 0, // All synced
                            batch_size: 50,
                        }));

                        // Now that sync is done, check if there are more cached messages to paginate
                        let cache_folder_id = app.imp().cache_folder_id.get();
                        let cache_offset = app.imp().cache_offset.get();
                        if cache_folder_id > 0 {
                            let has_more = app.check_cache_has_more(cache_folder_id, cache_offset);
                            if let Some(window) = app.active_window() {
                                if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                    if let Some(message_list) = win.message_list() {
                                        message_list.set_can_load_more(has_more);
                                    }
                                }
                            }
                        }

                        return Ok(());
                    }
                    FetchEvent::Error(e) => {
                        if !is_stale {
                            app.hide_sync_status();
                            // If we were showing loading spinner (no cache, first batch),
                            // restore message list to avoid stuck spinner
                            if first_batch && !has_cache {
                                if let Some(window) = app.active_window() {
                                    if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                        win.restore_message_list();
                                    }
                                }
                            }
                        }
                        return Err(e);
                    }
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::timeout_future(std::time::Duration::from_millis(50)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err("Connection lost".to_string());
                }
            }
        }
    }

    /// Load more messages for Google (Gmail)
    async fn load_more_google(
        email: String,
        access_token: String,
        state: FolderLoadState,
        app: &NorthMailApplication,
    ) -> Result<(), String> {
        let (sender, receiver) = std::sync::mpsc::channel::<FetchEvent>();
        let state_for_thread = state.clone();

        std::thread::spawn(move || {
            async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();

                match client.connect_gmail(&email, &access_token).await {
                    Ok(_) => {
                        Self::fetch_more(&mut client, &state_for_thread, &sender).await;
                    }
                    Err(e) => {
                        let _ = sender.send(FetchEvent::Error(format!("Auth failed: {}", e)));
                    }
                }
            });
        });

        Self::handle_load_more_events(receiver, state, app).await
    }

    /// Load more messages for Microsoft (Outlook/Hotmail)
    async fn load_more_microsoft(
        email: String,
        access_token: String,
        state: FolderLoadState,
        app: &NorthMailApplication,
    ) -> Result<(), String> {
        let (sender, receiver) = std::sync::mpsc::channel::<FetchEvent>();
        let state_for_thread = state.clone();

        std::thread::spawn(move || {
            async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();

                match client.connect_outlook(&email, &access_token).await {
                    Ok(_) => {
                        Self::fetch_more(&mut client, &state_for_thread, &sender).await;
                    }
                    Err(e) => {
                        let _ = sender.send(FetchEvent::Error(format!("Auth failed: {}", e)));
                    }
                }
            });
        });

        Self::handle_load_more_events(receiver, state, app).await
    }

    /// Load more messages using password auth
    async fn load_more_password(
        host: String,
        username: String,
        password: String,
        state: FolderLoadState,
        app: &NorthMailApplication,
    ) -> Result<(), String> {
        let (sender, receiver) = std::sync::mpsc::channel::<FetchEvent>();
        let state_for_thread = state.clone();

        std::thread::spawn(move || {
            async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();

                match client.connect_login(&host, 993, &username, &password).await {
                    Ok(_) => {
                        Self::fetch_more(&mut client, &state_for_thread, &sender).await;
                    }
                    Err(e) => {
                        let _ = sender.send(FetchEvent::Error(format!("Auth failed: {}", e)));
                    }
                }
            });
        });

        Self::handle_load_more_events(receiver, state, app).await
    }

    /// Fetch more older messages using SimpleImapClient
    async fn fetch_more(
        client: &mut SimpleImapClient,
        state: &FolderLoadState,
        sender: &std::sync::mpsc::Sender<FetchEvent>,
    ) {
        match client.select(&state.folder_path).await {
            Ok(_) => {
                if state.lowest_seq > 1 {
                    let end = state.lowest_seq - 1;
                    let start = if end > state.batch_size { end - state.batch_size + 1 } else { 1 };
                    let range = format!("{}:{}", start, end);

                    match client.fetch_headers(&range).await {
                        Ok(headers) => {
                            let messages = Self::headers_to_message_info(&headers);
                            let _ = sender.send(FetchEvent::Messages(messages));
                            let _ = sender.send(FetchEvent::InitialBatchDone { lowest_seq: start });
                        }
                        Err(e) => {
                            let _ = sender.send(FetchEvent::Error(format!("Fetch failed: {}", e)));
                        }
                    }
                } else {
                    let _ = sender.send(FetchEvent::InitialBatchDone { lowest_seq: 0 });
                }

                let _ = client.logout().await;
            }
            Err(e) => {
                let _ = client.logout().await;
                let _ = sender.send(FetchEvent::Error(format!("Select failed: {}", e)));
            }
        }
    }

    /// Handle load more events
    async fn handle_load_more_events(
        receiver: std::sync::mpsc::Receiver<FetchEvent>,
        mut state: FolderLoadState,
        app: &NorthMailApplication,
    ) -> Result<(), String> {
        loop {
            match receiver.try_recv() {
                Ok(event) => match event {
                    FetchEvent::FolderInfo { .. } => {}
                    FetchEvent::Messages(messages) => {
                        info!("Loaded {} more messages", messages.len());

                        if let Some(window) = app.active_window() {
                            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                if let Some(message_list) = win.message_list() {
                                    message_list.append_messages(messages);
                                }
                            }
                        }
                    }
                    FetchEvent::BackgroundMessages(_) | FetchEvent::SyncProgress { .. } => {
                        // Not used in load more
                    }
                    FetchEvent::InitialBatchDone { lowest_seq } | FetchEvent::FullSyncDone { total_synced: lowest_seq } => {
                        state.lowest_seq = lowest_seq;
                        app.imp().folder_load_state.replace(Some(state.clone()));

                        // Update "load more" visibility
                        if let Some(window) = app.active_window() {
                            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                if let Some(message_list) = win.message_list() {
                                    message_list.set_can_load_more(lowest_seq > 1);
                                }
                            }
                        }

                        return Ok(());
                    }
                    FetchEvent::Error(e) => {
                        // Finish loading state even on error
                        if let Some(window) = app.active_window() {
                            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                if let Some(message_list) = win.message_list() {
                                    message_list.finish_loading_more();
                                }
                            }
                        }
                        return Err(e);
                    }
                    FetchEvent::BodyPrefetched { .. } => {
                        // Body prefetching not done during "load more" - ignore
                    }
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::timeout_future(std::time::Duration::from_millis(50)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err("Connection lost".to_string());
                }
            }
        }
    }

    /// Convert headers to MessageInfo
    fn headers_to_message_info(headers: &[northmail_imap::MessageHeader]) -> Vec<MessageInfo> {
        headers
            .iter()
            .rev()
            .map(|h| {
                let date = h.envelope.date.clone().unwrap_or_default();
                let date_epoch = Self::parse_date_epoch(&date);
                MessageInfo {
                    id: h.uid as i64,
                    uid: h.uid,
                    folder_id: 0,
                    subject: decode_mime_header(&h.envelope.subject.clone().unwrap_or_default()),
                    from: h
                        .envelope
                        .from
                        .first()
                        .map(|a| {
                            if let Some(name) = &a.name {
                                decode_mime_header(name)
                            } else {
                                a.address.clone()
                            }
                        })
                        .unwrap_or_default(),
                    to: h.envelope.to.iter()
                        .map(|a| {
                            if let Some(name) = &a.name {
                                decode_mime_header(name)
                            } else {
                                a.address.clone()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                    date,
                    date_epoch,
                    snippet: None,
                    is_read: h.is_read(),
                    is_starred: h.is_starred(),
                    has_attachments: h.has_attachments,
                }
            })
            .collect()
    }

    /// Whether we are currently in unified inbox mode
    pub fn is_unified_mode(&self) -> bool {
        self.imp().cache_folder_id.get() == -1
    }

    /// Resolve a folder_id to (account_id, folder_path) via DB lookup
    /// Used in unified inbox mode to find which account/folder a message belongs to
    pub fn resolve_folder_info(&self, folder_id: i64) -> Option<(String, String)> {
        let db = self.database()?.clone();

        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(db.get_folder_by_id(folder_id));
            let _ = sender.send(result);
        });

        match receiver.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(Ok(Some(folder))) => Some((folder.account_id, folder.full_path)),
            _ => None,
        }
    }

    /// Fetch and display unified inbox (all inbox folders across all accounts)
    pub fn fetch_unified_inbox(&self) {
        let app = self.clone();

        // Set state
        {
            let mut state = self.imp().state.borrow_mut();
            state.unified_inbox = true;
            state.last_folder = None;
            state.save();
        }

        // Clear previous load state and set unified sentinel
        self.imp().folder_load_state.replace(None);
        self.imp().cache_offset.set(0);
        self.imp().cache_folder_id.set(-1);

        // Increment fetch generation
        let generation = self.imp().fetch_generation.get() + 1;
        self.imp().fetch_generation.set(generation);

        let db = match self.database() {
            Some(db) => db.clone(),
            None => {
                self.show_error("Database not available");
                return;
            }
        };

        glib::spawn_future_local(async move {
            info!("Fetching unified inbox (all accounts)");

            let (sender, receiver) = std::sync::mpsc::channel();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async {
                    let messages = db.get_inbox_messages(100, 0).await?;
                    let total = db.get_inbox_message_count().await?;
                    Ok::<_, northmail_core::CoreError>((messages, total))
                });
                let _ = sender.send(result);
            });

            // Poll for result
            let result = loop {
                match receiver.try_recv() {
                    Ok(result) => break Some(result),
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        glib::timeout_future(std::time::Duration::from_millis(10)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break None,
                }
            };

            match result {
                Some(Ok((messages, total))) => {
                    let loaded_count = messages.len() as i64;
                    info!(
                        "Unified inbox: loaded {} of {} messages",
                        loaded_count, total
                    );

                    app.imp().cache_offset.set(loaded_count);

                    let message_infos: Vec<MessageInfo> =
                        messages.iter().map(MessageInfo::from).collect();

                    if let Some(window) = app.active_window() {
                        if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                            win.restore_message_list();
                            if let Some(message_list) = win.message_list() {
                                message_list.set_messages(message_infos);

                                // Wire up "load more" from cache
                                let app_clone = app.clone();
                                message_list.connect_load_more(move || {
                                    app_clone.load_more_from_cache();
                                });

                                // Check if there are more messages in cache
                                let has_more = loaded_count < total;
                                message_list.set_can_load_more(has_more);
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    error!("Failed to load unified inbox: {}", e);
                    app.show_error(&format!("Failed to load inbox: {}", e));
                }
                None => {
                    warn!("Unified inbox load channel disconnected");
                }
            }
        });
    }

    /// Handle filter-changed: re-query DB with current filter state
    pub fn handle_filter_changed(&self) {
        let folder_id = self.imp().cache_folder_id.get();
        if folder_id == 0 {
            return;
        }

        let db = match self.database() {
            Some(db) => db.clone(),
            None => return,
        };

        // Read filter state from the message list widget
        let filter = if let Some(window) = self.active_window() {
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                if let Some(message_list) = win.message_list() {
                    message_list.get_message_filter()
                } else {
                    return;
                }
            } else {
                return;
            }
        } else {
            return;
        };

        let batch_size: i64 = 100;
        let app = self.clone();

        glib::spawn_future_local(async move {
            let (sender, receiver) = std::sync::mpsc::channel();
            let fid = folder_id;
            let f = filter.clone();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async {
                    let (messages, total) = if f.is_active() {
                        let msgs = if fid == -1 {
                            db.get_inbox_messages_filtered(batch_size, 0, &f).await?
                        } else {
                            db.get_messages_filtered(fid, batch_size, 0, &f).await?
                        };
                        let count = if fid == -1 {
                            db.get_inbox_messages_filtered_count(&f).await?
                        } else {
                            db.get_messages_filtered_count(fid, &f).await?
                        };
                        (msgs, count)
                    } else {
                        // No filter active: reload default page
                        let msgs = if fid == -1 {
                            db.get_inbox_messages(batch_size, 0).await?
                        } else {
                            db.get_messages(fid, batch_size, 0).await?
                        };
                        let count = if fid == -1 {
                            db.get_inbox_message_count().await?
                        } else {
                            db.get_message_count(fid).await?
                        };
                        (msgs, count)
                    };
                    Ok::<_, northmail_core::CoreError>((messages, total))
                });
                let _ = sender.send(result);
            });

            let result = loop {
                match receiver.try_recv() {
                    Ok(result) => break Some(result),
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        glib::timeout_future(std::time::Duration::from_millis(10)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break None,
                }
            };

            match result {
                Some(Ok((messages, total))) => {
                    let loaded = messages.len() as i64;
                    debug!("Filter query: {} results of {} total (filter_active={})",
                        loaded, total, filter.is_active());

                    app.imp().cache_offset.set(loaded);

                    let infos: Vec<MessageInfo> =
                        messages.iter().map(MessageInfo::from).collect();

                    if let Some(window) = app.active_window() {
                        if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                            if let Some(message_list) = win.message_list() {
                                message_list.set_messages(infos);
                                message_list.set_can_load_more(loaded < total);
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    error!("Filter query failed: {}", e);
                }
                None => {
                    warn!("Filter query channel disconnected");
                }
            }
        });
    }

    /// Parse an RFC 2822 date string into a Unix timestamp
    fn parse_date_epoch(date_str: &str) -> Option<i64> {
        let mut s = date_str.to_string();
        if let Some(paren) = s.rfind('(') {
            s = s[..paren].trim().to_string();
        }
        while s.contains("  ") {
            s = s.replace("  ", " ");
        }
        s = s.replace(" ,", ",");
        chrono::DateTime::parse_from_rfc2822(&s)
            .map(|dt| dt.timestamp())
            .ok()
    }

    /// Fetch a message body by UID
    pub fn fetch_message_body(&self, uid: u32, msg_folder_id: Option<i64>, callback: impl FnOnce(Result<ParsedEmailBody, String>) + 'static) {
        // Resolve account_id and folder_path: use folder_load_state if available,
        // otherwise resolve from msg_folder_id (unified inbox mode)
        let load_state = self.imp().folder_load_state.borrow().clone();
        let (resolved_account_id, resolved_folder_path) = if let Some(ref state) = load_state {
            (state.account_id.clone(), state.folder_path.clone())
        } else if let Some(fid) = msg_folder_id {
            match self.resolve_folder_info(fid) {
                Some((aid, fp)) => (aid, fp),
                None => {
                    error!("fetch_message_body: Could not resolve folder_id {}", fid);
                    callback(Err("Could not resolve folder".to_string()));
                    return;
                }
            }
        } else {
            error!("fetch_message_body: No folder_load_state and no msg_folder_id!");
            callback(Err("No folder selected".to_string()));
            return;
        };

        info!("fetch_message_body: uid={}, account={}, folder={}", uid, resolved_account_id, resolved_folder_path);

        let accounts = self.imp().accounts.borrow().clone();
        let account = match accounts.iter().find(|a| a.id == resolved_account_id) {
            Some(a) => a.clone(),
            None => {
                error!("fetch_message_body: Account not found! Looking for '{}', have: {:?}",
                    resolved_account_id,
                    accounts.iter().map(|a| &a.id).collect::<Vec<_>>());
                callback(Err("Account not found".to_string()));
                return;
            }
        };

        let folder_path = resolved_folder_path;
        let is_google = Self::is_google_account(&account);
        let is_microsoft = Self::is_microsoft_account(&account);
        let imap_host = account.imap_host.clone();
        let imap_username = account.imap_username.clone();
        let account_id = account.id.clone();
        let account_email = account.email.clone();
        let db = self.database().cloned();
        let pool = self.imap_pool();

        glib::spawn_future_local(async move {
            // First, check cache for message body
            if let Some(ref db) = db {
                if let Some(cached_body) = Self::get_cached_body(db, &account_id, &folder_path, uid).await {
                    info!("📧 Using cached body for message {} - instant display!", uid);
                    callback(Ok(cached_body));
                    return;
                }
            }

            info!("🌐 Fetching body from IMAP for message {} (not in cache)", uid);

            // Not in cache, fetch from IMAP using connection pool
            match AuthManager::new().await {
                Ok(auth_manager) => {
                    // Build credentials for pool
                    let credentials = if is_google {
                        match auth_manager.get_xoauth2_token_for_goa(&account_id).await {
                            Ok((email, access_token)) => {
                                Some(ImapCredentials::Gmail { email, access_token })
                            }
                            Err(e) => { callback(Err(format!("Auth failed: {}", e))); return; }
                        }
                    } else if is_microsoft {
                        match auth_manager.get_xoauth2_token_for_goa(&account_id).await {
                            Ok((email, access_token)) => {
                                Some(ImapCredentials::Microsoft { email, access_token })
                            }
                            Err(e) => { callback(Err(format!("Auth failed: {}", e))); return; }
                        }
                    } else {
                        let username = imap_username.unwrap_or(account_email);
                        let host = imap_host.unwrap_or_else(|| "imap.mail.me.com".to_string());
                        match auth_manager.get_goa_password(&account_id).await {
                            Ok(password) => {
                                Some(ImapCredentials::Password {
                                    host,
                                    port: 993,
                                    username,
                                    password,
                                })
                            }
                            Err(e) => { callback(Err(format!("Auth failed: {}", e))); return; }
                        }
                    };

                    let Some(credentials) = credentials else {
                        callback(Err("Failed to get credentials".to_string()));
                        return;
                    };

                    // Use pool to fetch body (reuses existing connection)
                    let result = Self::fetch_body_via_pool(&pool, credentials, &folder_path, uid).await;

                    // Save to cache if successful
                    if let Ok(ref body) = result {
                        if let Some(ref db) = db {
                            Self::save_body_to_cache(db, &account_id, &folder_path, uid, body);
                        }
                    }

                    callback(result);
                }
                Err(e) => {
                    callback(Err(format!("Auth manager error: {}", e)));
                }
            }
        });
    }

    /// Fetch body using connection pool (reuses existing IMAP connection)
    async fn fetch_body_via_pool(
        pool: &std::sync::Arc<ImapPool>,
        credentials: ImapCredentials,
        folder_path: &str,
        uid: u32,
    ) -> Result<ParsedEmailBody, String> {
        info!("fetch_body_via_pool: uid={} folder={}", uid, folder_path);

        // Try up to 2 times (retry once on connection failure)
        for attempt in 0..2 {
            let worker = pool.get_or_create(credentials.clone())
                .map_err(|e| format!("Pool error: {}", e))?;

            let (response_tx, response_rx) = std::sync::mpsc::channel();

            // Send fetch command - if send fails, worker is dead
            if let Err(e) = worker.send(ImapCommand::FetchBody {
                folder: folder_path.to_string(),
                uid,
                response_tx,
            }) {
                warn!("fetch_body_via_pool: send failed (attempt {}): {}", attempt, e);
                pool.remove_worker(&credentials);
                if attempt == 0 { continue; }
                return Err(format!("Failed to send command: {}", e));
            }

            debug!("fetch_body_via_pool: command sent, waiting for response");

            let timeout = std::time::Duration::from_secs(45);
            let start = std::time::Instant::now();

            loop {
                match response_rx.try_recv() {
                    Ok(ImapResponse::Body(body)) => {
                        info!("fetch_body_via_pool: got body, {} bytes", body.len());
                        return Ok(Self::parse_email_body(&body));
                    }
                    Ok(ImapResponse::Error(e)) => {
                        // If connection failed, remove stale worker and retry
                        if e.contains("Connection failed") && attempt == 0 {
                            warn!("fetch_body_via_pool: connection failed, retrying...");
                            pool.remove_worker(&credentials);
                            break; // break inner loop, continue outer
                        }
                        error!("fetch_body_via_pool: error: {}", e);
                        return Err(e);
                    }
                    Ok(other) => {
                        debug!("fetch_body_via_pool: unexpected response: {:?}", other);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        if start.elapsed() > timeout {
                            return Err(format!("Timeout waiting for body of message {}", uid));
                        }
                        glib::timeout_future(std::time::Duration::from_millis(50)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if attempt == 0 {
                            warn!("fetch_body_via_pool: worker disconnected, retrying...");
                            pool.remove_worker(&credentials);
                            break;
                        }
                        return Err("Pool worker disconnected".to_string());
                    }
                }
            }
        }
        Err(format!("Failed to fetch body for message {} after retries", uid))
    }

    /// Get cached message body from database
    async fn get_cached_body(
        db: &std::sync::Arc<northmail_core::Database>,
        account_id: &str,
        folder_path: &str,
        uid: u32,
    ) -> Option<ParsedEmailBody> {
        let db = db.clone();
        let account_id = account_id.to_string();
        let folder_path = folder_path.to_string();

        let (sender, receiver) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
                let folder_id = db.get_or_create_folder_id(&account_id, &folder_path).await?;
                db.get_message_body(folder_id, uid as i64).await
            });
            let _ = sender.send(result);
        });

        let timeout = std::time::Duration::from_secs(1);
        match receiver.recv_timeout(timeout) {
            Ok(Ok(Some((body_text, body_html)))) => {
                // Only return if we have at least one body part
                if body_text.is_some() || body_html.is_some() {
                    info!("📧 Body cache HIT: Found cached body for message {}", uid);
                    Some(ParsedEmailBody {
                        text: body_text,
                        html: body_html,
                    })
                } else {
                    info!("📭 Body cache MISS: No cached body for message {}", uid);
                    None
                }
            }
            _ => None,
        }
    }

    /// Save message body to cache (fire and forget)
    fn save_body_to_cache(
        db: &std::sync::Arc<northmail_core::Database>,
        account_id: &str,
        folder_path: &str,
        uid: u32,
        body: &ParsedEmailBody,
    ) {
        let db = db.clone();
        let account_id = account_id.to_string();
        let folder_path = folder_path.to_string();
        let body_text = body.text.clone();
        let body_html = body.html.clone();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                if let Ok(folder_id) = db.get_or_create_folder_id(&account_id, &folder_path).await {
                    if let Err(e) = db
                        .save_message_body(
                            folder_id,
                            uid as i64,
                            body_text.as_deref(),
                            body_html.as_deref(),
                        )
                        .await
                    {
                        warn!("Failed to cache message body: {}", e);
                    } else {
                        info!("💾 Body cache SAVE: Cached body for message {}", uid);
                    }
                }
            });
        });
    }

    /// Fetch body using OAuth2 (Gmail or Microsoft)
    async fn fetch_body_oauth2(
        email: String,
        access_token: String,
        folder_path: &str,
        uid: u32,
        is_gmail: bool,
    ) -> Result<ParsedEmailBody, String> {
        let (sender, receiver) = std::sync::mpsc::channel::<Result<String, String>>();
        let folder_path = folder_path.to_string();

        info!(
            "fetch_body_oauth2: fetching uid {} from folder '{}' for {} (gmail: {})",
            uid, folder_path, email, is_gmail
        );

        std::thread::spawn(move || {
            async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();

                let connect_result = if is_gmail {
                    client.connect_gmail(&email, &access_token).await
                } else {
                    client.connect_outlook(&email, &access_token).await
                };

                match connect_result {
                    Ok(_) => {
                        debug!("fetch_body_oauth2: connected to server");
                        match client.select(&folder_path).await {
                            Ok(folder_info) => {
                                debug!(
                                    "fetch_body_oauth2: selected folder, {} messages",
                                    folder_info.message_count.unwrap_or(0)
                                );
                                match client.fetch_body(uid).await {
                                    Ok(body) => {
                                        debug!("fetch_body_oauth2: got body, {} bytes", body.len());
                                        let _ = client.logout().await;
                                        let _ = sender.send(Ok(body));
                                    }
                                    Err(e) => {
                                        error!("fetch_body_oauth2: fetch failed: {}", e);
                                        let _ = client.logout().await;
                                        let _ = sender.send(Err(format!("Fetch failed: {}", e)));
                                    }
                                }
                            }
                            Err(e) => {
                                error!("fetch_body_oauth2: select failed for folder '{}': {}", folder_path, e);
                                let _ = client.logout().await;
                                let _ = sender.send(Err(format!("Select failed: {}", e)));
                            }
                        }
                    }
                    Err(e) => {
                        error!("fetch_body_oauth2: connect failed: {}", e);
                        let _ = sender.send(Err(format!("Connect failed: {}", e)));
                    }
                }
            });
        });

        // Poll for result
        loop {
            match receiver.try_recv() {
                Ok(result) => {
                    return result.map(|body| Self::parse_email_body(&body));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::timeout_future(std::time::Duration::from_millis(50)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err("Thread disconnected".to_string());
                }
            }
        }
    }

    /// Fetch body using connection pool (reuses existing connection)
    async fn fetch_body_pooled(
        pool: &std::sync::Arc<ImapPool>,
        credentials: ImapCredentials,
        folder_path: &str,
        uid: u32,
    ) -> Result<ParsedEmailBody, String> {
        debug!("fetch_body_pooled: getting worker for {}", credentials.pool_key());

        // Get or create a worker from the pool
        let worker = match pool.get_or_create(credentials.clone()) {
            Ok(w) => {
                debug!("fetch_body_pooled: got worker successfully");
                w
            }
            Err(e) => {
                error!("fetch_body_pooled: failed to get worker: {}", e);
                return Err(e);
            }
        };

        // Create response channel
        let (response_tx, response_rx) = std::sync::mpsc::channel();

        debug!("fetch_body_pooled: sending FetchBody command for uid {} in {}", uid, folder_path);

        // Send fetch command
        worker
            .send(ImapCommand::FetchBody {
                folder: folder_path.to_string(),
                uid,
                response_tx,
            })
            .map_err(|e| {
                error!("fetch_body_pooled: failed to send command: {}", e);
                format!("Failed to send command: {}", e)
            })?;

        debug!("fetch_body_pooled: command sent, waiting for response");

        // Wait for response with timeout
        let timeout = std::time::Duration::from_secs(30);
        let start = std::time::Instant::now();

        loop {
            match response_rx.try_recv() {
                Ok(ImapResponse::Body(body)) => {
                    info!("♻️ Received body via pooled connection");
                    return Ok(Self::parse_email_body(&body));
                }
                Ok(ImapResponse::Error(e)) => {
                    error!("Pool fetch body error: {}", e);
                    return Err(e);
                }
                Ok(other) => {
                    debug!("Unexpected response: {:?}", other);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if start.elapsed() > timeout {
                        return Err("Timeout waiting for body".to_string());
                    }
                    glib::timeout_future(std::time::Duration::from_millis(50)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err("Worker disconnected".to_string());
                }
            }
        }
    }

    /// Fetch body using password auth
    async fn fetch_body_password(
        host: String,
        username: String,
        password: String,
        folder_path: &str,
        uid: u32,
    ) -> Result<ParsedEmailBody, String> {
        let (sender, receiver) = std::sync::mpsc::channel::<Result<String, String>>();
        let folder_path = folder_path.to_string();

        std::thread::spawn(move || {
            async_std::task::block_on(async {
                let mut client = ImapClient::new(&host, 993);

                match client.authenticate_login(&username, &password).await {
                    Ok(_) => {
                        match client.select_folder(&folder_path).await {
                            Ok(_) => {
                                match client.fetch_body(uid).await {
                                    Ok(body_bytes) => {
                                        let _ = client.logout().await;
                                        // Convert bytes to string
                                        let body = String::from_utf8_lossy(&body_bytes).into_owned();
                                        let _ = sender.send(Ok(body));
                                    }
                                    Err(e) => {
                                        let _ = client.logout().await;
                                        let _ = sender.send(Err(format!("Fetch failed: {}", e)));
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = client.logout().await;
                                let _ = sender.send(Err(format!("Select failed: {}", e)));
                            }
                        }
                    }
                    Err(e) => {
                        let _ = sender.send(Err(format!("Connect failed: {}", e)));
                    }
                }
            });
        });

        // Poll for result
        loop {
            match receiver.try_recv() {
                Ok(result) => {
                    return result.map(|body| Self::parse_email_body(&body));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::timeout_future(std::time::Duration::from_millis(50)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err("Thread disconnected".to_string());
                }
            }
        }
    }

    /// Parse raw email body to extract text and HTML parts
    fn parse_email_body(raw: &str) -> ParsedEmailBody {
        let mut result = ParsedEmailBody::default();

        // Find the boundary if this is multipart
        let boundary = Self::find_boundary(raw);

        if let Some(boundary) = boundary {
            // Multipart message - find text/plain and text/html parts
            // Skip first element (preamble before first boundary marker)
            let parts: Vec<&str> = raw.split(&format!("--{}", boundary)).collect();

            for part in parts.iter().skip(1) {
                let part = part.trim();
                if part.is_empty() || part == "--" || part.starts_with("--") {
                    continue;
                }

                // Find content-type header in this part
                let content_type = Self::find_part_header(part, "Content-Type");
                let transfer_encoding = Self::find_part_header(part, "Content-Transfer-Encoding");

                // Split headers from body - headers end at first blank line
                let body = Self::extract_body_after_headers(part);

                if let Some(ct) = &content_type {
                    let ct_lower = ct.to_lowercase();
                    if ct_lower.contains("text/plain") && result.text.is_none() {
                        if !body.is_empty() {
                            result.text = Some(Self::decode_body_content(&body, transfer_encoding.as_deref()));
                        }
                    } else if ct_lower.contains("text/html") && result.html.is_none() {
                        if !body.is_empty() {
                            result.html = Some(Self::decode_body_content(&body, transfer_encoding.as_deref()));
                        }
                    } else if ct_lower.contains("multipart/") {
                        // Any nested multipart (alternative, related, mixed) - parse recursively
                        if let Some(nested_boundary) = Self::find_boundary_in_header(ct) {
                            let nested = Self::parse_multipart_recursive(part, &nested_boundary);
                            if result.text.is_none() && nested.text.is_some() {
                                result.text = nested.text;
                            }
                            if result.html.is_none() && nested.html.is_some() {
                                result.html = nested.html;
                            }
                        }
                    }
                } else if body.is_empty() {
                    continue;
                }
            }

            // Log when parsing yields no content for debugging
            if result.text.is_none() && result.html.is_none() {
                let top_ct = Self::find_part_header(raw, "Content-Type").unwrap_or_default();
                warn!("parse_email_body: No text/html found! Top Content-Type: {}", top_ct);
            }
        } else {
            // Single part message
            let content_type = Self::find_part_header(raw, "Content-Type");
            let transfer_encoding = Self::find_part_header(raw, "Content-Transfer-Encoding");

            // Find body after headers
            let body = Self::extract_body_after_headers(raw);
            if !body.is_empty() {
                let decoded = Self::decode_body_content(&body, transfer_encoding.as_deref());

                if let Some(ct) = content_type {
                    if ct.to_lowercase().contains("text/html") {
                        result.html = Some(decoded);
                    } else {
                        result.text = Some(decoded);
                    }
                } else {
                    // Assume plain text if no content-type
                    result.text = Some(decoded);
                }
            }
        }

        result
    }

    /// Parse a nested multipart section (alternative, related, mixed, etc.)
    fn parse_multipart_recursive(content: &str, boundary: &str) -> ParsedEmailBody {
        let mut result = ParsedEmailBody::default();
        // Skip first element (preamble before first boundary marker)
        let parts: Vec<&str> = content.split(&format!("--{}", boundary)).collect();

        for part in parts.iter().skip(1) {
            let part = part.trim();
            if part.is_empty() || part == "--" || part.starts_with("--") {
                continue;
            }

            let content_type = Self::find_part_header(part, "Content-Type");
            let transfer_encoding = Self::find_part_header(part, "Content-Transfer-Encoding");

            let body = Self::extract_body_after_headers(part);

            if let Some(ct) = &content_type {
                let ct_lower = ct.to_lowercase();
                if ct_lower.contains("text/plain") && result.text.is_none() {
                    if !body.is_empty() {
                        result.text = Some(Self::decode_body_content(&body, transfer_encoding.as_deref()));
                    }
                } else if ct_lower.contains("text/html") && result.html.is_none() {
                    if !body.is_empty() {
                        result.html = Some(Self::decode_body_content(&body, transfer_encoding.as_deref()));
                    }
                } else if ct_lower.contains("multipart/") {
                    // Recursively parse nested multipart
                    if let Some(nested_boundary) = Self::find_boundary_in_header(ct) {
                        let nested = Self::parse_multipart_recursive(part, &nested_boundary);
                        if result.text.is_none() && nested.text.is_some() {
                            result.text = nested.text;
                        }
                        if result.html.is_none() && nested.html.is_some() {
                            result.html = nested.html;
                        }
                    }
                }
            }
        }

        result
    }

    /// Extract body content after headers (headers end at blank line)
    fn extract_body_after_headers(content: &str) -> String {
        // Find the blank line that separates headers from body
        if let Some(pos) = content.find("\r\n\r\n") {
            content[pos + 4..].to_string()
        } else if let Some(pos) = content.find("\n\n") {
            content[pos + 2..].to_string()
        } else {
            // No blank line found - might be all headers or all body
            // Check if first line looks like a header
            if let Some(first_line) = content.lines().next() {
                if first_line.contains(':') && !first_line.starts_with(' ') && !first_line.starts_with('\t') {
                    // Looks like headers only
                    String::new()
                } else {
                    // Assume it's all body
                    content.to_string()
                }
            } else {
                String::new()
            }
        }
    }

    /// Find a header in a MIME part (only searches the header section)
    fn find_part_header(part: &str, header_name: &str) -> Option<String> {
        let header_lower = header_name.to_lowercase();

        // Only look at lines before the first blank line (header section)
        let header_section = if let Some(pos) = part.find("\r\n\r\n") {
            &part[..pos]
        } else if let Some(pos) = part.find("\n\n") {
            &part[..pos]
        } else {
            part
        };

        let mut current_header: Option<(String, String)> = None;

        for line in header_section.lines() {
            // Check for continuation line (starts with whitespace)
            if (line.starts_with(' ') || line.starts_with('\t')) && current_header.is_some() {
                if let Some((_, ref mut value)) = current_header {
                    value.push(' ');
                    value.push_str(line.trim());
                }
                continue;
            }

            // Check if previous header matches what we're looking for
            if let Some((name, value)) = current_header.take() {
                if name.to_lowercase() == header_lower {
                    return Some(value);
                }
            }

            // Parse new header
            if let Some(colon_pos) = line.find(':') {
                let name = line[..colon_pos].trim().to_string();
                let value = line[colon_pos + 1..].trim().to_string();
                current_header = Some((name, value));
            }
        }

        // Check last header
        if let Some((name, value)) = current_header {
            if name.to_lowercase() == header_lower {
                return Some(value);
            }
        }

        None
    }

    /// Find boundary in a Content-Type header value
    fn find_boundary_in_header(content_type: &str) -> Option<String> {
        if let Some(boundary_start) = content_type.find("boundary=") {
            let after = &content_type[boundary_start + 9..];
            let boundary = if after.starts_with('"') {
                after[1..].split('"').next()
            } else {
                after.split(&[';', ' ', '\r', '\n'][..]).next()
            };
            return boundary.map(|s| s.to_string());
        }
        None
    }

    /// Find MIME boundary in email
    fn find_boundary(raw: &str) -> Option<String> {
        // First, try to find boundary in Content-Type header
        let mut in_content_type = false;
        let mut content_type_value = String::new();

        for line in raw.lines() {
            let line_lower = line.to_lowercase();

            // Check for Content-Type header start
            if line_lower.starts_with("content-type:") {
                in_content_type = true;
                content_type_value = line.to_string();
                continue;
            }

            // Check for continuation of Content-Type header
            if in_content_type && (line.starts_with(' ') || line.starts_with('\t')) {
                content_type_value.push_str(line);
                continue;
            }

            // End of Content-Type header
            if in_content_type && !line.is_empty() {
                in_content_type = false;
                if let Some(boundary) = Self::find_boundary_in_header(&content_type_value) {
                    return Some(boundary);
                }
            }

            // Empty line means end of headers
            if line.is_empty() && !content_type_value.is_empty() {
                if let Some(boundary) = Self::find_boundary_in_header(&content_type_value) {
                    return Some(boundary);
                }
                break;
            }
        }

        // Check the accumulated Content-Type if we haven't found boundary yet
        if !content_type_value.is_empty() {
            if let Some(boundary) = Self::find_boundary_in_header(&content_type_value) {
                return Some(boundary);
            }
        }

        // Fallback: Try to detect boundary from content itself
        // Look for lines that look like MIME boundaries (start with --)
        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("--") && !trimmed.starts_with("---") && trimmed.len() > 10 {
                // This looks like a boundary line, extract the boundary (without leading --)
                let potential_boundary = &trimmed[2..];
                // Remove trailing -- if present (end marker)
                let boundary = potential_boundary.trim_end_matches("--").to_string();
                if !boundary.is_empty() && !boundary.contains(' ') {
                    return Some(boundary);
                }
            }
        }

        None
    }
    /// Decode body content based on transfer encoding
    fn decode_body_content(body: &str, transfer_encoding: Option<&str>) -> String {
        match transfer_encoding.map(|s| s.to_lowercase()).as_deref() {
            Some("base64") => {
                // Remove whitespace and decode
                let clean: String = body.chars().filter(|c| !c.is_whitespace()).collect();
                match base64::prelude::BASE64_STANDARD.decode(&clean) {
                    Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    Err(_) => body.to_string(),
                }
            }
            Some("quoted-printable") => {
                Self::decode_quoted_printable_body(body)
            }
            _ => body.to_string(),
        }
    }

    /// Decode quoted-printable body
    fn decode_quoted_printable_body(input: &str) -> String {
        let mut result = Vec::new();
        let mut lines = input.lines().peekable();

        while let Some(line) = lines.next() {
            let mut chars = line.chars().peekable();

            while let Some(c) = chars.next() {
                match c {
                    '=' => {
                        // Check for soft line break or hex encoding
                        let next1 = chars.next();
                        let next2 = chars.next();

                        match (next1, next2) {
                            (Some(h1), Some(h2)) => {
                                // Try to decode as hex
                                let hex = format!("{}{}", h1, h2);
                                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                                    result.push(byte);
                                } else {
                                    result.push(b'=');
                                    result.push(h1 as u8);
                                    result.push(h2 as u8);
                                }
                            }
                            (None, _) => {
                                // Soft line break - continue to next line
                            }
                            (Some(h1), None) => {
                                result.push(b'=');
                                result.push(h1 as u8);
                            }
                        }
                    }
                    _ if c.is_ascii() => result.push(c as u8),
                    _ => {
                        // Non-ASCII, encode as UTF-8
                        let mut buf = [0u8; 4];
                        result.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                    }
                }
            }

            // Add newline unless this line ended with a soft break
            if !line.ends_with('=') {
                result.push(b'\n');
            }
        }

        String::from_utf8_lossy(&result).into_owned()
    }

    /// Strip HTML tags from content (public wrapper)
    pub fn strip_html_tags_public(html: &str) -> String {
        Self::strip_html_tags(html)
    }

    /// Strip HTML tags from content
    fn strip_html_tags(html: &str) -> String {
        let mut result = String::new();
        let mut in_tag = false;
        let mut in_script = false;
        let mut in_style = false;

        let html_lower = html.to_lowercase();
        let chars: Vec<char> = html.chars().collect();
        let chars_lower: Vec<char> = html_lower.chars().collect();

        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];

            if c == '<' {
                // Check for script/style tags
                let remaining: String = chars_lower[i..].iter().collect();
                if remaining.starts_with("<script") {
                    in_script = true;
                } else if remaining.starts_with("</script") {
                    in_script = false;
                } else if remaining.starts_with("<style") {
                    in_style = true;
                } else if remaining.starts_with("</style") {
                    in_style = false;
                } else if remaining.starts_with("<br") || remaining.starts_with("<p") || remaining.starts_with("<div") {
                    result.push('\n');
                }
                in_tag = true;
            } else if c == '>' {
                in_tag = false;
            } else if !in_tag && !in_script && !in_style {
                result.push(c);
            }

            i += 1;
        }

        // Decode HTML entities
        let result = result
            .replace("&nbsp;", " ")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#39;", "'");

        // Clean up excessive whitespace
        let mut cleaned = String::new();
        let mut last_was_newline = false;
        for c in result.chars() {
            if c == '\n' {
                if !last_was_newline {
                    cleaned.push(c);
                    last_was_newline = true;
                }
            } else {
                cleaned.push(c);
                last_was_newline = false;
            }
        }

        cleaned.trim().to_string()
    }

    fn show_error(&self, message: &str) {
        if let Some(window) = self.active_window() {
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                // Restore message list (remove spinner if showing)
                win.restore_message_list();

                let toast = adw::Toast::new(message);
                toast.set_timeout(5);
                win.add_toast(toast);
            }
        }
    }

    fn show_toast(&self, message: &str) {
        if let Some(window) = self.active_window() {
            let toast = adw::Toast::new(message);
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                win.add_toast(toast);
            }
        }
    }

    fn setup_actions(&self) {
        // Quit action
        let quit_action = gio::ActionEntry::builder("quit")
            .activate(|app: &Self, _, _| {
                info!("Quit action triggered");
                app.quit();
            })
            .build();

        // About action
        let about_action = gio::ActionEntry::builder("about")
            .activate(|app: &Self, _, _| {
                app.show_about_dialog();
            })
            .build();

        // Add account action
        let add_account_action = gio::ActionEntry::builder("add-account")
            .activate(|app: &Self, _, _| {
                app.show_add_account_dialog();
            })
            .build();

        // Preferences/Settings action
        let preferences_action = gio::ActionEntry::builder("preferences")
            .activate(|app: &Self, _, _| {
                app.show_settings_window();
            })
            .build();

        // Show settings action (same as preferences, for sidebar button)
        let show_settings_action = gio::ActionEntry::builder("show-settings")
            .activate(|app: &Self, _, _| {
                app.show_settings_window();
            })
            .build();

        self.add_action_entries([
            quit_action,
            about_action,
            add_account_action,
            preferences_action,
            show_settings_action,
        ]);

        // Set up keyboard shortcuts
        self.set_accels_for_action("app.quit", &["<primary>q"]);
        self.set_accels_for_action("app.preferences", &["<primary>comma"]);
        self.set_accels_for_action("win.compose", &["<primary>n"]);
        self.set_accels_for_action("win.refresh", &["<primary>r", "F5"]);
    }

    fn show_about_dialog(&self) {
        let about = adw::AboutDialog::builder()
            .application_name("NorthMail")
            .application_icon("mail-send-receive")
            .developer_name("NorthMail Contributors")
            .version("0.1.0")
            .copyright("© 2024 NorthMail Contributors")
            .license_type(gtk4::License::Gpl30)
            .website("https://github.com/northmail/northmail")
            .issue_url("https://github.com/northmail/northmail/issues")
            .comments("A modern email client for GNOME")
            .build();

        about.add_acknowledgement_section(
            Some("Built With"),
            &["GTK4", "libadwaita", "Rust", "async-imap"],
        );

        if let Some(window) = self.active_window() {
            about.present(Some(&window));
        }
    }

    fn show_add_account_dialog(&self) {
        let app = self.clone();

        // Check for GOA accounts first (use glib async since AuthManager isn't Send)
        glib::spawn_future_local(async move {
            match AuthManager::new().await {
                Ok(auth_manager) => {
                    if auth_manager.is_goa_available() {
                        match auth_manager.list_goa_accounts().await {
                            Ok(accounts) => {
                                if !accounts.is_empty() {
                                    info!("Found {} GOA mail accounts", accounts.len());
                                    app.show_goa_account_selector(accounts);
                                    return;
                                }
                            }
                            Err(e) => {
                                warn!("Failed to list GOA accounts: {}", e);
                            }
                        }
                    }

                    // No GOA accounts, show OAuth2 flow option
                    app.show_oauth2_account_dialog();
                }
                Err(e) => {
                    error!("Failed to create auth manager: {}", e);
                }
            }
        });
    }

    fn show_goa_account_selector(&self, accounts: Vec<northmail_auth::GoaAccount>) {
        let dialog = adw::AlertDialog::builder()
            .heading("Add Email Account")
            .body("Select an account from GNOME Online Accounts or add a new one.")
            .build();

        for account in &accounts {
            dialog.add_response(
                &account.id,
                &format!("{} ({})", account.email, account.provider_name),
            );
        }

        dialog.add_response("settings", "Open Settings...");
        dialog.add_response("cancel", "Cancel");

        dialog.set_response_appearance("cancel", adw::ResponseAppearance::Default);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let app = self.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "settings" {
                // Open GNOME Settings to Online Accounts
                let _ = gio::AppInfo::launch_default_for_uri(
                    "gnome-control-center://online-accounts",
                    gio::AppLaunchContext::NONE,
                );
            } else if response != "cancel" {
                // Selected a GOA account
                info!("Selected GOA account: {}", response);
                app.add_goa_account(response);
            }
        });

        if let Some(window) = self.active_window() {
            dialog.present(Some(&window));
        }
    }

    fn show_oauth2_account_dialog(&self) {
        let dialog = adw::AlertDialog::builder()
            .heading("Add Gmail Account")
            .body("NorthMail will open your browser to authenticate with Google.")
            .build();

        dialog.add_response("cancel", "Cancel");
        dialog.add_response("authenticate", "Authenticate");
        dialog.set_response_appearance("authenticate", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("authenticate"));
        dialog.set_close_response("cancel");

        let app = self.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "authenticate" {
                app.start_oauth2_flow();
            }
        });

        if let Some(window) = self.active_window() {
            dialog.present(Some(&window));
        }
    }

    fn add_goa_account(&self, account_id: &str) {
        let account_id = account_id.to_string();
        let app = self.clone();

        // Use glib async since AuthManager isn't Send
        glib::spawn_future_local(async move {
            match AuthManager::new().await {
                Ok(auth_manager) => match auth_manager.list_goa_accounts().await {
                    Ok(accounts) => {
                        if let Some(goa_account) = accounts.iter().find(|a| a.id == account_id) {
                            info!("Adding GOA account: {}", goa_account.email);
                            app.show_toast(&format!("Added account: {}", goa_account.email));
                            // TODO: Save to database and start sync
                        }
                    }
                    Err(e) => {
                        error!("Failed to get GOA account: {}", e);
                        app.show_error(&format!("Failed to add account: {}", e));
                    }
                },
                Err(e) => {
                    error!("Failed to create auth manager: {}", e);
                    app.show_error(&format!("Failed to add account: {}", e));
                }
            }
        });
    }

    fn start_oauth2_flow(&self) {
        info!("Starting OAuth2 flow");

        // TODO: Implement standalone OAuth2 flow
        let dialog = adw::AlertDialog::builder()
            .heading("Not Implemented")
            .body("Standalone OAuth2 is not yet implemented. Please add your Gmail account in GNOME Settings → Online Accounts first.")
            .build();

        dialog.add_response("ok", "OK");
        dialog.add_response("settings", "Open Settings");
        dialog.set_default_response(Some("settings"));
        dialog.set_close_response("ok");

        dialog.connect_response(None, |_, response| {
            if response == "settings" {
                let _ = gio::AppInfo::launch_default_for_uri(
                    "gnome-control-center://online-accounts",
                    gio::AppLaunchContext::NONE,
                );
            }
        });

        if let Some(window) = self.active_window() {
            dialog.present(Some(&window));
        }
    }

    fn show_settings_window(&self) {
        let dialog = adw::PreferencesDialog::new();
        dialog.set_title("Settings");

        // General page
        let general_page = adw::PreferencesPage::builder()
            .title("General")
            .icon_name("preferences-system-symbolic")
            .build();

        let appearance_group = adw::PreferencesGroup::builder()
            .title("Appearance")
            .build();

        let theme_row = adw::ComboRow::builder()
            .title("Color Scheme")
            .subtitle("Choose the application color scheme")
            .build();

        let themes = gtk4::StringList::new(&["System", "Light", "Dark"]);
        theme_row.set_model(Some(&themes));

        // Set initial selection to match current color scheme
        let style_manager = adw::StyleManager::default();
        let current = match style_manager.color_scheme() {
            adw::ColorScheme::ForceLight => 1u32,
            adw::ColorScheme::ForceDark => 2u32,
            _ => 0u32, // Default/System
        };
        theme_row.set_selected(current);

        // Wire up theme changes to AdwStyleManager
        theme_row.connect_selected_notify(move |row| {
            let scheme = match row.selected() {
                1 => adw::ColorScheme::ForceLight,
                2 => adw::ColorScheme::ForceDark,
                _ => adw::ColorScheme::Default,
            };
            adw::StyleManager::default().set_color_scheme(scheme);
        });

        appearance_group.add(&theme_row);
        general_page.add(&appearance_group);

        // Behavior group
        let behavior_group = adw::PreferencesGroup::builder().title("Behavior").build();

        let notifications_row = adw::SwitchRow::builder()
            .title("Desktop Notifications")
            .subtitle("Show notifications for new emails")
            .active(true)
            .build();

        let sound_row = adw::SwitchRow::builder()
            .title("Notification Sound")
            .subtitle("Play a sound when new emails arrive")
            .active(true)
            .build();

        behavior_group.add(&notifications_row);
        behavior_group.add(&sound_row);
        general_page.add(&behavior_group);

        dialog.add(&general_page);

        // Accounts page
        let accounts_page = adw::PreferencesPage::builder()
            .title("Accounts")
            .icon_name("system-users-symbolic")
            .build();

        // Info about GOA
        let info_group = adw::PreferencesGroup::builder()
            .description("NorthMail uses GNOME Online Accounts to manage your email accounts. Add or remove accounts in System Settings.")
            .build();

        accounts_page.add(&info_group);

        // Button to open GNOME Settings
        let settings_group = adw::PreferencesGroup::new();

        let open_settings_row = adw::ActionRow::builder()
            .title("Online Accounts")
            .subtitle("Manage accounts in GNOME Settings")
            .activatable(true)
            .build();

        open_settings_row.add_suffix(&gtk4::Image::from_icon_name("external-link-symbolic"));

        open_settings_row.connect_activated(|_| {
            let _ = gio::AppInfo::launch_default_for_uri(
                "gnome-control-center://online-accounts",
                gio::AppLaunchContext::NONE,
            );
        });

        settings_group.add(&open_settings_row);
        accounts_page.add(&settings_group);

        // Account cache statistics
        let cache_group = adw::PreferencesGroup::builder()
            .title("Cached Messages")
            .description("Messages stored locally for offline access and fast loading")
            .build();

        // Add a row for each account with message count
        let accounts = self.imp().accounts.borrow().clone();
        let db = self.database().cloned();

        for account in &accounts {
            let row = adw::ActionRow::builder()
                .title(&account.email)
                .subtitle("Loading...")
                .build();

            // Add loading indicator
            let spinner = gtk4::Spinner::builder()
                .spinning(true)
                .build();
            row.add_suffix(&spinner);

            cache_group.add(&row);

            // Load message count asynchronously
            if let Some(ref db) = db {
                let db = db.clone();
                let account_id = account.id.clone();
                let row_clone = row.clone();
                let spinner_clone = spinner.clone();

                glib::spawn_future_local(async move {
                    let (sender, receiver) = std::sync::mpsc::channel();
                    let db_for_thread = db.clone();
                    let account_id_clone = account_id.clone();

                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        let result = rt.block_on(async {
                            let msg_count = db_for_thread.get_account_message_count(&account_id_clone).await.unwrap_or(0);
                            let body_count = db_for_thread.get_account_body_count(&account_id_clone).await.unwrap_or(0);
                            (msg_count, body_count)
                        });
                        let _ = sender.send(result);
                    });

                    // Wait for result
                    loop {
                        match receiver.try_recv() {
                            Ok((msg_count, body_count)) => {
                                spinner_clone.set_spinning(false);
                                spinner_clone.set_visible(false);
                                row_clone.set_subtitle(&format!("{} messages, {} bodies cached", msg_count, body_count));
                                break;
                            }
                            Err(std::sync::mpsc::TryRecvError::Empty) => {
                                glib::timeout_future(std::time::Duration::from_millis(100)).await;
                            }
                            Err(_) => break,
                        }
                    }
                });
            }
        }

        accounts_page.add(&cache_group);

        // Cache management buttons
        let cache_actions_group = adw::PreferencesGroup::builder()
            .title("Cache Management")
            .build();

        // Clear all cache button
        let clear_cache_row = adw::ActionRow::builder()
            .title("Clear All Cache")
            .subtitle("Delete all cached messages and bodies")
            .activatable(true)
            .build();

        clear_cache_row.add_suffix(&gtk4::Image::from_icon_name("user-trash-symbolic"));

        let app_for_clear = self.clone();
        let dialog_ref = dialog.downgrade();
        clear_cache_row.connect_activated(move |_| {
            let app = app_for_clear.clone();
            let dialog_weak = dialog_ref.clone();

            glib::spawn_future_local(async move {
                if let Some(db) = app.database() {
                    let db = db.clone();
                    let (sender, receiver) = std::sync::mpsc::channel();

                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        let result = rt.block_on(async {
                            db.clear_all_cache().await
                        });
                        let _ = sender.send(result);
                    });

                    // Wait for result
                    loop {
                        match receiver.try_recv() {
                            Ok(Ok(())) => {
                                info!("Cache cleared successfully");
                                // Close and reopen dialog to refresh counts
                                if let Some(dialog) = dialog_weak.upgrade() {
                                    dialog.close();
                                }
                                app.show_settings_window();
                                break;
                            }
                            Ok(Err(e)) => {
                                error!("Failed to clear cache: {}", e);
                                break;
                            }
                            Err(std::sync::mpsc::TryRecvError::Empty) => {
                                glib::timeout_future(std::time::Duration::from_millis(100)).await;
                            }
                            Err(_) => break,
                        }
                    }
                }
            });
        });

        cache_actions_group.add(&clear_cache_row);

        // Reload all messages button
        let reload_row = adw::ActionRow::builder()
            .title("Reload All Messages")
            .subtitle("Re-sync all messages from all accounts")
            .activatable(true)
            .build();

        reload_row.add_suffix(&gtk4::Image::from_icon_name("view-refresh-symbolic"));

        let app_for_reload = self.clone();
        reload_row.connect_activated(move |_| {
            let app = app_for_reload.clone();
            // Trigger a full sync of all accounts
            app.sync_all_accounts();
        });

        cache_actions_group.add(&reload_row);
        accounts_page.add(&cache_actions_group);

        // Refresh accounts button
        let refresh_group = adw::PreferencesGroup::new();

        let refresh_button = gtk4::Button::builder()
            .label("Refresh Accounts")
            .halign(gtk4::Align::Center)
            .css_classes(["pill"])
            .build();

        let app = self.clone();
        refresh_button.connect_clicked(move |_| {
            app.load_accounts();
        });

        refresh_group.add(&refresh_button);
        accounts_page.add(&refresh_group);

        dialog.add(&accounts_page);

        if let Some(window) = self.active_window() {
            dialog.present(Some(&window));
        }
    }

    /// Send a message via SMTP using the selected account
    pub fn send_message(
        &self,
        account_index: u32,
        to: Vec<String>,
        cc: Vec<String>,
        subject: String,
        body: String,
        callback: impl FnOnce(Result<(), String>) + 'static,
    ) {
        let accounts = self.imp().accounts.borrow().clone();
        let account = match accounts.get(account_index as usize) {
            Some(a) => a.clone(),
            None => {
                callback(Err("Invalid account selection".to_string()));
                return;
            }
        };

        let smtp_host = account.smtp_host.clone().unwrap_or_else(|| {
            match account.provider_type.as_str() {
                "google" => "smtp.gmail.com".to_string(),
                "windows_live" | "microsoft" => "smtp.office365.com".to_string(),
                _ => "smtp.mail.me.com".to_string(),
            }
        });

        let account_id = account.id.clone();
        let email = account.email.clone();
        let auth_type = account.auth_type.clone();

        // Get user's display name from the system for From header
        let real_name = glib::real_name().to_string_lossy().to_string();
        let from_name = if real_name.is_empty() || real_name == "Unknown" {
            None
        } else {
            Some(real_name)
        };

        eprintln!("[send] account: {} ({}) smtp: {} auth: {:?}", email, account.provider_type, smtp_host, auth_type);
        eprintln!("[send] to: {:?}, cc: {:?}, subject: {:?}", to, cc, subject);
        if let Some(ref name) = from_name {
            eprintln!("[send] from_name: {:?}", name);
        }

        // Build OutgoingMessage
        let mut msg = northmail_smtp::OutgoingMessage::new(&email, &subject);
        if let Some(name) = from_name {
            msg = msg.from_name(name);
        }
        for addr in &to {
            msg = msg.to(addr);
        }
        for addr in &cc {
            msg = msg.cc(addr);
        }
        msg = msg.text(&body);

        // Spawn async task for sending
        glib::spawn_future_local(async move {
            let (sender, receiver) = std::sync::mpsc::channel();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    let result = async {
                        let auth_manager = AuthManager::new().await
                            .map_err(|e| format!("Auth init failed: {}", e))?;

                        let smtp_client = northmail_smtp::SmtpClient::new(&smtp_host, 587);

                        match auth_type {
                            northmail_auth::GoaAuthType::OAuth2 => {
                                let (email, token) = auth_manager
                                    .get_xoauth2_token_for_goa(&account_id)
                                    .await
                                    .map_err(|e| format!("Failed to get token: {}", e))?;
                                smtp_client
                                    .send_xoauth2(&email, &token, msg)
                                    .await
                                    .map_err(|e| format!("Send failed: {}", e))
                            }
                            northmail_auth::GoaAuthType::Password => {
                                let password = auth_manager
                                    .get_goa_password(&account_id)
                                    .await
                                    .map_err(|e| format!("Failed to get password: {}", e))?;
                                smtp_client
                                    .send_password(&email, &password, msg)
                                    .await
                                    .map_err(|e| format!("Send failed: {}", e))
                            }
                            northmail_auth::GoaAuthType::Unknown => {
                                Err("Unsupported auth type".to_string())
                            }
                        }
                    }.await;
                    match &result {
                        Ok(()) => eprintln!("[send] success!"),
                        Err(e) => eprintln!("[send] error: {}", e),
                    }
                    let _ = sender.send(result);
                });
            });

            // Poll for result
            let result = loop {
                match receiver.try_recv() {
                    Ok(result) => break result,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        glib::timeout_future(std::time::Duration::from_millis(50)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        break Err("Send thread crashed".to_string());
                    }
                }
            };

            callback(result);
        });
    }

    /// Query EDS (Evolution Data Server) contacts matching a prefix
    pub fn query_contacts(
        &self,
        prefix: String,
        callback: impl FnOnce(Vec<(String, String)>) + 'static,
    ) {
        glib::spawn_future_local(async move {
            let (sender, receiver) = std::sync::mpsc::channel();
            let prefix_clone = prefix.clone();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    let results = Self::eds_query_contacts(&prefix_clone).await;
                    let _ = sender.send(results);
                });
            });

            let results = loop {
                match receiver.try_recv() {
                    Ok(results) => break results,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        glib::timeout_future(std::time::Duration::from_millis(20)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        break Vec::new();
                    }
                }
            };

            callback(results);
        });
    }

    /// Query EDS address books via D-Bus for contacts matching a prefix
    async fn eds_query_contacts(prefix: &str) -> Vec<(String, String)> {
        let mut results = Vec::new();

        let conn = match zbus::Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[eds] session bus error: {}", e);
                return results;
            }
        };

        // Discover versioned EDS service names dynamically
        let (sources_bus, addressbook_bus) = match Self::eds_discover_services(&conn).await {
            Some(pair) => {
                eprintln!("[eds] services: {} / {}", pair.0, pair.1);
                pair
            }
            None => {
                eprintln!("[eds] EDS services not found");
                return results;
            }
        };

        // Get address book source UIDs via ObjectManager
        let source_uids = match Self::eds_get_address_book_uids(&conn, &sources_bus).await {
            Ok(uids) => {
                eprintln!("[eds] {} address books found", uids.len());
                uids
            }
            Err(e) => {
                eprintln!("[eds] get UIDs error: {}", e);
                return results;
            }
        };

        // Query each address book
        let escaped = prefix.replace('\\', "\\\\").replace('\"', "\\\"");
        let sexp = format!(
            "(or (contains \"full_name\" \"{}\") (contains \"email\" \"{}\"))",
            escaped, escaped
        );

        for uid in &source_uids {
            match Self::eds_query_address_book(&conn, &addressbook_bus, uid, &sexp).await {
                Ok(contacts) => {
                    eprintln!("[eds] {} contacts from {}", contacts.len(), uid);
                    results.extend(contacts);
                }
                Err(e) => {
                    eprintln!("[eds] query error for {}: {}", uid, e);
                }
            }
        }

        // Deduplicate by email
        results.sort_by(|a, b| a.1.cmp(&b.1));
        results.dedup_by(|a, b| a.1 == b.1);
        results
    }

    /// Discover versioned EDS D-Bus service names (e.g. Sources5, AddressBook10)
    /// Returns (sources_bus_name, addressbook_bus_name) or None if unavailable
    async fn eds_discover_services(conn: &zbus::Connection) -> Option<(String, String)> {
        let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
        let names = dbus.list_activatable_names().await.ok()?;

        let mut sources_name = None;
        let mut addressbook_name = None;

        for name in &names {
            let s = name.as_str();
            if s.starts_with("org.gnome.evolution.dataserver.Sources") && sources_name.is_none() {
                sources_name = Some(s.to_string());
            }
            if s.starts_with("org.gnome.evolution.dataserver.AddressBook") && !s.contains("Factory") && addressbook_name.is_none() {
                addressbook_name = Some(s.to_string());
            }
        }

        match (sources_name, addressbook_name) {
            (Some(s), Some(a)) => {
                debug!("Discovered EDS services: Sources={}, AddressBook={}", s, a);
                Some((s, a))
            }
            _ => {
                debug!("EDS services not found in activatable names");
                None
            }
        }
    }

    /// Build a zbus Proxy with the given destination, path, and interface
    async fn eds_build_proxy<'a>(
        conn: &zbus::Connection,
        destination: &'a str,
        path: &'a str,
        interface: &'a str,
    ) -> Result<zbus::Proxy<'a>, String> {
        zbus::proxy::Builder::<'a, zbus::Proxy<'a>>::new(conn)
            .destination(destination)
            .map_err(|e| e.to_string())?
            .path(path)
            .map_err(|e| e.to_string())?
            .interface(interface)
            .map_err(|e| e.to_string())?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
            .map_err(|e| e.to_string())
    }

    /// Get UIDs of EDS address books using ObjectManager.GetManagedObjects
    async fn eds_get_address_book_uids(
        conn: &zbus::Connection,
        sources_bus: &str,
    ) -> Result<Vec<String>, String> {
        // Use ObjectManager to enumerate all sources
        let obj_mgr = zbus::fdo::ObjectManagerProxy::builder(conn)
            .destination(sources_bus)
            .map_err(|e| e.to_string())?
            .path("/org/gnome/evolution/dataserver/SourceManager")
            .map_err(|e| e.to_string())?
            .build()
            .await
            .map_err(|e| format!("Failed to build ObjectManager proxy: {}", e))?;

        let objects = obj_mgr
            .get_managed_objects()
            .await
            .map_err(|e| format!("GetManagedObjects failed: {}", e))?;

        debug!("EDS ObjectManager returned {} objects", objects.len());

        let mut uids = Vec::new();

        for (path, interfaces) in &objects {
            let source_iface = interfaces.get("org.gnome.evolution.dataserver.Source");
            let source_props = match source_iface {
                Some(props) => props,
                None => continue,
            };

            let uid = source_props
                .get("UID")
                .and_then(|v| <String as TryFrom<zbus::zvariant::OwnedValue>>::try_from(v.clone()).ok())
                .unwrap_or_default();

            // Check if the Data property contains "[Address Book]" ini section header
            let data_str = source_props
                .get("Data")
                .and_then(|v| <String as TryFrom<zbus::zvariant::OwnedValue>>::try_from(v.clone()).ok())
                .unwrap_or_default();

            let has_addressbook = data_str.contains("[Address Book]");

            if has_addressbook {
                debug!("EDS address book found: UID={} path={}", uid, path);
                if !uid.is_empty() {
                    uids.push(uid);
                }
            }
        }

        debug!("Found {} EDS address book sources", uids.len());
        Ok(uids)
    }

    /// Query a specific EDS address book for contacts
    async fn eds_query_address_book(
        conn: &zbus::Connection,
        addressbook_bus: &str,
        uid: &str,
        sexp: &str,
    ) -> Result<Vec<(String, String)>, String> {
        // Open the address book via the factory
        let factory_proxy = Self::eds_build_proxy(
            conn,
            addressbook_bus,
            "/org/gnome/evolution/dataserver/AddressBookFactory",
            "org.gnome.evolution.dataserver.AddressBookFactory",
        )
        .await
        .map_err(|e| format!("Failed to build factory proxy: {}", e))?;

        // OpenAddressBook(uid) returns (object_path_str, bus_name) — both as strings
        let (book_path, bus_name): (String, String) = factory_proxy
            .call("OpenAddressBook", &(uid,))
            .await
            .map_err(|e| format!("Failed to open address book '{}': {}", uid, e))?;

        // Create proxy to the address book
        let book_path_str = &book_path;
        let book_proxy = Self::eds_build_proxy(
            conn,
            &bus_name,
            book_path_str,
            "org.gnome.evolution.dataserver.AddressBook",
        )
        .await
        .map_err(|e| format!("Failed to build address book proxy: {}", e))?;

        // Open the backend before querying (returns `as` — array of strings)
        let _: Vec<String> = book_proxy
            .call("Open", &())
            .await
            .map_err(|e| format!("Open failed: {}", e))?;

        // GetContactList returns Vec<vcard_string>
        let vcards: Vec<String> = book_proxy
            .call("GetContactList", &(sexp,))
            .await
            .map_err(|e| format!("GetContactList failed: {}", e))?;

        let mut contacts = Vec::new();
        for vcard in &vcards {
            let parsed = Self::parse_vcard_contacts(vcard);
            contacts.extend(parsed);
        }
        Ok(contacts)
    }

    /// Parse a vCard string to extract name and email pairs
    fn parse_vcard_contacts(vcard: &str) -> Vec<(String, String)> {
        let mut name = String::new();
        let mut emails = Vec::new();

        for line in vcard.lines() {
            let line = line.trim();
            if line.starts_with("FN:") || line.starts_with("FN;") {
                // Full name - handle FN;CHARSET=...: or plain FN:
                if let Some(val) = line.splitn(2, ':').nth(1) {
                    name = val.trim().to_string();
                }
            } else if line.starts_with("EMAIL") {
                // EMAIL;TYPE=...: or EMAIL:
                if let Some(val) = line.splitn(2, ':').nth(1) {
                    let email = val.trim().to_string();
                    if !email.is_empty() {
                        emails.push(email);
                    }
                }
            }
        }

        if name.is_empty() {
            name = "Unknown".to_string();
        }

        emails
            .into_iter()
            .map(|email| (name.clone(), email))
            .collect()
    }
}

impl Default for NorthMailApplication {
    fn default() -> Self {
        Self::new()
    }
}
