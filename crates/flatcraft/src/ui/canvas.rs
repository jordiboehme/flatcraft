//! The mat canvas: the mat in its colour, the printed grid, rulers, the
//! objects and the selection. It holds no geometry logic: it draws the
//! placed paths the engine gives (see `prepare::Snapshot`) and converts
//! between screen points and millimetres with [`CanvasView`]. Moves and
//! scales are sent as one registry command when the drag ends.

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use flatcraft_device::MediaKind;
use serde_json::json;

use super::context_menu;
use super::prepare::{Drag, NUDGE_BURST, Nudge, ObjView, PrepareState, Snapshot};
use super::theme::Tokens;
use super::{Action, fonts};

/// Points per millimetre at 100 %: the CSS reference of 96 points per inch.
pub const PX_PER_MM_100: f32 = 96.0 / 25.4;
/// Zoom limits, in points per millimetre.
pub const MIN_PX_PER_MM: f32 = 0.1;
pub const MAX_PX_PER_MM: f32 = 200.0;
/// Distance of the printed grid lines.
pub const GRID_MM: f64 = 10.0;
/// Width of the rulers, in points.
pub const RULER: f32 = 18.0;
/// Half size of a selection handle, in points.
pub const HANDLE: f32 = 4.0;
/// Pick distance around a handle, in points.
pub const HANDLE_PICK: f32 = 7.0;

/// Pan and zoom of the mat canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasView {
    /// Screen position of the mm origin (the grid's 0,0), in points.
    pub offset: Vec2,
    /// Screen points per millimetre.
    pub px_per_mm: f32,
}

impl Default for CanvasView {
    fn default() -> Self {
        Self {
            offset: Vec2::ZERO,
            px_per_mm: 2.0,
        }
    }
}

impl CanvasView {
    pub fn screen_to_mm(&self, p: Pos2) -> kurbo::Point {
        let s = f64::from(self.px_per_mm);
        kurbo::Point::new(
            (f64::from(p.x) - f64::from(self.offset.x)) / s,
            (f64::from(p.y) - f64::from(self.offset.y)) / s,
        )
    }

    pub fn mm_to_screen(&self, p: kurbo::Point) -> Pos2 {
        let s = f64::from(self.px_per_mm);
        pos2(
            (p.x * s + f64::from(self.offset.x)) as f32,
            (p.y * s + f64::from(self.offset.y)) as f32,
        )
    }

    pub fn rect_to_screen(&self, r: kurbo::Rect) -> Rect {
        Rect::from_two_pos(
            self.mm_to_screen(kurbo::Point::new(r.x0, r.y0)),
            self.mm_to_screen(kurbo::Point::new(r.x1, r.y1)),
        )
    }

    /// Zooms by `factor`, keeping the millimetre under `at` in place.
    pub fn zoom_about(&mut self, at: Pos2, factor: f32) {
        let mm = self.screen_to_mm(at);
        self.px_per_mm = (self.px_per_mm * factor).clamp(MIN_PX_PER_MM, MAX_PX_PER_MM);
        let now = self.mm_to_screen(mm);
        self.offset += at - now;
    }

    /// The view that shows `mm` as large as fits into `screen`, centred.
    pub fn fit(mm: kurbo::Rect, screen: Rect) -> Self {
        let w = mm.width().max(1.0) as f32;
        let h = mm.height().max(1.0) as f32;
        let px = (screen.width() / w)
            .min(screen.height() / h)
            .clamp(MIN_PX_PER_MM, MAX_PX_PER_MM);
        let centre = mm.center();
        let offset = screen.center().to_vec2() - vec2(centre.x as f32 * px, centre.y as f32 * px);
        Self {
            offset,
            px_per_mm: px,
        }
    }
}

// ---------------------------------------------------------------- colours

pub use flatcraft_core::colour::{
    contrast_colour, contrast_ratio, cut_colour, luminance, margin_colour, object_colour,
    selection_colour,
};

pub fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

// ---------------------------------------------------------------- drawing

/// What the canvas found under the pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Hit {
    /// Handle `0..8` of the selection box: the corners (top-left,
    /// top-right, bottom-right, bottom-left), then the middles of the sides
    /// (top, right, bottom, left).
    Handle(usize),
    Object(u64),
    Nothing,
}

/// The anchor names `object.scale` takes, in handle order.
const ANCHORS: [&str; 8] = [
    "top_left",
    "top_right",
    "bottom_right",
    "bottom_left",
    "top",
    "right",
    "bottom",
    "left",
];

/// The handle points of a box: the corners, then the middles of the sides
/// (see [`Hit::Handle`]).
fn handle_points(r: kurbo::Rect) -> [kurbo::Point; 8] {
    let c = r.center();
    [
        kurbo::Point::new(r.x0, r.y0),
        kurbo::Point::new(r.x1, r.y0),
        kurbo::Point::new(r.x1, r.y1),
        kurbo::Point::new(r.x0, r.y1),
        kurbo::Point::new(c.x, r.y0),
        kurbo::Point::new(r.x1, c.y),
        kurbo::Point::new(c.x, r.y1),
        kurbo::Point::new(r.x0, c.y),
    ]
}

/// The handle across the box from `handle`.
fn opposite(handle: usize) -> usize {
    if handle < 4 {
        (handle + 2) % 4
    } else {
        4 + (handle - 4 + 2) % 4
    }
}

/// A side of the box shorter than this on screen has no middle handle:
/// it would crowd the corners.
const SIDE_HANDLE_MIN: f32 = 4.0 * HANDLE_PICK;

/// Whether handle `i` is shown for a box of `screen` size.
fn handle_shown(i: usize, screen: Rect) -> bool {
    match i {
        4 | 6 => screen.width() >= SIDE_HANDLE_MIN,
        5 | 7 => screen.height() >= SIDE_HANDLE_MIN,
        _ => true,
    }
}

/// The common box of the selected objects, in mm.
pub fn selection_box(snap: &Snapshot) -> Option<kurbo::Rect> {
    snap.objects
        .iter()
        .filter(|o| snap.selection.contains(&o.id))
        .filter_map(ObjView::bounds)
        .reduce(|a, b| a.union(b))
}

/// The selection box as it is right now: during a drag, where the drag
/// has it. The Position and size fields and the selection frame show it.
pub fn live_box(snap: &Snapshot, st: &PrepareState) -> Option<kurbo::Rect> {
    let b = selection_box(snap)?;
    Some(match &st.drag {
        Some(Drag::Move { start, current, .. }) => b + (*current - *start),
        Some(d @ Drag::Scale { .. }) => drag_stretch(d).map_or(b, |s| s.apply(b)),
        _ => b,
    })
}

/// The scale in percent a handle drag has reached, when it keeps the
/// shape.
pub fn live_percent(st: &PrepareState) -> Option<f64> {
    st.drag
        .as_ref()
        .and_then(drag_stretch)
        .filter(Stretch::uniform)
        .map(|s| s.fx * 100.0)
}

/// Whether width and height of these objects can change apart: one
/// object, turned by a multiple of 90 degrees (`object.scale` refuses
/// other angles, and a group keeps its shape).
pub fn free_aspect(snap: &Snapshot, ids: &[u64]) -> bool {
    match ids {
        [id] => snap.object(*id).is_some_and(|o| {
            let q = o.transform.rotation_deg / 90.0;
            (q - q.round()).abs() < 1e-9
        }),
        _ => false,
    }
}

/// The factor a corner drag scales by when it keeps the shape: the pointer
/// projected onto the diagonal from the fixed point.
fn corner_factor(anchor: kurbo::Point, corner: kurbo::Point, pointer: kurbo::Point) -> f64 {
    let d = corner - anchor;
    let len2 = d.hypot2();
    if len2 <= 1e-12 {
        return 1.0;
    }
    ((pointer - anchor).dot(d) / len2).max(MIN_FACTOR)
}

/// The smallest factor a handle drag gives: a drag past the fixed point
/// does not mirror.
const MIN_FACTOR: f64 = 0.01;

/// What a handle drag does: the factors along x and y about a fixed point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stretch {
    pub fx: f64,
    pub fy: f64,
    pub anchor: kurbo::Point,
    /// The `object.scale` anchor of the fixed point.
    pub anchor_name: &'static str,
}

impl Stretch {
    pub fn uniform(&self) -> bool {
        (self.fx - self.fy).abs() < 1e-12
    }

    pub fn affine(&self) -> kurbo::Affine {
        let a = self.anchor.to_vec2();
        kurbo::Affine::translate(a)
            * kurbo::Affine::scale_non_uniform(self.fx, self.fy)
            * kurbo::Affine::translate(-a)
    }

    pub fn apply(&self, r: kurbo::Rect) -> kurbo::Rect {
        self.affine().transform_rect_bbox(r)
    }
}

/// The stretch of a drag of `handle` of the box `start` to `pointer`.
/// `keep` keeps the shape; `centre` scales about the centre instead of
/// the opposite corner or side. A corner changes both sides, a side only
/// its own unless the shape is kept.
pub fn stretch(
    start: kurbo::Rect,
    handle: usize,
    pointer: kurbo::Point,
    keep: bool,
    centre: bool,
) -> Stretch {
    let points = handle_points(start);
    let grip = points[handle];
    let (anchor, anchor_name) = if centre {
        (start.center(), "center")
    } else {
        let o = opposite(handle);
        (points[o], ANCHORS[o])
    };
    let along = |p: f64, g: f64, a: f64| {
        if (g - a).abs() <= 1e-9 {
            1.0
        } else {
            ((p - a) / (g - a)).max(MIN_FACTOR)
        }
    };
    let fx = along(pointer.x, grip.x, anchor.x);
    let fy = along(pointer.y, grip.y, anchor.y);
    let (fx, fy) = match (handle, keep) {
        (0..=3, true) => {
            let f = corner_factor(anchor, grip, pointer);
            (f, f)
        }
        (0..=3, false) => (fx, fy),
        (5 | 7, true) => (fx, fx),
        (5 | 7, false) => (fx, 1.0),
        (_, true) => (fy, fy),
        (_, false) => (1.0, fy),
    };
    Stretch {
        fx,
        fy,
        anchor,
        anchor_name,
    }
}

/// The stretch of a handle drag so far, with its modifiers.
fn drag_stretch(d: &Drag) -> Option<Stretch> {
    match d {
        Drag::Scale {
            handle,
            start,
            current,
            keep,
            free,
            flip,
            centre,
            ..
        } => {
            let keep = !*free || (*keep != *flip);
            Some(stretch(*start, *handle, *current, keep, *centre))
        }
        _ => None,
    }
}

fn hit_test(snap: &Snapshot, view: &CanvasView, at: Pos2) -> Hit {
    if let Some(b) = selection_box(snap) {
        let screen = view.rect_to_screen(b);
        // The corners first: they win where handles overlap.
        for (i, c) in handle_points(b).into_iter().enumerate() {
            if handle_shown(i, screen) && view.mm_to_screen(c).distance(at) <= HANDLE_PICK {
                return Hit::Handle(i);
            }
        }
    }
    let mm = view.screen_to_mm(at);
    let slack = f64::from(3.0 / view.px_per_mm);
    snap.objects
        .iter()
        .rev()
        .filter(|o| o.visible)
        .find(|o| {
            o.bounds()
                .is_some_and(|b| b.inflate(slack, slack).contains(mm))
        })
        .map_or(Hit::Nothing, |o| Hit::Object(o.id))
}

/// How a dragged object is shown before the drag is sent. A scale also
/// shows the linked copies of the scaled objects that are not dragged:
/// they take the scale and keep their own centre, as `object.scale` does.
fn drag_preview(drag: Option<&Drag>, snap: &Snapshot, id: u64) -> Option<kurbo::Affine> {
    match drag? {
        Drag::Move {
            ids,
            start,
            current,
            ..
        } if ids.contains(&id) => Some(kurbo::Affine::translate(*current - *start)),
        d @ Drag::Scale { ids, .. } => {
            let st = drag_stretch(d)?;
            if ids.contains(&id) {
                return Some(st.affine());
            }
            let o = snap.object(id)?;
            let group = o.link_group?;
            let own = Stretch {
                anchor: o.transform.center_mm,
                ..st
            };
            ids.iter()
                .any(|i| snap.object(*i).is_some_and(|m| m.link_group == Some(group)))
                .then(|| own.affine())
        }
        _ => None,
    }
}

/// The visible objects whose placed bounds touch `frame` (mm), in
/// document order.
fn framed(snap: &Snapshot, frame: kurbo::Rect) -> Vec<u64> {
    let f = frame.abs();
    snap.objects
        .iter()
        .filter(|o| o.visible)
        .filter(|o| {
            o.bounds()
                .is_some_and(|b| b.x0 <= f.x1 && b.x1 >= f.x0 && b.y0 <= f.y1 && b.y1 >= f.y0)
        })
        .map(|o| o.id)
        .collect()
}

/// Draws the canvas into `rect` and handles the pointer.
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    snap: &Snapshot,
    st: &mut PrepareState,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    st.canvas_rect = rect;
    if st.view_doc != Some(snap.doc_id) {
        st.view_doc = Some(snap.doc_id);
        st.fit_pending = true;
        // A drag belongs to the document it started in.
        st.drag = None;
    }
    if st.fit_pending {
        if let Some(m) = &snap.mat {
            st.view = CanvasView::fit(snap.mat_rect(m), fit_rect(rect));
        }
        st.fit_pending = false;
    }

    let resp = ui.interact(
        rect,
        ui.scope_id().with("flatcraft-canvas"),
        Sense::click_and_drag(),
    );
    pointer(ui, &resp, snap, st, actions);
    handle_card(ui, &resp, snap, st);
    if resp.secondary_clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        // The same hit test as the left click.
        st.context = Some(match hit_test(snap, &st.view, p) {
            Hit::Object(id) => context_menu::target(snap, Some(id), actions),
            Hit::Handle(_) => snap.selection.iter().copied().collect(),
            Hit::Nothing => context_menu::target(snap, None, actions),
        });
    }
    resp.context_menu(|ui| context_menu::show(ui, snap, st, actions));

    let painter = ui.painter_at(rect);
    let view = st.view;
    let (mat_rgb, contrast) = draw_mat(&painter, rect, snap, &view, st.show_grid, t);
    draw_objects(&painter, snap, &view, mat_rgb, 1.0, false, |id| {
        drag_preview(st.drag.as_ref(), snap, id)
    });
    if let Some(p) = &st.panel {
        draw_ghosts(
            &painter,
            snap,
            &view,
            mat_rgb,
            &super::arrange::ghosts(snap, p),
        );
    }
    draw_area(&painter, snap, &view, t, contrast);
    if let Some(r) = draw_selection(&painter, snap, st, rgb(selection_colour(mat_rgb))) {
        link_badge(ui, r, snap, contrast);
    }
    if let Some(Drag::Frame { start, current, .. }) = &st.drag {
        let r = view.rect_to_screen(kurbo::Rect::from_points(*start, *current));
        let pick = rgb(selection_colour(mat_rgb));
        painter.rect_filled(r, 0.0, pick.gamma_multiply(0.12));
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, pick), egui::StrokeKind::Middle);
    }
    if st.show_rulers {
        draw_rulers(&painter, rect, &view, t);
    }
}

/// The mat colour, falling back to paper.
pub fn mat_rgb(snap: &Snapshot) -> [u8; 3] {
    snap.mat
        .as_ref()
        .map_or(flatcraft_device::DEFAULT_MAT_COLOR, |m| m.color)
}

/// Fills the canvas, draws the mat (the margin in a darker shade of its
/// colour, the cutting area in the mat colour itself) and the printed grid.
/// Returns the mat colour and its contrast colour.
pub(crate) fn draw_mat(
    painter: &egui::Painter,
    rect: Rect,
    snap: &Snapshot,
    view: &CanvasView,
    show_grid: bool,
    t: &Tokens,
) -> ([u8; 3], Color32) {
    painter.rect_filled(rect, 0.0, t.bg);
    let mat_rgb = mat_rgb(snap);
    let contrast = rgb(contrast_colour(mat_rgb));
    if let Some(m) = &snap.mat {
        painter.rect_filled(
            view.rect_to_screen(snap.mat_rect(m)),
            0.0,
            rgb(margin_colour(m.color)),
        );
        let area = snap.area.unwrap_or_else(|| snap.mat_rect(m));
        painter.rect_filled(view.rect_to_screen(area), 0.0, rgb(m.color));
        if show_grid {
            draw_grid(painter, view, m.grid.width_mm, m.grid.height_mm, contrast);
        }
    }
    (mat_rgb, contrast)
}

/// Draws the visible objects in their colours (contrast rule), each placed
/// by its centre and an optional extra affine (a drag preview). `alpha`
/// fades them (the Cut workspace draws toolpaths on top). An object that is
/// skipped (not cut) is drawn dimmed with a dashed outline, or not at all
/// when `cut_preview` is set.
pub(crate) fn draw_objects(
    painter: &egui::Painter,
    snap: &Snapshot,
    view: &CanvasView,
    mat_rgb: [u8; 3],
    alpha: f32,
    cut_preview: bool,
    extra: impl Fn(u64) -> Option<kurbo::Affine>,
) {
    for o in snap.objects.iter().filter(|o| o.visible) {
        if cut_preview && !o.cut_enabled {
            continue;
        }
        let alpha = if o.cut_enabled { alpha } else { alpha * 0.6 };
        let base = kurbo::Affine::translate(o.transform.center_mm.to_vec2());
        let place = extra(o.id).map_or(base, |p| p * base);
        for line in &o.shape.lines {
            let colour = rgb(object_colour(line.colour, mat_rgb)).gamma_multiply(alpha);
            let pts: Vec<Pos2> = line
                .points
                .iter()
                .map(|p| view.mm_to_screen(place * *p))
                .collect();
            let stroke = Stroke::new(1.25, colour);
            if !o.cut_enabled {
                let mut pts = pts;
                if line.closed
                    && let Some(first) = pts.first().copied()
                {
                    pts.push(first);
                }
                painter.extend(egui::Shape::dashed_line(&pts, stroke, 5.0, 4.0));
            } else if line.closed {
                painter.add(egui::Shape::closed_line(pts, stroke));
            } else {
                painter.add(egui::Shape::line(pts, stroke));
            }
        }
    }
}

/// The colour of the ghost preview's dashes.
pub fn ghost_colour(mat: [u8; 3]) -> Color32 {
    rgb(selection_colour(mat)).gamma_multiply(0.8)
}

/// Draws the would-be objects of an open Arrange panel faintly: each
/// ghost is an object's shape placed by its affine (see
/// [`super::arrange::ghosts`]). Nothing in the document changes.
pub(crate) fn draw_ghosts(
    painter: &egui::Painter,
    snap: &Snapshot,
    view: &CanvasView,
    mat: [u8; 3],
    ghosts: &[(u64, kurbo::Affine)],
) {
    let stroke = Stroke::new(1.0, ghost_colour(mat));
    for (id, place) in ghosts {
        let Some(o) = snap.object(*id) else {
            continue;
        };
        for line in &o.shape.lines {
            let mut pts: Vec<Pos2> = line
                .points
                .iter()
                .map(|p| view.mm_to_screen(*place * *p))
                .collect();
            if line.closed
                && let Some(first) = pts.first().copied()
            {
                pts.push(first);
            }
            painter.extend(egui::Shape::dashed_line(&pts, stroke, 4.0, 3.0));
        }
    }
}

/// Shows the band of an uncalibrated mat and outlines the cutting area.
pub(crate) fn draw_area(
    painter: &egui::Painter,
    snap: &Snapshot,
    view: &CanvasView,
    t: &Tokens,
    contrast: Color32,
) {
    if let Some(area) = snap.area {
        let a = view.rect_to_screen(area);
        let uncalibrated = snap
            .mat
            .as_ref()
            .is_some_and(|m| m.kind == MediaKind::Mat && !m.calibrated);
        if uncalibrated && let Some(m) = &snap.mat {
            let grid = view.rect_to_screen(kurbo::Rect::new(
                0.0,
                0.0,
                m.grid.width_mm,
                m.grid.height_mm,
            ));
            let band = t.accent_cut.gamma_multiply(0.22);
            for r in outside(grid, a) {
                painter.rect_filled(r, 0.0, band);
            }
        }
        painter.rect_stroke(a, 0.0, Stroke::new(1.0, contrast), egui::StrokeKind::Middle);
    }
}

/// The rect the view fits the mat into: the canvas minus rulers and a margin.
pub(crate) fn fit_rect(rect: Rect) -> Rect {
    Rect::from_min_max(rect.min + vec2(RULER, RULER), rect.max).shrink(24.0)
}

/// The parts of `outer` that are not inside `inner`.
fn outside(outer: Rect, inner: Rect) -> Vec<Rect> {
    let inner = inner.intersect(outer);
    if !inner.is_positive() {
        return vec![outer];
    }
    [
        Rect::from_min_max(outer.min, pos2(outer.max.x, inner.min.y)),
        Rect::from_min_max(pos2(outer.min.x, inner.max.y), outer.max),
        Rect::from_min_max(
            pos2(outer.min.x, inner.min.y),
            pos2(inner.min.x, inner.max.y),
        ),
        Rect::from_min_max(
            pos2(inner.max.x, inner.min.y),
            pos2(outer.max.x, inner.max.y),
        ),
    ]
    .into_iter()
    .filter(|r| r.is_positive())
    .collect()
}

fn draw_grid(painter: &egui::Painter, view: &CanvasView, w: f64, h: f64, contrast: Color32) {
    if f64::from(view.px_per_mm) * GRID_MM < 4.0 {
        return;
    }
    let minor = Stroke::new(0.5, contrast.gamma_multiply(0.25));
    let major = Stroke::new(0.75, contrast.gamma_multiply(0.45));
    let mut i = 0;
    while f64::from(i) * GRID_MM <= w + 1e-9 {
        let x = f64::from(i) * GRID_MM;
        let s = if i % 5 == 0 { major } else { minor };
        painter.line_segment(
            [
                view.mm_to_screen(kurbo::Point::new(x, 0.0)),
                view.mm_to_screen(kurbo::Point::new(x, h)),
            ],
            s,
        );
        i += 1;
    }
    let mut j = 0;
    while f64::from(j) * GRID_MM <= h + 1e-9 {
        let y = f64::from(j) * GRID_MM;
        let s = if j % 5 == 0 { major } else { minor };
        painter.line_segment(
            [
                view.mm_to_screen(kurbo::Point::new(0.0, y)),
                view.mm_to_screen(kurbo::Point::new(w, y)),
            ],
            s,
        );
        j += 1;
    }
}

/// A link mark just outside the top-right corner of the selection box
/// `r` (screen) when the selection holds linked copies.
fn link_badge(ui: &mut Ui, r: Rect, snap: &Snapshot, contrast: Color32) {
    let Some(n) = snap.linked_reach() else {
        return;
    };
    let size = super::ICON_SIZE * 0.8;
    let at = Rect::from_min_size(
        r.right_top() + Vec2::new(HANDLE_PICK + 2.0, -size - HANDLE_PICK - 2.0),
        Vec2::splat(size),
    );
    if !ui.clip_rect().intersects(at) {
        return;
    }
    let tex = super::icons::texture_colored(ui.ctx(), super::icons::Icon::Linked, size, contrast);
    ui.put(
        at,
        egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), Vec2::splat(size)))
            .alt_text("Linked copies selected"),
    )
    .on_hover_text(format!(
        "{n} linked copies share the source, the cut options and the scale"
    ));
}

/// Draws the selection box and its handles. Returns the box on screen.
fn draw_selection(
    painter: &egui::Painter,
    snap: &Snapshot,
    st: &PrepareState,
    pick: Color32,
) -> Option<Rect> {
    let b = live_box(snap, st)?;
    let r = st.view.rect_to_screen(b);
    painter.rect_stroke(r, 0.0, Stroke::new(1.5, pick), egui::StrokeKind::Outside);
    for (i, c) in handle_points(b).into_iter().enumerate() {
        if handle_shown(i, r) {
            let h = Rect::from_center_size(st.view.mm_to_screen(c), Vec2::splat(HANDLE * 2.0));
            painter.rect_filled(h, 1.0, pick);
        }
    }
    Some(r)
}

/// Over a handle (and not dragging): its resize cursor and its card.
fn handle_card(ui: &Ui, resp: &egui::Response, snap: &Snapshot, st: &PrepareState) {
    if st.drag.is_some() {
        return;
    }
    let Some(at) = resp.hover_pos() else { return };
    let Hit::Handle(i) = hit_test(snap, &st.view, at) else {
        return;
    };
    ui.ctx().set_cursor_icon(handle_cursor(i));
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    let why = (!free_aspect(snap, &ids)).then_some(if ids.len() > 1 {
        "Several objects always scale together and keep their shape, so Shift does nothing here."
    } else {
        "This object is turned by an angle that is not a multiple of 90 degrees, so it always keeps its shape and Shift does nothing here."
    });
    let title = if i < 4 {
        "Corner handle"
    } else {
        "Side handle"
    };
    super::groups::card(
        resp,
        title,
        super::tools::Setting::ScaleHandle.card(),
        why,
        false,
    );
}

/// The resize cursor of a handle.
fn handle_cursor(i: usize) -> egui::CursorIcon {
    match i {
        0 | 2 => egui::CursorIcon::ResizeNwSe,
        1 | 3 => egui::CursorIcon::ResizeNeSw,
        4 | 6 => egui::CursorIcon::ResizeVertical,
        _ => egui::CursorIcon::ResizeHorizontal,
    }
}

/// A tick step in mm that keeps ticks at least `min_px` apart.
fn step_mm(px_per_mm: f32, min_px: f32) -> f64 {
    for s in [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0] {
        if s as f32 * px_per_mm >= min_px {
            return s;
        }
    }
    1000.0
}

/// Tick indices `k` with `k * step` between `from` and `to`. Integer
/// indices keep the ticks free of accumulated float error.
fn ticks(from: f64, to: f64, step: f64) -> std::ops::RangeInclusive<i64> {
    (from / step).ceil() as i64..=(to / step).floor() as i64
}

/// True if `v` is a multiple of `step`.
fn on_step(v: f64, step: f64) -> bool {
    ((v / step).round() * step - v).abs() < 1e-6
}

pub(crate) fn draw_rulers(painter: &egui::Painter, rect: Rect, view: &CanvasView, t: &Tokens) {
    let top = Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + RULER));
    let left = Rect::from_min_max(rect.min, pos2(rect.min.x + RULER, rect.max.y));
    painter.rect_filled(top, 0.0, t.panel);
    painter.rect_filled(left, 0.0, t.panel);
    let minor = step_mm(view.px_per_mm, 6.0);
    let label = step_mm(view.px_per_mm, 44.0).max(10.0);
    let font = fonts::medium(9.0);
    let stroke = Stroke::new(1.0, t.text_dim);
    // Along x.
    let x0 = view.screen_to_mm(pos2(top.min.x + RULER, 0.0)).x;
    let x1 = view.screen_to_mm(pos2(top.max.x, 0.0)).x;
    for k in ticks(x0, x1, minor) {
        let x = k as f64 * minor;
        let sx = view.mm_to_screen(kurbo::Point::new(x, 0.0)).x;
        let is_label = on_step(x, label);
        let len = if is_label { RULER * 0.6 } else { RULER * 0.25 };
        painter.line_segment([pos2(sx, top.max.y - len), pos2(sx, top.max.y)], stroke);
        if is_label {
            painter.text(
                pos2(sx + 2.0, top.min.y + 1.0),
                Align2::LEFT_TOP,
                format!("{x:.0}"),
                font.clone(),
                t.text_dim,
            );
        }
    }
    // Along y.
    let y0 = view.screen_to_mm(pos2(0.0, left.min.y + RULER)).y;
    let y1 = view.screen_to_mm(pos2(0.0, left.max.y)).y;
    for k in ticks(y0, y1, minor) {
        let y = k as f64 * minor;
        let sy = view.mm_to_screen(kurbo::Point::new(0.0, y)).y;
        let is_label = on_step(y, label);
        let len = if is_label { RULER * 0.6 } else { RULER * 0.25 };
        painter.line_segment([pos2(left.max.x - len, sy), pos2(left.max.x, sy)], stroke);
        if is_label {
            painter.text(
                pos2(left.min.x + 1.0, sy + 2.0),
                Align2::LEFT_TOP,
                format!("{y:.0}"),
                font.clone(),
                t.text_dim,
            );
        }
    }
    painter.rect_filled(
        Rect::from_min_size(rect.min, Vec2::splat(RULER)),
        0.0,
        t.panel,
    );
}

// ---------------------------------------------------------------- keys

/// How far one arrow-key press moves the selection: 10 mm with Shift,
/// 0.1 mm with Alt (Option), else 1 mm. `None` for keys that are not
/// arrows and for Cmd or Ctrl with an arrow, which are left to others.
fn nudge_step(key: egui::Key, modifiers: egui::Modifiers) -> Option<kurbo::Vec2> {
    if modifiers.command || modifiers.ctrl || modifiers.mac_cmd || !super::help::is_nudge_key(key) {
        return None;
    }
    let mm = if modifiers.shift {
        10.0
    } else if modifiers.alt {
        0.1
    } else {
        1.0
    };
    Some(match key {
        egui::Key::ArrowLeft => kurbo::Vec2::new(-mm, 0.0),
        egui::Key::ArrowRight => kurbo::Vec2::new(mm, 0.0),
        egui::Key::ArrowUp => kurbo::Vec2::new(0.0, -mm),
        egui::Key::ArrowDown => kurbo::Vec2::new(0.0, mm),
        _ => return None,
    })
}

/// The arrow keys move the selection through `object.move`. The presses
/// of one frame add up to one move. A held key (key repeat) and presses
/// less than [`NUDGE_BURST`] apart on the same objects share one
/// `coalesce_key`, so the engine keeps the burst as one undo step. The
/// caller checks that no text field has the keyboard.
pub(crate) fn nudge_keys(
    ctx: &egui::Context,
    snap: &Snapshot,
    st: &mut PrepareState,
    now: std::time::Instant,
    actions: &mut Vec<Action>,
) {
    if st.drag.is_some() {
        return;
    }
    let mut delta = kurbo::Vec2::ZERO;
    let mut first_repeat = None;
    ctx.input_mut(|i| {
        i.events.retain(|e| match e {
            egui::Event::Key {
                key,
                pressed,
                modifiers,
                repeat,
                ..
            } => match nudge_step(*key, *modifiers) {
                Some(step) => {
                    if *pressed {
                        delta += step;
                        first_repeat.get_or_insert(*repeat);
                    }
                    false
                }
                None => true,
            },
            _ => true,
        });
    });
    let Some(repeat) = first_repeat else {
        return;
    };
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    let burst = match &st.nudge {
        Some(n)
            if n.doc_id == snap.doc_id
                && n.ids == ids
                && (repeat || now.saturating_duration_since(n.last) < NUDGE_BURST) =>
        {
            n.burst
        }
        Some(n) => n.burst + 1,
        None => 1,
    };
    st.nudge = Some(Nudge {
        doc_id: snap.doc_id,
        ids: ids.clone(),
        burst,
        last: now,
    });
    if delta.hypot() > 1e-12 {
        actions.push(Action::run(
            snap.doc_id,
            "object.move",
            json!({
                "ids": ids,
                "dx_mm": delta.x,
                "dy_mm": delta.y,
                "coalesce_key": format!("nudge-{burst}"),
            }),
        ));
    }
}

// ---------------------------------------------------------------- pointer

/// Zoom factor per mouse wheel line. A first guess: Jordi tunes it on a
/// real mouse. Too small feels sticky, too large jumps past the detail.
const WHEEL_ZOOM_PER_LINE: f32 = 1.15;
/// Lines in one page step (a wheel set to scroll a page at a time). Also a
/// guess, tuned together with `WHEEL_ZOOM_PER_LINE`.
const WHEEL_LINES_PER_PAGE: f32 = 10.0;

/// What one scroll event does to the canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Wheel {
    Pan(Vec2),
    Zoom(f32),
}

/// Scroll events without the zoom modifier: smooth (point) scrolling pans,
/// line and page steps of a mouse wheel zoom. Events with Cmd or Ctrl are
/// left to egui's `zoom_delta`.
fn wheel_events(events: &[egui::Event]) -> Vec<Wheel> {
    events
        .iter()
        .filter_map(|e| match e {
            egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } if !modifiers.command && !modifiers.ctrl => Some(match unit {
                egui::MouseWheelUnit::Point => Wheel::Pan(*delta),
                egui::MouseWheelUnit::Line => Wheel::Zoom(WHEEL_ZOOM_PER_LINE.powf(delta.y)),
                egui::MouseWheelUnit::Page => {
                    Wheel::Zoom(WHEEL_ZOOM_PER_LINE.powf(delta.y * WHEEL_LINES_PER_PAGE))
                }
            }),
            _ => None,
        })
        .collect()
}

/// Trackpad scrolling pans; a mouse wheel, Cmd/Ctrl+scroll
/// and pinch zoom around the pointer. egui turns Cmd/Ctrl+scroll and
/// pinch into `zoom_delta`.
fn scroll(
    resp: &egui::Response,
    view: &mut CanvasView,
    wheels: Vec<Wheel>,
    zoom: f32,
    hover: Option<Pos2>,
) {
    if resp.hovered()
        && let Some(at) = hover
    {
        let mut factor = zoom;
        for w in wheels {
            match w {
                Wheel::Pan(d) => view.offset += d,
                Wheel::Zoom(f) => factor *= f,
            }
        }
        if (factor - 1.0).abs() > 1e-6 {
            view.zoom_about(at, factor);
        }
    }
}

/// Pan and zoom only, for a canvas that edits nothing (Cut): scrolling as
/// in Prepare, and any drag pans.
pub(crate) fn navigate(ui: &Ui, resp: &egui::Response, view: &mut CanvasView) {
    let (wheels, zoom, hover) = ui.input(|i| {
        (
            wheel_events(&i.raw.events),
            i.zoom_delta(),
            i.pointer.hover_pos(),
        )
    });
    scroll(resp, view, wheels, zoom, hover);
    if resp.dragged() {
        view.offset += resp.drag_delta();
    }
}

fn pointer(
    ui: &Ui,
    resp: &egui::Response,
    snap: &Snapshot,
    st: &mut PrepareState,
    actions: &mut Vec<Action>,
) {
    let (space, shift, mods, wheels, zoom, hover) = ui.input(|i| {
        (
            i.key_down(super::help::pan_key()),
            i.modifiers.shift,
            i.modifiers,
            wheel_events(&i.raw.events),
            i.zoom_delta(),
            i.pointer.hover_pos(),
        )
    });
    let space = space && !ui.ctx().egui_wants_keyboard_input();

    scroll(resp, &mut st.view, wheels, zoom, hover);

    // Middle drag or space drag pans.
    if resp.dragged_by(egui::PointerButton::Middle)
        || (space && resp.dragged_by(egui::PointerButton::Primary))
        || matches!(st.drag, Some(Drag::Pan))
    {
        if resp.drag_started() {
            st.drag = Some(Drag::Pan);
        }
        st.view.offset += resp.drag_delta();
        if resp.drag_stopped() {
            st.drag = None;
        }
        return;
    }

    let doc_id = snap.doc_id;
    if resp.drag_started_by(egui::PointerButton::Primary) {
        let origin = ui
            .input(|i| i.pointer.press_origin())
            .or(resp.interact_pointer_pos())
            .unwrap_or_default();
        let mm = st.view.screen_to_mm(origin);
        st.drag = match hit_test(snap, &st.view, origin) {
            Hit::Handle(i) => {
                let b = selection_box(snap).expect("a handle needs a selection");
                let ids: Vec<u64> = snap.selection.iter().copied().collect();
                Some(Drag::Scale {
                    doc_id,
                    free: free_aspect(snap, &ids),
                    ids,
                    handle: i,
                    start: b,
                    current: handle_points(b)[i],
                    keep: st.fields.keep_aspect,
                    flip: false,
                    centre: false,
                })
            }
            Hit::Object(id) => {
                let ids: Vec<u64> = if snap.selection.contains(&id) {
                    snap.selection.iter().copied().collect()
                } else {
                    actions.push(Action::run(doc_id, "object.select", json!({ "ids": [id] })));
                    vec![id]
                };
                Some(Drag::Move {
                    doc_id,
                    ids,
                    start: mm,
                    current: mm,
                })
            }
            // A selection frame; Shift adds what it touches.
            Hit::Nothing => Some(Drag::Frame {
                doc_id,
                start: mm,
                current: mm,
                add: shift,
            }),
        };
    }
    if let Some(p) = resp.interact_pointer_pos() {
        let mm = st.view.screen_to_mm(p);
        if let Some(
            Drag::Move { current, .. } | Drag::Scale { current, .. } | Drag::Frame { current, .. },
        ) = &mut st.drag
        {
            *current = mm;
        }
    }
    // Shift and Option count while the handle is dragged, up to the drop.
    if let Some(Drag::Scale { flip, centre, .. }) = &mut st.drag {
        *flip = super::help::held(mods, super::help::PrepareKey::FlipKeepAspect);
        *centre = super::help::held(mods, super::help::PrepareKey::ScaleFromCentre);
    }
    if let Some(Drag::Scale { handle, .. }) = &st.drag {
        ui.ctx().set_cursor_icon(handle_cursor(*handle));
    }
    if resp.drag_stopped() {
        match st.drag.take() {
            Some(Drag::Move {
                doc_id,
                ids,
                start,
                current,
            }) => {
                let d = current - start;
                if d.hypot() > 1e-9 {
                    actions.push(Action::run(
                        doc_id,
                        "object.move",
                        json!({ "ids": ids, "dx_mm": d.x, "dy_mm": d.y }),
                    ));
                }
            }
            Some(d @ Drag::Scale { .. }) => {
                if let Some(a) = scale_action(&d) {
                    actions.push(a);
                }
            }
            Some(Drag::Frame {
                doc_id,
                start,
                current,
                add,
            }) => {
                let mut ids: Vec<u64> = if add || shift {
                    snap.selection.iter().copied().collect()
                } else {
                    Vec::new()
                };
                for id in framed(snap, kurbo::Rect::from_points(start, current)) {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                actions.push(Action::run(doc_id, "object.select", json!({ "ids": ids })));
            }
            _ => {}
        }
    }

    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let ids: Vec<u64> = match hit_test(snap, &st.view, p) {
            Hit::Object(id) if shift => {
                let mut ids: Vec<u64> = snap.selection.iter().copied().collect();
                if !ids.contains(&id) {
                    ids.push(id);
                }
                ids
            }
            Hit::Object(id) => vec![id],
            Hit::Handle(_) => return,
            Hit::Nothing if shift => return,
            Hit::Nothing => Vec::new(),
        };
        actions.push(Action::run(doc_id, "object.select", json!({ "ids": ids })));
    }
}

/// The `object.scale` a handle drag sends when it ends, if it changed
/// anything: one factor when it keeps the shape (several objects as one
/// group), else the new width and height of the one object.
fn scale_action(d: &Drag) -> Option<Action> {
    let Drag::Scale {
        doc_id, ids, start, ..
    } = d
    else {
        return None;
    };
    let s = drag_stretch(d)?;
    if (s.fx - 1.0).abs() <= 1e-9 && (s.fy - 1.0).abs() <= 1e-9 {
        return None;
    }
    let params = if s.uniform() {
        json!({
            "ids": ids,
            "factor": s.fx,
            "anchor": s.anchor_name,
            "group": ids.len() > 1,
        })
    } else {
        let mut p = json!({ "ids": ids, "keep_aspect": false, "anchor": s.anchor_name });
        if (s.fx - 1.0).abs() > 1e-9 {
            p["width_mm"] = json!(start.width() * s.fx);
        }
        if (s.fy - 1.0).abs() > 1e-9 {
            p["height_mm"] = json!(start.height() * s.fy);
        }
        p
    };
    Some(Action::run(*doc_id, "object.scale", params))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wheel_page_zooms_like_its_lines() {
        let event = |unit| egui::Event::MouseWheel {
            unit,
            delta: vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: egui::Modifiers::NONE,
        };
        let zoom = |unit| match wheel_events(&[event(unit)])[0] {
            Wheel::Zoom(f) => f,
            Wheel::Pan(_) => panic!("panned"),
        };
        assert_eq!(zoom(egui::MouseWheelUnit::Line), WHEEL_ZOOM_PER_LINE);
        let page = WHEEL_ZOOM_PER_LINE.powf(WHEEL_LINES_PER_PAGE);
        assert!((zoom(egui::MouseWheelUnit::Page) - page).abs() < 1e-4);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        let mut v = CanvasView {
            offset: vec2(10.0, 20.0),
            px_per_mm: 2.0,
        };
        let at = pos2(300.0, 200.0);
        let before = v.screen_to_mm(at);
        v.zoom_about(at, 1.7);
        assert!((v.screen_to_mm(at) - before).hypot() < 1e-3);
        assert!((v.px_per_mm - 3.4).abs() < 1e-6);
    }

    #[test]
    fn fit_centres_the_rect() {
        let v = CanvasView::fit(
            kurbo::Rect::new(0.0, 0.0, 100.0, 50.0),
            Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 400.0)),
        );
        assert_eq!(v.px_per_mm, 4.0);
        assert_eq!(
            v.mm_to_screen(kurbo::Point::new(50.0, 25.0)),
            pos2(200.0, 200.0)
        );
    }

    #[test]
    fn corner_factor_projects_on_the_diagonal() {
        let a = kurbo::Point::new(0.0, 0.0);
        let c = kurbo::Point::new(50.0, 50.0);
        assert!((corner_factor(a, c, kurbo::Point::new(75.0, 75.0)) - 1.5).abs() < 1e-12);
        assert!((corner_factor(a, c, kurbo::Point::new(-10.0, -10.0)) - 0.01).abs() < 1e-12);
    }

    #[test]
    fn handles_face_their_opposites() {
        assert_eq!(
            (0..8).map(opposite).collect::<Vec<_>>(),
            [2, 3, 0, 1, 6, 7, 4, 5]
        );
        for i in 0..8 {
            assert!(!ANCHORS[opposite(i)].is_empty());
        }
    }

    #[test]
    fn a_stretch_follows_keep_aspect_and_the_handle() {
        let b = kurbo::Rect::new(0.0, 0.0, 40.0, 20.0);
        let p = kurbo::Point::new(60.0, 30.0);
        // Bottom-right corner, free: both sides follow the pointer.
        let s = stretch(b, 2, p, false, false);
        assert_eq!((s.fx, s.fy, s.anchor_name), (1.5, 1.5, "top_left"));
        let s = stretch(b, 2, kurbo::Point::new(60.0, 20.0), false, false);
        assert_eq!((s.fx, s.fy), (1.5, 1.0));
        // The right side, free: only the width.
        let s = stretch(b, 5, kurbo::Point::new(80.0, 99.0), false, false);
        assert_eq!((s.fx, s.fy, s.anchor_name), (2.0, 1.0, "left"));
        // The right side, kept: both, about the middle of the left side.
        let s = stretch(b, 5, kurbo::Point::new(80.0, 99.0), true, false);
        assert_eq!((s.fx, s.fy), (2.0, 2.0));
        assert_eq!(s.apply(b), kurbo::Rect::new(0.0, -10.0, 80.0, 30.0));
        // From the centre: the half-size grows to the pointer.
        let s = stretch(b, 5, kurbo::Point::new(40.0, 10.0), false, true);
        assert_eq!((s.fx, s.anchor_name), (1.0, "center"));
        let s = stretch(b, 5, kurbo::Point::new(60.0, 10.0), false, true);
        assert_eq!(s.apply(b), kurbo::Rect::new(-20.0, 0.0, 60.0, 20.0));
        // Past the fixed side it does not mirror.
        let s = stretch(b, 5, kurbo::Point::new(-30.0, 10.0), false, false);
        assert_eq!(s.fx, MIN_FACTOR);
    }
}
