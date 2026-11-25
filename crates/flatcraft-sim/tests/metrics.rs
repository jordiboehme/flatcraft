use flatcraft_core::document::ObjectId;
use flatcraft_path::{
    AxisRun, PathRef, PipelineSettings, PlacedPath, Toolpath, ToolpathKind, WASTE_SEG, plan,
};
use flatcraft_sim::{
    Grade, JobMetrics, SimSettings, SimTrace, SpotKind, TimeModel, estimate_seconds, measure, run,
    simulate,
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
    assert_ne!(m.grade, Grade::Good, "score {}", m.score);
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
    assert!((m.uncut_mm - 5.0).abs() <= 0.05, "{}", m.uncut_mm);
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
        cutting_area: Rect::new(0.0, 0.0, 300.0, 300.0),
        flatten_tolerance_mm: 0.01,
        join_tolerance_mm: 0.01,
        overcut_mm: 0.5,
        passes: 2,
        reverse_alternate_passes: true,
        home: Point::new(-5.0, -3.0),
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
