//! Main application window

use crate::application::{NorthMailApplication, ParsedAttachment};
use crate::widgets::{FolderSidebar, MessageList, MessageView};
use gtk4::{gio, glib, prelude::*, subclass::prelude::*};
use libadwaita as adw;
use libadwaita::prelude::*;
use std::rc::Rc;
use tracing::debug;


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
                                            <object class="GtkMenuButton" id="primary_menu_button">
                                                <property name="icon-name">open-menu-symbolic</property>
                                                <property name="tooltip-text">Main Menu</property>
                                                <property name="menu-model">primary_menu</property>
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
                                    <object class="AdwOverlaySplitView" id="outer_split">
                                        <property name="sidebar-position">start</property>
                                        <property name="collapsed">false</property>
                                        <property name="max-sidebar-width">280</property>
                                        <property name="min-sidebar-width">200</property>
                                        <property name="sidebar">
                                            <object class="GtkBox" id="sidebar_box">
                                                <property name="orientation">vertical</property>
                                                <property name="width-request">200</property>
                                            </object>
                                        </property>
                                        <property name="content">
                                            <object class="AdwNavigationSplitView" id="inner_split">
                                                <property name="sidebar-width-fraction">0.4</property>
                                                <property name="min-sidebar-width">300</property>
                                                <property name="max-sidebar-width">600</property>
                                                <property name="sidebar">
                                                    <object class="AdwNavigationPage">
                                                        <property name="title">Messages</property>
                                                        <property name="child">
                                                            <object class="GtkBox" id="message_list_box">
                                                                <property name="orientation">vertical</property>
                                                            </object>
                                                        </property>
                                                    </object>
                                                </property>
                                                <property name="content">
                                                    <object class="AdwNavigationPage">
                                                        <property name="title">Message</property>
                                                        <property name="child">
                                                            <object class="GtkBox" id="message_view_box">
                                                                <property name="orientation">vertical</property>
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
                    </object>
                </property>
            </template>
            <menu id="primary_menu">
                <section>
                    <item>
                        <attribute name="label" translatable="yes">Add Account</attribute>
                        <attribute name="action">app.add-account</attribute>
                    </item>
                </section>
                <section>
                    <item>
                        <attribute name="label" translatable="yes">Preferences</attribute>
                        <attribute name="action">app.preferences</attribute>
                    </item>
                    <item>
                        <attribute name="label" translatable="yes">Keyboard Shortcuts</attribute>
                        <attribute name="action">win.show-help-overlay</attribute>
                    </item>
                    <item>
                        <attribute name="label" translatable="yes">About NorthMail</attribute>
                        <attribute name="action">app.about</attribute>
                    </item>
                </section>
            </menu>
        </interface>
    "#)]
    pub struct NorthMailWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub header_bar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub outer_split: TemplateChild<adw::OverlaySplitView>,
        /// Sidebar toggle button (created in setup_widgets)
        pub sidebar_toggle: std::cell::RefCell<Option<gtk4::ToggleButton>>,
        #[template_child]
        pub inner_split: TemplateChild<adw::NavigationSplitView>,
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
             }"
        );
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER,
        );

        // Create sidebar toggle button and add to header bar
        // Position: after the title, with margin to align with sidebar's right edge
        let sidebar_toggle = gtk4::ToggleButton::builder()
            .icon_name("dock-left-symbolic")
            .tooltip_text("Toggle Sidebar")
            .active(true)
            .margin_start(126)  // Push to align with sidebar right edge (when visible)
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

        // Adjust button positions when sidebar visibility changes
        let toggle_for_signal = sidebar_toggle.clone();
        let compose_for_signal = compose_button.clone();
        imp.outer_split.connect_notify_local(Some("show-sidebar"), move |split, _| {
            if split.shows_sidebar() {
                // Sidebar visible: push buttons to align with columns
                toggle_for_signal.set_margin_start(126);
                compose_for_signal.set_margin_start(0);
            } else {
                // Sidebar hidden: move buttons next to title
                toggle_for_signal.set_margin_start(8);
                compose_for_signal.set_margin_start(8);
            }
        });

        // Create and add folder sidebar
        let folder_sidebar = FolderSidebar::new();
        imp.sidebar_box.append(&folder_sidebar);
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

        // Find the message in the list
        let messages = message_list.imp().messages.borrow();
        let msg = messages.iter().find(|m| m.uid == uid).cloned();
        drop(messages); // Release borrow

        if let Some(msg) = msg {
            // Clear current content
            while let Some(child) = imp.message_view_box.first_child() {
                imp.message_view_box.remove(&child);
            }

            // Create message view content
            let content = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(0)
                .vexpand(true)
                .build();

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
                .build();

            let spacer = gtk4::Box::builder()
                .hexpand(true)
                .build();

            let star_button = gtk4::ToggleButton::builder()
                .icon_name("starred-symbolic")
                .tooltip_text("Star")
                .active(msg.is_starred)
                .build();

            toolbar.append(&reply_button);
            toolbar.append(&reply_all_button);
            toolbar.append(&forward_button);
            toolbar.append(&archive_button);
            toolbar.append(&delete_button);
            toolbar.append(&spacer);
            toolbar.append(&star_button);

            content.append(&toolbar);

            // Separator
            let separator1 = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            content.append(&separator1);

            // Header area
            let header_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(6)
                .margin_start(16)
                .margin_end(16)
                .margin_top(12)
                .margin_bottom(12)
                .build();

            // Subject
            let subject_label = gtk4::Label::builder()
                .label(&msg.subject)
                .xalign(0.0)
                .wrap(true)
                .css_classes(["title-2"])
                .build();
            header_box.append(&subject_label);

            // From
            let from_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(8)
                .build();

            let from_label = gtk4::Label::builder()
                .label("From:")
                .css_classes(["dim-label"])
                .xalign(1.0)
                .width_request(38)
                .build();
            from_box.append(&from_label);

            let from_value = gtk4::Label::builder()
                .label(&msg.from)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build();
            from_box.append(&from_value);

            header_box.append(&from_box);

            // To
            if !msg.to.is_empty() {
                let to_box = gtk4::Box::builder()
                    .orientation(gtk4::Orientation::Horizontal)
                    .spacing(8)
                    .build();

                let to_label = gtk4::Label::builder()
                    .label("To:")
                    .css_classes(["dim-label"])
                    .xalign(1.0)
                    .width_request(38)
                    .build();
                to_box.append(&to_label);

                let to_value = gtk4::Label::builder()
                    .label(&msg.to)
                    .xalign(0.0)
                    .hexpand(true)
                    .wrap(true)
                    .build();
                to_box.append(&to_value);

                header_box.append(&to_box);
            }

            // Date — format using system locale via GLib DateTime
            let formatted_date = if let Some(epoch) = msg.date_epoch {
                glib::DateTime::from_unix_local(epoch)
                    .and_then(|dt| dt.format("%c"))
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| msg.date.clone())
            } else {
                msg.date.clone()
            };

            let date_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(8)
                .build();

            let date_label = gtk4::Label::builder()
                .label("Date:")
                .css_classes(["dim-label"])
                .xalign(1.0)
                .width_request(38)
                .build();
            date_box.append(&date_label);

            let date_value = gtk4::Label::builder()
                .label(&formatted_date)
                .xalign(0.0)
                .hexpand(true)
                .build();
            date_box.append(&date_value);

            // Attachment dropdown placeholder (populated after body fetch)
            // Sits on the same row as the date, pushed to the right
            let attachment_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .halign(gtk4::Align::End)
                .build();
            date_box.append(&attachment_box);

            header_box.append(&date_box);

            content.append(&header_box);

            // Separator
            let separator2 = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            content.append(&separator2);

            // Body area with loading indicator initially
            let body_scrolled = gtk4::ScrolledWindow::builder()
                .vexpand(true)
                .hexpand(true)
                .build();

            let body_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .margin_start(16)
                .margin_end(16)
                .margin_top(12)
                .margin_bottom(12)
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
    }

    fn setup_bindings(&self) {
        let imp = self.imp();

        // Bind sidebar toggle to split view
        if let Some(ref toggle) = *imp.sidebar_toggle.borrow() {
            toggle
                .bind_property("active", &*imp.outer_split, "show-sidebar")
                .sync_create()
                .bidirectional()
                .build();
        }
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
        debug!("Opening compose window");

        // Compose-specific CSS
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string(
            "
            .compose-entry { background: transparent; border: none; outline: none; box-shadow: none; min-height: 28px; }
            .compose-entry:focus { background: transparent; border: none; outline: none; box-shadow: none; }
            .compose-entry > text { background: transparent; border: none; outline: none; box-shadow: none; }
            .compose-chip { background: alpha(currentColor, 0.08); border-radius: 12px; padding: 0px 2px 0px 8px; margin: 0; }
            .compose-chip label { font-size: 0.9em; }
            .compose-chip button { min-width: 18px; min-height: 18px; padding: 0; margin: 0; }
            .compose-field-label { font-size: 0.9em; min-width: 52px; }
            .attachment-pill { background: alpha(currentColor, 0.08); border-radius: 12px; padding: 2px 4px 2px 8px; }
            .attachment-pill:hover { background: alpha(currentColor, 0.12); }
            .attachment-pill label { font-size: 0.85em; }
            .attachment-pill image { margin-right: 2px; }
            .attachment-pill button { min-width: 16px; min-height: 16px; padding: 0; margin: 0; }
            .warning { color: @warning_color; }
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

        // Header bar — Send on right, draft status on left
        let header = adw::HeaderBar::new();

        // Draft status label (hidden by default)
        let draft_label = gtk4::Label::builder()
            .label("Saved as Draft")
            .css_classes(["dim-label"])
            .visible(false)
            .build();
        header.pack_start(&draft_label);

        let send_button = gtk4::Button::builder()
            .label("Send")
            .css_classes(["suggested-action", "pill"])
            .build();

        header.pack_end(&send_button);
        toolbar_view.add_top_bar(&header);

        // Main content
        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .build();

        // --- Header fields (From, To, Cc, Subject) ---
        let fields_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .margin_start(12)
            .margin_end(12)
            .build();

        // Label width for alignment
        let label_width = 56;

        // From selector
        let from_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .build();

        let from_label = gtk4::Label::builder()
            .label("From")
            .xalign(1.0)
            .width_request(label_width)
            .css_classes(["dim-label", "compose-field-label"])
            .build();

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
            .hexpand(true)
            .css_classes(["flat"])
            .build();

        // Warning icon button (hidden by default, shown for non-sendable accounts)
        let warning_button = gtk4::Button::builder()
            .icon_name("dialog-warning-symbolic")
            .css_classes(["flat", "circular", "warning"])
            .tooltip_text("This account cannot send emails")
            .visible(false)
            .build();

        from_box.append(&from_label);
        from_box.append(&from_dropdown);
        from_box.append(&warning_button);
        fields_box.append(&from_box);
        fields_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        // To / Cc chip rows
        let to_chips: std::rc::Rc<std::cell::RefCell<Vec<String>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let cc_chips: std::rc::Rc<std::cell::RefCell<Vec<String>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));

        let to_row = Self::build_chip_row("To", to_chips.clone(), self, label_width);
        let cc_row = Self::build_chip_row("Cc", cc_chips.clone(), self, label_width);

        fields_box.append(&to_row);
        fields_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
        fields_box.append(&cc_row);
        fields_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        // Subject
        let subject_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
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
        fields_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

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

        let scrolled = gtk4::ScrolledWindow::builder()
            .child(&text_view)
            .vexpand(true)
            .build();

        content.append(&scrolled);

        // Attachments area at bottom (hidden until files are added)
        let attachments_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(8)
            .build();

        let attachments_flow = gtk4::FlowBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .homogeneous(false)
            .row_spacing(4)
            .column_spacing(6)
            .max_children_per_line(20)
            .build();

        attachments_box.append(&attachments_flow);
        attachments_box.set_visible(false);
        content.append(&attachments_box);

        toolbar_view.set_content(Some(&content));
        compose_window.set_content(Some(&toolbar_view));

        // --- Attachment button handler ---
        let add_attachment_to_ui = {
            let attachments = attachments.clone();
            let attachments_flow = attachments_flow.clone();
            let attachments_box = attachments_box.clone();

            move |filename: String, mime_type: String, data: Vec<u8>, temp_path: Option<std::path::PathBuf>| {
                attachments.borrow_mut().push((filename.clone(), mime_type, data, temp_path.clone()));
                let filename_for_remove = filename.clone();

                // Create pill widget - natural width based on filename
                let pill = gtk4::Box::builder()
                    .orientation(gtk4::Orientation::Horizontal)
                    .spacing(4)
                    .css_classes(["attachment-pill"])
                    .build();

                let icon = gtk4::Image::builder()
                    .icon_name("mail-attachment-symbolic")
                    .pixel_size(14)
                    .build();

                let label = gtk4::Label::builder()
                    .label(&filename)
                    .ellipsize(gtk4::pango::EllipsizeMode::Middle)
                    .max_width_chars(25)
                    .build();

                let remove_btn = gtk4::Button::builder()
                    .icon_name("window-close-symbolic")
                    .css_classes(["flat", "circular"])
                    .tooltip_text("Remove attachment")
                    .build();

                pill.append(&icon);
                pill.append(&label);
                pill.append(&remove_btn);

                // Click on pill to open the file
                let gesture = gtk4::GestureClick::new();
                let temp_path_open = temp_path.clone();
                let filename_open = filename.clone();
                gesture.connect_released(move |_, _, _, _| {
                    if let Some(ref path) = temp_path_open {
                        let _ = std::process::Command::new("xdg-open")
                            .arg(path)
                            .spawn();
                    } else {
                        debug!("No temp path for attachment {}", filename_open);
                    }
                });
                pill.add_controller(gesture);

                // Remove button handler
                let attachments_remove = attachments.clone();
                let attachments_box_remove = attachments_box.clone();
                let attachments_flow_remove = attachments_flow.clone();
                let pill_weak = pill.downgrade();
                let filename_to_remove = filename_for_remove.clone();
                remove_btn.connect_clicked(move |_| {
                    // Find and remove from list by filename
                    let mut atts = attachments_remove.borrow_mut();
                    if let Some(pos) = atts.iter().position(|(f, _, _, _)| f == &filename_to_remove) {
                        atts.remove(pos);
                    }
                    // Remove pill from flow
                    if let Some(pill) = pill_weak.upgrade() {
                        if let Some(parent) = pill.parent() {
                            if let Some(flow_child) = parent.downcast_ref::<gtk4::FlowBoxChild>() {
                                attachments_flow_remove.remove(flow_child);
                            }
                        }
                    }
                    // Hide box if no attachments left
                    if attachments_remove.borrow().is_empty() {
                        attachments_box_remove.set_visible(false);
                    }
                });

                attachments_flow.append(&pill);
                attachments_box.set_visible(true);
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
        let drop_target = gtk4::DropTarget::new(gtk4::gio::File::static_type(), gtk4::gdk::DragAction::COPY);
        {
            let add_attachment = add_attachment_to_ui.clone();
            drop_target.connect_drop(move |_, value, _, _| {
                if let Ok(file) = value.get::<gtk4::gio::File>() {
                    if let Some(path) = file.path() {
                        if let Ok(data) = std::fs::read(&path) {
                            let filename = path.file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| "attachment".to_string());

                            let mime_type = Self::guess_mime_type(&filename);
                            add_attachment(filename, mime_type, data, Some(path));
                            return true;
                        }
                    }
                }
                false
            });
        }
        text_view.add_controller(drop_target);

        // Also add drop target on the header fields area
        let drop_target2 = gtk4::DropTarget::new(gtk4::gio::File::static_type(), gtk4::gdk::DragAction::COPY);
        {
            let add_attachment = add_attachment_to_ui.clone();
            drop_target2.connect_drop(move |_, value, _, _| {
                if let Ok(file) = value.get::<gtk4::gio::File>() {
                    if let Some(path) = file.path() {
                        if let Ok(data) = std::fs::read(&path) {
                            let filename = path.file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| "attachment".to_string());

                            let mime_type = Self::guess_mime_type(&filename);
                            add_attachment(filename, mime_type, data, Some(path));
                            return true;
                        }
                    }
                }
                false
            });
        }
        fields_box.add_controller(drop_target2);

        // --- Draft auto-save state ---
        // Track the saved draft: (account_index, uid)
        let draft_state: std::rc::Rc<std::cell::RefCell<Option<(u32, u32)>>> =
            std::rc::Rc::new(std::cell::RefCell::new(None));
        // Track the auto-save timer source ID so we can cancel/reset it
        let timer_source: std::rc::Rc<std::cell::Cell<Option<glib::SourceId>>> =
            std::rc::Rc::new(std::cell::Cell::new(None));
        // Whether the message was sent (skip close confirmation)
        let was_sent: std::rc::Rc<std::cell::Cell<bool>> =
            std::rc::Rc::new(std::cell::Cell::new(false));
        // Whether draft save is currently in progress (prevent overlapping saves)
        let save_in_progress: std::rc::Rc<std::cell::Cell<bool>> =
            std::rc::Rc::new(std::cell::Cell::new(false));

        // --- Auto-save helper: resets timer on any edit ---
        let setup_auto_save_timer = {
            let timer_source = timer_source.clone();
            let draft_state = draft_state.clone();
            let save_in_progress = save_in_progress.clone();
            let to_chips_save = to_chips.clone();
            let cc_chips_save = cc_chips.clone();
            let subject_entry_save = subject_entry.clone();
            let text_view_save = text_view.clone();
            let from_dropdown_save = from_dropdown.clone();
            let main_window = self.clone();
            let draft_label_save = draft_label.clone();

            move || {
                eprintln!("[draft] Reset timer called - scheduling 5s auto-save");
                // Cancel any existing timer
                if let Some(source_id) = timer_source.take() {
                    eprintln!("[draft] Cancelled previous timer");
                    source_id.remove();
                }

                // Don't schedule if a save is already in progress
                if save_in_progress.get() {
                    eprintln!("[draft] Save in progress, skipping");
                    return;
                }

                let draft_state_timer = draft_state.clone();
                let save_in_progress_timer = save_in_progress.clone();
                let to_chips_timer = to_chips_save.clone();
                let cc_chips_timer = cc_chips_save.clone();
                let subject_entry_timer = subject_entry_save.clone();
                let text_view_timer = text_view_save.clone();
                let from_dropdown_timer = from_dropdown_save.clone();
                let main_window_timer = main_window.clone();
                let draft_label_timer = draft_label_save.clone();

                let source_id = glib::timeout_add_seconds_local_once(5, move || {
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

                    // Show immediate visual feedback that save is starting
                    draft_label_timer.set_label("Saving...");
                    draft_label_timer.set_visible(true);

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
                    for addr in &to_list {
                        msg = msg.to(addr);
                    }
                    for addr in &cc_list {
                        msg = msg.cc(addr);
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
                        let draft_label_cb = draft_label_timer.clone();
                        eprintln!("[draft] Deleting old draft uid={} then saving new", old_uid);
                        app_delete.delete_draft(old_acct, old_uid, move |_| {
                            // Ignore delete errors — old draft may already be gone
                            let draft_label_inner = draft_label_cb.clone();
                            eprintln!("[draft] Calling save_draft (after delete) for account {}", account_index);
                            app_save.save_draft(account_index, msg, move |result| {
                                save_in_progress_cb.set(false);
                                match result {
                                    Ok(Some(uid)) => {
                                        eprintln!("[draft] Saved! uid={}", uid);
                                        *draft_state_cb.borrow_mut() = Some((account_index, uid));
                                        draft_label_inner.set_label("Saved as Draft");
                                    }
                                    Ok(None) => {
                                        eprintln!("[draft] Saved (no uid returned)");
                                        *draft_state_cb.borrow_mut() = None;
                                        draft_label_inner.set_label("Saved as Draft");
                                    }
                                    Err(e) => {
                                        eprintln!("[draft] Save FAILED: {}", e);
                                        draft_label_inner.set_label("Save failed");
                                    }
                                }
                            });
                        });
                    } else {
                        let app_save = app.clone();
                        let draft_label_cb = draft_label_timer.clone();
                        eprintln!("[draft] Calling save_draft for account {}", account_index);
                        app_save.save_draft(account_index, msg, move |result| {
                            save_in_progress_cb.set(false);
                            match result {
                                Ok(Some(uid)) => {
                                    eprintln!("[draft] Saved! uid={}", uid);
                                    *draft_state_cb.borrow_mut() = Some((account_index, uid));
                                    draft_label_cb.set_label("Saved as Draft");
                                }
                                Ok(None) => {
                                    eprintln!("[draft] Saved (no uid returned)");
                                    *draft_state_cb.borrow_mut() = None;
                                    draft_label_cb.set_label("Saved as Draft");
                                }
                                Err(e) => {
                                    eprintln!("[draft] Save FAILED: {}", e);
                                    draft_label_cb.set_label("Save failed");
                                }
                            }
                        });
                    }
                });

                timer_source.set(Some(source_id));
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
        let timer_source_send = timer_source.clone();
        let attachments_send = attachments.clone();
        send_button.connect_clicked(move |_| {
            let to_list = to_chips.borrow().clone();
            let cc_list = cc_chips.borrow().clone();
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

            // Cancel auto-save timer
            if let Some(source_id) = timer_source_send.take() {
                source_id.remove();
            }

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
        let timer_source_close = timer_source;
        compose_window.connect_close_request(move |win| {
            // Cancel any pending auto-save timer
            if let Some(source_id) = timer_source_close.take() {
                source_id.remove();
            }

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
        window: &NorthMailWindow,
        label_width: i32,
    ) -> gtk4::Box {
        let row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
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

        // FlowBox wraps chips + entry to the next line when space runs out.
        // Wrapped in a ScrolledWindow (no horizontal scroll) to constrain
        // width so the FlowBox actually triggers line wrapping.
        let chip_flow = gtk4::FlowBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .homogeneous(false)
            .max_children_per_line(100)
            .min_children_per_line(1)
            .hexpand(true)
            .row_spacing(4)
            .column_spacing(4)
            .build();

        // Inline entry (no frame, blends into the row)
        let entry = gtk4::Entry::builder()
            .hexpand(true)
            .has_frame(false)
            .placeholder_text("Add recipient")
            .css_classes(["compose-entry"])
            .build();

        chip_flow.append(&entry);

        let chip_scroll = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Never)
            .hexpand(true)
            .child(&chip_flow)
            .build();

        row.append(&label);
        row.append(&chip_scroll);

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
        let add_chip = {
            let chip_flow = chip_flow.clone();
            let chips = chips.clone();
            let entry = entry.clone();
            move |display: &str, email: &str| {
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
                    .build();

                let chip_label = gtk4::Label::builder()
                    .label(&chip_text)
                    .ellipsize(gtk4::pango::EllipsizeMode::End)
                    .max_width_chars(24)
                    .build();

                let remove_btn = gtk4::Button::builder()
                    .icon_name("window-close-symbolic")
                    .css_classes(["flat", "circular"])
                    .valign(gtk4::Align::Center)
                    .build();

                chip.append(&chip_label);
                chip.append(&remove_btn);

                // Count children to find position before the entry (which is always last)
                let mut count = 0;
                let mut child = chip_flow.first_child();
                while let Some(c) = child {
                    count += 1;
                    child = c.next_sibling();
                }
                // Insert chip before the entry's FlowBoxChild (entry is at position count-1)
                chip_flow.insert(&chip, count as i32 - 1);

                // Remove handler — remove the FlowBoxChild wrapper
                let chip_flow_ref = chip_flow.clone();
                let chips_ref = chips.clone();
                let email_owned = email.to_string();
                let chip_ref = chip.clone();
                remove_btn.connect_clicked(move |_| {
                    // chip_ref's parent is the FlowBoxChild wrapper
                    if let Some(flow_child) = chip_ref.parent() {
                        chip_flow_ref.remove(&flow_child);
                    }
                    chips_ref.borrow_mut().retain(|e| e != &email_owned);
                });

                entry.set_text("");
                entry.grab_focus();
            }
        };

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
        let suggestion_list_ref = suggestion_list;
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

        row
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
        if folder_id == 0 {
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
                                message_list.set_messages(infos);
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
