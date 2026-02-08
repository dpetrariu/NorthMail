//! Authentication module for NorthMail
//!
//! Provides OAuth2 authentication for Gmail through two methods:
//! 1. GNOME Online Accounts (GOA) - Primary, uses system-configured accounts
//! 2. Standalone OAuth2 with PKCE - Fallback for non-GNOME environments

mod error;
mod goa;
mod oauth2;
mod secrets;
mod xoauth2;

pub use error::{AuthError, AuthResult};
pub use goa::{GoaAccount, GoaAuthType, GoaManager};
pub use oauth2::{OAuth2Config, OAuth2Flow, OAuth2Provider, TokenPair};
pub use secrets::SecretStore;
pub use xoauth2::XOAuth2Token;

/// Gmail OAuth2 configuration
pub mod gmail {
    use super::OAuth2Config;

    /// Gmail OAuth2 scope for full mail access
    pub const MAIL_SCOPE: &str = "https://mail.google.com/";

    /// Gmail IMAP server
    pub const IMAP_HOST: &str = "imap.gmail.com";
    pub const IMAP_PORT: u16 = 993;

    /// Gmail SMTP server
    pub const SMTP_HOST: &str = "smtp.gmail.com";
    pub const SMTP_PORT: u16 = 587;

    /// Create Gmail OAuth2 configuration
    ///
    /// Note: You must register your own OAuth2 client at
    /// https://console.cloud.google.com/ and replace this client ID
    pub fn oauth2_config(client_id: &str) -> OAuth2Config {
        OAuth2Config {
            client_id: client_id.to_string(),
            // Native apps use PKCE and don't need a client secret
            client_secret: None,
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
            token_url: "https://oauth2.googleapis.com/token".to_string(),
            scopes: vec![MAIL_SCOPE.to_string()],
            redirect_port: 8855,
        }
    }
}

/// Authentication method used for an account
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AuthMethod {
    /// Account from GNOME Online Accounts
    Goa { account_id: String },
    /// Standalone OAuth2 with tokens in libsecret
    OAuth2 { email: String },
}

impl AuthMethod {
    pub fn identifier(&self) -> &str {
        match self {
            AuthMethod::Goa { account_id } => account_id,
            AuthMethod::OAuth2 { email } => email,
        }
    }
}

/// Manages authentication for email accounts
pub struct AuthManager {
    goa_manager: GoaManager,
    secret_store: SecretStore,
}

impl AuthManager {
    /// Create a new authentication manager
    pub async fn new() -> AuthResult<Self> {
        let goa_manager = GoaManager::new().await?;
        let secret_store = SecretStore::new();

        Ok(Self {
            goa_manager,
            secret_store,
        })
    }

    /// Get all available mail accounts from GOA
    pub async fn list_goa_accounts(&self) -> AuthResult<Vec<GoaAccount>> {
        self.goa_manager.list_mail_accounts().await
    }

    /// Check if GOA is available on this system
    pub fn is_goa_available(&self) -> bool {
        self.goa_manager.is_available()
    }

    /// Get an access token for a GOA account
    pub async fn get_goa_token(&self, account_id: &str) -> AuthResult<String> {
        self.goa_manager.get_access_token(account_id).await
    }

    /// Get email and access token for a GOA account (for XOAUTH2 auth)
    pub async fn get_xoauth2_token_for_goa(&self, account_id: &str) -> AuthResult<(String, String)> {
        let account = self
            .goa_manager
            .get_account(account_id)
            .await?
            .ok_or_else(|| AuthError::AccountNotFound(account_id.to_string()))?;
        let access_token = self.goa_manager.get_access_token(account_id).await?;
        Ok((account.email, access_token))
    }

    /// Get password for a password-based GOA account (iCloud, generic IMAP, etc.)
    pub async fn get_goa_password(&self, account_id: &str) -> AuthResult<String> {
        self.goa_manager.get_password(account_id).await
    }

    /// Start standalone OAuth2 flow for Gmail
    pub async fn start_oauth2_flow(&self, config: OAuth2Config) -> AuthResult<OAuth2Flow> {
        OAuth2Flow::new(config)
    }

    /// Store OAuth2 tokens in libsecret
    pub async fn store_tokens(&self, email: &str, tokens: &TokenPair) -> AuthResult<()> {
        self.secret_store.store_tokens(email, tokens).await
    }

    /// Retrieve OAuth2 tokens from libsecret
    pub async fn get_tokens(&self, email: &str) -> AuthResult<Option<TokenPair>> {
        self.secret_store.get_tokens(email).await
    }

    /// Delete stored tokens
    pub async fn delete_tokens(&self, email: &str) -> AuthResult<()> {
        self.secret_store.delete_tokens(email).await
    }

    /// Get an XOAUTH2 token for IMAP/SMTP authentication
    pub async fn get_xoauth2_token(&self, auth_method: &AuthMethod) -> AuthResult<XOAuth2Token> {
        match auth_method {
            AuthMethod::Goa { account_id } => {
                let account = self
                    .goa_manager
                    .get_account(account_id)
                    .await?
                    .ok_or_else(|| AuthError::AccountNotFound(account_id.clone()))?;
                let access_token = self.goa_manager.get_access_token(account_id).await?;
                Ok(XOAuth2Token::new(&account.email, &access_token))
            }
            AuthMethod::OAuth2 { email } => {
                let tokens = self
                    .secret_store
                    .get_tokens(email)
                    .await?
                    .ok_or_else(|| AuthError::TokenNotFound(email.clone()))?;

                // Check if token needs refresh
                if tokens.is_expired() {
                    // TODO: Implement token refresh
                    return Err(AuthError::TokenExpired);
                }

                Ok(XOAuth2Token::new(email, &tokens.access_token))
            }
        }
    }
}
