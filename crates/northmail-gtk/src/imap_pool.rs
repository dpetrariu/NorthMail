//! IMAP Connection Pool
//!
//! Maintains persistent IMAP connections per account to avoid repeated
//! connection/authentication overhead.

use northmail_imap::SimpleImapClient;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// Commands that can be sent to an IMAP worker
#[derive(Debug)]
pub enum ImapCommand {
    /// Select a folder and fetch headers
    FetchHeaders {
        folder: String,
        /// Sequence range like "1:50" or "*:*" for last N
        range: String,
        response_tx: mpsc::Sender<ImapResponse>,
    },
    /// Fetch a message body
    FetchBody {
        folder: String,
        uid: u32,
        response_tx: mpsc::Sender<ImapResponse>,
    },
    /// Check connection health
    Noop {
        response_tx: mpsc::Sender<ImapResponse>,
    },
    /// Shutdown the worker
    Shutdown,
}

/// Responses from the IMAP worker
#[derive(Debug)]
pub enum ImapResponse {
    /// Folder info after select
    FolderInfo {
        message_count: u32,
        uid_next: Option<u32>,
        uidvalidity: Option<u32>,
    },
    /// Message headers
    Headers(Vec<northmail_imap::MessageHeader>),
    /// Message body (raw)
    Body(String),
    /// Operation completed successfully
    Ok,
    /// Error occurred
    Error(String),
}

/// Credentials for connecting to an IMAP server
#[derive(Clone, Debug)]
pub enum ImapCredentials {
    /// OAuth2 for Gmail
    Gmail {
        email: String,
        access_token: String,
    },
    /// OAuth2 for Microsoft
    Microsoft {
        email: String,
        access_token: String,
    },
    /// Traditional password auth
    Password {
        host: String,
        port: u16,
        username: String,
        password: String,
    },
}

impl ImapCredentials {
    /// Get a key for this credential (for pooling)
    pub fn pool_key(&self) -> String {
        match self {
            ImapCredentials::Gmail { email, .. } => format!("gmail:{}", email),
            ImapCredentials::Microsoft { email, .. } => format!("microsoft:{}", email),
            ImapCredentials::Password { host, username, .. } => {
                format!("password:{}@{}", username, host)
            }
        }
    }
}

/// Handle to communicate with an IMAP worker
pub struct ImapWorkerHandle {
    command_tx: mpsc::Sender<ImapCommand>,
    last_used: Instant,
}

impl ImapWorkerHandle {
    /// Send a command to the worker
    pub fn send(&self, command: ImapCommand) -> Result<(), String> {
        self.command_tx
            .send(command)
            .map_err(|e| format!("Failed to send command: {}", e))
    }

    /// Update last used timestamp
    pub fn touch(&mut self) {
        self.last_used = Instant::now();
    }

    /// Check if the connection is stale (unused for too long)
    pub fn is_stale(&self, timeout: Duration) -> bool {
        self.last_used.elapsed() > timeout
    }
}

/// IMAP Connection Pool
pub struct ImapPool {
    workers: Mutex<HashMap<String, ImapWorkerHandle>>,
    /// How long to keep idle connections
    idle_timeout: Duration,
}

impl ImapPool {
    /// Create a new connection pool
    pub fn new() -> Self {
        Self {
            workers: Mutex::new(HashMap::new()),
            idle_timeout: Duration::from_secs(300), // 5 minutes
        }
    }

    /// Remove a dead worker so the next call to get_or_create reconnects
    pub fn remove_worker(&self, credentials: &ImapCredentials) {
        let key = credentials.pool_key();
        let mut workers = self.workers.lock().unwrap();
        if workers.remove(&key).is_some() {
            info!("Removed dead worker for {}", key);
        }
    }

    /// Get or create a worker for the given credentials
    pub fn get_or_create(&self, credentials: ImapCredentials) -> Result<mpsc::Sender<ImapCommand>, String> {
        let key = credentials.pool_key();
        let mut workers = self.workers.lock().unwrap();

        // Check if we have an existing worker
        if let Some(handle) = workers.get_mut(&key) {
            // Just check if the channel is still connected by trying to clone the sender
            // If the worker has died, the receiver is dropped and this will work,
            // but the next actual command will fail and we'll detect it then
            if !handle.is_stale(self.idle_timeout) {
                debug!("♻️ Reusing existing IMAP connection for {}", key);
                handle.touch();
                return Ok(handle.command_tx.clone());
            } else {
                debug!("Connection is stale, creating new one");
                // Send shutdown to old worker if still alive
                let _ = handle.send(ImapCommand::Shutdown);
                workers.remove(&key);
            }
        }

        // Create new worker
        info!("🔌 Creating new IMAP connection for {}", key);
        let (command_tx, command_rx) = mpsc::channel();

        // Spawn worker thread - it will connect and then start processing commands
        // Commands sent before connection completes will queue up in the channel
        let creds = credentials.clone();
        std::thread::spawn(move || {
            Self::run_worker(creds, command_rx);
        });

        // Store handle immediately - the worker will start processing once connected
        let handle = ImapWorkerHandle {
            command_tx: command_tx.clone(),
            last_used: Instant::now(),
        };
        workers.insert(key, handle);

        Ok(command_tx)
    }

    /// Run the IMAP worker in a dedicated thread
    fn run_worker(credentials: ImapCredentials, command_rx: mpsc::Receiver<ImapCommand>) {
        info!("IMAP worker thread started for {}", credentials.pool_key());

        async_std::task::block_on(async {
            let mut client = SimpleImapClient::new();

            info!("IMAP worker connecting...");

            // Connect based on credentials
            let connect_result = match &credentials {
                ImapCredentials::Gmail { email, access_token } => {
                    client.connect_gmail(email, access_token).await
                }
                ImapCredentials::Microsoft { email, access_token } => {
                    client.connect_outlook(email, access_token).await
                }
                ImapCredentials::Password {
                    host,
                    port,
                    username,
                    password,
                } => {
                    client.connect_login(host, *port, username, password).await
                }
            };

            if let Err(e) = connect_result {
                error!("IMAP worker failed to connect: {}", e);
                // Drain any pending commands with error responses
                while let Ok(cmd) = command_rx.try_recv() {
                    Self::send_error_response(&cmd, &format!("Connection failed: {}", e));
                }
                return;
            }

            info!("IMAP worker connected for {}", credentials.pool_key());

            // Track currently selected folder to avoid redundant SELECTs
            let mut current_folder: Option<String> = None;

            // Process commands
            loop {
                match command_rx.recv_timeout(Duration::from_secs(60)) {
                    Ok(command) => {
                        match command {
                            ImapCommand::Shutdown => {
                                debug!("IMAP worker shutting down");
                                let _ = client.logout().await;
                                return;
                            }
                            ImapCommand::Noop { response_tx } => {
                                match client.noop().await {
                                    Ok(_) => {
                                        let _ = response_tx.send(ImapResponse::Ok);
                                    }
                                    Err(e) => {
                                        let _ = response_tx.send(ImapResponse::Error(e.to_string()));
                                        // Connection is dead, exit
                                        return;
                                    }
                                }
                            }
                            ImapCommand::FetchHeaders {
                                folder,
                                range,
                                response_tx,
                            } => {
                                Self::handle_fetch_headers(
                                    &mut client,
                                    &folder,
                                    &range,
                                    &response_tx,
                                )
                                .await;
                                current_folder = Some(folder);
                            }
                            ImapCommand::FetchBody {
                                folder,
                                uid,
                                response_tx,
                            } => {
                                Self::handle_fetch_body(&mut client, &folder, uid, &response_tx, &mut current_folder)
                                    .await;
                            }
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // Send NOOP to keep connection alive
                        if let Err(e) = client.noop().await {
                            warn!("NOOP failed, connection may be dead: {}", e);
                            return;
                        }
                        debug!("IMAP keepalive NOOP sent");
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        debug!("IMAP worker channel closed, shutting down");
                        let _ = client.logout().await;
                        return;
                    }
                }
            }
        });
    }

    /// Handle FetchHeaders command
    async fn handle_fetch_headers(
        client: &mut SimpleImapClient,
        folder: &str,
        range: &str,
        response_tx: &mpsc::Sender<ImapResponse>,
    ) {
        // Select folder
        match client.select(folder).await {
            Ok(info) => {
                let _ = response_tx.send(ImapResponse::FolderInfo {
                    message_count: info.message_count.unwrap_or(0),
                    uid_next: info.uid_next,
                    uidvalidity: info.uidvalidity,
                });

                // Fetch headers
                match client.fetch_headers(range).await {
                    Ok(headers) => {
                        let _ = response_tx.send(ImapResponse::Headers(headers));
                    }
                    Err(e) => {
                        let _ = response_tx.send(ImapResponse::Error(format!(
                            "Failed to fetch headers: {}",
                            e
                        )));
                    }
                }
            }
            Err(e) => {
                let _ = response_tx.send(ImapResponse::Error(format!(
                    "Failed to select folder: {}",
                    e
                )));
            }
        }
    }

    /// Handle FetchBody command (with folder tracking to avoid redundant SELECTs)
    async fn handle_fetch_body(
        client: &mut SimpleImapClient,
        folder: &str,
        uid: u32,
        response_tx: &mpsc::Sender<ImapResponse>,
        current_folder: &mut Option<String>,
    ) {
        // Only SELECT if folder changed (like Geary's approach)
        if current_folder.as_deref() != Some(folder) {
            debug!("handle_fetch_body: selecting folder {} (was {:?})", folder, current_folder);
            match client.select(folder).await {
                Ok(info) => {
                    debug!("handle_fetch_body: selected folder, {} messages",
                           info.message_count.unwrap_or(0));
                    *current_folder = Some(folder.to_string());
                }
                Err(e) => {
                    error!("handle_fetch_body: failed to select folder: {}", e);
                    *current_folder = None;
                    let _ = response_tx.send(ImapResponse::Error(format!(
                        "Failed to select folder: {}",
                        e
                    )));
                    return;
                }
            }
        } else {
            debug!("handle_fetch_body: folder {} already selected", folder);
        }

        debug!("handle_fetch_body: fetching body for uid {}", uid);

        // Fetch body
        match client.fetch_body(uid).await {
            Ok(body) => {
                debug!("handle_fetch_body: got body, {} bytes", body.len());
                let _ = response_tx.send(ImapResponse::Body(body));
            }
            Err(e) => {
                error!("handle_fetch_body: failed to fetch body: {}", e);
                let _ = response_tx.send(ImapResponse::Error(format!(
                    "Failed to fetch body: {}",
                    e
                )));
            }
        }
    }

    /// Send an error response for a command
    fn send_error_response(cmd: &ImapCommand, error: &str) {
        match cmd {
            ImapCommand::FetchHeaders { response_tx, .. } => {
                let _ = response_tx.send(ImapResponse::Error(error.to_string()));
            }
            ImapCommand::FetchBody { response_tx, .. } => {
                let _ = response_tx.send(ImapResponse::Error(error.to_string()));
            }
            ImapCommand::Noop { response_tx } => {
                let _ = response_tx.send(ImapResponse::Error(error.to_string()));
            }
            ImapCommand::Shutdown => {}
        }
    }

    /// Clean up stale connections
    #[allow(dead_code)]
    pub fn cleanup_stale(&self) {
        let mut workers = self.workers.lock().unwrap();
        let stale_keys: Vec<_> = workers
            .iter()
            .filter(|(_, h)| h.is_stale(self.idle_timeout))
            .map(|(k, _)| k.clone())
            .collect();

        for key in stale_keys {
            if let Some(handle) = workers.remove(&key) {
                info!("Removing stale IMAP connection: {}", key);
                let _ = handle.send(ImapCommand::Shutdown);
            }
        }
    }

    /// Shutdown all workers
    pub fn shutdown(&self) {
        let mut workers = self.workers.lock().unwrap();
        for (key, handle) in workers.drain() {
            info!("Shutting down IMAP worker: {}", key);
            let _ = handle.send(ImapCommand::Shutdown);
        }
    }
}

impl Default for ImapPool {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ImapPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}
