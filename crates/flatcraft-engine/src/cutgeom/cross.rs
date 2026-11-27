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
                out.area = boolean(&fill, &[], rule, tol)
                    .into_iter()
                    .flatten()
                    .collect();
            }
            out.no_area = out.area.is_empty() && out.lines.is_empty() && paint.stroke.is_none();
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
pub(crate) fn visible_alone(pieces: &[CutPiece], painted: &[Painted]) -> (Vec<CutPiece>, bool) {
    let mut out = Vec::new();
    let mut clipped = false;
    for piece in pieces {
        let later = piece
            .source
            .map_or(&[][..], |i| painted.get(i + 1..).unwrap_or_default());
        let b = piece.path.bounding_box();
        let cover: Vec<Contour> = later
            .iter()
            .filter(|p| p.area_box.is_some_and(|a| boxes_meet(a, b)))
            .flat_map(|p| p.area.iter().cloned())
            .collect();
        let (parts, changed) = clip_piece(piece, &cover, super::BOUNDS_TOLERANCE_MM);
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
    let before = length(&lines);
    if (before - length(&clipped)).abs() <= 1e-9 * before.max(1.0) {
        return (vec![piece.clone()], false);
    }
    let pieces = chain_once(&clipped)
        .into_iter()
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
        for a in 0..candidates.len() {
            for b in a + 1..candidates.len() {
                let (oa, ob) = (&objects[candidates[a]], &objects[candidates[b]]);
                if oa.keep != ob.keep {
                    continue;
                }
                let (Some(ba), Some(bb)) = (areas[a].1, areas[b].1) else {
                    continue;
                };
                if !boxes_meet(ba, bb) || find(&mut root, a) == find(&mut root, b) {
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

    // Visible outline: clipped by every shown painted area in front.
    let mut visible = Vec::new();
    for k in 0..objects.len() {
        let o = &objects[k];
        if o.cut.mode != CutMode::VisibleOutline {
            continue;
        }
        let base = out.solo[&o.id].pieces.clone();
        let front: Vec<&Painted> = objects[k + 1..]
            .iter()
            .enumerate()
            .filter(|(_, f)| f.visible)
            .flat_map(|(j, _)| painted(k + 1 + j).iter())
            .filter(|p| p.area_box.is_some())
            .collect();
        let mut pieces = Vec::new();
        let mut clipped = out.solo[&o.id].clipped;
        for piece in base.iter() {
            let b = piece.path.bounding_box();
            let cover: Vec<Contour> = front
                .iter()
                .filter(|p| p.area_box.is_some_and(|a| boxes_meet(a, b)))
                .flat_map(|p| p.area.iter().cloned())
                .collect();
            let (parts, changed) = clip_piece(piece, &cover, tol);
            clipped |= changed;
            pieces.extend(parts);
        }
        let covered = !base.is_empty() && pieces.is_empty();
        // The outline of what is left of a clipped object: its pieces are
        // no rings, so the plan reads this for what is kept.
        let region = (clipped && !covered && o.visible && o.cut_enabled).then(|| {
            let (own, own_box) = area_of(k);
            let cover: Vec<Contour> = front
                .iter()
                .filter(|p| {
                    p.area_box
                        .zip(own_box)
                        .is_some_and(|(a, b)| boxes_meet(a, b))
                })
                .flat_map(|p| p.area.iter().cloned())
                .collect();
            visible_region(&own, &cover, tol)
        });
        visible.push((o.id, covered, pieces, region));
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
    for (id, covered, pieces, region) in visible {
        out.per_object.insert(id, std::sync::Arc::new(pieces));
        if covered {
            out.covered.insert(id);
        }
        if let Some(region) = region.filter(|r| !r.is_empty()) {
            out.regions.insert(id, std::sync::Arc::new(region));
        }
    }
}

/// The painted area `own` without what `cover` covers: the outline of what
/// is seen of a visible outline, outer edges and holes. Every edge of it
/// is cut, by the object itself or by a shape in front.
fn visible_region(own: &[Contour], cover: &[Contour], tol: f64) -> Vec<Contour> {
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
        .flatten()
        .filter(|c| c.len() >= 3)
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
