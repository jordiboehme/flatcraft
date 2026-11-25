//! The knife stage (`flatcraft_path::knife`) judged by simulation: float
//! mode, d = 0.3 mm, the planner's offset equal to the real one.

use flatcraft_core::document::ObjectId;
use flatcraft_path::knife::{self, WASTE_CLEARANCE_MM};
use flatcraft_path::{
    AlignmentStroke, AxisRun, Compensation, CornerMode, CutPlan, HostSettings, KeepMode, KeepSide,
    KeptMaterial, PathRef, PipelineSettings, PlacedPath, Toolpath, ToolpathKind, WASTE_SEG, plan,
};
use flatcraft_sim::{ALGORITHM_LIMIT_MM, JobMetrics, SimSettings, SimTrace, measure, simulate};
use kurbo::{BezPath, Circle, Point, Rect, Shape, Vec2};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed};

const D: f64 = 0.3;

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn area() -> Rect {
    Rect::new(0.0, 0.0, 300.0, 300.0)
}

fn host(corner_mode: CornerMode) -> PipelineSettings {
    PipelineSettings {
        compensation: Compensation::Host(HostSettings::new(D, 0.0)),
        corner_mode,
        ..PipelineSettings::with_defaults(area(), Point::ZERO)
    }
}

const LIFT: CornerMode = CornerMode::LiftOvershoot { overshoot_mm: 0.1 };

/// A fixed seed, so CI on every OS runs the same cases (Ruling F30).
fn fixed(cases: u32) -> Config {
    Config {
        cases,
        rng_seed: RngSeed::Fixed(0x6b6e_6966_6539),
        failure_persistence: None,
        ..Config::default()
    }
}

fn source(object: u64) -> PathRef {
    PathRef {
        object: ObjectId(object),
        path_index: 0,
    }
}

fn polygon(pts: &[Point]) -> BezPath {
    let mut b = BezPath::new();
    b.move_to(pts[0]);
    for &q in &pts[1..] {
        b.line_to(q);
    }
    b.close_path();
    b
}

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    polygon(&[p(x, y), p(x + w, y), p(x + w, y + h), p(x, y + h)])
}

fn l_shape() -> BezPath {
    polygon(&[
        p(10.0, 10.0),
        p(30.0, 10.0),
        p(30.0, 20.0),
        p(20.0, 20.0),
        p(20.0, 30.0),
        p(10.0, 30.0),
    ])
}

const L_CONCAVE: Point = Point::new(20.0, 20.0);

fn placed(object: u64, path: BezPath) -> PlacedPath {
    PlacedPath {
        path,
        source: source(object),
    }
}

fn sim_settings(start_heading: Vec2) -> SimSettings {
    SimSettings {
        start_heading,
        ..SimSettings::float(D)
    }
}

fn simulated(paths: &[Toolpath], home: Point, h: Vec2) -> (SimTrace, JobMetrics) {
    let s = sim_settings(h);
    let trace = simulate(paths, home, &s).unwrap();
    let (m, _) = measure(paths, &trace, &s);
    (trace, m)
}

fn planned(input: &[PlacedPath], s: &PipelineSettings) -> (CutPlan, SimTrace, JobMetrics) {
    let out = plan(input, s).unwrap();
    let (trace, m) = simulated(&out.toolpaths, out.home, s.start_heading);
    (out, trace, m)
}

/// `paths` through `knife::apply` with the job's kept material.
fn applied(mut paths: Vec<Toolpath>, s: &PipelineSettings, h: Vec2) -> Vec<Toolpath> {
    let kept =
        KeptMaterial::from_toolpaths(&paths, |_| KeepMode::Pieces, area(), WASTE_CLEARANCE_MM);
    knife::apply(&mut paths, s, &kept, h);
    paths
}

fn runs(t: &Toolpath) -> &[AxisRun] {
    t.axis.as_deref().expect("host compensation sets the axis")
}

/// Where run `r` of `t` starts on the path: the vertex P0 its first path
/// move starts from, the tangent t0 there and the index of `P0 + d * t0`
/// in the run (where the alignment ends).
fn run_start(t: &Toolpath, r: usize) -> (Point, Vec2, usize) {
    let run = &runs(t)[r];
    let first = run
        .seg
        .iter()
        .position(|&s| s != WASTE_SEG)
        .expect("a run cuts the path");
    let j = run.seg[first] as usize;
    let (p0, t0) = (t.points[j], (t.points[j + 1] - t.points[j]).normalize());
    let target = p0 + D * t0;
    let at = run.points[..first]
        .iter()
        .position(|q| q.distance(target) < 1e-9)
        .expect("the alignment ends at P0 + d * t0");
    (p0, t0, at)
}

/// The simulated tip when the axis of run `r` of toolpath `i` first
/// reaches `target`.
fn tip_at(trace: &SimTrace, i: usize, r: usize, target: Point) -> Point {
    trace
        .samples
        .iter()
        .find(|x| x.down && x.toolpath == i && x.run == r && x.axis.distance(target) < 1e-9)
        .expect("the axis reaches the target")
        .tip
}

/// Every run of every `Cut` toolpath starts aligned: the tip is within
/// `ALGORITHM_LIMIT_MM` of P0 when the axis first reaches `P0 + d * t0`.
fn assert_starts_aligned(paths: &[Toolpath], trace: &SimTrace) {
    for (i, t) in paths.iter().enumerate() {
        if t.kind != ToolpathKind::Cut {
            continue;
        }
        for r in 0..runs(t).len() {
            let (p0, t0, _) = run_start(t, r);
            let tip = tip_at(trace, i, r, p0 + D * t0);
            assert!(
                tip.distance(p0) <= ALGORITHM_LIMIT_MM,
                "toolpath {i} run {r}: tip {tip:?} is {} from {p0:?}",
                tip.distance(p0)
            );
        }
    }
}

/// The index in `points` where run `r` of `t` lifts: the end of its last
/// path move.
fn lift_index(t: &Toolpath, r: usize) -> usize {
    let run = &runs(t)[r];
    let last = run.seg.iter().rposition(|&s| s != WASTE_SEG).unwrap();
    run.seg[last] as usize + 1
}

/// The vertex where run `r` of `t` lifts: the end of its last path move.
fn lift_vertex(t: &Toolpath, r: usize) -> Point {
    let run = &runs(t)[r];
    let last = run.seg.iter().rposition(|&s| s != WASTE_SEG).unwrap();
    t.points[run.seg[last] as usize + 1]
}

fn heading(deg: f64) -> Vec2 {
    Vec2::from_angle(deg.to_radians())
}

#[test]
fn the_lead_in_limit_is_the_algorithm_limit() {
    assert_eq!(knife::ALGORITHM_LIMIT_MM, ALGORITHM_LIMIT_MM);
}

/// A star-shaped ring around `c`: `n` vertices at increasing angles.
fn star_ring(c: Point, radii: &[f64], jitter: &[f64]) -> BezPath {
    let n = radii.len();
    let pts: Vec<Point> = (0..n)
        .map(|k| {
            let a = std::f64::consts::TAU * (k as f64 + jitter[k]) / n as f64;
            c + radii[k] * Vec2::from_angle(a)
        })
        .collect();
    polygon(&pts)
}

proptest! {
    #![proptest_config(fixed(48))]

    #[test]
    fn every_start_is_aligned(
        radii in prop::collection::vec(3.0..15.0f64, 3..9),
        jitter in prop::collection::vec(0.0..0.8f64, 9),
        start in 0.0..360.0f64,
        line_dir in 0.0..360.0f64,
        line_len in 3.0..30.0f64,
        lift in any::<bool>(),
    ) {
        let mut line = BezPath::new();
        line.move_to((150.0, 150.0));
        line.line_to(p(150.0, 150.0) + line_len * heading(line_dir));
        let s = PipelineSettings {
            start_heading: heading(start),
            ..host(if lift { LIFT } else { CornerMode::Arc })
        };
        let input = [
            placed(1, star_ring(p(60.0, 60.0), &radii, &jitter)),
            placed(2, line),
        ];
        let (out, trace, m) = planned(&input, &s);
        assert_starts_aligned(&out.toolpaths, &trace);
        prop_assert_eq!(m.pushed, 0);
    }
}

fn square_ring(x: f64, y: f64, s: f64, object: u64) -> Toolpath {
    Toolpath::new(
        vec![p(x, y), p(x + s, y), p(x + s, y + s), p(x, y + s), p(x, y)],
        true,
        source(object),
    )
}

#[test]
fn lead_in_reaches_the_start() {
    let mut ring = square_ring(10.0, 10.0, 10.0, 1);
    ring.keep = KeepSide::Inside;
    // Entered at (10, 10) along +x, coming from a 135 deg turn.
    let h = heading(135.0);
    let out = applied(vec![ring], &host(CornerMode::Arc), h);
    let (p0, t0, at) = run_start(&out[0], 0);
    assert_eq!((p0, t0), (p(10.0, 10.0), Vec2::new(1.0, 0.0)));
    let run = &runs(&out[0])[0];
    let lead = run.points[at] - run.points[at - 1];
    assert!(
        lead.cross(t0).abs() < 1e-9 && lead.dot(t0) > 0.0,
        "{lead:?}"
    );
    assert!(
        (0.6..=2.4).contains(&lead.hypot()),
        "lead-in {} mm",
        lead.hypot()
    );
    let (trace, m) = simulated(&out, p(0.0, 0.0), h);
    let tip = tip_at(&trace, 0, 0, p0 + D * t0);
    assert!(
        tip.distance(p0) <= ALGORITHM_LIMIT_MM,
        "{}",
        tip.distance(p0)
    );
    assert_eq!(m.pushed, 0);
    assert_eq!(m.max_into_kept_mm, 0.0);
}

/// A circle of `n` vertices from angle 0, in the given turning sense.
fn circle_ring(c: Point, r: f64, n: usize, object: u64, positive: bool) -> Toolpath {
    let mut pts: Vec<Point> = (0..=n)
        .map(|k| {
            let a = std::f64::consts::TAU * (k % n) as f64 / n as f64;
            c + r * Vec2::from_angle(if positive { a } else { -a })
        })
        .collect();
    pts.dedup();
    Toolpath::new(pts, true, source(object))
}

#[test]
fn round_hole_gets_an_arc_lead_in() {
    // A 10 mm round hole in a 40 mm square piece. The order cuts the hole
    // first; its start tangent is +y or -y, the blade comes along +x.
    for positive in [true, false] {
        let mut hole = circle_ring(p(40.0, 40.0), 5.0, 180, 1, positive);
        hole.keep = KeepSide::Outside;
        let mut piece = square_ring(20.0, 20.0, 40.0, 1);
        piece.keep = KeepSide::Inside;
        let h = Vec2::new(1.0, 0.0);
        let out = applied(vec![hole, piece], &host(CornerMode::Arc), h);
        let (p0, t0, at) = run_start(&out[0], 0);
        let run = &runs(&out[0])[0];
        // Not a straight lead-in and not a swivel in place: the drop tip
        // lies off the cut, inside the hole.
        let drop_tip = run.points[0] - D * h;
        assert!(drop_tip.distance(p0) > 0.5, "{drop_tip:?}");
        assert!(drop_tip.distance(p(40.0, 40.0)) < 5.0 - WASTE_CLEARANCE_MM);
        assert!((run.points[at] - run.points[at - 1]).cross(t0).abs() > 1e-6);
        let (trace, m) = simulated(&out, p(0.0, 0.0), h);
        assert_eq!(m.pushed, 0);
        assert_eq!(m.max_into_kept_mm, 0.0);
        assert!(
            m.per_path[0].max_start_deviation_mm <= ALGORITHM_LIMIT_MM,
            "{}",
            m.per_path[0].max_start_deviation_mm
        );
        let tip = tip_at(&trace, 0, 0, p0 + D * t0);
        assert!(
            tip.distance(p0) <= ALGORITHM_LIMIT_MM,
            "{}",
            tip.distance(p0)
        );
    }
}

/// The start-alignment assertion on the arc lead-in: round holes of
/// several sizes, entered at several headings. A round hole has no usable
/// backward extension, so every start takes the arc lead-in.
#[test]
fn every_arc_lead_in_start_is_aligned() {
    for r in [3.0, 5.0, 10.0, 20.0] {
        for deg in [0.0, 30.0, 60.0, 120.0, 150.0, 180.0, 210.0, 330.0] {
            for positive in [true, false] {
                let c = p(60.0, 60.0);
                let mut hole = circle_ring(c, r, 180, 1, positive);
                hole.keep = KeepSide::Outside;
                let mut piece = square_ring(c.x - r - 5.0, c.y - r - 5.0, 2.0 * r + 10.0, 1);
                piece.keep = KeepSide::Inside;
                let h = heading(deg);
                let out = applied(vec![hole, piece], &host(CornerMode::Arc), h);
                let case = format!("r {r}, heading {deg}, positive {positive}");
                // The arc lead-in branch: the drop tip lies off the cut,
                // inside the hole, and the last move onto P0 + d * t0 is
                // not a straight lead-in.
                let (p0, t0, at) = run_start(&out[0], 0);
                let run = &runs(&out[0])[0];
                let drop_tip = run.points[0] - D * h;
                assert!(drop_tip.distance(p0) > 0.4, "{case}: {drop_tip:?}");
                assert!(
                    drop_tip.distance(c) < r - WASTE_CLEARANCE_MM,
                    "{case}: {drop_tip:?}"
                );
                assert!(
                    (run.points[at] - run.points[at - 1]).cross(t0).abs() > 1e-6,
                    "{case}"
                );
                let (trace, m) = simulated(&out, p(0.0, 0.0), h);
                assert_starts_aligned(&out, &trace);
                assert_eq!(m.pushed, 0, "{case}");
                assert_eq!(m.max_into_kept_mm, 0.0, "{case}");
            }
        }
    }
}

#[test]
fn open_path_swivels_in_place() {
    let line = Toolpath::new(vec![p(10.0, 10.0), p(30.0, 10.0)], false, source(1));
    let h = Vec2::new(0.0, 1.0);
    let out = applied(vec![line], &host(CornerMode::Arc), h);
    let (p0, t0, at) = run_start(&out[0], 0);
    let run = &runs(&out[0])[0];
    // No lead-in: every alignment point lies on the circle of radius d
    // around P0.
    assert!(at >= 6);
    for q in &run.points[..=at] {
        assert!((q.distance(p0) - D).abs() < 1e-9, "{q:?}");
    }
    assert_eq!(run.points[0], p0 + D * h);
    let (trace, m) = simulated(&out, p(0.0, 0.0), h);
    let tip = tip_at(&trace, 0, 0, p0 + D * t0);
    assert!(
        tip.distance(p0) <= ALGORITHM_LIMIT_MM,
        "{}",
        tip.distance(p0)
    );
    assert_eq!(m.pushed, 0);
}

/// One cell of the nesting test, `size` wide at `x`.
fn cell(kind: u8, x: f64, y: f64, size: f64, inset: f64, object: u64) -> Vec<PlacedPath> {
    let c = p(x + size / 2.0, y + size / 2.0);
    let ring = |r: f64| Circle::new(c, r).to_path(0.01);
    let r = size / 2.0;
    let paths = match kind {
        0 => vec![rect_path(x, y, size, size)],
        1 => vec![
            rect_path(x, y, size, size),
            rect_path(x + inset, y + inset, size - 2.0 * inset, size - 2.0 * inset),
        ],
        2 => {
            let i2 = inset + (size / 2.0 - inset) / 2.0;
            vec![
                rect_path(x, y, size, size),
                rect_path(x + inset, y + inset, size - 2.0 * inset, size - 2.0 * inset),
                rect_path(x + i2, y + i2, size - 2.0 * i2, size - 2.0 * i2),
            ]
        }
        3 => vec![ring(r)],
        _ => vec![ring(r), ring(r - inset)],
    };
    paths.into_iter().map(|b| placed(object, b)).collect()
}

/// How a run starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Branch {
    Swivel,
    LeadIn,
    HoleArc,
}

/// The alignment branch run `r` of `t` took: every alignment point on the
/// circle of radius d round P0 is a swivel, a last move along t0 of at
/// least 2d is a lead-in, anything else is the arc lead-in.
fn branch(t: &Toolpath, r: usize) -> Branch {
    let (p0, t0, at) = run_start(t, r);
    let run = &runs(t)[r];
    if run.points[..=at]
        .iter()
        .all(|q| (q.distance(p0) - D).abs() < 1e-9)
    {
        return Branch::Swivel;
    }
    let last = run.points[at] - run.points[at - 1];
    if last.cross(t0).abs() < 1e-9 && last.dot(t0) >= 2.0 * D - 1e-9 {
        Branch::LeadIn
    } else {
        Branch::HoleArc
    }
}

/// Random nestings plus neighbours 0.5 mm apart: no waste move touches
/// kept material. The fixed cases reach both the lead-in and the arc
/// lead-in, so the check is not vacuous for either.
#[test]
fn waste_moves_never_touch_kept_material() {
    use proptest::test_runner::TestRunner;
    let strategy = (
        prop::collection::vec((0u8..5, 4.0..12.0f64, 0.0..1.0f64), 2..7),
        0.0..360.0f64,
        any::<bool>(),
        1u32..3,
    );
    let seen = std::cell::RefCell::new(Vec::new());
    let mut runner = TestRunner::new(fixed(24));
    runner
        .run(&strategy, |(cells, start, lift, passes)| {
            // A row of objects 0.5 mm apart, some with holes and pieces in
            // their holes.
            let mut input = Vec::new();
            let mut x = 20.0;
            for (k, &(kind, size, frac)) in cells.iter().enumerate() {
                let inset = 0.5 + frac * (size / 4.0 - 0.5);
                input.extend(cell(kind, x, 20.0, size, inset, k as u64 + 1));
                x += size + 0.5;
            }
            let s = PipelineSettings {
                start_heading: heading(start),
                passes,
                ..host(if lift { LIFT } else { CornerMode::Arc })
            };
            let (out, _, m) = planned(&input, &s);
            for t in &out.toolpaths {
                for r in 0..runs(t).len() {
                    seen.borrow_mut().push(branch(t, r));
                }
            }
            prop_assert_eq!(m.max_into_kept_mm, 0.0);
            Ok(())
        })
        .unwrap();
    for b in [Branch::LeadIn, Branch::HoleArc, Branch::Swivel] {
        assert!(seen.borrow().contains(&b), "no {b:?} in the fixed cases");
    }
}

/// Reversed passes start on their overcut. The vertices inside it, up to
/// the lap start, never lift, so no run is a short stub.
#[test]
fn reversed_passes_keep_their_overcut_in_one_run() {
    for corner in [CornerMode::Arc, LIFT] {
        let s = PipelineSettings {
            passes: 2,
            reverse_alternate_passes: true,
            ..host(corner)
        };
        let input = [
            placed(1, l_shape()),
            placed(2, rect_path(50.0, 10.0, 0.3, 10.0)),
        ];
        let (out, trace, m) = planned(&input, &s);
        assert_eq!(out.toolpaths.len(), 4);
        assert_starts_aligned(&out.toolpaths, &trace);
        assert_eq!(m.pushed, 0, "{corner:?}");
        assert_eq!(m.max_into_kept_mm, 0.0, "{corner:?}");
        let shortest = s.overcut_mm + 2.0 * D;
        for (i, t) in out.toolpaths.iter().enumerate() {
            let r = runs(t);
            for (k, run) in r.iter().enumerate() {
                let len: f64 = run.points.windows(2).map(|w| w[0].distance(w[1])).sum();
                assert!(len >= shortest, "{corner:?} toolpath {i} run {k}: {len}");
            }
            // Where the lifts are: never inside the leading overcut of a
            // reversed pass, up to and including the lap start.
            for k in 0..r.len() - 1 {
                let at = lift_index(t, k);
                assert!(
                    at > t.lap_range.start,
                    "{corner:?} toolpath {i} lifts at {at}, inside its overcut"
                );
            }
            // The L lifts at its 4 convex corners after the start in both
            // directions: 5 runs. A lift at the lap start would make 6.
            if corner == LIFT && t.source.object == ObjectId(1) {
                assert_eq!(r.len(), 5, "toolpath {i} (pass {})", t.pass);
            }
        }
        assert!(out.toolpaths.iter().any(|t| t.lap_range.start > 0));
    }
}

/// An exact hairpin with a pivot threshold above 0 still gets its arc: the
/// tip stays on the vertex while the blade turns, and nothing is pushed.
#[test]
fn hairpin_with_a_pivot_threshold_turns_on_the_vertex() {
    let s = PipelineSettings {
        compensation: Compensation::Host(HostSettings::new(D, 2.0)),
        ..PipelineSettings::with_defaults(area(), Point::ZERO)
    };
    let v = p(30.0, 10.0);
    let line = Toolpath::new(vec![p(10.0, 10.0), v, p(10.0, 10.0)], false, source(1));
    let h = Vec2::new(1.0, 0.0);
    let out = applied(vec![line], &s, h);
    let (trace, m) = simulated(&out, p(0.0, 0.0), h);
    assert_eq!(m.pushed, 0);
    let samples = &trace.samples;
    let arrive = samples
        .iter()
        .position(|x| x.down && x.axis.distance(v + D * h) < 1e-9)
        .unwrap();
    let leave = arrive
        + samples[arrive..]
            .iter()
            .position(|x| x.axis.distance(v - D * h) < 1e-9)
            .unwrap();
    assert!(leave > arrive + 6);
    for x in &samples[arrive..=leave] {
        assert!(
            x.tip.distance(v) <= ALGORITHM_LIMIT_MM,
            "{}",
            x.tip.distance(v)
        );
    }
    assert!(m.max_correspondence_mm <= ALGORITHM_LIMIT_MM);
}

/// A turn at or below the pivot threshold gets no arc, and the tip then
/// strays as the model predicts: the axis leaves the vertex d * sin(theta)
/// off the next segment's axis line, and the trailing tip deviates by just
/// under that. With an arc the same turn stays within the limit.
#[test]
fn skipped_pivots_deviate_as_the_model_predicts() {
    for deg in [1.0f64, 2.0, 5.0, 10.0] {
        let a = deg.to_radians();
        let pts = vec![
            p(10.0, 10.0),
            p(30.0, 10.0),
            p(30.0, 10.0) + 20.0 * heading(deg),
        ];
        let line = Toolpath::new(pts, false, source(1));
        let measured = |threshold: f64| {
            let s = PipelineSettings {
                compensation: Compensation::Host(HostSettings::new(D, threshold)),
                ..PipelineSettings::with_defaults(area(), Point::ZERO)
            };
            let h = Vec2::new(1.0, 0.0);
            let out = applied(vec![line.clone()], &s, h);
            let (_, m) = simulated(&out, p(0.0, 0.0), h);
            assert_eq!(m.pushed, 0);
            m.max_correspondence_mm
        };
        let predicted = D * a.sin();
        let skipped = measured(20.0);
        assert!(
            (0.9 * predicted..=predicted).contains(&skipped),
            "{deg} deg: {skipped} vs {predicted}"
        );
        assert!(measured(0.0) <= ALGORITHM_LIMIT_MM, "{deg} deg");
    }
}

/// The device addresses the axis: an overshoot whose axis would leave the
/// cutting area is refused, and the corner gets its arc instead. A ring
/// 0.2 mm from the right edge of the grid.
#[test]
fn waste_axis_points_stay_in_the_cutting_area() {
    for corner in [CornerMode::Arc, LIFT] {
        let input = [placed(1, rect_path(289.8, 100.0, 10.0, 10.0))];
        let s = PipelineSettings {
            start_heading: heading(150.0),
            ..host(corner)
        };
        let (out, trace, m) = planned(&input, &s);
        let t = &out.toolpaths[0];
        // Axis points off the grid are only the ones every compensated
        // path has: within d of a path vertex.
        for run in runs(t) {
            for q in &run.points {
                if !area().contains(*q) {
                    let near = t.points.iter().any(|v| v.distance(*q) <= D + 1e-9);
                    assert!(near, "{corner:?}: {q:?} is off the grid");
                }
            }
        }
        assert_starts_aligned(&out.toolpaths, &trace);
        assert_eq!(m.max_into_kept_mm, 0.0);
    }
}

/// Within 2d of the join the check samples finely enough that a kept
/// sliver 0.05 mm wide across the lead-in cannot fall between samples.
#[test]
fn a_thin_kept_sliver_near_the_join_refuses_the_lead_in() {
    let mut ring = square_ring(10.0, 10.0, 10.0, 1);
    ring.keep = KeepSide::Inside;
    // A separate piece 0.05 mm wide, 0.25 mm before the start corner,
    // across the backward extension of the first segment.
    let mut sliver = Toolpath::new(
        vec![
            p(9.7, 9.0),
            p(9.75, 9.0),
            p(9.75, 11.0),
            p(9.7, 11.0),
            p(9.7, 9.0),
        ],
        true,
        source(2),
    );
    sliver.keep = KeepSide::Inside;
    let h = heading(135.0);
    let out = applied(vec![ring, sliver], &host(CornerMode::Arc), h);
    assert_ne!(branch(&out[0], 0), Branch::LeadIn);
    let (trace, m) = simulated(&out, p(0.0, 0.0), h);
    for x in trace.samples.iter().filter(|x| x.down && x.toolpath == 0) {
        let inside = x.tip.x > 9.7 + ALGORITHM_LIMIT_MM
            && x.tip.x < 9.75 - ALGORITHM_LIMIT_MM
            && (9.0..=11.0).contains(&x.tip.y);
        assert!(!inside, "{:?}", x.tip);
    }
    assert_eq!(m.max_into_kept_mm, 0.0);
}

#[test]
fn lift_overshoot_only_on_waste_side_corners() {
    let (out, trace, m) = planned(&[placed(1, l_shape())], &host(LIFT));
    let t = &out.toolpaths[0];
    let start = t.points[0];
    assert_ne!(start, L_CONCAVE, "entered at a convex corner");
    let r = runs(t);
    assert_eq!(r.len(), 5, "split at the 4 other convex corners");
    let mut lifts: Vec<Point> = (0..r.len() - 1).map(|k| lift_vertex(t, k)).collect();
    lifts.sort_by(|a, b| (a.x, a.y).partial_cmp(&(b.x, b.y)).unwrap());
    let mut convex: Vec<Point> = [
        p(10.0, 10.0),
        p(30.0, 10.0),
        p(30.0, 20.0),
        p(20.0, 30.0),
        p(10.0, 30.0),
    ]
    .into_iter()
    .filter(|&q| q != start)
    .collect();
    convex.sort_by(|a, b| (a.x, a.y).partial_cmp(&(b.x, b.y)).unwrap());
    assert_eq!(lifts, convex);
    // Each lift overshoots 0.1 mm straight on, the axis d further.
    for (k, run) in r[..r.len() - 1].iter().enumerate() {
        let n = run.points.len();
        assert_eq!(run.seg[n - 1], WASTE_SEG);
        let v = lift_vertex(t, k);
        assert!((run.points[n - 1].distance(v) - (0.1 + D)).abs() < 1e-9);
    }
    // The concave corner and the closing corner get arcs.
    let all: Vec<Point> = r.iter().flat_map(|run| run.points.clone()).collect();
    for v in [L_CONCAVE, start] {
        let on = all
            .iter()
            .filter(|q| (q.distance(v) - D).abs() < 1e-9)
            .count();
        assert!(on >= 6, "{v:?}: {on}");
    }
    let last = r.last().unwrap();
    let closing_arc = last
        .points
        .iter()
        .zip(&last.seg)
        .filter(|(q, s)| **s != WASTE_SEG && (q.distance(start) - D).abs() < 1e-9)
        .count();
    assert!(closing_arc >= 6, "{closing_arc}");
    assert_starts_aligned(&out.toolpaths, &trace);
    assert_eq!(m.pushed, 0);
    assert_eq!(m.max_into_kept_mm, 0.0);
}

/// The vertex where one lap joins the next is a normal corner: it lifts in
/// LiftOvershoot mode. Only the closing vertex of the last lap is exempt.
#[test]
fn lift_overshoot_lifts_at_the_lap_join() {
    let s = PipelineSettings {
        passes: 2,
        ..host(LIFT)
    };
    let (out, trace, m) = planned(&[placed(1, l_shape())], &s);
    assert_eq!(out.toolpaths.len(), 1);
    let t = &out.toolpaths[0];
    assert_eq!(t.laps, 2);
    let r = runs(t);
    // 4 convex corners per lap plus the lap join.
    assert_eq!(r.len(), 10);
    let lifts: Vec<Point> = (0..r.len() - 1).map(|k| lift_vertex(t, k)).collect();
    assert_eq!(lifts.iter().filter(|&&q| q == t.points[0]).count(), 1);
    assert!(!lifts.contains(&L_CONCAVE));
    assert_starts_aligned(&out.toolpaths, &trace);
    assert_eq!(m.pushed, 0);
    assert_eq!(m.max_into_kept_mm, 0.0);
}

#[test]
fn lift_overshoot_leaves_curves_alone() {
    let circle = Circle::new((60.0, 60.0), 10.0).to_path(0.01);
    let (out, _, m) = planned(&[placed(1, circle)], &host(LIFT));
    assert_eq!(runs(&out.toolpaths[0]).len(), 1);
    assert_eq!(m.lifts, 1);
}

#[test]
fn lift_overshoot_restart_uses_lead_in() {
    let (out, trace, _) = planned(&[placed(1, l_shape())], &host(LIFT));
    let t = &out.toolpaths[0];
    let r = runs(t);
    assert!(r.len() > 1);
    for (k, run) in r.iter().enumerate().skip(1) {
        let (p0, t0, at) = run_start(t, k);
        assert_eq!(at, 1, "run {k}: a lead-in, then the path");
        let lead = run.points[1] - run.points[0];
        assert!(lead.cross(t0).abs() < 1e-9 && lead.dot(t0) > 0.0, "run {k}");
        assert!(
            (2.0 * D..=8.0 * D).contains(&lead.hypot()),
            "run {k}: {}",
            lead.hypot()
        );
        let tip = tip_at(&trace, 0, k, p0 + D * t0);
        assert!(tip.distance(p0) <= ALGORITHM_LIMIT_MM);
    }
}

#[test]
fn alignment_stroke_is_first_run_and_sets_heading() {
    let mut line = BezPath::new();
    line.move_to((10.0, 10.0));
    line.line_to((30.0, 10.0));
    let s = PipelineSettings {
        alignment_stroke: Some(AlignmentStroke {
            start: p(5.0, 5.0),
            direction: Vec2::new(0.0, 1.0),
            length_mm: 5.0,
        }),
        ..host(CornerMode::Arc)
    };
    // The real blade starts at some other heading: the stroke turns it.
    let out = plan(&[placed(1, line)], &s).unwrap();
    let (trace, m) = simulated(&out.toolpaths, out.home, heading(200.0));
    assert_eq!(out.toolpaths[0].kind, ToolpathKind::Alignment);
    assert_eq!(runs(&out.toolpaths[0]).len(), 1);
    let cut = &out.toolpaths[1];
    let (p0, _, _) = run_start(cut, 0);
    // The first cut's swivel starts from +y.
    assert_eq!(runs(cut)[0].points[0], p0 + D * Vec2::new(0.0, 1.0));
    assert_starts_aligned(&out.toolpaths, &trace);
    assert_eq!(m.pushed, 0);
}

/// Ruling F3: with two chained laps the kept material still comes from one
/// lap, so a lead-in from a hole corner into the frame around it is
/// refused.
#[test]
fn waste_check_holds_with_passes_2() {
    let mut frame = rect_path(20.0, 20.0, 30.0, 30.0);
    frame.extend(rect_path(30.0, 30.0, 10.0, 10.0));
    let s = PipelineSettings {
        passes: 2,
        // Not along any edge, so every start turns and tries a lead-in.
        start_heading: heading(45.0),
        ..host(CornerMode::Arc)
    };
    let (out, trace, m) = planned(&[placed(1, frame)], &s);
    assert!(out.toolpaths.iter().all(|t| t.laps == 2));
    let hole = out
        .toolpaths
        .iter()
        .position(|t| t.keep == KeepSide::Outside)
        .unwrap();
    // The hole has no waste-side corner and no room for an arc: the blade
    // swivels in place.
    let (p0, _, at) = run_start(&out.toolpaths[hole], 0);
    for q in &runs(&out.toolpaths[hole])[0].points[..=at] {
        assert!((q.distance(p0) - D).abs() < 1e-9, "{q:?}");
    }
    assert_starts_aligned(&out.toolpaths, &trace);
    assert_eq!(m.max_into_kept_mm, 0.0);
    assert_eq!(m.pushed, 0);
}
