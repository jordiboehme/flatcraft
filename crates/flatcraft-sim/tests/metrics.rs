use flatcraft_core::document::ObjectId;
use flatcraft_path::{
    AxisRun, PathRef, PipelineSettings, PlacedPath, Toolpath, ToolpathKind, WASTE_SEG, plan,
};
use flatcraft_sim::{
    DeviceGrid, Grade, JobMetrics, SimSettings, SimTrace, SpotKind, TimeModel, UNCUT_LIMIT_MM,
    estimate_seconds, measure, run, simulate,
};
use kurbo::{BezPath, Circle, Point, Rect, Shape};

const W: u32 = WASTE_SEG;

fn source(object: u64) -> PathRef {
    PathRef {
        object: ObjectId(object),
        path_index: 0,
    }
}

fn pts(p: &[(f64, f64)]) -> Vec<Point> {
    p.iter().map(|&p| p.into()).collect()
}

/// A closed square ring from `(x, y)`, first edge along +x.
fn square(x: f64, y: f64, size: f64) -> Vec<Point> {
    pts(&[
        (x, y),
        (x + size, y),
        (x + size, y + size),
        (x, y + size),
        (x, y),
    ])
}

fn closed(points: Vec<Point>, object: u64) -> Toolpath {
    Toolpath::new(points, true, source(object))
}

fn open(points: Vec<Point>) -> Toolpath {
    Toolpath::new(points, false, source(1))
}

fn with_runs(mut t: Toolpath, runs: Vec<(Vec<Point>, Vec<u32>)>) -> Toolpath {
    t.axis = Some(
        runs.into_iter()
            .map(|(points, seg)| AxisRun { points, seg })
            .collect(),
    );
    t
}

fn sim(toolpaths: &[Toolpath], home: Point, s: &SimSettings) -> (SimTrace, JobMetrics, Vec<f32>) {
    let trace = simulate(toolpaths, home, s).unwrap();
    let (m, dev) = measure(toolpaths, &trace, s);
    assert_eq!(dev.len(), trace.samples.len());
    for (x, d) in trace.samples.iter().zip(&dev) {
        assert_eq!(x.down, !d.is_nan(), "{x:?}: {d}");
    }
    (trace, m, dev)
}

#[test]
fn perfect_tracking_scores_100() {
    let s = SimSettings::float(0.0);
    let ring = square(10.0, 10.0, 50.0);
    let (_, m, dev) = sim(&[closed(ring, 1)], Point::new(10.0, 10.0), &s);
    for d in dev.iter().filter(|d| !d.is_nan()) {
        assert!(*d < 1e-9, "{d}");
    }
    let p = &m.per_path[0];
    for (name, v) in [
        ("wander", p.wander_mm),
        ("miss", p.miss_mm),
        ("hausdorff", m.max_hausdorff_mm),
        ("correspondence", m.max_correspondence_mm),
        ("frechet", m.max_span_frechet_mm),
        ("corner", m.max_corner_deviation_mm),
        ("start", m.max_start_deviation_mm),
        ("closure", m.max_closure_gap_mm),
        ("into kept", m.max_into_kept_mm),
        ("uncut", m.uncut_mm),
        ("stray", m.stray_mm),
        ("waste", m.waste_mm),
        ("travel", m.travel_mm),
    ] {
        assert!(v.abs() < 1e-9, "{name}: {v}");
    }
    assert!(p.closure_gap_mm.is_some());
    assert!((m.cut_mm - 200.0).abs() < 1e-9, "{}", m.cut_mm);
    assert_eq!((m.segments, m.lifts, m.pushed), (4, 1, 0));
    assert_eq!(m.score, 100.0);
    assert_eq!(m.grade, Grade::Good);
    assert!(m.worst_spots.is_empty(), "{:?}", m.worst_spots);
}

#[test]
fn uncompensated_square_is_not_good() {
    let s = SimSettings::float(0.3);
    let ring = square(10.0, 10.0, 50.0);
    let corners = ring.clone();
    let (_, m, _) = sim(&[closed(ring, 1)], Point::new(10.0, 10.0), &s);
    assert!(
        m.max_corner_deviation_mm > 0.15,
        "{}",
        m.max_corner_deviation_mm
    );
    // Measured (M2-P14 rules, 2026-11-25): score 98.67, Hausdorff 0.285,
    // into-kept 0.1427 at three corners, uncut 1.125, stray 1.535. The
    // corner rounding cuts 0.14 mm into the piece, beyond the 0.05 mm path
    // tolerance, so H1 makes it Poor although the score is above 98.
    // Task 10 coarsened the sample step from d / 20 to d / 10, and the
    // sampled into-kept depth reads 0.1395 (the swing round a corner is
    // the fastest tip motion there is).
    assert!(m.score >= 98.0, "{}", m.score);
    assert!(
        (m.max_into_kept_mm - 0.1395).abs() < 0.001,
        "{}",
        m.max_into_kept_mm
    );
    assert_eq!(m.grade, Grade::Poor, "score {}", m.score);
    let first = m.worst_spots.first().expect("a worst spot");
    assert!(
        corners.iter().any(|c| c.distance(first.at) < 1.0),
        "{first:?}"
    );
}

#[test]
fn removed_segment_is_uncut() {
    let s = SimSettings::float(0.0);
    let t = with_runs(
        open(pts(&[(0.0, 0.0), (20.0, 0.0)])),
        vec![
            (pts(&[(0.0, 0.0), (5.0, 0.0)]), vec![W, 0]),
            (pts(&[(10.0, 0.0), (20.0, 0.0)]), vec![W, 0]),
        ],
    );
    let (_, m, _) = sim(&[t], Point::ZERO, &s);
    // M2-P14: the cut severs up to UNCUT_LIMIT_MM past each end of the gap.
    let expected = 5.0 - 2.0 * UNCUT_LIMIT_MM;
    assert!((m.uncut_mm - expected).abs() <= 0.05, "{}", m.uncut_mm);
    assert!(m.per_path[0].miss_mm > 1.0, "{}", m.per_path[0].miss_mm);
    assert_eq!(m.lifts, 2);
    assert!(
        m.worst_spots
            .iter()
            .any(|x| x.kind == SpotKind::Uncut && (5.0..=10.0).contains(&x.at.x)),
        "{:?}",
        m.worst_spots
    );
    assert_ne!(m.grade, Grade::Good);
}

#[test]
fn pushed_blade_is_flagged() {
    // The blade trails towards +x; the first cut runs straight at the tip.
    let s = SimSettings::float(0.3);
    let t = open(pts(&[(10.0, 0.0), (0.0, 0.0)]));
    let (_, m, _) = sim(&[t], Point::new(10.0, 0.0), &s);
    assert!(m.pushed > 0);
    assert!(m.per_path[0].pushed > 0);
    assert!(
        m.worst_spots.iter().any(|x| x.kind == SpotKind::Pushed),
        "{:?}",
        m.worst_spots
    );
    // The exact push keeps the tip on the line, so only `pushed` sees it:
    // the pushed length counts as stray. Without that the score would be
    // about 97 (only the 0.3 mm overshoot at the end strays).
    assert!((m.stray_mm - 10.0).abs() < 1e-9, "{}", m.stray_mm);
    assert!(m.score < 1.0, "{}", m.score);
    assert_ne!(m.grade, Grade::Good);
}

#[test]
fn wrong_side_trace_is_caught_by_frechet() {
    // A 0.2 mm slot. The knife cuts the far side while it executes the near one.
    let s = SimSettings::float(0.0);
    let slot = pts(&[(0.0, 0.0), (20.0, 0.0), (20.0, 0.2), (0.0, 0.2), (0.0, 0.0)]);
    let axis = pts(&[(0.0, 0.2), (20.0, 0.2), (20.0, 0.0), (0.0, 0.0), (0.0, 0.2)]);
    let t = with_runs(closed(slot, 1), vec![(axis, vec![W, 0, 1, 2, 3])]);
    let (_, m, _) = sim(&[t], Point::new(0.0, 0.2), &s);
    assert!(m.max_span_frechet_mm > 0.15, "{}", m.max_span_frechet_mm);
    // Hausdorff can not see it: every sample lies on the slot.
    assert!(m.max_hausdorff_mm < 0.01, "{}", m.max_hausdorff_mm);
}

#[test]
fn into_kept_is_measured_globally() {
    let s = SimSettings::float(0.0);
    // Piece B lies left of piece A, with a 4 mm gap of waste between them.
    let b = closed(square(-14.0, -5.0, 10.0), 2);
    let a_with_lead_in = |from: f64| {
        let mut axis = vec![Point::new(from, 0.0)];
        axis.extend(square(0.0, 0.0, 10.0));
        with_runs(
            closed(square(0.0, 0.0, 10.0), 1),
            vec![(axis, vec![W, W, 0, 1, 2, 3])],
        )
    };

    // The lead-in starts 3 mm deep in B.
    let (_, m, _) = sim(
        &[a_with_lead_in(-7.0), b.clone()],
        Point::new(-7.0, 0.0),
        &s,
    );
    assert!(
        (m.max_into_kept_mm - 3.0).abs() < 0.05,
        "{}",
        m.max_into_kept_mm
    );
    let spot = m.worst_spots.first().expect("a worst spot");
    assert_eq!(spot.kind, SpotKind::IntoKept);
    assert!(spot.at.distance(Point::new(-7.0, 0.0)) < 0.05, "{spot:?}");
    assert!((m.waste_mm - 7.0).abs() < 1e-9, "{}", m.waste_mm);
    assert_eq!(m.grade, Grade::Poor);

    // The lead-in starts in the gap: waste, and no harm done.
    let (_, m, _) = sim(&[a_with_lead_in(-3.0), b], Point::new(-3.0, 0.0), &s);
    assert_eq!(m.max_into_kept_mm, 0.0);
    assert!((m.waste_mm - 3.0).abs() < 1e-9, "{}", m.waste_mm);
    assert!(m.max_hausdorff_mm < 1e-9, "{}", m.max_hausdorff_mm);
    assert_eq!(m.grade, Grade::Good);
}

#[test]
fn open_path_inside_a_piece_is_not_into_kept() {
    // A score line inside a piece: its own cut is not kept material, and
    // neither is a cut of another object.
    let s = SimSettings::float(0.0);
    let piece = closed(square(0.0, 0.0, 20.0), 1);
    for object in [1, 2] {
        let line = Toolpath::new(pts(&[(5.0, 10.0), (15.0, 10.0)]), false, source(object));
        let (_, m, _) = sim(&[line, piece.clone()], Point::new(5.0, 10.0), &s);
        assert_eq!(m.max_into_kept_mm, 0.0, "object {object}");
        assert_eq!(m.grade, Grade::Good, "object {object}: {:?}", m.worst_spots);
    }
}

#[test]
fn kept_material_with_two_laps_is_still_kept() {
    // Ruling F3: two chained laps must not cancel the ring to waste.
    let s = SimSettings::float(0.0);
    let lap = square(0.0, 0.0, 10.0);
    let mut points = lap.clone();
    points.extend_from_slice(&lap[1..]);
    let mut t = closed(points.clone(), 1);
    t.laps = 2;
    t.lap_range = 0..lap.len();
    let mut axis = vec![Point::new(5.0, 5.0)];
    axis.extend(points);
    let seg = [W, W].into_iter().chain(0..8).collect();
    let t = with_runs(t, vec![(axis, seg)]);
    let (_, m, _) = sim(&[t], Point::new(5.0, 5.0), &s);
    assert!(
        (m.max_into_kept_mm - 5.0).abs() < 0.05,
        "{}",
        m.max_into_kept_mm
    );
}

#[test]
fn alignment_stroke_counts_as_waste() {
    let s = SimSettings::float(0.0);
    let mut stroke = open(pts(&[(-10.0, 0.0), (-5.0, 0.0)]));
    stroke.kind = ToolpathKind::Alignment;
    let piece = closed(square(0.0, 0.0, 10.0), 1);
    let (trace, m, dev) = sim(&[stroke, piece], Point::new(-10.0, 0.0), &s);
    assert!((m.waste_mm - 5.0).abs() < 1e-9, "{}", m.waste_mm);
    assert!((m.per_path[0].waste_mm - 5.0).abs() < 1e-9);
    assert_eq!(m.per_path[0].cut_mm, 0.0);
    assert!((m.cut_mm - 40.0).abs() < 1e-9, "{}", m.cut_mm);
    assert!(m.max_hausdorff_mm < 1e-9, "{}", m.max_hausdorff_mm);
    assert_eq!(m.stray_mm, 0.0);
    // A waste sample's deviation is its distance to the nearest cut.
    let drop = trace.samples.iter().position(|x| x.down).unwrap();
    assert!((f64::from(dev[drop]) - 10.0).abs() < 1e-5, "{}", dev[drop]);
}

#[test]
fn closure_gap_measures_the_final_close() {
    let s = SimSettings::float(0.0);
    let lap = square(0.0, 0.0, 10.0);
    let mut points = lap.clone();
    points.extend_from_slice(&lap[1..]);
    points.extend_from_slice(&lap[1..]);
    *points.last_mut().unwrap() = Point::new(0.0, 0.2);
    let mut t = closed(points, 1);
    t.laps = 3;
    t.lap_range = 0..lap.len();
    let (_, m, _) = sim(&[t], Point::ZERO, &s);
    let gap = m.per_path[0]
        .closure_gap_mm
        .expect("a closed path has a gap");
    assert!((gap - 0.2).abs() < 1e-6, "{gap}");
    assert!((m.max_closure_gap_mm - 0.2).abs() < 1e-6);
}

#[test]
fn travel_crossing_counted() {
    let s = SimSettings::float(0.0);
    let cut = open(pts(&[(0.0, -5.0), (0.0, 5.0)]));
    let right = open(pts(&[(3.0, 0.0), (3.0, 1.0)]));
    let left = open(pts(&[(-3.0, 0.0), (-3.0, 1.0)]));
    let home = Point::new(0.0, -5.0);

    // The travel from `right` to `left` crosses the finished cut once.
    let (_, m, _) = sim(&[cut.clone(), right.clone(), left], home, &s);
    assert_eq!(m.travel_crossings, 1);
    let travel = Point::new(0.0, 5.0).distance(Point::new(3.0, 0.0))
        + Point::new(3.0, 1.0).distance(Point::new(-3.0, 0.0));
    assert!((m.travel_mm - travel).abs() < 1e-9, "{}", m.travel_mm);

    let (_, m, _) = sim(&[cut, right], home, &s);
    assert_eq!(m.travel_crossings, 0);
}

#[test]
fn chebyshev_time() {
    let t = [open(pts(&[(0.0, 0.0), (30.0, 40.0)]))];
    let time = TimeModel::PROVISIONAL;
    // max(30, 40) / (5 * 10) + one lift
    let secs = estimate_seconds(&t, Point::ZERO, 5, &time);
    assert!((secs - 0.95).abs() < 1e-12, "{secs}");
    // Travel from the home counts the same way: 10 mm at 50 mm/s.
    let secs = estimate_seconds(&t, Point::new(-10.0, 0.0), 5, &time);
    assert!((secs - 1.15).abs() < 1e-12, "{secs}");
    let r = run(&t, Point::ZERO, &SimSettings::float(0.25), 5, &time).unwrap();
    assert!((r.seconds - 0.95).abs() < 1e-12, "{}", r.seconds);
    assert_eq!(r.deviation_mm.len(), r.trace.samples.len());
}

#[test]
fn measured_travel_and_cut_equal_the_plan() {
    let settings = PipelineSettings {
        passes: 2,
        reverse_alternate_passes: true,
        ..PipelineSettings::with_defaults(Rect::new(0.0, 0.0, 300.0, 300.0), Point::new(-5.0, -3.0))
    };
    let mut line = BezPath::new();
    line.move_to((100.0, 10.0));
    line.line_to((140.0, 60.0));
    let input: Vec<PlacedPath> = [
        Rect::new(10.0, 10.0, 40.0, 30.0).to_path(0.1),
        Circle::new((80.0, 80.0), 12.0).to_path(0.1),
        line,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, path)| PlacedPath {
        path,
        source: source(i as u64 + 1),
    })
    .collect();
    let out = plan(&input, &settings).unwrap();
    let s = SimSettings::float(0.25);
    let (_, m, _) = sim(&out.toolpaths, out.home, &s);
    assert!(
        (m.travel_mm - out.travel_mm).abs() < 1e-9,
        "{}",
        m.travel_mm
    );
    assert!((m.cut_mm - out.cut_mm).abs() < 1e-9, "{}", m.cut_mm);
    assert_eq!(m.lifts, out.toolpaths.len());
    assert_eq!(m.per_path.len(), out.toolpaths.len());
}

/// A square, then a line behind it on the right (nearer) and one on the
/// left (no crossing from the square's start corner).
fn crossing_job() -> Vec<PlacedPath> {
    let line = |a: (f64, f64), b: (f64, f64)| {
        let mut p = BezPath::new();
        p.move_to(a);
        p.line_to(b);
        p
    };
    [
        Rect::new(50.0, 50.0, 60.0, 60.0).to_path(0.1),
        line((62.0, 51.0), (62.0, 59.0)),
        line((36.0, 50.0), (35.0, 50.0)),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, path)| PlacedPath {
        path,
        source: source(i as u64 + 1),
    })
    .collect()
}

/// The simulated travel crossings and the order of [`crossing_job`] with
/// the Fast strategy, the crossing penalty on or off, and `rounds`
/// improvement rounds (`None`: the default).
fn crossing_run(penalty: bool, rounds: Option<u32>) -> (usize, Vec<u64>) {
    let mut strategy =
        flatcraft_path::StrategySettings::for_strategy(flatcraft_core::document::CutStrategy::Fast);
    strategy.crossing_penalty = penalty;
    if let Some(rounds) = rounds {
        strategy.rounds = rounds;
    }
    let settings = PipelineSettings {
        overcut_mm: 0.0,
        strategy,
        ..PipelineSettings::with_defaults(Rect::new(0.0, 0.0, 300.0, 300.0), Point::new(49.0, 49.0))
    };
    let out = plan(&crossing_job(), &settings).unwrap();
    let order: Vec<u64> = out.toolpaths.iter().map(|t| t.source.object.0).collect();
    let s = SimSettings::float(0.0);
    (sim(&out.toolpaths, out.home, &s).1.travel_crossings, order)
}

#[test]
fn crossing_penalty_steers_the_construction() {
    // The construction alone (no improvement rounds). With the penalty the
    // order goes left first and crosses the square once, on the way to
    // the right; without it, it crosses on the way right and again on the
    // way back.
    assert_eq!(crossing_run(true, Some(0)), (1, vec![1, 3, 2]));
    assert_eq!(crossing_run(false, Some(0)), (2, vec![1, 2, 3]));
}

#[test]
fn crossing_penalty_lowers_simulated_travel_crossings() {
    // The shipped pipeline, improvement included: the improvement scores
    // crossings too, so it never ends with more than the construction.
    let (built, _) = crossing_run(true, Some(0));
    let (with, order) = crossing_run(true, None);
    assert!(with <= built, "{with} > {built}: {order:?}");
    assert_eq!(with, 1, "{order:?}");
    let (without, order) = crossing_run(false, None);
    assert_eq!(without, 2, "{order:?}");
}

#[test]
fn order_improvement_lowers_simulated_travel_crossings() {
    // 40 squares of 0.5 to 4.5 mm packed in 60 x 60 mm, from a fixed
    // generator. The improvement scores crossings, so here it strictly
    // lowers the simulated crossings of the construction (23 to 11 when
    // measured), with less travel. It does not on every job: on 118 such
    // jobs it lowered them on 87 and raised them on 18, because it counts
    // unfinished cuts too.
    let mut seed = 4u64;
    let mut next = || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let input: Vec<PlacedPath> = (0..40)
        .map(|k| {
            let (x, y) = (10.0 + next() * 60.0, 10.0 + next() * 60.0);
            let (w, h) = (0.5 + next() * 4.0, 0.5 + next() * 4.0);
            PlacedPath {
                path: Rect::new(x, y, x + w, y + h).to_path(0.1),
                source: source(k),
            }
        })
        .collect();
    let run = |rounds: Option<u32>| {
        let mut strategy = flatcraft_path::StrategySettings::default();
        if let Some(rounds) = rounds {
            strategy.rounds = rounds;
        }
        let settings = PipelineSettings {
            strategy,
            ..PipelineSettings::with_defaults(Rect::new(0.0, 0.0, 300.0, 300.0), Point::ZERO)
        };
        let out = plan(&input, &settings).unwrap();
        let (_, m, _) = sim(&out.toolpaths, out.home, &SimSettings::float(0.0));
        (m.travel_crossings, m.travel_mm)
    };
    let (built, built_travel) = run(Some(0));
    let (improved, improved_travel) = run(None);
    assert!(
        improved < built,
        "{built} -> {improved} crossings ({built_travel} -> {improved_travel} mm)"
    );
    assert!(improved_travel < built_travel);
}

#[test]
fn order_improvement_lowers_simulated_travel_and_time() {
    // 100 small squares from a fixed generator. The same job is planned
    // with the nearest-neighbour construction alone (no improvement rounds)
    // and with the default 2-opt, Or-opt and entry DP rounds, then run
    // through the simulator.
    let mut seed = 7u64;
    let mut next = || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let input: Vec<PlacedPath> = (0..100)
        .map(|i| {
            let (x, y) = (5.0 + next() * 280.0, 5.0 + next() * 280.0);
            PlacedPath {
                path: Rect::new(x, y, x + 2.0, y + 2.0).to_path(0.1),
                source: source(i + 1),
            }
        })
        .collect();
    let base =
        PipelineSettings::with_defaults(Rect::new(0.0, 0.0, 300.0, 300.0), Point::new(0.0, 0.0));
    let s = SimSettings::float(0.25);
    let time = TimeModel::PROVISIONAL;
    let measured = |rounds: u32| {
        let settings = PipelineSettings {
            strategy: flatcraft_path::StrategySettings {
                rounds,
                ..base.strategy
            },
            ..base.clone()
        };
        let out = plan(&input, &settings).unwrap();
        let (_, m, _) = sim(&out.toolpaths, out.home, &s);
        let secs = estimate_seconds(&out.toolpaths, out.home, 5, &time);
        (m.travel_mm, secs)
    };
    let (nn_travel, nn_secs) = measured(0);
    let (travel, secs) = measured(base.strategy.rounds);
    eprintln!("travel {nn_travel} -> {travel} mm, time {nn_secs} -> {secs} s");
    assert!(travel < 0.95 * nn_travel, "{nn_travel} -> {travel}");
    assert!(secs < nn_secs, "{nn_secs} -> {secs}");
}

/// A closed 20000-gon of 2000 mm.
fn long_ring() -> Vec<Point> {
    let n = 20_000;
    let r = 2000.0 / (2.0 * std::f64::consts::PI);
    let c = Point::new(400.0, 400.0);
    (0..=n)
        .map(|k| {
            let a = std::f64::consts::TAU * (k % n) as f64 / n as f64;
            c + r * kurbo::Vec2::new(a.cos(), a.sin())
        })
        .collect()
}

/// Timed (Ruling F30): the algorithm bound is the counter test
/// `metrics::tests::long_path_cost_is_bounded`. This one measured about
/// 0.31 s locally (2026-11-25), and its bound is 3x that. Run it with
/// `cargo test -p flatcraft-sim --test metrics -- --ignored`.
#[test]
#[ignore = "wall-clock perf check, run by hand or by the quality bench"]
fn metrics_of_long_path_are_fast() {
    let ring = long_ring();
    let home = ring[0];
    let s = SimSettings::float(0.25);
    let t = [closed(ring, 1)];
    let trace = simulate(&t, home, &s).unwrap();
    let start = std::time::Instant::now();
    let (m, _) = measure(&t, &trace, &s);
    let took = start.elapsed();
    assert!((m.cut_mm - 2000.0).abs() < 1.0, "{}", m.cut_mm);
    assert!(took.as_secs_f64() < 1.0, "{took:?}");
}

/// A 20 mm square piece whose knife runs `offset` inside it (d = 0).
fn offset_square(offset: f64) -> JobMetrics {
    let o = offset;
    let axis = pts(&[
        (o, o),
        (20.0 - o, o),
        (20.0 - o, 20.0 - o),
        (o, 20.0 - o),
        (o, o),
    ]);
    let t = with_runs(
        closed(square(0.0, 0.0, 20.0), 1),
        vec![(axis, vec![W, 0, 1, 2, 3])],
    );
    sim(&[t], Point::new(o, o), &SimSettings::float(0.0)).1
}

#[test]
fn uncut_limit_is_twice_the_end_to_end_limit() {
    // structural: Ruling M2-P14 pins the constant
    assert_eq!(UNCUT_LIMIT_MM, 2.0 * flatcraft_sim::END_TO_END_LIMIT_MM);
}

#[test]
fn a_small_inward_offset_is_good() {
    // M2-P14: 0.03 mm off is within the end-to-end limit: no stray, no
    // uncut, and not into kept material.
    let m = offset_square(0.03);
    assert_eq!((m.uncut_mm, m.stray_mm), (0.0, 0.0));
    assert_eq!(m.max_into_kept_mm, 0.0);
    assert!(m.score >= 98.0, "{}", m.score);
    assert_eq!(m.grade, Grade::Good);
}

#[test]
fn an_offset_between_the_limits_is_stray_not_uncut() {
    // M2-P14: 0.07 mm off severs the material, in the wrong place.
    let m = offset_square(0.07);
    assert!(m.stray_mm > 70.0, "{}", m.stray_mm);
    assert_eq!(m.uncut_mm, 0.0);
}

#[test]
fn a_large_offset_is_stray_and_uncut() {
    // M2-P14: 0.5 mm off is a wrong cut and a missing cut.
    let m = offset_square(0.5);
    assert!(m.stray_mm > 70.0, "{}", m.stray_mm);
    assert!(m.uncut_mm > 70.0, "{}", m.uncut_mm);
}

#[test]
fn device_quantization_alone_is_not_uncut() {
    // M2-P14: a perfect knife on the device grid (d = 0) leaves no uncut.
    let grid = DeviceGrid {
        units_per_mm: 20.0,
        origin: Point::new(3.33, 7.12),
    };
    let s = SimSettings::device(0.0, grid);
    let ring: Vec<Point> = (0..=37)
        .map(|k| {
            let a = std::f64::consts::TAU * (k % 37) as f64 / 37.0;
            Point::new(50.0 + 12.3 * a.cos(), 40.0 + 9.7 * a.sin())
        })
        .collect();
    let diagonal = open(pts(&[(10.013, 10.027), (30.0377, 22.4113)]));
    let (_, m, _) = sim(&[closed(ring, 1), diagonal], Point::ZERO, &s);
    assert_eq!(m.uncut_mm, 0.0);
    assert!(m.max_hausdorff_mm > 0.0);
}

#[test]
fn a_shallow_lead_in_into_kept_material_is_reported() {
    // M2-P14: waste keeps the strict tolerance. The lead-in starts 0.03 mm
    // inside the piece.
    let mut axis = pts(&[(5.0, 0.03)]);
    axis.extend(square(0.0, 0.0, 20.0));
    let t = with_runs(
        closed(square(0.0, 0.0, 20.0), 1),
        vec![(axis, vec![W, W, 0, 1, 2, 3])],
    );
    let (_, m, _) = sim(&[t], Point::new(5.0, 0.03), &SimSettings::float(0.0));
    assert!(
        (m.max_into_kept_mm - 0.03).abs() < 1e-6,
        "{}",
        m.max_into_kept_mm
    );
}

/// The uncompensated d = 0.3 square of 50 mm, plus `overcut` along the first edge.
fn uncompensated_square_gap(overcut: f64) -> f64 {
    let mut ring = square(10.0, 10.0, 50.0);
    if overcut > 0.0 {
        ring.push(Point::new(10.0 + overcut, 10.0));
    }
    let (_, m, _) = sim(
        &[closed(ring, 1)],
        Point::new(10.0, 10.0),
        &SimSettings::float(0.3),
    );
    m.per_path[0]
        .closure_gap_mm
        .expect("a closed path has a gap")
}

#[test]
fn closure_gap_without_overcut_is_the_lag() {
    // M2-P14: the last tip stops d short of the start corner, beside the
    // first edge's trace.
    let gap = uncompensated_square_gap(0.0);
    assert!((gap - 0.3).abs() < 0.01, "{gap}");
}

#[test]
fn closure_gap_with_overcut_is_the_tractrix_lag() {
    // After the start corner the uncompensated tip swings back onto the
    // first edge along a tractrix: after an overcut `o` it is still
    // d * sin(2 atan(exp(-o / d))) beside it. With 0.5 mm at d = 0.3 that
    // is 0.1094 mm; with 2 mm the ring closes.
    let d: f64 = 0.3;
    let lag = |o: f64| d * (2.0 * (-o / d).exp().atan()).sin();
    let gap = uncompensated_square_gap(0.5);
    assert!((gap - lag(0.5)).abs() < 1e-6, "{gap} vs {}", lag(0.5));
    let gap = uncompensated_square_gap(2.0);
    assert!(gap < 0.001, "{gap}");
}

#[test]
fn a_pivot_at_a_corner_twists_90_degrees() {
    // Host-style pivot: the axis swings round the tip at the corner (10, 0).
    let d = 0.3;
    let mut axis = pts(&[(d, 0.0), (10.0 + d, 0.0)]);
    let mut seg = vec![W, 0];
    for k in 1..=90 {
        let a = (k as f64).to_radians();
        axis.push(Point::new(10.0 + d * a.cos(), d * a.sin()));
        seg.push(0);
    }
    axis.push(Point::new(10.0, 10.0 + d));
    seg.push(1);
    let t = with_runs(
        open(pts(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)])),
        vec![(axis, seg)],
    );
    let (_, m, _) = sim(&[t], Point::new(d, 0.0), &SimSettings::float(d));
    assert!((m.max_twist_deg - 90.0).abs() < 2.0, "{}", m.max_twist_deg);
    assert!(
        m.max_corner_deviation_mm < 0.01,
        "{}",
        m.max_corner_deviation_mm
    );
}

#[test]
fn a_misaligned_start_deviates_about_d() {
    // The blade trails towards +x and the cut starts along +y.
    let s = SimSettings::float(0.3);
    let t = open(pts(&[(0.0, 0.0), (0.0, 20.0)]));
    let (_, m, _) = sim(&[t], Point::ZERO, &s);
    // The largest path sample is the first, one sample step in: the tip
    // has swung from 90 deg to 2 * atan(exp(-step / d)) off the line.
    let step = 1.0 / flatcraft_sim::SAMPLE_STEP_FACTOR;
    let want = 0.3 * (2.0 * (-step).exp().atan()).sin();
    assert!(
        (m.max_start_deviation_mm - want).abs() < 1e-4,
        "{} vs {want}",
        m.max_start_deviation_mm
    );
    assert_eq!(
        m.max_start_deviation_mm,
        m.per_path[0].max_start_deviation_mm
    );
}

#[test]
fn worst_spots_rank_and_keep_apart() {
    let s = SimSettings::float(0.0);
    // A piece, and a line with spikes of 0.1 to 1.0 mm. Its run starts with
    // a lead-in from inside the piece.
    let piece = closed(square(0.0, 10.0, 10.0), 2);
    let mut axis = pts(&[(5.0, 15.0), (0.0, 0.0)]);
    let mut seg = vec![W, W];
    for (x, h) in [
        (5.0, 0.1),
        (10.0, 0.2),
        (20.0, 0.4),
        (30.0, 0.6),
        (40.0, 0.8),
        (50.0, 1.0),
        (51.0, 0.9),
    ] {
        axis.extend(pts(&[(x - 0.5, 0.0), (x, h), (x + 0.5, 0.0)]));
        seg.extend([0, 0, 0]);
    }
    axis.push(Point::new(60.0, 0.0));
    seg.push(0);
    let line = with_runs(open(pts(&[(0.0, 0.0), (60.0, 0.0)])), vec![(axis, seg)]);
    let (_, m, _) = sim(&[line, piece], Point::new(5.0, 15.0), &s);
    let spots = &m.worst_spots;
    assert_eq!(spots.len(), 5, "{spots:?}");
    assert_eq!(spots[0].kind, SpotKind::IntoKept);
    assert!(
        spots[0].at.distance(Point::new(5.0, 15.0)) < 1e-9,
        "{spots:?}"
    );
    for (i, a) in spots.iter().enumerate() {
        for b in &spots[i + 1..] {
            assert!(a.at.distance(b.at) >= 2.0, "{a:?} {b:?}");
        }
    }
    let rank = |k: SpotKind| matches!(k, SpotKind::IntoKept | SpotKind::Pushed);
    for w in spots.windows(2) {
        assert!(rank(w[0].kind) >= rank(w[1].kind), "{spots:?}");
        if rank(w[0].kind) == rank(w[1].kind) {
            assert!(w[0].deviation_mm >= w[1].deviation_mm, "{spots:?}");
        }
    }
    let deviation: Vec<Point> = spots
        .iter()
        .filter(|x| x.kind == SpotKind::Deviation)
        .map(|x| x.at)
        .collect();
    // The 1.0 spike leads; the 0.9 spike is within 2 mm of it.
    assert!(
        deviation[0].distance(Point::new(50.0, 1.0)) < 1e-9,
        "{spots:?}"
    );
    assert!(
        deviation
            .iter()
            .all(|p| p.distance(Point::new(51.0, 0.9)) > 1e-9)
    );
}

#[test]
fn a_drop_into_a_neighbour_followed_by_a_cut_is_into_kept() {
    // The drop rule: a drop point takes the path tolerance when a path move
    // follows, and a plunge deeper than that is still reported. The blade
    // trails towards -x, so it plunges 0.2 mm inside the neighbour B.
    let a = closed(square(0.0, 0.0, 10.0), 1);
    let b = closed(square(-10.1, -5.0, 10.0), 2);
    let (trace, m, _) = sim(&[a, b], Point::ZERO, &SimSettings::float(0.3));
    let drop = trace.samples.iter().find(|x| x.down).unwrap();
    assert!(drop.tip.distance(Point::new(-0.3, 0.0)) < 1e-12, "{drop:?}");
    // The drop itself is the deepest point: 0.2 mm from B's edge at x = -0.1.
    assert!(
        (m.max_into_kept_mm - 0.2).abs() < 1e-9,
        "{}",
        m.max_into_kept_mm
    );
    assert_eq!(m.worst_spots[0].kind, SpotKind::IntoKept);
}

#[test]
fn a_ring_cut_without_tip_travel_has_no_closure_gap() {
    // Path samples, but the knife never moves: no early trace to measure.
    let t = with_runs(
        closed(square(0.0, 0.0, 10.0), 1),
        vec![(pts(&[(0.0, 0.0), (0.0, 0.0), (0.0, 0.0)]), vec![W, 0, 1])],
    );
    let (trace, m, _) = sim(&[t], Point::ZERO, &SimSettings::float(0.0));
    assert!(trace.samples.iter().any(|x| x.down && x.seg != W));
    assert_eq!(m.per_path[0].closure_gap_mm, None);
    assert_eq!(m.max_closure_gap_mm, 0.0);
}
