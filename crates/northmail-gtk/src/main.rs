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

    // Set GSettings schema directory for development builds
    // This must happen before any GSettings are accessed
    if std::env::var("GSETTINGS_SCHEMA_DIR").is_err() {
        if let Ok(exe) = std::env::current_exe() {
            // Check if running from target/debug or target/release
            if let Some(target_dir) = exe.parent() {
                if let Some(project_root) = target_dir.parent().and_then(|p| p.parent()) {
                    let schema_dir = project_root.join("data");
                    if schema_dir.join("gschemas.compiled").exists() {
                        std::env::set_var("GSETTINGS_SCHEMA_DIR", &schema_dir);
                        tracing::debug!("Set GSETTINGS_SCHEMA_DIR to {:?}", schema_dir);
                    }
                }
            }
        }
    }

    // Create and run the application
    let app = NorthMailApplication::new();
    std::process::exit(app.run().into());
}
