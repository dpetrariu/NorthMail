//! Error types for SMTP operations

use thiserror::Error;

/// Result type for SMTP operations
pub type SmtpResult<T> = Result<T, SmtpError>;

/// Errors that can occur during SMTP operations
#[derive(Debug, Error)]
pub enum SmtpError {
    /// Connection failed
    #[error("Failed to connect to SMTP server: {0}")]
    ConnectionFailed(String),

    /// Authentication failed
    #[error("SMTP authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Failed to send message
    #[error("Failed to send message: {0}")]
    SendFailed(String),

    /// Invalid email address
    #[error("Invalid email address: {0}")]
    InvalidAddress(String),

    /// Message building error
    #[error("Failed to build message: {0}")]
    MessageBuildError(String),

    /// TLS error
    #[error("TLS error: {0}")]
    TlsError(String),
}
