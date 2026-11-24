//! Engine session: open documents, profiles, preferences and command dispatch.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use flatcraft_core::document::{Document, JobSettings, ObjectId};
use flatcraft_core::undo::UndoStack;
use flatcraft_device::gpgl::EmittedJob;
use flatcraft_device::{ByteLog, ProfileStore, TransportSpec};
use thiserror::Error;

use crate::prefs::Preferences;
use crate::registry;

/// Who started a command. Agents get a few extra limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    User,
    Agent,
}

/// Errors of a registry command. The texts are read by people and agents.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CommandError {
    #[error("Unknown command '{0}'. Ask for the command list to see the available commands.")]
    UnknownCommand(String),
    #[error("Invalid parameters: {0}")]
    InvalidParams(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Refused: {0}")]
    Refused(String),
    #[error("Failed: {0}")]
    Failed(String),
}

pub use crate::job::JobResult;

/// What a job will do, for the confirmation dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct JobSummary {
    pub toolpaths: usize,
    pub cut_mm: f64,
    pub travel_mm: f64,
    pub warnings: Vec<String>,
    pub speed: u8,
    pub force: u8,
    pub passes: u32,
}

/// An agent's USB send that waits for the person at the FlatCraft window.
/// It is a snapshot: confirming sends exactly these bytes through this
/// transport, whatever happened to the document since.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingSend {
    pub id: u64,
    /// The sanitised job name.
    pub name: String,
    pub requested_at: Instant,
    /// `DocState::id` of the document the job was built from.
    pub doc: u64,
    pub plotter_id: String,
    pub transport: TransportSpec,
    pub job: EmittedJob,
    pub summary: JobSummary,
}

/// Media used by new documents.
pub const DEFAULT_MEDIA: &str = "mat-12x12";

/// One open document with its selection and undo history.
#[derive(Debug)]
pub struct DocState {
    /// Stable id, unique for the life of the process. A replaced document
    /// and a clone get a new id.
    id: u64,
    pub document: Document,
    pub selection: BTreeSet<ObjectId>,
    pub undo: UndoStack<Document>,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub last_job: Option<JobResult>,
}

fn next_doc_id() -> u64 {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// A clone is a new document: it gets its own id, so it never keeps a
/// pending send of the original alive.
impl Clone for DocState {
    fn clone(&self) -> Self {
        Self {
            id: next_doc_id(),
            document: self.document.clone(),
            selection: self.selection.clone(),
            undo: self.undo.clone(),
            path: self.path.clone(),
            dirty: self.dirty,
            last_job: self.last_job.clone(),
        }
    }
}

impl DocState {
    /// Stable id of this open document.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn new(document: Document) -> Self {
        Self {
            id: next_doc_id(),
            document,
            selection: BTreeSet::new(),
            undo: UndoStack::default(),
            path: None,
            dirty: false,
            last_job: None,
        }
    }

    /// True for an empty document nobody has touched: no objects, no
    /// unsaved changes, no path and nothing to undo or redo. Opening a
    /// project or starting a new one may reuse it.
    pub fn is_pristine(&self) -> bool {
        self.document.objects.is_empty()
            && !self.dirty
            && self.path.is_none()
            && !self.undo.can_undo()
            && !self.undo.can_redo()
    }

    /// Drops selected ids that no longer exist in the document.
    pub fn prune_selection(&mut self) {
        let doc = &self.document;
        self.selection.retain(|id| doc.get(*id).is_some());
    }
}

/// The engine state shared by the GUI, the MCP server and tests.
#[derive(Debug)]
pub struct Session {
    pub docs: Vec<DocState>,
    pub active: usize,
    pub profiles: ProfileStore,
    pub prefs: Preferences,
    pub has_ui: bool,
    /// Directory of the preferences and user presets. Agents may not write here.
    pub config_dir: PathBuf,
    pub last_agent_activity: Option<Instant>,
    /// Bytes sent to and read from plotters in this session.
    pub byte_log: ByteLog,
    /// A send that waits for confirmation in the GUI (see `confirm_pending`).
    pub(crate) pending_send: Option<PendingSend>,
    next_pending_id: u64,
}

impl Session {
    /// Starts with one empty document that uses the preference defaults and
    /// the media `mat-12x12`.
    pub fn new(
        profiles: ProfileStore,
        prefs: Preferences,
        has_ui: bool,
        config_dir: PathBuf,
    ) -> Self {
        let mut s = Self {
            docs: Vec::new(),
            active: 0,
            profiles,
            prefs,
            has_ui,
            config_dir,
            last_agent_activity: None,
            byte_log: ByteLog::new(),
            pending_send: None,
            next_pending_id: 1,
        };
        s.docs.push(s.new_doc_state());
        s
    }

    /// A fresh, empty document with the default job settings.
    pub fn new_doc_state(&self) -> DocState {
        DocState::new(Document::new(JobSettings {
            plotter_id: self.prefs.default_plotter.clone(),
            material_id: self.prefs.default_material.clone(),
            media_id: DEFAULT_MEDIA.to_string(),
            speed: None,
            force: None,
            passes: None,
        }))
    }

    pub fn doc(&self) -> &DocState {
        &self.docs[self.active]
    }

    pub fn doc_mut(&mut self) -> &mut DocState {
        &mut self.docs[self.active]
    }

    /// Index of the open document with this stable id.
    pub fn doc_index(&self, doc_id: u64) -> Result<usize, CommandError> {
        self.docs
            .iter()
            .position(|d| d.id() == doc_id)
            .ok_or_else(|| CommandError::NotFound("That document is no longer open.".into()))
    }

    /// The send that waits for confirmation in the GUI, if any.
    pub fn pending_send(&self) -> Option<&PendingSend> {
        self.pending_send.as_ref()
    }

    /// Stores a send that waits for confirmation in the GUI and returns its
    /// id. It replaces an older pending send.
    pub(crate) fn request_confirmation(
        &mut self,
        name: String,
        plotter_id: String,
        transport: TransportSpec,
        job: EmittedJob,
        summary: JobSummary,
    ) -> u64 {
        let id = self.next_pending_id;
        self.next_pending_id += 1;
        self.pending_send = Some(PendingSend {
            id,
            name,
            requested_at: Instant::now(),
            doc: self.doc().id(),
            plotter_id,
            transport,
            job,
            summary,
        });
        id
    }

    /// Drops the pending send when its document is no longer open (closed,
    /// or replaced by another document in its slot).
    fn prune_pending(&mut self) {
        if let Some(p) = &self.pending_send
            && !self.docs.iter().any(|d| d.id() == p.doc)
        {
            self.pending_send = None;
        }
    }

    /// Sends the pending job after the person confirmed it in the GUI. It
    /// runs as `Origin::User` and sends exactly the snapshot taken when the
    /// agent asked: the same bytes through the same transport. The document
    /// is not looked at again. This is not a registry command, so agents
    /// cannot reach it. A wrong id leaves the pending send in place.
    pub fn confirm_pending(&mut self, id: u64) -> Result<serde_json::Value, CommandError> {
        self.prune_pending();
        let pending = match &self.pending_send {
            Some(p) if p.id == id => p,
            _ => {
                return Err(CommandError::NotFound(format!(
                    "no send with id {id} is waiting for confirmation"
                )));
            }
        };
        if !matches!(pending.transport, TransportSpec::Usb { .. }) {
            return Err(CommandError::Refused(
                "only USB sends wait for confirmation".into(),
            ));
        }
        let pending = self.pending_send.take().expect("checked above");
        let report = crate::job::deliver_built(
            self,
            &pending.transport,
            &pending.job,
            &pending.name,
            Origin::User,
        )?;
        Ok(crate::job::sent_json(&report, &pending.summary))
    }

    /// Runs a registry command. Commands that change the document record an
    /// undo step and mark it dirty. A failed command leaves the document as it
    /// was and records nothing.
    pub fn execute(
        &mut self,
        id: &str,
        params: serde_json::Value,
        origin: Origin,
    ) -> Result<serde_json::Value, CommandError> {
        if origin == Origin::Agent {
            self.last_agent_activity = Some(Instant::now());
        }
        let command =
            registry::get(id).ok_or_else(|| CommandError::UnknownCommand(id.to_string()))?;
        let result = self.run_command(command, params, origin);
        self.prune_pending();
        result
    }

    fn run_command(
        &mut self,
        command: &dyn registry::Command,
        params: serde_json::Value,
        origin: Origin,
    ) -> Result<serde_json::Value, CommandError> {
        if !command.mutates_document() {
            return command.run(self, params, origin);
        }
        let index = self.active;
        let before = self.docs[index].document.clone();
        let result = command.run(self, params, origin);
        let state = &mut self.docs[index];
        match result {
            Ok(value) => {
                state.undo.record(before);
                state.dirty = true;
                state.prune_selection();
                Ok(value)
            }
            Err(e) => {
                state.document = before;
                state.prune_selection();
                Err(e)
            }
        }
    }
}
