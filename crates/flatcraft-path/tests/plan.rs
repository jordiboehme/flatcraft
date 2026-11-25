use flatcraft_core::document::ObjectId;
use flatcraft_path::clean::length;
use flatcraft_path::{PathRef, PipelineSettings, PlacedPath, PlanError, SettingsField, plan};
use kurbo::{BezPath, Circle, Point, Rect, Shape};
use proptest::prelude::*;

fn point_at(ring: &[Point], mut d: f64) -> Point {
    for w in ring.windows(2) {
        let seg = w[0].distance(w[1]);
        if d <= seg {
            return w[0].lerp(w[1], d / seg);
        }
        d -= seg;
    }
    *ring.last().unwrap()
}

fn settings() -> PipelineSettings {
    PipelineSettings {
        cutting_area: Rect::new(5.0, 5.0, 295.0, 395.0),
        flatten_tolerance_mm: 0.01,
        join_tolerance_mm: 0.01,
        overcut_mm: 0.0,
        passes: 1,
        reverse_alternate_passes: false,
        home: Point::ZERO,
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

fn placed(id: u64, path: BezPath) -> PlacedPath {
    PlacedPath {
        path,
        source: PathRef {
            object: ObjectId(id),
            path_index: 0,
        },
    }
}

#[test]
fn align_to_margin_is_inside() {
    let out = plan(&[placed(1, rect_path(5.0, 5.0, 10.0, 10.0))], &settings());
    assert!(out.is_ok(), "{out:?}");
}

#[test]
fn just_outside_margin_is_refused() {
    let out = plan(&[placed(1, rect_path(4.99, 5.0, 10.0, 10.0))], &settings());
    assert_eq!(
        out,
        Err(PlanError::OutOfArea {
            objects: vec![ObjectId(1)]
        })
    );
}

#[test]
fn out_of_area_names_objects() {
    let input = vec![
        placed(5, rect_path(-1.0, 10.0, 10.0, 10.0)),
        placed(1, rect_path(10.0, 10.0, 10.0, 10.0)),
        placed(2, rect_path(10.0, 390.0, 10.0, 10.0)),
        placed(5, rect_path(300.0, 10.0, 10.0, 10.0)),
    ];
    let err = plan(&input, &settings()).unwrap_err();
    assert_eq!(
        err,
        PlanError::OutOfArea {
            objects: vec![ObjectId(2), ObjectId(5)]
        }
    );
    assert!(!err.to_string().contains('\u{2014}'));
}

/// A MoveTo with no drawing after it moves no blade: it does not count
/// for the area check, wherever it lies.
#[test]
fn a_stray_move_outside_the_area_is_ignored() {
    for stray in [
        // before the drawing, followed by another move
        {
            let mut p = BezPath::new();
            p.move_to((-50.0, -50.0));
            p.extend(rect_path(10.0, 10.0, 10.0, 10.0));
            p
        },
        // after the drawing, at the end of the path
        {
            let mut p = rect_path(10.0, 10.0, 10.0, 10.0);
            p.move_to((500.0, 500.0));
            p
        },
        // followed by a close only
        {
            let mut p = rect_path(10.0, 10.0, 10.0, 10.0);
            p.move_to((500.0, 500.0));
            p.close_path();
            p
        },
    ] {
        let out = plan(&[placed(1, stray.clone())], &settings());
        let out = out.unwrap_or_else(|e| panic!("{e}: {stray:?}"));
        assert_eq!(out.toolpaths.len(), 1);
    }
}

/// The MoveTo that starts a drawn line is where the blade goes down: it
/// is checked.
#[test]
fn a_move_that_starts_a_line_outside_the_area_is_refused() {
    let mut p = BezPath::new();
    p.move_to((1.0, 10.0));
    p.line_to((20.0, 10.0));
    assert_eq!(
        plan(&[placed(1, p)], &settings()),
        Err(PlanError::OutOfArea {
            objects: vec![ObjectId(1)]
        })
    );
}

#[test]
fn curve_bulging_out_of_area_is_caught() {
    // control points inside, but circle edge pokes out: area check uses flattened points
    let c = Circle::new((8.0, 100.0), 5.0).to_path(0.01);
    let out = plan(&[placed(3, c)], &settings());
    assert_eq!(
        out,
        Err(PlanError::OutOfArea {
            objects: vec![ObjectId(3)]
        })
    );
}

#[test]
fn empty_is_refused() {
    assert_eq!(plan(&[], &settings()), Err(PlanError::Empty));
    assert_eq!(
        plan(&[placed(1, BezPath::new())], &settings()),
        Err(PlanError::Empty)
    );
}

#[test]
fn nearest_neighbour_order() {
    let input = vec![
        placed(200, rect_path(200.0, 10.0, 10.0, 10.0)),
        placed(10, rect_path(10.0, 10.0, 10.0, 10.0)),
        placed(100, rect_path(100.0, 10.0, 10.0, 10.0)),
    ];
    let out = plan(&input, &settings()).unwrap();
    let ids: Vec<u64> = out.toolpaths.iter().map(|t| t.source.object.0).collect();
    assert_eq!(ids, vec![10, 100, 200]);
}

#[test]
fn overcut_extends_closed_square_in_plan() {
    let mut s = settings();
    s.overcut_mm = 0.5;
    let out = plan(&[placed(1, rect_path(10.0, 10.0, 10.0, 10.0))], &s).unwrap();
    let t = &out.toolpaths[0];
    assert!(t.closed);
    assert!((length(&t.points) - 40.5).abs() < 1e-9);
    assert_eq!(*t.points.last().unwrap(), Point::new(10.5, 10.0));
    assert!((out.cut_mm - 40.5).abs() < 1e-9);
}

#[test]
fn open_paths_get_no_overcut() {
    let mut s = settings();
    s.overcut_mm = 0.5;
    let mut p = BezPath::new();
    p.move_to((10.0, 10.0));
    p.line_to((30.0, 10.0));
    let out = plan(&[placed(1, p)], &s).unwrap();
    assert_eq!(out.toolpaths[0].points.len(), 2);
    assert!((out.cut_mm - 20.0).abs() < 1e-9);
}

#[test]
fn travel_and_cut_totals() {
    let mut s = settings();
    s.passes = 2;
    let input = vec![
        placed(1, rect_path(10.0, 10.0, 10.0, 10.0)),
        placed(2, rect_path(40.0, 10.0, 10.0, 10.0)),
    ];
    let out = plan(&input, &s).unwrap();
    assert_eq!(out.toolpaths.len(), 4);
    assert!((out.cut_mm - 160.0).abs() < 1e-9);
    // origin -> (10,10); (10,10) -> (10,10); (10,10) -> (40,10); (40,10) -> (40,10)
    let expected = Point::ZERO.distance(Point::new(10.0, 10.0)) + 30.0;
    assert!((out.travel_mm - expected).abs() < 1e-9, "{}", out.travel_mm);
}

#[test]
fn circle_becomes_closed_polygon() {
    let c = Circle::new((100.0, 100.0), 10.0).to_path(0.001);
    let out = plan(&[placed(1, c)], &settings()).unwrap();
    assert_eq!(out.toolpaths.len(), 1);
    let t = &out.toolpaths[0];
    assert!(t.closed);
    assert_eq!(t.points.first(), t.points.last());
}

#[test]
fn reversed_closed_pass_keeps_overcut_overlap() {
    let mut s = settings();
    s.overcut_mm = 0.5;
    s.passes = 2;
    s.reverse_alternate_passes = true;
    let out = plan(&[placed(1, rect_path(10.0, 10.0, 10.0, 10.0))], &s).unwrap();
    assert_eq!(out.toolpaths.len(), 2);
    let (a, b) = (&out.toolpaths[0], &out.toolpaths[1]);
    assert_eq!((a.pass, b.pass), (1, 2));
    assert_eq!(b.points[0], *a.points.last().unwrap());
    assert_eq!(b.points[1], Point::new(10.0, 10.0));
    assert_eq!(*b.points.last().unwrap(), Point::new(10.0, 10.0));
    assert!((length(&b.points) - 40.5).abs() < 1e-9);
    for corner in [(20.0, 10.0), (20.0, 20.0), (10.0, 20.0)] {
        assert!(b.points.contains(&Point::new(corner.0, corner.1)));
    }
    assert!((out.cut_mm - 81.0).abs() < 1e-9);
    // pass 2 starts where pass 1 ended: no travel after the first approach
    assert!((out.travel_mm - Point::ZERO.distance(Point::new(10.0, 10.0))).abs() < 1e-9);
}

#[test]
fn two_closed_subpaths_stay_two_rings() {
    let mut o = rect_path(10.0, 10.0, 30.0, 30.0);
    o.extend(rect_path(20.0, 20.0, 10.0, 10.0));
    let out = plan(&[placed(7, o)], &settings()).unwrap();
    assert_eq!(out.toolpaths.len(), 2);
    assert!(
        out.toolpaths
            .iter()
            .all(|t| t.closed && t.source.object == ObjectId(7))
    );
}

#[test]
fn nan_input_is_out_of_area() {
    let mut p = BezPath::new();
    p.move_to((10.0, 10.0));
    p.line_to((f64::NAN, 20.0));
    assert_eq!(
        plan(&[placed(4, p)], &settings()),
        Err(PlanError::OutOfArea {
            objects: vec![ObjectId(4)]
        })
    );
}

#[test]
fn close_right_after_move_is_dropped() {
    let mut p = BezPath::new();
    p.move_to((10.0, 10.0));
    p.close_path();
    assert_eq!(
        plan(&[placed(1, p.clone())], &settings()),
        Err(PlanError::Empty)
    );
    p.extend(rect_path(20.0, 20.0, 5.0, 5.0));
    assert_eq!(
        plan(&[placed(1, p)], &settings()).unwrap().toolpaths.len(),
        1
    );
}

#[test]
fn flat_ring_becomes_open_cut() {
    // a "ring" along one line collapses to [a, c, a]: the 20 mm line is still cut, once
    let mut p = BezPath::new();
    p.move_to((10.0, 10.0));
    p.line_to((20.0, 10.0));
    p.line_to((30.0, 10.0));
    p.close_path();
    let mut s = settings();
    s.overcut_mm = 0.5;
    let out = plan(&[placed(1, p)], &s).unwrap();
    assert_eq!(out.toolpaths.len(), 1);
    let t = &out.toolpaths[0];
    assert!(!t.closed);
    assert_eq!(
        t.points,
        vec![Point::new(10.0, 10.0), Point::new(30.0, 10.0)]
    );
    assert!((out.cut_mm - 20.0).abs() < 1e-9);
}

#[test]
fn zero_length_ring_is_dropped() {
    // a ring far below the flatten tolerance collapses to [a, a]
    let mut p = BezPath::new();
    p.move_to((10.0, 10.0));
    p.line_to((10.0005, 10.0));
    p.line_to((10.0, 10.0005));
    p.close_path();
    assert_eq!(plan(&[placed(1, p)], &settings()), Err(PlanError::Empty));
}

#[test]
fn invalid_settings_message_is_readable() {
    let msg = PlanError::InvalidSettings {
        field: SettingsField::FlattenTolerance,
        value: f64::NAN,
    }
    .to_string();
    assert_eq!(
        msg,
        "The flatten tolerance must be a number from 0.001 to 1 mm (got NaN)."
    );
    for (field, label) in [
        (SettingsField::JoinTolerance, "join tolerance"),
        (SettingsField::Overcut, "overcut"),
        (SettingsField::Passes, "number of passes"),
        (SettingsField::CuttingArea, "cutting area"),
    ] {
        let msg = PlanError::InvalidSettings { field, value: -1.0 }.to_string();
        assert!(msg.starts_with(&format!("The {label} must")), "{msg}");
        assert!(!msg.contains('\u{2014}') && !msg.contains('_'), "{msg}");
    }
}

fn invalid(s: PipelineSettings, field: SettingsField) {
    let out = plan(&[placed(1, rect_path(10.0, 10.0, 10.0, 10.0))], &s);
    match out {
        Err(PlanError::InvalidSettings { field: f, .. }) => {
            assert_eq!(f, field);
            assert!(
                !PlanError::InvalidSettings {
                    field: f,
                    value: 0.0
                }
                .to_string()
                .contains('\u{2014}')
            );
        }
        other => panic!("expected InvalidSettings({field:?}), got {other:?}"),
    }
}

#[test]
fn zero_flatten_tolerance_is_refused() {
    let mut s = settings();
    s.flatten_tolerance_mm = 0.0;
    invalid(s, SettingsField::FlattenTolerance);
}

#[test]
fn bad_flatten_tolerances_are_refused() {
    for v in [-0.01, f64::NAN, f64::INFINITY, 0.0005] {
        let mut s = settings();
        s.flatten_tolerance_mm = v;
        invalid(s, SettingsField::FlattenTolerance);
    }
}

#[test]
fn bad_join_tolerances_are_refused() {
    for v in [0.0, -1.0, f64::NAN, f64::INFINITY, 0.0005] {
        let mut s = settings();
        s.join_tolerance_mm = v;
        invalid(s, SettingsField::JoinTolerance);
    }
}

#[test]
fn minimum_tolerances_are_accepted() {
    let mut s = settings();
    s.flatten_tolerance_mm = 0.001;
    s.join_tolerance_mm = 0.001;
    assert!(plan(&[placed(1, rect_path(10.0, 10.0, 10.0, 10.0))], &s).is_ok());
}

#[test]
fn bad_overcut_is_refused() {
    for v in [-0.1, f64::NAN, f64::INFINITY] {
        let mut s = settings();
        s.overcut_mm = v;
        invalid(s, SettingsField::Overcut);
    }
}

#[test]
fn zero_passes_is_refused() {
    let mut s = settings();
    s.passes = 0;
    invalid(s, SettingsField::Passes);
}

#[test]
fn bad_cutting_area_is_refused() {
    for r in [
        Rect::new(5.0, 5.0, 5.0, 100.0),
        Rect::new(5.0, 5.0, 100.0, 4.0),
        Rect::new(f64::NAN, 5.0, 100.0, 100.0),
        Rect::new(5.0, 5.0, f64::INFINITY, 100.0),
    ] {
        let mut s = settings();
        s.cutting_area = r;
        invalid(s, SettingsField::CuttingArea);
    }
}

#[test]
fn overcut_larger_than_the_ring_is_one_lap() {
    // Found by closed_rings_stay_closed: a triangle whose requested overcut
    // (2.49 x its first edge) exceeds its perimeter. The pipeline caps at one lap.
    // Shrunk to 4 % so the overcut stays below the 5 mm limit.
    let k = 0.04;
    let pts = [
        (150.0 + k * 24.63864338450413, 200.0),
        (150.0 - k * 13.237620590007365, 200.0 + k * 22.93),
        (150.0 - k * 2.5, 200.0 - k * 4.330127018922193),
    ];
    let mut path = BezPath::new();
    path.move_to(pts[0]);
    path.line_to(pts[1]);
    path.line_to(pts[2]);
    path.close_path();
    let mut ring: Vec<Point> = pts.iter().map(|&p| p.into()).collect();
    ring.push(ring[0]);
    let perimeter = length(&ring);
    let mut s = settings();
    s.flatten_tolerance_mm = 0.001;
    s.overcut_mm = 2.5 * ring[0].distance(ring[1]);
    assert!(s.overcut_mm > perimeter);
    let out = plan(&[placed(1, path)], &s).unwrap();
    let t = &out.toolpaths[0];
    assert!(t.closed);
    assert!((length(&t.points) - 2.0 * perimeter).abs() < 1e-6);
    assert!(t.points.last().unwrap().distance(ring[0]) < 1e-9);
}

proptest! {
    #[test]
    fn closed_rings_stay_closed(
        radii in proptest::collection::vec(5.0f64..40.0, 3..64),
        overcut_factor in 0.0f64..2.5,
    ) {
        // star-shaped polygon: increasing angles, so it is simple
        let n = radii.len();
        let pts: Vec<Point> = radii
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                Point::new(150.0 + r * a.cos(), 200.0 + r * a.sin())
            })
            .collect();
        let mut path = BezPath::new();
        path.move_to(pts[0]);
        for p in &pts[1..] {
            path.line_to(*p);
        }
        path.close_path();
        let mut ring = pts.clone();
        ring.push(pts[0]);
        let unmerged = ring.clone();
        // overcut up to 2.5x the first edge, so it often crosses a corner,
        // but never above the pipeline's limit (it refuses larger ones)
        let overcut = (overcut_factor * ring[0].distance(ring[1]))
            .min(flatcraft_core::limits::MAX_OVERCUT_MM);
        // the plan merges collinear points with the same tolerance
        let ring = flatcraft_path::flatten::merge_collinear(&ring, 0.001);
        let perimeter = length(&ring);

        let mut s = settings();
        s.overcut_mm = overcut;
        s.flatten_tolerance_mm = 0.001;
        let out = plan(&[placed(1, path)], &s).unwrap();
        prop_assert_eq!(out.toolpaths.len(), 1);
        let t = &out.toolpaths[0];
        prop_assert!(t.closed);
        prop_assert_eq!(t.points[0], ring[0]);
        // the overcut is capped at one lap of the ring, and the generator can
        // ask for more than that on a small polygon
        prop_assert!(length(&t.points) >= perimeter + overcut.min(perimeter) - 1e-6);
        let expect = point_at(&ring, overcut);
        prop_assert!(t.points.last().unwrap().distance(expect) < 1e-9);
        // against the unmerged source ring
        prop_assert_eq!(t.points[0], pts[0]);
        let expect_src = point_at(&unmerged, overcut);
        prop_assert!(t.points.last().unwrap().distance(expect_src) <= 2.0 * 0.001);
    }

    #[test]
    fn rings_stay_closed_with_merge_and_reversed_passes(
        radii in proptest::collection::vec(5.0f64..40.0, 3..16),
        overcut in 0.0f64..1.0,
    ) {
        let n = radii.len();
        let mut path = BezPath::new();
        for (i, r) in radii.iter().enumerate() {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            let p = Point::new(150.0 + r * a.cos(), 200.0 + r * a.sin());
            if i == 0 { path.move_to(p) } else { path.line_to(p) }
        }
        path.close_path();
        let mut s = settings();
        s.overcut_mm = overcut;
        s.passes = 2;
        s.reverse_alternate_passes = true;
        let out = plan(&[placed(1, path)], &s).unwrap();
        prop_assert_eq!(out.toolpaths.len(), 2);
        let (a, b) = (&out.toolpaths[0], &out.toolpaths[1]);
        prop_assert!(a.closed && b.closed);
        prop_assert_eq!(a.points.first(), b.points.last());
        prop_assert_eq!(a.points.last(), b.points.first());
        let mut rev = b.points.clone();
        rev.reverse();
        prop_assert_eq!(&a.points, &rev);
    }
}

#[test]
fn huge_tolerances_and_overcut_are_refused() {
    let mut s = settings();
    s.flatten_tolerance_mm = 50.0;
    invalid(s, SettingsField::FlattenTolerance);
    let mut s = settings();
    s.join_tolerance_mm = 50.0;
    invalid(s, SettingsField::JoinTolerance);
    let mut s = settings();
    s.overcut_mm = 80.0;
    invalid(s, SettingsField::Overcut);
}

#[test]
fn travel_starts_at_the_device_home() {
    // The grid starts 10 mm right and 20 mm down from the media corner, so
    // the device home is at -10,-20 in app coordinates.
    let mut s = settings();
    s.home = Point::new(-10.0, -20.0);
    let out = plan(&[placed(1, rect_path(20.0, 30.0, 10.0, 10.0))], &s).unwrap();
    let expected = Point::new(-10.0, -20.0).distance(Point::new(20.0, 30.0));
    assert!((out.travel_mm - expected).abs() < 1e-9, "{}", out.travel_mm);
}

#[test]
fn the_first_path_is_the_one_nearest_the_home() {
    let mut s = settings();
    s.home = Point::new(300.0, 0.0);
    let out = plan(
        &[
            placed(1, rect_path(10.0, 10.0, 10.0, 10.0)),
            placed(2, rect_path(270.0, 10.0, 10.0, 10.0)),
        ],
        &s,
    )
    .unwrap();
    assert_eq!(out.toolpaths[0].source.object, ObjectId(2));
}

#[test]
fn lap_is_the_first_ring_without_overcut_in_every_pass() {
    let mut s = settings();
    s.overcut_mm = 0.5;
    for (passes, reverse) in [1, 2, 3].into_iter().flat_map(|n| [(n, false), (n, true)]) {
        s.passes = passes;
        s.reverse_alternate_passes = reverse;
        let input = [
            placed(1, rect_path(10.0, 10.0, 10.0, 10.0)),
            placed(2, Circle::new((60.0, 60.0), 5.0).to_path(0.1)),
        ];
        let out = plan(&input, &s).unwrap();
        assert_eq!(out.toolpaths.len(), 2 * passes as usize);
        for t in &out.toolpaths {
            let lap = t.lap();
            assert_eq!(
                lap.first(),
                lap.last(),
                "reverse {reverse}, pass {}",
                t.pass
            );
            // the ring is the toolpath without the 0.5 mm overcut
            assert!(
                (length(lap) - (length(&t.points) - 0.5)).abs() < 1e-9,
                "reverse {reverse}, pass {}",
                t.pass
            );
        }
        let square = &out.toolpaths[0];
        assert_eq!(square.lap().len(), 5);
    }
}

#[test]
fn lap_of_an_open_path_is_the_whole_path() {
    let mut p = BezPath::new();
    p.move_to((10.0, 10.0));
    p.line_to((20.0, 10.0));
    p.line_to((20.0, 30.0));
    let mut s = settings();
    s.passes = 2;
    s.reverse_alternate_passes = true;
    let out = plan(&[placed(1, p)], &s).unwrap();
    for t in &out.toolpaths {
        assert_eq!(t.lap(), &t.points[..]);
    }
}

/// Structural: the default run restates `points` with `seg = [WASTE_SEG, 0, 1, ..]`.
#[test]
fn default_run_is_the_points_with_their_segments() {
    let out = plan(&[placed(1, rect_path(10.0, 10.0, 10.0, 10.0))], &settings()).unwrap();
    let t = &out.toolpaths[0];
    assert_eq!(t.axis, None);
    let runs = t.runs();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].points, t.points);
    assert_eq!(
        runs[0].seg,
        vec![flatcraft_path::WASTE_SEG, 0, 1, 2, 3],
        "moving to points[j] executes segment j - 1"
    );
    assert_eq!(out.compensation, flatcraft_path::AppliedCompensation::Tip);
    assert_eq!(
        (t.laps, t.keep, t.kind),
        (
            1,
            flatcraft_path::KeepSide::Both,
            flatcraft_path::ToolpathKind::Cut
        )
    );
}

/// Structural: `runs()` returns explicit axis runs unchanged.
#[test]
fn explicit_axis_runs_are_returned_as_they_are() {
    let mut out = plan(&[placed(1, rect_path(10.0, 10.0, 10.0, 10.0))], &settings()).unwrap();
    let t = &mut out.toolpaths[0];
    let runs = vec![
        flatcraft_path::AxisRun {
            points: vec![Point::new(10.0, 10.0), Point::new(20.0, 10.0)],
            seg: vec![flatcraft_path::WASTE_SEG, 0],
        },
        flatcraft_path::AxisRun {
            points: vec![Point::new(20.0, 10.0), Point::new(20.0, 20.0)],
            seg: vec![flatcraft_path::WASTE_SEG, 1],
        },
    ];
    t.axis = Some(runs.clone());
    assert_eq!(t.runs(), runs);
}
