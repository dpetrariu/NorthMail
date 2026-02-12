//! Message view widget

use gtk4::{glib, prelude::*, subclass::prelude::*};
use libadwaita as adw;
#[cfg(feature = "webkit")]
use webkit6::prelude::*;

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct MessageView {
        pub header_box: RefCell<Option<gtk4::Box>>,
        pub content_box: RefCell<Option<gtk4::Box>>,
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

        // Toolbar with message actions
        let toolbar = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();

        let reply_button = gtk4::Button::builder()
            .icon_name("mail-reply-sender-symbolic")
            .tooltip_text("Reply")
            .build();

        let reply_all_button = gtk4::Button::builder()
            .icon_name("mail-reply-all-symbolic")
            .tooltip_text("Reply All")
            .build();

        let forward_button = gtk4::Button::builder()
            .icon_name("mail-forward-symbolic")
            .tooltip_text("Forward")
            .build();

        let archive_button = gtk4::Button::builder()
            .icon_name("folder-symbolic")
            .tooltip_text("Archive")
            .build();

        let delete_button = gtk4::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Delete")
            .css_classes(["destructive-action"])
            .build();

        let spacer = gtk4::Box::builder()
            .hexpand(true)
            .build();

        let star_button = gtk4::ToggleButton::builder()
            .icon_name("starred-symbolic")
            .tooltip_text("Star")
            .build();

        let more_button = gtk4::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text("More Actions")
            .build();

        toolbar.append(&reply_button);
        toolbar.append(&reply_all_button);
        toolbar.append(&forward_button);
        toolbar.append(&archive_button);
        toolbar.append(&delete_button);
        toolbar.append(&spacer);
        toolbar.append(&star_button);
        toolbar.append(&more_button);

        self.append(&toolbar);

        // Separator
        let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        self.append(&separator);

        // Header section
        let header_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .margin_bottom(12)
            .build();

        self.append(&header_box);
        imp.header_box.replace(Some(header_box));

        // Another separator
        let separator2 = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        self.append(&separator2);

        // Content area (will contain WebKitWebView for HTML or TextView for plain text)
        let content_scrolled = gtk4::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .css_classes(["view"])
            .build();

        let content_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .margin_bottom(12)
            .build();

        // Placeholder
        let placeholder = adw::StatusPage::builder()
            .icon_name("mail-read-symbolic")
            .title("No Message Selected")
            .description("Select a message to view its contents")
            .vexpand(true)
            .build();

        content_box.append(&placeholder);
        content_scrolled.set_child(Some(&content_box));
        self.append(&content_scrolled);

        imp.content_box.replace(Some(content_box));
    }

    /// Display a message
    pub fn show_message(&self, message: &MessageDetails) {
        let imp = self.imp();

        // Update header
        if let Some(header_box) = imp.header_box.borrow().as_ref() {
            // Clear existing content
            while let Some(child) = header_box.first_child() as Option<gtk4::Widget> {
                header_box.remove(&child);
            }

            // Subject
            let subject_label = gtk4::Label::builder()
                .label(&message.subject)
                .xalign(0.0)
                .css_classes(["title-2"])
                .wrap(true)
                .build();
            header_box.append(&subject_label);

            // From
            let from_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(6)
                .build();

            let from_label = gtk4::Label::builder()
                .label("From:")
                .css_classes(["dim-label"])
                .build();

            let from_value = gtk4::Label::builder()
                .label(&message.from)
                .xalign(0.0)
                .hexpand(true)
                .build();

            from_box.append(&from_label);
            from_box.append(&from_value);
            header_box.append(&from_box);

            // To
            let to_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(6)
                .build();

            let to_label = gtk4::Label::builder()
                .label("To:")
                .css_classes(["dim-label"])
                .build();

            let to_value = gtk4::Label::builder()
                .label(&message.to.join(", "))
                .xalign(0.0)
                .hexpand(true)
                .wrap(true)
                .build();

            to_box.append(&to_label);
            to_box.append(&to_value);
            header_box.append(&to_box);

            // Date
            let date_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(6)
                .build();

            let date_label = gtk4::Label::builder()
                .label("Date:")
                .css_classes(["dim-label"])
                .build();

            let date_value = gtk4::Label::builder()
                .label(&message.date)
                .xalign(0.0)
                .build();

            date_box.append(&date_label);
            date_box.append(&date_value);
            header_box.append(&date_box);
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
                    let webview = webkit6::WebView::new();
                    webview.set_vexpand(true);
                    webview.set_hexpand(true);

                    // Configure settings for email display
                    let settings: webkit6::Settings = webkit6::prelude::WebViewExt::settings(&webview).unwrap();
                    settings.set_enable_javascript(false);  // Security: no JS in emails
                    settings.set_auto_load_images(true);

                    webview.load_html(html, None);
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
                    .build();

                text_view.buffer().set_text(&text);
                content_box.append(&text_view);
            } else {
                let label = gtk4::Label::builder()
                    .label("[No content]")
                    .css_classes(["dim-label"])
                    .build();
                content_box.append(&label);
            }
        }
    }

    /// Clear the message view
    pub fn clear(&self) {
        let imp = self.imp();

        if let Some(header_box) = imp.header_box.borrow().as_ref() {
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
                .title("No Message Selected")
                .description("Select a message to view its contents")
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
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub date: String,
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
