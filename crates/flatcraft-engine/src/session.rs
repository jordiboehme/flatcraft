//! Engine session: open documents, profiles, preferences and command dispatch.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

use flatcraft_core::document::{Document, JobSettings, ObjectId};
use flatcraft_core::undo::UndoStack;
use flatcraft_device::ProfileStore;
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

/// Result of the last job build. Placeholder until the job pipeline exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobResult;

/// Media used by new documents.
pub const DEFAULT_MEDIA: &str = "mat-12x12";

/// One open document with its selection and undo history.
#[derive(Debug, Clone)]
pub struct DocState {
    pub document: Document,
    pub selection: BTreeSet<ObjectId>,
    pub undo: UndoStack<Document>,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub last_job: Option<JobResult>,
}

impl DocState {
    pub fn new(document: Document) -> Self {
        Self {
            document,
            selection: BTreeSet::new(),
            undo: UndoStack::default(),
            path: None,
            dirty: false,
            last_job: None,
        }
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
