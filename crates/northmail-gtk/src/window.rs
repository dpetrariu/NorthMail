//! Main application window

use crate::application::NorthMailApplication;
use crate::widgets::{FolderSidebar, MessageList, MessageView};
use gtk4::{gio, glib, prelude::*, subclass::prelude::*};
use libadwaita as adw;
use libadwaita::prelude::*;
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
                                        <child type="start">
                                            <object class="GtkToggleButton" id="sidebar_toggle">
                                                <property name="icon-name">sidebar-show-symbolic</property>
                                                <property name="tooltip-text">Toggle Sidebar</property>
                                                <property name="active">true</property>
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
                                            <object class="GtkButton" id="compose_button">
                                                <property name="icon-name">mail-message-new-symbolic</property>
                                                <property name="tooltip-text">Compose</property>
                                                <property name="action-name">win.compose</property>
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
        pub sidebar_toggle: TemplateChild<gtk4::ToggleButton>,
        #[template_child]
        pub outer_split: TemplateChild<adw::OverlaySplitView>,
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
                .build();
            date_box.append(&date_value);

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
        imp.sidebar_toggle
            .bind_property("active", &*imp.outer_split, "show-sidebar")
            .sync_create()
            .bidirectional()
            .build();
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
        debug!("Opening compose dialog");

        // Compose-specific CSS
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string(
            "
            .compose-entry { background: transparent; border: none; outline: none; box-shadow: none; min-height: 28px; }
            .compose-entry:focus { background: transparent; border: none; outline: none; box-shadow: none; }
            .compose-entry > text { background: transparent; border: none; outline: none; box-shadow: none; }
            .compose-chip { background: alpha(currentColor, 0.08); border-radius: 14px; padding: 2px 4px 2px 10px; margin: 1px 0; }
            .compose-chip label { font-size: 0.9em; }
            .compose-chip button { min-width: 20px; min-height: 20px; padding: 0; margin: 0; }
            .compose-field-label { font-size: 0.9em; min-width: 52px; }
            ",
        );
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let dialog = adw::Dialog::builder()
            .title("New Message")
            .content_width(640)
            .content_height(560)
            .build();

        let toolbar_view = adw::ToolbarView::new();

        // Header bar — Send on right, close via dialog X
        let header = adw::HeaderBar::new();

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

        let from_model = gtk4::StringList::new(&[]);
        if let Some(app) = self.application() {
            if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                let accs = app.imp().accounts.borrow();
                for acc in accs.iter() {
                    from_model.append(&acc.email);
                }
            }
        }

        let from_dropdown = gtk4::DropDown::builder()
            .model(&from_model)
            .hexpand(true)
            .css_classes(["flat"])
            .build();

        from_box.append(&from_label);
        from_box.append(&from_dropdown);
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

        subject_box.append(&subject_label);
        subject_box.append(&subject_entry);
        fields_box.append(&subject_box);
        fields_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        content.append(&fields_box);

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

        toolbar_view.set_content(Some(&content));
        dialog.set_child(Some(&toolbar_view));

        // Send
        let window = self.clone();
        let dialog_ref = dialog.clone();
        let send_btn_ref = send_button.clone();
        send_button.connect_clicked(move |_| {
            let to_list = to_chips.borrow().clone();
            let cc_list = cc_chips.borrow().clone();
            let subject = subject_entry.text().to_string();
            let body = {
                let buf = text_view.buffer();
                let (start, end) = buf.bounds();
                buf.text(&start, &end, false).to_string()
            };

            if to_list.is_empty() {
                if let Some(win) = window.downcast_ref::<NorthMailWindow>() {
                    win.add_toast(adw::Toast::new("Please add at least one recipient"));
                }
                return;
            }

            let account_index = from_dropdown.selected();

            send_btn_ref.set_sensitive(false);
            send_btn_ref.set_label("Sending…");

            if let Some(app) = window.application() {
                if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                    let dialog_close = dialog_ref.clone();
                    let window_ref = window.clone();
                    let send_btn_restore = send_btn_ref.clone();
                    app.send_message(
                        account_index,
                        to_list,
                        cc_list,
                        subject,
                        body,
                        move |result| {
                            match result {
                                Ok(()) => {
                                    if let Some(win) = window_ref.downcast_ref::<NorthMailWindow>() {
                                        win.add_toast(adw::Toast::new("Message sent"));
                                    }
                                    dialog_close.close();
                                }
                                Err(e) => {
                                    if let Some(win) = window_ref.downcast_ref::<NorthMailWindow>() {
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

        dialog.present(Some(self));
    }

    /// Build an inline chip-based recipient row (label + chips + entry)
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

        // Horizontal box for chips + entry
        let chip_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(4)
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .build();

        // Inline entry (no frame, blends into the row)
        let entry = gtk4::Entry::builder()
            .hexpand(true)
            .has_frame(false)
            .placeholder_text("Add recipient")
            .css_classes(["compose-entry"])
            .build();

        chip_box.append(&entry);

        row.append(&label);
        row.append(&chip_box);

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
            let chip_box = chip_box.clone();
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

                // Insert chip before the entry
                chip_box.insert_child_after(&chip, entry.prev_sibling().as_ref());
                // Move entry to the end if needed — it should already be last
                // but GTK keeps insertion order, so just reorder
                chip_box.reorder_child_after(&entry, Some(&chip));

                // Remove handler
                let chip_box_ref = chip_box.clone();
                let chips_ref = chips.clone();
                let email_owned = email.to_string();
                let chip_ref = chip.clone();
                remove_btn.connect_clicked(move |_| {
                    chip_box_ref.remove(&chip_ref);
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

        // Debounced autocomplete
        let debounce_source: std::rc::Rc<std::cell::RefCell<Option<glib::SourceId>>> =
            std::rc::Rc::new(std::cell::RefCell::new(None));

        let window_clone = window.clone();
        let popover_change = popover.clone();
        let suggestion_list_ref = suggestion_list;
        entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            eprintln!("[autocomplete] changed: {:?}", text);

            if let Some(source_id) = debounce_source.borrow_mut().take() {
                source_id.remove();
            }

            if text.trim().is_empty() {
                popover_change.popdown();
                return;
            }

            let window_ref = window_clone.clone();
            let popover_ref = popover_change.clone();
            let suggestion_list_clone = suggestion_list_ref.clone();
            let debounce_ref = debounce_source.clone();
            let entry_ref = entry.clone();

            let source_id = glib::timeout_add_local_once(
                std::time::Duration::from_millis(300),
                move || {
                    debounce_ref.borrow_mut().take();
                    eprintln!("[autocomplete] debounce fired for {:?}", text);

                    if let Some(app) = window_ref.application() {
                        if let Some(app) = app.downcast_ref::<NorthMailApplication>() {
                            let popover_cb = popover_ref.clone();
                            let list_cb = suggestion_list_clone.clone();
                            let entry_popup = entry_ref.clone();
                            app.query_contacts(text, move |results| {
                                eprintln!("[autocomplete] callback: {} results", results.len());
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

                                // Position popover below the entry text area
                                let w = entry_popup.allocated_width();
                                let h = entry_popup.allocated_height();
                                popover_cb.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(0, 0, w, h)));
                                popover_cb.popup();
                            });
                        }
                    }
                },
            );

            debounce_source.borrow_mut().replace(source_id);
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
