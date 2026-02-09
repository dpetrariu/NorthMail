//! Sync engine for email synchronization

use crate::{Account, CoreError, CoreResult, Database};
use northmail_auth::AuthManager;
use northmail_imap::ImapClient;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

/// Commands sent from UI to sync engine
#[derive(Debug, Clone)]
pub enum SyncCommand {
    /// Sync all folders for an account
    SyncAccount { account_id: String },
    /// Sync a specific folder
    SyncFolder {
        account_id: String,
        folder_path: String,
    },
    /// Fetch full message body
    FetchMessage {
        account_id: String,
        folder_path: String,
        uid: u32,
    },
    /// Mark message as read/unread
    SetRead {
        account_id: String,
        folder_path: String,
        uid: u32,
        is_read: bool,
    },
    /// Move message to another folder
    MoveMessage {
        account_id: String,
        from_folder: String,
        to_folder: String,
        uid: u32,
    },
    /// Stop the sync engine
    Shutdown,
}

/// Events sent from sync engine to UI
#[derive(Debug, Clone)]
pub enum SyncEvent {
    /// Sync started for an account
    SyncStarted { account_id: String },
    /// Sync completed for an account
    SyncCompleted { account_id: String },
    /// Sync failed for an account
    SyncFailed { account_id: String, error: String },
    /// Folder list updated
    FoldersUpdated { account_id: String },
    /// Messages updated for a folder
    MessagesUpdated {
        account_id: String,
        folder_path: String,
    },
    /// Message body fetched
    MessageFetched {
        account_id: String,
        folder_path: String,
        uid: u32,
        body: Vec<u8>,
    },
    /// Unread count changed
    UnreadCountChanged {
        account_id: String,
        folder_path: String,
        count: u32,
    },
    /// Error occurred
    Error { message: String },
}

/// Sync engine that runs in a background tokio task
pub struct SyncEngine {
    database: Arc<Database>,
    auth_manager: Arc<AuthManager>,
    command_rx: mpsc::Receiver<SyncCommand>,
    event_tx: mpsc::Sender<SyncEvent>,
}

impl SyncEngine {
    /// Create a new sync engine
    pub fn new(
        database: Arc<Database>,
        auth_manager: Arc<AuthManager>,
        command_rx: mpsc::Receiver<SyncCommand>,
        event_tx: mpsc::Sender<SyncEvent>,
    ) -> Self {
        Self {
            database,
            auth_manager,
            command_rx,
            event_tx,
        }
    }

    /// Run the sync engine
    pub async fn run(mut self) {
        info!("Sync engine started");

        while let Some(command) = self.command_rx.recv().await {
            match command {
                SyncCommand::Shutdown => {
                    info!("Sync engine shutting down");
                    break;
                }
                cmd => {
                    if let Err(e) = self.handle_command(cmd).await {
                        error!("Error handling sync command: {}", e);
                        let _ = self
                            .event_tx
                            .send(SyncEvent::Error {
                                message: e.to_string(),
                            })
                            .await;
                    }
                }
            }
        }

        info!("Sync engine stopped");
    }

    /// Handle a sync command
    async fn handle_command(&mut self, command: SyncCommand) -> CoreResult<()> {
        match command {
            SyncCommand::SyncAccount { account_id } => {
                self.sync_account(&account_id).await?;
            }
            SyncCommand::SyncFolder {
                account_id,
                folder_path,
            } => {
                self.sync_folder(&account_id, &folder_path).await?;
            }
            SyncCommand::FetchMessage {
                account_id,
                folder_path,
                uid,
            } => {
                self.fetch_message(&account_id, &folder_path, uid).await?;
            }
            SyncCommand::SetRead {
                account_id,
                folder_path,
                uid,
                is_read,
            } => {
                self.set_read(&account_id, &folder_path, uid, is_read)
                    .await?;
            }
            SyncCommand::MoveMessage {
                account_id,
                from_folder,
                to_folder,
                uid,
            } => {
                self.move_message(&account_id, &from_folder, &to_folder, uid)
                    .await?;
            }
            SyncCommand::Shutdown => unreachable!(),
        }

        Ok(())
    }

    /// Get an authenticated IMAP client for an account
    async fn get_imap_client(&self, account: &Account) -> CoreResult<ImapClient> {
        let token = self
            .auth_manager
            .get_xoauth2_token(&account.auth_method)
            .await?;

        let mut client = ImapClient::new(&account.config.imap_host, account.config.imap_port);
        client
            .authenticate_xoauth2(token.email(), token.access_token())
            .await?;

        Ok(client)
    }

    /// Sync all folders for an account
    async fn sync_account(&mut self, account_id: &str) -> CoreResult<()> {
        info!("Syncing account {}", account_id);

        let _ = self
            .event_tx
            .send(SyncEvent::SyncStarted {
                account_id: account_id.to_string(),
            })
            .await;

        // Get account from database
        let accounts = self.database.get_accounts().await?;
        let account = accounts
            .iter()
            .find(|a| a.id == account_id)
            .ok_or_else(|| CoreError::AccountNotFound(account_id.to_string()))?;

        // Connect to IMAP
        let mut client = match self.get_imap_client(account).await {
            Ok(c) => c,
            Err(e) => {
                let _ = self
                    .event_tx
                    .send(SyncEvent::SyncFailed {
                        account_id: account_id.to_string(),
                        error: e.to_string(),
                    })
                    .await;
                return Err(e);
            }
        };

        // List and sync folders
        let folders = client.list_folders().await?;
        for folder in &folders {
            if !folder.is_selectable() {
                continue;
            }

            let folder_type = format!("{:?}", folder.folder_type).to_lowercase();
            self.database
                .upsert_folder(account_id, &folder.name, &folder.full_path, &folder_type)
                .await?;
        }

        let _ = self
            .event_tx
            .send(SyncEvent::FoldersUpdated {
                account_id: account_id.to_string(),
            })
            .await;

        // Sync inbox first (most important)
        for folder in &folders {
            if folder.folder_type == northmail_imap::FolderType::Inbox {
                self.sync_folder_internal(&mut client, account_id, &folder.full_path)
                    .await?;
                break;
            }
        }

        client.logout().await?;

        let _ = self
            .event_tx
            .send(SyncEvent::SyncCompleted {
                account_id: account_id.to_string(),
            })
            .await;

        info!("Account sync completed: {}", account_id);
        Ok(())
    }

    /// Sync a specific folder
    async fn sync_folder(&mut self, account_id: &str, folder_path: &str) -> CoreResult<()> {
        let accounts = self.database.get_accounts().await?;
        let account = accounts
            .iter()
            .find(|a| a.id == account_id)
            .ok_or_else(|| CoreError::AccountNotFound(account_id.to_string()))?;

        let mut client = self.get_imap_client(account).await?;
        self.sync_folder_internal(&mut client, account_id, folder_path)
            .await?;
        client.logout().await?;

        Ok(())
    }

    /// Internal folder sync with an existing client
    async fn sync_folder_internal(
        &mut self,
        client: &mut ImapClient,
        account_id: &str,
        folder_path: &str,
    ) -> CoreResult<()> {
        debug!("Syncing folder: {}", folder_path);

        // Select the folder
        let folder_info = client.select_folder(folder_path).await?;

        // Get folder ID from database
        let folders = self.database.get_folders(account_id).await?;
        let db_folder = folders
            .iter()
            .find(|f| f.full_path == folder_path)
            .ok_or_else(|| CoreError::FolderNotFound(folder_path.to_string()))?;

        // Check UIDVALIDITY - if changed, we need to re-sync everything
        let uidvalidity = folder_info.uidvalidity.unwrap_or(0) as i64;
        let needs_full_sync = db_folder.uidvalidity != Some(uidvalidity);

        if needs_full_sync {
            info!(
                "UIDVALIDITY changed for {}, performing full sync",
                folder_path
            );
        }

        // Fetch message headers
        let message_count = folder_info.message_count.unwrap_or(0);
        if message_count > 0 {
            // Fetch last 100 messages for now (TODO: pagination)
            let start = if message_count > 100 {
                message_count - 100
            } else {
                1
            };
            let uid_range = format!("{}:*", start);

            let headers = client.fetch_headers(&uid_range).await?;
            let mut unread_count = 0;

            for header in &headers {
                if !header.is_read() {
                    unread_count += 1;
                }

                let db_msg = crate::database::DbMessage {
                    id: 0, // Will be assigned by database
                    folder_id: db_folder.id,
                    uid: header.uid as i64,
                    message_id: header.envelope.message_id.clone(),
                    subject: header.envelope.subject.clone(),
                    from_address: header.envelope.from.first().map(|a| a.address.clone()),
                    from_name: header.envelope.from.first().and_then(|a| a.name.clone()),
                    to_addresses: Some(
                        header
                            .envelope
                            .to
                            .iter()
                            .map(|a| a.address.clone())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    date_sent: header.envelope.date.clone(),
                    date_epoch: header.envelope.date.as_deref().and_then(|d| {
                        let mut s = d.to_string();
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
                    }),
                    snippet: None, // Would need BODY[TEXT] for snippet
                    is_read: header.is_read(),
                    is_starred: header.is_starred(),
                    has_attachments: header.has_attachments,
                    size: header.size as i64,
                    maildir_path: None,
                    body_text: None,
                    body_html: None,
                };

                self.database.upsert_message(db_folder.id, &db_msg).await?;
            }

            // Update folder sync state
            self.database
                .update_folder_sync(
                    db_folder.id,
                    uidvalidity,
                    folder_info.uid_next.unwrap_or(0) as i64,
                    message_count as i64,
                    unread_count,
                )
                .await?;

            let _ = self
                .event_tx
                .send(SyncEvent::UnreadCountChanged {
                    account_id: account_id.to_string(),
                    folder_path: folder_path.to_string(),
                    count: unread_count as u32,
                })
                .await;
        }

        let _ = self
            .event_tx
            .send(SyncEvent::MessagesUpdated {
                account_id: account_id.to_string(),
                folder_path: folder_path.to_string(),
            })
            .await;

        debug!("Folder sync completed: {}", folder_path);
        Ok(())
    }

    /// Fetch a full message body
    async fn fetch_message(
        &mut self,
        account_id: &str,
        folder_path: &str,
        uid: u32,
    ) -> CoreResult<()> {
        let accounts = self.database.get_accounts().await?;
        let account = accounts
            .iter()
            .find(|a| a.id == account_id)
            .ok_or_else(|| CoreError::AccountNotFound(account_id.to_string()))?;

        let mut client = self.get_imap_client(account).await?;
        client.select_folder(folder_path).await?;

        let body = client.fetch_body(uid).await?;
        client.logout().await?;

        let _ = self
            .event_tx
            .send(SyncEvent::MessageFetched {
                account_id: account_id.to_string(),
                folder_path: folder_path.to_string(),
                uid,
                body,
            })
            .await;

        Ok(())
    }

    /// Set message read status
    async fn set_read(
        &mut self,
        account_id: &str,
        folder_path: &str,
        uid: u32,
        is_read: bool,
    ) -> CoreResult<()> {
        let accounts = self.database.get_accounts().await?;
        let account = accounts
            .iter()
            .find(|a| a.id == account_id)
            .ok_or_else(|| CoreError::AccountNotFound(account_id.to_string()))?;

        let mut client = self.get_imap_client(account).await?;
        client.select_folder(folder_path).await?;

        if is_read {
            client.mark_read(uid).await?;
        } else {
            client.mark_unread(uid).await?;
        }

        client.logout().await?;

        Ok(())
    }

    /// Move a message to another folder
    async fn move_message(
        &mut self,
        account_id: &str,
        from_folder: &str,
        to_folder: &str,
        uid: u32,
    ) -> CoreResult<()> {
        let accounts = self.database.get_accounts().await?;
        let account = accounts
            .iter()
            .find(|a| a.id == account_id)
            .ok_or_else(|| CoreError::AccountNotFound(account_id.to_string()))?;

        let mut client = self.get_imap_client(account).await?;
        client.select_folder(from_folder).await?;
        client.move_message(uid, to_folder).await?;
        client.logout().await?;

        Ok(())
    }
}

/// Create sync engine channels
/// Returns (command_sender, command_receiver, event_sender, event_receiver)
#[allow(dead_code)]
pub fn create_sync_channels() -> (
    mpsc::Sender<SyncCommand>,
    mpsc::Receiver<SyncCommand>,
    mpsc::Sender<SyncEvent>,
    mpsc::Receiver<SyncEvent>,
) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<SyncCommand>(100);
    let (evt_tx, evt_rx) = mpsc::channel::<SyncEvent>(100);
    (cmd_tx, cmd_rx, evt_tx, evt_rx)
}
