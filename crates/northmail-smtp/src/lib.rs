//! SMTP implementation for NorthMail
//!
//! Provides email sending via SMTP with XOAUTH2 support for Gmail.

mod client;
mod error;

pub use client::SmtpClient;
pub use error::{SmtpError, SmtpResult};
