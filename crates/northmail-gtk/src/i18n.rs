use gettextrs::gettext;
use gettextrs::ngettext;

/// Initialize gettext for the application.
pub fn init() {
    gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");

    // For meson builds, LOCALEDIR is set at compile time.
    // For cargo builds, fall back to a locale dir relative to the project root,
    // or /usr/share/locale for installed builds.
    let localedir = option_env!("LOCALEDIR").unwrap_or_else(|| {
        // During development with cargo run, look for po/ in the project root
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let project_root = std::path::Path::new(manifest_dir)
            .parent()
            .and_then(|p| p.parent());
        if let Some(root) = project_root {
            let po_dir = root.join("po").join("locale");
            if po_dir.exists() {
                // Leak the string to get a &'static str — only called once at startup
                return Box::leak(po_dir.to_string_lossy().into_owned().into_boxed_str());
            }
        }
        "/usr/share/locale"
    });

    gettextrs::bindtextdomain("northmail", localedir).expect("Unable to bind text domain");
    gettextrs::textdomain("northmail").expect("Unable to set text domain");
}

/// Translate a string.
#[inline]
pub fn tr(s: &str) -> String {
    gettext(s)
}

/// Translate a string with plural form.
#[inline]
pub fn ntr(singular: &str, plural: &str, n: u32) -> String {
    ngettext(singular, plural, n)
}
