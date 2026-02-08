//! GNOME Online Accounts integration via D-Bus
//!
//! Provides access to email accounts configured in GNOME Settings.
//! This is the preferred authentication method as it:
//! - Reuses existing account configurations
//! - Handles token refresh automatically
//! - Provides a consistent user experience

use crate::{AuthError, AuthResult};
use tracing::{debug, info, warn};
use zbus::{proxy, Connection};
use zbus::zvariant::ObjectPath;

/// D-Bus proxy for GOA Account interface
#[proxy(
    interface = "org.gnome.OnlineAccounts.Account",
    default_service = "org.gnome.OnlineAccounts"
)]
trait GoaAccountInterface {
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn provider_type(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn provider_name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn presentation_identity(&self) -> zbus::Result<String>;

    fn ensure_credentials(&self) -> zbus::Result<i32>;
}

/// D-Bus proxy for GOA Mail interface
#[proxy(
    interface = "org.gnome.OnlineAccounts.Mail",
    default_service = "org.gnome.OnlineAccounts"
)]
trait GoaMailInterface {
    #[zbus(property)]
    fn email_address(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn imap_host(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn imap_use_ssl(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn imap_user_name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn smtp_host(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn smtp_use_ssl(&self) -> zbus::Result<bool>;
}

/// D-Bus proxy for GOA OAuth2Based interface
#[proxy(
    interface = "org.gnome.OnlineAccounts.OAuth2Based",
    default_service = "org.gnome.OnlineAccounts"
)]
trait GoaOAuth2Interface {
    fn get_access_token(&self) -> zbus::Result<(String, i32)>;
}

/// D-Bus proxy for GOA PasswordBased interface (for iCloud, etc.)
#[proxy(
    interface = "org.gnome.OnlineAccounts.PasswordBased",
    default_service = "org.gnome.OnlineAccounts"
)]
trait GoaPasswordBasedInterface {
    fn get_password(&self, id: &str) -> zbus::Result<String>;
}

/// Authentication type for an account
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoaAuthType {
    /// OAuth2 (Gmail, etc.)
    OAuth2,
    /// Password-based (iCloud, generic IMAP)
    Password,
    /// Unknown/unsupported
    Unknown,
}

/// Represents a mail-enabled GOA account
#[derive(Debug, Clone)]
pub struct GoaAccount {
    /// Unique identifier for this account
    pub id: String,
    /// Object path on D-Bus
    pub object_path: String,
    /// Email address
    pub email: String,
    /// Display name (provider name like "Google")
    pub provider_name: String,
    /// Provider type (e.g., "google", "imap_smtp")
    pub provider_type: String,
    /// Whether mail is enabled for this account
    pub mail_enabled: bool,
    /// IMAP host if available
    pub imap_host: Option<String>,
    /// IMAP username if available
    pub imap_username: Option<String>,
    /// SMTP host if available
    pub smtp_host: Option<String>,
    /// Authentication type
    pub auth_type: GoaAuthType,
}

/// Manager for GNOME Online Accounts
pub struct GoaManager {
    connection: Option<Connection>,
}

impl GoaManager {
    /// Create a new GOA manager
    pub async fn new() -> AuthResult<Self> {
        match Connection::session().await {
            Ok(conn) => {
                // Check if GOA service is available
                let dbus = zbus::fdo::DBusProxy::new(&conn)
                    .await
                    .map_err(|e| AuthError::DbusError(e.to_string()))?;

                let has_goa = dbus
                    .name_has_owner("org.gnome.OnlineAccounts".try_into().unwrap())
                    .await
                    .unwrap_or(false);

                if has_goa {
                    info!("Connected to GNOME Online Accounts service");
                    Ok(Self {
                        connection: Some(conn),
                    })
                } else {
                    warn!("GNOME Online Accounts service is not running");
                    Ok(Self { connection: None })
                }
            }
            Err(e) => {
                warn!("Could not connect to session bus: {}", e);
                Ok(Self { connection: None })
            }
        }
    }

    /// Check if GOA is available
    pub fn is_available(&self) -> bool {
        self.connection.is_some()
    }

    /// List all accounts with mail enabled
    pub async fn list_mail_accounts(&self) -> AuthResult<Vec<GoaAccount>> {
        let conn = self.connection.as_ref().ok_or(AuthError::GoaUnavailable)?;

        // Use ObjectManager to list all accounts
        let object_manager = zbus::fdo::ObjectManagerProxy::builder(conn)
            .destination("org.gnome.OnlineAccounts")
            .map_err(|e| AuthError::DbusError(e.to_string()))?
            .path("/org/gnome/OnlineAccounts")
            .map_err(|e| AuthError::DbusError(e.to_string()))?
            .build()
            .await
            .map_err(|e| AuthError::DbusError(e.to_string()))?;

        let objects = object_manager
            .get_managed_objects()
            .await
            .map_err(|e| AuthError::DbusError(e.to_string()))?;

        let mut accounts = Vec::new();

        for (path, interfaces) in objects {
            // Check if this object has the Mail interface
            if !interfaces.contains_key("org.gnome.OnlineAccounts.Mail") {
                continue;
            }

            // Get account info
            let account_proxy = GoaAccountInterfaceProxy::builder(conn)
                .destination("org.gnome.OnlineAccounts")
                .map_err(|e| AuthError::DbusError(e.to_string()))?
                .path(path.clone())
                .map_err(|e| AuthError::DbusError(e.to_string()))?
                .build()
                .await
                .map_err(|e| AuthError::DbusError(e.to_string()))?;

            let mail_proxy = GoaMailInterfaceProxy::builder(conn)
                .destination("org.gnome.OnlineAccounts")
                .map_err(|e| AuthError::DbusError(e.to_string()))?
                .path(path.clone())
                .map_err(|e| AuthError::DbusError(e.to_string()))?
                .build()
                .await
                .map_err(|e| AuthError::DbusError(e.to_string()))?;

            let id = account_proxy.id().await.unwrap_or_default();
            let provider_type = account_proxy.provider_type().await.unwrap_or_default();
            let provider_name = account_proxy.provider_name().await.unwrap_or_default();
            let email = mail_proxy.email_address().await.unwrap_or_default();
            let imap_host = mail_proxy.imap_host().await.ok();
            let imap_username = mail_proxy.imap_user_name().await.ok();
            let smtp_host = mail_proxy.smtp_host().await.ok();

            if email.is_empty() {
                debug!("Skipping account {} with no email", id);
                continue;
            }

            // Detect auth type based on available interfaces
            let auth_type = if interfaces.contains_key("org.gnome.OnlineAccounts.OAuth2Based") {
                GoaAuthType::OAuth2
            } else if interfaces.contains_key("org.gnome.OnlineAccounts.PasswordBased") {
                GoaAuthType::Password
            } else {
                GoaAuthType::Unknown
            };

            debug!(
                "Account {} ({}) auth_type: {:?}",
                email, provider_type, auth_type
            );

            accounts.push(GoaAccount {
                id,
                object_path: path.to_string(),
                email,
                provider_name,
                provider_type,
                mail_enabled: true,
                imap_host,
                imap_username,
                smtp_host,
                auth_type,
            });
        }

        info!("Found {} mail-enabled GOA accounts", accounts.len());
        Ok(accounts)
    }

    /// Get a specific account by ID
    pub async fn get_account(&self, account_id: &str) -> AuthResult<Option<GoaAccount>> {
        let accounts = self.list_mail_accounts().await?;
        Ok(accounts.into_iter().find(|a| a.id == account_id))
    }

    /// Get an access token for an account
    ///
    /// GOA handles token refresh automatically in the background,
    /// so the returned token should always be valid.
    pub async fn get_access_token(&self, account_id: &str) -> AuthResult<String> {
        let conn = self.connection.as_ref().ok_or(AuthError::GoaUnavailable)?;

        // Find the account
        let account = self
            .get_account(account_id)
            .await?
            .ok_or_else(|| AuthError::AccountNotFound(account_id.to_string()))?;

        // Ensure credentials are fresh
        let object_path = ObjectPath::try_from(account.object_path.as_str())
            .map_err(|e| AuthError::DbusError(format!("Invalid object path: {}", e)))?;
        let account_proxy = GoaAccountInterfaceProxy::builder(conn)
            .destination("org.gnome.OnlineAccounts")
            .map_err(|e| AuthError::DbusError(e.to_string()))?
            .path(object_path.clone())
            .map_err(|e| AuthError::DbusError(e.to_string()))?
            .build()
            .await
            .map_err(|e| AuthError::DbusError(e.to_string()))?;

        account_proxy
            .ensure_credentials()
            .await
            .map_err(|e| AuthError::AuthorizationFailed(e.to_string()))?;

        // Get OAuth2 access token
        let oauth2_proxy = GoaOAuth2InterfaceProxy::builder(conn)
            .destination("org.gnome.OnlineAccounts")
            .map_err(|e| AuthError::DbusError(e.to_string()))?
            .path(object_path)
            .map_err(|e| AuthError::DbusError(e.to_string()))?
            .build()
            .await
            .map_err(|e| AuthError::DbusError(e.to_string()))?;

        let (access_token, _expires_in) = oauth2_proxy
            .get_access_token()
            .await
            .map_err(|e| AuthError::TokenExchangeFailed(e.to_string()))?;

        debug!("Got access token for account {}", account_id);
        Ok(access_token)
    }

    /// Get the password for a password-based account (iCloud, generic IMAP, etc.)
    pub async fn get_password(&self, account_id: &str) -> AuthResult<String> {
        let conn = self.connection.as_ref().ok_or(AuthError::GoaUnavailable)?;

        // Find the account
        let account = self
            .get_account(account_id)
            .await?
            .ok_or_else(|| AuthError::AccountNotFound(account_id.to_string()))?;

        // Ensure credentials are fresh
        let object_path = ObjectPath::try_from(account.object_path.as_str())
            .map_err(|e| AuthError::DbusError(format!("Invalid object path: {}", e)))?;
        let account_proxy = GoaAccountInterfaceProxy::builder(conn)
            .destination("org.gnome.OnlineAccounts")
            .map_err(|e| AuthError::DbusError(e.to_string()))?
            .path(object_path.clone())
            .map_err(|e| AuthError::DbusError(e.to_string()))?
            .build()
            .await
            .map_err(|e| AuthError::DbusError(e.to_string()))?;

        account_proxy
            .ensure_credentials()
            .await
            .map_err(|e| AuthError::AuthorizationFailed(e.to_string()))?;

        // Get password from PasswordBased interface
        let password_proxy = GoaPasswordBasedInterfaceProxy::builder(conn)
            .destination("org.gnome.OnlineAccounts")
            .map_err(|e| AuthError::DbusError(e.to_string()))?
            .path(object_path)
            .map_err(|e| AuthError::DbusError(e.to_string()))?
            .build()
            .await
            .map_err(|e| AuthError::DbusError(e.to_string()))?;

        // For IMAP, we use "imap-password" as the id
        let password = password_proxy
            .get_password("imap-password")
            .await
            .map_err(|e| AuthError::TokenExchangeFailed(format!("Failed to get password: {}", e)))?;

        debug!("Got password for account {}", account_id);
        Ok(password)
    }
}
