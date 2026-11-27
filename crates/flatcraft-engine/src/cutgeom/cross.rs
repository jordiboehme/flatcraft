//! The cut modes that look across objects: merged outline and visible
//! outline.
//!
//! Both read the painted area of every source path ([`painted`]): its fill
//! under its own rule, united with its stroke when the stroke is cut as an
//! outline. Paths with no area (centre-line strokes, lines hidden under
//! their stroke, unpainted paths) are cut as their lines.
//!
//! - **Merged outline** ("weld"): shown, cut objects in this mode whose
//!   painted areas overlap form groups, transitively. A group is cut along
//!   the rings of the union of its painted areas, so the lines where they
//!   overlap are not cut; holes stay holes. The lines of the members are
//!   cut where they leave the union. Only members with the same keep mode
//!   weld: the whole weld belongs to the group's backmost object (the
//!   first in document order), so its keep mode is the one that counts,
//!   and a group of one keep mode never takes another member's choice.
//! - **Visible outline**: the z order is the document order (later objects
//!   in front), then the order of the source paths. Every shown object
//!   with painted area covers the objects behind it, whatever its mode and
//!   whether it is cut or skipped, since that is what the canvas shows. Only
//!   the objects in visible outline are clipped: the parts of their cuts
//!   that a shape in front covers are left out. A cut along the edge of a
//!   shape in front is left out too, so an edge both share is cut once, by
//!   the shape in front. A ring nothing covers stays closed; the parts of a
//!   clipped ring are open paths. An object in all lines covers with its
//!   fill only: it cuts no stroke outline, so its cut is the fill's edge.
//!
//! Hidden objects never cover, never weld, and are still cut as if they
//! were shown (the report checks them).

use std::collections::BTreeMap;

use flatcraft_core::document::{CutMode, CutOptions};
use flatcraft_core::geometry::CutPath;
use i_overlay::core::fill_rule::FillRule as OverlayFill;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::core::solver::Solver;
use i_overlay::float::clip::FloatClip;
use i_overlay::float::overlay::{FloatOverlay, OverlayOptions};
use i_overlay::string::clip::ClipRule;
use kurbo::{Affine, BezPath, Point, Rect, Shape};

use super::{
    Contour, CutGeometry, CutPiece, MIN_TOLERANCE_MM, PieceFacts, boolean, chain_once, fill_rule,
    hairline_mm, is_finite, mean_width, outline_style, outlined_area, placed_width, ring, spine,
    subpaths,
};
use crate::job::JobObject;

/// The painted parts of one placed source path.
#[derive(Debug, Clone, Default)]
pub(crate) struct Painted {
    /// The painted area: oriented contours (outer edges one way, holes the
    /// other), read with the non-zero rule.
    pub area: Vec<Contour>,
    /// The box of `area`.
    pub area_box: Option<Rect>,
    /// What is cut as a line: the parts without area.
    pub lines: Vec<BezPath>,
    /// A fill without stroke that gives nothing to cut (no area, or
    /// thinner than the tolerance).
    pub no_area: bool,
    /// Filled shapes without stroke thinner than the knife's narrowest
    /// feature: not cut and not covering (counted as slivers).
    pub slivers: usize,
}

fn tolerance_of(tol: f64) -> f64 {
    if tol.is_finite() {
        tol.max(MIN_TOLERANCE_MM)
    } else {
        MIN_TOLERANCE_MM
    }
}

/// The painted area of `path` placed with `affine`: the oriented fill (its
/// rule) united with its stroke when `options` cut the stroke as an
/// outline. A subpath whose fill is hidden under its centre-line stroke is
/// a line, not area (as in object outlines). Empty for a path without area.
pub fn painted_area(
    path: &CutPath,
    options: &CutOptions,
    affine: Affine,
    tol: f64,
) -> Vec<Contour> {
    painted(path, options, affine, tol, 0.0).area
}

/// [`painted_area`] and the lines of `path`. A stroked subpath narrower
/// than `min_width` is a line too, as in object outlines.
pub(crate) fn painted(
    path: &CutPath,
    options: &CutOptions,
    affine: Affine,
    tol: f64,
    min_width: f64,
) -> Painted {
    let tol = tolerance_of(tol);
    let min_width = if min_width.is_finite() {
        min_width.max(0.0)
    } else {
        0.0
    };
    let placed = affine * &path.path;
    let mut out = Painted::default();
    if !flatcraft_core::geometry::has_segments(&placed) {
        return out;
    }
    if !is_finite(&placed) {
        // Booleans need finite coordinates; the pipeline refuses the rest.
        out.lines.push(placed);
        return out;
    }
    let paint = path.paint;
    let style = options
        .mode
        .cuts_outlines()
        .then(|| outline_style(paint, options, affine))
        .flatten();
    match (paint.fill, style) {
        (None, None) => out.lines.push(placed),
        (Some(rule), None) => {
            let rule = fill_rule(rule);
            let stroke_width = paint
                .stroke
                .map(|s| placed_width(s.width_mm, affine))
                .filter(|w| w.is_finite());
            let mut fill: Vec<Contour> = Vec::new();
            for sub in subpaths(placed.iter()) {
                let contour = sub.contour(tol / 2.0);
                let own = match &contour {
                    Some(c) => boolean(std::slice::from_ref(c), &[], rule, tol),
                    None => Vec::new(),
                };
                let hidden_below = match stroke_width {
                    Some(w) if sub.ends_meet() => w.max(min_width),
                    Some(w) => (w / 2.0).max(min_width),
                    None => 0.0,
                };
                let width = mean_width(&own);
                if width < hidden_below.max(tol) {
                    if stroke_width.is_some() && !sub.segments.is_empty() {
                        let hairline = sub.ends_meet() && width < hairline_mm(tol, min_width);
                        out.lines
                            .push(match hairline.then(|| spine(&sub.points(tol / 2.0))) {
                                Some(Some(spine)) => spine,
                                _ => sub.elements().collect(),
                            });
                    }
                } else if let Some(c) = contour {
                    fill.push(c);
                }
            }
            if !fill.is_empty() {
                // As in object outlines (`fill_edges`): a shape thinner
                // than the knife's narrowest feature is not cut as a
                // shape, so it covers nothing either. A stroked one is cut
                // once, along its middle: that is a line.
                for shape in boolean(&fill, &[], rule, tol) {
                    let w = mean_width(std::slice::from_ref(&shape));
                    if w < tol {
                        continue;
                    }
                    if w < min_width {
                        match stroke_width {
                            Some(_) => out.lines.extend(super::sliver_once(&shape, &[], min_width)),
                            None => out.slivers += 1,
                        }
                        continue;
                    }
                    out.area.extend(shape);
                }
            }
            out.no_area = out.area.is_empty()
                && out.lines.is_empty()
                && out.slivers == 0
                && paint.stroke.is_none();
        }
        (fill, Some(style)) => {
            out.area = outlined_area(&placed, fill, &style, tol)
                .into_iter()
                .flatten()
                .collect();
        }
    }
    out.area.retain(|c| c.len() >= 3);
    out.area_box = contours_box(&out.area);
    out
}

fn contours_box(contours: &[Contour]) -> Option<Rect> {
    contours
        .iter()
        .flatten()
        .map(|p| Rect::from_points((p[0], p[1]), (p[0], p[1])))
        .reduce(|a, b| a.union(b))
}

fn boxes_meet(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// The pieces of one welded group: `members` are the source paths of each
/// member and their painted parts, the backmost member first. The rings of
/// the union of every painted area, then the lines of the members where
/// they leave it. A ring keeps its source path only when one path of a
/// lone object makes it; a weld of several paths has none. The lines of
/// the first member keep their source, the others' have none (their
/// index would name a path of another object).
pub(crate) fn weld(
    members: &[(&[CutPath], &[Painted])],
    tol: f64,
    min_width: f64,
) -> (Vec<CutPiece>, PieceFacts) {
    let tol = tolerance_of(tol);
    let min_width = if min_width.is_finite() {
        min_width.max(0.0)
    } else {
        0.0
    };
    let mut facts = PieceFacts::default();
    let mut contours: Vec<Contour> = Vec::new();
    let mut makers: Vec<(usize, usize)> = Vec::new();
    for (m, (_, painted)) in members.iter().enumerate() {
        for (i, p) in painted.iter().enumerate() {
            if p.no_area {
                facts.no_area += 1;
            }
            facts.slivers += p.slivers;
            if !p.area.is_empty() {
                makers.push((m, i));
                contours.extend(p.area.iter().cloned());
            }
        }
    }
    let union = boolean(&contours, &[], OverlayFill::NonZero, tol);
    let mut kept: Vec<Contour> = Vec::new();
    for shape in union {
        let w = mean_width(std::slice::from_ref(&shape));
        if w < tol {
            continue;
        }
        if w < min_width {
            facts.slivers += 1;
            continue;
        }
        kept.extend(shape.into_iter().filter(|c| c.len() >= 3));
    }
    let (source, label) = match makers.as_slice() {
        [(0, i)] if members.len() == 1 => (Some(*i), members[0].0[*i].label.clone()),
        _ => (None, None),
    };
    let mut pieces: Vec<CutPiece> = kept
        .iter()
        .map(|c| CutPiece {
            source,
            label: label.clone(),
            path: ring(c),
        })
        .collect();
    let union_box = contours_box(&kept);
    for (m, (geometry, painted)) in members.iter().enumerate() {
        for (i, p) in painted.iter().enumerate() {
            for line in &p.lines {
                let piece = CutPiece {
                    source: (m == 0).then_some(i),
                    label: if m == 0 {
                        geometry[i].label.clone()
                    } else {
                        None
                    },
                    path: line.clone(),
                };
                let cover = match union_box {
                    Some(b) if boxes_meet(b, line.bounding_box()) => kept.as_slice(),
                    _ => &[],
                };
                pieces.extend(clip_piece(&piece, cover, tol).0);
            }
        }
    }
    (pieces, facts)
}

/// The pieces of one object in visible outline, alone: the cuts of each
/// source path (`pieces`, cut as object outlines) without what its later
/// paths (`painted`) cover, and whether anything was left out.
pub(crate) fn visible_alone(
    pieces: &[CutPiece],
    painted: &[Painted],
    tol: f64,
) -> (Vec<CutPiece>, bool) {
    let mut out = Vec::new();
    let mut clipped = false;
    for piece in pieces {
        let later = piece
            .source
            .map_or(&[][..], |i| painted.get(i + 1..).unwrap_or_default());
        // Grown by the join tolerance: an edge just outside counts too.
        let near = super::CLOSE_TOLERANCE_MM;
        let b = piece.path.bounding_box().inflate(near, near);
        let cover: Vec<Contour> = later
            .iter()
            .filter(|p| p.area_box.is_some_and(|a| boxes_meet(a, b)))
            .flat_map(|p| p.area.iter().cloned())
            .collect();
        let (parts, changed) = clip_piece(piece, &cover, tol);
        clipped |= changed;
        out.extend(parts);
    }
    (out, clipped)
}

/// The parts of `piece` outside the painted area `cover` (oriented
/// contours, read with the non-zero rule). A part along the edge of
/// `cover` is left out: the shape in front cuts it. `piece` itself when
/// nothing of it is covered, so a ring stays closed; otherwise open paths,
/// joined where they meet. Curves are flattened within `tol / 2`.
fn clip_piece(piece: &CutPiece, cover: &[Contour], tol: f64) -> (Vec<CutPiece>, bool) {
    if cover.is_empty() {
        return (vec![piece.clone()], false);
    }
    let mut lines: Vec<Contour> = Vec::new();
    for sub in subpaths(piece.path.iter()) {
        let mut points = sub.points(tol / 2.0);
        if sub.closed
            && let Some(first) = points.first().copied()
        {
            points.push(first);
        }
        if points.len() >= 2 {
            lines.push(points);
        }
    }
    let length = |c: &[Contour]| -> f64 {
        c.iter()
            .flat_map(|p| p.windows(2))
            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
            .sum()
    };
    // The edge is never part of the result. With `invert`, i_overlay
    // turns `boundary_included` around (true leaves the edge out of the
    // outside part; pinned by `spurs_keep_both_sides_and_leave_the_edge`).
    let rule = ClipRule {
        invert: true,
        boundary_included: true,
    };
    let cover = cover.to_vec();
    let clipped: Vec<Contour> = lines.clip_by_as::<i64>(&cover, OverlayFill::NonZero, rule);
    // A part that runs along the edge of the cover within the join
    // tolerance, just outside it, is that edge too: the shape in front
    // cuts it, so it is left out (cut once, not twice a hair apart). A
    // short part that crosses the band (where the cut meets the front
    // shape's edge) stays.
    let band = edge_band(&cover, super::CLOSE_TOLERANCE_MM);
    let away: Vec<Contour> = clipped.clip_by_as::<i64>(&band, OverlayFill::NonZero, rule);
    let along: Vec<Contour> = clipped.clip_by_as::<i64>(
        &band,
        OverlayFill::NonZero,
        ClipRule {
            invert: false,
            boundary_included: false,
        },
    );
    let crossing = along
        .into_iter()
        .filter(|c| length(std::slice::from_ref(c)) <= 3.0 * super::CLOSE_TOLERANCE_MM);
    let kept: Vec<Contour> = away.into_iter().chain(crossing).collect();
    let before = length(&lines);
    if (before - length(&kept)).abs() <= 1e-9 * before.max(1.0) {
        return (vec![piece.clone()], false);
    }
    let pieces = chain_once(&kept)
        .into_iter()
        .map(|c| straight(&c))
        .filter(|c| c.len() >= 2)
        .map(|c| {
            let closed = c.len() >= 4 && c.first() == c.last();
            let path = if closed {
                ring(&c[..c.len() - 1])
            } else {
                let mut path = BezPath::new();
                path.move_to(Point::new(c[0][0], c[0][1]));
                for q in &c[1..] {
                    path.line_to(Point::new(q[0], q[1]));
                }
                path
            };
            CutPiece {
                source: piece.source,
                label: piece.label.clone(),
                path,
            }
        })
        .collect();
    (pieces, true)
}

/// `line` without the vertices that lie on a straight run (where the
/// clips split a segment), so a clipped edge stays one segment.
fn straight(line: &[[f64; 2]]) -> Contour {
    let mut out: Contour = Vec::with_capacity(line.len());
    for &p in line {
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let (u, v) = ([b[0] - a[0], b[1] - a[1]], [p[0] - b[0], p[1] - b[1]]);
            let cross = u[0] * v[1] - u[1] * v[0];
            let dot = u[0] * v[0] + u[1] * v[1];
            let scale = u[0].hypot(u[1]) * v[0].hypot(v[1]);
            if dot > 0.0 && cross.abs() <= 1e-9 * scale {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

/// One rectangle per edge of the closed `contours`, `half` to each side,
/// all wound the same way: the band along their edges.
fn edge_band(contours: &[Contour], half: f64) -> Vec<Contour> {
    let mut out = Vec::new();
    for c in contours {
        let n = c.len();
        for k in 0..n {
            let (a, b) = (c[k], c[(k + 1) % n]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = dx.hypot(dy);
            if len <= 0.0 {
                continue;
            }
            // Grown a little along the edge too, so the corners are in.
            let (ux, uy) = (dx * half / len, dy * half / len);
            let (nx, ny) = (-uy, ux);
            out.push(vec![
                [a[0] - ux - nx, a[1] - uy - ny],
                [b[0] + ux - nx, b[1] + uy - ny],
                [b[0] + ux + nx, b[1] + uy + ny],
                [a[0] - ux + nx, a[1] - uy + ny],
            ]);
        }
    }
    out
}

/// Boxes sorted into a grid of cells about one box wide, built once, for
/// asking which boxes meet a box. A box that would cover too many cells,
/// and a query that would, fall back to a plain walk over the boxes.
struct BoxGrid {
    boxes: Vec<Option<Rect>>,
    cell: f64,
    cells: std::collections::HashMap<(i64, i64), Vec<usize>>,
    huge: Vec<usize>,
}

/// A box over more cells than this is not put into the grid.
const MAX_GRID_CELLS: i64 = 4096;

impl BoxGrid {
    fn new(boxes: Vec<Option<Rect>>) -> Self {
        let sides: Vec<f64> = boxes
            .iter()
            .flatten()
            .map(|b| b.width().max(b.height()))
            .collect();
        let mean = sides.iter().sum::<f64>() / sides.len().max(1) as f64;
        let mut grid = Self {
            boxes,
            cell: mean.max(1e-3),
            cells: Default::default(),
            huge: Vec::new(),
        };
        for i in 0..grid.boxes.len() {
            let Some(b) = grid.boxes[i] else {
                continue;
            };
            match grid.span(b) {
                Some((x0, y0, x1, y1)) => {
                    for x in x0..=x1 {
                        for y in y0..=y1 {
                            grid.cells.entry((x, y)).or_default().push(i);
                        }
                    }
                }
                None => grid.huge.push(i),
            }
        }
        grid
    }

    /// The cells of `b`, or `None` when they are too many.
    /// Huge or broken coordinates (beyond the i64 range, NaN) never
    /// overflow: such a box is "too many cells".
    fn span(&self, b: Rect) -> Option<(i64, i64, i64, i64)> {
        let c = |v: f64| {
            let k = (v / self.cell).floor();
            (k.is_finite() && k.abs() < 1e15).then_some(k as i64)
        };
        let (x0, y0, x1, y1) = (c(b.x0)?, c(b.y0)?, c(b.x1)?, c(b.y1)?);
        let w = x1.checked_sub(x0)?.checked_add(1)?;
        let h = y1.checked_sub(y0)?.checked_add(1)?;
        (w > 0 && h > 0 && w.checked_mul(h)? <= MAX_GRID_CELLS).then_some((x0, y0, x1, y1))
    }

    /// Calls `f` once with the index of every box that meets `b`, in no
    /// particular order.
    fn query(&self, b: Rect, mut f: impl FnMut(usize)) {
        let mut test = |i: usize| {
            #[cfg(test)]
            tests::BOX_TESTS.with(|n| n.set(n.get() + 1));
            if self.boxes[i].is_some_and(|a| boxes_meet(a, b)) {
                f(i);
            }
        };
        let Some((x0, y0, x1, y1)) = self.span(b) else {
            (0..self.boxes.len()).for_each(test);
            return;
        };
        let mut seen = std::collections::HashSet::new();
        for x in x0..=x1 {
            for y in y0..=y1 {
                for &i in self.cells.get(&(x, y)).into_iter().flatten() {
                    if seen.insert(i) {
                        test(i);
                    }
                }
            }
        }
        for &i in &self.huge {
            test(i);
        }
    }
}

/// True if the painted areas `a` and `b` overlap with an area larger than
/// the tolerance squared.
fn areas_overlap(a: &[Contour], b: &[Contour], tol: f64) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let mut options = OverlayOptions::<f64>::default();
    options.min_output_area = tol * tol;
    let (a, b) = (a.to_vec(), b.to_vec());
    !FloatOverlay::with_subj_and_clip_custom(&a, &b, options, Solver::default())
        .overlay(OverlayRule::Intersect, OverlayFill::NonZero)
        .is_empty()
}

/// Works out the cross-object modes of `objects` over the first layer
/// already in `out` (its solo pieces), and writes the final pieces, facts
/// and boxes of the objects in those modes.
pub(super) fn apply(objects: &[JobObject], out: &mut CutGeometry, tol: f64, min_width: f64) {
    let tol = tolerance_of(tol);
    let painted = |k: usize| -> &[Painted] {
        let o = &objects[k];
        out.solo[&o.id].painted(o, tol, min_width)
    };
    let area_of = |k: usize| -> (Vec<Contour>, Option<Rect>) {
        let parts = painted(k);
        let area: Vec<Contour> = parts.iter().flat_map(|p| p.area.iter().cloned()).collect();
        let b = parts
            .iter()
            .filter_map(|p| p.area_box)
            .reduce(|a, b| a.union(b));
        (area, b)
    };

    // Merged outline: groups of overlapping shown, cut objects with the
    // same keep mode.
    let candidates: Vec<usize> = (0..objects.len())
        .filter(|&k| {
            let o = &objects[k];
            o.cut.mode == CutMode::MergedOutline && o.visible && o.cut_enabled
        })
        .collect();
    let mut welds: Vec<(usize, Vec<usize>)> = Vec::new();
    if candidates.len() > 1 {
        let areas: Vec<(Vec<Contour>, Option<Rect>)> =
            candidates.iter().map(|&k| area_of(k)).collect();
        let mut root: Vec<usize> = (0..candidates.len()).collect();
        fn find(root: &mut [usize], mut a: usize) -> usize {
            while root[a] != a {
                root[a] = root[root[a]];
                a = root[a];
            }
            a
        }
        // Only candidates whose boxes meet are compared (a grid of boxes),
        // so a sheet of many objects costs about as many tests as it has
        // objects, not their square.
        let grid = BoxGrid::new(areas.iter().map(|a| a.1).collect());
        for a in 0..candidates.len() {
            let Some(ba) = areas[a].1 else {
                continue;
            };
            let mut near = Vec::new();
            grid.query(ba, |b| {
                if b > a {
                    near.push(b);
                }
            });
            for b in near {
                let (oa, ob) = (&objects[candidates[a]], &objects[candidates[b]]);
                if oa.keep != ob.keep || find(&mut root, a) == find(&mut root, b) {
                    continue;
                }
                if areas_overlap(&areas[a].0, &areas[b].0, tol) {
                    let (ra, rb) = (find(&mut root, a), find(&mut root, b));
                    root[ra.max(rb)] = ra.min(rb);
                }
            }
        }
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (a, &k) in candidates.iter().enumerate() {
            let r = find(&mut root, a);
            groups.entry(r).or_default().push(k);
        }
        welds = groups
            .into_values()
            .filter(|g| g.len() > 1)
            .map(|g| (g[0], g))
            .collect();
    }
    let mut welded = Vec::new();
    for (back, group) in &welds {
        let members: Vec<(&[CutPath], &[Painted])> = group
            .iter()
            .map(|&k| (objects[k].geometry.as_slice(), painted(k)))
            .collect();
        let (pieces, facts) = weld(&members, tol, min_width);
        welded.push((*back, group.clone(), pieces, facts));
    }

    // Visible outline: clipped by every shown painted area in front. The
    // painted paths of the shown objects sit in a grid of boxes, so each
    // piece is compared only with the shapes near it.
    let mut visible = Vec::new();
    if objects
        .iter()
        .any(|o| o.cut.mode == CutMode::VisibleOutline)
    {
        let first = objects
            .iter()
            .position(|o| o.cut.mode == CutMode::VisibleOutline)
            .unwrap_or(0);
        let shapes: Vec<(usize, &Painted)> = (first + 1..objects.len())
            .filter(|&j| objects[j].visible)
            .flat_map(|j| painted(j).iter().map(move |p| (j, p)))
            .filter(|(_, p)| p.area_box.is_some())
            .collect();
        let grid = BoxGrid::new(shapes.iter().map(|(_, p)| p.area_box).collect());
        // The painted areas in front of object `k` whose boxes meet `b`.
        let cover_of = |k: usize, b: Rect| -> Vec<Contour> {
            let mut hits = Vec::new();
            grid.query(b, |i| {
                if shapes[i].0 > k {
                    hits.push(i);
                }
            });
            hits.sort_unstable();
            hits.into_iter()
                .flat_map(|i| shapes[i].1.area.iter().cloned())
                .collect()
        };
        for (k, o) in objects.iter().enumerate().skip(first) {
            if o.cut.mode != CutMode::VisibleOutline {
                continue;
            }
            let base = out.solo[&o.id].pieces.clone();
            let mut pieces = Vec::new();
            let mut clipped = out.solo[&o.id].clipped;
            for piece in base.iter() {
                // Grown by the join tolerance: an edge just outside a
                // front shape is its edge too.
                let near = super::CLOSE_TOLERANCE_MM;
                let cover = cover_of(k, piece.path.bounding_box().inflate(near, near));
                let (parts, changed) = clip_piece(piece, &cover, tol);
                clipped |= changed;
                pieces.extend(parts);
            }
            let covered = !base.is_empty() && pieces.is_empty();
            let mut facts = out.facts(o.id);
            let mut seen = Vec::new();
            // The outline of what is left of a clipped object: its pieces
            // are no rings, so the plan reads this for what is kept.
            let mut region = Vec::new();
            if clipped && !covered {
                let (own, own_box) = area_of(k);
                let cover = own_box.map_or_else(Vec::new, |b| cover_of(k, b));
                let stroked = o.geometry.iter().any(|p| p.paint.stroke.is_some());
                for shape in visible_region(&own, &cover, tol) {
                    let w = mean_width(std::slice::from_ref(&shape));
                    seen.push((w, contours_box(&shape)));
                    if w >= min_width {
                        region.extend(shape);
                        continue;
                    }
                    // A seen strip thinner than the knife's narrowest
                    // feature, as for any filled sliver: its own cut is
                    // dropped (the front shape's cut stays), and a stroked
                    // one is cut once, along its middle.
                    let source = thin_strip(&mut pieces, &shape);
                    if w >= tol {
                        match stroked
                            .then(|| super::sliver_once(&shape, &[], min_width))
                            .flatten()
                        {
                            Some(path) => pieces.push(CutPiece {
                                source,
                                label: source
                                    .and_then(|i| o.geometry.get(i))
                                    .and_then(|p| p.label.clone()),
                                path,
                            }),
                            None => facts.slivers += 1,
                        }
                    }
                }
            }
            let region = (o.visible && o.cut_enabled && !region.is_empty()).then_some(region);
            visible.push((o.id, covered, pieces, region, facts, seen));
        }
    }

    for (back, group, pieces, facts) in welded {
        let back_id = objects[back].id;
        let weld_box = pieces
            .iter()
            .map(|p| p.path.bounding_box())
            .reduce(|a, b| a.union(b));
        let own = out.bounds(back_id);
        let b = match (own, weld_box) {
            (Some(a), Some(b)) => Some(a.union(b)),
            (a, b) => a.or(b),
        };
        out.per_object.insert(back_id, std::sync::Arc::new(pieces));
        out.facts.insert(back_id, facts);
        out.bounds.insert(back_id, b);
        for &k in &group[1..] {
            let id = objects[k].id;
            out.per_object.insert(id, std::sync::Arc::new(Vec::new()));
            out.facts.insert(id, PieceFacts::default());
            out.merged_into.insert(id, back_id);
        }
    }
    for (id, covered, pieces, region, facts, seen) in visible {
        out.per_object.insert(id, std::sync::Arc::new(pieces));
        out.facts.insert(id, facts);
        if !seen.is_empty() {
            out.seen_shapes.insert(id, seen);
        }
        if covered {
            out.covered.insert(id);
        }
        if let Some(region) = region.filter(|r| !r.is_empty()) {
            out.regions.insert(id, std::sync::Arc::new(region));
        }
    }
}

/// Takes the pieces of a visible outline that run along the seen strip
/// `shape` (every vertex on its edge) out of `pieces`, and gives the
/// source path of the first one.
fn thin_strip(pieces: &mut Vec<CutPiece>, shape: &[Contour]) -> Option<usize> {
    let edges: Vec<Contour> = shape
        .iter()
        .filter_map(|c| {
            let mut c = c.clone();
            c.push(*c.first()?);
            Some(c)
        })
        .collect();
    let refs: Vec<&[[f64; 2]]> = edges.iter().map(Vec::as_slice).collect();
    let index = super::NearIndex::new(&refs, 1e-3);
    let mut source = None;
    pieces.retain(|p| {
        let on = p
            .path
            .elements()
            .iter()
            .filter_map(|e| e.end_point())
            .all(|q| index.near([q.x, q.y]));
        if on {
            source = source.or(p.source);
        }
        !on
    });
    source
}

/// The painted area `own` without what `cover` covers, as shapes (an
/// outer edge and its holes): what is seen of a visible outline. Every
/// edge of it is cut, by the object itself or by a shape in front.
fn visible_region(own: &[Contour], cover: &[Contour], tol: f64) -> Vec<Vec<Contour>> {
    if own.is_empty() {
        return Vec::new();
    }
    let mut options = OverlayOptions::<f64>::default();
    options.min_output_area = tol * tol;
    let (own, cover) = (own.to_vec(), cover.to_vec());
    let rule = if cover.is_empty() {
        OverlayRule::Subject
    } else {
        OverlayRule::Difference
    };
    FloatOverlay::with_subj_and_clip_custom(&own, &cover, options, Solver::default())
        .overlay(rule, OverlayFill::NonZero)
        .into_iter()
        .map(|shape| {
            shape
                .into_iter()
                .filter(|c| c.len() >= 3)
                .collect::<Vec<_>>()
        })
        .filter(|shape| !shape.is_empty())
        .collect()
}

/// True if `a` and `b` would weld when both were set to merged outline:
/// both shown and cut, the same keep mode, and painted areas that overlap.
/// What the "Merge into one outline" fix checks before it is offered, so
/// it is offered only when it clears the hint.
pub(crate) fn would_weld(a: &JobObject, b: &JobObject, tol: f64, min_width: f64) -> bool {
    if !(a.visible && a.cut_enabled && b.visible && b.cut_enabled) || a.keep != b.keep {
        return false;
    }
    let tol = tolerance_of(tol);
    let area = |o: &JobObject| -> Vec<Contour> {
        let options = CutOptions {
            mode: CutMode::MergedOutline,
            ..o.cut
        };
        let affine = o.transform.affine(o.local_bounds);
        o.geometry
            .iter()
            .flat_map(|p| painted(p, &options, affine, tol, min_width).area)
            .collect()
    };
    areas_overlap(&area(a), &area(b), tol)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    thread_local! {
        /// How many boxes the grids of the cross-object pass tested on
        /// this thread.
        pub(crate) static BOX_TESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// `n` squares of 8 mm on a grid, each overlapping its neighbours by
    /// 1 mm, all in `mode`.
    fn sheet(n: usize, mode: CutMode) -> Vec<JobObject> {
        use flatcraft_core::document::{Document, JobSettings, Source};
        let mut doc = Document::new(JobSettings {
            plotter_id: "cameo2".into(),
            material_id: "cardstock".into(),
            media_id: "a4".into(),
            speed: None,
            force: None,
            passes: None,
            strategy: Default::default(),
            weed: Default::default(),
        });
        let side = (n as f64).sqrt().ceil() as usize;
        for k in 0..n {
            let (x, y) = ((k % side) as f64 * 7.0, (k / side) as f64 * 7.0);
            let path = Rect::new(x, y, x + 8.0, y + 8.0).to_path(0.01);
            let id = doc.add(
                format!("s{k}"),
                Source::Svg {
                    name: "s.svg".into(),
                    bytes: Vec::new().into(),
                },
                vec![CutPath {
                    path,
                    color: None,
                    label: None,
                    paint: Default::default(),
                }],
            );
            let o = doc.get_mut(id).unwrap();
            o.transform.center_mm = o.local_bounds.center();
            o.cut.mode = mode;
        }
        crate::job::JobInput::objects_of(&doc)
    }

    /// The box tests of one build of the cross-object pass grow about
    /// linearly with the number of objects: four times the objects may
    /// cost at most six times the tests (a square would be sixteen).
    #[test]
    fn the_cross_object_pass_is_not_quadratic() {
        for mode in [CutMode::MergedOutline, CutMode::VisibleOutline] {
            let tests_for = |n: usize| {
                let objects = sheet(n, mode);
                BOX_TESTS.with(|c| c.set(0));
                let _ = crate::cutgeom::objects_cut_geometry(&objects, 0.01, 0.125, None);
                BOX_TESTS.with(|c| c.get())
            };
            let (small, large) = (tests_for(100), tests_for(400));
            assert!(small > 0, "{mode:?}");
            assert!(
                large <= 6 * small,
                "{mode:?}: {small} tests for 100 objects, {large} for 400"
            );
        }
    }

    /// Coordinates far beyond the i64 range of the cells, or not finite,
    /// never overflow or walk billions of cells: such a box is tested on
    /// every query, and such a query tests every box.
    #[test]
    fn the_box_grid_takes_huge_coordinates() {
        let boxes = vec![
            Some(Rect::new(0.0, 0.0, 1.0, 1.0)),
            Some(Rect::new(1e25, 1e25, 1e25 + 1.0, 1e25 + 1.0)),
            Some(Rect::new(-1e300, -1e300, 1e300, 1e300)),
            Some(Rect::new(f64::NAN, 0.0, 1.0, 1.0)),
        ];
        let grid = BoxGrid::new(boxes);
        for q in [
            Rect::new(-1e30, -1e30, 1.0, 1.0),
            Rect::new(1e25, 1e25, 1e25 + 1.0, 1e25 + 1.0),
            Rect::new(0.5, 0.5, 0.6, 0.6),
        ] {
            let mut got = Vec::new();
            grid.query(q, |i| got.push(i));
            got.sort_unstable();
            assert!(got.contains(&2), "{q:?}: {got:?}");
        }
        let mut got = Vec::new();
        grid.query(Rect::new(0.5, 0.5, 0.6, 0.6), |i| got.push(i));
        got.sort_unstable();
        assert_eq!(got, [0, 2]);
    }

    #[test]
    fn the_box_grid_finds_exactly_the_boxes_that_meet() {
        let boxes: Vec<Option<Rect>> = (0..50)
            .map(|k| {
                let x = f64::from(k % 10) * 5.0;
                let y = f64::from(k / 10) * 5.0;
                (k % 7 != 3).then(|| Rect::new(x, y, x + 6.0, y + 6.0))
            })
            .chain([Some(Rect::new(-1e6, -1e6, 1e6, 1e6))])
            .collect();
        let grid = BoxGrid::new(boxes.clone());
        for q in [
            Rect::new(0.0, 0.0, 1.0, 1.0),
            Rect::new(12.0, 7.0, 13.0, 30.0),
            Rect::new(-1e7, -1e7, 1e7, 1e7),
        ] {
            let mut got = Vec::new();
            grid.query(q, |i| got.push(i));
            got.sort_unstable();
            let want: Vec<usize> = (0..boxes.len())
                .filter(|&i| boxes[i].is_some_and(|a| boxes_meet(a, q)))
                .collect();
            assert_eq!(got, want, "{q:?}");
        }
    }
}
