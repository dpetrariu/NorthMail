//! Test GOA account detection and auth types
//!
//! Run with: cargo run -p northmail-auth --example test_goa

use northmail_auth::{AuthManager, GoaAuthType};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use tokio for async runtime
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            run_test().await
        })
}

async fn run_test() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing GOA account detection...\n");

    let auth_manager = AuthManager::new().await?;

    if !auth_manager.is_goa_available() {
        println!("GOA is not available!");
        return Ok(());
    }

    let accounts = auth_manager.list_goa_accounts().await?;
    println!("Found {} accounts:\n", accounts.len());

    for account in &accounts {
        println!("Account: {}", account.email);
        println!("  Provider: {} ({})", account.provider_name, account.provider_type);
        println!("  Auth type: {:?}", account.auth_type);
        println!("  IMAP host: {:?}", account.imap_host);
        println!("  IMAP username: {:?}", account.imap_username);

        match account.auth_type {
            GoaAuthType::OAuth2 => {
                println!("  Testing OAuth2 token retrieval...");
                match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                    Ok((email, token)) => {
                        println!("    SUCCESS: Got token for {} ({} chars)", email, token.len());
                    }
                    Err(e) => {
                        println!("    FAILED: {}", e);
                    }
                }
            }
            GoaAuthType::Password => {
                println!("  Testing password retrieval...");
                match auth_manager.get_goa_password(&account.id).await {
                    Ok(password) => {
                        println!("    SUCCESS: Got password ({} chars)", password.len());
                    }
                    Err(e) => {
                        println!("    FAILED: {}", e);
                    }
                }
            }
            GoaAuthType::Unknown => {
                println!("  Unknown auth type - skipping test");
            }
        }
        println!();
    }

    Ok(())
}
