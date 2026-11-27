//! The mat canvas: the mat in its colour, the printed grid, rulers, the
//! objects and the selection. It holds no geometry logic: it draws the
//! placed paths the engine gives (see `prepare::Snapshot`) and converts
//! between screen points and millimetres with [`CanvasView`]. Moves and
//! scales are sent as one registry command when the drag ends.

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use flatcraft_device::MediaKind;
use flatcraft_engine::placement;
use serde_json::json;

use super::context_menu;
use super::help::{self, PrepareKey};
use super::prepare::{Drag, NUDGE_BURST, Nudge, ObjView, PrepareState, SelectTool, Snapshot};
use super::theme::Tokens;
use super::{Action, fonts};
use flatcraft_core::document::ObjectId;
use flatcraft_engine::lasso::{self, Lasso, Outlines};

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
/// How far from a corner the rotate zone reaches, in points: from the end
/// of the handle's pick area out to 14 points past the handle's edge, and
/// only outside the selection box.
pub const ROTATE_REACH: f32 = HANDLE + 14.0;
/// The steps of a turn with Shift held, in degrees.
pub const SNAP_DEG: f64 = 15.0;

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
    /// This view moved so `mm` sits in the middle of `screen`, at the same
    /// zoom when it fits there, else zoomed to fit it.
    pub fn showing(self, mm: kurbo::Rect, screen: Rect) -> Self {
        let w = mm.width() as f32 * self.px_per_mm;
        let h = mm.height() as f32 * self.px_per_mm;
        if w > screen.width() || h > screen.height() {
            return Self::fit(mm, screen);
        }
        let centre = mm.center();
        Self {
            offset: screen.center().to_vec2()
                - vec2(
                    centre.x as f32 * self.px_per_mm,
                    centre.y as f32 * self.px_per_mm,
                ),
            px_per_mm: self.px_per_mm,
        }
    }

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
    selection_colour, weed_colour,
};

/// The weed path colour on `mat` for the theme `t` (see [`weed_colour`]).
pub fn weed_rgb(mat: [u8; 3], t: &Tokens) -> Color32 {
    let w = t.weed_path;
    rgb(weed_colour(mat, [w.r(), w.g(), w.b()]))
}

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
    /// The rotate zone just outside corner `0..4` of the selection box.
    Rotate(usize),
    Object(u64),
    Nothing,
}

/// What the pointer looks like over a hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Look {
    /// The resize cursor of handle `i`.
    Resize(usize),
    /// The painted rotate cursor.
    Rotate,
    Plain,
}

/// The look of the pointer over `hit` with `mods` held: a corner handle
/// with Cmd (macOS) or Ctrl held rotates, so it shows the rotate cursor.
fn look(hit: Hit, mods: egui::Modifiers) -> Look {
    match hit {
        Hit::Handle(i) if i < 4 && help::held(mods, PrepareKey::RotateOnCorner) => Look::Rotate,
        Hit::Handle(i) => Look::Resize(i),
        Hit::Rotate(_) => Look::Rotate,
        Hit::Object(_) | Hit::Nothing => Look::Plain,
    }
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

/// The common box of the selected objects, in mm: the shown ones, or all
/// when every one is hidden.
pub fn selection_box(snap: &Snapshot) -> Option<kurbo::Rect> {
    // Hidden members do not count while something shown is selected.
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    snap.box_of(&ids)
}

/// The selection box as it is right now: during a drag, where the drag
/// has it. The Position and size fields and the selection frame show it.
pub fn live_box(snap: &Snapshot, st: &PrepareState) -> Option<kurbo::Rect> {
    let b = selection_box(snap)?;
    Some(match &st.drag {
        Some(Drag::Move { start, current, .. }) => b + (*current - *start),
        Some(d @ Drag::Scale { .. }) => drag_stretch(d).map_or(b, |s| s.apply(b)),
        Some(d @ Drag::Rotate { ids, .. }) => drag_turn(d)
            .and_then(|a| turned_box(snap, ids, a))
            .unwrap_or(b),
        _ => b,
    })
}

/// The box of the objects `ids` placed with the extra affine `a`: the box
/// of their turned outlines.
fn turned_box(snap: &Snapshot, ids: &[u64], a: kurbo::Affine) -> Option<kurbo::Rect> {
    ids.iter()
        .filter_map(|id| snap.object(*id))
        .filter_map(|o| {
            let place = a * kurbo::Affine::translate(o.transform.center_mm.to_vec2());
            let mut pts = o.shape.lines.iter().flat_map(|l| l.points.iter());
            let first = pts.next().map(|p| place * *p);
            match first {
                Some(f) => Some(pts.fold(kurbo::Rect::from_points(f, f), |r, p| {
                    r.union_pt(place * *p)
                })),
                // No outline (yet): the turned box.
                None => o.bounds().map(|b| a.transform_rect_bbox(b)),
            }
        })
        .reduce(|a, b| a.union(b))
}

/// The angle the Angle field shows: the rotation all selected objects
/// share, plus the turn of a rotate drag, in [0, 360). `None` while they
/// are turned different ways (the field says Mixed) or nothing is
/// selected.
pub fn live_angle(snap: &Snapshot, st: &PrepareState) -> Option<f64> {
    // A whole group shows its own angle.
    let common = common_angle(snap.units().into_iter().filter_map(|u| snap.unit_angle(u)))?;
    let turn = match &st.drag {
        Some(d @ Drag::Rotate { .. }) => rotate_delta_of(d).unwrap_or(0.0),
        _ => 0.0,
    };
    Some(normalise_deg(common + turn))
}

/// The one angle of `angles`, if they are all the same (359.9999 counts as
/// 0); `None` for none or different ones.
pub fn common_angle(mut angles: impl Iterator<Item = f64>) -> Option<f64> {
    let first = angles.next()?;
    angles
        .all(|a| wrap_180(a - first).abs() < 1e-6)
        .then_some(first)
}

/// `deg` in [0, 360), with a value that would show as 360.00 as 0.
pub fn normalise_deg(deg: f64) -> f64 {
    let r = deg.rem_euclid(360.0);
    // + 0.0 turns -0.0 into 0.0.
    if 360.0 - r < 0.005 { 0.0 } else { r + 0.0 }
}

/// `deg` in (-180, 180].
fn wrap_180(deg: f64) -> f64 {
    let r = deg.rem_euclid(360.0);
    if r > 180.0 { r - 360.0 } else { r }
}

/// The turn of a rotate drag from `start` to `current` about `pivot`, in
/// degrees clockwise on screen (y down), in (-180, 180]. With `snap` the
/// result snaps to [`SNAP_DEG`] steps: the angle of the one object turned
/// (`base`), or the turn itself for several (`base` None).
pub fn rotate_delta(
    pivot: kurbo::Point,
    start: kurbo::Point,
    current: kurbo::Point,
    base: Option<f64>,
    snap: bool,
) -> f64 {
    // atan2 grows clockwise on screen, because y points down.
    let angle = |p: kurbo::Point| (p - pivot).atan2().to_degrees();
    let d = wrap_180(angle(current) - angle(start));
    if !snap {
        return d;
    }
    let step = |v: f64| (v / SNAP_DEG).round() * SNAP_DEG;
    match base {
        Some(b) => wrap_180(step(b + d) - b),
        None => wrap_180(step(d)),
    }
}

/// The turn of a rotate drag so far, in degrees.
fn rotate_delta_of(d: &Drag) -> Option<f64> {
    match d {
        Drag::Rotate {
            pivot,
            start,
            current,
            base,
            snap,
            ..
        } => Some(rotate_delta(*pivot, *start, *current, *base, *snap)),
        _ => None,
    }
}

/// The affine of a rotate drag so far: the turn about its pivot.
fn drag_turn(d: &Drag) -> Option<kurbo::Affine> {
    let deg = rotate_delta_of(d)?;
    let Drag::Rotate { pivot, .. } = d else {
        return None;
    };
    let p = pivot.to_vec2();
    Some(
        kurbo::Affine::translate(p)
            * kurbo::Affine::rotate(deg.to_radians())
            * kurbo::Affine::translate(-p),
    )
}

/// A rotate drag of the selection that starts at `start` (mm): one
/// object turns about its own centre, several about the mean of their
/// centres, as `object.rotate` does.
fn rotate_drag(snap: &Snapshot, start: kurbo::Point) -> Option<Drag> {
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    // A whole group turns as one piece about its box centre.
    let units = snap.units_of(&ids);
    let centres: Vec<kurbo::Point> = units.iter().filter_map(|u| snap.unit_centre(*u)).collect();
    let pivot = placement::mean_centre(&centres)?;
    let base = match units.as_slice() {
        [one] => snap.unit_angle(*one),
        _ => None,
    };
    Some(Drag::Rotate {
        doc_id: snap.doc_id,
        ids,
        pivot,
        start,
        current: start,
        base,
        snap: false,
    })
}

/// The `object.rotate` a rotate drag sends when it ends, if it turned
/// anything: one object gets its new angle, several turn as one group.
fn rotate_action(d: &Drag) -> Option<Action> {
    let Drag::Rotate {
        doc_id, ids, base, ..
    } = d
    else {
        return None;
    };
    let deg = rotate_delta_of(d)?;
    if deg.abs() <= 1e-9 {
        return None;
    }
    Some(match base {
        Some(b) => Action::run(
            *doc_id,
            "object.rotate",
            json!({ "ids": ids, "angle_deg": normalise_deg(b + deg) }),
        ),
        None => super::ribbon::rotate_action(*doc_id, ids, deg, false),
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

/// Why the width and height of these objects cannot change apart, and
/// what to do about it, or `None` when they can. `object.scale` changes
/// them apart only for objects turned by a multiple of 90 degrees, and
/// linked copies (which share one scale) must be turned the same way.
pub fn aspect_lock(snap: &Snapshot, ids: &[u64]) -> Option<&'static str> {
    let objects: Vec<_> = ids.iter().filter_map(|id| snap.object(*id)).collect();
    let quarter = |deg: f64| {
        let q = deg / 90.0;
        ((q - q.round()).abs() < 1e-9).then(|| (q.round() as i64).rem_euclid(2) == 1)
    };
    if objects
        .iter()
        .any(|o| quarter(o.transform.rotation_deg).is_none())
    {
        return Some(if objects.len() > 1 {
            "One of these objects is turned by an angle that is not a multiple of 90 degrees, so they keep their shape. Type 0, 90, 180 or 270 in the Angle field of Position and size to change width and height apart."
        } else {
            "This object is turned by an angle that is not a multiple of 90 degrees, so it keeps its shape. Type 0, 90, 180 or 270 in the Angle field of Position and size to change width and height apart."
        });
    }
    let crossed = objects.iter().any(|a| {
        objects.iter().any(|b| {
            a.link_group.is_some()
                && a.link_group == b.link_group
                && quarter(a.transform.rotation_deg) != quarter(b.transform.rotation_deg)
        })
    });
    crossed.then_some(
        "Linked copies here are turned different ways and share one scale, so they keep their shape. Select only copies turned the same way, or Unlink copies, to change width and height apart.",
    )
}

/// Whether width and height of these objects can change apart (see
/// [`aspect_lock`]).
pub fn free_aspect(snap: &Snapshot, ids: &[u64]) -> bool {
    !ids.is_empty() && aspect_lock(snap, ids).is_none()
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

/// The group the person is in: it still exists in this document and the
/// selection lies in it (a selection made elsewhere, in the Objects panel
/// or with Select all, leaves it).
pub fn entered(snap: &Snapshot, st: &PrepareState) -> Option<u64> {
    let (doc, g) = st.entered?;
    let g = flatcraft_core::document::ObjectId(g);
    (doc == snap.doc_id
        && snap.tree.is_group(g)
        && !snap.selection.is_empty()
        && snap
            .selection
            .iter()
            .all(|id| snap.tree.within(flatcraft_core::document::ObjectId(*id), g)))
    .then_some(g.0)
}

/// What a click on object `leaf` selects, and the group the person is in
/// afterwards. A click picks the whole outermost group of the object, as
/// one object. `deep` (a double-click, or Cmd-click on macOS and
/// Ctrl-click elsewhere) picks the object alone and goes into its group.
/// Inside a group, a click on one of its members picks that member (or
/// the nested group it is in); a click outside leaves the group.
pub fn pick(snap: &Snapshot, st: &PrepareState, leaf: u64, deep: bool) -> (Vec<u64>, Option<u64>) {
    let tree = &snap.tree;
    let id = flatcraft_core::document::ObjectId(leaf);
    if deep {
        return (vec![leaf], tree.parent(id).map(|g| g.0));
    }
    if let Some(g) = entered(snap, st)
        && tree.within(id, flatcraft_core::document::ObjectId(g))
    {
        // The child of the group that holds the object.
        let up = tree.ancestors(id);
        let child = match up.iter().position(|a| a.0 == g) {
            Some(0) | None => leaf,
            Some(k) => up[k - 1].0,
        };
        return (snap.leaves_of(child), Some(g));
    }
    (snap.leaves_of(tree.top(id).0), None)
}

/// Escape inside a group: leaves it and returns the whole group's objects
/// to select, or `None` when the person is in no group.
pub fn leave_group(snap: &Snapshot, st: &mut PrepareState) -> Option<Vec<u64>> {
    let g = entered(snap, st)?;
    st.entered = None;
    Some(snap.leaves_of(g))
}

/// The last hit test and what it depended on: the hover card asks every
/// frame, and testing the fill of a large traced picture is not free, so
/// the test runs again only when the pointer moved, the view changed, or
/// the document, the selection or a shape did.
#[derive(Debug, Clone, Default)]
pub struct HitMemo {
    last: std::cell::RefCell<Option<(HitKey, Hit)>>,
    /// How many hit tests really ran (not answered from the memo).
    pub runs: std::cell::Cell<usize>,
}

#[derive(Debug, Clone, PartialEq)]
struct HitKey {
    doc_id: u64,
    doc_rev: u64,
    view: CanvasView,
    at: Pos2,
    /// The selection and every object's shape (its cached allocation) and
    /// centre, hashed.
    state: u64,
}

impl HitKey {
    fn of(snap: &Snapshot, view: &CanvasView, at: Pos2) -> Self {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        snap.selection.hash(&mut h);
        for o in &snap.objects {
            o.id.hash(&mut h);
            o.visible.hash(&mut h);
            (std::sync::Arc::as_ptr(&o.shape) as usize).hash(&mut h);
            o.transform.center_mm.x.to_bits().hash(&mut h);
            o.transform.center_mm.y.to_bits().hash(&mut h);
        }
        Self {
            doc_id: snap.doc_id,
            doc_rev: snap.doc_rev,
            view: *view,
            at,
            state: h.finish(),
        }
    }
}

/// [`hit_test`], answered from `memo` when nothing it reads changed.
fn hit_test_memo(snap: &Snapshot, view: &CanvasView, memo: &HitMemo, at: Pos2) -> Hit {
    let key = HitKey::of(snap, view, at);
    if let Some((k, hit)) = memo.last.borrow().as_ref()
        && *k == key
    {
        return *hit;
    }
    memo.runs.set(memo.runs.get() + 1);
    let hit = hit_test(snap, view, at);
    *memo.last.borrow_mut() = Some((key, hit));
    hit
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
        // Then the rotate zone, just outside a corner. A handle always wins
        // inside its own pick area (above). An object that is not selected
        // wins over the zone only near one of its outlines, and never one
        // that lies behind the whole selection (a backing piece).
        if !screen.contains(at) {
            for (i, c) in handle_points(b).into_iter().take(4).enumerate() {
                if view.mm_to_screen(c).distance(at) <= ROTATE_REACH {
                    return outline_at(snap, view, at, b).map_or(Hit::Rotate(i), Hit::Object);
                }
            }
        }
    }
    object_at(snap, view, at).map_or(Hit::Nothing, Hit::Object)
}

/// The topmost visible object that is not selected and is hit at `at` as
/// a click hits it (on its line or fill, see [`hits`]), leaving out
/// objects whose box holds the whole selection box `selection` (mm).
fn outline_at(snap: &Snapshot, view: &CanvasView, at: Pos2, selection: kurbo::Rect) -> Option<u64> {
    let mm = view.screen_to_mm(at);
    let reach = f64::from(LINE_PICK / view.px_per_mm);
    // With a hair of slack: shapes that share an edge come out of the
    // import a few nanometres apart.
    let contains = |outer: kurbo::Rect, inner: kurbo::Rect| {
        let outer = outer.inflate(1e-3, 1e-3);
        outer.x0 <= inner.x0 && outer.y0 <= inner.y0 && outer.x1 >= inner.x1 && outer.y1 >= inner.y1
    };
    snap.objects
        .iter()
        .rev()
        .filter(|o| o.visible && !snap.selection.contains(&o.id))
        .filter(|o| o.bounds().is_some_and(|b| !contains(b, selection)))
        .find(|o| hits(o, mm, reach))
        .map(|o| o.id)
}

/// How far from an object's line a click still hits it, in points.
pub const LINE_PICK: f32 = 4.0;

/// The segments of a drawn line, closing ones included, in object space.
fn line_segments(
    l: &super::prepare::Line,
) -> impl Iterator<Item = (kurbo::Point, kurbo::Point)> + '_ {
    let pts = &l.points;
    let closing = (l.closed && pts.len() > 2).then(|| (pts[pts.len() - 1], pts[0]));
    pts.windows(2).map(|w| (w[0], w[1])).chain(closing)
}

/// Whether `mm` lies within `reach` mm of one of the lines of `o`, or of
/// the painted edge of a stroke (half its width out).
fn near_outline(o: &ObjView, mm: kurbo::Point, reach: f64) -> bool {
    let p = mm - o.transform.center_mm.to_vec2();
    o.shape.lines.iter().any(|l| {
        let r = reach + l.stroke_mm / 2.0;
        line_segments(l).any(|(a, b)| segment_distance(p, a, b) <= r)
    })
}

/// Whether `mm` lies in a filled area of `o`: inside a filled path by its
/// fill rule (a hole of an even-odd sheet is not filled).
fn in_fill(o: &ObjView, mm: kurbo::Point) -> bool {
    use flatcraft_core::geometry::FillRule;
    let p = mm - o.transform.center_mm.to_vec2();
    let mut paths: std::collections::BTreeMap<usize, (FillRule, i32)> = Default::default();
    for l in &o.shape.lines {
        let Some(rule) = l.fill else { continue };
        let pts = &l.points;
        if pts.len() < 3 {
            continue;
        }
        // The winding number of the ring (a filled open line closes).
        let mut wind: i32 = 0;
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            let side = (b - a).cross(p - a);
            if a.y <= p.y {
                if b.y > p.y && side > 0.0 {
                    wind += 1;
                }
            } else if b.y <= p.y && side < 0.0 {
                wind -= 1;
            }
        }
        let e = paths.entry(l.path).or_insert((rule, 0));
        e.1 += match rule {
            FillRule::NonZero => wind,
            FillRule::EvenOdd => wind.abs() % 2,
        };
    }
    paths.values().any(|(rule, w)| match rule {
        FillRule::NonZero => *w != 0,
        FillRule::EvenOdd => w % 2 != 0,
    })
}

/// Whether a click at `mm` hits `o`, as vector apps do: on its painted
/// area (a fill) or within `reach` mm of its line, not anywhere in its box,
/// so the open middle of a frame is empty mat. An object with no lines
/// yet (a picture not traced, a shape still being built) is hit in its box.
fn hits(o: &ObjView, mm: kurbo::Point, reach: f64) -> bool {
    let Some(b) = o.bounds() else { return false };
    // The box may leave out the outer half of a thick stroke.
    let stroke = o
        .shape
        .lines
        .iter()
        .map(|l| l.stroke_mm / 2.0)
        .fold(0.0, f64::max);
    if !b.inflate(reach + stroke, reach + stroke).contains(mm) {
        return false;
    }
    o.shape.lines.is_empty() || near_outline(o, mm, reach) || in_fill(o, mm)
}

/// The distance from `p` to the segment `a`-`b`.
fn segment_distance(p: kurbo::Point, a: kurbo::Point, b: kurbo::Point) -> f64 {
    let d = b - a;
    let len2 = d.hypot2();
    let t = if len2 <= 1e-12 {
        0.0
    } else {
        ((p - a).dot(d) / len2).clamp(0.0, 1.0)
    };
    (p - (a + d * t)).hypot()
}

/// The topmost visible object under `at`: on its line or its fill (see
/// [`hits`]). Only when no object is hit that way, a selected object is
/// hit anywhere in its own box (3 points of slack), so the selection stays
/// easy to grab, while smaller objects inside a selected frame stay
/// clickable (as in Illustrator).
fn object_at(snap: &Snapshot, view: &CanvasView, at: Pos2) -> Option<u64> {
    let mm = view.screen_to_mm(at);
    let reach = f64::from(LINE_PICK / view.px_per_mm);
    let slack = f64::from(3.0 / view.px_per_mm);
    let shown = || snap.objects.iter().rev().filter(|o| o.visible);
    shown()
        .find(|o| hits(o, mm, reach))
        .or_else(|| {
            shown().find(|o| {
                snap.selection.contains(&o.id)
                    && o.bounds()
                        .is_some_and(|b| b.inflate(slack, slack).contains(mm))
            })
        })
        .map(|o| o.id)
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
        // Linked copies keep their own rotation: only the turned objects.
        d @ Drag::Rotate { ids, .. } if ids.contains(&id) => drag_turn(d),
        _ => None,
    }
}

/// The snapshot's objects as the lasso reads them: the cached shapes the
/// canvas draws and their boxes of what is cut.
struct SnapOutlines<'a> {
    by_id: std::collections::HashMap<u64, &'a ObjView>,
}

impl<'a> SnapOutlines<'a> {
    fn of(snap: &'a Snapshot) -> Self {
        Self {
            by_id: snap.objects.iter().map(|o| (o.id, o)).collect(),
        }
    }
}

impl Outlines for SnapOutlines<'_> {
    fn visible(&self, id: ObjectId) -> bool {
        self.by_id.get(&id.0).is_some_and(|o| o.visible)
    }

    fn bounds(&self, id: ObjectId) -> Option<kurbo::Rect> {
        self.by_id.get(&id.0)?.bounds()
    }

    fn with_lines(&self, id: ObjectId, f: &mut lasso::LinesVisitor<'_>) -> bool {
        let Some(o) = self.by_id.get(&id.0) else {
            return false;
        };
        let lines: Vec<(&[kurbo::Point], bool)> = o
            .shape
            .lines
            .iter()
            .map(|l| (l.points.as_slice(), l.closed))
            .collect();
        f(&lines, o.transform.center_mm.to_vec2())
    }
}

/// What the lasso `l` picks on the canvas, the same rule as
/// `selection.set_in_polygon`: inside the group the person is in, its
/// members; elsewhere whole groups. `touch` for the lasso, fully inside
/// for the box.
pub fn lasso_pick(snap: &Snapshot, st: &PrepareState, l: &Lasso, touch: bool) -> lasso::Picked {
    let within = entered(snap, st).map(ObjectId);
    lasso::pick(&snap.tree, &SnapOutlines::of(snap), l, touch, within)
}

/// The selection after a pick of `units`: they replace it, or with
/// `toggle` (Shift) each flips in or out of it.
fn combined(snap: &Snapshot, units: &[Vec<ObjectId>], toggle: bool) -> Vec<u64> {
    let current: Vec<ObjectId> = snap.selection.iter().map(|i| ObjectId(*i)).collect();
    let mode = if toggle {
        lasso::Mode::Toggle
    } else {
        lasso::Mode::Replace
    };
    lasso::combine(&current, units, mode)
        .into_iter()
        .map(|i| i.0)
        .collect()
}

/// A lasso or box smaller than this on screen, in points, is a click.
pub const TINY_LASSO: f32 = 2.0;

/// A lasso point is kept only this far from the last one, in points.
const LASSO_STEP: f32 = 1.5;

/// Draws the canvas into `rect` and handles the pointer.
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    snap: &Snapshot,
    st: &mut PrepareState,
    cut: &mut super::cut::CutState,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    st.canvas_rect = rect;
    st.rotate_cursor = None;
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
    if let Some(ids) = st.center_on.take()
        && let Some(b) = snap
            .objects
            .iter()
            .filter(|o| ids.contains(&o.id))
            .filter_map(ObjView::bounds)
            .reduce(|a, b| a.union(b))
    {
        st.view = st.view.showing(b, fit_rect(rect));
    }

    let resp = ui.interact(
        rect,
        ui.scope_id().with("flatcraft-canvas"),
        Sense::click_and_drag(),
    );
    let picking = st.keep_pick.is_some();
    if picking {
        // The keep picker owns the pointer: a click marks a region, any
        // drag pans, and nothing is selected, moved or offered a menu.
        navigate(ui, &resp, &mut st.view);
        let hover = resp.hover_pos();
        let clicked = resp.clicked_by(egui::PointerButton::Primary);
        let job = cut.job();
        if let Some(pick) = &mut st.keep_pick {
            pick.pointer(ui.ctx(), job.as_ref(), &st.view, hover, clicked);
            if pick.hover.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
    } else {
        pointer(ui, &resp, snap, st, actions);
        handle_card(ui, &resp, snap, st);
    }
    if !picking
        && resp.secondary_clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        // The same hit test as the left click.
        st.context = Some(match hit_test_memo(snap, &st.view, &st.hit_memo, p) {
            Hit::Object(id) => {
                let (ids, inside) = pick(snap, st, id, false);
                if !ids.iter().any(|i| snap.selection.contains(i)) {
                    st.entered = inside.map(|g| (snap.doc_id, g));
                }
                context_menu::target(snap, Some(ids), actions)
            }
            Hit::Handle(_) | Hit::Rotate(_) => snap.selection.iter().copied().collect(),
            Hit::Nothing => context_menu::target(snap, None, actions),
        });
    }
    if !picking {
        resp.context_menu(|ui| context_menu::show(ui, snap, st, actions));
    }

    let painter = ui.painter_at(rect);
    let view = st.view;
    let (mat_rgb, contrast) = draw_mat(&painter, rect, snap, &view, st.show_grid, t);
    // The keep picker shows what Done would keep, whatever the View
    // toggle says.
    if let (Some(pick), Some(job), Some(area)) = (&mut st.keep_pick, cut.job(), snap.area) {
        pick.draw(
            ui.ctx(),
            &painter,
            &view,
            &job,
            area,
            &mut cut.kept,
            mat_rgb,
            t,
        );
    } else if !picking
        // The plan is rebuilt only without a drag or a nudge burst, so the
        // overlay waits for them to end. Without a plan the picker's bar
        // says why.
        && st.show_kept_waste
        && st.drag.is_none()
        && st.nudge.is_none()
        && let (Some(job), Some(area)) = (cut.job(), snap.area)
    {
        cut.kept
            .draw(ui.ctx(), &painter, &view, &job, area, mat_rgb, t);
    } else if !picking && st.show_kept_waste && matches!(cut.plan, Some(Err(_))) {
        painter.text(
            rect.left_top() + Vec2::splat(RULER + t.spacing),
            egui::Align2::LEFT_TOP,
            super::kept::NO_PLAN,
            super::fonts::medium(12.0),
            t.text_dim,
        );
    }
    draw_objects(&painter, snap, &view, mat_rgb, 1.0, false, |id| {
        drag_preview(st.drag.as_ref(), snap, id)
    });
    let mut did_not_fit = Vec::new();
    if let Some(p) = st.panel.clone() {
        // Packed once per change, for the ghosts, the fixed objects and
        // the ones that do not fit alike.
        let plan = st.auto_plan.get(snap, &p);
        if let Some(plan) = plan {
            draw_fixed(
                &painter,
                snap,
                &view,
                contrast,
                &plan.fixed,
                p.values.spacing_mm,
            );
            did_not_fit.clone_from(&plan.did_not_fit);
        }
        draw_ghosts(
            &painter,
            snap,
            &view,
            mat_rgb,
            &super::arrange::ghosts_with(snap, &p, plan),
        );
    }
    draw_area(&painter, snap, &view, t, contrast);
    // The handles of the selection would look draggable while picking.
    if !picking && let Some(r) = draw_selection(&painter, snap, st, rgb(selection_colour(mat_rgb)))
    {
        link_badge(ui, r, snap, contrast);
    }
    // Over the selection box, which would hide the outline otherwise.
    draw_did_not_fit(&painter, snap, &view, t.danger, &did_not_fit);
    match &st.drag {
        Some(Drag::Frame {
            start,
            current,
            lasso: None,
            ..
        }) => {
            let r = view.rect_to_screen(kurbo::Rect::from_points(*start, *current));
            let pick = rgb(selection_colour(mat_rgb));
            painter.rect_filled(r, 0.0, pick.gamma_multiply(0.12));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, pick), egui::StrokeKind::Middle);
        }
        // The lasso's path as a dashed line that closes back to its start.
        Some(Drag::Frame {
            current,
            lasso: Some(points),
            ..
        }) => {
            let pick = rgb(selection_colour(mat_rgb));
            let mut path: Vec<Pos2> = points.iter().map(|p| view.mm_to_screen(*p)).collect();
            path.push(view.mm_to_screen(*current));
            path.push(path[0]);
            painter.extend(egui::Shape::dashed_line(
                &path,
                Stroke::new(1.5, pick),
                6.0,
                4.0,
            ));
        }
        _ => {}
    }
    if st.show_rulers {
        draw_rulers(&painter, rect, &view, t);
    }
    if let Some(at) = st.rotate_cursor {
        rotate_cursor(ui, at, t);
    }
}

/// egui has no rotate cursor: it hides the pointer and paints a turning
/// arrow at `at`, on top of everything.
fn rotate_cursor(ui: &Ui, at: Pos2, t: &Tokens) {
    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::unique("flatcraft-rotate-cursor"),
    ));
    let size = 16.0;
    painter.circle(
        at,
        size * 0.5 + 3.0,
        t.panel.gamma_multiply(0.9),
        Stroke::new(1.0, t.border),
    );
    let tex = super::icons::texture_colored(ui.ctx(), super::icons::Icon::Rotate, size, t.text);
    painter.image(
        tex.id(),
        Rect::from_center_size(at, Vec2::splat(size)),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
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

/// The Auto arrange box of an object, or of a whole group (its visible
/// members).
fn unit_arrange_box(snap: &Snapshot, id: u64) -> Option<kurbo::Rect> {
    if let Some(o) = snap.object(id) {
        return o.arrange_bounds();
    }
    snap.leaves_of(id)
        .iter()
        .filter_map(|l| snap.object(*l))
        .filter(|o| o.visible)
        .filter_map(ObjView::arrange_bounds)
        .reduce(|a, b| a.union(b))
}

/// Marks the objects an open Auto arrange panel packs around (see
/// [`super::arrange::fixed`]): their box, and faintly the band of
/// `spacing_mm` around it that the packed objects keep clear of.
pub(crate) fn draw_fixed(
    painter: &egui::Painter,
    snap: &Snapshot,
    view: &CanvasView,
    contrast: Color32,
    fixed: &[u64],
    spacing_mm: f64,
) {
    let stroke = Stroke::new(1.0, contrast.gamma_multiply(0.7));
    for id in fixed {
        let Some(b) = unit_arrange_box(snap, *id) else {
            continue;
        };
        let zone = view.rect_to_screen(b.inflate(spacing_mm, spacing_mm));
        painter.rect_filled(zone, 0.0, contrast.gamma_multiply(0.08));
        painter.rect_stroke(
            view.rect_to_screen(b),
            0.0,
            stroke,
            egui::StrokeKind::Middle,
        );
    }
}

/// Marks the chosen objects an open Auto arrange panel cannot place (see
/// [`super::arrange::did_not_fit_note`]): a faint red fill and a red
/// outline of their box.
pub(crate) fn draw_did_not_fit(
    painter: &egui::Painter,
    snap: &Snapshot,
    view: &CanvasView,
    danger: Color32,
    ids: &[u64],
) {
    for id in ids {
        if let Some(b) = unit_arrange_box(snap, *id) {
            let r = view.rect_to_screen(b);
            painter.rect_filled(r, 0.0, danger.gamma_multiply(0.12));
            painter.rect_stroke(r, 0.0, Stroke::new(2.0, danger), egui::StrokeKind::Outside);
        }
    }
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
        // A picture that is not traced yet has no lines: its image box.
        if o.shape.lines.is_empty()
            && let Some(f) = o.shape.arrange
        {
            let pts: Vec<Pos2> = [
                (f.x0, f.y0),
                (f.x1, f.y0),
                (f.x1, f.y1),
                (f.x0, f.y1),
                (f.x0, f.y0),
            ]
            .into_iter()
            .map(|(x, y)| view.mm_to_screen(*place * kurbo::Point::new(x, y)))
            .collect();
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
pub fn fit_rect(rect: Rect) -> Rect {
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
        // Past the rotate zone of the corner, so it never covers it.
        r.right_top() + Vec2::new(ROTATE_REACH + 2.0, -size - ROTATE_REACH - 2.0),
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
        "{n} linked copies share the source, the cut options, the keep mode and the scale"
    ));
}

/// Draws the selection box and its handles. Returns the box on screen.
fn draw_selection(
    painter: &egui::Painter,
    snap: &Snapshot,
    st: &PrepareState,
    pick: Color32,
) -> Option<Rect> {
    // Inside a group: its box, dashed, around the picked member.
    if let Some(g) = entered(snap, st)
        && let Some(gb) = snap.box_of(&snap.leaves_of(g))
    {
        let r = st.view.rect_to_screen(gb).expand(4.0);
        let dash = Stroke::new(1.0, pick.gamma_multiply(0.8));
        let corners = [
            r.left_top(),
            r.right_top(),
            r.right_bottom(),
            r.left_bottom(),
        ];
        for i in 0..4 {
            painter.extend(egui::Shape::dashed_line(
                &[corners[i], corners[(i + 1) % 4]],
                dash,
                5.0,
                4.0,
            ));
        }
    }
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

/// Over a handle or the rotate zone (and not dragging): its cursor and
/// its card.
fn handle_card(ui: &Ui, resp: &egui::Response, snap: &Snapshot, st: &mut PrepareState) {
    // A lasso being drawn, or the Lasso tool ready: the cross aims.
    let lassoing = matches!(st.drag, Some(Drag::Frame { lasso: Some(_), .. }));
    if lassoing {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if st.drag.is_some() {
        return;
    }
    let Some(at) = resp.hover_pos() else { return };
    let hit = hit_test_memo(snap, &st.view, &st.hit_memo, at);
    match look(hit, ui.input(|i| i.modifiers)) {
        Look::Resize(i) => ui.ctx().set_cursor_icon(handle_cursor(i)),
        Look::Rotate => st.rotate_cursor = Some(at),
        Look::Plain if st.select_tool == SelectTool::Lasso => {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair)
        }
        Look::Plain => {}
    }
    let (title, card, why) = match hit {
        Hit::Handle(i) => {
            let ids: Vec<u64> = snap.selection.iter().copied().collect();
            let title = if i < 4 {
                "Corner handle"
            } else {
                "Side handle"
            };
            (
                title,
                super::tools::Setting::ScaleHandle.card(),
                aspect_lock(snap, &ids),
            )
        }
        Hit::Rotate(_) => ("Rotate", super::tools::Setting::RotateZone.card(), None),
        Hit::Object(_) | Hit::Nothing => return,
    };
    // At a top corner the card opens above the pointer, away from the
    // selection.
    let above = matches!(hit, Hit::Rotate(0 | 1) | Hit::Handle(0 | 1 | 4));
    super::groups::card_at_pointer(resp, title, card, why, above);
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
    let (space, mods, wheels, zoom, hover) = ui.input(|i| {
        (
            i.key_down(help::pan_key()),
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
        let hit = hit_test_memo(snap, &st.view, &st.hit_memo, origin);
        let lasso_tool = st.select_tool == SelectTool::Lasso;
        // The box or lasso: Option (Alt) at the start draws a lasso; Shift
        // flips what it picks.
        let frame = |lasso: bool| {
            Some(Drag::Frame {
                doc_id,
                start: mm,
                current: mm,
                toggle: help::held(mods, PrepareKey::ToggleInSelection),
                lasso: lasso.then(|| vec![mm]),
            })
        };
        st.drag = match hit {
            // The Lasso tool lassos from over objects too, so a packed
            // layout can be picked from; handles and the rotate zone keep
            // their meaning.
            Hit::Object(_) if lasso_tool => frame(true),
            // The rotate zone, or a corner with Cmd (macOS) or Ctrl held.
            Hit::Rotate(_) | Hit::Handle(_) if look(hit, mods) == Look::Rotate => {
                rotate_drag(snap, mm)
            }
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
                // Cmd (Ctrl) picks one member and drags it alone, even out
                // of a selected group.
                let ids: Vec<u64> = if snap.selection.contains(&id) && !deep_click(mods) {
                    snap.selection.iter().copied().collect()
                } else {
                    let (ids, inside) = pick(snap, st, id, deep_click(mods));
                    st.entered = inside.map(|g| (doc_id, g));
                    actions.push(Action::run(doc_id, "object.select", json!({ "ids": ids })));
                    ids
                };
                Some(Drag::Move {
                    doc_id,
                    ids,
                    start: mm,
                    current: mm,
                })
            }
            // Option (Alt) alone: AltGr on Windows and Linux keyboards
            // reports Ctrl+Alt and must not start a lasso.
            Hit::Rotate(_) | Hit::Nothing => frame(
                lasso_tool || (help::held(mods, PrepareKey::Lasso) && !mods.ctrl && !mods.command),
            ),
        };
    }
    if let Some(p) = resp.interact_pointer_pos() {
        let mm = st.view.screen_to_mm(p);
        if let Some(
            Drag::Move { current, .. }
            | Drag::Scale { current, .. }
            | Drag::Rotate { current, .. }
            | Drag::Frame { current, .. },
        ) = &mut st.drag
        {
            *current = mm;
        }
        if let Some(Drag::Frame {
            lasso: Some(points),
            ..
        }) = &mut st.drag
            && points
                .last()
                .is_none_or(|l| st.view.mm_to_screen(*l).distance(p) >= LASSO_STEP)
        {
            points.push(mm);
        }
    }
    // Shift and Option count while the handle is dragged, up to the drop.
    if let Some(Drag::Scale { flip, centre, .. }) = &mut st.drag {
        *flip = help::held(mods, PrepareKey::FlipKeepAspect);
        *centre = help::held(mods, PrepareKey::ScaleFromCentre);
    }
    if let Some(Drag::Scale { handle, .. }) = &st.drag {
        ui.ctx().set_cursor_icon(handle_cursor(*handle));
    }
    // Shift snaps the turn while rotating, up to the drop.
    if let Some(Drag::Rotate { snap, .. }) = &mut st.drag {
        *snap = help::held(mods, PrepareKey::SnapAngle);
        st.rotate_cursor = resp.interact_pointer_pos().or(hover);
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
            Some(d @ Drag::Rotate { .. }) => {
                if let Some(a) = rotate_action(&d) {
                    actions.push(a);
                }
            }
            Some(Drag::Frame {
                doc_id,
                start,
                current,
                toggle,
                lasso,
            }) => {
                let mut points = lasso.clone().unwrap_or_else(|| vec![start]);
                points.push(current);
                let screen = points.iter().fold(Rect::NOTHING, |r, p| {
                    r.union(Rect::from_min_size(st.view.mm_to_screen(*p), Vec2::ZERO))
                });
                if screen.width() < TINY_LASSO && screen.height() < TINY_LASSO {
                    // Hardly moved: a click where it began.
                    let at = st.view.mm_to_screen(start);
                    click_at(snap, st, at, false, toggle, actions);
                    return;
                }
                let shape = match &lasso {
                    Some(_) => Lasso::new(points),
                    None => Lasso::rect(kurbo::Rect::from_points(start, current)),
                };
                let units = shape.map_or_else(Vec::new, |l| {
                    lasso_pick(snap, st, &l, lasso.is_some()).units
                });
                let ids = combined(snap, &units, toggle);
                actions.push(Action::run(doc_id, "object.select", json!({ "ids": ids })));
            }
            _ => {}
        }
    }

    // A double-click goes into the group: the object alone. The first
    // click of it already picked the whole group.
    let deep = deep_click(mods) || resp.double_clicked();
    if (resp.clicked() || resp.double_clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let toggle = help::held(mods, PrepareKey::ToggleInSelection);
        click_at(snap, st, p, deep, toggle, actions);
    }
}

/// A click at `p`: picks what is under it (`deep` goes into its group),
/// or clears the selection on the empty mat. With `toggle` (Shift) the
/// picked object or group flips in or out of the selection instead, and
/// the empty mat changes nothing.
fn click_at(
    snap: &Snapshot,
    st: &mut PrepareState,
    p: Pos2,
    deep: bool,
    toggle: bool,
    actions: &mut Vec<Action>,
) {
    let doc_id = snap.doc_id;
    let hit = match hit_test_memo(snap, &st.view, &st.hit_memo, p) {
        // Inside the selection box a double-click still reaches the
        // object under the pointer.
        Hit::Handle(_) | Hit::Rotate(_) if deep => {
            object_at(snap, &st.view, p).map_or(Hit::Nothing, Hit::Object)
        }
        other => other,
    };
    let ids: Vec<u64> = match hit {
        Hit::Object(id) => {
            let (picked, inside) = pick(snap, st, id, deep);
            st.entered = inside.map(|g| (doc_id, g));
            // The Objects panel's Shift range starts at the row clicked
            // here (the object, or its group as one row).
            if let [row] = snap.units_of(&picked).as_slice() {
                st.list_anchor = Some((doc_id, *row));
            }
            if toggle {
                let unit: Vec<ObjectId> = picked.into_iter().map(ObjectId).collect();
                combined(snap, &[unit], true)
            } else {
                picked
            }
        }
        Hit::Handle(_) | Hit::Rotate(_) => return,
        Hit::Nothing if toggle => return,
        Hit::Nothing => {
            st.entered = None;
            Vec::new()
        }
    };
    actions.push(Action::run(doc_id, "object.select", json!({ "ids": ids })));
}

/// A click with Cmd (macOS) or Ctrl held picks one member of a group.
fn deep_click(mods: egui::Modifiers) -> bool {
    mods.command
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
        let mut p = json!({
            "ids": ids,
            "keep_aspect": false,
            "anchor": s.anchor_name,
            "group": ids.len() > 1,
        });
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

    #[test]
    fn the_distance_to_a_segment_is_to_its_nearest_point() {
        let a = kurbo::Point::new(0.0, 0.0);
        let b = kurbo::Point::new(10.0, 0.0);
        assert_eq!(segment_distance(kurbo::Point::new(5.0, 3.0), a, b), 3.0);
        assert_eq!(segment_distance(kurbo::Point::new(13.0, 4.0), a, b), 5.0);
        assert_eq!(segment_distance(kurbo::Point::new(3.0, 4.0), a, a), 5.0);
    }

    #[test]
    fn a_turn_grows_clockwise_on_screen() {
        let o = kurbo::Point::new(0.0, 0.0);
        let right = kurbo::Point::new(10.0, 0.0);
        let below = kurbo::Point::new(0.0, 10.0);
        // From the right of the pivot to below it is a quarter turn
        // clockwise with y down.
        assert!((rotate_delta(o, right, below, None, false) - 90.0).abs() < 1e-9);
        assert!((rotate_delta(o, below, right, None, false) + 90.0).abs() < 1e-9);
        // Past half a turn it goes the short way.
        let up_left = kurbo::Point::new(-10.0, -1.0);
        assert!(rotate_delta(o, right, up_left, None, false) < 0.0);
    }

    #[test]
    fn shift_snaps_the_angle_of_one_and_the_turn_of_several() {
        let o = kurbo::Point::new(0.0, 0.0);
        let at = |deg: f64| {
            let r = deg.to_radians();
            kurbo::Point::new(r.cos() * 10.0, r.sin() * 10.0)
        };
        let start = at(0.0);
        // One object at 10 degrees, turned 52: 62 snaps to 60.
        assert!((rotate_delta(o, start, at(52.0), Some(10.0), true) - 50.0).abs() < 1e-9);
        // Several: the turn itself snaps, 52 to 45.
        assert!((rotate_delta(o, start, at(52.0), None, true) - 45.0).abs() < 1e-9);
        // Without Shift nothing snaps.
        assert!((rotate_delta(o, start, at(52.0), Some(10.0), false) - 52.0).abs() < 1e-9);
    }

    #[test]
    fn angles_are_shown_in_0_to_360() {
        assert_eq!(normalise_deg(-30.0), 330.0);
        assert_eq!(normalise_deg(360.0), 0.0);
        assert_eq!(normalise_deg(359.999), 0.0);
        assert_eq!(normalise_deg(-0.0).to_string(), "0");
        assert_eq!(common_angle([10.0, 10.0].into_iter()), Some(10.0));
        assert_eq!(common_angle([0.0, 359.999_999_9].into_iter()), Some(0.0));
        assert_eq!(common_angle([10.0, 20.0].into_iter()), None);
        assert_eq!(common_angle(std::iter::empty()), None);
    }

    #[test]
    fn the_pointer_looks_like_what_a_drag_does() {
        let cmd = help::drag_modifiers(help::Os::CURRENT, PrepareKey::RotateOnCorner);
        let none = egui::Modifiers::NONE;
        assert_eq!(look(Hit::Rotate(2), none), Look::Rotate);
        assert_eq!(look(Hit::Handle(2), none), Look::Resize(2));
        assert_eq!(look(Hit::Handle(2), cmd), Look::Rotate);
        // A side handle never rotates.
        assert_eq!(look(Hit::Handle(5), cmd), Look::Resize(5));
        assert_eq!(look(Hit::Object(1), cmd), Look::Plain);
    }
}
