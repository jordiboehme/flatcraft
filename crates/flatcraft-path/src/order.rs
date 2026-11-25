//! Stage 7: the cut order (spec 8.3, R8).
//!
//! Each path becomes a [`Unit`]: a ring turned to the job's direction, or an
//! open path, with the places it may be entered. [`construct`] builds one
//! tour from the start by nearest neighbour under [`cost`]. Every child
//! comes before its parent on one global tour (R8, Dewil), not depth first:
//! a piece's outline still comes after all its holes and inner pieces, just
//! not necessarily right after them.
//!
//! Only units that are ready sit in a uniform grid of their entry points,
//! and each step searches rings of cells around the head, so a job of
//! thousands of paths stays fast.

use std::collections::HashMap;

use kurbo::{Point, Rect, Vec2};

use crate::containment::ContainmentTree;
use crate::plan::{ASSUMED_START_HEADING, PipelineSettings, Toolpath};
use crate::rings;

/// Move and edge evaluations the order improvement may spend over the whole
/// job (Task 6).
pub const BUDGET: u64 = 4_000_000;

/// Added to the cost of entering a ring at a corner whose extension lies on
/// the kept side, when the ring has a waste-side corner to use instead.
pub const KEPT_SIDE_PENALTY_MM: f64 = 1000.0;

/// Added to the cost for each finished cut a travel move crosses.
pub const CROSSING_PENALTY_MM: f64 = 5.0;

/// The edge of an entry grid cell, in millimetres. A sparse job gets larger
/// cells, so the grid never has more than [`CELLS_PER_ENTRY`] cells per
/// entry point (or [`MIN_CELLS`] in all).
pub const GRID_CELL_MM: f64 = 5.0;

/// How many of the best scored candidates get the crossing test.
pub const CROSSING_CHECKS: usize = 4;

const CELLS_PER_ENTRY: usize = 4;
const MIN_CELLS: usize = 4096;

/// A crossing closer than this to either end of a travel move does not
/// count: a travel starts on the cut that just finished.
const KEEP_OFF_MM: f64 = 1e-6;

/// Grows the cells a segment piece is indexed in, so a point on a cell
/// line finds it.
const EPS: f64 = 1e-9;

/// The cost of a travel move: both axes move together, so the slower axis
/// decides (R8, Chebyshev).
pub fn travel_cost(a: Point, b: Point) -> f64 {
    (b.x - a.x).abs().max((b.y - a.y).abs())
}

/// The angle between two headings in radians, 0 to pi. A zero heading
/// counts as no turn.
pub fn turn_rad(a: Vec2, b: Vec2) -> f64 {
    if a.hypot2() == 0.0 || b.hypot2() == 0.0 {
        return 0.0;
    }
    a.cross(b).abs().atan2(a.dot(b))
}

/// One way into a unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entry {
    /// The vertex the cut starts at: an index into the oriented ring, or 0
    /// (forward) or the last index (reversed) of an open path.
    pub index: usize,
    /// The ring corner's forward extension lies in waste.
    pub waste_corner: bool,
    /// Where the blade goes down.
    pub at: Point,
    /// The unit heading of the first cut.
    pub tangent: Vec2,
    /// Where the head is after the cut (after the overcut for a ring).
    pub exit: Point,
    /// The blade heading there.
    pub exit_heading: Vec2,
}

/// A path as the order sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    /// The ring in the job's direction without its repeated closing point,
    /// or the open path as given.
    pub points: Vec<Point>,
    pub closed: bool,
    /// Ring entries from [`rings::entry_candidates`], best first, or the two
    /// ends of an open path. Every strategy gets the same ones.
    pub candidates: Vec<Entry>,
    /// At least one candidate is a waste-side corner.
    pub has_waste_corner: bool,
}

/// The unit heading from vertex `i` to the next vertex of `points` that
/// differs from it, walking forward (or backward). Zero when there is none.
fn heading_from(points: &[Point], i: usize, forward: bool, wrap: bool) -> Vec2 {
    let n = points.len();
    let here = points[i];
    let next = (1..n)
        .map_while(|k| {
            if wrap {
                Some(
                    points[if forward {
                        (i + k) % n
                    } else {
                        (i + n - k) % n
                    }],
                )
            } else if forward {
                points.get(i + k).copied()
            } else {
                i.checked_sub(k).map(|j| points[j])
            }
        })
        .find(|&q| q != here);
    next.map_or(Vec2::ZERO, |q| (q - here).normalize())
}

impl Unit {
    /// The unit of toolpath `t`: its first lap turned to `s.direction`, with
    /// candidates for `t.keep` and exit states after `s.overcut_mm`.
    pub fn new(t: &Toolpath, s: &PipelineSettings) -> Self {
        if t.closed {
            let ring = rings::orient(t.lap(), s.direction);
            let mut cands = rings::entry_candidates(&ring, t.keep);
            if cands.is_empty() && !ring.is_empty() {
                cands.push(rings::Candidate {
                    index: 0,
                    waste_corner: false,
                });
            }
            let candidates: Vec<Entry> = cands
                .iter()
                .map(|c| {
                    let (exit, exit_heading) = rings::exit_state(&ring, c.index, s.overcut_mm);
                    Entry {
                        index: c.index,
                        waste_corner: c.waste_corner,
                        at: ring[c.index],
                        tangent: heading_from(&ring, c.index, true, true),
                        exit,
                        exit_heading,
                    }
                })
                .collect();
            let has_waste_corner = candidates.iter().any(|c| c.waste_corner);
            Self {
                points: ring,
                closed: true,
                candidates,
                has_waste_corner,
            }
        } else {
            let points = t.points.clone();
            let candidates = match points.len() {
                0 => Vec::new(),
                n => {
                    let last = n - 1;
                    let end = |index: usize, forward: bool| {
                        let exit_at = if forward { last } else { 0 };
                        Entry {
                            index,
                            waste_corner: false,
                            at: points[index],
                            tangent: heading_from(&points, index, forward, false),
                            exit: points[exit_at],
                            // The heading of the last move into the far end.
                            exit_heading: -heading_from(&points, exit_at, !forward, false),
                        }
                    };
                    vec![end(0, true), end(last, false)]
                }
            };
            Self {
                points,
                closed: false,
                candidates,
                has_waste_corner: false,
            }
        }
    }

    /// The cut path of this unit entered at `candidate`: a ring starts at
    /// the entry and is closed again; an open path runs from the chosen end.
    pub fn path(&self, candidate: usize) -> Vec<Point> {
        let Some(e) = self.candidates.get(candidate) else {
            return self.points.clone();
        };
        if self.closed {
            let mut ring = rings::rotate_start(&self.points, e.index);
            if let Some(&first) = ring.first() {
                ring.push(first);
            }
            ring
        } else if e.index == 0 {
            self.points.clone()
        } else {
            self.points.iter().rev().copied().collect()
        }
    }

    /// The cut segments of this unit (the ring's closing edge included).
    fn segments(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        let n = self.points.len();
        let count = if self.closed && n > 2 {
            n
        } else {
            n.saturating_sub(1)
        };
        (0..count).map(move |i| (self.points[i], self.points[(i + 1) % n]))
    }
}

/// The cost of entering unit `u` at `e` from the head at `head` with the
/// blade at `heading`, without the crossing term (spec 8.3, R8):
/// Chebyshev travel, plus the turn weight times the blade turn, plus
/// [`KEPT_SIDE_PENALTY_MM`] for a ring entered off its waste-side corners.
pub fn cost(head: Point, heading: Vec2, u: &Unit, e: &Entry, s: &PipelineSettings) -> f64 {
    let kept_side = u.closed && !e.waste_corner && u.has_waste_corner;
    travel_cost(head, e.at)
        + s.strategy.turn_weight_mm_per_rad * turn_rad(heading, e.tangent)
        + if kept_side { KEPT_SIDE_PENALTY_MM } else { 0.0 }
}

/// Unit `unit` entered at its candidate `candidate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub unit: usize,
    pub candidate: usize,
}

/// What a [`construct`] run did, for the cost tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConstructStats {
    /// Entry grid cells visited by all the searches.
    pub cells_visited: u64,
    /// Candidates scored with [`cost`].
    pub scored: u64,
    /// Finished cut segments tested for a crossing.
    pub crossing_tests: u64,
}

/// The nearest-neighbour tour over `units` from `start` (spec 8.3, R8).
///
/// A unit is ready when all its children in `tree` are placed; with
/// `outlines_last` a root is ready only when every other unit is placed.
/// Each step searches the entry grid in growing rings of cells around the
/// head. The ready units in the first ring that holds one, and in the ring
/// after it (a Chebyshev point there can still be nearer), are scored with
/// all their candidates by [`cost`]. The best [`CROSSING_CHECKS`] then get
/// [`CROSSING_PENALTY_MM`] per finished cut their travel crosses, and the
/// cheapest wins, ties to the lower unit and candidate index. The head and
/// heading move to the entry's exit state.
pub fn construct(
    units: &[Unit],
    tree: &ContainmentTree,
    start: Point,
    s: &PipelineSettings,
) -> Vec<Placement> {
    construct_with_stats(units, tree, start, s).0
}

/// [`construct`], with what it cost.
pub fn construct_with_stats(
    units: &[Unit],
    tree: &ContainmentTree,
    start: Point,
    s: &PipelineSettings,
) -> (Vec<Placement>, ConstructStats) {
    let n = units.len();
    debug_assert_eq!(tree.parent.len(), n, "the tree is of other paths");
    let mut stats = ConstructStats::default();
    let parent = |u: usize| tree.parent.get(u).copied().flatten();
    let mut waiting: Vec<usize> = (0..n)
        .map(|u| tree.children.get(u).map_or(0, Vec::len))
        .collect();
    let mut nonroots_left = (0..n).filter(|&u| parent(u).is_some()).count();
    let outlines_last = s.strategy.outlines_last;
    let ready = |u: usize, waiting: &[usize], nonroots_left: usize| {
        waiting[u] == 0 && (!outlines_last || parent(u).is_some() || nonroots_left == 0)
    };

    let mut grid = EntryGrid::new(units);
    for (u, unit) in units.iter().enumerate() {
        if ready(u, &waiting, nonroots_left) {
            grid.insert(u, unit);
        }
    }
    let mut cuts = CutIndex::default();
    let mut placed = vec![false; n];
    let mut seen = vec![0u32; n];
    let mut query = 0u32;
    let mut found = Vec::new();
    let mut scored: Vec<(f64, usize, usize)> = Vec::new();
    let mut out = Vec::with_capacity(n);
    let (mut head, mut heading) = (start, ASSUMED_START_HEADING);

    while out.len() < n {
        query += 1;
        found.clear();
        grid.search(head, &mut seen, query, &mut found, &mut stats);
        scored.clear();
        for &u in &found {
            for (ci, e) in units[u].candidates.iter().enumerate() {
                scored.push((cost(head, heading, &units[u], e, s), u, ci));
            }
        }
        stats.scored += scored.len() as u64;
        let by_cost = |a: &(f64, usize, usize), b: &(f64, usize, usize)| {
            a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2))
        };
        if scored.len() > CROSSING_CHECKS {
            scored.select_nth_unstable_by(CROSSING_CHECKS - 1, by_cost);
            scored.truncate(CROSSING_CHECKS);
        }
        if s.strategy.crossing_penalty {
            for c in &mut scored {
                let at = units[c.1].candidates[c.2].at;
                c.0 += CROSSING_PENALTY_MM * cuts.crossings(head, at, &mut stats) as f64;
            }
        }
        let pick = scored
            .iter()
            .min_by(|a, b| by_cost(a, b))
            .map(|c| (c.1, c.2));
        let (u, ci) = pick.unwrap_or_else(|| {
            // Only reachable when a unit has no usable entry point (no
            // points, or not finite): place the first one left as it is.
            let u = (0..n).find(|&u| !placed[u]).expect("a unit is left");
            (u, 0)
        });

        placed[u] = true;
        grid.remove(u, &units[u]);
        out.push(Placement {
            unit: u,
            candidate: ci,
        });
        if let Some(e) = units[u].candidates.get(ci) {
            head = e.exit;
            heading = e.exit_heading;
        }
        if s.strategy.crossing_penalty {
            for (a, b) in units[u].segments() {
                cuts.insert(a, b);
            }
        }
        if let Some(p) = parent(u) {
            waiting[p] = waiting[p].saturating_sub(1);
            if !placed[p] && ready(p, &waiting, nonroots_left) {
                grid.insert(p, &units[p]);
            }
            nonroots_left -= 1;
            if outlines_last && nonroots_left == 0 {
                for &r in &tree.roots {
                    if !placed[r] && ready(r, &waiting, nonroots_left) {
                        grid.insert(r, &units[r]);
                    }
                }
            }
        }
    }
    (out, stats)
}

/// `paths` in cut order: each ring turned to `s.direction` and started at
/// its chosen entry (closed again), each open path run from its chosen
/// end. `keep` stays as `plan` set it; `lap_range` is the whole ring.
pub fn order(
    paths: Vec<Toolpath>,
    tree: &ContainmentTree,
    start: Point,
    s: &PipelineSettings,
) -> Vec<Toolpath> {
    let units: Vec<Unit> = paths.iter().map(|t| Unit::new(t, s)).collect();
    let tour = construct(&units, tree, start, s);
    let mut slots: Vec<Option<Toolpath>> = paths.into_iter().map(Some).collect();
    tour.iter()
        .filter_map(|p| {
            let mut t = slots[p.unit].take()?;
            t.points = units[p.unit].path(p.candidate);
            t.lap_range = 0..t.points.len();
            Some(t)
        })
        .collect()
}

/// The entry points of the ready units, in square cells over the box of all
/// entry points.
struct EntryGrid {
    cell: f64,
    /// The first column and row, and how many there are.
    c0: i64,
    r0: i64,
    cols: i64,
    rows: i64,
    /// (unit, candidate) per cell, row by row.
    cells: Vec<Vec<(u32, u32)>>,
}

impl EntryGrid {
    fn new(units: &[Unit]) -> Self {
        let pts = units
            .iter()
            .flat_map(|u| u.candidates.iter().map(|e| e.at))
            .filter(|p| p.is_finite());
        let mut count = 0usize;
        let mut bounds: Option<Rect> = None;
        for p in pts {
            count += 1;
            bounds = Some(bounds.map_or(Rect::from_points(p, p), |b| b.union_pt(p)));
        }
        let b = bounds.unwrap_or(Rect::ZERO);
        let cap = (CELLS_PER_ENTRY * count).max(MIN_CELLS) as f64;
        let mut cell = GRID_CELL_MM;
        let span = |v0: f64, v1: f64, cell: f64| (v1 / cell).floor() - (v0 / cell).floor() + 1.0;
        while span(b.x0, b.x1, cell) * span(b.y0, b.y1, cell) > cap {
            cell *= 2.0;
        }
        let (c0, r0) = ((b.x0 / cell).floor() as i64, (b.y0 / cell).floor() as i64);
        let cols = span(b.x0, b.x1, cell) as i64;
        let rows = span(b.y0, b.y1, cell) as i64;
        Self {
            cell,
            c0,
            r0,
            cols,
            rows,
            cells: vec![Vec::new(); (cols * rows) as usize],
        }
    }

    fn cell_of(&self, p: Point) -> (i64, i64) {
        (
            (p.x / self.cell).floor() as i64,
            (p.y / self.cell).floor() as i64,
        )
    }

    fn index(&self, (col, row): (i64, i64)) -> Option<usize> {
        let (c, r) = (col - self.c0, row - self.r0);
        ((0..self.cols).contains(&c) && (0..self.rows).contains(&r))
            .then(|| (r * self.cols + c) as usize)
    }

    fn insert(&mut self, u: usize, unit: &Unit) {
        for (ci, e) in unit.candidates.iter().enumerate() {
            if let Some(i) =
                e.at.is_finite()
                    .then(|| self.index(self.cell_of(e.at)))
                    .flatten()
            {
                self.cells[i].push((u as u32, ci as u32));
            }
        }
    }

    fn remove(&mut self, u: usize, unit: &Unit) {
        for e in &unit.candidates {
            if let Some(i) =
                e.at.is_finite()
                    .then(|| self.index(self.cell_of(e.at)))
                    .flatten()
            {
                self.cells[i].retain(|x| x.0 != u as u32);
            }
        }
    }

    /// Push onto `found` each unit with an entry in the first ring of cells
    /// around `head` that holds one, or in the ring after it. `seen` and
    /// `query` keep each unit once.
    fn search(
        &self,
        head: Point,
        seen: &mut [u32],
        query: u32,
        found: &mut Vec<usize>,
        stats: &mut ConstructStats,
    ) {
        let (c1, r1) = (self.c0 + self.cols - 1, self.r0 + self.rows - 1);
        // A head far outside (a home metres away) is pulled in to a fixed
        // distance, so the cell arithmetic cannot overflow.
        const FAR: i64 = 1 << 32;
        let (hc, hr) = if head.is_finite() {
            let (c, r) = self.cell_of(head);
            (
                c.clamp(self.c0 - FAR, c1 + FAR),
                r.clamp(self.r0 - FAR, r1 + FAR),
            )
        } else {
            (self.c0, self.r0)
        };
        // The first ring that touches the grid, and the one that covers it.
        let k_min = (self.c0 - hc)
            .max(hc - c1)
            .max(self.r0 - hr)
            .max(hr - r1)
            .max(0);
        let k_max = (hc - self.c0).max(c1 - hc).max(hr - self.r0).max(r1 - hr);
        let mut hit: Option<i64> = None;
        for k in k_min..=k_max {
            if hit.is_some_and(|h| k > h + 1) {
                break;
            }
            for row in (hr - k).max(self.r0)..=(hr + k).min(r1) {
                let mut visit = |col: i64| {
                    stats.cells_visited += 1;
                    let i = ((row - self.r0) * self.cols + (col - self.c0)) as usize;
                    for &(u, _) in &self.cells[i] {
                        let u = u as usize;
                        if seen[u] != query {
                            seen[u] = query;
                            found.push(u);
                        }
                    }
                };
                if row == hr - k || row == hr + k {
                    ((hc - k).max(self.c0)..=(hc + k).min(c1)).for_each(&mut visit);
                } else {
                    [hc - k, hc + k]
                        .into_iter()
                        .filter(|c| (self.c0..=c1).contains(c))
                        .for_each(&mut visit);
                }
            }
            if hit.is_none() && !found.is_empty() {
                hit = Some(k);
            }
        }
    }
}

/// The finished cuts, in [`GRID_CELL_MM`] cells, for the crossing test.
#[derive(Debug, Default)]
struct CutIndex {
    segs: Vec<(Point, Point)>,
    cells: HashMap<(i64, i64), Vec<u32>>,
    stamp: Vec<u32>,
    query: u32,
}

/// The cells along `a`-`b`, in pieces of half a cell, so a long diagonal
/// fills a band of cells instead of its whole box. Every point of the
/// segment lies in one of them.
fn cells_along(a: Point, b: Point, mut visit: impl FnMut((i64, i64))) {
    let cell = |v: f64| (v / GRID_CELL_MM).floor() as i64;
    let pieces = (a.distance(b) / (GRID_CELL_MM / 2.0)).ceil().max(1.0) as usize;
    for k in 0..pieces {
        let p0 = a.lerp(b, k as f64 / pieces as f64);
        let p1 = a.lerp(b, (k + 1) as f64 / pieces as f64);
        let r = Rect::from_points(p0, p1).inflate(EPS, EPS);
        for row in cell(r.y0)..=cell(r.y1) {
            for col in cell(r.x0)..=cell(r.x1) {
                visit((col, row));
            }
        }
    }
}

impl CutIndex {
    fn insert(&mut self, a: Point, b: Point) {
        if !(a.is_finite() && b.is_finite()) {
            return;
        }
        let id = self.segs.len() as u32;
        self.segs.push((a, b));
        self.stamp.push(0);
        let cells = &mut self.cells;
        cells_along(a, b, |c| {
            let ids = cells.entry(c).or_default();
            // One segment is indexed before the next, so a repeat can only
            // be the last entry.
            if ids.last() != Some(&id) {
                ids.push(id);
            }
        });
    }

    /// How many finished cuts the travel `a`-`b` properly crosses, away
    /// from its ends.
    fn crossings(&mut self, a: Point, b: Point, stats: &mut ConstructStats) -> usize {
        if self.segs.is_empty() || !(a.is_finite() && b.is_finite()) {
            return 0;
        }
        self.query += 1;
        let query = self.query;
        let mut count = 0;
        let (segs, cells, stamp) = (&self.segs, &self.cells, &mut self.stamp);
        cells_along(a, b, |c| {
            for &id in cells.get(&c).into_iter().flatten() {
                let id = id as usize;
                if stamp[id] != query {
                    stamp[id] = query;
                    stats.crossing_tests += 1;
                    let (q1, q2) = segs[id];
                    count += usize::from(crosses(a, b, q1, q2));
                }
            }
        });
        count
    }
}

/// True when the travel `p1`-`p2` meets the cut `q1`-`q2` more than
/// [`KEEP_OFF_MM`] from both ends of the travel.
fn crosses(p1: Point, p2: Point, q1: Point, q2: Point) -> bool {
    let (r, s) = (p2 - p1, q2 - q1);
    let denom = r.cross(s);
    if denom == 0.0 {
        return false;
    }
    let t = (q1 - p1).cross(s) / denom;
    let u = (q1 - p1).cross(r) / denom;
    if !((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)) {
        return false;
    }
    let at = p1 + t * r;
    at.distance(p1) > KEEP_OFF_MM && at.distance(p2) > KEEP_OFF_MM
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::containment::{self, KeepMode};
    use crate::plan::{KeepSide, PathRef};
    use crate::strategy::StrategySettings;
    use flatcraft_core::document::{CutStrategy, ObjectId};
    use proptest::prelude::*;

    fn tp(id: u64, pts: &[(f64, f64)], closed: bool) -> Toolpath {
        Toolpath::new(
            pts.iter().map(|&p| p.into()).collect(),
            closed,
            PathRef {
                object: ObjectId(id),
                path_index: 0,
            },
        )
    }

    fn ring(id: u64, x0: f64, y0: f64, x1: f64, y1: f64) -> Toolpath {
        tp(
            id,
            &[(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)],
            true,
        )
    }

    fn settings(strategy: CutStrategy) -> PipelineSettings {
        PipelineSettings {
            strategy: StrategySettings::for_strategy(strategy),
            ..PipelineSettings::with_defaults(
                Rect::new(-1000.0, -1000.0, 1000.0, 1000.0),
                Point::ZERO,
            )
        }
    }

    /// The containment tree of `paths`, with every keep side set as `plan`
    /// sets it (one object per id, all Pieces).
    fn prepared(mut paths: Vec<Toolpath>) -> (Vec<Toolpath>, ContainmentTree) {
        let tree = containment::build(&paths);
        for (i, t) in paths.iter_mut().enumerate() {
            t.keep = containment::keep_side(&tree, i, t.closed, KeepMode::Pieces);
        }
        (paths, tree)
    }

    fn run(paths: Vec<Toolpath>, start: Point, s: &PipelineSettings) -> Vec<Toolpath> {
        let (paths, tree) = prepared(paths);
        order(paths, &tree, start, s)
    }

    fn ids(out: &[Toolpath]) -> Vec<u64> {
        out.iter().map(|t| t.source.object.0).collect()
    }

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn travel_cost_is_the_slower_axis() {
        assert_eq!(travel_cost(p(0.0, 0.0), p(30.0, -40.0)), 40.0);
        assert_eq!(travel_cost(p(1.0, 1.0), p(-2.0, 2.0)), 3.0);
        assert_eq!(travel_cost(p(5.0, 5.0), p(5.0, 5.0)), 0.0);
    }

    #[test]
    fn open_path_starts_at_nearest_end() {
        let s = settings(CutStrategy::Quality);
        let out = run(
            vec![tp(1, &[(100.0, 0.0), (10.0, 0.0)], false)],
            p(0.0, 0.0),
            &s,
        );
        assert_eq!(out[0].points, vec![p(10.0, 0.0), p(100.0, 0.0)]);
    }

    #[test]
    fn next_path_is_chosen_from_previous_end() {
        let s = settings(CutStrategy::Quality);
        let a = tp(1, &[(0.0, 1.0), (50.0, 1.0)], false);
        let far = tp(2, &[(500.0, 0.0), (400.0, 0.0)], false);
        let near_end = tp(3, &[(60.0, 0.0), (51.0, 0.0)], false);
        let out = run(vec![far, a, near_end], Point::ZERO, &s);
        assert_eq!(ids(&out), vec![1, 3, 2]);
        assert_eq!(out[1].points[0], p(51.0, 0.0));
        assert_eq!(out[2].points[0], p(400.0, 0.0));
    }

    #[test]
    fn a_home_far_away_does_not_overflow_the_grid() {
        let s = settings(CutStrategy::Quality);
        let paths = || {
            vec![
                tp(1, &[(10.0, 0.0), (20.0, 0.0)], false),
                tp(2, &[(10.0, 90.0), (20.0, 90.0)], false),
            ]
        };
        assert_eq!(run(paths(), p(1e20, 1e20), &s).len(), 2);
        assert_eq!(ids(&run(paths(), p(-1e20, -1e20), &s)), vec![1, 2]);
    }

    #[test]
    fn empty_input_is_empty() {
        let s = settings(CutStrategy::Quality);
        assert!(run(vec![], Point::ZERO, &s).is_empty());
    }

    #[test]
    fn hole_is_cut_before_outline() {
        // The outline's start is nearer to the origin than the hole's.
        let s = settings(CutStrategy::Quality);
        let outer = ring(1, 10.0, 10.0, 60.0, 60.0);
        let hole = ring(2, 30.0, 30.0, 40.0, 40.0);
        let out = run(vec![outer, hole], Point::ZERO, &s);
        assert_eq!(ids(&out), vec![2, 1]);
    }

    #[test]
    fn three_deep_nesting_cuts_inside_out() {
        let s = settings(CutStrategy::Quality);
        let a = ring(1, 0.0, 0.0, 100.0, 100.0);
        let b = ring(2, 10.0, 10.0, 90.0, 90.0);
        let c = ring(3, 40.0, 40.0, 60.0, 60.0);
        let out = run(vec![a, b, c], Point::ZERO, &s);
        assert_eq!(ids(&out), vec![3, 2, 1]);
    }

    #[test]
    fn two_siblings_come_before_their_parent() {
        let s = settings(CutStrategy::Quality);
        let parent = ring(1, 0.0, 0.0, 100.0, 50.0);
        let left = ring(2, 10.0, 10.0, 30.0, 30.0);
        let right = ring(3, 60.0, 10.0, 80.0, 30.0);
        let out = run(vec![parent, right, left], Point::ZERO, &s);
        assert_eq!(ids(&out), vec![2, 3, 1]);
    }

    #[test]
    fn an_open_line_inside_a_ring_is_cut_before_it() {
        // The containment tree makes the line a child of the ring, so it is
        // cut while the ring still holds the material around it.
        let s = settings(CutStrategy::Quality);
        let outer = ring(1, 10.0, 10.0, 60.0, 60.0);
        let line = tp(2, &[(30.0, 30.0), (40.0, 40.0)], false);
        let out = run(vec![outer, line], Point::ZERO, &s);
        assert_eq!(ids(&out), vec![2, 1]);
    }

    #[test]
    fn rings_are_rotated_to_their_entry_and_closed() {
        let s = settings(CutStrategy::Quality);
        // Clockwise as given; the nearest corner to the start is (20, 20).
        let out = run(vec![ring(1, 10.0, 10.0, 20.0, 20.0)], p(25.0, 25.0), &s);
        let t = &out[0];
        assert_eq!(t.points.len(), 5);
        assert_eq!(t.points[0], p(20.0, 20.0));
        assert_eq!(t.points.first(), t.points.last());
        assert!(containment::signed_area(&t.points) < 0.0);
        assert_eq!(t.lap_range, 0..5);
    }

    #[test]
    fn keep_sheet_cuts_all_holes_before_any_outline() {
        // Two frames with a hole each; the head starts at the left one.
        let paths = || {
            vec![
                ring(1, 10.0, 10.0, 60.0, 60.0),
                ring(2, 30.0, 30.0, 40.0, 40.0),
                ring(3, 100.0, 10.0, 150.0, 60.0),
                ring(4, 120.0, 30.0, 130.0, 40.0),
            ]
        };
        let quality = run(paths(), Point::ZERO, &settings(CutStrategy::Quality));
        assert_eq!(
            ids(&quality),
            vec![2, 1, 4, 3],
            "Quality frees each piece in turn"
        );
        let sheet = run(paths(), Point::ZERO, &settings(CutStrategy::KeepSheet));
        // Both holes first, then the outlines, nearest first.
        assert_eq!(ids(&sheet), vec![2, 4, 3, 1]);
    }

    #[test]
    fn fast_strategy_ignores_turn_weight() {
        // A square outline: every corner is a waste-side corner, so the
        // kept-side penalty does not apply. Counter-clockwise from (10, 10)
        // the cut goes down first; from (10, 20) it goes right, the blade's
        // start heading.
        let t = {
            let mut t = ring(1, 10.0, 10.0, 20.0, 20.0);
            t.keep = KeepSide::Inside;
            t
        };
        let (tree, start) = (containment::build(std::slice::from_ref(&t)), p(5.0, 14.5));
        let fast = settings(CutStrategy::Fast);
        let quality = settings(CutStrategy::Quality);
        let unit = Unit::new(&t, &fast);
        assert_eq!(unit, Unit::new(&t, &quality), "same candidates");
        assert_eq!(unit.candidates.len(), 4);
        assert!(unit.candidates.iter().all(|c| c.waste_corner));
        let entry = |s: &PipelineSettings| {
            let tour = construct(std::slice::from_ref(&unit), &tree, start, s);
            unit.candidates[tour[0].candidate].at
        };
        let by_travel = unit
            .candidates
            .iter()
            .min_by(|a, b| travel_cost(start, a.at).total_cmp(&travel_cost(start, b.at)))
            .unwrap()
            .at;
        assert_eq!(by_travel, p(10.0, 10.0));
        assert_eq!(entry(&fast), by_travel);
        // Not vacuous: the turn weight moves Quality to the corner whose
        // first cut needs no blade turn.
        assert_eq!(entry(&quality), p(10.0, 20.0));
    }

    #[test]
    fn crossing_penalty_avoids_travel_over_a_finished_cut() {
        // After the square, the line on the right is nearer (12 mm) but the
        // travel to it runs through the square and crosses its right side
        // once (+5 mm); the line on the left (14 mm) crosses nothing.
        let paths = vec![
            ring(1, 0.0, 0.0, 10.0, 10.0),
            tp(2, &[(12.0, 1.0), (13.0, 1.0)], false),
            tp(3, &[(-14.0, 0.0), (-15.0, 0.0)], false),
        ];
        let (paths, tree) = prepared(paths);
        let mut s = settings(CutStrategy::Fast);
        s.overcut_mm = 0.0;
        let units: Vec<Unit> = paths.iter().map(|t| Unit::new(t, &s)).collect();
        let second = |s: &PipelineSettings| construct(&units, &tree, p(-1.0, -1.0), s)[1].unit;
        assert!(s.strategy.crossing_penalty);
        assert_eq!(second(&s), 2);
        s.strategy.crossing_penalty = false;
        assert_eq!(second(&s), 1);
    }

    #[test]
    fn a_ring_is_entered_off_its_waste_corners_only_when_it_has_none() {
        let s = settings(CutStrategy::Quality);
        // A circle offers no corner, so no candidate is penalised.
        let n = 64;
        let pts: Vec<(f64, f64)> = (0..=n)
            .map(|k| {
                let a = std::f64::consts::TAU * (k % n) as f64 / n as f64;
                (50.0 + 10.0 * a.cos(), 50.0 + 10.0 * a.sin())
            })
            .collect();
        let (paths, _) = prepared(vec![tp(1, &pts, true)]);
        let unit = Unit::new(&paths[0], &s);
        assert!(!unit.has_waste_corner);
        for e in &unit.candidates {
            let c = cost(Point::ZERO, Vec2::ZERO, &unit, e, &s);
            assert!(c < KEPT_SIDE_PENALTY_MM, "{c}");
        }
    }

    /// A tiny linear congruential generator, so the big tests need no seed
    /// crate and give the same squares every run.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    fn random_squares(n: usize) -> Vec<Toolpath> {
        let mut rng = Lcg(42);
        (0..n)
            .map(|i| {
                let (x, y) = (rng.next() * 600.0, rng.next() * 900.0);
                let side = 0.5 + rng.next() * 2.0;
                ring(i as u64, x, y, x + side, y + side)
            })
            .collect()
    }

    #[test]
    fn nn_is_grid_accelerated() {
        // The guard is a count, so it holds on any machine: a brute-force
        // nearest neighbour scores every candidate of every remaining
        // unit, about n * n / 2 * 4 = 800 million for 20000 squares.
        let n = 20_000;
        let s = settings(CutStrategy::Quality);
        let (paths, tree) = prepared(random_squares(n));
        let units: Vec<Unit> = paths.iter().map(|t| Unit::new(t, &s)).collect();
        let (tour, stats) = construct_with_stats(&units, &tree, Point::ZERO, &s);
        assert_eq!(tour.len(), n);
        let mut seen = vec![false; n];
        for pl in &tour {
            assert!(!std::mem::replace(&mut seen[pl.unit], true));
        }
        let n = n as u64;
        assert!(stats.cells_visited <= 60 * n, "{stats:?}");
        assert!(stats.scored <= 60 * n, "{stats:?}");
        assert!(stats.crossing_tests <= 60 * n, "{stats:?}");
    }

    /// Wall-clock check (Ruling F30): run by hand or by the quality bench.
    /// Measured 0.09 to 0.15 s locally (Apple silicon, test profile), so
    /// the bound is 0.5 s, not the plan's 3 s.
    #[test]
    #[ignore = "wall-clock perf check, run by hand or by the quality bench"]
    fn nn_of_20000_squares_is_fast_in_debug() {
        let s = settings(CutStrategy::Quality);
        let (paths, tree) = prepared(random_squares(20_000));
        let t0 = std::time::Instant::now();
        let out = order(paths, &tree, Point::ZERO, &s);
        let secs = t0.elapsed().as_secs_f64();
        eprintln!("order of 20000 squares: {secs} s");
        assert_eq!(out.len(), 20_000);
        assert!(secs < 0.5, "{secs} s");
    }

    fn rects() -> impl Strategy<Value = Vec<(f64, f64, f64, f64, bool)>> {
        proptest::collection::vec(
            (
                0.0f64..200.0,
                0.0f64..200.0,
                1.0f64..120.0,
                1.0f64..120.0,
                proptest::bool::weighted(0.2),
            ),
            1..40,
        )
    }

    proptest! {
        #[test]
        fn child_before_parent_holds_for_random_trees(
            rects in rects(),
            keep_sheet in proptest::bool::ANY,
        ) {
            let paths: Vec<Toolpath> = rects
                .iter()
                .enumerate()
                .map(|(i, &(x, y, w, h, open))| {
                    if open {
                        tp(i as u64, &[(x, y), (x + w / 4.0, y + h / 4.0)], false)
                    } else {
                        ring(i as u64, x, y, x + w, y + h)
                    }
                })
                .collect();
            let (paths, tree) = prepared(paths);
            let strategy = if keep_sheet { CutStrategy::KeepSheet } else { CutStrategy::Quality };
            let s = settings(strategy);
            let units: Vec<Unit> = paths.iter().map(|t| Unit::new(t, &s)).collect();
            let tour = construct(&units, &tree, Point::ZERO, &s);
            prop_assert_eq!(tour.len(), paths.len());
            let mut pos = vec![usize::MAX; paths.len()];
            for (k, pl) in tour.iter().enumerate() {
                prop_assert_eq!(pos[pl.unit], usize::MAX, "placed twice");
                prop_assert!(pl.candidate < units[pl.unit].candidates.len());
                pos[pl.unit] = k;
            }
            for (child, parent) in tree.parent.iter().enumerate() {
                if let Some(parent) = parent {
                    prop_assert!(pos[child] < pos[*parent], "{} after its parent {}", child, parent);
                }
            }
            if keep_sheet {
                let last_inner = (0..paths.len()).filter(|&u| tree.parent[u].is_some()).map(|u| pos[u]).max();
                let first_root = tree.roots.iter().map(|&u| pos[u]).min();
                if let (Some(a), Some(b)) = (last_inner, first_root) {
                    prop_assert!(a < b, "an outline before an inner path");
                }
            }
        }
    }
}
