//! FlatCraft's MCP server. Agents start `flatcraft mcp` and drive the same
//! command registry the GUI uses, over stdin and stdout. Only protocol bytes
//! go to stdout; logs go to stderr.

pub mod server;
pub mod stdio;

use std::sync::{Arc, Mutex};

use flatcraft_engine::session::Session;

pub use server::{FlatcraftMcp, INSTRUCTIONS};

/// The session the GUI and the MCP server share. Never hold the lock across
/// an await.
pub type SharedSession = Arc<Mutex<Session>>;

/// Called after an agent call, so the GUI can repaint.
pub type OnChange = Arc<dyn Fn() + Send + Sync>;
