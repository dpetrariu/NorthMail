//! IMAP folder types and operations

/// Type of email folder
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderType {
    /// Inbox folder
    Inbox,
    /// Sent messages
    Sent,
    /// Draft messages
    Drafts,
    /// Trash/deleted messages
    Trash,
    /// Spam/junk
    Spam,
    /// Archive
    Archive,
    /// User-created folder
    Other,
}

impl FolderType {
    /// Detect folder type from Gmail special-use attributes or name
    pub fn from_attributes_and_name(attributes: &[String], name: &str) -> Self {
        // Check special-use attributes first (RFC 6154)
        for attr in attributes {
            match attr.to_lowercase().as_str() {
                "\\inbox" => return FolderType::Inbox,
                "\\sent" => return FolderType::Sent,
                "\\drafts" => return FolderType::Drafts,
                "\\trash" => return FolderType::Trash,
                "\\junk" => return FolderType::Spam,
                "\\archive" | "\\all" => return FolderType::Archive,
                _ => {}
            }
        }

        // Fall back to name-based detection for Gmail
        let name_lower = name.to_lowercase();
        if name_lower == "inbox" {
            FolderType::Inbox
        } else if name_lower.contains("sent") {
            FolderType::Sent
        } else if name_lower.contains("draft") {
            FolderType::Drafts
        } else if name_lower.contains("trash") || name_lower.contains("bin") {
            FolderType::Trash
        } else if name_lower.contains("spam") || name_lower.contains("junk") {
            FolderType::Spam
        } else if name_lower.contains("archive") || name_lower.contains("all mail") {
            FolderType::Archive
        } else {
            FolderType::Other
        }
    }
}

/// Represents an IMAP folder/mailbox
#[derive(Debug, Clone)]
pub struct Folder {
    /// Folder name (display name)
    pub name: String,
    /// Full path including hierarchy delimiter
    pub full_path: String,
    /// Folder type
    pub folder_type: FolderType,
    /// Hierarchy delimiter (e.g., "/" for Gmail)
    pub delimiter: Option<char>,
    /// Special-use attributes
    pub attributes: Vec<String>,
    /// UIDVALIDITY value
    pub uidvalidity: Option<u32>,
    /// Number of messages
    pub message_count: Option<u32>,
    /// Number of unread messages
    pub unread_count: Option<u32>,
    /// Highest UID in folder
    pub uid_next: Option<u32>,
}

impl Folder {
    /// Create a new folder from IMAP LIST response
    pub fn new(name: String, full_path: String, delimiter: Option<char>, attributes: Vec<String>) -> Self {
        let folder_type = FolderType::from_attributes_and_name(&attributes, &name);

        Self {
            name,
            full_path,
            folder_type,
            delimiter,
            attributes,
            uidvalidity: None,
            message_count: None,
            unread_count: None,
            uid_next: None,
        }
    }

    /// Check if this folder can be selected
    pub fn is_selectable(&self) -> bool {
        !self.attributes.iter().any(|a| {
            let lower = a.to_lowercase();
            lower == "\\noselect" || lower == "\\nonexistent"
        })
    }

    /// Check if this folder has children
    pub fn has_children(&self) -> bool {
        self.attributes.iter().any(|a| a.to_lowercase() == "\\haschildren")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_folder_type_detection() {
        // Special-use attributes
        assert_eq!(
            FolderType::from_attributes_and_name(&["\\Sent".to_string()], "Sent"),
            FolderType::Sent
        );

        // Name-based for Gmail
        assert_eq!(
            FolderType::from_attributes_and_name(&[], "[Gmail]/Sent Mail"),
            FolderType::Sent
        );

        assert_eq!(
            FolderType::from_attributes_and_name(&[], "INBOX"),
            FolderType::Inbox
        );
    }
}
