//! XOAUTH2 authenticator for async-imap

use base64::prelude::*;

/// XOAUTH2 authenticator for async-imap
///
/// Implements the SASL XOAUTH2 mechanism for Gmail IMAP authentication.
#[derive(Debug, Clone)]
pub struct XOAuth2Authenticator {
    /// Email address
    email: String,
    /// OAuth2 access token
    access_token: String,
}

impl XOAuth2Authenticator {
    /// Create a new XOAUTH2 authenticator
    pub fn new(email: impl Into<String>, access_token: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            access_token: access_token.into(),
        }
    }

    /// Generate the XOAUTH2 authentication string
    ///
    /// Format: "user={email}\x01auth=Bearer {token}\x01\x01"
    fn auth_string(&self) -> String {
        format!(
            "user={}\x01auth=Bearer {}\x01\x01",
            self.email, self.access_token
        )
    }

    /// Get the base64-encoded authentication response
    pub fn response(&self) -> String {
        BASE64_STANDARD.encode(self.auth_string())
    }
}

/// Implement the async-imap Authenticator trait
impl async_imap::Authenticator for XOAuth2Authenticator {
    type Response = String;

    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        self.response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xoauth2_response() {
        let auth = XOAuth2Authenticator::new("user@gmail.com", "ya29.test_token");

        // Verify the auth string format
        let expected = "user=user@gmail.com\x01auth=Bearer ya29.test_token\x01\x01";
        assert_eq!(auth.auth_string(), expected);

        // Verify base64 encoding
        let encoded = auth.response();
        let decoded = String::from_utf8(BASE64_STANDARD.decode(&encoded).unwrap()).unwrap();
        assert_eq!(decoded, expected);
    }
}
