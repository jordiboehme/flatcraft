//! Cut geometry of an object per cut mode (`CutMode`): centre lines, fill
//! edges and stroke outlines, in mat millimetres.
//!
//! Stroke outlines come from kurbo's stroker, run on the placed path with
//! the placed width, so the width scales with the object. kurbo's outline
//! is meant for non-zero filling: it overlaps itself at inner corners and
//! where a path crosses itself. So the outline is flattened and united with
//! itself (i_overlay, non-zero rule); what is left are the real edges of the
//! painted area, with real counters kept as holes. A filled path with an
//! outline stroke is cut along the edge of fill united with stroke. A
//! subpath whose fill is hidden under its thin stroke is cut as a line.
//! All booleans stay inside one path of one object; merging across objects is
//! milestone 2.
//!
//! Error budget per cut path (tolerance `t`, the plotter's flatten
//! tolerance, default 0.01 mm): stroker `t/4` plus flattening `t/4`, or
//! flattening `t/2` for fills, so the shape is within `t/2` before the
//! booleans. The booleans work on a fixed-point grid far below that (about
//! 1e-7 mm for 100 mm). The pipeline then flattens the straight lines
//! without error, but its cleaning merges nearly collinear points within
//! up to `t`. Worst case about `1.5 t` (0.015 mm by default), below the
//! 0.05 mm plotter step.
//!
//! Traced raster images are fills, so in object outlines they go through
//! the fill boolean too; for their simple outlines this only normalises
//! direction and start point.

use std::collections::BTreeMap;
use std::sync::Arc;

use flatcraft_core::document::{CutMode, CutOptions, Object, ObjectId, StrokeMode};
use flatcraft_core::geometry::{Cap, CutPath, FillRule, Join, Paint};
use flatcraft_core::transform::Transform;
use i_overlay::core::fill_rule::FillRule as OverlayFill;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::core::solver::Solver;
use i_overlay::float::overlay::{FloatOverlay, OverlayOptions};
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape, Stroke, StrokeOpts};

use crate::job::{JobInput, JobObject};
use flatcraft_core::document::Document;

/// Smallest tolerance used for stroking and flattening, mm. Guards the
/// stroker against a zero or broken profile value; the pipeline refuses
/// such a profile anyway.
const MIN_TOLERANCE_MM: f64 = 1e-4;

/// Subpath ends closer than this count as closed (the pipeline's join
/// tolerance), mm.
pub const CLOSE_TOLERANCE_MM: f64 = 0.01;

/// The reach of the spine helpers of a closed hairline ([`spine`],
/// [`hairline_ends`], [`ring_samples`]), mm: two plotter steps of 0.05 mm.
/// Which closed lines are hairlines is decided by [`hairline_mm`], from the
/// minimum feature size.
pub const HAIRLINE_MM: f64 = 0.1;

/// Stroker tolerance for bounds of the cut geometry, mm (the default
/// flatten tolerance).
pub const BOUNDS_TOLERANCE_MM: f64 = 0.01;

type Contour = Vec<[f64; 2]>;

/// One cut path of an object, in mat mm: what the job, the export, the
/// report and the window read. `source` is the index of the object's
/// source path it came from (`None` for a path made of several source
/// paths, such as a welded outline), `label` that path's label. Listing
/// notes and labels come from here, never from a piece's index.
#[derive(Debug, Clone, PartialEq)]
pub struct CutPiece {
    pub source: Option<usize>,
    pub label: Option<String>,
    pub path: BezPath,
}

/// What an object's cut left out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PieceFacts {
    /// Filled source paths without stroke that give nothing to cut: no
    /// area, or thinner than the cut tolerance. Paths left out only as
    /// slivers are counted in `slivers` instead.
    pub no_area: usize,
    /// Filled shapes dropped as slivers (see [`fill_edges`]).
    pub slivers: usize,
}

/// Everything an object's pieces depend on. The geometry `Arc` is held, so
/// its address cannot be reused by other geometry while the key names it.
#[derive(Debug, Clone)]
struct PieceKey {
    geometry: Arc<Vec<CutPath>>,
    transform: Transform,
    local_bounds: Rect,
    cut: CutOptions,
    name: String,
}

impl PieceKey {
    fn of(o: &JobObject) -> Self {
        Self {
            geometry: o.geometry.clone(),
            transform: o.transform,
            local_bounds: o.local_bounds,
            cut: o.cut,
            name: o.name.clone(),
        }
    }

    fn fits(&self, o: &JobObject) -> bool {
        Arc::ptr_eq(&self.geometry, &o.geometry)
            && self.transform == o.transform
            && self.local_bounds == o.local_bounds
            && self.cut == o.cut
            && self.name == o.name
    }
}

/// The cut geometry of a whole document: the pieces of every object
/// (hidden and skipped ones too, which the report checks), what each cut
/// left out, and the warnings about it in document order. Built by
/// [`document_cut_geometry`] and cached per document
/// (`DocState::cut_geometry`); the job, the export, the report and the
/// window all read this one copy.
#[derive(Debug, Clone, Default)]
pub struct CutGeometry {
    pub per_object: BTreeMap<ObjectId, Arc<Vec<CutPiece>>>,
    pub facts: BTreeMap<ObjectId, PieceFacts>,
    /// The box of what is cut of each object (see [`path_box`]): slivers
    /// that are not cut never count. Every check that measures an object
    /// reads it here.
    pub bounds: BTreeMap<ObjectId, Option<Rect>>,
    pub warnings: Vec<String>,
    /// The flatten tolerance it was built with, mm.
    pub tolerance_mm: f64,
    /// The minimum feature width it was built with, mm.
    pub min_width_mm: f64,
    keys: BTreeMap<ObjectId, PieceKey>,
    /// Object ids in document order.
    order: Vec<ObjectId>,
}

impl CutGeometry {
    /// The pieces of object `id` (empty for an unknown id).
    pub fn pieces(&self, id: ObjectId) -> &[CutPiece] {
        self.per_object.get(&id).map_or(&[], |p| p.as_slice())
    }

    /// The box of what is cut of object `id`.
    pub fn bounds(&self, id: ObjectId) -> Option<Rect> {
        self.bounds.get(&id).copied().flatten()
    }

    /// What the cut of object `id` left out.
    pub fn facts(&self, id: ObjectId) -> PieceFacts {
        self.facts.get(&id).copied().unwrap_or_default()
    }

    /// True if this is the geometry of exactly `objects` (same ids in the
    /// same order, each with the same geometry `Arc`, placement, cut
    /// options and name) at this tolerance and width. Cheap: nothing is
    /// walked but the list.
    pub fn is_for(&self, objects: &[JobObject], tolerance_mm: f64, min_width_mm: f64) -> bool {
        same_f64(self.tolerance_mm, tolerance_mm)
            && same_f64(self.min_width_mm, min_width_mm)
            && self.order.len() == objects.len()
            && self
                .order
                .iter()
                .zip(objects)
                .all(|(id, o)| *id == o.id && self.keys.get(id).is_some_and(|k| k.fits(o)))
    }
}

fn same_f64(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits()
}

/// The cut geometry of `doc` at the flatten tolerance `tol` and the
/// minimum feature width `min_width_mm`: the document-level entry.
pub fn document_cut_geometry(doc: &Document, tol: f64, min_width_mm: f64) -> CutGeometry {
    let objects = JobInput::objects_of(doc);
    (*objects_cut_geometry(&objects, tol, min_width_mm, None)).clone()
}

/// The cut geometry of `objects`. `previous` is an earlier build: when it
/// is for exactly these objects it is returned as it is, else the pieces
/// of every object it holds unchanged are taken over and only the others
/// are cut again. So a stale `previous` costs nothing but time and can
/// never give a wrong piece.
pub fn objects_cut_geometry(
    objects: &[JobObject],
    tol: f64,
    min_width_mm: f64,
    previous: Option<&Arc<CutGeometry>>,
) -> Arc<CutGeometry> {
    if let Some(p) = previous
        && p.is_for(objects, tol, min_width_mm)
    {
        return p.clone();
    }
    let reuse = previous
        .filter(|p| same_f64(p.tolerance_mm, tol) && same_f64(p.min_width_mm, min_width_mm));
    let mut out = CutGeometry {
        tolerance_mm: tol,
        min_width_mm,
        ..CutGeometry::default()
    };
    for o in objects {
        let kept = reuse.and_then(|p| {
            let k = p.keys.get(&o.id).filter(|k| k.fits(o))?;
            Some((
                k.clone(),
                p.per_object.get(&o.id)?.clone(),
                p.facts(o.id),
                p.bounds(o.id),
            ))
        });
        let (key, pieces, facts, bounds) = match kept {
            Some(k) => k,
            None => {
                #[cfg(test)]
                tests::OBJECT_CUTS.with(|n| n.set(n.get() + 1));
                let affine = o.transform.affine(o.local_bounds);
                let (pieces, facts, bounds) =
                    object_pieces(&o.geometry, &o.cut, affine, tol, min_width_mm);
                (PieceKey::of(o), Arc::new(pieces), facts, bounds)
            }
        };
        if facts.slivers > 0 {
            out.warnings
                .push(sliver_warning(&o.name, facts.slivers, min_width_mm));
        }
        out.order.push(o.id);
        out.keys.insert(o.id, key);
        out.per_object.insert(o.id, pieces);
        out.facts.insert(o.id, facts);
        out.bounds.insert(o.id, bounds);
    }
    Arc::new(out)
}

/// `primary` with the pieces of `extra` added for the objects `primary`
/// does not hold, as a seed for [`objects_cut_geometry`]: a command that
/// ran on a snapshot built the pieces of its new object there, and the
/// current document's build takes them over. Only pieces whose key still
/// fits are ever taken, so a seed never gives a wrong piece.
pub(crate) fn merged_seed(
    primary: &Arc<CutGeometry>,
    extra: &Arc<CutGeometry>,
) -> Arc<CutGeometry> {
    if !same_f64(primary.tolerance_mm, extra.tolerance_mm)
        || !same_f64(primary.min_width_mm, extra.min_width_mm)
    {
        return primary.clone();
    }
    let mut out = (**primary).clone();
    for (id, key) in &extra.keys {
        if out.keys.contains_key(id) {
            continue;
        }
        if let Some(pieces) = extra.per_object.get(id) {
            out.keys.insert(*id, key.clone());
            out.per_object.insert(*id, pieces.clone());
            out.facts.insert(*id, extra.facts(*id));
            out.bounds.insert(*id, extra.bounds(*id));
        }
    }
    Arc::new(out)
}

/// The warning about `n` filled slivers of the object `name` that are
/// thinner than `width_mm` and not cut.
pub fn sliver_warning(name: &str, n: usize, width_mm: f64) -> String {
    let name: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let (what, verb) = if n == 1 {
        ("1 filled sliver is".to_string(), "is")
    } else {
        (format!("{n} filled slivers are"), "are")
    };
    format!("'{name}': {what} thinner than {width_mm} mm and {verb} not cut.")
}

/// The pieces of one object's source paths placed with `affine`, what the
/// cut left out, and the box of what is cut ([`path_box`] of each path).
pub fn object_pieces(
    geometry: &[CutPath],
    options: &CutOptions,
    affine: Affine,
    tol: f64,
    min_width_mm: f64,
) -> (Vec<CutPiece>, PieceFacts, Option<Rect>) {
    let mut pieces = Vec::new();
    let mut facts = PieceFacts::default();
    let mut bounds: Option<Rect> = None;
    for (i, p) in geometry.iter().enumerate() {
        let (cut, slivers) = cut_path_counted(p, options, affine, tol, min_width_mm);
        facts.slivers += slivers;
        if let Some(b) = path_box(p, options, affine, &cut, slivers) {
            bounds = Some(bounds.map_or(b, |a| a.union(b)));
        }
        // A stroke-less fill that gives nothing, not even a sliver: no
        // area, or thinner than the tolerance. It is not visible.
        if cut.is_empty()
            && slivers == 0
            && p.paint.stroke.is_none()
            && !p.path.elements().is_empty()
        {
            facts.no_area += 1;
        }
        pieces.extend(cut.into_iter().map(|path| CutPiece {
            source: Some(i),
            label: p.label.clone(),
            path,
        }));
    }
    (pieces, facts, bounds)
}

/// The cut paths of `object` placed with `placed_affine`, in mat mm.
pub fn cut_paths(
    object: &Object,
    placed_affine: Affine,
    tolerance_mm: f64,
    min_width_mm: f64,
) -> Vec<BezPath> {
    object
        .geometry
        .iter()
        .flat_map(|p| cut_path(p, &object.cut, placed_affine, tolerance_mm, min_width_mm))
        .collect()
}

/// What a box of what is cut is measured with: the plotter's flatten
/// tolerance and the knife's narrowest feature (`report::min_width_mm`),
/// the same two the cut geometry is built with, so a command's box and
/// the cached one ([`CutGeometry::bounds`]) are equal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Knife {
    pub tolerance_mm: f64,
    pub min_width_mm: f64,
}

impl Default for Knife {
    /// The default plotter: 0.01 mm tolerance, a 0.25 mm blade offset.
    fn default() -> Self {
        Self {
            tolerance_mm: BOUNDS_TOLERANCE_MM,
            min_width_mm: 0.125,
        }
    }
}

/// Bounding box of what is cut of the object in mat mm, cut with `knife`;
/// `None` if the object has no paths. The same rule as the boxes of the
/// cut geometry ([`CutGeometry::bounds`]): a fill is cut, so a sliver that
/// is not cut never counts (see [`path_box`]). Each box is kept per
/// shape, scale, turn and cut options (not position), so asking again is
/// cheap. A fill-only path with no area (cut as nothing) still counts with
/// its centre line.
pub fn cut_bounds(object: &Object, knife: Knife) -> Option<Rect> {
    #[cfg(test)]
    tests::CUT_BOUNDS_CALLS.with(|n| n.set(n.get() + 1));
    let mut centred = object.transform;
    centred.center_mm = Point::ORIGIN;
    let t = &centred;
    let key = BoundsMemoKey {
        geometry: Arc::as_ptr(&object.geometry) as usize,
        numbers: [
            t.scale_x,
            t.scale_y,
            t.rotation_deg,
            object.local_bounds.x0,
            object.local_bounds.y0,
            object.local_bounds.x1,
            object.local_bounds.y1,
            object.cut.auto_threshold_mm,
            knife.min_width_mm,
            knife.tolerance_mm,
        ]
        .map(f64::to_bits),
        mirror: t.mirror,
        mode: object.cut.mode as u8,
        strokes: object.cut.strokes as u8,
    };
    let memo = BOUNDS_MEMO.get_or_init(Default::default);
    let found = memo
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key, &object.geometry);
    let at_origin = match found {
        Some(b) => b,
        None => {
            let affine = centred.affine(object.local_bounds);
            let b = cut_bounds_of(&object.geometry, &object.cut, affine, knife);
            memo.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .put(key, &object.geometry, b);
            b
        }
    };
    at_origin.map(|b| b + object.transform.center_mm.to_vec2())
}

/// What a box of [`cut_bounds`] depends on, but the position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct BoundsMemoKey {
    geometry: usize,
    numbers: [u64; 10],
    mirror: bool,
    mode: u8,
    strokes: u8,
}

/// The boxes [`cut_bounds`] worked out, the most recently used ones. Each
/// holds its geometry weakly: it never keeps a closed document's shapes
/// alive, and a box whose geometry is gone (or whose address now belongs
/// to other geometry) is never used. At most [`BOUNDS_MEMO_SIZE`] boxes.
#[derive(Debug, Default)]
struct BoundsMemo {
    entries: std::collections::HashMap<BoundsMemoKey, MemoEntry>,
    clock: u64,
}

#[derive(Debug)]
struct MemoEntry {
    geometry: std::sync::Weak<Vec<CutPath>>,
    bounds: Option<Rect>,
    used: u64,
}

impl BoundsMemo {
    fn get(&mut self, key: &BoundsMemoKey, geometry: &Arc<Vec<CutPath>>) -> Option<Option<Rect>> {
        self.clock += 1;
        let clock = self.clock;
        let e = self.entries.get_mut(key)?;
        if !e
            .geometry
            .upgrade()
            .is_some_and(|g| Arc::ptr_eq(&g, geometry))
        {
            self.entries.remove(key);
            return None;
        }
        e.used = clock;
        Some(e.bounds)
    }

    fn put(&mut self, key: BoundsMemoKey, geometry: &Arc<Vec<CutPath>>, bounds: Option<Rect>) {
        if self.entries.len() >= BOUNDS_MEMO_SIZE {
            self.entries.retain(|_, e| e.geometry.strong_count() > 0);
        }
        while self.entries.len() >= BOUNDS_MEMO_SIZE {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| *k)
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
        self.clock += 1;
        self.entries.insert(
            key,
            MemoEntry {
                geometry: Arc::downgrade(geometry),
                bounds,
                used: self.clock,
            },
        );
    }
}

/// The memo of [`cut_bounds`], for every session of the process.
static BOUNDS_MEMO: std::sync::OnceLock<std::sync::Mutex<BoundsMemo>> = std::sync::OnceLock::new();

/// At most this many boxes are kept; the least recently used goes first.
const BOUNDS_MEMO_SIZE: usize = 256;

#[cfg(test)]
pub(crate) fn bounds_memo_len() -> usize {
    BOUNDS_MEMO
        .get()
        .map_or(0, |m| m.lock().unwrap().entries.len())
}

/// [`cut_bounds`] of geometry placed with `affine` and cut with `options`,
/// worked out every time.
pub fn cut_bounds_of(
    geometry: &[CutPath],
    options: &CutOptions,
    affine: Affine,
    knife: Knife,
) -> Option<Rect> {
    geometry
        .iter()
        .filter_map(|p| {
            if cuts_its_fill(p, options, affine) {
                let (cut, slivers) =
                    cut_path_counted(p, options, affine, knife.tolerance_mm, knife.min_width_mm);
                path_box(p, options, affine, &cut, slivers)
            } else {
                shape_bounds(p, options, affine)
            }
        })
        .reduce(|a, b| a.union(b))
}

/// True if the cut of `path` is the edge of its fill (object outlines,
/// a fill, no outline stroke): only such a cut can drop slivers.
fn cuts_its_fill(path: &CutPath, options: &CutOptions, affine: Affine) -> bool {
    options.mode == CutMode::ObjectOutlines
        && path.paint.fill.is_some()
        && outline_style(path.paint, options, affine).is_none()
}

/// The box of what is cut of `path`, given its cut `cut` and how many
/// slivers it dropped: the pieces themselves when slivers were dropped (so
/// they never count), else the box of the drawn shape ([`shape_bounds`]),
/// which follows curves exactly and holds every piece.
pub(crate) fn path_box(
    path: &CutPath,
    options: &CutOptions,
    affine: Affine,
    cut: &[BezPath],
    slivers: usize,
) -> Option<Rect> {
    if slivers > 0 {
        return cut
            .iter()
            .map(Shape::bounding_box)
            .reduce(|a, b| a.union(b));
    }
    shape_bounds(path, options, affine)
}

/// The placed width of the stroke of `paint` when the stroke mode Auto
/// cuts it once, along its middle, because it is thinner than the
/// object's threshold, else `None`.
pub fn auto_centre_line_width(paint: Paint, options: &CutOptions, affine: Affine) -> Option<f64> {
    let s = paint.stroke?;
    let width = placed_width(s.width_mm, affine);
    (options.mode == CutMode::ObjectOutlines
        && options.strokes == StrokeMode::Auto
        && width.is_finite()
        && width < options.auto_threshold_mm)
        .then_some(width)
}

/// The placed width of the stroke of `paint` when it is cut around its
/// outline (both edges), else `None`.
pub fn outline_width(paint: Paint, options: &CutOptions, affine: Affine) -> Option<f64> {
    match options.mode {
        CutMode::AllLines => None,
        CutMode::ObjectOutlines => outline_style(paint, options, affine).map(|s| s.width),
    }
}

/// The box of the drawn shape of `path` as it is cut: the stroke outline
/// for an outlined stroke, else the centre line (a fill's edge lies on it).
fn shape_bounds(path: &CutPath, options: &CutOptions, affine: Affine) -> Option<Rect> {
    let placed = affine * &path.path;
    if !flatcraft_core::geometry::has_segments(&placed) {
        return None;
    }
    let centre = placed.bounding_box();
    let style = match options.mode {
        CutMode::AllLines => None,
        CutMode::ObjectOutlines => outline_style(path.paint, options, affine),
    };
    let Some(style) = style.filter(|_| is_finite(&placed)) else {
        return Some(centre);
    };
    let closed = close_meeting_ends(&placed);
    let outline = kurbo::stroke(
        closed.iter(),
        &style,
        &StrokeOpts::default(),
        BOUNDS_TOLERANCE_MM,
    );
    let stroke = outline.bounding_box();
    Some(if path.paint.fill.is_some() {
        stroke.union(centre)
    } else {
        stroke
    })
}

/// The cut paths of one source path, in mat mm. One source path may give
/// several cut paths (the edges of its painted area).
pub fn cut_path(
    path: &CutPath,
    options: &CutOptions,
    placed_affine: Affine,
    tolerance_mm: f64,
    min_width_mm: f64,
) -> Vec<BezPath> {
    cut_path_counted(path, options, placed_affine, tolerance_mm, min_width_mm).0
}

/// [`cut_path`], and how many filled slivers it dropped.
pub fn cut_path_counted(
    path: &CutPath,
    options: &CutOptions,
    placed_affine: Affine,
    tolerance_mm: f64,
    min_width_mm: f64,
) -> (Vec<BezPath>, usize) {
    let placed = placed_affine * &path.path;
    match options.mode {
        CutMode::AllLines => (vec![placed], 0),
        CutMode::ObjectOutlines => {
            let tolerance = if tolerance_mm.is_finite() {
                tolerance_mm.max(MIN_TOLERANCE_MM)
            } else {
                MIN_TOLERANCE_MM
            };
            let min_width = if min_width_mm.is_finite() {
                min_width_mm.max(0.0)
            } else {
                0.0
            };
            object_outlines(
                placed,
                path.paint,
                options,
                placed_affine,
                tolerance,
                min_width,
            )
        }
    }
}

/// Placed stroke width: the source width times `sqrt(|det|)` of the
/// placement.
fn placed_width(width_mm: f64, affine: Affine) -> f64 {
    width_mm * affine.determinant().abs().sqrt()
}

/// The kurbo stroke style if this stroke is cut as an outline.
fn outline_style(paint: Paint, options: &CutOptions, affine: Affine) -> Option<Stroke> {
    let s = paint.stroke?;
    let width = placed_width(s.width_mm, affine);
    let outline = match options.strokes {
        StrokeMode::CentreLine => false,
        StrokeMode::Outline => true,
        StrokeMode::Auto => width >= options.auto_threshold_mm,
    };
    if !outline || !width.is_finite() || width <= 0.0 {
        return None;
    }
    let style = Stroke::new(width).with_caps(match s.cap {
        Cap::Butt => kurbo::Cap::Butt,
        Cap::Round => kurbo::Cap::Round,
        Cap::Square => kurbo::Cap::Square,
    });
    Some(match s.join {
        Join::Miter { limit } => style.with_join(kurbo::Join::Miter).with_miter_limit(limit),
        Join::Round => style.with_join(kurbo::Join::Round),
        Join::Bevel => style.with_join(kurbo::Join::Bevel),
    })
}

fn object_outlines(
    placed: BezPath,
    paint: Paint,
    options: &CutOptions,
    affine: Affine,
    tolerance: f64,
    min_width: f64,
) -> (Vec<BezPath>, usize) {
    if !is_finite(&placed) {
        // Booleans need finite coordinates; the pipeline refuses the rest.
        return (vec![placed], 0);
    }
    let style = outline_style(paint, options, affine);
    let shapes = match (paint.fill, style) {
        // Stroke-only paths with a centre-line stroke and unpainted paths:
        // the path itself.
        (None, None) => return (vec![placed], 0),
        // A fill with no stroke or a centre-line stroke.
        (Some(rule), None) => {
            let stroke_width = paint
                .stroke
                .map(|s| placed_width(s.width_mm, affine))
                .filter(|w| w.is_finite());
            return fill_edges(&placed, fill_rule(rule), stroke_width, tolerance, min_width);
        }
        (fill, Some(style)) => {
            let closed = close_meeting_ends(&placed);
            let outline = kurbo::stroke(
                closed.iter(),
                &style,
                &StrokeOpts::default(),
                tolerance / 4.0,
            );
            let outline = contours(outline.iter(), tolerance / 4.0);
            let outline = boolean(&outline, &[], OverlayFill::NonZero, tolerance);
            let outline: Vec<Contour> = outline.into_iter().flatten().collect();
            let bands = bands(&closed, style.width / 2.0, tolerance / 2.0);
            let stroke = boolean(&outline, &bands, OverlayFill::NonZero, tolerance);
            let stroke: Vec<Contour> = stroke.into_iter().flatten().collect();
            match fill {
                // Fill first, as its own rule, then united with the stroke.
                Some(rule) => {
                    let fill = contours(placed.iter(), tolerance / 2.0);
                    let fill = boolean(&fill, &[], fill_rule(rule), tolerance);
                    let fill: Vec<Contour> = fill.into_iter().flatten().collect();
                    boolean(&fill, &stroke, OverlayFill::NonZero, tolerance)
                }
                None => stroke.into_iter().map(|c| vec![c]).collect(),
            }
        }
    };
    let rings = shapes
        .iter()
        .flatten()
        .filter(|c| c.len() >= 3)
        .map(|c| ring(c))
        .collect();
    (rings, 0)
}

/// The cut paths of a fill with no stroke (`stroke_width` is `None`) or
/// with a centre-line stroke of that placed width: the edge of the
/// fill under its own rule. The thin stroke lies on that edge,
/// except along the implicit closing edge of an open subpath, which
/// must be cut to free the fill.
///
/// A subpath whose own fill is hidden under its stroke is a line (a
/// straight line, a hand-drawn line with jitter, a line with a
/// slight kink, often drawn with the default fill). It shows only
/// through its stroke, so its centre line is cut and it stays out
/// of the fill: its slivers would be cut as tiny loops or as a
/// second cut next to the first. "Hidden" compares the mean width
/// of the fill (`mean_width`) with the stroke width, at least the
/// tolerance. An open subpath has no stroke along its implicit
/// closing chord, so there the bound is half the stroke width; it is
/// cut as drawn. A stroked subpath narrower than `min_width` (half the
/// blade offset) is a line as well, whatever its stroke. A closed line
/// is a thin slot: with a mean width of [`hairline_mm`] or more it is cut
/// as drawn, a ring with both sides. Below that it is a hairline that
/// runs back along itself, and it is cut once, along its `spine`.
/// Without a stroke a fill thinner than the tolerance is not visible and
/// no knife can cut it, so it is dropped; one thinner than `min_width`
/// is a sliver the blade cannot follow, so it is dropped and counted
/// (the second value). A sliver the boolean makes of a stroked path (a
/// thin even-odd frame) is cut once, along its middle ([`sliver_once`]),
/// never along both edges.
///
/// The stroked centre line of a subpath that makes the fill is cut too
/// where it leaves the fill's edge ([`spurs`]): a closed shape with a tail
/// drawn in the same subpath keeps its tail.
///
/// Known gaps: a stroked subpath without area that lies inside a filled
/// area of the same path is cut into the piece. The ends of a closed
/// hairline are looked
/// for over a reach of 0.2 mm, which can pick the apex of a narrow "V"
/// instead of an end; such a ring is then cut as drawn, with both sides.
/// Milestone 2 improves the end detection together with the medial axis.
fn fill_edges(
    placed: &BezPath,
    rule: OverlayFill,
    stroke_width: Option<f64>,
    tolerance: f64,
    min_width: f64,
) -> (Vec<BezPath>, usize) {
    let mut fill: Vec<Contour> = Vec::new();
    let mut lines: Vec<BezPath> = Vec::new();
    // The stroked centre lines of the subpaths that make the fill, for
    // their spurs.
    let mut centre: Vec<Contour> = Vec::new();
    // The fill of the last subpath that is not a line: the result if it
    // is the only one.
    let mut last = Vec::new();
    for sub in subpaths(placed.iter()) {
        let contour = sub.contour(tolerance / 2.0);
        let own = match &contour {
            Some(c) => boolean(std::slice::from_ref(c), &[], rule, tolerance),
            None => Vec::new(),
        };
        // A stroked fill below the knife's narrowest feature is a line
        // too: cut once, never dropped, since its stroke shows.
        let hidden_below = match stroke_width {
            Some(w) if sub.ends_meet() => w.max(min_width),
            Some(w) => (w / 2.0).max(min_width),
            None => 0.0,
        };
        let width = mean_width(&own);
        if width < hidden_below.max(tolerance) {
            if stroke_width.is_some() && !sub.segments.is_empty() {
                let hairline = sub.ends_meet() && width < hairline_mm(tolerance, min_width);
                lines.push(
                    match hairline.then(|| spine(&sub.points(tolerance / 2.0))) {
                        Some(Some(spine)) => spine,
                        _ => sub.elements().collect(),
                    },
                );
            }
        } else if let Some(c) = contour {
            if stroke_width.is_some() {
                let mut line = sub.points(tolerance / 2.0);
                if sub.closed
                    && let Some(first) = line.first().copied()
                {
                    line.push(first);
                }
                centre.push(line);
            }
            fill.push(c);
            last = own;
        }
    }
    let edges = if fill.len() == 1 {
        last
    } else {
        boolean(&fill, &[], rule, tolerance)
    };
    // The spurs first, against every shape of the fill: a centre line on
    // the edge of a band that is dropped or cut once below is that band's
    // edge, never a spur.
    let spur_cuts = spurs(&centre, &edges, tolerance);
    // Thinner than the tolerance: not visible, dropped without a word.
    // Thinner than the knife's narrowest feature: a sliver, whose two
    // edges both lie inside the blade's swing, so they are never both
    // cut. A stroked sliver shows through its stroke: it is cut once,
    // along its middle ([`sliver_once`]). A fill-only one is dropped and
    // counted.
    let mut slivers = 0;
    let mut once: Vec<BezPath> = Vec::new();
    let mut kept: Vec<Vec<Contour>> = Vec::new();
    for shape in edges {
        let w = mean_width(std::slice::from_ref(&shape));
        if w < tolerance {
            continue;
        }
        if w < min_width {
            match stroke_width {
                Some(_) => once.extend(sliver_once(&shape, &centre, min_width)),
                None => slivers += 1,
            }
            continue;
        }
        kept.push(shape);
    }
    let mut out: Vec<BezPath> = kept
        .iter()
        .flatten()
        .filter(|c| c.len() >= 3)
        .map(|c| ring(c))
        .collect();
    out.extend(lines);
    out.extend(once);
    out.extend(spur_cuts);
    (out, slivers)
}

/// The one cut of a stroked sliver `shape` (an outer contour and its
/// holes, from the fill boolean): never both of its edges.
///
/// - A band between two rings (one hole, like a thin frame): the middle
///   ring, each point halfway between a point of the outer ring and the
///   nearest point of the hole ([`mid_ring`]).
/// - A strip with no hole (a crescent): its spine ([`spine`]).
/// - When neither can be built and checked: the longest of the stroked
///   centre lines `centre` that runs along the shape, else the shape's
///   outer contour. One line either way.
fn sliver_once(shape: &[Contour], centre: &[Contour], min_width: f64) -> Option<BezPath> {
    let reach = 4.0 * min_width.max(HAIRLINE_MM);
    let built = match shape {
        [outer, hole] => mid_ring(outer, hole, reach),
        [outer] => spine(outer),
        _ => None,
    };
    if built.is_some() {
        return built;
    }
    let edges: Vec<Contour> = shape
        .iter()
        .filter_map(|c| {
            let mut c = c.clone();
            c.push(*c.first()?);
            Some(c)
        })
        .collect();
    let edge_refs: Vec<&[[f64; 2]]> = edges.iter().map(Vec::as_slice).collect();
    let index = NearIndex::new(&edge_refs, reach);
    let length = |c: &Contour| -> f64 {
        c.windows(2)
            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
            .sum()
    };
    let polyline = |c: &Contour, closed: bool| {
        let mut path = BezPath::new();
        path.move_to(Point::new(c[0][0], c[0][1]));
        for q in &c[1..] {
            path.line_to(Point::new(q[0], q[1]));
        }
        if closed {
            path.close_path();
        }
        path
    };
    let line = centre
        .iter()
        .filter(|c| c.len() >= 2 && c.iter().all(|p| index.near(*p)))
        .max_by(|a, b| length(a).total_cmp(&length(b)));
    match line {
        Some(c) => Some(polyline(c, false)),
        None => shape
            .first()
            .filter(|c| c.len() >= 3)
            .map(|c| polyline(c, true)),
    }
}

/// The ring halfway between the ring `outer` and the ring `hole` inside
/// it: for points along `outer` (its vertices and points at most a
/// quarter `reach` apart), the midpoint to the nearest point of `hole`.
/// `None` unless it is proven to run inside the band: every such pair at
/// most `reach` apart, and the middle about as long as the two rings on
/// average.
fn mid_ring(outer: &[[f64; 2]], hole: &[[f64; 2]], reach: f64) -> Option<BezPath> {
    if outer.len() < 3 || hole.len() < 3 {
        return None;
    }
    let ring_len = |r: &[[f64; 2]]| -> f64 {
        (0..r.len())
            .map(|k| {
                let (a, b) = (r[k], r[(k + 1) % r.len()]);
                (b[0] - a[0]).hypot(b[1] - a[1])
            })
            .sum()
    };
    let nearest = |p: [f64; 2]| -> [f64; 2] {
        let mut best = (f64::INFINITY, hole[0]);
        for k in 0..hole.len() {
            let (a, b) = (hole[k], hole[(k + 1) % hole.len()]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 {
                (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let q = [a[0] + dx * t, a[1] + dy * t];
            let d = (p[0] - q[0]).hypot(p[1] - q[1]);
            if d < best.0 {
                best = (d, q);
            }
        }
        best.1
    };
    let step = (reach / 4.0).max(1e-3);
    let n = outer.len();
    let mut mids: Vec<Point> = Vec::new();
    for k in 0..n {
        let (a, b) = (outer[k], outer[(k + 1) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let parts = (dx.hypot(dy) / step).ceil().max(1.0) as usize;
        for m in 0..parts {
            let t = m as f64 / parts as f64;
            let p = [a[0] + dx * t, a[1] + dy * t];
            let q = nearest(p);
            if (p[0] - q[0]).hypot(p[1] - q[1]) > reach {
                return None;
            }
            let mid = Point::new((p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0);
            // A point on the straight run of the two before it says
            // nothing new.
            if mids.len() >= 2 {
                let (u, v) = (mids[mids.len() - 2], mids[mids.len() - 1]);
                if (v - u).cross(mid - v).abs() < 1e-12 * (1.0 + (v - u).hypot2()) {
                    mids.pop();
                }
            }
            if mids.last() != Some(&mid) {
                mids.push(mid);
            }
        }
    }
    if mids.len() < 3 {
        return None;
    }
    let mid_len: f64 = mids.windows(2).map(|w| w[0].distance(w[1])).sum::<f64>()
        + mids[mids.len() - 1].distance(mids[0]);
    let average = (ring_len(outer) + ring_len(hole)) / 2.0;
    if (mid_len - average).abs() > (0.05 * average).max(4.0 * reach) {
        return None;
    }
    let mut path = BezPath::new();
    path.move_to(mids[0]);
    for p in &mids[1..] {
        path.line_to(*p);
    }
    path.close_path();
    Some(path)
}

/// The parts of the stroked centre lines `centre` that do not lie on the
/// edge of the fill `fill` (spurs and tails drawn in a subpath that also
/// encloses area), cut as open pieces. The parts on the edge are cut by
/// the fill's ring already, so they are left out.
///
/// Outside the fill every part is cut: there the stroke is all that shows.
/// Inside it only a tail is cut, a part with an end inside the fill. A
/// loop or a chord from edge to edge inside the fill is the edge of a
/// subpath hidden under the fill (a square nested in a square of the same
/// direction is one filled square); cutting it would cut the piece apart.
///
/// Rounding of the booleans can leave a part a hair off the edge; one
/// whose every vertex and segment middle lies within `tolerance` of the
/// edge is the edge too.
fn spurs(centre: &[Contour], fill: &[Vec<Contour>], tolerance: f64) -> Vec<BezPath> {
    use i_overlay::float::clip::FloatClip;
    use i_overlay::string::clip::ClipRule;
    if centre.is_empty() || fill.is_empty() {
        return Vec::new();
    }
    let centre = centre.to_vec();
    let shapes = fill.to_vec();
    let edges: Vec<Contour> = fill
        .iter()
        .flatten()
        .filter_map(|c| {
            let mut c = c.clone();
            c.push(*c.first()?);
            Some(c)
        })
        .collect();
    let edges: Vec<&[[f64; 2]]> = edges.iter().map(Vec::as_slice).collect();
    // Built once: every part asks it.
    let index = NearIndex::new(&edges, tolerance);
    // A segment whose ends and middle all lie on the edge is the edge.
    // Looked at segment by segment: i_overlay hands back a tail and the
    // stretch of edge it runs into as one string.
    let on_edge = |a: [f64; 2], b: [f64; 2]| {
        index.near(a) && index.near(b) && index.near([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0])
    };
    let clip = |invert: bool| {
        // The edge is never part of the result. With `invert`, i_overlay
        // turns `boundary_included` around (true leaves the edge out of
        // the outside part; pinned by a test).
        let rule = ClipRule {
            invert,
            boundary_included: invert,
        };
        let segments: Vec<Contour> = centre
            .clip_by_as::<i64>(&shapes, OverlayFill::NonZero, rule)
            .iter()
            .flat_map(|p| p.windows(2))
            .filter(|w| !on_edge(w[0], w[1]))
            .map(|w| w.to_vec())
            .collect();
        chain_once(&segments)
    };
    let free_end = |p: &Contour| {
        let (first, last) = (p[0], p[p.len() - 1]);
        first != last && (!index.near(first) || !index.near(last))
    };
    let inside = clip(false).into_iter().filter(free_end);
    clip(true)
        .into_iter()
        .chain(inside)
        .filter(|p| {
            let length: f64 = p
                .windows(2)
                .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
                .sum();
            length >= tolerance
        })
        .map(|p| {
            let mut path = BezPath::new();
            path.move_to(Point::new(p[0][0], p[0][1]));
            for q in &p[1..] {
                path.line_to(Point::new(q[0], q[1]));
            }
            path
        })
        .collect()
}

/// The segments of `parts`, each cut once (a line drawn out and back
/// along itself gives its segments twice), joined again into polylines:
/// a polyline runs on through every point where exactly two segments
/// meet. Points are matched to 1e-6 mm, far below the booleans' rounding
/// that the clip kept apart anyway.
fn chain_once(parts: &[Contour]) -> Vec<Contour> {
    use std::collections::{BTreeMap, BTreeSet};
    let key = |p: [f64; 2]| ((p[0] * 1e6).round() as i64, (p[1] * 1e6).round() as i64);
    let mut points: BTreeMap<(i64, i64), [f64; 2]> = BTreeMap::new();
    let mut segments: BTreeSet<((i64, i64), (i64, i64))> = BTreeSet::new();
    for part in parts {
        for w in part.windows(2) {
            let (a, b) = (key(w[0]), key(w[1]));
            if a == b {
                continue;
            }
            points.entry(a).or_insert(w[0]);
            points.entry(b).or_insert(w[1]);
            segments.insert((a.min(b), a.max(b)));
        }
    }
    let mut ends: BTreeMap<(i64, i64), Vec<(i64, i64)>> = BTreeMap::new();
    for &(a, b) in &segments {
        ends.entry(a).or_default().push(b);
        ends.entry(b).or_default().push(a);
    }
    let mut used: BTreeSet<((i64, i64), (i64, i64))> = BTreeSet::new();
    let mut out = Vec::new();
    // Start at the points where a polyline must end, then at what is left
    // (closed loops).
    let starts: Vec<(i64, i64)> = ends
        .iter()
        .filter(|(_, n)| n.len() != 2)
        .map(|(k, _)| *k)
        .chain(ends.keys().copied())
        .collect();
    for start in starts {
        while let Some(&next) = ends[&start]
            .iter()
            .find(|n| !used.contains(&(start.min(**n), start.max(**n))))
        {
            let mut line = vec![points[&start]];
            let (mut at, mut to) = (start, next);
            loop {
                used.insert((at.min(to), at.max(to)));
                line.push(points[&to]);
                let n = &ends[&to];
                if n.len() != 2 {
                    break;
                }
                match n
                    .iter()
                    .find(|m| !used.contains(&(to.min(**m), to.max(**m))))
                {
                    Some(&m) => {
                        at = to;
                        to = m;
                    }
                    None => break,
                }
            }
            out.push(line);
        }
    }
    out
}

/// A closed line whose fill is on average narrower than this is a
/// hairline and cut once, along its spine, mm: the knife's narrowest
/// feature (`min_width`, half the blade offset, see
/// [`crate::report::min_width_mm`]), at least the tolerance. Ruling M2-P3:
/// the hairline limit follows the minimum feature size.
pub fn hairline_mm(tolerance: f64, min_width: f64) -> f64 {
    min_width.max(tolerance)
}

/// One rectangle per flattened segment of `path`, `half` to each side, all
/// wound the same way. kurbo's outline follows parallel curves and has no
/// joins between smooth segments, so where the stroke is wider than a
/// curve's radius (a small dot) it leaves a hole that the paint covers.
/// The bands fill exactly that: everything within `half` of a segment is
/// paint. Joins and caps still come from kurbo.
fn bands(path: &BezPath, half: f64, tolerance: f64) -> Vec<Contour> {
    let mut out = Vec::new();
    let mut band = |a: [f64; 2], b: [f64; 2]| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dy);
        if len <= 0.0 {
            return;
        }
        let (nx, ny) = (-dy * half / len, dx * half / len);
        // Counter-clockwise in the shoelace sense for every segment.
        out.push(vec![
            [a[0] - nx, a[1] - ny],
            [b[0] - nx, b[1] - ny],
            [b[0] + nx, b[1] + ny],
            [a[0] + nx, a[1] + ny],
        ]);
    };
    for sub in subpaths(path.iter()) {
        let points = sub.points(tolerance);
        for pair in points.windows(2) {
            band(pair[0], pair[1]);
        }
        if sub.closed
            && let (Some(first), Some(last)) = (points.first(), points.last())
        {
            band(*last, *first);
        }
    }
    out
}

/// One subpath: its start, its drawing segments and whether it is closed.
struct Subpath {
    start: Point,
    segments: Vec<PathEl>,
    closed: bool,
}

impl Subpath {
    fn new(start: Point) -> Self {
        Self {
            start,
            segments: Vec::new(),
            closed: false,
        }
    }

    fn elements(&self) -> impl Iterator<Item = PathEl> + '_ {
        std::iter::once(PathEl::MoveTo(self.start))
            .chain(self.segments.iter().copied())
            .chain(self.closed.then_some(PathEl::ClosePath))
    }

    /// Closed, or drawn back to its start within `CLOSE_TOLERANCE_MM`.
    fn ends_meet(&self) -> bool {
        self.closed
            || self
                .segments
                .last()
                .and_then(PathEl::end_point)
                .is_some_and(|end| end.distance(self.start) < CLOSE_TOLERANCE_MM)
    }

    /// The flattened points, from the start to the last drawn point.
    fn points(&self, tolerance: f64) -> Contour {
        let mut c: Contour = Vec::new();
        kurbo::flatten(self.elements(), tolerance, |el| match el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => c.push([p.x, p.y]),
            // `flatten` only emits moves, lines and closes.
            _ => {}
        });
        c
    }

    /// The flattened contour, closed implicitly as a fill does; `None`
    /// with fewer than three points.
    fn contour(&self, tolerance: f64) -> Option<Contour> {
        Some(self.points(tolerance)).filter(|c| c.len() >= 3)
    }
}

/// The subpaths of `path`. The one place that knows where a subpath ends:
/// a move starts a new one, a close ends one, and drawing on after a close
/// starts again at the closed subpath's start, as in SVG. A move with
/// nothing drawn after it is no subpath.
fn subpaths(path: impl IntoIterator<Item = PathEl>) -> Vec<Subpath> {
    let mut out = Vec::new();
    let mut cur: Option<Subpath> = None;
    let mut start = Point::ORIGIN;
    for el in path {
        match el {
            PathEl::MoveTo(p) => {
                out.extend(cur.take().filter(|c| !c.segments.is_empty()));
                start = p;
                cur = Some(Subpath::new(p));
            }
            PathEl::ClosePath => {
                if let Some(mut c) = cur.take() {
                    c.closed = true;
                    out.push(c);
                }
            }
            _ => cur
                .get_or_insert_with(|| Subpath::new(start))
                .segments
                .push(el),
        }
    }
    out.extend(cur.filter(|c| !c.segments.is_empty()));
    out
}

/// The single cut of a closed hairline with the flattened vertices `ring`.
///
/// A hairline runs out and back along itself, so it has two ends where it
/// turns around (`hairline_ends`). They split the ring into two sides. The
/// cut is the spine through the middle: the midpoints of the two sides at
/// the same share of each side's length. Every choice is made from the
/// coordinates and from each vertex's own surroundings, and the midpoints
/// treat both sides alike, so the cut does not depend on where the ring
/// starts. A blunt end (a thin rectangle) starts at one of its corners,
/// half a width off the middle.
///
/// The spine is used only if it is proven to lie on the hairline: the two
/// sides are never more than two hairline widths apart at the same share,
/// and the spine is about half the ring long. Otherwise the two sides do
/// not run alike, and the longer side is cut instead: it is part of the
/// ring, so it is never more than the hairline's width off and never a
/// second cut.
///
/// `None` if the ring has no two clear ends (a loop without tips), or if
/// neither the spine nor one side covers the whole ring; it is then cut
/// as drawn, which leaves nothing uncut.
fn spine(ring: &[[f64; 2]]) -> Option<BezPath> {
    let mut ring = ring.to_vec();
    ring.dedup();
    if ring.len() > 1 && ring.first() == ring.last() {
        ring.pop();
    }
    let (i, j) = hairline_ends(&ring)?;
    let (i, j) = (i.min(j), i.max(j));
    // Both sides, from the lower end to the upper end.
    let lo = if point_order(&ring[i], &ring[j]).is_le() {
        ring[i]
    } else {
        ring[j]
    };
    let mut sides = [
        ring[i..=j].to_vec(),
        ring[j..].iter().chain(&ring[..=i]).copied().collect(),
    ];
    for side in &mut sides {
        if side[0] != lo {
            side.reverse();
        }
    }
    let [a, b] = sides.map(Side::new);
    // Every vertex of either side gives one spine point.
    let mut shares: Vec<f64> = a.shares.iter().chain(&b.shares).copied().collect();
    shares.sort_by(f64::total_cmp);
    shares.dedup_by(|x, y| (*x - *y).abs() < 1e-12);
    let mut together = true;
    let mut points: Vec<Point> = shares
        .iter()
        .map(|&u| {
            let (p, q) = (a.at(u), b.at(u));
            // The midpoint is at most half this far from the ring.
            together &= p.distance(q) <= 2.0 * HAIRLINE_MM;
            p.midpoint(q)
        })
        .collect();
    points.dedup();
    let length: f64 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    let half_ring = (a.length + b.length) / 2.0;
    let slack = (4.0 * HAIRLINE_MM).max(0.02 * half_ring);
    if !together || (length - half_ring).abs() > slack {
        // The longer side; equal sides go by their coordinates.
        let pick_a = a
            .length
            .total_cmp(&b.length)
            .then_with(|| {
                a.points
                    .iter()
                    .zip(&b.points)
                    .map(|(p, q)| point_order(p, q))
                    .find(|o| o.is_ne())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .is_ge();
        let side = if pick_a { a } else { b };
        // One side stands for the whole ring only if the rest of the ring
        // runs along it. If not (the ends were picked at the apex of a
        // narrow "V", so each side is one arm), cutting it would leave
        // the other part uncut: the ring is cut as drawn.
        if !all_near(ring_samples(&ring), &side.points, 2.0 * HAIRLINE_MM) {
            return None;
        }
        points = side.points.iter().map(|p| Point::new(p[0], p[1])).collect();
    }
    let mut path = BezPath::new();
    path.move_to(*points.first()?);
    for p in &points[1..] {
        path.line_to(*p);
    }
    Some(path)
}

/// The vertices of the closed `ring` and points along each of its edges,
/// the closing edge too, at most `HAIRLINE_MM` apart. A long edge between
/// two vertices must be covered as well as the vertices. The points are
/// made one at a time as they are used, so a long ring needs no buffer.
fn ring_samples(ring: &[[f64; 2]]) -> impl Iterator<Item = [f64; 2]> + '_ {
    let n = ring.len();
    (0..n).flat_map(move |k| {
        let (a, b) = (ring[k], ring[(k + 1) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let parts = (dx.hypot(dy) / HAIRLINE_MM).ceil().max(1.0) as usize;
        (0..parts).map(move |m| {
            let t = m as f64 / parts as f64;
            [a[0] + dx * t, a[1] + dy * t]
        })
    })
}

/// Whether every one of `points` lies within `reach` of the polyline
/// `line`. The line's segments are sorted into a grid of cells two
/// `reach` wide, so each point is compared only with the segments near it.
fn all_near(points: impl IntoIterator<Item = [f64; 2]>, line: &[[f64; 2]], reach: f64) -> bool {
    all_near_lines(points, &[line], reach)
}

/// [`all_near`] for several polylines: every point lies within `reach` of
/// one of `lines`.
fn all_near_lines(
    points: impl IntoIterator<Item = [f64; 2]>,
    lines: &[&[[f64; 2]]],
    reach: f64,
) -> bool {
    let index = NearIndex::new(lines, reach);
    points.into_iter().all(|p| index.near(p))
}

/// The segments of some polylines sorted into a grid of cells two `reach`
/// wide, built once, for asking of many points whether they lie within
/// `reach` of a line.
struct NearIndex<'a> {
    lines: &'a [&'a [[f64; 2]]],
    reach: f64,
    cell: f64,
    grid: std::collections::HashMap<(i64, i64), Vec<(usize, usize)>>,
}

impl<'a> NearIndex<'a> {
    fn new(lines: &'a [&'a [[f64; 2]]], reach: f64) -> Self {
        let cell = 2.0 * reach;
        let mut index = Self {
            lines,
            reach,
            cell,
            grid: std::collections::HashMap::new(),
        };
        for (l, line) in lines.iter().enumerate() {
            for (k, w) in line.windows(2).enumerate() {
                let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
                // A mark every half cell: the nearest mark to any point of
                // the segment is a quarter cell away, so a point within
                // `reach` of the segment finds it in its own cell or a
                // neighbour.
                let marks = (dx.hypot(dy) / (cell / 2.0)).ceil().max(1.0) as usize;
                for m in 0..=marks {
                    let t = m as f64 / marks as f64;
                    let key = index.key(w[0][0] + dx * t, w[0][1] + dy * t);
                    let cells = index.grid.entry(key).or_default();
                    if cells.last() != Some(&(l, k)) {
                        cells.push((l, k));
                    }
                }
            }
        }
        index
    }

    fn key(&self, x: f64, y: f64) -> (i64, i64) {
        (
            (x / self.cell).floor() as i64,
            (y / self.cell).floor() as i64,
        )
    }

    /// True if `p` lies within `reach` of one of the lines.
    fn near(&self, p: [f64; 2]) -> bool {
        let distance = |(l, k): (usize, usize)| {
            let (a, b) = (self.lines[l][k], self.lines[l][k + 1]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 {
                (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (p[0] - a[0] - dx * t).hypot(p[1] - a[1] - dy * t)
        };
        let (cx, cy) = self.key(p[0], p[1]);
        (-1..=1).any(|ox| {
            (-1..=1).any(|oy| {
                self.grid
                    .get(&(cx + ox, cy + oy))
                    .is_some_and(|near| near.iter().any(|&k| distance(k) <= self.reach))
            })
        })
    }
}

/// Indices of the two ends of a closed hairline: the two vertices where
/// the ring turns around most sharply. The farthest two vertices would be
/// wrong for a hairline bent by more than half a turn (a "C").
///
/// The turn at a vertex is measured between the two points a short way
/// (two hairline widths, at most a quarter of the ring) along the ring to
/// either side, so a blunt or rounded end counts like a pointed one and
/// jitter does not. The first end is the sharpest turn; the second is the
/// sharpest one at least a quarter of the ring away from it. Equal turns
/// go by coordinates. Each turn is measured by walking from its own
/// vertex, so the answer does not depend on where the ring starts.
///
/// `None` if either end turns by less than 120 degrees: the ring has no
/// two clear ends.
fn hairline_ends(ring: &[[f64; 2]]) -> Option<(usize, usize)> {
    /// Cosine of the angle between the two arms at an end that turns by
    /// 120 degrees.
    const CLEAR_END: f64 = 0.5;
    let n = ring.len();
    if n < 2 {
        return None;
    }
    let step = |k: usize| {
        let (p, q) = (ring[k], ring[(k + 1) % n]);
        (q[0] - p[0]).hypot(q[1] - p[1])
    };
    // Length along the ring from vertex 0 to every vertex.
    let mut along = Vec::with_capacity(n);
    let mut total = 0.0;
    for k in 0..n {
        along.push(total);
        total += step(k);
    }
    let reach = (2.0 * HAIRLINE_MM).min(total / 4.0);
    if reach <= 0.0 {
        return None;
    }
    // The point `reach` along the ring from vertex `k`, forwards or back,
    // relative to the vertex. The walk stops after `MAX_ARM_STEPS`
    // vertices: in a tiny ring (reach a quarter of the ring) or a long run
    // of repeated vertices, every vertex would otherwise walk most of the
    // ring, O(n^2) in all. A stopped arm ends at the vertex it reached; on
    // a run of copies that is no direction, so no end there.
    const MAX_ARM_STEPS: usize = 64;
    let arm = |k: usize, forwards: bool| {
        let mut left = reach;
        let mut at = k;
        for _ in 0..MAX_ARM_STEPS {
            let next = if forwards {
                (at + 1) % n
            } else {
                (at + n - 1) % n
            };
            let len = step(if forwards { at } else { next });
            if len >= left && len > 0.0 {
                let t = left / len;
                let (p, q) = (ring[at], ring[next]);
                return [
                    p[0] + (q[0] - p[0]) * t - ring[k][0],
                    p[1] + (q[1] - p[1]) * t - ring[k][1],
                ];
            }
            left -= len;
            at = next;
        }
        [ring[at][0] - ring[k][0], ring[at][1] - ring[k][1]]
    };
    // How sharply the ring turns around at every vertex: the cosine of
    // the angle between its two arms, 1 where it folds back onto itself.
    let sharp: Vec<f64> = (0..n)
        .map(|k| {
            let (f, b) = (arm(k, true), arm(k, false));
            let norm = f[0].hypot(f[1]) * b[0].hypot(b[1]);
            if norm > 0.0 {
                (f[0] * b[0] + f[1] * b[1]) / norm
            } else {
                // Both arms end on the vertex: no direction, no end.
                -1.0
            }
        })
        .collect();
    let sharpest = |allowed: &dyn Fn(usize) -> bool| {
        (0..n).filter(|&k| allowed(k)).max_by(|&a, &b| {
            sharp[a]
                .total_cmp(&sharp[b])
                .then(point_order(&ring[a], &ring[b]))
        })
    };
    let first = sharpest(&|_| true)?;
    let second = sharpest(&|k| {
        let d = (along[k] - along[first]).abs();
        d.min(total - d) >= total / 4.0
    })?;
    (sharp[first] >= CLEAR_END && sharp[second] >= CLEAR_END).then_some((first, second))
}

/// One side of a hairline with, for every vertex, its share of the side's
/// length (0 at the first vertex, 1 at the last).
struct Side {
    points: Contour,
    shares: Vec<f64>,
    length: f64,
}

impl Side {
    fn new(points: Contour) -> Self {
        let mut shares = Vec::with_capacity(points.len());
        let mut length = 0.0;
        shares.push(0.0);
        for w in points.windows(2) {
            length += (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            shares.push(length);
        }
        for s in &mut shares {
            // A side without length is one point; every share is its start.
            *s = if length > 0.0 { *s / length } else { 0.0 };
        }
        if let Some(last) = shares.last_mut().filter(|_| length > 0.0) {
            *last = 1.0;
        }
        Self {
            points,
            shares,
            length,
        }
    }

    /// The point at share `u` of the side's length.
    fn at(&self, u: f64) -> Point {
        let point = |k: usize| Point::new(self.points[k][0], self.points[k][1]);
        // The first vertex with a share of `u` or more.
        let k = self.shares.partition_point(|s| *s < u);
        if k == 0 {
            return point(0);
        }
        if k >= self.points.len() {
            return point(self.points.len() - 1);
        }
        let (s0, s1) = (self.shares[k - 1], self.shares[k]);
        let t = if s1 > s0 { (u - s0) / (s1 - s0) } else { 1.0 };
        point(k - 1).lerp(point(k), t)
    }
}

fn point_order(a: &[f64; 2], b: &[f64; 2]) -> std::cmp::Ordering {
    a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1]))
}

/// Mean width of boolean result shapes: twice their area over the length
/// of all their edges (holes included). A strip of length `l` and width
/// `w` has area `l w` and about `2 l` of edge, so this is `w`; a triangle
/// of height `h` over a long base gives `h / 2`. Zero without shapes.
fn mean_width(shapes: &[Vec<Contour>]) -> f64 {
    let mut area = 0.0;
    let mut edge = 0.0;
    for shape in shapes {
        let mut signed = 0.0;
        for c in shape {
            let n = c.len();
            for i in 0..n {
                let (a, b) = (c[i], c[(i + 1) % n]);
                signed += a[0] * b[1] - b[0] * a[1];
                edge += (b[0] - a[0]).hypot(b[1] - a[1]);
            }
        }
        // Holes run against their outer edge, so they subtract.
        area += (signed / 2.0).abs();
    }
    if edge > 0.0 { 2.0 * area / edge } else { 0.0 }
}

fn fill_rule(rule: FillRule) -> OverlayFill {
    match rule {
        FillRule::NonZero => OverlayFill::NonZero,
        FillRule::EvenOdd => OverlayFill::EvenOdd,
    }
}

/// `subj` (and `clip`, if any) united, each under `rule`. The result's
/// contours are oriented (outer edges one way, holes the other), so a
/// second boolean may read them with the non-zero rule.
fn boolean(
    subj: &[Contour],
    clip: &[Contour],
    rule: OverlayFill,
    tolerance: f64,
) -> Vec<Vec<Contour>> {
    if subj.is_empty() && clip.is_empty() {
        return Vec::new();
    }
    let mut options = OverlayOptions::<f64>::default();
    // Drops slivers that no knife can cut.
    options.min_output_area = tolerance * tolerance;
    let overlay_rule = if clip.is_empty() {
        OverlayRule::Subject
    } else {
        OverlayRule::Union
    };
    let subj = subj.to_vec();
    let clip = clip.to_vec();
    FloatOverlay::with_subj_and_clip_custom(&subj, &clip, options, Solver::default())
        .overlay(overlay_rule, rule)
}

fn is_finite(path: &BezPath) -> bool {
    path.elements().iter().all(|el| match el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => p.is_finite(),
        PathEl::QuadTo(a, b) => a.is_finite() && b.is_finite(),
        PathEl::CurveTo(a, b, c) => a.is_finite() && b.is_finite() && c.is_finite(),
        PathEl::ClosePath => true,
    })
}

/// `path` with every subpath whose ends meet within `CLOSE_TOLERANCE_MM`
/// closed, its last point moved onto its start, so the stroker joins the
/// ends instead of capping them.
fn close_meeting_ends(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    for mut sub in subpaths(path.iter()) {
        let start = sub.start;
        if !sub.closed
            && let Some(last) = sub.segments.last_mut()
            && let Some(end) = last.end_point()
            && end.distance(start) < CLOSE_TOLERANCE_MM
        {
            *last = match *last {
                PathEl::LineTo(_) => PathEl::LineTo(start),
                PathEl::QuadTo(a, _) => PathEl::QuadTo(a, start),
                PathEl::CurveTo(a, b, _) => PathEl::CurveTo(a, b, start),
                other => other,
            };
            sub.closed = true;
        }
        out.extend(sub.elements());
    }
    out
}

/// Flattened closed contours of a path; open subpaths are closed
/// implicitly, as a fill does.
fn contours(path: impl IntoIterator<Item = PathEl>, tolerance: f64) -> Vec<Contour> {
    subpaths(path)
        .iter()
        .filter_map(|sub| sub.contour(tolerance))
        .collect()
}

fn ring(c: &[[f64; 2]]) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(Point::new(c[0][0], c[0][1]));
    for q in &c[1..] {
        p.line_to(Point::new(q[0], q[1]));
    }
    p.close_path();
    p
}

#[cfg(test)]
pub(crate) mod tests {
    thread_local! {
        /// How often `cut_bounds` ran on this thread.
        pub(crate) static CUT_BOUNDS_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        /// How often an object was cut (not taken over) on this thread.
        pub(crate) static OBJECT_CUTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    use super::*;
    use kurbo::ParamCurveArclen as _;

    #[test]
    fn a_path_without_segments_has_no_cut_bounds() {
        for stray in ["M50 50", "M50 50 Z"] {
            let path = CutPath {
                path: BezPath::from_svg(stray).unwrap(),
                color: None,
                label: None,
                paint: Default::default(),
            };
            for mode in [CutMode::AllLines, CutMode::ObjectOutlines] {
                let options = CutOptions {
                    mode,
                    ..CutOptions::default()
                };
                assert_eq!(
                    shape_bounds(&path, &options, Affine::IDENTITY),
                    None,
                    "{stray}"
                );
            }
        }
    }

    /// A tiny ring (its reach is a quarter of the ring) with a run of
    /// 60000 copies of one vertex: every vertex in the run used to walk
    /// the whole run, about 7e9 steps. There is no step counter, so the
    /// limit is generous: it only catches that blow-up, not timing noise.
    #[test]
    fn hairline_ends_of_a_tiny_ring_with_many_duplicates_are_quick() {
        let mut ring = vec![[0.0, 0.0]];
        ring.extend(std::iter::repeat_n([0.3, 0.0], 60_000));
        ring.extend([[0.6, 0.0], [0.6, 0.02], [0.0, 0.02]]);
        let start = std::time::Instant::now();
        let _ = hairline_ends(&ring);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn meeting_ends_are_closed() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((10.0, 10.0));
        p.line_to((0.005, 0.0));
        let c = close_meeting_ends(&p);
        assert_eq!(c.elements().last(), Some(&PathEl::ClosePath));
        assert_eq!(c.elements()[3], PathEl::LineTo(Point::new(0.0, 0.0)));
        let mut open = BezPath::new();
        open.move_to((0.0, 0.0));
        open.line_to((10.0, 0.0));
        assert_eq!(close_meeting_ends(&open), open);
    }

    #[test]
    fn drawing_after_a_close_starts_at_the_closed_start() {
        let mut p = BezPath::new();
        p.move_to((1.0, 2.0));
        p.line_to((10.0, 2.0));
        p.close_path();
        p.line_to((1.0, 9.0));
        p.move_to((5.0, 5.0));
        let subs = subpaths(p.iter());
        assert_eq!(subs.len(), 2, "a move alone is no subpath");
        assert!(subs[0].closed && !subs[1].closed);
        assert_eq!(subs[1].start, Point::new(1.0, 2.0));
        assert_eq!(subs[1].segments, vec![PathEl::LineTo(Point::new(1.0, 9.0))]);
    }

    #[test]
    fn spine_of_a_there_and_back_line_is_the_line() {
        let s = spine(&[[0.0, 0.0], [50.0, 0.0], [0.0, 0.0]]).unwrap();
        let line = [
            PathEl::MoveTo(Point::new(0.0, 0.0)),
            PathEl::LineTo(Point::new(50.0, 0.0)),
        ];
        assert_eq!(s.elements(), line.as_slice());
        assert!(spine(&[[1.0, 1.0], [1.0, 1.0]]).is_none());
    }

    #[test]
    fn spine_of_a_thin_rectangle_runs_through_its_middle() {
        let s = spine(&[[0.0, 0.0], [50.0, 0.0], [50.0, 0.08], [0.0, 0.08]]).unwrap();
        let pts: Vec<Point> = s.elements().iter().filter_map(|e| e.end_point()).collect();
        assert_eq!(pts.len(), 4, "{pts:?}");
        // From a corner to a corner, with the long run on the middle.
        assert!(pts[1].distance(Point::new(0.04, 0.04)) < 1e-3, "{pts:?}");
        assert!(pts[2].distance(Point::new(49.96, 0.04)) < 1e-3, "{pts:?}");
    }

    #[test]
    fn all_near_finds_far_points_along_long_segments() {
        let line = [[0.0, 0.0], [30.0, 40.0], [30.0, 40.1]];
        // On the long diagonal, off it by 0.15 and by 0.25.
        assert!(all_near([[15.0, 20.0], [15.12, 19.91]], &line, 0.2));
        assert!(!all_near([[15.0, 20.0], [15.2, 19.85]], &line, 0.2));
        assert!(!all_near([[30.0, 40.4]], &line, 0.2));
        assert!(all_near(std::iter::empty(), &line, 0.2));
    }

    #[test]
    fn a_side_that_misses_a_long_ring_edge_is_not_cut_alone() {
        // Two ends, one side over an apex 5 mm up, the other side one
        // straight 50 mm edge. The longer side passes through all three
        // vertices, but the straight edge is up to 5 mm away from it.
        let ring = [[0.0, 0.0], [25.0, 5.0], [50.0, 0.0]];
        assert_eq!(hairline_ends(&ring), Some((0, 2)));
        assert!(spine(&ring).is_none(), "{:?}", spine(&ring));
    }

    /// The samples are streamed, never collected: a 1000 km edge costs
    /// nothing until its samples are asked for.
    #[test]
    fn ring_samples_are_streamed() {
        let ring = [[0.0, 0.0], [1e9, 0.0], [1e9, 0.05]];
        let first: Vec<[f64; 2]> = ring_samples(&ring).take(3).collect();
        assert_eq!(first[0], [0.0, 0.0]);
        assert!((first[1][0] - HAIRLINE_MM).abs() < 1e-6, "{first:?}");
    }

    #[test]
    fn ring_samples_are_at_most_a_hairline_apart() {
        let ring = [[0.0, 0.0], [50.0, 0.0], [50.0, 0.05], [10.0, 0.25]];
        let samples: Vec<[f64; 2]> = ring_samples(&ring).collect();
        assert!(ring.iter().all(|v| samples.contains(v)));
        let n = samples.len();
        for k in 0..n {
            let (a, b) = (samples[k], samples[(k + 1) % n]);
            let d = (b[0] - a[0]).hypot(b[1] - a[1]);
            assert!(d <= HAIRLINE_MM + 1e-12, "{a:?} {b:?}");
        }
        // About one sample per 0.1 mm of ring, not more.
        assert!(n < 1100, "{n}");
    }

    #[test]
    fn a_loop_without_tips_has_no_ends() {
        let circle: Vec<[f64; 2]> = (0..48)
            .map(|k| {
                let a = f64::from(k) * std::f64::consts::TAU / 48.0;
                [0.05 * a.cos(), 0.05 * a.sin()]
            })
            .collect();
        assert_eq!(hairline_ends(&circle), None);
        assert!(spine(&circle).is_none());
    }

    #[test]
    fn sides_that_do_not_run_alike_give_one_side() {
        // Out along a straight line, back with a small zigzag near one
        // end. The zigzag makes that side longer, so points at the same
        // share of the two sides drift more than 1 mm apart and the
        // middle is not trusted. The zigzag side still runs within
        // 0.2 mm of the whole ring, so it is cut alone.
        let mut ring = vec![[0.0, 0.0], [50.0, 0.0]];
        for k in 0..=20 {
            let y = if k % 2 == 0 { 0.02 } else { 0.17 };
            ring.push([10.0 - 0.1 * f64::from(k), y]);
        }
        let s = spine(&ring).unwrap();
        let pts: Vec<Point> = s.elements().iter().filter_map(|e| e.end_point()).collect();
        let mut side: Vec<Point> = ring[1..].iter().map(|p| Point::new(p[0], p[1])).collect();
        side.push(Point::new(0.0, 0.0));
        side.reverse();
        assert_eq!(pts, side);
    }

    #[test]
    fn a_side_with_a_detour_does_not_stand_for_the_ring() {
        // Back with a 3 mm detour: the longer side leaves the straight
        // side uncut under the detour, so neither is cut alone.
        let ring = [
            [0.0, 0.0],
            [50.0, 0.0],
            [30.0, 0.02],
            [25.0, 3.0],
            [20.0, 0.02],
        ];
        assert!(spine(&ring).is_none(), "{:?}", spine(&ring));
    }

    /// Pins the clip rules: a line on the fill's edge gives nothing, a
    /// line across it gives its part inside and its parts outside.
    #[test]
    fn spurs_keep_both_sides_and_leave_the_edge() {
        let square = vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]];
        let on_edge = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        assert!(spurs(&[on_edge], std::slice::from_ref(&square), 0.01).is_empty());
        // Along the edge, then away from it: only the part away is cut.
        let edge_then_tail = vec![[0.0, 10.0], [0.0, 0.0], [-5.0, -5.0]];
        let parts = spurs(&[edge_then_tail], std::slice::from_ref(&square), 0.01);
        assert_eq!(parts.len(), 1, "{parts:?}");
        let length: f64 = parts[0].segments().map(|s| s.arclen(1e-9)).sum();
        assert!((length - 50f64.sqrt()).abs() < 1e-6, "{parts:?}");
        // Across: the two parts outside are cut; the chord inside runs
        // from edge to edge, so it is hidden under the fill.
        let across = vec![[-5.0, 5.0], [15.0, 5.0]];
        let parts = spurs(&[across], std::slice::from_ref(&square), 0.01);
        let lengths: Vec<f64> = parts
            .iter()
            .map(|p| p.segments().map(|s| s.arclen(1e-9)).sum::<f64>())
            .collect();
        assert_eq!(lengths.len(), 2, "{parts:?}");
        assert!(
            lengths.iter().all(|l| (l - 5.0).abs() < 1e-6),
            "{lengths:?}"
        );
        // A tail into the fill has a free end inside: it is cut.
        let tail = vec![[0.0, 0.0], [4.0, 4.0]];
        let parts = spurs(&[tail], std::slice::from_ref(&square), 0.01);
        assert_eq!(parts.len(), 1, "{parts:?}");
        // A loop inside is a hidden edge.
        let inner = vec![[2.0, 2.0], [8.0, 2.0], [8.0, 8.0], [2.0, 8.0], [2.0, 2.0]];
        assert!(spurs(&[inner], &[square], 0.01).is_empty());
    }

    #[test]
    fn chain_once_cuts_a_there_and_back_line_once() {
        let out = chain_once(&[
            vec![[0.0, 0.0], [5.0, 5.0], [0.0, 0.0]],
            vec![[5.0, 5.0], [9.0, 5.0]],
        ]);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].len(), 3, "{out:?}");
    }

    /// The box memo stays within its size, and never keeps geometry alive.
    #[test]
    fn the_box_memo_is_bounded_and_holds_no_geometry() {
        let mut objects = Vec::new();
        for k in 0..(BOUNDS_MEMO_SIZE + 60) {
            let x = k as f64;
            let path = kurbo::Rect::new(x, 0.0, x + 5.0, 5.0).to_path(0.01);
            let geometry = vec![CutPath {
                path: path.clone(),
                color: None,
                label: None,
                paint: Default::default(),
            }];
            let o = Object::shape_only(
                Arc::new(geometry),
                path.bounding_box(),
                Default::default(),
                CutOptions::default(),
            );
            let _ = cut_bounds(&o, Knife::default());
            assert_eq!(Arc::strong_count(&o.geometry), 1, "held only weakly");
            assert!(
                bounds_memo_len() <= BOUNDS_MEMO_SIZE,
                "{}",
                bounds_memo_len()
            );
            objects.push(o);
        }
        assert_eq!(bounds_memo_len(), BOUNDS_MEMO_SIZE);
        // A second ask of a recent box is served from the memo.
        let last = objects.last().unwrap();
        assert_eq!(
            cut_bounds(last, Knife::default()),
            cut_bounds(last, Knife::default())
        );
        // Closed documents leave dead boxes; the next one clears them.
        objects.clear();
        let path = kurbo::Rect::new(0.0, 0.0, 1.0, 1.0).to_path(0.01);
        let o = Object::shape_only(
            Arc::new(vec![CutPath {
                path: path.clone(),
                color: None,
                label: None,
                paint: Default::default(),
            }]),
            path.bounding_box(),
            Default::default(),
            CutOptions::default(),
        );
        let _ = cut_bounds(&o, Knife::default());
        assert_eq!(bounds_memo_len(), 1);
    }

    #[test]
    fn mean_width_of_a_strip_is_its_width() {
        let strip = vec![vec![[0.0, 0.0], [50.0, 0.0], [50.0, 0.1], [0.0, 0.1]]];
        assert!((mean_width(&[strip]) - 0.1).abs() < 1e-3);
        assert_eq!(mean_width(&[]), 0.0);
    }

    #[test]
    fn open_subpaths_close_for_fills() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((10.0, 10.0));
        assert_eq!(contours(p.iter(), 0.01).len(), 1);
    }
}
