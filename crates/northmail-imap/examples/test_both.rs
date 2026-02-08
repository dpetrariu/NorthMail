//! Test both Gmail and iCloud accounts
use northmail_auth::{AuthManager, GoaAuthType};
use northmail_imap::{ImapClient, SimpleImapClient};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    async_std::task::block_on(async { run_test().await })
}

async fn run_test() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing both accounts...\n");
    
    let auth_manager = AuthManager::new().await?;
    let accounts = auth_manager.list_goa_accounts().await?;
    
    for account in &accounts {
        println!("Testing: {} ({})", account.email, account.provider_type);
        
        match account.auth_type {
            GoaAuthType::OAuth2 if account.provider_type == "google" => {
                // Test Gmail with SimpleImapClient
                match auth_manager.get_xoauth2_token_for_goa(&account.id).await {
                    Ok((email, token)) => {
                        println!("  Got OAuth2 token");
                        
                        let mut client = SimpleImapClient::new();
                        match client.connect_gmail(&email, &token).await {
                            Ok(_) => {
                                println!("  Connected to Gmail!");
                                match client.select("INBOX").await {
                                    Ok(folder) => {
                                        println!("  INBOX has {} messages", folder.message_count.unwrap_or(0));
                                    }
                                    Err(e) => println!("  Select error: {}", e),
                                }
                                let _ = client.logout().await;
                            }
                            Err(e) => println!("  Connect error: {}", e),
                        }
                    }
                    Err(e) => println!("  Token error: {}", e),
                }
            }
            GoaAuthType::Password => {
                // Test iCloud with ImapClient
                match auth_manager.get_goa_password(&account.id).await {
                    Ok(password) => {
                        println!("  Got password");
                        
                        let host = account.imap_host.clone().unwrap_or("imap.mail.me.com".to_string());
                        let username = account.imap_username.clone().unwrap_or(account.email.clone());
                        
                        let mut client = ImapClient::new(&host, 993);
                        match client.authenticate_login(&username, &password).await {
                            Ok(_) => {
                                println!("  Connected to iCloud!");
                                match client.select_folder("INBOX").await {
                                    Ok(folder) => {
                                        println!("  INBOX has {} messages", folder.message_count.unwrap_or(0));
                                    }
                                    Err(e) => println!("  Select error: {}", e),
                                }
                                let _ = client.logout().await;
                            }
                            Err(e) => println!("  Connect error: {}", e),
                        }
                    }
                    Err(e) => println!("  Password error: {}", e),
                }
            }
            _ => println!("  Skipping (other provider)"),
        }
        println!();
    }
    
    Ok(())
}
