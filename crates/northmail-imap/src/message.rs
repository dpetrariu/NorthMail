//! IMAP message types

use std::collections::HashSet;

/// Email message flags
#[derive(Debug, Clone, Default)]
pub struct MessageFlags {
    /// Message has been read
    pub seen: bool,
    /// Message has been answered
    pub answered: bool,
    /// Message is flagged/starred
    pub flagged: bool,
    /// Message is marked for deletion
    pub deleted: bool,
    /// Message is a draft
    pub draft: bool,
    /// Custom flags (Gmail labels, etc.)
    pub custom: HashSet<String>,
}

impl MessageFlags {
    /// Parse flags from IMAP FETCH response
    pub fn from_imap_flags(flags: &[&str]) -> Self {
        let mut result = MessageFlags::default();

        for flag in flags {
            match flag.to_lowercase().as_str() {
                "\\seen" => result.seen = true,
                "\\answered" => result.answered = true,
                "\\flagged" => result.flagged = true,
                "\\deleted" => result.deleted = true,
                "\\draft" => result.draft = true,
                other => {
                    result.custom.insert(other.to_string());
                }
            }
        }

        result
    }

    /// Convert to IMAP flag strings for STORE command
    pub fn to_imap_flags(&self) -> Vec<String> {
        let mut flags = Vec::new();

        if self.seen {
            flags.push("\\Seen".to_string());
        }
        if self.answered {
            flags.push("\\Answered".to_string());
        }
        if self.flagged {
            flags.push("\\Flagged".to_string());
        }
        if self.deleted {
            flags.push("\\Deleted".to_string());
        }
        if self.draft {
            flags.push("\\Draft".to_string());
        }

        flags.extend(self.custom.iter().cloned());
        flags
    }
}

/// Email address with optional display name
#[derive(Debug, Clone)]
pub struct EmailAddress {
    /// Display name (e.g., "John Doe")
    pub name: Option<String>,
    /// Email address (e.g., "john@example.com")
    pub address: String,
}

impl EmailAddress {
    pub fn new(name: Option<String>, address: String) -> Self {
        Self { name, address }
    }

    /// Format as "Name <address>" or just "address"
    pub fn to_display_string(&self) -> String {
        match &self.name {
            Some(name) if !name.is_empty() => format!("{} <{}>", name, self.address),
            _ => self.address.clone(),
        }
    }
}

impl std::fmt::Display for EmailAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_display_string())
    }
}

/// Envelope data from IMAP FETCH
#[derive(Debug, Clone)]
pub struct Envelope {
    /// Message-ID header
    pub message_id: Option<String>,
    /// Subject line
    pub subject: Option<String>,
    /// From addresses
    pub from: Vec<EmailAddress>,
    /// To addresses
    pub to: Vec<EmailAddress>,
    /// CC addresses
    pub cc: Vec<EmailAddress>,
    /// Reply-To addresses
    pub reply_to: Vec<EmailAddress>,
    /// Date sent
    pub date: Option<String>,
    /// In-Reply-To header
    pub in_reply_to: Option<String>,
}

impl Default for Envelope {
    fn default() -> Self {
        Self {
            message_id: None,
            subject: None,
            from: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
            reply_to: Vec::new(),
            date: None,
            in_reply_to: None,
        }
    }
}

/// Message header information from IMAP
#[derive(Debug, Clone)]
pub struct MessageHeader {
    /// Server-assigned UID
    pub uid: u32,
    /// Message sequence number (can change)
    pub seq: u32,
    /// Envelope data
    pub envelope: Envelope,
    /// Message flags
    pub flags: MessageFlags,
    /// Size in bytes
    pub size: u32,
    /// Body structure (for attachment detection)
    pub has_attachments: bool,
}

impl MessageHeader {
    /// Get the subject, with a default for empty
    pub fn subject(&self) -> &str {
        self.envelope
            .subject
            .as_deref()
            .unwrap_or("(No subject)")
    }

    /// Get the primary sender's display string
    pub fn from_display(&self) -> String {
        self.envelope
            .from
            .first()
            .map(|a| a.to_display_string())
            .unwrap_or_else(|| "(Unknown sender)".to_string())
    }

    /// Check if message is read
    pub fn is_read(&self) -> bool {
        self.flags.seen
    }

    /// Check if message is starred/flagged
    pub fn is_starred(&self) -> bool {
        self.flags.flagged
    }
}
