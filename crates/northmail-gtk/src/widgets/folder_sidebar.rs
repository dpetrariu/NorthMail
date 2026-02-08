//! Folder sidebar widget

use gtk4::{glib, prelude::*, subclass::prelude::*};
use std::cell::RefCell;
use std::collections::HashMap;

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use std::sync::OnceLock;

    #[derive(Default)]
    pub struct FolderSidebar {
        pub main_box: RefCell<Option<gtk4::Box>>,
        pub expanders: RefCell<HashMap<String, gtk4::Expander>>,
        pub accounts: RefCell<Vec<super::AccountFolders>>,
        pub sync_status_box: RefCell<Option<gtk4::Box>>,
        pub sync_spinner: RefCell<Option<gtk4::Spinner>>,
        pub sync_label: RefCell<Option<gtk4::Label>>,
        pub sync_progress: RefCell<Option<gtk4::ProgressBar>>,
        pub sync_detail_label: RefCell<Option<gtk4::Label>>,
        /// All inbox ListBoxes (unified + per-account) for coordinated selection
        pub inbox_listboxes: RefCell<Vec<gtk4::ListBox>>,
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

    fn setup_ui(&self) {
        let imp = self.imp();

        // Main scrolled area
        let scrolled = gtk4::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .build();

        let main_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .build();

        // Placeholder - will be populated when accounts load
        let placeholder = gtk4::Label::builder()
            .label("Loading accounts...")
            .css_classes(["dim-label"])
            .margin_top(24)
            .margin_bottom(24)
            .build();
        main_box.append(&placeholder);

        scrolled.set_child(Some(&main_box));
        self.append(&scrolled);

        // Bottom section with sync status and settings
        let bottom_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .build();

        // Sync status area (hidden by default) - ABOVE the separator
        let sync_status_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(4)
            .margin_start(12)
            .margin_end(12)
            .margin_top(8)
            .margin_bottom(8)
            .visible(false)
            .build();

        // Top row: spinner + status text
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

        // Progress bar
        let sync_progress = gtk4::ProgressBar::builder()
            .show_text(false)
            .build();
        sync_progress.add_css_class("osd");
        sync_status_box.append(&sync_progress);

        // Detail label (e.g., "Loading messages...")
        let sync_detail_label = gtk4::Label::builder()
            .label("")
            .css_classes(["dim-label", "caption"])
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .visible(false)
            .build();
        sync_status_box.append(&sync_detail_label);

        bottom_box.append(&sync_status_box);

        // Separator below sync status
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
            // Activate the settings action
            if let Some(app) = gtk4::gio::Application::default() {
                app.activate_action("show-settings", None);
            }
        });

        bottom_box.append(&settings_button);
        self.append(&bottom_box);

        imp.main_box.replace(Some(main_box));
        imp.sync_status_box.replace(Some(sync_status_box));
        imp.sync_spinner.replace(Some(sync_spinner));
        imp.sync_label.replace(Some(sync_label));
        imp.sync_progress.replace(Some(sync_progress));
        imp.sync_detail_label.replace(Some(sync_detail_label));
    }

    /// Show sync status with a message
    /// Show sync status with optional progress bar
    /// For background sync, use show_simple = true to hide progress bar
    pub fn show_sync_status(&self, message: &str) {
        self.show_sync_status_internal(message, true);
    }

    /// Show simple sync status (just spinner + message, no progress bar)
    /// Used for background sync operations
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
        // Show/hide progress bar based on mode
        if let Some(progress) = imp.sync_progress.borrow().as_ref() {
            progress.set_visible(show_progress);
            if show_progress {
                progress.set_fraction(0.0);
            }
        }
        // Hide detail label initially
        if let Some(detail) = imp.sync_detail_label.borrow().as_ref() {
            detail.set_visible(false);
        }
    }

    /// Update sync progress (0.0 to 1.0)
    pub fn set_sync_progress(&self, fraction: f64) {
        let imp = self.imp();
        if let Some(progress) = imp.sync_progress.borrow().as_ref() {
            progress.set_fraction(fraction.clamp(0.0, 1.0));
        }
    }

    /// Pulse the progress bar (for indeterminate progress)
    pub fn pulse_sync_progress(&self) {
        let imp = self.imp();
        if let Some(progress) = imp.sync_progress.borrow().as_ref() {
            progress.pulse();
        }
    }

    /// Set detail text below progress bar
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

    /// Hide sync status
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

    /// Update folder list with accounts and their folders
    pub fn set_accounts(&self, accounts: Vec<AccountFolders>) {
        let imp = self.imp();

        // Store accounts for later reference
        imp.accounts.replace(accounts.clone());

        if let Some(main_box) = imp.main_box.borrow().as_ref() {
            // Clear existing content
            while let Some(child) = main_box.first_child() {
                main_box.remove(&child);
            }

            // Load saved expander states
            let saved_states = self.load_expander_states();

            // Collect all inbox listboxes for coordinated selection
            let mut inbox_listboxes: Vec<gtk4::ListBox> = Vec::new();

            // Unified Inbox at top (if any accounts)
            if !accounts.is_empty() {
                let unified_list = gtk4::ListBox::builder()
                    .selection_mode(gtk4::SelectionMode::Single)
                    .css_classes(["navigation-sidebar"])
                    .build();

                let unified_row = self.create_folder_row(
                    "mail-inbox-symbolic",
                    "All Inboxes",
                    Some(0), // TODO: sum of all unread
                    false,
                );
                unified_row.add_css_class("unified-inbox");
                unified_list.append(&unified_row);

                inbox_listboxes.push(unified_list.clone());
                main_box.append(&unified_list);

                // Add small separator
                let sep = gtk4::Separator::builder()
                    .margin_top(6)
                    .margin_bottom(6)
                    .build();
                main_box.append(&sep);
            }

            // Account inboxes section - each in its own ListBox for selection
            for account in &accounts {
                let inbox_list = gtk4::ListBox::builder()
                    .selection_mode(gtk4::SelectionMode::Single)
                    .css_classes(["navigation-sidebar"])
                    .build();

                let inbox_row = self.create_folder_row(
                    "mail-inbox-symbolic",
                    &account.email,
                    account.inbox_unread,
                    false,
                );
                inbox_list.append(&inbox_row);

                inbox_listboxes.push(inbox_list.clone());
                main_box.append(&inbox_list);
            }

            // Store all inbox listboxes for coordinated selection
            imp.inbox_listboxes.replace(inbox_listboxes.clone());

            // Now connect selection handlers that can clear other selections
            for (idx, listbox) in inbox_listboxes.iter().enumerate() {
                let sidebar = self.clone();
                let all_listboxes = inbox_listboxes.clone();
                let current_idx = idx;

                // Determine if this is the unified inbox (index 0) or an account inbox
                let is_unified = idx == 0;
                let account_id = if is_unified {
                    String::new()
                } else {
                    // Account index is idx - 1 (since unified is at 0)
                    accounts.get(idx - 1).map(|a| a.id.clone()).unwrap_or_default()
                };

                listbox.connect_row_activated(move |activated_list, row| {
                    // Clear selection in all OTHER inbox listboxes
                    for (i, lb) in all_listboxes.iter().enumerate() {
                        if i != current_idx {
                            lb.unselect_all();
                        }
                    }

                    // Keep this row selected
                    activated_list.select_row(Some(row));

                    // Emit the folder-selected signal
                    sidebar.emit_by_name::<()>("folder-selected", &[&account_id, &"INBOX", &is_unified]);
                });
            }

            // Separator before collapsible sections
            if !accounts.is_empty() {
                let sep = gtk4::Separator::builder()
                    .margin_top(12)
                    .margin_bottom(6)
                    .build();
                main_box.append(&sep);
            }

            // Collapsible sections for each account
            let mut expanders = HashMap::new();
            for account in &accounts {
                let expander = self.create_account_expander(account, &saved_states);
                expanders.insert(account.id.clone(), expander.clone());
                main_box.append(&expander);
            }

            imp.expanders.replace(expanders);
        }
    }

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

        let icon = gtk4::Image::from_icon_name(icon_name);
        content.append(&icon);

        let label_widget = gtk4::Label::builder()
            .label(label)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        content.append(&label_widget);

        if let Some(count) = unread_count {
            if count > 0 {
                let badge = gtk4::Label::builder()
                    .label(&count.to_string())
                    .css_classes(["dim-label"])
                    .build();
                content.append(&badge);
            }
        }

        row.set_child(Some(&content));
        row
    }

    fn create_account_expander(
        &self,
        account: &AccountFolders,
        saved_states: &HashMap<String, bool>,
    ) -> gtk4::Expander {
        // Default to collapsed
        let expanded = saved_states.get(&account.id).copied().unwrap_or(false);

        let expander = gtk4::Expander::builder()
            .expanded(expanded)
            .margin_start(4)
            .margin_end(4)
            .margin_top(8)  // Added top margin for spacing between sections
            .margin_bottom(8) // Added bottom margin for spacing between sections
            .build();

        // Header with account name
        let header = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .build();

        let icon = gtk4::Image::from_icon_name("avatar-default-symbolic");
        header.append(&icon);

        let label = gtk4::Label::builder()
            .label(&account.email)
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .css_classes(["heading"])
            .build();
        header.append(&label);

        expander.set_label_widget(Some(&header));

        // Content - folder list
        let folder_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .margin_start(8)
            .build();

        // Store folder paths for lookup
        let folder_paths: Vec<String> = account.folders.iter().map(|f| f.full_path.clone()).collect();

        for folder in &account.folders {
            let row = self.create_folder_row(&folder.icon_name, &folder.name, folder.unread_count, true);
            folder_list.append(&row);
        }

        // Connect folder selection
        let sidebar = self.clone();
        let account_id = account.id.clone();
        folder_list.connect_row_activated(move |list_box, row| {
            // Clear all inbox selections when selecting a folder
            sidebar.clear_inbox_selections();

            let index = row.index() as usize;
            if let Some(folder_path) = folder_paths.get(index) {
                sidebar.emit_by_name::<()>(
                    "folder-selected",
                    &[&account_id, &folder_path.as_str(), &false],
                );
            }
            // Deselect so it can be clicked again
            list_box.unselect_row(row);
        });

        expander.set_child(Some(&folder_list));

        // Save state when toggled
        let account_id = account.id.clone();
        let sidebar_ref = self.clone();
        expander.connect_expanded_notify(move |exp| {
            sidebar_ref.save_expander_state(&account_id, exp.is_expanded());
        });

        expander
    }

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

    /// Clear selection in all inbox listboxes
    fn clear_inbox_selections(&self) {
        let imp = self.imp();
        for listbox in imp.inbox_listboxes.borrow().iter() {
            listbox.unselect_all();
        }
    }

    /// Clear selection in all folder expanders
    fn clear_folder_selections(&self) {
        let imp = self.imp();
        for expander in imp.expanders.borrow().values() {
            if let Some(list_box) = expander.child().and_then(|c| c.downcast::<gtk4::ListBox>().ok()) {
                list_box.unselect_all();
            }
        }
    }

    /// Programmatically select a folder (used on startup to highlight restored folder)
    pub fn select_folder(&self, account_id: &str, folder_path: &str) {
        let imp = self.imp();

        // Clear all selections first
        self.clear_inbox_selections();
        self.clear_folder_selections();

        if folder_path.eq_ignore_ascii_case("INBOX") {
            // Find the inbox ListBox for this account
            let accounts = imp.accounts.borrow();
            let inbox_listboxes = imp.inbox_listboxes.borrow();

            // inbox_listboxes[0] = unified, inbox_listboxes[i+1] = accounts[i]
            for (i, account) in accounts.iter().enumerate() {
                if account.id == account_id {
                    // Select the inbox row (index i+1, since 0 is unified)
                    if let Some(listbox) = inbox_listboxes.get(i + 1) {
                        if let Some(row) = listbox.row_at_index(0) {
                            listbox.select_row(Some(&row));
                        }
                    }
                    break;
                }
            }
        } else {
            // Find the expander for this account and expand it
            let expanders = imp.expanders.borrow();
            if let Some(expander) = expanders.get(account_id) {
                expander.set_expanded(true);

                // Find and select the folder row
                if let Some(list_box) = expander.child().and_then(|c| c.downcast::<gtk4::ListBox>().ok()) {
                    let accounts = imp.accounts.borrow();
                    if let Some(account) = accounts.iter().find(|a| a.id == account_id) {
                        for (i, folder) in account.folders.iter().enumerate() {
                            if folder.full_path == folder_path {
                                if let Some(row) = list_box.row_at_index(i as i32) {
                                    list_box.select_row(Some(&row));
                                }
                                break;
                            }
                        }
                    }
                }
            }
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
