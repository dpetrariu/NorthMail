//! Folder sidebar widget — single ListBox with header_func separators
//! and collapsible per-account folder sections.

use gtk4::{glib, prelude::*, subclass::prelude::*};
use std::cell::RefCell;
use std::collections::HashMap;

use crate::i18n::tr;

/// Row widget name encoding: "section:kind:account_id:folder_path"
/// Parsed with splitn(4, ':') so folder_path can contain ':'.
///
/// Sections:
///   0 — unified inbox
///   1 — per-account inboxes
///   2+ — per-account folder groups (2 = first account, 3 = second, …)
///
/// Kinds: unified, inbox, header, folder

fn encode_row_name(section: usize, kind: &str, account_id: &str, folder_path: &str) -> String {
    format!("{}:{}:{}:{}", section, kind, account_id, folder_path)
}

/// Format a number with thousand separators (e.g., 1234 -> "1,234")
fn format_number(n: u32) -> String {
    if n < 1000 {
        return n.to_string();
    }
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

/// Returns (section, kind, account_id, folder_path)
fn decode_row_name(name: &str) -> (usize, &str, &str, &str) {
    let mut parts = name.splitn(4, ':');
    let section = parts
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let kind = parts.next().unwrap_or("");
    let account_id = parts.next().unwrap_or("");
    let folder_path = parts.next().unwrap_or("");
    (section, kind, account_id, folder_path)
}

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use std::sync::OnceLock;

    #[derive(Default)]
    pub struct FolderSidebar {
        /// ListBox for the inboxes section (unified + per-account inboxes)
        pub inboxes_list_box: RefCell<Option<gtk4::ListBox>>,
        /// Container for the inboxes section (to toggle active/inactive style)
        pub inboxes_container: RefCell<Option<gtk4::Box>>,
        /// ListBox for the folders section (collapsible per-account folders)
        pub folders_list_box: RefCell<Option<gtk4::ListBox>>,
        pub accounts: RefCell<Vec<super::AccountFolders>>,
        /// Persisted expand/collapse state per account id.
        pub expanded_states: RefCell<HashMap<String, bool>>,
        // -- sync-status widgets (unchanged) --
        pub sync_status_box: RefCell<Option<gtk4::Box>>,
        pub sync_spinner: RefCell<Option<gtk4::Spinner>>,
        pub sync_label: RefCell<Option<gtk4::Label>>,
        pub sync_progress: RefCell<Option<gtk4::ProgressBar>>,
        pub sync_detail_label: RefCell<Option<gtk4::Label>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FolderSidebar {
        const NAME: &'static str = "NorthMailFolderSidebar";
        type Type = super::FolderSidebar;
        type ParentType = gtk4::Box;
    }

    impl ObjectImpl for FolderSidebar {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("folder-selected")
                        .param_types([
                            String::static_type(), // account_id
                            String::static_type(), // folder_path
                            bool::static_type(),   // is_unified
                        ])
                        .build(),
                    Signal::builder("message-dropped")
                        .param_types([
                            u32::static_type(),    // message uid
                            i64::static_type(),    // message id (db)
                            String::static_type(), // source account_id
                            String::static_type(), // source folder_path
                            String::static_type(), // target account_id
                            String::static_type(), // target folder_path
                        ])
                        .build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            obj.set_orientation(gtk4::Orientation::Vertical);
            obj.set_vexpand(true);
            obj.add_css_class("sidebar-pane");

            obj.setup_ui();
        }
    }

    impl WidgetImpl for FolderSidebar {}
    impl BoxImpl for FolderSidebar {}
}

glib::wrapper! {
    pub struct FolderSidebar(ObjectSubclass<imp::FolderSidebar>)
        @extends gtk4::Box, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Orientable;
}

impl FolderSidebar {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Connect to the folder-selected signal
    pub fn connect_folder_selected<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, &str, &str, bool) + 'static,
    {
        self.connect_closure(
            "folder-selected",
            false,
            glib::closure_local!(move |sidebar: &FolderSidebar,
                                       account_id: &str,
                                       folder_path: &str,
                                       is_unified: bool| {
                f(sidebar, account_id, folder_path, is_unified);
            }),
        )
    }

    /// Connect to the message-dropped signal (drag-and-drop move)
    pub fn connect_message_dropped<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, u32, i64, &str, &str, &str, &str) + 'static,
    {
        self.connect_closure(
            "message-dropped",
            false,
            glib::closure_local!(move |sidebar: &FolderSidebar,
                                       uid: u32,
                                       msg_id: i64,
                                       source_account_id: &str,
                                       source_folder_path: &str,
                                       target_account_id: &str,
                                       target_folder_path: &str| {
                f(sidebar, uid, msg_id, source_account_id, source_folder_path, target_account_id, target_folder_path);
            }),
        )
    }

    /// Parse drop data (single or multi) and emit message-dropped for each message.
    /// Returns true if at least one message was processed.
    fn handle_drop_data(&self, data: &str, target_account_id: &str, target_folder_path: &str) -> bool {
        if data.starts_with("multi|") {
            // Multi-message drop: "multi|uid:msg_id:acct:folder|uid:msg_id:acct:folder|..."
            let mut handled = false;
            for entry in data.split('|').skip(1) {
                let parts: Vec<&str> = entry.split(':').collect();
                if parts.len() >= 4 {
                    if let (Ok(uid), Ok(msg_id)) = (
                        parts[0].parse::<u32>(),
                        parts[1].parse::<i64>(),
                    ) {
                        let source_account_id = parts[2].to_string();
                        let source_folder_path = parts[3..].join(":");
                        tracing::debug!(
                            "Multi-drop message: uid={} from {}/{} to {}/{}",
                            uid, source_account_id, source_folder_path, target_account_id, target_folder_path
                        );
                        self.emit_by_name::<()>(
                            "message-dropped",
                            &[&uid, &msg_id, &source_account_id, &source_folder_path, &target_account_id.to_string(), &target_folder_path.to_string()],
                        );
                        handled = true;
                    }
                }
            }
            handled
        } else {
            // Single message: "uid:msg_id:source_account_id:source_folder_path"
            let parts: Vec<&str> = data.split(':').collect();
            if parts.len() >= 4 {
                if let (Ok(uid), Ok(msg_id)) = (
                    parts[0].parse::<u32>(),
                    parts[1].parse::<i64>(),
                ) {
                    let source_account_id = parts[2].to_string();
                    let source_folder_path = parts[3..].join(":");
                    tracing::debug!(
                        "Message dropped: uid={} from {}/{} to {}/{}",
                        uid, source_account_id, source_folder_path, target_account_id, target_folder_path
                    );
                    self.emit_by_name::<()>(
                        "message-dropped",
                        &[&uid, &msg_id, &source_account_id, &source_folder_path, &target_account_id.to_string(), &target_folder_path.to_string()],
                    );
                    return true;
                }
            }
            false
        }
    }

    // ── UI setup ─────────────────────────────────────────────────────

    fn setup_ui(&self) {
        let imp = self.imp();

        // Add CSS for sidebar styling
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string(
            "
            /* Inboxes section - accent background with white text */
            .inboxes-container {
                background-color: @accent_bg_color;
                border-radius: 12px;
                margin: 8px;
                padding: 4px;
                transition: background-color 150ms ease;
            }
            /* Inactive state when folder is selected */
            .inboxes-container.inactive {
                background-color: alpha(black, 0.08);
            }
            .inboxes-container.inactive .inboxes-list > row {
                color: @view_fg_color;
            }
            .inboxes-container.inactive .inboxes-list > row * {
                color: @view_fg_color;
            }
            .inboxes-container.inactive .inboxes-list > row .dim-label {
                color: alpha(@view_fg_color, 0.7);
            }
            .inboxes-container.inactive .inboxes-list separator {
                background-color: alpha(@view_fg_color, 0.2);
            }
            .inboxes-list {
                background: transparent;
            }
            .inboxes-list > row {
                border-radius: 8px;
                margin: 2px 4px;
                color: @accent_fg_color;
            }
            .inboxes-list > row * {
                color: @accent_fg_color;
            }
            .inboxes-list > row .dim-label {
                color: alpha(@accent_fg_color, 0.85);
            }
            /* Selected inbox: inverted (white bg, accent text) */
            .inboxes-list > row:selected {
                background-color: white;
                color: @accent_bg_color;
            }
            .inboxes-list > row:selected * {
                color: @accent_bg_color;
            }
            .inboxes-list > row:selected .dim-label {
                color: alpha(@accent_bg_color, 0.85);
            }
            /* Separator inside inboxes list */
            .inboxes-list separator {
                background-color: alpha(white, 0.4);
                min-height: 1px;
                margin-left: 8px;
                margin-right: 8px;
                margin-top: 4px;
                margin-bottom: 4px;
            }

            /* Folders section - transparent background */
            .folders-list {
                background: transparent;
            }
            .folders-list > row {
                background: transparent;
                border-radius: 8px;
                margin: 3px 6px;
            }
            .folders-list > row:selected {
                background-color: @accent_bg_color;
                color: @accent_fg_color;
                border-radius: 8px;
            }
            .folders-list > row:selected * {
                color: @accent_fg_color;
            }
            .folders-list > row:selected .dim-label,
            .folders-list > row:selected .caption {
                color: alpha(@accent_fg_color, 0.85);
            }
            /* Section header styling - smaller, non-bold */
            .folders-list .section-header-label {
                font-weight: normal;
                font-size: 0.9em;
            }
            /* Folder entries - smaller */
            .folders-list .folder-entry {
                font-size: 0.9em;
            }
            .folders-list .folder-entry-row {
                min-height: 0;
            }
            /* Separators in folders list - add margins */
            .folders-list separator {
                margin-left: 12px;
                margin-right: 12px;
                margin-top: 4px;
                margin-bottom: 4px;
            }
            /* Drop highlight for drag-and-drop - subtle background only */
            .folders-list > row.drop-highlight {
                background-color: alpha(@accent_bg_color, 0.25);
            }
            /* Drop highlight for inbox rows - more visible on accent background */
            .inboxes-list > row.drop-highlight {
                background-color: alpha(white, 0.4);
            }
            "
        );
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER + 1,
        );

        // ── Inboxes section (styled container) - fixed at top ──
        let inboxes_container = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .css_classes(["inboxes-container"])
            .build();

        let inboxes_list_box = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["inboxes-list"])
            .build();

        // Header func for separators between unified and per-account inboxes
        inboxes_list_box.set_header_func(|row, before| {
            let row_section = decode_row_name(&row.widget_name()).0;
            if let Some(before) = before {
                let before_section = decode_row_name(&before.widget_name()).0;
                if row_section != before_section {
                    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
                    row.set_header(Some(&sep));
                } else {
                    row.set_header(None::<&gtk4::Widget>);
                }
            } else {
                row.set_header(None::<&gtk4::Widget>);
            }
        });

        // Shared references for cross-list coordination
        let folders_list_cell = std::rc::Rc::new(RefCell::new(None::<gtk4::ListBox>));
        let inboxes_container_cell = std::rc::Rc::new(RefCell::new(inboxes_container.clone()));

        // Inboxes row activation handler
        let sidebar = self.clone();
        let folders_list_cell_clone = folders_list_cell.clone();
        let inboxes_container_for_inboxes = inboxes_container_cell.clone();
        inboxes_list_box.connect_row_activated(move |_list_box, row| {
            let name = row.widget_name();
            let (_section, kind, account_id, _folder_path) = decode_row_name(&name);

            // Deselect folders list when inbox is selected
            if let Some(ref folders_list) = *folders_list_cell_clone.borrow() {
                folders_list.unselect_all();
            }

            // Set inboxes container to active (accent color)
            inboxes_container_for_inboxes.borrow().remove_css_class("inactive");

            match kind {
                "unified" => {
                    sidebar.emit_by_name::<()>(
                        "folder-selected",
                        &[&"", &"INBOX", &true],
                    );
                }
                "inbox" => {
                    sidebar.emit_by_name::<()>(
                        "folder-selected",
                        &[&account_id, &"INBOX", &false],
                    );
                }
                _ => {}
            }
        });

        inboxes_container.append(&inboxes_list_box);
        self.append(&inboxes_container);

        // ── Folders section (collapsible per-account folders) ──
        let folders_list_box = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["folders-list"])
            .build();

        // Store reference for inboxes handler to deselect
        folders_list_cell.replace(Some(folders_list_box.clone()));

        // Header func for separators between account sections
        folders_list_box.set_header_func(|row, before| {
            let row_section = decode_row_name(&row.widget_name()).0;
            if let Some(before) = before {
                let before_section = decode_row_name(&before.widget_name()).0;
                if row_section != before_section {
                    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
                    row.set_header(Some(&sep));
                } else {
                    row.set_header(None::<&gtk4::Widget>);
                }
            } else {
                row.set_header(None::<&gtk4::Widget>);
            }
        });

        // Folders row activation handler
        let sidebar2 = self.clone();
        let inboxes_list_for_folders = inboxes_list_box.clone();
        let inboxes_container_for_folders = inboxes_container_cell.clone();
        folders_list_box.connect_row_activated(move |list_box, row| {
            let name = row.widget_name();
            let (_section, kind, account_id, folder_path) = decode_row_name(&name);

            match kind {
                "header" => {
                    // Toggle expansion — don't select header rows
                    list_box.unselect_row(row);
                    sidebar2.toggle_account_expansion(account_id);
                }
                "folder" => {
                    // Deselect inboxes list when folder is selected
                    inboxes_list_for_folders.unselect_all();

                    // Set inboxes container to inactive (grey)
                    inboxes_container_for_folders.borrow().add_css_class("inactive");

                    sidebar2.emit_by_name::<()>(
                        "folder-selected",
                        &[&account_id, &folder_path, &false],
                    );
                }
                _ => {}
            }
        });

        // Scrolled area for folders section only
        let scrolled = gtk4::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .build();

        scrolled.set_child(Some(&folders_list_box));
        self.append(&scrolled);

        // ── Bottom section (sync status + settings) ──
        let bottom_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .build();

        // Sync status area (hidden by default) - styled as a card for visibility
        let sync_status_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .margin_start(8)
            .margin_end(8)
            .margin_top(8)
            .margin_bottom(8)
            .visible(false)
            .css_classes(["card"])
            .build();

        let sync_inner = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .margin_start(12)
            .margin_end(12)
            .margin_top(10)
            .margin_bottom(10)
            .build();

        let sync_top_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .build();

        let sync_spinner = gtk4::Spinner::builder()
            .spinning(true)
            .width_request(16)
            .height_request(16)
            .build();

        let sync_label = gtk4::Label::builder()
            .label(&tr("Syncing..."))
            .css_classes(["caption"])
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        sync_top_row.append(&sync_spinner);
        sync_top_row.append(&sync_label);
        sync_inner.append(&sync_top_row);

        let sync_progress = gtk4::ProgressBar::builder()
            .show_text(false)
            .build();
        sync_inner.append(&sync_progress);
        sync_status_box.append(&sync_inner);

        let sync_detail_label = gtk4::Label::builder()
            .label("")
            .css_classes(["dim-label", "caption"])
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .visible(false)
            .build();
        sync_inner.append(&sync_detail_label);

        bottom_box.append(&sync_status_box);
        self.append(&bottom_box);

        // Store references
        imp.inboxes_list_box.replace(Some(inboxes_list_box));
        imp.inboxes_container.replace(Some(inboxes_container));
        imp.folders_list_box.replace(Some(folders_list_box));
        imp.sync_status_box.replace(Some(sync_status_box));
        imp.sync_spinner.replace(Some(sync_spinner));
        imp.sync_label.replace(Some(sync_label));
        imp.sync_progress.replace(Some(sync_progress));
        imp.sync_detail_label.replace(Some(sync_detail_label));
    }

    // ── Building the row list ────────────────────────────────────────

    /// Rebuild the sidebar content from account data.
    pub fn set_accounts(&self, accounts: Vec<AccountFolders>) {
        let imp = self.imp();
        imp.accounts.replace(accounts.clone());

        let inboxes_list = match imp.inboxes_list_box.borrow().as_ref() {
            Some(lb) => lb.clone(),
            None => return,
        };
        let folders_list = match imp.folders_list_box.borrow().as_ref() {
            Some(lb) => lb.clone(),
            None => return,
        };

        // Remember the currently selected row so we can restore it after rebuild
        let selected_name = inboxes_list.selected_row()
            .map(|row| row.widget_name().to_string())
            .or_else(|| folders_list.selected_row().map(|row| row.widget_name().to_string()));

        // Clear all rows from both lists
        while let Some(row) = inboxes_list.row_at_index(0) {
            inboxes_list.remove(&row);
        }
        while let Some(row) = folders_list.row_at_index(0) {
            folders_list.remove(&row);
        }

        if accounts.is_empty() {
            return;
        }

        // Load persisted expansion states
        let saved = self.load_expander_states();
        let mut expanded_states = HashMap::new();

        // ── Section 0: Unified Inbox (in inboxes list) ──
        // No drop target for unified inbox (can't drop to all accounts at once)
        let total_unread: u32 = accounts.iter().filter_map(|a| a.inbox_unread).sum();
        let row = self.create_inbox_row("mail-inbox-symbolic", &tr("All Inboxes"), Some(total_unread), None);
        row.set_widget_name(&encode_row_name(0, "unified", "", ""));
        inboxes_list.append(&row);

        // ── Section 1: Per-account inboxes (in inboxes list) ──
        // These have drop targets so users can drag messages back to inbox
        for account in &accounts {
            let row = self.create_inbox_row(
                "mail-inbox-symbolic",
                &account.email,
                account.inbox_unread,
                Some(&account.id),
            );
            row.set_widget_name(&encode_row_name(1, "inbox", &account.id, ""));
            inboxes_list.append(&row);
        }

        // ── Section 2+: Per-account folder groups (in folders list) ──
        for (i, account) in accounts.iter().enumerate() {
            let section = i + 2;
            let expanded = saved.get(&account.id).copied().unwrap_or(false);
            expanded_states.insert(account.id.clone(), expanded);

            // Section header row (not selectable, just toggles expansion)
            let header = self.create_section_header_row(&account.email, expanded);
            header.set_widget_name(&encode_row_name(section, "header", &account.id, ""));
            folders_list.append(&header);

            // Folder rows (hidden when collapsed)
            for folder in &account.folders {
                let row = self.create_folder_row(
                    &folder.icon_name,
                    &folder.name,
                    folder.unread_count,
                    true,
                    &account.id,
                    &folder.full_path,
                );
                row.set_widget_name(&encode_row_name(
                    section,
                    "folder",
                    &account.id,
                    &folder.full_path,
                ));
                row.set_visible(expanded);
                folders_list.append(&row);
            }
        }

        imp.expanded_states.replace(expanded_states);

        // Restore the previously selected row
        if let Some(ref name) = selected_name {
            // Try inboxes list first
            let mut idx = 0;
            let mut found = false;
            while let Some(row) = inboxes_list.row_at_index(idx) {
                if row.widget_name() == name.as_str() {
                    inboxes_list.select_row(Some(&row));
                    found = true;
                    break;
                }
                idx += 1;
            }
            // Try folders list if not found
            if !found {
                idx = 0;
                while let Some(row) = folders_list.row_at_index(idx) {
                    if row.widget_name() == name.as_str() {
                        folders_list.select_row(Some(&row));
                        break;
                    }
                    idx += 1;
                }
            }
        }
    }

    // ── Row factories ────────────────────────────────────────────────

    /// Create a row for the inboxes section (white text on accent background)
    /// If account_id is provided, add a drop target for drag-and-drop
    fn create_inbox_row(
        &self,
        icon_name: &str,
        label: &str,
        unread_count: Option<u32>,
        account_id: Option<&str>,
    ) -> gtk4::ListBoxRow {
        let row = gtk4::ListBoxRow::builder()
            .selectable(true)
            .activatable(true)
            .build();

        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(12)
            .margin_start(8)
            .margin_end(8)
            .margin_top(8)
            .margin_bottom(8)
            .build();

        content.append(&gtk4::Image::from_icon_name(icon_name));

        content.append(
            &gtk4::Label::builder()
                .label(label)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build(),
        );

        if let Some(count) = unread_count {
            if count > 0 {
                content.append(
                    &gtk4::Label::builder()
                        .label(&format_number(count))
                        .css_classes(["dim-label"])
                        .build(),
                );
            }
        }

        row.set_child(Some(&content));

        // Add drop target for per-account inbox rows (not unified inbox)
        if let Some(account_id) = account_id {
            let drop_target = gtk4::DropTarget::builder()
                .actions(gtk4::gdk::DragAction::MOVE)
                .build();
            drop_target.set_types(&[glib::Type::STRING]);

            let sidebar = self.clone();
            let target_account_id = account_id.to_string();
            let row_weak = row.downgrade();

            drop_target.connect_drop(move |_target, value, _x, _y| {
                if let Ok(data) = value.get::<String>() {
                    return sidebar.handle_drop_data(&data, &target_account_id, "INBOX");
                }
                false
            });

            // Visual feedback when dragging over
            drop_target.connect_enter(move |_target, _x, _y| {
                if let Some(row) = row_weak.upgrade() {
                    row.add_css_class("drop-highlight");
                }
                gtk4::gdk::DragAction::MOVE
            });

            let row_weak2 = row.downgrade();
            drop_target.connect_leave(move |_target| {
                if let Some(row) = row_weak2.upgrade() {
                    row.remove_css_class("drop-highlight");
                }
            });

            row.add_controller(drop_target);
        }

        row
    }

    /// Create a row for the folders section (normal styling)
    fn create_folder_row(
        &self,
        icon_name: &str,
        label: &str,
        unread_count: Option<u32>,
        indent: bool,
        account_id: &str,
        folder_path: &str,
    ) -> gtk4::ListBoxRow {
        let row = gtk4::ListBoxRow::builder()
            .selectable(true)
            .activatable(true)
            .css_classes(["folder-entry-row"])
            .build();

        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(10)
            .margin_start(if indent { 32 } else { 12 })
            .margin_end(12)
            .margin_top(4)
            .margin_bottom(4)
            .css_classes(["folder-entry"])
            .build();

        content.append(&gtk4::Image::from_icon_name(icon_name));

        content.append(
            &gtk4::Label::builder()
                .label(label)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .css_classes(["folder-entry"])
                .build(),
        );

        if let Some(count) = unread_count {
            if count > 0 {
                content.append(
                    &gtk4::Label::builder()
                        .label(&format_number(count))
                        .css_classes(["dim-label"])
                        .build(),
                );
            }
        }

        row.set_child(Some(&content));

        // Add drop target for drag-and-drop message moving
        let drop_target = gtk4::DropTarget::builder()
            .actions(gtk4::gdk::DragAction::MOVE)
            .build();
        drop_target.set_types(&[glib::Type::STRING]);

        let sidebar = self.clone();
        let target_account_id = account_id.to_string();
        let target_folder_path = folder_path.to_string();
        let row_weak = row.downgrade();

        drop_target.connect_drop(move |_target, value, _x, _y| {
            if let Ok(data) = value.get::<String>() {
                return sidebar.handle_drop_data(&data, &target_account_id, &target_folder_path);
            }
            false
        });

        // Visual feedback when dragging over
        drop_target.connect_enter(move |_target, _x, _y| {
            if let Some(row) = row_weak.upgrade() {
                row.add_css_class("drop-highlight");
            }
            gtk4::gdk::DragAction::MOVE
        });

        let row_weak2 = row.downgrade();
        drop_target.connect_leave(move |_target| {
            if let Some(row) = row_weak2.upgrade() {
                row.remove_css_class("drop-highlight");
            }
        });

        row.add_controller(drop_target);
        row
    }

    fn create_section_header_row(&self, email: &str, expanded: bool) -> gtk4::ListBoxRow {
        let row = gtk4::ListBoxRow::builder()
            .selectable(false)
            .activatable(true)
            .build();

        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();

        // Disclosure indicator
        let arrow_icon = if expanded {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        };
        let arrow = gtk4::Image::builder()
            .icon_name(arrow_icon)
            .pixel_size(12)
            .build();
        arrow.set_widget_name("disclosure-arrow");
        content.append(&arrow);

        content.append(
            &gtk4::Label::builder()
                .label(email)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .css_classes(["section-header-label"])
                .build(),
        );

        row.set_child(Some(&content));
        row
    }

    // ── Expand / collapse ────────────────────────────────────────────

    fn toggle_account_expansion(&self, account_id: &str) {
        let imp = self.imp();
        let mut states = imp.expanded_states.borrow_mut();
        let expanded = states.get(account_id).copied().unwrap_or(false);
        let new_state = !expanded;
        states.insert(account_id.to_string(), new_state);

        // Persist
        drop(states);
        self.save_expander_state(account_id, new_state);

        // Update row visibility + disclosure arrow in folders list
        let folders_list = match imp.folders_list_box.borrow().as_ref() {
            Some(lb) => lb.clone(),
            None => return,
        };

        let mut idx = 0;
        while let Some(row) = folders_list.row_at_index(idx) {
            let name = row.widget_name();
            let (_section, kind, aid, _path) = decode_row_name(&name);
            if aid == account_id {
                match kind {
                    "header" => {
                        // Update disclosure arrow
                        if let Some(content) = row.child().and_then(|c| c.downcast::<gtk4::Box>().ok()) {
                            if let Some(arrow) = content.first_child().and_then(|c| c.downcast::<gtk4::Image>().ok()) {
                                if arrow.widget_name() == "disclosure-arrow" {
                                    arrow.set_icon_name(Some(if new_state {
                                        "pan-down-symbolic"
                                    } else {
                                        "pan-end-symbolic"
                                    }));
                                }
                            }
                        }
                    }
                    "folder" => {
                        row.set_visible(new_state);
                    }
                    _ => {}
                }
            }
            idx += 1;
        }
    }

    // ── Programmatic selection ───────────────────────────────────────

    /// Programmatically select the unified inbox row
    pub fn select_unified_inbox(&self) {
        let imp = self.imp();
        let inboxes_list = match imp.inboxes_list_box.borrow().as_ref() {
            Some(lb) => lb.clone(),
            None => return,
        };

        // Deselect folders list
        if let Some(folders_list) = imp.folders_list_box.borrow().as_ref() {
            folders_list.unselect_all();
        }

        let mut idx = 0;
        while let Some(row) = inboxes_list.row_at_index(idx) {
            let name = row.widget_name();
            let (_section, kind, _aid, _path) = decode_row_name(&name);
            if kind == "unified" {
                inboxes_list.select_row(Some(&row));
                break;
            }
            idx += 1;
        }
    }

    /// Programmatically select a folder (used on startup to highlight restored folder)
    pub fn select_folder(&self, account_id: &str, folder_path: &str) {
        let imp = self.imp();
        let inboxes_list = match imp.inboxes_list_box.borrow().as_ref() {
            Some(lb) => lb.clone(),
            None => return,
        };
        let folders_list = match imp.folders_list_box.borrow().as_ref() {
            Some(lb) => lb.clone(),
            None => return,
        };

        inboxes_list.unselect_all();
        folders_list.unselect_all();

        // Check if it's an inbox (in inboxes list)
        if folder_path.eq_ignore_ascii_case("INBOX") {
            let mut idx = 0;
            while let Some(row) = inboxes_list.row_at_index(idx) {
                let name = row.widget_name();
                let (_section, kind, aid, _path) = decode_row_name(&name);
                if kind == "inbox" && aid == account_id {
                    inboxes_list.select_row(Some(&row));
                    return;
                }
                idx += 1;
            }
        }

        // Otherwise check folders list
        let mut idx = 0;
        while let Some(row) = folders_list.row_at_index(idx) {
            let name = row.widget_name();
            let (_section, kind, aid, path) = decode_row_name(&name);

            if kind == "folder" && aid == account_id && path == folder_path {
                // Ensure the row is visible (expand section if needed)
                if !row.is_visible() {
                    self.toggle_account_expansion(account_id);
                }
                folders_list.select_row(Some(&row));
                break;
            }
            idx += 1;
        }
    }

    // ── Expansion state persistence ──────────────────────────────────

    fn get_state_file_path() -> std::path::PathBuf {
        let data_dir = glib::user_data_dir().join("northmail");
        std::fs::create_dir_all(&data_dir).ok();
        data_dir.join("sidebar_state.json")
    }

    fn load_expander_states(&self) -> HashMap<String, bool> {
        let path = Self::get_state_file_path();
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(states) = serde_json::from_str(&content) {
                return states;
            }
        }
        HashMap::new()
    }

    fn save_expander_state(&self, account_id: &str, expanded: bool) {
        let path = Self::get_state_file_path();
        let mut states = self.load_expander_states();
        states.insert(account_id.to_string(), expanded);
        if let Ok(content) = serde_json::to_string(&states) {
            std::fs::write(&path, content).ok();
        }
    }

    // ── Sync status (unchanged) ──────────────────────────────────────

    pub fn show_sync_status(&self, message: &str) {
        self.show_sync_status_internal(message, true);
    }

    pub fn show_simple_sync_status(&self, message: &str) {
        self.show_sync_status_internal(message, false);
    }

    fn show_sync_status_internal(&self, message: &str, show_progress: bool) {
        let imp = self.imp();
        if let Some(sync_box) = imp.sync_status_box.borrow().as_ref() {
            sync_box.set_visible(true);
        }
        if let Some(spinner) = imp.sync_spinner.borrow().as_ref() {
            spinner.set_spinning(true);
        }
        if let Some(label) = imp.sync_label.borrow().as_ref() {
            label.set_label(message);
        }
        if let Some(progress) = imp.sync_progress.borrow().as_ref() {
            progress.set_visible(show_progress);
            if show_progress {
                progress.set_fraction(0.0);
            }
        }
        if let Some(detail) = imp.sync_detail_label.borrow().as_ref() {
            detail.set_visible(false);
        }
    }

    pub fn set_sync_progress(&self, fraction: f64) {
        let imp = self.imp();
        if let Some(progress) = imp.sync_progress.borrow().as_ref() {
            progress.set_fraction(fraction.clamp(0.0, 1.0));
        }
    }

    pub fn pulse_sync_progress(&self) {
        let imp = self.imp();
        if let Some(progress) = imp.sync_progress.borrow().as_ref() {
            progress.pulse();
        }
    }

    pub fn set_sync_detail(&self, detail: &str) {
        let imp = self.imp();
        if let Some(label) = imp.sync_detail_label.borrow().as_ref() {
            if detail.is_empty() {
                label.set_visible(false);
            } else {
                label.set_label(detail);
                label.set_visible(true);
            }
        }
    }

    pub fn hide_sync_status(&self) {
        let imp = self.imp();
        if let Some(sync_box) = imp.sync_status_box.borrow().as_ref() {
            sync_box.set_visible(false);
        }
        if let Some(spinner) = imp.sync_spinner.borrow().as_ref() {
            spinner.set_spinning(false);
        }
        if let Some(detail) = imp.sync_detail_label.borrow().as_ref() {
            detail.set_visible(false);
        }
    }

    // Keep old method for compatibility
    pub fn set_folders(&self, _folders: Vec<FolderInfo>) {
        // Deprecated - use set_accounts instead
    }
}

impl Default for FolderSidebar {
    fn default() -> Self {
        Self::new()
    }
}

/// Account with its folders
#[derive(Clone)]
pub struct AccountFolders {
    pub id: String,
    pub email: String,
    pub inbox_unread: Option<u32>,
    pub folders: Vec<FolderInfo>,
}

/// Information about a folder for display
#[derive(Clone)]
pub struct FolderInfo {
    pub name: String,
    pub full_path: String,
    pub icon_name: String,
    pub unread_count: Option<u32>,
    pub is_header: bool,
}
