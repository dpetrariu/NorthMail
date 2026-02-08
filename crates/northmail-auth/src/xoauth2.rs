//! XOAUTH2 SASL mechanism for IMAP/SMTP
//!
//! Implements the XOAUTH2 authentication mechanism as specified at:
//! https://developers.google.com/workspace/gmail/imap/xoauth2-protocol

use base64::prelude::*;

/// XOAUTH2 token for IMAP/SMTP authentication
#[derive(Debug, Clone)]
pub struct XOAuth2Token {
    /// Email address
    email: String,
    /// OAuth2 access token
    access_token: String,
}

impl XOAuth2Token {
    /// Create a new XOAUTH2 token
    pub fn new(email: &str, access_token: &str) -> Self {
        Self {
            email: email.to_string(),
            access_token: access_token.to_string(),
        }
    }

    /// Get the email address
    pub fn email(&self) -> &str {
        &self.email
    }

    /// Get the access token
    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// Generate the XOAUTH2 authentication string
    ///
    /// Format: "user={email}\x01auth=Bearer {token}\x01\x01"
    pub fn auth_string(&self) -> String {
        format!(
            "user={}\x01auth=Bearer {}\x01\x01",
            self.email, self.access_token
        )
    }

    /// Generate the base64-encoded XOAUTH2 authentication string
    ///
    /// This is what gets sent to the IMAP/SMTP server
    pub fn auth_string_base64(&self) -> String {
        BASE64_STANDARD.encode(self.auth_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xoauth2_format() {
        let token = XOAuth2Token::new("user@gmail.com", "ya29.access_token_here");

        // Check raw format
        let auth_string = token.auth_string();
        assert_eq!(
            auth_string,
            "user=user@gmail.com\x01auth=Bearer ya29.access_token_here\x01\x01"
        );

        // Verify base64 encoding works
        let encoded = token.auth_string_base64();
        let decoded = String::from_utf8(BASE64_STANDARD.decode(&encoded).unwrap()).unwrap();
        assert_eq!(decoded, auth_string);
    }
}
