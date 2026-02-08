//! Core business logic for NorthMail
//!
//! Provides the sync engine, storage, and data models.

mod account;
mod database;
mod error;
mod sync;

pub use account::{Account, AccountConfig};
pub use database::Database;
pub use error::{CoreError, CoreResult};
pub use sync::{SyncCommand, SyncEngine, SyncEvent};

/// Re-export models for convenience
pub mod models {
    pub use crate::database::{DbFolder, DbMessage};
}
