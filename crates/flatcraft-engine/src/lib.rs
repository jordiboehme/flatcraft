//! FlatCraft engine: session, preferences and the command registry that the
//! GUI, the MCP server and tests share.

pub mod commands;
pub mod gate;
pub mod job;
pub mod prefs;
pub mod registry;
pub mod session;

pub use job::JobResult;
pub use prefs::Preferences;
pub use registry::Command;
pub use session::{CommandError, DocState, Origin, PendingSend, Session};
