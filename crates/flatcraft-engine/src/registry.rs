//! Command registry. The GUI, the MCP server and tests call the same commands.

use crate::commands::{
    arrange, check, edit, export, job, notes, object, profiles, project, sources, tools,
};
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
    /// True if a command that does not mutate the document still changes
    /// session state: open documents, the active one, their selection,
    /// undo history, path or dirty flag. After a panic such a command is
    /// rolled back (see `Session::execute`).
    fn changes_session(&self) -> bool {
        false
    }
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
    &notes::DocumentNotes,
    &notes::DocumentAppendNotes,
    &notes::DocumentSetNotes,
    &sources::DocumentCheckSources,
    &check::DocumentCheck,
    &object::ObjectImport,
    &object::ObjectImportSvg,
    &object::ObjectRetrace,
    &sources::ObjectReload,
    &sources::ObjectRelink,
    &sources::ObjectUnlink,
    &object::ObjectSelect,
    &object::ObjectSetVisible,
    &object::ObjectSetCut,
    &object::ObjectRename,
    &object::ObjectDelete,
    &object::ObjectDuplicate,
    &object::ObjectCopies,
    &object::ObjectCircularCopies,
    &object::ObjectLink,
    &object::ObjectUnlinkCopies,
    &object::ObjectMove,
    &object::ObjectScale,
    &object::ObjectRotate,
    &object::ObjectMirror,
    &object::ObjectSetCutOptions,
    &arrange::ArrangeAlign,
    &arrange::ArrangeDistribute,
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
    &job::JobHistory,
    &job::JobTroubleshoot,
    &tools::ToolsTestPatterns,
    &tools::ToolsTestShape,
    &tools::DocumentPreview,
    &export::DocumentExport,
];

/// Every command, in a stable order.
pub fn all() -> &'static [&'static dyn Command] {
    ALL
}

/// The command with this id.
pub fn get(id: &str) -> Option<&'static dyn Command> {
    ALL.iter().copied().find(|c| c.id() == id)
}
