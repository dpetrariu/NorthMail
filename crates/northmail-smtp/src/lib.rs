//! SMTP implementation for NorthMail
//!
//! Provides email sending via SMTP with XOAUTH2 support for Gmail.

mod client;
mod error;

pub use client::{build_lettre_message, OutgoingAttachment, OutgoingMessage, SmtpClient};
pub use error::{SmtpError, SmtpResult};
