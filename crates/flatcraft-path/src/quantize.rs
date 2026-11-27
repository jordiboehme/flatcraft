//! Rounding host-compensated axis runs to the device units.
//!
//! The device takes whole units (20 per mm on the Cameo 2), so every axis
//! point moves by up to half a unit on each axis. A pivot arc of radius d
//! is only a few units across, and rounding its points can make a move
//! that runs back at the trailing tip: a pushed blade, which tears the
//! material. Where the knife stage knows the device units
//! ([`crate::HostSettings::device_grid`]) it rounds the runs itself and
//! drops each rounded point whose move would push the blade, using the
//! drag knife model ([`crate::model`]) to follow the tip.

use kurbo::{Point, Vec2};

use crate::containment::KeptMaterial;
use crate::model::{PUSHED_DEG, drag, push_angle_deg};
use crate::plan::{ASSUMED_START_HEADING, AxisRun, Toolpath, WASTE_SEG};

pub use flatcraft_core::device_grid::DeviceGrid;

/// The farthest a dropped axis point may lie from the rounded run that
/// replaces it, in millimetres (not in units): one device unit on the
/// Cameo 2, and within `flatcraft_sim::DEVICE_LIMIT_MM` (which this crate
/// cannot see) for any plotter. On a plotter with coarser units the bound
/// stays 0.05 mm, so fewer points can be dropped there.
pub const MAX_DROP_MM: f64 = 0.05;

/// How deep a rounded waste move's tip may go into kept material: inside
/// kept material and farther than this from every cut is a cut into it.
/// The simulator's device mode allows one device unit
/// (`flatcraft_sim::DEVICE_WASTE_TOLERANCE_MM`, 0.05 mm, which this crate
/// cannot see); the rounding keeps 0.01 mm inside that, so its own tip
/// samples and the simulator's need not fall on the same spots.
pub const KEPT_TOLERANCE_MM: f64 = 0.04;

/// The tip is followed in steps of at most `d` over this along a waste
/// move, as the simulator samples it (`flatcraft_sim::SAMPLE_STEP_FACTOR`).
const TIP_STEPS_PER_D: f64 = 20.0;

/// Rounding slack on [`MAX_DROP_MM`]: a point one unit off its chord lies
/// 0.05 mm away give or take the last bits.
const DROP_EPS: f64 = 1e-9;

/// What [`round_runs`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct RoundReport {
    /// Points dropped because their move would push the blade.
    pub dropped: usize,
    /// The largest distance of a dropped (planned) point from the rounded
    /// run that replaces it; at most [`MAX_DROP_MM`].
    pub max_drop_mm: f64,
    /// Moves that still push the blade: dropping their point would have
    /// moved the cut more than [`MAX_DROP_MM`]. The simulator reports them
    /// as pushed.
    pub pushes_kept: usize,
    /// Waste points (drops, lead-ins, swivels, overshoots) whose nearest
    /// unit point would take the tip into kept material and that went to
    /// another unit point round them instead.
    pub kept_moved: usize,
    /// Waste points no unit point round them keeps out of kept material
    /// (deeper than [`KEPT_TOLERANCE_MM`]): the one that takes the tip
    /// least deep was taken. Also a deep point where all four unit points
    /// push, and a waste end whose step got longer when the points before
    /// it were dropped. The job warns about them
    /// (`flatcraft_engine::job`).
    pub kept_left: usize,
}

impl RoundReport {
    /// The reports of two plans of one job, as one.
    pub fn merge(self, other: Self) -> Self {
        Self {
            dropped: self.dropped + other.dropped,
            max_drop_mm: self.max_drop_mm.max(other.max_drop_mm),
            pushes_kept: self.pushes_kept + other.pushes_kept,
            kept_moved: self.kept_moved + other.kept_moved,
            kept_left: self.kept_left + other.kept_left,
        }
    }
}

/// One point of a rounded run: where it was planned and where it went.
#[derive(Clone, Copy)]
struct Kept {
    planned: Point,
    at: Point,
    seg: u32,
    /// The tip once the axis is here.
    tip: Point,
    /// The move here pushes the blade (counted in `pushes_kept`).
    pushed: bool,
}

/// A run being rounded.
struct Rounding<'a> {
    d: f64,
    kept: &'a KeptMaterial,
    out: Vec<Kept>,
    /// Planned points dropped since the last kept one, oldest first.
    pending: Vec<(Point, Point, u32)>,
    report: &'a mut RoundReport,
}

impl Rounding<'_> {
    fn last(&self) -> Kept {
        self.out[self.out.len() - 1]
    }

    fn pushes_to(&self, b: Point) -> bool {
        let a = self.last();
        push_angle_deg(a.at, b, a.tip) > PUSHED_DEG
    }

    /// Keep `b` after the last point, counting a push it cannot avoid.
    fn keep(&mut self, planned: Point, b: Point, seg: u32) {
        let a = self.last();
        let pushed = push_angle_deg(a.at, b, a.tip) > PUSHED_DEG;
        if pushed {
            self.report.pushes_kept += 1;
        }
        self.out.push(Kept {
            planned,
            at: b,
            seg,
            tip: drag(a.at, b, a.tip, self.d),
            pushed,
        });
    }

    /// The device point for the planned point `p`: the nearest unit point,
    /// unless its move would push the blade. Then, of the other three unit
    /// points round `p`, the one that does not push and leaves the tip
    /// nearest to where the planned run puts it (`planned_tip`). When all
    /// four push, the nearest one (the caller drops or keeps it).
    ///
    /// A waste point (`waste`: a lead-in, a swivel, an overshoot) also
    /// keeps the tip out of kept material ([`tip_depth`] at most
    /// [`KEPT_TOLERANCE_MM`]): when the nearest unit point would take it
    /// in, only the unit points that do not push and keep it out are
    /// chosen from, the same way ([`RoundReport::kept_moved`]). When none
    /// keeps it out, the one that does not push and takes the tip least
    /// deep ([`RoundReport::kept_left`]).
    fn choose(&mut self, grid: &DeviceGrid, p: Point, planned_tip: Point, waste: bool) -> Point {
        let a = self.last();
        let nearest = grid.snap(p);
        if nearest == a.at {
            return nearest;
        }
        let depth = |r: &Self, c: Point| {
            if waste {
                tip_depth(r.kept, a.at, c, a.tip, r.d)
            } else {
                0.0
            }
        };
        let nearest_depth = depth(self, nearest);
        if !self.pushes_to(nearest) && nearest_depth <= KEPT_TOLERANCE_MM {
            return nearest;
        }
        let u = grid.to_units(p);
        // (depth over the tolerance, tip error, distance to p, the point)
        let mut best: Option<(f64, f64, f64, Point)> = None;
        for ux in [u.x.floor(), u.x.ceil()] {
            for uy in [u.y.floor(), u.y.ceil()] {
                let c = grid.from_units(Point::new(ux, uy));
                if c == a.at || push_angle_deg(a.at, c, a.tip) > PUSHED_DEG {
                    continue;
                }
                let key = (
                    (depth(self, c) - KEPT_TOLERANCE_MM).max(0.0),
                    drag(a.at, c, a.tip, self.d).distance(planned_tip),
                    c.distance(p),
                );
                if best.is_none_or(|(x, e, n, _)| key < (x, e, n)) {
                    best = Some((key.0, key.1, key.2, c));
                }
            }
        }
        let Some((over, _, _, c)) = best else {
            // All four push: the caller drops the point or keeps it as a
            // counted push. A deep tip is never silent either.
            if nearest_depth > KEPT_TOLERANCE_MM {
                self.report.kept_left += 1;
            }
            return nearest;
        };
        if nearest_depth > KEPT_TOLERANCE_MM {
            if over > 0.0 {
                self.report.kept_left += 1;
            } else {
                self.report.kept_moved += 1;
            }
        }
        c
    }

    /// The farthest pending point from the step from the last point to `b`.
    fn pending_error(&self, b: Point) -> f64 {
        let a = self.last().at;
        self.pending
            .iter()
            .map(|&(p, _, _)| distance_to_segment(p, a, b))
            .fold(0.0, f64::max)
    }

    /// Move on to `b`: first keep the oldest pending points until every
    /// other one lies within [`MAX_DROP_MM`] of the new step.
    fn accept(&mut self, planned: Point, b: Point, seg: u32) {
        while !self.pending.is_empty() && self.pending_error(b) > MAX_DROP_MM + DROP_EPS {
            let (p, q, s) = self.pending.remove(0);
            self.report.dropped -= 1;
            self.keep(p, q, s);
        }
        let err = self.pending_error(b);
        self.report.max_drop_mm = self.report.max_drop_mm.max(err);
        self.pending.clear();
        self.keep(planned, b, seg);
    }
}

fn distance_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    if len2 == 0.0 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + t * ab)
}

/// How deep the tip, at `tip` with the axis at `from`, goes into kept
/// material while the axis moves straight to `to`: the largest
/// [`tip_in_kept`] depth of the tip on the way, followed with the drag
/// model in steps of at most `d / 20` (0 when it stays in waste).
fn tip_depth(kept: &KeptMaterial, from: Point, to: Point, tip: Point, d: f64) -> f64 {
    let step = (d / TIP_STEPS_PER_D).max(1e-3);
    let n = (from.distance(to) / step).ceil().max(1.0) as usize;
    let (mut at, mut t, mut worst) = (from, tip, 0.0f64);
    for k in 1..=n {
        let next = from.lerp(to, k as f64 / n as f64);
        t = drag(at, next, t, d);
        at = next;
        worst = worst.max(tip_in_kept(kept, t));
    }
    worst
}

/// How deep `tip` lies in kept material: its distance from the nearest
/// cut when it is kept, else 0.
fn tip_in_kept(kept: &KeptMaterial, tip: Point) -> f64 {
    if kept.is_kept(tip) {
        kept.distance_to_cut(tip)
    } else {
        0.0
    }
}

/// The drop point for the planned drop `p` with the tip `offset` behind
/// the axis: the nearest unit point, unless its tip lies deeper than
/// [`KEPT_TOLERANCE_MM`] in kept material ([`tip_in_kept`]); then the
/// nearest of the four unit points round `p` whose tip does not
/// ([`RoundReport::kept_moved`]), or the least deep one
/// ([`RoundReport::kept_left`]).
fn drop_point(
    grid: &DeviceGrid,
    p: Point,
    offset: Vec2,
    kept: &KeptMaterial,
    report: &mut RoundReport,
) -> Point {
    let nearest = grid.snap(p);
    let nearest_depth = tip_in_kept(kept, nearest - offset);
    if nearest_depth <= KEPT_TOLERANCE_MM {
        return nearest;
    }
    let u = grid.to_units(p);
    // (depth over the tolerance, distance to p, the point)
    let mut best = (f64::INFINITY, f64::INFINITY, nearest);
    for ux in [u.x.floor(), u.x.ceil()] {
        for uy in [u.y.floor(), u.y.ceil()] {
            let c = grid.from_units(Point::new(ux, uy));
            let key = (
                (tip_in_kept(kept, c - offset) - KEPT_TOLERANCE_MM).max(0.0),
                c.distance(p),
            );
            if key < (best.0, best.1) {
                best = (key.0, key.1, c);
            }
        }
    }
    if best.0 > 0.0 {
        report.kept_left += 1;
    } else {
        report.kept_moved += 1;
    }
    best.2
}

/// Round every axis run of `paths` (in cut order, the blade starting at
/// `start_heading`) to `grid`, for a blade of offset `d`:
///
/// - Every point goes to its nearest device unit point. Where that move
///   would push the blade, it goes to the one of the other three unit
///   points round it that does not push and leaves the tip nearest to
///   where the exact run puts it (the model follows both). A point that
///   lands on the one before it is dropped.
/// - A point whose rounded move would push the blade (push angle above
///   [`PUSHED_DEG`] against the tip the model predicts) is dropped, and the
///   next point is tried from the same place.
/// - The first point (the drop) and the last point (the end of the run)
///   always stay. When the step onto the last point would push, the points
///   before it are dropped, newest first, until it does not.
/// - A waste point (the drop, a lead-in, a swivel, an overshoot) keeps the
///   tip out of `kept` (the planner's own kept material): when its nearest
///   unit point would take the tip deeper than [`KEPT_TOLERANCE_MM`] into
///   it, another unit point round it that does not push and keeps the tip
///   out is taken, or the least deep one when none does
///   ([`RoundReport::kept_moved`], [`RoundReport::kept_left`]). Task 16's
///   property found a hole start rounded 0.057 mm into its piece.
/// - A dropped point never lies farther than [`MAX_DROP_MM`] from the
///   rounded run that replaces it. Where a push cannot be avoided within
///   that bound, the point stays and the push is counted
///   ([`RoundReport::pushes_kept`]); the simulator reports it too.
///
/// The tip is followed across runs as the simulator does: blade-up travel
/// keeps the tip's offset from the axis. Toolpaths without axis runs are
/// left alone.
pub fn round_runs(
    paths: &mut [Toolpath],
    grid: &DeviceGrid,
    d: f64,
    start_heading: Vec2,
    kept: &KeptMaterial,
) -> RoundReport {
    let h = if start_heading.hypot() > 0.0 && start_heading.hypot().is_finite() {
        start_heading.normalize()
    } else {
        ASSUMED_START_HEADING
    };
    let mut report = RoundReport::default();
    // The axis minus the tip.
    let mut offset = d * h;
    for t in paths.iter_mut() {
        let Some(runs) = t.axis.as_mut() else {
            continue;
        };
        for run in runs.iter_mut() {
            let Some(&first) = run.points.first() else {
                continue;
            };
            let at = drop_point(grid, first, offset, kept, &mut report);
            let mut r = Rounding {
                d,
                kept,
                out: vec![Kept {
                    planned: first,
                    at,
                    seg: run.seg[0],
                    tip: at - offset,
                    pushed: false,
                }],
                pending: Vec::new(),
                report: &mut report,
            };
            let last = run.points.len() - 1;
            // The tip along the planned (exact) run.
            let (mut prev, mut planned_tip) = (first, first - offset);
            for (k, (&p, &seg)) in run.points.iter().zip(&run.seg).enumerate().skip(1) {
                planned_tip = drag(prev, p, planned_tip, d);
                prev = p;
                let b = r.choose(grid, p, planned_tip, seg == WASTE_SEG);
                if k < last {
                    if b == r.last().at {
                        continue;
                    }
                    if r.pushes_to(b) {
                        r.pending.push((p, b, seg));
                        r.report.dropped += 1;
                        continue;
                    }
                    r.accept(p, b, seg);
                    continue;
                }
                // The end of the run stays: drop the points before it,
                // newest first, while the step onto it pushes and every
                // dropped point stays within the bound.
                let (before, deep_before) = (r.out.len(), {
                    let a = r.last();
                    seg == WASTE_SEG && tip_depth(kept, a.at, b, a.tip, d) > KEPT_TOLERANCE_MM
                });
                while r.out.len() > 1 && r.pushes_to(b) {
                    let k = r.last();
                    r.out.pop();
                    r.pending.insert(0, (k.planned, k.at, k.seg));
                    if r.pending_error(b) > MAX_DROP_MM + DROP_EPS {
                        r.pending.remove(0);
                        r.out.push(k);
                        break;
                    }
                    r.report.dropped += 1;
                    if k.pushed {
                        // That push is gone with its point.
                        r.report.pushes_kept -= 1;
                    }
                }
                if seg == WASTE_SEG && r.out.len() < before && !deep_before {
                    // The step onto the end now starts earlier: a waste end
                    // (an overshoot) is checked again, and counted when
                    // its tip goes too deep into kept material.
                    let a = r.last();
                    if tip_depth(kept, a.at, b, a.tip, d) > KEPT_TOLERANCE_MM {
                        r.report.kept_left += 1;
                    }
                }
                if b == r.last().at {
                    // The end rounds onto the point before it: that point
                    // is the end now.
                    r.report.max_drop_mm = r.report.max_drop_mm.max(r.pending_error(b));
                    r.pending.clear();
                    continue;
                }
                r.accept(p, b, seg);
            }
            let end = r.last();
            offset = end.at - end.tip;
            *run = AxisRun {
                points: r.out.iter().map(|k| k.at).collect(),
                seg: r.out.iter().map(|k| k.seg).collect(),
            };
        }
    }
    debug_assert!(
        report.max_drop_mm <= MAX_DROP_MM + DROP_EPS,
        "a dropped point moved the cut {} mm",
        report.max_drop_mm
    );
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{PathRef, WASTE_SEG};
    use flatcraft_core::document::ObjectId;

    fn grid() -> DeviceGrid {
        DeviceGrid::new(20.0, Point::new(0.3, 0.0))
    }

    /// No kept material at all.
    fn nothing_kept() -> KeptMaterial {
        KeptMaterial::from_toolpaths(
            &[],
            |_| crate::containment::KeepMode::Pieces,
            kurbo::Rect::new(0.0, 0.0, 300.0, 300.0),
        )
    }

    fn toolpath(points: Vec<Point>) -> Toolpath {
        let mut t = Toolpath::new(
            points.clone(),
            false,
            PathRef {
                object: ObjectId(1),
                path_index: 0,
            },
        );
        let mut seg = vec![WASTE_SEG];
        seg.extend(0..points.len() as u32 - 1);
        t.axis = Some(vec![AxisRun { points, seg }]);
        t
    }

    /// Structural: every point lands on the grid, repeats are dropped and
    /// the run keeps its first and last point.
    #[test]
    fn points_land_on_the_grid() {
        let pts = vec![
            Point::new(10.01, 10.0),
            Point::new(10.02, 10.0),
            Point::new(20.013, 10.004),
            Point::new(20.0, 20.0),
        ];
        let mut paths = vec![toolpath(pts)];
        round_runs(
            &mut paths,
            &grid(),
            0.25,
            Vec2::new(1.0, 0.0),
            &nothing_kept(),
        );
        let run = &paths[0].axis.as_ref().unwrap()[0];
        for q in &run.points {
            assert_eq!(grid().snap(*q), *q);
        }
        assert_eq!(run.points.len(), 3, "{:?}", run.points);
        assert_eq!(run.points[0], grid().snap(Point::new(10.01, 10.0)));
        assert_eq!(*run.points.last().unwrap(), Point::new(20.0, 20.0));
        assert_eq!(run.seg.len(), run.points.len());
    }

    /// A point that would move the axis straight back at the tip is dropped.
    #[test]
    fn a_move_back_at_the_tip_is_dropped() {
        // The blade trails along +x; the second point steps back by a unit.
        let pts = vec![
            Point::new(10.0, 10.0),
            Point::new(12.0, 10.0),
            Point::new(11.95, 10.0),
            Point::new(12.0, 12.0),
        ];
        let mut paths = vec![toolpath(pts)];
        round_runs(
            &mut paths,
            &grid(),
            0.25,
            Vec2::new(1.0, 0.0),
            &nothing_kept(),
        );
        let run = &paths[0].axis.as_ref().unwrap()[0];
        assert!(
            !run.points.contains(&Point::new(11.95, 10.0)),
            "{:?}",
            run.points
        );
        assert_eq!(run.points.len(), 3);
    }

    /// The end of a run that steps back at the tip stays, and the point
    /// before it goes, so the last step does not push.
    #[test]
    fn a_last_move_back_at_the_tip_drops_the_point_before() {
        let pts = vec![
            Point::new(10.0, 10.0),
            Point::new(12.0, 10.0),
            Point::new(11.95, 10.0),
        ];
        let mut paths = vec![toolpath(pts)];
        round_runs(
            &mut paths,
            &grid(),
            0.25,
            Vec2::new(1.0, 0.0),
            &nothing_kept(),
        );
        let run = &paths[0].axis.as_ref().unwrap()[0];
        assert_eq!(
            run.points,
            vec![Point::new(10.0, 10.0), Point::new(11.95, 10.0)]
        );
        assert_eq!(run.seg, vec![WASTE_SEG, 1]);
        // Every step trails the tip the model follows.
        let mut tip = run.points[0] - Vec2::new(0.25, 0.0);
        for w in run.points.windows(2) {
            assert!(push_angle_deg(w[0], w[1], tip) <= PUSHED_DEG);
            tip = drag(w[0], w[1], tip, 0.25);
        }
    }

    /// A point whose drop would move the cut more than `MAX_DROP_MM`
    /// stays, and its push is counted, never silent.
    #[test]
    fn a_push_that_cannot_be_dropped_is_kept_and_counted() {
        let pts = vec![
            Point::new(10.0, 10.0),
            Point::new(20.0, 10.0),
            Point::new(19.0, 10.0),
            Point::new(19.0, 20.0),
        ];
        let mut paths = vec![toolpath(pts.clone())];
        let report = round_runs(
            &mut paths,
            &grid(),
            0.25,
            Vec2::new(1.0, 0.0),
            &nothing_kept(),
        );
        let run = &paths[0].axis.as_ref().unwrap()[0];
        assert_eq!(run.points, pts);
        assert_eq!(report.pushes_kept, 1);
        assert_eq!(report.dropped, 0);
    }

    /// Dropped points stay within `MAX_DROP_MM` of the run, and the report
    /// says how far.
    #[test]
    fn dropped_points_stay_within_the_bound() {
        let pts = vec![
            Point::new(10.0, 10.0),
            Point::new(12.0, 10.0),
            Point::new(11.95, 10.0),
            Point::new(11.8, 12.0),
        ];
        let mut paths = vec![toolpath(pts)];
        let report = round_runs(
            &mut paths,
            &grid(),
            0.25,
            Vec2::new(1.0, 0.0),
            &nothing_kept(),
        );
        assert_eq!((report.dropped, report.pushes_kept), (1, 0));
        assert!(report.max_drop_mm > 0.04 && report.max_drop_mm <= MAX_DROP_MM);
    }

    /// A counted push whose point the end-of-run pop loop removes again is
    /// no longer counted.
    #[test]
    fn a_popped_push_is_no_longer_counted() {
        let pts = vec![
            Point::new(10.0, 10.0),
            Point::new(20.0, 10.0),
            Point::new(19.0, 10.0),
            Point::new(19.05, 12.0),
            Point::new(19.0, 12.0),
        ];
        let mut paths = vec![toolpath(pts)];
        let report = round_runs(
            &mut paths,
            &grid(),
            0.25,
            Vec2::new(1.0, 0.0),
            &nothing_kept(),
        );
        let run = &paths[0].axis.as_ref().unwrap()[0];
        // Replay the run with the model: the count matches the pushes left.
        let mut tip = run.points[0] - Vec2::new(0.25, 0.0);
        let mut pushes = 0;
        for w in run.points.windows(2) {
            if push_angle_deg(w[0], w[1], tip) > PUSHED_DEG {
                pushes += 1;
            }
            tip = drag(w[0], w[1], tip, 0.25);
        }
        assert_eq!(report.pushes_kept, pushes, "{:?}", run.points);
    }
}
