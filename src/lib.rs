pub mod cli;
pub mod error;
pub mod init;
pub mod installer;
pub mod mcp;
pub mod output;
pub mod store;

pub use error::{Error, Result};
pub use mcp::McpServer;
pub use store::{Store, SyncReport};
