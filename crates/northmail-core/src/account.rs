//! Account management

use northmail_auth::AuthMethod;
use serde::{Deserialize, Serialize};

/// Email account configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountConfig {
    /// IMAP server hostname
    pub imap_host: String,
    /// IMAP server port
    pub imap_port: u16,
    /// SMTP server hostname
    pub smtp_host: String,
    /// SMTP server port
    pub smtp_port: u16,
}

impl AccountConfig {
    /// Gmail configuration
    pub fn gmail() -> Self {
        Self {
            imap_host: "imap.gmail.com".to_string(),
            imap_port: 993,
            smtp_host: "smtp.gmail.com".to_string(),
            smtp_port: 587,
        }
    }
}

/// Represents an email account
#[derive(Debug, Clone)]
pub struct Account {
    /// Unique identifier
    pub id: String,
    /// Email address
    pub email: String,
    /// Display name
    pub display_name: Option<String>,
    /// Provider (e.g., "gmail")
    pub provider: String,
    /// Authentication method
    pub auth_method: AuthMethod,
    /// Server configuration
    pub config: AccountConfig,
}

impl Account {
    /// Create a new Gmail account from GOA
    pub fn gmail_from_goa(account_id: String, email: String) -> Self {
        Self {
            id: format!("goa:{}", account_id),
            email,
            display_name: None,
            provider: "gmail".to_string(),
            auth_method: AuthMethod::Goa { account_id },
            config: AccountConfig::gmail(),
        }
    }

    /// Create a new Gmail account with standalone OAuth2
    pub fn gmail_from_oauth2(email: String) -> Self {
        Self {
            id: format!("oauth2:{}", email),
            email: email.clone(),
            display_name: None,
            provider: "gmail".to_string(),
            auth_method: AuthMethod::OAuth2 { email },
            config: AccountConfig::gmail(),
        }
    }
}
