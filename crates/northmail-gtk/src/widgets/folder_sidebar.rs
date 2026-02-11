//! Folder sidebar widget — single ListBox with header_func separators
//! and collapsible per-account folder sections.

use gtk4::{glib, prelude::*, subclass::prelude::*};
use std::cell::RefCell;
use std::collections::HashMap;

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
        /// The single ListBox that contains every row.
        pub list_box: RefCell<Option<gtk4::ListBox>>,
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
                vec![Signal::builder("folder-selected")
                    .param_types([
                        String::static_type(), // account_id
                        String::static_type(), // folder_path
                        bool::static_type(),   // is_unified
                    ])
                    .build()]
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

    // ── UI setup ─────────────────────────────────────────────────────

    fn setup_ui(&self) {
        let imp = self.imp();

        // Scrolled area
        let scrolled = gtk4::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .build();

        // Single flat ListBox
        let list_box = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();

        // Header func: auto-insert separators between sections
        list_box.set_header_func(|row, before| {
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

        // Row activation handler (connected once, persists across rebuilds)
        let sidebar = self.clone();
        list_box.connect_row_activated(move |list_box, row| {
            let name = row.widget_name();
            let (_section, kind, account_id, folder_path) = decode_row_name(&name);

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
                "header" => {
                    // Toggle expansion — don't select header rows
                    list_box.unselect_row(row);
                    sidebar.toggle_account_expansion(account_id);
                }
                "folder" => {
                    sidebar.emit_by_name::<()>(
                        "folder-selected",
                        &[&account_id, &folder_path, &false],
                    );
                }
                _ => {}
            }
        });

        // Placeholder
        let placeholder = gtk4::Label::builder()
            .label("Loading accounts...")
            .css_classes(["dim-label"])
            .margin_top(24)
            .margin_bottom(24)
            .build();
        list_box.append(&gtk4::ListBoxRow::builder()
            .child(&placeholder)
            .selectable(false)
            .activatable(false)
            .build());

        scrolled.set_child(Some(&list_box));
        self.append(&scrolled);

        // ── Bottom section (sync status + settings) ──
        let bottom_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .build();

        // Sync status area (hidden by default)
        let sync_status_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(4)
            .margin_start(12)
            .margin_end(12)
            .margin_top(8)
            .margin_bottom(8)
            .visible(false)
            .build();

        let sync_top_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .build();

        let sync_spinner = gtk4::Spinner::builder()
            .spinning(true)
            .width_request(12)
            .height_request(12)
            .build();

        let sync_label = gtk4::Label::builder()
            .label("Syncing...")
            .css_classes(["dim-label", "caption"])
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        sync_top_row.append(&sync_spinner);
        sync_top_row.append(&sync_label);
        sync_status_box.append(&sync_top_row);

        let sync_progress = gtk4::ProgressBar::builder()
            .show_text(false)
            .build();
        sync_progress.add_css_class("osd");
        sync_status_box.append(&sync_progress);

        let sync_detail_label = gtk4::Label::builder()
            .label("")
            .css_classes(["dim-label", "caption"])
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .visible(false)
            .build();
        sync_status_box.append(&sync_detail_label);

        bottom_box.append(&sync_status_box);

        let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        bottom_box.append(&separator);

        // Settings button
        let settings_button = gtk4::Button::builder()
            .child(
                &gtk4::Box::builder()
                    .orientation(gtk4::Orientation::Horizontal)
                    .spacing(12)
                    .margin_start(12)
                    .margin_end(12)
                    .margin_top(8)
                    .margin_bottom(8)
                    .build(),
            )
            .css_classes(["flat"])
            .build();

        let button_content = settings_button
            .child()
            .unwrap()
            .downcast::<gtk4::Box>()
            .unwrap();
        button_content.append(&gtk4::Image::from_icon_name("emblem-system-symbolic"));
        button_content.append(
            &gtk4::Label::builder()
                .label("Settings")
                .xalign(0.0)
                .hexpand(true)
                .build(),
        );

        settings_button.connect_clicked(|_| {
            if let Some(app) = gtk4::gio::Application::default() {
                app.activate_action("show-settings", None);
            }
        });

        bottom_box.append(&settings_button);
        self.append(&bottom_box);

        // Store references
        imp.list_box.replace(Some(list_box));
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

        let list_box = match imp.list_box.borrow().as_ref() {
            Some(lb) => lb.clone(),
            None => return,
        };

        // Remember the currently selected row so we can restore it after rebuild
        let selected_name = list_box.selected_row()
            .map(|row| row.widget_name().to_string());

        // Clear all rows
        while let Some(row) = list_box.row_at_index(0) {
            list_box.remove(&row);
        }

        if accounts.is_empty() {
            return;
        }

        // Load persisted expansion states
        let saved = self.load_expander_states();
        let mut expanded_states = HashMap::new();

        // ── Section 0: Unified Inbox ──
        let total_unread: u32 = accounts.iter().filter_map(|a| a.inbox_unread).sum();
        let row = self.create_folder_row("mail-inbox-symbolic", "All Inboxes", Some(total_unread), false);
        row.set_widget_name(&encode_row_name(0, "unified", "", ""));
        list_box.append(&row);

        // ── Section 1: Per-account inboxes ──
        for account in &accounts {
            let row = self.create_folder_row(
                "mail-inbox-symbolic",
                &account.email,
                account.inbox_unread,
                false,
            );
            row.set_widget_name(&encode_row_name(1, "inbox", &account.id, ""));
            list_box.append(&row);
        }

        // ── Section 2+: Per-account folder groups ──
        for (i, account) in accounts.iter().enumerate() {
            let section = i + 2;
            let expanded = saved.get(&account.id).copied().unwrap_or(false);
            expanded_states.insert(account.id.clone(), expanded);

            // Section header row (not selectable, just toggles expansion)
            let header = self.create_section_header_row(&account.email, expanded);
            header.set_widget_name(&encode_row_name(section, "header", &account.id, ""));
            list_box.append(&header);

            // Folder rows (hidden when collapsed)
            for folder in &account.folders {
                let row = self.create_folder_row(
                    &folder.icon_name,
                    &folder.name,
                    folder.unread_count,
                    true,
                );
                row.set_widget_name(&encode_row_name(
                    section,
                    "folder",
                    &account.id,
                    &folder.full_path,
                ));
                row.set_visible(expanded);
                list_box.append(&row);
            }
        }

        imp.expanded_states.replace(expanded_states);

        // Restore the previously selected row
        if let Some(ref name) = selected_name {
            let mut idx = 0;
            while let Some(row) = list_box.row_at_index(idx) {
                if row.widget_name() == name.as_str() {
                    list_box.select_row(Some(&row));
                    break;
                }
                idx += 1;
            }
        }
    }

    // ── Row factories ────────────────────────────────────────────────

    fn create_folder_row(
        &self,
        icon_name: &str,
        label: &str,
        unread_count: Option<u32>,
        indent: bool,
    ) -> gtk4::ListBoxRow {
        let row = gtk4::ListBoxRow::builder()
            .selectable(true)
            .activatable(true)
            .build();

        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(12)
            .margin_start(if indent { 32 } else { 12 })
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
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
                        .label(&count.to_string())
                        .css_classes(["dim-label"])
                        .build(),
                );
            }
        }

        row.set_child(Some(&content));
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
            .margin_top(4)
            .margin_bottom(4)
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

        content.append(&gtk4::Image::from_icon_name("avatar-default-symbolic"));

        content.append(
            &gtk4::Label::builder()
                .label(email)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .css_classes(["heading"])
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

        // Update row visibility + disclosure arrow
        let list_box = match imp.list_box.borrow().as_ref() {
            Some(lb) => lb.clone(),
            None => return,
        };

        let mut idx = 0;
        while let Some(row) = list_box.row_at_index(idx) {
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
        let list_box = match imp.list_box.borrow().as_ref() {
            Some(lb) => lb.clone(),
            None => return,
        };

        list_box.unselect_all();

        let mut idx = 0;
        while let Some(row) = list_box.row_at_index(idx) {
            let name = row.widget_name();
            let (_section, kind, _aid, _path) = decode_row_name(&name);
            if kind == "unified" {
                list_box.select_row(Some(&row));
                break;
            }
            idx += 1;
        }
    }

    /// Programmatically select a folder (used on startup to highlight restored folder)
    pub fn select_folder(&self, account_id: &str, folder_path: &str) {
        let imp = self.imp();
        let list_box = match imp.list_box.borrow().as_ref() {
            Some(lb) => lb.clone(),
            None => return,
        };

        list_box.unselect_all();

        let mut idx = 0;
        while let Some(row) = list_box.row_at_index(idx) {
            let name = row.widget_name();
            let (_section, kind, aid, path) = decode_row_name(&name);

            let matches = match kind {
                "inbox" if folder_path.eq_ignore_ascii_case("INBOX") => aid == account_id,
                "folder" => aid == account_id && path == folder_path,
                _ => false,
            };

            if matches {
                // Ensure the row is visible (expand section if needed)
                if kind == "folder" && !row.is_visible() {
                    self.toggle_account_expansion(account_id);
                }
                list_box.select_row(Some(&row));
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
