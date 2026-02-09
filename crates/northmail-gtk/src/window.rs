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

            // Date
            let date_box = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(8)
                .build();

            let date_label = gtk4::Label::builder()
                .label("Date:")
                .css_classes(["dim-label"])
                .build();
            date_box.append(&date_label);

            let date_value = gtk4::Label::builder()
                .label(&msg.date)
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
                    app.fetch_message_body(uid, move |result| {
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

        let dialog = adw::Dialog::builder()
            .title("New Message")
            .content_width(600)
            .content_height(500)
            .build();

        let toolbar_view = adw::ToolbarView::new();

        // Header bar with actions
        let header = adw::HeaderBar::new();

        let send_button = gtk4::Button::builder()
            .label("Send")
            .css_classes(["suggested-action"])
            .build();

        let discard_button = gtk4::Button::builder()
            .label("Discard")
            .css_classes(["destructive-action"])
            .build();

        header.pack_start(&discard_button);
        header.pack_end(&send_button);

        toolbar_view.add_top_bar(&header);

        // Compose form
        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(12)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .margin_bottom(12)
            .build();

        let to_entry = adw::EntryRow::builder()
            .title("To")
            .build();

        let cc_entry = adw::EntryRow::builder()
            .title("Cc")
            .build();

        let subject_entry = adw::EntryRow::builder()
            .title("Subject")
            .build();

        let recipients_group = adw::PreferencesGroup::new();
        recipients_group.add(&to_entry);
        recipients_group.add(&cc_entry);
        recipients_group.add(&subject_entry);

        content.append(&recipients_group);

        // Text editor
        let text_view = gtk4::TextView::builder()
            .vexpand(true)
            .hexpand(true)
            .wrap_mode(gtk4::WrapMode::Word)
            .left_margin(12)
            .right_margin(12)
            .top_margin(12)
            .bottom_margin(12)
            .build();

        let scrolled = gtk4::ScrolledWindow::builder()
            .child(&text_view)
            .vexpand(true)
            .build();

        let frame = gtk4::Frame::builder()
            .child(&scrolled)
            .vexpand(true)
            .build();

        content.append(&frame);

        toolbar_view.set_content(Some(&content));
        dialog.set_child(Some(&toolbar_view));

        // Connect close on discard
        let dialog_ref = dialog.clone();
        discard_button.connect_clicked(move |_| {
            dialog_ref.close();
        });

        // Connect send action
        let dialog_ref = dialog.clone();
        send_button.connect_clicked(move |_| {
            // TODO: Actually send the message
            dialog_ref.close();
        });

        dialog.present(Some(self));
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
