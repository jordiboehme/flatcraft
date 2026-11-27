//! Stage 9: the knife. Start alignment, exact host compensation and corner
//! modes.

use flatcraft_core::document::ObjectId;
use kurbo::{Point, Rect, Vec2};

use crate::containment::{KeptMaterial, signed_area};
use crate::plan::{
    ASSUMED_START_HEADING, AlignmentStroke, AppliedCompensation, AxisRun, CORNER_DEG, KeepSide,
    PathRef, PipelineSettings, Toolpath, ToolpathKind, WASTE_SEG,
};
use crate::quantize::{self, DeviceGrid};
use crate::rings;

/// The largest start turn swivelled in place at a waste-side corner. A
/// lead-in turns only up to this much along its line. Provisional: the
/// model cannot rate it, and the M3 test cuts decide it.
pub const SWIVEL_MAX_DEG: f64 = 90.0;

/// The tip radius of the arc lead-in. Pinned by Task 10's
/// `hole_lead_in_choice`: the smallest of 0.5, 1, 2 and 3 mm whose starts
/// stay within `ALGORITHM_LIMIT_MM`, push nothing, touch no kept material
/// and take the arc lead-in in every hole (radius 1 to 10 mm). Every radius
/// meets the deviation, push and kept limits; only 0.5 mm takes the arc on
/// every hole (1 mm and larger do not fit the 1 or 2 mm holes). It also
/// wastes the least (0.8 mm against 3.2 mm at 2 mm).
pub const HOLE_LEAD_IN_RADIUS_MM: f64 = 0.5;

/// The angle between two axis points of an arc. Pinned by Task 10's
/// `arc_step_choice`: the largest of 30, 20, 15, 10 and 5 deg whose corner
/// deviation and pivot drift stay within `ALGORITHM_LIMIT_MM` on every
/// star tip and on 178 and 180 deg turns. 10 deg measures 0.0026 mm at
/// 180 deg; 15 deg measures 0.0058.
pub const ARC_STEP_DEG: f64 = 10.0;

/// The start error a lead-in aims for: the same value as
/// `flatcraft_sim::ALGORITHM_LIMIT_MM`, which this crate cannot see.
pub const ALGORITHM_LIMIT_MM: f64 = 0.005;

/// How far a waste move keeps off every cut.
pub const WASTE_CLEARANCE_MM: f64 = 0.1;

/// What host compensation does at a corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CornerMode {
    Arc,
    LiftOvershoot { overshoot_mm: f64 },
}

/// The host compensation values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostSettings {
    pub offset_mm: f64,
    pub pivot_threshold_deg: f64,
    pub swivel_max_deg: f64,
    pub hole_lead_in_radius_mm: f64,
    pub arc_step_deg: f64,
    /// The device units, when the plan is for a known device: the runs are
    /// then rounded to them without pushing the blade
    /// ([`crate::quantize::round_runs`]). `None` keeps the exact (float)
    /// axis runs.
    pub device_grid: Option<DeviceGrid>,
}

impl HostSettings {
    pub fn new(offset_mm: f64, pivot_threshold_deg: f64) -> Self {
        Self {
            offset_mm,
            pivot_threshold_deg,
            swivel_max_deg: SWIVEL_MAX_DEG,
            hole_lead_in_radius_mm: HOLE_LEAD_IN_RADIUS_MM,
            arc_step_deg: ARC_STEP_DEG,
            device_grid: None,
        }
    }
}

/// Where the knife is steered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Compensation {
    Tip,
    Host(HostSettings),
}

/// The turn below which a start needs no swivel arc: the blade drops on
/// P0 and moves on.
pub const ALIGNED_DEG: f64 = 1.0;

/// Axis points closer than this are one point.
const SAME_POINT_MM: f64 = 1e-9;

/// The largest spacing of the samples of a waste check, along the planned
/// tip path. It holds two guarantees for a move that crosses kept
/// material at any angle:
/// - Farther than 2d from the join, a crossed cut has a sample within
///   `CHECK_STEP_MM / 2` of it, which is below [`WASTE_CLEARANCE_MM`], so
///   the clearance test refuses the move.
/// - Within 2d of the join, where a sample may touch kept material by up
///   to [`ALGORITHM_LIMIT_MM`], a kept sliver at least
///   `2 * ALGORITHM_LIMIT_MM + CHECK_STEP_MM` (0.03 mm) wide has a sample
///   deeper than that in it, so the move is refused.
const CHECK_STEP_MM: f64 = 0.02;

/// Apply the knife stage to the ordered `paths` (after passes).
///
/// - With an alignment stroke, a [`ToolpathKind::Alignment`] toolpath along
///   it is inserted first. It is cut with the blade as it is, so it turns
///   the blade to the stroke direction.
/// - [`Compensation::Tip`] leaves every `axis` at `None` and returns
///   [`AppliedCompensation::Tip`].
/// - [`Compensation::Host`] sets `axis` on every toolpath and returns
///   [`AppliedCompensation::Host`]. The axis runs are the exact inverse of
///   the drag knife model: the axis is `d` ahead of the tip along the
///   tangent, with an arc round every vertex that turns more than the pivot
///   threshold, and each run starts aligned (see [`Knife::align`]).
///
/// The heading into toolpath k is the blade heading after toolpath k - 1:
/// the direction of its last move, which for a closed ring is the
/// `exit_state` heading after its overcut. It is `start_heading` for the
/// first toolpath, or the stroke direction after an alignment stroke.
/// `kept` is the job's kept material: every waste move is checked against
/// it. With [`HostSettings::device_grid`] the runs are then rounded to the
/// device units ([`quantize::round_runs`]).
pub fn apply(
    paths: &mut Vec<Toolpath>,
    s: &PipelineSettings,
    kept: &KeptMaterial,
    start_heading: Vec2,
) -> AppliedCompensation {
    apply_rounded(paths, s, kept, start_heading).0
}

/// [`apply`], with what the rounding to the device units did (the
/// default report when the runs are not rounded).
pub fn apply_rounded(
    paths: &mut Vec<Toolpath>,
    s: &PipelineSettings,
    kept: &KeptMaterial,
    start_heading: Vec2,
) -> (AppliedCompensation, quantize::RoundReport) {
    if let Some(stroke) = s.alignment_stroke {
        let source = paths.first().map_or(
            PathRef {
                object: ObjectId(0),
                path_index: 0,
            },
            |t| t.source,
        );
        paths.insert(0, stroke_toolpath(&stroke, source));
    }
    let Compensation::Host(hs) = s.compensation else {
        return (AppliedCompensation::Tip, quantize::RoundReport::default());
    };
    let knife = Knife {
        d: hs.offset_mm,
        hs,
        corner: s.corner_mode,
        kept,
        area: s.cutting_area,
    };
    let mut heading = unit_or(start_heading, ASSUMED_START_HEADING);
    for t in paths.iter_mut() {
        let runs = match t.kind {
            ToolpathKind::Alignment => vec![AxisRun {
                points: t.points.clone(),
                seg: vec![WASTE_SEG; t.points.len()],
            }],
            ToolpathKind::Cut | ToolpathKind::Weed => knife.runs(t, heading),
        };
        t.axis = Some(runs);
        heading = heading_after(t, heading);
    }
    // A push it keeps (to hold the cut within `MAX_DROP_MM`) and a waste
    // move it cannot keep out of kept material are counted in the report:
    // the plan carries it and the job warns.
    let report = hs
        .device_grid
        .map_or_else(quantize::RoundReport::default, |grid| {
            quantize::round_runs(paths, &grid, hs.offset_mm, start_heading, kept)
        });
    (
        AppliedCompensation::Host {
            offset_mm: hs.offset_mm,
        },
        report,
    )
}

/// The head position and the blade heading after `paths`, starting at
/// `home` with `start_heading` (Ruling F9). The head is the last point of
/// the last axis run, the heading the direction of the last move.
pub fn end_state(paths: &[Toolpath], home: Point, start_heading: Vec2) -> (Point, Vec2) {
    let mut end = home;
    let mut heading = unit_or(start_heading, ASSUMED_START_HEADING);
    for t in paths {
        if let Some(&last) = t.runs().iter().rev().find_map(|r| r.points.last()) {
            end = last;
        }
        heading = heading_after(t, heading);
    }
    (end, heading)
}

/// A lone tight vertex that turns at least this many degrees, between
/// segments of at least [`CORNER_SEGMENT_FACTOR`] times the minimum radius,
/// is a corner, not a tight curve ([`min_radius_spots`]): the blade swivels
/// there, and a glyph corner between short segments must not warn. The
/// same 30 deg as [`CORNER_DEG`].
pub const TIGHT_CORNER_DEG: f64 = 30.0;

/// How long both segments of a lone tight vertex must be, in minimum radii,
/// for it to count as a corner ([`TIGHT_CORNER_DEG`]). It makes the rule
/// independent of the flatten tolerance: an arc of radius `r` below the
/// minimum radius `R` that flattens to a single vertex has two chords of
/// `2 r sin(A / 4)` for an arc of `A`, shorter than `1.5 R` for every arc
/// up to 194 deg, so such a rounded corner always warns, however coarse the
/// flattening. The cost: a fillet of more than 194 deg that flattens to one
/// vertex, which a sane tolerance never gives, is taken for a corner.
/// 1.5 and not 2: a glyph corner of 98 deg between segments of 0.39 and
/// 0.42 mm (the maker-quotes sample, minimum radius 0.25) is a corner.
pub const CORNER_SEGMENT_FACTOR: f64 = 1.5;

/// Vertices bending with a local radius below this many minimum radii
/// that lie between two tight curve vertices are part of the same curve
/// ([`min_radius_spots`]): an arc right on the limit can measure a hair
/// above it in its middle (0.2504 mm on a 0.25 mm arc), and the warning
/// counts curves, not pieces of them.
pub const CURVE_BRIDGE_FACTOR: f64 = 2.0;

/// Places on `points` (a polyline) where the path bends tighter than
/// `min_radius_mm`. The local radius at a vertex is
/// `((|seg_in| + |seg_out|) / 2) / turn`, with the turn in radians. A run of
/// neighbouring vertices below the limit (a flattened arc) is one spot, at
/// half its length; vertices that still bend clearly between two tight
/// ones ([`CURVE_BRIDGE_FACTOR`]) do not split it. A single vertex that
/// turns [`TIGHT_CORNER_DEG`] or more between segments of at least [`CORNER_SEGMENT_FACTOR`] minimum radii is
/// a corner, not a curve: the blade swivels there, so it is not a spot. A
/// path that ends where it starts is a ring: its start vertex is checked
/// too, and a run across the start is one spot. Used for warnings in every
/// mode.
pub fn min_radius_spots(points: &[Point], min_radius_mm: f64) -> Vec<Point> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    let ring = n > 3 && points[0] == points[n - 1];
    // The vertices and their neighbours: every ring vertex, or the
    // interior vertices of a polyline.
    let m = if ring { n - 1 } else { n - 2 };
    let vertex = |k: usize| if ring { k } else { k + 1 };
    let at = |k: usize| points[vertex(k)];
    // A corner on its own: a turn of at least TIGHT_CORNER_DEG between long
    // segments.
    let corner_turn = TIGHT_CORNER_DEG.to_radians();
    let corner_len = CORNER_SEGMENT_FACTOR * min_radius_mm;
    // The local radius at vertex k, and whether it would be a corner.
    let bend = |k: usize| {
        let v = vertex(k);
        let prev = if v == 0 { points[n - 2] } else { points[v - 1] };
        let next = points[v + 1];
        let (a, b) = (points[v] - prev, next - points[v]);
        let turn = a.cross(b).atan2(a.dot(b)).abs();
        let radius = if turn > 0.0 {
            (a.hypot() + b.hypot()) / 2.0 / turn
        } else {
            f64::INFINITY
        };
        let corner = turn >= corner_turn && a.hypot() >= corner_len && b.hypot() >= corner_len;
        (radius, corner)
    };
    let bends: Vec<(f64, bool)> = (0..m).map(bend).collect();
    // When vertex k bends tighter than the limit: whether it would be a
    // corner on its own.
    let turns: Vec<Option<bool>> = bends
        .iter()
        .map(|&(r, corner)| (r < min_radius_mm).then_some(corner))
        .collect();
    // Vertices that still bend clearly (below CURVE_BRIDGE_FACTOR minimum
    // radii) between two tight curve vertices join them: an arc right on
    // the limit can measure a hair above it in its middle, and it is still
    // one curve.
    let bendy = |k: usize| turns[k].is_none() && bends[k].0 < CURVE_BRIDGE_FACTOR * min_radius_mm;
    let step = |k: usize, back: bool| -> Option<usize> {
        match (back, ring) {
            (true, _) if k > 0 => Some(k - 1),
            (true, true) => Some(m - 1),
            (false, _) if k + 1 < m => Some(k + 1),
            (false, true) => Some(0),
            _ => None,
        }
    };
    // From a bendy vertex, the first vertex that is not bendy in a
    // direction is a tight curve vertex.
    let ends_in_curve = |k: usize, back: bool| {
        let mut at = k;
        for _ in 0..m {
            match step(at, back) {
                Some(next) if bendy(next) => at = next,
                Some(next) => return turns[next] == Some(false),
                None => return false,
            }
        }
        false
    };
    let flags: Vec<bool> = (0..m)
        .map(|k| {
            turns[k].is_some() || (bendy(k) && ends_in_curve(k, true) && ends_in_curve(k, false))
        })
        .collect();
    if ring && flags.iter().all(|&f| f) {
        return vec![halfway(points)];
    }
    // On a ring, start the scan after a vertex that is not tight, so a run
    // never splits at the start.
    let first = if ring {
        flags.iter().position(|&f| !f).map_or(0, |k| k + 1)
    } else {
        0
    };
    let mut spots = Vec::new();
    let mut run: Vec<usize> = Vec::new();
    let mut close = |run: &mut Vec<usize>| {
        let lone_corner = run.len() == 1 && turns[run[0]] == Some(true);
        if !run.is_empty() && !lone_corner {
            let pts: Vec<Point> = run.iter().map(|&k| at(k)).collect();
            spots.push(halfway(&pts));
        }
        run.clear();
    };
    for q in 0..m {
        let k = (first + q) % m;
        if flags[k] {
            run.push(k);
        } else {
            close(&mut run);
        }
    }
    close(&mut run);
    spots
}

/// The point at half the length of `pts`.
fn halfway(pts: &[Point]) -> Point {
    let total: f64 = pts.windows(2).map(|w| w[0].distance(w[1])).sum();
    let mut left = total / 2.0;
    for w in pts.windows(2) {
        let len = w[0].distance(w[1]);
        if left <= len && len > 0.0 {
            return w[0].lerp(w[1], left / len);
        }
        left -= len;
    }
    pts[0]
}

/// The first knife value of `s` that is out of range, for `plan`'s
/// validation: offsets, radii and the overshoot must be finite and at least
/// 0, the angles from 0 to 180 degrees, the arc step above 0 and a device
/// grid without an invalid value ([`DeviceGrid::invalid_value`]), which is
/// the value reported.
pub(crate) fn invalid_value(s: &PipelineSettings) -> Option<f64> {
    let mut bad = Vec::new();
    if let Compensation::Host(h) = s.compensation {
        let length = |v: f64| !(v.is_finite() && v >= 0.0);
        let angle = |v: f64| !(v.is_finite() && (0.0..=180.0).contains(&v));
        bad.extend(
            [
                (h.offset_mm, length(h.offset_mm)),
                (h.hole_lead_in_radius_mm, length(h.hole_lead_in_radius_mm)),
                (h.pivot_threshold_deg, angle(h.pivot_threshold_deg)),
                (h.swivel_max_deg, angle(h.swivel_max_deg)),
                (
                    h.arc_step_deg,
                    angle(h.arc_step_deg) || h.arc_step_deg == 0.0,
                ),
                (
                    h.device_grid.and_then(|g| g.invalid_value()).unwrap_or(0.0),
                    h.device_grid.is_some_and(|g| g.invalid_value().is_some()),
                ),
            ]
            .into_iter()
            .filter(|&(_, b)| b),
        );
    }
    if let CornerMode::LiftOvershoot { overshoot_mm } = s.corner_mode
        && !(overshoot_mm.is_finite() && overshoot_mm >= 0.0)
    {
        bad.push((overshoot_mm, true));
    }
    bad.first().map(|&(v, _)| v)
}

/// The toolpath of an alignment stroke: one open waste move.
fn stroke_toolpath(stroke: &AlignmentStroke, source: PathRef) -> Toolpath {
    let dir = unit_or(stroke.direction, ASSUMED_START_HEADING);
    let mut t = Toolpath::new(
        vec![stroke.start, stroke.start + stroke.length_mm * dir],
        false,
        source,
    );
    t.kind = ToolpathKind::Alignment;
    t
}

/// The blade heading after `t`: the direction of its last move, or
/// `before` when it has none.
fn heading_after(t: &Toolpath, before: Vec2) -> Vec2 {
    t.points
        .windows(2)
        .rev()
        .map(|w| w[1] - w[0])
        .find(|v| v.hypot() > 0.0)
        .map_or(before, Vec2::normalize)
}

fn unit_or(v: Vec2, fallback: Vec2) -> Vec2 {
    let len = v.hypot();
    if len > 0.0 && len.is_finite() {
        v / len
    } else {
        fallback
    }
}

/// The signed turn in radians from `a` to `b`, in (-pi, pi]. An exact
/// reversal turns -pi: counter-clockwise as seen on the canvas (y down),
/// the sense of `RingDirection::Ccw`.
fn signed_turn(a: Vec2, b: Vec2) -> f64 {
    let t = a.cross(b).atan2(a.dot(b));
    if t.abs() == std::f64::consts::PI {
        -std::f64::consts::PI
    } else {
        t
    }
}

/// `v` turned by `angle` radians (positive turns +x towards +y).
fn rotate(v: Vec2, angle: f64) -> Vec2 {
    let (sin, cos) = angle.sin_cos();
    Vec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
}

/// One axis run under construction.
#[derive(Default)]
struct Run {
    points: Vec<Point>,
    seg: Vec<u32>,
}

impl Run {
    /// Add `p`, unless it is the same point as the last one. The first
    /// point is the drop point and always a waste move.
    fn push(&mut self, p: Point, seg: u32) {
        match self.points.last() {
            Some(last) if last.distance(p) < SAME_POINT_MM => {}
            Some(_) => {
                self.points.push(p);
                self.seg.push(seg);
            }
            None => {
                self.points.push(p);
                self.seg.push(WASTE_SEG);
            }
        }
    }

    fn finish(self) -> AxisRun {
        AxisRun {
            points: self.points,
            seg: self.seg,
        }
    }
}

/// How a run start may be aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Start {
    /// An open path: it has no waste side.
    Open,
    /// The start of a closed ring. `waste_corner`: P0 is a corner whose
    /// forward extension lies in waste.
    Ring { waste_corner: bool },
    /// The restart after a LiftOvershoot lift.
    Restart,
}

/// Host compensation of one job.
struct Knife<'a> {
    d: f64,
    hs: HostSettings,
    corner: CornerMode,
    kept: &'a KeptMaterial,
    area: Rect,
}

impl Knife<'_> {
    /// The axis runs of the `Cut` toolpath `t`, entered with heading `h`.
    fn runs(&self, t: &Toolpath, h: Vec2) -> Vec<AxisRun> {
        let p = &t.points;
        let d = self.d;
        // The segments with length, with their unit tangents. A
        // zero-length segment has no tangent and moves nothing.
        let segs: Vec<(usize, Vec2)> = p
            .windows(2)
            .enumerate()
            .filter_map(|(i, w)| {
                let v = w[1] - w[0];
                let len = v.hypot();
                (len > 0.0).then(|| (i, v / len))
            })
            .collect();
        let Some(&(first, t0)) = segs.first() else {
            let mut run = Run::default();
            if let Some(&p0) = p.first() {
                run.push(p0 + d * h, WASTE_SEG);
            }
            return vec![run.finish()];
        };
        let area = if t.closed {
            signed_area(rings::ring_of(t.lap()))
        } else {
            0.0
        };
        let start = if !t.closed {
            Start::Open
        } else {
            Start::Ring {
                waste_corner: self.start_is_waste_corner(t, area, t0),
            }
        };
        let (lead_end, closing) = lift_exempt(t);

        let mut runs = Vec::new();
        let mut run = Run::default();
        self.align(&mut run, p[first], h, t0, start);
        for (k, &(i, ti)) in segs.iter().enumerate() {
            run.push(p[i + 1] + d * ti, i as u32);
            let Some(&(j, tj)) = segs.get(k + 1) else {
                break;
            };
            let v = p[j];
            let turn = signed_turn(ti, tj);
            let lift = match self.corner {
                CornerMode::LiftOvershoot { overshoot_mm } => (t.closed
                    && turn.abs().to_degrees() >= CORNER_DEG
                    && j < closing
                    && lead_end.is_none_or(|end| i + 1 > end)
                    && waste_side(t.keep, area, ti, tj)
                    && self.overshoot_ok(v, ti, overshoot_mm))
                .then_some(overshoot_mm),
                CornerMode::Arc => None,
            };
            let lift = lift.filter(|&e| self.in_area(v + (e + d) * ti));
            if let Some(e) = lift {
                run.push(v + (e + d) * ti, WASTE_SEG);
                runs.push(std::mem::take(&mut run).finish());
                self.align(&mut run, v, ti, tj, Start::Restart);
            } else if turn.abs().to_degrees() > self.hs.pivot_threshold_deg {
                self.arc(&mut run, v, ti, tj, i as u32);
            }
        }
        runs.push(run.finish());
        runs
    }

    /// Whether a closed ring starts at a waste-side corner: `points[0]` is
    /// the first vertex of the lap, the turn there is a corner and its
    /// forward extension lies in waste. A pass that starts on its overcut
    /// (a reversed pass) does not start at a corner.
    fn start_is_waste_corner(&self, t: &Toolpath, area: f64, t0: Vec2) -> bool {
        if t.lap_range.start != 0 {
            return false;
        }
        let Some(t_in) = t
            .lap()
            .windows(2)
            .rev()
            .map(|w| w[1] - w[0])
            .find(|v| v.hypot() > 0.0)
        else {
            return false;
        };
        let t_in = t_in.normalize();
        signed_turn(t_in, t0).abs().to_degrees() >= CORNER_DEG && waste_side(t.keep, area, t_in, t0)
    }

    /// Push the axis points of an arc of radius d round `c`, from the
    /// heading `from` to `to` the short way, at most `arc_step_deg` apart.
    /// The last point is exactly `c + d * to`.
    fn arc(&self, run: &mut Run, c: Point, from: Vec2, to: Vec2, seg: u32) {
        let turn = signed_turn(from, to);
        let m = (turn.abs().to_degrees() / self.hs.arc_step_deg)
            .ceil()
            .max(1.0) as usize;
        for k in 1..m {
            run.push(c + self.d * rotate(from, turn * k as f64 / m as f64), seg);
        }
        run.push(c + self.d * to, seg);
    }

    /// Start a run at P0 = `p0` with the blade at heading `h`, so the tip
    /// is on P0 and the blade along `t0` when the axis reaches
    /// `p0 + d * t0`. With theta the turn from `h` to `t0`:
    ///
    /// - theta <= [`ALIGNED_DEG`]: no swivel arc, the tip drops on P0.
    /// - An open path: a swivel in place (it has no waste side, and a
    ///   lead-in would lengthen it).
    /// - A ring start at a waste-side corner with theta <= `swivel_max_deg`:
    ///   a swivel in place.
    /// - Otherwise a tractrix lead-in on the backward extension, then the
    ///   arc lead-in, then a swivel in place. A restart after a lift tries
    ///   the lead-in first.
    fn align(&self, run: &mut Run, p0: Point, h: Vec2, t0: Vec2, start: Start) {
        let theta = signed_turn(h, t0).abs().to_degrees();
        let swivel = theta <= ALIGNED_DEG
            || self.d <= 0.0
            || match start {
                Start::Open => true,
                Start::Ring { waste_corner } => waste_corner && theta <= self.hs.swivel_max_deg,
                Start::Restart => false,
            };
        if swivel || !(self.lead_in(run, p0, h, t0) || self.hole_arc(run, p0, h, t0)) {
            self.swivel(run, p0, h, t0);
        }
    }

    /// Drop with the tip on `p0` and swivel the blade from `h` to `t0`.
    fn swivel(&self, run: &mut Run, p0: Point, h: Vec2, t0: Vec2) {
        run.push(p0 + self.d * h, WASTE_SEG);
        self.arc(run, p0, h, t0, WASTE_SEG);
    }

    /// The tractrix lead-in. The blade swivels in place only by the turn
    /// above `swivel_max_deg` (to h'), then the axis runs a length L along
    /// the axis line of the first segment, so the tip follows a tractrix
    /// onto the backward extension and reaches P0 with its angle down to
    /// th1 = asin(ALGORITHM_LIMIT_MM / 2d). L is clamped to 2d..8d.
    fn lead_in(&self, run: &mut Run, p0: Point, h: Vec2, t0: Vec2) -> bool {
        let d = self.d;
        let turn = signed_turn(h, t0);
        let theta = turn.abs();
        let thr = theta.min(self.hs.swivel_max_deg.to_radians());
        let h2 = rotate(h, turn.signum() * (theta - thr));
        let th1 = (ALGORITHM_LIMIT_MM / (2.0 * d)).min(1.0).asin();
        let l = (d * ((thr / 2.0).tan() / (th1 / 2.0).tan()).ln()).clamp(2.0 * d, 8.0 * d);
        let a1 = p0 + d * t0 - l * t0;
        let td = a1 - d * h2;
        if !self.drop_ok(td) {
            return false;
        }
        // The tip along the tractrix: phi is the angle from t0 to the
        // tip-to-axis direction, and tan(phi / 2) decays as exp(-s / d).
        let phi0 = signed_turn(t0, h2);
        let n = (l / CHECK_STEP_MM).ceil().max(1.0) as usize;
        let ok = (1..=n).all(|k| {
            let s = l * k as f64 / n as f64;
            let phi = 2.0 * ((phi0 / 2.0).tan() * (-s / d).exp()).atan();
            let tip = a1 + s * t0 - d * rotate(t0, phi);
            self.path_ok(tip, p0, true)
        });
        if !ok {
            return false;
        }
        let mut moves = Run::default();
        moves.push(td + d * h, WASTE_SEG);
        if theta - thr > 0.0 {
            self.arc(&mut moves, td, h, h2, WASTE_SEG);
        }
        moves.push(a1, WASTE_SEG);
        self.take(run, moves, p0, t0)
    }

    /// Append the waste axis points `moves` and then `p0 + d * t0` to
    /// `run`, if every one of `moves` lies in the cutting area: the device
    /// addresses the axis, not the tip. Otherwise push nothing and return
    /// false, so the next option is tried.
    fn take(&self, run: &mut Run, moves: Run, p0: Point, t0: Vec2) -> bool {
        if !moves.points.iter().all(|&q| self.in_area(q)) {
            return false;
        }
        for q in moves.points {
            run.push(q, WASTE_SEG);
        }
        run.push(p0 + self.d * t0, WASTE_SEG);
        true
    }

    /// The arc lead-in: a 90 deg tip arc of radius rho =
    /// `hole_lead_in_radius_mm` that ends tangent to `t0` at P0, on the
    /// waste side: the side the waste check accepts. The drop point needs
    /// the clearance; the rest of the arc only needs to be waste. Its axis
    /// points are [`hole_arc_step_deg`] apart. There is no smaller radius
    /// to fall back to: the default is already the smallest the Task 10
    /// sweep tried, so a hole too small for it swivels in place.
    fn hole_arc(&self, run: &mut Run, p0: Point, h: Vec2, t0: Vec2) -> bool {
        let d = self.d;
        let quarter = std::f64::consts::FRAC_PI_2;
        let normal = t0.turn_90();
        let rho = self.hs.hole_lead_in_radius_mm;
        for n in [normal, -normal] {
            // The tip at angle a before P0, and its direction of motion.
            let tip = |a: f64| p0 + rho * (1.0 - a.cos()) * n - rho * a.sin() * t0;
            let dir = |a: f64| a.cos() * t0 - a.sin() * n;
            let td = tip(quarter);
            if !self.drop_ok(td) {
                continue;
            }
            let samples = (rho * quarter / CHECK_STEP_MM).ceil().max(1.0) as usize;
            let ok = (1..samples).all(|k| {
                let a = quarter * (1.0 - k as f64 / samples as f64);
                self.path_ok(tip(a), p0, false)
            });
            if !ok {
                continue;
            }
            let mut moves = Run::default();
            moves.push(td + d * h, WASTE_SEG);
            self.arc(&mut moves, td, h, dir(quarter), WASTE_SEG);
            let step = hole_arc_step_deg(rho, self.hs.arc_step_deg);
            let m = (90.0 / step).ceil().max(1.0) as usize;
            for k in 1..m {
                let a = quarter * (1.0 - k as f64 / m as f64);
                moves.push(tip(a) + d * dir(a), WASTE_SEG);
            }
            if self.take(run, moves, p0, t0) {
                return true;
            }
        }
        false
    }

    /// Whether the overshoot of `e` past the vertex `v` along `t` passes
    /// the waste check.
    fn overshoot_ok(&self, v: Point, t: Vec2, e: f64) -> bool {
        let n = (e / CHECK_STEP_MM).ceil().max(1.0) as usize;
        (1..=n).all(|k| self.path_ok(v + e * k as f64 / n as f64 * t, v, true))
    }

    fn in_area(&self, p: Point) -> bool {
        const EPS: f64 = 1e-6;
        let a = self.area;
        p.is_finite()
            && (a.x0 - EPS..=a.x1 + EPS).contains(&p.x)
            && (a.y0 - EPS..=a.y1 + EPS).contains(&p.y)
    }

    /// A drop point: in the cutting area, in waste and at least
    /// [`WASTE_CLEARANCE_MM`] off every cut.
    fn drop_ok(&self, p: Point) -> bool {
        self.in_area(p)
            && !self.kept.is_kept(p)
            && self.kept.distance_to_cut(p) >= WASTE_CLEARANCE_MM
    }

    /// One sample of a waste move that joins the path at `join` (review
    /// C3). It lies in the cutting area and in waste. Within 2d of the join
    /// the tip is on its way onto the cut, so it may touch kept material
    /// by at most [`ALGORITHM_LIMIT_MM`]; farther off it also keeps
    /// [`WASTE_CLEARANCE_MM`] off every cut when `clearance` is set.
    fn path_ok(&self, p: Point, join: Point, clearance: bool) -> bool {
        if !self.in_area(p) {
            return false;
        }
        if p.distance(join) <= 2.0 * self.d {
            return !self.kept.is_kept(p) || self.kept.distance_to_cut(p) <= ALGORITHM_LIMIT_MM;
        }
        !self.kept.is_kept(p) && (!clearance || self.kept.distance_to_cut(p) >= WASTE_CLEARANCE_MM)
    }
}

/// The angle between two axis points of the arc lead-in of radius `rho`:
/// the chord error rule. The step is
/// `min(arc_step_deg, 2 * acos(1 - (ALGORITHM_LIMIT_MM / 4) / rho))`, so
/// the sagitta `rho * (1 - cos(step / 2))` of the tip arc between two
/// points stays at or below a quarter of [`ALGORITHM_LIMIT_MM`] for any
/// radius. The tip follows the arc onto P0, so its chord error becomes the
/// start error. Corner pivot arcs (radius d, the tip stays on the vertex)
/// keep `arc_step_deg`.
pub fn hole_arc_step_deg(rho: f64, arc_step_deg: f64) -> f64 {
    let cos = (1.0 - ALGORITHM_LIMIT_MM / 4.0 / rho).clamp(-1.0, 1.0);
    arc_step_deg.min(2.0 * cos.acos().to_degrees())
}

/// The vertices of a closed ring where LiftOvershoot does not apply, as
/// `(lead_end, closing)` indices into `points`:
///
/// - Every vertex at or after `closing`, the end of the last lap when the
///   overcut follows it (Ruling F7): the overcut stays in Arc mode, also
///   where it runs past a short first segment. `usize::MAX` when there is
///   none (an open path, no overcut, or a reversed pass, whose lap ends
///   the toolpath).
/// - In a reversed pass, which starts with its overcut stretch before the
///   lap, every vertex up to and including `lead_end`, the lap start where
///   the overcut joins the ring. `None` when the lap starts the toolpath.
fn lift_exempt(t: &Toolpath) -> (Option<usize>, usize) {
    if !t.closed {
        return (None, usize::MAX);
    }
    let lead_end = (t.lap_range.start > 0).then_some(t.lap_range.start);
    let lap = t.lap_range.len().saturating_sub(1);
    let end = t.lap_range.start + t.laps as usize * lap;
    let closing = if end + 1 < t.points.len() {
        end
    } else {
        usize::MAX
    };
    (lead_end, closing)
}

/// Whether the forward extension of `t_in` past a vertex where the path
/// turns to `t_out` lies in waste, for a ring of signed area `area` kept
/// on side `keep` ([`rings::waste_extension`], the rule of
/// `rings::extension_in_waste`). An open path (`KeepSide::Both`) has no
/// waste side.
fn waste_side(keep: KeepSide, area: f64, t_in: Vec2, t_out: Vec2) -> bool {
    rings::waste_extension(t_in.cross(t_out), area, keep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::containment::KeepMode;
    use crate::plan::{
        AlignmentStroke, KeepSide, PathRef, PlacedPath, PlanError, SettingsField, ToolpathKind,
        WASTE_SEG, plan,
    };
    use flatcraft_core::document::ObjectId;
    use kurbo::{Arc, BezPath, Rect, Shape};

    const D: f64 = 0.3;

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    fn source() -> PathRef {
        PathRef {
            object: ObjectId(1),
            path_index: 0,
        }
    }

    fn area() -> Rect {
        Rect::new(0.0, 0.0, 300.0, 300.0)
    }

    fn host(pivot_threshold_deg: f64) -> PipelineSettings {
        PipelineSettings {
            overcut_mm: 0.0,
            compensation: Compensation::Host(HostSettings::new(D, pivot_threshold_deg)),
            ..PipelineSettings::with_defaults(area(), Point::ZERO)
        }
    }

    fn kept_of(paths: &[Toolpath]) -> KeptMaterial {
        KeptMaterial::from_toolpaths(paths, |_| KeepMode::Pieces, area())
    }

    /// `paths` through `apply` with `s`, starting with heading `h`.
    fn applied(mut paths: Vec<Toolpath>, s: &PipelineSettings, h: Vec2) -> Vec<Toolpath> {
        let kept = kept_of(&paths);
        let got = apply(&mut paths, s, &kept, h);
        if let Compensation::Host(hs) = s.compensation {
            assert_eq!(
                got,
                AppliedCompensation::Host {
                    offset_mm: hs.offset_mm
                }
            );
        }
        paths
    }

    fn rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
        let mut b = BezPath::new();
        b.move_to((x, y));
        b.line_to((x + w, y));
        b.line_to((x + w, y + h));
        b.line_to((x, y + h));
        b.close_path();
        b
    }

    fn placed(path: BezPath) -> PlacedPath {
        PlacedPath {
            path,
            source: source(),
        }
    }

    /// Structural.
    #[test]
    fn straight_segment_ends_offset_along_tangent() {
        let line = Toolpath::new(vec![p(0.3, 0.0), p(10.3, 0.0)], false, source());
        let out = applied(vec![line], &host(0.0), Vec2::new(1.0, 0.0));
        let runs = out[0].axis.clone().unwrap();
        assert_eq!(runs.len(), 1);
        let want = [p(0.3 + D, 0.0), p(10.3 + D, 0.0)];
        assert_eq!(runs[0].points, want);
        assert_eq!(runs[0].seg, vec![WASTE_SEG, 0]);
    }

    /// Structural: the axis runs round every vertex on a circle of radius
    /// d, at most `ARC_STEP_DEG` apart, carrying the segment into it.
    #[test]
    fn every_vertex_gets_an_arc() {
        let mut ring = Toolpath::new(
            vec![
                p(10.0, 10.0),
                p(20.0, 10.0),
                p(20.0, 20.0),
                p(10.0, 20.0),
                p(10.0, 10.0),
            ],
            true,
            source(),
        );
        ring.keep = KeepSide::Inside;
        let out = applied(vec![ring.clone()], &host(0.0), Vec2::new(1.0, 0.0));
        let runs = out[0].axis.clone().unwrap();
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        for j in 1..4 {
            let v = ring.points[j];
            let on: Vec<usize> = (0..run.points.len())
                .filter(|&k| (run.points[k].distance(v) - D).abs() < 1e-9)
                .collect();
            // The segment end plus 90 / ARC_STEP_DEG arc points.
            let arc = (90.0 / ARC_STEP_DEG).ceil() as usize;
            assert_eq!(on.len(), 1 + arc, "vertex {j}: {on:?}");
            assert!(on.iter().all(|&k| run.seg[k] == j as u32 - 1));
            for w in on.windows(2) {
                let (a, b) = (run.points[w[0]] - v, run.points[w[1]] - v);
                let step = a.cross(b).atan2(a.dot(b)).abs().to_degrees();
                assert!(step <= ARC_STEP_DEG + 1e-9, "{step}");
            }
        }
        assert_eq!(
            *run.points.last().unwrap(),
            p(10.0, 10.0) + D * Vec2::new(0.0, -1.0)
        );
    }

    /// Structural: a turn at or below the pivot threshold gets no arc.
    #[test]
    fn pivot_threshold_skips_small_turns() {
        let a = 5f64.to_radians();
        let path = vec![
            p(10.0, 10.0),
            p(20.0, 10.0),
            p(20.0 + 10.0 * a.cos(), 10.0 + 10.0 * a.sin()),
        ];
        let line = Toolpath::new(path.clone(), false, source());
        let flat = applied(vec![line.clone()], &host(10.0), Vec2::new(1.0, 0.0));
        let arcs = applied(vec![line], &host(0.0), Vec2::new(1.0, 0.0));
        assert_eq!(flat[0].axis.as_ref().unwrap()[0].points.len(), 3);
        assert_eq!(arcs[0].axis.as_ref().unwrap()[0].points.len(), 4);
    }

    /// Structural: an exact hairpin turns the way of `RingDirection::Ccw`
    /// on the canvas (y down), that is towards -y from +x.
    #[test]
    fn hairpin_turns_counter_clockwise_on_the_canvas() {
        let line = Toolpath::new(
            vec![p(10.0, 10.0), p(20.0, 10.0), p(10.0, 10.0)],
            false,
            source(),
        );
        let out = applied(vec![line], &host(0.0), Vec2::new(1.0, 0.0));
        let run = &out[0].axis.as_ref().unwrap()[0];
        let v = p(20.0, 10.0);
        let arc: Vec<Point> = run
            .points
            .iter()
            .copied()
            .filter(|q| (q.distance(v) - D).abs() < 1e-9)
            .collect();
        assert!(arc.iter().all(|q| q.y <= 10.0 + 1e-9), "{arc:?}");
        assert!(arc.iter().any(|q| (*q - p(20.0, 10.0 - D)).hypot() < 1e-9));
    }

    #[test]
    fn tip_mode_leaves_axis_none() {
        let s = PipelineSettings {
            alignment_stroke: Some(AlignmentStroke {
                start: p(5.0, 5.0),
                direction: Vec2::new(0.0, 1.0),
                length_mm: 5.0,
            }),
            ..PipelineSettings::with_defaults(area(), Point::ZERO)
        };
        let out = plan(&[placed(rect_path(10.0, 10.0, 10.0, 10.0))], &s).unwrap();
        assert_eq!(out.compensation, AppliedCompensation::Tip);
        assert_eq!(out.toolpaths.len(), 2);
        assert_eq!(out.toolpaths[0].kind, ToolpathKind::Alignment);
        assert_eq!(out.toolpaths[0].points, vec![p(5.0, 5.0), p(5.0, 10.0)]);
        assert_eq!(out.toolpaths[1].kind, ToolpathKind::Cut);
        assert!(out.toolpaths.iter().all(|t| t.axis.is_none()));
    }

    #[test]
    fn host_mode_sets_axis_on_every_toolpath() {
        let mut line = BezPath::new();
        line.move_to((50.0, 50.0));
        line.line_to((60.0, 55.0));
        let s = PipelineSettings {
            alignment_stroke: Some(AlignmentStroke {
                start: p(5.0, 5.0),
                direction: Vec2::new(0.0, 2.0),
                length_mm: 5.0,
            }),
            ..host(0.0)
        };
        let out = plan(
            &[placed(rect_path(10.0, 10.0, 10.0, 10.0)), placed(line)],
            &s,
        )
        .unwrap();
        assert_eq!(out.compensation, AppliedCompensation::Host { offset_mm: D });
        assert_eq!(out.toolpaths.len(), 3);
        assert!(out.toolpaths.iter().all(|t| t.axis.is_some()));
        let stroke = &out.toolpaths[0].axis.as_ref().unwrap()[0];
        assert_eq!(stroke.points, vec![p(5.0, 5.0), p(5.0, 10.0)]);
        assert_eq!(stroke.seg, vec![WASTE_SEG, WASTE_SEG]);
        // Every run starts with a drop point that executes no segment.
        for t in &out.toolpaths {
            for r in t.axis.as_ref().unwrap() {
                assert_eq!(r.seg.len(), r.points.len());
                assert_eq!(r.seg[0], WASTE_SEG);
                assert!(r.points.windows(2).all(|w| w[0].distance(w[1]) >= 1e-9));
            }
        }
        // Blade-down length is the axis length.
        let axis: f64 = out
            .toolpaths
            .iter()
            .flat_map(Toolpath::runs)
            .map(|r| crate::clean::length(&r.points))
            .sum();
        assert!((out.cut_mm - axis).abs() < 1e-9);
    }

    /// Ruling F9: the head and the blade heading after the last toolpath.
    #[test]
    fn plan_reports_the_end_and_its_heading() {
        let mut line = BezPath::new();
        line.move_to((50.0, 50.0));
        line.line_to((50.0, 60.0));
        let tip = PipelineSettings::with_defaults(area(), Point::ZERO);
        let out = plan(&[placed(line.clone())], &tip).unwrap();
        assert_eq!(
            (out.end, out.end_heading),
            (p(50.0, 60.0), Vec2::new(0.0, 1.0))
        );
        let out = plan(&[placed(line)], &host(0.0)).unwrap();
        assert_eq!(out.end, p(50.0, 60.0 + D));
        assert_eq!(out.end_heading, Vec2::new(0.0, 1.0));
        // A reversed even pass of a ring ends at the ring start, coming
        // back along the overcut.
        let s = PipelineSettings {
            passes: 2,
            reverse_alternate_passes: true,
            ..PipelineSettings::with_defaults(area(), Point::ZERO)
        };
        let out = plan(&[placed(rect_path(10.0, 10.0, 10.0, 10.0))], &s).unwrap();
        let last = out.toolpaths.last().unwrap();
        assert_eq!(out.end, *last.points.last().unwrap());
        assert_eq!(out.end, last.lap()[0]);
        let n = last.points.len();
        let dir = (last.points[n - 1] - last.points[n - 2]).normalize();
        assert!((out.end_heading - dir).hypot() < 1e-12);
    }

    #[test]
    fn bad_knife_settings_are_refused() {
        let bad_host = |f: fn(&mut HostSettings)| {
            let mut h = HostSettings::new(D, 0.0);
            f(&mut h);
            PipelineSettings {
                compensation: Compensation::Host(h),
                ..host(0.0)
            }
        };
        let cases = [
            bad_host(|h| h.offset_mm = -0.1),
            bad_host(|h| h.offset_mm = f64::NAN),
            bad_host(|h| h.pivot_threshold_deg = -1.0),
            bad_host(|h| h.swivel_max_deg = 200.0),
            bad_host(|h| h.hole_lead_in_radius_mm = f64::INFINITY),
            bad_host(|h| h.arc_step_deg = 0.0),
            bad_host(|h| h.device_grid = Some(DeviceGrid::new(0.0, Point::ZERO))),
            bad_host(|h| h.device_grid = Some(DeviceGrid::new(20.0, Point::new(f64::NAN, 0.0)))),
            PipelineSettings {
                corner_mode: CornerMode::LiftOvershoot { overshoot_mm: -1.0 },
                ..host(0.0)
            },
        ];
        for s in cases {
            let got = plan(&[placed(rect_path(10.0, 10.0, 10.0, 10.0))], &s);
            assert!(
                matches!(
                    got,
                    Err(PlanError::InvalidSettings {
                        field: SettingsField::Knife,
                        ..
                    })
                ),
                "{got:?}"
            );
        }
        let s = PipelineSettings {
            alignment_stroke: Some(AlignmentStroke {
                start: p(5.0, 5.0),
                direction: Vec2::ZERO,
                length_mm: 5.0,
            }),
            ..host(0.0)
        };
        let got = plan(&[placed(rect_path(10.0, 10.0, 10.0, 10.0))], &s);
        assert!(
            matches!(
                got,
                Err(PlanError::InvalidSettings {
                    field: SettingsField::AlignmentStroke,
                    ..
                })
            ),
            "{got:?}"
        );
        for field in [SettingsField::Knife, SettingsField::AlignmentStroke] {
            let msg = PlanError::InvalidSettings { field, value: -1.0 }.to_string();
            assert!(msg.starts_with("The "), "{msg}");
            assert!(!msg.contains('\u{2014}') && !msg.contains('_'), "{msg}");
        }
    }

    /// Structural: the arc lead-in keeps its sagitta at or below a quarter
    /// of `ALGORITHM_LIMIT_MM` for every radius (the chord error rule).
    #[test]
    fn hole_arc_sagitta_stays_below_a_quarter_of_the_limit() {
        let empty = kept_of(&[]);
        let (p0, t0, h) = (p(100.0, 100.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0));
        for rho in [0.5, 1.0, 2.0, 4.0] {
            let mut hs = HostSettings::new(D, 0.0);
            hs.hole_lead_in_radius_mm = rho;
            let knife = Knife {
                d: D,
                hs,
                corner: CornerMode::Arc,
                kept: &empty,
                area: area(),
            };
            let mut run = Run::default();
            assert!(knife.hole_arc(&mut run, p0, h, t0));
            // The arc centre: P0 plus rho along the side the arc took.
            let target = p0 + D * t0;
            let end = run
                .points
                .iter()
                .position(|q| q.distance(target) < 1e-9)
                .unwrap();
            let c = [1.0, -1.0]
                .map(|side| p0 + side * rho * t0.turn_90())
                .into_iter()
                .find(|c| (run.points[end - 1].distance(*c) - rho.hypot(D)).abs() < 1e-9)
                .unwrap();
            // The axis arc points lie at rho.hypot(d) from the centre, at the
            // tip arc's angles. The drop point (before the swivel) also lies
            // on that circle, so it is left out.
            let arc: Vec<Vec2> = run.points[1..=end]
                .iter()
                .map(|q| *q - c)
                .filter(|v| (v.hypot() - rho.hypot(D)).abs() < 1e-9)
                .collect();
            assert!(arc.len() >= 7, "rho {rho}: {}", arc.len());
            let max_step = arc
                .windows(2)
                .map(|w| w[0].cross(w[1]).atan2(w[0].dot(w[1])).abs())
                .fold(0.0, f64::max);
            let sagitta = rho * (1.0 - (max_step / 2.0).cos());
            assert!(
                sagitta <= ALGORITHM_LIMIT_MM / 4.0 + 1e-12,
                "rho {rho}: {sagitta}"
            );
            assert!(max_step.to_degrees() <= ARC_STEP_DEG + 1e-9);
        }
    }

    /// The vertex index where run `r` lifts: the end of its last path move.
    fn lift_index(run: &AxisRun) -> usize {
        let last = run.seg.iter().rposition(|&s| s != WASTE_SEG).unwrap();
        run.seg[last] as usize + 1
    }

    /// Ruling F7: no lift inside the overcut, even where it runs past a
    /// first segment shorter than itself.
    #[test]
    fn no_lift_inside_the_overcut() {
        let mut ring = Toolpath::new(
            vec![
                p(10.0, 10.0),
                p(10.3, 10.0),
                p(10.3, 20.0),
                p(10.0, 20.0),
                p(10.0, 10.0),
            ],
            true,
            source(),
        );
        ring.keep = KeepSide::Inside;
        let ring = crate::passes::chain(vec![ring], 1, false, 0.5).remove(0);
        // The overcut runs over the 0.3 mm first segment and its corner.
        assert_eq!(ring.points.len(), 7);
        let s = PipelineSettings {
            corner_mode: CornerMode::LiftOvershoot { overshoot_mm: 0.1 },
            ..host(0.0)
        };
        let out = applied(vec![ring], &s, Vec2::new(0.0, -1.0));
        let runs = out[0].axis.as_ref().unwrap();
        let lifts: Vec<usize> = runs[..runs.len() - 1].iter().map(lift_index).collect();
        // The three lap corners after the start lift; the closing vertex
        // (4) and the corner inside the overcut (5) do not.
        assert_eq!(lifts, vec![1, 2, 3]);
    }

    /// The mirror of `no_lift_inside_the_overcut`: a reversed pass starts
    /// with its overcut stretch, and nothing up to and including the lap
    /// start lifts.
    #[test]
    fn no_lift_inside_the_leading_overcut_of_a_reversed_pass() {
        let mut ring = Toolpath::new(
            vec![
                p(10.0, 10.0),
                p(10.3, 10.0),
                p(10.3, 20.0),
                p(10.0, 20.0),
                p(10.0, 10.0),
            ],
            true,
            source(),
        );
        ring.keep = KeepSide::Inside;
        let ring = crate::passes::separate(vec![ring], 2, true, 0.5).remove(1);
        // Reversed: the overcut (back over the 0.3 mm segment and its
        // corner) comes first, then the lap from index 2.
        assert_eq!(ring.points.len(), 7);
        assert_eq!(ring.lap_range, 2..7);
        let s = PipelineSettings {
            corner_mode: CornerMode::LiftOvershoot { overshoot_mm: 0.1 },
            ..host(0.0)
        };
        let out = applied(vec![ring], &s, Vec2::new(0.0, 1.0));
        let runs = out[0].axis.as_ref().unwrap();
        let lifts: Vec<usize> = runs[..runs.len() - 1].iter().map(lift_index).collect();
        // The corner inside the overcut (1) and the lap start (2) do not
        // lift; the three lap corners after it do.
        assert_eq!(lifts, vec![3, 4, 5]);
    }

    #[test]
    fn min_radius_run_across_a_ring_start_is_one_spot() {
        // A square with one corner rounded at r = 0.2, closed. The ring
        // starts in the middle of the tight arc.
        let (corner, mid) = rounded_corner(0.2);
        let arc: Vec<Point> = corner[1..corner.len() - 1].to_vec();
        let k = arc.len() / 2;
        let mut ring: Vec<Point> = arc[k..].to_vec();
        ring.extend([p(50.2, 70.0), p(30.0, 70.0), p(30.0, 49.8)]);
        ring.extend_from_slice(&arc[..k]);
        ring.push(ring[0]);
        let spots = min_radius_spots(&ring, 0.25);
        assert_eq!(spots.len(), 1, "{spots:?}");
        assert!(spots[0].distance(mid) <= 0.05, "{spots:?} vs {mid:?}");
    }

    /// A line, a 90 deg arc of radius `r` flattened at 0.01 mm, a line.
    fn rounded_corner(r: f64) -> (Vec<Point>, Point) {
        let c = p(50.0, 50.0);
        let arc = Arc::new(
            c,
            (r, r),
            -std::f64::consts::FRAC_PI_2,
            std::f64::consts::FRAC_PI_2,
            0.0,
        );
        let mut pts = vec![p(40.0, 50.0 - r)];
        let mut b = BezPath::new();
        b.move_to(c + Vec2::new(0.0, -r));
        for el in arc.path_elements(0.1) {
            b.push(el);
        }
        let flat = crate::flatten::flatten(&b, 0.01);
        for el in flat.elements() {
            match el {
                kurbo::PathEl::MoveTo(q) | kurbo::PathEl::LineTo(q) => pts.push(*q),
                _ => {}
            }
        }
        pts.dedup();
        pts.push(p(50.0 + r, 60.0));
        let mid = c + r * Vec2::new(1.0, -1.0) / 2f64.sqrt();
        (pts, mid)
    }

    #[test]
    fn min_radius_spots_found() {
        let (pts, mid) = rounded_corner(0.2);
        let spots = min_radius_spots(&pts, 0.25);
        assert!(
            spots.iter().any(|s| s.distance(mid) <= 0.05),
            "{spots:?} vs {mid:?}"
        );
        let (pts, _) = rounded_corner(1.0);
        assert_eq!(min_radius_spots(&pts, 0.25), Vec::<Point>::new());
    }

    /// Every rounded corner of radius 0.25 mm warns once at the minimum
    /// radius 0.25 mm, however coarsely it is flattened.
    #[test]
    fn a_rounded_corner_warns_at_any_flatten_tolerance() {
        for tol in [0.005, 0.01, 0.02, 0.03, 0.05] {
            let r = 0.25;
            // A rectangle 10 x 6 mm with corners of radius r, closed.
            let rect = kurbo::RoundedRect::new(10.0, 10.0, 20.0, 16.0, r);
            let flat = crate::flatten::flatten(&rect.to_path(0.01), tol);
            let mut pts: Vec<Point> = Vec::new();
            for el in flat.elements() {
                match el {
                    kurbo::PathEl::MoveTo(q) | kurbo::PathEl::LineTo(q) => pts.push(*q),
                    kurbo::PathEl::ClosePath => pts.push(pts[0]),
                    _ => {}
                }
            }
            pts.dedup();
            let spots = min_radius_spots(&pts, 0.25);
            // One spot per corner: the radius sits right on the limit, and
            // a corner's arc that measures a hair above it in its middle is
            // still one curve.
            assert_eq!(spots.len(), 4, "tolerance {tol}: {spots:?}");
            for c in [p(10.0, 10.0), p(20.0, 10.0), p(20.0, 16.0), p(10.0, 16.0)] {
                assert!(
                    spots.iter().any(|q| q.distance(c) < 0.25),
                    "tolerance {tol}: no spot at {c:?}: {spots:?}"
                );
            }
        }
    }

    /// A sharp corner between short segments is a corner, not a tight
    /// curve: the blade swivels there. A tiny circle is still a curve.
    #[test]
    fn a_lone_sharp_corner_is_not_a_tight_curve() {
        // 98 deg between segments of about 0.4 mm (radius 0.24 mm by the
        // formula), as in a glyph outline.
        let pts = [
            p(208.68, 91.85),
            p(208.93, 92.15),
            p(208.58, 92.37),
            p(208.24, 92.59),
        ];
        assert_eq!(min_radius_spots(&pts, 0.25), Vec::<Point>::new());
        // The same short segments turning 20 deg are a tight wiggle.
        let wiggle = [p(0.0, 0.0), p(0.05, 0.0), p(0.097, 0.017), p(0.15, 0.017)];
        assert_eq!(min_radius_spots(&wiggle, 0.25).len(), 1);
        let ring: Vec<Point> = (0..=12)
            .map(|k| {
                let a = f64::from(k) / 12.0 * std::f64::consts::TAU;
                p(10.0 + 0.1 * a.cos(), 10.0 + 0.1 * a.sin())
            })
            .collect();
        assert_eq!(min_radius_spots(&ring, 0.25).len(), 1);
    }
}
