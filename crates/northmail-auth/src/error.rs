//! Error types for the auth module

use thiserror::Error;

/// Result type for auth operations
pub type AuthResult<T> = Result<T, AuthError>;

/// Errors that can occur during authentication
#[derive(Debug, Error)]
pub enum AuthError {
    /// GOA service is not available
    #[error("GNOME Online Accounts service is not available")]
    GoaUnavailable,

    /// GOA account not found
    #[error("Account not found: {0}")]
    AccountNotFound(String),

    /// Token not found in secret storage
    #[error("Token not found for: {0}")]
    TokenNotFound(String),

    /// Token has expired and needs refresh
    #[error("Token has expired")]
    TokenExpired,

    /// OAuth2 flow was cancelled by user
    #[error("OAuth2 flow was cancelled")]
    FlowCancelled,

    /// OAuth2 authorization failed
    #[error("OAuth2 authorization failed: {0}")]
    AuthorizationFailed(String),

    /// Token exchange failed
    #[error("Token exchange failed: {0}")]
    TokenExchangeFailed(String),

    /// Failed to start local callback server
    #[error("Failed to start callback server: {0}")]
    CallbackServerFailed(String),

    /// Secret storage error
    #[error("Secret storage error: {0}")]
    SecretError(String),

    /// D-Bus communication error
    #[error("D-Bus error: {0}")]
    DbusError(String),

    /// Network error
    #[error("Network error: {0}")]
    NetworkError(String),

    /// Invalid configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// Generic IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}
