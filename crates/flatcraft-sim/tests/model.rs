use std::sync::LazyLock;

use flatcraft_core::document::ObjectId;
use flatcraft_device::profile::{Media, PlotterProfile};
use flatcraft_path::{AxisRun, PathRef, PipelineSettings, PlacedPath, Toolpath, WASTE_SEG, plan};
use flatcraft_sim::model::{drag, push_angle_deg};
use flatcraft_sim::{DeviceGrid, Sample, SimError, SimSettings, simulate};
use kurbo::{BezPath, Circle, Point, Rect, Shape, Vec2};
use proptest::prelude::*;

fn source() -> PathRef {
    PathRef {
        object: ObjectId(1),
        path_index: 0,
    }
}

fn open(pts: &[(f64, f64)]) -> Toolpath {
    Toolpath::new(pts.iter().map(|&p| p.into()).collect(), false, source())
}

fn down(samples: &[Sample]) -> impl Iterator<Item = &Sample> {
    samples.iter().filter(|s| s.down)
}

#[test]
fn aligned_line_has_no_deviation() {
    let s = SimSettings::float(0.3);
    let trace = simulate(&[open(&[(0.0, 0.0), (50.0, 0.0)])], Point::ZERO, &s).unwrap();
    assert_eq!(trace.samples[0].tip, Point::new(-0.3, 0.0));
    assert!(down(&trace.samples).count() > 50);
    for x in down(&trace.samples) {
        assert!(x.tip.y.abs() < 1e-12, "{x:?}");
    }
    let last = trace.samples.last().unwrap().tip;
    assert!(last.distance(Point::new(49.7, 0.0)) < 1e-12, "{last:?}");
}

#[test]
fn closed_form_matches_small_step_integration() {
    let d = 0.3;
    let (from, to) = (Point::ZERO, Point::new(2.0, 0.0));
    // misaligned by 90 deg: the tip is beside the axis, not behind it
    let start = Point::new(0.0, d);
    let steps = (2.0 / (d / 1000.0)).round() as usize;
    let mut tip = start;
    for k in 1..=steps {
        let a = from.lerp(to, k as f64 / steps as f64);
        tip = a - d * (a - tip).normalize();
        if k % (steps / 20) == 0 {
            let exact = drag(from, a, start, d);
            assert!(exact.distance(tip) < 1e-4, "at {a:?}: {exact:?} vs {tip:?}");
        }
    }
    assert!(drag(from, to, start, d).distance(tip) < 1e-4);
}

/// Analytic: `min_x sqrt(sech^2 x + (x - tanh x)^2) = 0.662743...` at x = 1.1997.
const CORNER_FACTOR: f64 = 0.662_743_419_349;

#[test]
fn uncompensated_right_angle_corner_deviates_0_66274_d() {
    let d = 0.3;
    let trace = simulate(
        &[open(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)])],
        Point::ZERO,
        &SimSettings::float(d),
    )
    .unwrap();
    let corner = Point::new(10.0, 0.0);
    let min = down(&trace.samples)
        .map(|s| s.tip.distance(corner))
        .fold(f64::INFINITY, f64::min);
    assert!((min - CORNER_FACTOR * d).abs() <= 1e-5, "min {min}");
}

#[test]
fn steady_circle_tip_radius_is_sqrt_r2_minus_d2() {
    let (r, d, n) = (5.0, 0.3, 3600);
    let lap: Vec<Point> = (0..n)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            Point::new(r * a.cos(), r * a.sin())
        })
        .collect();
    let mut points: Vec<Point> = (0..3).flat_map(|_| lap.iter().copied()).collect();
    points.push(lap[0]);
    let mut t = Toolpath::new(points, true, source());
    t.laps = 3;
    t.lap_range = 0..n + 1;
    let trace = simulate(&[t], Point::new(r, 0.0), &SimSettings::float(d)).unwrap();
    let perimeter = std::f64::consts::TAU * r;
    let third: Vec<&Sample> = down(&trace.samples)
        .filter(|s| s.t_mm > 2.0 * perimeter)
        .collect();
    assert!(third.len() > 1000);
    let want = (r * r - d * d).sqrt();
    // the axis runs counter-clockwise (in x-right, y-up terms) and leads the
    // tip by asin(d / R): the tip trails, it does not lead
    let lead = (d / r).asin();
    for s in third {
        let got = s.tip.to_vec2().length();
        assert!((got - want).abs() <= 0.002, "{got} vs {want}");
        let mut ahead = s.axis.to_vec2().atan2() - s.tip.to_vec2().atan2();
        if ahead <= -std::f64::consts::PI {
            ahead += std::f64::consts::TAU;
        } else if ahead > std::f64::consts::PI {
            ahead -= std::f64::consts::TAU;
        }
        assert!(
            (ahead - lead).abs() <= 1e-3,
            "axis ahead by {ahead}, want {lead}"
        );
    }
}

#[test]
fn blade_up_keeps_heading() {
    let d = 0.3;
    let trace = simulate(
        &[
            open(&[(0.0, 0.0), (10.0, 0.0)]),
            open(&[(10.0, 20.0), (10.0, 30.0)]),
        ],
        Point::ZERO,
        &SimSettings::float(d),
    )
    .unwrap();
    let up: Vec<&Sample> = trace.samples.iter().filter(|s| !s.down).collect();
    assert!(up.iter().any(|s| s.axis.y > 5.0 && s.axis.y < 15.0));
    for s in up {
        assert!(
            (s.axis - s.tip - Vec2::new(d, 0.0)).length() < 1e-9,
            "{s:?}"
        );
    }
}

#[test]
fn zero_offset_tip_is_axis() {
    let trace = simulate(
        &[
            open(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (3.0, 1.0)]),
            open(&[(20.0, 20.0), (25.0, 20.0)]),
        ],
        Point::new(-3.0, -2.0),
        &SimSettings::float(0.0),
    )
    .unwrap();
    assert!(trace.samples.len() > 10);
    for s in &trace.samples {
        assert_eq!(s.tip, s.axis);
        assert!(!s.pushed);
    }
}

#[test]
fn reversal_is_marked_pushed() {
    let tip = Point::new(9.7, 0.0);
    let back = Point::new(5.0, 0.0);
    assert!((push_angle_deg(Point::new(10.0, 0.0), back, tip) - 180.0).abs() < 1e-9);
    assert!(push_angle_deg(Point::ZERO, Point::new(10.0, 0.0), Point::new(-0.3, 0.0)).abs() < 1e-9);
    // an exact push carries the tip along with the axis
    assert!(drag(Point::new(10.0, 0.0), back, tip, 0.3).distance(Point::new(4.7, 0.0)) < 1e-12);

    let trace = simulate(
        &[open(&[(0.0, 0.0), (10.0, 0.0), (5.0, 0.0)])],
        Point::ZERO,
        &SimSettings::float(0.3),
    )
    .unwrap();
    let forward: Vec<&Sample> = down(&trace.samples).filter(|s| s.seg == 0).collect();
    let backward: Vec<&Sample> = down(&trace.samples).filter(|s| s.seg == 1).collect();
    assert!(!forward.is_empty() && !backward.is_empty());
    assert!(forward.iter().all(|s| !s.pushed));
    assert!(backward.iter().all(|s| s.pushed));
}

#[test]
fn a_right_angle_turn_is_not_pushed() {
    let trace = simulate(
        &[open(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)])],
        Point::ZERO,
        &SimSettings::float(0.3),
    )
    .unwrap();
    assert!(trace.samples.len() > 100);
    assert!(trace.samples.iter().all(|s| !s.pushed));
}

/// The real emitter profile with the grid origin of the test and its
/// preconditions checked: no mirror and scale 1.
static EMITTER: LazyLock<(PlotterProfile, Media)> = LazyLock::new(|| {
    let dir = tempfile::tempdir().unwrap();
    let store = flatcraft_device::ProfileStore::load(dir.path()).unwrap();
    let p = store.plotter("file-output-gpgl-cameo2").unwrap();
    assert!(!p.axis.mirror_x && !p.axis.mirror_y);
    assert_eq!((p.scale_x, p.scale_y, p.units_per_mm), (1.0, 1.0, 20.0));
    let mut m = p.media[0].clone();
    m.grid.origin_x_mm = 3.3;
    m.grid.origin_y_mm = 7.1;
    (p, m)
});

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn grid_snap_matches_emitter_rounding(x in -3.3f64..300.0, y in -7.1f64..600.0) {
        let (plotter, media) = &*EMITTER;
        let grid = DeviceGrid { units_per_mm: 20.0, origin: Point::new(3.3, 7.1) };
        let snapped = grid.snap(Point::new(x, y));
        let dev = flatcraft_device::gpgl::quantize::to_device(Point::new(x, y), plotter, media);
        prop_assert!(((snapped.x + 3.3) * 20.0 - f64::from(dev.x)).abs() < 1e-9, "x {x}: {snapped:?} vs {dev:?}");
        prop_assert!(((snapped.y + 7.1) * 20.0 - f64::from(dev.y)).abs() < 1e-9, "y {y}: {snapped:?} vs {dev:?}");
    }
}

/// Structural: pins the step formula and the settings constants.
#[test]
fn sample_step_has_a_floor() {
    assert_eq!(SimSettings::float(0.0).sample_step_mm, 0.01);
    assert_eq!(SimSettings::float(0.05).sample_step_mm, 0.005);
    assert_eq!(SimSettings::float(0.3).sample_step_mm, 0.3 / 20.0);
    assert_eq!(SimSettings::float(0.3).uncut_eps_mm, 0.02);
    let grid = DeviceGrid {
        units_per_mm: 20.0,
        origin: Point::ZERO,
    };
    let dev = SimSettings::device(0.3, grid);
    assert_eq!(dev.uncut_eps_mm, flatcraft_sim::DEVICE_LIMIT_MM);
    assert_eq!(dev.grid, Some(grid));
    assert_eq!(dev.start_heading, flatcraft_path::ASSUMED_START_HEADING);
}

#[test]
fn samples_are_at_most_one_step_apart_and_hit_every_vertex() {
    let s = SimSettings::float(0.25);
    let pts = [(1.0, 1.0), (7.3, 1.0), (7.3, 4.4), (2.0, 9.0)];
    let trace = simulate(
        &[open(&pts), open(&[(20.0, 20.0), (21.0, 23.0)])],
        Point::ZERO,
        &s,
    )
    .unwrap();
    for w in trace.samples.windows(2) {
        assert!(w[0].axis.distance(w[1].axis) <= s.sample_step_mm + 1e-12);
        assert!((w[1].t_mm - w[0].t_mm - w[0].axis.distance(w[1].axis)).abs() < 1e-9);
    }
    for p in pts.iter().chain(&[(20.0, 20.0), (21.0, 23.0)]) {
        let p = Point::from(*p);
        assert!(down(&trace.samples).any(|x| x.axis == p), "{p:?}");
    }
}

#[test]
fn runs_lift_between() {
    let d = 0.3;
    let mut t = open(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
    t.axis = Some(vec![
        AxisRun {
            points: vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)],
            seg: vec![WASTE_SEG, 0],
        },
        AxisRun {
            points: vec![Point::new(10.0, -1.0), Point::new(10.0, 10.0)],
            seg: vec![WASTE_SEG, 1],
        },
    ]);
    let trace = simulate(&[t], Point::ZERO, &SimSettings::float(d)).unwrap();
    let end0 = trace
        .samples
        .iter()
        .rposition(|s| s.run == 0 && s.down)
        .unwrap();
    let start1 = trace
        .samples
        .iter()
        .position(|s| s.run == 1 && s.down)
        .unwrap();
    assert_eq!(trace.samples[end0].axis, Point::new(10.0, 0.0));
    assert_eq!(trace.samples[start1].axis, Point::new(10.0, -1.0));
    assert_eq!(trace.samples[start1].seg, WASTE_SEG);
    let lift = &trace.samples[end0 + 1..start1];
    assert!(lift.len() >= 2);
    let offset = trace.samples[end0].axis - trace.samples[end0].tip;
    for s in lift {
        assert!(!s.down);
        assert_eq!(s.run, 1);
        assert!((s.axis - s.tip - offset).length() < 1e-12);
    }
    // the lift arrives at the drop point blade up, then the blade goes down there
    assert_eq!(lift.last().unwrap().axis, trace.samples[start1].axis);
    assert!(
        down(&trace.samples)
            .filter(|s| s.run == 1)
            .all(|s| s.axis.x == 10.0)
    );
}

#[test]
fn device_mode_snaps_every_run_point() {
    let grid = DeviceGrid {
        units_per_mm: 20.0,
        origin: Point::new(3.3, 7.1),
    };
    let trace = simulate(
        &[open(&[(1.01, 1.02), (5.333, 1.02), (5.333, 4.977)])],
        Point::ZERO,
        &SimSettings::device(0.3, grid),
    )
    .unwrap();
    for p in [(1.01, 1.02), (5.333, 1.02), (5.333, 4.977)] {
        let want = grid.snap(p.into());
        assert!(down(&trace.samples).any(|s| s.axis == want), "{want:?}");
        assert!(down(&trace.samples).all(|s| s.axis != Point::from(p)));
    }
}

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((x, y));
    p.line_to((x + w, y));
    p.line_to((x + w, y + h));
    p.line_to((x, y + h));
    p.close_path();
    p
}

#[test]
fn sim_travel_equals_plan_travel() {
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
        rect_path(10.0, 10.0, 30.0, 20.0),
        Circle::new((80.0, 80.0), 12.0).to_path(0.1),
        line,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, path)| PlacedPath {
        path,
        source: PathRef {
            object: ObjectId(i as u64 + 1),
            path_index: 0,
        },
    })
    .collect();
    let out = plan(&input, &settings).unwrap();
    let d = 0.25;
    let trace = simulate(&out.toolpaths, out.home, &SimSettings::float(d)).unwrap();
    let first = trace.samples[0];
    assert_eq!((first.axis, first.down, first.t_mm), (out.home, false, 0.0));
    assert_eq!(
        first.tip,
        out.home - d * flatcraft_path::ASSUMED_START_HEADING
    );
    let (mut travel, mut cut) = (0.0, 0.0);
    for w in trace.samples.windows(2) {
        let step = w[0].axis.distance(w[1].axis);
        if w[1].down {
            cut += step;
        } else {
            travel += step;
        }
    }
    assert!(
        (travel - out.travel_mm).abs() < 1e-9,
        "{travel} vs {}",
        out.travel_mm
    );
    assert!((cut - out.cut_mm).abs() < 1e-9, "{cut} vs {}", out.cut_mm);
}

#[test]
fn a_bad_sample_step_is_floored() {
    for step in [0.0, -1.0, f64::NAN, 1e-300] {
        let mut s = SimSettings::float(0.3);
        s.sample_step_mm = step;
        let trace = simulate(&[open(&[(0.0, 0.0), (1.0, 0.0)])], Point::ZERO, &s).unwrap();
        for w in trace.samples.windows(2) {
            assert!(
                w[0].axis.distance(w[1].axis) <= 0.005 + 1e-12,
                "step {step}"
            );
        }
        assert!(
            trace.samples.len() <= 2 + 1.0f64.div_euclid(0.005) as usize + 2,
            "step {step}"
        );
    }
    let mut s = SimSettings::float(0.3);
    s.sample_step_mm = f64::INFINITY;
    let trace = simulate(&[open(&[(0.0, 0.0), (1.0, 0.0)])], Point::ZERO, &s).unwrap();
    assert_eq!(trace.samples.last().unwrap().axis, Point::new(1.0, 0.0));
}

#[test]
fn non_finite_input_is_refused() {
    let s = SimSettings::float(0.3);
    let bad = open(&[(0.0, 0.0), (f64::NAN, 1.0)]);
    assert_eq!(
        simulate(&[open(&[(0.0, 0.0), (1.0, 0.0)]), bad], Point::ZERO, &s),
        Err(SimError::NonFinite {
            toolpath: Some(1),
            run: 0,
            index: 1
        })
    );
    let inf = open(&[(f64::INFINITY, 0.0), (1.0, 0.0)]);
    assert!(matches!(
        simulate(&[inf], Point::ZERO, &s),
        Err(SimError::NonFinite {
            toolpath: Some(0),
            run: 0,
            index: 0
        })
    ));
    let line = [open(&[(0.0, 0.0), (1.0, 0.0)])];
    assert!(matches!(
        simulate(&line, Point::new(f64::NAN, 0.0), &s),
        Err(SimError::NonFinite { toolpath: None, .. })
    ));
    for d in [f64::NAN, f64::INFINITY, -0.1] {
        assert!(
            matches!(
                simulate(&line, Point::ZERO, &SimSettings::float(d)),
                Err(SimError::BladeOffset { .. })
            ),
            "d {d}"
        );
    }
}

#[test]
fn a_huge_trace_is_refused_before_it_is_built() {
    let s = SimSettings::float(0.3);
    let far = open(&[(0.0, 0.0), (1e9, 0.0)]);
    assert_eq!(
        simulate(&[far], Point::ZERO, &s),
        Err(SimError::TooManySamples {
            limit: flatcraft_sim::MAX_SAMPLES
        })
    );
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "one seg per run point")]
fn a_short_seg_is_a_bug_in_debug() {
    let mut t = open(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)]);
    t.axis = Some(vec![AxisRun {
        points: t.points.clone(),
        seg: vec![WASTE_SEG, 0],
    }]);
    let _ = simulate(&[t], Point::ZERO, &SimSettings::float(0.3));
}
