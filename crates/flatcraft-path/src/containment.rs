//! Containment: which path lies in which ring, the side of each ring that
//! is kept, and the job-wide classification of kept material versus waste
//! (spec 8.3).
//!
//! Rings are read as one lap (Ruling F3): pass a toolpath's `lap()`, never
//! its `points`, because a ring traced twice cancels itself under even-odd.

use std::collections::HashMap;

use kurbo::{ParamCurveNearest as _, Point, Rect};

use crate::clean::Polyline;
use crate::plan::KeepSide;

/// Points closer than this to a ring count as on it.
pub const EPS: f64 = 1e-9;

/// How many vertices of an inner ring are tried before giving up.
pub const PROBES: usize = 3;

#[cfg(test)]
thread_local! {
    /// Even-odd tests run on this thread (`point_in_ring`).
    pub(crate) static INSIDE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Shoelace area of a ring in mat coordinates (y down). A ring that runs
/// clockwise on screen has a positive area.
pub fn signed_area(ring: &[Point]) -> f64 {
    let n = ring.len();
    let twice: f64 = (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    twice / 2.0
}

/// Even-odd test of `p` against the polygon `ring`. `None` when `p` lies
/// on (or within [`EPS`] of) the boundary, where the answer is ambiguous.
pub fn point_in_ring(p: Point, ring: &[Point]) -> Option<bool> {
    #[cfg(test)]
    INSIDE_CALLS.with(|c| c.set(c.get() + 1));
    let mut inside = false;
    let n = ring.len();
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        if kurbo::Line::new(a, b).nearest(p, 0.0).distance_sq <= EPS * EPS {
            return None;
        }
        if crosses_ray(a, b, p).is_some() {
            inside = !inside;
        }
    }
    Some(inside)
}

/// Where segment `a`-`b` crosses the ray from `p` towards +x, if it does.
/// A segment's lower end is half-open, so a ray through a vertex counts
/// it once.
fn crosses_ray(a: Point, b: Point, p: Point) -> Option<f64> {
    if (a.y > p.y) == (b.y > p.y) {
        return None;
    }
    let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
    (p.x < x).then_some(x)
}

/// The bounding box of `points`, `None` when there are none.
pub(crate) fn bbox(points: &[Point]) -> Option<Rect> {
    let (first, rest) = points.split_first()?;
    Some(
        rest.iter()
            .fold(Rect::from_points(*first, *first), |r, p| r.union_pt(*p)),
    )
}

fn box_in(inner: Rect, outer: Rect) -> bool {
    inner.x0 >= outer.x0 && inner.y0 >= outer.y0 && inner.x1 <= outer.x1 && inner.y1 <= outer.y1
}

/// True if closed ring `inner` lies in closed ring `outer` (spec 8.3,
/// R32a, Ruling F4): its box lies in the outer box, its absolute area `ia`
/// is smaller than `oa`, and the first of up to [`PROBES`] vertices that
/// is not on the outer boundary is inside. Partial overlaps fail the box
/// test. A ring that pokes out of a parent sharing its box can nest
/// wrongly; M1 accepted that cost.
pub(crate) fn nested(
    inner: &[Point],
    ib: Rect,
    ia: f64,
    outer: &[Point],
    ob: Rect,
    oa: f64,
) -> bool {
    if !box_in(ib, ob) || ia >= oa {
        return false;
    }
    inner
        .iter()
        .take(PROBES)
        .find_map(|p| point_in_ring(*p, outer))
        .unwrap_or(false)
}

/// Which path lies in which closed path. Indices are those of the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainmentTree {
    /// The smallest closed path that holds each path.
    pub parent: Vec<Option<usize>>,
    /// The paths each path holds directly, in input order. Only closed
    /// paths have children.
    pub children: Vec<Vec<usize>>,
    /// The paths no closed path holds, in input order.
    pub roots: Vec<usize>,
    /// The number of closed paths around each path.
    pub depth: Vec<usize>,
}

/// Build the containment tree of `paths` (spec 8.3). Toolpath callers pass
/// one lap per ring (`Toolpath::lap()`).
///
/// - A closed path B lies in a closed path A by [`nested`].
/// - An open path lies in a closed path when its box lies in the ring's
///   box and none of its vertices is outside (the boundary counts as
///   inside).
/// - The parent is the smallest container. Candidates are searched from
///   the smallest larger ring upward and the first hit wins, so deep
///   nesting costs about one nesting test per ring.
pub fn build(paths: &[Polyline]) -> ContainmentTree {
    let n = paths.len();
    // (index, box, absolute area) of every closed path, smallest first.
    let mut rings: Vec<(usize, Rect, f64)> = paths
        .iter()
        .enumerate()
        .filter(|(_, p)| p.closed)
        .filter_map(|(i, p)| Some((i, bbox(&p.points)?, signed_area(&p.points).abs())))
        .collect();
    rings.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)));

    let mut parent = vec![None; n];
    for (k, &(j, jb, ja)) in rings.iter().enumerate() {
        let larger = &rings[k..];
        let larger = &larger[larger.partition_point(|r| r.2 <= ja)..];
        parent[j] = larger
            .iter()
            .find(|&&(i, ib, ia)| nested(&paths[j].points, jb, ja, &paths[i].points, ib, ia))
            .map(|r| r.0);
    }
    let open: Vec<usize> = (0..n).filter(|&j| !paths[j].closed).collect();
    for &j in &open {
        let Some(jb) = bbox(&paths[j].points) else {
            continue;
        };
        parent[j] = rings
            .iter()
            .find(|&&(i, ib, ia)| {
                ia > 0.0
                    && box_in(jb, ib)
                    && paths[j]
                        .points
                        .iter()
                        .all(|q| point_in_ring(*q, &paths[i].points) != Some(false))
            })
            .map(|r| r.0);
    }

    // A parent is a larger closed ring, so going from the largest down
    // sets every parent's depth before its children's.
    let mut depth = vec![0; n];
    for j in rings.iter().rev().map(|r| r.0).chain(open) {
        if let Some(p) = parent[j] {
            depth[j] = depth[p] + 1;
        }
    }
    let mut children = vec![Vec::new(); n];
    let mut roots = Vec::new();
    for (j, p) in parent.iter().enumerate() {
        match p {
            Some(p) => children[*p].push(j),
            None => roots.push(j),
        }
    }
    ContainmentTree {
        parent,
        children,
        roots,
        depth,
    }
}

/// The side of path `i` the kept material lies on: inside for a ring at
/// even depth (a piece), outside at odd depth (a hole), both for an open
/// path.
pub fn keep_side(tree: &ContainmentTree, i: usize, closed: bool) -> KeepSide {
    if !closed {
        KeepSide::Both
    } else if tree.depth[i].is_multiple_of(2) {
        KeepSide::Inside
    } else {
        KeepSide::Outside
    }
}

/// Which side of an object's cuts is kept material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeepMode {
    /// The cut shapes are kept: inside an odd number of the object's rings.
    /// Outside and the holes are waste.
    #[default]
    Pieces,
    /// The sheet is kept and the cut shapes fall out: inside an odd number
    /// of the object's rings is waste, the rest of the cutting area is kept.
    Stencil,
    /// Both sides are kept: the pieces, and everything within the
    /// clearance of the object's rings, so no lead-in or overshoot lands
    /// next to its cuts.
    Both,
}

/// The closed `Cut` rings of one object, one lap each, and its keep mode.
#[derive(Debug, Clone)]
pub struct KeptObject<'a> {
    pub rings: Vec<&'a [Point]>,
    pub mode: KeepMode,
}

/// The edge of a grid cell of [`KeptMaterial`], in millimetres.
const CELL_MM: f64 = 1.0;

/// Segments are indexed in pieces no longer than this, so a long diagonal
/// fills a band of cells instead of its whole box.
const PIECE_MM: f64 = CELL_MM / 2.0;

fn cell(v: f64) -> i64 {
    (v / CELL_MM).floor() as i64
}

/// What is kept material and what is waste, for the whole job (review C3).
/// Classifying the whole job, not one path, means a waste move is never
/// judged safe while it cuts a neighbour or crosses a hole into its parent.
///
/// Each object classifies by its [`KeepMode`], counting even-odd over its
/// own rings (piece 1, hole 2, piece in a hole 3). Across objects kept
/// wins: a point is waste only when no object calls it kept.
#[derive(Debug, Clone)]
pub struct KeptMaterial {
    /// Every ring edge as (start, end, ring index).
    segs: Vec<(Point, Point, usize)>,
    /// The object each ring belongs to.
    ring_object: Vec<usize>,
    /// The keep mode of each object.
    modes: Vec<KeepMode>,
    /// A stencil's sheet: the whole cutting area, boundary included.
    sheet: Rect,
    /// How far from a `Both` object's rings everything is kept.
    clearance_mm: f64,
    /// A uniform grid of [`CELL_MM`] cells: the edges near each cell.
    cells: HashMap<(i64, i64), Vec<usize>>,
    /// The last column of each row that holds an edge.
    row_end: HashMap<i64, i64>,
    /// The cells that hold edges: (col0, row0, col1, row1).
    extent: Option<(i64, i64, i64, i64)>,
}

impl KeptMaterial {
    /// Index all closed `Cut` rings of the job, one lap each, as one
    /// [`KeepMode::Pieces`] object: a point is kept when it lies strictly
    /// inside an odd number of them.
    pub fn new(rings: &[&[Point]]) -> Self {
        Self::with_modes(
            &[KeptObject {
                rings: rings.to_vec(),
                mode: KeepMode::Pieces,
            }],
            Rect::ZERO,
            0.0,
        )
    }

    /// Index the closed `Cut` rings of each object, one lap each.
    /// `cutting_area` is the sheet a [`KeepMode::Stencil`] keeps, and
    /// `clearance_mm` how far from a [`KeepMode::Both`] object's rings
    /// everything is kept.
    pub fn with_modes(objects: &[KeptObject], cutting_area: Rect, clearance_mm: f64) -> Self {
        let mut kept = Self {
            segs: Vec::new(),
            ring_object: Vec::new(),
            modes: objects.iter().map(|o| o.mode).collect(),
            sheet: cutting_area,
            clearance_mm,
            cells: HashMap::new(),
            row_end: HashMap::new(),
            extent: None,
        };
        for (o, object) in objects.iter().enumerate() {
            for ring in &object.rings {
                let r = kept.ring_object.len();
                kept.ring_object.push(o);
                let n = ring.len();
                for i in 0..n {
                    let (a, b) = (ring[i], ring[(i + 1) % n]);
                    if a != b && a.is_finite() && b.is_finite() {
                        kept.insert(a, b, r);
                    }
                }
            }
        }
        kept
    }

    fn insert(&mut self, a: Point, b: Point, ring: usize) {
        let id = self.segs.len();
        self.segs.push((a, b, ring));
        let pieces = (a.distance(b) / PIECE_MM).ceil().max(1.0) as usize;
        for k in 0..pieces {
            let p0 = a.lerp(b, k as f64 / pieces as f64);
            let p1 = a.lerp(b, (k + 1) as f64 / pieces as f64);
            let r = Rect::from_points(p0, p1).inflate(EPS, EPS);
            for row in cell(r.y0)..=cell(r.y1) {
                for col in cell(r.x0)..=cell(r.x1) {
                    let ids = self.cells.entry((col, row)).or_default();
                    // One segment is indexed before the next, so a repeat
                    // can only be the last entry.
                    if ids.last() != Some(&id) {
                        ids.push(id);
                    }
                    let end = self.row_end.entry(row).or_insert(col);
                    *end = (*end).max(col);
                    self.extent = Some(match self.extent {
                        None => (col, row, col, row),
                        Some((c0, r0, c1, r1)) => {
                            (c0.min(col), r0.min(row), c1.max(col), r1.max(row))
                        }
                    });
                }
            }
        }
    }

    /// The objects inside an odd number of whose rings `p` lies strictly.
    /// A ring that `p` lies on (within [`EPS`]) does not count, so the
    /// ring's own object counts only its other rings.
    fn odd_objects(&self, p: Point) -> Vec<usize> {
        let (pc, pr) = (cell(p.x), cell(p.y));
        let Some(&end) = self.row_end.get(&pr) else {
            return Vec::new();
        };
        // An edge within EPS of p is indexed in p's own cell.
        let on: Vec<usize> = self
            .cells
            .get(&(pc, pr))
            .into_iter()
            .flatten()
            .map(|&id| self.segs[id])
            .filter(|&(a, b, _)| kurbo::Line::new(a, b).nearest(p, 0.0).distance_sq <= EPS * EPS)
            .map(|s| s.2)
            .collect();
        let mut objects = Vec::new();
        // Walk p's row towards +x. Each crossing is counted in the one cell
        // that holds the crossing point.
        for col in pc..=end {
            for &id in self.cells.get(&(col, pr)).into_iter().flatten() {
                let (a, b, ring) = self.segs[id];
                if let Some(x) = crosses_ray(a, b, p)
                    && cell(x) == col
                    && !on.contains(&ring)
                {
                    objects.push(self.ring_object[ring]);
                }
            }
        }
        objects.sort_unstable();
        let mut odd = Vec::new();
        for run in objects.chunk_by(|a, b| a == b) {
            if run.len() % 2 == 1 {
                odd.push(run[0]);
            }
        }
        odd
    }

    /// True when `p` is kept material: some object calls it kept.
    pub fn is_kept(&self, p: Point) -> bool {
        let odd = self.odd_objects(p);
        if odd.iter().any(|&o| self.modes[o] != KeepMode::Stencil) {
            return true;
        }
        let stencils = self
            .modes
            .iter()
            .filter(|m| **m == KeepMode::Stencil)
            .count();
        let on_sheet = p.x >= self.sheet.x0
            && p.x <= self.sheet.x1
            && p.y >= self.sheet.y0
            && p.y <= self.sheet.y1;
        // `odd` holds only stencils here; a stencil not in it keeps p.
        if on_sheet && odd.len() < stencils {
            return true;
        }
        self.modes.contains(&KeepMode::Both)
            && self.nearest(p, |o| self.modes[o] == KeepMode::Both, self.clearance_mm)
                <= self.clearance_mm
    }

    /// The distance from `p` to the nearest ring, `f64::INFINITY` when
    /// there is none.
    pub fn distance_to_cut(&self, p: Point) -> f64 {
        self.nearest(p, |_| true, f64::INFINITY)
    }

    /// The distance from `p` to the nearest ring of an object that `take`
    /// accepts, `f64::INFINITY` when there is none. The search may stop
    /// early with any value above `limit` once no ring can be that near.
    fn nearest(&self, p: Point, take: impl Fn(usize) -> bool, limit: f64) -> f64 {
        let Some((c0, r0, c1, r1)) = self.extent else {
            return f64::INFINITY;
        };
        let (pc, pr) = (cell(p.x), cell(p.y));
        // Cells nearer to p's cell than the extent hold no edges.
        let mut k = (c0 - pc).max(pc - c1).max(r0 - pr).max(pr - r1).max(0);
        let last = (pc - c0).max(c1 - pc).max(pr - r0).max(r1 - pr);
        let mut best = f64::INFINITY;
        loop {
            for row in (pr - k).max(r0)..=(pr + k).min(r1) {
                let edge_row = row == pr - k || row == pr + k;
                let cols: Vec<i64> = if edge_row {
                    ((pc - k).max(c0)..=(pc + k).min(c1)).collect()
                } else {
                    [pc - k, pc + k]
                        .into_iter()
                        .filter(|c| (c0..=c1).contains(c))
                        .collect()
                };
                for col in cols {
                    for &id in self.cells.get(&(col, row)).into_iter().flatten() {
                        let (a, b, ring) = self.segs[id];
                        if take(self.ring_object[ring]) {
                            let d = kurbo::Line::new(a, b).nearest(p, 0.0).distance_sq.sqrt();
                            best = best.min(d);
                        }
                    }
                }
            }
            // An edge first met beyond ring k of cells is at least k cells away.
            if best <= k as f64 * CELL_MM || k as f64 * CELL_MM > limit || k >= last {
                return best;
            }
            k += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{PathRef, Toolpath};
    use flatcraft_core::document::ObjectId;
    use kurbo::Rect;

    /// A toolpath of the given points (Ruling F3: rings are read through `lap()`).
    fn tp(pts: &[(f64, f64)], closed: bool) -> Toolpath {
        Toolpath::new(
            pts.iter().map(|&p| p.into()).collect(),
            closed,
            PathRef {
                object: ObjectId(1),
                path_index: 0,
            },
        )
    }

    fn poly(t: &Toolpath) -> Polyline {
        Polyline {
            points: t.lap().to_vec(),
            closed: t.closed,
        }
    }

    /// A closed axis-aligned square ring from `(x0, y0)` to `(x1, y1)`.
    fn sq(x0: f64, y0: f64, x1: f64, y1: f64) -> Toolpath {
        tp(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)], true)
    }

    /// A circle of `n` vertices, closed by repeating the first.
    fn circle(c: (f64, f64), r: f64, n: usize) -> Toolpath {
        let mut pts: Vec<(f64, f64)> = (0..n)
            .map(|k| {
                let a = k as f64 / n as f64 * std::f64::consts::TAU;
                (c.0 + r * a.cos(), c.1 + r * a.sin())
            })
            .collect();
        pts.push(pts[0]);
        tp(&pts, true)
    }

    fn tree_of(paths: &[Toolpath]) -> ContainmentTree {
        build(&paths.iter().map(poly).collect::<Vec<_>>())
    }

    fn sides(tree: &ContainmentTree, paths: &[Toolpath]) -> Vec<KeepSide> {
        paths
            .iter()
            .enumerate()
            .map(|(i, t)| keep_side(tree, i, t.closed))
            .collect()
    }

    fn even_odd_calls() -> usize {
        INSIDE_CALLS.with(|c| c.get())
    }

    fn reset_even_odd_calls() {
        INSIDE_CALLS.with(|c| c.set(0));
    }

    #[test]
    fn signed_area_sign_in_mat_coordinates() {
        let t = tp(
            &[
                (0.0, 0.0),
                (0.0, 10.0),
                (10.0, 10.0),
                (10.0, 0.0),
                (0.0, 0.0),
            ],
            true,
        );
        assert_eq!(signed_area(t.lap()), -100.0);
        let mut rev = t.lap().to_vec();
        rev.reverse();
        assert_eq!(signed_area(&rev), 100.0);
    }

    #[test]
    fn point_on_edge_is_none() {
        let t = sq(0.0, 0.0, 10.0, 10.0);
        assert_eq!(point_in_ring(Point::new(5.0, 0.0), t.lap()), None);
        assert_eq!(point_in_ring(Point::new(10.0, 10.0), t.lap()), None);
        assert_eq!(point_in_ring(Point::new(0.0, 7.5), t.lap()), None);
    }

    #[test]
    fn point_inside_and_outside() {
        let t = sq(0.0, 0.0, 10.0, 10.0);
        assert_eq!(point_in_ring(Point::new(5.0, 5.0), t.lap()), Some(true));
        assert_eq!(point_in_ring(Point::new(15.0, 5.0), t.lap()), Some(false));
        assert_eq!(point_in_ring(Point::new(-1.0, 5.0), t.lap()), Some(false));
        assert_eq!(point_in_ring(Point::new(5.0, 11.0), t.lap()), Some(false));
    }

    #[test]
    fn hole_in_square_is_child() {
        let paths = [sq(0.0, 0.0, 50.0, 50.0), sq(20.0, 20.0, 30.0, 30.0)];
        let tree = tree_of(&paths);
        assert_eq!(tree.depth, vec![0, 1]);
        assert_eq!(tree.parent, vec![None, Some(0)]);
        assert_eq!(tree.children, vec![vec![1], vec![]]);
        assert_eq!(tree.roots, vec![0]);
        assert_eq!(
            sides(&tree, &paths),
            vec![KeepSide::Inside, KeepSide::Outside]
        );
    }

    #[test]
    fn piece_inside_hole_is_kept_inside() {
        // Listed out of nesting order on purpose.
        let paths = [
            sq(20.0, 20.0, 30.0, 30.0),
            sq(0.0, 0.0, 50.0, 50.0),
            sq(10.0, 10.0, 40.0, 40.0),
        ];
        let tree = tree_of(&paths);
        assert_eq!(tree.parent, vec![Some(2), None, Some(1)]);
        assert_eq!(tree.depth, vec![2, 0, 1]);
        assert_eq!(
            sides(&tree, &paths),
            vec![KeepSide::Inside, KeepSide::Inside, KeepSide::Outside]
        );
    }

    #[test]
    fn partially_overlapping_rings_do_not_nest() {
        let paths = [sq(0.0, 0.0, 20.0, 20.0), sq(10.0, 0.0, 30.0, 20.0)];
        let tree = tree_of(&paths);
        assert_eq!(tree.parent, vec![None, None]);
        assert_eq!(tree.roots, vec![0, 1]);
        assert_eq!(tree.depth, vec![0, 0]);
    }

    #[test]
    fn open_line_attaches_to_smallest_container() {
        let paths = [
            sq(0.0, 0.0, 100.0, 100.0),
            sq(10.0, 10.0, 60.0, 60.0),
            // Inside both squares; one end on the inner square's edge.
            tp(&[(20.0, 20.0), (30.0, 30.0), (60.0, 40.0)], false),
            // Leaves the inner square: only the outer one holds it.
            tp(&[(20.0, 20.0), (80.0, 80.0)], false),
            // Outside everything.
            tp(&[(200.0, 0.0), (210.0, 0.0)], false),
        ];
        let tree = tree_of(&paths);
        assert_eq!(tree.parent, vec![None, Some(0), Some(1), Some(0), None]);
        assert_eq!(tree.depth, vec![0, 1, 2, 1, 0]);
        assert_eq!(tree.children[0], vec![1, 3]);
        assert_eq!(tree.children[1], vec![2]);
        assert_eq!(tree.roots, vec![0, 4]);
        let s = sides(&tree, &paths);
        assert_eq!(s[2], KeepSide::Both);
        assert_eq!(s[3], KeepSide::Both);
        assert_eq!(s[4], KeepSide::Both);
    }

    #[test]
    fn touching_rings_are_siblings() {
        // Two squares sharing an edge, and a square touching the inner
        // corner of an L from outside while lying in the L's box.
        let l = tp(
            &[
                (0.0, 0.0),
                (100.0, 0.0),
                (100.0, 20.0),
                (20.0, 20.0),
                (20.0, 100.0),
                (0.0, 100.0),
                (0.0, 0.0),
            ],
            true,
        );
        let paths = [
            sq(200.0, 0.0, 210.0, 10.0),
            sq(210.0, 0.0, 220.0, 10.0),
            l,
            sq(20.0, 20.0, 40.0, 40.0),
        ];
        let tree = tree_of(&paths);
        assert_eq!(tree.parent, vec![None; 4]);
        assert_eq!(tree.depth, vec![0; 4]);
        assert_eq!(tree.roots, vec![0, 1, 2, 3]);
    }

    #[test]
    fn a_ring_touching_the_outline_from_inside_is_a_child() {
        // Ported from order.rs `a_ring_touching_the_outline_still_counts_as_inside`.
        // The inner ring's first vertex lies on the outline's left edge, so
        // the probe skips it and tries a vertex that is clearly inside.
        let paths = [
            sq(10.0, 10.0, 60.0, 60.0),
            tp(
                &[(10.0, 30.0), (30.0, 25.0), (30.0, 35.0), (10.0, 30.0)],
                true,
            ),
        ];
        let tree = tree_of(&paths);
        assert_eq!(tree.parent, vec![None, Some(0)]);
        assert_eq!(
            sides(&tree, &paths),
            vec![KeepSide::Inside, KeepSide::Outside]
        );
    }

    #[test]
    fn a_ring_beside_another_in_its_box_does_not_nest() {
        // Ported from order.rs `a_ring_beside_another_in_its_box_does_not_block`.
        let l = tp(
            &[
                (0.0, 0.0),
                (100.0, 0.0),
                (100.0, 20.0),
                (20.0, 20.0),
                (20.0, 100.0),
                (0.0, 100.0),
                (0.0, 0.0),
            ],
            true,
        );
        let paths = [l, sq(50.0, 50.0, 70.0, 70.0)];
        let tree = tree_of(&paths);
        assert_eq!(tree.parent, vec![None, None]);
        assert_eq!(tree.roots, vec![0, 1]);
    }

    #[test]
    fn identical_rings_are_siblings() {
        // Ported from order.rs `identical_rings_fall_back_to_the_nearest`.
        let paths = [sq(10.0, 10.0, 60.0, 60.0), sq(10.0, 10.0, 60.0, 60.0)];
        let tree = tree_of(&paths);
        assert_eq!(tree.parent, vec![None, None]);
        assert_eq!(tree.roots, vec![0, 1]);
        assert_eq!(
            sides(&tree, &paths),
            vec![KeepSide::Inside, KeepSide::Inside]
        );
    }

    #[test]
    fn deep_nesting_builds_fast() {
        // Ported from order.rs `deep_nesting_orders_fast`: 200 concentric
        // rings of 400 vertices. The search goes from the smallest
        // container upward and stops at the first, so each ring costs at
        // most PROBES even-odd tests. The count is the check, not the clock.
        let paths: Vec<Toolpath> = (1..=200)
            .map(|k| circle((300.0, 300.0), k as f64, 400))
            .collect();
        let polys: Vec<Polyline> = paths.iter().map(poly).collect();
        reset_even_odd_calls();
        let tree = build(&polys);
        let calls = even_odd_calls();
        assert!(calls <= PROBES * 199, "{calls} even-odd tests");
        let want: Vec<usize> = (0..200).map(|k| 199 - k).collect();
        assert_eq!(tree.depth, want);
        assert_eq!(tree.roots, vec![199]);
    }

    #[test]
    fn containment_of_2000_rings_is_bounded() {
        // Ruling F4: 2000 concentric rings plus 2000 disjoint ones, outside
        // the concentric rings' box. Ruling F30: the guard is the number
        // of even-odd tests, never a wall-clock limit.
        let mut paths: Vec<Toolpath> = (1..=2000)
            .map(|k| circle((0.0, 0.0), k as f64 * 0.1, 64))
            .collect();
        paths.extend((0..2000).map(|k| {
            let x0 = 300.0 + (k % 50) as f64 * 3.0;
            let y0 = (k / 50) as f64 * 3.0;
            sq(x0, y0, x0 + 2.0, y0 + 2.0)
        }));
        let polys: Vec<Polyline> = paths.iter().map(poly).collect();
        reset_even_odd_calls();
        let tree = build(&polys);
        let calls = even_odd_calls();
        assert!(calls <= PROBES * polys.len(), "{calls} even-odd tests");
        for k in 0..2000 {
            assert_eq!(tree.depth[k], 1999 - k, "concentric ring {k}");
        }
        assert!(tree.depth[2000..].iter().all(|&d| d == 0));
        assert_eq!(tree.roots.len(), 2001);
    }

    #[test]
    fn kept_material_counts_rings() {
        let outer = sq(0.0, 0.0, 50.0, 50.0);
        let hole = sq(20.0, 20.0, 30.0, 30.0);
        let neighbour = sq(60.0, 0.0, 80.0, 20.0);
        let kept = KeptMaterial::new(&[outer.lap(), hole.lap(), neighbour.lap()]);
        assert!(kept.is_kept(Point::new(10.0, 10.0)), "band");
        assert!(!kept.is_kept(Point::new(25.0, 25.0)), "hole");
        assert!(!kept.is_kept(Point::new(55.0, 25.0)), "outside");
        assert!(!kept.is_kept(Point::new(-5.0, -5.0)), "outside");
        assert!(kept.is_kept(Point::new(70.0, 10.0)), "neighbour");
    }

    #[test]
    fn piece_in_hole_is_kept_material() {
        let outer = sq(0.0, 0.0, 50.0, 50.0);
        let hole = sq(10.0, 10.0, 40.0, 40.0);
        let piece = sq(20.0, 20.0, 30.0, 30.0);
        let kept = KeptMaterial::new(&[outer.lap(), hole.lap(), piece.lap()]);
        assert!(kept.is_kept(Point::new(5.0, 5.0)));
        assert!(!kept.is_kept(Point::new(15.0, 15.0)));
        assert!(kept.is_kept(Point::new(25.0, 25.0)));
    }

    #[test]
    fn a_point_on_a_cut_is_not_strictly_inside_it() {
        let outer = sq(0.0, 0.0, 50.0, 50.0);
        let hole = sq(20.0, 20.0, 30.0, 30.0);
        let kept = KeptMaterial::new(&[outer.lap(), hole.lap()]);
        // On the outline: inside nothing.
        assert!(!kept.is_kept(Point::new(0.0, 25.0)));
        // On the hole's edge: inside the outline only.
        assert!(kept.is_kept(Point::new(20.0, 25.0)));
        // A ray along an edge of the hole must not count it.
        assert!(kept.is_kept(Point::new(10.0, 20.0)));
        assert!(!kept.is_kept(Point::new(60.0, 20.0)));
        // Inside the hole within EPS of its right edge, which lies on a
        // grid line, so p falls in the cell left of the edge's own cell.
        // The edge is still found: p is on the hole, inside the outline.
        assert!(kept.is_kept(Point::new(30.0 - 0.5e-9, 25.0)));
        assert!(kept.is_kept(Point::new(25.0, 30.0 - 0.5e-9)));
    }

    #[test]
    fn kept_material_with_many_laps_reads_the_first_lap() {
        // Ruling F3: a ring traced twice would cancel itself under
        // even-odd. `lap()` gives one lap.
        let mut t = sq(0.0, 0.0, 10.0, 10.0);
        let ring = t.points.clone();
        t.points.extend_from_slice(&ring[1..]);
        t.laps = 2;
        let kept = KeptMaterial::new(&[t.lap()]);
        assert!(kept.is_kept(Point::new(5.0, 5.0)));
    }

    #[test]
    fn distance_to_cut_is_the_nearest_ring() {
        let outer = sq(0.0, 0.0, 50.0, 50.0);
        let hole = sq(20.0, 20.0, 30.0, 30.0);
        let kept = KeptMaterial::new(&[outer.lap(), hole.lap()]);
        let close = |p: (f64, f64), d: f64| {
            let got = kept.distance_to_cut(p.into());
            assert!((got - d).abs() < 1e-9, "{p:?}: {got} != {d}");
        };
        close((25.0, 25.0), 5.0);
        close((10.0, 25.0), 10.0);
        close((15.0, 25.0), 5.0);
        close((25.0, 0.0), 0.0);
        close((-3.0, 25.0), 3.0);
        // Far away and off the grid.
        close((-300.0, 25.0), 300.0);
        close((53.0, 54.0), 5.0);
        let none = KeptMaterial::new(&[]);
        assert_eq!(none.distance_to_cut(Point::ZERO), f64::INFINITY);
        assert!(!none.is_kept(Point::ZERO));
    }

    #[test]
    fn distance_to_cut_matches_brute_force_on_a_circle() {
        let c = circle((40.0, 40.0), 17.3, 97);
        let kept = KeptMaterial::new(&[c.lap()]);
        let ring = c.lap();
        for i in 0..40 {
            for j in 0..40 {
                let p = Point::new(i as f64 * 2.1 - 2.0, j as f64 * 2.1 - 2.0);
                let brute = ring
                    .windows(2)
                    .map(|w| {
                        kurbo::Line::new(w[0], w[1])
                            .nearest(p, 0.0)
                            .distance_sq
                            .sqrt()
                    })
                    .fold(f64::INFINITY, f64::min);
                let got = kept.distance_to_cut(p);
                assert!((got - brute).abs() < 1e-9, "{p:?}: {got} != {brute}");
                let inside = point_in_ring(p, ring) == Some(true);
                assert_eq!(kept.is_kept(p), inside, "{p:?}");
            }
        }
    }
    fn area() -> Rect {
        Rect::new(0.0, 0.0, 200.0, 200.0)
    }

    #[test]
    fn stencil_keeps_the_sheet_and_wastes_the_shape() {
        let r = sq(50.0, 50.0, 80.0, 70.0);
        let kept = KeptMaterial::with_modes(
            &[KeptObject {
                rings: vec![r.lap()],
                mode: KeepMode::Stencil,
            }],
            area(),
            1.0,
        );
        assert!(!kept.is_kept(Point::new(60.0, 60.0)), "cut shape is waste");
        assert!(kept.is_kept(Point::new(40.0, 60.0)), "sheet beside it");
        assert!(kept.is_kept(Point::new(150.0, 150.0)), "sheet far away");
        assert!(
            !kept.is_kept(Point::new(250.0, 60.0)),
            "off the cutting area"
        );
    }

    #[test]
    fn stencil_with_a_piece_in_a_hole_wastes_only_the_odd_regions() {
        // A letter O: the ring between the two outlines falls out, the
        // counter (the middle) stays with the sheet.
        let outer = sq(50.0, 50.0, 90.0, 90.0);
        let inner = sq(60.0, 60.0, 80.0, 80.0);
        let kept = KeptMaterial::with_modes(
            &[KeptObject {
                rings: vec![outer.lap(), inner.lap()],
                mode: KeepMode::Stencil,
            }],
            area(),
            1.0,
        );
        assert!(!kept.is_kept(Point::new(55.0, 70.0)));
        assert!(kept.is_kept(Point::new(70.0, 70.0)));
    }

    #[test]
    fn both_leaves_no_waste_near_the_rings() {
        let outer = sq(50.0, 50.0, 90.0, 90.0);
        let hole = sq(60.0, 60.0, 80.0, 80.0);
        let kept = KeptMaterial::with_modes(
            &[KeptObject {
                rings: vec![outer.lap(), hole.lap()],
                mode: KeepMode::Both,
            }],
            area(),
            2.0,
        );
        assert!(kept.is_kept(Point::new(55.0, 70.0)), "the piece");
        assert!(
            kept.is_kept(Point::new(48.5, 70.0)),
            "sheet within clearance"
        );
        assert!(
            kept.is_kept(Point::new(61.5, 70.0)),
            "hole within clearance"
        );
        assert!(kept.is_kept(Point::new(50.0, 70.0)), "on the cut");
        assert!(
            !kept.is_kept(Point::new(40.0, 70.0)),
            "sheet beyond clearance"
        );
        assert!(
            !kept.is_kept(Point::new(70.0, 70.0)),
            "hole beyond clearance"
        );
    }

    #[test]
    fn overlapping_pieces_and_stencil_keep_wins() {
        let piece = sq(0.0, 0.0, 20.0, 20.0);
        let stencil = sq(10.0, 0.0, 30.0, 20.0);
        let kept = KeptMaterial::with_modes(
            &[
                KeptObject {
                    rings: vec![piece.lap()],
                    mode: KeepMode::Pieces,
                },
                KeptObject {
                    rings: vec![stencil.lap()],
                    mode: KeepMode::Stencil,
                },
            ],
            area(),
            1.0,
        );
        assert!(
            kept.is_kept(Point::new(15.0, 10.0)),
            "piece wins over stencil cut-out"
        );
        assert!(
            kept.is_kept(Point::new(5.0, 10.0)),
            "piece and stencil sheet"
        );
        assert!(
            !kept.is_kept(Point::new(25.0, 10.0)),
            "stencil cut-out only"
        );
        assert!(kept.is_kept(Point::new(100.0, 100.0)), "stencil sheet");
    }

    #[test]
    fn overlapping_pieces_of_two_objects_stay_kept() {
        // Job-wide even-odd would call the overlap waste (count 2). Per
        // object, each piece keeps it.
        let a = sq(0.0, 0.0, 20.0, 20.0);
        let b = sq(10.0, 0.0, 30.0, 20.0);
        let per_object = KeptMaterial::with_modes(
            &[
                KeptObject {
                    rings: vec![a.lap()],
                    mode: KeepMode::Pieces,
                },
                KeptObject {
                    rings: vec![b.lap()],
                    mode: KeepMode::Pieces,
                },
            ],
            area(),
            1.0,
        );
        assert!(per_object.is_kept(Point::new(15.0, 10.0)));
        assert!(
            !per_object.is_kept(Point::new(50.0, 10.0)),
            "no stencil: outside is waste"
        );
        assert_eq!(KeepMode::default(), KeepMode::Pieces);
    }
}
