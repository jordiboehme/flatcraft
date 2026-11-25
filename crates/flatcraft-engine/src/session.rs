//! Engine session: open documents, profiles, preferences and command dispatch.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A command result that `project.open` would refuse to place.
const TOO_LARGE: &str = "That would make the object too large (over 1 km). Undo or scale it down.";

use flatcraft_core::document::{CutOptions, Document, JobSettings, Object, ObjectId};
use flatcraft_core::geometry::CutPath;
use flatcraft_core::transform::Transform;
use flatcraft_core::undo::UndoStack;
use flatcraft_device::gpgl::CheckedJob;
use flatcraft_device::{ByteLog, ProfileStore, TransportSpec};
use kurbo::Rect;
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
    /// Checked when the agent asked, so only safe bytes ever wait.
    pub job: CheckedJob,
    pub summary: JobSummary,
}

/// A send that waits for confirmation expires after this long.
pub const PENDING_TTL: Duration = Duration::from_secs(5 * 60);

/// Refusal of a confirmation that came too late.
pub const PENDING_EXPIRED: &str =
    "This cut request expired after 5 minutes. Ask the agent to send it again.";

impl PendingSend {
    /// True once [`PENDING_TTL`] has passed since the agent asked. `now`
    /// comes from the monotonic clock (`Instant`).
    pub fn expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.requested_at) >= PENDING_TTL
    }
}

/// Media new documents start on when the plotter lists it (see
/// [`Session::default_media`]).
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
    /// The job last built by `job.preview` or `job.send`.
    last_job: Option<Arc<JobResult>>,
    /// The job last sent from this document, kept apart from previews.
    last_sent: Option<Arc<JobResult>>,
    /// How many jobs were sent from this document.
    last_sent_rev: u64,
    /// Counts changes of `document`, so work done outside the session lock
    /// can tell whether the document changed meanwhile.
    revision: u64,
    /// `placed_bounds` of each object, for `document.state`.
    bounds_cache: HashMap<ObjectId, BoundsEntry>,
}

/// Everything `placed_bounds` of an object depends on.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BoundsKey {
    geometry: usize,
    transform: Transform,
    local_bounds: Rect,
    cut: CutOptions,
}

/// A cached bounding box. The geometry `Arc` is kept so its address cannot
/// be reused by other geometry while the key names it.
#[derive(Debug, Clone)]
struct BoundsEntry {
    key: BoundsKey,
    _geometry: Arc<Vec<CutPath>>,
    bounds: Option<Rect>,
}

impl BoundsKey {
    fn of(o: &Object) -> Self {
        Self {
            geometry: Arc::as_ptr(&o.geometry) as usize,
            transform: o.transform,
            local_bounds: o.local_bounds,
            cut: o.cut,
        }
    }
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
            last_sent: self.last_sent.clone(),
            last_sent_rev: self.last_sent_rev,
            revision: self.revision,
            bounds_cache: self.bounds_cache.clone(),
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
            last_sent: None,
            last_sent_rev: 0,
            revision: 0,
            bounds_cache: HashMap::new(),
        }
    }

    /// Counts changes of the document. Commands that change it, undo, redo
    /// and saving (links) raise it.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// `commands::placed_bounds` of every object, in document order. Only
    /// objects whose geometry, transform or cut options changed since the
    /// last call are computed again.
    pub fn placed_bounds_all(&mut self) -> Vec<Option<Rect>> {
        let cache = &mut self.bounds_cache;
        let objects = &self.document.objects;
        cache.retain(|id, _| objects.iter().any(|o| o.id == *id));
        objects
            .iter()
            .map(|o| {
                let key = BoundsKey::of(o);
                match cache.get(&o.id) {
                    Some(e) if e.key == key => e.bounds,
                    _ => {
                        let bounds = crate::commands::placed_bounds(o);
                        cache.insert(
                            o.id,
                            BoundsEntry {
                                key,
                                _geometry: o.geometry.clone(),
                                bounds,
                            },
                        );
                        bounds
                    }
                }
            })
            .collect()
    }

    /// Notes a change of the document.
    pub(crate) fn touch(&mut self) {
        self.revision += 1;
    }

    /// The job last built by `job.preview` or `job.send`.
    pub fn last_job(&self) -> Option<&JobResult> {
        self.last_job.as_deref()
    }

    /// The job last sent from this document, whatever was previewed since.
    pub fn last_sent(&self) -> Option<&Arc<JobResult>> {
        self.last_sent.as_ref()
    }

    /// How many jobs were sent from this document, so a viewer can spot a
    /// new send without comparing bytes.
    pub fn last_sent_rev(&self) -> u64 {
        self.last_sent_rev
    }

    /// A built (previewed) job.
    pub(crate) fn set_last_job(&mut self, job: JobResult) {
        self.last_job = Some(Arc::new(job));
    }

    /// A sent job: the last job and the last sent one.
    pub(crate) fn set_last_sent(&mut self, job: JobResult) {
        let job = Arc::new(job);
        self.last_job = Some(job.clone());
        self.last_sent = Some(job);
        self.last_sent_rev += 1;
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

/// Compares embedded sources, the expensive part (up to 50 MB of bytes;
/// shared bytes compare by pointer first).
fn same_source(a: &flatcraft_core::document::Source, b: &flatcraft_core::document::Source) -> bool {
    #[cfg(test)]
    tests::SOURCE_COMPARES.with(|n| n.set(n.get() + 1));
    a == b
}

/// True if two documents differ at most in the objects' source links.
/// The cheap fields of every object first, then geometry (by `Arc`, then by
/// value) and the embedded bytes last.
fn same_except_links(a: &Document, b: &Document) -> bool {
    let pairs = || a.objects.iter().zip(&b.objects);
    a.job == b.job
        && a.next_id == b.next_id
        && a.format_version == b.format_version
        && a.objects.len() == b.objects.len()
        && pairs().all(|(x, y)| {
            x.id == y.id
                && x.name == y.name
                && x.visible == y.visible
                && x.transform == y.transform
                && x.cut == y.cut
                && x.frame == y.frame
                && x.local_bounds == y.local_bounds
        })
        && pairs().all(|(x, y)| {
            std::sync::Arc::ptr_eq(&x.geometry, &y.geometry) || x.geometry == y.geometry
        })
        && pairs().all(|(x, y)| same_source(&x.source, &y.source))
}

/// True if a command left the document as it was.
fn same_document(a: &Document, b: &Document) -> bool {
    a.objects.len() == b.objects.len()
        && a.objects
            .iter()
            .zip(&b.objects)
            .all(|(x, y)| x.link == y.link)
        && same_except_links(a, b)
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
    /// Set by a mutating command that ended up changing nothing the user
    /// would undo (see `mark_no_user_change`). Reset before every command.
    no_user_change: bool,
    /// Set by [`Session::watch_prefs_file`] (headless MCP).
    prefs_watch: Option<PrefsWatch>,
}

/// How often a headless session looks at the preferences file.
pub const PREFS_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// What a headless session knows about the preferences file.
#[derive(Debug, Clone)]
struct PrefsWatch {
    last_check: Option<Instant>,
    stamp: Option<(Option<std::time::SystemTime>, u64)>,
}

impl Session {
    /// Starts with one empty document that uses the preference defaults and
    /// the default mat ([`Session::default_media`]).
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
            no_user_change: false,
            prefs_watch: None,
        };
        s.docs.push(s.new_doc_state());
        s
    }

    /// The mat new documents on `plotter_id` start on: [`DEFAULT_MEDIA`]
    /// when the plotter lists it (hidden mats are not listed), else the
    /// plotter's first mat. [`DEFAULT_MEDIA`] when the plotter is unknown
    /// or has no mat.
    pub fn default_media(&self, plotter_id: &str) -> String {
        let Ok(p) = self.profiles.plotter(plotter_id) else {
            return DEFAULT_MEDIA.to_string();
        };
        p.media
            .iter()
            .find(|m| m.id == DEFAULT_MEDIA)
            .or_else(|| p.media.first())
            .map_or_else(|| DEFAULT_MEDIA.to_string(), |m| m.id.clone())
    }

    /// A fresh, empty document with the default job settings.
    pub fn new_doc_state(&self) -> DocState {
        DocState::new(Document::new(JobSettings {
            plotter_id: self.prefs.default_plotter.clone(),
            material_id: self.prefs.default_material.clone(),
            media_id: self.default_media(&self.prefs.default_plotter),
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

    /// Re-read `preferences.toml` before commands when its modification time
    /// or size changed, checked at most once per [`PREFS_CHECK_INTERVAL`], so
    /// a change made by another FlatCraft process (a window or a headless
    /// `flatcraft mcp`) reaches this one. A window also calls
    /// [`Session::refresh_prefs`] every frame.
    pub fn watch_prefs_file(&mut self) {
        self.prefs_watch = Some(PrefsWatch {
            last_check: None,
            stamp: Preferences::stamp(&self.config_dir),
        });
    }

    /// Reloads the preferences when the file changed since the last look,
    /// at most once per [`PREFS_CHECK_INTERVAL`] at `now`. A file that cannot
    /// be read or parsed keeps the preferences as they are, except that
    /// agent cuts turn off (fail closed) until the file is fixed. Returns
    /// true if it reloaded them. Never writes the file.
    pub fn refresh_prefs(&mut self, now: Instant) -> bool {
        let Some(w) = &mut self.prefs_watch else {
            return false;
        };
        if w.last_check
            .is_some_and(|t| now.saturating_duration_since(t) < PREFS_CHECK_INTERVAL)
        {
            return false;
        }
        w.last_check = Some(now);
        let stamp = Preferences::stamp(&self.config_dir);
        if stamp == w.stamp {
            return false;
        }
        // Once per change of the file, also a broken one: fixing it changes
        // the time or size again.
        w.stamp = stamp;
        match Preferences::read(&self.config_dir) {
            Ok(prefs) => {
                self.prefs = prefs;
                true
            }
            Err(e) => {
                eprintln!(
                    "warning: {e}. Keeping the preferences read before, with agent cuts off until the file is fixed."
                );
                self.prefs.allow_agent_cuts = false;
                false
            }
        }
    }

    /// Changes the preferences and stores them: re-reads the file, applies
    /// `change` to the fresh copy and writes it (see
    /// [`Preferences::update`]), so a change another FlatCraft process
    /// stored meanwhile is kept. The session keeps the new preferences even
    /// when writing failed. For the GUI; no registry command calls this.
    pub fn update_prefs(&mut self, change: impl FnOnce(&mut Preferences)) -> std::io::Result<()> {
        let (fresh, written) = Preferences::update(&self.config_dir, &self.prefs, change);
        self.prefs = fresh;
        if let Some(w) = &mut self.prefs_watch {
            // Our own write is no news.
            w.stamp = Preferences::stamp(&self.config_dir);
        }
        written
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
        job: CheckedJob,
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

    /// Drops the pending send if it is older than [`PENDING_TTL`] at
    /// `now`. Returns true if it dropped one. GUI only, not a command.
    pub fn expire_pending(&mut self, now: Instant) -> bool {
        if self.pending_send.as_ref().is_some_and(|p| p.expired(now)) {
            self.pending_send = None;
            return true;
        }
        false
    }

    /// The person said no: drops the pending send with this id. Returns
    /// false (and keeps it) for any other id. GUI only, not a command.
    pub fn deny_pending(&mut self, id: u64) -> bool {
        if self.pending_send.as_ref().is_some_and(|p| p.id == id) {
            self.pending_send = None;
            return true;
        }
        false
    }

    /// Sends the pending job after the person confirmed it in the GUI. It
    /// runs as `Origin::User` and sends exactly the snapshot taken when the
    /// agent asked: the same bytes through the same transport. The document
    /// is not looked at again. This is not a registry command, so agents
    /// cannot reach it. A wrong id leaves the pending send in place.
    /// A request older than [`PENDING_TTL`] at `now` is dropped and
    /// refused. `now` comes from the caller's monotonic clock, the same one
    /// it passes to [`Session::expire_pending`], so both agree.
    pub fn confirm_pending(
        &mut self,
        id: u64,
        now: Instant,
    ) -> Result<serde_json::Value, CommandError> {
        self.prune_pending();
        if self.pending_send.as_ref().is_some_and(|p| p.id == id) && self.expire_pending(now) {
            return Err(CommandError::Refused(PENDING_EXPIRED.into()));
        }
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
        self.refresh_prefs(Instant::now());
        let command =
            registry::get(id).ok_or_else(|| CommandError::UnknownCommand(id.to_string()))?;
        let result = self.run_command(command, params, origin);
        self.prune_pending();
        result
    }

    /// Called by a mutating command whose run changed nothing the user would
    /// undo (`object.reload` when no file changed). A command that leaves the
    /// document exactly as it was needs no call: `run_command` notices that
    /// itself and records nothing. Small bookkeeping it
    /// did, like a refreshed modification time, stays in the document, but
    /// no undo step is recorded and the document is not marked as changed.
    /// Only link fields may differ from the state before the command; debug
    /// builds check that (`same_except_links`).
    pub(crate) fn mark_no_user_change(&mut self) {
        self.no_user_change = true;
    }

    fn run_command(
        &mut self,
        command: &dyn registry::Command,
        params: serde_json::Value,
        origin: Origin,
    ) -> Result<serde_json::Value, CommandError> {
        if !command.mutates_document() {
            return run_caught(command, self, params, origin)
                .unwrap_or_else(|()| Err(internal_error()));
        }
        let index = self.active;
        let doc_id = self.docs[index].id();
        let before = self.docs[index].document.clone();
        self.no_user_change = false;
        let caught = run_caught(command, self, params, origin);
        let no_user_change = std::mem::take(&mut self.no_user_change);
        let Ok(result) = caught else {
            // A panic: the command may have stopped half way. Put the
            // document back, if it is still open.
            if let Some(state) = self.docs.iter_mut().find(|d| d.id() == doc_id) {
                state.document = before;
                state.prune_selection();
            }
            return Err(internal_error());
        };
        let state = &mut self.docs[index];
        match result {
            Ok(value) if no_user_change => {
                debug_assert!(
                    same_except_links(&before, &state.document),
                    "a command marked as no user change changed the document"
                );
                state.touch();
                state.prune_selection();
                Ok(value)
            }
            Ok(value) if same_document(&before, &state.document) => {
                // Nothing changed: no undo step, not dirty.
                state.prune_selection();
                Ok(value)
            }
            Ok(_)
                if state
                    .document
                    .objects
                    .iter()
                    .any(|o| !o.placement_is_valid()) =>
            {
                // project.open would refuse this placement, so it could
                // never be saved and reopened.
                state.document = before;
                state.prune_selection();
                Err(CommandError::Refused(TOO_LARGE.into()))
            }
            Ok(value) => {
                state.touch();
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

/// What a command that panicked returns.
fn internal_error() -> CommandError {
    CommandError::Failed(
        "FlatCraft hit an internal error, so this command changed nothing. Please report it at https://github.com/jordiboehme/flatcraft/issues."
            .into(),
    )
}

/// Runs a command; `Err(())` if it panicked. The caller turns that into
/// [`internal_error`], so a bug in one command never takes down the window
/// or the MCP server.
fn run_caught(
    command: &dyn registry::Command,
    s: &mut Session,
    params: serde_json::Value,
    origin: Origin,
) -> Result<Result<serde_json::Value, CommandError>, ()> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        command.run(s, params, origin)
    }))
    .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatcraft_core::document::Source;
    use std::cell::Cell;

    thread_local! {
        pub(super) static SOURCE_COMPARES: Cell<usize> = const { Cell::new(0) };
    }

    /// A mutating command that changes the document and then panics.
    struct HalfDone;

    impl registry::Command for HalfDone {
        fn id(&self) -> &'static str {
            "test.half_done"
        }
        fn title(&self) -> &'static str {
            "Half done"
        }
        fn description(&self) -> &'static str {
            "Renames every object, then panics."
        }
        fn params_schema(&self) -> schemars::Schema {
            schemars::json_schema!({})
        }
        fn mutates_document(&self) -> bool {
            true
        }
        fn run(
            &self,
            s: &mut Session,
            _: serde_json::Value,
            _: Origin,
        ) -> Result<serde_json::Value, CommandError> {
            for o in &mut s.doc_mut().document.objects {
                o.name = "half".into();
            }
            panic!("a bug in the middle of a command");
        }
    }

    #[test]
    fn a_panicking_command_leaves_the_document_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let profiles = flatcraft_device::ProfileStore::load(dir.path()).unwrap();
        let mut s = Session::new(
            profiles,
            Preferences::default(),
            false,
            dir.path().to_path_buf(),
        );
        s.doc_mut().document.add(
            "whole".into(),
            Source::Svg {
                name: "whole.svg".into(),
                bytes: Vec::new().into(),
            },
            Vec::new(),
        );
        let e = s
            .run_command(&HalfDone, serde_json::json!({}), Origin::Agent)
            .unwrap_err();
        assert!(matches!(e, CommandError::Failed(_)), "{e}");
        assert!(e.to_string().contains("internal error"), "{e}");
        assert_eq!(s.doc().document.objects[0].name, "whole");
        assert!(!s.doc().dirty);
        assert!(!s.doc().undo.can_undo());
    }

    #[test]
    fn documents_differ_by_cheap_fields_before_any_bytes_are_compared() {
        let mut a = Document::new(JobSettings {
            plotter_id: "p".into(),
            material_id: "m".into(),
            media_id: "x".into(),
            speed: None,
            force: None,
            passes: None,
        });
        for name in ["one", "two"] {
            a.add(
                name.into(),
                Source::Svg {
                    name: name.into(),
                    bytes: vec![7; 1000].into(),
                },
                Vec::new(),
            );
        }
        let mut b = a.clone();
        b.objects[1].name = "renamed".into();
        SOURCE_COMPARES.with(|n| n.set(0));
        assert!(!same_document(&a, &b));
        assert_eq!(SOURCE_COMPARES.with(Cell::get), 0, "a cheap field decided");
        SOURCE_COMPARES.with(|n| n.set(0));
        assert!(same_document(&a, &a.clone()));
        assert_eq!(SOURCE_COMPARES.with(Cell::get), 2);
    }
}
