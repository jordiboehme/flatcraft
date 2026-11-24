//! The Prepare workspace: the Objects panel on the left, the Settings panel
//! on the right, the canvas in the middle and its pills.
//!
//! Each frame starts with a [`Snapshot`] of the active document, taken
//! under one short lock. Everything is drawn from the snapshot; changes go
//! out as [`Action`]s that name the document they were made for.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Rect, RichText, Ui};
use flatcraft_core::document::{CutOptions, JobSettings, Object, Source};
use flatcraft_core::geometry::CutPath;
use flatcraft_core::transform::Transform;
use flatcraft_device::{MaterialProfile, Media, MediaKind, PlotterProfile, PresetInfo};
use flatcraft_engine::commands::placed_bounds;
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
    /// A corner handle of a single selected object.
    Scale {
        doc_id: u64,
        id: u64,
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
    pub last_check: Option<Instant>,
    /// Kinds of check errors already shown, so each kind toasts once.
    pub reported: Vec<String>,
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
    pub cache: ShapeCache,
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
            cache: ShapeCache::default(),
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

impl ShapeKey {
    fn of(o: &Object) -> Self {
        let t = &o.transform;
        Self {
            geometry: Arc::as_ptr(&o.geometry) as usize,
            scale_x: t.scale_x,
            scale_y: t.scale_y,
            rotation_deg: t.rotation_deg,
            mirror: t.mirror,
            local_bounds: o.local_bounds,
            cut: o.cut,
        }
    }
}

/// A cached shape, its key and the geometry the key points to.
type CacheEntry = (ShapeKey, Arc<Vec<CutPath>>, Arc<Shape>);

/// Shapes of the active document by object id.
#[derive(Debug, Clone, Default)]
pub struct ShapeCache {
    doc_id: u64,
    /// The geometry `Arc` is kept so its address cannot be reused by
    /// other geometry while the key names it.
    shapes: HashMap<u64, CacheEntry>,
}

/// The shape of an object, built from the engine's placed paths and bounds
/// with the object's centre moved to 0,0.
fn build_shape(mut o: Object) -> Shape {
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
                    if let Some(l) = &mut current {
                        l.points.push(pt);
                    }
                }
                kurbo::PathEl::ClosePath => {
                    if let Some(mut l) = current.take() {
                        l.closed = true;
                        let start = l.points[0];
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
    pub auto_reload: bool,
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

    /// Visible objects that are not inside the cutting area.
    pub fn outside_area(&self) -> usize {
        const SLACK: f64 = 1e-6;
        let Some(a) = self.area else {
            return 0;
        };
        self.objects
            .iter()
            .filter(|o| o.visible)
            .filter_map(ObjView::bounds)
            .filter(|b| {
                b.x0 < a.x0 - SLACK
                    || b.y0 < a.y0 - SLACK
                    || b.x1 > a.x1 + SLACK
                    || b.y1 > a.y1 + SLACK
            })
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
pub fn snapshot(app: &mut FlatCraftApp) -> Snapshot {
    let cache = &mut app.ui.prepare.cache;
    let mut misses: Vec<(usize, Object)> = Vec::new();
    let mut snap = {
        let s = lock(&app.session);
        let st = s.doc();
        if cache.doc_id != st.id() {
            cache.doc_id = st.id();
            cache.shapes.clear();
        }
        let job = st.document.job.clone();
        let plotter = s.profiles.plotter(&job.plotter_id).ok();
        let material = s
            .profiles
            .material(&job.material_id)
            .ok()
            .map(|m| m.for_plotter(&job.plotter_id));
        let mat = plotter
            .as_ref()
            .and_then(|p| p.media.iter().find(|m| m.id == job.media_id).cloned());
        let area = plotter.as_ref().and_then(|p| p.cutting_area(&job.media_id));
        let mut objects = Vec::with_capacity(st.document.objects.len());
        for (i, o) in st.document.objects.iter().enumerate() {
            let key = ShapeKey::of(o);
            let shape = match cache.shapes.get(&o.id.0) {
                Some((k, _, shape)) if *k == key => shape.clone(),
                _ => {
                    misses.push((i, o.clone()));
                    Arc::new(Shape::default())
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
                shape,
            });
        }
        Snapshot {
            doc_id: st.id(),
            objects,
            selection: st.selection.iter().map(|i| i.0).collect(),
            job,
            plotter,
            material,
            plotters: s.profiles.plotters(),
            materials: s.profiles.materials(),
            mat,
            area,
            auto_reload: s.prefs.auto_reload,
        }
    };
    for (i, o) in misses {
        let key = ShapeKey::of(&o);
        let id = o.id.0;
        let geometry = o.geometry.clone();
        let shape = Arc::new(build_shape(o));
        cache.shapes.insert(id, (key, geometry, shape.clone()));
        snap.objects[i].shape = shape;
    }
    let ids: BTreeSet<u64> = snap.objects.iter().map(|o| o.id).collect();
    cache.shapes.retain(|id, _| ids.contains(id));

    let src = &mut app.ui.prepare.sources;
    if src.doc_id != snap.doc_id {
        // The lists belong to the old document. Check the new one at once
        // (not the very first document: a new window has nothing linked,
        // and opening a project asks for its own check).
        let switched = src.doc_id != 0;
        *src = SourceStatus {
            doc_id: snap.doc_id,
            check_pending: src.check_pending || switched,
            check_now: switched,
            last_check: src.last_check,
            reported: std::mem::take(&mut src.reported),
            ..SourceStatus::default()
        };
    }
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

/// A short name for the kind of a command error.
fn error_kind(e: &flatcraft_engine::CommandError) -> &'static str {
    use flatcraft_engine::CommandError as E;
    match e {
        E::UnknownCommand(_) => "unknown",
        E::InvalidParams(_) => "invalid",
        E::NotFound(_) => "not found",
        E::Refused(_) => "refused",
        E::Failed(_) => "failed",
    }
}

impl FlatCraftApp {
    /// Runs a pending check of linked files unless a drag is in progress or
    /// the last check was less than [`CHECK_INTERVAL`] ago. With the
    /// `auto_reload` preference on, changed files are reloaded right away
    /// (only those; never missing or unreadable ones).
    pub fn check_sources(&mut self, ctx: &egui::Context, now: Instant) {
        let p = &mut self.ui.prepare;
        if !p.sources.check_pending || p.drag.is_some() {
            return;
        }
        if !p.sources.check_now
            && let Some(last) = p.sources.last_check
        {
            let since = now.saturating_duration_since(last);
            if since < CHECK_INTERVAL {
                ctx.request_repaint_after(CHECK_INTERVAL - since);
                return;
            }
        }
        p.sources.check_pending = false;
        p.sources.check_now = false;
        p.sources.last_check = Some(now);
        let (doc_id, result, auto) = {
            let mut s = lock(&self.session);
            let doc_id = s.doc().id();
            let auto = s.prefs.auto_reload;
            let r = s.execute(
                "document.check_sources",
                json!({}),
                flatcraft_engine::Origin::User,
            );
            (doc_id, r, auto)
        };
        let v = match result {
            Ok(v) => v,
            Err(e) => {
                eprintln!("warning: cannot check the linked files: {e}");
                let kind = error_kind(&e).to_string();
                if !self.ui.prepare.sources.reported.contains(&kind) {
                    self.ui.prepare.sources.reported.push(kind);
                    self.toast(super::Toast::warning(format!(
                        "Cannot check the linked files: {}",
                        super::person_text(&e)
                    )));
                }
                return;
            }
        };
        let ids = |k: &str| -> Vec<u64> {
            v[k].as_array()
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_u64)
                .collect()
        };
        let src = &mut self.ui.prepare.sources;
        src.doc_id = doc_id;
        src.changed = ids("changed");
        src.missing = ids("missing");
        src.unreadable = ids("unreadable");
        if auto && !src.changed.is_empty() {
            let changed = src.changed.clone();
            src.auto_reloaded = changed.clone();
            self.apply(
                ctx,
                vec![Action::run(
                    doc_id,
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
