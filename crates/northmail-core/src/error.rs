//! Error types for the core module

use thiserror::Error;

/// Result type for core operations
pub type CoreResult<T> = Result<T, CoreError>;

/// Errors that can occur in core operations
#[derive(Debug, Error)]
pub enum CoreError {
    /// Database error
    #[error("Database error: {0}")]
    DatabaseError(String),

    /// Account not found
    #[error("Account not found: {0}")]
    AccountNotFound(String),

    /// Folder not found
    #[error("Folder not found: {0}")]
    FolderNotFound(String),

    /// Message not found
    #[error("Message not found: {0}")]
    MessageNotFound(i64),

    /// Authentication error
    #[error("Authentication error: {0}")]
    AuthError(String),

    /// IMAP error
    #[error("IMAP error: {0}")]
    ImapError(String),

    /// SMTP error
    #[error("SMTP error: {0}")]
    SmtpError(String),

    /// Sync error
    #[error("Sync error: {0}")]
    SyncError(String),

    /// Storage error
    #[error("Storage error: {0}")]
    StorageError(String),

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

impl From<sqlx::Error> for CoreError {
    fn from(e: sqlx::Error) -> Self {
        CoreError::DatabaseError(e.to_string())
    }
}

impl From<northmail_auth::AuthError> for CoreError {
    fn from(e: northmail_auth::AuthError) -> Self {
        CoreError::AuthError(e.to_string())
    }
}

impl From<northmail_imap::ImapError> for CoreError {
    fn from(e: northmail_imap::ImapError) -> Self {
        CoreError::ImapError(e.to_string())
    }
}

impl From<northmail_smtp::SmtpError> for CoreError {
    fn from(e: northmail_smtp::SmtpError) -> Self {
        CoreError::SmtpError(e.to_string())
    }
}
