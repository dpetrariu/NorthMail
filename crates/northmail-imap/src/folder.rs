//! IMAP folder types and operations

/// Type of email folder
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    /// Detect folder type from IMAP special-use attributes only (RFC 6154)
    /// Also matches without backslash prefix (some servers send "Trash" instead of "\Trash")
    pub fn from_attributes(attributes: &[String]) -> Option<Self> {
        for attr in attributes {
            let lower = attr.to_lowercase();
            // Strip leading backslash for matching
            let normalized = lower.trim_start_matches('\\');
            match normalized {
                "inbox" => return Some(FolderType::Inbox),
                "sent" => return Some(FolderType::Sent),
                "drafts" => return Some(FolderType::Drafts),
                "trash" => return Some(FolderType::Trash),
                "junk" => return Some(FolderType::Spam),
                "archive" | "all" => return Some(FolderType::Archive),
                _ => {}
            }
        }
        None
    }

    /// Detect folder type from name only (fallback when no attributes)
    pub fn from_name(name: &str) -> Self {
        let name_lower = name.to_lowercase();
        if name_lower == "inbox" {
            FolderType::Inbox
        } else if name_lower.contains("sent") {
            FolderType::Sent
        } else if name_lower.contains("draft") {
            FolderType::Drafts
        } else if name_lower.contains("trash") || name_lower.contains("bin") || name_lower.contains("deleted") {
            FolderType::Trash
        } else if name_lower.contains("spam") || name_lower.contains("junk") {
            FolderType::Spam
        } else if name_lower.contains("archive") || name_lower.contains("all mail") {
            FolderType::Archive
        } else {
            FolderType::Other
        }
    }

    /// Detect folder type from attributes first, then fall back to name.
    /// Use this for single-folder detection (e.g., cached folders without attributes).
    pub fn from_attributes_and_name(attributes: &[String], name: &str) -> Self {
        Self::from_attributes(attributes).unwrap_or_else(|| Self::from_name(name))
    }

    /// Deduplicate folder types across a list of folders.
    /// Attribute-detected types always win. Name-based detection only applies
    /// if no folder already claimed that type via IMAP special-use attributes.
    /// When no attributes exist, only the first folder per type is kept.
    pub fn deduplicate_folder_types(folders: &mut [Folder]) {
        use std::collections::HashSet;

        // First pass: collect types claimed by attribute detection
        let mut attr_types = HashSet::new();
        for folder in folders.iter() {
            if Self::from_attributes(&folder.attributes).is_some() {
                attr_types.insert(folder.folder_type.clone());
            }
        }

        // Second pass: downgrade name-only detections that conflict with attribute ones
        for folder in folders.iter_mut() {
            if folder.folder_type == FolderType::Other || folder.folder_type == FolderType::Inbox {
                continue;
            }
            if Self::from_attributes(&folder.attributes).is_none()
                && attr_types.contains(&folder.folder_type)
            {
                folder.folder_type = FolderType::Other;
            }
        }

        // Third pass: for types with NO attribute-based winner, keep only the first
        // name-detected folder per type (prevents 3x Trash, 3x Sent, etc.)
        let mut seen_name_types = HashSet::new();
        for folder in folders.iter_mut() {
            if folder.folder_type == FolderType::Other || folder.folder_type == FolderType::Inbox {
                continue;
            }
            // Skip attribute-detected folders — they already won
            if Self::from_attributes(&folder.attributes).is_some() {
                continue;
            }
            // For name-only detected folders, keep only the first per type
            if !seen_name_types.insert(folder.folder_type.clone()) {
                folder.folder_type = FolderType::Other;
            }
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

    #[test]
    fn test_deduplication() {
        let mut folders = vec![
            Folder::new("Trash".into(), "Trash".into(), Some('/'), vec!["\\Trash".into()]),
            Folder::new("Deleted Items".into(), "Deleted Items".into(), Some('/'), vec![]),
            Folder::new("Deleted Messages".into(), "Deleted Messages".into(), Some('/'), vec![]),
        ];
        assert_eq!(folders[0].folder_type, FolderType::Trash);
        assert_eq!(folders[1].folder_type, FolderType::Trash); // name-based
        assert_eq!(folders[2].folder_type, FolderType::Trash); // name-based

        FolderType::deduplicate_folder_types(&mut folders);

        assert_eq!(folders[0].folder_type, FolderType::Trash); // attribute — kept
        assert_eq!(folders[1].folder_type, FolderType::Other); // name-only — downgraded
        assert_eq!(folders[2].folder_type, FolderType::Other); // name-only — downgraded
    }

    #[test]
    fn test_deduplication_no_attributes() {
        // When no folder has attributes, keep only the first per type
        let mut folders = vec![
            Folder::new("Sent".into(), "Sent".into(), Some('/'), vec![]),
            Folder::new("Sent Items".into(), "Sent Items".into(), Some('/'), vec![]),
            Folder::new("Sent Messages".into(), "Sent Messages".into(), Some('/'), vec![]),
            Folder::new("Trash".into(), "Trash".into(), Some('/'), vec![]),
            Folder::new("Deleted Items".into(), "Deleted Items".into(), Some('/'), vec![]),
        ];

        FolderType::deduplicate_folder_types(&mut folders);

        assert_eq!(folders[0].folder_type, FolderType::Sent);  // first Sent — kept
        assert_eq!(folders[1].folder_type, FolderType::Other);  // duplicate — downgraded
        assert_eq!(folders[2].folder_type, FolderType::Other);  // duplicate — downgraded
        assert_eq!(folders[3].folder_type, FolderType::Trash);  // first Trash — kept
        assert_eq!(folders[4].folder_type, FolderType::Other);  // duplicate — downgraded
    }
}
