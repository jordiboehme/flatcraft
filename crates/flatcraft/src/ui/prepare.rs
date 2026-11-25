//! The Prepare workspace: the Objects panel on the left, the Settings panel
//! on the right, the canvas in the middle and its pills.
//!
//! Each frame starts with a [`Snapshot`] of the active document, taken
//! under one short lock. Everything is drawn from the snapshot; changes go
//! out as [`Action`]s that name the document they were made for.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use egui::{Rect, RichText, Ui};
use flatcraft_core::document::{CutOptions, JobSettings, Object, Source};
use flatcraft_core::geometry::CutPath;
use flatcraft_core::transform::Transform;
use flatcraft_device::{MaterialProfile, Media, MediaKind, PlotterProfile, PresetInfo};
use flatcraft_engine::commands::{placed_bounds, sources};
use flatcraft_engine::job::JobInput;
use serde_json::json;

use super::canvas::{self, CanvasView};
use super::theme::Tokens;
use super::{Action, FlatCraftApp, lock, objects, settings};

/// Linked files are checked at most this often.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// Curves are drawn as polylines this close to the true curve, in mm.
const DRAW_TOLERANCE_MM: f64 = 0.02;

/// The ribbon tabs of Prepare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RibbonTab {
    #[default]
    Home,
    Arrange,
    View,
}

impl RibbonTab {
    pub const ALL: [RibbonTab; 3] = [RibbonTab::Home, RibbonTab::Arrange, RibbonTab::View];

    pub fn name(self) -> &'static str {
        match self {
            RibbonTab::Home => "Home",
            RibbonTab::Arrange => "Arrange",
            RibbonTab::View => "View",
        }
    }
}

/// A drag on the canvas. Nothing is sent until it ends, and then only to
/// the document it started in; a document switch cancels it.
#[derive(Debug, Clone, PartialEq)]
pub enum Drag {
    Pan,
    Move {
        doc_id: u64,
        ids: Vec<u64>,
        start: kurbo::Point,
        current: kurbo::Point,
    },
    /// A corner handle of the selection box. Several objects scale as one
    /// group (`object.scale` with `group`).
    Scale {
        doc_id: u64,
        ids: Vec<u64>,
        /// 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left.
        corner_index: usize,
        corner: kurbo::Point,
        /// The opposite corner, which stays where it is.
        anchor: kurbo::Point,
        current: kurbo::Point,
    },
}

/// What the last check of linked files found, for one document.
#[derive(Debug, Clone, Default)]
pub struct SourceStatus {
    /// The document these lists belong to.
    pub doc_id: u64,
    pub changed: Vec<u64>,
    pub missing: Vec<u64>,
    pub unreadable: Vec<u64>,
    /// The ids the last automatic reload asked for.
    pub auto_reloaded: Vec<u64>,
    /// A check was asked for (project open, window focus) and waits for the
    /// debounce or the end of a drag.
    pub check_pending: bool,
    /// The pending check skips the debounce (a document switch).
    pub check_now: bool,
    /// When the last check ran, for any document (the debounce).
    pub last_check: Option<Instant>,
    /// When a check last filled these lists.
    pub checked_at: Option<Instant>,
    /// Objects whose file `object.relink` also found in the new folder.
    pub also_found: Vec<(u64, PathBuf)>,
}

impl SourceStatus {
    /// Forgets these objects in every list.
    pub fn forget(&mut self, ids: &[u64]) {
        for list in [&mut self.changed, &mut self.missing, &mut self.unreadable] {
            list.retain(|i| !ids.contains(i));
        }
        self.also_found.retain(|(i, _)| !ids.contains(i));
    }

    pub fn broken(&self, id: u64) -> bool {
        self.missing.contains(&id) || self.unreadable.contains(&id)
    }
}

/// Edit buffers of the ribbon's number fields.
#[derive(Debug, Clone)]
pub struct Fields {
    pub width: String,
    pub height: String,
    pub percent: String,
    pub rotate: String,
    pub keep_aspect: bool,
    /// Align to the cutting area instead of the selection.
    pub align_to_area: bool,
    pub search: String,
}

impl Default for Fields {
    fn default() -> Self {
        Self {
            width: String::new(),
            height: String::new(),
            percent: String::new(),
            rotate: "15".into(),
            keep_aspect: true,
            align_to_area: false,
            search: String::new(),
        }
    }
}

/// Speed, force or passes while its drag value is being dragged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Override {
    Speed,
    Force,
    Passes,
}

/// Everything Prepare remembers between frames.
#[derive(Debug, Clone)]
pub struct PrepareState {
    pub view: CanvasView,
    /// Fit the mat into the canvas on the next frame.
    pub fit_pending: bool,
    /// The document the view was fitted for.
    pub view_doc: Option<u64>,
    /// The canvas area of the last frame, in points.
    pub canvas_rect: Rect,
    pub tab: RibbonTab,
    pub show_objects: bool,
    pub show_settings: bool,
    pub show_grid: bool,
    pub show_rulers: bool,
    pub fields: Fields,
    /// The search hits stay open (see `ribbon::search_box`).
    pub search_open: bool,
    pub drag: Option<Drag>,
    /// A drag value being dragged, and its value so far.
    pub live_override: Option<(Override, f64)>,
    /// Warnings of the last import, until dismissed.
    pub notices: Vec<String>,
    pub sources: SourceStatus,
    /// The link status of the other open documents, so switching back
    /// shows it at once and a quick switch does not check again.
    pub source_memory: HashMap<u64, SourceStatus>,
    pub cache: ShapeCache,
    /// The open Copies dialog.
    pub copies: Option<CopiesDialog>,
    /// The settings of the last copies made, for the next dialog.
    pub copies_last: CopiesSettings,
}

/// What the Copies dialog asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CopiesSettings {
    pub count: u32,
    pub grid: bool,
    pub columns: u32,
    pub gap_mm: f64,
    /// Linked copies share the source, the cut options and the scale.
    pub linked: bool,
}

impl Default for CopiesSettings {
    fn default() -> Self {
        Self {
            count: 3,
            grid: false,
            columns: 4,
            gap_mm: 5.0,
            linked: false,
        }
    }
}

/// The Copies dialog: its object and the settings so far.
#[derive(Debug, Clone, PartialEq)]
pub struct CopiesDialog {
    pub doc_id: u64,
    /// The object to copy.
    pub id: u64,
    pub name: String,
    pub count: u32,
    pub grid: bool,
    pub columns: u32,
    pub gap_mm: f64,
    pub linked: bool,
}

impl CopiesDialog {
    pub fn new(doc_id: u64, id: u64, name: String, s: CopiesSettings) -> Self {
        Self {
            doc_id,
            id,
            name,
            count: s.count,
            grid: s.grid,
            columns: s.columns,
            gap_mm: s.gap_mm,
            linked: s.linked,
        }
    }

    pub fn settings(&self) -> CopiesSettings {
        CopiesSettings {
            count: self.count,
            grid: self.grid,
            columns: self.columns,
            gap_mm: self.gap_mm,
            linked: self.linked,
        }
    }
}

impl Default for PrepareState {
    fn default() -> Self {
        Self {
            view: CanvasView::default(),
            fit_pending: true,
            view_doc: None,
            canvas_rect: Rect::NOTHING,
            tab: RibbonTab::default(),
            show_objects: true,
            show_settings: true,
            show_grid: true,
            show_rulers: true,
            fields: Fields::default(),
            search_open: false,
            drag: None,
            live_override: None,
            notices: Vec::new(),
            sources: SourceStatus::default(),
            source_memory: HashMap::new(),
            cache: ShapeCache::default(),
            copies: None,
            copies_last: CopiesSettings::default(),
        }
    }
}

// ---------------------------------------------------------------- snapshot

/// One drawn polyline, relative to the object's centre, in mm.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub points: Vec<kurbo::Point>,
    pub closed: bool,
    pub colour: Option<[u8; 3]>,
}

/// An object's placed paths and engine bounds, relative to its centre.
/// Moving an object does not change it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shape {
    pub lines: Vec<Line>,
    /// What `document.state` reports as `bounds_mm`, minus the centre.
    pub bounds: Option<kurbo::Rect>,
}

/// Everything a cached shape depends on, except the centre.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ShapeKey {
    geometry: usize,
    scale_x: f64,
    scale_y: f64,
    rotation_deg: f64,
    mirror: bool,
    local_bounds: kurbo::Rect,
    cut: CutOptions,
}

/// A cached shape, its key and the geometry the key points to.
type CacheEntry = (ShapeKey, Arc<Vec<CutPath>>, Arc<Shape>);

/// Shapes with more path elements than this are built on a worker thread,
/// so scaling or rotating a huge object never freezes the window. Smaller
/// ones are built at once, in the frame that needs them.
pub const INLINE_ELEMENTS: usize = 4000;

/// What a shape is built from: the parts of an object it depends on. The
/// geometry is shared with the document (one `Arc`), so taking it under the
/// session lock is cheap.
#[derive(Debug, Clone)]
pub struct ShapeInput {
    pub geometry: Arc<Vec<CutPath>>,
    pub transform: Transform,
    pub local_bounds: kurbo::Rect,
    pub cut: CutOptions,
}

impl ShapeInput {
    fn of(o: &Object) -> Self {
        Self {
            geometry: o.geometry.clone(),
            transform: o.transform,
            local_bounds: o.local_bounds,
            cut: o.cut,
        }
    }

    fn key(&self) -> ShapeKey {
        let t = &self.transform;
        ShapeKey {
            geometry: Arc::as_ptr(&self.geometry) as usize,
            scale_x: t.scale_x,
            scale_y: t.scale_y,
            rotation_deg: t.rotation_deg,
            mirror: t.mirror,
            local_bounds: self.local_bounds,
            cut: self.cut,
        }
    }

    fn elements(&self) -> usize {
        self.geometry.iter().map(|p| p.path.elements().len()).sum()
    }

    /// An object with only what drawing and bounds read.
    fn object(&self) -> Object {
        Object {
            id: flatcraft_core::document::ObjectId(0),
            name: String::new(),
            source: Source::Svg {
                name: String::new(),
                bytes: Vec::new().into(),
            },
            transform: self.transform,
            visible: true,
            cut: self.cut,
            link: None,
            geometry: self.geometry.clone(),
            local_bounds: self.local_bounds,
            frame: kurbo::Rect::ZERO,
            link_group: None,
        }
    }
}

/// Builds a shape on the worker thread; tests replace it.
pub type ShapeBuilder = Arc<dyn Fn(&ShapeInput) -> Shape + Send + Sync>;

/// A shape for the worker: document, object, input.
struct ShapeJob {
    doc_id: u64,
    id: u64,
    input: ShapeInput,
    ctx: egui::Context,
}

/// A finished shape from the worker.
struct ShapeDone {
    doc_id: u64,
    id: u64,
    key: ShapeKey,
    geometry: Arc<Vec<CutPath>>,
    shape: Arc<Shape>,
}

/// Shapes by document id, then object id. Kept for every open document,
/// so switching back to a document needs no rebuild.
pub struct ShapeCache {
    /// The geometry `Arc` is kept so its address cannot be reused by
    /// other geometry while the key names it.
    docs: HashMap<u64, HashMap<u64, CacheEntry>>,
    /// Shapes the worker is building, with the key they are built for.
    pending: HashMap<(u64, u64), ShapeKey>,
    builder: ShapeBuilder,
    jobs: Option<mpsc::Sender<ShapeJob>>,
    done: Option<mpsc::Receiver<ShapeDone>>,
    /// The worker died (or could not start): shapes are built inline.
    dead: bool,
}

/// The worker loop: takes every job waiting and builds only the newest one
/// per object, so a burst of scales builds the last shape, not each step.
fn shape_worker(
    rx: &mpsc::Receiver<ShapeJob>,
    tx: &mpsc::Sender<ShapeDone>,
    builder: &ShapeBuilder,
) {
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        let mut newest: Vec<ShapeJob> = Vec::new();
        for job in batch {
            match newest
                .iter_mut()
                .find(|j| (j.doc_id, j.id) == (job.doc_id, job.id))
            {
                Some(slot) => *slot = job,
                None => newest.push(job),
            }
        }
        for job in newest {
            let shape = Arc::new(builder(&job.input));
            let reply = ShapeDone {
                doc_id: job.doc_id,
                id: job.id,
                key: job.input.key(),
                geometry: job.input.geometry.clone(),
                shape,
            };
            if tx.send(reply).is_err() {
                return;
            }
            job.ctx.request_repaint();
        }
    }
}

impl Default for ShapeCache {
    fn default() -> Self {
        Self {
            docs: HashMap::new(),
            pending: HashMap::new(),
            builder: Arc::new(build_shape),
            jobs: None,
            done: None,
            dead: false,
        }
    }
}

impl std::fmt::Debug for ShapeCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShapeCache")
            .field("docs", &self.docs.len())
            .field("pending", &self.pending.len())
            .finish()
    }
}

/// A clone starts empty: it never shares the worker.
impl Clone for ShapeCache {
    fn clone(&self) -> Self {
        Self {
            builder: self.builder.clone(),
            ..Self::default()
        }
    }
}

impl ShapeCache {
    /// Builds worker shapes with `f` instead of [`build_shape`] (tests).
    pub fn set_builder(&mut self, f: impl Fn(&ShapeInput) -> Shape + Send + Sync + 'static) {
        self.builder = Arc::new(f);
        self.jobs = None;
        self.done = None;
        self.pending.clear();
        self.dead = false;
    }

    /// The cached shape of an object, whatever transform it was built for.
    pub fn shape(&self, doc_id: u64, id: u64) -> Option<Arc<Shape>> {
        self.docs.get(&doc_id)?.get(&id).map(|e| e.2.clone())
    }

    /// How many shapes the worker is building.
    pub fn building(&self) -> usize {
        self.pending.len()
    }

    /// The shape for this key, or `None` when it must be built.
    fn current(&self, doc_id: u64, id: u64, key: &ShapeKey) -> Option<Arc<Shape>> {
        match self.docs.get(&doc_id)?.get(&id) {
            Some((k, _, shape)) if k == key => Some(shape.clone()),
            _ => None,
        }
    }

    /// Takes the shapes the worker finished. A worker that died is
    /// dropped with what it was building, so those shapes are asked for
    /// again (and built inline, see `submit`).
    fn collect(&mut self) {
        let Some(done) = &self.done else { return };
        loop {
            match done.try_recv() {
                Ok(d) => {
                    if self.pending.get(&(d.doc_id, d.id)) == Some(&d.key) {
                        self.pending.remove(&(d.doc_id, d.id));
                    }
                    self.docs
                        .entry(d.doc_id)
                        .or_default()
                        .insert(d.id, (d.key, d.geometry, d.shape));
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker_died();
                    break;
                }
            }
        }
    }

    /// Forgets a dead worker and what it was building.
    fn worker_died(&mut self) {
        self.jobs = None;
        self.done = None;
        self.pending.clear();
        self.dead = true;
    }

    /// Builds a shape on this thread.
    fn build_inline(&mut self, job: ShapeJob) {
        let key = job.input.key();
        let shape = Arc::new(build_shape(&job.input));
        self.docs
            .entry(job.doc_id)
            .or_default()
            .insert(job.id, (key, job.input.geometry, shape));
    }

    /// Hands a shape to the worker, unless it is being built already. The
    /// worker starts on first use.
    fn submit(&mut self, job: ShapeJob) {
        let key = job.input.key();
        if self.pending.get(&(job.doc_id, job.id)) == Some(&key) {
            return;
        }
        if self.dead {
            self.build_inline(job);
            return;
        }
        if self.jobs.is_none() {
            let (jobs, rx) = mpsc::channel::<ShapeJob>();
            let (tx, done) = mpsc::channel();
            let builder = self.builder.clone();
            let spawned = std::thread::Builder::new()
                .name("shape-cache".into())
                .spawn(move || shape_worker(&rx, &tx, &builder));
            if spawned.is_err() {
                // No thread: build it here rather than never.
                self.dead = true;
                self.build_inline(job);
                return;
            }
            self.jobs = Some(jobs);
            self.done = Some(done);
        }
        self.pending.insert((job.doc_id, job.id), key);
        if let Some(jobs) = &self.jobs
            && let Err(mpsc::SendError(job)) = jobs.send(job)
        {
            // The worker died: build inline from now on.
            self.worker_died();
            self.build_inline(job);
        }
    }

    /// Forgets closed documents and deleted objects.
    fn prune(&mut self, open: &[u64], doc_id: u64, ids: &BTreeSet<u64>) {
        self.docs.retain(|d, _| open.contains(d));
        if let Some(shapes) = self.docs.get_mut(&doc_id) {
            shapes.retain(|id, _| ids.contains(id));
        }
        self.pending
            .retain(|(d, id), _| open.contains(d) && (*d != doc_id || ids.contains(id)));
    }
}

/// The shape of an object, built from the engine's placed paths and bounds
/// with the object's centre moved to 0,0.
pub fn build_shape(input: &ShapeInput) -> Shape {
    let mut o = input.object();
    o.transform.center_mm = kurbo::Point::ORIGIN;
    let mut lines = Vec::new();
    for p in o.placed_paths() {
        let mut current: Option<Line> = None;
        kurbo::flatten(
            p.path.elements().iter().copied(),
            DRAW_TOLERANCE_MM,
            |el| match el {
                kurbo::PathEl::MoveTo(pt) => {
                    if let Some(l) = current.take()
                        && l.points.len() > 1
                    {
                        lines.push(l);
                    }
                    current = Some(Line {
                        points: vec![pt],
                        closed: false,
                        colour: p.color,
                    });
                }
                kurbo::PathEl::LineTo(pt) => {
                    // A repeated point draws nothing.
                    if let Some(l) = &mut current
                        && l.points.last() != Some(&pt)
                    {
                        l.points.push(pt);
                    }
                }
                kurbo::PathEl::ClosePath => {
                    if let Some(mut l) = current.take() {
                        l.closed = true;
                        let start = l.points[0];
                        // The closing edge comes from `closed`; a last
                        // point on the start would repeat it.
                        while l.points.len() > 1 && l.points.last() == Some(&start) {
                            l.points.pop();
                        }
                        if l.points.len() > 1 {
                            lines.push(l);
                        }
                        current = Some(Line {
                            points: vec![start],
                            closed: false,
                            colour: p.color,
                        });
                    }
                }
                _ => {}
            },
        );
        if let Some(l) = current
            && l.points.len() > 1
        {
            lines.push(l);
        }
    }
    Shape {
        lines,
        bounds: placed_bounds(&o),
    }
}

/// One object as Prepare shows it.
#[derive(Debug, Clone)]
pub struct ObjView {
    pub id: u64,
    pub name: String,
    pub raster: bool,
    pub visible: bool,
    pub transform: Transform,
    pub cut: CutOptions,
    /// Last known path of the linked file.
    pub link: Option<PathBuf>,
    /// The group of linked copies the object belongs to.
    pub link_group: Option<u64>,
    pub shape: Arc<Shape>,
}

impl ObjView {
    /// The engine's bounds in mat mm, as `document.state` reports them.
    pub fn bounds(&self) -> Option<kurbo::Rect> {
        self.shape
            .bounds
            .map(|b| b + self.transform.center_mm.to_vec2())
    }
}

/// The active document as Prepare draws it.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub doc_id: u64,
    pub objects: Vec<ObjView>,
    pub selection: BTreeSet<u64>,
    pub job: JobSettings,
    pub plotter: Option<PlotterProfile>,
    /// The material with the plotter's overrides applied.
    pub material: Option<MaterialProfile>,
    pub plotters: Vec<PresetInfo>,
    pub materials: Vec<PresetInfo>,
    pub mat: Option<Media>,
    /// The cutting area, in mm.
    pub area: Option<kurbo::Rect>,
    /// Set when the document's mat is gone and the plotter's first mat is
    /// used instead.
    pub mat_warning: Option<String>,
    /// Why the plotter or material preset cannot be used (a bad value).
    pub profile_problem: Option<String>,
    pub auto_reload: bool,
    /// The image or the trace settings of the Trace dialog's object
    /// changed since the dialog opened (an agent retraced or reloaded it).
    pub trace_source_changed: bool,
    /// What a job build reads, when the snapshot was asked for it (Cut,
    /// Debug panel). An error says why there is no job (a missing profile).
    pub job_input: Option<Result<JobInput, flatcraft_engine::CommandError>>,
    /// The output folder of the preferences.
    pub output_pref: Option<PathBuf>,
    pub config_dir: PathBuf,
}

impl Snapshot {
    /// The physical mat in app coordinates (the grid's 0,0 is the origin).
    pub fn mat_rect(&self, m: &Media) -> kurbo::Rect {
        kurbo::Rect::new(
            -m.grid.origin_x_mm,
            -m.grid.origin_y_mm,
            m.width_mm - m.grid.origin_x_mm,
            m.height_mm - m.grid.origin_y_mm,
        )
    }

    pub fn selected(&self) -> impl Iterator<Item = &ObjView> {
        self.objects
            .iter()
            .filter(|o| self.selection.contains(&o.id))
    }

    pub fn object(&self, id: u64) -> Option<&ObjView> {
        self.objects.iter().find(|o| o.id == id)
    }

    /// How many objects are in the group of linked copies `group`.
    pub fn group_size(&self, group: u64) -> usize {
        self.objects
            .iter()
            .filter(|o| o.link_group == Some(group))
            .count()
    }

    /// How many linked copies a change of a shared setting (cut options,
    /// scale) of the selection changes: every member of the groups of the
    /// selected objects. `None` when no selected object is a linked copy.
    pub fn linked_reach(&self) -> Option<usize> {
        let groups: BTreeSet<u64> = self.selected().filter_map(|o| o.link_group).collect();
        (!groups.is_empty()).then(|| {
            self.objects
                .iter()
                .filter(|o| o.link_group.is_some_and(|g| groups.contains(&g)))
                .count()
        })
    }

    /// Visible objects that are not inside the cutting area.
    pub fn outside_area(&self) -> usize {
        let Some(a) = self.area else {
            return 0;
        };
        self.objects
            .iter()
            .filter(|o| o.visible)
            .filter_map(ObjView::bounds)
            .filter(|b| !flatcraft_engine::commands::inside(*b, a))
            .count()
    }

    pub fn uncalibrated_mat(&self) -> bool {
        self.mat
            .as_ref()
            .is_some_and(|m| m.kind == MediaKind::Mat && !m.calibrated)
    }
}

/// Takes the snapshot. The lock is held only to copy what is needed;
/// shapes that changed are rebuilt after it is released.
pub fn snapshot(app: &mut FlatCraftApp, ctx: &egui::Context) -> Snapshot {
    snapshot_with(app, ctx, false)
}

/// [`snapshot`], plus the job input when `want_job` is true. Shapes with
/// more than [`INLINE_ELEMENTS`] path elements are built on a worker
/// thread; until one is ready the object shows its last shape (or none),
/// and the window repaints when it arrives.
pub fn snapshot_with(app: &mut FlatCraftApp, ctx: &egui::Context, want_job: bool) -> Snapshot {
    let cache = &mut app.ui.prepare.cache;
    cache.collect();
    let trace = &mut app.ui.trace;
    let mut misses: Vec<(usize, ShapeInput)> = Vec::new();
    let open: Vec<u64>;
    let mut snap = {
        let s = lock(&app.session);
        open = s.docs.iter().map(|d| d.id()).collect();
        let st = s.doc();
        let doc_id = st.id();
        let job = st.document.job.clone();
        let plotter_result = s.profiles.plotter(&job.plotter_id);
        let material_result = s.profiles.material(&job.material_id);
        // A preset with a bad value: say why and how to fix it.
        let profile_problem = [
            plotter_result.as_ref().err(),
            material_result.as_ref().err(),
        ]
        .into_iter()
        .flatten()
        .find(|e| matches!(e, flatcraft_device::ProfileError::Invalid { .. }))
        .map(flatcraft_engine::commands::profile_problem_text);
        let plotter = plotter_result.ok();
        let material = material_result.ok().map(|m| m.for_plotter(&job.plotter_id));
        // The mat is drawn from the unchecked preset, so a bad value never
        // leaves the canvas blank.
        let shown_plotter = plotter
            .clone()
            .or_else(|| s.profiles.plotter_unchecked(&job.plotter_id).ok());
        let effective = shown_plotter
            .as_ref()
            .and_then(|p| flatcraft_engine::commands::effective_media(p, &job).ok())
            .map(|(m, warning)| (m.clone(), warning));
        let area = shown_plotter
            .as_ref()
            .zip(effective.as_ref())
            .and_then(|(p, (m, _))| p.cutting_area(&m.id));
        let (mat, mat_warning) = effective.unzip();
        let mat_warning = mat_warning.flatten();
        let trace_source_changed = trace.as_mut().is_some_and(|d| {
            d.doc_id == st.id()
                && match st.document.get(flatcraft_core::document::ObjectId(d.id)) {
                    Some(
                        o @ Object {
                            source: Source::Raster { bytes, trace, .. },
                            ..
                        },
                    ) => d.rev.changed(bytes, trace, &o.geometry),
                    _ => false,
                }
        });
        let mut objects = Vec::with_capacity(st.document.objects.len());
        for (i, o) in st.document.objects.iter().enumerate() {
            let input = ShapeInput::of(o);
            let shape = match cache.current(doc_id, o.id.0, &input.key()) {
                Some(shape) => shape,
                None => {
                    let stale = cache.shape(doc_id, o.id.0);
                    misses.push((i, input));
                    stale.unwrap_or_default()
                }
            };
            objects.push(ObjView {
                id: o.id.0,
                name: o.name.clone(),
                raster: matches!(o.source, Source::Raster { .. }),
                visible: o.visible,
                transform: o.transform,
                cut: o.cut,
                link: o.link.as_ref().map(|l| l.path.clone()),
                link_group: o.link_group.map(|g| g.0),
                shape,
            });
        }
        Snapshot {
            doc_id: st.id(),
            objects,
            selection: st.selection.iter().map(|i| i.0).collect(),
            job,
            plotter: plotter.clone(),
            material: material.clone(),
            plotters: s.profiles.plotters(),
            materials: s.profiles.materials(),
            mat,
            area,
            mat_warning,
            profile_problem,
            auto_reload: s.prefs.auto_reload,
            trace_source_changed,
            job_input: want_job.then(|| match (&plotter, &material) {
                (Some(p), Some(m)) => Ok(JobInput::from_parts(&st.document, p.clone(), m)),
                _ => JobInput::of(&s),
            }),
            output_pref: s.prefs.output_dir.clone(),
            config_dir: s.config_dir.clone(),
        }
    };
    let doc_id = snap.doc_id;
    for (i, input) in misses {
        let id = snap.objects[i].id;
        if input.elements() > INLINE_ELEMENTS {
            cache.submit(ShapeJob {
                doc_id,
                id,
                input,
                ctx: ctx.clone(),
            });
            continue;
        }
        let shape = Arc::new(build_shape(&input));
        cache
            .docs
            .entry(doc_id)
            .or_default()
            .insert(id, (input.key(), input.geometry, shape.clone()));
        snap.objects[i].shape = shape;
    }
    let ids: BTreeSet<u64> = snap.objects.iter().map(|o| o.id).collect();
    cache.prune(&open, doc_id, &ids);

    let p = &mut app.ui.prepare;
    let src = &mut p.sources;
    if src.doc_id != snap.doc_id {
        // The lists belong to the old document: keep them for a switch
        // back. Check the new one soon (not the very first document: a new
        // window has nothing linked, and opening a project asks for its
        // own check).
        let switched = src.doc_id != 0;
        let old = std::mem::take(src);
        let next = p.source_memory.remove(&snap.doc_id).unwrap_or_default();
        *src = SourceStatus {
            doc_id: snap.doc_id,
            check_pending: old.check_pending || switched,
            check_now: switched,
            last_check: old.last_check,
            ..next
        };
        if switched {
            p.source_memory.insert(old.doc_id, old);
        }
    }
    p.source_memory.retain(|id, _| open.contains(id));
    let src = &mut p.sources;
    // Objects deleted since the check are no longer reported.
    let gone: Vec<u64> = [&src.changed, &src.missing, &src.unreadable]
        .into_iter()
        .flatten()
        .copied()
        .filter(|id| !ids.contains(id))
        .collect();
    src.forget(&gone);
    snap
}

// ---------------------------------------------------------------- sources

/// A finished check of linked files: the result and whether it still
/// matches the document.
pub type CheckDone = (sources::CheckResult, bool);

/// How long a frame waits for a check it just started. Small checks land in
/// the same frame; a long one (hashing big files) finishes on its thread
/// and is taken in a later frame.
const CHECK_WAIT: Duration = Duration::from_millis(50);

impl FlatCraftApp {
    /// Runs a pending check of linked files unless a drag is in progress or
    /// the last check was less than [`CHECK_INTERVAL`] ago. A document
    /// switch checks at once, unless the document's lists are younger than
    /// [`CHECK_INTERVAL`] (an agent switching in a loop). The check runs on
    /// a worker thread through `sources::check_unlocked`: files are only
    /// stated under the session lock and hashed without it, and a result
    /// for a document that changed meanwhile is dropped and checked again.
    /// With the `auto_reload` preference on, changed files are reloaded
    /// right away (only those; never missing or unreadable ones).
    pub fn check_sources(&mut self, ctx: &egui::Context, now: Instant) {
        if let Some(rx) = &self.source_check_job {
            match rx.try_recv() {
                Ok(done) => {
                    self.source_check_job = None;
                    self.checked(ctx, now, done);
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    // The worker died: check again later.
                    self.source_check_job = None;
                    self.ui.prepare.sources.check_pending = true;
                }
            }
            return;
        }
        let p = &mut self.ui.prepare;
        if !p.sources.check_pending || p.drag.is_some() {
            return;
        }
        let recent = |t: Option<Instant>| {
            t.map(|t| now.saturating_duration_since(t))
                .filter(|since| *since < CHECK_INTERVAL)
        };
        if p.sources.check_now {
            if recent(p.sources.checked_at).is_some() {
                p.sources.check_pending = false;
                p.sources.check_now = false;
                return;
            }
        } else if let Some(since) = recent(p.sources.last_check) {
            ctx.request_repaint_after(CHECK_INTERVAL - since);
            return;
        }
        p.sources.check_pending = false;
        p.sources.check_now = false;
        p.sources.last_check = Some(now);
        let (tx, rx) = mpsc::channel();
        let session = self.session.clone();
        let check = self.source_check.clone();
        let repaint = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("source-check".into())
            .spawn(move || {
                let _ = tx.send(check(&session));
                repaint.request_repaint();
            });
        if spawned.is_err() {
            self.ui.prepare.sources.check_pending = true;
            return;
        }
        match rx.recv_timeout(CHECK_WAIT) {
            Ok(done) => self.checked(ctx, now, done),
            Err(_) => self.source_check_job = Some(rx),
        }
    }

    /// Takes a finished check into the link status of the active document.
    fn checked(&mut self, ctx: &egui::Context, now: Instant, (result, current): CheckDone) {
        let src = &mut self.ui.prepare.sources;
        if !current || src.doc_id != result.doc_id {
            // The document changed while its files were hashed.
            src.check_pending = true;
            ctx.request_repaint_after(CHECK_INTERVAL);
            return;
        }
        let ids = |v: &[flatcraft_core::document::ObjectId]| v.iter().map(|i| i.0).collect();
        src.changed = ids(&result.changed);
        src.missing = ids(&result.missing);
        src.unreadable = ids(&result.unreadable);
        src.checked_at = Some(now);
        let auto = lock(&self.session).prefs.auto_reload;
        let src = &mut self.ui.prepare.sources;
        if auto && !src.changed.is_empty() {
            let changed = src.changed.clone();
            src.auto_reloaded = changed.clone();
            self.apply(
                ctx,
                vec![Action::run(
                    result.doc_id,
                    "object.reload",
                    json!({ "ids": changed }),
                )],
            );
        }
    }
}

// ---------------------------------------------------------------- layout

/// Draws the panels and the canvas of Prepare.
pub fn show(ui: &mut Ui, app: &mut FlatCraftApp, snap: &Snapshot, actions: &mut Vec<Action>) {
    let t = app.tokens;
    let mut show_objects = app.ui.prepare.show_objects;
    egui::Panel::left("flatcraft-objects")
        .resizable(true)
        .default_size(240.0)
        .frame(egui::Frame::new().fill(t.panel).inner_margin(t.spacing))
        .show_collapsible(ui, &mut show_objects, |ui| {
            objects::show(ui, &app.ui.prepare, snap, &t, actions);
        });
    app.ui.prepare.show_objects = show_objects;

    let mut show_settings = app.ui.prepare.show_settings;
    egui::Panel::right("flatcraft-settings")
        .resizable(true)
        .default_size(260.0)
        .frame(egui::Frame::new().fill(t.panel).inner_margin(t.spacing))
        .show_collapsible(ui, &mut show_settings, |ui| {
            settings::show(ui, &mut app.ui.prepare, snap, &t, actions);
        });
    app.ui.prepare.show_settings = show_settings;

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(t.bg))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            canvas::show(ui, rect, snap, &mut app.ui.prepare, &t, actions);
            pills(ui, rect, snap, &mut app.ui.prepare, &t, actions);
        });
    let p = &mut app.ui.prepare;
    if let Some(used) = copies_window(ui.ctx(), &mut p.copies, snap, &t, actions) {
        p.copies_last = used;
    }
}

/// The Copies dialog. It closes when its document is no longer shown or its
/// object is gone. Returns the settings when copies were made.
fn copies_window(
    ctx: &egui::Context,
    dialog: &mut Option<CopiesDialog>,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) -> Option<CopiesSettings> {
    use flatcraft_engine::commands::object::MAX_COPIES;
    let Some(d) = dialog else {
        return None;
    };
    if d.doc_id != snap.doc_id || !snap.objects.iter().any(|o| o.id == d.id) {
        *dialog = None;
        return None;
    }
    let mut close = false;
    let mut made = None;
    let modal = egui::Modal::new(egui::Id::unique("flatcraft-copies")).show(ctx, |ui| {
        ui.set_width(320.0);
        ui.label(RichText::new(format!("Copies of {}", d.name)).font(super::fonts::semibold(15.0)));
        ui.add_space(t.spacing / 2.0);
        egui::Grid::new("flatcraft-copies-grid")
            .num_columns(2)
            .spacing([t.spacing, t.spacing / 2.0])
            .show(ui, |ui| {
                settings::painted_label(ui, "Number of copies");
                let r = ui.add(
                    egui::DragValue::new(&mut d.count)
                        .range(1..=MAX_COPIES)
                        .speed(0.2),
                );
                settings::set_name(ui, &r, "Number of copies");
                ui.end_row();

                settings::painted_label(ui, "Layout");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut d.grid, false, "Row")
                        .on_hover_text("One row to the right of the original");
                    ui.radio_value(&mut d.grid, true, "Grid").on_hover_text(
                        "Rows to the right, then down. The original is the first cell.",
                    );
                });
                ui.end_row();

                settings::painted_label(ui, "Columns");
                let r = ui.add_enabled(
                    d.grid,
                    egui::DragValue::new(&mut d.columns)
                        .range(1..=MAX_COPIES)
                        .speed(0.1),
                );
                settings::set_name(ui, &r, "Columns");
                r.on_disabled_hover_text("Pick Grid to set the columns.");
                ui.end_row();

                settings::painted_label(ui, "Gap");
                let r = ui.add(
                    egui::DragValue::new(&mut d.gap_mm)
                        .range(0.0..=1000.0)
                        .speed(0.1)
                        .max_decimals(2)
                        .suffix(" mm"),
                );
                settings::set_name(ui, &r, "Gap (mm)");
                ui.end_row();
            });
        ui.checkbox(&mut d.linked, "Linked copies").on_hover_text(
            "One design, many bodies, like the Clone Army: the copies share the source, the cut options and the scale. Each keeps its own place, rotation and mirror.",
        );
        ui.label(
            RichText::new(
                "Copies that land off the mat still arrive. The red pill tells you how many.",
            )
            .color(t.text_dim),
        );
        ui.add_space(t.spacing / 2.0);
        ui.horizontal(|ui| {
            if ui.button("Make copies").clicked() {
                let layout = if d.grid {
                    json!({ "grid": { "columns": d.columns } })
                } else {
                    json!("row")
                };
                actions.push(Action::run(
                    d.doc_id,
                    "object.copies",
                    json!({
                        "id": d.id,
                        "count": d.count,
                        "layout": layout,
                        "gap_mm": d.gap_mm,
                        "linked": d.linked,
                    }),
                ));
                made = Some(d.settings());
                close = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if close || modal.should_close() {
        *dialog = None;
    }
    made
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The messages at the bottom-left of the canvas, stacked upwards, so
/// they never cover the top-left of the cutting area where imports land.
fn pills(
    ui: &mut Ui,
    rect: Rect,
    snap: &Snapshot,
    st: &mut PrepareState,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let area = Rect::from_min_max(
        rect.min + egui::vec2(canvas::RULER + t.spacing, canvas::RULER + t.spacing),
        rect.max - egui::vec2(t.spacing, t.spacing),
    );
    let doc_id = snap.doc_id;
    let src = &mut st.sources;
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(area)
            .layout(egui::Layout::bottom_up(egui::Align::Min)),
        |ui| {
            let pill = |ui: &mut Ui, colour: egui::Color32, add: &mut dyn FnMut(&mut Ui)| {
                egui::Frame::new()
                    .fill(t.surface)
                    .stroke(egui::Stroke::new(1.0, colour))
                    .corner_radius(t.radius * 2)
                    .inner_margin(egui::Margin::symmetric(t.spacing as i8 * 2, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| add(ui));
                    });
            };
            if let Some(w) = &snap.profile_problem {
                pill(ui, t.danger, &mut |ui| {
                    ui.label(w.as_str());
                });
            }
            if let Some(w) = &snap.mat_warning {
                pill(ui, t.accent_cut, &mut |ui| {
                    ui.label(w.as_str());
                });
            }
            if snap.uncalibrated_mat() {
                pill(ui, t.accent_cut, &mut |ui| {
                    ui.label("This mat is not calibrated yet").on_hover_text(
                        "Objects keep 5 mm from the edge of the grid until the mat is calibrated.",
                    );
                });
            }
            let outside = snap.outside_area();
            if outside > 0 {
                let text = if outside == 1 {
                    "1 object is outside the cutting area".to_string()
                } else {
                    format!("{outside} objects are outside the cutting area")
                };
                pill(ui, t.danger, &mut |ui| {
                    ui.label(text.as_str())
                        .on_hover_text("Even the Millennium Falcon needs a hangar. Move it onto the mat.");
                });
            }
            if !st.notices.is_empty() {
                let first = st.notices[0].clone();
                let more = st.notices.len() - 1;
                let mut dismiss = false;
                pill(ui, t.accent_cut, &mut |ui| {
                    ui.label(first.as_str());
                    if more > 0 {
                        ui.label(RichText::new(format!("and {more} more")).color(t.text_dim));
                    }
                    dismiss = ui.button("Dismiss").clicked();
                });
                if dismiss {
                    st.notices.clear();
                }
            }
            if !src.changed.is_empty() {
                let text = plural(src.changed.len(), "source file changed", "source files changed");
                let ids = src.changed.clone();
                pill(ui, t.accent_prepare, &mut |ui| {
                    ui.label(text.as_str());
                    if ui
                        .button("Reload changed files")
                        .on_hover_text("Fresh from the workshop. Pull in the new version.")
                        .clicked()
                    {
                        actions.push(Action::run(doc_id, "object.reload", json!({ "ids": ids })));
                    }
                });
            }
            if !src.missing.is_empty() || !src.unreadable.is_empty() {
                // The selected broken object first, else the first one.
                let target = snap
                    .selection
                    .iter()
                    .copied()
                    .find(|id| src.broken(*id))
                    .or_else(|| src.missing.first().copied())
                    .or_else(|| src.unreadable.first().copied());
                pill(ui, t.danger, &mut |ui| {
                    if !src.missing.is_empty() {
                        ui.label(plural(
                            src.missing.len(),
                            "source file not found",
                            "source files not found",
                        ))
                        .on_hover_text("Your file moved out. Reconnect? The project keeps working from its own copy.");
                    }
                    if !src.unreadable.is_empty() {
                        ui.label(plural(
                            src.unreadable.len(),
                            "source file can't be read",
                            "source files can't be read",
                        ));
                    }
                    if let Some(id) = target
                        && ui.button("Reconnect...").clicked()
                    {
                        actions.push(Action::Reconnect { doc_id, id });
                    }
                });
            }
            if !src.also_found.is_empty() {
                let n = src.also_found.len();
                let text = if n == 1 {
                    "1 more file found in that folder".to_string()
                } else {
                    format!("{n} more files found in that folder")
                };
                let items = src.also_found.clone();
                let mut dismiss = false;
                pill(ui, t.accent_prepare, &mut |ui| {
                    ui.label(text.as_str());
                    if ui.button("Reconnect them too").clicked() {
                        actions.push(Action::ReconnectAll {
                            doc_id,
                            items: items.clone(),
                        });
                        dismiss = true;
                    }
                    dismiss |= ui.button("No thanks").clicked();
                });
                if dismiss {
                    src.also_found.clear();
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatcraft_core::geometry::{CutPath, Paint};

    /// A ring drawn back to its start before `Z`, with a doubled corner,
    /// is four corners: no repeated point and no closing point.
    #[test]
    fn a_closed_shape_has_no_repeated_points() {
        let mut path = kurbo::BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((10.0, 0.0));
        path.line_to((10.0, 0.0));
        path.line_to((10.0, 10.0));
        path.line_to((0.0, 10.0));
        path.line_to((0.0, 0.0));
        path.close_path();
        let geometry = vec![CutPath {
            path,
            color: None,
            label: None,
            paint: Paint::default(),
        }];
        let input = ShapeInput {
            local_bounds: flatcraft_core::geometry::bounds(&geometry).unwrap(),
            geometry: Arc::new(geometry),
            transform: Transform::default(),
            cut: CutOptions::default(),
        };
        let shape = build_shape(&input);
        assert_eq!(shape.lines.len(), 1);
        let line = &shape.lines[0];
        assert!(line.closed);
        assert_eq!(line.points.len(), 4, "{:?}", line.points);
    }
}
