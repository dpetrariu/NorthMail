//! NorthMail - A modern GNOME email client
//!
//! Built with GTK4/libadwaita for a native GNOME experience.

mod application;
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

    // Create and run the application
    let app = NorthMailApplication::new();
    std::process::exit(app.run().into());
}
