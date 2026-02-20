//! Message view widget

use gtk4::{gio, glib, prelude::*, subclass::prelude::*};
use libadwaita as adw;
use crate::i18n::tr;
#[cfg(feature = "webkit")]
use webkit6::prelude::*;

#[cfg(feature = "webkit")]
use std::cell::RefCell as StdRefCell;

/// Thread-local storage for HTML content to be served by the `northmail:` URI scheme handler.
/// We set this before calling `load_uri()` and the scheme handler reads it.
#[cfg(feature = "webkit")]
thread_local! {
    static PENDING_HTML: StdRefCell<String> = StdRefCell::new(String::new());
}

/// Ensure the `northmail` and `northmail-link` URI schemes are registered exactly once.
#[cfg(feature = "webkit")]
pub fn ensure_uri_schemes_registered() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        if let Some(ctx) = webkit6::WebContext::default() {
            // Scheme for serving email HTML content
            ctx.register_uri_scheme("northmail", |request| {
                let html = PENDING_HTML.with(|cell| cell.borrow().clone());
                let bytes = glib::Bytes::from(html.as_bytes());
                let stream = gio::MemoryInputStream::from_bytes(&bytes);
                request.finish(&stream, html.len() as i64, Some("text/html; charset=utf-8"));
            });

            // Scheme for intercepting link clicks — extract the real URL and open externally
            ctx.register_uri_scheme("northmail-link", |request| {
                if let Some(uri) = request.uri() {
                    let uri_str: String = uri.into();
                    // The real URL is encoded after "northmail-link://open?"
                    if let Some(encoded) = uri_str.strip_prefix("northmail-link://open?") {
                        let real_url = urlencoding::decode(encoded)
                            .unwrap_or_else(|_| encoded.into())
                            .into_owned();
                        tracing::info!("Opening external link: {}", real_url);
                        if let Err(e) = gtk4::gio::AppInfo::launch_default_for_uri(&real_url, gtk4::gio::AppLaunchContext::NONE) {
                            tracing::warn!("launch_default_for_uri failed: {}, trying xdg-open", e);
                            let _ = std::process::Command::new("xdg-open").arg(&real_url).spawn();
                        }
                    }
                }
                // Return an empty response to prevent navigation
                let bytes = glib::Bytes::from(&[]);
                let stream = gio::MemoryInputStream::from_bytes(&bytes);
                request.finish(&stream, 0, Some("text/plain"));
            });
        }
    });
}

/// Rewrite all `href` attributes in HTML so links go through our `northmail-link://` scheme,
/// which opens them in the user's default browser instead of navigating inline.
#[cfg(feature = "webkit")]
pub fn rewrite_links_for_external_open(html: &str) -> String {
    // Match href="..." or href='...' attributes
    let re = regex::Regex::new(r##"(?i)href\s*=\s*"([^"]*)""##).unwrap();
    let pass1 = re.replace_all(html, |caps: &regex::Captures| {
        let url = &caps[1];
        if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
            let encoded = urlencoding::encode(url);
            format!(r#"href="northmail-link://open?{}""#, encoded)
        } else if url.starts_with('#') || url.is_empty() {
            // Keep anchor links and empty hrefs as-is (they won't navigate away)
            format!(r#"href="{}""#, url)
        } else {
            // For relative URLs or other schemes, just disable the link
            "href=\"#\"".to_string()
        }
    });

    // Also handle single-quoted hrefs
    let re2 = regex::Regex::new(r#"(?i)href\s*=\s*'([^']*)'"#).unwrap();
    re2.replace_all(&pass1, |caps: &regex::Captures| {
        let url = &caps[1];
        if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
            let encoded = urlencoding::encode(url);
            format!(r#"href='northmail-link://open?{}'"#, encoded)
        } else if url.starts_with('#') || url.is_empty() {
            format!(r#"href='{}'"#, url)
        } else {
            r#"href='#'"#.to_string()
        }
    }).into_owned()
}

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct MessageView {
        pub header_card: RefCell<Option<gtk4::Box>>,
        pub content_box: RefCell<Option<gtk4::Box>>,
        pub star_button: RefCell<Option<gtk4::ToggleButton>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageView {
        const NAME: &'static str = "NorthMailMessageView";
        type Type = super::MessageView;
        type ParentType = gtk4::Box;
    }

    impl ObjectImpl for MessageView {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            obj.set_orientation(gtk4::Orientation::Vertical);
            obj.set_vexpand(true);
            obj.set_hexpand(true);

            obj.setup_ui();
        }
    }

    impl WidgetImpl for MessageView {}
    impl BoxImpl for MessageView {}
}

glib::wrapper! {
    pub struct MessageView(ObjectSubclass<imp::MessageView>)
        @extends gtk4::Box, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Orientable;
}

impl MessageView {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn setup_ui(&self) {
        let imp = self.imp();

        // Add CSS for styling
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string(
            r#"
            .message-view-container {
                background-color: @view_bg_color;
            }
            .message-header-card {
                background-color: alpha(@view_bg_color, 0.7);
                border-radius: 12px;
                margin: 12px;
                padding: 0;
            }
            .message-header-card-inner {
                padding: 12px 16px;
            }
            .message-action-bar {
                padding: 8px 12px;
                border-bottom: 1px solid alpha(black, 0.08);
            }
            .message-subject {
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
            .email-address-chip {
                padding: 4px 8px;
                border-radius: 6px;
                background: transparent;
                transition: background 150ms ease;
            }
            .email-address-chip:hover {
                background: alpha(@view_fg_color, 0.08);
            }
            .message-date {
                font-size: 12px;
                color: alpha(@view_fg_color, 0.6);
            }
            .message-recipients {
                font-size: 12px;
                color: alpha(@view_fg_color, 0.7);
            }
            .message-recipients-label {
                font-size: 12px;
                color: alpha(@view_fg_color, 0.5);
                min-width: 24px;
            }
            .avatar-circle {
                border-radius: 50%;
                min-width: 40px;
                min-height: 40px;
            }
            .message-content-area {
                background-color: @view_bg_color;
                padding: 16px;
            }
            "#,
        );
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER,
        );

        // Main container with white background
        self.add_css_class("message-view-container");

        // Scrolled window for entire message view
        let scrolled = gtk4::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .build();

        let main_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .build();

        // Header card (floating rounded box)
        let header_card = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .css_classes(["message-header-card"])
            .build();

        // Action bar at top of card
        let action_bar = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .css_classes(["message-action-bar"])
            .build();

        // Star button on left
        let star_button = gtk4::ToggleButton::builder()
            .icon_name("non-starred-symbolic")
            .tooltip_text(&tr("Star"))
            .css_classes(["flat"])
            .build();

        // Update icon when toggled
        star_button.connect_toggled(|btn| {
            if btn.is_active() {
                btn.set_icon_name("starred-symbolic");
            } else {
                btn.set_icon_name("non-starred-symbolic");
            }
        });

        let action_spacer = gtk4::Box::builder()
            .hexpand(true)
            .build();

        // Action buttons on right
        let reply_button = gtk4::Button::builder()
            .icon_name("mail-reply-sender-symbolic")
            .tooltip_text(&tr("Reply"))
            .css_classes(["flat"])
            .build();

        let reply_all_button = gtk4::Button::builder()
            .icon_name("mail-reply-all-symbolic")
            .tooltip_text(&tr("Reply All"))
            .css_classes(["flat"])
            .build();

        let forward_button = gtk4::Button::builder()
            .icon_name("mail-forward-symbolic")
            .tooltip_text(&tr("Forward"))
            .css_classes(["flat"])
            .build();

        let archive_button = gtk4::Button::builder()
            .icon_name("folder-symbolic")
            .tooltip_text(&tr("Archive"))
            .css_classes(["flat"])
            .build();

        let delete_button = gtk4::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(&tr("Delete"))
            .css_classes(["flat"])
            .build();

        action_bar.append(&star_button);
        action_bar.append(&action_spacer);
        action_bar.append(&reply_button);
        action_bar.append(&reply_all_button);
        action_bar.append(&forward_button);
        action_bar.append(&archive_button);
        action_bar.append(&delete_button);

        header_card.append(&action_bar);

        // Header content (will be populated when message is shown)
        let header_content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(8)
            .css_classes(["message-header-card-inner"])
            .build();

        header_card.append(&header_content);
        main_box.append(&header_card);

        imp.header_card.replace(Some(header_content));
        imp.star_button.replace(Some(star_button));

        // Content area
        let content_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .css_classes(["message-content-area"])
            .vexpand(true)
            .build();

        // Placeholder
        let placeholder = adw::StatusPage::builder()
            .icon_name("mail-read-symbolic")
            .title(&tr("No Message Selected"))
            .description(&tr("Select a message to view its contents"))
            .vexpand(true)
            .build();

        content_box.append(&placeholder);
        main_box.append(&content_box);

        scrolled.set_child(Some(&main_box));
        self.append(&scrolled);

        imp.content_box.replace(Some(content_box));
    }

    /// Generate a color from a string (for avatar background)
    fn string_to_color(s: &str) -> String {
        let colors = [
            "#4A90D9", "#E74C3C", "#2ECC71", "#9B59B6",
            "#F39C12", "#1ABC9C", "#E91E63", "#3F51B5",
            "#00BCD4", "#8BC34A", "#FF5722", "#607D8B",
        ];

        let hash: usize = s.bytes().fold(0, |acc, b| acc.wrapping_add(b as usize));
        colors[hash % colors.len()].to_string()
    }

    /// Get initials from a name or email
    fn get_initials(name: &str, email: &str) -> String {
        // Try to get initials from name first
        let display = if name.is_empty() || name == email {
            email.split('@').next().unwrap_or("?")
        } else {
            name
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

    /// Create an avatar widget
    fn create_avatar(&self, name: &str, email: &str, _contact_photo: Option<&str>) -> gtk4::Widget {
        // TODO: If contact_photo is available, use it instead of initials
        let initials = Self::get_initials(name, email);
        let color = Self::string_to_color(email);

        let avatar_box = gtk4::Box::builder()
            .width_request(40)
            .height_request(40)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .build();

        let drawing_area = gtk4::DrawingArea::builder()
            .width_request(40)
            .height_request(40)
            .build();

        let initials_clone = initials.clone();
        let color_clone = color.clone();
        drawing_area.set_draw_func(move |_, cr, width, height| {
            // Parse color
            let r = u8::from_str_radix(&color_clone[1..3], 16).unwrap_or(74) as f64 / 255.0;
            let g = u8::from_str_radix(&color_clone[3..5], 16).unwrap_or(144) as f64 / 255.0;
            let b = u8::from_str_radix(&color_clone[5..7], 16).unwrap_or(217) as f64 / 255.0;

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

            let extents = cr.text_extents(&initials_clone).unwrap();
            let x = cx - extents.width() / 2.0 - extents.x_bearing();
            let y = cy - extents.height() / 2.0 - extents.y_bearing();

            cr.move_to(x, y);
            let _ = cr.show_text(&initials_clone);
        });

        avatar_box.append(&drawing_area);
        avatar_box.upcast()
    }

    /// Display a message
    pub fn show_message(&self, message: &MessageDetails) {
        let imp = self.imp();

        // Update star button to match message state
        if let Some(star_btn) = imp.star_button.borrow().as_ref() {
            star_btn.set_active(message.is_starred);
        }

        // Update header
        if let Some(header_box) = imp.header_card.borrow().as_ref() {
            // Clear existing content
            while let Some(child) = header_box.first_child() as Option<gtk4::Widget> {
                header_box.remove(&child);
            }

            // Top row: Avatar + Sender info + Date
            let sender_row = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(12)
                .build();

            // Avatar
            let avatar = self.create_avatar(&message.from_name, &message.from_email, None);
            sender_row.append(&avatar);

            // Sender name and email (wrapped in clickable box with context menu)
            let sender_info = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(2)
                .hexpand(true)
                .valign(gtk4::Align::Center)
                .build();

            let sender_name = if message.from_name.is_empty() || message.from_name == message.from_email {
                message.from_email.split('@').next().unwrap_or(&message.from_email).to_string()
            } else {
                message.from_name.clone()
            };

            // Clickable email chip with hover effect
            let email_chip = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(2)
                .css_classes(["email-address-chip"])
                .build();

            let name_label = gtk4::Label::builder()
                .label(&sender_name)
                .xalign(0.0)
                .css_classes(["message-sender-name"])
                .build();

            let email_label = gtk4::Label::builder()
                .label(&format!("<{}>", message.from_email))
                .xalign(0.0)
                .css_classes(["message-sender-email"])
                .build();

            email_chip.append(&name_label);
            email_chip.append(&email_label);

            // Create context menu popover for sender
            let from_email = message.from_email.clone();
            let from_name = sender_name.clone();

            let popover = gtk4::Popover::new();
            popover.set_parent(&email_chip);
            popover.set_has_arrow(false);

            let menu_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(2)
                .margin_top(6)
                .margin_bottom(6)
                .margin_start(6)
                .margin_end(6)
                .build();

            // New Email button
            let new_email_btn = gtk4::Button::builder()
                .label(&tr("New Email"))
                .css_classes(["flat"])
                .build();
            let email_for_compose = from_email.clone();
            let popover_clone = popover.clone();
            new_email_btn.connect_clicked(move |btn| {
                popover_clone.popdown();
                // Emit signal to open compose with this address
                if let Some(window) = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok()) {
                    window.activate_action("win.compose-to", Some(&email_for_compose.to_variant()));
                }
            });
            menu_box.append(&new_email_btn);

            // Copy Address button
            let copy_btn = gtk4::Button::builder()
                .label(&tr("Copy Address"))
                .css_classes(["flat"])
                .build();
            let email_for_copy = from_email.clone();
            let popover_clone2 = popover.clone();
            copy_btn.connect_clicked(move |btn| {
                popover_clone2.popdown();
                let display = btn.display();
                let clipboard = display.clipboard();
                clipboard.set_text(&email_for_copy);
            });
            menu_box.append(&copy_btn);

            // Add to Contacts button
            let add_contact_btn = gtk4::Button::builder()
                .label(&tr("Add to Contacts"))
                .css_classes(["flat"])
                .build();
            let email_for_contact = from_email.clone();
            let name_for_contact = from_name.clone();
            let popover_clone3 = popover.clone();
            add_contact_btn.connect_clicked(move |btn| {
                popover_clone3.popdown();
                // Add to GNOME Contacts via Evolution Data Server
                let name = name_for_contact.clone();
                let email = email_for_contact.clone();
                glib::spawn_future_local(async move {
                    if let Err(e) = Self::add_to_gnome_contacts(&name, &email).await {
                        tracing::error!("Failed to add contact: {}", e);
                    }
                });
            });
            menu_box.append(&add_contact_btn);

            popover.set_child(Some(&menu_box));

            // Right-click gesture for context menu
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(3); // Right click
            let popover_clone = popover.clone();
            gesture.connect_pressed(move |gesture, _n, x, y| {
                gesture.set_state(gtk4::EventSequenceState::Claimed);
                popover_clone.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                popover_clone.popup();
            });
            email_chip.add_controller(gesture);

            sender_info.append(&email_chip);
            sender_row.append(&sender_info);

            // Date on right
            let date_label = gtk4::Label::builder()
                .label(&message.date)
                .css_classes(["message-date"])
                .valign(gtk4::Align::Start)
                .build();
            sender_row.append(&date_label);

            header_box.append(&sender_row);

            // Subject
            let subject_label = gtk4::Label::builder()
                .label(&message.subject)
                .xalign(0.0)
                .css_classes(["message-subject"])
                .wrap(true)
                .margin_top(8)
                .build();
            header_box.append(&subject_label);

            // Recipients section
            let recipients_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(4)
                .margin_top(8)
                .build();

            // To:
            if !message.to.is_empty() {
                let to_row = gtk4::Box::builder()
                    .orientation(gtk4::Orientation::Horizontal)
                    .spacing(6)
                    .build();

                let to_label = gtk4::Label::builder()
                    .label(&tr("To:"))
                    .css_classes(["message-recipients-label"])
                    .xalign(0.0)
                    .build();

                let to_value = gtk4::Label::builder()
                    .label(&message.to.join(", "))
                    .css_classes(["message-recipients"])
                    .xalign(0.0)
                    .hexpand(true)
                    .wrap(true)
                    .build();

                to_row.append(&to_label);
                to_row.append(&to_value);
                recipients_box.append(&to_row);
            }

            // Cc:
            if !message.cc.is_empty() {
                let cc_row = gtk4::Box::builder()
                    .orientation(gtk4::Orientation::Horizontal)
                    .spacing(6)
                    .build();

                let cc_label = gtk4::Label::builder()
                    .label(&tr("Cc:"))
                    .css_classes(["message-recipients-label"])
                    .xalign(0.0)
                    .build();

                let cc_value = gtk4::Label::builder()
                    .label(&message.cc.join(", "))
                    .css_classes(["message-recipients"])
                    .xalign(0.0)
                    .hexpand(true)
                    .wrap(true)
                    .build();

                cc_row.append(&cc_label);
                cc_row.append(&cc_value);
                recipients_box.append(&cc_row);
            }

            header_box.append(&recipients_box);
        }

        // Update content
        if let Some(content_box) = imp.content_box.borrow().as_ref() {
            // Clear existing content
            while let Some(child) = content_box.first_child() as Option<gtk4::Widget> {
                content_box.remove(&child);
            }

            // Prefer HTML rendering via WebView, fall back to plain text
            #[cfg(feature = "webkit")]
            {
                if let Some(ref html) = message.html_body {
                    ensure_uri_schemes_registered();

                    // Rewrite all links to go through our northmail-link:// scheme
                    // which opens them in the default browser
                    let rewritten_html = rewrite_links_for_external_open(html);

                    // Log a snippet so we can verify links were rewritten
                    if let Some(pos) = rewritten_html.find("northmail-link://") {
                        let snippet_end = (pos + 80).min(rewritten_html.len());
                        tracing::info!("Links rewritten OK, sample: {}", &rewritten_html[pos..snippet_end]);
                    } else {
                        tracing::warn!("No links were rewritten in this email HTML ({} bytes)", html.len());
                    }

                    let webview = webkit6::WebView::new();
                    webview.set_vexpand(true);
                    webview.set_hexpand(true);

                    // Configure settings for email display
                    let settings: webkit6::Settings = webkit6::prelude::WebViewExt::settings(&webview).unwrap();
                    settings.set_enable_javascript(false);  // Security: no JS in emails
                    settings.set_auto_load_images(true);
                    settings.set_enable_developer_extras(true);  // Allow Web Inspector for debugging

                    // Load HTML directly — no custom URI scheme needed for the content itself
                    webview.load_html(&rewritten_html, None);
                    content_box.append(&webview);
                    return;
                }
            }

            // Plain text display (or fallback when no webkit)
            let display_text = if let Some(ref text) = message.text_body {
                Some(text.clone())
            } else if let Some(ref html) = message.html_body {
                Some(Self::strip_html_for_display(html))
            } else {
                None
            };

            if let Some(text) = display_text {
                let text_view = gtk4::TextView::builder()
                    .editable(false)
                    .cursor_visible(false)
                    .wrap_mode(gtk4::WrapMode::Word)
                    .vexpand(true)
                    .build();

                text_view.buffer().set_text(&text);
                content_box.append(&text_view);
            } else {
                let label = gtk4::Label::builder()
                    .label(&tr("[No content]"))
                    .css_classes(["dim-label"])
                    .build();
                content_box.append(&label);
            }
        }
    }

    /// Update the starred state shown in the message view header
    pub fn set_starred(&self, is_starred: bool) {
        if let Some(star_btn) = self.imp().star_button.borrow().as_ref() {
            star_btn.set_active(is_starred);
        }
    }

    /// Clear the message view
    pub fn clear(&self) {
        let imp = self.imp();

        if let Some(header_box) = imp.header_card.borrow().as_ref() {
            while let Some(child) = header_box.first_child() as Option<gtk4::Widget> {
                header_box.remove(&child);
            }
        }

        if let Some(content_box) = imp.content_box.borrow().as_ref() {
            while let Some(child) = content_box.first_child() as Option<gtk4::Widget> {
                content_box.remove(&child);
            }

            let placeholder = adw::StatusPage::builder()
                .icon_name("mail-read-symbolic")
                .title(&tr("No Message Selected"))
                .description(&tr("Select a message to view its contents"))
                .vexpand(true)
                .build();

            content_box.append(&placeholder);
        }
    }

    /// Strip HTML tags and convert to plain text for display
    fn strip_html_for_display(html: &str) -> String {
        let mut result = String::new();
        let mut in_tag = false;
        let mut in_style = false;
        let mut in_script = false;
        let mut last_was_space = true;

        let html_lower = html.to_lowercase();
        let mut chars = html.chars().peekable();
        let mut pos = 0;

        while let Some(c) = chars.next() {
            // Check for style/script start
            if !in_tag && html_lower[pos..].starts_with("<style") {
                in_style = true;
            } else if !in_tag && html_lower[pos..].starts_with("<script") {
                in_script = true;
            } else if in_style && html_lower[pos..].starts_with("</style") {
                in_style = false;
            } else if in_script && html_lower[pos..].starts_with("</script") {
                in_script = false;
            }

            match c {
                '<' => in_tag = true,
                '>' => {
                    in_tag = false;
                    // Add newline after block elements
                    if pos >= 4 {
                        let prev = &html_lower[pos.saturating_sub(10)..pos + 1];
                        if prev.contains("</p>")
                            || prev.contains("</div>")
                            || prev.contains("</br>")
                            || prev.contains("<br>")
                            || prev.contains("<br/>")
                            || prev.contains("<br />")
                            || prev.contains("</h1>")
                            || prev.contains("</h2>")
                            || prev.contains("</h3>")
                            || prev.contains("</li>")
                            || prev.contains("</tr>")
                        {
                            if !result.ends_with('\n') {
                                result.push('\n');
                                last_was_space = true;
                            }
                        }
                    }
                }
                _ if !in_tag && !in_style && !in_script => {
                    // Decode common HTML entities
                    if c == '&' {
                        let rest: String = chars.clone().take(10).collect();
                        if rest.starts_with("nbsp;") {
                            result.push(' ');
                            for _ in 0..5 {
                                chars.next();
                                pos += 1;
                            }
                        } else if rest.starts_with("lt;") {
                            result.push('<');
                            for _ in 0..3 {
                                chars.next();
                                pos += 1;
                            }
                        } else if rest.starts_with("gt;") {
                            result.push('>');
                            for _ in 0..3 {
                                chars.next();
                                pos += 1;
                            }
                        } else if rest.starts_with("amp;") {
                            result.push('&');
                            for _ in 0..4 {
                                chars.next();
                                pos += 1;
                            }
                        } else if rest.starts_with("quot;") {
                            result.push('"');
                            for _ in 0..5 {
                                chars.next();
                                pos += 1;
                            }
                        } else if rest.starts_with("apos;") {
                            result.push('\'');
                            for _ in 0..5 {
                                chars.next();
                                pos += 1;
                            }
                        } else if rest.starts_with("#39;") {
                            result.push('\'');
                            for _ in 0..4 {
                                chars.next();
                                pos += 1;
                            }
                        } else {
                            result.push('&');
                        }
                    } else if c.is_whitespace() {
                        if !last_was_space {
                            result.push(' ');
                            last_was_space = true;
                        }
                    } else {
                        result.push(c);
                        last_was_space = false;
                    }
                }
                _ => {}
            }
            pos += c.len_utf8();
        }

        // Clean up: collapse multiple newlines
        let mut cleaned = String::new();
        let mut last_was_newline = true;
        for c in result.chars() {
            if c == '\n' {
                if !last_was_newline {
                    cleaned.push('\n');
                    last_was_newline = true;
                }
            } else {
                cleaned.push(c);
                last_was_newline = false;
            }
        }

        cleaned.trim().to_string()
    }

    /// Add a contact to GNOME Contacts
    async fn add_to_gnome_contacts(name: &str, email: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Open GNOME Contacts - it will handle adding the contact
        // Note: gnome-contacts doesn't have a --new flag with contact data in most versions
        // So we just open it and let the user add manually for now
        tracing::info!("Opening GNOME Contacts to add: {} <{}>", name, email);

        // Try xdg-open first as it handles desktop environment detection
        let result = std::process::Command::new("xdg-open")
            .arg(format!("mailto:{}?", email))
            .spawn();

        if result.is_err() {
            // Fallback to gnome-contacts directly
            let _ = std::process::Command::new("gnome-contacts")
                .spawn();
        }

        Ok(())
    }
}

impl Default for MessageView {
    fn default() -> Self {
        Self::new()
    }
}

/// Full message details for display
pub struct MessageDetails {
    pub id: i64,
    pub uid: u32,
    pub subject: String,
    pub from_name: String,
    pub from_email: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub date: String,
    pub is_read: bool,
    pub is_starred: bool,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
    pub attachments: Vec<AttachmentInfo>,
}

/// Attachment information
pub struct AttachmentInfo {
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
}
