//! Quality metrics of a simulated job (R7).
//!
//! The intended path of a toolpath is its `points` (laps and overcut
//! included). Its *path samples* are its blade-down samples that execute an
//! intended segment (`seg != WASTE_SEG`). Waste samples (swivels, lead-ins,
//! overshoots, the drop point) and every sample of an `Alignment` toolpath
//! count only as waste, into-kept, twist and pushed, never as wander,
//! Hausdorff, Frechet, corner, start or stray.
//!
//! A sample stands for the move into it: lengths are tip moves from the
//! previous sample, and the trace is the polyline of those moves.

use std::collections::HashMap;

use flatcraft_path::{CORNER_DEG, KeepMode, KeptMaterial, Toolpath, ToolpathKind, WASTE_SEG};
use kurbo::{Point, Rect, Vec2};

use crate::grid::{SegmentGrid, segment_distance};
use crate::trace::sample_step;
use crate::{
    ALGORITHM_LIMIT_MM, END_TO_END_LIMIT_MM, Sample, SimSettings, SimTrace, UNCUT_LIMIT_MM,
};

/// The start deviation looks at this much tip travel after the first path sample.
pub const START_WINDOW_MM: f64 = 3.0;
/// Down samples whose tip is this close to a corner add to its twist.
pub const TWIST_RADIUS_MM: f64 = 0.05;
/// Each side of a span is thinned to at most this many points for Frechet.
pub const FRECHET_MAX_POINTS: usize = 2000;
/// At most this many worst spots are reported.
pub const WORST_SPOTS: usize = 5;
/// Worst spots are at least this far apart.
pub const SPOT_SEPARATION_MM: f64 = 2.0;

/// The metrics of one toolpath. An `Alignment` toolpath only has waste,
/// segments and pushed.
#[derive(Debug, Clone, PartialEq)]
pub struct PathMetrics {
    pub toolpath: usize,
    /// Trace to path: the largest distance of a path sample from the
    /// intended path.
    pub wander_mm: f64,
    /// Path to trace: the largest distance of the intended path from the
    /// trace (`f64::INFINITY` when nothing of it was cut).
    pub miss_mm: f64,
    /// The larger of `wander_mm` and `miss_mm`.
    pub hausdorff_mm: f64,
    /// The largest distance of a path sample from the intended segment it
    /// executes (its `seg`).
    pub max_correspondence_mm: f64,
    /// The largest discrete Frechet distance of a corner-to-corner span.
    pub max_span_frechet_mm: f64,
    /// The largest correspondence deviation within `2 d` of a corner.
    pub max_corner_deviation_mm: f64,
    /// The largest blade turn while down within [`TWIST_RADIUS_MM`] of one corner.
    pub max_twist_deg: f64,
    /// The largest correspondence deviation in the first [`START_WINDOW_MM`] of tip travel.
    pub max_start_deviation_mm: f64,
    /// Closed `Cut` toolpaths: the distance from the last path tip to the
    /// early trace (the first half lap) of the same ring. About 0 when
    /// the overcut closes the ring, and about `d` for an uncompensated
    /// ring that starts and ends at a corner without overcut. `None` when
    /// the ring has no path move with tip travel.
    pub closure_gap_mm: Option<f64>,
    /// Intended length with no path move within [`UNCUT_LIMIT_MM`]: the
    /// material is not severed there (Ruling M2-P14).
    pub uncut_mm: f64,
    /// Path-sample tip length that strays more than `END_TO_END_LIMIT_MM`
    /// from the intended path, or that the blade cut pushed from behind.
    /// A trace 0.05 to 0.1 mm off is stray only; farther off it is stray
    /// and (where nothing else cuts) uncut too.
    pub stray_mm: f64,
    /// Waste-sample tip length.
    pub waste_mm: f64,
    /// The length of the intended `points` of a `Cut` toolpath, 0 otherwise.
    pub cut_mm: f64,
    /// Axis segments of the toolpath's runs.
    pub segments: usize,
    /// Pushed-blade events: runs of consecutive pushed samples.
    pub pushed: usize,
}

/// What makes a [`Spot`] worth a look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotKind {
    /// The trace strays from the intended path.
    Deviation,
    /// The intended path is not cut here.
    Uncut,
    /// The blade is pushed from behind here.
    Pushed,
    /// The tip cuts into kept material here.
    IntoKept,
}

/// One place worth a look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spot {
    pub at: Point,
    /// The deviation of the sample (or the miss of an uncut stretch, or
    /// the depth into kept material).
    pub deviation_mm: f64,
    pub toolpath: usize,
    pub kind: SpotKind,
}

/// The quality grade of a job (H1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    Good,
    Fair,
    Poor,
}

/// The metrics of a whole job. The `max_*` fields are the largest of the
/// toolpaths' values (0 when there is none), the lengths and counts are sums.
#[derive(Debug, Clone, PartialEq)]
pub struct JobMetrics {
    pub per_path: Vec<PathMetrics>,
    pub max_hausdorff_mm: f64,
    pub max_correspondence_mm: f64,
    pub max_span_frechet_mm: f64,
    pub max_corner_deviation_mm: f64,
    /// Over all down samples, waste included: the largest distance to the
    /// nearest cut of a tip that lies in kept material farther from any
    /// cut than a tolerance. A cut is a closed ring or an open `Cut` path.
    /// The tolerance is `END_TO_END_LIMIT_MM` for path samples (their
    /// deviation is judged by stray) and the strict `uncut_eps_mm` for
    /// waste samples (lead-ins, overshoots, alignment), which should
    /// never touch kept material (Ruling M2-P14). A drop point takes the
    /// tolerance of the move after it.
    pub max_into_kept_mm: f64,
    pub max_twist_deg: f64,
    pub max_start_deviation_mm: f64,
    pub max_closure_gap_mm: f64,
    pub uncut_mm: f64,
    pub stray_mm: f64,
    pub waste_mm: f64,
    pub cut_mm: f64,
    /// Blade-up axis travel from the home.
    pub travel_mm: f64,
    /// Travel moves that cross the trace of an earlier blade-down move.
    pub travel_crossings: usize,
    pub segments: usize,
    /// Blade-down runs (each ends with a lift).
    pub lifts: usize,
    pub pushed: usize,
    /// `100 * max(0, 1 - (uncut_mm + stray_mm) / cut_mm)`.
    pub score: f64,
    pub grade: Grade,
    /// Up to [`WORST_SPOTS`], at least [`SPOT_SEPARATION_MM`] apart:
    /// into-kept and pushed first, then by deviation.
    pub worst_spots: Vec<Spot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Up,
    Waste,
    Path,
}

fn class(toolpaths: &[Toolpath], x: &Sample) -> Class {
    if !x.down {
        Class::Up
    } else if x.seg == WASTE_SEG
        || toolpaths
            .get(x.toolpath)
            .is_none_or(|t| t.kind != ToolpathKind::Cut)
    {
        Class::Waste
    } else {
        Class::Path
    }
}

fn length(points: &[Point]) -> f64 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// The turn from `a` to `b` in degrees, `None` when one has no length.
fn turn_deg(a: Vec2, b: Vec2) -> Option<f64> {
    (a.hypot2() > 0.0 && b.hypot2() > 0.0).then(|| a.cross(b).atan2(a.dot(b)).abs().to_degrees())
}

/// The corners of a toolpath: the interior vertices of `points` that turn
/// by at least `CORNER_DEG`, plus the start of a closed path that ends
/// where it starts when its close turns that much.
fn corners(t: &Toolpath) -> Vec<usize> {
    let p = &t.points;
    let mut out = Vec::new();
    if p.len() < 3 {
        return out;
    }
    let n = p.len();
    if t.closed
        && p[0].distance(p[n - 1]) < 1e-9
        && turn_deg(p[n - 1] - p[n - 2], p[1] - p[0]).is_some_and(|a| a >= CORNER_DEG)
    {
        out.push(0);
    }
    out.extend(
        (1..n - 1).filter(|&j| {
            turn_deg(p[j] - p[j - 1], p[j + 1] - p[j]).is_some_and(|a| a >= CORNER_DEG)
        }),
    );
    out
}

/// At most `max` of `points`, evenly by index, first and last kept.
fn thin(points: &[Point], max: usize) -> Vec<Point> {
    if points.len() <= max {
        return points.to_vec();
    }
    let last = points.len() - 1;
    (0..max)
        .map(|q| points[(q * last + (max - 1) / 2) / (max - 1)])
        .collect()
}

/// The discrete Frechet distance (Eiter and Mannila) between the intended
/// polyline `path` and the trace points `trace`, each thinned to at most
/// [`FRECHET_MAX_POINTS`]. The coupling cost of a trace point and an
/// intended segment is the point-to-segment distance, so two identical
/// curves score 0 whatever their sample spacing.
///
/// Limits of this form:
/// - The value is at most the continuous Frechet distance plus the
///   spacing of the (thinned) trace points.
/// - It is not order-sensitive inside one intended segment: trace points
///   that run back and forth along one segment all couple to it.
/// - Thinning keeps points evenly by index, so an excursion of the trace
///   shorter than the thinned spacing can be dropped. Hausdorff and
///   correspondence see every sample, so they still catch it.
fn span_frechet(path: &[Point], trace: &[Point]) -> f64 {
    let a = thin(path, FRECHET_MAX_POINTS);
    let b = thin(trace, FRECHET_MAX_POINTS);
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let segs: Vec<(Point, Point)> = if a.len() == 1 {
        vec![(a[0], a[0])]
    } else {
        a.windows(2).map(|w| (w[0], w[1])).collect()
    };
    let mut prev = vec![f64::INFINITY; b.len()];
    let mut row = vec![f64::INFINITY; b.len()];
    for (i, &(s0, s1)) in segs.iter().enumerate() {
        for (j, &q) in b.iter().enumerate() {
            #[cfg(test)]
            tests::DP_CELLS.with(|c| c.set(c.get() + 1));
            let c = segment_distance(q, s0, s1);
            let reach = match (i, j) {
                (0, 0) => c,
                (0, _) => row[j - 1],
                (_, 0) => prev[0],
                _ => prev[j].min(row[j - 1]).min(prev[j - 1]),
            };
            row[j] = c.max(reach);
        }
        std::mem::swap(&mut prev, &mut row);
    }
    prev[b.len() - 1]
}

/// True when the travel `p1`-`p2` properly crosses the move `q1`-`q2` more
/// than `keep_off` from both ends of the travel (a travel starts where the
/// last cut ended, and the tip trails the axis by `d`).
fn crosses(p1: Point, p2: Point, q1: Point, q2: Point, keep_off: f64) -> bool {
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
    at.distance(p1) > keep_off && at.distance(p2) > keep_off
}

/// The corners of the job in 1 mm cells, for the radius queries.
struct CornerGrid {
    corners: Vec<(Point, usize)>,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

impl CornerGrid {
    fn new(corners: Vec<(Point, usize)>) -> Self {
        let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (c, &(p, _)) in corners.iter().enumerate() {
            cells
                .entry((p.x.floor() as i64, p.y.floor() as i64))
                .or_default()
                .push(c);
        }
        Self { corners, cells }
    }

    /// The corners of toolpath `t` within `r` of `p`, with their distance.
    fn near(&self, p: Point, t: usize, r: f64) -> impl Iterator<Item = (usize, f64)> + '_ {
        let (x0, x1) = ((p.x - r).floor() as i64, (p.x + r).floor() as i64);
        let (y0, y1) = ((p.y - r).floor() as i64, (p.y + r).floor() as i64);
        (y0..=y1)
            .flat_map(move |row| (x0..=x1).map(move |col| (col, row)))
            .filter_map(|cell| self.cells.get(&cell))
            .flatten()
            .filter_map(move |&c| {
                let (v, ct) = self.corners[c];
                let dist = v.distance(p);
                (ct == t && dist <= r).then_some((c, dist))
            })
    }
}

/// Running state of one toolpath while the samples are walked.
struct PathState {
    started: bool,
    last_tip: Point,
    tip_len: f64,
    /// Closed `Cut` toolpaths: the length of one lap.
    lap_mm: Option<f64>,
    /// The path moves of the first half lap, with the tip length before each.
    early: Vec<(f64, Point, Point)>,
    /// Per span: the tips of its path samples, in trace order.
    span_tips: Vec<Vec<Point>>,
    /// Interior vertex indices where spans start.
    span_starts: Vec<usize>,
}

/// Measure a simulated job. `trace` must come from `simulate(toolpaths, ..)`
/// with the same `s`; the home is its first sample. A sample that names a
/// toolpath past the end of `toolpaths` is a bug: it panics in debug and
/// is skipped in release.
///
/// Returns the metrics and the per-sample deviation: NaN when the blade is
/// up, the distance to the nearest `Cut` path for waste samples, and the
/// distance to the toolpath's own intended path for path samples.
///
/// Kept material is classified with every object as
/// [`KeepMode::Pieces`] until per-object keep modes exist (Task 29).
pub fn measure(
    toolpaths: &[Toolpath],
    trace: &SimTrace,
    s: &SimSettings,
) -> (JobMetrics, Vec<f32>) {
    let samples = &trace.samples;
    let d = s.blade_offset_mm;
    let eps = s.uncut_eps_mm;
    let step = sample_step(s.sample_step_mm);
    let classes: Vec<Class> = samples.iter().map(|x| class(toolpaths, x)).collect();
    let is_cut = |i: usize| toolpaths[i].kind == ToolpathKind::Cut;

    let mut per: Vec<PathMetrics> = toolpaths
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let runs = t.runs();
            PathMetrics {
                toolpath: i,
                wander_mm: 0.0,
                miss_mm: 0.0,
                hausdorff_mm: 0.0,
                max_correspondence_mm: 0.0,
                max_span_frechet_mm: 0.0,
                max_corner_deviation_mm: 0.0,
                max_twist_deg: 0.0,
                max_start_deviation_mm: 0.0,
                closure_gap_mm: None,
                uncut_mm: 0.0,
                stray_mm: 0.0,
                waste_mm: 0.0,
                cut_mm: if is_cut(i) { length(&t.points) } else { 0.0 },
                segments: runs.iter().map(|r| r.points.len().saturating_sub(1)).sum(),
                pushed: 0,
            }
        })
        .collect();
    let lifts = toolpaths
        .iter()
        .flat_map(Toolpath::runs)
        .filter(|r| !r.points.is_empty())
        .count();

    let mut intended = SegmentGrid::default();
    let mut corner_list = Vec::new();
    let mut state: Vec<PathState> = Vec::with_capacity(toolpaths.len());
    for (i, t) in toolpaths.iter().enumerate() {
        let cut = is_cut(i);
        let starts = if cut { corners(t) } else { Vec::new() };
        if cut {
            for w in t.points.windows(2) {
                intended.insert(w[0], w[1], i);
            }
            corner_list.extend(starts.iter().map(|&j| (t.points[j], i)));
        }
        let span_starts: Vec<usize> = starts.into_iter().filter(|&j| j > 0).collect();
        state.push(PathState {
            started: false,
            last_tip: Point::ZERO,
            tip_len: 0.0,
            lap_mm: (cut && t.closed).then(|| length(t.lap())),
            early: Vec::new(),
            span_tips: vec![Vec::new(); span_starts.len() + 1],
            span_starts,
        });
    }
    let corner_grid = CornerGrid::new(corner_list);
    let mut corner_dev = vec![0.0f64; corner_grid.corners.len()];
    let mut twist = vec![0.0f64; corner_grid.corners.len()];

    // Pieces for every object until Task 29. The sheet and the clearance
    // only matter for Stencil and Both, so any finite area will do.
    let area = toolpaths
        .iter()
        .flat_map(|t| t.points.iter().copied())
        .chain(samples.iter().map(|x| x.tip))
        .filter(|p| p.is_finite())
        .fold(None, |r: Option<Rect>, p| {
            Some(r.map_or(Rect::from_points(p, p), |r| r.union_pt(p)))
        })
        .unwrap_or(Rect::ZERO)
        .inflate(1.0, 1.0);
    let kept = KeptMaterial::from_toolpaths(toolpaths, |_| KeepMode::Pieces, area, 0.0);

    let mut dev = vec![f32::NAN; samples.len()];
    let mut moves = SegmentGrid::default();
    let mut spots = SpotCells::default();
    let mut max_into_kept = 0.0f64;
    let (mut travel_mm, mut travel_crossings) = (0.0, 0usize);
    let mut travel_from = samples.first().map_or(Point::ZERO, |x| x.axis);
    let keep_off = 2.0 * d + eps;
    let corner_r = TWIST_RADIUS_MM.max(2.0 * d);

    for k in 1..samples.len() {
        let (prev, x) = (&samples[k - 1], &samples[k]);
        if classes[k] == Class::Up {
            if classes[k - 1] != Class::Up {
                travel_from = prev.axis;
            }
            travel_mm += prev.axis.distance(x.axis);
            let last = classes.get(k + 1).is_none_or(|c| *c != Class::Up);
            if last {
                let crossed = moves.along(travel_from, x.axis).into_iter().any(|id| {
                    let (a, b, _) = moves.segment(id);
                    crosses(travel_from, x.axis, a, b, keep_off)
                });
                travel_crossings += usize::from(crossed);
            }
            continue;
        }
        let i = x.toolpath;
        if i >= toolpaths.len() {
            debug_assert!(
                false,
                "sample {k} names toolpath {i} of {}",
                toolpaths.len()
            );
            continue;
        }
        let len = prev.tip.distance(x.tip);
        moves.insert(prev.tip, x.tip, k);

        let deviation = if classes[k] == Class::Waste {
            per[i].waste_mm += len;
            intended.nearest(x.tip, |_| true, 0.0)
        } else {
            intended.nearest(x.tip, |t| t == i, 0.0)
        };
        dev[k] = deviation as f32;

        // The drop point is judged with the move that follows it: on a cut
        // that starts straight away it is the start of the path trace.
        let drop = classes[k - 1] == Class::Up || prev.toolpath != i || prev.run != x.run;
        let judged = if drop {
            classes.get(k + 1).copied().unwrap_or(classes[k])
        } else {
            classes[k]
        };
        let tolerance = if judged == Class::Path {
            END_TO_END_LIMIT_MM
        } else {
            eps
        };
        if kept.is_clear(x.tip, tolerance) && kept.is_kept(x.tip) {
            // `KeptMaterial` knows only the closed rings. An open cut (a
            // score line inside a piece) is a cut too, not kept material.
            let to_open_cut = if classes[k] == Class::Waste {
                deviation
            } else {
                intended.nearest(x.tip, |_| true, 0.0)
            };
            let depth = kept.distance_to_cut(x.tip).min(to_open_cut);
            if depth > tolerance {
                max_into_kept = max_into_kept.max(depth);
                spots.offer(Spot {
                    at: x.tip,
                    deviation_mm: depth,
                    toolpath: i,
                    kind: SpotKind::IntoKept,
                });
            }
        }
        if x.pushed && !(prev.pushed && prev.down && prev.toolpath == i) {
            per[i].pushed += 1;
            spots.offer(Spot {
                at: x.tip,
                deviation_mm: deviation,
                toolpath: i,
                kind: SpotKind::Pushed,
            });
        }

        // The blade turn while down near a corner (waste moves included).
        let (h0, h1) = (prev.axis - prev.tip, x.axis - x.tip);
        let turn = if h0.hypot() > 1e-9 && h1.hypot() > 1e-9 {
            turn_deg(h0, h1).unwrap_or(0.0)
        } else {
            0.0
        };
        let correspondence = (classes[k] == Class::Path).then(|| {
            let p = &toolpaths[i].points;
            if p.len() < 2 {
                return deviation;
            }
            let seg = x.seg as usize;
            debug_assert!(
                seg + 1 < p.len(),
                "seg {seg} of toolpath {i} is past its points"
            );
            let seg = seg.min(p.len() - 2);
            segment_distance(x.tip, p[seg], p[seg + 1])
        });
        for (c, dist) in corner_grid.near(x.tip, i, corner_r) {
            if dist <= TWIST_RADIUS_MM {
                twist[c] += turn;
            }
            if let Some(corr) = correspondence
                && dist <= 2.0 * d
            {
                corner_dev[c] = corner_dev[c].max(corr);
            }
        }

        let Some(corr) = correspondence else {
            continue;
        };
        let m = &mut per[i];
        m.wander_mm = m.wander_mm.max(deviation);
        m.max_correspondence_mm = m.max_correspondence_mm.max(corr);
        if deviation > END_TO_END_LIMIT_MM || x.pushed {
            m.stray_mm += len;
        }
        if deviation > ALGORITHM_LIMIT_MM {
            spots.offer(Spot {
                at: x.tip,
                deviation_mm: deviation,
                toolpath: i,
                kind: SpotKind::Deviation,
            });
        }
        let st = &mut state[i];
        st.started = true;
        if st.tip_len < START_WINDOW_MM {
            m.max_start_deviation_mm = m.max_start_deviation_mm.max(corr);
        }
        if st.lap_mm.is_some_and(|lap| st.tip_len < 0.5 * lap) {
            st.early.push((st.tip_len, prev.tip, x.tip));
        }
        st.tip_len += len;
        st.last_tip = x.tip;
        let span = st.span_starts.partition_point(|&j| j <= x.seg as usize);
        st.span_tips[span].push(x.tip);
    }

    // Path to trace: the intended path sampled at the sample step against
    // the path moves of its own toolpath.
    for (i, t) in toolpaths.iter().enumerate().filter(|&(i, _)| is_cut(i)) {
        let own = |k: usize| classes[k] == Class::Path && samples[k].toolpath == i;
        let near = |p: Point| moves.nearest(p, own, 0.0);
        let m = &mut per[i];
        let mut stretch: Option<(f64, Point)> = None;
        let close = |stretch: &mut Option<(f64, Point)>, spots: &mut SpotCells| {
            if let Some((miss, at)) = stretch.take() {
                spots.offer(Spot {
                    at,
                    deviation_mm: miss,
                    toolpath: i,
                    kind: SpotKind::Uncut,
                });
            }
        };
        for w in t.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let seg_len = a.distance(b);
            if seg_len == 0.0 {
                continue;
            }
            let n = (seg_len / step).ceil().max(1.0) as usize;
            let piece = seg_len / n as f64;
            for q in 0..n {
                let mid = a.lerp(b, (q as f64 + 0.5) / n as f64);
                let at_mid = near(mid);
                m.miss_mm = m
                    .miss_mm
                    .max(near(a.lerp(b, q as f64 / n as f64)))
                    .max(at_mid);
                if at_mid > UNCUT_LIMIT_MM {
                    m.uncut_mm += piece;
                    if stretch.is_none_or(|(miss, _)| at_mid > miss) {
                        stretch = Some((at_mid, mid));
                    }
                } else {
                    close(&mut stretch, &mut spots);
                }
            }
        }
        close(&mut stretch, &mut spots);
        if let Some(&end) = t.points.last() {
            m.miss_mm = m.miss_mm.max(near(end));
        }
        m.hausdorff_mm = m.wander_mm.max(m.miss_mm);

        let st = &state[i];
        let mut bounds = vec![0];
        bounds.extend(&st.span_starts);
        bounds.push(t.points.len() - 1);
        for (q, tips) in st.span_tips.iter().enumerate() {
            if !tips.is_empty() {
                let path = &t.points[bounds[q]..=bounds[q + 1]];
                m.max_span_frechet_mm = m.max_span_frechet_mm.max(span_frechet(path, tips));
            }
        }
        if let Some(lap) = st.lap_mm
            && st.started
        {
            // The first half lap, or half the trace when it is shorter, so
            // the last tip is never measured against its own moves.
            let early = 0.5 * lap.min(st.tip_len);
            let gap = st
                .early
                .iter()
                .filter(|e| e.0 < early)
                .map(|&(_, a, b)| segment_distance(st.last_tip, a, b))
                .fold(f64::INFINITY, f64::min);
            // No early trace (the tip never moved): there is no gap to measure.
            m.closure_gap_mm = gap.is_finite().then_some(gap);
        }
    }
    for (c, &(_, i)) in corner_grid.corners.iter().enumerate() {
        let m = &mut per[i];
        m.max_corner_deviation_mm = m.max_corner_deviation_mm.max(corner_dev[c]);
        m.max_twist_deg = m.max_twist_deg.max(twist[c]);
    }

    let max = |f: fn(&PathMetrics) -> f64| per.iter().map(f).fold(0.0, f64::max);
    let sum = |f: fn(&PathMetrics) -> f64| per.iter().map(f).sum::<f64>();
    let (uncut_mm, stray_mm, cut_mm) =
        (sum(|m| m.uncut_mm), sum(|m| m.stray_mm), sum(|m| m.cut_mm));
    let score = if cut_mm > 0.0 {
        100.0 * (1.0 - (uncut_mm + stray_mm) / cut_mm).max(0.0)
    } else if uncut_mm + stray_mm > 0.0 {
        0.0
    } else {
        100.0
    };
    let max_hausdorff_mm = max(|m| m.hausdorff_mm);
    let pushed = per.iter().map(|m| m.pushed).sum();
    let grade = if score >= 98.0
        && max_hausdorff_mm <= END_TO_END_LIMIT_MM
        && pushed == 0
        && max_into_kept == 0.0
    {
        Grade::Good
    } else if score >= 90.0 && max_into_kept == 0.0 {
        Grade::Fair
    } else {
        Grade::Poor
    };

    let metrics = JobMetrics {
        max_hausdorff_mm,
        max_correspondence_mm: max(|m| m.max_correspondence_mm),
        max_span_frechet_mm: max(|m| m.max_span_frechet_mm),
        max_corner_deviation_mm: max(|m| m.max_corner_deviation_mm),
        max_into_kept_mm: max_into_kept,
        max_twist_deg: max(|m| m.max_twist_deg),
        max_start_deviation_mm: max(|m| m.max_start_deviation_mm),
        max_closure_gap_mm: max(|m| m.closure_gap_mm.unwrap_or(0.0)),
        uncut_mm,
        stray_mm,
        waste_mm: sum(|m| m.waste_mm),
        cut_mm,
        travel_mm,
        travel_crossings,
        segments: per.iter().map(|m| m.segments).sum(),
        lifts,
        pushed,
        score,
        grade,
        worst_spots: worst_spots(spots.into_vec()),
        per_path: per,
    };
    (metrics, dev)
}

/// The worst spot candidates, one per cell of half the spot separation
/// and rank, so memory grows with the area cut, not with the samples.
/// A spot lower than its cell's best but farther than the separation from
/// every chosen spot can be lost; the cell's best is at most
/// `SPOT_SEPARATION_MM / sqrt(2)` from it.
#[derive(Default)]
struct SpotCells(HashMap<(i64, i64, u8), Spot>);

impl SpotCells {
    fn offer(&mut self, s: Spot) {
        let size = SPOT_SEPARATION_MM / 2.0;
        let key = (
            (s.at.x / size).floor() as i64,
            (s.at.y / size).floor() as i64,
            rank(&s),
        );
        let best = self.0.entry(key).or_insert(s);
        if s.deviation_mm > best.deviation_mm {
            *best = s;
        }
    }

    fn into_vec(self) -> Vec<Spot> {
        self.0.into_values().collect()
    }
}

/// Into-kept and pushed spots rank 0, the rest 1.
fn rank(s: &Spot) -> u8 {
    match s.kind {
        SpotKind::IntoKept | SpotKind::Pushed => 0,
        SpotKind::Deviation | SpotKind::Uncut => 1,
    }
}

/// Into-kept and pushed spots first, then by deviation; greedy, at least
/// [`SPOT_SEPARATION_MM`] apart, at most [`WORST_SPOTS`].
fn worst_spots(mut spots: Vec<Spot>) -> Vec<Spot> {
    // The position breaks ties, so the order does not depend on the map.
    spots.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then(b.deviation_mm.total_cmp(&a.deviation_mm))
            .then(a.at.x.total_cmp(&b.at.x))
            .then(a.at.y.total_cmp(&b.at.y))
    });
    let mut out: Vec<Spot> = Vec::with_capacity(WORST_SPOTS);
    for s in spots {
        if out.len() == WORST_SPOTS {
            break;
        }
        if out
            .iter()
            .all(|o| o.at.distance(s.at) >= SPOT_SEPARATION_MM)
        {
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use flatcraft_core::document::ObjectId;
    use flatcraft_path::PathRef;

    use super::*;
    use crate::grid::{CELL_VISITS, QUERIES, SEGMENT_CHECKS};
    use crate::simulate;

    thread_local! {
        /// Frechet DP cells filled, for the cost test.
        pub(super) static DP_CELLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }

    /// Ruling F30: the cost bound of `measure` on a 2000 mm path of 20000
    /// vertices is a count, not a time. Each nearest-segment query visits
    /// a few cells around its point, and the one span's Frechet table has
    /// at most `FRECHET_MAX_POINTS^2` cells.
    #[test]
    fn long_path_cost_is_bounded() {
        let n = 20_000;
        let r = 2000.0 / std::f64::consts::TAU;
        let c = Point::new(400.0, 400.0);
        let ring: Vec<Point> = (0..=n)
            .map(|k| {
                let a = std::f64::consts::TAU * (k % n) as f64 / n as f64;
                c + r * Vec2::new(a.cos(), a.sin())
            })
            .collect();
        let source = PathRef {
            object: ObjectId(1),
            path_index: 0,
        };
        let t = [Toolpath::new(ring.clone(), true, source)];
        let s = SimSettings::float(0.25);
        let trace = simulate(&t, ring[0], &s).unwrap();
        for counter in [&CELL_VISITS, &QUERIES, &SEGMENT_CHECKS, &DP_CELLS] {
            counter.with(|c| c.set(0));
        }
        let (m, _) = measure(&t, &trace, &s);
        assert!((m.cut_mm - 2000.0).abs() < 1.0, "{}", m.cut_mm);
        let (cells, queries) = (CELL_VISITS.with(|c| c.get()), QUERIES.with(|c| c.get()));
        let dp = DP_CELLS.with(|c| c.get());
        let checks = SEGMENT_CHECKS.with(|c| c.get());
        // one query per down sample, two per intended piece, and the ends
        let pieces = (2000.0 / s.sample_step_mm).ceil() as u64;
        assert!(
            queries <= trace.samples.len() as u64 + 2 * pieces + n as u64 + 1,
            "{queries}"
        );
        // Measured 2026-11-25: 8.8 cells and 193 segment checks per query
        // (a 1 mm cell holds about 80 trace moves at d = 0.25). A scan of
        // every segment would be about 180000 per query.
        assert!(cells <= 16 * queries, "{cells} cells for {queries} queries");
        assert!(
            checks <= 400 * queries,
            "{checks} checks for {queries} queries"
        );
        let max = FRECHET_MAX_POINTS as u64;
        assert!(dp <= max * max, "{dp}");
    }

    #[test]
    fn thin_keeps_the_ends_and_the_cap() {
        let p: Vec<Point> = (0..5000).map(|k| Point::new(k as f64, 0.0)).collect();
        let t = thin(&p, FRECHET_MAX_POINTS);
        assert_eq!(t.len(), FRECHET_MAX_POINTS);
        assert_eq!((t[0], *t.last().unwrap()), (p[0], p[4999]));
        assert_eq!(thin(&p[..10], FRECHET_MAX_POINTS).len(), 10);
    }
}
