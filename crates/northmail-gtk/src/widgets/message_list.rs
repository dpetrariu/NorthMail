//! Message list widget - Apple Mail inspired design

use gtk4::{glib, prelude::*, subclass::prelude::*};
use libadwaita as adw;
use std::cell::Cell;
use std::rc::Rc;

/// Escape XML/Pango markup special characters
fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Format email date for display (like Apple Mail)
/// Input: "Sat, 08 Feb 2025 14:30:45 +0000" or similar
/// Output: "2:30 PM" (today), "Yesterday", "Feb 7", "12/25/24"
fn format_date(date_str: &str) -> String {
    // Try to parse the date
    if let Some(formatted) = try_parse_email_date(date_str) {
        return formatted;
    }

    // Fallback: just show a cleaned up version
    // Remove timezone offset and seconds
    let cleaned = date_str
        .trim()
        .replace(" +0000", "")
        .replace(" -0000", "");

    // Try to extract just time or date
    if let Some(time_start) = cleaned.rfind(' ') {
        let time_part = &cleaned[time_start + 1..];
        // If it looks like a time (HH:MM:SS), strip seconds
        if time_part.len() >= 5 && time_part.chars().nth(2) == Some(':') {
            return time_part[..5].to_string();
        }
    }

    // Just return first 10 chars or the whole thing
    if cleaned.len() > 16 {
        cleaned[..16].to_string()
    } else {
        cleaned
    }
}

fn try_parse_email_date(date_str: &str) -> Option<String> {
    // Parse RFC 2822 style date: "Sat, 08 Feb 2025 14:30:45 +0000"
    let parts: Vec<&str> = date_str.split_whitespace().collect();

    if parts.len() >= 5 {
        // Extract components
        let day: u32 = parts.get(1)?.parse().ok()?;
        let month_str = *parts.get(2)?;
        let year: i32 = parts.get(3)?.parse().ok()?;
        let time_str = *parts.get(4)?;

        // Parse time (HH:MM:SS)
        let time_parts: Vec<&str> = time_str.split(':').collect();
        let hour: u32 = time_parts.get(0)?.parse().ok()?;
        let minute: u32 = time_parts.get(1)?.parse().ok()?;

        // Get current date for comparison
        let now = glib::DateTime::now_local().ok()?;
        let today_day = now.day_of_month();
        let today_month = now.month();
        let today_year = now.year();

        let month = match month_str.to_lowercase().as_str() {
            "jan" => 1, "feb" => 2, "mar" => 3, "apr" => 4,
            "may" => 5, "jun" => 6, "jul" => 7, "aug" => 8,
            "sep" => 9, "oct" => 10, "nov" => 11, "dec" => 12,
            _ => return None,
        };

        // Format based on how old the message is
        if year == today_year && month == today_month && day as i32 == today_day {
            // Today - show time
            let hour_12 = if hour == 0 { 12 } else if hour > 12 { hour - 12 } else { hour };
            let am_pm = if hour < 12 { "AM" } else { "PM" };
            Some(format!("{}:{:02} {}", hour_12, minute, am_pm))
        } else if year == today_year && month == today_month && day as i32 == today_day - 1 {
            // Yesterday
            Some("Yesterday".to_string())
        } else if year == today_year {
            // This year - show month and day
            Some(format!("{} {}", month_str, day))
        } else {
            // Older - show short date
            Some(format!("{}/{}/{}", month, day, year % 100))
        }
    } else {
        None
    }
}

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use std::cell::RefCell;
    use std::sync::OnceLock;

    #[derive(Default, Clone)]
    pub struct FilterState {
        pub unread_only: bool,
        pub starred_only: bool,
        pub has_attachments: bool,
        pub from_contains: String,
        pub to_cc_contains: String,
        pub date_after: Option<i64>,
        pub date_before: Option<i64>,
    }

    impl FilterState {
        pub fn is_active(&self) -> bool {
            self.unread_only
                || self.starred_only
                || self.has_attachments
                || !self.from_contains.is_empty()
                || !self.to_cc_contains.is_empty()
                || self.date_after.is_some()
                || self.date_before.is_some()
        }
    }

    #[derive(Default)]
    pub struct MessageList {
        pub list_box: RefCell<Option<gtk4::ListBox>>,
        pub search_entry: RefCell<Option<gtk4::SearchEntry>>,
        pub filter_button: RefCell<Option<gtk4::MenuButton>>,
        pub scrolled: RefCell<Option<gtk4::ScrolledWindow>>,
        pub load_more_row: RefCell<Option<gtk4::ListBoxRow>>,
        pub can_load_more: Cell<bool>,
        pub is_loading_more: Cell<bool>,
        pub on_load_more: RefCell<Option<Box<dyn Fn()>>>,
        pub on_filter_changed: RefCell<Option<Box<dyn Fn()>>>,
        pub message_count: Cell<usize>,
        pub total_count: Cell<u32>,
        /// Store message info for each row
        pub messages: RefCell<Vec<super::MessageInfo>>,
        /// Whether scroll handler for infinite scroll is connected
        pub scroll_handler_connected: Cell<bool>,
        /// Whether row activation handler is connected
        pub row_handler_connected: Cell<bool>,
        /// Multi-field filter state
        pub filter_state: RefCell<FilterState>,
        /// Current text search query (as-you-type)
        pub search_query: RefCell<String>,
        /// Currently selected message UIDs (to preserve selection across rebuilds)
        pub selected_uids: RefCell<Vec<u32>>,
        /// Current folder context for drag-and-drop (account_id)
        pub current_account_id: RefCell<String>,
        /// Current folder context for drag-and-drop (folder_path)
        pub current_folder_path: RefCell<String>,
        /// Whether skeleton loading is currently shown
        pub is_loading: Cell<bool>,
        /// Whether a context menu is currently open (prevents auto-selection during rebuilds)
        pub context_menu_open: Cell<bool>,
        /// Anchor row index for Shift+click range selection
        pub anchor_index: Cell<Option<i32>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageList {
        const NAME: &'static str = "NorthMailMessageList";
        type Type = super::MessageList;
        type ParentType = gtk4::Box;
    }

    impl ObjectImpl for MessageList {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("message-selected")
                        .param_types([u32::static_type()])
                        .build(),
                    Signal::builder("search-requested")
                        .param_types([String::static_type()])
                        .build(),
                    Signal::builder("star-toggled")
                        .param_types([u32::static_type(), i64::static_type(), i64::static_type(), bool::static_type()])
                        .build(),
                    // Context menu signals: (uid, msg_id, folder_id, ...)
                    Signal::builder("mark-read")
                        .param_types([u32::static_type(), i64::static_type(), i64::static_type(), bool::static_type()])
                        .build(),
                    Signal::builder("archive")
                        .param_types([u32::static_type(), i64::static_type(), i64::static_type()])
                        .build(),
                    Signal::builder("trash")
                        .param_types([u32::static_type(), i64::static_type(), i64::static_type()])
                        .build(),
                    Signal::builder("spam")
                        .param_types([u32::static_type(), i64::static_type(), i64::static_type()])
                        .build(),
                    Signal::builder("reply")
                        .param_types([u32::static_type()])
                        .build(),
                    Signal::builder("reply-all")
                        .param_types([u32::static_type()])
                        .build(),
                    Signal::builder("forward")
                        .param_types([u32::static_type()])
                        .build(),
                    // Bulk action signals: data is pipe-delimited "uid:msg_id:folder_id|..."
                    Signal::builder("bulk-archive")
                        .param_types([String::static_type()])
                        .build(),
                    Signal::builder("bulk-trash")
                        .param_types([String::static_type()])
                        .build(),
                    Signal::builder("bulk-spam")
                        .param_types([String::static_type()])
                        .build(),
                    Signal::builder("bulk-mark-read")
                        .param_types([String::static_type(), bool::static_type()])
                        .build(),
                    Signal::builder("bulk-star")
                        .param_types([String::static_type(), bool::static_type()])
                        .build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            obj.set_orientation(gtk4::Orientation::Vertical);
            obj.set_vexpand(true);
            obj.set_hexpand(true);
            obj.add_css_class("message-list-container");

            obj.setup_ui();
        }
    }

    impl WidgetImpl for MessageList {}
    impl BoxImpl for MessageList {}
}

glib::wrapper! {
    pub struct MessageList(ObjectSubclass<imp::MessageList>)
        @extends gtk4::Box, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Orientable;
}

impl MessageList {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        // Search bar + filter button
        let search_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .css_classes(["search-bar-container"])
            .build();

        let search_entry = gtk4::SearchEntry::builder()
            .placeholder_text("Search messages...")
            .hexpand(true)
            .build();

        // Store search entry reference early so we can connect signals after setup
        let search_entry_for_signals = search_entry.clone();

        // --- Filter MenuButton with Popover ---
        let filter_button = self.build_filter_button();

        search_box.append(&search_entry);
        search_box.append(&filter_button);
        self.append(&search_box);

        imp.search_entry.replace(Some(search_entry));
        imp.filter_button.replace(Some(filter_button));

        // Separator
        let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        self.append(&separator);

        // Message list
        let scrolled = gtk4::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .css_classes(["view"])
            .build();

        let list_box = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Multiple)
            .css_classes(["message-list"])
            .build();

        // Add separator between rows
        list_box.set_header_func(|row, before| {
            if before.is_some() {
                let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
                sep.add_css_class("message-separator");
                row.set_header(Some(&sep));
            } else {
                row.set_header(None::<&gtk4::Widget>);
            }
        });

        // Add CSS for message list selection styling
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string(
            "
            .message-list > row {
                border-radius: 8px;
                margin: 2px 6px;
            }
            .message-separator {
                margin-left: 12px;
                margin-right: 12px;
                background-color: alpha(@view_fg_color, 0.1);
            }
            .message-list > row:selected {
                background-color: @accent_bg_color;
                color: @accent_fg_color;
                border-radius: 8px;
            }
            .message-list > row:selected * {
                color: @accent_fg_color;
            }
            .message-list > row:selected .dim-label,
            .message-list > row:selected .caption {
                color: alpha(@accent_fg_color, 0.85);
            }
            .unread-dot {
                background-color: @accent_color;
                border-radius: 4px;
            }
            .message-list > row:selected .unread-dot {
                background-color: @accent_fg_color;
            }
            .message-list-container {
                background-color: white;
            }
            .search-bar-container {
                background-color: white;
            }
            /* Drag preview styling */
            .drag-preview {
                background-color: @card_bg_color;
                border-radius: 12px;
                box-shadow: 0 4px 16px alpha(black, 0.3);
                min-width: 220px;
                padding: 12px 16px;
            }
            /* Row being dragged */
            .message-row.dragging {
                background-color: alpha(@accent_bg_color, 0.1);
            }
            /* Skeleton loading animation */
            @keyframes skeleton-pulse {
                0% { opacity: 0.4; }
                50% { opacity: 0.7; }
                100% { opacity: 0.4; }
            }
            .skeleton-row {
                animation: skeleton-pulse 1.5s ease-in-out infinite;
            }
            .skeleton-box {
                background-color: alpha(@view_fg_color, 0.1);
                border-radius: 4px;
            }
            .skeleton-circle {
                background-color: alpha(@view_fg_color, 0.1);
                border-radius: 50%;
            }
            button.context-menu-item {
                padding: 4px 8px;
                min-height: 28px;
                border-radius: 6px;
            }
            button.context-menu-item:hover {
                background-color: alpha(@view_fg_color, 0.08);
            }
            "
        );
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER + 1,
        );

        // Placeholder content - initially empty, will be populated when folder is selected
        let placeholder = adw::StatusPage::builder()
            .icon_name("mail-inbox-symbolic")
            .title("Select a folder")
            .description("Choose a folder from the sidebar to view messages")
            .build();

        scrolled.set_child(Some(&placeholder));
        self.append(&scrolled);

        imp.scrolled.replace(Some(scrolled));
        imp.list_box.replace(Some(list_box));

        // Connect search signals AFTER all widgets are fully initialized
        // As-you-type filtering with simple debounce (no source removal)
        let widget_search = self.clone();
        let pending_query: std::rc::Rc<std::cell::RefCell<Option<String>>> =
            std::rc::Rc::new(std::cell::RefCell::new(None));
        let pending_for_closure = pending_query.clone();
        search_entry_for_signals.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            widget_search.imp().search_query.replace(query.clone());
            widget_search.apply_filter();

            // Store pending query and schedule debounced FTS search
            if !query.is_empty() {
                *pending_for_closure.borrow_mut() = Some(query.clone());
                let widget_weak = widget_search.downgrade();
                let pending = pending_for_closure.clone();
                let expected_query = query;
                glib::timeout_add_local_once(
                    std::time::Duration::from_millis(300),
                    move || {
                        // Only emit if the query hasn't changed since we scheduled this
                        let current = pending.borrow().clone();
                        if current.as_ref() == Some(&expected_query) {
                            if let Some(widget) = widget_weak.upgrade() {
                                widget.emit_by_name::<()>("search-requested", &[&expected_query]);
                            }
                        }
                    },
                );
            } else {
                *pending_for_closure.borrow_mut() = None;
            }
        });

        // Enter → immediate FTS database search
        let widget_activate = self.clone();
        search_entry_for_signals.connect_activate(move |entry| {
            let query = entry.text().to_string();
            widget_activate.emit_by_name::<()>("search-requested", &[&query]);
        });

        // Escape / clear → reset search, emit empty search-requested to reload
        let widget_stop = self.clone();
        search_entry_for_signals.connect_stop_search(move |entry| {
            entry.set_text("");
            widget_stop.imp().search_query.replace(String::new());
            widget_stop.emit_by_name::<()>("search-requested", &[&String::new()]);
        });
    }

    /// Build the filter MenuButton with its popover
    fn build_filter_button(&self) -> gtk4::MenuButton {
        let popover_content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(8)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .margin_bottom(12)
            .build();

        // --- Toggle switches ---
        let unread_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(12)
            .build();
        let unread_label = gtk4::Label::builder()
            .label("Unread only")
            .hexpand(true)
            .xalign(0.0)
            .build();
        let unread_check = gtk4::Switch::new();
        unread_row.append(&unread_label);
        unread_row.append(&unread_check);

        let starred_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(12)
            .build();
        let starred_label = gtk4::Label::builder()
            .label("Starred")
            .hexpand(true)
            .xalign(0.0)
            .build();
        let starred_check = gtk4::Switch::new();
        starred_row.append(&starred_label);
        starred_row.append(&starred_check);

        let attachment_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(12)
            .build();
        let attachment_label = gtk4::Label::builder()
            .label("Has attachments")
            .hexpand(true)
            .xalign(0.0)
            .build();
        let attachment_check = gtk4::Switch::new();
        attachment_row.append(&attachment_label);
        attachment_row.append(&attachment_check);

        popover_content.append(&unread_row);
        popover_content.append(&starred_row);
        popover_content.append(&attachment_row);

        popover_content.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        // --- From filter ---
        let from_entry = gtk4::Entry::builder()
            .placeholder_text("From...")
            .build();
        popover_content.append(&from_entry);

        // --- To/Cc filter ---
        let to_cc_entry = gtk4::Entry::builder()
            .placeholder_text("To/Cc...")
            .build();
        popover_content.append(&to_cc_entry);

        popover_content.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        // --- Date filters ---
        let after_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .build();
        let after_label = gtk4::Label::new(Some("After:"));
        after_label.set_width_request(50);
        after_label.set_xalign(0.0);
        let after_entry = gtk4::Entry::builder()
            .placeholder_text("YYYY-MM-DD")
            .build();
        after_box.append(&after_label);
        after_box.append(&after_entry);
        popover_content.append(&after_box);

        let before_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .build();
        let before_label = gtk4::Label::new(Some("Before:"));
        before_label.set_width_request(50);
        before_label.set_xalign(0.0);
        let before_entry = gtk4::Entry::builder()
            .placeholder_text("YYYY-MM-DD")
            .build();
        before_box.append(&before_label);
        before_box.append(&before_entry);
        popover_content.append(&before_box);

        popover_content.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        // --- Clear Filters button ---
        let clear_button = gtk4::Button::builder()
            .label("Clear Filters")
            .build();
        popover_content.append(&clear_button);

        let popover = gtk4::Popover::builder()
            .child(&popover_content)
            .build();

        let filter_button = gtk4::MenuButton::builder()
            .icon_name("funnel-symbolic")
            .tooltip_text("Filter messages")
            .popover(&popover)
            .build();
        filter_button.add_css_class("flat");

        // --- Connect switch signals ---
        let widget = self.clone();
        let btn_ref = filter_button.clone();
        unread_check.connect_active_notify(move |switch| {
            widget.imp().filter_state.borrow_mut().unread_only = switch.is_active();
            widget.update_filter_indicator(&btn_ref);
            widget.apply_filter();
        });

        let widget = self.clone();
        let btn_ref = filter_button.clone();
        starred_check.connect_active_notify(move |switch| {
            widget.imp().filter_state.borrow_mut().starred_only = switch.is_active();
            widget.update_filter_indicator(&btn_ref);
            widget.apply_filter();
        });

        let widget = self.clone();
        let btn_ref = filter_button.clone();
        attachment_check.connect_active_notify(move |switch| {
            widget.imp().filter_state.borrow_mut().has_attachments = switch.is_active();
            widget.update_filter_indicator(&btn_ref);
            widget.apply_filter();
        });

        // --- From entry ---
        let widget = self.clone();
        let btn_ref = filter_button.clone();
        from_entry.connect_changed(move |entry| {
            widget.imp().filter_state.borrow_mut().from_contains = entry.text().to_string();
            widget.update_filter_indicator(&btn_ref);
            widget.apply_filter();
        });

        // --- To/Cc entry ---
        let widget = self.clone();
        let btn_ref = filter_button.clone();
        to_cc_entry.connect_changed(move |entry| {
            widget.imp().filter_state.borrow_mut().to_cc_contains = entry.text().to_string();
            widget.update_filter_indicator(&btn_ref);
            widget.apply_filter();
        });

        // --- Date entries ---
        let widget = self.clone();
        let btn_ref = filter_button.clone();
        after_entry.connect_changed(move |entry| {
            widget.imp().filter_state.borrow_mut().date_after = parse_date_to_epoch(&entry.text());
            widget.update_filter_indicator(&btn_ref);
            widget.apply_filter();
        });

        let widget = self.clone();
        let btn_ref = filter_button.clone();
        before_entry.connect_changed(move |entry| {
            widget.imp().filter_state.borrow_mut().date_before = parse_date_to_epoch(&entry.text());
            widget.update_filter_indicator(&btn_ref);
            widget.apply_filter();
        });

        // --- Clear button ---
        let widget = self.clone();
        let btn_ref = filter_button.clone();
        let unread_c = unread_check.clone();
        let starred_c = starred_check.clone();
        let attachment_c = attachment_check.clone();
        let from_c = from_entry.clone();
        let to_cc_c = to_cc_entry.clone();
        let after_c = after_entry.clone();
        let before_c = before_entry.clone();
        clear_button.connect_clicked(move |_| {
            // Reset UI controls (will trigger their signals -> apply_filter)
            unread_c.set_active(false);
            starred_c.set_active(false);
            attachment_c.set_active(false);
            from_c.set_text("");
            to_cc_c.set_text("");
            after_c.set_text("");
            before_c.set_text("");
            // Ensure state is clean
            *widget.imp().filter_state.borrow_mut() = imp::FilterState::default();
            widget.update_filter_indicator(&btn_ref);
            widget.apply_filter();
        });

        filter_button
    }

    /// Update the filter button visual indicator
    fn update_filter_indicator(&self, button: &gtk4::MenuButton) {
        let state = self.imp().filter_state.borrow();
        if state.is_active() {
            button.add_css_class("suggested-action");
        } else {
            button.remove_css_class("suggested-action");
        }
    }

    /// Set total message count in folder (for progress display)
    pub fn set_total_count(&self, count: u32) {
        self.imp().total_count.set(count);
    }

    /// Connect to the message-selected signal
    pub fn connect_message_selected<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, u32) + 'static,
    {
        self.connect_closure(
            "message-selected",
            false,
            glib::closure_local!(move |list: &MessageList, uid: u32| {
                f(list, uid);
            }),
        )
    }

    /// Connect to the search-requested signal (fired on Enter in search bar)
    pub fn connect_search_requested<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, String) + 'static,
    {
        self.connect_closure(
            "search-requested",
            false,
            glib::closure_local!(move |list: &MessageList, query: String| {
                f(list, query);
            }),
        )
    }

    /// Connect callback for when star button is toggled in message list
    pub fn connect_star_toggled<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, u32, i64, i64, bool) + 'static,
    {
        self.connect_closure(
            "star-toggled",
            false,
            glib::closure_local!(move |list: &MessageList, uid: u32, msg_id: i64, folder_id: i64, is_starred: bool| {
                f(list, uid, msg_id, folder_id, is_starred);
            }),
        )
    }

    /// Connect callback for when user wants to load more messages
    pub fn connect_load_more<F: Fn() + 'static>(&self, callback: F) {
        self.imp().on_load_more.replace(Some(Box::new(callback)));
    }

    /// Connect callback for when filter state changes (triggers DB-level query)
    pub fn connect_filter_changed<F: Fn() + 'static>(&self, callback: F) {
        self.imp().on_filter_changed.replace(Some(Box::new(callback)));
    }

    /// Get the current filter state as a MessageFilter for DB queries
    pub fn get_message_filter(&self) -> northmail_core::models::MessageFilter {
        let state = self.imp().filter_state.borrow();
        northmail_core::models::MessageFilter {
            unread_only: state.unread_only,
            starred_only: state.starred_only,
            has_attachments: state.has_attachments,
            from_contains: state.from_contains.clone(),
            date_after: state.date_after,
            date_before: state.date_before,
        }
    }

    /// Check if any filter or search query is currently active
    pub fn has_active_filter(&self) -> bool {
        let state = self.imp().filter_state.borrow();
        let query = self.imp().search_query.borrow();
        state.is_active() || !query.is_empty()
    }

    /// Set the current folder context for drag-and-drop operations
    pub fn set_folder_context(&self, account_id: &str, folder_path: &str) {
        let imp = self.imp();
        *imp.current_account_id.borrow_mut() = account_id.to_string();
        *imp.current_folder_path.borrow_mut() = folder_path.to_string();
    }

    /// Get the current folder context (account_id, folder_path)
    pub fn folder_context(&self) -> (String, String) {
        let imp = self.imp();
        (
            imp.current_account_id.borrow().clone(),
            imp.current_folder_path.borrow().clone(),
        )
    }

    /// Clear the search query and search entry text
    pub fn clear_search(&self) {
        let imp = self.imp();
        imp.search_query.replace(String::new());
        if let Some(entry) = imp.search_entry.borrow().as_ref() {
            entry.set_text("");
        }
    }

    /// Show or hide load more capability (with infinite scroll)
    pub fn set_can_load_more(&self, can_load: bool) {
        tracing::info!("set_can_load_more({})", can_load);
        let imp = self.imp();
        imp.can_load_more.set(can_load);

        if let Some(list_box) = imp.list_box.borrow().as_ref() {
            // Remove existing load more row if any
            if let Some(row) = imp.load_more_row.take() {
                list_box.remove(&row);
            }

            if can_load {
                // Add loading indicator row (spinner, not button)
                let row = self.create_loading_row();
                list_box.append(&row);
                imp.load_more_row.replace(Some(row));

                // Set up scroll detection for infinite scroll
                self.setup_infinite_scroll();
            }
        }
    }

    /// Set up infinite scroll detection
    fn setup_infinite_scroll(&self) {
        let imp = self.imp();

        // Only connect once
        if imp.scroll_handler_connected.get() {
            return;
        }

        if let Some(scrolled) = imp.scrolled.borrow().as_ref() {
            imp.scroll_handler_connected.set(true);

            let vadjustment = scrolled.vadjustment();
            let widget = self.clone();

            vadjustment.connect_value_changed(move |adj| {
                let imp = widget.imp();

                // Don't trigger if we can't load more or already loading
                if !imp.can_load_more.get() || imp.is_loading_more.get() {
                    return;
                }

                // Check if we're near the bottom (within 200 pixels)
                let value = adj.value();
                let upper = adj.upper();
                let page_size = adj.page_size();
                let threshold = 200.0;

                if value + page_size + threshold >= upper {
                    // Near bottom - trigger load more
                    tracing::info!("Scroll near bottom, triggering load more");
                    imp.is_loading_more.set(true);

                    // Show the loading spinner
                    if let Some(row) = imp.load_more_row.borrow().as_ref() {
                        row.set_visible(true);
                        if let Some(hbox) = row.child().and_downcast::<gtk4::Box>() {
                            if let Some(spinner) = hbox.first_child().and_downcast::<gtk4::Spinner>() {
                                spinner.start();
                            }
                        }
                    }

                    // Call the load more callback
                    if let Some(callback) = imp.on_load_more.borrow().as_ref() {
                        tracing::info!("Calling load more callback");
                        callback();
                    } else {
                        tracing::warn!("No load more callback set!");
                    }
                }
            });
        }
    }

    /// Create a loading indicator row (for infinite scroll)
    fn create_loading_row(&self) -> gtk4::ListBoxRow {
        let row = gtk4::ListBoxRow::builder()
            .activatable(false)
            .selectable(false)
            .build();

        let hbox = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .halign(gtk4::Align::Center)
            .margin_top(12)
            .margin_bottom(12)
            .build();

        let spinner = gtk4::Spinner::builder()
            .spinning(false)
            .build();

        let label = gtk4::Label::builder()
            .label("Loading more...")
            .css_classes(["dim-label"])
            .build();

        hbox.append(&spinner);
        hbox.append(&label);
        row.set_child(Some(&hbox));

        // Initially hidden until we scroll near bottom
        row.set_visible(false);

        row
    }

    /// Called when loading more is complete
    pub fn finish_loading_more(&self) {
        let imp = self.imp();
        imp.is_loading_more.set(false);

        // Hide the loading row - it will show again when user scrolls
        if let Some(row) = imp.load_more_row.borrow().as_ref() {
            row.set_visible(false);
            if let Some(hbox) = row.child().and_downcast::<gtk4::Box>() {
                if let Some(spinner) = hbox.first_child().and_downcast::<gtk4::Spinner>() {
                    spinner.stop();
                }
            }
        }
    }

    /// Check if a message passes all active filters and search query
    fn message_matches(&self, msg: &MessageInfo) -> bool {
        self.message_matches_with_options(msg, false)
    }

    /// Check if a message passes filters, optionally skipping search query filter
    fn message_matches_with_options(&self, msg: &MessageInfo, skip_search_filter: bool) -> bool {
        let state = self.imp().filter_state.borrow();
        let query = self.imp().search_query.borrow();

        // Checkbox filters
        if state.unread_only && msg.is_read {
            return false;
        }
        if state.starred_only && !msg.is_starred {
            return false;
        }
        if state.has_attachments && !msg.has_attachments {
            return false;
        }

        // From substring filter
        if !state.from_contains.is_empty() {
            let from_lower = msg.from.to_lowercase();
            if !from_lower.contains(&state.from_contains.to_lowercase()) {
                return false;
            }
        }

        // To/Cc substring filter
        if !state.to_cc_contains.is_empty() {
            let to_lower = msg.to.to_lowercase();
            let cc_lower = msg.cc.to_lowercase();
            let search = state.to_cc_contains.to_lowercase();
            if !to_lower.contains(&search) && !cc_lower.contains(&search) {
                return false;
            }
        }

        // Date range filters (use date_epoch if available)
        if let Some(after) = state.date_after {
            match msg.date_epoch {
                Some(epoch) if epoch >= after => {}
                Some(_) => return false,
                None => return false,
            }
        }
        if let Some(before) = state.date_before {
            match msg.date_epoch {
                Some(epoch) if epoch <= before => {}
                Some(_) => return false,
                None => return false,
            }
        }

        // Text search (subject + from + to/cc + snippet) - skip if showing FTS results
        if !skip_search_filter && !query.is_empty() {
            let q = query.to_lowercase();
            let in_subject = msg.subject.to_lowercase().contains(&q);
            let in_from = msg.from.to_lowercase().contains(&q);
            let in_to = msg.to.to_lowercase().contains(&q);
            let in_cc = msg.cc.to_lowercase().contains(&q);
            let in_snippet = msg
                .snippet
                .as_deref()
                .map(|s| s.to_lowercase().contains(&q))
                .unwrap_or(false);
            if !in_subject && !in_from && !in_to && !in_cc && !in_snippet {
                return false;
            }
        }

        true
    }

    /// Show skeleton loading rows while content is being fetched
    pub fn show_loading(&self) {
        let imp = self.imp();
        imp.is_loading.set(true);

        let scrolled = imp.scrolled.borrow();
        let list_box = imp.list_box.borrow();

        if let (Some(scrolled), Some(list_box)) = (scrolled.as_ref(), list_box.as_ref()) {
            // Clear existing content
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }
            imp.load_more_row.replace(None);

            // Add 5 skeleton rows
            for i in 0..5 {
                let row = self.create_skeleton_row(i);
                list_box.append(&row);
            }

            scrolled.set_child(Some(list_box));
        }
    }

    /// Create a single skeleton loading row
    fn create_skeleton_row(&self, _index: usize) -> gtk4::ListBoxRow {
        let row = gtk4::ListBoxRow::builder()
            .selectable(false)
            .activatable(false)
            .build();
        row.add_css_class("skeleton-row");

        let row_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(12)
            .margin_start(12)
            .margin_end(12)
            .margin_top(10)
            .margin_bottom(10)
            .build();

        // Avatar placeholder (circle)
        let avatar = gtk4::Box::builder()
            .width_request(36)
            .height_request(36)
            .valign(gtk4::Align::Start)
            .build();
        avatar.add_css_class("skeleton-circle");
        row_box.append(&avatar);

        // Content column
        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .hexpand(true)
            .build();

        // Top row: sender name and date
        let top_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .build();

        let sender = gtk4::Box::builder()
            .width_request(120)
            .height_request(14)
            .valign(gtk4::Align::Center)
            .build();
        sender.add_css_class("skeleton-box");

        let date = gtk4::Box::builder()
            .width_request(50)
            .height_request(12)
            .valign(gtk4::Align::Center)
            .halign(gtk4::Align::End)
            .hexpand(true)
            .build();
        date.add_css_class("skeleton-box");

        top_row.append(&sender);
        top_row.append(&date);
        content.append(&top_row);

        // Subject line
        let subject = gtk4::Box::builder()
            .width_request(200)
            .height_request(14)
            .build();
        subject.add_css_class("skeleton-box");
        content.append(&subject);

        // Snippet preview
        let snippet = gtk4::Box::builder()
            .height_request(12)
            .hexpand(true)
            .build();
        snippet.add_css_class("skeleton-box");
        content.append(&snippet);

        row_box.append(&content);
        row.set_child(Some(&row_box));
        row
    }

    /// Clear and set initial messages
    pub fn set_messages(&self, messages: Vec<MessageInfo>) {
        self.set_messages_inner(messages, false);
    }

    /// Set messages from FTS search results (skip local search filtering since DB already filtered)
    pub fn set_search_results(&self, messages: Vec<MessageInfo>) {
        self.set_messages_inner(messages, true);
    }

    fn set_messages_inner(&self, messages: Vec<MessageInfo>, is_search_results: bool) {
        let imp = self.imp();

        // Clear loading state
        imp.is_loading.set(false);

        let scrolled = imp.scrolled.borrow();
        let list_box = imp.list_box.borrow();

        // Deduplicate by UID (keep first occurrence)
        let mut seen_uids = std::collections::HashSet::new();
        let deduped: Vec<MessageInfo> = messages
            .into_iter()
            .filter(|m| seen_uids.insert(m.uid))
            .collect();

        // Sort messages by date (newest first) to ensure correct order
        let mut sorted_messages = deduped;
        sorted_messages.sort_by(|a, b| {
            // Sort by date_epoch descending (newest first), fall back to uid descending
            match (b.date_epoch, a.date_epoch) {
                (Some(b_date), Some(a_date)) => b_date.cmp(&a_date),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => b.uid.cmp(&a.uid),
            }
        });

        // Reset counters and store messages
        imp.message_count.set(sorted_messages.len());
        imp.messages.replace(sorted_messages.clone());
        let messages = sorted_messages;

        if let (Some(scrolled), Some(list_box)) = (scrolled.as_ref(), list_box.as_ref()) {
            // Remember selected UIDs before clearing
            let selected_uids = imp.selected_uids.borrow().clone();

            // Clear existing rows
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }
            imp.load_more_row.replace(None);

            // Apply filters to decide which messages to show
            // For search results, skip the text search filter (DB already did FTS)
            let visible: Vec<&MessageInfo> = messages.iter()
                .filter(|m| self.message_matches_with_options(m, is_search_results))
                .collect();

            if visible.is_empty() {
                let filter_state = imp.filter_state.borrow();
                let search_query = imp.search_query.borrow();
                let (title, desc) = if !search_query.is_empty() && !messages.is_empty() {
                    ("No Results", "No messages match your search")
                } else if filter_state.is_active() && !messages.is_empty() {
                    ("No Matching Messages", "Try adjusting your filters")
                } else {
                    ("Empty Folder", "There are no messages in this folder")
                };
                let placeholder = adw::StatusPage::builder()
                    .icon_name("mail-inbox-symbolic")
                    .title(title)
                    .description(desc)
                    .build();
                scrolled.set_child(Some(&placeholder));
            } else {
                // Add visible messages
                for msg in &visible {
                    self.add_message_row(list_box, msg);
                }

                // Connect click gesture handler only once
                // With SelectionMode::Multiple, we use GestureClick to distinguish
                // plain click (select one + show message) from Ctrl/Shift click (multi-select)
                if !imp.row_handler_connected.get() {
                    let widget = self.clone();
                    let gesture = gtk4::GestureClick::new();
                    gesture.set_button(1); // Left click only
                    gesture.connect_released(move |gesture, _n, _x, y| {
                        let lb = {
                            let lb_ref = widget.imp().list_box.borrow();
                            match lb_ref.as_ref() {
                                Some(lb) => lb.clone(),
                                None => return,
                            }
                        };

                        // Find which row was clicked
                        let row = if let Some(row) = lb.row_at_y(y as i32) {
                            row
                        } else {
                            return;
                        };

                        // Check modifier state for Ctrl/Shift
                        let (has_ctrl, has_shift) = if let Some(event) = gesture.last_event(gesture.current_sequence().as_ref()) {
                            let state = event.modifier_state();
                            (
                                state.contains(gtk4::gdk::ModifierType::CONTROL_MASK),
                                state.contains(gtk4::gdk::ModifierType::SHIFT_MASK),
                            )
                        } else {
                            (false, false)
                        };

                        let index = row.index();
                        let imp = widget.imp();
                        let messages = imp.messages.borrow();
                        let filtered: Vec<&MessageInfo> = messages.iter()
                            .filter(|m| widget.message_matches(m))
                            .collect();

                        if has_shift {
                            // Shift+click: range select from anchor to clicked row
                            let anchor = imp.anchor_index.get().unwrap_or(0);
                            let start = anchor.min(index);
                            let end = anchor.max(index);

                            // If Ctrl isn't also held, clear existing selection first
                            if !has_ctrl {
                                lb.unselect_all();
                            }

                            // Select all rows in the range
                            let mut uids = imp.selected_uids.borrow_mut();
                            if !has_ctrl {
                                uids.clear();
                            }
                            for i in start..=end {
                                if let Some(r) = lb.row_at_index(i) {
                                    lb.select_row(Some(&r));
                                    if let Some(msg) = filtered.get(i as usize) {
                                        if !uids.contains(&msg.uid) {
                                            uids.push(msg.uid);
                                        }
                                    }
                                }
                            }
                            // Don't update anchor on shift-click (preserve it for further shift-clicks)
                            tracing::debug!("Shift-select: {} messages selected (range {}..={})", uids.len(), start, end);
                        } else if has_ctrl {
                            // Ctrl+click: toggle individual row in selection
                            let is_selected = lb.selected_rows().iter().any(|r| r.index() == index);
                            let mut uids = imp.selected_uids.borrow_mut();

                            if is_selected {
                                // GTK already selected it; if we want toggle behavior, unselect
                                lb.unselect_row(&row);
                                if let Some(msg) = filtered.get(index as usize) {
                                    uids.retain(|u| *u != msg.uid);
                                }
                            } else {
                                lb.select_row(Some(&row));
                                if let Some(msg) = filtered.get(index as usize) {
                                    if !uids.contains(&msg.uid) {
                                        uids.push(msg.uid);
                                    }
                                }
                            }
                            imp.anchor_index.set(Some(index));
                            tracing::debug!("Ctrl-select: {} messages selected", uids.len());
                        } else {
                            // Plain click: select only this row, show message
                            lb.unselect_all();
                            lb.select_row(Some(&row));
                            imp.anchor_index.set(Some(index));
                            let clicked_uid = filtered.get(index as usize).map(|m| m.uid);
                            drop(filtered);
                            drop(messages);
                            if let Some(uid) = clicked_uid {
                                tracing::debug!("Row clicked: index={}, uid={}", index, uid);
                                let mut uids = imp.selected_uids.borrow_mut();
                                uids.clear();
                                uids.push(uid);
                                drop(uids);
                                widget.emit_by_name::<()>("message-selected", &[&uid]);
                            }
                        }
                    });
                    list_box.add_controller(gesture);
                    imp.row_handler_connected.set(true);
                    tracing::debug!("Click gesture handler connected");
                }

                scrolled.set_child(Some(list_box));

                // Restore user's selection AFTER re-parenting (set_child resets GTK selection state)
                if !selected_uids.is_empty() {
                    let lb = imp.list_box.borrow().clone();
                    let uids = selected_uids.clone();
                    let msgs: Vec<(usize, u32)> = visible.iter().enumerate()
                        .filter(|(_, m)| uids.contains(&m.uid))
                        .map(|(idx, m)| (idx, m.uid))
                        .collect();
                    glib::idle_add_local_once(move || {
                        if let Some(list_box) = lb.as_ref() {
                            list_box.unselect_all();
                            for (idx, _uid) in &msgs {
                                if let Some(row) = list_box.row_at_index(*idx as i32) {
                                    list_box.select_row(Some(&row));
                                }
                            }
                        }
                    });
                } else {
                    // If no user selection, deselect after layout to prevent GTK auto-selecting first row
                    let lb = imp.list_box.borrow().clone();
                    glib::idle_add_local_once(move || {
                        if let Some(list_box) = lb.as_ref() {
                            list_box.unselect_all();
                        }
                    });
                }
            }
        }
    }

    /// Re-filter the displayed messages based on current filter state.
    /// If a filter-changed callback is wired up, delegate to DB-level filtering;
    /// otherwise fall back to client-side filtering of loaded messages.
    fn apply_filter(&self) {
        if let Some(callback) = self.imp().on_filter_changed.borrow().as_ref() {
            callback();
        } else {
            let messages = self.imp().messages.borrow().clone();
            self.set_messages(messages);
        }
    }

    /// Append more messages to the existing list
    pub fn append_messages(&self, messages: Vec<MessageInfo>) {
        let imp = self.imp();

        // Add new messages to stored list
        {
            let mut stored = imp.messages.borrow_mut();
            stored.extend(messages);
            imp.message_count.set(stored.len());

            // Sort all messages by date (newest first)
            stored.sort_by(|a, b| {
                match (b.date_epoch, a.date_epoch) {
                    (Some(b_date), Some(a_date)) => b_date.cmp(&a_date),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => b.uid.cmp(&a.uid),
                }
            });
        }

        // Rebuild visible rows to show sorted messages
        self.rebuild_visible_rows();
        self.finish_loading_more();
    }

    /// Append messages, skipping any whose UID is already in the list (dedup).
    /// Used during background sync to add new messages without duplicating
    /// those already loaded from cache or a previous batch.
    pub fn append_new_messages(&self, messages: Vec<MessageInfo>) {
        let existing_uids: std::collections::HashSet<u32> = self.imp().messages.borrow()
            .iter()
            .map(|m| m.uid)
            .collect();
        let new_msgs: Vec<MessageInfo> = messages.into_iter()
            .filter(|m| !existing_uids.contains(&m.uid))
            .collect();
        if !new_msgs.is_empty() {
            self.append_messages(new_msgs);
        }
    }

    fn add_message_row(&self, list_box: &gtk4::ListBox, msg: &MessageInfo) {
        // Create a custom row layout like Apple Mail:
        // ┌─────────────────────────────────────────────────────┐
        // │ [●] Sender Name                          2:30 PM ⭐ │
        // │     Subject line here                          📎 │
        // │     Preview text snippet...                        │
        // └─────────────────────────────────────────────────────┘

        let row = gtk4::ListBoxRow::builder()
            .activatable(true)
            .build();

        // Main horizontal box
        let hbox = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .margin_start(12)
            .margin_end(12)
            .margin_top(8)
            .margin_bottom(8)
            .build();

        // Indicator column (unread dot only)
        let indicator_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .width_request(12)
            .valign(gtk4::Align::Start)
            .margin_top(4)
            .spacing(4)
            .build();

        // Unread indicator (blue dot)
        if !msg.is_read {
            let dot = gtk4::Box::builder()
                .width_request(8)
                .height_request(8)
                .css_classes(["unread-dot"])
                .halign(gtk4::Align::Center)
                .build();
            indicator_box.append(&dot);
        }

        hbox.append(&indicator_box);

        // Content area (sender, subject, preview)
        let content_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();

        // Top row: Sender + Date
        let top_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .build();

        // Sender name
        let sender_label = gtk4::Label::builder()
            .label(&escape_markup(&msg.from))
            .use_markup(true)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        if !msg.is_read {
            sender_label.add_css_class("heading");
        }
        top_row.append(&sender_label);

        // Date (formatted nicely)
        let formatted_date = format_date(&msg.date);
        let date_label = gtk4::Label::builder()
            .label(&formatted_date)
            .css_classes(["dim-label", "caption"])
            .build();
        top_row.append(&date_label);

        content_box.append(&top_row);

        // Middle row: Subject + attachment icon
        let middle_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(4)
            .build();

        let subject_text = if msg.subject.is_empty() {
            "(No Subject)".to_string()
        } else {
            msg.subject.clone()
        };

        let subject_label = gtk4::Label::builder()
            .label(&escape_markup(&subject_text))
            .use_markup(true)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        if msg.is_read {
            subject_label.add_css_class("dim-label");
        }
        middle_row.append(&subject_label);

        // Attachment indicator
        if msg.has_attachments {
            let attachment = gtk4::Image::from_icon_name("mail-attachment-symbolic");
            attachment.add_css_class("dim-label");
            attachment.set_pixel_size(14);
            middle_row.append(&attachment);
        }

        // Star button (always visible, clickable)
        let star_button = gtk4::ToggleButton::builder()
            .icon_name(if msg.is_starred { "starred-symbolic" } else { "non-starred-symbolic" })
            .active(msg.is_starred)
            .css_classes(["flat", "circular", "star-button"])
            .valign(gtk4::Align::Center)
            .build();

        // Connect star button to emit signal
        let widget = self.clone();
        let msg_uid = msg.uid;
        let msg_id = msg.id;
        let msg_folder_id = msg.folder_id;
        star_button.connect_toggled(move |button| {
            let is_starred = button.is_active();
            // Update icon
            button.set_icon_name(if is_starred { "starred-symbolic" } else { "non-starred-symbolic" });
            // Emit signal for the window to handle IMAP sync
            widget.emit_by_name::<()>("star-toggled", &[&msg_uid, &msg_id, &msg_folder_id, &is_starred]);
        });
        middle_row.append(&star_button);

        content_box.append(&middle_row);

        // Bottom row: Preview snippet (if available)
        if let Some(snippet) = &msg.snippet {
            if !snippet.is_empty() {
                let preview_label = gtk4::Label::builder()
                    .label(&escape_markup(snippet))
                    .use_markup(true)
                    .xalign(0.0)
                    .ellipsize(gtk4::pango::EllipsizeMode::End)
                    .css_classes(["dim-label", "caption"])
                    .build();
                content_box.append(&preview_label);
            }
        }

        hbox.append(&content_box);
        row.set_child(Some(&hbox));

        // Add drag source for drag-and-drop to folders
        let drag_source = gtk4::DragSource::builder()
            .actions(gtk4::gdk::DragAction::MOVE)
            .build();

        // Store message data for drag (use folder context from MessageList, not msg.folder_id which may be 0)
        let (account_id, folder_path) = self.folder_context();
        let drag_msg_uid = msg.uid;
        let drag_msg_id = msg.id;
        let drag_account_id = account_id.clone();
        let drag_folder_path = folder_path.clone();

        let widget_for_prepare = self.clone();
        drag_source.connect_prepare(move |_source, _x, _y| {
            let selected_uids = widget_for_prepare.imp().selected_uids.borrow();
            let is_multi = selected_uids.len() > 1 && selected_uids.contains(&drag_msg_uid);

            if is_multi {
                // Encode all selected messages: "multi|uid:msg_id:acct:folder|uid:msg_id:acct:folder|..."
                let messages = widget_for_prepare.imp().messages.borrow();
                let (acct, fpath) = widget_for_prepare.folder_context();
                let entries: Vec<String> = messages.iter()
                    .filter(|m| selected_uids.contains(&m.uid))
                    .map(|m| format!("{}:{}:{}:{}", m.uid, m.id, acct, fpath))
                    .collect();
                let data = format!("multi|{}", entries.join("|"));
                Some(gtk4::gdk::ContentProvider::for_value(&data.to_value()))
            } else {
                // Single message: "uid:msg_id:account_id:folder_path"
                let data = format!("{}:{}:{}:{}", drag_msg_uid, drag_msg_id, drag_account_id, drag_folder_path);
                Some(gtk4::gdk::ContentProvider::for_value(&data.to_value()))
            }
        });

        // Show message info as drag icon (handle UTF-8 properly)
        let subject_for_drag = if msg.subject.is_empty() {
            "(No Subject)".to_string()
        } else {
            let chars: Vec<char> = msg.subject.chars().collect();
            if chars.len() > 40 {
                format!("{}...", chars[..37].iter().collect::<String>())
            } else {
                msg.subject.clone()
            }
        };
        let from_for_drag = {
            let chars: Vec<char> = msg.from.chars().collect();
            if chars.len() > 25 {
                format!("{}...", chars[..22].iter().collect::<String>())
            } else {
                msg.from.clone()
            }
        };

        // Use the row itself as the drag icon source for proper rendering
        let row_weak = row.downgrade();
        let widget_for_begin = self.clone();
        drag_source.connect_drag_begin(move |_source, _drag| {
            let selected_uids = widget_for_begin.imp().selected_uids.borrow();
            let is_multi = selected_uids.len() > 1 && selected_uids.contains(&drag_msg_uid);

            // Create a styled container for the drag preview
            let drag_widget = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(12)
                .css_classes(["card", "drag-preview"])
                .build();

            // Add mail icon
            let icon = gtk4::Image::builder()
                .icon_name("mail-unread-symbolic")
                .pixel_size(24)
                .css_classes(["accent"])
                .build();
            drag_widget.append(&icon);

            if is_multi {
                // Multi-drag: show badge with count
                let count = selected_uids.len();
                let label = gtk4::Label::builder()
                    .label(&format!("{} Messages", count))
                    .xalign(0.0)
                    .css_classes(["heading"])
                    .build();
                drag_widget.append(&label);

                // Make all selected rows semi-transparent
                if let Some(lb) = widget_for_begin.imp().list_box.borrow().as_ref() {
                    let messages = widget_for_begin.imp().messages.borrow();
                    let filtered: Vec<&super::MessageInfo> = messages.iter()
                        .filter(|m| widget_for_begin.message_matches(m))
                        .collect();
                    for (idx, msg) in filtered.iter().enumerate() {
                        if selected_uids.contains(&msg.uid) {
                            if let Some(row) = lb.row_at_index(idx as i32) {
                                row.set_opacity(0.4);
                                row.add_css_class("dragging");
                            }
                        }
                    }
                }
            } else {
                // Single drag: show from + subject
                let text_box = gtk4::Box::builder()
                    .orientation(gtk4::Orientation::Vertical)
                    .spacing(2)
                    .build();

                let from_label = gtk4::Label::builder()
                    .label(&from_for_drag)
                    .xalign(0.0)
                    .css_classes(["heading"])
                    .build();

                let subject_label = gtk4::Label::builder()
                    .label(&subject_for_drag)
                    .xalign(0.0)
                    .css_classes(["dim-label"])
                    .build();

                text_box.append(&from_label);
                text_box.append(&subject_label);
                drag_widget.append(&text_box);

                // Make the original row semi-transparent during drag
                if let Some(row) = row_weak.upgrade() {
                    row.set_opacity(0.4);
                    row.add_css_class("dragging");
                }
            }

            // Get the DragIcon from the drag and set content directly
            let drag_icon = gtk4::DragIcon::for_drag(_drag);
            drag_icon.set_child(Some(&drag_widget));
        });

        // Restore opacity when drag ends
        let row_weak2 = row.downgrade();
        let widget_for_end = self.clone();
        drag_source.connect_drag_end(move |_source, _drag, _delete| {
            // Restore all rows (handles both single and multi-drag)
            if let Some(lb) = widget_for_end.imp().list_box.borrow().as_ref() {
                let mut idx = 0;
                while let Some(row) = lb.row_at_index(idx) {
                    row.set_opacity(1.0);
                    row.remove_css_class("dragging");
                    idx += 1;
                }
            } else if let Some(row) = row_weak2.upgrade() {
                row.set_opacity(1.0);
                row.remove_css_class("dragging");
            }
        });

        row.add_controller(drag_source);

        // Add context menu for right-click
        self.add_row_context_menu(&row, msg);

        // Add separator between messages
        list_box.append(&row);
    }

    /// Helper to create a context menu item button in a popover vbox
    fn make_context_menu_item(vbox: &gtk4::Box, label: &str) -> gtk4::Button {
        let lbl = gtk4::Label::new(None);
        lbl.set_markup(&format!("<span color='#1c1c1c' weight='normal'>{}</span>", glib::markup_escape_text(label)));
        lbl.set_xalign(0.0);
        let btn = gtk4::Button::new();
        btn.set_child(Some(&lbl));
        btn.add_css_class("flat");
        btn.add_css_class("context-menu-item");
        btn.set_hexpand(true);
        btn.set_halign(gtk4::Align::Fill);
        vbox.append(&btn);
        btn
    }

    /// Helper to add a separator to a context menu vbox
    fn add_context_menu_separator(vbox: &gtk4::Box) {
        let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        sep.set_margin_top(4);
        sep.set_margin_bottom(4);
        vbox.append(&sep);
    }

    /// Build the single-message context menu popover (used when right-clicking one message)
    fn build_single_context_menu(&self, row: &gtk4::ListBoxRow, msg: &MessageInfo) -> gtk4::Popover {
        let msg_uid = msg.uid;
        let msg_id = msg.id;
        let msg_folder_id = msg.folder_id;
        let is_read = msg.is_read;
        let is_starred = msg.is_starred;

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        vbox.set_margin_top(4);
        vbox.set_margin_bottom(4);

        let popover = gtk4::Popover::new();
        popover.set_parent(row);
        popover.set_has_arrow(false);
        popover.set_child(Some(&vbox));

        // Read/Unread toggle
        let widget = self.clone();
        if is_read {
            let btn = Self::make_context_menu_item(&vbox, "Mark as Unread");
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                w.emit_by_name::<()>("mark-read", &[&msg_uid, &msg_id, &msg_folder_id, &false]);
            });
        } else {
            let btn = Self::make_context_menu_item(&vbox, "Mark as Read");
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                w.emit_by_name::<()>("mark-read", &[&msg_uid, &msg_id, &msg_folder_id, &true]);
            });
        }

        // Star toggle
        if is_starred {
            let btn = Self::make_context_menu_item(&vbox, "Unstar");
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                w.emit_by_name::<()>("star-toggled", &[&msg_uid, &msg_id, &msg_folder_id, &false]);
            });
        } else {
            let btn = Self::make_context_menu_item(&vbox, "Star");
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                w.emit_by_name::<()>("star-toggled", &[&msg_uid, &msg_id, &msg_folder_id, &true]);
            });
        }

        Self::add_context_menu_separator(&vbox);

        // Reply / Reply All / Forward
        {
            let btn = Self::make_context_menu_item(&vbox, "Reply");
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                w.emit_by_name::<()>("reply", &[&msg_uid]);
            });
        }
        {
            let btn = Self::make_context_menu_item(&vbox, "Reply All");
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                w.emit_by_name::<()>("reply-all", &[&msg_uid]);
            });
        }
        {
            let btn = Self::make_context_menu_item(&vbox, "Forward");
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                w.emit_by_name::<()>("forward", &[&msg_uid]);
            });
        }

        Self::add_context_menu_separator(&vbox);

        // Archive / Trash / Spam
        {
            let btn = Self::make_context_menu_item(&vbox, "Archive");
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                w.emit_by_name::<()>("archive", &[&msg_uid, &msg_id, &msg_folder_id]);
            });
        }
        {
            let btn = Self::make_context_menu_item(&vbox, "Move to Trash");
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                w.emit_by_name::<()>("trash", &[&msg_uid, &msg_id, &msg_folder_id]);
            });
        }
        {
            let btn = Self::make_context_menu_item(&vbox, "Mark as Spam");
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                w.emit_by_name::<()>("spam", &[&msg_uid, &msg_id, &msg_folder_id]);
            });
        }

        popover
    }

    /// Build a bulk context menu popover for multiple selected messages
    fn build_bulk_context_menu(&self, row: &gtk4::ListBoxRow, count: usize) -> gtk4::Popover {
        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        vbox.set_margin_top(4);
        vbox.set_margin_bottom(4);

        let popover = gtk4::Popover::new();
        popover.set_parent(row);
        popover.set_has_arrow(false);
        popover.set_child(Some(&vbox));

        let widget = self.clone();

        // Mark as Read / Mark as Unread
        {
            let btn = Self::make_context_menu_item(&vbox, &format!("Mark {} as Read", count));
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                let data = w.encode_bulk_data();
                w.emit_by_name::<()>("bulk-mark-read", &[&data, &true]);
            });
        }
        {
            let btn = Self::make_context_menu_item(&vbox, &format!("Mark {} as Unread", count));
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                let data = w.encode_bulk_data();
                w.emit_by_name::<()>("bulk-mark-read", &[&data, &false]);
            });
        }

        // Star / Unstar
        {
            let btn = Self::make_context_menu_item(&vbox, &format!("Star {}", count));
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                let data = w.encode_bulk_data();
                w.emit_by_name::<()>("bulk-star", &[&data, &true]);
            });
        }
        {
            let btn = Self::make_context_menu_item(&vbox, &format!("Unstar {}", count));
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                let data = w.encode_bulk_data();
                w.emit_by_name::<()>("bulk-star", &[&data, &false]);
            });
        }

        Self::add_context_menu_separator(&vbox);

        // Archive / Trash / Spam
        {
            let btn = Self::make_context_menu_item(&vbox, &format!("Archive {} Messages", count));
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                let data = w.encode_bulk_data();
                w.emit_by_name::<()>("bulk-archive", &[&data]);
            });
        }
        {
            let btn = Self::make_context_menu_item(&vbox, &format!("Move {} to Trash", count));
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                let data = w.encode_bulk_data();
                w.emit_by_name::<()>("bulk-trash", &[&data]);
            });
        }
        {
            let btn = Self::make_context_menu_item(&vbox, &format!("Mark {} as Spam", count));
            let w = widget.clone();
            let p = popover.clone();
            btn.connect_clicked(move |_| {
                p.popdown();
                w.imp().context_menu_open.set(false);
                let data = w.encode_bulk_data();
                w.emit_by_name::<()>("bulk-spam", &[&data]);
            });
        }

        popover
    }

    /// Add context menu to a message row using a manual Popover + Box + Buttons.
    /// We avoid PopoverMenu/gio::Menu because GTK4 inherits the selected row's
    /// white text color into the popover, making menu items invisible.
    fn add_row_context_menu(&self, row: &gtk4::ListBoxRow, msg: &MessageInfo) {
        let msg_uid = msg.uid;

        // Build the single-message popover (always created)
        let single_popover = self.build_single_context_menu(row, msg);

        // Track when single popover closes
        let widget_for_close = self.clone();
        single_popover.connect_closed(move |_| {
            widget_for_close.imp().context_menu_open.set(false);
        });

        // Store a RefCell for the dynamically created bulk popover
        let bulk_popover: Rc<std::cell::RefCell<Option<gtk4::Popover>>> = Rc::new(std::cell::RefCell::new(None));

        // Right-click gesture
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(3);
        let single_popover_clone = single_popover.clone();
        let widget_for_gesture = self.clone();
        let row_weak = row.downgrade();
        let bulk_popover_clone = bulk_popover.clone();
        gesture.connect_pressed(move |gesture, _n, x, y| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            widget_for_gesture.imp().context_menu_open.set(true);

            let sel_count = widget_for_gesture.selection_count();
            let is_in_selection = widget_for_gesture.imp().selected_uids.borrow().contains(&msg_uid);

            if sel_count > 1 && is_in_selection {
                // Show bulk context menu
                // Clean up previous bulk popover if any
                if let Some(old) = bulk_popover_clone.borrow_mut().take() {
                    old.unparent();
                }

                if let Some(row) = row_weak.upgrade() {
                    let popover = widget_for_gesture.build_bulk_context_menu(&row, sel_count);
                    let w = widget_for_gesture.clone();
                    popover.connect_closed(move |_| {
                        w.imp().context_menu_open.set(false);
                    });
                    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                    popover.popup();
                    *bulk_popover_clone.borrow_mut() = Some(popover);
                }
            } else {
                // Show single-message context menu
                single_popover_clone.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                single_popover_clone.popup();
            }
        });
        row.add_controller(gesture);
    }

    /// Update a message's starred status in the list
    pub fn update_message_starred(&self, uid: u32, is_starred: bool) {
        let imp = self.imp();
        let mut messages = imp.messages.borrow_mut();
        if let Some(msg) = messages.iter_mut().find(|m| m.uid == uid) {
            msg.is_starred = is_starred;
        }
        drop(messages);
        // Rebuild the list to reflect the change
        self.rebuild_visible_rows();
    }

    /// Update a message's read status in the list
    pub fn update_message_read(&self, uid: u32, is_read: bool) {
        let imp = self.imp();
        let mut messages = imp.messages.borrow_mut();
        if let Some(msg) = messages.iter_mut().find(|m| m.uid == uid) {
            msg.is_read = is_read;
        }
        drop(messages);
        self.rebuild_visible_rows();
    }

    /// Remove a message from the list by UID
    pub fn remove_message(&self, uid: u32) {
        let imp = self.imp();
        let mut messages = imp.messages.borrow_mut();
        messages.retain(|m| m.uid != uid);
        imp.message_count.set(messages.len());
        drop(messages);
        // Rebuild the list to reflect the change
        self.rebuild_visible_rows();
    }

    /// Rebuild visible rows from stored messages (used after status updates)
    fn rebuild_visible_rows(&self) {
        let imp = self.imp();

        // If there's an active filter with a DB callback, delegate to it instead of
        // doing client-side filtering (which only filters in-memory messages, not DB)
        if self.has_active_filter() {
            if let Some(callback) = imp.on_filter_changed.borrow().as_ref() {
                callback();
                return;
            }
        }

        let list_box = imp.list_box.borrow();
        let scrolled = imp.scrolled.borrow();
        let messages = imp.messages.borrow();

        if let (Some(list_box), Some(_scrolled)) = (list_box.as_ref(), scrolled.as_ref()) {
            // Remember selected UIDs
            let selected_uids = imp.selected_uids.borrow().clone();

            // Clear existing rows
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }
            imp.load_more_row.replace(None);

            // Rebuild with current filters (client-side)
            let visible: Vec<&MessageInfo> = messages.iter()
                .filter(|m| self.message_matches(m))
                .collect();

            for msg in &visible {
                self.add_message_row(list_box, msg);
            }

            // Re-add load more row if needed
            if imp.can_load_more.get() {
                let load_row = self.create_loading_row();
                load_row.set_visible(false);
                list_box.append(&load_row);
                imp.load_more_row.replace(Some(load_row));
            }

            // Restore user's selection or deselect (deferred to idle so GTK finishes layout first)
            let lb = list_box.clone();
            if !selected_uids.is_empty() {
                let restore: Vec<i32> = visible.iter().enumerate()
                    .filter(|(_, m)| selected_uids.contains(&m.uid))
                    .map(|(idx, _)| idx as i32)
                    .collect();
                glib::idle_add_local_once(move || {
                    lb.unselect_all();
                    for idx in &restore {
                        if let Some(row) = lb.row_at_index(*idx) {
                            lb.select_row(Some(&row));
                        }
                    }
                });
            } else {
                glib::idle_add_local_once(move || {
                    lb.unselect_all();
                });
            }
        }
    }

    /// Return cloned info for all currently selected messages
    pub fn selected_messages(&self) -> Vec<MessageInfo> {
        let imp = self.imp();
        let uids = imp.selected_uids.borrow();
        let messages = imp.messages.borrow();
        messages.iter()
            .filter(|m| uids.contains(&m.uid))
            .cloned()
            .collect()
    }

    /// Return the number of currently selected messages
    pub fn selection_count(&self) -> usize {
        self.imp().selected_uids.borrow().len()
    }

    /// Bulk remove messages by UIDs and rebuild
    pub fn remove_messages(&self, uids: &[u32]) {
        let imp = self.imp();
        let mut messages = imp.messages.borrow_mut();
        messages.retain(|m| !uids.contains(&m.uid));
        imp.message_count.set(messages.len());
        drop(messages);
        // Clear selection for removed messages
        {
            let mut sel = imp.selected_uids.borrow_mut();
            sel.retain(|uid| !uids.contains(uid));
        }
        self.rebuild_visible_rows();
    }

    /// Encode selected messages as pipe-delimited string "uid:msg_id:folder_id|..."
    fn encode_bulk_data(&self) -> String {
        let msgs = self.selected_messages();
        msgs.iter()
            .map(|m| format!("{}:{}:{}", m.uid, m.id, m.folder_id))
            .collect::<Vec<_>>()
            .join("|")
    }
}

impl Default for MessageList {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a "YYYY-MM-DD" string to a Unix epoch timestamp (start of day UTC)
/// Parse a date string to epoch, accepting partial formats:
/// "2025" → 2025-01-01, "2025-03" → 2025-03-01, "2025-03-15" → 2025-03-15
fn parse_date_to_epoch(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Strip trailing dashes (user may type "2026-" mid-entry)
    let s = s.trim_end_matches('-');
    let parts: Vec<&str> = s.split('-').collect();
    let year: i32 = parts.first()?.parse().ok()?;
    if !(1970..=2100).contains(&year) {
        return None;
    }
    let month: i32 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(1);
    let day: i32 = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(1);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let dt = glib::DateTime::from_utc(year, month, day, 0, 0, 0.0).ok()?;
    tracing::debug!("parse_date_to_epoch({:?}) → {}", s, dt.to_unix());
    Some(dt.to_unix())
}


/// Information about a message for display
#[derive(Clone)]
pub struct MessageInfo {
    pub id: i64,
    pub uid: u32,
    pub folder_id: i64,
    pub message_id: Option<String>,
    pub subject: String,
    pub from: String,
    pub from_address: String,
    pub to: String,
    pub cc: String,
    pub date: String,
    pub date_epoch: Option<i64>,
    pub snippet: Option<String>,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
}

impl From<&northmail_core::models::DbMessage> for MessageInfo {
    fn from(db_msg: &northmail_core::models::DbMessage) -> Self {
        MessageInfo {
            id: db_msg.id,
            uid: db_msg.uid as u32,
            folder_id: db_msg.folder_id,
            message_id: db_msg.message_id.clone(),
            subject: db_msg.subject.clone().unwrap_or_default(),
            from: db_msg
                .from_name
                .clone()
                .or_else(|| db_msg.from_address.clone())
                .unwrap_or_else(|| "Unknown".to_string()),
            from_address: db_msg.from_address.clone().unwrap_or_default(),
            to: db_msg.to_addresses.clone().unwrap_or_default(),
            cc: db_msg.cc_addresses.clone().unwrap_or_default(),
            date: db_msg.date_sent.clone().unwrap_or_default(),
            date_epoch: db_msg.date_epoch,
            snippet: db_msg.snippet.clone(),
            is_read: db_msg.is_read,
            is_starred: db_msg.is_starred,
            has_attachments: db_msg.has_attachments,
        }
    }
}
