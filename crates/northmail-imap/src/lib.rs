//! IMAP protocol implementation for NorthMail
//!
//! Provides async IMAP operations with XOAUTH2 support for Gmail.

mod client;
mod error;
mod folder;
mod message;
mod oauth2;
mod simple_client;

pub use client::ImapClient;
pub use error::{ImapError, ImapResult};
pub use folder::{Folder, FolderType};
pub use message::{Envelope, MessageFlags, MessageHeader};
pub use oauth2::XOAuth2Authenticator;
pub use simple_client::SimpleImapClient;
