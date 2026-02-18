pub mod client;
pub mod error;
pub mod types;

pub use client::GraphMailClient;
pub use error::{GraphError, GraphResult};
pub use types::*;
