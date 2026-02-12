//! Main application window

use crate::application::{NorthMailApplication, ParsedAttachment};
use crate::widgets::{FolderSidebar, MessageList, MessageView};
use gtk4::{gio, glib, prelude::*, subclass::prelude::*};
use libadwaita as adw;
use libadwaita::prelude::*;
use std::rc::Rc;
use tracing::debug;

/// Mode for compose dialog
#[derive(Clone)]
pub enum ComposeMode {
    New {
        to: Option<(String, String)>,  // Optional (email, display_name) for pre-filled recipient
    },
    Reply {
        to: String,           // email address
        to_display: String,   // display name (for chip UI)
        subject: String,
        quoted_body: String,
        in_reply_to: Option<String>,
        references: Vec<String>,
    },
    ReplyAll {
        to: Vec<(String, String)>,   // (email, display_name) pairs
        cc: Vec<(String, String)>,   // (email, display_name) pairs
        subject: String,
        quoted_body: String,
        in_reply_to: Option<String>,
        references: Vec<String>,
    },
    Forward {
        subject: String,
        quoted_body: String,
        attachments: Vec<(String, String, Vec<u8>)>, // (filename, mime_type, data)
    },
    EditDraft {
        to: Vec<String>,       // recipient emails
        cc: Vec<String>,       // cc emails
        subject: String,
        body: String,
        draft_uid: u32,        // UID of draft to delete after sending
        account_index: u32,    // Account the draft belongs to
    },
}

/// Extract email address from a "Name <email>" or "email" string
fn extract_email_address(from: &str) -> String {
    if let Some(start) = from.find('<') {
        if let Some(end) = from.find('>') {
            return from[start + 1..end].trim().to_string();
        }
    }
    from.trim().to_string()
}

/// Format the quoted body for reply
fn format_quoted_body(from: &str, date: &str, body: &str) -> String {
    let mut quoted = format!("\n\nOn {}, {} wrote:\n", date, from);
    for line in body.lines() {
        quoted.push_str(&format!("> {}\n", line));
    }
    quoted
}

/// Format the body for forward
fn format_forward_body(from: &str, to: &[String], date: &str, subject: &str, body: &str) -> String {
    let mut fwd = String::from("\n\n---------- Forwarded message ----------\n");
    fwd.push_str(&format!("From: {}\n", from));
    fwd.push_str(&format!("Date: {}\n", date));
    fwd.push_str(&format!("Subject: {}\n", subject));
    if !to.is_empty() {
        fwd.push_str(&format!("To: {}\n", to.join(", ")));
    }
    fwd.push('\n');
    fwd.push_str(body);
    fwd
}

/// Generate a color from a string (for avatar background)
fn string_to_avatar_color(s: &str) -> (f64, f64, f64) {
    let colors: [(f64, f64, f64); 12] = [
        (0.29, 0.56, 0.85), // #4A90D9
        (0.91, 0.30, 0.24), // #E74C3C
        (0.18, 0.80, 0.44), // #2ECC71
        (0.61, 0.35, 0.71), // #9B59B6
        (0.95, 0.61, 0.07), // #F39C12
        (0.10, 0.74, 0.61), // #1ABC9C
        (0.91, 0.12, 0.39), // #E91E63
        (0.25, 0.32, 0.71), // #3F51B5
        (0.00, 0.74, 0.83), // #00BCD4
        (0.55, 0.76, 0.29), // #8BC34A
        (1.00, 0.34, 0.13), // #FF5722
        (0.38, 0.49, 0.55), // #607D8B
    ];

    let hash: usize = s.bytes().fold(0, |acc, b| acc.wrapping_add(b as usize));
    colors[hash % colors.len()]
}

/// Get initials from a name or email
fn get_initials(name: &str, email: &str) -> String {
    let display = if name.is_empty() || name == email || name.contains('@') {
        email.split('@').next().unwrap_or("?")
    } else {
        // Remove email part if present
        if name.contains('<') {
            name.split('<').next().unwrap_or(name).trim()
        } else {
            name
        }
    };

    let words: Vec<&str> = display.split_whitespace().collect();
    match words.len() {
        0 => "?".to_string(),
        1 => words[0].chars().next().unwrap_or('?').to_uppercase().to_string(),
        _ => {
            let first = words[0].chars().next().unwrap_or('?');
            let last = words[words.len() - 1].chars().next().unwrap_or('?');
            format!("{}{}", first, last).to_uppercase()
        }
    }
}

/// Create an avatar widget with initials
fn create_avatar(name: &str, email: &str) -> gtk4::Widget {
    let initials = get_initials(name, email);
    let (r, g, b) = string_to_avatar_color(email);

    let drawing_area = gtk4::DrawingArea::builder()
        .width_request(40)
        .height_request(40)
        .valign(gtk4::Align::Center)
        .build();

    drawing_area.set_draw_func(move |_, cr, width, height| {
        // Draw circle
        let radius = (width.min(height) as f64) / 2.0;
        let cx = width as f64 / 2.0;
        let cy = height as f64 / 2.0;

        cr.arc(cx, cy, radius, 0.0, 2.0 * std::f64::consts::PI);
        cr.set_source_rgb(r, g, b);
        let _ = cr.fill();

        // Draw text
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.select_font_face("Sans", gtk4::cairo::FontSlant::Normal, gtk4::cairo::FontWeight::Bold);
        cr.set_font_size(16.0);

        let extents = cr.text_extents(&initials).unwrap();
        let x = cx - extents.width() / 2.0 - extents.x_bearing();
        let y = cy - extents.height() / 2.0 - extents.y_bearing();

        cr.move_to(x, y);
        let _ = cr.show_text(&initials);
    });

    drawing_area.upcast()
}

mod imp {
    use super::*;
    use libadwaita::subclass::prelude::*;
    use std::cell::OnceCell;

    #[derive(Default, gtk4::CompositeTemplate)]
    #[template(string = r#"
        <?xml version="1.0" encoding="UTF-8"?>
        <interface>
            <template class="NorthMailWindow" parent="AdwApplicationWindow">
                <property name="title">NorthMail</property>
                <property name="default-width">1200</property>
                <property name="default-height">800</property>
                <property name="content">
                    <object class="AdwToastOverlay" id="toast_overlay">
                        <property name="child">
                            <object class="AdwToolbarView">
                                <child type="top">
                                    <object class="AdwHeaderBar" id="header_bar">
                                        <property name="show-title">false</property>
                                        <child type="start">
                                            <object class="GtkBox">
                                                <property name="orientation">horizontal</property>
                                                <property name="spacing">2</property>
                                                <property name="margin-start">4</property>
                                                <child>
                                                    <object class="GtkImage">
                                                        <property name="icon-name">org.northmail.NorthMail</property>
                                                        <property name="pixel-size">28</property>
                                                    </object>
                                                </child>
                                                <child>
                                                    <object class="GtkLabel">
                                                        <property name="label">NorthMail</property>
                                                        <attributes>
                                                            <attribute name="weight" value="bold"/>
                                                        </attributes>
                                                    </object>
                                                </child>
                                            </object>
                                        </child>
                                        <child type="end">
                                            <object class="GtkButton" id="settings_button">
                                                <property name="icon-name">emblem-system-symbolic</property>
                                                <property name="tooltip-text">Settings</property>
                                                <property name="action-name">app.show-settings</property>
                                            </object>
                                        </child>
                                        <child type="end">
                                            <object class="GtkButton" id="refresh_button">
                                                <property name="icon-name">view-refresh-symbolic</property>
                                                <property name="tooltip-text">Refresh</property>
                                                <property name="action-name">win.refresh</property>
                                            </object>
                                        </child>
                                    </object>
                                </child>
                                <property name="content">
                                    <object class="GtkPaned" id="outer_paned">
                                        <property name="orientation">horizontal</property>
                                        <property name="shrink-start-child">true</property>
                                        <property name="shrink-end-child">false</property>
                                        <property name="resize-start-child">true</property>
                                        <property name="resize-end-child">true</property>
                                        <property name="position">240</property>
                                        <property name="start-child">
                                            <object class="GtkBox" id="sidebar_box">
                                                <property name="orientation">vertical</property>
                                            </object>
                                        </property>
                                        <property name="end-child">
                                            <object class="GtkPaned" id="inner_paned">
                                                <property name="orientation">horizontal</property>
                                                <property name="shrink-start-child">false</property>
                                                <property name="shrink-end-child">false</property>
                                                <property name="resize-start-child">true</property>
                                                <property name="resize-end-child">true</property>
                                                <property name="position">400</property>
                                                <property name="start-child">
                                                    <object class="GtkBox" id="message_list_box">
                                                        <property name="orientation">vertical</property>
                                                        <property name="width-request">300</property>
                                                    </object>
                                                </property>
                                                <property name="end-child">
                                                    <object class="GtkBox" id="message_view_box">
                                                        <property name="orientation">vertical</property>
                                                        <property name="width-request">300</property>
                                                    </object>
                                                </property>
                                            </object>
                                        </property>
                                    </object>
                                </property>
                            </object>
                        </property>
                    </object>
                </property>
            </template>
        </interface>
    "#)]
    pub struct NorthMailWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub header_bar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub outer_paned: TemplateChild<gtk4::Paned>,
        /// Sidebar toggle button (created in setup_widgets)
        pub sidebar_toggle: std::cell::RefCell<Option<gtk4::ToggleButton>>,
        #[template_child]
        pub inner_paned: TemplateChild<gtk4::Paned>,
        #[template_child]
        pub sidebar_box: TemplateChild<gtk4::Box>,
        #[template_child]
        pub message_list_box: TemplateChild<gtk4::Box>,
        #[template_child]
        pub message_view_box: TemplateChild<gtk4::Box>,

        pub folder_sidebar: OnceCell<FolderSidebar>,
        pub message_list: OnceCell<MessageList>,
        pub message_view: OnceCell<MessageView>,
        /// Loading status label (for updating loading progress)
        pub loading_label: std::cell::RefCell<Option<gtk4::Label>>,
        /// Loading progress label (e.g., "24 of 150 messages")
        pub loading_progress_label: std::cell::RefCell<Option<gtk4::Label>>,
        /// Currently displayed message UID (to avoid reloading the same message)
        pub current_message_uid: std::cell::RefCell<Option<u32>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NorthMailWindow {
        const NAME: &'static str = "NorthMailWindow";
        type Type = super::NorthMailWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for NorthMailWindow {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            obj.setup_widgets();
            obj.setup_actions();
            obj.setup_bindings();
        }
    }

    impl WidgetImpl for NorthMailWindow {}
    impl WindowImpl for NorthMailWindow {}
    impl ApplicationWindowImpl for NorthMailWindow {}
    impl AdwApplicationWindowImpl for NorthMailWindow {}
}

glib::wrapper! {
    pub struct NorthMailWindow(ObjectSubclass<imp::NorthMailWindow>)
        @extends adw::ApplicationWindow, gtk4::ApplicationWindow, gtk4::Window, gtk4::Widget,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl NorthMailWindow {
    pub fn new(app: &NorthMailApplication) -> Self {
        glib::Object::builder()
            .property("application", app)
            .build()
    }

    /// Add a toast notification
    pub fn add_toast(&self, toast: adw::Toast) {
        self.imp().toast_overlay.add_toast(toast);
    }

    fn setup_widgets(&self) {
        let imp = self.imp();

        // Add custom CSS for flat sidebar toggle (no background in any state)
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string(
            "button.sidebar-toggle-flat,
             button.sidebar-toggle-flat:checked,
             button.sidebar-toggle-flat:active {
                 background: transparent;
                 background-color: transparent;
                 box-shadow: none;
                 border: none;
                 outline: none;
                 transition: margin 200ms ease-out;
             }
             button.sidebar-toggle-flat:hover,
             button.sidebar-toggle-flat:hover:checked {
                 background: alpha(currentColor, 0.1);
             }
             .header-button-animated {
                 transition: margin 200ms ease-out;
             }
             #message_list_box {
                 background-color: @view_bg_color;
             }
             /* Make entire message view area white */
             #inner_paned,
             #message_view_box,
             #message_view_box > *,
             #message_view_box scrolledwindow,
             #message_view_box scrolledwindow > *,
             #message_view_box viewport,
             #message_view_box viewport > *,
             .message-view-content {
                 background-color: white;
                 background: white;
             }
             /* Force white on inner paned end child */
             #inner_paned > :last-child {
                 background-color: white;
                 background: white;
             }
             /* Message view header card */
             .message-header-card {
                 background-color: #f5f5f5;
                 border-radius: 12px;
                 margin: 12px;
             }
             .message-action-bar {
                 padding: 8px 12px;
                 border-bottom: 1px solid alpha(black, 0.06);
             }
             .message-action-bar button {
                 background: transparent;
                 box-shadow: none;
             }
             .message-header-content {
                 padding: 12px 16px;
             }
             .message-subject-large {
                 font-size: 18px;
                 font-weight: 600;
             }
             .message-sender-name {
                 font-size: 14px;
                 font-weight: 600;
             }
             .message-sender-email {
                 font-size: 12px;
                 color: alpha(@view_fg_color, 0.6);
             }
             .message-date-small {
                 font-size: 12px;
                 color: alpha(@view_fg_color, 0.6);
             }
             .message-recipients-label {
                 font-size: 12px;
                 color: alpha(@view_fg_color, 0.5);
                 min-width: 28px;
             }
             .message-recipients-value {
                 font-size: 12px;
                 color: alpha(@view_fg_color, 0.7);
             }
             /* Sender chip with hover effect */
             .sender-chip {
                 padding: 6px 10px;
                 border-radius: 8px;
                 background: transparent;
                 transition: background 150ms ease;
             }
             .sender-chip:hover {
                 background: alpha(@view_fg_color, 0.08);
                 cursor: pointer;
             }
             /* Context menu item styling */
             .context-menu-item {
                 font-size: 13px;
                 padding: 6px 12px;
                 min-height: 28px;
             }
             .context-menu-item > label {
                 font-size: 13px;
             }
             /* Clickable sender button styling */
             .sender-clickable {
                 padding: 0;
                 margin: 0;
                 background: transparent;
                 border: none;
                 box-shadow: none;
             }
             .sender-clickable:hover {
                 background: alpha(@accent_bg_color, 0.1);
                 border-radius: 6px;
             }
             .message-body-area {
                 background-color: white;
                 padding: 0 16px 16px 16px;
             }
             .message-body-scrolled,
             .message-body-scrolled > *,
             .message-body-box {
                 background-color: white;
                 background: white;
             }
             /* Hide paned separators */
             paned > separator {
                 min-width: 1px;
                 min-height: 1px;
                 background: none;
                 background-color: transparent;
                 border: none;
                 box-shadow: none;
                 opacity: 0;
             }
             .sidebar-pane {
                 border-right: none;
                 border: none;
                 box-shadow: none;
             }
             /* Star button styling */
             .star-button {
                 color: alpha(@window_fg_color, 0.3);
                 min-width: 24px;
                 min-height: 24px;
                 padding: 2px;
             }
             .star-button:hover {
                 color: #f5c211;
             }
             .star-button:checked,
             .star-button:checked:hover {
                 color: #f5c211;
             }
             .star-button:checked image,
             .star-button image:checked {
                 color: #f5c211;
                 -gtk-icon-style: symbolic;
             }
             /* Star button in selected row - ensure visibility */
             row:selected .star-button {
                 color: rgba(255, 255, 255, 0.5);
             }
             row:selected .star-button:checked {
                 color: #f5c211;
             }
             row:selected .star-button:hover {
                 color: #f5c211;
             }
             /* Star indicator in message list */
             .star-indicator {
                 color: #f5c211;
             }"
        );
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER + 1,
        );

        // Create sidebar toggle button and add to header bar
        // Position: after the title, with margin to align with sidebar's right edge
        let sidebar_toggle = gtk4::ToggleButton::builder()
            .icon_name("dock-left-symbolic")
            .tooltip_text("Toggle Sidebar")
            .active(true)
            .margin_start(92)  // Initial position, updated dynamically based on paned
            .build();
        sidebar_toggle.add_css_class("flat");
        sidebar_toggle.add_css_class("sidebar-toggle-flat");
        imp.header_bar.pack_start(&sidebar_toggle);
        imp.sidebar_toggle.replace(Some(sidebar_toggle.clone()));

        // Create compose button and add to header bar
        // Position: after sidebar toggle, aligned with message list left edge
        let compose_button = gtk4::Button::builder()
            .icon_name("mail-message-new-symbolic")
            .tooltip_text("Compose")
            .margin_start(0)
            .build();
        compose_button.add_css_class("flat");
        compose_button.add_css_class("header-button-animated");
        compose_button.set_action_name(Some("win.compose"));
        imp.header_bar.pack_start(&compose_button);

        // Sidebar toggle functionality using paned position
        let outer_paned = imp.outer_paned.clone();
        let saved_position = std::rc::Rc::new(std::cell::Cell::new(240i32));
        let is_toggling = std::rc::Rc::new(std::cell::Cell::new(false));
        let saved_pos_clone = saved_position.clone();
        let is_toggling_clone = is_toggling.clone();
        sidebar_toggle.connect_toggled(move |toggle| {
            is_toggling_clone.set(true);
            if toggle.is_active() {
                // Show sidebar: restore saved position
                outer_paned.set_position(saved_pos_clone.get().max(200));
            } else {
                // Hide sidebar: save current position and set to 0
                let pos = outer_paned.position();
                if pos > 0 {
                    saved_pos_clone.set(pos);
                }
                outer_paned.set_position(0);
            }
            is_toggling_clone.set(false);
        });

        // Helper function to calculate button positions
        let calc_toggle_margin = |outer_pos: i32| -> i32 {
            // Position toggle button at right edge of sidebar
            // Account for: icon+title (~100px), button width (~32px), padding (~16px)
            let header_title_width = 100;
            let button_width = 32;
            let padding = 16;
            outer_pos.saturating_sub(header_title_width + button_width + padding).max(8)
        };

        let calc_compose_margin = |outer_pos: i32, inner_pos: i32| -> i32 {
            // Position compose button at left edge of message view
            // When sidebar visible: needs to account for sidebar width
            // When sidebar hidden: message list starts at left edge
            if outer_pos == 0 {
                // Sidebar collapsed - compose above filter button
                let offset = 215; // header title + toggle button + padding
                inner_pos.saturating_sub(offset).max(8)
            } else {
                // Sidebar visible - only inner_pos matters (toggle already at sidebar edge)
                let offset = 58;
                inner_pos.saturating_sub(offset).max(8)
            }
        };

        // Update button positions when outer paned position changes
        let toggle_for_signal = sidebar_toggle.clone();
        let compose_for_outer = compose_button.clone();
        let is_toggling_for_signal = is_toggling.clone();
        let saved_pos_for_signal = saved_position.clone();
        let inner_paned_for_outer = imp.inner_paned.clone();
        imp.outer_paned.connect_notify_local(Some("position"), move |paned, _| {
            let pos = paned.position();

            // Enforce minimum width of 200 when user is dragging (not toggling)
            if !is_toggling_for_signal.get() && pos > 0 && pos < 200 {
                paned.set_position(200);
                return;
            }

            // Save position if it's a valid sidebar width
            if pos >= 200 {
                saved_pos_for_signal.set(pos);
            }

            // Update toggle state based on position
            if pos == 0 && toggle_for_signal.is_active() {
                toggle_for_signal.set_active(false);
            } else if pos > 0 && !toggle_for_signal.is_active() {
                toggle_for_signal.set_active(true);
            }

            toggle_for_signal.set_margin_start(calc_toggle_margin(pos));
            // Update compose position when sidebar collapses/expands
            compose_for_outer.set_margin_start(calc_compose_margin(pos, inner_paned_for_outer.position()));
        });

        // Update compose button position when inner paned position changes
        let compose_for_inner = compose_button.clone();
        let outer_paned_for_inner = imp.outer_paned.clone();
        imp.inner_paned.connect_notify_local(Some("position"), move |paned, _| {
            let inner_pos = paned.position();
            let outer_pos = outer_paned_for_inner.position();
            compose_for_inner.set_margin_start(calc_compose_margin(outer_pos, inner_pos));
        });

        // Set initial button positions
        let initial_outer_pos = imp.outer_paned.position();
        let initial_inner_pos = imp.inner_paned.position();
        sidebar_toggle.set_margin_start(calc_toggle_margin(initial_outer_pos));
        compose_button.set_margin_start(calc_compose_margin(initial_outer_pos, initial_inner_pos));

        // Create and add folder sidebar
        let folder_sidebar = FolderSidebar::new();
        imp.sidebar_box.append(&folder_sidebar);

        // Connect message-dropped signal for drag-and-drop move
        let window = self.clone();
        folder_sidebar.connect_message_dropped(move |_sidebar, uid, msg_id, source_account_id, source_folder_path, target_account_id, target_folder_path| {
            debug!(
                "Message dropped: uid={}, from {}/{}, to {}/{}",
                uid, source_account_id, source_folder_path, target_account_id, target_folder_path
            );
            if let Some(app) = window.application() {
                if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                    // Move the message (returns false for cross-account moves)
                    if app.move_message_to_folder(msg_id, uid, source_account_id, source_folder_path, target_account_id, target_folder_path) {
                        // Remove from message list UI
                        let imp = window.imp();
                        if let Some(message_list) = imp.message_list.get() {
                            message_list.remove_message(uid);
                        }

                        // Clear message view if this message was being displayed
                        if *imp.current_message_uid.borrow() == Some(uid) {
                            while let Some(child) = imp.message_view_box.first_child() {
                                imp.message_view_box.remove(&child);
                            }
                            *imp.current_message_uid.borrow_mut() = None;
                        }

                        // Extract just the folder name for a friendlier message
                        let folder_name = target_folder_path.rsplit('/').next().unwrap_or(target_folder_path);
                        window.add_toast(adw::Toast::new(&format!("Moved to {}", folder_name)));
                    } else {
                        window.add_toast(adw::Toast::new("Cannot move between different accounts"));
                    }
                }
            }
        });

        imp.folder_sidebar.set(folder_sidebar).unwrap();

        // Create and add message list
        let message_list = MessageList::new();
        imp.message_list_box.append(&message_list);

        // Connect message selection to show in message view
        let window = self.clone();
        message_list.connect_message_selected(move |list, uid| {
            debug!("Message selected: UID {}", uid);
            window.show_message(list, uid);
        });

        // Connect search-requested signal (Enter in search bar / Escape to clear)
        let window = self.clone();
        message_list.connect_search_requested(move |_list, query| {
            debug!("Search requested: {:?}", query);
            window.handle_search_requested(&query);
        });

        // Connect filter-changed callback (triggers DB-level filtering)
        let window = self.clone();
        message_list.connect_filter_changed(move || {
            if let Some(app) = window.application() {
                if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                    app.handle_filter_changed();
                }
            }
        });

        // Connect star-toggled callback (star button clicked in message list)
        let window = self.clone();
        message_list.connect_star_toggled(move |list, uid, msg_id, folder_id, is_starred| {
            debug!("Star toggled in list: uid={}, is_starred={}", uid, is_starred);
            // Update the message info in the list
            if is_starred {
                list.update_message_starred(uid, true);
            } else {
                list.update_message_starred(uid, false);
            }
            // Sync to database and IMAP via application
            if let Some(app) = window.application() {
                if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                    app.set_message_starred(msg_id, uid, folder_id, is_starred);
                }
            }
        });

        imp.message_list.set(message_list).unwrap();

        // Create and add message view
        let message_view = MessageView::new();
        imp.message_view_box.append(&message_view);
        imp.message_view.set(message_view).unwrap();

        // Show welcome state if no accounts
        self.show_welcome_state();
    }

    /// Show a message in the message view
    fn show_message(&self, message_list: &MessageList, uid: u32) {
        let imp = self.imp();

        // Skip if already showing this message
        if *imp.current_message_uid.borrow() == Some(uid) {
            debug!("Message UID {} already displayed, skipping reload", uid);
            return;
        }

        // Find the message in the list
        let messages = message_list.imp().messages.borrow();
        let msg = messages.iter().find(|m| m.uid == uid).cloned();
        drop(messages); // Release borrow

        if let Some(msg) = msg {
            // Track the currently displayed message
            *imp.current_message_uid.borrow_mut() = Some(uid);

            // Clear current content
            while let Some(child) = imp.message_view_box.first_child() {
                imp.message_view_box.remove(&child);
            }

            // Create message view content - white background container
            let content = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(0)
                .vexpand(true)
                .css_classes(["message-view-content"])
                .build();

            // Header card (floating rounded box)
            let header_card = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .css_classes(["message-header-card"])
                .build();

            // Action bar at top of card
            let toolbar = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(6)
                .css_classes(["message-action-bar"])
                .build();

            let reply_button = gtk4::Button::builder()
                .icon_name("mail-reply-sender-symbolic")
                .tooltip_text("Reply")
                .css_classes(["flat"])
                .build();

            let reply_all_button = gtk4::Button::builder()
                .icon_name("mail-reply-all-symbolic")
                .tooltip_text("Reply All")
                .css_classes(["flat"])
                .build();

            let forward_button = gtk4::Button::builder()
                .icon_name("mail-forward-symbolic")
                .tooltip_text("Forward")
                .css_classes(["flat"])
                .build();

            // Shared state for body text (populated when body loads)
            let body_text: Rc<std::cell::RefCell<Option<String>>> = Rc::new(std::cell::RefCell::new(None));
            // Shared state for attachments (populated when body loads)
            let attachments_data: Rc<std::cell::RefCell<Vec<(String, String, Vec<u8>)>>> = Rc::new(std::cell::RefCell::new(Vec::new()));

            // Connect reply button
            {
                let window = self.clone();
                let msg_clone = msg.clone();
                let body_text = body_text.clone();
                reply_button.connect_clicked(move |_| {
                    let body = body_text.borrow().clone().unwrap_or_else(|| {
                        "(Message body is still loading...)".to_string()
                    });
                    // Use from_address if it looks like an email, otherwise extract from 'from'
                    let reply_to_email = if !msg_clone.from_address.is_empty() && msg_clone.from_address.contains('@') {
                        msg_clone.from_address.clone()
                    } else {
                        extract_email_address(&msg_clone.from)
                    };
                    let reply_to_display = msg_clone.from.clone();
                    let subject = if msg_clone.subject.to_lowercase().starts_with("re:") {
                        msg_clone.subject.clone()
                    } else {
                        format!("Re: {}", msg_clone.subject)
                    };
                    let quoted = format_quoted_body(&msg_clone.from, &msg_clone.date, &body);
                    let mode = ComposeMode::Reply {
                        to: reply_to_email,
                        to_display: reply_to_display,
                        subject,
                        quoted_body: quoted,
                        in_reply_to: None,
                        references: Vec::new(),
                    };
                    window.show_compose_dialog_with_mode(mode);
                });
            }

            // Connect reply-all button
            {
                let window = self.clone();
                let msg_clone = msg.clone();
                let body_text = body_text.clone();
                reply_all_button.connect_clicked(move |_| {
                    let body = body_text.borrow().clone().unwrap_or_else(|| {
                        "(Message body is still loading...)".to_string()
                    });
                    // Use from_address if it looks like an email, otherwise extract from 'from'
                    let reply_to_email = if !msg_clone.from_address.is_empty() && msg_clone.from_address.contains('@') {
                        msg_clone.from_address.clone()
                    } else {
                        extract_email_address(&msg_clone.from)
                    };
                    let reply_to_display = msg_clone.from.clone();
                    // For reply-all, include the sender as primary To
                    let to_addrs = vec![(reply_to_email.clone(), reply_to_display)];
                    // Parse additional recipients from the To field (comma-separated)
                    let cc_addrs: Vec<(String, String)> = msg_clone.to
                        .split(',')
                        .map(|s| {
                            let email = extract_email_address(s.trim());
                            (email.clone(), email) // email as both display and address
                        })
                        .filter(|(e, _)| !e.is_empty() && e.contains('@') && e != &reply_to_email)
                        .collect();

                    let subject = if msg_clone.subject.to_lowercase().starts_with("re:") {
                        msg_clone.subject.clone()
                    } else {
                        format!("Re: {}", msg_clone.subject)
                    };
                    let quoted = format_quoted_body(&msg_clone.from, &msg_clone.date, &body);
                    let mode = ComposeMode::ReplyAll {
                        to: to_addrs,
                        cc: cc_addrs,
                        subject,
                        quoted_body: quoted,
                        in_reply_to: None,
                        references: Vec::new(),
                    };
                    window.show_compose_dialog_with_mode(mode);
                });
            }

            // Connect forward button
            {
                let window = self.clone();
                let msg_clone = msg.clone();
                let body_text = body_text.clone();
                let attachments_data = attachments_data.clone();
                forward_button.connect_clicked(move |_| {
                    let body = body_text.borrow().clone().unwrap_or_else(|| {
                        "(Message body is still loading...)".to_string()
                    });
                    let subject = if msg_clone.subject.to_lowercase().starts_with("fwd:") {
                        msg_clone.subject.clone()
                    } else {
                        format!("Fwd: {}", msg_clone.subject)
                    };
                    let to_list: Vec<String> = msg_clone.to
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect();
                    let quoted = format_forward_body(&msg_clone.from, &to_list, &msg_clone.date, &msg_clone.subject, &body);

                    let stored_attachments = attachments_data.borrow().clone();
                    if !stored_attachments.is_empty() {
                        // Ask user if they want to include attachments
                        let dialog = adw::AlertDialog::builder()
                            .heading("Include Attachments?")
                            .body(&format!("This message has {} attachment{}. Do you want to include {} in the forwarded message?",
                                stored_attachments.len(),
                                if stored_attachments.len() == 1 { "" } else { "s" },
                                if stored_attachments.len() == 1 { "it" } else { "them" }))
                            .build();
                        dialog.add_response("no", "No");
                        dialog.add_response("yes", "Yes");
                        dialog.set_response_appearance("yes", adw::ResponseAppearance::Suggested);
                        dialog.set_default_response(Some("yes"));

                        let window_ref = window.clone();
                        let subject_ref = subject.clone();
                        let quoted_ref = quoted.clone();
                        let attachments_ref = stored_attachments.clone();
                        dialog.choose(window.upcast_ref::<gtk4::Window>(), None::<&gio::Cancellable>, move |response| {
                            let attachments = if response == "yes" {
                                attachments_ref.clone()
                            } else {
                                Vec::new()
                            };
                            let mode = ComposeMode::Forward {
                                subject: subject_ref.clone(),
                                quoted_body: quoted_ref.clone(),
                                attachments,
                            };
                            window_ref.show_compose_dialog_with_mode(mode);
                        });
                    } else {
                        // No attachments, forward directly
                        let mode = ComposeMode::Forward {
                            subject,
                            quoted_body: quoted,
                            attachments: Vec::new(),
                        };
                        window.show_compose_dialog_with_mode(mode);
                    }
                });
            }

            let archive_button = gtk4::Button::builder()
                .icon_name("folder-symbolic")
                .tooltip_text("Archive")
                .css_classes(["flat"])
                .build();

            let delete_button = gtk4::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text("Delete")
                .css_classes(["flat"])
                .build();

            // Edit button (only visible for drafts) - blue accent color
            let edit_button = gtk4::Button::builder()
                .label("Edit")
                .css_classes(["suggested-action"])
                .build();

            // Check if we're viewing drafts folder
            let is_drafts = if let Some(app) = self.application() {
                if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                    app.current_folder_type() == "drafts"
                } else {
                    false
                }
            } else {
                false
            };
            edit_button.set_visible(is_drafts);

            // Connect edit button
            if is_drafts {
                let window = self.clone();
                let msg_clone = msg.clone();
                let body_text = body_text.clone();
                edit_button.connect_clicked(move |_| {
                    // Check if body is loaded yet
                    let body = match body_text.borrow().clone() {
                        Some(b) => b,
                        None => {
                            window.add_toast(adw::Toast::new("Please wait for the message to load"));
                            return;
                        }
                    };

                    // Parse To addresses, removing placeholder (sender's own email)
                    let sender_email = if let Some(app) = window.application() {
                        if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                            let accs = app.imp().accounts.borrow();
                            accs.first().map(|a| a.email.clone()).unwrap_or_default()
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };

                    let to: Vec<String> = msg_clone.to
                        .split(',')
                        .map(|s| extract_email_address(s.trim()))
                        .filter(|e| !e.is_empty() && e != &sender_email)
                        .collect();

                    // For now, CC is not stored in message view - would need to parse from raw headers
                    let cc: Vec<String> = Vec::new();

                    let mode = ComposeMode::EditDraft {
                        to,
                        cc,
                        subject: msg_clone.subject.clone(),
                        body,
                        draft_uid: msg_clone.uid,
                        account_index: 0, // TODO: get correct account index
                    };
                    window.show_compose_dialog_with_mode(mode);
                });
            }

            // Connect archive button
            {
                let window = self.clone();
                let message_id = msg.id;
                let msg_uid = msg.uid;
                let msg_folder_id = msg.folder_id;
                archive_button.connect_clicked(move |_| {
                    debug!("Archive button clicked: uid={}", msg_uid);
                    if let Some(app) = window.application() {
                        if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                            app.archive_message(message_id, msg_uid, msg_folder_id);
                            // Update message list by removing this message
                            let imp = window.imp();
                            if let Some(message_list) = imp.message_list.get() {
                                message_list.remove_message(msg_uid);
                            }
                            // Clear message view
                            while let Some(child) = imp.message_view_box.first_child() {
                                imp.message_view_box.remove(&child);
                            }
                            *imp.current_message_uid.borrow_mut() = None;
                            window.add_toast(adw::Toast::new("Message archived"));
                        }
                    }
                });
            }

            // Connect delete button
            {
                let window = self.clone();
                let message_id = msg.id;
                let msg_uid = msg.uid;
                let msg_folder_id = msg.folder_id;
                delete_button.connect_clicked(move |btn| {
                    debug!("Delete button clicked: uid={}", msg_uid);
                    let window = window.clone();

                    // Show confirmation dialog
                    let dialog = adw::AlertDialog::builder()
                        .heading("Delete Message?")
                        .body("This message will be moved to Trash.")
                        .build();

                    dialog.add_response("cancel", "Cancel");
                    dialog.add_response("delete", "Delete");
                    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                    dialog.set_default_response(Some("cancel"));
                    dialog.set_close_response("cancel");

                    dialog.connect_response(None, move |_, response| {
                        debug!("Delete dialog response: {}", response);
                        if response == "delete" {
                            debug!("User confirmed delete for uid={}", msg_uid);
                            if let Some(app) = window.application() {
                                if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                                    debug!("Calling delete_message");
                                    app.delete_message(message_id, msg_uid, msg_folder_id);
                                    // Update message list by removing this message
                                    let imp = window.imp();
                                    if let Some(message_list) = imp.message_list.get() {
                                        message_list.remove_message(msg_uid);
                                    }
                                    // Clear message view
                                    while let Some(child) = imp.message_view_box.first_child() {
                                        imp.message_view_box.remove(&child);
                                    }
                                    *imp.current_message_uid.borrow_mut() = None;
                                    window.add_toast(adw::Toast::new("Message deleted"));
                                }
                            }
                        }
                    });

                    dialog.present(Some(&btn.root().unwrap().downcast::<gtk4::Window>().unwrap()));
                });
            }

            let spacer = gtk4::Box::builder()
                .hexpand(true)
                .build();

            let star_button = gtk4::ToggleButton::builder()
                .icon_name(if msg.is_starred { "starred-symbolic" } else { "non-starred-symbolic" })
                .tooltip_text(if msg.is_starred { "Unstar" } else { "Star" })
                .active(msg.is_starred)
                .css_classes(["flat", "star-button"])
                .build();

            // Connect star button toggle
            {
                let window = self.clone();
                let message_id = msg.id;
                let msg_uid = msg.uid;
                let msg_folder_id = msg.folder_id;
                star_button.connect_toggled(move |button| {
                    let is_starred = button.is_active();
                    // Update icon and tooltip
                    button.set_icon_name(if is_starred { "starred-symbolic" } else { "non-starred-symbolic" });
                    button.set_tooltip_text(Some(if is_starred { "Unstar" } else { "Star" }));
                    // Update database and IMAP via application
                    if let Some(app) = window.application() {
                        if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                            app.set_message_starred(message_id, msg_uid, msg_folder_id, is_starred);
                        }
                    }
                    // Update the message list indicator
                    let imp = window.imp();
                    if let Some(message_list) = imp.message_list.get() {
                        message_list.update_message_starred(msg_uid, is_starred);
                    }
                });
            }

            // Read/Unread toggle button (icon shows action)
            let read_button = gtk4::ToggleButton::builder()
                .icon_name(if msg.is_read { "mail-read-symbolic" } else { "mail-unread-symbolic" })
                .tooltip_text(if msg.is_read { "Mark as Unread" } else { "Mark as Read" })
                .active(msg.is_read)
                .css_classes(["flat"])
                .build();

            // Connect read button toggle
            {
                let window = self.clone();
                let message_id = msg.id;
                let msg_uid = msg.uid;
                let msg_folder_id = msg.folder_id;
                read_button.connect_toggled(move |button| {
                    let is_read = button.is_active();
                    // Update icon and tooltip (icon shows action)
                    button.set_icon_name(if is_read { "mail-read-symbolic" } else { "mail-unread-symbolic" });
                    button.set_tooltip_text(Some(if is_read { "Mark as Unread" } else { "Mark as Read" }));
                    // Update database and IMAP via application
                    if let Some(app) = window.application() {
                        if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                            app.set_message_read(message_id, msg_uid, msg_folder_id, is_read);
                        }
                    }
                    // Update the message list indicator
                    let imp = window.imp();
                    if let Some(message_list) = imp.message_list.get() {
                        message_list.update_message_read(msg_uid, is_read);
                    }
                });
            }

            // Star and read on left, actions on right
            toolbar.append(&star_button);
            toolbar.append(&read_button);
            toolbar.append(&spacer);
            // For drafts, show Edit first; for others, show Reply/Forward
            if is_drafts {
                toolbar.append(&edit_button);
            }
            toolbar.append(&reply_button);
            toolbar.append(&reply_all_button);
            toolbar.append(&forward_button);
            toolbar.append(&archive_button);
            toolbar.append(&delete_button);

            header_card.append(&toolbar);

            // Header content inside the card
            let header_content = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(8)
                .css_classes(["message-header-content"])
                .build();

            // Sender row: Avatar + Name/Email + Date
            let sender_row = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(12)
                .build();

            // Avatar with initials
            let from_email = if !msg.from_address.is_empty() {
                msg.from_address.clone()
            } else {
                extract_email_address(&msg.from)
            };
            let from_name = msg.from.clone();
            let avatar = create_avatar(&from_name, &from_email);
            sender_row.append(&avatar);

            // Sender name and email (clickable to compose)
            let sender_info = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(2)
                .hexpand(true)
                .valign(gtk4::Align::Center)
                .build();

            let display_name = if from_name.is_empty() || from_name == from_email {
                from_email.split('@').next().unwrap_or(&from_email).to_string()
            } else {
                // Extract just the name part if it contains email
                if from_name.contains('<') {
                    from_name.split('<').next().unwrap_or(&from_name).trim().to_string()
                } else {
                    from_name.clone()
                }
            };

            let name_label = gtk4::Label::builder()
                .label(&display_name)
                .xalign(0.0)
                .css_classes(["message-sender-name"])
                .build();

            let email_label = gtk4::Label::builder()
                .label(&from_email)
                .xalign(0.0)
                .css_classes(["message-sender-email"])
                .build();

            sender_info.append(&name_label);
            sender_info.append(&email_label);

            // Wrap sender info in a box with padding for hover effect
            let sender_chip = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .css_classes(["sender-chip"])
                .build();
            sender_chip.append(&sender_info);

            // Create context menu for right-click
            let popover = gtk4::Popover::new();
            popover.set_parent(&sender_chip);
            popover.set_has_arrow(false);
            popover.add_css_class("menu");

            let menu_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(0)
                .build();

            // New Email button
            let new_email_btn = gtk4::Button::builder()
                .label("New Email")
                .css_classes(["flat", "context-menu-item"])
                .build();
            new_email_btn.set_halign(gtk4::Align::Fill);
            if let Some(child) = new_email_btn.child() {
                if let Some(label) = child.downcast_ref::<gtk4::Label>() {
                    label.set_xalign(0.0);
                }
            }
            {
                let window = self.clone();
                let to_email = from_email.clone();
                let to_name = display_name.clone();
                let popover_clone = popover.clone();
                new_email_btn.connect_clicked(move |_| {
                    popover_clone.popdown();
                    let mode = ComposeMode::New {
                        to: Some((to_email.clone(), to_name.clone())),
                    };
                    window.show_compose_dialog_with_mode(mode);
                });
            }
            menu_box.append(&new_email_btn);

            // Copy Address button
            let copy_btn = gtk4::Button::builder()
                .label("Copy Address")
                .css_classes(["flat", "context-menu-item"])
                .build();
            copy_btn.set_halign(gtk4::Align::Fill);
            if let Some(child) = copy_btn.child() {
                if let Some(label) = child.downcast_ref::<gtk4::Label>() {
                    label.set_xalign(0.0);
                }
            }
            {
                let email_for_copy = from_email.clone();
                let popover_clone = popover.clone();
                copy_btn.connect_clicked(move |btn| {
                    popover_clone.popdown();
                    let display = btn.display();
                    let clipboard = display.clipboard();
                    clipboard.set_text(&email_for_copy);
                });
            }
            menu_box.append(&copy_btn);

            // Add to Contacts button
            let add_contact_btn = gtk4::Button::builder()
                .label("Add to Contacts")
                .css_classes(["flat", "context-menu-item"])
                .build();
            add_contact_btn.set_halign(gtk4::Align::Fill);
            if let Some(child) = add_contact_btn.child() {
                if let Some(label) = child.downcast_ref::<gtk4::Label>() {
                    label.set_xalign(0.0);
                }
            }
            {
                let popover_clone = popover.clone();
                let contact_email = from_email.clone();
                let contact_name = display_name.clone();
                let window = self.clone();
                add_contact_btn.connect_clicked(move |_| {
                    popover_clone.popdown();

                    let email = contact_email.clone();
                    let name = contact_name.clone();
                    let win = window.clone();

                    // Add contact via Evolution Data Server D-Bus API
                    glib::spawn_future_local(async move {
                        match add_contact_to_eds(&name, &email).await {
                            Ok(()) => {
                                let toast = adw::Toast::new(&format!("Added {} to contacts", name));
                                win.imp().toast_overlay.add_toast(toast);
                            }
                            Err(e) => {
                                tracing::error!("Failed to add contact: {}", e);
                                let toast = adw::Toast::new("Failed to add contact");
                                win.imp().toast_overlay.add_toast(toast);
                            }
                        }
                    });
                });
            }
            menu_box.append(&add_contact_btn);

            popover.set_child(Some(&menu_box));

            // Right-click gesture for context menu
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(3); // Right mouse button
            let popover_clone = popover.clone();
            gesture.connect_released(move |_gesture, _n_press, x, y| {
                popover_clone.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                popover_clone.popup();
            });
            sender_chip.add_controller(gesture);

            // Left-click to compose
            let click_gesture = gtk4::GestureClick::new();
            click_gesture.set_button(1); // Left mouse button
            {
                let window = self.clone();
                let to_email = from_email.clone();
                let to_name = display_name.clone();
                click_gesture.connect_released(move |_, _, _, _| {
                    let mode = ComposeMode::New {
                        to: Some((to_email.clone(), to_name.clone())),
                    };
                    window.show_compose_dialog_with_mode(mode);
                });
            }
            sender_chip.add_controller(click_gesture);

            sender_row.append(&sender_chip);

            // To: row (separate from clickable sender)
            let to_display = if msg.to.is_empty() { "(sync to update)".to_string() } else { msg.to.clone() };
            let to_row = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(4)
                .margin_start(48) // Align with sender info (after avatar)
                .margin_top(4)
                .build();
            let to_label = gtk4::Label::builder()
                .label("To:")
                .css_classes(["message-recipients-label"])
                .xalign(0.0)
                .build();
            let to_value = gtk4::Label::builder()
                .label(&to_display)
                .css_classes(["message-recipients-value"])
                .xalign(0.0)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build();
            to_row.append(&to_label);
            to_row.append(&to_value);

            // Date on right — format nicely
            let formatted_date = if let Some(epoch) = msg.date_epoch {
                glib::DateTime::from_unix_local(epoch)
                    .and_then(|dt| dt.format("%b %d, %Y %H:%M"))
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| msg.date.clone())
            } else {
                msg.date.clone()
            };

            let date_label = gtk4::Label::builder()
                .label(&formatted_date)
                .css_classes(["message-date-small"])
                .valign(gtk4::Align::Start)
                .build();
            sender_row.append(&date_label);

            header_content.append(&sender_row);
            header_content.append(&to_row);

            // Cc: (if available) - shown separately below sender row
            if !msg.cc.is_empty() {
                let cc_row = gtk4::Box::builder()
                    .orientation(gtk4::Orientation::Horizontal)
                    .spacing(6)
                    .margin_top(4)
                    .build();

                let cc_label = gtk4::Label::builder()
                    .label("Cc:")
                    .css_classes(["message-recipients-label"])
                    .xalign(0.0)
                    .build();

                let cc_value = gtk4::Label::builder()
                    .label(&msg.cc)
                    .css_classes(["message-recipients-value"])
                    .xalign(0.0)
                    .hexpand(true)
                    .wrap(true)
                    .ellipsize(gtk4::pango::EllipsizeMode::End)
                    .max_width_chars(60)
                    .build();

                cc_row.append(&cc_label);
                cc_row.append(&cc_value);
                header_content.append(&cc_row);
            }

            // Subject row with attachment indicator on right
            let subject_row = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(8)
                .margin_top(8)
                .build();

            let subject_label = gtk4::Label::builder()
                .label(&msg.subject)
                .xalign(0.0)
                .wrap(true)
                .hexpand(true)
                .css_classes(["message-subject-large"])
                .build();
            subject_row.append(&subject_label);

            // Attachment indicator (populated after body fetch or from has_attachments flag)
            let attachment_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .halign(gtk4::Align::End)
                .valign(gtk4::Align::End)
                .spacing(4)
                .build();
            subject_row.append(&attachment_box);

            header_content.append(&subject_row);
            header_card.append(&header_content);
            content.append(&header_card);

            // Body area with loading indicator initially
            let body_scrolled = gtk4::ScrolledWindow::builder()
                .vexpand(true)
                .hexpand(true)
                .css_classes(["message-body-scrolled"])
                .build();

            let body_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .margin_start(16)
                .margin_end(16)
                .margin_top(12)
                .margin_bottom(12)
                .css_classes(["message-body-box"])
                .build();

            // Show loading spinner initially
            let loading_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .valign(gtk4::Align::Center)
                .halign(gtk4::Align::Center)
                .vexpand(true)
                .spacing(12)
                .build();

            let spinner = gtk4::Spinner::builder()
                .spinning(true)
                .width_request(32)
                .height_request(32)
                .build();

            let loading_label = gtk4::Label::builder()
                .label("Loading message...")
                .css_classes(["dim-label"])
                .build();

            loading_box.append(&spinner);
            loading_box.append(&loading_label);
            body_box.append(&loading_box);

            body_scrolled.set_child(Some(&body_box));
            content.append(&body_scrolled);

            imp.message_view_box.append(&content);

            // Fetch message body
            let body_box_ref = body_box.clone();
            let attachment_box_ref = attachment_box.clone();
            let body_text_for_fetch = body_text.clone();
            let attachments_data_for_fetch = attachments_data.clone();
            if let Some(app) = self.application() {
                if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                    let msg_folder_id = if msg.folder_id != 0 { Some(msg.folder_id) } else { None };
                    app.fetch_message_body(uid, msg_folder_id, move |result| {
                        // Clear loading indicator
                        while let Some(child) = body_box_ref.first_child() {
                            body_box_ref.remove(&child);
                        }

                        match result {
                            Ok(parsed) => {
                                // Store plain text for reply/forward
                                // Prefer text version, fall back to stripped HTML
                                let plain_text = if let Some(ref text) = parsed.text {
                                    text.clone()
                                } else if let Some(ref html) = parsed.html {
                                    NorthMailApplication::strip_html_tags_public(html)
                                } else {
                                    String::new()
                                };
                                *body_text_for_fetch.borrow_mut() = Some(plain_text);

                                // Store attachments for forwarding
                                let stored: Vec<(String, String, Vec<u8>)> = parsed.attachments.iter()
                                    .map(|a| (a.filename.clone(), a.mime_type.clone(), a.data.clone()))
                                    .collect();
                                *attachments_data_for_fetch.borrow_mut() = stored;

                                // Prefer HTML if available, otherwise use plain text
                                if let Some(html) = parsed.html {
                                    #[cfg(feature = "webkit")]
                                    {
                                        // Use WebKitWebView for HTML rendering
                                        use webkit6::prelude::WebViewExt;

                                        let web_view = webkit6::WebView::new();
                                        web_view.set_vexpand(true);
                                        web_view.set_hexpand(true);

                                        // Security settings for email display
                                        if let Some(settings) = WebViewExt::settings(&web_view) {
                                            settings.set_enable_javascript(false);
                                            settings.set_auto_load_images(true);
                                            settings.set_allow_modal_dialogs(false);
                                            settings.set_enable_html5_database(false);
                                            settings.set_enable_html5_local_storage(false);
                                        }

                                        // Handle WebKit process crash - show plain text fallback
                                        let body_box_crash = body_box_ref.clone();
                                        let html_fallback = html.clone();
                                        web_view.connect_web_process_terminated(move |_wv, _reason| {
                                            tracing::warn!("WebKit process crashed, falling back to plain text");
                                            while let Some(child) = body_box_crash.first_child() {
                                                body_box_crash.remove(&child);
                                            }
                                            let text = NorthMailApplication::strip_html_tags_public(&html_fallback);
                                            let text_view = gtk4::TextView::builder()
                                                .editable(false)
                                                .cursor_visible(false)
                                                .wrap_mode(gtk4::WrapMode::Word)
                                                .vexpand(true)
                                                .build();
                                            text_view.buffer().set_text(&text);
                                            body_box_crash.append(&text_view);
                                        });

                                        // Load the HTML content
                                        web_view.load_html(&html, None);

                                        body_box_ref.append(&web_view);
                                    }

                                    #[cfg(not(feature = "webkit"))]
                                    {
                                        // Fallback: strip HTML tags and show as plain text
                                        let text = NorthMailApplication::strip_html_tags_public(&html);
                                        let text_view = gtk4::TextView::builder()
                                            .editable(false)
                                            .cursor_visible(false)
                                            .wrap_mode(gtk4::WrapMode::Word)
                                            .vexpand(true)
                                            .build();

                                        text_view.buffer().set_text(&text);
                                        body_box_ref.append(&text_view);
                                    }
                                } else if let Some(text) = parsed.text {
                                    // Show plain text
                                    let text_view = gtk4::TextView::builder()
                                        .editable(false)
                                        .cursor_visible(false)
                                        .wrap_mode(gtk4::WrapMode::Word)
                                        .vexpand(true)
                                        .build();

                                    text_view.buffer().set_text(&text);
                                    body_box_ref.append(&text_view);
                                } else {
                                    let label = gtk4::Label::builder()
                                        .label("No content available")
                                        .css_classes(["dim-label"])
                                        .build();
                                    body_box_ref.append(&label);
                                }

                                // Show attachment dropdown in header if any
                                if !parsed.attachments.is_empty() {
                                    let count = parsed.attachments.len();

                                    // Build button content: attachment icon + count
                                    let btn_content = gtk4::Box::builder()
                                        .orientation(gtk4::Orientation::Horizontal)
                                        .spacing(4)
                                        .build();
                                    btn_content.append(&gtk4::Image::from_icon_name("mail-attachment-symbolic"));
                                    btn_content.append(&gtk4::Label::builder()
                                        .label(&format!("{}", count))
                                        .css_classes(["caption"])
                                        .build());

                                    let menu_btn = gtk4::MenuButton::builder()
                                        .child(&btn_content)
                                        .tooltip_text(&format!("{} attachment{}", count, if count == 1 { "" } else { "s" }))
                                        .css_classes(["flat"])
                                        .direction(gtk4::ArrowType::Down)
                                        .build();

                                    let popover = gtk4::Popover::builder()
                                        .halign(gtk4::Align::End)
                                        .build();
                                    popover.set_position(gtk4::PositionType::Bottom);
                                    let popover_box = gtk4::Box::builder()
                                        .orientation(gtk4::Orientation::Vertical)
                                        .spacing(0)
                                        .build();

                                    let list_box = gtk4::ListBox::builder()
                                        .selection_mode(gtk4::SelectionMode::None)
                                        .css_classes(["boxed-list"])
                                        .build();

                                    for attachment in parsed.attachments {
                                        let row = build_attachment_row(attachment);
                                        list_box.append(&row);
                                    }

                                    if count > 5 {
                                        let scrolled = gtk4::ScrolledWindow::builder()
                                            .max_content_height(300)
                                            .propagate_natural_height(true)
                                            .build();
                                        scrolled.set_child(Some(&list_box));
                                        popover_box.append(&scrolled);
                                    } else {
                                        popover_box.append(&list_box);
                                    }

                                    popover.set_child(Some(&popover_box));
                                    menu_btn.set_popover(Some(&popover));
                                    attachment_box_ref.append(&menu_btn);
                                }
                            }
                            Err(e) => {
                                debug!("Failed to fetch body: {}", e);
                                let error_box = gtk4::Box::builder()
                                    .orientation(gtk4::Orientation::Vertical)
                                    .valign(gtk4::Align::Center)
                                    .halign(gtk4::Align::Center)
                                    .vexpand(true)
                                    .spacing(8)
                                    .build();

                                let icon = gtk4::Image::builder()
                                    .icon_name("dialog-error-symbolic")
                                    .pixel_size(48)
                                    .css_classes(["dim-label"])
                                    .build();

                                let label = gtk4::Label::builder()
                                    .label("Failed to load message body")
                                    .css_classes(["dim-label"])
                                    .build();

                                error_box.append(&icon);
                                error_box.append(&label);
                                body_box_ref.append(&error_box);
                            }
                        }
                    });
                }
            }
        }
    }

    fn setup_actions(&self) {
        // Compose action
        let compose_action = gio::ActionEntry::builder("compose")
            .activate(|win: &Self, _, _| {
                win.show_compose_dialog();
            })
            .build();

        // Refresh action
        let refresh_action = gio::ActionEntry::builder("refresh")
            .activate(|win: &Self, _, _| {
                win.refresh_messages();
            })
            .build();

        // Search action
        let search_action = gio::ActionEntry::builder("search")
            .activate(|win: &Self, _, _| {
                win.toggle_search();
            })
            .build();

        self.add_action_entries([compose_action, refresh_action, search_action]);

        // Compose-to action (with email parameter)
        let compose_to_action = gio::SimpleAction::new("compose-to", Some(glib::VariantTy::STRING));
        compose_to_action.connect_activate(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, param| {
                if let Some(email) = param.and_then(|p| p.str()) {
                    win.show_compose_dialog_to(email);
                }
            }
        ));
        self.add_action(&compose_to_action);
    }

    fn setup_bindings(&self) {
        // Sidebar toggle is now handled directly in setup_widgets via connect_toggled
    }

    fn show_welcome_state(&self) {
        let imp = self.imp();

        // Create welcome status page
        let welcome = adw::StatusPage::builder()
            .icon_name("mail-send-receive-symbolic")
            .title("Welcome to NorthMail")
            .description("Add an email account to get started")
            .build();

        let add_button = gtk4::Button::builder()
            .label("Add Account")
            .halign(gtk4::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .action_name("app.add-account")
            .build();

        welcome.set_child(Some(&add_button));

        // Replace message view content with welcome
        while let Some(child) = imp.message_view_box.first_child() as Option<gtk4::Widget> {
            imp.message_view_box.remove(&child);
        }
        imp.message_view_box.append(&welcome);
    }

    fn show_compose_dialog(&self) {
        self.show_compose_dialog_with_mode(ComposeMode::New { to: None });
    }

    fn show_compose_dialog_to(&self, email: &str) {
        self.show_compose_dialog_with_mode(ComposeMode::New {
            to: Some((email.to_string(), email.to_string()))
        });
    }

    fn show_compose_dialog_with_mode(&self, mode: ComposeMode) {
        debug!("Opening compose window with mode");

        // Compose-specific CSS
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string(
            "
            .compose-fields { background: @view_bg_color; }
            .compose-entry { background: transparent; border: none; outline: none; box-shadow: none; min-height: 20px; padding: 0; margin: 0; font-size: 0.9em; }
            .compose-entry:focus { background: transparent; border: none; outline: none; box-shadow: none; }
            .compose-entry > text { background: transparent; border: none; outline: none; box-shadow: none; padding: 0; margin: 0; font-size: 0.9em; }
            .compose-chip { background: @accent_bg_color; border-radius: 14px; padding: 0 0 0 8px; margin: 0; min-height: 0; }
            .compose-chip label { font-size: 0.9em; margin: 0; padding: 2px 0; color: @accent_fg_color; }
            .chip-close { min-width: 16px; min-height: 16px; padding: 0; margin: 0 2px 0 4px; -gtk-icon-size: 12px; }
            .chip-close image { color: white; -gtk-icon-style: symbolic; }
            .chip-close:hover { background: alpha(white, 0.2); border-radius: 4px; }
            .compose-field-label { font-size: 0.9em; min-width: 52px; color: alpha(@view_fg_color, 0.55); }
            .compose-separator { background: alpha(@view_fg_color, 0.15); min-height: 1px; }
            .compose-body { background: @view_bg_color; }
            .attachment-pill { background: alpha(currentColor, 0.1); border-radius: 14px; padding: 1px 4px 1px 8px; }
            .attachment-pill:hover { background: alpha(currentColor, 0.15); }
            .attachment-pill label { font-size: 0.8em; }
            .attachment-pill button { min-width: 16px; min-height: 16px; padding: 0; margin: 0 0 0 2px; }
            .more-badge { background: alpha(@accent_color, 0.15); color: @accent_color; border-radius: 6px; padding: 1px 8px; font-size: 0.8em; font-weight: 500; }
            .more-badge:hover { background: alpha(@accent_color, 0.25); }
            .warning { color: @warning_color; }
            .compose-send { min-height: 24px; padding-top: 2px; padding-bottom: 2px; }
            .format-bar { background-color: white; }
            .format-bar button { min-height: 18px; min-width: 18px; padding: 1px; }
            .format-bar button image { -gtk-icon-size: 14px; }
            .format-bar dropdown { min-height: 18px; }
            .format-bar dropdown button { min-height: 18px; padding: 1px 4px; font-size: 0.85em; }
            .format-bar .linked button:checked { background: @accent_bg_color; color: @accent_fg_color; }
            ",
        );
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let compose_window = adw::Window::builder()
            .title("New Message")
            .default_width(640)
            .default_height(560)
            .build();

        let toolbar_view = adw::ToolbarView::new();

        // Toast overlay for in-window notifications (created early for closure capture)
        let toast_overlay = adw::ToastOverlay::new();

        // Header bar — From dropdown on left, Send on right
        let header = adw::HeaderBar::new();

        let send_button = gtk4::Button::builder()
            .label("Send")
            .css_classes(["suggested-action", "pill", "compose-send"])
            .build();

        header.pack_end(&send_button);
        toolbar_view.add_top_bar(&header);

        // Main content
        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .css_classes(["view"])
            .build();

        // --- From dropdown in header ---
        // Track which accounts can send (Microsoft consumer OAuth2 accounts cannot)
        let mut sendable_accounts: Vec<bool> = Vec::new();
        let from_model = gtk4::StringList::new(&[]);
        if let Some(app) = self.application() {
            if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                let accs = app.imp().accounts.borrow();
                for acc in accs.iter() {
                    let is_microsoft_oauth2 = (acc.provider_type == "windows_live" || acc.provider_type == "microsoft")
                        && acc.auth_type == northmail_auth::GoaAuthType::OAuth2;
                    sendable_accounts.push(!is_microsoft_oauth2);
                    from_model.append(&acc.email);
                }
            }
        }
        let sendable_accounts = std::rc::Rc::new(sendable_accounts);

        let from_dropdown = gtk4::DropDown::builder()
            .model(&from_model)
            .css_classes(["flat"])
            .build();

        // Warning icon button (hidden by default, shown for non-sendable accounts)
        let warning_button = gtk4::Button::builder()
            .icon_name("dialog-warning-symbolic")
            .css_classes(["flat", "circular", "warning"])
            .tooltip_text("This account cannot send emails")
            .visible(false)
            .build();

        // Add from dropdown and warning to header
        header.pack_start(&from_dropdown);
        header.pack_start(&warning_button);

        // --- Header fields (To, Cc, Subject) ---
        let fields_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .css_classes(["compose-fields"])
            .build();

        // Label width for alignment
        let label_width = 56;

        // To / Cc / Bcc chip rows
        let to_chips: std::rc::Rc<std::cell::RefCell<Vec<String>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let cc_chips: std::rc::Rc<std::cell::RefCell<Vec<String>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let bcc_chips: std::rc::Rc<std::cell::RefCell<Vec<String>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));

        let all_chips = vec![to_chips.clone(), cc_chips.clone(), bcc_chips.clone()];
        let (to_row, to_add_chip) = Self::build_chip_row("To", to_chips.clone(), all_chips.clone(), self, label_width);
        let (cc_row, cc_add_chip) = Self::build_chip_row("Cc", cc_chips.clone(), all_chips.clone(), self, label_width);
        let (bcc_row, _bcc_add_chip) = Self::build_chip_row("Bcc", bcc_chips.clone(), all_chips.clone(), self, label_width);

        // Bcc row starts hidden
        bcc_row.set_visible(false);
        let bcc_separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        bcc_separator.set_visible(false);

        // Bcc button (shown on Cc row, like attach button on Subject)
        let bcc_button = gtk4::Button::builder()
            .label("Bcc")
            .css_classes(["flat"])
            .tooltip_text("Add Bcc recipients")
            .valign(gtk4::Align::Center)
            .build();

        // Add Bcc button to Cc row
        cc_row.append(&bcc_button);

        let separator1 = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        separator1.add_css_class("compose-separator");
        separator1.set_margin_start(12);
        separator1.set_margin_end(12);

        // Separator between Cc and Bcc (hidden initially, shown with Bcc)
        let separator2 = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        separator2.add_css_class("compose-separator");
        separator2.set_margin_start(12);
        separator2.set_margin_end(12);
        separator2.set_visible(false);

        // Separator before Subject (always visible - between Cc/Bcc and Subject)
        bcc_separator.add_css_class("compose-separator");
        bcc_separator.set_margin_start(12);
        bcc_separator.set_margin_end(12);
        bcc_separator.set_visible(true); // Always visible

        // Wire Bcc button click (after separators created so we can reference separator2)
        {
            let bcc_row_ref = bcc_row.clone();
            let separator2_ref = separator2.clone();
            bcc_button.connect_clicked(move |btn| {
                btn.set_visible(false);
                bcc_row_ref.set_visible(true);
                separator2_ref.set_visible(true);
            });
        }

        fields_box.append(&to_row);
        fields_box.append(&separator1);
        fields_box.append(&cc_row);
        fields_box.append(&separator2);
        fields_box.append(&bcc_row);
        fields_box.append(&bcc_separator);

        // Subject
        let subject_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .margin_start(12)
            .margin_end(12)
            .margin_top(4)
            .margin_bottom(4)
            .build();

        let subject_label = gtk4::Label::builder()
            .label("Subject")
            .xalign(1.0)
            .width_request(label_width)
            .css_classes(["dim-label", "compose-field-label"])
            .build();

        let subject_entry = gtk4::Entry::builder()
            .hexpand(true)
            .has_frame(false)
            .placeholder_text("Subject")
            .css_classes(["compose-entry"])
            .build();

        // Attachment button (next to subject)
        let attach_button = gtk4::Button::builder()
            .icon_name("mail-attachment-symbolic")
            .tooltip_text("Attach file")
            .css_classes(["flat", "circular"])
            .build();

        subject_box.append(&subject_label);
        subject_box.append(&subject_entry);
        subject_box.append(&attach_button);
        fields_box.append(&subject_box);

        content.append(&fields_box);

        // Attachments storage (UI added at bottom after body)
        let attachments: std::rc::Rc<std::cell::RefCell<Vec<(String, String, Vec<u8>, Option<std::path::PathBuf>)>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));

        // Body text editor
        let text_view = gtk4::TextView::builder()
            .vexpand(true)
            .hexpand(true)
            .wrap_mode(gtk4::WrapMode::Word)
            .left_margin(20)
            .right_margin(20)
            .top_margin(12)
            .bottom_margin(12)
            .build();

        // Create text tags for formatting
        let buffer = text_view.buffer();
        let tag_table = buffer.tag_table();

        // Basic style tags
        let bold_tag = gtk4::TextTag::builder().name("bold").weight(700).build();
        let italic_tag = gtk4::TextTag::builder().name("italic").style(gtk4::pango::Style::Italic).build();
        let underline_tag = gtk4::TextTag::builder().name("underline").underline(gtk4::pango::Underline::Single).build();
        let strikethrough_tag = gtk4::TextTag::builder().name("strikethrough").strikethrough(true).build();

        // Alignment tags (for paragraph-level formatting)
        let align_left_tag = gtk4::TextTag::builder().name("align-left").justification(gtk4::Justification::Left).build();
        let align_center_tag = gtk4::TextTag::builder().name("align-center").justification(gtk4::Justification::Center).build();
        let align_right_tag = gtk4::TextTag::builder().name("align-right").justification(gtk4::Justification::Right).build();

        tag_table.add(&bold_tag);
        tag_table.add(&italic_tag);
        tag_table.add(&underline_tag);
        tag_table.add(&strikethrough_tag);
        tag_table.add(&align_left_tag);
        tag_table.add(&align_center_tag);
        tag_table.add(&align_right_tag);

        // Pre-create font family tags
        let font_names = ["Sans", "Serif", "Monospace", "Cantarell", "DejaVu Sans"];
        for font in &font_names {
            let tag = gtk4::TextTag::builder()
                .name(&format!("font-{}", font.to_lowercase().replace(' ', "-")))
                .family(*font)
                .build();
            tag_table.add(&tag);
        }

        // Pre-create font size tags
        let sizes = [10, 11, 12, 14, 16, 18, 20, 24, 28, 32];
        for size in &sizes {
            let tag = gtk4::TextTag::builder()
                .name(&format!("size-{}", size))
                .size_points(*size as f64)
                .build();
            tag_table.add(&tag);
        }

        // Formatting toolbar
        let format_bar = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .margin_start(12)
            .margin_end(12)
            .margin_top(2)
            .margin_bottom(2)
            .css_classes(["format-bar", "compose-fields"])
            .build();

        // Helper to create a button group box
        let create_button_group = || -> gtk4::Box {
            gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(0)
                .css_classes(["linked"])
                .build()
        };

        // Font family dropdown
        let font_families = gtk4::StringList::new(&font_names);
        let font_dropdown = gtk4::DropDown::builder()
            .model(&font_families)
            .tooltip_text("Font Family")
            .build();

        // Font size dropdown
        let size_strings: Vec<String> = sizes.iter().map(|s| s.to_string()).collect();
        let size_strs: Vec<&str> = size_strings.iter().map(|s| s.as_str()).collect();
        let font_sizes = gtk4::StringList::new(&size_strs);
        let size_dropdown = gtk4::DropDown::builder()
            .model(&font_sizes)
            .selected(2) // Default to 12
            .tooltip_text("Font Size")
            .build();

        // Text style group (Bold, Italic, Underline, Strikethrough)
        let style_group = create_button_group();

        let bold_btn = gtk4::ToggleButton::builder()
            .icon_name("format-text-bold-symbolic")
            .tooltip_text("Bold (Ctrl+B)")
            .build();

        let italic_btn = gtk4::ToggleButton::builder()
            .icon_name("format-text-italic-symbolic")
            .tooltip_text("Italic (Ctrl+I)")
            .build();

        let underline_btn = gtk4::ToggleButton::builder()
            .icon_name("format-text-underline-symbolic")
            .tooltip_text("Underline (Ctrl+U)")
            .build();

        let strikethrough_btn = gtk4::ToggleButton::builder()
            .icon_name("format-text-strikethrough-symbolic")
            .tooltip_text("Strikethrough")
            .build();

        style_group.append(&bold_btn);
        style_group.append(&italic_btn);
        style_group.append(&underline_btn);
        style_group.append(&strikethrough_btn);

        // Alignment group
        let align_group = create_button_group();

        let align_left_btn = gtk4::ToggleButton::builder()
            .icon_name("format-justify-left-symbolic")
            .tooltip_text("Align Left")
            .active(true)
            .build();

        let align_center_btn = gtk4::ToggleButton::builder()
            .icon_name("format-justify-center-symbolic")
            .tooltip_text("Center")
            .build();

        let align_right_btn = gtk4::ToggleButton::builder()
            .icon_name("format-justify-right-symbolic")
            .tooltip_text("Align Right")
            .build();

        align_center_btn.set_group(Some(&align_left_btn));
        align_right_btn.set_group(Some(&align_left_btn));

        align_group.append(&align_left_btn);
        align_group.append(&align_center_btn);
        align_group.append(&align_right_btn);

        // List group
        let list_group = create_button_group();

        let bullet_btn = gtk4::ToggleButton::builder()
            .icon_name("view-list-bullet-symbolic")
            .tooltip_text("Bullet List")
            .build();

        let numbered_btn = gtk4::ToggleButton::builder()
            .icon_name("view-list-ordered-symbolic")
            .tooltip_text("Numbered List")
            .build();

        list_group.append(&bullet_btn);
        list_group.append(&numbered_btn);

        format_bar.append(&font_dropdown);
        format_bar.append(&size_dropdown);
        format_bar.append(&style_group);
        format_bar.append(&align_group);
        format_bar.append(&list_group);

        // Helper to toggle tag on selection
        let toggle_tag = |buffer: &gtk4::TextBuffer, tag_name: &str| {
            if let Some((start, end)) = buffer.selection_bounds() {
                let tag_table = buffer.tag_table();
                if let Some(tag) = tag_table.lookup(tag_name) {
                    if start.has_tag(&tag) {
                        buffer.remove_tag(&tag, &start, &end);
                    } else {
                        buffer.apply_tag(&tag, &start, &end);
                    }
                }
            }
        };

        // Helper to apply tag to selection (replacing others of same type)
        let apply_tag_exclusive = |buffer: &gtk4::TextBuffer, tag_name: &str, prefix: &str| {
            if let Some((start, end)) = buffer.selection_bounds() {
                let tag_table = buffer.tag_table();
                // Remove all tags with this prefix
                let mut i = 0;
                loop {
                    if let Some(tag) = tag_table.lookup(&format!("{}-{}", prefix, i)) {
                        buffer.remove_tag(&tag, &start, &end);
                        i += 1;
                    } else {
                        break;
                    }
                }
                // Apply the new tag
                if let Some(tag) = tag_table.lookup(tag_name) {
                    buffer.apply_tag(&tag, &start, &end);
                }
            }
        };

        // Helper to get paragraph bounds for current selection/cursor
        let get_paragraph_bounds = |buffer: &gtk4::TextBuffer| -> Option<(gtk4::TextIter, gtk4::TextIter)> {
            let (start, end) = buffer.selection_bounds().unwrap_or_else(|| {
                let cursor = buffer.iter_at_offset(buffer.cursor_position());
                (cursor.clone(), cursor)
            });
            let mut para_start = start;
            para_start.set_line_offset(0);
            let mut para_end = end;
            if !para_end.ends_line() {
                para_end.forward_to_line_end();
            }
            Some((para_start, para_end))
        };

        // Connect font dropdown
        {
            let buffer = text_view.buffer();
            let font_names = font_names.clone();
            font_dropdown.connect_selected_notify(move |dropdown| {
                let idx = dropdown.selected() as usize;
                if idx < font_names.len() {
                    let font = font_names[idx];
                    let tag_name = format!("font-{}", font.to_lowercase().replace(' ', "-"));
                    if let Some((start, end)) = buffer.selection_bounds() {
                        // Remove other font tags first
                        for f in &font_names {
                            let other_tag = format!("font-{}", f.to_lowercase().replace(' ', "-"));
                            if let Some(tag) = buffer.tag_table().lookup(&other_tag) {
                                buffer.remove_tag(&tag, &start, &end);
                            }
                        }
                        if let Some(tag) = buffer.tag_table().lookup(&tag_name) {
                            buffer.apply_tag(&tag, &start, &end);
                        }
                    }
                }
            });
        }

        // Connect size dropdown
        {
            let buffer = text_view.buffer();
            size_dropdown.connect_selected_notify(move |dropdown| {
                let idx = dropdown.selected() as usize;
                if idx < sizes.len() {
                    let size = sizes[idx];
                    let tag_name = format!("size-{}", size);
                    if let Some((start, end)) = buffer.selection_bounds() {
                        // Remove other size tags first
                        for s in &sizes {
                            let other_tag = format!("size-{}", s);
                            if let Some(tag) = buffer.tag_table().lookup(&other_tag) {
                                buffer.remove_tag(&tag, &start, &end);
                            }
                        }
                        if let Some(tag) = buffer.tag_table().lookup(&tag_name) {
                            buffer.apply_tag(&tag, &start, &end);
                        }
                    }
                }
            });
        }

        // Connect formatting buttons
        {
            let buffer = buffer.clone();
            bold_btn.connect_clicked(move |_| {
                toggle_tag(&buffer, "bold");
            });
        }
        {
            let buffer = text_view.buffer();
            italic_btn.connect_clicked(move |_| {
                toggle_tag(&buffer, "italic");
            });
        }
        {
            let buffer = text_view.buffer();
            underline_btn.connect_clicked(move |_| {
                toggle_tag(&buffer, "underline");
            });
        }
        {
            let buffer = text_view.buffer();
            strikethrough_btn.connect_clicked(move |_| {
                toggle_tag(&buffer, "strikethrough");
            });
        }

        // Connect alignment buttons - apply to paragraph
        {
            let buffer = text_view.buffer();
            align_left_btn.connect_toggled(move |btn| {
                if btn.is_active() {
                    if let Some((start, end)) = get_paragraph_bounds(&buffer) {
                        // Remove other alignment tags
                        if let Some(tag) = buffer.tag_table().lookup("align-center") {
                            buffer.remove_tag(&tag, &start, &end);
                        }
                        if let Some(tag) = buffer.tag_table().lookup("align-right") {
                            buffer.remove_tag(&tag, &start, &end);
                        }
                        if let Some(tag) = buffer.tag_table().lookup("align-left") {
                            buffer.apply_tag(&tag, &start, &end);
                        }
                    }
                }
            });
        }
        {
            let buffer = text_view.buffer();
            align_center_btn.connect_toggled(move |btn| {
                if btn.is_active() {
                    if let Some((start, end)) = get_paragraph_bounds(&buffer) {
                        if let Some(tag) = buffer.tag_table().lookup("align-left") {
                            buffer.remove_tag(&tag, &start, &end);
                        }
                        if let Some(tag) = buffer.tag_table().lookup("align-right") {
                            buffer.remove_tag(&tag, &start, &end);
                        }
                        if let Some(tag) = buffer.tag_table().lookup("align-center") {
                            buffer.apply_tag(&tag, &start, &end);
                        }
                    }
                }
            });
        }
        {
            let buffer = text_view.buffer();
            align_right_btn.connect_toggled(move |btn| {
                if btn.is_active() {
                    if let Some((start, end)) = get_paragraph_bounds(&buffer) {
                        if let Some(tag) = buffer.tag_table().lookup("align-left") {
                            buffer.remove_tag(&tag, &start, &end);
                        }
                        if let Some(tag) = buffer.tag_table().lookup("align-center") {
                            buffer.remove_tag(&tag, &start, &end);
                        }
                        if let Some(tag) = buffer.tag_table().lookup("align-right") {
                            buffer.apply_tag(&tag, &start, &end);
                        }
                    }
                }
            });
        }

        // Connect bullet list button - add/remove "• " at line starts
        {
            let buffer = text_view.buffer();
            bullet_btn.connect_toggled(move |btn| {
                // Only act on user clicks, not programmatic updates
                if !btn.is_sensitive() { return; }

                if let Some((sel_start, sel_end)) = get_paragraph_bounds(&buffer) {
                    let start_line = sel_start.line();
                    let end_line = sel_end.line();

                    for line in start_line..=end_line {
                        let mut line_start = buffer.iter_at_line(line).unwrap();
                        let line_text = line_start.slice(&{
                            let mut end = line_start.clone();
                            end.forward_to_line_end();
                            end
                        });
                        let line_str = line_text.to_string();

                        if line_str.starts_with("• ") {
                            // Remove bullet
                            let mut bullet_end = line_start.clone();
                            bullet_end.forward_chars(2);
                            buffer.delete(&mut line_start, &mut bullet_end);
                        } else if !line_str.trim().is_empty() {
                            // First remove any existing number prefix
                            if line_str.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                                if let Some(pos) = line_str.find(". ") {
                                    let mut num_end = line_start.clone();
                                    num_end.forward_chars((pos + 2) as i32);
                                    buffer.delete(&mut line_start, &mut num_end);
                                }
                            }
                            // Add bullet
                            let mut line_start = buffer.iter_at_line(line).unwrap();
                            buffer.insert(&mut line_start, "• ");
                        }
                    }
                }
            });
        }

        // Connect numbered list button - add/remove "1. " etc at line starts
        {
            let buffer = text_view.buffer();
            numbered_btn.connect_toggled(move |btn| {
                // Only act on user clicks, not programmatic updates
                if !btn.is_sensitive() { return; }

                if let Some((sel_start, sel_end)) = get_paragraph_bounds(&buffer) {
                    let start_line = sel_start.line();
                    let end_line = sel_end.line();

                    // Check if first line has a number
                    let first_iter = buffer.iter_at_line(start_line).unwrap();
                    let first_text = first_iter.slice(&{
                        let mut end = first_iter.clone();
                        end.forward_to_line_end();
                        end
                    });
                    let first_str = first_text.to_string();
                    let is_numbered = first_str.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
                        && first_str.contains(". ");

                    let mut num = 1;
                    for line in start_line..=end_line {
                        let mut line_start = buffer.iter_at_line(line).unwrap();
                        let line_text = line_start.slice(&{
                            let mut end = line_start.clone();
                            end.forward_to_line_end();
                            end
                        });
                        let line_str = line_text.to_string();

                        if is_numbered {
                            // Remove number - find ". " and delete up to it
                            if let Some(pos) = line_str.find(". ") {
                                let mut num_end = line_start.clone();
                                num_end.forward_chars((pos + 2) as i32);
                                buffer.delete(&mut line_start, &mut num_end);
                            }
                        } else if !line_str.trim().is_empty() {
                            // First remove any existing bullet prefix
                            if line_str.starts_with("• ") {
                                let mut bullet_end = line_start.clone();
                                bullet_end.forward_chars(2);
                                buffer.delete(&mut line_start, &mut bullet_end);
                            }
                            // Add number
                            let mut line_start = buffer.iter_at_line(line).unwrap();
                            buffer.insert(&mut line_start, &format!("{}. ", num));
                            num += 1;
                        }
                    }
                }
            });
        }

        // Track cursor position to update button states
        // Create shared update function
        let update_list_buttons = {
            let bullet_btn = bullet_btn.clone();
            let numbered_btn = numbered_btn.clone();
            Rc::new(move |buf: &gtk4::TextBuffer| {
                let iter = buf.iter_at_offset(buf.cursor_position());

                // Check current line for bullet/numbered list
                let mut line_start = iter.clone();
                line_start.set_line_offset(0);
                let mut line_end = line_start.clone();
                line_end.forward_to_line_end();
                let line_text = line_start.slice(&line_end);

                let is_bullet = line_text.starts_with("• ");
                let is_numbered = line_text.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
                    && line_text.contains(". ");

                // Temporarily block signal by making insensitive during programmatic update
                bullet_btn.set_sensitive(false);
                bullet_btn.set_active(is_bullet);
                bullet_btn.set_sensitive(true);

                numbered_btn.set_sensitive(false);
                numbered_btn.set_active(is_numbered);
                numbered_btn.set_sensitive(true);
            })
        };

        {
            let bold_btn = bold_btn.clone();
            let italic_btn = italic_btn.clone();
            let underline_btn = underline_btn.clone();
            let strikethrough_btn = strikethrough_btn.clone();
            let update_list_buttons = update_list_buttons.clone();
            let buffer = text_view.buffer();

            buffer.connect_cursor_position_notify(move |buf| {
                let iter = buf.iter_at_offset(buf.cursor_position());
                let tag_table = buf.tag_table();

                if let Some(tag) = tag_table.lookup("bold") {
                    bold_btn.set_active(iter.has_tag(&tag));
                }
                if let Some(tag) = tag_table.lookup("italic") {
                    italic_btn.set_active(iter.has_tag(&tag));
                }
                if let Some(tag) = tag_table.lookup("underline") {
                    underline_btn.set_active(iter.has_tag(&tag));
                }
                if let Some(tag) = tag_table.lookup("strikethrough") {
                    strikethrough_btn.set_active(iter.has_tag(&tag));
                }

                update_list_buttons(buf);
            });
        }

        // Also update on buffer changes (for when bullet/number is added/removed)
        {
            let update_list_buttons = update_list_buttons.clone();
            let buffer = text_view.buffer();
            buffer.connect_changed(move |buf| {
                update_list_buttons(buf);
            });
        }

        content.append(&format_bar);

        let text_scrolled = gtk4::ScrolledWindow::builder()
            .child(&text_view)
            .vexpand(true)
            .css_classes(["compose-body"])
            .build();

        // Attachments bar - compact horizontal row at bottom
        let attachments_bar = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();

        let attachments_scroll = gtk4::ScrolledWindow::builder()
            .child(&attachments_bar)
            .hscrollbar_policy(gtk4::PolicyType::Automatic)
            .vscrollbar_policy(gtk4::PolicyType::Never)
            .propagate_natural_width(true)
            .build();

        let attachments_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .build();
        attachments_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
        attachments_box.append(&attachments_scroll);
        attachments_box.set_visible(false);

        content.append(&text_scrolled);
        content.append(&attachments_box);

        // Pre-fill fields based on compose mode
        match &mode {
            ComposeMode::New { to } => {
                if let Some((email, display)) = to {
                    to_add_chip(display, email);
                }
            }
            ComposeMode::Reply { to, to_display, subject, quoted_body, .. } => {
                to_add_chip(to_display, to);
                subject_entry.set_text(subject);
                text_view.buffer().set_text(quoted_body);
            }
            ComposeMode::ReplyAll { to, cc, subject, quoted_body, .. } => {
                for (email, display) in to {
                    to_add_chip(display, email);
                }
                for (email, display) in cc {
                    cc_add_chip(display, email);
                }
                subject_entry.set_text(subject);
                text_view.buffer().set_text(quoted_body);
            }
            ComposeMode::Forward { subject, quoted_body, attachments: fwd_attachments } => {
                subject_entry.set_text(subject);
                text_view.buffer().set_text(quoted_body);
                for (filename, mime_type, data) in fwd_attachments {
                    attachments.borrow_mut().push((
                        filename.clone(),
                        mime_type.clone(),
                        data.clone(),
                        None,
                    ));
                }
            }
            ComposeMode::EditDraft { to, cc, subject, body, .. } => {
                for email in to {
                    to_add_chip(email, email);
                }
                for email in cc {
                    cc_add_chip(email, email);
                }
                subject_entry.set_text(subject);
                text_view.buffer().set_text(body);
            }
        }

        toolbar_view.set_content(Some(&content));

        // Set up toast overlay with toolbar content
        toast_overlay.set_child(Some(&toolbar_view));
        compose_window.set_content(Some(&toast_overlay));

        // --- Attachment UI rebuild function ---
        let rebuild_attachments_ui: Rc<dyn Fn()> = {
            let attachments = attachments.clone();
            let attachments_bar = attachments_bar.clone();
            let attachments_box = attachments_box.clone();

            Rc::new(move || {
                // Clear bar
                while let Some(child) = attachments_bar.first_child() {
                    attachments_bar.remove(&child);
                }

                let atts = attachments.borrow();

                if atts.is_empty() {
                    attachments_box.set_visible(false);
                    return;
                }

                attachments_box.set_visible(true);

                // Create compact pill for each attachment
                for (filename, _mime, data, temp_path) in atts.iter() {
                    let pill = gtk4::Box::builder()
                        .orientation(gtk4::Orientation::Horizontal)
                        .spacing(2)
                        .css_classes(["attachment-pill"])
                        .build();

                    // Use content-type icon based on filename (like Files app)
                    // content_type_guess derives the type from filename extension
                    let (content_type, _uncertain) = gtk4::gio::content_type_guess(Some(filename), &[]);
                    let gicon = gtk4::gio::content_type_get_icon(&content_type);
                    let icon = gtk4::Image::builder()
                        .gicon(&gicon)
                        .icon_size(gtk4::IconSize::Normal) // Use normal size for colored icons
                        .build();

                    // Truncate filename for display, keep extension visible
                    let display_name = if filename.len() > 25 {
                        let ext_pos = filename.rfind('.').unwrap_or(filename.len());
                        let ext = &filename[ext_pos..];
                        let name_part = &filename[..ext_pos];
                        if name_part.len() > 20 {
                            format!("{}...{}", &name_part[..17], ext)
                        } else {
                            filename.clone()
                        }
                    } else {
                        filename.clone()
                    };
                    let label = gtk4::Label::new(Some(&display_name));

                    let remove_btn = gtk4::Button::builder()
                        .icon_name("window-close-symbolic")
                        .css_classes(["flat", "circular"])
                        .tooltip_text("Remove attachment")
                        .build();

                    pill.append(&icon);
                    pill.append(&label);
                    pill.append(&remove_btn);

                    // Double-click to open file
                    let gesture = gtk4::GestureClick::new();
                    gesture.set_button(1); // Left click only
                    let filename_for_open = filename.clone();
                    let data_for_open = data.clone();
                    let temp_path_for_open = temp_path.clone();
                    gesture.connect_released(move |gesture, n_press, _, _| {
                        if n_press == 2 {
                            // Double-click
                            if let Some(ref path) = temp_path_for_open {
                                // File already exists on disk
                                let _ = std::process::Command::new("xdg-open").arg(path).spawn();
                            } else {
                                // Forwarded attachment - write to temp file first
                                let temp_dir = std::env::temp_dir();
                                let temp_path = temp_dir.join(&filename_for_open);
                                if std::fs::write(&temp_path, &data_for_open).is_ok() {
                                    let _ = std::process::Command::new("xdg-open").arg(&temp_path).spawn();
                                }
                            }
                        }
                    });
                    pill.add_controller(gesture);

                    // Remove button
                    let filename_to_remove = filename.clone();
                    let attachments_for_remove = attachments.clone();
                    let attachments_bar_for_rebuild = attachments_bar.clone();
                    let attachments_box_for_rebuild = attachments_box.clone();

                    remove_btn.connect_clicked(move |_| {
                        {
                            let mut atts = attachments_for_remove.borrow_mut();
                            if let Some(pos) = atts.iter().position(|(f, _, _, _)| f == &filename_to_remove) {
                                atts.remove(pos);
                            }
                        }
                        // Clear and check if empty
                        while let Some(child) = attachments_bar_for_rebuild.first_child() {
                            attachments_bar_for_rebuild.remove(&child);
                        }
                        if attachments_for_remove.borrow().is_empty() {
                            attachments_box_for_rebuild.set_visible(false);
                        }
                    });

                    attachments_bar.append(&pill);
                }
            })
        };

        // If we have pre-loaded attachments (from forwarding), show them now
        if !attachments.borrow().is_empty() {
            rebuild_attachments_ui();
        }

        // --- Attachment add handler ---
        let add_attachment_to_ui = {
            let attachments = attachments.clone();
            let rebuild = rebuild_attachments_ui.clone();

            move |filename: String, mime_type: String, data: Vec<u8>, temp_path: Option<std::path::PathBuf>| {
                attachments.borrow_mut().push((filename, mime_type, data, temp_path));
                rebuild();
            }
        };

        // Attach button click handler
        {
            let compose_win = compose_window.clone();
            let add_attachment = add_attachment_to_ui.clone();
            attach_button.connect_clicked(move |_| {
                let dialog = gtk4::FileDialog::builder()
                    .title("Attach File")
                    .modal(true)
                    .build();

                let add_att = add_attachment.clone();
                dialog.open(Some(&compose_win), None::<&gtk4::gio::Cancellable>, move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            if let Ok(data) = std::fs::read(&path) {
                                let filename = path.file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| "attachment".to_string());

                                // Guess MIME type from extension
                                let mime_type = Self::guess_mime_type(&filename);

                                add_att(filename, mime_type, data, Some(path));
                            }
                        }
                    }
                });
            });
        }

        // Drag-and-drop support - add directly to TextView to intercept before its built-in handler
        // Use FileList to support multiple files at once
        let drop_target = gtk4::DropTarget::new(gtk4::gdk::FileList::static_type(), gtk4::gdk::DragAction::COPY);
        {
            let add_attachment = add_attachment_to_ui.clone();
            drop_target.connect_drop(move |_, value, _, _| {
                if let Ok(file_list) = value.get::<gtk4::gdk::FileList>() {
                    let mut added_any = false;
                    for file in file_list.files() {
                        if let Some(path) = file.path() {
                            if let Ok(data) = std::fs::read(&path) {
                                let filename = path.file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| "attachment".to_string());

                                let mime_type = Self::guess_mime_type(&filename);
                                add_attachment(filename, mime_type, data, Some(path));
                                added_any = true;
                            }
                        }
                    }
                    return added_any;
                }
                false
            });
        }
        text_view.add_controller(drop_target);

        // Also add drop target on the header fields area
        let drop_target2 = gtk4::DropTarget::new(gtk4::gdk::FileList::static_type(), gtk4::gdk::DragAction::COPY);
        {
            let add_attachment = add_attachment_to_ui.clone();
            drop_target2.connect_drop(move |_, value, _, _| {
                if let Ok(file_list) = value.get::<gtk4::gdk::FileList>() {
                    let mut added_any = false;
                    for file in file_list.files() {
                        if let Some(path) = file.path() {
                            if let Ok(data) = std::fs::read(&path) {
                                let filename = path.file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| "attachment".to_string());

                                let mime_type = Self::guess_mime_type(&filename);
                                add_attachment(filename, mime_type, data, Some(path));
                                added_any = true;
                            }
                        }
                    }
                    return added_any;
                }
                false
            });
        }
        fields_box.add_controller(drop_target2);

        // --- Draft auto-save state ---
        // Track the saved draft: (account_index, uid)
        // If editing an existing draft, initialize with its info so we update it instead of creating new
        let initial_draft_state = match &mode {
            ComposeMode::EditDraft { draft_uid, account_index, .. } => Some((*account_index, *draft_uid)),
            _ => None,
        };
        let draft_state: std::rc::Rc<std::cell::RefCell<Option<(u32, u32)>>> =
            std::rc::Rc::new(std::cell::RefCell::new(initial_draft_state));
        // Generation counter for auto-save timer (avoid SourceId::remove panic)
        let timer_generation: std::rc::Rc<std::cell::Cell<u32>> =
            std::rc::Rc::new(std::cell::Cell::new(0));
        // Whether the message was sent (skip close confirmation)
        let was_sent: std::rc::Rc<std::cell::Cell<bool>> =
            std::rc::Rc::new(std::cell::Cell::new(false));
        // Whether draft save is currently in progress (prevent overlapping saves)
        let save_in_progress: std::rc::Rc<std::cell::Cell<bool>> =
            std::rc::Rc::new(std::cell::Cell::new(false));

        // --- Auto-save helper: resets timer on any edit ---
        let setup_auto_save_timer = {
            let timer_generation = timer_generation.clone();
            let draft_state = draft_state.clone();
            let save_in_progress = save_in_progress.clone();
            let to_chips_save = to_chips.clone();
            let cc_chips_save = cc_chips.clone();
            let subject_entry_save = subject_entry.clone();
            let text_view_save = text_view.clone();
            let from_dropdown_save = from_dropdown.clone();
            let main_window = self.clone();
            let toast_overlay_save = toast_overlay.clone();

            move || {
                eprintln!("[draft] Reset timer called - scheduling 5s auto-save");
                // Increment generation to invalidate any pending timer
                let current_gen = timer_generation.get().wrapping_add(1);
                timer_generation.set(current_gen);

                // Don't schedule if a save is already in progress
                if save_in_progress.get() {
                    eprintln!("[draft] Save in progress, skipping");
                    return;
                }

                let timer_generation_check = timer_generation.clone();
                let draft_state_timer = draft_state.clone();
                let save_in_progress_timer = save_in_progress.clone();
                let to_chips_timer = to_chips_save.clone();
                let cc_chips_timer = cc_chips_save.clone();
                let subject_entry_timer = subject_entry_save.clone();
                let text_view_timer = text_view_save.clone();
                let from_dropdown_timer = from_dropdown_save.clone();
                let main_window_timer = main_window.clone();
                let toast_overlay_timer = toast_overlay_save.clone();

                glib::timeout_add_seconds_local_once(5, move || {
                    // Check if this timer is still valid (not superseded)
                    if timer_generation_check.get() != current_gen {
                        eprintln!("[draft] Timer generation mismatch, ignoring");
                        return;
                    }
                    eprintln!("[draft] Auto-save timer fired");
                    let subject = subject_entry_timer.text().to_string();
                    let body = {
                        let buf = text_view_timer.buffer();
                        let (start, end) = buf.bounds();
                        buf.text(&start, &end, false).to_string()
                    };

                    // Only save if there's content in subject or body
                    if subject.trim().is_empty() && body.trim().is_empty() {
                        eprintln!("[draft] No content, skipping save");
                        return;
                    }
                    eprintln!("[draft] Saving draft: subject='{}' body_len={}", subject, body.len());

                    let to_list = to_chips_timer.borrow().clone();
                    let cc_list = cc_chips_timer.borrow().clone();
                    let account_index = from_dropdown_timer.selected();

                    let Some(app) = main_window_timer.application() else { return };
                    let Some(app) = app.downcast_ref::<NorthMailApplication>() else { return };

                    // Get account email for From
                    let email = {
                        let accs = app.imp().accounts.borrow();
                        match accs.get(account_index as usize) {
                            Some(a) => a.email.clone(),
                            None => return,
                        }
                    };

                    let real_name = glib::real_name().to_string_lossy().to_string();
                    let from_name = if real_name.is_empty() || real_name == "Unknown" {
                        None
                    } else {
                        Some(real_name)
                    };

                    let mut msg = northmail_smtp::OutgoingMessage::new(&email, &subject);
                    if let Some(name) = from_name {
                        msg = msg.from_name(name);
                    }
                    // For drafts without recipients, add sender as placeholder
                    // (lettre requires at least one recipient)
                    if to_list.is_empty() && cc_list.is_empty() {
                        msg = msg.to(&email);
                    } else {
                        for addr in &to_list {
                            msg = msg.to(addr);
                        }
                        for addr in &cc_list {
                            msg = msg.cc(addr);
                        }
                    }
                    msg = msg.text(&body);

                    save_in_progress_timer.set(true);

                    // Delete old draft first (if any), then save new one
                    let old_state = *draft_state_timer.borrow();
                    let draft_state_cb = draft_state_timer.clone();
                    let save_in_progress_cb = save_in_progress_timer.clone();

                    if let Some((old_acct, old_uid)) = old_state {
                        let app_delete = app.clone();
                        let app_save = app.clone();
                        let app_refresh = app.clone();
                        let toast_cb = toast_overlay_timer.clone();
                        eprintln!("[draft] Deleting old draft uid={} then saving new", old_uid);
                        app_delete.delete_draft(old_acct, old_uid, move |_| {
                            // Ignore delete errors — old draft may already be gone
                            let toast_inner = toast_cb.clone();
                            let app_refresh_inner = app_refresh.clone();
                            eprintln!("[draft] Calling save_draft (after delete) for account {}", account_index);
                            app_save.save_draft(account_index, msg, move |result| {
                                save_in_progress_cb.set(false);
                                match result {
                                    Ok(Some(uid)) => {
                                        eprintln!("[draft] Saved! uid={}", uid);
                                        *draft_state_cb.borrow_mut() = Some((account_index, uid));
                                        toast_inner.add_toast(adw::Toast::new("Draft saved"));
                                        app_refresh_inner.refresh_if_viewing_drafts();
                                    }
                                    Ok(None) => {
                                        eprintln!("[draft] Saved (no uid returned)");
                                        *draft_state_cb.borrow_mut() = None;
                                        toast_inner.add_toast(adw::Toast::new("Draft saved"));
                                        app_refresh_inner.refresh_if_viewing_drafts();
                                    }
                                    Err(e) => {
                                        eprintln!("[draft] Save FAILED: {}", e);
                                        toast_inner.add_toast(adw::Toast::new("Failed to save draft"));
                                    }
                                }
                            });
                        });
                    } else {
                        let app_save = app.clone();
                        let app_refresh = app.clone();
                        let toast_cb = toast_overlay_timer.clone();
                        eprintln!("[draft] Calling save_draft for account {}", account_index);
                        app_save.save_draft(account_index, msg, move |result| {
                            save_in_progress_cb.set(false);
                            match result {
                                Ok(Some(uid)) => {
                                    eprintln!("[draft] Saved! uid={}", uid);
                                    *draft_state_cb.borrow_mut() = Some((account_index, uid));
                                    toast_cb.add_toast(adw::Toast::new("Draft saved"));
                                    app_refresh.refresh_if_viewing_drafts();
                                }
                                Ok(None) => {
                                    eprintln!("[draft] Saved (no uid returned)");
                                    *draft_state_cb.borrow_mut() = None;
                                    toast_cb.add_toast(adw::Toast::new("Draft saved"));
                                    app_refresh.refresh_if_viewing_drafts();
                                }
                                Err(e) => {
                                    eprintln!("[draft] Save FAILED: {}", e);
                                    toast_cb.add_toast(adw::Toast::new("Failed to save draft"));
                                }
                            }
                        });
                    }
                });
            }
        };

        // Wire up change handlers to reset the auto-save timer
        {
            let reset = setup_auto_save_timer.clone();
            subject_entry.connect_changed(move |_| {
                reset();
            });
        }
        {
            let reset = setup_auto_save_timer.clone();
            text_view.buffer().connect_changed(move |_| {
                reset();
            });
        }

        // Handle from_dropdown selection changes - show/hide warning icon, enable/disable send
        {
            let sendable = sendable_accounts.clone();
            let warning_btn = warning_button.clone();
            let send_btn = send_button.clone();

            // Check initial selection
            let initial_idx = from_dropdown.selected() as usize;
            if initial_idx < sendable.len() && !sendable[initial_idx] {
                warning_btn.set_visible(true);
                send_btn.set_sensitive(false);
                send_btn.set_tooltip_text(Some("Cannot send from this account"));
            }

            from_dropdown.connect_selected_notify(move |dropdown| {
                let idx = dropdown.selected() as usize;
                if idx < sendable.len() {
                    if sendable[idx] {
                        warning_btn.set_visible(false);
                        send_btn.set_sensitive(true);
                        send_btn.set_tooltip_text(None);
                    } else {
                        warning_btn.set_visible(true);
                        send_btn.set_sensitive(false);
                        send_btn.set_tooltip_text(Some("Cannot send from this account"));
                    }
                }
            });
        }

        // Warning button click handler - show explanation dialog
        {
            let compose_win = compose_window.clone();
            warning_button.connect_clicked(move |_| {
                let dialog = adw::AlertDialog::builder()
                    .heading("Cannot Send from This Account")
                    .body("Microsoft/Hotmail consumer accounts don't support OAuth2 for sending emails. This is a Microsoft limitation.\n\nTo send from this account, you can:\n1. Go to GNOME Settings → Online Accounts\n2. Remove this account\n3. Re-add it as \"IMAP and SMTP\" with your email and an App Password\n\nYou can generate an App Password in your Microsoft account security settings (requires 2-factor authentication).")
                    .build();
                dialog.add_response("ok", "OK");
                dialog.set_default_response(Some("ok"));
                dialog.present(Some(&compose_win));
            });
        }

        // Send button
        let window_ref = self.clone();
        let compose_win_ref = compose_window.clone();
        let send_btn_ref = send_button.clone();
        let was_sent_send = was_sent.clone();
        let draft_state_send = draft_state.clone();
        let timer_generation_send = timer_generation.clone();
        let attachments_send = attachments.clone();
        let bcc_chips_send = bcc_chips.clone();
        send_button.connect_clicked(move |_| {
            let to_list = to_chips.borrow().clone();
            let cc_list = cc_chips.borrow().clone();
            let bcc_list = bcc_chips_send.borrow().clone();
            let subject = subject_entry.text().to_string();
            let body = {
                let buf = text_view.buffer();
                let (start, end) = buf.bounds();
                buf.text(&start, &end, false).to_string()
            };

            // Collect attachments: (filename, mime_type, data)
            let att_list: Vec<(String, String, Vec<u8>)> = attachments_send
                .borrow()
                .iter()
                .map(|(f, m, d, _)| (f.clone(), m.clone(), d.clone()))
                .collect();

            if to_list.is_empty() {
                if let Some(win) = window_ref.downcast_ref::<NorthMailWindow>() {
                    win.add_toast(adw::Toast::new("Please add at least one recipient"));
                }
                return;
            }

            let account_index = from_dropdown.selected();

            // Invalidate any pending auto-save timer
            timer_generation_send.set(timer_generation_send.get().wrapping_add(1));

            send_btn_ref.set_sensitive(false);
            send_btn_ref.set_label("Sending…");

            if let Some(app) = window_ref.application() {
                if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                    let compose_win_close = compose_win_ref.clone();
                    let window_for_toast = window_ref.clone();
                    let send_btn_restore = send_btn_ref.clone();
                    let was_sent_cb = was_sent_send.clone();
                    let draft_state_cb = draft_state_send.clone();
                    let app_for_delete = app.clone();
                    app.send_message(
                        account_index,
                        to_list,
                        cc_list,
                        bcc_list,
                        subject,
                        body,
                        att_list,
                        move |result| {
                            match result {
                                Ok(()) => {
                                    if let Some(win) = window_for_toast.downcast_ref::<NorthMailWindow>() {
                                        win.add_toast(adw::Toast::new("Message sent"));
                                    }
                                    was_sent_cb.set(true);

                                    // Delete draft if one was saved
                                    if let Some((acct_idx, uid)) = *draft_state_cb.borrow() {
                                        app_for_delete.delete_draft(acct_idx, uid, |_| {});
                                    }

                                    compose_win_close.close();
                                }
                                Err(e) => {
                                    if let Some(win) = window_for_toast.downcast_ref::<NorthMailWindow>() {
                                        win.add_toast(adw::Toast::new(&format!("Send failed: {}", e)));
                                    }
                                    send_btn_restore.set_sensitive(true);
                                    send_btn_restore.set_label("Send");
                                }
                            }
                        },
                    );
                }
            }
        });

        // Handle close: ask user whether to keep or delete the draft
        let main_window_close = self.clone();
        let was_sent_close = was_sent;
        let draft_state_close = draft_state;
        let timer_generation_close = timer_generation;
        compose_window.connect_close_request(move |win| {
            // Invalidate any pending auto-save timer
            timer_generation_close.set(timer_generation_close.get().wrapping_add(1));

            // If already sent, just close
            if was_sent_close.get() {
                return glib::Propagation::Proceed;
            }

            // If we have a saved draft, ask the user
            let saved_state = *draft_state_close.borrow();
            if let Some((acct_idx, uid)) = saved_state {
                let dialog = adw::AlertDialog::builder()
                    .heading("Delete draft?")
                    .body("A draft of this message has been saved. Do you want to keep it or delete it?")
                    .build();
                dialog.add_response("keep", "Keep Draft");
                dialog.add_response("delete", "Delete Draft");
                dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                dialog.set_default_response(Some("keep"));
                dialog.set_close_response("keep");

                let win_ref = win.clone();
                let main_window_ref = main_window_close.clone();
                dialog.connect_response(None, move |_dlg, response| {
                    if response == "delete" {
                        if let Some(app) = main_window_ref.application() {
                            if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                                app.delete_draft(acct_idx, uid, |_| {});
                            }
                        }
                    }
                    win_ref.destroy();
                });
                dialog.present(Some(win));
                return glib::Propagation::Stop;
            }

            glib::Propagation::Proceed
        });

        compose_window.present();
    }

    /// Guess MIME type from filename extension
    fn guess_mime_type(filename: &str) -> String {
        let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
        match ext.as_str() {
            "pdf" => "application/pdf",
            "doc" => "application/msword",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "xls" => "application/vnd.ms-excel",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "ppt" => "application/vnd.ms-powerpoint",
            "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "txt" => "text/plain",
            "html" | "htm" => "text/html",
            "css" => "text/css",
            "js" => "application/javascript",
            "json" => "application/json",
            "xml" => "application/xml",
            "zip" => "application/zip",
            "gz" | "gzip" => "application/gzip",
            "tar" => "application/x-tar",
            "rar" => "application/vnd.rar",
            "7z" => "application/x-7z-compressed",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "ico" => "image/x-icon",
            "bmp" => "image/bmp",
            "tiff" | "tif" => "image/tiff",
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            "ogg" => "audio/ogg",
            "flac" => "audio/flac",
            "m4a" => "audio/mp4",
            "mp4" => "video/mp4",
            "webm" => "video/webm",
            "avi" => "video/x-msvideo",
            "mov" => "video/quicktime",
            "mkv" => "video/x-matroska",
            "eml" => "message/rfc822",
            _ => "application/octet-stream",
        }.to_string()
    }

    /// Build an inline chip-based recipient row (label + wrapping chips + entry)
    fn build_chip_row(
        label_text: &str,
        chips: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
        all_chips: Vec<std::rc::Rc<std::cell::RefCell<Vec<String>>>>,
        window: &NorthMailWindow,
        label_width: i32,
    ) -> (gtk4::Box, Rc<dyn Fn(&str, &str)>) {
        let row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .margin_start(12)
            .margin_end(12)
            .margin_top(4)
            .margin_bottom(4)
            .build();

        let label = gtk4::Label::builder()
            .label(label_text)
            .xalign(1.0)
            .width_request(label_width)
            .valign(gtk4::Align::Center)
            .css_classes(["dim-label", "compose-field-label"])
            .build();

        // Content box holds chips + entry. Entry is outside FlowBox so it can expand.
        let content_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(0) // No spacing - chips have their own margin
            .hexpand(true)
            .build();

        // Box for chips - use regular Box instead of FlowBox for proper sizing
        let chip_flow = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(4)
            .hexpand(false) // Don't expand - only take natural size
            .valign(gtk4::Align::Center)
            .margin_end(4) // Space between chips and entry
            .build();

        // Inline entry (no frame, blends into the row) - OUTSIDE FlowBox so it expands
        let entry = gtk4::Entry::builder()
            .hexpand(true)
            .has_frame(false)
            .placeholder_text("Add recipient")
            .css_classes(["compose-entry"])
            .build();

        // Start hidden - will show when first chip is added
        chip_flow.set_visible(false);

        content_box.append(&chip_flow);
        content_box.append(&entry);

        row.append(&label);
        row.append(&content_box);

        // Autocomplete popover — appears directly below the entry
        let popover = gtk4::Popover::builder()
            .has_arrow(false)
            .autohide(false)
            .position(gtk4::PositionType::Bottom)
            .build();
        popover.add_css_class("menu");

        let suggestion_list = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .build();

        let suggestion_scrolled = gtk4::ScrolledWindow::builder()
            .child(&suggestion_list)
            .max_content_height(240)
            .propagate_natural_height(true)
            .min_content_width(320)
            .build();

        popover.set_child(Some(&suggestion_scrolled));
        popover.set_parent(&entry);

        // --- Add chip helper ---
        let add_chip: Rc<dyn Fn(&str, &str)> = {
            let chip_flow = chip_flow.clone();
            let chips = chips.clone();
            let all_chips = all_chips.clone();
            let entry = entry.clone();
            Rc::new(move |display: &str, email: &str| {
                // Check for duplicates across all recipient lists (To, Cc, Bcc)
                let email_lower = email.to_lowercase();
                for chip_list in &all_chips {
                    if chip_list.borrow().iter().any(|e| e.to_lowercase() == email_lower) {
                        // Already exists, just clear entry and return
                        entry.set_text("");
                        return;
                    }
                }

                // Show just the name in the chip (or email if no name)
                let chip_text = if display.is_empty() || display == email {
                    email.to_string()
                } else {
                    display.to_string()
                };

                chips.borrow_mut().push(email.to_string());

                let chip = gtk4::Box::builder()
                    .orientation(gtk4::Orientation::Horizontal)
                    .spacing(2)
                    .css_classes(["compose-chip"])
                    .tooltip_text(email) // Show email on hover
                    .build();

                let chip_label = gtk4::Label::builder()
                    .label(&chip_text)
                    .ellipsize(gtk4::pango::EllipsizeMode::End)
                    .max_width_chars(24)
                    .build();

                let remove_btn = gtk4::Button::builder()
                    .icon_name("window-close-symbolic")
                    .css_classes(["flat", "chip-close"])
                    .valign(gtk4::Align::Center)
                    .build();

                chip.append(&chip_label);
                chip.append(&remove_btn);

                // Append chip to chip box
                chip_flow.append(&chip);
                chip_flow.set_visible(true); // Show chip box when chips exist

                // Remove handler — remove chip directly from Box
                let chip_box_ref = chip_flow.clone();
                let chips_ref = chips.clone();
                let email_owned = email.to_string();
                let chip_ref = chip.clone();
                remove_btn.connect_clicked(move |_| {
                    chip_box_ref.remove(&chip_ref);
                    chips_ref.borrow_mut().retain(|e| e != &email_owned);
                    // Hide chip box if no more chips
                    if chip_box_ref.first_child().is_none() {
                        chip_box_ref.set_visible(false);
                    }
                });

                entry.set_text("");
                entry.grab_focus();
            })
        };
        let add_chip_return = add_chip.clone();

        // Enter key → add manual entry
        let add_chip_enter = add_chip.clone();
        let popover_enter = popover.clone();
        entry.connect_activate(move |entry| {
            let text = entry.text().trim().to_string();
            if !text.is_empty() {
                add_chip_enter(&text, &text);
                popover_enter.popdown();
            }
        });

        // Suggestion selection
        let add_chip_suggest = add_chip.clone();
        let popover_suggest = popover.clone();
        let entry_suggest = entry.clone();
        suggestion_list.connect_row_activated(move |_list, row| {
            if let Some(tooltip) = row.tooltip_text() {
                let parts: Vec<&str> = tooltip.splitn(2, '\t').collect();
                if parts.len() == 2 {
                    add_chip_suggest(parts[0], parts[1]);
                } else {
                    add_chip_suggest("", &tooltip);
                }
            }
            popover_suggest.popdown();
            entry_suggest.set_text("");
            entry_suggest.grab_focus();
        });

        // Instant autocomplete — filter preloaded contacts on every keystroke
        let window_clone = window.clone();
        let popover_change = popover.clone();
        let suggestion_list_ref = suggestion_list.clone();
        let suggestion_list_key = suggestion_list; // For key handler below
        entry.connect_changed(move |entry| {
            let text = entry.text().to_string();

            if text.trim().is_empty() {
                popover_change.popdown();
                return;
            }

            let Some(app) = window_clone.application() else { return };
            let Some(app) = app.downcast_ref::<NorthMailApplication>() else { return };

            let popover_cb = popover_change.clone();
            let list_cb = suggestion_list_ref.clone();
            let entry_ref = entry.clone();

            app.query_contacts(text, move |results| {
                while let Some(row) = list_cb.row_at_index(0) {
                    list_cb.remove(&row);
                }

                if results.is_empty() {
                    popover_cb.popdown();
                    return;
                }

                for (name, email) in &results {
                    let row_box = gtk4::Box::builder()
                        .orientation(gtk4::Orientation::Vertical)
                        .spacing(1)
                        .margin_start(12)
                        .margin_end(12)
                        .margin_top(6)
                        .margin_bottom(6)
                        .build();

                    if !name.is_empty() && name != "Unknown" {
                        let name_lbl = gtk4::Label::builder()
                            .label(name)
                            .xalign(0.0)
                            .build();
                        row_box.append(&name_lbl);
                    }

                    let email_lbl = gtk4::Label::builder()
                        .label(email)
                        .xalign(0.0)
                        .css_classes(["dim-label", "caption"])
                        .build();
                    row_box.append(&email_lbl);

                    let suggestion_row = gtk4::ListBoxRow::builder()
                        .child(&row_box)
                        .tooltip_text(format!("{}\t{}", name, email))
                        .build();

                    list_cb.append(&suggestion_row);
                }

                // Position popover below the entry
                let h = entry_ref.height();
                popover_cb.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(0, 0, 1, h)));
                popover_cb.set_offset(160, 4);
                popover_cb.popup();
            });
        });

        // Keyboard navigation for suggestions (Down/Up/Enter/Escape)
        // Use CAPTURE phase so we process before entry's default activate
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let popover_key = popover.clone();
        let list_key = suggestion_list_key;
        let add_chip_key = add_chip.clone();
        let entry_key = entry.clone();
        key_controller.connect_key_pressed(move |_, keyval, _, _| {
            use gtk4::gdk::Key;

            if !popover_key.is_visible() {
                return gtk4::glib::Propagation::Proceed;
            }

            match keyval {
                k if k == Key::Down => {
                    // Move selection down (or select first if none selected)
                    let selected = list_key.selected_row();
                    if let Some(row) = selected {
                        let idx = row.index();
                        if let Some(next) = list_key.row_at_index(idx + 1) {
                            list_key.select_row(Some(&next));
                        }
                    } else if let Some(first) = list_key.row_at_index(0) {
                        list_key.select_row(Some(&first));
                    }
                    gtk4::glib::Propagation::Stop
                }
                k if k == Key::Up => {
                    // Move selection up
                    if let Some(row) = list_key.selected_row() {
                        let idx = row.index();
                        if idx > 0 {
                            if let Some(prev) = list_key.row_at_index(idx - 1) {
                                list_key.select_row(Some(&prev));
                            }
                        } else {
                            // At top, deselect and stay in entry
                            list_key.unselect_all();
                        }
                    }
                    gtk4::glib::Propagation::Stop
                }
                k if k == Key::Return || k == Key::KP_Enter => {
                    // If a row is selected, activate it
                    if let Some(row) = list_key.selected_row() {
                        if let Some(tooltip) = row.tooltip_text() {
                            let parts: Vec<&str> = tooltip.splitn(2, '\t').collect();
                            if parts.len() == 2 {
                                add_chip_key(parts[0], parts[1]);
                            } else {
                                add_chip_key("", &tooltip);
                            }
                        }
                        popover_key.popdown();
                        entry_key.set_text("");
                        return gtk4::glib::Propagation::Stop;
                    }
                    gtk4::glib::Propagation::Proceed
                }
                k if k == Key::Escape => {
                    // Close popover and deselect
                    list_key.unselect_all();
                    popover_key.popdown();
                    gtk4::glib::Propagation::Stop
                }
                _ => gtk4::glib::Propagation::Proceed,
            }
        });
        entry.add_controller(key_controller);

        (row, add_chip_return)
    }

    fn refresh_messages(&self) {
        debug!("Refreshing messages");
        // TODO: Trigger sync via SyncCommand
        self.add_toast(adw::Toast::new("Refreshing..."));
    }

    fn toggle_search(&self) {
        debug!("Toggling search");
        // TODO: Show/hide search bar
    }

    /// Handle FTS search-requested signal from message list
    fn handle_search_requested(&self, query: &str) {
        let Some(app) = self.application() else { return };
        let Some(app) = app.downcast_ref::<NorthMailApplication>() else { return };

        let folder_id = app.cache_folder_id();
        debug!("Search requested: query='{}', folder_id={}", query, folder_id);
        if folder_id == 0 {
            debug!("Search aborted: folder_id is 0 (not yet set)");
            return;
        }

        let is_unified = folder_id == -1;

        if query.is_empty() {
            // Empty query: reload normal folder messages from cache
            let db = match app.database_ref() {
                Some(db) => db.clone(),
                None => return,
            };
            let app_clone = app.clone();
            glib::spawn_future_local(async move {
                let (sender, receiver) = std::sync::mpsc::channel();
                let fid = folder_id;
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    let result = if fid == -1 {
                        rt.block_on(db.get_inbox_messages(100, 0))
                    } else {
                        rt.block_on(db.get_messages(fid, 100, 0))
                    };
                    let _ = sender.send(result);
                });

                let result = loop {
                    match receiver.try_recv() {
                        Ok(result) => break Some(result),
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            glib::timeout_future(std::time::Duration::from_millis(10)).await;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => break None,
                    }
                };

                if let Some(Ok(messages)) = result {
                    let infos: Vec<crate::widgets::MessageInfo> =
                        messages.iter().map(crate::widgets::MessageInfo::from).collect();
                    app_clone.set_cache_offset(infos.len() as i64);
                    if let Some(window) = app_clone.active_window() {
                        if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                            if let Some(message_list) = win.message_list() {
                                message_list.set_messages(infos);
                            }
                        }
                    }
                }
            });
        } else {
            // Non-empty query: FTS search in current folder (or all inboxes)
            let db = match app.database_ref() {
                Some(db) => db.clone(),
                None => return,
            };
            let query = query.to_string();
            let app_clone = app.clone();
            glib::spawn_future_local(async move {
                let (sender, receiver) = std::sync::mpsc::channel();
                let fid = folder_id;
                let q = query.clone();
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    let result = if fid == -1 {
                        rt.block_on(db.search_inbox_messages(&q, 200))
                    } else {
                        rt.block_on(db.search_messages_in_folder(fid, &q, 200))
                    };
                    let _ = sender.send(result);
                });

                let result = loop {
                    match receiver.try_recv() {
                        Ok(result) => break Some(result),
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            glib::timeout_future(std::time::Duration::from_millis(10)).await;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => break None,
                    }
                };

                if let Some(Ok(messages)) = result {
                    let infos: Vec<crate::widgets::MessageInfo> =
                        messages.iter().map(crate::widgets::MessageInfo::from).collect();
                    debug!("FTS search '{}' returned {} results (unified={})", query, infos.len(), is_unified);
                    if let Some(window) = app_clone.active_window() {
                        if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                            if let Some(message_list) = win.message_list() {
                                // Use set_search_results to skip local search filtering
                                // (FTS already searched body text which isn't in snippet)
                                message_list.set_search_results(infos);
                                message_list.set_can_load_more(false);
                            }
                        }
                    }
                }
            });
        }
    }

    /// Show the main view (message list + message view) instead of welcome
    pub fn show_main_view(&self) {
        let imp = self.imp();

        // Restore the message view widget if it was replaced
        while let Some(child) = imp.message_view_box.first_child() as Option<gtk4::Widget> {
            imp.message_view_box.remove(&child);
        }

        // Note: MessageView widget will be restored when a message is selected

        // Show a "select a message" placeholder in the message view
        let placeholder = adw::StatusPage::builder()
            .icon_name("mail-read-symbolic")
            .title("Select a Message")
            .description("Choose a message from the list to read it")
            .vexpand(true)
            .build();

        imp.message_view_box.append(&placeholder);
    }

    /// Get the folder sidebar widget
    pub fn folder_sidebar(&self) -> Option<&FolderSidebar> {
        self.imp().folder_sidebar.get()
    }

    /// Get the message list widget
    pub fn message_list(&self) -> Option<&MessageList> {
        self.imp().message_list.get()
    }

    /// Get the message view widget
    pub fn message_view(&self) -> Option<&MessageView> {
        self.imp().message_view.get()
    }

    /// Clear the currently displayed message tracking (called when switching folders)
    pub fn clear_current_message(&self) {
        *self.imp().current_message_uid.borrow_mut() = None;
    }

    /// Show loading spinner in the message list area
    pub fn show_loading(&self) {
        self.show_loading_with_status("Connecting...", None);
    }

    /// Show loading with a specific status message
    pub fn show_loading_with_status(&self, status: &str, progress: Option<&str>) {
        let imp = self.imp();

        // Clear message list box and show spinner
        while let Some(child) = imp.message_list_box.first_child() as Option<gtk4::Widget> {
            imp.message_list_box.remove(&child);
        }

        let spinner_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .valign(gtk4::Align::Center)
            .halign(gtk4::Align::Center)
            .vexpand(true)
            .spacing(8)
            .build();

        let spinner = gtk4::Spinner::builder()
            .spinning(true)
            .width_request(32)
            .height_request(32)
            .build();

        let status_label = gtk4::Label::builder()
            .label(status)
            .css_classes(["dim-label"])
            .build();

        let progress_label = gtk4::Label::builder()
            .label(progress.unwrap_or(""))
            .css_classes(["dim-label", "caption"])
            .visible(progress.is_some())
            .build();

        spinner_box.append(&spinner);
        spinner_box.append(&status_label);
        spinner_box.append(&progress_label);
        imp.message_list_box.append(&spinner_box);

        // Store references for updating
        imp.loading_label.replace(Some(status_label));
        imp.loading_progress_label.replace(Some(progress_label));
    }

    /// Update the loading status text
    pub fn update_loading_status(&self, status: &str, progress: Option<&str>) {
        let imp = self.imp();
        if let Some(label) = imp.loading_label.borrow().as_ref() {
            label.set_label(status);
        }
        if let Some(progress_label) = imp.loading_progress_label.borrow().as_ref() {
            if let Some(progress_text) = progress {
                progress_label.set_label(progress_text);
                progress_label.set_visible(true);
            } else {
                progress_label.set_visible(false);
            }
        }
    }

    /// Restore the message list widget after loading
    pub fn restore_message_list(&self) {
        let imp = self.imp();

        // Clear current content
        while let Some(child) = imp.message_list_box.first_child() as Option<gtk4::Widget> {
            imp.message_list_box.remove(&child);
        }

        // Re-add the message list widget
        if let Some(message_list) = imp.message_list.get() {
            imp.message_list_box.append(message_list);
        }
    }

    /// Update the window title to show unread count
    pub fn set_unread_count(&self, count: i64) {
        if count > 0 {
            self.set_title(Some(&format!("NorthMail ({})", count)));
        } else {
            self.set_title(Some("NorthMail"));
        }
    }
}

fn format_file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn icon_for_mime_type(mime_type: &str) -> &'static str {
    if mime_type.starts_with("image/") {
        "image-x-generic-symbolic"
    } else if mime_type.starts_with("audio/") {
        "audio-x-generic-symbolic"
    } else if mime_type.starts_with("video/") {
        "video-x-generic-symbolic"
    } else if mime_type == "application/pdf" {
        "x-office-document-symbolic"
    } else if mime_type.starts_with("text/") {
        "text-x-generic-symbolic"
    } else {
        "document-symbolic"
    }
}

fn build_attachment_row(attachment: ParsedAttachment) -> adw::ActionRow {
    let data = Rc::new(attachment.data);
    let filename = attachment.filename;
    let mime_type = attachment.mime_type;
    let size = data.len();

    let row = adw::ActionRow::builder()
        .title(&filename)
        .subtitle(&format!("{} — {}", mime_type, format_file_size(size)))
        .build();

    // Prefix: mime type icon
    let icon = gtk4::Image::builder()
        .icon_name(icon_for_mime_type(&mime_type))
        .build();
    row.add_prefix(&icon);

    // Suffix: Open button
    let open_btn = gtk4::Button::builder()
        .icon_name("eye-open-negative-filled-symbolic")
        .css_classes(["flat", "circular"])
        .tooltip_text("Preview")
        .valign(gtk4::Align::Center)
        .build();

    let data_open = data.clone();
    let filename_open = filename.clone();
    let open_btn_ref = open_btn.clone();
    open_btn.connect_clicked(move |_| {
        open_attachment(&filename_open, &data_open, &open_btn_ref);
    });
    row.add_suffix(&open_btn);

    // Suffix: Save button
    let save_btn = gtk4::Button::builder()
        .icon_name("document-save-symbolic")
        .css_classes(["flat", "circular"])
        .tooltip_text("Save")
        .valign(gtk4::Align::Center)
        .build();

    let data_save = data.clone();
    let filename_save = filename.clone();
    let save_btn_ref = save_btn.clone();
    save_btn.connect_clicked(move |_| {
        save_attachment(&filename_save, &data_save, &save_btn_ref);
    });
    row.add_suffix(&save_btn);

    row
}

fn open_attachment(filename: &str, data: &Rc<Vec<u8>>, widget: &impl gtk4::prelude::IsA<gtk4::Widget>) {
    let temp_dir = std::env::temp_dir().join("northmail-attachments");
    if std::fs::create_dir_all(&temp_dir).is_err() {
        tracing::warn!("Failed to create temp dir for attachment");
        return;
    }

    let temp_path = temp_dir.join(filename);
    if let Err(e) = std::fs::write(&temp_path, data.as_slice()) {
        tracing::warn!("Failed to write temp attachment: {}", e);
        return;
    }

    let file = gio::File::for_path(&temp_path);
    let launcher = gtk4::FileLauncher::new(Some(&file));
    let window = widget.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
    launcher.launch(window.as_ref(), gio::Cancellable::NONE, |result| {
        if let Err(e) = result {
            tracing::warn!("Failed to open attachment: {}", e);
        }
    });
}

fn save_attachment(filename: &str, data: &Rc<Vec<u8>>, widget: &impl gtk4::prelude::IsA<gtk4::Widget>) {
    let dialog = gtk4::FileDialog::builder()
        .initial_name(filename)
        .build();

    let window = widget.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
    let data = data.clone();
    dialog.save(window.as_ref(), gio::Cancellable::NONE, move |result| {
        match result {
            Ok(file) => {
                if let Some(path) = file.path() {
                    if let Err(e) = std::fs::write(&path, data.as_slice()) {
                        tracing::warn!("Failed to save attachment: {}", e);
                    }
                }
            }
            Err(e) => {
                // User cancelled or error
                if !e.matches(gio::IOErrorEnum::Cancelled) {
                    tracing::warn!("Save dialog error: {}", e);
                }
            }
        }
    });
}

/// Add a contact to Evolution Data Server via D-Bus
async fn add_contact_to_eds(name: &str, email: &str) -> Result<(), String> {
    use zbus::Connection;
    use zbus::zvariant::ObjectPath;

    // Create vCard
    let vcard = format!(
        "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:{}\r\nEMAIL:{}\r\nEND:VCARD",
        name, email
    );

    // Connect to session bus
    let connection = Connection::session()
        .await
        .map_err(|e| format!("Failed to connect to session bus: {}", e))?;

    // Open the system address book
    let reply: (String, String) = connection
        .call_method(
            Some("org.gnome.evolution.dataserver.AddressBook10"),
            "/org/gnome/evolution/dataserver/AddressBookFactory",
            Some("org.gnome.evolution.dataserver.AddressBookFactory"),
            "OpenAddressBook",
            &("system-address-book",),
        )
        .await
        .map_err(|e| format!("Failed to open address book: {}", e))?
        .body()
        .deserialize()
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let (object_path_str, _bus_name) = reply;
    let object_path = ObjectPath::try_from(object_path_str.as_str())
        .map_err(|e| format!("Invalid object path: {}", e))?;

    // Open the book (required before creating contacts)
    let _: Vec<String> = connection
        .call_method(
            Some("org.gnome.evolution.dataserver.AddressBook10"),
            &object_path,
            Some("org.gnome.evolution.dataserver.AddressBook"),
            "Open",
            &(),
        )
        .await
        .map_err(|e| format!("Failed to open book: {}", e))?
        .body()
        .deserialize()
        .map_err(|e| format!("Failed to parse open response: {}", e))?;

    // Create the contact
    let vcards: Vec<&str> = vec![&vcard];
    let opflags: u32 = 0;
    let _: Vec<String> = connection
        .call_method(
            Some("org.gnome.evolution.dataserver.AddressBook10"),
            &object_path,
            Some("org.gnome.evolution.dataserver.AddressBook"),
            "CreateContacts",
            &(vcards, opflags),
        )
        .await
        .map_err(|e| format!("Failed to create contact: {}", e))?
        .body()
        .deserialize()
        .map_err(|e| format!("Failed to parse create response: {}", e))?;

    Ok(())
}
