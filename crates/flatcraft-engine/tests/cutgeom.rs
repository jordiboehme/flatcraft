//! Cut geometry per cut mode and stroke treatment.

use flatcraft_core::document::{CutMode, CutOptions, Document, JobSettings, Object, Source};
use flatcraft_core::geometry::{Cap, CutPath, FillRule, Join, Paint, StrokeInfo};
use flatcraft_device::ProfileStore;
use flatcraft_engine::cutgeom::cut_paths;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{Origin, Session};
use kurbo::{BezPath, ParamCurveArclen, PathEl, Point, Rect, Shape};
use serde_json::{Value, json};

const TOL: f64 = 0.01;

fn job() -> JobSettings {
    JobSettings {
        plotter_id: "cameo2".into(),
        material_id: "cardstock".into(),
        media_id: "a4".into(),
        speed: None,
        force: None,
        passes: None,
    }
}

fn polyline(points: &[(f64, f64)], closed: bool) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(points[0]);
    for q in &points[1..] {
        p.line_to(*q);
    }
    if closed {
        p.close_path();
    }
    p
}

fn stroke(width_mm: f64, join: Join, cap: Cap) -> Option<StrokeInfo> {
    Some(StrokeInfo {
        width_mm,
        join,
        cap,
    })
}

const MITER: Join = Join::Miter { limit: 4.0 };

/// An object with one path whose transform places the path where it is.
fn object(path: BezPath, paint: Paint, cut: CutOptions) -> Object {
    let mut doc = Document::new(job());
    let id = doc.add(
        "t".into(),
        Source::Svg {
            name: "t.svg".into(),
            bytes: vec![],
        },
        vec![CutPath {
            path,
            color: None,
            label: None,
            paint,
        }],
    );
    let o = doc.get_mut(id).unwrap();
    o.transform.center_mm = o.local_bounds.center();
    o.cut = cut;
    o.clone()
}

fn cut(o: &Object) -> Vec<BezPath> {
    cut_paths(o, o.transform.affine(o.local_bounds), TOL)
}

/// Every subpath of `paths` as its own path, with whether it is closed.
fn subpaths(paths: &[BezPath]) -> Vec<(BezPath, bool)> {
    let mut out = Vec::new();
    for p in paths {
        let mut cur = BezPath::new();
        let mut closed = false;
        for el in p.elements() {
            if let PathEl::MoveTo(_) = el
                && !cur.elements().is_empty()
            {
                out.push((std::mem::take(&mut cur), closed));
                closed = false;
            }
            if let PathEl::ClosePath = el {
                closed = true;
            }
            cur.push(*el);
        }
        if !cur.elements().is_empty() {
            out.push((cur, closed));
        }
    }
    out
}

fn length(p: &BezPath) -> f64 {
    p.segments().map(|s| s.arclen(1e-6)).sum()
}

fn vertices(p: &BezPath) -> Vec<Point> {
    p.elements().iter().filter_map(|e| e.end_point()).collect()
}

fn has_vertex(p: &BezPath, at: Point, eps: f64) -> bool {
    vertices(p).iter().any(|v| v.distance(at) < eps)
}

fn assert_size(r: Rect, w: f64, h: f64, eps: f64) {
    assert!(
        (r.width() - w).abs() < eps && (r.height() - h).abs() < eps,
        "{r:?} is not {w} x {h}"
    );
}

/// True if two non-adjacent segments of the closed ring cross.
fn self_intersects(p: &BezPath) -> bool {
    let mut pts = Vec::new();
    kurbo::flatten(p.iter(), 1e-3, |el| {
        if let Some(q) = el.end_point() {
            pts.push(q);
        }
    });
    if pts.first() == pts.last() {
        pts.pop();
    }
    let n = pts.len();
    let seg = |i: usize| kurbo::Line::new(pts[i], pts[(i + 1) % n]);
    for i in 0..n {
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let (a, b) = (seg(i), seg(j));
            let d = |l: kurbo::Line, q: Point| (l.p1 - l.p0).cross(q - l.p0);
            let (d1, d2) = (d(a, b.p0), d(a, b.p1));
            let (d3, d4) = (d(b, a.p0), d(b, a.p1));
            if d1 * d2 < -1e-12 && d3 * d4 < -1e-12 {
                return true;
            }
        }
    }
    false
}

fn line_paint(width: f64, join: Join, cap: Cap) -> Paint {
    Paint {
        fill: None,
        stroke: stroke(width, join, cap),
    }
}

fn hline() -> BezPath {
    polyline(&[(10.0, 20.0), (60.0, 20.0)], false)
}

#[test]
fn thick_line_becomes_a_strip() {
    let o = object(
        hline(),
        line_paint(3.0, MITER, Cap::Butt),
        CutOptions::default(),
    );
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 1, "{subs:?}");
    let (ring, closed) = &subs[0];
    assert!(closed);
    assert_size(ring.bounding_box(), 50.0, 3.0, 1e-6);
    assert!((length(ring) - 106.0).abs() < 0.05, "{}", length(ring));
}

#[test]
fn thin_line_stays_a_centre_line() {
    let o = object(
        hline(),
        line_paint(0.2, MITER, Cap::Butt),
        CutOptions::default(),
    );
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 1);
    assert!(!subs[0].1);
    assert!((length(&subs[0].0) - 50.0).abs() < 1e-9);
}

fn ell() -> BezPath {
    polyline(&[(0.0, 0.0), (20.0, 0.0), (20.0, 20.0)], false)
}

#[test]
fn miter_corner_stays_sharp() {
    let o = object(
        ell(),
        line_paint(2.0, MITER, Cap::Butt),
        CutOptions::default(),
    );
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 1);
    // The outer miter point of the corner at (20, 0): y points down, so
    // the turn is clockwise on screen and the outer side is up-right.
    assert!(
        has_vertex(&subs[0].0, Point::new(21.0, -1.0), 1e-6),
        "{:?}",
        vertices(&subs[0].0)
    );
}

#[test]
fn inner_corner_has_no_loop() {
    let o = object(
        ell(),
        line_paint(2.0, MITER, Cap::Butt),
        CutOptions::default(),
    );
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 1);
    let ring = &subs[0].0;
    assert!(!self_intersects(ring), "{:?}", vertices(ring));
    assert!(
        has_vertex(ring, Point::new(19.0, 1.0), 1e-6),
        "{:?}",
        vertices(ring)
    );
    // No vertex at the centre line's corner.
    assert!(!has_vertex(ring, Point::new(20.0, 0.0), 1e-6));
}

#[test]
fn round_cap_extends_the_strip() {
    let o = object(
        hline(),
        line_paint(3.0, MITER, Cap::Round),
        CutOptions::default(),
    );
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 1);
    assert_size(subs[0].0.bounding_box(), 53.0, 3.0, 2.0 * TOL);
}

#[test]
fn all_lines_ignores_strokes() {
    let o = object(
        hline(),
        line_paint(3.0, MITER, Cap::Butt),
        CutOptions {
            mode: CutMode::AllLines,
            ..CutOptions::default()
        },
    );
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 1);
    assert!(!subs[0].1);
    assert!((length(&subs[0].0) - 50.0).abs() < 1e-9);
}

#[test]
fn fill_and_thick_stroke_cuts_outer_ring_only() {
    let square = polyline(
        &[(10.0, 10.0), (30.0, 10.0), (30.0, 30.0), (10.0, 30.0)],
        true,
    );
    let paint = Paint {
        fill: Some(FillRule::NonZero),
        stroke: stroke(2.0, MITER, Cap::Butt),
    };
    let o = object(square, paint, CutOptions::default());
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 1, "{subs:?}");
    assert!(subs[0].1);
    let b = subs[0].0.bounding_box();
    assert_size(b, 22.0, 22.0, 1e-6);
    assert!(
        (b.x0 - 9.0).abs() < 1e-6 && (b.y0 - 9.0).abs() < 1e-6,
        "{b:?}"
    );
}

#[test]
fn fill_and_thin_stroke_cuts_the_path_once() {
    let square = polyline(
        &[(10.0, 10.0), (30.0, 10.0), (30.0, 30.0), (10.0, 30.0)],
        true,
    );
    let paint = Paint {
        fill: Some(FillRule::NonZero),
        stroke: stroke(0.1, MITER, Cap::Butt),
    };
    let o = object(square, paint, CutOptions::default());
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 1);
    assert_size(subs[0].0.bounding_box(), 20.0, 20.0, 1e-9);
}

fn letter_o() -> BezPath {
    let mut p = polyline(&[(0.0, 0.0), (30.0, 0.0), (30.0, 30.0), (0.0, 30.0)], true);
    p.extend(polyline(
        &[(10.0, 10.0), (20.0, 10.0), (20.0, 20.0), (10.0, 20.0)],
        true,
    ));
    p
}

#[test]
fn evenodd_hole_stays_a_hole() {
    let fill_only = Paint {
        fill: Some(FillRule::EvenOdd),
        stroke: None,
    };
    let o = object(letter_o(), fill_only, CutOptions::default());
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 2);
    assert!(subs.iter().all(|(_, closed)| *closed));

    // With a thick stroke the hole is cut along the stroke's inner edge,
    // the border between paint and the empty hole.
    let stroked = Paint {
        fill: Some(FillRule::EvenOdd),
        stroke: stroke(2.0, MITER, Cap::Butt),
    };
    let o = object(letter_o(), stroked, CutOptions::default());
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 2, "{subs:?}");
    assert!(subs.iter().all(|(_, closed)| *closed));
    let mut sizes: Vec<f64> = subs.iter().map(|(p, _)| p.bounding_box().width()).collect();
    sizes.sort_by(f64::total_cmp);
    assert!((sizes[0] - 8.0).abs() < 1e-6, "{sizes:?}");
    assert!((sizes[1] - 32.0).abs() < 1e-6, "{sizes:?}");
}

#[test]
fn stroke_only_closed_path_cuts_both_rings() {
    let square = polyline(
        &[(10.0, 10.0), (30.0, 10.0), (30.0, 30.0), (10.0, 30.0)],
        true,
    );
    let o = object(
        square,
        line_paint(2.0, MITER, Cap::Butt),
        CutOptions::default(),
    );
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 2, "{subs:?}");
    let mut sizes: Vec<f64> = subs.iter().map(|(p, _)| p.bounding_box().width()).collect();
    sizes.sort_by(f64::total_cmp);
    assert!((sizes[0] - 18.0).abs() < 1e-6, "{sizes:?}");
    assert!((sizes[1] - 22.0).abs() < 1e-6, "{sizes:?}");
    for (p, _) in &subs {
        assert!(!self_intersects(p), "{:?}", vertices(p));
    }
}

#[test]
fn object_scale_scales_the_stroke() {
    let mut o = object(
        hline(),
        line_paint(3.0, MITER, Cap::Butt),
        CutOptions::default(),
    );
    o.transform.scale_x = 2.0;
    o.transform.scale_y = 2.0;
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 1);
    assert_size(subs[0].0.bounding_box(), 100.0, 6.0, 1e-6);

    // A 0.3 mm stroke scaled by 2 is 0.6 mm wide and crosses the 0.5 mm
    // auto threshold.
    let mut thin = object(
        hline(),
        line_paint(0.3, MITER, Cap::Butt),
        CutOptions::default(),
    );
    assert!(!subpaths(&cut(&thin))[0].1);
    thin.transform.scale_x = 2.0;
    thin.transform.scale_y = 2.0;
    assert!(subpaths(&cut(&thin))[0].1);
}

// ---------------------------------------------------------------- session

fn session(dir: &tempfile::TempDir) -> Session {
    let profiles = ProfileStore::load(dir.path()).unwrap();
    let prefs = Preferences {
        output_dir: Some(dir.path().join("output")),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, false, dir.path().to_path_buf())
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

const STROKED_LINE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><path d="M50 50 H100" fill="none" stroke="black" stroke-width="3"/></svg>"#;

fn import_line(s: &mut Session) -> u64 {
    run(
        s,
        "object.import_svg",
        json!({ "svg": STROKED_LINE, "name": "line" }),
    )["id"]
        .as_u64()
        .unwrap()
}

fn state_cut(s: &mut Session, id: u64) -> Value {
    let st = run(s, "document.state", json!({}));
    st["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .unwrap()["cut"]
        .clone()
}

#[test]
fn cut_options_roundtrip_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import_line(&mut s);
    assert_eq!(
        state_cut(&mut s, id),
        json!({ "mode": "object_outlines", "strokes": "auto", "auto_threshold_mm": 0.5 })
    );
    run(
        &mut s,
        "object.set_cut_options",
        json!({ "ids": [id], "mode": "all_lines", "strokes": "centre_line", "auto_threshold_mm": 1.5 }),
    );
    let set = json!({ "mode": "all_lines", "strokes": "centre_line", "auto_threshold_mm": 1.5 });
    assert_eq!(state_cut(&mut s, id), set);

    // Only the given fields change.
    run(
        &mut s,
        "object.set_cut_options",
        json!({ "ids": [id], "strokes": "outline" }),
    );
    let set = json!({ "mode": "all_lines", "strokes": "outline", "auto_threshold_mm": 1.5 });
    assert_eq!(state_cut(&mut s, id), set);

    let path = dir.path().join("p.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    let mut s2 = session(&dir);
    run(&mut s2, "project.open", json!({ "path": path }));
    assert_eq!(state_cut(&mut s2, id), set);

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(
        state_cut(&mut s, id),
        json!({ "mode": "all_lines", "strokes": "centre_line", "auto_threshold_mm": 1.5 })
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(
        state_cut(&mut s, id),
        json!({ "mode": "object_outlines", "strokes": "auto", "auto_threshold_mm": 0.5 })
    );
}

#[test]
fn set_cut_options_refuses_bad_input() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import_line(&mut s);
    for params in [
        json!({ "ids": [id], "auto_threshold_mm": 0.01 }),
        json!({ "ids": [id], "auto_threshold_mm": 11.0 }),
        json!({ "ids": [id], "mode": "merged_outline" }),
        json!({ "ids": [id], "strokes": "hairline" }),
        json!({ "ids": [id], "colour": "red" }),
        json!({ "ids": [id] }),
        json!({ "ids": [999], "mode": "all_lines" }),
    ] {
        assert!(
            s.execute("object.set_cut_options", params.clone(), Origin::Agent)
                .is_err(),
            "{params}"
        );
    }
    assert_eq!(
        state_cut(&mut s, id),
        json!({ "mode": "object_outlines", "strokes": "auto", "auto_threshold_mm": 0.5 })
    );
}

#[test]
fn job_uses_cut_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import_line(&mut s);
    run(&mut s, "job.preview", json!({}));
    let plan = &s.doc().last_job.as_ref().unwrap().plan;
    assert_eq!(plan.toolpaths.len(), 1, "{:?}", plan.toolpaths);
    let t = &plan.toolpaths[0];
    assert!(t.closed);
    assert_eq!(t.source.object.0, id);
    assert_eq!(t.source.path_index, 0);
    let b = t
        .points
        .iter()
        .fold(Rect::from_points(t.points[0], t.points[0]), |r, p| {
            r.union_pt(*p)
        });
    assert_size(b, 50.0, 3.0, 1e-3);

    // All lines cuts the centre line again.
    run(
        &mut s,
        "object.set_cut_options",
        json!({ "ids": [id], "mode": "all_lines" }),
    );
    run(&mut s, "job.preview", json!({}));
    let plan = &s.doc().last_job.as_ref().unwrap().plan;
    assert_eq!(plan.toolpaths.len(), 1);
    assert!(!plan.toolpaths[0].closed);
}

#[test]
fn curves_and_zigzags_give_clean_rings() {
    let circle = kurbo::Circle::new((50.0, 50.0), 5.0).to_path(1e-6);
    let o = object(
        circle,
        line_paint(2.0, Join::Round, Cap::Butt),
        CutOptions::default(),
    );
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 2, "{subs:?}");
    let mut sizes: Vec<f64> = subs.iter().map(|(p, _)| p.bounding_box().width()).collect();
    sizes.sort_by(f64::total_cmp);
    assert!(
        (sizes[0] - 8.0).abs() < 0.02 && (sizes[1] - 12.0).abs() < 0.02,
        "{sizes:?}"
    );

    let zigzag = polyline(
        &[
            (0.0, 0.0),
            (5.0, 10.0),
            (10.0, 0.0),
            (15.0, 10.0),
            (20.0, 0.0),
        ],
        false,
    );
    for join in [MITER, Join::Round, Join::Bevel] {
        let o = object(
            zigzag.clone(),
            line_paint(1.5, join, Cap::Round),
            CutOptions::default(),
        );
        let subs = subpaths(&cut(&o));
        assert_eq!(subs.len(), 1, "{join:?}");
        assert!(subs[0].1);
        assert!(
            !self_intersects(&subs[0].0),
            "{join:?}: {:?}",
            vertices(&subs[0].0)
        );
    }
}
