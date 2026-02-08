//! Secure credential storage using libsecret
//!
//! Stores OAuth2 tokens in the system keyring via libsecret.

use crate::{AuthError, AuthResult, TokenPair};
use std::collections::HashMap;
use tracing::{debug, info};

/// Schema for storing NorthMail credentials
const SCHEMA_NAME: &str = "org.northmail.Credentials";

/// Manages secure storage of credentials
pub struct SecretStore {
    schema: libsecret::Schema,
}

impl SecretStore {
    /// Create a new secret store
    pub fn new() -> Self {
        let mut attributes = HashMap::new();
        attributes.insert("type", libsecret::SchemaAttributeType::String);
        attributes.insert("email", libsecret::SchemaAttributeType::String);

        let schema = libsecret::Schema::new(
            SCHEMA_NAME,
            libsecret::SchemaFlags::NONE,
            attributes,
        );

        Self { schema }
    }

    /// Store OAuth2 tokens for an email account
    pub async fn store_tokens(&self, email: &str, tokens: &TokenPair) -> AuthResult<()> {
        let json = serde_json::to_string(tokens)
            .map_err(|e| AuthError::SecretError(format!("Failed to serialize tokens: {}", e)))?;

        let attributes = std::collections::HashMap::from([
            ("type", "oauth2_tokens"),
            ("email", email),
        ]);

        libsecret::password_store_future(
            Some(&self.schema),
            attributes,
            Some(libsecret::COLLECTION_DEFAULT),
            &format!("NorthMail OAuth2 tokens for {}", email),
            &json,
        )
        .await
        .map_err(|e| AuthError::SecretError(e.to_string()))?;

        info!("Stored OAuth2 tokens for {}", email);
        Ok(())
    }

    /// Retrieve OAuth2 tokens for an email account
    pub async fn get_tokens(&self, email: &str) -> AuthResult<Option<TokenPair>> {
        let attributes = std::collections::HashMap::from([
            ("type", "oauth2_tokens"),
            ("email", email),
        ]);

        let secret = libsecret::password_lookup_future(Some(&self.schema), attributes)
            .await
            .map_err(|e| AuthError::SecretError(e.to_string()))?;

        match secret {
            Some(json) => {
                let tokens: TokenPair = serde_json::from_str(&json)
                    .map_err(|e| AuthError::SecretError(format!("Failed to parse tokens: {}", e)))?;
                debug!("Retrieved OAuth2 tokens for {}", email);
                Ok(Some(tokens))
            }
            None => {
                debug!("No stored tokens found for {}", email);
                Ok(None)
            }
        }
    }

    /// Delete stored tokens for an email account
    pub async fn delete_tokens(&self, email: &str) -> AuthResult<()> {
        let attributes = std::collections::HashMap::from([
            ("type", "oauth2_tokens"),
            ("email", email),
        ]);

        libsecret::password_clear_future(Some(&self.schema), attributes)
            .await
            .map_err(|e| AuthError::SecretError(e.to_string()))?;

        info!("Deleted OAuth2 tokens for {}", email);
        Ok(())
    }
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::new()
    }
}
