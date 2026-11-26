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

use egui::{Pos2, Rect, Ui};
use flatcraft_core::document::{CutOptions, JobSettings, Object, ObjectId, Source};
use flatcraft_core::geometry::CutPath;
use flatcraft_core::nodes::Tree;
use flatcraft_core::transform::Transform;
use flatcraft_device::{MaterialProfile, Media, MediaKind, PlotterProfile, PresetInfo};
use flatcraft_engine::commands::{placed_bounds, sources};
use flatcraft_engine::job::JobInput;
use serde_json::json;

use super::arrange::{self, ArrangePanel, PanelValues};
use super::canvas::{self, CanvasView};
use super::rails::{self, Side, SidePanel};
use super::theme::Tokens;
use super::{Action, FlatCraftApp, lock, objects, settings};

/// Linked files are checked at most this often.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// Curves are drawn as polylines this close to the true curve, in mm.
const DRAW_TOLERANCE_MM: f64 = 0.02;

/// The ribbon tabs of Prepare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum RibbonTab {
    #[default]
    Layout,
    View,
}

impl RibbonTab {
    pub const ALL: [RibbonTab; 2] = [RibbonTab::Layout, RibbonTab::View];

    pub fn name(self) -> &'static str {
        match self {
            RibbonTab::Layout => "Layout",
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
    /// A handle of the selection box. Several objects scale as one group
    /// (`object.scale` with `group`). See `canvas::Stretch`.
    Scale {
        doc_id: u64,
        ids: Vec<u64>,
        /// 0..4 the corners (top-left, top-right, bottom-right,
        /// bottom-left), 4..8 the middles of the sides (top, right,
        /// bottom, left).
        handle: usize,
        /// The selection box when the drag began.
        start: kurbo::Rect,
        current: kurbo::Point,
        /// Keep aspect when the drag began.
        keep: bool,
        /// Width and height may change apart: one object, turned by a
        /// multiple of 90 degrees.
        free: bool,
        /// Shift is held: the opposite of `keep` for this drag.
        flip: bool,
        /// Option (Alt) is held: it scales about the centre.
        centre: bool,
    },
    /// A turn of the selection: from the rotate zone just outside a corner,
    /// or a corner handle with Cmd (macOS) or Ctrl held. One object turns
    /// about its own centre, several about the mean of their centres, as
    /// `object.rotate` does. See `canvas::rotate_delta`.
    Rotate {
        doc_id: u64,
        ids: Vec<u64>,
        /// The point it turns about, in mm.
        pivot: kurbo::Point,
        /// The pointer when the drag began, and now, in mm.
        start: kurbo::Point,
        current: kurbo::Point,
        /// The rotation of the one object turned (Shift snaps its angle),
        /// `None` for several (Shift snaps the turn).
        base: Option<f64>,
        /// Shift is held: 15 degree steps.
        snap: bool,
    },
    /// A selection frame drawn from the empty canvas. On release it selects
    /// the visible objects whose placed bounds it touches; with `add`
    /// (Shift) they join the selection.
    Frame {
        doc_id: u64,
        start: kurbo::Point,
        current: kurbo::Point,
        add: bool,
    },
}

/// Arrow-key presses that share one undo step (`object.move` with a
/// `coalesce_key`): a held key, or presses less than [`NUDGE_BURST`]
/// apart, on the same document and the same objects.
#[derive(Debug, Clone, PartialEq)]
pub struct Nudge {
    pub doc_id: u64,
    pub ids: Vec<u64>,
    /// Numbers the bursts of this window; the coalesce key.
    pub burst: u64,
    /// When the last press of the burst came.
    pub last: Instant,
}

/// Arrow-key presses closer than this join one undo step.
pub const NUDGE_BURST: Duration = Duration::from_millis(500);

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

/// A number field of the Position and size group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SizeField {
    X,
    Y,
    Width,
    Height,
    Percent,
    /// The rotation in degrees, clockwise.
    Angle,
}

/// The number field being edited: it applies when it loses the focus.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldEdit {
    pub field: SizeField,
    /// The text it showed when the editing began. An unchanged text
    /// applies nothing.
    pub shown: String,
    /// The document and the objects selected when the editing began.
    pub doc_id: u64,
    pub ids: Vec<u64>,
    /// Width and height of those objects may change apart (see
    /// `canvas::free_aspect`).
    pub free: bool,
    /// The value it showed, unrounded, for a relative edit (`+=10`);
    /// `None` when there was no single value (Mixed).
    pub value: Option<f64>,
}

/// Edit buffers of the ribbon's number fields.
#[derive(Debug, Clone)]
pub struct Fields {
    /// The top-left corner of the selection box, in mm from the grid's 0,0.
    pub x: String,
    pub y: String,
    pub width: String,
    pub height: String,
    pub percent: String,
    /// The rotation in degrees; empty while the selection is turned
    /// different ways (the field says Mixed).
    pub angle: String,
    pub keep_aspect: bool,
    /// Rotate and mirror each object about its own centre instead of the
    /// selection as one group.
    pub each: bool,
    /// Align to the cutting area instead of the selection.
    pub align_to_area: bool,
    pub search: String,
    /// The field being edited, if any.
    pub editing: Option<FieldEdit>,
    /// A field whose text could not be read, and why: it keeps the text
    /// and the focus, with a red outline and this message under it.
    pub error: Option<(SizeField, String)>,
}

impl Default for Fields {
    fn default() -> Self {
        Self {
            x: String::new(),
            y: String::new(),
            width: String::new(),
            height: String::new(),
            percent: String::new(),
            angle: String::new(),
            keep_aspect: true,
            each: false,
            align_to_area: false,
            search: String::new(),
            editing: None,
            error: None,
        }
    }
}

/// Speed, force or passes, the job settings a document can override.
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
    /// Room for the test shape, per document revision (Job card hint).
    pub test_cut_cache: super::materials::TestCutCache,
    /// The material picker's entries (see `materials::CardsCache`).
    pub cards_cache: super::materials::CardsCache,
    /// Fit the mat into the canvas on the next frame.
    pub fit_pending: bool,
    /// The document the view was fitted for.
    pub view_doc: Option<u64>,
    /// The canvas area of the last frame, in points.
    pub canvas_rect: Rect,
    pub tab: RibbonTab,
    /// Which side panels are collapsed (see `rails`).
    pub panels: super::rails::Panels,
    pub show_grid: bool,
    pub show_rulers: bool,
    /// The objects (document id, object id, material id) the stencil
    /// hint was shown for (`stencil_island`): it shows once per object
    /// and material.
    pub stencil_told: std::collections::BTreeSet<(u64, u64, String)>,
    /// The kept and waste overlay on the Prepare canvas (View, Show). Off
    /// until switched on; the Cut workspace always shows it.
    pub show_kept_waste: bool,
    /// The shared sheet is kept (see `cut::sheet_kept_now`), for the
    /// Weeding section. Set each frame.
    pub sheet_kept: bool,
    /// The keep picker on the canvas ("Pick what stays"), while it is on.
    pub keep_pick: Option<super::keep_pick::KeepPick>,
    pub fields: Fields,
    /// The search hits stay open (see `ribbon::search_box`).
    pub search_open: bool,
    pub drag: Option<Drag>,
    /// The burst of arrow-key nudges going on, if any.
    pub nudge: Option<Nudge>,
    /// A speed typed above the plotter's maximum (and clamped): the speed
    /// field says "These go to eleven." while that clamped value is still
    /// the document's speed override (see [`Eleven`]).
    pub speed_eleven: Option<Eleven>,
    /// The report of the last import, until dismissed or all fixed.
    pub report: Option<super::report::ImportCard>,
    /// Move the view to these objects on the next frame ("Show me").
    pub center_on: Option<Vec<u64>>,
    pub sources: SourceStatus,
    /// The link status of the other open documents, so switching back
    /// shows it at once and a quick switch does not check again.
    pub source_memory: HashMap<u64, SourceStatus>,
    pub cache: ShapeCache,
    /// The Arrange panel docked on the right, if one is open.
    pub panel: Option<ArrangePanel>,
    /// The values of the last panel that ran, for the next one.
    pub panel_last: PanelValues,
    /// The Auto arrange panel's last plan (see `arrange::AutoPlanCache`).
    pub auto_plan: super::arrange::AutoPlanCache,
    /// Open this group's list on the next frame (search asked for it).
    pub open_group: Option<super::tools::GroupId>,
    /// The screen area of the open Arrange panel, last frame.
    pub panel_rect: Option<Rect>,
    /// At the end of the last frame a text field outside the Arrange panel
    /// (the search box, a size field) had the keys. egui drops the focus on
    /// Escape before the key handler runs, so this is how the handler knows
    /// that the Escape only leaves that field.
    pub text_focus_elsewhere: bool,
    /// A popup (a ribbon group's list, a menu) was open at the end of the
    /// last frame, so an Escape now only closed it.
    pub popup_was_open: bool,
    /// The objects the open context menu works on (see `context_menu`).
    pub context: Option<Vec<u64>>,
    /// The group the person went into (double-click or Cmd-click on a
    /// member), with its document: clicks then pick its members one by
    /// one. Escape or a click outside it leaves it (see `canvas::pick`).
    pub entered: Option<(u64, u64)>,
    /// The groups folded open in the Objects panel, with their document.
    pub open_groups: BTreeSet<(u64, u64)>,
    /// Where the canvas painted its rotate cursor last frame (egui has no
    /// rotate cursor, so it hides the pointer and paints one), if it did.
    pub rotate_cursor: Option<Pos2>,
    /// The open Rename dialog.
    pub rename: Option<RenameDialog>,
    /// Where the ribbon drew its tab row, its panel and its tools row,
    /// last frame (see `ribbon::show`).
    pub ribbon_layout: Option<super::ribbon::Layout>,
    /// The Notes section under Objects is open.
    pub notes_open: bool,
    /// Counts the times a notes field got the keyboard: the typing of one
    /// visit to a field is one undo step (see `objects::notes`).
    pub notes_burst: u64,
}

/// The Rename dialog: its object and the name typed so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameDialog {
    pub doc_id: u64,
    pub id: u64,
    pub name: String,
}

/// The speed attempt above the maximum: the document, plotter and
/// material it was made for and the clamped value it became.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eleven {
    pub doc_id: u64,
    pub plotter_id: String,
    pub material_id: String,
    pub speed: u32,
}

impl Eleven {
    /// The attempt is still what the snapshot shows: same document,
    /// plotter and material, and the speed the job cuts at (the override,
    /// else the material's speed) is still the clamped value.
    pub fn current(&self, snap: &Snapshot) -> bool {
        self.doc_id == snap.doc_id
            && self.plotter_id == snap.job.plotter_id
            && self.material_id == snap.job.material_id
            && effective_speed(snap) == Some(self.speed)
    }
}

/// The speed the job cuts at: the override, else the material's own.
fn effective_speed(snap: &Snapshot) -> Option<u32> {
    snap.job
        .speed
        .or(snap.material.as_ref().map(|m| m.speed))
        .map(u32::from)
}

impl Default for PrepareState {
    fn default() -> Self {
        Self {
            view: CanvasView::default(),
            test_cut_cache: Default::default(),
            cards_cache: Default::default(),
            fit_pending: true,
            view_doc: None,
            canvas_rect: Rect::NOTHING,
            tab: RibbonTab::default(),
            panels: super::rails::Panels::default(),
            show_grid: true,
            show_rulers: true,
            show_kept_waste: false,
            keep_pick: None,
            sheet_kept: false,
            stencil_told: Default::default(),
            fields: Fields::default(),
            search_open: false,
            drag: None,
            nudge: None,
            speed_eleven: None,
            report: None,
            center_on: None,
            sources: SourceStatus::default(),
            source_memory: HashMap::new(),
            cache: ShapeCache::default(),
            panel: None,
            panel_last: PanelValues::default(),
            auto_plan: arrange::AutoPlanCache::default(),
            open_group: None,
            panel_rect: None,
            text_focus_elsewhere: false,
            popup_was_open: false,
            context: None,
            entered: None,
            open_groups: BTreeSet::new(),
            rotate_cursor: None,
            rename: None,
            ribbon_layout: None,
            notes_open: true,
            notes_burst: 0,
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
    /// The box Auto arrange packs or packs around, minus the centre (for
    /// an untraced picture its image): the engine's
    /// `centred_arrange_bounds`, so the preview matches the command.
    pub arrange: Option<kurbo::Rect>,
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
    frame: kurbo::Rect,
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
    /// The source's page frame in local mm (`Object::frame`).
    pub frame: kurbo::Rect,
    pub cut: CutOptions,
}

impl ShapeInput {
    fn of(o: &Object) -> Self {
        Self {
            geometry: o.geometry.clone(),
            transform: o.transform,
            local_bounds: o.local_bounds,
            frame: o.frame,
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
            frame: self.frame,
            cut: self.cut,
        }
    }

    fn elements(&self) -> usize {
        self.geometry.iter().map(|p| p.path.elements().len()).sum()
    }

    /// An object with only what drawing and bounds read.
    fn object(&self) -> Object {
        let mut o = Object::shape_only(
            self.geometry.clone(),
            self.local_bounds,
            self.transform,
            self.cut,
        );
        o.frame = self.frame;
        o
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
        arrange: flatcraft_engine::commands::centred_arrange_bounds(&o),
    }
}

/// One object as Prepare shows it.
#[derive(Debug, Clone)]
pub struct ObjView {
    pub id: u64,
    pub name: String,
    pub raster: bool,
    pub visible: bool,
    /// False: shown, but left out of the cut.
    pub cut_enabled: bool,
    pub transform: Transform,
    pub cut: CutOptions,
    /// What is kept after cutting (shared by linked copies).
    pub keep: flatcraft_core::document::KeepMode,
    /// Last known path of the linked file.
    pub link: Option<PathBuf>,
    /// The whole link, to find the file where Reload would read it
    /// ([`flatcraft_engine::commands::sources::find_linked_file`]).
    pub source_link: Option<flatcraft_core::document::SourceLink>,
    /// The group of linked copies the object belongs to.
    pub link_group: Option<u64>,
    /// The group of objects it is in (Group, Cmd+G), if any.
    pub parent: Option<u64>,
    /// The source bytes, shared with the document. See [`ObjView::same_source`].
    pub source: Arc<[u8]>,
    pub shape: Arc<Shape>,
}

impl ObjView {
    /// Same source, as `object.link` compares: the same allocation, else
    /// the same length and bytes (copies, and two imports of one file).
    pub fn same_source(&self, other: &ObjView) -> bool {
        Arc::ptr_eq(&self.source, &other.source) || *self.source == *other.source
    }

    /// The engine's bounds in mat mm, as `document.state` reports them.
    pub fn bounds(&self) -> Option<kurbo::Rect> {
        self.shape
            .bounds
            .map(|b| b + self.transform.center_mm.to_vec2())
    }

    /// The box Auto arrange packs or packs around, built as the engine's
    /// `arrange_bounds` (the cut geometry's box, or for an untraced
    /// picture its image), so both agree to the last bit. `None` while the
    /// shape is still being built.
    pub fn arrange_bounds(&self) -> Option<kurbo::Rect> {
        self.shape
            .arrange
            .map(|b| b + self.transform.center_mm.to_vec2())
    }
}

/// The active document as Prepare draws it.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub doc_id: u64,
    /// The document's revision (`DocState::revision`): it changes with
    /// every change of the document, undo and redo included.
    pub doc_rev: u64,
    pub objects: Vec<ObjView>,
    /// The groups of objects over `objects` (see `flatcraft_core::nodes`).
    pub tree: Tree,
    pub selection: BTreeSet<u64>,
    pub job: JobSettings,
    pub plotter: Option<PlotterProfile>,
    /// The material with the plotter's overrides applied.
    pub material: Option<MaterialProfile>,
    pub plotters: Vec<PresetInfo>,
    pub materials: Vec<PresetInfo>,
    /// The material picker's entries, in picker order.
    pub material_cards: Vec<super::materials::MaterialCard>,
    /// Set when the document's material was never cut on this computer:
    /// the Job card suggests a test cut.
    pub test_cut: Option<super::materials::TestCut>,
    /// Why the person's own preset file with the document's material id
    /// was skipped at load, when it was.
    pub material_skipped: Option<String>,
    pub mat: Option<Media>,
    /// The cutting area, in mm.
    pub area: Option<kurbo::Rect>,
    /// Set when the document's mat is gone and the plotter's first mat is
    /// used instead.
    pub mat_warning: Option<String>,
    /// Why the plotter or material preset cannot be used (a bad value).
    pub profile_problem: Option<String>,
    pub auto_reload: bool,
    /// The ribbon actions pinned as buttons (preference ids).
    pub ribbon_pins: Vec<String>,
    /// The spacing of the one-click Auto arrange, in mm (preferences).
    pub auto_spacing_mm: f64,
    /// The image or the trace settings of the Trace dialog's object
    /// changed since the dialog opened (an agent retraced or reloaded it).
    pub trace_source_changed: bool,
    /// What a job build reads, when the snapshot was asked for it (Cut,
    /// Debug panel). An error says why there is no job (a missing profile).
    pub job_input: Option<Result<JobInput, flatcraft_engine::CommandError>>,
    /// The output folder of the preferences.
    pub output_pref: Option<PathBuf>,
    pub config_dir: PathBuf,
    /// The document's project file, if it has one.
    pub project: Option<PathBuf>,
    /// The project's one-line description (empty when none).
    pub description: String,
    /// The project's notes (empty when none).
    pub notes: String,
    /// The cut the Job card recalls for the document's material (see
    /// `CutLog::last_time`).
    pub last_time: Option<flatcraft_engine::cutlog::CutEntry>,
    /// The newest cut with the document's material nobody rated yet (see
    /// `CutLog::to_rate`).
    pub to_rate: Option<flatcraft_engine::cutlog::CutEntry>,
    /// The newest cut with the document's material rated good (see
    /// `CutLog::last_good`), for the "It didn't cut right..." helper.
    pub last_good: Option<flatcraft_engine::cutlog::CutEntry>,
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

    /// What moves as one in the selection: whole groups, and objects
    /// picked inside a group or outside any (see `Tree::units`).
    pub fn units(&self) -> Vec<u64> {
        self.units_of(&self.selection.iter().copied().collect::<Vec<_>>())
    }

    /// What moves as one for the objects `ids`.
    pub fn units_of(&self, ids: &[u64]) -> Vec<u64> {
        let leaves: Vec<ObjectId> = ids.iter().map(|i| ObjectId(*i)).collect();
        self.tree.units(&leaves).into_iter().map(|u| u.0).collect()
    }

    /// The objects of a unit: the object itself, or all of a group's.
    pub fn leaves_of(&self, unit: u64) -> Vec<u64> {
        self.tree
            .leaves_of(ObjectId(unit))
            .into_iter()
            .map(|u| u.0)
            .collect()
    }

    pub fn is_group(&self, id: u64) -> bool {
        self.tree.is_group(ObjectId(id))
    }

    /// The common box of the objects `ids` (the engine's bounds).
    /// Hidden objects do not count, unless all of them are hidden, as the
    /// engine's `common_box`.
    pub fn box_of(&self, ids: &[u64]) -> Option<kurbo::Rect> {
        let objs: Vec<&ObjView> = ids.iter().filter_map(|id| self.object(*id)).collect();
        let any_shown = objs.iter().any(|o| o.visible);
        objs.into_iter()
            .filter(|o| o.visible || !any_shown)
            .filter_map(ObjView::bounds)
            .reduce(|a, b| a.union(b))
    }

    /// The point a unit turns about on its own, as `object.rotate` has it:
    /// an object's centre, a group's box centre.
    pub fn unit_centre(&self, unit: u64) -> Option<kurbo::Point> {
        match self.object(unit) {
            Some(o) => Some(o.transform.center_mm),
            None => self.box_of(&self.leaves_of(unit)).map(|b| b.center()),
        }
    }

    /// The angle of a unit as the Angle field shows it: an object's
    /// rotation, a group's own angle.
    pub fn unit_angle(&self, unit: u64) -> Option<f64> {
        match self.object(unit) {
            Some(o) => Some(o.transform.rotation_deg),
            None => self.tree.group(ObjectId(unit)).map(|g| g.rotation_deg),
        }
    }

    /// The name of an object or a group.
    pub fn name_of(&self, id: u64) -> Option<String> {
        match self.object(id) {
            Some(o) => Some(o.name.clone()),
            None => self.tree.group(ObjectId(id)).map(|g| g.name.clone()),
        }
    }

    /// How many different linked files the objects `ids` have: linked
    /// copies, and plain copies of one import, share one file. An object
    /// without a known file counts on its own.
    pub fn files_of(&self, ids: &[u64]) -> usize {
        let mut files: BTreeSet<&std::path::Path> = BTreeSet::new();
        let mut others = 0;
        for id in ids {
            match self.object(*id).and_then(|o| o.link.as_deref()) {
                Some(path) => {
                    files.insert(path);
                }
                None => others += 1,
            }
        }
        files.len() + others
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

    /// Visible objects that are cut and not inside the cutting area.
    pub fn outside_area(&self) -> usize {
        let Some(a) = self.area else {
            return 0;
        };
        self.objects
            .iter()
            .filter(|o| o.visible && o.cut_enabled)
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
    let test_cut_cache = &mut app.ui.prepare.test_cut_cache;
    let cards_cache = &mut app.ui.prepare.cards_cache;
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
        let cut_log = s.cut_log();
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
                cut_enabled: o.cut_enabled,
                transform: o.transform,
                cut: o.cut,
                keep: o.keep,
                link: o.link.as_ref().map(|l| l.path.clone()),
                source_link: o.link.clone(),
                link_group: o.link_group.map(|g| g.0),
                parent: o.parent.map(|g| g.0),
                source: match &o.source {
                    Source::Svg { bytes, .. } | Source::Raster { bytes, .. } => bytes.clone(),
                },
                shape,
            });
        }
        Snapshot {
            doc_id: st.id(),
            doc_rev: st.revision(),
            objects,
            tree: st.document.tree(),
            selection: st.selection.iter().map(|i| i.0).collect(),
            job,
            plotter: plotter.clone(),
            material: material.clone(),
            plotters: s.profiles.plotters(),
            materials: s.profiles.materials(),
            material_cards: cards_cache.get(&s.profiles, &cut_log),
            material_skipped: s
                .profiles
                .diagnostics()
                .iter()
                .find(|d| {
                    d.path
                        .parent()
                        .and_then(|p| p.file_name())
                        .is_some_and(|n| n == "materials")
                        && d.path
                            .file_stem()
                            .is_some_and(|f| *f == *st.document.job.material_id)
                })
                .map(|d| d.reason.clone()),
            test_cut: (!cut_log.has_cut(&st.document.job.material_id))
                .then(|| {
                    test_cut_cache.room(st.id(), st.revision(), &plotter, &material, || {
                        match (&plotter, &material) {
                            (Some(p), Some(m)) => {
                                let input = JobInput::from_parts(&st.document, p.clone(), m);
                                flatcraft_engine::troubleshoot::test_shape_spot(&input).is_some()
                            }
                            // Without a job the button says why when it runs.
                            _ => true,
                        }
                    })
                })
                .map(|room| super::materials::TestCut { room }),
            mat,
            area,
            mat_warning,
            profile_problem,
            auto_reload: s.prefs.auto_reload,
            ribbon_pins: s.prefs.ribbon_pins.clone(),
            auto_spacing_mm: s.prefs.auto_arrange_spacing_mm,
            trace_source_changed,
            job_input: want_job.then(|| match (&plotter, &material) {
                (Some(p), Some(m)) => Ok(JobInput::from_parts(&st.document, p.clone(), m)),
                _ => JobInput::of(&s),
            }),
            output_pref: s.prefs.output_dir.clone(),
            config_dir: s.config_dir.clone(),
            project: st.path.clone(),
            description: st.document.description.clone(),
            notes: st.document.notes.clone(),
            last_time: cut_log.last_time(&st.document.job.material_id).cloned(),
            to_rate: cut_log.to_rate(&st.document.job.material_id).cloned(),
            last_good: cut_log.last_good(&st.document.job.material_id).cloned(),
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
    arrange::keep_or_close(&mut app.ui.prepare, snap);
    // Another document or another selection cancels the keep picker.
    if app
        .ui
        .prepare
        .keep_pick
        .as_ref()
        .is_some_and(|k| !k.still_on(snap))
    {
        app.ui.prepare.keep_pick = None;
    }
    if let Some(pick) = &mut app.ui.prepare.keep_pick {
        pick.refresh(snap);
    }
    app.ui.prepare.panels.arrange_open = app.ui.prepare.panel.is_some();
    if app.ui.prepare.panels.collapsed(SidePanel::Objects) {
        let summary = rails::objects_summary(snap);
        if rails::show_rail(ui, SidePanel::Objects, Side::Left, &t, &summary) {
            app.ui
                .prepare
                .panels
                .set_collapsed(SidePanel::Objects, false);
        }
    } else {
        egui::Panel::left("flatcraft-objects")
            .resizable(true)
            .default_size(240.0)
            .frame(egui::Frame::new().fill(t.panel).inner_margin(t.spacing))
            .show(ui, |ui| {
                objects::show(ui, &mut app.ui.prepare, snap, &t, actions);
            });
    }

    if app.ui.prepare.panel.is_some() {
        let shown = egui::Panel::right("flatcraft-arrange-panel")
            .resizable(false)
            .exact_size(270.0)
            .frame(egui::Frame::new().fill(t.panel).inner_margin(t.spacing))
            .show(ui, |ui| {
                arrange::panel_ui(ui, &mut app.ui.prepare, snap, &t, actions);
            });
        app.ui.prepare.panel_rect = Some(shown.response.rect);
    } else {
        app.ui.prepare.panel_rect = None;
    }

    if app.ui.prepare.panels.collapsed(SidePanel::Settings) {
        let summary = rails::settings_summary(snap, rails::job_dot(snap, Some(&app.ui.cut)));
        if rails::show_rail(ui, SidePanel::Settings, Side::Right, &t, &summary) {
            app.ui
                .prepare
                .panels
                .set_collapsed(SidePanel::Settings, false);
        }
    } else {
        egui::Panel::right("flatcraft-settings")
            .resizable(true)
            .default_size(260.0)
            .frame(egui::Frame::new().fill(t.panel).inner_margin(t.spacing))
            .show(ui, |ui| {
                settings::show(ui, &mut app.ui.prepare, snap, &t, actions);
            });
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(t.bg))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            canvas::show(
                ui,
                rect,
                snap,
                &mut app.ui.prepare,
                &mut app.ui.cut,
                &t,
                actions,
            );
            pills(ui, rect, snap, &mut app.ui.prepare, &t, actions);
            let job = app.ui.cut.job();
            let p = &mut app.ui.prepare;
            if let Some(pick) = &mut p.keep_pick {
                let elsewhere = job.as_ref().is_some_and(|j| pick.sheet_kept_elsewhere(j));
                match pick.bar(ui, rect, snap, job.is_some(), elsewhere, &t) {
                    Some(super::keep_pick::Outcome::Done) => {
                        actions.extend(pick.done_action(snap));
                        p.keep_pick = None;
                    }
                    Some(super::keep_pick::Outcome::Cancel) => p.keep_pick = None,
                    None => {}
                }
            }
            let p = &mut app.ui.prepare;
            if let Some(card) = p.report.as_ref().filter(|c| c.doc_id == snap.doc_id)
                && !super::report::import_card(ui, rect, card, &t, actions)
            {
                p.report = None;
            }
        });
    let p = &mut app.ui.prepare;
    rename_window(ui.ctx(), &mut p.rename, snap, &t, actions);
}

/// The card of the Rename dialog's Rename button.
pub const RENAME_HINT: &str = "Give the object the name in the field.";
/// The card of its Cancel button.
pub const RENAME_CANCEL_HINT: &str = "Keep the old name.";
/// Why Rename is dimmed with an empty field.
pub const RENAME_NEEDS_NAME: &str = "Type a name first. An object needs a name you can find again.";

/// The Rename dialog (the shared [`super::dialog`]). It closes when its
/// document is no longer shown or its object (or group) is gone. Enter or Rename
/// sends `object.rename`; Escape, × and Cancel close it.
fn rename_window(
    ctx: &egui::Context,
    slot: &mut Option<RenameDialog>,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let Some(d) = slot else {
        return;
    };
    // An object or a group.
    if d.doc_id != snap.doc_id || snap.name_of(d.id).is_none() {
        *slot = None;
        return;
    }
    let dialog = super::dialog::Dialog::new("rename", super::dialog::Kind::Info, "Rename")
        .body("A name you will find again in the Objects panel.")
        .primary("Rename")
        .primary_hint(RENAME_HINT)
        .cancel_hint(RENAME_CANCEL_HINT);
    // Whether Rename can run is decided after the field took this frame's
    // typing, so the button and Enter see the name as it is now.
    let answer = dialog.show_with(ctx, t, |ui| {
        ui.add_space(t.spacing);
        let field = ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(f32::INFINITY));
        settings::set_name(ui, &field, "Name");
        if !field.has_focus() && !field.lost_focus() {
            field.request_focus();
        }
        d.name
            .trim()
            .is_empty()
            .then(|| RENAME_NEEDS_NAME.to_string())
    });
    match answer {
        Some(super::dialog::Answer::Primary) => {
            actions.push(Action::run(
                d.doc_id,
                "object.rename",
                json!({ "id": d.id, "name": d.name }),
            ));
            *slot = None;
        }
        Some(_) => *slot = None,
        None => {}
    }
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
            if !src.changed.is_empty() {
                let text = plural(
                    snap.files_of(&src.changed),
                    "source file changed",
                    "source files changed",
                );
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
                            snap.files_of(&src.missing),
                            "source file not found",
                            "source files not found",
                        ))
                        .on_hover_text("Your file moved out. Reconnect? The project keeps working from its own copy.");
                    }
                    if !src.unreadable.is_empty() {
                        ui.label(plural(
                            snap.files_of(&src.unreadable),
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
            frame: kurbo::Rect::ZERO,
            cut: CutOptions::default(),
        };
        let shape = build_shape(&input);
        assert_eq!(shape.lines.len(), 1);
        let line = &shape.lines[0];
        assert!(line.closed);
        assert_eq!(line.points.len(), 4, "{:?}", line.points);
    }
}
