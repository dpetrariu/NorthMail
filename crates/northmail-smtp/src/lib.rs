//! SMTP implementation for NorthMail
//!
//! Provides email sending via SMTP with XOAUTH2 support for Gmail,
//! and Microsoft Graph API sending for Outlook/Exchange accounts.

mod client;
mod error;
pub mod msgraph;

pub use client::{build_lettre_message, OutgoingAttachment, OutgoingMessage, SmtpClient};
pub use error::{SmtpError, SmtpResult};
