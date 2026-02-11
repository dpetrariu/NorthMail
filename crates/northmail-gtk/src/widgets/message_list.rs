//! Message list widget - Apple Mail inspired design

use gtk4::{glib, prelude::*, subclass::prelude::*};
use libadwaita as adw;
use std::cell::Cell;

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
        pub date_after: Option<i64>,
        pub date_before: Option<i64>,
    }

    impl FilterState {
        pub fn is_active(&self) -> bool {
            self.unread_only
                || self.starred_only
                || self.has_attachments
                || !self.from_contains.is_empty()
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
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            obj.set_orientation(gtk4::Orientation::Vertical);
            obj.set_vexpand(true);
            obj.set_hexpand(true);

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
            .build();

        let list_box = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .build();

        // Remove default styling for cleaner look
        list_box.add_css_class("navigation-sidebar");

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

        // --- Checkboxes (custom child labels for indicator–text spacing) ---
        let unread_check = gtk4::CheckButton::new();
        unread_check.set_child(Some(&gtk4::Label::builder().label("Unread only").margin_start(6).build()));
        let starred_check = gtk4::CheckButton::new();
        starred_check.set_child(Some(&gtk4::Label::builder().label("Starred").margin_start(6).build()));
        let attachment_check = gtk4::CheckButton::new();
        attachment_check.set_child(Some(&gtk4::Label::builder().label("Has attachments").margin_start(6).build()));

        popover_content.append(&unread_check);
        popover_content.append(&starred_check);
        popover_content.append(&attachment_check);

        popover_content.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        // --- From filter ---
        let from_entry = gtk4::Entry::builder()
            .placeholder_text("From...")
            .build();
        popover_content.append(&from_entry);

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

        // --- Connect checkbox signals ---
        let widget = self.clone();
        let btn_ref = filter_button.clone();
        unread_check.connect_toggled(move |check| {
            widget.imp().filter_state.borrow_mut().unread_only = check.is_active();
            widget.update_filter_indicator(&btn_ref);
            widget.apply_filter();
        });

        let widget = self.clone();
        let btn_ref = filter_button.clone();
        starred_check.connect_toggled(move |check| {
            widget.imp().filter_state.borrow_mut().starred_only = check.is_active();
            widget.update_filter_indicator(&btn_ref);
            widget.apply_filter();
        });

        let widget = self.clone();
        let btn_ref = filter_button.clone();
        attachment_check.connect_toggled(move |check| {
            widget.imp().filter_state.borrow_mut().has_attachments = check.is_active();
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
        let after_c = after_entry.clone();
        let before_c = before_entry.clone();
        clear_button.connect_clicked(move |_| {
            // Reset UI controls (will trigger their signals -> apply_filter)
            unread_c.set_active(false);
            starred_c.set_active(false);
            attachment_c.set_active(false);
            from_c.set_text("");
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

        // Text search (subject + from substring)
        if !query.is_empty() {
            let q = query.to_lowercase();
            let in_subject = msg.subject.to_lowercase().contains(&q);
            let in_from = msg.from.to_lowercase().contains(&q);
            let in_snippet = msg
                .snippet
                .as_deref()
                .map(|s| s.to_lowercase().contains(&q))
                .unwrap_or(false);
            if !in_subject && !in_from && !in_snippet {
                return false;
            }
        }

        true
    }

    /// Clear and set initial messages
    pub fn set_messages(&self, messages: Vec<MessageInfo>) {
        let imp = self.imp();

        let scrolled = imp.scrolled.borrow();
        let list_box = imp.list_box.borrow();

        // Reset counters and store messages
        imp.message_count.set(messages.len());
        imp.messages.replace(messages.clone());

        if let (Some(scrolled), Some(list_box)) = (scrolled.as_ref(), list_box.as_ref()) {
            // Clear existing rows
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }
            imp.load_more_row.replace(None);

            // Apply all filters + search to decide which messages to show
            let visible: Vec<&MessageInfo> = messages.iter()
                .filter(|m| self.message_matches(m))
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
                            widget.emit_by_name::<()>("message-selected", &[&msg.uid]);
                        } else {
                            tracing::warn!("Row activated but no message at index {}", index);
                        }
                    });
                    imp.row_handler_connected.set(true);
                    tracing::debug!("Row activation handler connected");
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

        if let Some(list_box) = imp.list_box.borrow().as_ref() {
            // Remove load more row temporarily
            let load_more_row = imp.load_more_row.take();
            if let Some(ref row) = load_more_row {
                list_box.remove(row);
            }

            // Add new messages (filtered)
            for msg in &messages {
                if self.message_matches(msg) {
                    self.add_message_row(list_box, msg);
                }
            }

            // Update count and stored messages
            imp.message_count.set(imp.message_count.get() + messages.len());
            imp.messages.borrow_mut().extend(messages);

            // Re-add load more row if we have one
            if let Some(row) = load_more_row {
                list_box.append(&row);
                imp.load_more_row.replace(Some(row));
            }
        }

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

        // Unread indicator (blue dot)
        let unread_indicator = gtk4::Box::builder()
            .width_request(8)
            .valign(gtk4::Align::Start)
            .margin_top(6)
            .build();

        if !msg.is_read {
            let dot = gtk4::DrawingArea::builder()
                .width_request(8)
                .height_request(8)
                .build();
            dot.set_draw_func(|_, cr, width, height| {
                cr.set_source_rgb(0.2, 0.5, 1.0); // Blue
                cr.arc(width as f64 / 2.0, height as f64 / 2.0, 4.0, 0.0, 2.0 * std::f64::consts::PI);
                let _ = cr.fill();
            });
            unread_indicator.append(&dot);
        }
        hbox.append(&unread_indicator);

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

        // Star indicator
        if msg.is_starred {
            let star = gtk4::Image::from_icon_name("starred-symbolic");
            star.add_css_class("warning");
            star.set_pixel_size(14);
            top_row.append(&star);
        }

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

        // Add separator between messages
        list_box.append(&row);
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
    pub to: String,
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
            to: db_msg.to_addresses.clone().unwrap_or_default(),
            date: db_msg.date_sent.clone().unwrap_or_default(),
            date_epoch: db_msg.date_epoch,
            snippet: db_msg.snippet.clone(),
            is_read: db_msg.is_read,
            is_starred: db_msg.is_starred,
            has_attachments: db_msg.has_attachments,
        }
    }
}
