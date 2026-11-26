//! Containment: which path lies in which ring, the side of each ring that
//! is kept, and the job-wide classification of kept material versus waste
//! (spec 8.3).
//!
//! Rings are read as one lap (Ruling F3): pass a toolpath's `lap()`, never
//! its `points`, because a ring traced twice cancels itself under even-odd.

use std::collections::HashMap;

use kurbo::{ParamCurveNearest as _, Point, Rect};

use flatcraft_core::document::ObjectId;

use crate::plan::{KeepSide, Toolpath, ToolpathKind};

/// Points closer than this to a ring count as on it.
pub const EPS: f64 = 1e-9;

/// How many vertices of an inner ring are tried before giving up.
pub const PROBES: usize = 3;

#[cfg(test)]
thread_local! {
    /// Even-odd tests run on this thread (`point_in_ring`).
    pub(crate) static INSIDE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Grid cells `is_kept` walked on this thread.
    pub(crate) static CELL_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Container candidates the parent search looked at on this thread.
    pub(crate) static PARENT_CANDIDATES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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

/// Where the row at height `y` meets the points within `c` of segment
/// `a`-`b` (a capsule, which is convex): `(from, to)`, `None` when it
/// misses.
fn capsule_span(a: Point, b: Point, y: f64, c: f64) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut take = |x: f64| {
        lo = lo.min(x);
        hi = hi.max(x);
    };
    for e in [a, b] {
        let dy = y - e.y;
        if dy.abs() <= c {
            let s = (c * c - dy * dy).sqrt();
            take(e.x - s);
            take(e.x + s);
        }
    }
    let v = b - a;
    let len = v.hypot();
    if len > 0.0 {
        let n = kurbo::Vec2::new(-v.y, v.x) * (c / len);
        let q = [a + n, b + n, b - n, a - n];
        for i in 0..4 {
            let (p, r) = (q[i], q[(i + 1) % 4]);
            if (p.y - y) * (r.y - y) <= 0.0 {
                if p.y == r.y {
                    take(p.x);
                    take(r.x);
                } else {
                    take(p.x + (y - p.y) / (r.y - p.y) * (r.x - p.x));
                }
            }
        }
    }
    (lo <= hi).then_some((lo, hi))
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
/// is not on the outer boundary is inside. When all of them lie on the
/// boundary (a diamond inscribed in a square), the midpoints of the first
/// [`PROBES`] edges are tried the same way. Partial overlaps fail the box
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
        .or_else(|| {
            inner
                .windows(2)
                .take(PROBES)
                .find_map(|w| point_in_ring(w[0].midpoint(w[1]), outer))
        })
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
    /// The number of closed paths of the same object around each path,
    /// counted in a tree of that object's paths alone.
    pub own_depth: Vec<usize>,
}

/// Build the containment tree of `paths` (spec 8.3). Rings are read
/// through `Toolpath::lap()` (Ruling F3) and grouped by
/// `source.object` for [`ContainmentTree::own_depth`].
///
/// - A closed path B lies in a closed path A by [`nested`].
/// - An open path lies in a closed path when its box lies in the ring's
///   box and none of its vertices is outside (the boundary counts as
///   inside).
/// - The parent is the smallest container. Candidates are searched from
///   the smallest larger ring upward and the first hit wins, so deep
///   nesting costs about one nesting test per ring: [`PROBES`] even-odd
///   tests, or up to 2 x [`PROBES`] when every probe vertex lies on the
///   boundary and the edge midpoints are tried too.
/// - `own_depth` repeats the search within each object when the job has
///   more than one, which at most doubles that cost.
pub fn build(paths: &[Toolpath]) -> ContainmentTree {
    let n = paths.len();
    // (index, box, absolute area) of every closed path, smallest first.
    let mut rings: Vec<Ring> = paths
        .iter()
        .enumerate()
        .filter(|(_, p)| p.closed)
        .filter_map(|(i, p)| Some((i, bbox(p.lap())?, signed_area(p.lap()).abs())))
        .collect();
    rings.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)));
    let open: Vec<usize> = (0..n).filter(|&j| !paths[j].closed).collect();

    let mut parent = vec![None; n];
    find_parents(paths, &rings, &open, &mut parent);

    // The same search within each object, for `own_depth`: a ring of the
    // path's object can hold it without being its ancestor, when another
    // object's ring lies in between in part.
    let mut own_parent = parent.clone();
    let first = paths.first().map(|p| p.source.object);
    if paths.iter().any(|p| Some(p.source.object) != first) {
        let mut groups: HashMap<ObjectId, (Vec<Ring>, Vec<usize>)> = HashMap::new();
        for r in &rings {
            groups
                .entry(paths[r.0].source.object)
                .or_default()
                .0
                .push(*r);
        }
        for &j in &open {
            groups.entry(paths[j].source.object).or_default().1.push(j);
        }
        for (group_rings, group_open) in groups.values() {
            find_parents(paths, group_rings, group_open, &mut own_parent);
        }
    }

    // A parent is a larger closed ring, so going from the largest down
    // sets every parent's depth before its children's.
    let mut depth = vec![0; n];
    let mut own_depth = vec![0; n];
    for j in rings.iter().rev().map(|r| r.0).chain(open) {
        if let Some(p) = parent[j] {
            depth[j] = depth[p] + 1;
        }
        if let Some(p) = own_parent[j] {
            own_depth[j] = own_depth[p] + 1;
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
        own_depth,
    }
}

/// (path index, box, absolute area) of a closed path.
type Ring = (usize, Rect, f64);

/// Set the parent of every ring in `rings` (smallest first) and every path
/// in `open` to its smallest container among `rings`. Each ring tries the
/// larger rings from the smallest upward and stops at the first hit.
///
/// A container's box holds the inner path's box, so it holds its top-left
/// corner: only the rings whose boxes cover that corner's cell in a
/// [`BoxIndex`] are tried, in the same order. The answer is the same as
/// trying every larger ring, without the n^2 box tests of a job with many
/// small paths.
fn find_parents(paths: &[Toolpath], rings: &[Ring], open: &[usize], parent: &mut [Option<usize>]) {
    let index = BoxIndex::new(rings);
    let all: Vec<u32> = (0..rings.len() as u32).collect();
    let candidates = |b: Rect| -> &[u32] {
        index
            .as_ref()
            .map_or(&all[..], |index| index.at(Point::new(b.x0, b.y0)))
    };
    let count = |_n: usize| {
        #[cfg(test)]
        PARENT_CANDIDATES.with(|c| c.set(c.get() + _n));
    };
    for &(j, jb, ja) in rings {
        let ranks = candidates(jb);
        // Ranks follow the area, so the larger rings are a suffix.
        let larger = &ranks[ranks.partition_point(|&r| rings[r as usize].2 <= ja)..];
        let mut tried = 0;
        parent[j] = larger
            .iter()
            .map(|&r| rings[r as usize])
            .find(|&(i, ib, ia)| {
                tried += 1;
                nested(paths[j].lap(), jb, ja, paths[i].lap(), ib, ia)
            })
            .map(|r| r.0);
        count(tried);
    }
    for &j in open {
        let Some(jb) = bbox(paths[j].lap()) else {
            continue;
        };
        let mut tried = 0;
        parent[j] = candidates(jb)
            .iter()
            .map(|&r| rings[r as usize])
            .find(|&(i, ib, ia)| {
                tried += 1;
                ia > 0.0
                    && box_in(jb, ib)
                    && paths[j]
                        .lap()
                        .iter()
                        .all(|q| point_in_ring(*q, paths[i].lap()) != Some(false))
            })
            .map(|r| r.0);
        count(tried);
    }
}

/// The boxes of rings sorted by area, in square cells: each cell lists the
/// ranks (positions in that order) of the rings whose boxes cover it, in
/// rank order.
struct BoxIndex {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<u32>>,
}

impl BoxIndex {
    /// The smallest cell edge, in millimetres.
    const MIN_CELL_MM: f64 = 1.0;
    /// The cells all boxes cover, at most this many per ring.
    const CELLS_PER_RING: f64 = 16.0;

    /// `None` when a box is not finite (the caller then tries every ring).
    /// The cell edge starts at the root of a quarter of the mean box area
    /// and doubles until the boxes cover at most [`Self::CELLS_PER_RING`]
    /// cells per ring, so one large outline cannot blow it up.
    fn new(rings: &[Ring]) -> Option<Self> {
        if rings.iter().any(|r| {
            let b = r.1;
            !(b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite())
        }) {
            return None;
        }
        let n = rings.len().max(1) as f64;
        let mean_area = rings.iter().map(|r| r.1.area()).sum::<f64>() / n;
        let mut cell = (mean_area / 4.0).sqrt().max(Self::MIN_CELL_MM);
        let span = |v0: f64, v1: f64, cell: f64| (v1 / cell).floor() - (v0 / cell).floor() + 1.0;
        while rings
            .iter()
            .map(|r| span(r.1.x0, r.1.x1, cell) * span(r.1.y0, r.1.y1, cell))
            .sum::<f64>()
            > Self::CELLS_PER_RING * n
        {
            cell *= 2.0;
        }
        let at = |v: f64| (v / cell).floor() as i64;
        let mut cells: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (rank, r) in rings.iter().enumerate() {
            for row in at(r.1.y0)..=at(r.1.y1) {
                for col in at(r.1.x0)..=at(r.1.x1) {
                    cells.entry((col, row)).or_default().push(rank as u32);
                }
            }
        }
        Some(Self { cell, cells })
    }

    /// The ranks of the rings whose boxes cover the cell of `p`.
    fn at(&self, p: Point) -> &[u32] {
        let key = (
            (p.x / self.cell).floor() as i64,
            (p.y / self.cell).floor() as i64,
        );
        self.cells.get(&key).map_or(&[], Vec::as_slice)
    }
}

/// The side of path `i` the kept material lies on, for a path of an
/// object in `mode`. Depth counts only the rings of the path's own object
/// ([`ContainmentTree::own_depth`]), so a piece lying on another object
/// stays a piece.
///
/// - [`KeepMode::Pieces`]: inside at even depth (a piece), outside at odd
///   depth (a hole).
/// - [`KeepMode::Stencil`]: the other way round.
/// - [`KeepMode::Both`] and open paths: both.
pub fn keep_side(tree: &ContainmentTree, i: usize, closed: bool, mode: KeepMode) -> KeepSide {
    if !closed || mode == KeepMode::Both {
        return KeepSide::Both;
    }
    let piece = tree.own_depth[i].is_multiple_of(2);
    if piece != (mode == KeepMode::Stencil) {
        KeepSide::Inside
    } else {
        KeepSide::Outside
    }
}

/// Which side of an object's cuts is kept material: the document's type.
pub use flatcraft_core::document::KeepMode;

/// The closed `Cut` rings of one object, one lap each, and its keep mode.
#[derive(Debug, Clone)]
struct KeptObject<'a> {
    rings: Vec<&'a [Point]>,
    mode: KeepMode,
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
/// wins: a point is waste only when no object calls it kept. The stencils
/// share one sheet: it is kept where no stencil cuts it out, so a shape
/// of one stencil is never kept as the sheet of another.
#[derive(Debug, Clone, PartialEq)]
pub struct KeptMaterial {
    /// Every ring edge as (start, end, ring index).
    segs: Vec<(Point, Point, usize)>,
    /// The object each ring belongs to.
    ring_object: Vec<usize>,
    /// The keep mode of each object.
    modes: Vec<KeepMode>,
    /// How many objects are [`KeepMode::Stencil`].
    stencils: usize,
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
    /// The kept material of a job: the closed `Cut` toolpaths grouped by
    /// source object, one lap each (Ruling F3) from pass 1 only, so passes
    /// and overcut never cancel a ring. Open paths and the alignment stroke
    /// add no rings. `mode_of` gives each object's [`KeepMode`],
    /// `cutting_area` is the sheet a [`KeepMode::Stencil`] keeps, and
    /// `clearance_mm` how far from a [`KeepMode::Both`] object's rings
    /// everything is kept.
    pub fn from_toolpaths(
        toolpaths: &[Toolpath],
        mode_of: impl Fn(ObjectId) -> KeepMode,
        cutting_area: Rect,
        clearance_mm: f64,
    ) -> Self {
        let mut index: HashMap<ObjectId, usize> = HashMap::new();
        let mut objects: Vec<KeptObject> = Vec::new();
        for t in toolpaths
            .iter()
            .filter(|t| t.closed && t.kind == ToolpathKind::Cut && t.pass == 1)
        {
            let o = t.source.object;
            let k = *index.entry(o).or_insert_with(|| {
                objects.push(KeptObject {
                    rings: Vec::new(),
                    mode: mode_of(o),
                });
                objects.len() - 1
            });
            objects[k].rings.push(t.lap());
        }
        Self::with_modes(&objects, cutting_area, clearance_mm)
    }

    /// One object of `rings` in `mode`, for tests.
    #[cfg(test)]
    fn single_object(rings: &[&[Point]], mode: KeepMode) -> Self {
        Self::with_modes(
            &[KeptObject {
                rings: rings.to_vec(),
                mode,
            }],
            Rect::new(-1000.0, -1000.0, 1000.0, 1000.0),
            0.0,
        )
    }

    /// True when no ring lies within `clearance_mm` of `p` (the distance
    /// is strictly larger). The search stops at the clearance. A
    /// non-finite `p` is never clear.
    pub fn is_clear(&self, p: Point, clearance_mm: f64) -> bool {
        p.is_finite() && self.nearest(p, |_| true, clearance_mm) > clearance_mm
    }

    /// Index the closed `Cut` rings of each object, one lap each.
    /// `cutting_area` is the sheet a [`KeepMode::Stencil`] keeps, and
    /// `clearance_mm` how far from a [`KeepMode::Both`] object's rings
    /// everything is kept.
    fn with_modes(objects: &[KeptObject], cutting_area: Rect, clearance_mm: f64) -> Self {
        let mut kept = Self {
            segs: Vec::new(),
            ring_object: Vec::new(),
            modes: objects.iter().map(|o| o.mode).collect(),
            stencils: objects
                .iter()
                .filter(|o| o.mode == KeepMode::Stencil)
                .count(),
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
        let (Some(&end), Some((c0, ..))) = (self.row_end.get(&pr), self.extent) else {
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
        // Cells left of the extent hold nothing: start at the extent.
        for col in pc.max(c0)..=end {
            #[cfg(test)]
            CELL_VISITS.with(|c| c.set(c.get() + 1));
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

    /// True when `p` lies strictly inside at least one ring, whatever its
    /// object and keep mode: a piece, or a hole (a letter's counter) of
    /// one. A point on a ring (within [`EPS`]) is not inside it. A
    /// non-finite `p` counts as inside.
    pub fn enclosed(&self, p: Point) -> bool {
        if !p.is_finite() {
            return true;
        }
        let (pc, pr) = (cell(p.x), cell(p.y));
        let (Some(&end), Some((c0, ..))) = (self.row_end.get(&pr), self.extent) else {
            return false;
        };
        // A ring p lies on does not count (as in `odd_objects`).
        let on: Vec<usize> = self
            .cells
            .get(&(pc, pr))
            .into_iter()
            .flatten()
            .map(|&id| self.segs[id])
            .filter(|&(a, b, _)| kurbo::Line::new(a, b).nearest(p, 0.0).distance_sq <= EPS * EPS)
            .map(|s| s.2)
            .collect();
        let mut rings = Vec::new();
        for col in pc.max(c0)..=end {
            for &id in self.cells.get(&(col, pr)).into_iter().flatten() {
                let (a, b, ring) = self.segs[id];
                if let Some(x) = crosses_ray(a, b, p)
                    && cell(x) == col
                    && !on.contains(&ring)
                {
                    rings.push(ring);
                }
            }
        }
        rings.sort_unstable();
        rings.chunk_by(|a, b| a == b).any(|run| run.len() % 2 == 1)
    }

    /// True when `p` is kept material: some object calls it kept.
    ///
    /// A point on a ring (within [`EPS`]) is not inside that ring. So a
    /// point on a [`KeepMode::Pieces`] outline counts as waste, and a point
    /// on a [`KeepMode::Stencil`] outline counts as kept (the sheet). This
    /// only says which side a point is on: a waste check must also keep
    /// off the cut itself with [`Self::distance_to_cut`] or
    /// [`Self::is_clear`]. A non-finite `p` is kept, so it is never
    /// judged safe.
    pub fn is_kept(&self, p: Point) -> bool {
        if !p.is_finite() {
            return true;
        }
        let odd = self.odd_objects(p);
        // `odd` holds the objects that call p inside: the stencils among
        // them call it waste, the rest kept.
        let stencils = odd
            .iter()
            .filter(|&&o| self.modes[o] == KeepMode::Stencil)
            .count();
        self.kept_by(odd.len() > stencils, stencils, p, || {
            self.nearest(p, |o| self.modes[o] == KeepMode::Both, self.clearance_mm)
                <= self.clearance_mm
        })
    }

    /// The rule of [`Self::is_kept`] for a point `p` inside an odd number
    /// of the rings of some object that is not a stencil (`odd_kept`) and
    /// of `odd_stencils` stencils. Kept wins across objects: a piece, the
    /// stencils' sheet where no stencil cuts `p` out, or the clearance of
    /// a [`KeepMode::Both`] object (`near_both`, asked last).
    fn kept_by(
        &self,
        odd_kept: bool,
        odd_stencils: usize,
        p: Point,
        near_both: impl FnOnce() -> bool,
    ) -> bool {
        if odd_kept {
            return true;
        }
        let on_sheet = p.x >= self.sheet.x0
            && p.x <= self.sheet.x1
            && p.y >= self.sheet.y0
            && p.y <= self.sheet.y1;
        if on_sheet && self.stencils > 0 && odd_stencils == 0 {
            return true;
        }
        self.modes.contains(&KeepMode::Both) && near_both()
    }

    /// The kept stretches of the row at height `y` between `x0` and `x1`,
    /// as `(from, to)` from left to right: where [`Self::is_kept`] says
    /// kept, exactly, up to the boundaries themselves (a ring, the sheet's
    /// edge, the clearance of a [`KeepMode::Both`] object). It asks the
    /// rule once per stretch, not once per point, so a canvas can draw the
    /// kept material of a whole job.
    pub fn kept_spans(&self, y: f64, x0: f64, x1: f64) -> Vec<(f64, f64)> {
        if !(y.is_finite() && x0.is_finite() && x1.is_finite()) || x1 <= x0 {
            return Vec::new();
        }
        // Every ring edge the row crosses, with its object, by the same
        // half-open rule as `crosses_ray`.
        let mut crossings: Vec<(f64, usize)> = Vec::new();
        let mut near: Vec<(f64, f64)> = Vec::new();
        if let Some((c0, r0, c1, r1)) = self.extent {
            let mut seen = std::collections::HashSet::new();
            let row = cell(y);
            for col in c0..=c1 {
                for &id in self.cells.get(&(col, row)).into_iter().flatten() {
                    let (a, b, ring) = self.segs[id];
                    if seen.insert(id) && (a.y > y) != (b.y > y) {
                        let x = a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x);
                        crossings.push((x, self.ring_object[ring]));
                    }
                }
            }
            // The clearance of the `Both` objects along the row. An edge
            // within it is indexed in a row of cells within it.
            let c = self.clearance_mm;
            if c > 0.0 && self.modes.contains(&KeepMode::Both) {
                seen.clear();
                for row in cell(y - c).max(r0)..=cell(y + c).min(r1) {
                    for col in c0..=c1 {
                        for &id in self.cells.get(&(col, row)).into_iter().flatten() {
                            let (a, b, ring) = self.segs[id];
                            if seen.insert(id)
                                && self.modes[self.ring_object[ring]] == KeepMode::Both
                                && let Some(span) = capsule_span(a, b, y, c)
                            {
                                near.push(span);
                            }
                        }
                    }
                }
            }
        }
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut cuts: Vec<f64> = crossings.iter().map(|c| c.0).collect();
        cuts.extend(near.iter().flat_map(|&(a, b)| [a, b]));
        cuts.extend([self.sheet.x0, self.sheet.x1, x0, x1]);
        cuts.retain(|&x| x >= x0 && x <= x1);
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        // Walk the stretches between the cuts, toggling the parity of each
        // object at its crossings.
        let mut parity = vec![false; self.modes.len()];
        let (mut odd_kept, mut odd_stencils) = (0usize, 0usize);
        let mut next = 0;
        let mut out: Vec<(f64, f64)> = Vec::new();
        for w in cuts.windows(2) {
            let (u, v) = (w[0], w[1]);
            let mid = (u + v) / 2.0;
            while next < crossings.len() && crossings[next].0 < mid {
                let o = crossings[next].1;
                parity[o] = !parity[o];
                let count = if self.modes[o] == KeepMode::Stencil {
                    &mut odd_stencils
                } else {
                    &mut odd_kept
                };
                if parity[o] {
                    *count += 1;
                } else {
                    *count -= 1;
                }
                next += 1;
            }
            let p = Point::new(mid, y);
            let kept = self.kept_by(odd_kept > 0, odd_stencils, p, || {
                near.iter().any(|&(a, b)| a <= mid && mid <= b)
            });
            if kept {
                match out.last_mut() {
                    Some(last) if last.1 == u => last.1 = v,
                    _ => out.push((u, v)),
                }
            }
        }
        out
    }

    /// The distance from `p` to the nearest ring, `f64::INFINITY` when
    /// there is none and 0 for a non-finite `p`.
    pub fn distance_to_cut(&self, p: Point) -> f64 {
        if !p.is_finite() {
            return 0.0;
        }
        self.nearest(p, |_| true, f64::INFINITY)
    }

    /// The distance from `p` to the nearest ring of an object that `take`
    /// accepts, `f64::INFINITY` when there is none. The search may stop
    /// early with any value above `limit` once no ring can be that near.
    fn nearest(&self, p: Point, take: impl Fn(usize) -> bool, limit: f64) -> f64 {
        let Some((c0, r0, c1, r1)) = self.extent else {
            return f64::INFINITY;
        };
        let dist = |id: usize| {
            let (a, b, ring) = self.segs[id];
            take(self.ring_object[ring])
                .then(|| kurbo::Line::new(a, b).nearest(p, 0.0).distance_sq.sqrt())
        };
        // The indexed area in millimetres, and how far p lies outside it.
        let (x0, y0) = (c0 as f64 * CELL_MM, r0 as f64 * CELL_MM);
        let (x1, y1) = ((c1 + 1) as f64 * CELL_MM, (r1 + 1) as f64 * CELL_MM);
        let off = (x0 - p.x)
            .max(p.x - x1)
            .max(0.0)
            .hypot((y0 - p.y).max(p.y - y1).max(0.0));
        if off > limit {
            return off;
        }
        if off > (x1 - x0).max(y1 - y0) {
            // Far off the grid the rings of cells would be mostly empty:
            // look at every edge once instead.
            return (0..self.segs.len())
                .filter_map(dist)
                .fold(f64::INFINITY, f64::min);
        }
        // From here p lies within a few extents of the grid, so cell
        // arithmetic cannot overflow.
        let (pc, pr) = (cell(p.x), cell(p.y));
        // Cells nearer to p's cell than the extent hold no edges.
        let mut k = (c0 - pc).max(pc - c1).max(r0 - pr).max(pr - r1).max(0);
        let last = (pc - c0).max(c1 - pc).max(pr - r0).max(r1 - pr);
        let mut best = f64::INFINITY;
        loop {
            for row in (pr - k).max(r0)..=(pr + k).min(r1) {
                let edge_row = row == pr - k || row == pr + k;
                let (lo, hi) = ((pc - k).max(c0), (pc + k).min(c1));
                // A middle row of ring k holds only its two side cells.
                // (A middle row exists only for k > 0.)
                let cols = if edge_row {
                    (lo..=hi).step_by(1)
                } else {
                    (pc - k..=pc + k).step_by((2 * k) as usize)
                }
                .filter(|c| (lo..=hi).contains(c));
                for col in cols {
                    for &id in self.cells.get(&(col, row)).into_iter().flatten() {
                        if let Some(d) = dist(id) {
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

    /// A toolpath of object 1 (Ruling F3: rings are read through `lap()`).
    fn tp(pts: &[(f64, f64)], closed: bool) -> Toolpath {
        tpo(1, pts, closed)
    }

    /// A toolpath of object `id`.
    fn tpo(id: u64, pts: &[(f64, f64)], closed: bool) -> Toolpath {
        Toolpath::new(
            pts.iter().map(|&p| p.into()).collect(),
            closed,
            PathRef {
                object: ObjectId(id),
                path_index: 0,
            },
        )
    }

    /// A square ring of object `id`.
    fn sqo(id: u64, x0: f64, y0: f64, x1: f64, y1: f64) -> Toolpath {
        let mut t = sq(x0, y0, x1, y1);
        t.source.object = ObjectId(id);
        t
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
        build(paths)
    }

    fn sides(tree: &ContainmentTree, paths: &[Toolpath]) -> Vec<KeepSide> {
        paths
            .iter()
            .enumerate()
            .map(|(i, t)| keep_side(tree, i, t.closed, KeepMode::Pieces))
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
        // most 2 x PROBES even-odd tests, and only PROBES here: no probe
        // vertex of a circle lies on the next circle, so the midpoint
        // fallback never runs. The count is the check, not the clock.
        let paths: Vec<Toolpath> = (1..=200)
            .map(|k| circle((300.0, 300.0), k as f64, 400))
            .collect();
        reset_even_odd_calls();
        let tree = build(&paths);
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
        // of even-odd tests, never a wall-clock limit. The worst case is
        // 2 x PROBES per ring (midpoint fallback); no probe vertex lies on
        // a boundary here, and one object needs no second search for
        // `own_depth`, so PROBES per ring holds.
        let mut paths: Vec<Toolpath> = (1..=2000)
            .map(|k| circle((0.0, 0.0), k as f64 * 0.1, 64))
            .collect();
        paths.extend((0..2000).map(|k| {
            let x0 = 300.0 + (k % 50) as f64 * 3.0;
            let y0 = (k / 50) as f64 * 3.0;
            sq(x0, y0, x0 + 2.0, y0 + 2.0)
        }));
        reset_even_odd_calls();
        let tree = build(&paths);
        let calls = even_odd_calls();
        assert!(calls <= PROBES * paths.len(), "{calls} even-odd tests");
        for k in 0..2000 {
            assert_eq!(tree.depth[k], 1999 - k, "concentric ring {k}");
        }
        assert!(tree.depth[2000..].iter().all(|&d| d == 0));
        assert_eq!(tree.roots.len(), 2001);
    }

    /// The parent by the rule alone: of the larger closed rings in (area,
    /// index) order, the first that holds path `j`; every ring is tried.
    fn parent_by_brute_force(paths: &[Toolpath], j: usize) -> Option<usize> {
        let mut rings: Vec<Ring> = paths
            .iter()
            .enumerate()
            .filter(|(_, p)| p.closed)
            .filter_map(|(i, p)| Some((i, bbox(p.lap())?, signed_area(p.lap()).abs())))
            .collect();
        rings.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)));
        let jb = bbox(paths[j].lap())?;
        if paths[j].closed {
            let ja = signed_area(paths[j].lap()).abs();
            rings
                .iter()
                .filter(|r| r.2 > ja)
                .find(|&&(i, ib, ia)| nested(paths[j].lap(), jb, ja, paths[i].lap(), ib, ia))
                .map(|r| r.0)
        } else {
            rings
                .iter()
                .find(|&&(i, ib, ia)| {
                    ia > 0.0
                        && box_in(jb, ib)
                        && paths[j]
                            .lap()
                            .iter()
                            .all(|q| point_in_ring(*q, paths[i].lap()) != Some(false))
                })
                .map(|r| r.0)
        }
    }

    proptest::proptest! {
        #[test]
        fn box_index_finds_the_brute_force_parent(
            shapes in proptest::collection::vec(
                (0.0f64..100.0, 0.0f64..100.0, 0.5f64..80.0, 0.5f64..80.0, proptest::bool::weighted(0.2)),
                1..60,
            ),
        ) {
            let paths: Vec<Toolpath> = shapes
                .iter()
                .map(|&(x, y, w, h, open)| {
                    if open {
                        tp(&[(x, y), (x + w / 3.0, y + h / 3.0)], false)
                    } else {
                        sq(x, y, x + w, y + h)
                    }
                })
                .collect();
            let tree = build(&paths);
            for j in 0..paths.len() {
                proptest::prop_assert_eq!(tree.parent[j], parent_by_brute_force(&paths, j), "path {}", j);
            }
        }
    }

    #[test]
    fn parent_search_of_many_small_rings_is_local() {
        // 20000 small squares of random sizes on a 600 x 900 mm sheet, and
        // one frame around them all. Trying every larger ring would look at
        // about n * n / 2 = 200 million candidates; the box index looks at
        // the few whose boxes cover each square's corner. The count is the
        // check, not the clock (Ruling F30).
        let mut seed = 7u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut paths: Vec<Toolpath> = (0..20_000)
            .map(|_| {
                let (x, y, side) = (
                    10.0 + next() * 580.0,
                    10.0 + next() * 880.0,
                    0.5 + next() * 2.0,
                );
                sq(x, y, x + side, y + side)
            })
            .collect();
        paths.push(sq(0.0, 0.0, 600.0, 900.0));
        PARENT_CANDIDATES.with(|c| c.set(0));
        let tree = build(&paths);
        let tried = PARENT_CANDIDATES.with(|c| c.get());
        eprintln!("{tried} candidates");
        assert!(tried <= 4 * paths.len(), "{tried} candidates");
        assert_eq!(tree.roots, vec![20_000]);
    }

    #[test]
    fn kept_material_counts_rings() {
        let outer = sq(0.0, 0.0, 50.0, 50.0);
        let hole = sq(20.0, 20.0, 30.0, 30.0);
        let neighbour = sq(60.0, 0.0, 80.0, 20.0);
        let kept = KeptMaterial::single_object(
            &[outer.lap(), hole.lap(), neighbour.lap()],
            KeepMode::Pieces,
        );
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
        let kept =
            KeptMaterial::single_object(&[outer.lap(), hole.lap(), piece.lap()], KeepMode::Pieces);
        assert!(kept.is_kept(Point::new(5.0, 5.0)));
        assert!(!kept.is_kept(Point::new(15.0, 15.0)));
        assert!(kept.is_kept(Point::new(25.0, 25.0)));
    }

    #[test]
    fn a_point_on_a_cut_is_not_strictly_inside_it() {
        let outer = sq(0.0, 0.0, 50.0, 50.0);
        let hole = sq(20.0, 20.0, 30.0, 30.0);
        let kept = KeptMaterial::single_object(&[outer.lap(), hole.lap()], KeepMode::Pieces);
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
        let kept = KeptMaterial::single_object(&[t.lap()], KeepMode::Pieces);
        assert!(kept.is_kept(Point::new(5.0, 5.0)));
    }

    #[test]
    fn distance_to_cut_is_the_nearest_ring() {
        let outer = sq(0.0, 0.0, 50.0, 50.0);
        let hole = sq(20.0, 20.0, 30.0, 30.0);
        let kept = KeptMaterial::single_object(&[outer.lap(), hole.lap()], KeepMode::Pieces);
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
        let none = KeptMaterial::single_object(&[], KeepMode::Pieces);
        assert_eq!(none.distance_to_cut(Point::ZERO), f64::INFINITY);
        assert!(!none.is_kept(Point::ZERO));
    }

    #[test]
    fn distance_to_cut_matches_brute_force_on_a_circle() {
        let c = circle((40.0, 40.0), 17.3, 97);
        let kept = KeptMaterial::single_object(&[c.lap()], KeepMode::Pieces);
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
    #[test]
    fn keep_side_follows_the_keep_mode() {
        let paths = [sq(0.0, 0.0, 50.0, 50.0), sq(20.0, 20.0, 30.0, 30.0)];
        let tree = tree_of(&paths);
        let side = |i: usize, mode| keep_side(&tree, i, true, mode);
        assert_eq!(side(0, KeepMode::Pieces), KeepSide::Inside);
        assert_eq!(side(1, KeepMode::Pieces), KeepSide::Outside, "hole");
        assert_eq!(side(0, KeepMode::Stencil), KeepSide::Outside);
        assert_eq!(side(1, KeepMode::Stencil), KeepSide::Inside);
        assert_eq!(side(0, KeepMode::Both), KeepSide::Both);
        assert_eq!(side(1, KeepMode::Both), KeepSide::Both);
        assert_eq!(
            keep_side(&tree, 0, false, KeepMode::Stencil),
            KeepSide::Both
        );
    }

    #[test]
    fn a_pieces_object_inside_another_object_keeps_its_inside() {
        // Object 2 lies on object 1's piece, and object 1's hole lies on
        // object 2's piece. Depth counts within each object only.
        let paths = [
            sqo(1, 0.0, 0.0, 100.0, 100.0),
            sqo(2, 20.0, 20.0, 80.0, 80.0),
            sqo(1, 40.0, 40.0, 60.0, 60.0),
        ];
        let tree = tree_of(&paths);
        assert_eq!(tree.depth, vec![0, 1, 2]);
        assert_eq!(tree.own_depth, vec![0, 0, 1]);
        assert_eq!(
            sides(&tree, &paths),
            vec![KeepSide::Inside, KeepSide::Inside, KeepSide::Outside]
        );
    }

    #[test]
    fn a_diamond_inscribed_in_a_square_nests() {
        // Every vertex of the diamond lies on the square, so the vertex
        // probes all say "on the boundary".
        let paths = [
            sq(0.0, 0.0, 10.0, 10.0),
            tp(
                &[(5.0, 0.0), (10.0, 5.0), (5.0, 10.0), (0.0, 5.0), (5.0, 0.0)],
                true,
            ),
        ];
        let tree = tree_of(&paths);
        assert_eq!(tree.parent, vec![None, Some(0)]);
    }

    #[test]
    fn non_finite_points_are_kept_and_on_the_cut() {
        let outer = sq(0.0, 0.0, 50.0, 50.0);
        let kept = KeptMaterial::single_object(&[outer.lap()], KeepMode::Pieces);
        for p in [
            Point::new(f64::NAN, 5.0),
            Point::new(5.0, f64::INFINITY),
            Point::new(f64::NEG_INFINITY, 5.0),
        ] {
            assert!(kept.is_kept(p), "{p:?}");
            assert_eq!(kept.distance_to_cut(p), 0.0, "{p:?}");
            assert!(!kept.is_clear(p, 1.0), "{p:?}");
        }
        // Huge but finite coordinates neither overflow nor hang.
        assert!(!kept.is_kept(Point::new(-1e300, 5.0)));
        assert!(kept.distance_to_cut(Point::new(1e300, 5.0)) > 1e299);
    }

    #[test]
    fn a_point_far_to_the_left_walks_only_the_occupied_cells() {
        let outer = sq(0.0, 0.0, 50.0, 50.0);
        let kept = KeptMaterial::single_object(&[outer.lap()], KeepMode::Pieces);
        CELL_VISITS.with(|c| c.set(0));
        assert!(!kept.is_kept(Point::new(-1e6, 25.0)));
        let visits = CELL_VISITS.with(|c| c.get());
        assert!(visits <= 52, "{visits} cells visited");
    }

    #[test]
    fn is_clear_matches_the_distance() {
        let outer = sq(0.0, 0.0, 50.0, 50.0);
        let kept = KeptMaterial::single_object(&[outer.lap()], KeepMode::Pieces);
        assert!(kept.is_clear(Point::new(25.0, 25.0), 5.0));
        assert!(!kept.is_clear(Point::new(25.0, 3.0), 5.0));
        assert!(kept.is_clear(Point::new(-500.0, 25.0), 5.0));
        assert!(
            !kept.is_clear(Point::new(-5.0, 25.0), 5.0),
            "exactly at the clearance"
        );
        let none = KeptMaterial::single_object(&[], KeepMode::Pieces);
        assert!(none.is_clear(Point::ZERO, 5.0));
    }

    fn mat() -> Rect {
        Rect::new(0.0, 0.0, 300.0, 300.0)
    }

    #[test]
    fn from_toolpaths_groups_rings_by_object() {
        // Two overlapping pieces of two objects: the overlap stays kept.
        let paths = [sqo(1, 0.0, 0.0, 20.0, 20.0), sqo(2, 10.0, 0.0, 30.0, 20.0)];
        let kept = KeptMaterial::from_toolpaths(&paths, |_| KeepMode::Pieces, mat(), 1.0);
        assert!(kept.is_kept(Point::new(15.0, 10.0)));
        assert!(kept.is_kept(Point::new(5.0, 10.0)));
        assert!(!kept.is_kept(Point::new(50.0, 10.0)));
    }

    #[test]
    fn from_toolpaths_reads_the_mode_per_object() {
        let paths = [
            sqo(1, 0.0, 0.0, 20.0, 20.0),
            sqo(2, 100.0, 100.0, 120.0, 120.0),
        ];
        let mode = |o: ObjectId| {
            if o == ObjectId(2) {
                KeepMode::Stencil
            } else {
                KeepMode::Pieces
            }
        };
        let kept = KeptMaterial::from_toolpaths(&paths, mode, mat(), 1.0);
        assert!(kept.is_kept(Point::new(10.0, 10.0)), "piece of object 1");
        assert!(
            !kept.is_kept(Point::new(110.0, 110.0)),
            "cut-out of the stencil"
        );
        assert!(kept.is_kept(Point::new(200.0, 200.0)), "stencil sheet");
    }

    #[test]
    fn from_toolpaths_reads_one_lap_of_pass_one_and_skips_the_rest() {
        let ring = sq(0.0, 0.0, 20.0, 20.0);
        let mut paths = crate::passes::separate(vec![ring], 2, true, 0.5);
        // An alignment stroke and an open path add no rings.
        let mut align = tp(&[(5.0, 5.0), (15.0, 5.0), (15.0, 15.0), (5.0, 5.0)], true);
        align.kind = ToolpathKind::Alignment;
        paths.push(align);
        paths.push(tp(&[(5.0, 10.0), (15.0, 10.0)], false));
        let kept = KeptMaterial::from_toolpaths(&paths, |_| KeepMode::Pieces, mat(), 1.0);
        assert!(kept.is_kept(Point::new(10.0, 12.0)));
        assert!(kept.is_kept(Point::new(10.0, 10.0)), "on the open path");
        assert!(
            kept.is_kept(Point::new(10.0, 6.0)),
            "inside the alignment triangle"
        );
        assert!(!kept.is_kept(Point::new(25.0, 10.0)));
    }

    #[test]
    fn stencil_with_an_island_in_a_counter() {
        // Outline, counter and island of one stencil object (3 levels):
        // the band and the island fall out, the counter stays.
        let outer = sq(50.0, 50.0, 110.0, 110.0);
        let counter = sq(60.0, 60.0, 100.0, 100.0);
        let island = sq(70.0, 70.0, 90.0, 90.0);
        let kept = KeptMaterial::single_object(
            &[outer.lap(), counter.lap(), island.lap()],
            KeepMode::Stencil,
        );
        assert!(!kept.is_kept(Point::new(55.0, 80.0)), "band");
        assert!(kept.is_kept(Point::new(65.0, 80.0)), "counter");
        assert!(!kept.is_kept(Point::new(80.0, 80.0)), "island");
        assert!(kept.is_kept(Point::new(30.0, 80.0)), "sheet");
    }
    #[test]
    fn own_depth_counts_a_same_object_ring_that_is_not_an_ancestor() {
        // Object 2 overlaps object 1's outline in part and holds its hole,
        // so the hole's parent is object 2 and object 1's outline is not
        // its ancestor. It still lies in that outline.
        let paths = [
            sqo(1, 0.0, 0.0, 100.0, 100.0),
            sqo(2, 50.0, 20.0, 150.0, 80.0),
            sqo(1, 60.0, 40.0, 80.0, 60.0),
        ];
        let tree = tree_of(&paths);
        assert_eq!(tree.parent, vec![None, None, Some(1)]);
        assert_eq!(tree.own_depth, vec![0, 0, 1]);
        assert_eq!(
            sides(&tree, &paths),
            vec![KeepSide::Inside, KeepSide::Inside, KeepSide::Outside]
        );
    }

    /// True when `x` lies in one of `spans`.
    fn in_spans(spans: &[(f64, f64)], x: f64) -> bool {
        spans.iter().any(|&(a, b)| a <= x && x <= b)
    }

    /// Asks [`KeptMaterial::is_kept`] on a grid of points that never lie
    /// on a ring, and the spans of each grid row, and checks they agree.
    /// Returns how many points were kept.
    fn spans_agree(kept: &KeptMaterial, area: Rect) -> usize {
        let mut n = 0;
        let step = 0.37;
        let mut y = area.y0 + 0.123;
        while y < area.y1 {
            let spans = kept.kept_spans(y, area.x0, area.x1);
            for w in spans.windows(2) {
                assert!(w[0].1 < w[1].0, "spans in order and apart: {spans:?}");
            }
            let mut x = area.x0 + 0.0517;
            while x < area.x1 {
                let p = Point::new(x, y);
                // Points within a hair of a boundary may round either way.
                let edge = spans
                    .iter()
                    .any(|&(a, b)| (x - a).abs() < 1e-6 || (x - b).abs() < 1e-6);
                if !edge {
                    let k = kept.is_kept(p);
                    assert_eq!(in_spans(&spans, x), k, "at {p:?}: {spans:?}");
                    n += usize::from(k);
                }
                x += step;
            }
            y += step;
        }
        n
    }

    #[test]
    fn kept_spans_match_is_kept_for_every_mode_and_overlap() {
        let area = Rect::new(0.0, 0.0, 60.0, 60.0);
        // A square with a hole and a piece in the hole (object 1), a
        // circle (object 2) and a square that overlaps the first (3).
        let paths = vec![
            sqo(1, 5.0, 5.0, 30.0, 30.0),
            sqo(1, 10.0, 10.0, 25.0, 25.0),
            sqo(1, 14.0, 14.0, 20.0, 20.0),
            {
                let mut c = circle((42.0, 42.0), 8.0, 40);
                c.source.object = ObjectId(2);
                c
            },
            sqo(3, 22.0, 22.0, 40.0, 33.0),
        ];
        let modes = [KeepMode::Pieces, KeepMode::Stencil, KeepMode::Both];
        for m1 in modes {
            for m2 in modes {
                for m3 in modes {
                    let mode = |o: ObjectId| match o.0 {
                        1 => m1,
                        2 => m2,
                        _ => m3,
                    };
                    let kept = KeptMaterial::from_toolpaths(&paths, mode, area, 0.8);
                    let n = spans_agree(&kept, area.inflate(2.0, 2.0));
                    assert!(n > 0, "{m1:?} {m2:?} {m3:?}");
                }
            }
        }
    }

    #[test]
    fn kept_spans_of_nothing_and_of_a_bad_row() {
        let area = Rect::new(0.0, 0.0, 10.0, 10.0);
        let none = KeptMaterial::from_toolpaths(&[], |_| KeepMode::Pieces, area, 0.0);
        assert!(none.kept_spans(5.0, 0.0, 10.0).is_empty());
        let stencil = KeptMaterial::from_toolpaths(
            &[sq(2.0, 2.0, 4.0, 4.0)],
            |_| KeepMode::Stencil,
            area,
            0.0,
        );
        assert_eq!(
            stencil.kept_spans(3.0, -5.0, 20.0),
            vec![(0.0, 2.0), (4.0, 10.0)],
            "the sheet ends at the cutting area"
        );
        assert!(stencil.kept_spans(f64::NAN, 0.0, 10.0).is_empty());
        assert!(stencil.kept_spans(3.0, 5.0, 5.0).is_empty());
    }

    #[test]
    fn two_stencils_share_one_sheet() {
        // Two letters of a stencil imported as two objects: each one's
        // shape falls out, it is not kept as the other one's sheet.
        let area = Rect::new(0.0, 0.0, 100.0, 100.0);
        let paths = vec![
            sqo(1, 10.0, 10.0, 20.0, 20.0),
            sqo(2, 40.0, 10.0, 50.0, 20.0),
        ];
        let kept = KeptMaterial::from_toolpaths(&paths, |_| KeepMode::Stencil, area, 0.0);
        assert!(!kept.is_kept(Point::new(15.0, 15.0)));
        assert!(!kept.is_kept(Point::new(45.0, 15.0)));
        assert!(kept.is_kept(Point::new(30.0, 15.0)), "the sheet between");
        assert!(!kept.is_kept(Point::new(150.0, 15.0)), "off the sheet");
        // A piece in a stencil's hole still wins.
        let paths = vec![
            sqo(1, 10.0, 10.0, 40.0, 40.0),
            sqo(2, 20.0, 20.0, 30.0, 30.0),
        ];
        let mode = |o: ObjectId| [KeepMode::Stencil, KeepMode::Pieces][o.0 as usize - 1];
        let kept = KeptMaterial::from_toolpaths(&paths, mode, area, 0.0);
        assert!(kept.is_kept(Point::new(25.0, 25.0)));
        assert!(!kept.is_kept(Point::new(15.0, 15.0)));
    }
}
