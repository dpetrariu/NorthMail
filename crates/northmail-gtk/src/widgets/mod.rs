//! Custom widgets for NorthMail

mod folder_sidebar;
mod message_list;
mod message_view;

pub use folder_sidebar::{AccountFolders, FolderInfo, FolderSidebar};
pub use message_list::{MessageInfo, MessageList};
pub use message_view::MessageView;
#[cfg(feature = "webkit")]
pub use message_view::{ensure_uri_schemes_registered, rewrite_links_for_external_open};
