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
//!
//! [`improve`] then shortens that tour with 2-opt and Or-opt moves over
//! neighbour lists and picks every entry again with the exact [`entry_dp`],
//! within an evaluation [`BUDGET`]. No move puts a child after its parent.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};

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
/// head. Every ready unit found is scored with all its candidates by
/// [`cost`], and the search goes on while a farther ring could still hold a
/// cheaper one: every cost term is at least 0, so a cost is at least its
/// travel, and the search is exact for travel, turn and kept side. The best
/// [`CROSSING_CHECKS`] then get [`CROSSING_PENALTY_MM`] per finished cut
/// their travel crosses, and the cheapest wins, ties to the lower unit and
/// candidate index. The head and heading move to the entry's exit state.
///
/// Crossings here count every finished cut segment the travel crosses,
/// more than 1e-6 mm from its ends. The simulator's `travel_crossings`
/// counts crossing travels instead, with a keep-off of twice the blade
/// offset, so the two numbers can differ. That is a design choice: the
/// order wants a cost that grows with each cut crossed.
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
    let (mut head, mut heading) = (start, s.start_heading);

    while out.len() < n {
        query += 1;
        found.clear();
        scored.clear();
        let mut best = f64::INFINITY;
        grid.search(head, &mut seen, query, &mut found, &mut stats, |new| {
            for &u in new {
                for (ci, e) in units[u].candidates.iter().enumerate() {
                    let c = cost(head, heading, &units[u], e, s);
                    best = best.min(c);
                    scored.push((c, u, ci));
                }
            }
            best
        });
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
                c.0 += CROSSING_PENALTY_MM * cuts.crossings(head, at, c.1, &mut stats) as f64;
            }
        }
        let pick = scored
            .iter()
            .min_by(|a, b| by_cost(a, b))
            .map(|c| (c.1, c.2));
        let (u, ci) = pick.unwrap_or_else(|| {
            // Only reachable when a ready unit has no usable entry point (no
            // points, or not finite): place the first ready one as it is.
            // A unit whose children wait is never taken while one is ready.
            let u = (0..n)
                .find(|&u| !placed[u] && ready(u, &waiting, nonroots_left))
                .or_else(|| (0..n).find(|&u| !placed[u]))
                .expect("a unit is left");
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
                cuts.insert(a, b, u);
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

/// `paths` in cut order ([`construct`], then [`improve`]): each ring turned
/// to `s.direction` and started at its chosen entry (closed again), each
/// open path run from its chosen end. `keep` stays as `plan` set it; `lap_range` is the whole ring.
///
/// With `s.monotonic_feed` the paths are cut in feed bands ([`crate::feed`]),
/// in increasing band order, and each band is ordered on its own. The head
/// position and blade heading carry over from one band to the next, and the
/// improvement budget is shared by the bands in proportion to their paths.
/// A path whose parent lies in a later band is not a root in its band, but
/// the tree of a band only holds the paths of the band, so with
/// `outlines_last` it waits as an outline of that band.
pub fn order(
    paths: Vec<Toolpath>,
    tree: &ContainmentTree,
    start: Point,
    s: &PipelineSettings,
) -> Vec<Toolpath> {
    if !s.monotonic_feed {
        return order_group(paths, tree, start, s).0;
    }
    let band = crate::feed::bands(&paths, tree);
    let mut distinct = band.clone();
    distinct.sort_unstable();
    distinct.dedup();
    let total = paths.len().max(1) as u64;
    let mut slots: Vec<Option<Toolpath>> = paths.into_iter().map(Some).collect();
    let (mut head, mut heading) = (start, s.start_heading);
    let mut out = Vec::with_capacity(slots.len());
    for b in distinct {
        let members: Vec<usize> = (0..slots.len()).filter(|&i| band[i] == b).collect();
        let local: HashMap<usize, usize> =
            members.iter().enumerate().map(|(l, &g)| (g, l)).collect();
        let map = |g: usize| local.get(&g).copied();
        let sub = ContainmentTree {
            parent: members
                .iter()
                .map(|&g| tree.parent.get(g).copied().flatten().and_then(map))
                .collect(),
            children: members
                .iter()
                .map(|&g| {
                    let kids = tree.children.get(g).map_or(&[][..], Vec::as_slice);
                    kids.iter().filter_map(|&c| map(c)).collect()
                })
                .collect(),
            roots: members
                .iter()
                .filter(|&&g| {
                    tree.parent
                        .get(g)
                        .copied()
                        .flatten()
                        .and_then(map)
                        .is_none()
                })
                .filter_map(|&g| map(g))
                .collect(),
            depth: members
                .iter()
                .map(|&g| tree.depth.get(g).copied().unwrap_or(0))
                .collect(),
            own_depth: members
                .iter()
                .map(|&g| tree.own_depth.get(g).copied().unwrap_or(0))
                .collect(),
        };
        let mut band_settings = s.clone();
        band_settings.start_heading = heading;
        band_settings.strategy.budget =
            (u128::from(s.strategy.budget) * members.len() as u128 / u128::from(total)) as u64;
        let group: Vec<Toolpath> = members.iter().filter_map(|&g| slots[g].take()).collect();
        let (ordered, exit) = order_group(group, &sub, head, &band_settings);
        if let Some((h, hd)) = exit {
            (head, heading) = (h, hd);
        }
        out.extend(ordered);
    }
    out
}

/// One group of `paths` in cut order from `start`, and the head and blade
/// heading after the last one (`None` when the group has no usable entry).
fn order_group(
    paths: Vec<Toolpath>,
    tree: &ContainmentTree,
    start: Point,
    s: &PipelineSettings,
) -> (Vec<Toolpath>, Option<(Point, Vec2)>) {
    let units: Vec<Unit> = paths.iter().map(|t| Unit::new(t, s)).collect();
    let mut tour = construct(&units, tree, start, s);
    improve(&units, &mut tour, tree, start, s);
    let exit = tour
        .iter()
        .rev()
        .find_map(|p| units[p.unit].candidates.get(p.candidate))
        .map(|e| (e.exit, e.exit_heading));
    let mut slots: Vec<Option<Toolpath>> = paths.into_iter().map(Some).collect();
    let ordered = tour
        .iter()
        .filter_map(|p| {
            let mut t = slots[p.unit].take()?;
            t.points = units[p.unit].path(p.candidate);
            t.lap_range = 0..t.points.len();
            Some(t)
        })
        .collect();
    (ordered, exit)
}

/// How many nearest units each unit's improvement moves look at (R8).
pub const NEIGHBOURS: usize = 8;

/// The longest chain an Or-opt move carries.
pub const OR_OPT_CHAIN: usize = 3;

/// A move is applied only when it lowers the tour cost by more than this
/// (or the rounding bound of the cached sums, when larger), so float noise
/// never makes the search go round in circles.
const MIN_GAIN_MM: f64 = 1e-9;

/// What an [`improve`] run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImproveStats {
    /// 2-opt and Or-opt move evaluations plus entry DP edge evaluations.
    /// Never more than the strategy's budget.
    pub evaluations: u64,
    /// Rounds started.
    pub rounds: u32,
    /// 2-opt moves applied.
    pub two_opt_moves: u64,
    /// Or-opt moves applied.
    pub or_opt_moves: u64,
    /// Entry DP runs that changed an entry.
    pub entry_dp_changes: u32,
    /// With `crossing_penalty`: stretches of changed entries the DP
    /// applied (each is scored with its crossings on its own).
    pub entry_dp_runs: u64,
    /// Cut segments tested for a crossing (with `crossing_penalty`): the
    /// joins of each move that passed the cost screen, the joins of each
    /// DP run and each round's join cache. Not in the budget.
    pub crossing_tests: u64,
    /// Moves and DP runs that were not applied because of their
    /// crossings (or, for a DP run, because they gained nothing).
    pub crossing_rejections: u64,
}

/// The cost of `tour` from `start` under [`cost`], without the crossing
/// term: what [`improve`] and [`entry_dp`] lower. A unit without an entry
/// costs nothing and leaves the head where it is, as in [`construct`].
pub fn tour_cost(units: &[Unit], tour: &[Placement], start: Point, s: &PipelineSettings) -> f64 {
    let (mut head, mut heading) = (start, s.start_heading);
    let mut total = 0.0;
    for p in tour {
        let u = &units[p.unit];
        if let Some(e) = u.candidates.get(p.candidate) {
            total += cost(head, heading, u, e, s);
            head = e.exit;
            heading = e.exit_heading;
        }
    }
    total
}

/// Every cut segment of `units`, for the improvement's crossing test.
fn all_cuts(units: &[Unit]) -> CutIndex {
    let mut cuts = CutIndex::default();
    for (k, u) in units.iter().enumerate() {
        for (a, b) in u.segments() {
            cuts.insert(a, b, k);
        }
    }
    cuts
}

/// The head and blade heading after `from` (the start for `None`).
fn head_after(units: &[Unit], from: Option<Placement>, start: Point) -> (Point, Vec2) {
    match from.and_then(|p| units[p.unit].candidates.get(p.candidate)) {
        Some(e) => (e.exit, e.exit_heading),
        None => (start, ASSUMED_START_HEADING),
    }
}

/// How many finished cut segments the travels of `tour` cross, more than
/// 1e-6 mm from their ends, as [`construct`] counts them: a travel into
/// the unit at position p counts only the cuts of the units at positions
/// before p. Segments are counted, not travels, so the simulator's
/// `travel_crossings` can be lower.
pub fn tour_crossings(units: &[Unit], tour: &[Placement], start: Point) -> usize {
    let mut cuts = all_cuts(units);
    cuts.rank_by(tour, units.len());
    let mut stats = ConstructStats::default();
    let (mut head, mut count) = (start, 0);
    for p in tour {
        if let Some(e) = units[p.unit].candidates.get(p.candidate) {
            count += cuts.crossings(head, e.at, p.unit, &mut stats);
            head = e.exit;
        }
    }
    count
}

/// True when every child in `tree` comes before its parent and, with
/// `outlines_last`, every unit inside another before any root.
pub fn respects_precedence(
    tour: &[Placement],
    tree: &ContainmentTree,
    outlines_last: bool,
) -> bool {
    let n = tree.parent.len();
    if tour.len() != n {
        return false;
    }
    let mut pos = vec![usize::MAX; n];
    for (k, p) in tour.iter().enumerate() {
        if p.unit >= n || pos[p.unit] != usize::MAX {
            return false;
        }
        pos[p.unit] = k;
    }
    let children_first = (0..n).all(|u| tree.parent[u].is_none_or(|p| pos[u] < pos[p]));
    let roots_last = !outlines_last || {
        let inner = (0..n).filter(|&u| tree.parent[u].is_some()).count();
        tour[..inner].iter().all(|p| tree.parent[p.unit].is_some())
    };
    children_first && roots_last
}

/// The entry of every unit in `tour`, chosen for the fixed order by a
/// layered shortest path over the candidates under [`cost`] (R8). It is
/// exact: no other choice of entries gives a lower [`tour_cost`]. The tour
/// changes only when the cost drops. Returns the edges evaluated.
pub fn entry_dp(units: &[Unit], tour: &mut [Placement], start: Point, s: &PipelineSettings) -> u64 {
    let (chosen, evaluations) = entry_dp_choice(units, tour, start, s);
    if tour_cost(units, &chosen, start, s) < tour_cost(units, tour, start, s) {
        tour.copy_from_slice(&chosen);
    }
    evaluations
}

/// The positions of `tour` whose unit has candidates: the layers of the
/// [`entry_dp`], and the travels a tour makes.
fn layers(units: &[Unit], tour: &[Placement]) -> Vec<usize> {
    (0..tour.len())
        .filter(|&k| !units[tour[k].unit].candidates.is_empty())
        .collect()
}

/// The entries the [`entry_dp`] picks for `tour`, and the edges evaluated.
fn entry_dp_choice(
    units: &[Unit],
    tour: &[Placement],
    start: Point,
    s: &PipelineSettings,
) -> (Vec<Placement>, u64) {
    let layers = layers(units, tour);
    let Some(&last) = layers.last() else {
        return (tour.to_vec(), 0);
    };
    let mut evaluations = 0u64;
    let mut back: Vec<Vec<u32>> = Vec::with_capacity(layers.len());
    let mut prev_cost = vec![0.0];
    let mut prev_at: Option<usize> = None;
    for &k in &layers {
        let u = &units[tour[k].unit];
        let mut here = vec![f64::INFINITY; u.candidates.len()];
        let mut from = vec![0u32; u.candidates.len()];
        for (b, e) in u.candidates.iter().enumerate() {
            for (a, &before) in prev_cost.iter().enumerate() {
                let (head, heading) = match prev_at {
                    None => (start, s.start_heading),
                    Some(pk) => {
                        let pe = &units[tour[pk].unit].candidates[a];
                        (pe.exit, pe.exit_heading)
                    }
                };
                evaluations += 1;
                let c = before + cost(head, heading, u, e, s);
                // Ties keep the lower predecessor, so the result is stable.
                if c < here[b] {
                    here[b] = c;
                    from[b] = a as u32;
                }
            }
        }
        back.push(from);
        prev_cost = here;
        prev_at = Some(k);
    }
    debug_assert_eq!(prev_at, Some(last));
    let mut pick = (0..prev_cost.len())
        .min_by(|&a, &b| prev_cost[a].total_cmp(&prev_cost[b]).then(a.cmp(&b)))
        .unwrap_or(0);
    let mut chosen = tour.to_vec();
    for (li, &k) in layers.iter().enumerate().rev() {
        chosen[k].candidate = pick;
        pick = back[li][pick] as usize;
    }
    (chosen, evaluations)
}

/// Apply the entries of `chosen` (the [`entry_dp_choice`] for `tour`) run
/// by run, scoring crossings against `cuts`. A run is a stretch of layers
/// whose entry changed; it changes only the travels into its layers and
/// into the layer after it, so runs are independent. A run is applied when
/// its cost gain is above [`MIN_GAIN_MM`] and still positive with
/// [`CROSSING_PENALTY_MM`] per crossing. Returns the change of the crossing
/// count.
fn apply_dp_runs(
    units: &[Unit],
    tour: &mut [Placement],
    chosen: &[Placement],
    start: Point,
    s: &PipelineSettings,
    cuts: &mut CutIndex,
    stats: &mut ImproveStats,
) -> i64 {
    let layers = layers(units, tour);
    // Runs are independent, so every run is scored against the old tour.
    let old = tour.to_vec();
    // The cost and crossings of the travel into layer `li`, with
    // `chosen`'s placements at the layers where `new` holds.
    let mut join = |li: usize, new: &dyn Fn(usize) -> bool, stats: &mut ImproveStats| {
        let place = |lj: usize| {
            let k = layers[lj];
            if new(lj) { chosen[k] } else { old[k] }
        };
        let to = place(li);
        let (head, heading) = match li.checked_sub(1) {
            Some(lp) => {
                let e = &units[place(lp).unit].candidates[place(lp).candidate];
                (e.exit, e.exit_heading)
            }
            None => (start, s.start_heading),
        };
        let u = &units[to.unit];
        let e = &u.candidates[to.candidate];
        let mut cs = ConstructStats::default();
        let crossed = cuts.crossings(head, e.at, to.unit, &mut cs);
        stats.crossing_tests += cs.crossing_tests;
        (cost(head, heading, u, e, s), crossed as i64)
    };
    let changed = |lj: usize| old[layers[lj]] != chosen[layers[lj]];
    let mut delta = 0i64;
    let mut li = 0;
    while li < layers.len() {
        if !changed(li) {
            li += 1;
            continue;
        }
        let first = li;
        while li < layers.len() && changed(li) {
            li += 1;
        }
        let run = first..li;
        let last_join = li.min(layers.len() - 1);
        let (mut gain, mut crossed) = (0.0, 0i64);
        for lj in first..=last_join {
            let (old_cost, old_crossed) = join(lj, &|_| false, stats);
            let (new_cost, new_crossed) = join(lj, &|x| run.contains(&x), stats);
            gain += old_cost - new_cost;
            crossed += new_crossed - old_crossed;
        }
        if gain > MIN_GAIN_MM && gain - CROSSING_PENALTY_MM * crossed as f64 > MIN_GAIN_MM {
            for lj in run {
                tour[layers[lj]] = chosen[layers[lj]];
            }
            delta += crossed;
            stats.entry_dp_runs += 1;
        } else {
            stats.crossing_rejections += 1;
        }
    }
    delta
}

/// The edges [`entry_dp`] evaluates on `tour`.
fn entry_dp_edges(units: &[Unit], tour: &[Placement]) -> u64 {
    let mut prev = 1u64;
    let mut total = 0u64;
    for p in tour {
        let c = units[p.unit].candidates.len() as u64;
        if c > 0 {
            total += prev * c;
            prev = c;
        }
    }
    total
}

/// Improve the [`construct`] tour (R8): 2-opt over neighbour lists with
/// don't-look bits, then Or-opt chains of 1 to [`OR_OPT_CHAIN`] units,
/// optionally reversed, then the [`entry_dp`], for up to
/// `s.strategy.rounds` rounds or until a round gains nothing.
///
/// Each unit's moves join it to one of its [`NEIGHBOURS`] nearest units by
/// [`travel_cost`] between entry points (or to the start, when that is as
/// near). Reversing a run flips its open paths; a ring keeps its entry, so
/// its exit state stays the same. A move that puts a child after its parent
/// (or, with `outlines_last`, mixes roots and inner units) is never applied,
/// and a move is applied only when its exact cost change is negative.
///
/// With `crossing_penalty` (ruling F6), a move that passes that screen also
/// counts the cut segments its new travels cross, and it is applied only
/// when its gain with [`CROSSING_PENALTY_MM`] per crossing is still
/// positive. The count uses the same segment test as [`construct`], but
/// against every cut of the job except those of the unit the travel goes
/// to, finished or not, so a travel's count does not depend on where a
/// move puts it. That charges travels over cuts not made yet, too; it was
/// measured to leave fewer real (finished-only, [`tour_crossings`])
/// crossings than counting only the cuts finished at the round's start. The DP result is
/// applied run by run: each stretch of changed entries is scored the same
/// way on its own. The crossing tests are paid per move that passed the
/// screen, per DP run and for each round's join cache, never per
/// evaluation; they are counted in `crossing_tests`, not in the budget.
/// A round counts as a gain only when the cost with the crossing term
/// drops. `s.strategy.budget` caps the evaluations;
/// when it runs out, the best tour so far is kept. A tour that already
/// breaks precedence keeps its order, and the DP still runs.
pub fn improve(
    units: &[Unit],
    tour: &mut Vec<Placement>,
    tree: &ContainmentTree,
    start: Point,
    s: &PipelineSettings,
) -> ImproveStats {
    let mut stats = ImproveStats::default();
    let budget = s.strategy.budget;
    let valid_entries = tour
        .iter()
        .all(|p| p.unit < units.len() && p.candidate < units[p.unit].candidates.len().max(1));
    if tour.is_empty() || !valid_entries {
        return stats;
    }
    let finite = |e: &Entry| {
        e.at.is_finite()
            && e.exit.is_finite()
            && e.tangent.is_finite()
            && e.exit_heading.is_finite()
    };
    let searchable = tour.len() > 1
        && tour.iter().all(|p| {
            units[p.unit]
                .candidates
                .get(p.candidate)
                .is_some_and(finite)
        })
        && respects_precedence(tour, tree, s.strategy.outlines_last);
    let neighbours = if searchable && s.strategy.rounds > 0 {
        neighbour_lists(units, start, NEIGHBOURS)
    } else {
        Vec::new()
    };
    let cuts = (s.strategy.crossing_penalty && s.strategy.rounds > 0)
        .then(|| RefCell::new(all_cuts(units)));
    for _ in 0..s.strategy.rounds {
        stats.rounds += 1;
        let before = tour_cost(units, tour, start, s);
        // The change of the crossing count in this round.
        let mut crossed = 0i64;
        let mut spent = false;
        if searchable {
            let mut search = Search::new(
                units,
                tree,
                start,
                s,
                cuts.as_ref(),
                std::mem::take(tour),
                &mut stats,
            );
            spent = !search.run(&neighbours);
            search.flush();
            crossed += search.crossed;
            *tour = search.tour;
        }
        if s.strategy.entry_dp && !spent {
            let edges = entry_dp_edges(units, tour);
            if stats.evaluations + edges <= budget {
                match &cuts {
                    None => {
                        let old = tour.clone();
                        stats.evaluations += entry_dp(units, tour, start, s);
                        stats.entry_dp_changes += u32::from(*tour != old);
                    }
                    Some(cuts) => {
                        let (chosen, evaluations) = entry_dp_choice(units, tour, start, s);
                        stats.evaluations += evaluations;
                        let runs = stats.entry_dp_runs;
                        let index = &mut cuts.borrow_mut();
                        crossed += apply_dp_runs(units, tour, &chosen, start, s, index, &mut stats);
                        stats.entry_dp_changes += u32::from(stats.entry_dp_runs > runs);
                    }
                }
            } else {
                spent = true;
            }
        }
        debug_assert!(!searchable || respects_precedence(tour, tree, s.strategy.outlines_last));
        let after = tour_cost(units, tour, start, s) + CROSSING_PENALTY_MM * crossed as f64;
        if spent || after >= before - MIN_GAIN_MM {
            break;
        }
    }
    stats
}

/// The [`NEIGHBOURS`] nearest units of each unit, nearest first (ties to the
/// lower index), by the least [`travel_cost`] between their entry points,
/// and whether the start is as near as the farthest of them.
fn neighbour_lists(units: &[Unit], start: Point, k: usize) -> Vec<(Vec<usize>, bool)> {
    let n = units.len();
    let mut grid = EntryGrid::new(units);
    for (u, unit) in units.iter().enumerate() {
        grid.insert(u, unit);
    }
    let by = |a: &(f64, usize), b: &(f64, usize)| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1));
    let mut seen = vec![0u32; n];
    let mut query = 0u32;
    let mut found = Vec::new();
    let mut stats = ConstructStats::default();
    let mut nearest: Vec<(f64, usize)> = Vec::new();
    let mut merged: Vec<(f64, usize)> = Vec::new();
    let mut out = Vec::with_capacity(n);
    for (u, unit) in units.iter().enumerate() {
        merged.clear();
        // The k nearest of u are among the k nearest of its entry points.
        for e in unit.candidates.iter().filter(|e| e.at.is_finite()) {
            query += 1;
            found.clear();
            nearest.clear();
            grid.search(e.at, &mut seen, query, &mut found, &mut stats, |new| {
                for &v in new.iter().filter(|&&v| v != u) {
                    let d = units[v]
                        .candidates
                        .iter()
                        .map(|c| travel_cost(e.at, c.at))
                        .fold(f64::INFINITY, f64::min);
                    nearest.push((d, v));
                }
                nearest.sort_by(by);
                nearest.truncate(k);
                if nearest.len() < k {
                    f64::INFINITY
                } else {
                    nearest[k - 1].0
                }
            });
            merged.extend_from_slice(&nearest);
        }
        merged.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.total_cmp(&b.0)));
        merged.dedup_by_key(|x| x.1);
        merged.sort_by(by);
        merged.truncate(k);
        let to_start = unit
            .candidates
            .iter()
            .map(|c| travel_cost(start, c.at))
            .fold(f64::INFINITY, f64::min);
        let start_near = merged.len() < k || merged.last().is_some_and(|x| to_start <= x.0);
        out.push((merged.iter().map(|x| x.1).collect(), start_near));
    }
    out
}

/// Prefix sums of a cost array in blocks of [`Sums::BLOCK`], so a change in
/// a window updates the window's blocks and the block offsets only.
#[derive(Default)]
struct Sums {
    /// The sum from the start of `k`'s block up to (not including) `k`.
    local: Vec<f64>,
    /// The sum of all blocks before block `b`.
    offset: Vec<f64>,
}

impl Sums {
    const BLOCK: usize = 256;

    /// The sum of `v[..k]`, for `k` up to `v.len()`.
    fn upto(&self, k: usize) -> f64 {
        self.offset[k / Self::BLOCK] + self.local[k]
    }

    /// Recompute after `v[lo..hi]` changed.
    fn update(&mut self, v: &[f64], lo: usize, hi: usize) {
        let n = v.len();
        let blocks = n / Self::BLOCK + 1;
        if self.local.len() != n + 1 {
            self.local = vec![0.0; n + 1];
            self.offset = vec![0.0; blocks + 1];
            return self.update(v, 0, n);
        }
        let first = lo / Self::BLOCK;
        let last = (hi.max(lo + 1) - 1).min(n.saturating_sub(1)) / Self::BLOCK;
        for b in first..=last.min(blocks - 1) {
            let start = b * Self::BLOCK;
            let end = (start + Self::BLOCK).min(n + 1);
            let mut acc = 0.0;
            for (local, &x) in self.local[start..end]
                .iter_mut()
                .zip(v[start..].iter().chain(std::iter::once(&0.0)))
            {
                *local = acc;
                acc += x;
            }
        }
        for b in first..blocks {
            let start = b * Self::BLOCK;
            let end = (start + Self::BLOCK).min(n);
            let block_sum = if start < n {
                self.local[end - 1] + v[end - 1]
            } else {
                0.0
            };
            self.offset[b + 1] = self.offset[b] + block_sum;
        }
    }
}

/// One kind of join value per tour position, with prefix sums.
#[derive(Default)]
struct Joins {
    /// `edge[k]`: the value into position k (from the start for k = 0).
    edge: Vec<f64>,
    /// `back[k]`: the value between positions k - 1 and k when both are
    /// reversed and run the other way (0 for k = 0).
    back: Vec<f64>,
    edge_sum: Sums,
    back_sum: Sums,
}

/// What a join is measured in: the [`cost`], or the cuts its travel
/// crosses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Cost,
    Crossings,
}

/// A range minimum over positions, for the 2-opt precedence check.
#[derive(Default)]
struct MinTree {
    size: usize,
    t: Vec<usize>,
}

impl MinTree {
    fn rebuild(&mut self, vals: &[usize]) {
        self.size = vals.len().next_power_of_two();
        self.t.clear();
        self.t.resize(2 * self.size, usize::MAX);
        self.t[self.size..self.size + vals.len()].copy_from_slice(vals);
        for k in (1..self.size).rev() {
            self.t[k] = self.t[2 * k].min(self.t[2 * k + 1]);
        }
    }

    /// Set position `k` to `v`. The walk up stops at the first node that
    /// keeps its value, so a window of unchanged values costs little.
    fn set(&mut self, k: usize, v: usize) {
        let mut k = k + self.size;
        if self.t[k] == v {
            return;
        }
        self.t[k] = v;
        while k > 1 {
            k >>= 1;
            let m = self.t[2 * k].min(self.t[2 * k + 1]);
            if self.t[k] == m {
                break;
            }
            self.t[k] = m;
        }
    }

    /// The least value at positions `l..=r`.
    fn min(&self, l: usize, r: usize) -> usize {
        let (mut l, mut r) = (l + self.size, r + self.size + 1);
        let mut m = usize::MAX;
        while l < r {
            if l & 1 == 1 {
                m = m.min(self.t[l]);
                l += 1;
            }
            if r & 1 == 1 {
                r -= 1;
                m = m.min(self.t[r]);
            }
            l >>= 1;
            r >>= 1;
        }
        m
    }
}

/// An improvement move: reverse `tour[i..=j]`, or carry the chain
/// `tour[i..i + len]` (reversed or not) to before position `slot`.
#[derive(Debug, Clone, Copy)]
enum Move {
    Reverse {
        i: usize,
        j: usize,
    },
    Shift {
        i: usize,
        len: usize,
        slot: usize,
        reversed: bool,
    },
}

/// The 2-opt and Or-opt local search over one tour. Every unit in the tour
/// has a finite entry, and the tour keeps precedence.
struct Search<'a> {
    units: &'a [Unit],
    tree: &'a ContainmentTree,
    s: &'a PipelineSettings,
    start: Point,
    stats: &'a mut ImproveStats,
    /// Every cut of the job, when crossings are scored.
    cuts: Option<&'a RefCell<CutIndex>>,
    /// Crossing tests not yet added to `stats`.
    crossing_tests: Cell<u64>,
    /// The change of the crossing count by the applied moves.
    crossed: i64,
    tour: Vec<Placement>,
    pos: Vec<usize>,
    /// The [`cost`] of each join.
    costs: Joins,
    /// The cuts each join's travel crosses (empty without `cuts`).
    crossings: Joins,
    /// The position of each position's parent (`usize::MAX` for a root).
    parent_at: MinTree,
    /// With `outlines_last`: the first root position.
    roots_from: Option<usize>,
}

impl<'a> Search<'a> {
    fn new(
        units: &'a [Unit],
        tree: &'a ContainmentTree,
        start: Point,
        s: &'a PipelineSettings,
        cuts: Option<&'a RefCell<CutIndex>>,
        tour: Vec<Placement>,
        stats: &'a mut ImproveStats,
    ) -> Self {
        let roots_from = s
            .strategy
            .outlines_last
            .then(|| tree.parent.iter().filter(|p| p.is_some()).count());
        let mut search = Self {
            units,
            tree,
            s,
            start,
            stats,
            cuts,
            crossing_tests: Cell::new(0),
            crossed: 0,
            pos: vec![0; units.len()],
            costs: Joins::default(),
            crossings: Joins::default(),
            parent_at: MinTree::default(),
            roots_from,
            tour,
        };
        search.rebuild();
        search
    }

    /// Fill the caches for the whole tour.
    fn rebuild(&mut self) {
        let n = self.tour.len();
        for joins in [&mut self.costs, &mut self.crossings] {
            joins.edge = vec![0.0; n];
            joins.back = vec![0.0; n];
        }
        self.parent_at.rebuild(&vec![usize::MAX; n]);
        self.refresh(0, n, &[]);
    }

    /// Update the caches after `tour[lo..hi]` was rewritten: the positions
    /// and edges there, the edge after it, the sums from `lo` on, and the
    /// parent positions of the window and of its units' children.
    ///
    /// `src` gives, for each new position in the window, the old position
    /// of its unit and whether it lies in a reversed run (empty: compute
    /// every edge). A pair that was adjacent before keeps its cached edge,
    /// and a reversed pair swaps its two directions, so only the joins are
    /// computed again. The values are the same as computed afresh.
    fn refresh(&mut self, lo: usize, hi: usize, src: &[(usize, bool)]) {
        let n = self.tour.len();
        for k in lo..hi {
            self.pos[self.tour[k].unit] = k;
        }
        let kinds: &[Kind] = if self.cuts.is_some() {
            &[Kind::Cost, Kind::Crossings]
        } else {
            &[Kind::Cost]
        };
        for &kind in kinds {
            let joins = self.joins(kind);
            let old_edge = joins.edge[lo..hi].to_vec();
            let old_back = joins.back[lo..hi].to_vec();
            for k in lo..(hi + 1).min(n) {
                let kept = (k > lo && k < hi && !src.is_empty())
                    .then(|| {
                        let ((a, ra), (b, rb)) = (src[k - 1 - lo], src[k - lo]);
                        if !ra && !rb && b == a + 1 {
                            Some((old_edge[b - lo], old_back[b - lo]))
                        } else if ra && rb && a == b + 1 {
                            Some((old_back[a - lo], old_edge[a - lo]))
                        } else {
                            None
                        }
                    })
                    .flatten();
                let (edge, back) = kept.unwrap_or_else(|| {
                    let prev = k.checked_sub(1).map(|j| self.tour[j]);
                    let back = match prev {
                        Some(p) => self.join(kind, Some(self.flip(self.tour[k])), self.flip(p)),
                        None => 0.0,
                    };
                    (self.join(kind, prev, self.tour[k]), back)
                });
                let joins = self.joins_mut(kind);
                joins.edge[k] = edge;
                joins.back[k] = back;
            }
            let changed = (hi + 1).min(n);
            let joins = self.joins_mut(kind);
            joins.edge_sum.update(&joins.edge, lo, changed);
            joins.back_sum.update(&joins.back, lo, changed);
        }
        for k in lo..hi {
            let u = self.tour[k].unit;
            let at = self.parent(u).map_or(usize::MAX, |q| self.pos[q]);
            self.parent_at.set(k, at);
            for &c in self.tree.children.get(u).into_iter().flatten() {
                self.parent_at.set(self.pos[c], k);
            }
        }
    }

    fn parent(&self, u: usize) -> Option<usize> {
        self.tree.parent.get(u).copied().flatten()
    }

    fn joins(&self, kind: Kind) -> &Joins {
        match kind {
            Kind::Cost => &self.costs,
            Kind::Crossings => &self.crossings,
        }
    }

    fn joins_mut(&mut self, kind: Kind) -> &mut Joins {
        match kind {
            Kind::Cost => &mut self.costs,
            Kind::Crossings => &mut self.crossings,
        }
    }

    /// The value of going on to `to` after `from` (the start for `None`).
    fn join(&self, kind: Kind, from: Option<Placement>, to: Placement) -> f64 {
        match (kind, self.cuts) {
            (Kind::Cost, _) => self.cost(from, to),
            (Kind::Crossings, None) => 0.0,
            (Kind::Crossings, Some(cuts)) => {
                let (head, _) = head_after(self.units, from, self.start);
                let at = self.units[to.unit].candidates[to.candidate].at;
                let mut cs = ConstructStats::default();
                let n = cuts.borrow_mut().crossings(head, at, to.unit, &mut cs);
                self.crossing_tests
                    .set(self.crossing_tests.get() + cs.crossing_tests);
                n as f64
            }
        }
    }

    /// Add the pending crossing tests to the stats.
    fn flush(&mut self) {
        self.stats.crossing_tests += self.crossing_tests.take();
    }

    /// `p` cut the other way: an open path from its other end. A ring
    /// keeps its entry and direction.
    fn flip(&self, p: Placement) -> Placement {
        let u = &self.units[p.unit];
        if !u.closed && u.candidates.len() == 2 {
            Placement {
                unit: p.unit,
                candidate: 1 - p.candidate,
            }
        } else {
            p
        }
    }

    fn flippable(&self, p: Placement) -> bool {
        self.flip(p) != p
    }

    /// The cost of going on to `to` after `from` (the start for `None`).
    fn cost(&self, from: Option<Placement>, to: Placement) -> f64 {
        let (head, heading) = match from {
            None => (self.start, self.s.start_heading),
            Some(p) => {
                let e = &self.units[p.unit].candidates[p.candidate];
                (e.exit, e.exit_heading)
            }
        };
        let u = &self.units[to.unit];
        cost(head, heading, u, &u.candidates[to.candidate], self.s)
    }

    fn prev(&self, i: usize) -> Option<Placement> {
        i.checked_sub(1).map(|k| self.tour[k])
    }

    /// The least gain a move must show. The delta comes from prefix sums,
    /// whose rounding grows with the tour; above this bound a move that
    /// looks better is better.
    fn min_gain(&self) -> f64 {
        let n = self.tour.len();
        let total = self.costs.edge_sum.upto(n).abs() + self.costs.back_sum.upto(n).abs();
        MIN_GAIN_MM.max(4.0 * n as f64 * f64::EPSILON * total)
    }

    /// Count one evaluation, or false when the budget is spent.
    fn spend(&mut self) -> bool {
        if self.stats.evaluations >= self.s.strategy.budget {
            return false;
        }
        self.stats.evaluations += 1;
        true
    }

    /// The zone of position k: false for inner units, true for roots.
    fn zone(&self, k: usize) -> bool {
        self.roots_from.is_some_and(|m| k >= m)
    }

    fn legal(&self, m: Move) -> bool {
        let pos_of = |u: usize| self.pos[u];
        match m {
            Move::Reverse { i, j } => {
                // No unit in the run may have its parent in the run.
                self.zone(i) == self.zone(j) && self.parent_at.min(i, j) > j
            }
            Move::Shift {
                i,
                len,
                slot,
                reversed,
            } => {
                let end = i + len;
                if self.zone(i) != self.zone(end - 1) {
                    return false;
                }
                if let Some(roots) = self.roots_from {
                    if slot > end && end - 1 < roots && slot > roots {
                        return false;
                    }
                    if slot < i && i >= roots && slot < roots {
                        return false;
                    }
                }
                self.tour[i..end].iter().all(|c| {
                    let parent_in_chain = self
                        .parent(c.unit)
                        .map(pos_of)
                        .is_some_and(|q| (i..end).contains(&q));
                    if reversed && parent_in_chain {
                        return false;
                    }
                    if slot > end {
                        // Moved later: its parent must not be passed.
                        self.parent(c.unit)
                            .map(pos_of)
                            .is_none_or(|q| q < end || q >= slot)
                    } else {
                        // Moved earlier: no passed unit may be its child.
                        self.tree
                            .children
                            .get(c.unit)
                            .is_none_or(|ch| ch.iter().all(|&x| !(slot..i).contains(&pos_of(x))))
                    }
                })
            }
        }
    }

    /// The change of `kind` under `m`, from the cached joins and the new
    /// joins.
    fn delta(&self, kind: Kind, m: Move) -> f64 {
        let n = self.tour.len();
        let joins = self.joins(kind);
        match m {
            Move::Reverse { i, j } => {
                let mut old = joins.edge_sum.upto(j + 1) - joins.edge_sum.upto(i);
                let mut new = self.join(kind, self.prev(i), self.flip(self.tour[j]))
                    + (joins.back_sum.upto(j + 1) - joins.back_sum.upto(i + 1));
                if j + 1 < n {
                    old += joins.edge[j + 1];
                    new += self.join(kind, Some(self.flip(self.tour[i])), self.tour[j + 1]);
                }
                new - old
            }
            Move::Shift {
                i,
                len,
                slot,
                reversed,
            } => {
                let end = i + len;
                let (first, last, inner) = if reversed {
                    (
                        self.flip(self.tour[end - 1]),
                        self.flip(self.tour[i]),
                        joins.back_sum.upto(end) - joins.back_sum.upto(i + 1),
                    )
                } else {
                    (
                        self.tour[i],
                        self.tour[end - 1],
                        joins.edge_sum.upto(end) - joins.edge_sum.upto(i + 1),
                    )
                };
                let mut old = joins.edge_sum.upto(end) - joins.edge_sum.upto(i);
                let mut new = self.join(kind, self.prev(slot), first) + inner;
                if end < n {
                    old += joins.edge[end];
                    new += self.join(kind, self.prev(i), self.tour[end]);
                }
                if slot < n {
                    old += joins.edge[slot];
                    new += self.join(kind, Some(last), self.tour[slot]);
                }
                new - old
            }
        }
    }

    /// The window `lo..hi` the move rewrites, and its new contents: each
    /// placement with its old position and whether it is in a reversed run.
    fn rewrite(&self, m: Move) -> (usize, usize, Vec<(Placement, usize, bool)>) {
        let kept = |k: usize| (self.tour[k], k, false);
        let turned = |k: usize| (self.flip(self.tour[k]), k, true);
        match m {
            Move::Reverse { i, j } => (i, j + 1, (i..=j).rev().map(turned).collect()),
            Move::Shift {
                i,
                len,
                slot,
                reversed,
            } => {
                let end = i + len;
                let chain: Vec<_> = if reversed {
                    (i..end).rev().map(turned).collect()
                } else {
                    (i..end).map(kept).collect()
                };
                if slot < i {
                    (
                        slot,
                        end,
                        chain.into_iter().chain((slot..i).map(kept)).collect(),
                    )
                } else {
                    (i, slot, (end..slot).map(kept).chain(chain).collect())
                }
            }
        }
    }

    /// Evaluate `m` and apply it when it gains. `None` when the budget is
    /// spent; otherwise whether it was applied, with the units whose edges
    /// changed pushed onto `touched`.
    fn attempt(&mut self, m: Move, touched: &mut Vec<usize>) -> Option<bool> {
        if !self.legal(m) {
            return Some(false);
        }
        if !self.spend() {
            return None;
        }
        let gain = -self.delta(Kind::Cost, m);
        if gain <= self.min_gain() {
            return Some(false);
        }
        // Only a move that passed the screen pays for its crossing tests.
        if self.cuts.is_some() {
            let crossed = self.delta(Kind::Crossings, m);
            if gain - CROSSING_PENALTY_MM * crossed <= self.min_gain() {
                self.stats.crossing_rejections += 1;
                return Some(false);
            }
            // Sums of whole counts: exact.
            self.crossed += crossed as i64;
        }
        let (lo, hi, seq) = self.rewrite(m);
        touched.clear();
        let around = [lo.checked_sub(1), Some(lo), Some(hi - 1), Some(hi)];
        let mut ends: Vec<usize> = around.into_iter().flatten().collect();
        if let Move::Shift { i, len, .. } = m {
            ends.extend(
                [i.checked_sub(1), Some(i), Some(i + len - 1), Some(i + len)]
                    .into_iter()
                    .flatten(),
            );
        }
        touched.extend(
            ends.into_iter()
                .filter_map(|k| self.tour.get(k))
                .map(|p| p.unit),
        );
        for (k, &(p, _, _)) in seq.iter().enumerate() {
            self.tour[lo + k] = p;
        }
        match m {
            Move::Reverse { .. } => self.stats.two_opt_moves += 1,
            Move::Shift { .. } => self.stats.or_opt_moves += 1,
        }
        let src: Vec<(usize, bool)> = seq.iter().map(|&(_, at, rev)| (at, rev)).collect();
        self.refresh(lo, hi, &src);
        Some(true)
    }

    /// Try the moves of unit `u` until one gains (first improvement).
    fn improve_unit(
        &mut self,
        u: usize,
        neighbours: &(Vec<usize>, bool),
        touched: &mut Vec<usize>,
    ) -> Option<bool> {
        let n = self.tour.len();
        let x = self.pos[u];
        // 2-opt: reverse a run so u and a neighbour (or the start) meet.
        let mut runs: Vec<(usize, usize)> = Vec::new();
        if neighbours.1 && x > 0 {
            runs.push((0, x));
        }
        for &v in &neighbours.0 {
            let y = self.pos[v];
            let (lo, hi) = (x.min(y), x.max(y));
            runs.push((lo + 1, hi));
            runs.push((lo, hi - 1));
        }
        for (i, j) in runs {
            if i > j || (i == j && !self.flippable(self.tour[i])) {
                continue;
            }
            if self.attempt(Move::Reverse { i, j }, touched)? {
                return Some(true);
            }
        }
        if !self.s.strategy.or_opt {
            return Some(false);
        }
        // Or-opt: carry the chain from u next to a neighbour (or the start).
        for len in 1..=OR_OPT_CHAIN.min(n - x) {
            let mut slots: Vec<usize> = Vec::new();
            if neighbours.1 {
                slots.push(0);
            }
            for &v in &neighbours.0 {
                let y = self.pos[v];
                slots.extend([y + 1, y]);
            }
            for slot in slots {
                if (x..=x + len).contains(&slot) {
                    continue;
                }
                for reversed in [false, true] {
                    if reversed && len == 1 && !self.flippable(self.tour[x]) {
                        continue;
                    }
                    let m = Move::Shift {
                        i: x,
                        len,
                        slot,
                        reversed,
                    };
                    if self.attempt(m, touched)? {
                        return Some(true);
                    }
                }
            }
        }
        Some(false)
    }

    /// Run until no unit's moves gain. False when the budget ran out.
    fn run(&mut self, neighbours: &[(Vec<usize>, bool)]) -> bool {
        let mut queue: VecDeque<usize> = self.tour.iter().map(|p| p.unit).collect();
        let mut queued = vec![false; self.units.len()];
        for &u in &queue {
            queued[u] = true;
        }
        let mut touched = Vec::new();
        while let Some(u) = queue.pop_front() {
            queued[u] = false;
            match self.improve_unit(u, &neighbours[u], &mut touched) {
                None => return false,
                Some(false) => {}
                Some(true) => {
                    for &t in touched.iter().chain(std::iter::once(&u)) {
                        if !queued[t] {
                            queued[t] = true;
                            queue.push_back(t);
                        }
                    }
                }
            }
        }
        true
    }
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

    /// Push onto `found` the ready units around `head`, ring by ring, and
    /// hand each ring's new units to `score`, which returns the best cost
    /// so far. The search stops before a ring whose cells all lie at least
    /// that far away, so no cheaper unit is missed. `seen` and `query` keep
    /// each unit once.
    fn search(
        &self,
        head: Point,
        seen: &mut [u32],
        query: u32,
        found: &mut Vec<usize>,
        stats: &mut ConstructStats,
        mut score: impl FnMut(&[usize]) -> f64,
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
        let mut best = f64::INFINITY;
        let mut scored_up_to = 0;
        for k in k_min..=k_max {
            // A cell k rings away lies more than k - 1 cells from the head
            // (Chebyshev), and every cost is at least the travel.
            if k >= 1 && best <= (k - 1) as f64 * self.cell {
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
            if found.len() > scored_up_to {
                best = score(&found[scored_up_to..]);
                scored_up_to = found.len();
            }
        }
    }
}

/// The finished cuts, in [`GRID_CELL_MM`] cells, for the crossing test.
#[derive(Debug, Default)]
struct CutIndex {
    /// Each cut and the unit it belongs to.
    segs: Vec<(Point, Point, usize)>,
    cells: HashMap<(i64, i64), Vec<u32>>,
    stamp: Vec<u32>,
    query: u32,
    /// The box of every indexed cut.
    bounds: Option<Rect>,
    /// When set, the position of each unit in a tour: a travel into unit
    /// `to` counts only the cuts of units ranked before `to`, the cuts
    /// finished by then. Empty: every indexed cut counts.
    rank: Vec<usize>,
}

/// The part of `a`-`b` inside `r` (Liang-Barsky), or `None` when the
/// segment misses `r`. An end inside `r` is kept as it is.
fn clip(a: Point, b: Point, r: Rect) -> Option<(Point, Point)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [
        (-d.x, a.x - r.x0),
        (d.x, r.x1 - a.x),
        (-d.y, a.y - r.y0),
        (d.y, r.y1 - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    if t0 > t1 {
        return None;
    }
    let at = |t: f64| {
        if t <= 0.0 {
            a
        } else if t >= 1.0 {
            b
        } else {
            a + d * t
        }
    };
    Some((at(t0), at(t1)))
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
    /// Count only finished cuts from now on, by the positions in `tour`
    /// (a unit not in `tour` is never finished).
    fn rank_by(&mut self, tour: &[Placement], units: usize) {
        self.rank.clear();
        self.rank.resize(units, usize::MAX);
        for (k, p) in tour.iter().enumerate() {
            if let Some(r) = self.rank.get_mut(p.unit) {
                *r = k;
            }
        }
    }

    fn insert(&mut self, a: Point, b: Point, unit: usize) {
        if !(a.is_finite() && b.is_finite()) {
            return;
        }
        let id = self.segs.len() as u32;
        self.segs.push((a, b, unit));
        self.stamp.push(0);
        let r = Rect::from_points(a, b);
        self.bounds = Some(self.bounds.map_or(r, |old| old.union(r)));
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

    /// How many cuts the travel `a`-`b` into unit `to` properly crosses,
    /// away from its ends. The cuts of `to` itself never count: they are
    /// not cut yet when the head travels to it. With a rank, only the cuts
    /// of units ranked before `to` count. Only the part of the travel inside the cuts' box
    /// walks the cells, so a travel from a far start stays cheap.
    fn crossings(&mut self, a: Point, b: Point, to: usize, stats: &mut ConstructStats) -> usize {
        if !(a.is_finite() && b.is_finite()) {
            return 0;
        }
        let Some(bounds) = self.bounds else {
            return 0;
        };
        let Some((ca, cb)) = clip(a, b, bounds.inflate(1.0, 1.0)) else {
            return 0;
        };
        self.query += 1;
        let query = self.query;
        let mut count = 0;
        let (segs, cells, stamp) = (&self.segs, &self.cells, &mut self.stamp);
        let rank = &self.rank;
        let limit = rank.get(to).copied().unwrap_or(usize::MAX);
        let finished = |unit: usize| {
            unit != to && (rank.is_empty() || rank.get(unit).is_some_and(|&r| r < limit))
        };
        cells_along(ca, cb, |c| {
            for &id in cells.get(&c).into_iter().flatten() {
                let id = id as usize;
                if stamp[id] != query {
                    stamp[id] = query;
                    stats.crossing_tests += 1;
                    let (q1, q2, unit) = segs[id];
                    count += usize::from(finished(unit) && crosses(a, b, q1, q2));
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
    fn a_far_waste_corner_does_not_hide_a_nearer_unit() {
        // The head is 1 mm from the L's concave corner, which is on the kept
        // side (+1000). The L's nearest waste corner is 51 mm away, a
        // square's corner 15 mm. The search must keep going past the rings
        // around the concave corner and find the square.
        let s = settings(CutStrategy::Quality);
        let l = tp(
            1,
            &[
                (0.0, 0.0),
                (200.0, 0.0),
                (200.0, 50.0),
                (50.0, 50.0),
                (50.0, 200.0),
                (0.0, 200.0),
                (0.0, 0.0),
            ],
            true,
        );
        let square = ring(2, 66.0, 66.0, 76.0, 76.0);
        let out = run(vec![l, square], p(51.0, 51.0), &s);
        assert_eq!(ids(&out), vec![2, 1]);
        assert_eq!(out[0].points[0], p(66.0, 66.0));
        // The L is still entered at a waste corner afterwards.
        assert_ne!(out[1].points[0], p(50.0, 50.0));
    }

    #[test]
    fn the_fallback_never_takes_a_parent_before_its_child() {
        // The child has no entry point, so the grid never finds it; the
        // parent waits for it. The fallback must place the child first.
        let s = settings(CutStrategy::Quality);
        let (paths, _) = prepared(vec![ring(1, 0.0, 0.0, 10.0, 10.0)]);
        let units = vec![
            Unit::new(&paths[0], &s),
            Unit {
                points: Vec::new(),
                closed: false,
                candidates: Vec::new(),
                has_waste_corner: false,
            },
        ];
        let tree = ContainmentTree {
            parent: vec![None, Some(0)],
            children: vec![vec![1], vec![]],
            roots: vec![0],
            depth: vec![0, 1],
            own_depth: vec![0, 1],
        };
        let tour = construct(&units, &tree, Point::ZERO, &s);
        let order: Vec<usize> = tour.iter().map(|pl| pl.unit).collect();
        assert_eq!(order, vec![1, 0]);
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
    /// Measured 0.33 to 0.40 s locally with the exact stop rule (Apple
    /// silicon, test profile, on a busy machine), so the bound is 1.2 s,
    /// not the plan's 3 s. It times the construction alone; the
    /// improvement has its own timing test.
    #[test]
    #[ignore = "wall-clock perf check, run by hand or by the quality bench"]
    fn nn_of_20000_squares_is_fast_in_debug() {
        let s = settings(CutStrategy::Quality);
        let (paths, tree) = prepared(random_squares(20_000));
        let units: Vec<Unit> = paths.iter().map(|t| Unit::new(t, &s)).collect();
        let t0 = std::time::Instant::now();
        let out = construct(&units, &tree, Point::ZERO, &s);
        let secs = t0.elapsed().as_secs_f64();
        eprintln!("nn of 20000 squares: {secs} s");
        assert_eq!(out.len(), 20_000);
        assert!(secs < 1.2, "{secs} s");
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

    fn placements(units: &[usize]) -> Vec<Placement> {
        units
            .iter()
            .map(|&unit| Placement { unit, candidate: 0 })
            .collect()
    }

    fn unit_order(tour: &[Placement]) -> Vec<usize> {
        tour.iter().map(|pl| pl.unit).collect()
    }

    #[test]
    fn two_opt_fixes_a_crossing_tour() {
        // Four short lines at the corners of a tall box. The tour A, C, B,
        // D runs A->C and B->D, which cross; 2-opt uncrosses it.
        let line = |id: u64, x: f64, y: f64| tp(id, &[(x, y), (x + 0.1, y)], false);
        let paths = vec![
            line(0, 10.0, 0.0),
            line(1, 20.0, 0.0),
            line(2, 20.0, 50.0),
            line(3, 10.0, 50.0),
        ];
        let (paths, tree) = prepared(paths);
        let mut s = settings(CutStrategy::Fast);
        s.strategy.or_opt = false;
        s.strategy.entry_dp = false;
        let units: Vec<Unit> = paths.iter().map(|t| Unit::new(t, &s)).collect();
        let mut tour = placements(&[0, 2, 1, 3]);
        let before = tour_cost(&units, &tour, Point::ZERO, &s);
        let stats = improve(&units, &mut tour, &tree, Point::ZERO, &s);
        assert_eq!(unit_order(&tour), vec![0, 1, 2, 3]);
        assert!(stats.two_opt_moves >= 1, "{stats:?}");
        assert_eq!(stats.or_opt_moves, 0);
        let after = tour_cost(&units, &tour, Point::ZERO, &s);
        assert!(after < before - 50.0, "{before} -> {after}");
    }

    /// Three pieces with a hole each on a row, and a stray square far
    /// below the row, cut between the first and second piece. Every 2-opt
    /// run that could carry the stray holds a hole and its outline, so only
    /// Or-opt can move it.
    fn stray_job() -> (Vec<Unit>, ContainmentTree, Vec<Placement>, PipelineSettings) {
        let mut paths = Vec::new();
        for (k, x) in [20.0, 40.0, 60.0].into_iter().enumerate() {
            paths.push(ring(2 * k as u64, x - 2.0, -2.0, x + 2.0, 2.0));
            paths.push(ring(2 * k as u64 + 1, x - 5.0, -5.0, x + 5.0, 5.0));
        }
        paths.push(ring(6, 38.0, 98.0, 42.0, 102.0));
        let (paths, tree) = prepared(paths);
        let mut s = settings(CutStrategy::Fast);
        s.strategy.entry_dp = false;
        let units: Vec<Unit> = paths.iter().map(|t| Unit::new(t, &s)).collect();
        let tour = placements(&[0, 1, 6, 2, 3, 4, 5]);
        assert!(respects_precedence(&tour, &tree, false));
        (units, tree, tour, s)
    }

    #[test]
    fn or_opt_moves_a_stray_unit() {
        let (units, tree, mut tour, s) = stray_job();
        let before = tour_cost(&units, &tour, Point::ZERO, &s);
        let stats = improve(&units, &mut tour, &tree, Point::ZERO, &s);
        assert!(stats.or_opt_moves >= 1, "{stats:?}");
        assert_eq!(unit_order(&tour), vec![0, 1, 2, 3, 4, 5, 6]);
        assert!(respects_precedence(&tour, &tree, false));
        let after = tour_cost(&units, &tour, Point::ZERO, &s);
        assert!(after < before - 50.0, "{before} -> {after}");
    }

    #[test]
    fn or_opt_off_skips_or_opt() {
        // Ruling F6: the same job without Or-opt keeps the stray.
        let (units, tree, mut tour, mut s) = stray_job();
        s.strategy.or_opt = false;
        let stats = improve(&units, &mut tour, &tree, Point::ZERO, &s);
        assert_eq!(stats.or_opt_moves, 0, "{stats:?}");
        assert_eq!(unit_order(&tour), vec![0, 1, 6, 2, 3, 4, 5]);
    }

    /// Six squares of different sizes on a zigzag, every corner a waste
    /// corner, then an L piece (its concave corner is on the kept side, so
    /// entering there pays the kept-side penalty) and an open line, in a
    /// fixed order.
    fn dp_job() -> (Vec<Unit>, Vec<Placement>, PipelineSettings) {
        let s = settings(CutStrategy::Quality);
        let mut units: Vec<Unit> = (0..6)
            .map(|k| {
                let (x, y) = (15.0 * k as f64, if k % 2 == 0 { 0.0 } else { 12.0 });
                let side = 3.0 + k as f64;
                let mut t = ring(k as u64, x, y, x + side, y + side);
                t.keep = KeepSide::Inside;
                Unit::new(&t, &s)
            })
            .collect();
        for u in &units {
            assert_eq!(u.candidates.len(), 4);
            assert!(u.candidates.iter().all(|c| c.waste_corner));
        }
        let l_pts = [
            (90.0, 0.0),
            (100.0, 0.0),
            (100.0, 4.0),
            (94.0, 4.0),
            (94.0, 10.0),
            (90.0, 10.0),
            (90.0, 0.0),
        ];
        let mut l = tp(6, &l_pts, true);
        l.keep = KeepSide::Inside;
        let l = Unit::new(&l, &s);
        assert_eq!(l.candidates.len(), 6);
        assert_eq!(l.candidates.iter().filter(|c| !c.waste_corner).count(), 1);
        units.push(l);
        let line = Unit::new(&tp(7, &[(105.0, 12.0), (112.0, 15.0)], false), &s);
        assert_eq!(line.candidates.len(), 2);
        units.push(line);
        (units, placements(&[0, 1, 2, 3, 4, 5, 6, 7]), s)
    }

    #[test]
    fn entry_dp_is_exact() {
        let (units, mut tour, s) = dp_job();
        let start = p(-5.0, 3.0);
        let counts: Vec<usize> = units.iter().map(|u| u.candidates.len()).collect();
        let mut best = f64::INFINITY;
        let mut trial = tour.clone();
        // Brute force: 4^6 for the squares, times 6 for the L and 2 for
        // the line.
        let all: usize = counts.iter().product();
        assert_eq!(all, 4usize.pow(6) * 6 * 2);
        for code in 0..all {
            let mut rest = code;
            for (pl, &c) in trial.iter_mut().zip(&counts) {
                pl.candidate = rest % c;
                rest /= c;
            }
            best = best.min(tour_cost(&units, &trial, start, &s));
        }
        let first = tour_cost(&units, &tour, start, &s);
        assert!(first > best + 1.0, "not vacuous: {first} vs {best}");
        let evaluations = entry_dp(&units, &mut tour, start, &s);
        let edges = counts[0] + counts.windows(2).map(|w| w[0] * w[1]).sum::<usize>();
        assert_eq!(evaluations, edges as u64);
        assert_eq!(evaluations, 4 + 5 * 16 + 4 * 6 + 6 * 2);
        let got = tour_cost(&units, &tour, start, &s);
        assert!((got - best).abs() < 1e-9, "{got} vs {best}");
        assert_eq!(unit_order(&tour), vec![0, 1, 2, 3, 4, 5, 6, 7]);
        // The L is not entered at its kept-side corner.
        assert!(units[6].candidates[tour[6].candidate].waste_corner);
    }

    #[test]
    fn entry_dp_off_keeps_the_entries() {
        // Ruling F6. The order is already right, so only the DP can lower
        // the cost, by changing entries.
        let (units, tour, mut s) = dp_job();
        let n = units.len();
        let tree = ContainmentTree {
            parent: vec![None; n],
            children: vec![Vec::new(); n],
            roots: (0..n).collect(),
            depth: vec![0; n],
            own_depth: vec![0; n],
        };
        let start = p(-5.0, 3.0);
        let mut on = tour.clone();
        let stats = improve(&units, &mut on, &tree, start, &s);
        assert_eq!(stats.entry_dp_changes, 1, "{stats:?}");
        assert_ne!(on, tour);
        s.strategy.entry_dp = false;
        let mut off = tour.clone();
        let stats = improve(&units, &mut off, &tree, start, &s);
        assert_eq!(stats.entry_dp_changes, 0, "{stats:?}");
        let entries = |t: &[Placement]| t.iter().map(|pl| pl.candidate).collect::<Vec<_>>();
        assert_eq!(entries(&off), entries(&tour));
    }

    #[test]
    fn order_budget_bounds_large_jobs() {
        // The guard is the evaluation count (Ruling F30); the timing twin
        // below is ignored. The crossing tests are paid only per move that
        // passed the cost screen, per DP run and per round's join cache:
        // measured 711k (142 n) here and 4.1M (207 n, the budget binds)
        // for 20000 squares. One test per evaluation would be about 2700 n.
        let n = 5000;
        let s = settings(CutStrategy::Quality);
        let (paths, tree) = prepared(random_squares(n));
        let units: Vec<Unit> = paths.iter().map(|t| Unit::new(t, &s)).collect();
        let mut tour = construct(&units, &tree, Point::ZERO, &s);
        let before = tour_cost(&units, &tour, Point::ZERO, &s);
        let stats = improve(&units, &mut tour, &tree, Point::ZERO, &s);
        let after = tour_cost(&units, &tour, Point::ZERO, &s);
        eprintln!("{stats:?}: {before} -> {after}");
        assert!(stats.evaluations <= BUDGET, "{stats:?}");
        assert!(stats.crossing_tests <= 300 * n as u64, "{stats:?}");
        assert!(stats.crossing_rejections > 0, "{stats:?}");
        assert!(respects_precedence(&tour, &tree, false));
        assert!(after < before, "{before} -> {after}");
    }

    /// Wall-clock check (Ruling F30): construct plus improve of 5000
    /// squares in a debug build, bound 5 s from the plan.
    #[test]
    #[ignore = "wall-clock perf check, run by hand or by the quality bench"]
    fn order_of_5000_squares_improves_in_under_5_s() {
        let s = settings(CutStrategy::Quality);
        let (paths, tree) = prepared(random_squares(5000));
        let t0 = std::time::Instant::now();
        let out = order(paths, &tree, Point::ZERO, &s);
        let secs = t0.elapsed().as_secs_f64();
        eprintln!("order of 5000 squares with improvement: {secs} s");
        assert_eq!(out.len(), 5000);
        assert!(secs < 5.0, "{secs} s");
    }

    #[test]
    fn a_small_budget_stops_early_and_keeps_a_valid_tour() {
        let mut s = settings(CutStrategy::Quality);
        s.strategy.budget = 50;
        let (paths, tree) = prepared(random_squares(300));
        let units: Vec<Unit> = paths.iter().map(|t| Unit::new(t, &s)).collect();
        let mut tour = construct(&units, &tree, Point::ZERO, &s);
        let before = tour_cost(&units, &tour, Point::ZERO, &s);
        let stats = improve(&units, &mut tour, &tree, Point::ZERO, &s);
        assert_eq!(stats.evaluations, 50, "{stats:?}");
        assert_eq!(stats.entry_dp_changes, 0, "the DP does not fit");
        assert!(respects_precedence(&tour, &tree, false));
        assert!(tour_cost(&units, &tour, Point::ZERO, &s) <= before);
    }

    /// Rings, open lines and L pieces (kind 0, 1 and 2) in random boxes.
    fn shapes() -> impl Strategy<Value = Vec<(f64, f64, f64, f64, u8)>> {
        proptest::collection::vec(
            (
                0.0f64..200.0,
                0.0f64..200.0,
                1.0f64..120.0,
                1.0f64..120.0,
                prop_oneof![6 => Just(0u8), 2 => Just(1u8), 2 => Just(2u8)],
            ),
            1..40,
        )
    }

    #[test]
    fn tour_crossings_counts_only_finished_cuts() {
        // A square and a short line to its right, from a start on its left.
        let s = settings(CutStrategy::Quality);
        let (paths, _) = prepared(vec![
            ring(0, 0.0, 0.0, 10.0, 10.0),
            tp(1, &[(15.0, 5.0), (16.0, 5.0)], false),
        ]);
        let units: Vec<Unit> = paths.iter().map(|t| Unit::new(t, &s)).collect();
        let left = units[0]
            .candidates
            .iter()
            .position(|e| e.at.x == 0.0 && e.exit.x == 0.0)
            .unwrap();
        let start = p(-5.0, 5.0);
        let square = Placement {
            unit: 0,
            candidate: left,
        };
        let line = Placement {
            unit: 1,
            candidate: 0,
        };
        // The line first: the travel over the square crosses nothing cut.
        assert_eq!(tour_crossings(&units, &[line, square], start), 0);
        // The square first: the travel to the line crosses its right side.
        assert_eq!(tour_crossings(&units, &[square, line], start), 1);
        // The improvement's term counts the square either way.
        assert_eq!(scored_crossings(&units, &[line, square], start), 2);
    }

    /// The crossing term [`improve`] scores: every cut except the target
    /// unit's own, finished or not.
    fn scored_crossings(units: &[Unit], tour: &[Placement], start: Point) -> usize {
        let mut cuts = all_cuts(units);
        let mut stats = ConstructStats::default();
        let (mut head, mut count) = (start, 0);
        for pl in tour {
            if let Some(e) = units[pl.unit].candidates.get(pl.candidate) {
                count += cuts.crossings(head, e.at, pl.unit, &mut stats);
                head = e.exit;
            }
        }
        count
    }

    proptest! {
        #[test]
        fn improvement_never_increases_cost_and_keeps_precedence(
            shapes in shapes(),
            strategy in 0usize..3,
            start in (-50.0f64..250.0, -50.0f64..250.0),
        ) {
            let paths: Vec<Toolpath> = shapes
                .iter()
                .enumerate()
                .map(|(i, &(x, y, w, h, kind))| match kind {
                    0 => ring(i as u64, x, y, x + w, y + h),
                    1 => tp(i as u64, &[(x, y), (x + w / 4.0, y + h / 4.0)], false),
                    _ => tp(
                        i as u64,
                        &[
                            (x, y),
                            (x + w, y),
                            (x + w, y + h / 2.0),
                            (x + w / 2.0, y + h / 2.0),
                            (x + w / 2.0, y + h),
                            (x, y + h),
                            (x, y),
                        ],
                        true,
                    ),
                })
                .collect();
            let (paths, tree) = prepared(paths);
            let strategy = [CutStrategy::Quality, CutStrategy::Fast, CutStrategy::KeepSheet][strategy];
            let s = settings(strategy);
            let start = p(start.0, start.1);
            let units: Vec<Unit> = paths.iter().map(|t| Unit::new(t, &s)).collect();
            let built = construct(&units, &tree, start, &s);
            let before = tour_cost(&units, &built, start, &s);
            let mut tour = built.clone();
            let stats = improve(&units, &mut tour, &tree, start, &s);
            prop_assert!(stats.evaluations <= s.strategy.budget);
            prop_assert!(respects_precedence(&tour, &tree, s.strategy.outlines_last));
            for pl in &tour {
                prop_assert!(pl.candidate < units[pl.unit].candidates.len());
            }
            let after = tour_cost(&units, &tour, start, &s);
            prop_assert!(after <= before + 1e-9, "{} -> {}", before, after);
            // With the crossing term it scores too.
            let full = |t: &[Placement], cost: f64| {
                cost + CROSSING_PENALTY_MM * scored_crossings(&units, t, start) as f64
            };
            let (full_before, full_after) = (full(&built, before), full(&tour, after));
            prop_assert!(full_after <= full_before + 1e-9, "{} -> {}", full_before, full_after);
            // The same input gives the same tour.
            let mut again = built.clone();
            improve(&units, &mut again, &tree, start, &s);
            prop_assert_eq!(again, tour);
        }
    }
}
