//! Main application setup

use crate::idle_manager::{IdleAuthType, IdleCredentials, IdleManager, IdleManagerEvent};
use crate::imap_pool::{ImapCommand, ImapCredentials, ImapPool, ImapResponse};
use crate::widgets::MessageInfo;
use crate::window::NorthMailWindow;
use base64::Engine;
use gtk4::{gio, glib, prelude::*, subclass::prelude::*};
use libadwaita as adw;
use libadwaita::prelude::*;
use northmail_auth::AuthManager;
use northmail_imap::{ImapClient, SimpleImapClient};
use mail_parser::MimeHeaders;
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

/// Format a number with thousand separators (e.g., 62208 -> "62,208")
fn format_number(n: impl Into<i64>) -> String {
    let n: i64 = n.into();
    if n < 1000 {
        return n.to_string();
    }
    let s = n.abs().to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    if n < 0 {
        result.push('-');
    }
    result.chars().rev().collect()
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
    /// Flags updated for cached messages: Vec<(uid, is_read, is_starred)>
    FlagsUpdated(Vec<(u32, bool, bool)>),
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

/// Folder info from IMAP LIST + STATUS or Graph API
struct SyncedFolder {
    name: String,
    full_path: String,
    folder_type: String,
    message_count: u32,
    unseen_count: u32,
    /// Graph API folder ID (only set for ms_graph accounts)
    graph_folder_id: Option<String>,
}

/// A single attachment extracted from an email
#[derive(Debug, Clone, Default)]
pub struct ParsedAttachment {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
    pub size: usize,
    pub content_id: Option<String>,
}

/// Parsed email body
#[derive(Debug, Clone, Default)]
pub struct ParsedEmailBody {
    pub text: Option<String>,
    pub html: Option<String>,
    pub attachments: Vec<ParsedAttachment>,
}

mod imp {
    use super::*;
    use libadwaita::subclass::prelude::*;
    use std::cell::{Cell, OnceCell, RefCell};
    use std::collections::{HashMap, HashSet};
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
        pub(super) fetch_generation: Cell<u64>,
        /// IMAP connection pool for reusing connections
        pub(super) imap_pool: OnceCell<Arc<ImapPool>>,
        /// Current cache pagination offset (how many messages already loaded from cache)
        pub(super) cache_offset: Cell<i64>,
        /// Current folder ID in the database (for cache-based pagination)
        pub(super) cache_folder_id: Cell<i64>,
        /// Current folder type (inbox, drafts, sent, etc.) for UI behavior
        pub(super) current_folder_type: RefCell<String>,
        /// Cached contacts from EDS (preloaded at startup) — (name, email, photo_bytes)
        pub(super) contacts_cache: RefCell<Vec<(String, String, Option<Vec<u8>>)>>,
        /// Timer source ID for periodic mail checking
        pub(super) sync_timer_source: RefCell<Option<glib::SourceId>>,
        /// Whether a sync is currently in progress (prevent overlapping syncs)
        pub(super) sync_in_progress: Cell<bool>,
        /// Last known inbox message counts per account (for detecting new mail)
        pub(super) last_inbox_counts: RefCell<HashMap<String, i64>>,
        /// IMAP IDLE manager for real-time push notifications
        pub(super) idle_manager: OnceCell<Arc<IdleManager>>,
        /// Receiver for IDLE manager events
        pub(super) idle_event_receiver: RefCell<Option<std::sync::mpsc::Receiver<IdleManagerEvent>>>,
        /// Receiver for GOA account change events
        pub(super) goa_event_receiver: RefCell<Option<std::sync::mpsc::Receiver<northmail_auth::GoaAccountEvent>>>,
        /// Accounts currently being synced (prevents duplicate concurrent syncs)
        pub(super) syncing_accounts: RefCell<std::collections::HashSet<String>>,
        /// UIDs pending IMAP deletion: (folder_id, uid) pairs
        /// Prevents re-insertion from cache/sync while IMAP move is in flight
        pub(super) pending_deletes: RefCell<HashSet<(i64, u32)>>,
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

                // Quit the application when the main window is closed
                win.connect_close_request(move |_| {
                    std::process::exit(0);
                });

                win.present();
                win
            });

            window.present();

            // Load accounts on startup
            app.load_accounts();

            // Preload contacts from GNOME Contacts (EDS) in background
            app.preload_contacts();

            // Start periodic mail checking timer
            app.start_sync_timer();

            // Initialize IDLE manager for real-time push notifications
            app.init_idle_manager();

            // Monitor GOA account changes at runtime
            app.start_goa_account_monitor();
        }

        fn shutdown(&self) {
            info!("Application shutting down");
            // Gracefully stop all IDLE workers
            if let Some(idle_manager) = self.idle_manager.get() {
                idle_manager.shutdown();
            }
            self.parent_shutdown();
        }

        fn startup(&self) {
            self.parent_startup();
            info!("Application starting up");

            // Set human-readable application name
            glib::set_application_name("NorthMail");

            // Load bundled resources (compiled by build.rs)
            if let Some(gresource_path) = option_env!("GRESOURCE_FILE") {
                if let Ok(resource) = gio::Resource::load(gresource_path) {
                    gio::resources_register(&resource);
                    info!("Loaded bundled resources from {}", gresource_path);
                }
            }

            // Add local icons directory to icon theme search path (for development)
            if let Some(display) = gtk4::gdk::Display::default() {
                let icon_theme = gtk4::IconTheme::for_display(&display);

                // Add bundled resource path for icons
                icon_theme.add_resource_path("/org/northmail/NorthMail/icons");

                // Add project's data/icons directory for development builds
                let exe_path = std::env::current_exe().ok();
                if let Some(exe) = exe_path {
                    // Check if running from target/debug or target/release
                    if let Some(target_dir) = exe.parent() {
                        let project_root = target_dir.parent().and_then(|p| p.parent());
                        if let Some(root) = project_root {
                            let icons_path = root.join("data").join("icons");
                            if icons_path.exists() {
                                icon_theme.add_search_path(&icons_path);
                                info!("Added icon search path: {:?}", icons_path);
                            }

                            // Install desktop file and icon for dev builds so GNOME dock
                            // shows "NorthMail" name and the correct icon instead of the raw app ID
                            Self::install_dev_desktop_entry(root);
                        }
                    }
                }

                // Set the default window icon based on user preference
                let icon_settings = gio::Settings::new(APP_ID);
                let icon_name = if icon_settings.string("app-icon") == "system" {
                    "email"
                } else {
                    "org.northmail.NorthMail"
                };
                gtk4::Window::set_default_icon_name(icon_name);
            }

            let app = self.obj();
            app.setup_actions();
        }
    }

    impl NorthMailApplication {
        /// Install .desktop file and icon to ~/.local for dev builds.
        /// This makes GNOME show "NorthMail" in the dock tooltip and the correct app icon.
        fn install_dev_desktop_entry(project_root: &std::path::Path) {
            let home = match std::env::var("HOME") {
                Ok(h) => std::path::PathBuf::from(h),
                Err(_) => return,
            };

            // Install desktop file
            let desktop_src = project_root.join("data").join("org.northmail.NorthMail.desktop");
            let desktop_dst_dir = home.join(".local/share/applications");
            let desktop_dst = desktop_dst_dir.join("org.northmail.NorthMail.desktop");
            if desktop_src.exists() {
                let _ = std::fs::create_dir_all(&desktop_dst_dir);
                if let Ok(contents) = std::fs::read_to_string(&desktop_src) {
                    // Determine which icon name to use from GSettings
                    let icon_setting = gio::Settings::new(APP_ID);
                    let desktop_icon = if icon_setting.string("app-icon") == "system" {
                        "email"
                    } else {
                        "org.northmail.NorthMail"
                    };
                    // Rewrite Exec= and Icon= for the dev binary and icon preference
                    let exe = std::env::current_exe().unwrap_or_default();
                    let patched = contents
                        .lines()
                        .map(|line| {
                            if line.starts_with("Exec=") {
                                format!("Exec={} %U", exe.display())
                            } else if line.starts_with("Icon=") {
                                format!("Icon={}", desktop_icon)
                            } else {
                                line.to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    if std::fs::write(&desktop_dst, patched).is_ok() {
                        info!("Installed dev desktop file to {:?}", desktop_dst);
                    }
                }
            }

            // Install icon
            let icon_src = project_root.join("data/icons/hicolor/128x128/apps/org.northmail.NorthMail.png");
            let icon_dst_dir = home.join(".local/share/icons/hicolor/128x128/apps");
            let icon_dst = icon_dst_dir.join("org.northmail.NorthMail.png");
            if icon_src.exists() && !icon_dst.exists() {
                let _ = std::fs::create_dir_all(&icon_dst_dir);
                if std::fs::copy(&icon_src, &icon_dst).is_ok() {
                    info!("Installed dev icon to {:?}", icon_dst);
                }
            }
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

    /// Get application settings
    fn settings(&self) -> gio::Settings {
        gio::Settings::new(APP_ID)
    }

    /// Start the periodic mail sync timer based on GSettings interval
    fn start_sync_timer(&self) {
        // Stop any existing timer first
        self.stop_sync_timer();

        let settings = self.settings();
        let interval_minutes = settings.int("sync-interval") as u32;
        let interval_seconds = interval_minutes * 60;

        info!("Starting mail sync timer with {} minute interval", interval_minutes);

        // Do an immediate check on startup (after a short delay for UI to settle)
        let app_immediate = self.clone();
        glib::timeout_add_seconds_local_once(3, move || {
            app_immediate.check_for_new_mail();
        });

        let app = self.clone();
        let source_id = glib::timeout_add_seconds_local(interval_seconds, move || {
            app.check_for_new_mail();
            glib::ControlFlow::Continue
        });

        self.imp().sync_timer_source.replace(Some(source_id));

        // Connect to settings changes to restart timer if interval changes
        let app_for_settings = self.clone();
        settings.connect_changed(Some("sync-interval"), move |settings, _| {
            let new_interval = settings.int("sync-interval");
            info!("Sync interval changed to {} minutes, restarting timer", new_interval);
            app_for_settings.start_sync_timer();
        });
    }

    /// Stop the periodic mail sync timer
    fn stop_sync_timer(&self) {
        if let Some(source_id) = self.imp().sync_timer_source.take() {
            source_id.remove();
            info!("Stopped mail sync timer");
        }
    }

    /// Check for new mail by comparing IMAP counts with previously seen counts
    fn check_for_new_mail(&self) {
        // Prevent overlapping syncs
        if self.imp().sync_in_progress.get() {
            debug!("Sync already in progress, skipping scheduled check");
            return;
        }

        self.imp().sync_in_progress.set(true);
        info!("Starting scheduled mail check");

        let app = self.clone();
        glib::spawn_future_local(async move {
            let accounts = app.imp().accounts.borrow().clone();
            let mut new_messages: Vec<(String, i64)> = Vec::new();
            let mut accounts_to_refresh: Vec<northmail_auth::GoaAccount> = Vec::new();

            // Check each account for new messages via IMAP STATUS
            for account in &accounts {
                if !Self::is_supported_account(account) {
                    continue;
                }

                // Get IMAP inbox count via STATUS
                let imap_count = app.get_imap_inbox_count(account).await;

                // Compare with last known IMAP count (not cache count)
                let last_count = app.imp().last_inbox_counts.borrow()
                    .get(&account.id)
                    .copied()
                    .unwrap_or(imap_count); // If not initialized, assume no new

                if imap_count > last_count {
                    let diff = imap_count - last_count;
                    info!("Account {} has {} new messages (IMAP: {}, last: {})",
                          account.email, diff, imap_count, last_count);
                    new_messages.push((account.id.clone(), diff));
                    accounts_to_refresh.push(account.clone());
                }

                // Update last known count
                app.imp().last_inbox_counts.borrow_mut()
                    .insert(account.id.clone(), imap_count);
            }

            // Fetch new messages for accounts that have them
            for account in &accounts_to_refresh {
                info!("Fetching new messages for {}", account.email);
                app.stream_inbox_to_cache(account).await;
            }

            // If we found new messages, refresh the UI
            if !accounts_to_refresh.is_empty() {
                // Show notification
                app.notify_new_mail(&new_messages).await;

                // Refresh sidebar folder counts
                app.refresh_sidebar_folders();

                // Refresh current view if showing unified inbox
                if app.imp().state.borrow().unified_inbox {
                    app.fetch_unified_inbox();
                }
            }

            // Update window title with unread count
            app.update_unread_badge();

            app.imp().sync_in_progress.set(false);
        });
    }

    /// Get inbox message count from IMAP via STATUS query
    async fn get_imap_inbox_count(&self, account: &northmail_auth::GoaAccount) -> i64 {
        let auth_manager = match AuthManager::new().await {
            Ok(am) => am,
            Err(_) => return 0,
        };

        let result: i64 = match account.provider_type.as_str() {
            "google" => {
                match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                    Ok((email, access_token)) => {
                        Self::get_inbox_count_google(&email, &access_token).await.unwrap_or(0) as i64
                    }
                    Err(_) => 0,
                }
            }
            "windows_live" | "microsoft" => {
                match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                    Ok((email, access_token)) => {
                        Self::get_inbox_count_microsoft(&email, &access_token).await.unwrap_or(0) as i64
                    }
                    Err(_) => 0,
                }
            }
            "ms_graph" => {
                // Graph API: get inbox count from DB cache (populated by sync)
                self.get_inbox_count_for_account(&account.id).await
            }
            _ => {
                // Password auth (iCloud, etc.)
                let host = account.imap_host.clone().unwrap_or_else(|| "imap.mail.me.com".to_string());
                let username = account.imap_username.clone().unwrap_or_else(|| account.email.clone());
                match auth_manager.get_goa_password(&account.id).await {
                    Ok(password) => {
                        Self::get_inbox_count_password(&host, &username, &password).await.unwrap_or(0) as i64
                    }
                    Err(_) => 0,
                }
            }
        };

        result
    }

    /// Get inbox count from Gmail via IMAP STATUS
    async fn get_inbox_count_google(email: &str, access_token: &str) -> Option<u32> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let email = email.to_string();
        let token = access_token.to_string();

        std::thread::spawn(move || {
            let result = async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();
                client.connect_gmail(&email, &token).await?;
                let (count, _) = client.folder_status("INBOX").await?;
                client.logout().await.ok();
                Ok::<_, northmail_imap::ImapError>(count)
            });
            let _ = sender.send(result);
        });

        loop {
            match receiver.try_recv() {
                Ok(Ok(count)) => return Some(count),
                Ok(Err(_)) => return None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::timeout_future(std::time::Duration::from_millis(50)).await;
                }
                Err(_) => return None,
            }
        }
    }

    /// Get inbox count from Outlook via IMAP STATUS
    async fn get_inbox_count_microsoft(email: &str, access_token: &str) -> Option<u32> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let email = email.to_string();
        let token = access_token.to_string();

        std::thread::spawn(move || {
            let result = async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();
                client.connect_outlook(&email, &token).await?;
                let (count, _) = client.folder_status("INBOX").await?;
                client.logout().await.ok();
                Ok::<_, northmail_imap::ImapError>(count)
            });
            let _ = sender.send(result);
        });

        loop {
            match receiver.try_recv() {
                Ok(Ok(count)) => return Some(count),
                Ok(Err(_)) => return None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::timeout_future(std::time::Duration::from_millis(50)).await;
                }
                Err(_) => return None,
            }
        }
    }

    /// Get inbox count via password auth IMAP STATUS
    async fn get_inbox_count_password(host: &str, username: &str, password: &str) -> Option<u32> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let host = host.to_string();
        let username = username.to_string();
        let password = password.to_string();

        std::thread::spawn(move || {
            let result = async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();
                client.connect_login(&host, 993, &username, &password).await?;
                let (count, _) = client.folder_status("INBOX").await?;
                client.logout().await.ok();
                Ok::<_, northmail_imap::ImapError>(count)
            });
            let _ = sender.send(result);
        });

        loop {
            match receiver.try_recv() {
                Ok(Ok(count)) => return Some(count),
                Ok(Err(_)) => return None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::timeout_future(std::time::Duration::from_millis(50)).await;
                }
                Err(_) => return None,
            }
        }
    }

    /// Get inbox message counts for all accounts
    async fn get_all_inbox_counts(&self) -> std::collections::HashMap<String, i64> {
        let mut counts = std::collections::HashMap::new();
        let Some(db) = self.database() else {
            return counts;
        };

        let accounts = self.imp().accounts.borrow().clone();
        for account in accounts {
            let db = db.clone();
            let account_id = account.id.clone();

            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async {
                    db.get_inbox_message_count_for_account(&account_id).await
                });
                let _ = sender.send(result);
            });

            // Wait for result
            loop {
                match receiver.try_recv() {
                    Ok(Ok(count)) => {
                        counts.insert(account.id.clone(), count);
                        break;
                    }
                    Ok(Err(_)) => break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        glib::timeout_future(std::time::Duration::from_millis(50)).await;
                    }
                    Err(_) => break,
                }
            }
        }

        counts
    }

    /// Show desktop notification for new mail
    async fn notify_new_mail(&self, new_messages: &[(String, i64)]) {
        info!("notify_new_mail called with {} accounts", new_messages.len());
        let settings = self.settings();

        // Check if notifications are enabled
        let notifications_enabled = settings.boolean("notifications-enabled");
        info!("notifications-enabled setting: {}", notifications_enabled);
        if !notifications_enabled {
            info!("Notifications disabled in settings, skipping");
            return;
        }

        // Check Do Not Disturb
        if settings.boolean("do-not-disturb") {
            debug!("Do Not Disturb enabled, skipping notification");
            return;
        }

        let total_new: i64 = new_messages.iter().map(|(_, count)| count).sum();
        let show_preview = settings.boolean("notification-preview-enabled");

        // Build notification
        let (summary, body) = if total_new == 1 && show_preview {
            // Single message - try to get sender and subject
            if let Some((account_id, _)) = new_messages.first() {
                if let Some(msg_info) = self.get_latest_message_info(account_id).await {
                    (msg_info.0, msg_info.1) // (from, subject)
                } else {
                    ("New Email".to_string(), "You have a new message".to_string())
                }
            } else {
                ("New Email".to_string(), "You have a new message".to_string())
            }
        } else if total_new > 1 {
            // Multiple messages
            let summary = format!("{} New Emails", total_new);
            let body = if show_preview {
                new_messages
                    .iter()
                    .map(|(account_id, count)| {
                        let accounts = self.imp().accounts.borrow();
                        let email = accounts
                            .iter()
                            .find(|a| a.id == *account_id)
                            .map(|a| a.email.as_str())
                            .unwrap_or("Unknown");
                        format!("{}: {} new", email, count)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                "You have new messages".to_string()
            };
            (summary, body)
        } else {
            ("New Email".to_string(), "You have a new message".to_string())
        };

        // Send notification using libnotify (works on both X11 and Wayland)
        // Spawn in a thread to avoid blocking the GTK main loop
        // IMPORTANT: Must wait for notification to complete for GNOME 46+ Wayland
        // otherwise D-Bus connection closes before notification is displayed
        let summary_clone = summary.clone();
        let body_clone = body.clone();

        // Find the app icon path for the notification
        let icon_path = Self::find_app_icon_path();

        std::thread::spawn(move || {
            let notification = notify_rust::Notification::new()
                .summary(&summary_clone)
                .body(&body_clone)
                .icon(&icon_path)
                .appname("NorthMail")
                .hint(notify_rust::Hint::Category("email.arrived".to_string()))
                .urgency(notify_rust::Urgency::Normal)
                .timeout(notify_rust::Timeout::Milliseconds(5000))
                .finalize();

            match notification.show() {
                Ok(handle) => {
                    tracing::info!("Notification sent, waiting for close");
                    // Wait for notification to close - required for GNOME Wayland
                    handle.wait_for_action(|_| {});
                }
                Err(e) => tracing::error!("Failed to show notification: {}", e),
            }
        });
        info!("Showed notification: {}", summary);
    }

    /// Find the app icon path for notifications
    fn find_app_icon_path() -> String {
        // Try development path first (running from target/debug or target/release)
        if let Ok(exe) = std::env::current_exe() {
            if let Some(target_dir) = exe.parent() {
                if let Some(project_root) = target_dir.parent().and_then(|p| p.parent()) {
                    let dev_icon = project_root
                        .join("data")
                        .join("icons")
                        .join("hicolor")
                        .join("128x128")
                        .join("apps")
                        .join("org.northmail.NorthMail.png");
                    if dev_icon.exists() {
                        return dev_icon.to_string_lossy().to_string();
                    }
                }
            }
        }

        // Try installed paths
        let installed_paths = [
            "/usr/share/icons/hicolor/128x128/apps/org.northmail.NorthMail.png",
            "/usr/local/share/icons/hicolor/128x128/apps/org.northmail.NorthMail.png",
        ];
        for path in &installed_paths {
            if std::path::Path::new(path).exists() {
                return path.to_string();
            }
        }

        // Try home directory
        if let Ok(home) = std::env::var("HOME") {
            let home_icon = format!(
                "{}/.local/share/icons/hicolor/128x128/apps/org.northmail.NorthMail.png",
                home
            );
            if std::path::Path::new(&home_icon).exists() {
                return home_icon;
            }
        }

        // Fallback to icon name (may not show the colored icon)
        "org.northmail.NorthMail".to_string()
    }

    /// Get sender and subject of the latest inbox message for an account
    async fn get_latest_message_info(&self, account_id: &str) -> Option<(String, String)> {
        let db = self.database()?.clone();
        let account_id = account_id.to_string();

        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
                db.get_latest_inbox_message(&account_id).await
            });
            let _ = sender.send(result);
        });

        // Wait for result
        loop {
            match receiver.try_recv() {
                Ok(Ok(Some(msg))) => {
                    let from = msg.from_name.or(msg.from_address).unwrap_or_else(|| "Unknown".to_string());
                    let subject = msg.subject.unwrap_or_else(|| "(No subject)".to_string());
                    return Some((from, subject));
                }
                Ok(Ok(None)) => return None,
                Ok(Err(_)) => return None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::timeout_future(std::time::Duration::from_millis(50)).await;
                }
                Err(_) => return None,
            }
        }
    }

    /// Initialize the IDLE manager and start event loop
    fn init_idle_manager(&self) {
        let (idle_manager, event_rx) = IdleManager::new();

        // Store the manager and receiver
        if self.imp().idle_manager.set(idle_manager.clone()).is_err() {
            warn!("IDLE manager already initialized");
            return;
        }
        self.imp().idle_event_receiver.replace(Some(event_rx));

        info!("IDLE manager initialized");

        // Start the event loop
        self.start_idle_event_loop();

        // Start IDLE for all accounts (will happen after accounts are loaded)
        // The actual IDLE connections will be started in load_accounts or after sync
    }

    /// Start the IDLE event processing loop
    fn start_idle_event_loop(&self) {
        let app = self.clone();

        // Poll for IDLE events every 500ms
        glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
            let receiver = app.imp().idle_event_receiver.borrow();
            if let Some(rx) = receiver.as_ref() {
                while let Ok(event) = rx.try_recv() {
                    match event {
                        IdleManagerEvent::NewMail { account_id } => {
                            info!("IDLE: New mail for account {}", account_id);
                            // Trigger a quick sync for this account
                            app.quick_sync_account(&account_id);
                        }
                        IdleManagerEvent::ConnectionLost { account_id } => {
                            warn!("IDLE: Connection lost for account {}", account_id);
                            // Will auto-reconnect via the worker
                        }
                        IdleManagerEvent::NotSupported { account_id } => {
                            warn!("IDLE: Not supported for account {}, falling back to periodic sync", account_id);
                            // Stop the IDLE worker - periodic sync timer handles polling
                            if let Some(idle_mgr) = app.imp().idle_manager.get() {
                                idle_mgr.stop_idle(&account_id);
                            }
                        }
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    /// Start monitoring GOA account changes (additions/removals) at runtime
    fn start_goa_account_monitor(&self) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.imp().goa_event_receiver.replace(Some(rx));

        // Spawn background thread with its own tokio runtime
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            if let Err(e) = rt.block_on(northmail_auth::GoaManager::watch_account_changes(tx)) {
                warn!("GOA account watcher stopped with error: {}", e);
            }
        });

        info!("GOA account monitor started");

        // Poll for GOA events every 500ms (same pattern as IDLE event loop)
        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
            let receiver = app.imp().goa_event_receiver.borrow();
            if let Some(rx) = receiver.as_ref() {
                while let Ok(event) = rx.try_recv() {
                    match event {
                        northmail_auth::GoaAccountEvent::AccountAdded => {
                            info!("GOA: Account added, reloading accounts");
                            app.reload_goa_accounts();
                        }
                        northmail_auth::GoaAccountEvent::AccountRemoved => {
                            info!("GOA: Account removed, reloading accounts");
                            app.reload_goa_accounts();
                        }
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    /// Reload GOA accounts after a runtime change (account added/removed)
    fn reload_goa_accounts(&self) {
        let app = self.clone();
        glib::spawn_future_local(async move {
            let auth_manager = match AuthManager::new().await {
                Ok(am) => am,
                Err(e) => {
                    warn!("Failed to create auth manager during reload: {}", e);
                    return;
                }
            };

            let mut new_accounts = match auth_manager.list_goa_accounts().await {
                Ok(accts) => accts,
                Err(e) => {
                    warn!("Failed to list GOA accounts during reload: {}", e);
                    return;
                }
            };

            new_accounts.sort_by(|a, b| a.email.to_lowercase().cmp(&b.email.to_lowercase()));

            let old_accounts = app.imp().accounts.borrow().clone();

            // Find added accounts (in new but not in old)
            let added: Vec<_> = new_accounts
                .iter()
                .filter(|na| !old_accounts.iter().any(|oa| oa.id == na.id))
                .collect();

            // Find removed accounts (in old but not in new)
            let removed: Vec<_> = old_accounts
                .iter()
                .filter(|oa| !new_accounts.iter().any(|na| na.id == oa.id))
                .collect();

            if added.is_empty() && removed.is_empty() {
                debug!("GOA reload: no account changes detected");
                return;
            }

            // Show toasts for changes
            for acct in &added {
                info!("GOA account added: {}", acct.email);
                app.show_toast(&format!("Account added: {}", acct.email));
            }
            for acct in &removed {
                info!("GOA account removed: {}", acct.email);
                // Stop IDLE for removed account
                if let Some(idle_mgr) = app.imp().idle_manager.get() {
                    idle_mgr.stop_idle(&acct.id);
                }
                app.show_toast(&format!("Account removed: {}", acct.email));
            }

            // Save new accounts to DB
            app.save_accounts_to_db(&new_accounts);

            // Update stored accounts and sidebar
            app.imp().accounts.replace(new_accounts.clone());
            app.update_sidebar_with_accounts(&new_accounts);

            // Start IDLE for newly added accounts
            if !added.is_empty() {
                app.start_idle_for_all_accounts();
                // Sync the new accounts
                app.sync_all_accounts();
            }
        });
    }

    /// Start IDLE connections for all supported accounts
    fn start_idle_for_all_accounts(&self) {
        let Some(idle_manager) = self.imp().idle_manager.get() else {
            return;
        };

        let accounts = self.imp().accounts.borrow().clone();
        let idle_manager = idle_manager.clone();
        let app = self.clone();

        glib::spawn_future_local(async move {
            // Initialize last_inbox_counts from IMAP before starting IDLE
            // This prevents false "new mail" notifications on startup
            for account in &accounts {
                if !Self::is_supported_account(account) {
                    continue;
                }
                // Use IMAP count (not cache count) as baseline
                let count = app.get_imap_inbox_count(account).await;
                app.imp().last_inbox_counts.borrow_mut()
                    .insert(account.id.clone(), count);
                info!("Initialized IMAP inbox count for {}: {}", account.email, count);
            }

            // Now start IDLE for each account
            for account in accounts {
                if !Self::is_supported_account(&account) {
                    continue;
                }

                app.start_idle_for_account_async(&account, &idle_manager).await;
            }
        });
    }

    /// Start IDLE for a single account (async to get credentials)
    async fn start_idle_for_account_async(
        &self,
        account: &northmail_auth::GoaAccount,
        idle_manager: &std::sync::Arc<IdleManager>,
    ) {
        let auth_manager = match AuthManager::new().await {
            Ok(am) => am,
            Err(e) => {
                warn!("IDLE: Failed to create auth manager for {}: {}", account.email, e);
                return;
            }
        };

        let credentials = match account.provider_type.as_str() {
            // ms_graph accounts use Graph API, not IMAP — skip IDLE entirely
            "ms_graph" => {
                info!("IDLE: Skipping ms_graph account {} (no IMAP, using sync timer)", account.email);
                return;
            }
            "google" => {
                match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                    Ok((email, access_token)) => {
                        IdleCredentials {
                            account_id: account.id.clone(),
                            email,
                            auth_type: IdleAuthType::OAuth2 {
                                host: "imap.gmail.com".to_string(),
                                access_token,
                            },
                        }
                    }
                    Err(e) => {
                        warn!("IDLE: Failed to get OAuth2 token for Gmail {}: {}", account.email, e);
                        return;
                    }
                }
            }
            "windows_live" | "microsoft" => {
                match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                    Ok((email, access_token)) => {
                        IdleCredentials {
                            account_id: account.id.clone(),
                            email,
                            auth_type: IdleAuthType::OAuth2 {
                                host: "outlook.office365.com".to_string(),
                                access_token,
                            },
                        }
                    }
                    Err(e) => {
                        warn!("IDLE: Failed to get OAuth2 token for Outlook {}: {}", account.email, e);
                        return;
                    }
                }
            }
            _ => {
                // Password-based auth (iCloud, etc.)
                let host = account.imap_host.clone().unwrap_or_else(|| {
                    "imap.mail.me.com".to_string()
                });
                let username = account.imap_username.clone().unwrap_or_else(|| {
                    account.email.clone()
                });
                match auth_manager.get_goa_password(&account.id).await {
                    Ok(password) => {
                        IdleCredentials {
                            account_id: account.id.clone(),
                            email: account.email.clone(),
                            auth_type: IdleAuthType::Password {
                                host,
                                port: 993, // Standard IMAPS port
                                username,
                                password,
                            },
                        }
                    }
                    Err(e) => {
                        warn!("IDLE: Failed to get password for {}: {}", account.email, e);
                        return;
                    }
                }
            }
        };

        idle_manager.start_idle(credentials);
        info!("IDLE: Started for {}", account.email);
    }

    /// Quick sync for a single account (triggered by IDLE event)
    fn quick_sync_account(&self, account_id: &str) {
        let accounts = self.imp().accounts.borrow().clone();
        let account = match accounts.iter().find(|a| a.id == account_id) {
            Some(a) => a.clone(),
            None => {
                warn!("Account {} not found for quick sync", account_id);
                return;
            }
        };

        // Get old count for comparison
        let old_count = self.imp().last_inbox_counts.borrow()
            .get(account_id)
            .copied()
            .unwrap_or(0);

        let app = self.clone();
        let account_id = account_id.to_string();

        glib::spawn_future_local(async move {
            // Actually fetch new messages from IMAP (not just STATUS)
            info!("IDLE quick sync: fetching new messages for {}", account.email);
            app.stream_inbox_to_cache(&account).await;

            // Refresh sidebar folder counts
            app.refresh_sidebar_folders();

            // Check for new messages
            let new_count = app.get_inbox_count_for_account(&account_id).await;

            info!("IDLE sync: old_count={}, new_count={} for {}", old_count, new_count, account_id);
            if new_count > old_count {
                let diff = new_count - old_count;
                info!("IDLE sync found {} new messages, triggering notification", diff);

                // Update stored count
                app.imp().last_inbox_counts.borrow_mut()
                    .insert(account_id.clone(), new_count);

                // Show notification
                let new_messages = vec![(account_id.clone(), diff)];
                app.notify_new_mail(&new_messages).await;
            } else {
                info!("IDLE sync: no new messages detected (count unchanged)");
            }

            // Refresh unified inbox if that's what we're viewing
            if app.imp().state.borrow().unified_inbox {
                app.fetch_unified_inbox();
            }

            // Update window title with unread count
            app.update_unread_badge();
        });
    }

    /// Get inbox message count for a single account
    async fn get_inbox_count_for_account(&self, account_id: &str) -> i64 {
        let Some(db) = self.database() else {
            return 0;
        };

        let db = db.clone();
        let account_id = account_id.to_string();

        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
                db.get_inbox_message_count_for_account(&account_id).await
            });
            let _ = sender.send(result);
        });

        loop {
            match receiver.try_recv() {
                Ok(Ok(count)) => return count,
                Ok(Err(_)) => return 0,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::timeout_future(std::time::Duration::from_millis(50)).await;
                }
                Err(_) => return 0,
            }
        }
    }

    /// Update the window title to show total unread count
    fn update_unread_badge(&self) {
        let Some(db) = self.database() else {
            return;
        };
        let db = db.clone();
        let app = self.clone();

        glib::spawn_future_local(async move {
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async {
                    db.get_total_unread_count().await
                });
                let _ = sender.send(result);
            });

            loop {
                match receiver.try_recv() {
                    Ok(Ok(count)) => {
                        if let Some(window) = app.active_window() {
                            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                win.set_unread_count(count);
                            }
                        }
                        break;
                    }
                    Ok(Err(_)) => break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        glib::timeout_future(std::time::Duration::from_millis(50)).await;
                    }
                    Err(_) => break,
                }
            }
        });
    }

    /// Get the current cache folder ID
    pub fn cache_folder_id(&self) -> i64 {
        self.imp().cache_folder_id.get()
    }

    /// Get the current folder type (inbox, drafts, sent, etc.)
    pub fn current_folder_type(&self) -> String {
        self.imp().current_folder_type.borrow().clone()
    }

    /// Get the email address of the currently selected account
    pub fn current_account_email(&self) -> Option<String> {
        let state = self.imp().folder_load_state.borrow();
        if let Some(s) = state.as_ref() {
            let accounts = self.imp().accounts.borrow();
            accounts.iter().find(|a| a.id == s.account_id).map(|a| a.email.clone())
        } else {
            None
        }
    }

    /// Refresh the current folder if we're viewing drafts
    /// Called after saving a draft to update the message list
    pub fn refresh_if_viewing_drafts(&self) {
        if self.current_folder_type() != "drafts" {
            return;
        }
        // Get current folder info from state
        let state = self.imp().state.borrow();
        if let Some((account_id, folder_path)) = &state.last_folder {
            let account_id = account_id.clone();
            let folder_path = folder_path.clone();
            drop(state); // Release borrow before calling fetch_folder
            self.fetch_folder(&account_id, &folder_path);
        }
    }

    /// Set the cache offset
    pub fn set_cache_offset(&self, offset: i64) {
        self.imp().cache_offset.set(offset);
    }

    /// Save GOA accounts to database and remove stale accounts no longer in GOA.
    ///
    /// This reconciles the DB with the current GOA account list:
    /// - Upserts all current GOA accounts
    /// - Deletes any DB accounts whose IDs are not in the GOA list
    ///   (cascading deletes clean up folders, messages, and attachments)
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
                // Collect current GOA account IDs
                let goa_ids: std::collections::HashSet<String> =
                    accounts.iter().map(|a| a.id.clone()).collect();

                // Remove stale accounts from DB that are no longer in GOA
                match db.get_accounts().await {
                    Ok(db_accounts) => {
                        for db_account in &db_accounts {
                            if !goa_ids.contains(&db_account.id) {
                                info!(
                                    "Removing stale account {} ({}) from database — no longer in GOA",
                                    db_account.email, db_account.id
                                );
                                if let Err(e) = db.delete_account(&db_account.id).await {
                                    warn!("Failed to delete stale account {}: {}", db_account.id, e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to list DB accounts for reconciliation: {}", e);
                    }
                }

                // Upsert current GOA accounts
                for account in &accounts {
                    let config = if account.provider_type == "google" {
                        northmail_core::AccountConfig::gmail()
                    } else if account.provider_type == "windows_live" || account.provider_type == "microsoft" || account.provider_type == "ms_graph" {
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
                info!("Reconciled {} GOA accounts with database", accounts.len());
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

                                    // Sort accounts alphabetically by email for consistent ordering
                                    let mut accounts = accounts;
                                    accounts.sort_by(|a, b| a.email.to_lowercase().cmp(&b.email.to_lowercase()));

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

                                    // Check if DB is fresh (no cached messages)
                                    let is_fresh_db = if let Some(db) = app.database() {
                                        let db = db.clone();
                                        let (sender, receiver) = std::sync::mpsc::channel();
                                        std::thread::spawn(move || {
                                            let rt = tokio::runtime::Runtime::new().unwrap();
                                            let count = rt.block_on(db.get_inbox_message_count()).unwrap_or(0);
                                            let _ = sender.send(count);
                                        });
                                        let count = loop {
                                            match receiver.try_recv() {
                                                Ok(c) => break c,
                                                Err(std::sync::mpsc::TryRecvError::Empty) => {
                                                    glib::timeout_future(std::time::Duration::from_millis(5)).await;
                                                }
                                                Err(std::sync::mpsc::TryRecvError::Disconnected) => break 0,
                                            }
                                        };
                                        count == 0
                                    } else {
                                        true
                                    };

                                    if is_fresh_db {
                                        // Fresh DB: show loading state, sync will populate inbox
                                        info!("Fresh database detected, showing loading state");
                                        {
                                            let mut state = app.imp().state.borrow_mut();
                                            state.unified_inbox = true;
                                            state.last_folder = None;
                                        }
                                        app.imp().state.borrow().save();
                                        if let Some(window) = app.active_window() {
                                            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                                win.show_loading_with_status("Loading messages...", None);
                                                if let Some(sidebar) = win.folder_sidebar() {
                                                    sidebar.select_unified_inbox();
                                                }
                                            }
                                        }
                                    } else {
                                        // Existing DB: restore last selected folder
                                        app.restore_last_folder();
                                    }

                                    // Start background sync for all supported accounts
                                    // On fresh DB, this will also stream INBOX messages
                                    app.sync_all_accounts();

                                    // Start IDLE connections for real-time notifications
                                    app.start_idle_for_all_accounts();
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

    /// Check if an account is Microsoft (Outlook/Hotmail) — legacy IMAP-capable providers only
    fn is_microsoft_account(account: &northmail_auth::GoaAccount) -> bool {
        account.provider_type == "windows_live"
            || account.provider_type == "microsoft"
    }

    /// Check if an account uses Microsoft Graph API (ms_graph provider from GNOME Online Accounts).
    /// These accounts have OAuth2 tokens scoped for Graph API, not IMAP XOAUTH2.
    fn is_ms_graph_account(account: &northmail_auth::GoaAccount) -> bool {
        account.provider_type == "ms_graph"
    }

    /// Check if a Microsoft account can send via Graph API (only ms_graph provider has mail.send scope)
    fn can_send_microsoft(account: &northmail_auth::GoaAccount) -> bool {
        account.provider_type == "ms_graph"
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
        Self::is_google_account(account) || Self::is_microsoft_account(account) || Self::is_ms_graph_account(account) || Self::is_password_account(account)
    }

    /// Convert folder path to a friendly display name
    fn friendly_folder_name(folder_path: &str) -> String {
        // Handle Gmail special folders like "[Gmail]/Sent Mail"
        let name = if let Some(stripped) = folder_path.strip_prefix("[Gmail]/") {
            stripped
        } else if let Some(stripped) = folder_path.strip_prefix("[Google Mail]/") {
            stripped
        } else {
            // Use the last path component for nested folders
            folder_path.rsplit('/').next().unwrap_or(folder_path)
        };

        // Title case common folder names
        match name.to_uppercase().as_str() {
            "INBOX" => "Inbox".to_string(),
            "SENT" | "SENT MAIL" | "SENT MESSAGES" | "SENT ITEMS" => "Sent".to_string(),
            "DRAFTS" | "DRAFT" => "Drafts".to_string(),
            "TRASH" | "DELETED" | "DELETED ITEMS" | "DELETED MESSAGES" => "Trash".to_string(),
            "SPAM" | "JUNK" | "JUNK E-MAIL" | "JUNK MAIL" => "Junk".to_string(),
            "ARCHIVE" | "ALL MAIL" => "Archive".to_string(),
            "STARRED" | "FLAGGED" => "Starred".to_string(),
            "IMPORTANT" => "Important".to_string(),
            _ => name.to_string(),
        }
    }

    /// Guess folder type from path (inbox, sent, drafts, trash, spam, archive, other)
    fn guess_folder_type(folder_path: &str) -> String {
        let lower = folder_path.to_lowercase();
        if lower == "inbox" {
            "inbox".to_string()
        } else if lower.contains("sent") {
            "sent".to_string()
        } else if lower.contains("draft") {
            "drafts".to_string()
        } else if lower.contains("trash") || lower.contains("deleted") {
            "trash".to_string()
        } else if lower.contains("spam") || lower.contains("junk") {
            "spam".to_string()
        } else if lower.contains("archive") || lower.contains("all mail") {
            "archive".to_string()
        } else {
            "other".to_string()
        }
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
    pub fn sync_all_accounts(&self) {
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

        let total_accounts = supported_accounts.len();
        info!("Starting parallel sync of {} accounts", total_accounts);

        // Show initial sync status
        self.update_sync_status_multi(&supported_accounts.iter().map(|a| a.email.clone()).collect::<Vec<_>>(), 0);

        // Track sync completion with a shared counter
        let completed = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let accounts_syncing = std::rc::Rc::new(std::cell::RefCell::new(
            supported_accounts.iter().map(|a| (a.email.clone(), "starting")).collect::<std::collections::HashMap<_, _>>()
        ));

        // Spawn parallel sync tasks for each account
        for account in supported_accounts {
            let app = app.clone();
            let completed = completed.clone();
            let accounts_syncing = accounts_syncing.clone();
            let email = account.email.clone();

            glib::spawn_future_local(async move {
                // Update status to syncing
                accounts_syncing.borrow_mut().insert(email.clone(), "syncing");
                app.update_sync_status_from_map(&accounts_syncing.borrow());

                // Sync this account's folder metadata (STATUS queries)
                app.sync_account_inbox(&account.id).await;

                // Refresh sidebar after this account so counts appear progressively
                app.refresh_sidebar_folders();

                // Check if this account has no cached inbox messages
                let needs_streaming = app.account_inbox_is_empty(&account.id).await;
                if needs_streaming {
                    accounts_syncing.borrow_mut().insert(email.clone(), "loading");
                    app.update_sync_status_from_map(&accounts_syncing.borrow());

                    app.stream_inbox_to_cache(&account).await;

                    // Refresh unified inbox if that's the current view
                    if app.imp().state.borrow().unified_inbox {
                        app.fetch_unified_inbox();
                    }
                }

                // Mark this account as done
                accounts_syncing.borrow_mut().insert(email.clone(), "done");
                let done = completed.get() + 1;
                completed.set(done);

                info!("Account {} sync complete ({}/{})", email, done, total_accounts);

                // Update status
                if done == total_accounts {
                    // All accounts done
                    app.update_simple_sync_status("Up to date");

                    // Final refresh of unified inbox
                    if app.imp().state.borrow().unified_inbox {
                        app.fetch_unified_inbox();
                    }

                    // Hide sync status after a short delay
                    glib::timeout_future(std::time::Duration::from_secs(2)).await;
                    app.hide_sync_status();
                } else {
                    // Show remaining accounts being synced
                    app.update_sync_status_from_map(&accounts_syncing.borrow());
                }
            });
        }
    }

    /// Update sync status showing multiple accounts
    fn update_sync_status_multi(&self, emails: &[String], completed: usize) {
        let total = emails.len();
        let status = if completed == 0 {
            format!("Syncing {} accounts...", total)
        } else {
            format!("Syncing... {}/{} accounts", completed, total)
        };
        self.update_simple_sync_status(&status);
    }

    /// Update sync status from account status map
    fn update_sync_status_from_map(&self, statuses: &std::collections::HashMap<String, &str>) {
        let syncing: Vec<_> = statuses.iter()
            .filter(|(_, s)| **s != "done")
            .map(|(email, status)| {
                let short_email = email.split('@').next().unwrap_or(email);
                match *status {
                    "loading" => format!("{} (loading)", short_email),
                    "syncing" => short_email.to_string(),
                    _ => short_email.to_string(),
                }
            })
            .collect();

        let done_count = statuses.iter().filter(|(_, s)| **s == "done").count();
        let total = statuses.len();

        if syncing.is_empty() {
            self.update_simple_sync_status("Up to date");
        } else if syncing.len() <= 2 {
            self.update_simple_sync_status(&format!("Syncing {}... ({}/{})", syncing.join(", "), done_count, total));
        } else {
            self.update_simple_sync_status(&format!("Syncing {} accounts... ({}/{})", syncing.len(), done_count, total));
        }
    }

    /// Update sync status with simple display (no progress bar)
    fn update_simple_sync_status(&self, message: &str) {
        debug!("Updating sync status: {}", message);
        if let Some(window) = self.active_window() {
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                if let Some(sidebar) = win.folder_sidebar() {
                    sidebar.show_simple_sync_status(message);
                } else {
                    debug!("No sidebar found");
                }
            } else {
                debug!("Window is not NorthMailWindow");
            }
        } else {
            debug!("No active window");
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
            let email_for_log = account.email.clone();
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async {
                    db.get_folders(&acct_id).await
                });
                let _ = sender.send(result);
            });
            // Non-blocking polling to avoid freezing UI
            let start = std::time::Instant::now();
            let timeout = std::time::Duration::from_secs(2);
            let mut result = None;
            loop {
                match receiver.try_recv() {
                    Ok(Ok(folders)) if folders.len() > 1 => {
                        let cached: Vec<(String, String, String)> = folders
                            .iter()
                            .map(|f| (f.full_path.clone(), f.name.clone(), f.folder_type.clone()))
                            .collect();
                        info!("Using {} cached folders for {}, skipping list_folders()", cached.len(), email_for_log);
                        result = Some(cached);
                        break;
                    }
                    Ok(_) => break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        if start.elapsed() > timeout { break; }
                        glib::timeout_future(std::time::Duration::from_millis(5)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }
            result
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
                } else if Self::is_ms_graph_account(&account) {
                    match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                        Ok((_email, access_token)) => {
                            debug!("Got Graph API token for {}", account.email);
                            match Self::fetch_inbox_graph_async(access_token, cached_folders.clone()).await {
                                Ok(sr) => {
                                    info!("Synced {} folders via Graph API for {}", sr.folders.len(), account.email);
                                    Some(sr)
                                }
                                Err(e) => {
                                    warn!("Failed to sync via Graph API {}: {}", account.email, e);
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to get Graph API token for {}: {}", account.email, e);
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
                                let res = if let Some(ref gid) = f.graph_folder_id {
                                    db.upsert_folder_graph(
                                        &acct_id,
                                        &f.name,
                                        &f.full_path,
                                        &f.folder_type,
                                        Some(f.message_count as i64),
                                        Some(f.unseen_count as i64),
                                        gid,
                                    )
                                    .await
                                } else {
                                    db.upsert_folder_with_counts(
                                        &acct_id,
                                        &f.name,
                                        &f.full_path,
                                        &f.folder_type,
                                        Some(f.message_count as i64),
                                        Some(f.unseen_count as i64),
                                    )
                                    .await
                                };
                                if let Err(e) = res {
                                    warn!("Failed to upsert folder {}: {}", f.full_path, e);
                                }
                            }
                        });
                        let _ = sender.send(result);
                    });

                    // Wait for DB writes (non-blocking polling)
                    let start = std::time::Instant::now();
                    let timeout = std::time::Duration::from_secs(3);
                    loop {
                        match receiver.try_recv() {
                            Ok(_) => {
                                info!("Saved {} folders for {}", folder_count, account.email);
                                break;
                            }
                            Err(std::sync::mpsc::TryRecvError::Empty) => {
                                if start.elapsed() > timeout {
                                    warn!("Timed out saving folders for {}", account.email);
                                    break;
                                }
                                glib::timeout_future(std::time::Duration::from_millis(5)).await;
                            }
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                warn!("Channel disconnected saving folders for {}", account.email);
                                break;
                            }
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
                                graph_folder_id: None,
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
                                graph_folder_id: None,
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

    /// Prefetch message bodies via Graph API for ms_graph accounts
    async fn body_prefetch_graph(
        db: &std::sync::Arc<northmail_core::Database>,
        account_id: &str,
        folder_path: &str,
    ) {
        // Get folder_id
        let folder_id = {
            let db_clone = db.clone();
            let aid = account_id.to_string();
            let fp = folder_path.to_string();
            let (s, r) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(db_clone.get_or_create_folder_id(&aid, &fp));
                let _ = s.send(result);
            });
            let start = std::time::Instant::now();
            loop {
                match r.try_recv() {
                    Ok(Ok(fid)) => break fid,
                    Ok(Err(e)) => {
                        warn!("Body prefetch (graph): couldn't get folder_id: {}", e);
                        return;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        if start.elapsed() > std::time::Duration::from_secs(5) { return; }
                        glib::timeout_future(std::time::Duration::from_millis(20)).await;
                    }
                    Err(_) => return,
                }
            }
        };

        // Get messages needing bodies
        let messages_to_fetch: Vec<(i64, bool)> = {
            let db_clone = db.clone();
            let (s, r) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(db_clone.get_messages_needing_body_prefetch(folder_id, 30, 50));
                let _ = s.send(result);
            });
            let start = std::time::Instant::now();
            loop {
                match r.try_recv() {
                    Ok(Ok(msgs)) => break msgs,
                    Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        if start.elapsed() > std::time::Duration::from_secs(10) { return; }
                        glib::timeout_future(std::time::Duration::from_millis(20)).await;
                    }
                }
            }
        };

        if messages_to_fetch.is_empty() {
            return;
        }

        info!("Body prefetch (graph): {} messages for {}/{}", messages_to_fetch.len(), account_id, folder_path);

        // Get access token
        let auth_manager = match AuthManager::new().await {
            Ok(am) => am,
            Err(_) => return,
        };
        let access_token = match auth_manager.get_xoauth2_token_for_goa(account_id).await {
            Ok((_email, token)) => token,
            Err(_) => return,
        };

        for (uid, _is_unread) in messages_to_fetch {
            let uid_u32 = uid as u32;

            // Get graph_message_id
            let graph_id = {
                let db_clone = db.clone();
                let (s, r) = std::sync::mpsc::channel();
                let fid = folder_id;
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    let result = rt.block_on(db_clone.get_graph_message_id(fid, uid));
                    let _ = s.send(result);
                });
                let start = std::time::Instant::now();
                loop {
                    match r.try_recv() {
                        Ok(Ok(id)) => break id,
                        Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => break None,
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            if start.elapsed() > std::time::Duration::from_secs(5) { break None; }
                            glib::timeout_future(std::time::Duration::from_millis(10)).await;
                        }
                    }
                }
            };

            let Some(gid) = graph_id else { continue };

            // Fetch MIME body
            let token = access_token.clone();
            let (s, r) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async {
                    let client = northmail_graph::GraphMailClient::new(token);
                    client.fetch_mime_body(&gid).await.map_err(|e| e.to_string())
                });
                let _ = s.send(result);
            });

            let body_result = loop {
                match r.try_recv() {
                    Ok(r) => break r,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        glib::timeout_future(std::time::Duration::from_millis(10)).await;
                    }
                    Err(_) => break Err("Channel disconnected".to_string()),
                }
            };

            if let Ok(raw_body) = body_result {
                let parsed = Self::parse_email_body(&raw_body);
                Self::save_body_to_cache(db, account_id, folder_path, uid_u32, &parsed);
            }

            // Small delay between fetches
            glib::timeout_future(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Look up the Graph message ID from the database for a given UID
    async fn get_graph_message_id_for_uid(
        db: &std::sync::Arc<northmail_core::Database>,
        account_id: &str,
        folder_path: &str,
        uid: u32,
    ) -> Option<String> {
        let db = db.clone();
        let acct_id = account_id.to_string();
        let fp = folder_path.to_string();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
                let folder_id = db.get_or_create_folder_id(&acct_id, &fp).await.ok()?;
                db.get_graph_message_id(folder_id, uid as i64).await.ok()?
            });
            let _ = sender.send(result);
        });
        loop {
            match receiver.try_recv() {
                Ok(result) => return result,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::timeout_future(std::time::Duration::from_millis(5)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return None,
            }
        }
    }

    /// Hash a Graph API message ID to a 31-bit positive integer for use as a UID.
    /// The real Graph ID is stored separately in graph_message_id for API operations.
    fn graph_id_to_uid(graph_id: &str) -> u32 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        graph_id.hash(&mut hasher);
        (hasher.finish() & 0x7FFF_FFFF) as u32
    }

    /// Convert a Graph API message envelope to a MessageInfo for display
    fn graph_envelope_to_message_info(env: &northmail_graph::GraphMessageEnvelope, folder_id: i64) -> MessageInfo {
        let uid = Self::graph_id_to_uid(&env.id);
        let from_name = env.from.as_ref()
            .and_then(|f| f.email_address.name.clone())
            .unwrap_or_default();
        let from_address = env.from.as_ref()
            .and_then(|f| f.email_address.address.clone())
            .unwrap_or_default();
        let from_display = if from_name.is_empty() { from_address.clone() } else { from_name.clone() };

        let to_addresses: Vec<String> = env.to_recipients.iter()
            .filter_map(|r| r.email_address.address.clone())
            .collect();

        let cc_addresses: Vec<String> = env.cc_recipients.iter()
            .filter_map(|r| r.email_address.address.clone())
            .collect();

        let date_str = env.received_date_time.clone().unwrap_or_default();
        let date_epoch = chrono::DateTime::parse_from_rfc3339(&date_str)
            .map(|dt| dt.timestamp())
            .ok();

        let is_starred = env.flag.as_ref()
            .map(|f| f.flag_status == "flagged")
            .unwrap_or(false);

        MessageInfo {
            id: 0, // Will be set by DB upsert
            uid,
            folder_id,
            message_id: env.internet_message_id.clone(),
            subject: env.subject.clone().unwrap_or_default(),
            from: from_display,
            from_address,
            to: to_addresses.join(", "),
            cc: cc_addresses.join(", "),
            date: date_str,
            date_epoch,
            snippet: None,
            is_read: env.is_read,
            is_starred,
            has_attachments: env.has_attachments,
        }
    }

    /// Convert a Graph API message envelope to a DbMessage for database storage
    fn graph_envelope_to_db_message(env: &northmail_graph::GraphMessageEnvelope) -> northmail_core::models::DbMessage {
        let uid = Self::graph_id_to_uid(&env.id) as i64;
        let from_name = env.from.as_ref()
            .and_then(|f| f.email_address.name.clone());
        let from_address = env.from.as_ref()
            .and_then(|f| f.email_address.address.clone());

        let to_addresses: Vec<String> = env.to_recipients.iter()
            .filter_map(|r| r.email_address.address.clone())
            .collect();

        let cc_addresses: Vec<String> = env.cc_recipients.iter()
            .filter_map(|r| r.email_address.address.clone())
            .collect();

        let date_str = env.received_date_time.clone();
        let date_epoch = date_str.as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp());

        let is_starred = env.flag.as_ref()
            .map(|f| f.flag_status == "flagged")
            .unwrap_or(false);

        northmail_core::models::DbMessage {
            id: 0,
            folder_id: 0, // Set by caller
            uid,
            message_id: env.internet_message_id.clone(),
            subject: env.subject.clone(),
            from_address,
            from_name,
            to_addresses: if to_addresses.is_empty() { None } else { Some(to_addresses.join(", ")) },
            cc_addresses: if cc_addresses.is_empty() { None } else { Some(cc_addresses.join(", ")) },
            date_sent: date_str,
            date_epoch,
            snippet: None,
            is_read: env.is_read,
            is_starred,
            has_attachments: env.has_attachments,
            size: 0,
            maildir_path: None,
            body_text: None,
            body_html: None,
        }
    }

    /// Stream inbox messages from Graph API to cache (background sync for ms_graph accounts)
    async fn stream_inbox_to_cache_graph(
        access_token: String,
        account_id: &str,
        db: Option<std::sync::Arc<northmail_core::Database>>,
        sender: &std::sync::mpsc::Sender<FetchEvent>,
    ) {
        let client = northmail_graph::GraphMailClient::new(access_token);

        // Get the Inbox folder ID
        let folders = match client.list_folders().await {
            Ok(f) => f,
            Err(e) => {
                let _ = sender.send(FetchEvent::Error(format!("Graph list_folders failed: {}", e)));
                return;
            }
        };

        let inbox_folder = match folders.iter().find(|f| f.display_name == "Inbox") {
            Some(f) => f,
            None => {
                let _ = sender.send(FetchEvent::Error("Inbox folder not found via Graph API".to_string()));
                return;
            }
        };

        let _ = sender.send(FetchEvent::FolderInfo {
            total_count: inbox_folder.total_item_count as u32,
        });

        // Get folder_id from DB for this account's INBOX
        let folder_id = if let Some(ref db) = db {
            match db.get_or_create_folder_id(account_id, "INBOX").await {
                Ok(id) => id,
                Err(_) => 0,
            }
        } else {
            0
        };

        // Fetch messages in batches of 50
        let batch_size = 50u32;
        let mut skip = 0u32;
        let mut total_synced = 0u32;
        let mut all_uids: Vec<i64> = Vec::new();
        let mut is_first_batch = true;

        loop {
            let (messages, next_link) = match client.list_messages(&inbox_folder.id, batch_size, skip).await {
                Ok(result) => result,
                Err(e) => {
                    warn!("Graph list_messages failed at skip={}: {}", skip, e);
                    let _ = sender.send(FetchEvent::Error(format!("Graph list_messages failed: {}", e)));
                    return;
                }
            };

            if messages.is_empty() {
                break;
            }

            let count = messages.len() as u32;

            // Convert to MessageInfo for UI and DbMessage for DB
            let message_infos: Vec<MessageInfo> = messages.iter()
                .map(|env| Self::graph_envelope_to_message_info(env, folder_id))
                .collect();

            // Collect UIDs for stale cleanup
            for info in &message_infos {
                all_uids.push(info.uid as i64);
            }

            // Save to DB with graph_message_id
            if let Some(ref db) = db {
                let db_messages: Vec<(northmail_core::models::DbMessage, String)> = messages.iter()
                    .map(|env| (Self::graph_envelope_to_db_message(env), env.id.clone()))
                    .collect();
                if let Err(e) = db.upsert_messages_batch_graph(folder_id, &db_messages).await {
                    warn!("Failed to save Graph messages to cache: {}", e);
                }
            }

            // Send to UI
            if is_first_batch {
                let _ = sender.send(FetchEvent::Messages(message_infos));
                is_first_batch = false;
            } else {
                let _ = sender.send(FetchEvent::BackgroundMessages(message_infos));
            }

            total_synced += count;

            let _ = sender.send(FetchEvent::SyncProgress {
                synced: total_synced,
                total: inbox_folder.total_item_count as u32,
            });

            if next_link.is_none() || count < batch_size {
                break;
            }

            skip += batch_size;
        }

        // Signal completion
        if is_first_batch {
            // No messages at all
            let _ = sender.send(FetchEvent::InitialBatchDone { lowest_seq: 0 });
        } else {
            let _ = sender.send(FetchEvent::InitialBatchDone { lowest_seq: 1 });
        }
    }

    /// Map Graph API folder displayName to folder type string
    fn graph_folder_type(display_name: &str) -> &'static str {
        match display_name {
            "Inbox" => "inbox",
            "Sent Items" => "sent",
            "Drafts" => "drafts",
            "Deleted Items" => "trash",
            "Junk Email" => "spam",
            "Archive" => "archive",
            _ => "other",
        }
    }

    /// Fetch inbox folder list via Microsoft Graph API (for ms_graph accounts)
    async fn fetch_inbox_graph_async(
        access_token: String,
        _cached_folders: Option<Vec<(String, String, String)>>,
    ) -> Result<SyncResult, String> {
        let (sender, receiver) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
                let client = northmail_graph::GraphMailClient::new(access_token);

                let graph_folders = client.list_folders().await
                    .map_err(|e| format!("Graph list_folders failed: {}", e))?;

                let mut folders = Vec::new();
                let mut inbox_count: usize = 0;

                for gf in &graph_folders {
                    let folder_type = Self::graph_folder_type(&gf.display_name);
                    if folder_type == "inbox" {
                        inbox_count = gf.total_item_count as usize;
                    }
                    // Normalize full_path: use "INBOX" to match IMAP convention
                    let full_path = if folder_type == "inbox" {
                        "INBOX".to_string()
                    } else {
                        gf.display_name.clone()
                    };
                    folders.push(SyncedFolder {
                        name: gf.display_name.clone(),
                        full_path,
                        folder_type: folder_type.to_string(),
                        message_count: gf.total_item_count as u32,
                        unseen_count: gf.unread_item_count as u32,
                        graph_folder_id: Some(gf.id.clone()),
                    });
                }

                Ok(SyncResult { inbox_count, folders })
            });

            let _ = sender.send(result);
        });

        Self::poll_result_channel(receiver).await
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
                                graph_folder_id: None,
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

        // Longer timeout to handle database contention
        let timeout = std::time::Duration::from_secs(15);
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
    /// This is async to avoid blocking the main thread
    fn refresh_sidebar_folders(&self) {
        let accounts = self.imp().accounts.borrow().clone();
        if accounts.is_empty() {
            return;
        }

        let db = match self.database() {
            Some(db) => db.clone(),
            None => return,
        };

        let app = self.clone();
        let account_ids: Vec<String> = accounts.iter().map(|a| a.id.clone()).collect();

        // Spawn database query in background thread
        let (tx, rx) = std::sync::mpsc::channel();
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
            let _ = tx.send(result);
        });

        // Poll for results without blocking main thread
        glib::spawn_future_local(async move {
            let start = std::time::Instant::now();
            let timeout = std::time::Duration::from_secs(10);

            loop {
                match rx.try_recv() {
                    Ok(cached_folders_map) => {
                        // Build account folders from the results
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

                        // Don't clear sidebar if we failed to load folders
                        let total_folders: usize = account_folders.iter().map(|a| a.folders.len()).sum();
                        if total_folders == 0 && !accounts.is_empty() {
                            debug!("refresh_sidebar_folders: skipping update - no folders loaded");
                            return;
                        }

                        if let Some(window) = app.active_window() {
                            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                if let Some(sidebar) = win.folder_sidebar() {
                                    sidebar.set_accounts(account_folders);
                                }
                            }
                        }
                        return;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        if start.elapsed() > timeout {
                            debug!("refresh_sidebar_folders: timeout waiting for folders");
                            return;
                        }
                        // Yield to GTK main loop
                        glib::timeout_future(std::time::Duration::from_millis(100)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        return;
                    }
                }
            }
        });
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

        // Non-blocking poll with yield to GTK main loop
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(2);
        loop {
            match receiver.try_recv() {
                Ok(Ok((folder_id, messages))) => {
                    if messages.is_empty() {
                        info!("📭 Cache MISS: No cached messages for {}/{}", account_id, folder_path);
                        return None;
                    } else {
                        info!(
                            "📬 Cache HIT: Loaded {} cached messages for {}/{}",
                            messages.len(),
                            account_id,
                            folder_path
                        );
                        let pending = self.imp().pending_deletes.borrow();
                        let message_infos: Vec<MessageInfo> = messages
                            .iter()
                            .map(MessageInfo::from)
                            .filter(|m| !pending.contains(&(folder_id, m.uid)))
                            .collect();
                        if message_infos.is_empty() {
                            return None;
                        }
                        return Some((folder_id, message_infos));
                    }
                }
                Ok(Err(e)) => {
                    warn!("Failed to load cached messages: {}", e);
                    return None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if start.elapsed() > timeout {
                        warn!("Cache load timed out");
                        return None;
                    }
                    glib::timeout_future(std::time::Duration::from_millis(5)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    warn!("Cache load thread disconnected");
                    return None;
                }
            }
        }
    }

    /// Check if cache has more messages beyond what's loaded
    async fn check_cache_has_more(&self, folder_id: i64, loaded_count: i64) -> bool {
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

        // Non-blocking poll with yield to GTK main loop
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(500);
        loop {
            match receiver.try_recv() {
                Ok(Ok(total)) => {
                    debug!("Cache has {} total messages, loaded {}", total, loaded_count);
                    return total > loaded_count;
                }
                Ok(Err(_)) => return false,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if start.elapsed() > timeout {
                        return false;
                    }
                    glib::timeout_future(std::time::Duration::from_millis(5)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return false,
            }
        }
    }

    /// Load more messages from the SQLite cache (pagination)
    fn load_more_from_cache(&self) {
        let folder_id = self.imp().cache_folder_id.get();
        let offset = self.imp().cache_offset.get();
        let batch_size: i64 = 50;

        info!("load_more_from_cache called: folder_id={}, offset={}", folder_id, offset);

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
        // Snapshot pending deletes to filter out messages being moved/deleted
        let pending = self.imp().pending_deletes.borrow().clone();

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

                // Filter out messages with pending deletes
                let messages: Vec<&MessageInfo> = messages
                    .iter()
                    .filter(|m| !pending.contains(&(folder_id, m.uid)))
                    .collect();

                if messages.is_empty() {
                    return;
                }

                // Build batch of DbMessages
                let db_messages: Vec<northmail_core::models::DbMessage> = messages
                    .iter()
                    .map(|msg| {
                        northmail_core::models::DbMessage {
                            id: 0,
                            folder_id,
                            uid: msg.uid as i64,
                            message_id: msg.message_id.clone(),
                            subject: Some(msg.subject.clone()),
                            from_address: Some(msg.from_address.clone()),
                            from_name: Some(msg.from.clone()),
                            to_addresses: if msg.to.is_empty() { None } else { Some(msg.to.clone()) },
                            cc_addresses: if msg.cc.is_empty() { None } else { Some(msg.cc.clone()) },
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

        // Track folder type for UI behavior (e.g., show Edit button for drafts)
        let folder_type = Self::guess_folder_type(&folder_path);
        *self.imp().current_folder_type.borrow_mut() = folder_type;

        // Highlight the selected folder in the sidebar and update window title
        if let Some(window) = self.active_window() {
            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                if let Some(sidebar) = win.folder_sidebar() {
                    sidebar.select_folder(&account_id, &folder_path);
                }
                // Clear tracked message when switching folders
                win.clear_current_message();
            }
            // Update window title with friendly folder name
            let folder_name = Self::friendly_folder_name(&folder_path);
            window.set_title(Some(&format!("{} — NorthMail", folder_name)));
        }

        let account_email = account.email.clone();
        let account_id_clone = account.id.clone();
        let is_google = Self::is_google_account(&account);
        let is_microsoft = Self::is_microsoft_account(&account);
        let is_ms_graph = Self::is_ms_graph_account(&account);
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
                            // Clear search when switching folders
                            message_list.clear_search();
                            // Set folder context for drag-and-drop
                            message_list.set_folder_context(&account_id, &folder_path);
                            message_list.set_messages(cached_messages);

                            // Wire up "load more" from cache
                            let app_clone = app.clone();
                            message_list.connect_load_more(move || {
                                app_clone.load_more_from_cache();
                            });

                            // Check if there are more messages in cache
                            let has_more = app.check_cache_has_more(folder_id, loaded_count).await;
                            message_list.set_can_load_more(has_more);
                        }
                    }
                }

                // Show simple sync status for background update
                app.update_simple_sync_status("Checking for updates...");
                true
            } else {
                // No cache - show skeleton loading for immediate feedback
                if let Some(window) = app.active_window() {
                    if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                        win.restore_message_list();
                        if let Some(message_list) = win.message_list() {
                            message_list.show_loading();
                        }
                    }
                }
                app.update_simple_sync_status("Loading messages...");
                false
            };

            // Query min cached UID for resume sync
            let min_cached_uid = if has_cache {
                let cache_fid = app.imp().cache_folder_id.get();
                if cache_fid > 0 {
                    if let Some(db) = app.database() {
                        let db = db.clone();
                        let (s, r) = std::sync::mpsc::channel();
                        std::thread::spawn(move || {
                            let rt = tokio::runtime::Runtime::new().unwrap();
                            let result = rt.block_on(db.get_min_uid(cache_fid));
                            let _ = s.send(result);
                        });
                        // Non-blocking poll
                        let start = std::time::Instant::now();
                        let timeout = std::time::Duration::from_millis(500);
                        loop {
                            match r.try_recv() {
                                Ok(Ok(uid)) => {
                                    if uid.is_some() {
                                        info!("Resume sync: min_cached_uid={:?} for {}/{}", uid, account_email, folder_path);
                                    }
                                    break uid;
                                }
                                Ok(Err(_)) => break None,
                                Err(std::sync::mpsc::TryRecvError::Empty) => {
                                    if start.elapsed() > timeout {
                                        break None;
                                    }
                                    glib::timeout_future(std::time::Duration::from_millis(5)).await;
                                }
                                Err(std::sync::mpsc::TryRecvError::Disconnected) => break None,
                            }
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // Phase 2: Fetch from IMAP (updates cache and UI)
            debug!(
                "Starting IMAP sync for {}/{} (has_cache: {}, min_cached_uid: {:?})",
                account_email, folder_path, has_cache, min_cached_uid
            );

            match AuthManager::new().await {
                Ok(auth_manager) => {
                    if is_ms_graph {
                        // Microsoft Graph API (no IMAP)
                        match auth_manager
                            .get_xoauth2_token_for_goa(&account_id_clone)
                            .await
                        {
                            Ok((_email, access_token)) => {
                                debug!("Got Graph API token for folder fetch");
                                let result = Self::fetch_folder_graph(
                                    account_id_clone.clone(),
                                    access_token,
                                    folder_path.clone(),
                                    has_cache,
                                    generation,
                                    &app,
                                ).await;

                                if let Err(e) = result {
                                    error!("Failed to fetch messages via Graph: {}", e);
                                    app.show_error(&format!("Failed to fetch messages: {}", e));
                                }
                            }
                            Err(e) => {
                                error!("Failed to get Graph API token: {}", e);
                                app.show_error(&format!("Authentication failed: {}", e));
                            }
                        }
                    } else if is_google {
                        // Google OAuth2 auth
                        match auth_manager
                            .get_xoauth2_token_for_goa(&account_id_clone)
                            .await
                        {
                            Ok((email, access_token)) => {
                                debug!("Got OAuth2 token for {}", email);

                                let folder_path_clone = folder_path.clone();
                                let result =
                                    Self::fetch_folder_streaming_oauth2(account_id_clone.clone(), email, access_token, folder_path_clone, has_cache, generation, min_cached_uid, &app)
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
                                    Self::fetch_folder_streaming_microsoft(account_id_clone.clone(), email, access_token, folder_path_clone, has_cache, generation, min_cached_uid, &app)
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
                                    Self::fetch_folder_streaming_password(account_id_clone.clone(), host, username, password, folder_path_clone, has_cache, generation, min_cached_uid, &app)
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
        let is_ms_graph = Self::is_ms_graph_account(&account);
        let imap_host = account.imap_host.clone();
        let imap_username = account.imap_username.clone();
        let account_id = account.id.clone();

        glib::spawn_future_local(async move {
            info!("Loading more messages for {}", state.folder_path);

            match AuthManager::new().await {
                Ok(auth_manager) => {
                    if is_ms_graph {
                        // Graph API pagination is handled via cache — load more from DB
                        info!("load_more_messages: ms_graph accounts load from cache");
                        app.load_more_from_cache();
                    } else if is_google {
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
        min_cached_uid: Option<u32>,
        app: &NorthMailApplication,
    ) -> Result<(), String> {
        let (sender, receiver) = std::sync::mpsc::channel::<FetchEvent>();
        let folder_path_clone = folder_path.clone();

        std::thread::spawn(move || {
            async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();

                match client.connect_gmail(&email, &access_token).await {
                    Ok(_) => {
                        Self::fetch_streaming(&mut client, &folder_path_clone, &sender, true, min_cached_uid).await;
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
        min_cached_uid: Option<u32>,
        app: &NorthMailApplication,
    ) -> Result<(), String> {
        let (sender, receiver) = std::sync::mpsc::channel::<FetchEvent>();
        let folder_path_clone = folder_path.clone();

        std::thread::spawn(move || {
            async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();

                match client.connect_outlook(&email, &access_token).await {
                    Ok(_) => {
                        Self::fetch_streaming(&mut client, &folder_path_clone, &sender, true, min_cached_uid).await;
                    }
                    Err(e) => {
                        let _ = sender.send(FetchEvent::Error(format!("Authentication failed: {}", e)));
                    }
                }
            });
        });

        Self::handle_fetch_events(receiver, &account_id, &folder_path, has_cache, generation, app).await
    }

    /// Fetch folder messages via Microsoft Graph API
    async fn fetch_folder_graph(
        account_id: String,
        access_token: String,
        folder_path: String,
        has_cache: bool,
        generation: u64,
        app: &NorthMailApplication,
    ) -> Result<(), String> {
        let (sender, receiver) = std::sync::mpsc::channel::<FetchEvent>();
        let folder_path_clone = folder_path.clone();
        let account_id_clone = account_id.clone();
        let db = app.database().cloned();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let client = northmail_graph::GraphMailClient::new(access_token);

                // Resolve Graph folder ID from display name
                let graph_folder_id = match Self::resolve_graph_folder_id(&client, &folder_path_clone).await {
                    Ok(id) => id,
                    Err(e) => {
                        let _ = sender.send(FetchEvent::Error(e));
                        return;
                    }
                };

                // Get DB folder_id
                let folder_id = if let Some(ref db) = db {
                    db.get_or_create_folder_id(&account_id_clone, &folder_path_clone).await.unwrap_or(0)
                } else {
                    0
                };

                // Fetch messages
                let batch_size = 50u32;
                let mut skip = 0u32;
                let mut is_first = true;
                let mut total_fetched = 0u32;

                loop {
                    let (messages, next_link) = match client.list_messages(&graph_folder_id, batch_size, skip).await {
                        Ok(r) => r,
                        Err(e) => {
                            let _ = sender.send(FetchEvent::Error(format!("Graph list_messages: {}", e)));
                            return;
                        }
                    };

                    if messages.is_empty() {
                        break;
                    }

                    let count = messages.len() as u32;
                    total_fetched += count;
                    let message_infos: Vec<MessageInfo> = messages.iter()
                        .map(|env| Self::graph_envelope_to_message_info(env, folder_id))
                        .collect();

                    // Save to DB
                    if let Some(ref db) = db {
                        let db_messages: Vec<(northmail_core::models::DbMessage, String)> = messages.iter()
                            .map(|env| (Self::graph_envelope_to_db_message(env), env.id.clone()))
                            .collect();
                        let _ = db.upsert_messages_batch_graph(folder_id, &db_messages).await;
                    }

                    if is_first {
                        let _ = sender.send(FetchEvent::Messages(message_infos));
                        is_first = false;
                    } else {
                        let _ = sender.send(FetchEvent::BackgroundMessages(message_infos));
                    }

                    if next_link.is_none() || count < batch_size {
                        break;
                    }
                    skip += batch_size;
                }

                let _ = sender.send(FetchEvent::InitialBatchDone { lowest_seq: 0 });
                let _ = sender.send(FetchEvent::FullSyncDone { total_synced: total_fetched });
            });
        });

        Self::handle_fetch_events(receiver, &account_id, &folder_path, has_cache, generation, app).await
    }

    /// Resolve a Graph API folder ID from a display name
    async fn resolve_graph_folder_id(
        client: &northmail_graph::GraphMailClient,
        folder_display_name: &str,
    ) -> Result<String, String> {
        // Well-known folder names can be used directly as IDs in Graph API
        match folder_display_name {
            "Inbox" | "INBOX" => return Ok("Inbox".to_string()),
            "Drafts" => return Ok("Drafts".to_string()),
            "Sent Items" => return Ok("SentItems".to_string()),
            "Deleted Items" => return Ok("DeletedItems".to_string()),
            "Junk Email" => return Ok("JunkEmail".to_string()),
            "Archive" => return Ok("Archive".to_string()),
            _ => {}
        }

        // For other folders, look up by listing
        let folders = client.list_folders().await
            .map_err(|e| format!("Failed to list folders: {}", e))?;

        folders.iter()
            .find(|f| f.display_name == folder_display_name)
            .map(|f| f.id.clone())
            .ok_or_else(|| format!("Folder '{}' not found", folder_display_name))
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
        min_cached_uid: Option<u32>,
        app: &NorthMailApplication,
    ) -> Result<(), String> {
        let (sender, receiver) = std::sync::mpsc::channel::<FetchEvent>();
        let folder_path_clone = folder_path.clone();

        std::thread::spawn(move || {
            async_std::task::block_on(async {
                let mut client = SimpleImapClient::new();

                match client.connect_login(&host, 993, &username, &password).await {
                    Ok(_) => {
                        Self::fetch_streaming(&mut client, &folder_path_clone, &sender, true, min_cached_uid).await;
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
    /// Fetches initial batch for display, syncs flags, then continues syncing remaining messages.
    /// If `min_cached_uid` is provided, Phase 2 resumes from that UID downward using UID FETCH.
    async fn fetch_streaming(
        client: &mut SimpleImapClient,
        folder_path: &str,
        sender: &std::sync::mpsc::Sender<FetchEvent>,
        _is_initial: bool,
        min_cached_uid: Option<u32>,
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

                // Phase 1: Fetch initial batch for immediate display (sequence-number FETCH)
                const INITIAL_BATCH: u32 = 50;
                const PREFETCH_BODIES: usize = 5;

                let initial_end = count;
                let initial_start = if count > INITIAL_BATCH { count - INITIAL_BATCH + 1 } else { 1 };

                let range = format!("{}:{}", initial_start, initial_end);
                match client.fetch_headers(&range).await {
                    Ok(headers) => {
                        let messages = Self::headers_to_message_info(&headers, 0);

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

                // Phase 1.5: Sync flags for all cached messages
                // Lightweight UID FETCH 1:* (FLAGS) to detect read/starred changes from other devices
                tracing::info!("Phase 1.5: syncing flags for all messages");
                match client.uid_fetch_flags("1:*").await {
                    Ok(flags) => {
                        if !flags.is_empty() {
                            tracing::info!("Flags sync: got {} flag entries", flags.len());
                            let _ = sender.send(FetchEvent::FlagsUpdated(flags));
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Flags sync failed (non-fatal): {}", e);
                    }
                }

                // Phase 2: Background sync - fetch remaining messages
                if let Some(min_uid) = min_cached_uid {
                    // Resume mode: only fetch UIDs below the oldest cached message
                    if min_uid > 1 {
                        let mut synced = INITIAL_BATCH.min(count);
                        let mut current_upper = min_uid - 1;
                        const UID_BATCH: u32 = 5000;

                        tracing::info!(
                            "Phase 2 (resume): fetching UIDs 1..{} (below min_cached_uid={})",
                            current_upper, min_uid
                        );

                        while current_upper > 0 {
                            let batch_lower = if current_upper > UID_BATCH {
                                current_upper - UID_BATCH + 1
                            } else {
                                1
                            };

                            let range = format!("{}:{}", batch_lower, current_upper);
                            match client.uid_fetch_headers(&range).await {
                                Ok(headers) => {
                                    let messages = Self::headers_to_message_info(&headers, 0);
                                    let batch_count = messages.len() as u32;
                                    synced += batch_count;

                                    if sender.send(FetchEvent::BackgroundMessages(messages)).is_err() {
                                        tracing::info!("Background sync cancelled (receiver dropped) at {}/{}", synced, count);
                                        break;
                                    }
                                    let _ = sender.send(FetchEvent::SyncProgress {
                                        synced,
                                        total: count,
                                    });

                                    current_upper = if batch_lower > 1 { batch_lower - 1 } else { 0 };
                                }
                                Err(e) => {
                                    tracing::warn!("Background UID sync batch failed: {}", e);
                                    current_upper = if current_upper > UID_BATCH {
                                        current_upper - UID_BATCH
                                    } else {
                                        0
                                    };
                                }
                            }
                        }

                        tracing::info!("Background sync (resume) complete: {} messages synced", synced);
                        let _ = sender.send(FetchEvent::FullSyncDone { total_synced: synced });
                    } else {
                        // min_cached_uid == 1 means all UIDs are cached
                        tracing::info!("All UIDs already cached (min_uid=1), skipping Phase 2");
                        let _ = sender.send(FetchEvent::FullSyncDone { total_synced: count });
                    }
                } else {
                    // First sync: use sequence-number FETCH (original behavior)
                    if initial_start > 1 {
                        let mut synced = INITIAL_BATCH.min(count);
                        let mut current_end = initial_start - 1;
                        const BACKGROUND_BATCH: u32 = 500;

                        tracing::info!(
                            "Phase 2 (first sync): {} more messages to fetch",
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
                                    let messages = Self::headers_to_message_info(&headers, 0);
                                    let batch_count = messages.len() as u32;
                                    synced += batch_count;

                                    if sender.send(FetchEvent::BackgroundMessages(messages)).is_err() {
                                        tracing::info!("Background sync cancelled (receiver dropped) at {}/{}", synced, count);
                                        break;
                                    }
                                    let _ = sender.send(FetchEvent::SyncProgress {
                                        synced,
                                        total: count,
                                    });

                                    current_end = batch_start - 1;
                                }
                                Err(e) => {
                                    tracing::warn!("Background sync batch failed: {}", e);
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

    /// Check if an account has no cached inbox messages
    async fn account_inbox_is_empty(&self, account_id: &str) -> bool {
        let Some(db) = self.database() else { return true };
        let db = db.clone();
        let aid = account_id.to_string();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let count = rt.block_on(db.get_account_message_count(&aid)).unwrap_or(0);
            let _ = sender.send(count);
        });
        loop {
            match receiver.try_recv() {
                Ok(count) => return count == 0,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::timeout_future(std::time::Duration::from_millis(5)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return true,
            }
        }
    }

    /// Stream an account's INBOX messages from IMAP to cache (background sync)
    /// Returns after the initial batch (first ~50 messages) is cached.
    /// Remaining messages continue syncing in a background task.
    async fn stream_inbox_to_cache(&self, account: &northmail_auth::GoaAccount) {
        let account_id = account.id.clone();
        let email = account.email.clone();

        // Prevent duplicate concurrent syncs for the same account
        {
            let mut syncing = self.imp().syncing_accounts.borrow_mut();
            if syncing.contains(&account_id) {
                info!("Skipping sync for {} - already in progress", email);
                return;
            }
            syncing.insert(account_id.clone());
        }
        let is_google = Self::is_google_account(account);
        let is_microsoft = Self::is_microsoft_account(account);
        let is_ms_graph = Self::is_ms_graph_account(account);
        let is_password = Self::is_password_account(account);
        let imap_username = account.imap_username.clone();
        let imap_host = account.imap_host.clone();

        // Get auth credentials
        let auth_manager = match AuthManager::new().await {
            Ok(am) => am,
            Err(e) => {
                warn!("Failed to create auth manager for background sync of {}: {}", email, e);
                return;
            }
        };

        let (sender, receiver) = std::sync::mpsc::channel::<FetchEvent>();

        if is_ms_graph {
            // Microsoft Graph API path — no IMAP
            match auth_manager.get_xoauth2_token_for_goa(&account_id).await {
                Ok((_email_addr, access_token)) => {
                    let db = self.database().cloned();
                    let acct_id = account_id.clone();
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        rt.block_on(async {
                            Self::stream_inbox_to_cache_graph(access_token, &acct_id, db, &sender).await;
                        });
                    });
                }
                Err(e) => {
                    warn!("Failed to get Graph API token for {}: {}", email, e);
                    self.imp().syncing_accounts.borrow_mut().remove(&account_id);
                    return;
                }
            }
        } else if is_google || is_microsoft {
            match auth_manager.get_xoauth2_token_for_goa(&account_id).await {
                Ok((email_addr, access_token)) => {
                    let is_gmail = is_google;
                    std::thread::spawn(move || {
                        async_std::task::block_on(async {
                            let mut client = SimpleImapClient::new();
                            let result = if is_gmail {
                                client.connect_gmail(&email_addr, &access_token).await
                            } else {
                                client.connect_outlook(&email_addr, &access_token).await
                            };
                            match result {
                                Ok(_) => {
                                    Self::fetch_streaming(&mut client, "INBOX", &sender, true, None).await;
                                }
                                Err(e) => {
                                    let _ = sender.send(FetchEvent::Error(format!("Auth failed: {}", e)));
                                }
                            }
                        });
                    });
                }
                Err(e) => {
                    warn!("Failed to get OAuth2 token for {}: {}", email, e);
                    return;
                }
            }
        } else if is_password {
            match auth_manager.get_goa_password(&account_id).await {
                Ok(password) => {
                    let username = imap_username.unwrap_or(email.clone());
                    let host = imap_host.unwrap_or_else(|| "imap.mail.me.com".to_string());
                    std::thread::spawn(move || {
                        async_std::task::block_on(async {
                            let mut client = SimpleImapClient::new();
                            match client.connect_login(&host, 993, &username, &password).await {
                                Ok(_) => {
                                    Self::fetch_streaming(&mut client, "INBOX", &sender, true, None).await;
                                }
                                Err(e) => {
                                    let _ = sender.send(FetchEvent::Error(format!("Auth failed: {}", e)));
                                }
                            }
                        });
                    });
                }
                Err(e) => {
                    warn!("Failed to get password for {}: {}", email, e);
                    return;
                }
            }
        } else {
            return;
        }

        // Process events until initial batch is done, then continue in background
        let account_id_ref = &account_id;
        loop {
            match receiver.try_recv() {
                Ok(event) => match event {
                    FetchEvent::FolderInfo { total_count } => {
                        info!("Background streaming {}: INBOX has {} messages", email, total_count);
                        if total_count > 0 {
                            self.update_simple_sync_status(
                                &format!("Loading {}... 0/{}", email, format_number(total_count)),
                            );
                        }
                    }
                    FetchEvent::Messages(messages) => {
                        let count = messages.len();
                        self.save_messages_to_cache(account_id_ref, "INBOX", &messages);
                        info!("Background streaming {}: cached {} messages", email, count);
                    }
                    FetchEvent::BackgroundMessages(messages) => {
                        self.save_messages_to_cache(account_id_ref, "INBOX", &messages);
                    }
                    FetchEvent::BodyPrefetched { uid, body } => {
                        let parsed = Self::parse_email_body(&body);
                        if let Some(db) = self.imp().database.get() {
                            Self::save_body_to_cache(db, account_id_ref, "INBOX", uid, &parsed);
                        }
                    }
                    FetchEvent::FlagsUpdated(flags) => {
                        // FlagsUpdated contains ALL server UIDs - use for stale cleanup too
                        if let Some(db) = self.database() {
                            let db = db.clone();
                            let aid = account_id_ref.to_string();
                            let server_uids: Vec<i64> = flags.iter().map(|&(uid, _, _)| uid as i64).collect();
                            std::thread::spawn(move || {
                                let rt = tokio::runtime::Runtime::new().unwrap();
                                rt.block_on(async {
                                    if let Ok(folder_id) = db.get_or_create_folder_id(&aid, "INBOX").await {
                                        // Update flags
                                        match db.batch_update_flags(folder_id, &flags).await {
                                            Ok(updated) => {
                                                tracing::info!("Background flags sync: updated {} cached messages for {}", updated, aid);
                                            }
                                            Err(e) => {
                                                tracing::warn!("Background flags sync failed: {}", e);
                                            }
                                        }
                                        // Clean up stale messages not on server anymore
                                        if !server_uids.is_empty() {
                                            match db.delete_messages_not_in_uids(folder_id, &server_uids).await {
                                                Ok(deleted) => {
                                                    if deleted > 0 {
                                                        tracing::info!("Background cache cleanup: removed {} stale messages from INBOX for {}", deleted, aid);
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::warn!("Background stale cleanup failed: {}", e);
                                                }
                                            }
                                        }
                                    }
                                });
                            });
                        }
                    }
                    FetchEvent::SyncProgress { synced, total } => {
                        self.update_simple_sync_status(
                            &format!("Loading {}... {}/{}", email, format_number(synced), format_number(total)),
                        );
                    }
                    FetchEvent::InitialBatchDone { .. } => {
                        info!("Background streaming {}: initial batch done", email);
                        // Drop the receiver - this will cause the IMAP thread's
                        // Phase 2 sends to fail, stopping background sync early.
                        // We only need the initial batch for unified inbox display.
                        drop(receiver);
                        self.imp().syncing_accounts.borrow_mut().remove(&account_id);
                        return;
                    }
                    FetchEvent::FullSyncDone { .. } => {
                        info!("Background streaming {}: complete", email);
                        self.imp().syncing_accounts.borrow_mut().remove(&account_id);
                        return;
                    }
                    FetchEvent::Error(e) => {
                        warn!("Background streaming {} error: {}", email, e);
                        self.imp().syncing_accounts.borrow_mut().remove(&account_id);
                        return;
                    }
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::timeout_future(std::time::Duration::from_millis(10)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.imp().syncing_accounts.borrow_mut().remove(&account_id);
                    return;
                }
            }
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
        // Track resolved folder_id to avoid redundant blocking lookups
        let mut sync_folder_id: Option<i64> = None;

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
                    FetchEvent::Messages(mut messages) => {
                        loaded_count += messages.len() as u32;
                        info!("Received batch of {} messages ({}/{})", messages.len(), loaded_count, total_count);

                        // Look up folder_id and update messages (they come from IMAP with folder_id=0)
                        if let Some(db) = app.database() {
                            let db_clone = db.clone();
                            let aid = account_id.to_string();
                            let fpath = folder_path.to_string();
                            let (sender, receiver) = std::sync::mpsc::channel();
                            std::thread::spawn(move || {
                                let rt = tokio::runtime::Runtime::new().unwrap();
                                let result = rt.block_on(db_clone.get_or_create_folder_id(&aid, &fpath));
                                let _ = sender.send(result);
                            });
                            if let Ok(Ok(folder_id)) = receiver.recv_timeout(std::time::Duration::from_millis(500)) {
                                sync_folder_id = Some(folder_id);
                                for msg in &mut messages {
                                    msg.folder_id = folder_id;
                                }
                                // Filter out messages with pending deletes
                                {
                                    let pending = app.imp().pending_deletes.borrow();
                                    if !pending.is_empty() {
                                        let before = messages.len();
                                        messages.retain(|m| !pending.contains(&(folder_id, m.uid)));
                                        if messages.len() < before {
                                            info!("Filtered {} pending-delete messages from IMAP batch", before - messages.len());
                                        }
                                    }
                                }
                                debug!("Updated {} messages with folder_id={}", messages.len(), folder_id);
                            }
                        }

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
                                    if first_batch {
                                        // When a filter/search is active and we already have
                                        // cached data, don't replace the filtered view with the
                                        // small IMAP initial batch (only ~50 msgs). Instead,
                                        // just append any genuinely new messages (deduped by UID).
                                        // The IMAP data is already saved to cache above.
                                        if has_cache && message_list.has_active_filter() {
                                            info!("Filter active — keeping filtered view, appending {} new IMAP messages", messages.len());
                                            message_list.append_new_messages(messages);
                                        } else {
                                            info!("Replacing message list with {} fresh messages from IMAP", messages.len());
                                            let msg_count = messages.len() as i64;
                                            // Set folder context for drag-and-drop
                                            message_list.set_folder_context(account_id, folder_path);
                                            message_list.set_messages(messages);

                                            // Wire up cache-based "load more" for IMAP path too
                                            // (after IMAP saves to cache, pagination pulls from SQLite)
                                            let app_for_load_more = app.clone();
                                            message_list.connect_load_more(move || {
                                                app_for_load_more.load_more_from_cache();
                                            });
                                            app.imp().cache_offset.set(msg_count);

                                            // Resolve cache_folder_id now so load_more_from_cache works
                                            // before InitialBatchDone fires
                                            if app.imp().cache_folder_id.get() == 0 {
                                                if let Some(db) = app.database() {
                                                    let db = db.clone();
                                                    let aid = account_id.to_string();
                                                    let fp = folder_path.to_string();
                                                    let (s, r) = std::sync::mpsc::channel();
                                                    std::thread::spawn(move || {
                                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                                        let result = rt.block_on(db.get_or_create_folder_id(&aid, &fp));
                                                        let _ = s.send(result);
                                                    });
                                                    // Non-blocking poll
                                                    let start = std::time::Instant::now();
                                                    let timeout = std::time::Duration::from_millis(500);
                                                    loop {
                                                        match r.try_recv() {
                                                            Ok(Ok(fid)) => {
                                                                app.imp().cache_folder_id.set(fid);
                                                                break;
                                                            }
                                                            Ok(Err(_)) => break,
                                                            Err(std::sync::mpsc::TryRecvError::Empty) => {
                                                                if start.elapsed() > timeout {
                                                                    break;
                                                                }
                                                                glib::timeout_future(std::time::Duration::from_millis(5)).await;
                                                            }
                                                            Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        first_batch = false;
                                    } else {
                                        // Subsequent IMAP batches — append with dedup
                                        message_list.append_new_messages(messages);
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
                    FetchEvent::FlagsUpdated(flags) => {
                        // FlagsUpdated comes from UID FETCH 1:* (FLAGS), so it contains ALL server UIDs.
                        // Track them for cache cleanup (critical for resume sync where Phase 2
                        // only fetches a subset of UIDs).
                        synced_uids.extend(flags.iter().map(|&(uid, _, _)| uid as i64));

                        // Batch update flags in cache so next load shows correct read/starred state
                        let flag_count = flags.len();
                        if let Some(db) = app.database() {
                            let db = db.clone();
                            let aid = account_id.to_string();
                            let fp = folder_path.to_string();
                            std::thread::spawn(move || {
                                let rt = tokio::runtime::Runtime::new().unwrap();
                                rt.block_on(async {
                                    if let Ok(folder_id) = db.get_or_create_folder_id(&aid, &fp).await {
                                        match db.batch_update_flags(folder_id, &flags).await {
                                            Ok(updated) => {
                                                tracing::info!("Flags sync: updated {}/{} cached messages for {}/{}", updated, flag_count, aid, fp);
                                            }
                                            Err(e) => {
                                                tracing::warn!("Failed to batch update flags: {}", e);
                                            }
                                        }
                                    }
                                });
                            });
                        }
                    }
                    FetchEvent::BackgroundMessages(messages) => {
                        // Track UIDs for cache cleanup
                        synced_uids.extend(messages.iter().map(|m| m.uid as i64));
                        // Save to cache only - DO NOT update UI here
                        // The UI already shows initial batch from cache.
                        // Updating UI with 500+ messages per batch causes O(n²) widget rebuilds
                        // which freezes the app when syncing large mailboxes (62k+ messages).
                        // Users can use "load more" (pagination) to see older messages.
                        app.save_messages_to_cache(account_id, folder_path, &messages);
                    }
                    FetchEvent::SyncProgress { synced, total } => {
                        // Update sync progress in sidebar (non-intrusive)
                        if !is_stale {
                            app.update_simple_sync_status(&format!("Syncing {}/{}...", format_number(synced), format_number(total)));
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
                            // Non-blocking poll
                            let start = std::time::Instant::now();
                            let timeout = std::time::Duration::from_millis(500);
                            loop {
                                match receiver.try_recv() {
                                    Ok(Ok(fid)) => {
                                        app.imp().cache_folder_id.set(fid);
                                        break;
                                    }
                                    Ok(Err(_)) => break,
                                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                                        if start.elapsed() > timeout {
                                            break;
                                        }
                                        glib::timeout_future(std::time::Duration::from_millis(5)).await;
                                    }
                                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                                }
                            }
                        }

                        // Enable "load more" from cache since IMAP has been saving to DB
                        let cache_folder_id = app.imp().cache_folder_id.get();
                        let cache_offset = app.imp().cache_offset.get();
                        if cache_folder_id > 0 {
                            let has_more = app.check_cache_has_more(cache_folder_id, cache_offset).await;
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

                        // Only clear pending deletes whose UIDs are gone from server
                        // (i.e., NOT in synced_uids). If a UID is still in synced_uids,
                        // the IMAP move hasn't completed yet — keep blocking re-insertion.
                        if let Some(folder_id) = sync_folder_id {
                            let mut pending = app.imp().pending_deletes.borrow_mut();
                            let before = pending.len();
                            pending.retain(|&(fid, uid)| {
                                if fid != folder_id {
                                    return true; // different folder, keep
                                }
                                // Only clear if UID is NOT on server anymore
                                synced_uids.contains(&(uid as i64))
                            });
                            let cleared = before - pending.len();
                            if cleared > 0 {
                                info!("Cleared {} pending deletes (server confirmed removal)", cleared);
                            }
                        }

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
                            let has_more = app.check_cache_has_more(cache_folder_id, cache_offset).await;
                            if let Some(window) = app.active_window() {
                                if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                    if let Some(message_list) = win.message_list() {
                                        message_list.set_can_load_more(has_more);
                                    }
                                }
                            }
                        }

                        // Start background body prefetch for recent messages (last 30 days)
                        app.start_body_prefetch(&account_id, &folder_path);

                        return Ok(());
                    }
                    FetchEvent::Error(e) => {
                        if !is_stale {
                            app.hide_sync_status();
                            // If we were showing loading spinner (no cache, first batch),
                            // restore message list and show empty state
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
                        }
                        return Err(e);
                    }
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::timeout_future(std::time::Duration::from_millis(50)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if !is_stale && first_batch && !has_cache {
                        app.hide_sync_status();
                        if let Some(window) = app.active_window() {
                            if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                                win.restore_message_list();
                                if let Some(message_list) = win.message_list() {
                                    message_list.set_messages(vec![]);
                                }
                            }
                        }
                    }
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
                            let messages = Self::headers_to_message_info(&headers, 0);
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
                    FetchEvent::BackgroundMessages(_) | FetchEvent::SyncProgress { .. } | FetchEvent::FlagsUpdated(_) => {
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
    fn headers_to_message_info(headers: &[northmail_imap::MessageHeader], folder_id: i64) -> Vec<MessageInfo> {
        headers
            .iter()
            .rev()
            .map(|h| {
                let date = h.envelope.date.clone().unwrap_or_default();
                let date_epoch = Self::parse_date_epoch(&date);
                MessageInfo {
                    id: h.uid as i64,
                    uid: h.uid,
                    folder_id,
                    message_id: h.envelope.message_id.clone(),
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
                    from_address: h.envelope.from.first().map(|a| a.address.clone()).unwrap_or_default(),
                    to: h.envelope.to.iter()
                        .map(|a| a.address.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                    cc: h.envelope.cc.iter()
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
        let db = match self.database() {
            Some(db) => db.clone(),
            None => {
                warn!("resolve_folder_info: no database available");
                return None;
            }
        };

        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(db.get_folder_by_id(folder_id));
            let _ = sender.send(result);
        });

        // Timeout for DB query - increased to 10s due to potential database locking
        match receiver.recv_timeout(std::time::Duration::from_millis(10000)) {
            Ok(Ok(Some(folder))) => {
                debug!("resolve_folder_info: folder_id {} -> account={}, path={}",
                       folder_id, folder.account_id, folder.full_path);
                Some((folder.account_id, folder.full_path))
            }
            Ok(Ok(None)) => {
                warn!("resolve_folder_info: folder_id {} not found in database", folder_id);
                None
            }
            Ok(Err(e)) => {
                warn!("resolve_folder_info: database error for folder_id {}: {}", folder_id, e);
                None
            }
            Err(_) => {
                warn!("resolve_folder_info: timeout waiting for folder_id {}", folder_id);
                None
            }
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

        // Track folder type as inbox for unified inbox
        *self.imp().current_folder_type.borrow_mut() = "inbox".to_string();

        // Update window title
        if let Some(window) = self.active_window() {
            window.set_title(Some("All Inboxes — NorthMail"));
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
                                // Clear search when switching folders
                                message_list.clear_search();
                                // Unified inbox: set empty context (drag-and-drop not supported)
                                message_list.set_folder_context("", "UNIFIED_INBOX");
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
        let is_ms_graph = Self::is_ms_graph_account(&account);
        let imap_host = account.imap_host.clone();
        let imap_username = account.imap_username.clone();
        let account_id = account.id.clone();
        let account_email = account.email.clone();
        let db = self.database().cloned();
        let pool = self.imap_pool();

        glib::spawn_future_local(async move {
            // Check cache for text/html body (instant display if no IMAP needed)
            let cached_body = if let Some(ref db) = db {
                Self::get_cached_body(db, &account_id, &folder_path, uid).await
            } else {
                None
            };

            // If we have cached body, check if attachments need data
            let mut cached_body = cached_body;
            if let Some(mut cached) = cached_body.take() {
                let has_empty_attachments = cached.attachments.iter().any(|a| a.data.is_empty() && a.size > 0);
                if !has_empty_attachments {
                    info!("📧 Using cached body for message {} (instant, all attachment data present)", uid);
                    callback(Ok(cached));
                    return;
                }

                // Attachments have metadata but no data — fetch data from server
                if is_ms_graph {
                    // Use Graph API list_attachments to get actual data
                    info!("📧 Cached body for message {} has {} attachments with empty data, fetching via Graph API",
                        uid, cached.attachments.len());
                    let graph_msg_id = if let Some(ref db) = db {
                        Self::get_graph_message_id_for_uid(db, &account_id, &folder_path, uid).await
                    } else { None };

                    if let Some(graph_id) = graph_msg_id {
                        match AuthManager::new().await {
                            Ok(auth_manager) => {
                                if let Ok(token) = auth_manager.get_goa_token(&account_id).await {
                                    let (sender, receiver) = std::sync::mpsc::channel();
                                    std::thread::spawn(move || {
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        let result = rt.block_on(async {
                                            let client = northmail_graph::GraphMailClient::new(token);
                                            client.list_attachments(&graph_id).await
                                                .map_err(|e| format!("Graph list_attachments failed: {}", e))
                                        });
                                        let _ = sender.send(result);
                                    });

                                    let att_result = loop {
                                        match receiver.try_recv() {
                                            Ok(r) => break r,
                                            Err(std::sync::mpsc::TryRecvError::Empty) => {
                                                glib::timeout_future(std::time::Duration::from_millis(10)).await;
                                            }
                                            Err(_) => break Err("Channel disconnected".to_string()),
                                        }
                                    };

                                    if let Ok(server_attachments) = att_result {
                                        // Match server attachments to cached ones by filename
                                        for cached_att in &mut cached.attachments {
                                            if cached_att.data.is_empty() {
                                                if let Some((_, _, data)) = server_attachments.iter()
                                                    .find(|(name, _, _)| name == &cached_att.filename)
                                                {
                                                    cached_att.data = data.clone();
                                                    cached_att.size = data.len();
                                                    info!("📎 Filled attachment data for '{}': {} bytes",
                                                        cached_att.filename, data.len());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => warn!("Auth error fetching attachments: {}", e),
                        }
                    }
                    callback(Ok(cached));
                    return;
                }

                // For IMAP: fall through to re-fetch full body from server
                // (the cache text/html is fine, but we need attachment data from IMAP BODY.PEEK[])
                info!("📧 Cached body for message {} has empty attachment data, re-fetching from IMAP", uid);
            }

            // No cache - fetch from server
            if is_ms_graph {
                // Graph API path: fetch raw MIME via $value endpoint
                info!("Fetching body from Graph API for message {}", uid);
                match AuthManager::new().await {
                    Ok(auth_manager) => {
                        match auth_manager.get_goa_token(&account_id).await {
                            Ok(access_token) => {
                                // Look up graph_message_id from DB
                                let graph_msg_id = if let Some(ref db) = db {
                                    Self::get_graph_message_id_for_uid(db, &account_id, &folder_path, uid).await
                                } else {
                                    None
                                };

                                if let Some(graph_id) = graph_msg_id {
                                    let (sender, receiver) = std::sync::mpsc::channel();
                                    std::thread::spawn(move || {
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        let result = rt.block_on(async {
                                            let client = northmail_graph::GraphMailClient::new(access_token);
                                            client.fetch_mime_body(&graph_id).await
                                                .map_err(|e| format!("Graph fetch body failed: {}", e))
                                        });
                                        let _ = sender.send(result);
                                    });

                                    // Poll for result
                                    let body_result = loop {
                                        match receiver.try_recv() {
                                            Ok(r) => break r,
                                            Err(std::sync::mpsc::TryRecvError::Empty) => {
                                                glib::timeout_future(std::time::Duration::from_millis(10)).await;
                                            }
                                            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                                break Err("Channel disconnected".to_string());
                                            }
                                        }
                                    };

                                    match body_result {
                                        Ok(raw_body) => {
                                            let parsed = Self::parse_email_body(&raw_body);
                                            // Save to DB cache
                                            if let Some(ref db) = db {
                                                Self::save_body_to_cache(db, &account_id, &folder_path, uid, &parsed);
                                            }
                                            callback(Ok(parsed));
                                        }
                                        Err(e) => {
                                            callback(Err(e));
                                        }
                                    }
                                } else {
                                    callback(Err("Graph message ID not found in cache".to_string()));
                                }
                            }
                            Err(e) => callback(Err(format!("Auth failed: {}", e))),
                        }
                    }
                    Err(e) => callback(Err(format!("Auth manager failed: {}", e))),
                }
                return;
            }

            info!("Fetching body from IMAP for message {} (no cache)", uid);

            match AuthManager::new().await {
                Ok(auth_manager) => {
                    // Build credentials for pool
                    let credentials = if is_google {
                        match auth_manager.get_xoauth2_token_for_goa(&account_id).await {
                            Ok((email, access_token)) => {
                                Some(ImapCredentials::Gmail { email, access_token })
                            }
                            Err(e) => {
                                // Fall back to cached body if available
                                if let Some(cached) = cached_body {
                                    callback(Ok(cached));
                                } else {
                                    callback(Err(format!("Auth failed: {}", e)));
                                }
                                return;
                            }
                        }
                    } else if is_microsoft {
                        match auth_manager.get_xoauth2_token_for_goa(&account_id).await {
                            Ok((email, access_token)) => {
                                Some(ImapCredentials::Microsoft { email, access_token })
                            }
                            Err(e) => {
                                if let Some(cached) = cached_body {
                                    callback(Ok(cached));
                                } else {
                                    callback(Err(format!("Auth failed: {}", e)));
                                }
                                return;
                            }
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
                            Err(e) => {
                                if let Some(cached) = cached_body {
                                    callback(Ok(cached));
                                } else {
                                    callback(Err(format!("Auth failed: {}", e)));
                                }
                                return;
                            }
                        }
                    };

                    let Some(credentials) = credentials else {
                        if let Some(cached) = cached_body {
                            callback(Ok(cached));
                        } else {
                            callback(Err("Failed to get credentials".to_string()));
                        }
                        return;
                    };

                    // Use pool to fetch body (reuses existing connection)
                    let result = Self::fetch_body_via_pool(&pool, credentials, &folder_path, uid).await;

                    match result {
                        Ok(body) => {
                            // Save to cache if successful
                            if let Some(ref db) = db {
                                Self::save_body_to_cache(db, &account_id, &folder_path, uid, &body);

                                // Only upgrade has_attachments to true if we found attachments.
                                // Never downgrade to false — the envelope's flag from the server
                                // is authoritative and our MIME parser may miss some types.
                                if !body.attachments.is_empty() {
                                    let db_clone = db.clone();
                                    let aid = account_id.clone();
                                    let fp = folder_path.clone();
                                    std::thread::spawn(move || {
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        rt.block_on(async {
                                            if let Ok(fid) = db_clone.get_or_create_folder_id(&aid, &fp).await {
                                                let _ = db_clone.set_message_has_attachments_by_uid(
                                                    fid, uid as i64, true,
                                                ).await;
                                            }
                                        });
                                    });
                                }
                            }
                            callback(Ok(body));
                        }
                        Err(e) => {
                            // IMAP failed — fall back to cached body if available
                            if let Some(cached) = cached_body {
                                info!("📧 IMAP fetch failed, using cached body for message {}", uid);
                                callback(Ok(cached));
                            } else {
                                callback(Err(e));
                            }
                        }
                    }
                }
                Err(e) => {
                    if let Some(cached) = cached_body {
                        callback(Ok(cached));
                    } else {
                        callback(Err(format!("Auth manager error: {}", e)));
                    }
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
                        info!("fetch_body_via_pool: got body, {} bytes for uid={}", body.len(), uid);
                        if body.is_empty() {
                            warn!("fetch_body_via_pool: EMPTY body returned for uid={}", uid);
                        }
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
                let body = db.get_message_body(folder_id, uid as i64).await?;
                let attachments = db.get_message_attachments(folder_id, uid as i64).await?;
                // Also get has_attachments flag to detect stale cache
                let has_attachments = db.get_message_has_attachments(folder_id, uid as i64).await.unwrap_or(false);
                Ok::<_, northmail_core::CoreError>((body, attachments, has_attachments))
            });
            let _ = sender.send(result);
        });

        // Non-blocking poll with yield to GTK main loop
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(1);
        loop {
            match receiver.try_recv() {
                Ok(Ok((Some((body_text, body_html)), attachments, has_attachments))) => {
                    // Only return if we have at least one body part
                    if body_text.is_some() || body_html.is_some() {
                        // If message is marked as having attachments but we have none cached,
                        // return None to force IMAP re-fetch (stale cache from before attachment caching)
                        if has_attachments && attachments.is_empty() {
                            info!("📭 Body cache STALE: message {} has_attachments=true but 0 cached, forcing re-fetch", uid);
                            return None;
                        }
                        info!("📧 Body cache HIT: Found cached body for message {} ({} attachments)", uid, attachments.len());
                        // Convert cached attachment metadata+data to ParsedAttachment
                        let cached_attachments: Vec<ParsedAttachment> = attachments
                            .into_iter()
                            .map(|a| {
                                let data = a.data.unwrap_or_default();
                                let size = if data.is_empty() { a.size as usize } else { data.len() };
                                ParsedAttachment {
                                    filename: a.filename,
                                    mime_type: a.mime_type,
                                    data,
                                    size,
                                    content_id: a.content_id,
                                }
                            })
                            .collect();
                        return Some(ParsedEmailBody {
                            text: body_text,
                            html: body_html,
                            attachments: cached_attachments,
                        });
                    } else {
                        info!("📭 Body cache MISS: No cached body for message {}", uid);
                        return None;
                    }
                }
                Ok(Ok((_, _, _))) => return None,
                Ok(_) => return None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if start.elapsed() > timeout {
                        return None;
                    }
                    // Yield to GTK main loop
                    glib::timeout_future(std::time::Duration::from_millis(5)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return None,
            }
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
        // Convert attachments to AttachmentInfo for saving (includes data)
        let attachments: Vec<northmail_core::models::AttachmentInfo> = body
            .attachments
            .iter()
            .map(|a| northmail_core::models::AttachmentInfo {
                filename: a.filename.clone(),
                mime_type: a.mime_type.clone(),
                size: a.size,
                content_id: a.content_id.clone(),
                is_inline: a.content_id.is_some(),
                data: a.data.clone(),
            })
            .collect();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                if let Ok(folder_id) = db.get_or_create_folder_id(&account_id, &folder_path).await {
                    // Save body
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
                    }
                    // Save attachment metadata
                    if !attachments.is_empty() {
                        if let Err(e) = db.save_message_attachments(folder_id, uid as i64, &attachments).await {
                            warn!("Failed to cache attachments: {}", e);
                        }
                    }
                    info!("💾 Body cache SAVE: Cached body + {} attachments for message {}", attachments.len(), uid);
                }
            });
        });
    }

    /// Start background body prefetch for recent messages (last 30 days)
    /// Prioritizes unread messages and fetches in batches
    pub fn start_body_prefetch(&self, account_id: &str, folder_path: &str) {
        let db = match self.database() {
            Some(db) => db.clone(),
            None => {
                info!("📭 Body prefetch skipped: no database");
                return;
            }
        };

        let accounts = self.imp().accounts.borrow().clone();
        let account = match accounts.iter().find(|a| a.id == account_id) {
            Some(a) => a.clone(),
            None => {
                warn!("Body prefetch: account {} not found", account_id);
                return;
            }
        };

        // ms_graph: prefetch bodies via Graph API
        if Self::is_ms_graph_account(&account) {
            let account_id = account_id.to_string();
            let folder_path = folder_path.to_string();
            let db_clone = db.clone();
            glib::spawn_future_local(async move {
                Self::body_prefetch_graph(&db_clone, &account_id, &folder_path).await;
            });
            return;
        }

        let pool = self.imap_pool();
        let account_id = account_id.to_string();
        let folder_path = folder_path.to_string();
        let is_google = Self::is_google_account(&account);
        let is_microsoft = Self::is_microsoft_account(&account);
        let imap_host = account.imap_host.clone();
        let imap_username = account.imap_username.clone();
        let account_email = account.email.clone();

        info!("Starting body prefetch for {}/{}", account_id, folder_path);

        glib::spawn_future_local(async move {
            // Get folder_id first
            let folder_id = {
                let db_clone = db.clone();
                let aid = account_id.clone();
                let fp = folder_path.clone();
                let (sender, receiver) = std::sync::mpsc::channel();

                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    let result = rt.block_on(async {
                        db_clone.get_or_create_folder_id(&aid, &fp).await
                    });
                    let _ = sender.send(result);
                });

                // Wait for folder_id with timeout
                let start = std::time::Instant::now();
                loop {
                    match receiver.try_recv() {
                        Ok(Ok(fid)) => break fid,
                        Ok(Err(e)) => {
                            warn!("Body prefetch: couldn't get folder_id: {}", e);
                            return;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            if start.elapsed() > std::time::Duration::from_secs(5) {
                                warn!("Body prefetch: timeout getting folder_id");
                                return;
                            }
                            glib::timeout_future(std::time::Duration::from_millis(20)).await;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                    }
                }
            };

            // Query messages needing body prefetch (last 30 days, limit 50)
            let messages_to_fetch: Vec<(i64, bool)> = {
                let db_clone = db.clone();
                let (sender, receiver) = std::sync::mpsc::channel();

                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    let result = rt.block_on(async {
                        db_clone.get_messages_needing_body_prefetch(folder_id, 30, 50).await
                    });
                    let _ = sender.send(result);
                });

                let start = std::time::Instant::now();
                loop {
                    match receiver.try_recv() {
                        Ok(Ok(msgs)) => break msgs,
                        Ok(Err(e)) => {
                            warn!("Body prefetch: query failed: {}", e);
                            return;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            if start.elapsed() > std::time::Duration::from_secs(10) {
                                warn!("Body prefetch: timeout querying messages");
                                return;
                            }
                            glib::timeout_future(std::time::Duration::from_millis(20)).await;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                    }
                }
            };

            if messages_to_fetch.is_empty() {
                info!("📭 Body prefetch: no messages need prefetching for {}/{}", account_id, folder_path);
                return;
            }

            info!("📦 Body prefetch: {} messages to fetch for {}/{}", messages_to_fetch.len(), account_id, folder_path);

            // Get credentials
            let auth_manager = match AuthManager::new().await {
                Ok(am) => am,
                Err(e) => {
                    warn!("Body prefetch: auth manager error: {}", e);
                    return;
                }
            };

            let credentials = if is_google {
                match auth_manager.get_xoauth2_token_for_goa(&account_id).await {
                    Ok((email, access_token)) => {
                        ImapCredentials::Gmail { email, access_token }
                    }
                    Err(e) => {
                        warn!("Body prefetch: Gmail auth failed: {}", e);
                        return;
                    }
                }
            } else if is_microsoft {
                match auth_manager.get_xoauth2_token_for_goa(&account_id).await {
                    Ok((email, access_token)) => {
                        ImapCredentials::Microsoft { email, access_token }
                    }
                    Err(e) => {
                        warn!("Body prefetch: Microsoft auth failed: {}", e);
                        return;
                    }
                }
            } else {
                let username = imap_username.unwrap_or(account_email);
                let host = imap_host.unwrap_or_else(|| "imap.mail.me.com".to_string());
                match auth_manager.get_goa_password(&account_id).await {
                    Ok(password) => {
                        ImapCredentials::Password {
                            host,
                            port: 993,
                            username,
                            password,
                        }
                    }
                    Err(e) => {
                        warn!("Body prefetch: password auth failed: {}", e);
                        return;
                    }
                }
            };

            // Fetch bodies in batches (with delay to avoid hammering server)
            let mut fetched = 0;
            let total_to_fetch = messages_to_fetch.len();
            for (uid, is_unread) in messages_to_fetch {
                let uid_u32 = uid as u32;

                // Fetch body via pool
                let result = Self::fetch_body_via_pool(&pool, credentials.clone(), &folder_path, uid_u32).await;

                match result {
                    Ok(body) => {
                        // Save to cache (includes attachment metadata)
                        let db_clone = db.clone();
                        let aid = account_id.clone();
                        let fp = folder_path.clone();
                        let body_text = body.text.clone();
                        let body_html = body.html.clone();
                        let attachments: Vec<northmail_core::models::AttachmentInfo> = body
                            .attachments
                            .iter()
                            .map(|a| northmail_core::models::AttachmentInfo {
                                filename: a.filename.clone(),
                                mime_type: a.mime_type.clone(),
                                size: a.size,
                                content_id: a.content_id.clone(),
                                is_inline: a.content_id.is_some(),
                                data: a.data.clone(),
                            })
                            .collect();

                        // Fire and forget save
                        std::thread::spawn(move || {
                            let rt = tokio::runtime::Runtime::new().unwrap();
                            rt.block_on(async {
                                if let Ok(fid) = db_clone.get_or_create_folder_id(&aid, &fp).await {
                                    let _ = db_clone.save_message_body(
                                        fid,
                                        uid,
                                        body_text.as_deref(),
                                        body_html.as_deref(),
                                    ).await;
                                    if !attachments.is_empty() {
                                        let _ = db_clone.save_message_attachments(fid, uid, &attachments).await;
                                    }
                                }
                            });
                        });

                        fetched += 1;
                        let status = if is_unread { "unread" } else { "read" };
                        debug!("📦 Prefetched body {}/{} ({}): uid {}", fetched, total_to_fetch, status, uid);
                    }
                    Err(e) => {
                        debug!("📦 Prefetch failed for uid {}: {}", uid, e);
                    }
                }

                // Small delay between fetches to be nice to the server
                glib::timeout_future(std::time::Duration::from_millis(100)).await;
            }

            info!("📦 Body prefetch complete: fetched {}/{} messages for {}/{}",
                fetched, total_to_fetch, account_id, folder_path);
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

    /// Parse raw email body to extract text, HTML, and attachments using mail-parser
    fn parse_email_body(raw: &str) -> ParsedEmailBody {
        use base64::Engine;

        let mut result = ParsedEmailBody::default();

        debug!("parse_email_body: raw input {} bytes", raw.len());

        let message = match mail_parser::MessageParser::default().parse(raw.as_bytes()) {
            Some(msg) => msg,
            None => {
                warn!("parse_email_body: mail_parser returned None for {} byte input", raw.len());
                return result;
            }
        };

        // Extract text and HTML body
        result.text = message.body_text(0).map(|s| s.into_owned());
        result.html = message.body_html(0).map(|s| s.into_owned());

        debug!("parse_email_body: text={} html={} attachment_parts={}",
            result.text.as_ref().map(|t| t.len()).unwrap_or(0),
            result.html.as_ref().map(|h| h.len()).unwrap_or(0),
            message.attachments().count());

        // Collect inline images (Content-ID parts) for cid: replacement in HTML
        // and separate real attachments from inline resources
        let mut cid_map: Vec<(String, String, Vec<u8>)> = Vec::new(); // (cid, mime_type, data)

        for attachment in message.attachments() {
            let mime_type = MimeHeaders::content_type(attachment)
                .map(|ct| {
                    if let Some(subtype) = ct.subtype() {
                        format!("{}/{}", ct.ctype(), subtype)
                    } else {
                        ct.ctype().to_string()
                    }
                })
                .unwrap_or_else(|| "application/octet-stream".to_string());
            let mime_lower = mime_type.to_lowercase();

            let att_name = attachment.attachment_name().unwrap_or("(unnamed)");
            debug!("parse_email_body: attachment part: name={}, type={}, cid={:?}, data_len={}",
                att_name, mime_type, attachment.content_id(), attachment.contents().len());

            // Skip S/MIME and PGP signatures — not user-facing attachments
            if mime_lower == "application/pkcs7-signature"
                || mime_lower == "application/x-pkcs7-signature"
                || mime_lower == "application/pgp-signature"
            {
                debug!("parse_email_body: skipping signature part: {}", mime_type);
                continue;
            }

            let data = attachment.contents().to_vec();

            // Parts with Content-ID are inline resources for the HTML body (images, etc.)
            // Collect them for cid: replacement, don't show as attachment pills
            if let Some(cid) = attachment.content_id() {
                let cid_clean = cid.trim_start_matches('<').trim_end_matches('>').to_string();
                debug!("parse_email_body: inline CID part: {} ({})", cid_clean, mime_type);
                cid_map.push((cid_clean, mime_type, data));
                continue;
            }

            let filename = attachment
                .attachment_name()
                .unwrap_or("attachment")
                .to_string();

            let size = data.len();
            result.attachments.push(ParsedAttachment {
                filename,
                mime_type,
                data,
                size,
                content_id: None,
            });
        }

        // Replace cid: references in HTML with data: URIs so WebKit can display inline images
        if let Some(ref mut html) = result.html {
            for (cid, mime_type, data) in &cid_map {
                let b64 = base64::prelude::BASE64_STANDARD.encode(data);
                let data_uri = format!("data:{};base64,{}", mime_type, b64);
                tracing::debug!(
                    "CID image: id={}, type={}, data_size={}",
                    cid, mime_type, data.len()
                );

                // Case-insensitive replacement of cid: references
                // Also handle URL-encoded CID values (e.g. %40 for @)
                let cid_url_encoded = cid.replace('@', "%40");
                let needles: Vec<String> = vec![
                    format!("cid:{}", cid),
                    format!("cid:{}", cid_url_encoded),
                ];

                let mut replaced = false;
                for needle in &needles {
                    // Case-insensitive search: find all positions where needle matches
                    let html_lower = html.to_lowercase();
                    let needle_lower = needle.to_lowercase();
                    if html_lower.contains(&needle_lower) {
                        // Replace all case-insensitive occurrences
                        let mut new_html = String::with_capacity(html.len());
                        let mut search_start = 0;
                        while let Some(pos) = html_lower[search_start..].find(&needle_lower) {
                            let abs_pos = search_start + pos;
                            new_html.push_str(&html[search_start..abs_pos]);
                            new_html.push_str(&data_uri);
                            search_start = abs_pos + needle.len();
                        }
                        new_html.push_str(&html[search_start..]);
                        *html = new_html;
                        replaced = true;
                        tracing::debug!("Replaced CID reference '{}' in HTML", needle);
                    }
                }
                if !replaced {
                    tracing::warn!(
                        "CID '{}' collected but no matching reference found in HTML",
                        cid
                    );
                }
            }
        }

        debug!("parse_email_body: RESULT: {} text, {} html, {} attachments, {} inline CIDs",
            result.text.as_ref().map(|t| format!("{} bytes", t.len())).unwrap_or_else(|| "None".to_string()),
            result.html.as_ref().map(|h| format!("{} bytes", h.len())).unwrap_or_else(|| "None".to_string()),
            result.attachments.len(),
            cid_map.len());

        result
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
            .application_icon("email")
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

                            // Add to in-memory account list
                            app.imp().accounts.borrow_mut().push(goa_account.clone());

                            // Save to database
                            app.save_accounts_to_db(&[goa_account.clone()]);

                            // Update sidebar and trigger sync
                            let all_accounts = app.imp().accounts.borrow().clone();
                            app.update_sidebar_with_accounts(&all_accounts);
                            app.sync_all_accounts();

                            app.show_toast(&format!("Added account: {}", goa_account.email));
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

        // App Icon picker
        let icon_picker_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        icon_picker_box.set_halign(gtk4::Align::Center);
        icon_picker_box.set_margin_top(8);
        icon_picker_box.set_margin_bottom(8);

        let make_icon_button = |icon_name: &str, label_text: &str| -> gtk4::ToggleButton {
            let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            vbox.set_halign(gtk4::Align::Center);

            let image = gtk4::Image::from_icon_name(icon_name);
            image.set_pixel_size(48);
            vbox.append(&image);

            let label = gtk4::Label::new(Some(label_text));
            label.add_css_class("caption");
            vbox.append(&label);

            let button = gtk4::ToggleButton::new();
            button.set_child(Some(&vbox));
            button.add_css_class("flat");
            button.set_size_request(100, -1);
            button
        };

        let custom_btn = make_icon_button("org.northmail.NorthMail", "NorthMail");
        let system_btn = make_icon_button("email", "System");
        system_btn.set_group(Some(&custom_btn));

        // Set initial state from GSettings
        let icon_settings = self.settings();
        let current_icon = icon_settings.string("app-icon");
        if current_icon == "system" {
            system_btn.set_active(true);
        } else {
            custom_btn.set_active(true);
        }

        // Restart banner (hidden until icon choice changes)
        let restart_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        restart_box.set_halign(gtk4::Align::Center);
        restart_box.set_margin_top(8);
        restart_box.set_visible(false);

        let restart_label = gtk4::Label::new(Some("Please restart for the changes to take effect."));
        restart_label.add_css_class("dim-label");
        restart_box.append(&restart_label);

        let initial_icon = current_icon.to_string();
        let restart_box_ref = restart_box.clone();
        let settings_for_icon = self.settings();
        system_btn.connect_toggled(move |btn| {
            let (value, icon) = if btn.is_active() {
                ("system", "email")
            } else {
                ("custom", "org.northmail.NorthMail")
            };
            let _ = settings_for_icon.set_string("app-icon", value);
            gtk4::Window::set_default_icon_name(icon);
            restart_box_ref.set_visible(value != initial_icon.as_str());

            // Update desktop file immediately so GNOME has the right icon on next launch
            if let Ok(home) = std::env::var("HOME") {
                let desktop_path = std::path::PathBuf::from(&home)
                    .join(".local/share/applications/org.northmail.NorthMail.desktop");
                if let Ok(contents) = std::fs::read_to_string(&desktop_path) {
                    let patched = contents
                        .lines()
                        .map(|line| {
                            if line.starts_with("Icon=") {
                                format!("Icon={}", icon)
                            } else {
                                line.to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let _ = std::fs::write(&desktop_path, patched);
                }
            }
        });

        icon_picker_box.append(&custom_btn);
        icon_picker_box.append(&system_btn);

        let icon_group = adw::PreferencesGroup::builder()
            .title("App Icon")
            .build();
        icon_group.add(&icon_picker_box);
        icon_group.add(&restart_box);

        general_page.add(&appearance_group);
        general_page.add(&icon_group);

        // Sync group
        let sync_group = adw::PreferencesGroup::builder()
            .title("Mail Checking")
            .build();

        let sync_interval_row = adw::ComboRow::builder()
            .title("Check for New Mail")
            .subtitle("How often to automatically check for new emails")
            .build();

        // Sync interval options: 1, 2, 5, 10, 15, 30 minutes
        let intervals = gtk4::StringList::new(&[
            "Every minute",
            "Every 2 minutes",
            "Every 5 minutes",
            "Every 10 minutes",
            "Every 15 minutes",
            "Every 30 minutes",
        ]);
        sync_interval_row.set_model(Some(&intervals));

        // Map stored interval value to dropdown index
        let settings = self.settings();
        let current_interval = settings.int("sync-interval");
        let interval_index = match current_interval {
            1 => 0u32,
            2 => 1,
            5 => 2,
            10 => 3,
            15 => 4,
            30 => 5,
            _ => 2, // Default to 5 minutes
        };
        sync_interval_row.set_selected(interval_index);

        // Wire up interval changes
        let settings_for_interval = settings.clone();
        sync_interval_row.connect_selected_notify(move |row| {
            let interval = match row.selected() {
                0 => 1,
                1 => 2,
                2 => 5,
                3 => 10,
                4 => 15,
                5 => 30,
                _ => 5,
            };
            let _ = settings_for_interval.set_int("sync-interval", interval);
        });

        sync_group.add(&sync_interval_row);
        general_page.add(&sync_group);

        // Notifications group
        let notifications_group = adw::PreferencesGroup::builder()
            .title("Notifications")
            .build();

        let notifications_row = adw::SwitchRow::builder()
            .title("Desktop Notifications")
            .subtitle("Show notifications for new emails")
            .build();

        // Bind to GSettings
        settings
            .bind("notifications-enabled", &notifications_row, "active")
            .build();

        let sound_row = adw::SwitchRow::builder()
            .title("Notification Sound")
            .subtitle("Play a sound when new emails arrive")
            .build();

        settings
            .bind("notification-sound", &sound_row, "active")
            .build();

        let preview_row = adw::SwitchRow::builder()
            .title("Show Message Preview")
            .subtitle("Display sender and subject in notifications")
            .build();

        settings
            .bind("notification-preview-enabled", &preview_row, "active")
            .build();

        let dnd_row = adw::SwitchRow::builder()
            .title("Do Not Disturb")
            .subtitle("Suppress all notifications")
            .build();

        settings
            .bind("do-not-disturb", &dnd_row, "active")
            .build();

        notifications_group.add(&notifications_row);
        notifications_group.add(&sound_row);
        notifications_group.add(&preview_row);
        notifications_group.add(&dnd_row);
        general_page.add(&notifications_group);

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
                                row_clone.set_subtitle(&format!("{} messages, {} bodies cached", format_number(msg_count), format_number(body_count)));
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
                                // Close dialog
                                if let Some(dialog) = dialog_weak.upgrade() {
                                    dialog.close();
                                }
                                // Trigger a fresh sync of all accounts
                                app.sync_all_accounts();
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
        bcc: Vec<String>,
        subject: String,
        body: String,
        attachments: Vec<(String, String, Vec<u8>)>, // (filename, mime_type, data)
        in_reply_to: Option<String>,
        references: Vec<String>,
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
        let provider_type = account.provider_type.clone();
        let imap_host = account.imap_host.clone();
        let imap_username = account.imap_username.clone();

        // Get user's display name from the system for From header
        let real_name = glib::real_name().to_string_lossy().to_string();
        let from_name = if real_name.is_empty() || real_name == "Unknown" {
            None
        } else {
            Some(real_name)
        };

        debug!("Send: account={} ({}) smtp={} auth={:?}", email, account.provider_type, smtp_host, auth_type);
        debug!("Send: to={:?}, cc={:?}, bcc={:?}, subject={:?}", to, cc, bcc, subject);
        if let Some(ref name) = from_name {
            debug!("Send: from_name={:?}", name);
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
        for addr in &bcc {
            msg = msg.bcc(addr);
        }
        msg = msg.text(&body);
        if let Some(ref reply_id) = in_reply_to {
            msg = msg.reply_to_message(reply_id);
        }
        for ref_id in &references {
            msg = msg.reference(ref_id);
        }
        for (filename, mime_type, data) in attachments {
            msg = msg.attachment(filename, mime_type, data);
        }

        // We need msg for both SMTP send and potentially Sent folder save
        let msg_for_sent = msg.clone();

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

                        let is_ms_graph = provider_type == "ms_graph";
                        let is_microsoft = is_ms_graph || provider_type == "windows_live" || provider_type == "microsoft";
                        let is_gmail = provider_type == "google";

                        let smtp_result = if is_ms_graph {
                            // Use Microsoft Graph API — ms_graph provider has mail.send scope
                            info!("Sending via Microsoft Graph API (ms_graph provider)");
                            let token = auth_manager
                                .get_goa_token(&account_id)
                                .await
                                .map_err(|e| format!("Failed to get token: {}", e))?;
                            northmail_smtp::msgraph::send_via_graph(&token, msg)
                                .await
                                .map_err(|e| format!("Graph API send failed: {}", e))
                        } else if provider_type == "windows_live" {
                            // Legacy windows_live provider uses wl.* scopes — incompatible with
                            // both Graph API (wrong audience) and SMTP XOAUTH2 (no SMTP.Send scope).
                            error!("Cannot send from windows_live account — token lacks mail.send scope");
                            Err("This Microsoft account uses a legacy authentication method that doesn't support sending. \
                                Please remove and re-add it in GNOME Settings → Online Accounts as \"Microsoft 365\".".to_string())
                        } else {
                            match auth_type.clone() {
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
                        };

                        // If send succeeded and not Gmail/Microsoft (both auto-save to Sent), save to Sent folder
                        if smtp_result.is_ok() && !is_gmail && !is_microsoft {
                            debug!("Saving to Sent folder...");
                            if let Err(e) = Self::save_to_sent_folder(
                                &auth_manager,
                                &account_id,
                                &email,
                                &auth_type,
                                &provider_type,
                                imap_host.as_deref(),
                                imap_username.as_deref(),
                                &msg_for_sent,
                            ).await {
                                // Log but don't fail the send - message was sent successfully
                                warn!("Failed to save to Sent folder: {}", e);
                            } else {
                                info!("Saved to Sent folder");
                            }
                        }

                        smtp_result
                    }.await;
                    match &result {
                        Ok(()) => info!("Email sent successfully"),
                        Err(e) => error!("Send failed: {}", e),
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

    /// Save a draft to the account's IMAP Drafts folder via APPEND.
    /// Returns the UID of the saved draft (if server provides APPENDUID).
    /// For ms_graph accounts with an existing_draft_uid, uses PATCH to update
    /// the existing draft (preserving server-side attachments) instead of creating new.
    pub fn save_draft(
        &self,
        account_index: u32,
        msg: northmail_smtp::OutgoingMessage,
        callback: impl FnOnce(Result<Option<u32>, String>) + 'static,
    ) {
        self.save_draft_inner(account_index, msg, None, callback);
    }

    /// Save a draft, optionally updating an existing ms_graph draft by UID.
    pub fn save_draft_update(
        &self,
        account_index: u32,
        msg: northmail_smtp::OutgoingMessage,
        existing_draft_uid: u32,
        callback: impl FnOnce(Result<Option<u32>, String>) + 'static,
    ) {
        self.save_draft_inner(account_index, msg, Some(existing_draft_uid), callback);
    }

    fn save_draft_inner(
        &self,
        account_index: u32,
        msg: northmail_smtp::OutgoingMessage,
        existing_draft_uid: Option<u32>,
        callback: impl FnOnce(Result<Option<u32>, String>) + 'static,
    ) {
        let accounts = self.imp().accounts.borrow().clone();
        let account = match accounts.get(account_index as usize) {
            Some(a) => a.clone(),
            None => {
                callback(Err("Invalid account selection".to_string()));
                return;
            }
        };

        let db = match self.database_ref() {
            Some(db) => db.clone(),
            None => {
                callback(Err("Database not initialized".to_string()));
                return;
            }
        };

        let account_id = account.id.clone();
        let email = account.email.clone();
        let auth_type = account.auth_type.clone();
        let provider_type = account.provider_type.clone();
        let imap_host = account.imap_host.clone();
        let imap_username = account.imap_username.clone();

        let is_ms_graph = provider_type == "ms_graph";

        glib::spawn_future_local(async move {
            let (sender, receiver) = std::sync::mpsc::channel();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    let result = async {
                        let auth_manager = AuthManager::new()
                            .await
                            .map_err(|e| format!("Auth init failed: {}", e))?;

                        if is_ms_graph {
                            // Use Graph API for drafts
                            let token = auth_manager
                                .get_goa_token(&account_id)
                                .await
                                .map_err(|e| format!("Failed to get token: {}", e))?;

                            // Filter out the account's own email from recipients
                            let to_filtered: Vec<String> = msg.to.iter()
                                .filter(|addr| !addr.eq_ignore_ascii_case(&email))
                                .cloned()
                                .collect();
                            let cc_filtered: Vec<String> = msg.cc.iter()
                                .filter(|addr| !addr.eq_ignore_ascii_case(&email))
                                .cloned()
                                .collect();

                            let client = northmail_graph::GraphMailClient::new(token);

                            // If updating an existing draft, use PATCH to preserve attachments
                            if let Some(old_uid) = existing_draft_uid {
                                // Look up graph_message_id from DB
                                let drafts_folder = db.get_drafts_folder(&account_id).await
                                    .map_err(|e| format!("DB error: {}", e))?
                                    .unwrap_or_else(|| "Drafts".to_string());
                                let folder_id = db.get_or_create_folder_id(&account_id, &drafts_folder).await
                                    .map_err(|e| format!("DB error: {}", e))?;

                                if let Ok(Some(graph_id)) = db.get_graph_message_id(folder_id, old_uid as i64).await {
                                    info!("Updating existing ms_graph draft via PATCH: {}", graph_id);
                                    client.update_draft(
                                        &graph_id,
                                        &msg.subject,
                                        msg.text_body.as_deref().unwrap_or(""),
                                        &to_filtered,
                                        &cc_filtered,
                                    )
                                    .await
                                    .map_err(|e| format!("Graph update draft failed: {}", e))?;

                                    // Return the same UID since the draft wasn't recreated
                                    return Ok(Some(old_uid));
                                }
                                // If graph_id not found, fall through to create new
                                warn!("No graph_message_id found for uid {}, creating new draft", old_uid);
                            }

                            // Create new draft (includes attachments)
                            let attachments: Vec<(String, String, Vec<u8>)> = msg.attachments.iter()
                                .filter(|att| !att.data.is_empty()) // Only include attachments with actual data
                                .map(|att| (att.filename.clone(), att.mime_type.clone(), att.data.clone()))
                                .collect();

                            client.create_draft_from_message(
                                &msg.subject,
                                msg.text_body.as_deref().unwrap_or(""),
                                &to_filtered,
                                &cc_filtered,
                                &attachments,
                            )
                            .await
                            .map_err(|e| format!("Graph create draft failed: {}", e))?;

                            // Graph drafts don't return a UID
                            Ok(None)
                        } else {
                            // Build RFC 2822 message bytes
                            let lettre_msg = northmail_smtp::build_lettre_message(&msg)
                                .map_err(|e| format!("Failed to build message: {}", e))?;
                            let message_bytes = lettre_msg.formatted();
                            // Find drafts folder path from DB
                            let drafts_path = db
                                .get_drafts_folder(&account_id)
                                .await
                                .map_err(|e| format!("DB error: {}", e))?
                                .unwrap_or_else(|| "Drafts".to_string());

                            // Connect a SimpleImapClient and APPEND
                            let mut client = SimpleImapClient::new();

                            match auth_type {
                                northmail_auth::GoaAuthType::OAuth2 => {
                                    let (_email, token) = auth_manager
                                        .get_xoauth2_token_for_goa(&account_id)
                                        .await
                                        .map_err(|e| format!("Failed to get token: {}", e))?;

                                    match provider_type.as_str() {
                                        "google" => client.connect_gmail(&email, &token).await,
                                        _ => client.connect_outlook(&email, &token).await,
                                    }
                                    .map_err(|e| format!("IMAP connect failed: {}", e))?;
                                }
                                northmail_auth::GoaAuthType::Password => {
                                    let password = auth_manager
                                        .get_goa_password(&account_id)
                                        .await
                                        .map_err(|e| format!("Failed to get password: {}", e))?;

                                    let host = imap_host
                                        .as_deref()
                                        .unwrap_or("imap.mail.me.com");
                                    let username = imap_username
                                        .as_deref()
                                        .unwrap_or(&email);

                                    client
                                        .connect_login(host, 993, username, &password)
                                        .await
                                        .map_err(|e| format!("IMAP connect failed: {}", e))?;
                                }
                                northmail_auth::GoaAuthType::Unknown => {
                                    return Err("Unsupported auth type".to_string());
                                }
                            }

                            let uid = client
                                .append(&drafts_path, &["\\Draft", "\\Seen"], &message_bytes)
                                .await
                                .map_err(|e| format!("APPEND failed: {}", e))?;

                            let _ = client.logout().await;
                            Ok(uid)
                        }
                    }
                    .await;

                    let _ = sender.send(result);
                });
            });

            let result = loop {
                match receiver.try_recv() {
                    Ok(result) => break result,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        glib::timeout_future(std::time::Duration::from_millis(50)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        break Err("Draft save thread crashed".to_string());
                    }
                }
            };

            callback(result);
        });
    }

    /// Save a sent message to the Sent folder via IMAP APPEND
    /// (For non-Gmail providers that don't auto-save sent messages)
    async fn save_to_sent_folder(
        auth_manager: &AuthManager,
        account_id: &str,
        email: &str,
        auth_type: &northmail_auth::GoaAuthType,
        provider_type: &str,
        imap_host: Option<&str>,
        imap_username: Option<&str>,
        msg: &northmail_smtp::OutgoingMessage,
    ) -> Result<(), String> {
        // Build RFC 2822 message bytes
        let lettre_msg = northmail_smtp::build_lettre_message(msg)
            .map_err(|e| format!("Failed to build message: {}", e))?;
        let message_bytes = lettre_msg.formatted();

        // Connect to IMAP
        let mut client = SimpleImapClient::new();

        match auth_type {
            northmail_auth::GoaAuthType::OAuth2 => {
                let (_email, token) = auth_manager
                    .get_xoauth2_token_for_goa(account_id)
                    .await
                    .map_err(|e| format!("Failed to get token: {}", e))?;

                match provider_type {
                    "google" => client.connect_gmail(email, &token).await,
                    _ => client.connect_outlook(email, &token).await,
                }
                .map_err(|e| format!("IMAP connect failed: {}", e))?;
            }
            northmail_auth::GoaAuthType::Password => {
                let password = auth_manager
                    .get_goa_password(account_id)
                    .await
                    .map_err(|e| format!("Failed to get password: {}", e))?;

                // Use provided IMAP host or default based on provider
                let host = imap_host.unwrap_or("imap.mail.me.com");
                let username = imap_username.unwrap_or(email);

                client
                    .connect_login(host, 993, username, &password)
                    .await
                    .map_err(|e| format!("IMAP connect failed: {}", e))?;
            }
            northmail_auth::GoaAuthType::Unknown => {
                return Err("Unsupported auth type".to_string());
            }
        }

        // APPEND to Sent folder with \Seen flag
        // Try common Sent folder names
        let sent_folders = ["Sent", "Sent Messages", "Sent Items", "[Gmail]/Sent Mail"];
        let mut appended = false;

        for sent_folder in &sent_folders {
            match client.append(sent_folder, &["\\Seen"], &message_bytes).await {
                Ok(_) => {
                    appended = true;
                    break;
                }
                Err(_) => continue, // Try next folder name
            }
        }

        let _ = client.logout().await;

        if appended {
            Ok(())
        } else {
            Err("Could not find Sent folder".to_string())
        }
    }

    /// Delete a draft from the IMAP Drafts folder by UID
    pub fn delete_draft(
        &self,
        account_index: u32,
        draft_uid: u32,
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

        let db = match self.database_ref() {
            Some(db) => db.clone(),
            None => {
                callback(Err("Database not initialized".to_string()));
                return;
            }
        };

        let account_id = account.id.clone();
        let email = account.email.clone();
        let auth_type = account.auth_type.clone();
        let provider_type = account.provider_type.clone();
        let imap_host = account.imap_host.clone();
        let imap_username = account.imap_username.clone();
        let is_ms_graph = provider_type == "ms_graph";

        glib::spawn_future_local(async move {
            let (sender, receiver) = std::sync::mpsc::channel();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    let result = async {
                        let auth_manager = AuthManager::new()
                            .await
                            .map_err(|e| format!("Auth init failed: {}", e))?;

                        if is_ms_graph {
                            // Use Graph API to delete the draft
                            let token = auth_manager
                                .get_goa_token(&account_id)
                                .await
                                .map_err(|e| format!("Failed to get token: {}", e))?;

                            // Look up the Graph message ID from the UID hash
                            let graph_id = db
                                .get_graph_message_id_by_uid(draft_uid as i64)
                                .await
                                .map_err(|e| format!("DB error: {}", e))?
                                .ok_or_else(|| format!("No graph_message_id for uid {}", draft_uid))?;

                            let client = northmail_graph::GraphMailClient::new(token);
                            client.delete_message(&graph_id)
                                .await
                                .map_err(|e| format!("Graph delete draft failed: {}", e))?;

                            return Ok(());
                        }

                        let drafts_path = db
                            .get_drafts_folder(&account_id)
                            .await
                            .map_err(|e| format!("DB error: {}", e))?
                            .unwrap_or_else(|| "Drafts".to_string());

                        let mut client = SimpleImapClient::new();

                        match auth_type {
                            northmail_auth::GoaAuthType::OAuth2 => {
                                let (_email, token) = auth_manager
                                    .get_xoauth2_token_for_goa(&account_id)
                                    .await
                                    .map_err(|e| format!("Failed to get token: {}", e))?;

                                match provider_type.as_str() {
                                    "google" => client.connect_gmail(&email, &token).await,
                                    _ => client.connect_outlook(&email, &token).await,
                                }
                                .map_err(|e| format!("IMAP connect failed: {}", e))?;
                            }
                            northmail_auth::GoaAuthType::Password => {
                                let password = auth_manager
                                    .get_goa_password(&account_id)
                                    .await
                                    .map_err(|e| format!("Failed to get password: {}", e))?;

                                let host = imap_host
                                    .as_deref()
                                    .unwrap_or("imap.mail.me.com");
                                let username = imap_username
                                    .as_deref()
                                    .unwrap_or(&email);

                                client
                                    .connect_login(host, 993, username, &password)
                                    .await
                                    .map_err(|e| format!("IMAP connect failed: {}", e))?;
                            }
                            northmail_auth::GoaAuthType::Unknown => {
                                return Err("Unsupported auth type".to_string());
                            }
                        }

                        // SELECT the Drafts folder, then delete
                        client
                            .select(&drafts_path)
                            .await
                            .map_err(|e| format!("SELECT Drafts failed: {}", e))?;

                        client
                            .uid_store_deleted_and_expunge(draft_uid)
                            .await
                            .map_err(|e| format!("Delete draft failed: {}", e))?;

                        let _ = client.logout().await;
                        Ok(())
                    }
                    .await;

                    let _ = sender.send(result);
                });
            });

            let result = loop {
                match receiver.try_recv() {
                    Ok(result) => break result,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        glib::timeout_future(std::time::Duration::from_millis(50)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        break Err("Draft delete thread crashed".to_string());
                    }
                }
            };

            callback(result);
        });
    }

    /// Query EDS (Evolution Data Server) contacts matching a prefix
    /// Preload all contacts from EDS at startup (runs in background)
    pub fn preload_contacts(&self) {
        let app = self.clone();
        glib::spawn_future_local(async move {
            let (sender, receiver) = std::sync::mpsc::channel();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    let results = Self::eds_fetch_all_contacts().await;
                    let _ = sender.send(results);
                });
            });

            let results = loop {
                match receiver.try_recv() {
                    Ok(results) => break results,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        glib::timeout_future(std::time::Duration::from_millis(50)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        break Vec::new();
                    }
                }
            };

            debug!("EDS: preloaded {} contacts", results.len());
            *app.imp().contacts_cache.borrow_mut() = results;
        });
    }

    /// Query contacts by filtering the preloaded cache (instant, no D-Bus)
    pub fn query_contacts(
        &self,
        prefix: String,
        callback: impl FnOnce(Vec<(String, String)>) + 'static,
    ) {
        let prefix_lower = prefix.to_lowercase();
        let cache = self.imp().contacts_cache.borrow();
        let results: Vec<(String, String)> = cache
            .iter()
            .filter(|(name, email, _)| {
                name.to_lowercase().contains(&prefix_lower)
                    || email.to_lowercase().contains(&prefix_lower)
            })
            .map(|(name, email, _)| (name.clone(), email.clone()))
            .collect();
        callback(results);
    }

    /// Look up a contact photo by email address (case-insensitive)
    pub fn get_contact_photo(&self, email: &str) -> Option<Vec<u8>> {
        let email_lower = email.to_lowercase();
        let cache = self.imp().contacts_cache.borrow();
        cache
            .iter()
            .find(|(_, e, _)| e.to_lowercase() == email_lower)
            .and_then(|(_, _, photo)| photo.clone())
    }

    /// Fetch ALL contacts from EDS address books (called once at startup)
    async fn eds_fetch_all_contacts() -> Vec<(String, String, Option<Vec<u8>>)> {
        let mut results = Vec::new();

        let conn = match zbus::Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                debug!("EDS: session bus error: {}", e);
                return results;
            }
        };

        let (sources_bus, addressbook_bus) = match Self::eds_discover_services(&conn).await {
            Some(pair) => {
                debug!("EDS: services: {} / {}", pair.0, pair.1);
                pair
            }
            None => {
                debug!("EDS: services not found");
                return results;
            }
        };

        let source_uids = match Self::eds_get_address_book_uids(&conn, &sources_bus).await {
            Ok(uids) => {
                debug!("EDS: {} address books found", uids.len());
                uids
            }
            Err(e) => {
                debug!("EDS: get UIDs error: {}", e);
                return results;
            }
        };

        // Fetch all contacts from each address book (empty sexp = all)
        for uid in &source_uids {
            match Self::eds_query_address_book(&conn, &addressbook_bus, uid, "").await {
                Ok(contacts) => {
                    debug!("EDS: {} contacts from {}", contacts.len(), uid);
                    results.extend(contacts);
                }
                Err(e) => {
                    debug!("EDS: fetch error for {}: {}", uid, e);
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
    ) -> Result<Vec<(String, String, Option<Vec<u8>>)>, String> {
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

    /// Parse a vCard string to extract name, email, and optional photo
    fn parse_vcard_contacts(vcard: &str) -> Vec<(String, String, Option<Vec<u8>>)> {
        use base64::Engine;
        let mut name = String::new();
        let mut emails = Vec::new();
        let mut photo_b64 = String::new();
        let mut in_photo = false;

        for line in vcard.lines() {
            let line_trimmed = line.trim_end();
            // Continuation lines in vCards start with a space or tab
            if in_photo {
                if line.starts_with(' ') || line.starts_with('\t') {
                    photo_b64.push_str(line_trimmed.trim());
                    continue;
                } else {
                    in_photo = false;
                }
            }
            let lt = line_trimmed.trim();
            if lt.starts_with("FN:") || lt.starts_with("FN;") {
                if let Some(val) = lt.splitn(2, ':').nth(1) {
                    name = val.trim().to_string();
                }
            } else if lt.starts_with("EMAIL") {
                if let Some(val) = lt.splitn(2, ':').nth(1) {
                    let email = val.trim().to_string();
                    if !email.is_empty() {
                        emails.push(email);
                    }
                }
            } else if lt.starts_with("PHOTO") {
                // PHOTO;ENCODING=b;TYPE=JPEG:<base64> or PHOTO;VALUE=uri:data:...;base64,...
                // Use find(':') to get everything after first colon, preserving URIs with colons
                if let Some(colon_pos) = lt.find(':') {
                    let val = &lt[colon_pos + 1..];
                    photo_b64.clear();
                    photo_b64.push_str(val.trim());
                    in_photo = true;
                }
            }
        }

        if name.is_empty() {
            name = "Unknown".to_string();
        }

        let photo = if !photo_b64.is_empty() {
            if photo_b64.starts_with("file://") {
                // File URI — read photo from disk
                let path = photo_b64.strip_prefix("file://").unwrap();
                match std::fs::read(path) {
                    Ok(bytes) if !bytes.is_empty() => {
                        Some(bytes)
                    }
                    Ok(_) => None,
                    Err(e) => {
                        debug!("EDS: failed to read photo file for '{}': {}", name, e);
                        None
                    }
                }
            } else {
                // Inline base64 (or data URI with base64)
                let b64_data = if let Some(pos) = photo_b64.find("base64,") {
                    &photo_b64[pos + 7..]
                } else {
                    &photo_b64
                };
                match base64::engine::general_purpose::STANDARD.decode(b64_data) {
                    Ok(bytes) if !bytes.is_empty() => {
                        Some(bytes)
                    }
                    Ok(_) => None,
                    Err(e) => {
                        debug!("EDS: base64 decode failed for '{}': {}", name, e);
                        None
                    }
                }
            }
        } else {
            None
        };

        emails
            .into_iter()
            .map(|email| (name.clone(), email, photo.clone()))
            .collect()
    }

    /// Toggle the starred status of a message
    pub fn set_message_starred(&self, message_id: i64, uid: u32, folder_id: i64, is_starred: bool) {
        let db = match self.database() {
            Some(db) => db.clone(),
            None => {
                warn!("set_message_starred: No database");
                return;
            }
        };

        // Update database in a thread with tokio runtime
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                if let Err(e) = db.set_message_starred(message_id, is_starred).await {
                    error!("Failed to update starred status in database: {}", e);
                } else {
                    info!("Updated starred status for message {} to {}", uid, is_starred);
                }
            });
        });

        // Use passed folder_id if valid, otherwise fall back to current folder
        let effective_folder_id = if folder_id > 0 {
            folder_id
        } else {
            self.cache_folder_id()
        };

        // Sync to IMAP
        if effective_folder_id > 0 {
            self.sync_flag_to_imap(effective_folder_id, uid, "\\Flagged", is_starred);
        } else {
            warn!("set_message_starred: Invalid folder_id {}", effective_folder_id);
        }
    }

    /// Toggle the read status of a message
    pub fn set_message_read(&self, message_id: i64, uid: u32, folder_id: i64, is_read: bool) {
        let db = match self.database() {
            Some(db) => db.clone(),
            None => {
                warn!("set_message_read: No database");
                return;
            }
        };

        // Update database in a thread with tokio runtime
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                if let Err(e) = db.set_message_read(message_id, is_read).await {
                    error!("Failed to update read status in database: {}", e);
                } else {
                    info!("Updated read status for message {} to {}", uid, is_read);
                }
            });
        });

        // Use passed folder_id if valid, otherwise fall back to current folder
        let effective_folder_id = if folder_id > 0 {
            folder_id
        } else {
            self.cache_folder_id()
        };

        // Sync to IMAP
        if effective_folder_id > 0 {
            self.sync_flag_to_imap(effective_folder_id, uid, "\\Seen", is_read);
        } else {
            warn!("set_message_read: Invalid folder_id {}", effective_folder_id);
        }
    }

    /// Sync a flag change to IMAP server
    fn sync_flag_to_imap(&self, folder_id: i64, uid: u32, flag: &str, add: bool) {
        // Resolve folder info
        let (account_id, folder_path) = match self.resolve_folder_info(folder_id) {
            Some(info) => info,
            None => {
                warn!("sync_flag_to_imap: Could not resolve folder_id {}", folder_id);
                return;
            }
        };

        let accounts = self.imp().accounts.borrow().clone();
        let account = match accounts.iter().find(|a| a.id == account_id) {
            Some(a) => a.clone(),
            None => {
                warn!("sync_flag_to_imap: Account not found: {}", account_id);
                return;
            }
        };

        // ms_graph: sync flags via Graph API instead of IMAP
        if Self::is_ms_graph_account(&account) {
            let db = self.database().cloned();
            let flag = flag.to_string();
            let acct_id = account.id.clone();
            let folder_path_clone = folder_path.clone();
            glib::spawn_future_local(async move {
                let auth_manager = match AuthManager::new().await {
                    Ok(am) => am,
                    Err(e) => {
                        error!("sync_flag_to_imap (graph): Failed to create auth manager: {}", e);
                        return;
                    }
                };
                let access_token = match auth_manager.get_xoauth2_token_for_goa(&acct_id).await {
                    Ok((_email, token)) => token,
                    Err(e) => {
                        error!("sync_flag_to_imap (graph): Failed to get token: {}", e);
                        return;
                    }
                };

                // Look up graph_message_id
                let graph_msg_id = if let Some(ref db) = db {
                    Self::get_graph_message_id_for_uid(db, &acct_id, &folder_path_clone, uid).await
                } else {
                    None
                };

                let Some(graph_id) = graph_msg_id else {
                    error!("sync_flag_to_imap (graph): No graph_message_id for uid {}", uid);
                    return;
                };

                let (sender, receiver) = std::sync::mpsc::channel();
                let flag_for_log = flag.clone();
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    let result = rt.block_on(async {
                        let client = northmail_graph::GraphMailClient::new(access_token);
                        match flag.as_str() {
                            "\\Seen" => client.set_read(&graph_id, add).await,
                            "\\Flagged" => client.set_flagged(&graph_id, add).await,
                            _ => {
                                tracing::warn!("sync_flag_to_imap (graph): Unknown flag: {}", flag);
                                Ok(())
                            }
                        }
                    });
                    let _ = sender.send(result);
                });

                // Poll with timeout
                let start = std::time::Instant::now();
                loop {
                    match receiver.try_recv() {
                        Ok(Ok(())) => {
                            info!("sync_flag_to_imap (graph): Synced {} for uid {}", flag_for_log, uid);
                            break;
                        }
                        Ok(Err(e)) => {
                            error!("sync_flag_to_imap (graph): Graph API error: {}", e);
                            break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            if start.elapsed() > std::time::Duration::from_secs(10) {
                                error!("sync_flag_to_imap (graph): Timeout");
                                break;
                            }
                            glib::timeout_future(std::time::Duration::from_millis(50)).await;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    }
                }
            });
            return;
        }

        let pool = self.imap_pool();
        let is_google = Self::is_google_account(&account);
        let is_microsoft = Self::is_microsoft_account(&account);
        let flag = flag.to_string();
        let imap_host = account.imap_host.clone();
        let imap_username = account.imap_username.clone();

        glib::spawn_future_local(async move {
            // Get credentials via AuthManager
            let auth_manager = match AuthManager::new().await {
                Ok(am) => am,
                Err(e) => {
                    error!("sync_flag_to_imap: Failed to create auth manager: {}", e);
                    return;
                }
            };

            let credentials = if is_google {
                match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                    Ok((email, access_token)) => ImapCredentials::Gmail { email, access_token },
                    Err(e) => {
                        error!("sync_flag_to_imap: Failed to get Google token: {}", e);
                        return;
                    }
                }
            } else if is_microsoft {
                match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                    Ok((email, access_token)) => ImapCredentials::Microsoft { email, access_token },
                    Err(e) => {
                        error!("sync_flag_to_imap: Failed to get Microsoft token: {}", e);
                        return;
                    }
                }
            } else {
                // Password auth (e.g., iCloud)
                let host = imap_host.unwrap_or_else(|| "imap.mail.me.com".to_string());
                let username = imap_username.unwrap_or(account.email.clone());
                match auth_manager.get_goa_password(&account.id).await {
                    Ok(password) => ImapCredentials::Password {
                        host,
                        port: 993,
                        username,
                        password,
                    },
                    Err(e) => {
                        error!("sync_flag_to_imap: Failed to get password: {}", e);
                        return;
                    }
                }
            };

            // Send the flag change via pool
            let worker = match pool.get_or_create(credentials) {
                Ok(w) => w,
                Err(e) => {
                    error!("sync_flag_to_imap: Failed to get IMAP worker: {}", e);
                    return;
                }
            };

            let (response_tx, response_rx) = std::sync::mpsc::channel();
            let add_flags = if add { vec![flag.clone()] } else { vec![] };
            let remove_flags = if add { vec![] } else { vec![flag.clone()] };

            if let Err(e) = worker.send(ImapCommand::StoreFlags {
                folder: folder_path.clone(),
                uid,
                add_flags,
                remove_flags,
                response_tx,
            }) {
                error!("sync_flag_to_imap: Failed to send command: {}", e);
                return;
            }

            // Wait for response (with timeout)
            match response_rx.recv_timeout(std::time::Duration::from_secs(10)) {
                Ok(ImapResponse::Ok) => {
                    info!("sync_flag_to_imap: Successfully synced {} flag for uid {} in {}", flag, uid, folder_path);
                }
                Ok(ImapResponse::Error(e)) => {
                    error!("sync_flag_to_imap: IMAP error: {}", e);
                }
                Ok(_) => {
                    debug!("sync_flag_to_imap: Unexpected response");
                }
                Err(e) => {
                    error!("sync_flag_to_imap: Timeout or channel error: {}", e);
                }
            }
        });
    }

    /// Archive a message (move to Archive folder)
    pub fn archive_message(&self, _message_id: i64, uid: u32, folder_id: i64) {
        info!("archive_message: uid={}, folder_id={}", uid, folder_id);

        // Use passed folder_id if valid, otherwise fall back to current folder
        let effective_folder_id = if folder_id > 0 {
            folder_id
        } else {
            self.cache_folder_id()
        };

        if effective_folder_id <= 0 {
            warn!("archive_message: Invalid folder_id {}", effective_folder_id);
            return;
        }

        // Mark as pending delete to prevent re-insertion from sync/cache
        self.imp().pending_deletes.borrow_mut().insert((effective_folder_id, uid));

        // Resolve account and folder info
        let (account_id, source_folder) = match self.resolve_folder_info(effective_folder_id) {
            Some(info) => info,
            None => {
                warn!("archive_message: Could not resolve folder_id {}", effective_folder_id);
                return;
            }
        };

        // Delete from local database by folder_id + uid (reliable)
        if let Some(db) = self.database() {
            let db_clone = db.clone();
            let fid = effective_folder_id;
            let u = uid as i64;
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    if let Err(e) = db_clone.delete_message_by_uid(fid, u).await {
                        error!("archive_message: Failed to delete from database: {}", e);
                    }
                });
            });
        }

        // Move on IMAP
        self.move_message_imap(&account_id, &source_folder, uid, "Archive");
    }

    /// Move a message to spam folder
    pub fn move_to_spam(&self, _message_id: i64, uid: u32, folder_id: i64) {
        info!("move_to_spam: uid={}, folder_id={}", uid, folder_id);

        let effective_folder_id = if folder_id > 0 {
            folder_id
        } else {
            self.cache_folder_id()
        };

        if effective_folder_id <= 0 {
            warn!("move_to_spam: Invalid folder_id {}", effective_folder_id);
            return;
        }

        // Mark as pending delete to prevent re-insertion from sync/cache
        self.imp().pending_deletes.borrow_mut().insert((effective_folder_id, uid));

        let (account_id, source_folder) = match self.resolve_folder_info(effective_folder_id) {
            Some(info) => info,
            None => {
                warn!("move_to_spam: Could not resolve folder_id {}", effective_folder_id);
                return;
            }
        };

        // Delete from local database by folder_id + uid (reliable)
        if let Some(db) = self.database() {
            let db_clone = db.clone();
            let fid = effective_folder_id;
            let u = uid as i64;
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    if let Err(e) = db_clone.delete_message_by_uid(fid, u).await {
                        error!("move_to_spam: Failed to delete from database: {}", e);
                    }
                });
            });
        }

        // Move on IMAP
        self.move_message_imap(&account_id, &source_folder, uid, "Spam");
    }

    /// Delete a message (move to Trash folder)
    pub fn delete_message(&self, _message_id: i64, uid: u32, folder_id: i64) {
        info!("delete_message: uid={}, folder_id={}", uid, folder_id);

        // Use passed folder_id if valid, otherwise fall back to current folder
        let effective_folder_id = if folder_id > 0 {
            folder_id
        } else {
            self.cache_folder_id()
        };

        if effective_folder_id <= 0 {
            warn!("delete_message: Invalid folder_id {}", effective_folder_id);
            return;
        }

        // Mark as pending delete to prevent re-insertion from sync/cache
        self.imp().pending_deletes.borrow_mut().insert((effective_folder_id, uid));

        // Resolve account and folder info
        let (account_id, source_folder) = match self.resolve_folder_info(effective_folder_id) {
            Some(info) => info,
            None => {
                warn!("delete_message: Could not resolve folder_id {}", effective_folder_id);
                return;
            }
        };

        // Look up the actual trash folder for this account and perform delete
        let db = match self.database() {
            Some(db) => db.clone(),
            None => {
                warn!("delete_message: No database available");
                return;
            }
        };

        let app = self.clone();
        let account_id_clone = account_id.clone();
        let source_folder_clone = source_folder.clone();

        let fid = effective_folder_id;
        let u = uid as i64;

        // Check if this is an ms_graph account
        let is_ms_graph = {
            let accs = self.imp().accounts.borrow();
            accs.iter()
                .find(|a| a.id == account_id)
                .map(|a| Self::is_ms_graph_account(a))
                .unwrap_or(false)
        };

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                // For ms_graph: look up graph_message_id BEFORE deleting from DB
                let graph_id = if is_ms_graph {
                    db.get_graph_message_id(fid, u).await.ok().flatten()
                } else {
                    None
                };

                // Delete from local database by folder_id + uid
                if let Err(e) = db.delete_message_by_uid(fid, u).await {
                    error!("delete_message: Failed to delete from database: {}", e);
                }

                // Look up actual trash folder path from database
                let trash_folder = match db.get_trash_folder(&account_id_clone).await {
                    Ok(Some(path)) => {
                        info!("delete_message: Using trash folder from DB: {}", path);
                        path
                    }
                    Ok(None) => {
                        warn!("delete_message: No trash folder found for account {}, using fallback", account_id_clone);
                        "Trash".to_string()
                    }
                    Err(e) => {
                        warn!("delete_message: Failed to lookup trash folder: {}, using fallback", e);
                        "Trash".to_string()
                    }
                };

                let result: (String, Option<String>) = (trash_folder, graph_id);
                let _ = tx.send(result);
            });
        });

        // Wait for trash folder lookup and then move via IMAP or Graph
        glib::spawn_future_local(async move {
            let start = std::time::Instant::now();
            let timeout = std::time::Duration::from_secs(5);
            loop {
                match rx.try_recv() {
                    Ok((trash_folder, graph_id)) => {
                        if let Some(graph_id) = graph_id {
                            // ms_graph: move via Graph API directly
                            let acct_id = account_id.clone();
                            glib::spawn_future_local(async move {
                                let auth_manager = match AuthManager::new().await {
                                    Ok(am) => am,
                                    Err(e) => {
                                        error!("delete_message (graph): Auth failed: {}", e);
                                        return;
                                    }
                                };
                                let token = match auth_manager.get_goa_token(&acct_id).await {
                                    Ok(t) => t,
                                    Err(e) => {
                                        error!("delete_message (graph): Token failed: {}", e);
                                        return;
                                    }
                                };
                                let graph_dest = match trash_folder.as_str() {
                                    "Deleted Items" => "DeletedItems",
                                    "Trash" => "DeletedItems",
                                    other => other,
                                };
                                let (stx, srx) = std::sync::mpsc::channel();
                                let graph_dest_owned = graph_dest.to_string();
                                std::thread::spawn(move || {
                                    let rt = tokio::runtime::Runtime::new().unwrap();
                                    let result = rt.block_on(async {
                                        let client = northmail_graph::GraphMailClient::new(token);
                                        client.move_message(&graph_id, &graph_dest_owned).await
                                    });
                                    let _ = stx.send(result);
                                });
                                // Wait for result
                                let start = std::time::Instant::now();
                                loop {
                                    match srx.try_recv() {
                                        Ok(Ok(new_id)) => {
                                            info!("delete_message (graph): Moved to {}, new_id={}", trash_folder, new_id);
                                            break;
                                        }
                                        Ok(Err(e)) => {
                                            error!("delete_message (graph): Move failed: {}", e);
                                            break;
                                        }
                                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                                            if start.elapsed() > std::time::Duration::from_secs(10) {
                                                error!("delete_message (graph): Timeout");
                                                break;
                                            }
                                            glib::timeout_future(std::time::Duration::from_millis(50)).await;
                                        }
                                        Err(_) => break,
                                    }
                                }
                            });
                        } else {
                            // IMAP: use existing move path
                            app.move_message_imap(&account_id, &source_folder_clone, uid, &trash_folder);
                        }
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        if start.elapsed() > timeout {
                            error!("delete_message: Timeout waiting for trash folder lookup");
                            break;
                        }
                        glib::timeout_future(std::time::Duration::from_millis(50)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        error!("delete_message: Channel disconnected");
                        break;
                    }
                }
            }
        });
    }

    /// Move a message to a specific folder (drag-and-drop)
    /// Returns false if the move cannot be performed (e.g., cross-account move)
    pub fn move_message_to_folder(
        &self,
        message_id: i64,
        uid: u32,
        source_account_id: &str,
        source_folder_path: &str,
        target_account_id: &str,
        dest_folder_path: &str,
    ) -> bool {
        info!(
            "move_message_to_folder: uid={}, msg_id={}, from {}/{}, to {}/{}",
            uid, message_id, source_account_id, source_folder_path, target_account_id, dest_folder_path
        );

        // Check if source folder context is valid
        if source_account_id.is_empty() || source_folder_path.is_empty() {
            warn!("move_message_to_folder: Invalid source folder context (account='{}', folder='{}')",
                source_account_id, source_folder_path);
            return false;
        }

        // Check if source and target accounts match
        if source_account_id != target_account_id {
            warn!(
                "move_message_to_folder: Cross-account move not supported (from '{}' to '{}')",
                source_account_id, target_account_id
            );
            return false;
        }

        // Use cached folder_id (non-blocking) to mark pending delete immediately
        let cached_fid = self.cache_folder_id();
        if cached_fid > 0 {
            self.imp().pending_deletes.borrow_mut().insert((cached_fid, uid));
        } else {
            warn!("move_message_to_folder: No cached folder_id, pending delete not set for uid={}", uid);
        }

        // Delete from DB in background
        if let Some(db) = self.database() {
            let db_clone = db.clone();
            let aid = source_account_id.to_string();
            let fp = source_folder_path.to_string();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    if let Ok(folder_id) = db_clone.get_or_create_folder_id(&aid, &fp).await {
                        if let Err(e) = db_clone.delete_message_by_uid(folder_id, uid as i64).await {
                            error!("move_message_to_folder: Failed to delete from database: {}", e);
                        }
                    }
                });
            });
        }

        // Move on IMAP
        self.move_message_imap(source_account_id, source_folder_path, uid, dest_folder_path);
        true
    }

    /// Move a message to another folder on IMAP
    fn move_message_imap(&self, account_id: &str, source_folder: &str, uid: u32, dest_folder_hint: &str) {
        let account_id = account_id.to_string();
        let source_folder = source_folder.to_string();

        let accounts = self.imp().accounts.borrow().clone();
        let account = match accounts.iter().find(|a| a.id == account_id) {
            Some(a) => a.clone(),
            None => {
                warn!("move_message_imap: Account not found: {}", account_id);
                return;
            }
        };

        // ms_graph: move via Graph API
        if Self::is_ms_graph_account(&account) {
            let db = self.database().cloned();
            let dest_hint = dest_folder_hint.to_string();
            let acct_id = account_id.clone();
            let src_folder = source_folder.clone();
            glib::spawn_future_local(async move {
                let auth_manager = match AuthManager::new().await {
                    Ok(am) => am,
                    Err(e) => {
                        error!("move_message_imap (graph): Failed to create auth manager: {}", e);
                        return;
                    }
                };
                let access_token = match auth_manager.get_goa_token(&acct_id).await {
                    Ok(token) => token,
                    Err(e) => {
                        error!("move_message_imap (graph): Failed to get token: {}", e);
                        return;
                    }
                };

                // Look up graph_message_id
                let graph_msg_id = if let Some(ref db) = db {
                    Self::get_graph_message_id_for_uid(db, &acct_id, &src_folder, uid).await
                } else {
                    None
                };

                let Some(graph_id) = graph_msg_id else {
                    error!("move_message_imap (graph): No graph_message_id for uid {}", uid);
                    return;
                };

                // Map dest hint to Graph well-known folder IDs
                let graph_dest = match dest_hint.as_str() {
                    "Archive" => "Archive",
                    "Trash" => "DeletedItems",
                    "Spam" | "Junk Email" => "JunkEmail",
                    "Inbox" | "INBOX" => "Inbox",
                    _ => &dest_hint,
                };

                let (sender, receiver) = std::sync::mpsc::channel();
                let graph_dest_owned = graph_dest.to_string();
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    let result = rt.block_on(async {
                        let client = northmail_graph::GraphMailClient::new(access_token);
                        client.move_message(&graph_id, &graph_dest_owned).await
                            .map_err(|e| format!("Graph move failed: {}", e))
                    });
                    let _ = sender.send(result);
                });

                let start = std::time::Instant::now();
                loop {
                    match receiver.try_recv() {
                        Ok(Ok(new_id)) => {
                            info!("move_message_imap (graph): Moved uid {} to {}, new id={}", uid, graph_dest, new_id);
                            break;
                        }
                        Ok(Err(e)) => {
                            error!("move_message_imap (graph): {}", e);
                            break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            if start.elapsed() > std::time::Duration::from_secs(30) {
                                error!("move_message_imap (graph): Timeout");
                                break;
                            }
                            glib::timeout_future(std::time::Duration::from_millis(50)).await;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    }
                }
            });
            return;
        }

        let pool = self.imap_pool();
        let is_google = Self::is_google_account(&account);
        let is_microsoft = Self::is_microsoft_account(&account);
        let imap_host = account.imap_host.clone();
        let imap_username = account.imap_username.clone();

        // Determine destination folder based on provider
        let dest_folder = if is_google {
            match dest_folder_hint {
                "Archive" => "[Gmail]/All Mail".to_string(),
                "Trash" => "[Gmail]/Trash".to_string(),
                _ => dest_folder_hint.to_string(),
            }
        } else if is_microsoft {
            match dest_folder_hint {
                "Archive" => "Archive".to_string(),
                "Trash" => "Deleted".to_string(),
                _ => dest_folder_hint.to_string(),
            }
        } else {
            // Generic IMAP (iCloud, etc.)
            match dest_folder_hint {
                "Archive" => "Archive".to_string(),
                "Trash" => "Deleted Messages".to_string(),
                _ => dest_folder_hint.to_string(),
            }
        };

        glib::spawn_future_local(async move {
            // Get credentials via AuthManager
            let auth_manager = match AuthManager::new().await {
                Ok(am) => am,
                Err(e) => {
                    error!("move_message_imap: Failed to create auth manager: {}", e);
                    return;
                }
            };

            let credentials = if is_google {
                match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                    Ok((email, access_token)) => ImapCredentials::Gmail { email, access_token },
                    Err(e) => {
                        error!("move_message_imap: Failed to get Google token: {}", e);
                        return;
                    }
                }
            } else if is_microsoft {
                match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                    Ok((email, access_token)) => ImapCredentials::Microsoft { email, access_token },
                    Err(e) => {
                        error!("move_message_imap: Failed to get Microsoft token: {}", e);
                        return;
                    }
                }
            } else {
                let host = imap_host.unwrap_or_else(|| "imap.mail.me.com".to_string());
                let username = imap_username.unwrap_or(account.email.clone());
                match auth_manager.get_goa_password(&account.id).await {
                    Ok(password) => ImapCredentials::Password {
                        host,
                        port: 993,
                        username,
                        password,
                    },
                    Err(e) => {
                        error!("move_message_imap: Failed to get password: {}", e);
                        return;
                    }
                }
            };

            let worker = match pool.get_or_create(credentials) {
                Ok(w) => w,
                Err(e) => {
                    error!("move_message_imap: Failed to get IMAP worker: {}", e);
                    return;
                }
            };

            let (response_tx, response_rx) = std::sync::mpsc::channel();

            if let Err(e) = worker.send(ImapCommand::MoveMessage {
                source_folder: source_folder.clone(),
                dest_folder: dest_folder.clone(),
                uid,
                response_tx,
            }) {
                error!("move_message_imap: Failed to send command: {}", e);
                return;
            }

            // Non-blocking poll with yield to GTK main loop
            let timeout = std::time::Duration::from_secs(30);
            let start = std::time::Instant::now();
            loop {
                match response_rx.try_recv() {
                    Ok(ImapResponse::Ok) => {
                        info!("move_message_imap: Successfully moved uid {} from {} to {}", uid, source_folder, dest_folder);
                        break;
                    }
                    Ok(ImapResponse::Error(e)) => {
                        error!("move_message_imap: IMAP error: {}", e);
                        break;
                    }
                    Ok(_) => {
                        debug!("move_message_imap: Unexpected response");
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        if start.elapsed() > timeout {
                            error!("move_message_imap: Timeout waiting for response");
                            break;
                        }
                        glib::timeout_future(std::time::Duration::from_millis(50)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        error!("move_message_imap: Channel disconnected");
                        break;
                    }
                }
            }
        });
    }

    /// Move a message to another folder on IMAP using exact folder path (no translation)
    fn move_message_imap_direct(&self, account_id: &str, source_folder: &str, uid: u32, dest_folder: &str) {
        let account_id = account_id.to_string();
        let source_folder = source_folder.to_string();
        let dest_folder = dest_folder.to_string();

        info!("move_message_imap_direct: uid={} from {} to {}", uid, source_folder, dest_folder);

        let accounts = self.imp().accounts.borrow().clone();
        let account = match accounts.iter().find(|a| a.id == account_id) {
            Some(a) => a.clone(),
            None => {
                warn!("move_message_imap_direct: Account not found: {}", account_id);
                return;
            }
        };

        let pool = self.imap_pool();
        let is_google = Self::is_google_account(&account);
        let is_microsoft = Self::is_microsoft_account(&account);
        let imap_host = account.imap_host.clone();
        let imap_username = account.imap_username.clone();

        glib::spawn_future_local(async move {
            // Get credentials via AuthManager
            let auth_manager = match AuthManager::new().await {
                Ok(am) => am,
                Err(e) => {
                    error!("move_message_imap_direct: Failed to create auth manager: {}", e);
                    return;
                }
            };

            let credentials = if is_google {
                match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                    Ok((email, access_token)) => ImapCredentials::Gmail { email, access_token },
                    Err(e) => {
                        error!("move_message_imap_direct: Failed to get Google token: {}", e);
                        return;
                    }
                }
            } else if is_microsoft {
                match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                    Ok((email, access_token)) => ImapCredentials::Microsoft { email, access_token },
                    Err(e) => {
                        error!("move_message_imap_direct: Failed to get Microsoft token: {}", e);
                        return;
                    }
                }
            } else {
                let host = imap_host.unwrap_or_else(|| "imap.mail.me.com".to_string());
                let username = imap_username.unwrap_or(account.email.clone());
                match auth_manager.get_goa_password(&account.id).await {
                    Ok(password) => ImapCredentials::Password {
                        host,
                        port: 993,
                        username,
                        password,
                    },
                    Err(e) => {
                        error!("move_message_imap_direct: Failed to get password: {}", e);
                        return;
                    }
                }
            };

            let worker = match pool.get_or_create(credentials) {
                Ok(w) => w,
                Err(e) => {
                    error!("move_message_imap_direct: Failed to get IMAP worker: {}", e);
                    return;
                }
            };

            let (response_tx, response_rx) = std::sync::mpsc::channel();

            if let Err(e) = worker.send(ImapCommand::MoveMessage {
                source_folder: source_folder.clone(),
                dest_folder: dest_folder.clone(),
                uid,
                response_tx,
            }) {
                error!("move_message_imap_direct: Failed to send command: {}", e);
                return;
            }

            // Non-blocking poll with yield to GTK main loop
            let timeout = std::time::Duration::from_secs(30);
            let start = std::time::Instant::now();
            loop {
                match response_rx.try_recv() {
                    Ok(ImapResponse::Ok) => {
                        info!("move_message_imap_direct: Successfully moved uid {} from {} to {}", uid, source_folder, dest_folder);
                        break;
                    }
                    Ok(ImapResponse::Error(e)) => {
                        error!("move_message_imap_direct: IMAP error: {}", e);
                        break;
                    }
                    Ok(_) => {
                        debug!("move_message_imap_direct: Unexpected response");
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        if start.elapsed() > timeout {
                            error!("move_message_imap_direct: Timeout waiting for response");
                            break;
                        }
                        glib::timeout_future(std::time::Duration::from_millis(50)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        error!("move_message_imap_direct: Channel disconnected");
                        break;
                    }
                }
            }
        });
    }
}

impl Default for NorthMailApplication {
    fn default() -> Self {
        Self::new()
    }
}
