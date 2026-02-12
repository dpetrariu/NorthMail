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
        /// Currently selected message UID (to preserve selection across rebuilds)
        pub selected_uid: Cell<Option<u32>>,
        /// Current folder context for drag-and-drop (account_id)
        pub current_account_id: RefCell<String>,
        /// Current folder context for drag-and-drop (folder_path)
        pub current_folder_path: RefCell<String>,
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
            .selection_mode(gtk4::SelectionMode::Single)
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

    /// Show or hide load more capability (with infinite scroll)
    pub fn set_can_load_more(&self, can_load: bool) {
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
                        callback();
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
            // Remember selected UID before clearing
            let selected_uid = imp.selected_uid.get();

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

                // Connect row activation handler only once
                if !imp.row_handler_connected.get() {
                    let widget = self.clone();
                    list_box.connect_row_activated(move |_, row| {
                        let index = row.index() as usize;
                        // Map display index to actual message via filtered list
                        let imp = widget.imp();
                        let messages = imp.messages.borrow();
                        let filtered: Vec<&MessageInfo> = messages.iter()
                            .filter(|m| widget.message_matches(m))
                            .collect();
                        if let Some(msg) = filtered.get(index) {
                            tracing::debug!("Row activated: index={}, uid={}", index, msg.uid);
                            // Store selected UID for preservation across rebuilds
                            imp.selected_uid.set(Some(msg.uid));
                            widget.emit_by_name::<()>("message-selected", &[&msg.uid]);
                        } else {
                            tracing::warn!("Row activated but no message at index {}", index);
                        }
                    });
                    imp.row_handler_connected.set(true);
                    tracing::debug!("Row activation handler connected");
                }

                // Restore selection if we had one
                if let Some(uid) = selected_uid {
                    for (idx, msg) in visible.iter().enumerate() {
                        if msg.uid == uid {
                            if let Some(row) = list_box.row_at_index(idx as i32) {
                                list_box.select_row(Some(&row));
                            }
                            break;
                        }
                    }
                }

                scrolled.set_child(Some(list_box));
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
        let drag_data = Rc::new((msg.uid, msg.id, account_id.clone(), folder_path.clone(), msg.subject.clone()));
        let drag_data_for_prepare = drag_data.clone();

        drag_source.connect_prepare(move |_source, _x, _y| {
            // Create content with message info as string: "uid:msg_id:account_id:folder_path"
            let data = format!("{}:{}:{}:{}",
                drag_data_for_prepare.0,
                drag_data_for_prepare.1,
                drag_data_for_prepare.2,
                drag_data_for_prepare.3);
            Some(gtk4::gdk::ContentProvider::for_value(&data.to_value()))
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
        drag_source.connect_drag_begin(move |source, _drag| {
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

            // Text container
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

            // Get the DragIcon from the drag and set content directly
            let drag_icon = gtk4::DragIcon::for_drag(_drag);
            drag_icon.set_child(Some(&drag_widget));

            // Make the original row semi-transparent during drag
            if let Some(row) = row_weak.upgrade() {
                row.set_opacity(0.4);
                row.add_css_class("dragging");
            }
        });

        // Restore opacity when drag ends
        let row_weak2 = row.downgrade();
        drag_source.connect_drag_end(move |_source, _drag, _delete| {
            if let Some(row) = row_weak2.upgrade() {
                row.set_opacity(1.0);
                row.remove_css_class("dragging");
            }
        });

        row.add_controller(drag_source);

        // Add separator between messages
        list_box.append(&row);
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
            // Remember selected UID
            let selected_uid = imp.selected_uid.get();

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

            // Restore selection
            if let Some(uid) = selected_uid {
                let filtered: Vec<&MessageInfo> = messages.iter()
                    .filter(|m| self.message_matches(m))
                    .collect();
                if let Some(pos) = filtered.iter().position(|m| m.uid == uid) {
                    if let Some(row) = list_box.row_at_index(pos as i32) {
                        list_box.select_row(Some(&row));
                    }
                }
            }
        }
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
