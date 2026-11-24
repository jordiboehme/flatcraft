//! Command registry. The GUI, the MCP server and tests call the same commands.

use crate::commands::{arrange, edit, job, object, profiles, project};
use crate::session::{CommandError, Origin, Session};

/// One registry command.
pub trait Command: Send + Sync {
    /// Dotted id, like `object.move`.
    fn id(&self) -> &'static str;
    /// Short title for menus.
    fn title(&self) -> &'static str;
    /// What the command does and returns, written for agents.
    fn description(&self) -> &'static str;
    /// JSON schema of the params object.
    fn params_schema(&self) -> schemars::Schema;
    /// True if the command changes the active document (one undo step).
    fn mutates_document(&self) -> bool;
    fn run(
        &self,
        s: &mut Session,
        params: serde_json::Value,
        origin: Origin,
    ) -> Result<serde_json::Value, CommandError>;
}

static ALL: &[&dyn Command] = &[
    &project::DocumentState,
    &project::DocumentNew,
    &project::DocumentSwitch,
    &project::DocumentClose,
    &project::ProjectSave,
    &project::ProjectOpen,
    &object::ObjectImport,
    &object::ObjectImportSvg,
    &object::ObjectRetrace,
    &object::ObjectSelect,
    &object::ObjectSetVisible,
    &object::ObjectDelete,
    &object::ObjectDuplicate,
    &object::ObjectMove,
    &object::ObjectScale,
    &object::ObjectRotate,
    &object::ObjectMirror,
    &arrange::ArrangeAlign,
    &arrange::ArrangeAuto,
    &edit::EditUndo,
    &edit::EditRedo,
    &profiles::ProfilesList,
    &profiles::ProfilesGet,
    &profiles::JobSetPlotter,
    &profiles::JobSetMaterial,
    &profiles::JobSetMedia,
    &profiles::JobSetOverrides,
    &job::JobPreview,
    &job::JobSend,
];

/// Every command, in a stable order.
pub fn all() -> &'static [&'static dyn Command] {
    ALL
}

/// The command with this id.
pub fn get(id: &str) -> Option<&'static dyn Command> {
    ALL.iter().copied().find(|c| c.id() == id)
}
