//! Test Gmail IMAP connection
//!
//! Run with: cargo run --example test_gmail

use northmail_auth::AuthManager;
use std::time::Duration;
use async_std::io::prelude::*;
use async_std::io::BufReader;

#[async_std::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    println!("=== Gmail IMAP Connection Test ===\n");

    // Step 1: Get GOA accounts
    println!("Step 1: Getting GOA accounts...");
    let auth_manager = AuthManager::new().await?;

    if !auth_manager.is_goa_available() {
        println!("ERROR: GOA is not available");
        return Ok(());
    }

    let accounts = auth_manager.list_goa_accounts().await?;
    println!("Found {} accounts:", accounts.len());

    let gmail_account = accounts.iter().find(|a| a.provider_type == "google");

    let account = match gmail_account {
        Some(a) => {
            println!("  Using: {} ({})", a.email, a.provider_name);
            a
        }
        None => {
            println!("ERROR: No Google account found");
            return Ok(());
        }
    };

    // Step 2: Get OAuth2 token
    println!("\nStep 2: Getting OAuth2 token...");
    let (email, access_token) = auth_manager
        .get_xoauth2_token_for_goa(&account.id)
        .await?;
    println!("  Got token for: {}", email);
    println!("  Token length: {} chars", access_token.len());

    // Step 3: Connect to IMAP manually with raw stream debugging
    println!("\nStep 3: Connecting to imap.gmail.com:993...");

    use async_std::net::TcpStream;
    use async_native_tls::TlsConnector;

    let start = std::time::Instant::now();

    // TCP connection
    println!("  Creating TCP connection...");
    let tcp_stream = TcpStream::connect("imap.gmail.com:993").await?;
    println!("  TCP connected in {:?}", start.elapsed());

    // TLS handshake
    println!("  Performing TLS handshake...");
    let tls_connector = TlsConnector::new();
    let tls_stream = tls_connector.connect("imap.gmail.com", tcp_stream).await?;
    println!("  TLS handshake complete in {:?}", start.elapsed());

    // Wrap in BufReader for line reading
    let mut stream = BufReader::new(tls_stream);

    // Read server greeting manually
    println!("\nStep 4: Reading server greeting...");
    let greeting_result = async_std::future::timeout(
        Duration::from_secs(5),
        async {
            let mut line = String::new();
            stream.read_line(&mut line).await?;
            Ok::<_, std::io::Error>(line)
        }
    ).await;

    match greeting_result {
        Ok(Ok(greeting)) => {
            println!("  Greeting: {}", greeting.trim());
        }
        Ok(Err(e)) => {
            println!("  ERROR reading greeting: {}", e);
            return Ok(());
        }
        Err(_) => {
            println!("  ERROR: Greeting timed out after 5s");
            return Ok(());
        }
    }

    // Step 5: Send CAPABILITY command
    println!("\nStep 5: Sending CAPABILITY command...");
    stream.get_mut().write_all(b"a001 CAPABILITY\r\n").await?;

    let cap_result = async_std::future::timeout(
        Duration::from_secs(5),
        async {
            let mut lines = Vec::new();
            loop {
                let mut line = String::new();
                stream.read_line(&mut line).await?;
                let is_done = line.starts_with("a001 ");
                lines.push(line);
                if is_done {
                    break;
                }
            }
            Ok::<_, std::io::Error>(lines)
        }
    ).await;

    match cap_result {
        Ok(Ok(lines)) => {
            for line in lines {
                println!("  {}", line.trim());
            }
        }
        Ok(Err(e)) => {
            println!("  ERROR: {}", e);
            return Ok(());
        }
        Err(_) => {
            println!("  ERROR: CAPABILITY timed out");
            return Ok(());
        }
    }

    // Step 6: Send AUTHENTICATE XOAUTH2
    println!("\nStep 6: Authenticating with XOAUTH2...");
    let auth_string = format!("user={}\x01auth=Bearer {}\x01\x01", email, access_token);
    let encoded = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, &auth_string);

    let auth_cmd = format!("a002 AUTHENTICATE XOAUTH2 {}\r\n", encoded);
    stream.get_mut().write_all(auth_cmd.as_bytes()).await?;

    // Read until we get a002 response
    let auth_result = async_std::future::timeout(
        Duration::from_secs(5),
        async {
            let mut lines = Vec::new();
            loop {
                let mut line = String::new();
                stream.read_line(&mut line).await?;
                let is_done = line.starts_with("a002 ");
                lines.push(line);
                if is_done {
                    break;
                }
            }
            Ok::<_, std::io::Error>(lines)
        }
    ).await;

    match auth_result {
        Ok(Ok(lines)) => {
            for line in &lines {
                println!("  {}", line.trim());
            }
            let last_line = lines.last().map(|s| s.as_str()).unwrap_or("");
            if last_line.contains("OK") {
                println!("  SUCCESS! Authentication worked!");

                // Try SELECT INBOX
                println!("\nStep 7: Selecting INBOX...");
                stream.get_mut().write_all(b"a003 SELECT INBOX\r\n").await?;

                // Read response
                loop {
                    let read_result = async_std::future::timeout(
                        Duration::from_secs(5),
                        async {
                            let mut line = String::new();
                            stream.read_line(&mut line).await?;
                            Ok::<_, std::io::Error>(line)
                        }
                    ).await;

                    match read_result {
                        Ok(Ok(line)) => {
                            println!("  {}", line.trim());
                            if line.starts_with("a003 ") {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
            } else if last_line.contains("NO") || last_line.contains("BAD") {
                println!("  FAILED! Server rejected authentication");
            }
        }
        Ok(Err(e)) => {
            println!("  ERROR: {}", e);
        }
        Err(_) => {
            println!("  ERROR: Authentication response timed out");
        }
    }

    // Logout
    println!("\nLogging out...");
    let _ = stream.get_mut().write_all(b"a999 LOGOUT\r\n").await;

    println!("\n=== Test Complete ===");
    Ok(())
}
