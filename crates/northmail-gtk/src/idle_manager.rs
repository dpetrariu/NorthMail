//! IMAP IDLE manager for real-time push notifications
//!
//! Manages persistent IDLE connections for each email account,
//! detecting new mail in real-time and sending events to the main application.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use northmail_imap::{IdleEvent, SimpleImapClient};
use tracing::{debug, error, info, warn};

/// Events sent from the IDLE manager to the application
#[derive(Debug, Clone)]
pub enum IdleManagerEvent {
    /// New mail detected for an account
    NewMail { account_id: String },
    /// Connection was lost for an account (will auto-reconnect)
    ConnectionLost { account_id: String },
    /// IDLE is not supported by this server
    NotSupported { account_id: String },
}

/// Credentials for connecting to an IMAP server
#[derive(Clone)]
pub struct IdleCredentials {
    pub account_id: String,
    pub email: String,
    pub auth_type: IdleAuthType,
}

/// Authentication type for IDLE connection
#[derive(Clone)]
pub enum IdleAuthType {
    /// OAuth2 (Gmail, Outlook)
    OAuth2 {
        host: String,
        access_token: String,
    },
    /// Password-based (iCloud, other providers)
    Password {
        host: String,
        port: u16,
        username: String,
        password: String,
    },
}

/// Handle to a running IDLE worker
struct IdleWorkerHandle {
    /// Channel to send shutdown signal
    shutdown_tx: mpsc::Sender<()>,
    /// Thread handle
    thread: Option<JoinHandle<()>>,
}

/// Manages IDLE connections for multiple accounts
pub struct IdleManager {
    /// Active workers keyed by account ID
    workers: Mutex<HashMap<String, IdleWorkerHandle>>,
    /// Channel to send events to the application
    event_tx: mpsc::Sender<IdleManagerEvent>,
}

impl IdleManager {
    /// Create a new IDLE manager
    ///
    /// Returns the manager and a receiver for events
    pub fn new() -> (Arc<Self>, mpsc::Receiver<IdleManagerEvent>) {
        let (event_tx, event_rx) = mpsc::channel();
        let manager = Arc::new(Self {
            workers: Mutex::new(HashMap::new()),
            event_tx,
        });
        (manager, event_rx)
    }

    /// Start IDLE monitoring for an account
    pub fn start_idle(&self, credentials: IdleCredentials) {
        let account_id = credentials.account_id.clone();

        // Check if already running
        {
            let workers = self.workers.lock().unwrap();
            if workers.contains_key(&account_id) {
                debug!("IDLE already running for account {}", account_id);
                return;
            }
        }

        info!("Starting IDLE for account {}", account_id);

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = mpsc::channel();

        // Clone event sender for the worker
        let event_tx = self.event_tx.clone();

        // Spawn worker thread
        let thread = thread::spawn(move || {
            idle_worker_loop(credentials, event_tx, shutdown_rx);
        });

        // Store handle
        let mut workers = self.workers.lock().unwrap();
        workers.insert(
            account_id,
            IdleWorkerHandle {
                shutdown_tx,
                thread: Some(thread),
            },
        );
    }

    /// Stop IDLE monitoring for an account
    pub fn stop_idle(&self, account_id: &str) {
        let mut workers = self.workers.lock().unwrap();
        if let Some(mut handle) = workers.remove(account_id) {
            info!("Stopping IDLE for account {}", account_id);
            // Send shutdown signal
            let _ = handle.shutdown_tx.send(());
            // Wait for thread to finish
            if let Some(thread) = handle.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// Stop all IDLE workers
    pub fn shutdown(&self) {
        let mut workers = self.workers.lock().unwrap();
        for (account_id, mut handle) in workers.drain() {
            info!("Shutting down IDLE for account {}", account_id);
            let _ = handle.shutdown_tx.send(());
            if let Some(thread) = handle.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

/// Worker loop that maintains IDLE connection for one account
fn idle_worker_loop(
    credentials: IdleCredentials,
    event_tx: mpsc::Sender<IdleManagerEvent>,
    shutdown_rx: mpsc::Receiver<()>,
) {
    let account_id = credentials.account_id.clone();

    // Use async-std runtime for this thread
    async_std::task::block_on(async {
        let mut reconnect_delay = Duration::from_secs(5);
        const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(300); // 5 minutes

        loop {
            // Check for shutdown signal (non-blocking)
            if shutdown_rx.try_recv().is_ok() {
                info!("IDLE worker shutdown requested for {}", account_id);
                break;
            }

            // Connect and authenticate
            let mut client = SimpleImapClient::new();
            let connect_result = match &credentials.auth_type {
                IdleAuthType::OAuth2 { host, access_token } => {
                    match host.as_str() {
                        "imap.gmail.com" => {
                            client.connect_gmail(&credentials.email, access_token).await
                        }
                        "outlook.office365.com" => {
                            client.connect_outlook(&credentials.email, access_token).await
                        }
                        _ => {
                            error!("Unknown OAuth2 host: {}", host);
                            Err(northmail_imap::ImapError::ServerError(
                                "Unknown host".to_string(),
                            ))
                        }
                    }
                }
                IdleAuthType::Password {
                    host,
                    port,
                    username,
                    password,
                } => {
                    client.connect_login(host, *port, username, password).await
                }
            };

            if let Err(e) = connect_result {
                error!("IDLE connect failed for {}: {}", account_id, e);
                let _ = event_tx.send(IdleManagerEvent::ConnectionLost {
                    account_id: account_id.clone(),
                });

                // Wait before reconnecting with exponential backoff
                async_std::task::sleep(reconnect_delay).await;
                reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
                continue;
            }

            info!("IDLE connected for {}", account_id);

            // Select INBOX
            if let Err(e) = client.select("INBOX").await {
                error!("IDLE select INBOX failed for {}: {}", account_id, e);
                let _ = client.logout().await;
                async_std::task::sleep(reconnect_delay).await;
                reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
                continue;
            }

            // Reset reconnect delay on successful connection
            reconnect_delay = Duration::from_secs(5);

            // IDLE loop
            loop {
                // Check for shutdown
                if shutdown_rx.try_recv().is_ok() {
                    info!("IDLE worker shutdown during IDLE for {}", account_id);
                    let _ = client.logout().await;
                    return;
                }

                // Enter IDLE mode with 28-minute timeout (RFC recommends <29 min)
                let idle_timeout = Duration::from_secs(28 * 60);
                match client.idle(idle_timeout).await {
                    Ok(IdleEvent::NewMessages(count)) => {
                        info!("IDLE: {} new messages for {}", count, account_id);
                        // Exit IDLE to allow sync
                        if let Err(e) = client.idle_done().await {
                            warn!("IDLE DONE failed for {}: {}", account_id, e);
                            break; // Reconnect
                        }
                        let _ = event_tx.send(IdleManagerEvent::NewMail {
                            account_id: account_id.clone(),
                        });
                        // Re-select to refresh state
                        if let Err(e) = client.select("INBOX").await {
                            warn!("IDLE re-select failed for {}: {}", account_id, e);
                            break; // Reconnect
                        }
                    }
                    Ok(IdleEvent::Expunge(_)) => {
                        // Message deleted - might want to sync
                        if let Err(e) = client.idle_done().await {
                            warn!("IDLE DONE failed for {}: {}", account_id, e);
                            break;
                        }
                        // Re-select to refresh state
                        if let Err(e) = client.select("INBOX").await {
                            warn!("IDLE re-select failed for {}: {}", account_id, e);
                            break;
                        }
                    }
                    Ok(IdleEvent::FlagsChanged) => {
                        // Flags changed - might want to sync
                        if let Err(e) = client.idle_done().await {
                            warn!("IDLE DONE failed for {}: {}", account_id, e);
                            break;
                        }
                        if let Err(e) = client.select("INBOX").await {
                            warn!("IDLE re-select failed for {}: {}", account_id, e);
                            break;
                        }
                    }
                    Ok(IdleEvent::Timeout) => {
                        // Normal timeout - send DONE and re-enter IDLE (keepalive)
                        debug!("IDLE timeout (keepalive) for {}", account_id);
                        if let Err(e) = client.idle_done().await {
                            warn!("IDLE DONE failed for {}: {}", account_id, e);
                            break;
                        }
                        // Quick NOOP to keep connection alive
                        if let Err(e) = client.noop().await {
                            warn!("NOOP failed for {}: {}", account_id, e);
                            break;
                        }
                    }
                    Ok(IdleEvent::ServerBye) => {
                        info!("Server closed IDLE connection for {}", account_id);
                        let _ = event_tx.send(IdleManagerEvent::ConnectionLost {
                            account_id: account_id.clone(),
                        });
                        break; // Reconnect
                    }
                    Err(e) => {
                        let err_msg = e.to_string();
                        // Detect IDLE not supported: server responds with BAD/NO
                        // instead of '+' continuation
                        if err_msg.contains("Expected '+' continuation") {
                            warn!("IDLE not supported by server for {}: {}", account_id, err_msg);
                            let _ = event_tx.send(IdleManagerEvent::NotSupported {
                                account_id: account_id.clone(),
                            });
                            let _ = client.logout().await;
                            return; // Stop entirely - don't reconnect
                        }
                        error!("IDLE error for {}: {}", account_id, e);
                        let _ = event_tx.send(IdleManagerEvent::ConnectionLost {
                            account_id: account_id.clone(),
                        });
                        break; // Reconnect
                    }
                }
            }

            // Connection lost - wait before reconnecting
            async_std::task::sleep(reconnect_delay).await;
            reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
        }
    });
}
