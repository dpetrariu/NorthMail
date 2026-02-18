//! NorthMail - A modern GNOME email client
//!
//! Built with GTK4/libadwaita for a native GNOME experience.

mod application;
mod idle_manager;
mod imap_pool;
mod window;
mod widgets;

use application::NorthMailApplication;
use gtk4::prelude::*;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn main() {
    // Initialize logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("northmail=debug".parse().unwrap()))
        .init();

    tracing::info!("Starting NorthMail");

    // Set GSettings schema directory for development builds.
    // This must happen before any GSettings are accessed.
    // For installed/Flatpak builds, the schema is in the system path and this is skipped.
    if std::env::var("GSETTINGS_SCHEMA_DIR").is_err() {
        // CARGO_MANIFEST_DIR is embedded at compile time — always points to the crate source dir
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        // crates/northmail-gtk -> project root
        let project_root = manifest_dir.parent().and_then(|p| p.parent());
        if let Some(root) = project_root {
            let schema_dir = root.join("data");
            if schema_dir.join("gschemas.compiled").exists() {
                std::env::set_var("GSETTINGS_SCHEMA_DIR", &schema_dir);
                tracing::debug!("Set GSETTINGS_SCHEMA_DIR to {:?}", schema_dir);
            }
        }
    }

    // Create and run the application
    let app = NorthMailApplication::new();
    std::process::exit(app.run().into());
}
