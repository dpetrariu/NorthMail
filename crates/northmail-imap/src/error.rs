//! Error types for IMAP operations

use thiserror::Error;

/// Result type for IMAP operations
pub type ImapResult<T> = Result<T, ImapError>;

/// Errors that can occur during IMAP operations
#[derive(Debug, Error)]
pub enum ImapError {
    /// Connection failed
    #[error("Failed to connect to IMAP server: {0}")]
    ConnectionFailed(String),

    /// Authentication failed
    #[error("IMAP authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Server returned an error
    #[error("IMAP server error: {0}")]
    ServerError(String),

    /// Folder not found
    #[error("Folder not found: {0}")]
    FolderNotFound(String),

    /// Message not found
    #[error("Message not found: UID {0}")]
    MessageNotFound(u32),

    /// Parse error
    #[error("Failed to parse IMAP response: {0}")]
    ParseError(String),

    /// TLS error
    #[error("TLS error: {0}")]
    TlsError(String),

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Session is not connected
    #[error("IMAP session is not connected")]
    NotConnected,

    /// Operation timed out
    #[error("Operation timed out")]
    Timeout,
}
