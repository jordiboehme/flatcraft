use flatcraft_core::document::ObjectId;
use flatcraft_path::clean::length;
use flatcraft_path::{PathRef, PipelineSettings, PlacedPath, PlanError, plan};
use kurbo::{BezPath, Circle, Point, Rect, Shape};
use proptest::prelude::*;

fn settings() -> PipelineSettings {
    PipelineSettings {
        cutting_area: Rect::new(5.0, 5.0, 295.0, 395.0),
        flatten_tolerance_mm: 0.01,
        join_tolerance_mm: 0.01,
        overcut_mm: 0.0,
        passes: 1,
        reverse_alternate_passes: false,
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

proptest! {
    #[test]
    fn closed_rings_stay_closed(
        radii in proptest::collection::vec(5.0f64..40.0, 3..16),
        overcut in 0.0f64..1.0,
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
        let perimeter = length(&ring);

        let mut s = settings();
        s.overcut_mm = overcut;
        s.flatten_tolerance_mm = 1e-9;
        let out = plan(&[placed(1, path)], &s).unwrap();
        prop_assert_eq!(out.toolpaths.len(), 1);
        let t = &out.toolpaths[0];
        prop_assert!(t.closed);
        prop_assert_eq!(t.points[0], pts[0]);
        prop_assert!(length(&t.points) >= perimeter + overcut - 1e-6);
    }
}
