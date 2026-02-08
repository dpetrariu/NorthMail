//! IMAP client implementation

use crate::{Folder, FolderType, ImapError, ImapResult, MessageHeader, XOAuth2Authenticator};
use crate::message::{EmailAddress, Envelope, MessageFlags};
use async_imap::Session;
use async_native_tls::TlsStream;
use async_std::net::TcpStream;
use futures::TryStreamExt;
use tracing::{debug, info};

// Type alias for our TLS stream
type ImapStream = TlsStream<TcpStream>;

/// IMAP client for email operations
pub struct ImapClient {
    session: Option<Session<ImapStream>>,
    host: String,
    port: u16,
}

impl ImapClient {
    /// Create a new IMAP client
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            session: None,
            host: host.into(),
            port,
        }
    }

    /// Create a Gmail IMAP client
    pub fn gmail() -> Self {
        Self::new("imap.gmail.com", 993)
    }

    /// Connect and authenticate using XOAUTH2 (for Gmail)
    pub async fn authenticate_xoauth2(
        &mut self,
        email: &str,
        access_token: &str,
    ) -> ImapResult<()> {
        info!("Connecting to {}:{}", self.host, self.port);

        // Create TCP connection using async-std
        let tcp_stream = TcpStream::connect(format!("{}:{}", self.host, self.port))
            .await
            .map_err(|e| ImapError::ConnectionFailed(e.to_string()))?;

        // Wrap with TLS
        let tls_connector = async_native_tls::TlsConnector::new();
        let tls_stream = tls_connector
            .connect(&self.host, tcp_stream)
            .await
            .map_err(|e| ImapError::TlsError(e.to_string()))?;

        debug!("TLS connection established");

        // Create IMAP client
        let client = async_imap::Client::new(tls_stream);

        info!("Authenticating with XOAUTH2 for {}", email);

        // Authenticate with XOAUTH2
        let auth = XOAuth2Authenticator::new(email, access_token);
        let session = client
            .authenticate("XOAUTH2", auth)
            .await
            .map_err(|(e, _)| ImapError::AuthenticationFailed(e.to_string()))?;

        self.session = Some(session);
        info!("XOAUTH2 authentication successful");
        Ok(())
    }

    /// Connect and authenticate using LOGIN (username/password) for standard IMAP
    pub async fn authenticate_login(
        &mut self,
        username: &str,
        password: &str,
    ) -> ImapResult<()> {
        info!("Connecting to {}:{}", self.host, self.port);

        // Create TCP connection
        let tcp_stream = TcpStream::connect(format!("{}:{}", self.host, self.port))
            .await
            .map_err(|e| ImapError::ConnectionFailed(e.to_string()))?;

        // Wrap with TLS
        let tls_connector = async_native_tls::TlsConnector::new();
        let tls_stream = tls_connector
            .connect(&self.host, tcp_stream)
            .await
            .map_err(|e| ImapError::TlsError(e.to_string()))?;

        debug!("TLS connection established");

        // Create IMAP client
        let client = async_imap::Client::new(tls_stream);

        info!("Authenticating with LOGIN for {}", username);

        // Use standard login
        let session = client
            .login(username, password)
            .await
            .map_err(|(e, _)| ImapError::AuthenticationFailed(e.to_string()))?;

        self.session = Some(session);
        info!("LOGIN authentication successful");
        Ok(())
    }

    /// Create an iCloud IMAP client
    pub fn icloud() -> Self {
        Self::new("imap.mail.me.com", 993)
    }

    /// Get the session, returning an error if not connected
    fn session_mut(&mut self) -> ImapResult<&mut Session<ImapStream>> {
        self.session.as_mut().ok_or(ImapError::NotConnected)
    }

    /// List all folders/mailboxes
    pub async fn list_folders(&mut self) -> ImapResult<Vec<Folder>> {
        let session = self.session_mut()?;

        let mailboxes = session
            .list(None, Some("*"))
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        let mut folders = Vec::new();

        let mut stream = mailboxes;
        while let Some(mailbox) = stream
            .try_next()
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?
        {
            let delim_str = mailbox.delimiter().unwrap_or("/");
            let delim_char = delim_str.chars().next();

            let name = mailbox
                .name()
                .split(delim_str)
                .last()
                .unwrap_or(mailbox.name())
                .to_string();

            let attributes: Vec<String> = mailbox
                .attributes()
                .iter()
                .map(|a| format!("{:?}", a))
                .collect();

            folders.push(Folder::new(
                name,
                mailbox.name().to_string(),
                delim_char,
                attributes,
            ));
        }

        debug!("Found {} folders", folders.len());
        Ok(folders)
    }

    /// Select a folder and get its status
    pub async fn select_folder(&mut self, folder_path: &str) -> ImapResult<Folder> {
        let session = self.session_mut()?;

        let mailbox = session
            .select(folder_path)
            .await
            .map_err(|e| ImapError::FolderNotFound(format!("{}: {}", folder_path, e)))?;

        let folder = Folder {
            name: folder_path
                .split('/')
                .last()
                .unwrap_or(folder_path)
                .to_string(),
            full_path: folder_path.to_string(),
            folder_type: FolderType::from_attributes_and_name(&[], folder_path),
            delimiter: Some('/'),
            attributes: Vec::new(),
            uidvalidity: mailbox.uid_validity,
            message_count: Some(mailbox.exists),
            unread_count: None,
            uid_next: mailbox.uid_next,
        };

        debug!(
            "Selected folder {} with {} messages",
            folder_path,
            folder.message_count.unwrap_or(0)
        );

        Ok(folder)
    }

    /// Fetch message headers for a range of UIDs
    pub async fn fetch_headers(&mut self, uids: &str) -> ImapResult<Vec<MessageHeader>> {
        let session = self.session_mut()?;

        let fetch_stream = session
            .uid_fetch(uids, "(UID FLAGS ENVELOPE RFC822.SIZE)")
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        let mut messages = Vec::new();

        let mut stream = fetch_stream;
        while let Some(fetch) = stream
            .try_next()
            .await
            .map_err(|e| ImapError::ParseError(e.to_string()))?
        {
            let uid = fetch.uid.ok_or_else(|| {
                ImapError::ParseError("Missing UID in FETCH response".to_string())
            })?;

            let envelope = fetch.envelope().map(|env| {
                let parse_addresses =
                    |addrs: Option<&Vec<imap_proto::types::Address>>| -> Vec<EmailAddress> {
                        addrs
                            .map(|v| {
                                v.iter()
                                    .map(|a| {
                                        let mailbox = a
                                            .mailbox
                                            .as_ref()
                                            .map(|s| String::from_utf8_lossy(s).to_string())
                                            .unwrap_or_default();
                                        let host = a
                                            .host
                                            .as_ref()
                                            .map(|s| String::from_utf8_lossy(s).to_string())
                                            .unwrap_or_default();
                                        let address = format!("{}@{}", mailbox, host);
                                        let name = a
                                            .name
                                            .as_ref()
                                            .map(|s| String::from_utf8_lossy(s).to_string());
                                        EmailAddress::new(name, address)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    };

                Envelope {
                    message_id: env
                        .message_id
                        .as_ref()
                        .map(|s| String::from_utf8_lossy(s).to_string()),
                    subject: env
                        .subject
                        .as_ref()
                        .map(|s| String::from_utf8_lossy(s).to_string()),
                    from: parse_addresses(env.from.as_ref()),
                    to: parse_addresses(env.to.as_ref()),
                    cc: parse_addresses(env.cc.as_ref()),
                    reply_to: parse_addresses(env.reply_to.as_ref()),
                    date: env
                        .date
                        .as_ref()
                        .map(|s| String::from_utf8_lossy(s).to_string()),
                    in_reply_to: env
                        .in_reply_to
                        .as_ref()
                        .map(|s| String::from_utf8_lossy(s).to_string()),
                }
            });

            // Parse flags - flags() returns an iterator directly
            let flag_strs: Vec<String> = fetch.flags()
                .map(|f| format!("{:?}", f))
                .collect();
            let flag_refs: Vec<&str> = flag_strs.iter().map(|s| s.as_str()).collect();
            let flags = MessageFlags::from_imap_flags(&flag_refs);

            messages.push(MessageHeader {
                uid,
                seq: fetch.message,
                envelope: envelope.unwrap_or_default(),
                flags,
                size: fetch.size.unwrap_or(0),
                has_attachments: false,
            });
        }

        debug!("Fetched {} message headers", messages.len());
        Ok(messages)
    }

    /// Fetch a complete message body
    pub async fn fetch_body(&mut self, uid: u32) -> ImapResult<Vec<u8>> {
        let session = self.session_mut()?;

        let fetch_stream = session
            .uid_fetch(uid.to_string(), "BODY[]")
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        let mut stream = fetch_stream;
        while let Some(fetch) = stream
            .try_next()
            .await
            .map_err(|e| ImapError::ParseError(e.to_string()))?
        {
            if let Some(body) = fetch.body() {
                return Ok(body.to_vec());
            }
        }

        Err(ImapError::MessageNotFound(uid))
    }

    /// Set flags on a message
    pub async fn set_flags(&mut self, uid: u32, flags: &[&str]) -> ImapResult<()> {
        let session = self.session_mut()?;

        let flags_str = flags.join(" ");
        session
            .uid_store(uid.to_string(), format!("+FLAGS ({})", flags_str))
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        Ok(())
    }

    /// Remove flags from a message
    pub async fn remove_flags(&mut self, uid: u32, flags: &[&str]) -> ImapResult<()> {
        let session = self.session_mut()?;

        let flags_str = flags.join(" ");
        session
            .uid_store(uid.to_string(), format!("-FLAGS ({})", flags_str))
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        Ok(())
    }

    /// Mark a message as read
    pub async fn mark_read(&mut self, uid: u32) -> ImapResult<()> {
        self.set_flags(uid, &["\\Seen"]).await
    }

    /// Mark a message as unread
    pub async fn mark_unread(&mut self, uid: u32) -> ImapResult<()> {
        self.remove_flags(uid, &["\\Seen"]).await
    }

    /// Move a message to another folder
    pub async fn move_message(&mut self, uid: u32, dest_folder: &str) -> ImapResult<()> {
        // Copy to destination
        {
            let session = self.session_mut()?;
            session
                .uid_copy(uid.to_string(), dest_folder)
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;
        }

        // Mark original as deleted
        self.set_flags(uid, &["\\Deleted"]).await?;

        // Expunge
        {
            let session = self.session_mut()?;
            session
                .expunge()
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?
                .try_collect::<Vec<_>>()
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;
        }

        Ok(())
    }

    /// Take the session for IDLE operations
    /// Returns the session, leaving the client disconnected.
    /// The caller is responsible for IDLE and restoring the session.
    pub fn take_session(&mut self) -> ImapResult<Session<ImapStream>> {
        self.session.take().ok_or(ImapError::NotConnected)
    }

    /// Restore a session after IDLE
    pub fn restore_session(&mut self, session: Session<ImapStream>) {
        self.session = Some(session);
    }

    /// Close the connection
    pub async fn logout(&mut self) -> ImapResult<()> {
        if let Some(mut session) = self.session.take() {
            session
                .logout()
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;
        }
        Ok(())
    }
}
