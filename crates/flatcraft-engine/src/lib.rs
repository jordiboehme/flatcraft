//! FlatCraft engine: session, preferences and the command registry that the
//! GUI, the MCP server and tests share.

pub mod commands;
pub mod cutgeom;
pub mod gate;
pub mod job;
pub mod patterns;
pub mod placement;
pub mod prefs;
pub mod preview;
pub mod registry;
pub mod session;
pub mod simulate;

pub use job::JobResult;
pub use prefs::Preferences;
pub use registry::Command;
pub use session::{CommandError, DocState, JobSummary, Origin, PendingSend, Session};
