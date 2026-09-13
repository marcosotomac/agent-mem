pub mod cli;
pub mod error;
pub mod init;
pub mod installer;
pub mod mcp;
pub mod output;
pub mod store;
#[cfg(feature = "tui")]
pub mod tui;

pub use error::{Error, Result};
pub use mcp::McpServer;
pub use store::{RuleRecord, Store, SyncReport};
