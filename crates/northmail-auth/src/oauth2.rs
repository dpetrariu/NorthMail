//! Standalone OAuth2 with PKCE support
//!
//! This is the fallback authentication method for non-GNOME environments.
//! It implements the OAuth2 authorization code flow with PKCE (RFC 7636).

use crate::{AuthError, AuthResult};
use oauth2::{
    basic::BasicClient, AuthUrl, AuthorizationCode, ClientId, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use tracing::{debug, info};

/// OAuth2 provider configuration
#[derive(Debug, Clone)]
pub struct OAuth2Config {
    /// OAuth2 client ID
    pub client_id: String,
    /// OAuth2 client secret (optional for native apps using PKCE)
    pub client_secret: Option<String>,
    /// Authorization endpoint URL
    pub auth_url: String,
    /// Token endpoint URL
    pub token_url: String,
    /// Required scopes
    pub scopes: Vec<String>,
    /// Local port for OAuth2 callback
    pub redirect_port: u16,
}

/// Token pair containing access and refresh tokens
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenPair {
    /// Access token for API calls
    pub access_token: String,
    /// Refresh token for obtaining new access tokens
    pub refresh_token: Option<String>,
    /// Token expiration timestamp (Unix seconds)
    pub expires_at: Option<i64>,
}

impl TokenPair {
    /// Check if the access token is expired or about to expire
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(expires_at) => {
                let now = chrono::Utc::now().timestamp();
                // Consider expired if less than 5 minutes remaining
                expires_at - now < 300
            }
            None => false,
        }
    }
}

/// OAuth2 provider presets
#[derive(Debug, Clone, Copy)]
pub enum OAuth2Provider {
    Gmail,
}

impl OAuth2Provider {
    /// Get default configuration for a provider
    pub fn config(&self, client_id: &str) -> OAuth2Config {
        match self {
            OAuth2Provider::Gmail => crate::gmail::oauth2_config(client_id),
        }
    }
}

/// Manages an OAuth2 authorization flow
pub struct OAuth2Flow {
    config: OAuth2Config,
    client: BasicClient,
    pkce_verifier: Option<PkceCodeVerifier>,
    csrf_token: Option<CsrfToken>,
}

impl OAuth2Flow {
    /// Create a new OAuth2 flow
    pub fn new(config: OAuth2Config) -> AuthResult<Self> {
        let client_id = ClientId::new(config.client_id.clone());
        let auth_url = AuthUrl::new(config.auth_url.clone())
            .map_err(|e| AuthError::InvalidConfig(format!("Invalid auth URL: {}", e)))?;
        let token_url = TokenUrl::new(config.token_url.clone())
            .map_err(|e| AuthError::InvalidConfig(format!("Invalid token URL: {}", e)))?;

        let redirect_url = RedirectUrl::new(format!(
            "http://127.0.0.1:{}/callback",
            config.redirect_port
        ))
        .map_err(|e| AuthError::InvalidConfig(format!("Invalid redirect URL: {}", e)))?;

        let client = BasicClient::new(client_id, None, auth_url, Some(token_url))
            .set_redirect_uri(redirect_url);

        Ok(Self {
            config,
            client,
            pkce_verifier: None,
            csrf_token: None,
        })
    }

    /// Generate the authorization URL for the user to visit
    pub fn get_auth_url(&mut self) -> String {
        // Generate PKCE challenge
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

        // Build authorization request
        let mut auth_request = self
            .client
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(pkce_challenge);

        // Add scopes
        for scope in &self.config.scopes {
            auth_request = auth_request.add_scope(Scope::new(scope.clone()));
        }

        let (auth_url, csrf_token) = auth_request.url();

        // Store verifier and CSRF token for later use
        self.pkce_verifier = Some(pkce_verifier);
        self.csrf_token = Some(csrf_token);

        auth_url.to_string()
    }

    /// Wait for the OAuth2 callback and exchange the code for tokens
    ///
    /// This starts a local HTTP server to receive the callback from the
    /// OAuth2 provider after the user authorizes the application.
    pub async fn wait_for_callback(&mut self) -> AuthResult<TokenPair> {
        let pkce_verifier = self
            .pkce_verifier
            .take()
            .ok_or_else(|| AuthError::InvalidConfig("Auth URL not generated".to_string()))?;

        let csrf_token = self
            .csrf_token
            .take()
            .ok_or_else(|| AuthError::InvalidConfig("Auth URL not generated".to_string()))?;

        // Start local server to receive callback
        let listener = TcpListener::bind(format!("127.0.0.1:{}", self.config.redirect_port))
            .map_err(|e| AuthError::CallbackServerFailed(e.to_string()))?;

        info!(
            "Listening for OAuth2 callback on port {}",
            self.config.redirect_port
        );

        // Set a timeout for the callback
        listener
            .set_nonblocking(false)
            .map_err(|e| AuthError::CallbackServerFailed(e.to_string()))?;

        // Wait for a connection
        let (mut stream, _) = listener
            .accept()
            .map_err(|e| AuthError::CallbackServerFailed(e.to_string()))?;

        // Read the HTTP request
        let mut reader = BufReader::new(&stream);
        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .map_err(|e| AuthError::CallbackServerFailed(e.to_string()))?;

        debug!("Received callback request: {}", request_line.trim());

        // Parse the authorization code from the URL
        let (code, state) = parse_callback_url(&request_line)?;

        // Verify CSRF token
        if state != *csrf_token.secret() {
            // Send error response
            send_http_response(&mut stream, "Error", "Invalid state parameter");
            return Err(AuthError::AuthorizationFailed(
                "CSRF token mismatch".to_string(),
            ));
        }

        // Send success response to browser
        send_http_response(
            &mut stream,
            "Success",
            "You can close this window and return to NorthMail.",
        );

        // Exchange authorization code for tokens
        let token_response = self
            .client
            .exchange_code(AuthorizationCode::new(code))
            .set_pkce_verifier(pkce_verifier)
            .request_async(oauth2::reqwest::async_http_client)
            .await
            .map_err(|e| AuthError::TokenExchangeFailed(e.to_string()))?;

        // Calculate expiration time
        let expires_at = token_response.expires_in().map(|duration| {
            chrono::Utc::now().timestamp() + duration.as_secs() as i64
        });

        Ok(TokenPair {
            access_token: token_response.access_token().secret().clone(),
            refresh_token: token_response.refresh_token().map(|t| t.secret().clone()),
            expires_at,
        })
    }

    /// Refresh an access token using a refresh token
    pub async fn refresh_token(&self, refresh_token: &str) -> AuthResult<TokenPair> {
        let token_response = self
            .client
            .exchange_refresh_token(&oauth2::RefreshToken::new(refresh_token.to_string()))
            .request_async(oauth2::reqwest::async_http_client)
            .await
            .map_err(|e| AuthError::TokenExchangeFailed(e.to_string()))?;

        let expires_at = token_response.expires_in().map(|duration| {
            chrono::Utc::now().timestamp() + duration.as_secs() as i64
        });

        Ok(TokenPair {
            access_token: token_response.access_token().secret().clone(),
            refresh_token: token_response
                .refresh_token()
                .map(|t| t.secret().clone())
                .or_else(|| Some(refresh_token.to_string())),
            expires_at,
        })
    }
}

/// Parse the authorization code and state from a callback URL
fn parse_callback_url(request_line: &str) -> AuthResult<(String, String)> {
    // Request line format: "GET /callback?code=xxx&state=yyy HTTP/1.1"
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(AuthError::AuthorizationFailed(
            "Invalid callback request".to_string(),
        ));
    }

    let path = parts[1];
    let url = url::Url::parse(&format!("http://localhost{}", path))
        .map_err(|e| AuthError::AuthorizationFailed(format!("Invalid callback URL: {}", e)))?;

    let mut code = None;
    let mut state = None;

    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.to_string()),
            "state" => state = Some(value.to_string()),
            "error" => {
                let description = url
                    .query_pairs()
                    .find(|(k, _)| k == "error_description")
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_else(|| value.to_string());
                return Err(AuthError::AuthorizationFailed(description));
            }
            _ => {}
        }
    }

    match (code, state) {
        (Some(c), Some(s)) => Ok((c, s)),
        _ => Err(AuthError::AuthorizationFailed(
            "Missing code or state in callback".to_string(),
        )),
    }
}

/// Send an HTTP response to the browser
fn send_http_response(stream: &mut std::net::TcpStream, title: &str, message: &str) {
    let body = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>{} - NorthMail</title>
    <style>
        body {{
            font-family: system-ui, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            margin: 0;
            background: #fafafa;
        }}
        .container {{
            text-align: center;
            padding: 2rem;
            background: white;
            border-radius: 8px;
            box-shadow: 0 2px 8px rgba(0,0,0,0.1);
        }}
        h1 {{ color: #333; }}
        p {{ color: #666; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>{}</h1>
        <p>{}</p>
    </div>
</body>
</html>"#,
        title, title, message
    );

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );

    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_pair_expiration() {
        // Token that expires in 1 hour - should not be expired
        let token = TokenPair {
            access_token: "test".to_string(),
            refresh_token: None,
            expires_at: Some(chrono::Utc::now().timestamp() + 3600),
        };
        assert!(!token.is_expired());

        // Token that expires in 2 minutes - should be expired (less than 5 min buffer)
        let token = TokenPair {
            access_token: "test".to_string(),
            refresh_token: None,
            expires_at: Some(chrono::Utc::now().timestamp() + 120),
        };
        assert!(token.is_expired());

        // Token that already expired
        let token = TokenPair {
            access_token: "test".to_string(),
            refresh_token: None,
            expires_at: Some(chrono::Utc::now().timestamp() - 100),
        };
        assert!(token.is_expired());
    }
}
