//! Engine session: open documents, profiles, preferences and command dispatch.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A command result that `project.open` would refuse to place.
const TOO_LARGE: &str = "That would put an object more than 1 km from the mat, or make it over 1 km in size. Undo it, or move or scale it back.";

/// A command result whose linked copies no longer match (a bug).
const LINKS_DRIFTED: &str = "That would make linked copies differ from each other, so nothing was changed. This is a bug, please report it.";

use flatcraft_core::document::{CutOptions, Document, JobSettings, Object, ObjectId};
use flatcraft_core::geometry::CutPath;
use flatcraft_core::transform::Transform;
use flatcraft_core::undo::UndoStack;
use flatcraft_device::gpgl::CheckedJob;
use flatcraft_device::{ByteLog, ProfileStore, TransportSpec};
use kurbo::Rect;
use thiserror::Error;

use crate::cutgeom::CutGeometry;
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
    /// The highest pass number: a ring of chained laps counts each lap.
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
    /// The cut log entry, made when the agent asked; a confirmed send logs
    /// it with the time and the target of the send.
    pub log: crate::cutlog::CutEntry,
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

/// What the cached cut log was read at: this session's revision, and the
/// file's length and modification time.
type CutLogKey = (u64, Option<u64>, Option<std::time::SystemTime>);

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
    /// The burst of coalescing moves whose undo step is the newest one.
    burst: Option<Burst>,
    /// The cut geometry last built for this document, with its key (see
    /// [`DocState::cut_geometry`]). Behind a lock so a reader that holds
    /// the session only for reading can fill it.
    cut_cache: Mutex<Option<(CacheKey, Arc<CutGeometry>)>>,
}

/// What a document's cached cut geometry was built for: the document's
/// revision, the profiles' revision, the plotter and material, and the
/// flatten tolerance and minimum feature width taken from them.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheKey {
    pub doc_revision: u64,
    pub profiles_revision: u64,
    pub plotter_id: String,
    pub material_id: String,
    pub tol: f64,
    pub min_width_mm: f64,
}

/// A run of `object.move` calls with one `coalesce_key` that share one
/// undo step (see [`Session::coalesce_with`]).
#[derive(Debug, Clone)]
struct Burst {
    key: String,
    ids: Vec<ObjectId>,
    /// The document revision after the burst's last move. Any other
    /// command, undo or redo raises it and ends the burst. A save ends it
    /// through [`DocState::end_burst`], since it leaves the revision alone.
    revision: u64,
    /// `dirty` before the burst's first move, for a burst that ends where
    /// it started.
    dirty_before: bool,
    /// The redo steps the burst's first move cleared, given back when the
    /// burst ends where it started.
    redo_before: Vec<Document>,
}

/// Everything `placed_bounds` of an object depends on.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BoundsKey {
    geometry: usize,
    transform: Transform,
    local_bounds: Rect,
    cut: CutOptions,
    /// The knife's narrowest feature, as bits (slivers do not count).
    min_width: u64,
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
    fn of(o: &Object, min_width: f64) -> Self {
        Self {
            geometry: Arc::as_ptr(&o.geometry) as usize,
            transform: o.transform,
            local_bounds: o.local_bounds,
            cut: o.cut,
            min_width: min_width.to_bits(),
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
            burst: self.burst.clone(),
            cut_cache: Mutex::new(self.cut_cache_entry()),
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
            burst: None,
            cut_cache: Mutex::default(),
        }
    }

    /// Replaces the document (undo, redo, a restored state) and notes the
    /// change, so everything keyed on the revision is built again.
    pub fn set_document(&mut self, document: Document) {
        self.document = document;
        self.touch();
    }

    /// Puts back the document a failed command started from. The revision
    /// stays: the document is the one it names again. The cut geometry
    /// cache cannot go stale by this, since it is checked object by object
    /// (see [`DocState::cut_geometry`]).
    pub(crate) fn restore_document(&mut self, document: Document) {
        self.document = document;
    }

    fn cut_cache_entry(&self) -> Option<(CacheKey, Arc<CutGeometry>)> {
        self.cut_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The cut geometry for `key`: the cached one when it was built for
    /// this key and still fits the document's objects, else `build`, which
    /// gets the cached one to take unchanged objects from (C2: the cache is
    /// keyed, never cleared by a flag, so undo, redo and profile edits
    /// cannot leave it stale). The object check also covers a command that
    /// reads the geometry half way, before its change raised the revision.
    /// `objects` are this document's objects as a job input reads them.
    pub fn cut_geometry(
        &self,
        key: CacheKey,
        objects: &[crate::job::JobObject],
        build: impl FnOnce(Option<&Arc<CutGeometry>>) -> Arc<CutGeometry>,
    ) -> Arc<CutGeometry> {
        let mut cache = self
            .cut_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((k, g)) = cache.as_ref()
            && *k == key
            && g.is_for(objects, key.tol, key.min_width_mm)
        {
            return g.clone();
        }
        let built = build(cache.as_ref().map(|(_, g)| g));
        *cache = Some((key, built.clone()));
        built
    }

    /// The cached cut geometry, whatever it was built for: a seed for a
    /// build outside the session lock (see `JobInput::with_seed`).
    pub fn cached_cut_geometry(&self) -> Option<Arc<CutGeometry>> {
        self.cut_cache_entry().map(|(_, g)| g)
    }

    /// The key of the cached cut geometry, if any.
    pub fn cut_cache_key(&self) -> Option<CacheKey> {
        self.cut_cache_entry().map(|(k, _)| k)
    }

    /// Stores cut geometry built outside the session lock, but only if the
    /// document is still at the revision of `key`: work done on an older
    /// state is never stored.
    pub fn store_cut_geometry(&self, key: CacheKey, geometry: Arc<CutGeometry>) -> bool {
        if key.doc_revision != self.revision {
            return false;
        }
        *self
            .cut_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((key, geometry));
        true
    }

    /// Counts changes of the document. Commands that change it, undo, redo
    /// and saving (links) raise it.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// `commands::placed_bounds` of every object, in document order, for a
    /// knife whose narrowest feature is `min_width`. Only objects whose
    /// geometry, transform or cut options (or the width) changed since the
    /// last call are computed again.
    pub fn placed_bounds_all(&mut self, min_width: f64) -> Vec<Option<Rect>> {
        let cache = &mut self.bounds_cache;
        let objects = &self.document.objects;
        cache.retain(|id, _| objects.iter().any(|o| o.id == *id));
        objects
            .iter()
            .map(|o| {
                let key = BoundsKey::of(o, min_width);
                match cache.get(&o.id) {
                    Some(e) if e.key == key => e.bounds,
                    _ => {
                        let bounds = crate::commands::placed_bounds(o, min_width);
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

    /// Ends the burst of coalescing moves, so the next move records a step
    /// of its own. A save calls it: the saved file is the new reference
    /// for `dirty`, so the burst can no longer give back its old flag.
    pub(crate) fn end_burst(&mut self) {
        self.burst = None;
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
        && a.description == b.description
        && a.notes == b.notes
        && a.next_id == b.next_id
        && a.format_version == b.format_version
        && a.objects.len() == b.objects.len()
        && a.groups == b.groups
        && a.ignored_pairs == b.ignored_pairs
        && a.ignored_groups == b.ignored_groups
        && pairs().all(|(x, y)| {
            x.id == y.id
                && x.parent == y.parent
                && x.name == y.name
                && x.visible == y.visible
                && x.cut_enabled == y.cut_enabled
                && x.transform == y.transform
                && x.cut == y.cut
                && x.keep == y.keep
                && x.frame == y.frame
                && x.local_bounds == y.local_bounds
                && x.link_group == y.link_group
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

/// What a command that changes session state may change in one open
/// document, kept to undo a panic (see [`Session::roll_back`]).
struct DocSnapshot {
    id: u64,
    document: Document,
    selection: BTreeSet<ObjectId>,
    undo: UndoStack<Document>,
    path: Option<PathBuf>,
    dirty: bool,
    /// Only for a document the command removed: the rest of its state.
    revision: u64,
    last_job: Option<Arc<JobResult>>,
    last_sent: Option<Arc<JobResult>>,
    last_sent_rev: u64,
}

/// The open documents, the active one and the send that waits for
/// confirmation, before a command.
pub(crate) struct SessionSnapshot {
    docs: Vec<DocSnapshot>,
    active: usize,
    pending_send: Option<PendingSend>,
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
    /// Set by a move that may join the undo step of the burst before it
    /// (see `coalesce_with`). Reset before every command.
    coalesce: Option<(String, Vec<ObjectId>)>,
    /// Set by [`Session::watch_prefs_file`] (headless MCP).
    prefs_watch: Option<PrefsWatch>,
    /// The last warning about a broken or unreadable preferences file that
    /// nobody showed yet (see [`Session::take_prefs_warning`]).
    prefs_warning: Option<String>,
    /// One save at a time (see [`Session::save_gate`]).
    save_gate: Arc<Mutex<()>>,
    /// Counts the lines this session added to the cut log.
    cut_log_rev: u64,
    /// The cut log as read at a revision and file stamp (see
    /// [`Session::cut_log`]).
    cut_log_cache: Mutex<Option<(CutLogKey, Arc<crate::cutlog::CutLog>)>>,
    /// Tests only: runs in the write step of every save, without the
    /// session lock, with the file the save replaces.
    #[doc(hidden)]
    pub save_write_hook: Option<SaveWriteHook>,
}

/// A test hook for the write step of a save (see
/// [`Session::save_write_hook`]).
#[derive(Clone)]
pub struct SaveWriteHook(pub Arc<dyn Fn(&std::path::Path) + Send + Sync>);

impl std::fmt::Debug for SaveWriteHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SaveWriteHook")
    }
}

/// How often a headless session looks at the preferences file.
pub const PREFS_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// Modification time and size of the preferences file (see
/// [`Preferences::stamp`]).
type PrefsStamp = Option<(Option<std::time::SystemTime>, u64)>;

/// What a headless session knows about the preferences file.
#[derive(Debug, Clone)]
struct PrefsWatch {
    last_check: Option<Instant>,
    /// The file as it was at the last successful read.
    stamp: PrefsStamp,
    /// The file as it was when a failed read was last reported, so a file
    /// that stays broken is reported once, not every second.
    warned: Option<PrefsStamp>,
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
            coalesce: None,
            prefs_watch: None,
            prefs_warning: None,
            save_gate: Arc::default(),
            cut_log_rev: 0,
            cut_log_cache: Mutex::default(),
            save_write_hook: None,
        };
        s.docs.push(s.new_doc_state());
        s
    }

    /// The gate every save of this session passes, so two saves never
    /// write at the same time and one finishes before the next begins
    /// (`commands::project::save_unlocked`). Take it before the session
    /// lock, never while holding it.
    pub fn save_gate(&self) -> Arc<Mutex<()>> {
        Arc::clone(&self.save_gate)
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
            strategy: self.prefs.default_strategy,
            weed: Default::default(),
        }))
    }

    pub fn doc(&self) -> &DocState {
        &self.docs[self.active]
    }

    pub fn doc_mut(&mut self) -> &mut DocState {
        &mut self.docs[self.active]
    }

    /// The cut geometry of the active document for its plotter and
    /// material, from the document's cache when it still fits. The job
    /// build, the preview, the export, the report and the window all read
    /// this one copy.
    pub fn cut_geometry(&self) -> Result<Arc<CutGeometry>, CommandError> {
        Ok(crate::job::JobInput::of(self)?.cut_geometry())
    }

    /// The key of the cut geometry of `input`, taken from the open
    /// document `st`.
    fn cut_key(&self, st: &DocState, input: &crate::job::JobInput) -> CacheKey {
        CacheKey {
            doc_revision: st.revision(),
            profiles_revision: self.profiles.revision(),
            plotter_id: input.plotter.id.clone(),
            material_id: input.material.id.clone(),
            tol: input.tolerance_mm(),
            min_width_mm: input.min_width_mm(),
        }
    }

    /// The cut geometry of `input`, an input of the active document, from
    /// and into the document's cache.
    pub(crate) fn cut_geometry_of(&self, input: &crate::job::JobInput) -> Arc<CutGeometry> {
        let st = self.doc();
        let key = self.cut_key(st, input);
        let (tol, min_width) = (key.tol, key.min_width_mm);
        st.cut_geometry(key, &input.objects, |prev| {
            crate::cutgeom::objects_cut_geometry(&input.objects, tol, min_width, prev)
        })
    }

    /// Stores the cut geometry a window built outside the session lock
    /// from `input`, an input of the open document `doc_id` taken at
    /// `doc_revision`. Nothing is stored when the document changed since,
    /// is closed, or the geometry was not built. Returns true if stored.
    pub fn offer_cut_geometry(
        &self,
        doc_id: u64,
        doc_revision: u64,
        input: &crate::job::JobInput,
    ) -> bool {
        let (Ok(index), Some(geometry)) = (self.doc_index(doc_id), input.built_cut_geometry())
        else {
            return false;
        };
        let st = &self.docs[index];
        if st.revision() != doc_revision
            || st
                .cached_cut_geometry()
                .is_some_and(|g| Arc::ptr_eq(&g, &geometry))
        {
            return false;
        }
        st.store_cut_geometry(self.cut_key(st, input), geometry)
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
            warned: None,
        });
    }

    /// Keeps a warning about the preferences file for the window to show
    /// (`main` hands over the one from loading at start). A newer warning
    /// replaces an older one nobody showed yet.
    pub fn note_prefs_warning(&mut self, text: String) {
        self.prefs_warning = Some(text);
    }

    /// The warning about a broken or unreadable preferences file that was
    /// not shown yet, if any. The window shows it as a toast.
    pub fn take_prefs_warning(&mut self) -> Option<String> {
        self.prefs_warning.take()
    }

    /// Reloads the preferences when the file changed since the last look,
    /// at most once per [`PREFS_CHECK_INTERVAL`] at `now`. A file that cannot
    /// be read or parsed keeps the preferences as they are, except that
    /// agent cuts turn off (fail closed) until the file is fixed; it is
    /// read again at the next check, so a passing error heals by itself,
    /// and reported once per change of the file (stderr and
    /// [`Session::take_prefs_warning`]). Returns true if it reloaded them.
    /// Never writes the file.
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
        // After a failed read the file is read again even when it looks
        // like the last good one: an exact restore (`cp -p`) has the same
        // time and size.
        if stamp == w.stamp && w.warned.is_none() {
            return false;
        }
        // The stamp is taken before the read: a change in between only
        // means one more read at the next check, never a missed one.
        match Preferences::read(&self.config_dir) {
            Ok(prefs) => {
                w.stamp = stamp;
                w.warned = None;
                self.prefs = prefs;
                true
            }
            Err(e) => {
                // The old stamp stays, so the next check reads again.
                self.prefs.allow_agent_cuts = false;
                if w.warned != Some(stamp) {
                    w.warned = Some(stamp);
                    self.prefs_warning = Some(crate::prefs::warning(
                        &e,
                        "Keeping the preferences read before, with agent cuts off until the file is fixed.",
                    ));
                }
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
        // A broken file was kept as a copy: the window says so.
        written.map(|warning| {
            if let Some(w) = warning {
                self.note_prefs_warning(w);
            }
        })
    }

    /// The cut log in the config dir. Read again after this session adds
    /// to it and when the file's length or modification time changed (a
    /// send from another FlatCraft), so the window can ask every frame.
    pub fn cut_log(&self) -> Arc<crate::cutlog::CutLog> {
        let meta = std::fs::metadata(crate::cutlog::path(&self.config_dir)).ok();
        let key: CutLogKey = (
            self.cut_log_rev,
            meta.as_ref().map(std::fs::Metadata::len),
            meta.and_then(|m| m.modified().ok()),
        );
        let mut cache = self
            .cut_log_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((k, log)) = cache.as_ref()
            && *k == key
        {
            return log.clone();
        }
        let log = Arc::new(crate::cutlog::read(&self.config_dir));
        *cache = Some((key, log.clone()));
        log
    }

    /// Appends a completed send to the cut log.
    pub(crate) fn log_cut(&mut self, entry: &crate::cutlog::CutEntry) -> std::io::Result<()> {
        let result = crate::cutlog::append(&self.config_dir, entry);
        self.cut_log_rev += 1;
        result
    }

    /// The person said how the cut `entry` went: appends it to the cut
    /// log. GUI only, not a registry command: agents read the log
    /// (`job.history`) but never write it. An entry that is not in the
    /// log is refused.
    pub fn record_outcome(
        &mut self,
        entry: &str,
        verdict: crate::cutlog::Verdict,
        note: &str,
    ) -> Result<(), String> {
        if self.cut_log().get(entry).is_none() {
            return Err(format!("the cut log has no cut {entry}"));
        }
        let outcome = crate::cutlog::Outcome {
            verdict,
            note: note.trim().to_string(),
            date: crate::cutlog::now(),
        };
        let result = crate::cutlog::append_outcome(&self.config_dir, entry, &outcome);
        self.cut_log_rev += 1;
        result.map_err(|e| format!("the cut log could not be written: {e}"))
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
        log: crate::cutlog::CutEntry,
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
            log,
        });
        id
    }

    /// Drops the pending send when its document is no longer open (closed,
    /// or replaced by another document in its slot).
    pub(crate) fn prune_pending(&mut self) {
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
        let mut value = crate::job::sent_json(&report, &pending.summary);
        crate::job::log_sent(self, pending.log, &pending.transport, &report, &mut value);
        Ok(value)
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

    /// Runs a registry command in the open document `doc_id`, active or
    /// not, and leaves the active document as it was. For a value typed
    /// into the window for one document and applied after another became
    /// active. Only for commands that change one document's objects
    /// (`object.move`, `object.scale`), never ones that open, close or
    /// switch documents.
    pub fn execute_in(
        &mut self,
        doc_id: u64,
        id: &str,
        params: serde_json::Value,
        origin: Origin,
    ) -> Result<serde_json::Value, CommandError> {
        let index = self.doc_index(doc_id)?;
        let active = std::mem::replace(&mut self.active, index);
        let result = self.execute(id, params, origin);
        self.active = active.min(self.docs.len().saturating_sub(1));
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

    /// Called by a move with a `coalesce_key`: if the newest undo step is a
    /// burst of moves with this key and these ids, and nothing changed the
    /// document since, this move joins that step instead of recording its
    /// own. A burst that ends where it started leaves no step at all.
    pub(crate) fn coalesce_with(&mut self, key: String, ids: Vec<ObjectId>) {
        self.coalesce = Some((key, ids));
    }

    fn run_command(
        &mut self,
        command: &dyn registry::Command,
        params: serde_json::Value,
        origin: Origin,
    ) -> Result<serde_json::Value, CommandError> {
        if !command.mutates_document() {
            let snapshot = command.changes_session().then(|| self.snapshot());
            let result = run_caught(command, self, params, origin);
            return match (result, snapshot) {
                // Linked copies in every open document still agree.
                (Ok(Ok(_)), Some(snapshot)) if !self.links_agree() => {
                    self.roll_back(snapshot);
                    Err(CommandError::Failed(LINKS_DRIFTED.into()))
                }
                (Ok(result), _) => result,
                // A panic: put the open documents back as they were.
                (Err(()), snapshot) => {
                    if let Some(snapshot) = snapshot {
                        self.roll_back(snapshot);
                    }
                    Err(internal_error(command.id()))
                }
            };
        }
        let index = self.active;
        let doc_id = self.docs[index].id();
        let before = self.docs[index].document.clone();
        // The other documents, to put back should the command break their
        // linked copies. Cheap: shapes and sources are shared `Arc`s.
        let others: Vec<(u64, Document)> = self
            .docs
            .iter()
            .filter(|d| d.id() != doc_id)
            .map(|d| (d.id(), d.document.clone()))
            .collect();
        self.no_user_change = false;
        self.coalesce = None;
        let caught = run_caught(command, self, params, origin);
        let no_user_change = std::mem::take(&mut self.no_user_change);
        let coalesce = self.coalesce.take();
        let Ok(result) = caught else {
            // A panic: the command may have stopped half way. Put the
            // document back, if it is still open.
            if let Some(state) = self.docs.iter_mut().find(|d| d.id() == doc_id) {
                state.restore_document(before);
                state.prune_selection();
            }
            return Err(internal_error(command.id()));
        };
        if result.is_ok() {
            // A group of one is no group, and members never drift apart:
            // the one place every document change passes. Every open
            // document is checked, not only the active one.
            self.docs[index].document.dissolve_lonely_groups();
            // The same for groups of objects: a group of one dissolves.
            self.docs[index].document.normalize_groups();
            // An ignored pair whose object is gone is forgotten, in the
            // same undo step as the delete.
            self.docs[index].document.prune_ignored_pairs();
            debug_assert!(self.docs[index].document.groups_valid());
            if !self.links_agree() {
                for d in &mut self.docs {
                    if d.id() == doc_id {
                        d.restore_document(before.clone());
                    } else if let Some((_, doc)) = others.iter().find(|(id, _)| *id == d.id()) {
                        d.restore_document(doc.clone());
                    }
                    d.prune_selection();
                }
                return Err(CommandError::Failed(LINKS_DRIFTED.into()));
            }
        }
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
                state.restore_document(before);
                state.prune_selection();
                Err(CommandError::Refused(TOO_LARGE.into()))
            }
            Ok(value) => {
                // A save after the burst's last move ends it (`end_burst`);
                // a clean document never joins a burst either way.
                let joined = match (&coalesce, state.burst.take()) {
                    (Some((key, ids)), Some(b))
                        if b.key == *key
                            && b.ids == *ids
                            && b.revision == state.revision
                            && state.dirty =>
                    {
                        Some(b)
                    }
                    _ => None,
                };
                let joining = joined.is_some();
                let (dirty_before, redo_before) = match joined {
                    Some(b) => (b.dirty_before, b.redo_before),
                    None => {
                        // A burst's first step: keep what it may give back.
                        let redo = if coalesce.is_some() {
                            state.undo.redo_states().to_vec()
                        } else {
                            Vec::new()
                        };
                        let dirty = state.dirty;
                        state.undo.record(before);
                        (dirty, redo)
                    }
                };
                state.touch();
                state.dirty = true;
                if joining
                    && state
                        .undo
                        .last()
                        .is_some_and(|start| same_document(start, &state.document))
                {
                    // Back where the burst started: nothing to undo, the
                    // redo steps it cleared are good again, and nothing
                    // more to save than before it.
                    state.undo.discard_last();
                    state.undo.restore_redo(redo_before);
                    state.dirty = dirty_before;
                } else {
                    state.burst = coalesce.map(|(key, ids)| Burst {
                        key,
                        ids,
                        revision: state.revision,
                        dirty_before,
                        redo_before,
                    });
                }
                state.prune_selection();
                Ok(value)
            }
            Err(e) => {
                state.restore_document(before);
                state.prune_selection();
                Err(e)
            }
        }
    }
}

impl Session {
    /// True if the linked copies of every open document agree
    /// ([`Document::link_groups_agree`]).
    fn links_agree(&self) -> bool {
        self.docs.iter().all(|d| d.document.link_groups_agree())
    }

    /// The open documents as they are, for [`Session::roll_back`]. Never a
    /// `DocState::clone`, which would give new ids.
    pub(crate) fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            docs: self
                .docs
                .iter()
                .map(|d| DocSnapshot {
                    id: d.id,
                    document: d.document.clone(),
                    selection: d.selection.clone(),
                    undo: d.undo.clone(),
                    path: d.path.clone(),
                    dirty: d.dirty,
                    revision: d.revision,
                    last_job: d.last_job.clone(),
                    last_sent: d.last_sent.clone(),
                    last_sent_rev: d.last_sent_rev,
                })
                .collect(),
            active: self.active,
            pending_send: self.pending_send.clone(),
        }
    }

    /// Puts the open documents back as `snapshot` saw them, after a command
    /// panicked half way. A document that is still open gets its contents
    /// back in place, so its id, its caches and a pending send on it stay;
    /// one the command removed or replaced comes back with its old id;
    /// documents the command added are dropped. Every restored document
    /// gets a new revision, so work started on any state meanwhile is not
    /// stored.
    pub(crate) fn roll_back(&mut self, snapshot: SessionSnapshot) {
        let mut current = std::mem::take(&mut self.docs);
        for d in snapshot.docs {
            let st = match current.iter().position(|c| c.id == d.id) {
                Some(i) => {
                    let mut st = current.swap_remove(i);
                    st.set_document(d.document);
                    st.selection = d.selection;
                    st.undo = d.undo;
                    st.path = d.path;
                    st.dirty = d.dirty;
                    st
                }
                None => DocState {
                    id: d.id,
                    document: d.document,
                    selection: d.selection,
                    undo: d.undo,
                    path: d.path,
                    dirty: d.dirty,
                    last_job: d.last_job,
                    last_sent: d.last_sent,
                    last_sent_rev: d.last_sent_rev,
                    revision: d.revision + 1,
                    bounds_cache: HashMap::new(),
                    burst: None,
                    cut_cache: Mutex::default(),
                },
            };
            self.docs.push(st);
        }
        // What is left in `current` was added by the command.
        self.active = snapshot.active.min(self.docs.len().saturating_sub(1));
        // A send the failed command asked for never waits.
        self.pending_send = snapshot.pending_send;
    }
}

/// What the command `id` returns when it panicked. The session state is
/// put back, but a file a command wrote may stay, so those commands do not
/// claim that nothing changed.
pub(crate) fn internal_error(id: &str) -> CommandError {
    const REPORT: &str = "Please report it at https://github.com/jordiboehme/flatcraft/issues.";
    let written = match id {
        "project.save" | "tools.test_patterns" => Some("the file may have been written"),
        "job.send" => Some("the job may have been written or sent"),
        _ => None,
    };
    CommandError::Failed(match written {
        Some(w) => format!(
            "FlatCraft hit an internal error: {w}; the document state was restored. {REPORT}"
        ),
        None => {
            format!("FlatCraft hit an internal error, so this command changed nothing. {REPORT}")
        }
    })
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
        let result = command.run(s, params, origin);
        #[cfg(test)]
        if tests::PANIC_AFTER.with(std::cell::Cell::get) == Some(command.id()) {
            panic!("test: {} panics after it ran", command.id());
        }
        result
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
        /// The command that panics right after it ran (it did all its work).
        pub(super) static PANIC_AFTER: Cell<Option<&'static str>> = const { Cell::new(None) };
    }

    const SQUARE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="20" y="20" width="30" height="30"/></svg>"#;

    /// Runs `id` as the user; it panics after it did its work.
    fn panicking(s: &mut Session, id: &'static str, params: serde_json::Value) {
        PANIC_AFTER.with(|p| p.set(Some(id)));
        let e = s.execute(id, params, Origin::User).unwrap_err();
        PANIC_AFTER.with(|p| p.set(None));
        assert!(e.to_string().contains("internal error"), "{id}: {e}");
    }

    fn ok(s: &mut Session, id: &str, params: serde_json::Value) -> serde_json::Value {
        s.execute(id, params, Origin::User)
            .unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    /// A window session on the Cameo 2 (USB) with one square in document
    /// A and an agent's cut waiting for confirmation, and a pristine
    /// document B, active.
    fn with_pending(dir: &tempfile::TempDir) -> Session {
        let profiles = flatcraft_device::ProfileStore::load(dir.path()).unwrap();
        let prefs = Preferences {
            output_dir: Some(dir.path().join("output")),
            ..Preferences::default()
        };
        let mut s = Session::new(profiles, prefs, true, dir.path().to_path_buf());
        ok(
            &mut s,
            "object.import_svg",
            serde_json::json!({ "svg": SQUARE, "name": "sq" }),
        );
        ok(
            &mut s,
            "job.set_plotter",
            serde_json::json!({ "id": "silhouette-cameo-2" }),
        );
        let e = s
            .execute(
                "job.send",
                serde_json::json!({ "name": "wait" }),
                Origin::Agent,
            )
            .unwrap_err();
        assert!(e.to_string().contains("Waiting for confirmation"), "{e}");
        ok(&mut s, "document.new", serde_json::json!({}));
        s
    }

    fn ids(s: &Session) -> Vec<u64> {
        s.docs.iter().map(DocState::id).collect()
    }

    #[test]
    fn panicking_open_keeps_doc_ids_and_pending_send() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = with_pending(&dir);
        let file = dir.path().join("p.flatcraft");
        flatcraft_core::project::save(&s.docs[0].document, &file).unwrap();
        let pending = s.pending_send().unwrap().id;
        let before = ids(&s);
        let revision = s.docs[1].revision();
        // B is pristine: open replaces it in its slot, then panics.
        assert!(s.doc().is_pristine());
        panicking(&mut s, "project.open", serde_json::json!({ "path": file }));
        assert_eq!(ids(&s), before, "the same documents, same ids");
        assert_eq!(s.active, 1);
        assert!(s.doc().is_pristine());
        assert!(s.doc().revision() > revision, "a restored document is news");
        assert_eq!(s.pending_send().map(|p| p.id), Some(pending));
        // A is active and not pristine: open pushes a document, then panics.
        ok(&mut s, "document.switch", serde_json::json!({ "index": 0 }));
        let a = s.docs[0].document.clone();
        panicking(&mut s, "project.open", serde_json::json!({ "path": file }));
        assert_eq!(ids(&s), before, "the pushed document is gone");
        assert_eq!(s.active, 0);
        assert!(same_document(&s.docs[0].document, &a));
        assert_eq!(s.pending_send().map(|p| p.id), Some(pending));
    }

    #[test]
    fn panicking_close_and_undo_put_the_documents_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = with_pending(&dir);
        let pending = s.pending_send().unwrap().id;
        let before = ids(&s);
        let a = s.docs[0].document.clone();
        let revision = s.docs[0].revision();
        assert!(s.docs[0].dirty);
        panicking(
            &mut s,
            "document.close",
            serde_json::json!({ "index": 0, "discard_changes": true }),
        );
        assert_eq!(ids(&s), before);
        assert_eq!(s.active, 1);
        let st = &s.docs[0];
        assert!(same_document(&st.document, &a));
        assert!(st.dirty && st.undo.can_undo());
        assert!(st.revision() > revision);
        assert_eq!(s.pending_send().map(|p| p.id), Some(pending));
        // Undo in A, by id, from B.
        let id = before[0];
        let revision = s.docs[0].revision();
        panicking(&mut s, "edit.undo", serde_json::json!({ "doc_id": id }));
        let st = &s.docs[0];
        assert!(same_document(&st.document, &a), "the undo is undone");
        assert!(st.undo.can_undo() && !st.undo.can_redo());
        assert!(st.revision() > revision);
        assert_eq!(ids(&s), before);
        assert_eq!(s.pending_send().map(|p| p.id), Some(pending));
    }

    /// A job.send that panics after it stored a pending send leaves no
    /// pending send behind: nothing waits for a request that failed.
    #[test]
    fn a_panicking_send_leaves_no_pending_send() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = with_pending(&dir);
        ok(&mut s, "document.switch", serde_json::json!({ "index": 0 }));
        s.deny_pending(s.pending_send().unwrap().id);
        PANIC_AFTER.with(|p| p.set(Some("job.send")));
        let e = s
            .execute(
                "job.send",
                serde_json::json!({ "name": "boom" }),
                Origin::Agent,
            )
            .unwrap_err();
        PANIC_AFTER.with(|p| p.set(None));
        assert!(e.to_string().contains("internal error"), "{e}");
        assert!(s.pending_send().is_none(), "{:?}", s.pending_send());
    }

    /// A panic after a command wrote a file does not claim that nothing
    /// changed.
    #[test]
    fn a_panic_after_a_write_says_the_file_may_be_written() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = with_pending(&dir);
        let file = dir.path().join("p.flatcraft");
        PANIC_AFTER.with(|p| p.set(Some("project.save")));
        let e = s
            .execute(
                "project.save",
                serde_json::json!({ "path": file }),
                Origin::User,
            )
            .unwrap_err();
        PANIC_AFTER.with(|p| p.set(None));
        let text = e.to_string();
        assert!(file.exists());
        assert!(!text.contains("changed nothing"), "{text}");
        assert!(text.contains("the file may have been written"), "{text}");
        assert!(text.contains("the document state was restored"), "{text}");
        assert!(s.doc().path.is_none(), "the path is rolled back");
    }

    /// Every command that does not change the document either changes no
    /// session state at all, or is rolled back after a panic. A new
    /// command must pick one.
    #[test]
    fn non_mutating_commands_declare_session_changes() {
        let read_only: Vec<&str> = registry::all()
            .iter()
            .filter(|c| !c.mutates_document() && !c.changes_session())
            .map(|c| c.id())
            .collect();
        assert_eq!(
            read_only,
            [
                "document.state",
                "document.notes",
                "document.check_sources",
                "document.check",
                "profiles.list",
                "profiles.get",
                "job.preview",
                "job.history",
                "job.troubleshoot",
                "document.preview",
                "document.export",
            ]
        );
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

    /// A mutating command that changes a shared field of one linked copy
    /// only (a bug the session must catch).
    struct Drifts;

    impl registry::Command for Drifts {
        fn id(&self) -> &'static str {
            "test.drifts"
        }
        fn title(&self) -> &'static str {
            "Drifts"
        }
        fn description(&self) -> &'static str {
            "Changes the cut options of the first object only."
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
            s.doc_mut().document.objects[0].cut.auto_threshold_mm = 2.0;
            Ok(serde_json::json!({}))
        }
    }

    #[test]
    fn linked_copies_that_would_drift_apart_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let profiles = flatcraft_device::ProfileStore::load(dir.path()).unwrap();
        let prefs = Preferences {
            output_dir: Some(dir.path().join("output")),
            ..Preferences::default()
        };
        let mut s = Session::new(profiles, prefs, false, dir.path().to_path_buf());
        let id = ok(
            &mut s,
            "object.import_svg",
            serde_json::json!({ "svg": SQUARE, "name": "sq" }),
        )["id"]
            .as_u64()
            .unwrap();
        ok(
            &mut s,
            "object.copies",
            serde_json::json!({ "id": id, "count": 1, "linked": true }),
        );
        let undo = s.doc().undo.undo_len();
        let revision = s.doc().revision();
        let e = s
            .run_command(&Drifts, serde_json::json!({}), Origin::Agent)
            .unwrap_err();
        assert_eq!(e, CommandError::Failed(LINKS_DRIFTED.into()));
        assert!(s.doc().document.link_groups_agree());
        assert_eq!(s.doc().document.objects[0].cut, CutOptions::default());
        assert_eq!(s.doc().undo.undo_len(), undo);
        assert_eq!(s.doc().revision(), revision);
    }

    /// Like [`Drifts`], but in the first open document, not the active one.
    struct DriftsElsewhere;

    impl registry::Command for DriftsElsewhere {
        fn id(&self) -> &'static str {
            "test.drifts_elsewhere"
        }
        fn title(&self) -> &'static str {
            "Drifts elsewhere"
        }
        fn description(&self) -> &'static str {
            "Changes the cut options of the first object of the first document only."
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
            s.docs[0].document.objects[0].cut.auto_threshold_mm = 2.0;
            Ok(serde_json::json!({}))
        }
    }

    #[test]
    fn linked_copies_of_another_document_are_checked_too() {
        let dir = tempfile::tempdir().unwrap();
        let profiles = flatcraft_device::ProfileStore::load(dir.path()).unwrap();
        let prefs = Preferences {
            output_dir: Some(dir.path().join("output")),
            ..Preferences::default()
        };
        let mut s = Session::new(profiles, prefs, false, dir.path().to_path_buf());
        let id = ok(
            &mut s,
            "object.import_svg",
            serde_json::json!({ "svg": SQUARE, "name": "sq" }),
        )["id"]
            .as_u64()
            .unwrap();
        ok(
            &mut s,
            "object.copies",
            serde_json::json!({ "id": id, "count": 1, "linked": true }),
        );
        ok(&mut s, "document.new", serde_json::json!({}));
        assert_eq!(s.active, 1);
        let e = s
            .run_command(&DriftsElsewhere, serde_json::json!({}), Origin::Agent)
            .unwrap_err();
        assert_eq!(e, CommandError::Failed(LINKS_DRIFTED.into()));
        assert!(s.docs[0].document.link_groups_agree());
        assert_eq!(s.docs[0].document.objects[0].cut, CutOptions::default());
    }

    #[test]
    fn a_change_of_the_link_group_alone_is_a_change() {
        let mut a = Document::new(JobSettings {
            plotter_id: "p".into(),
            material_id: "m".into(),
            media_id: "x".into(),
            speed: None,
            force: None,
            passes: None,
            strategy: Default::default(),
            weed: Default::default(),
        });
        a.add(
            "a".into(),
            Source::Svg {
                name: "a.svg".into(),
                bytes: Vec::new().into(),
            },
            Vec::new(),
        );
        let mut b = a.clone();
        b.objects[0].link_group = Some(flatcraft_core::document::GroupId(1));
        assert!(!same_document(&a, &b));
        assert!(!same_except_links(&a, &b));
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
            strategy: Default::default(),
            weed: Default::default(),
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
