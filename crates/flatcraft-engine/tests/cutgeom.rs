//! Cut geometry per cut mode and stroke treatment.

use flatcraft_core::document::{CutMode, CutOptions, Document, JobSettings, Object, Source};
use flatcraft_core::geometry::{Cap, CutPath, FillRule, Join, Paint, StrokeInfo};
use flatcraft_device::ProfileStore;
use flatcraft_engine::cutgeom::cut_paths;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{Origin, Session};
use kurbo::{BezPath, ParamCurveArclen, ParamCurveNearest, PathEl, Point, Rect, Shape};
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

// ---------------------------------------------------------------- booleans (fix round 1)

/// Distance from `q` to the nearest point of `path`.
fn dist(path: &BezPath, q: Point) -> f64 {
    path.segments()
        .map(|s| s.nearest(q, 1e-7).distance_sq.sqrt())
        .fold(f64::MAX, f64::min)
}

/// Number of cut samples that lie inside the stroke paint (closer than
/// w/2 - 0.02 to the centre line). Butt caps cross the ends of an open
/// centre line, so samples near its ends do not count.
fn cuts_inside_stroke(out: &[BezPath], centre: &BezPath, w: f64) -> usize {
    let ends: Vec<Point> = match centre.elements().last() {
        Some(PathEl::ClosePath) => Vec::new(),
        _ => vec![vertices(centre)[0], *vertices(centre).last().unwrap()],
    };
    let mut count = 0;
    for r in out {
        let mut pts = Vec::new();
        kurbo::flatten(r.iter(), 1e-4, |e| {
            if let Some(p) = e.end_point() {
                pts.push(p);
            }
        });
        let n = pts.len();
        for i in 0..n {
            for k in 0..8 {
                let q = pts[i].lerp(pts[(i + 1) % n], f64::from(k) / 8.0);
                let near_end = ends.iter().any(|e| e.distance(q) <= w / 2.0 + 1e-6);
                if dist(centre, q) < w / 2.0 - 0.02 && !near_end {
                    count += 1;
                }
            }
        }
    }
    count
}

fn pentagram(rad: f64) -> BezPath {
    let mut p = BezPath::new();
    for k in 0..5 {
        let a = -std::f64::consts::FRAC_PI_2 + f64::from(k) * 4.0 * std::f64::consts::PI / 5.0;
        let q = (20.0 + rad * a.cos(), 20.0 + rad * a.sin());
        if k == 0 {
            p.move_to(q);
        } else {
            p.line_to(q);
        }
    }
    p.close_path();
    p
}

#[test]
fn stroked_pentagram_keeps_all_counters() {
    let star = pentagram(15.0);
    let o = object(
        star.clone(),
        line_paint(2.0, MITER, Cap::Butt),
        CutOptions::default(),
    );
    let out = cut(&o);
    // The outer edge, the centre pentagon and the five point triangles.
    assert_eq!(subpaths(&out).len(), 7, "{out:?}");
    assert_eq!(cuts_inside_stroke(&out, &star, 2.0), 0);
}

#[test]
fn x_eye_polyline_keeps_eye_and_tip() {
    for short in [10.0, 6.0] {
        let p = polyline(
            &[(0.0, 0.0), (30.0, 20.0), (30.0, 20.0 - short), (0.0, 30.0)],
            false,
        );
        let o = object(
            p.clone(),
            line_paint(1.0, MITER, Cap::Butt),
            CutOptions::default(),
        );
        let out = cut(&o);
        assert_eq!(subpaths(&out).len(), 2, "short edge {short}: {out:?}");
        assert_eq!(cuts_inside_stroke(&out, &p, 1.0), 0, "short edge {short}");
        let b = out
            .iter()
            .map(Shape::bounding_box)
            .reduce(|a, b| a.union(b))
            .unwrap();
        assert!(b.x1 > 30.4, "short edge {short}: tip not cut, {b:?}");
    }
}

#[test]
fn small_dot_is_one_ring() {
    let dot = kurbo::Circle::new((20.0, 20.0), 0.5).to_path(1e-6);
    let o = object(
        dot,
        line_paint(2.0, Join::Round, Cap::Butt),
        CutOptions::default(),
    );
    let out = cut(&o);
    let subs = subpaths(&out);
    assert_eq!(subs.len(), 1, "{subs:?}");
    assert_size(subs[0].0.bounding_box(), 3.0, 3.0, 0.02);
}

fn fill_stroke(rule: FillRule) -> Paint {
    Paint {
        fill: Some(rule),
        stroke: stroke(2.0, MITER, Cap::Butt),
    }
}

#[test]
fn fill_and_stroke_square_without_close_is_one_ring() {
    let square = polyline(
        &[
            (10.0, 10.0),
            (30.0, 10.0),
            (30.0, 30.0),
            (10.0, 30.0),
            (10.0, 10.0),
        ],
        false,
    );
    let o = object(
        square,
        fill_stroke(FillRule::NonZero),
        CutOptions::default(),
    );
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 1, "{subs:?}");
    assert_size(subs[0].0.bounding_box(), 22.0, 22.0, 1e-6);
    // The whole 22 x 22 area, not a 22 x 22 strip around the open path
    // (whose area would be 160).
    let area = subs[0].0.area().abs();
    assert!((area - 484.0).abs() < 1e-3, "{area}");
}

#[test]
fn open_filled_triangle_with_stroke_frees_the_fill() {
    let tri = polyline(&[(10.0, 10.0), (30.0, 10.0), (20.0, 30.0)], false);
    let o = object(tri, fill_stroke(FillRule::NonZero), CutOptions::default());
    let out = cut(&o);
    let subs = subpaths(&out);
    assert_eq!(subs.len(), 1, "{subs:?}");
    assert!(subs[0].1);
    // The ring goes around the fill, not just around the stroke.
    assert_ne!(subs[0].0.winding(Point::new(20.0, 16.0)), 0);
}

fn square(x: f64, s: f64, reverse: bool) -> BezPath {
    let mut pts = vec![(x, x), (x + s, x), (x + s, x + s), (x, x + s)];
    if reverse {
        pts.reverse();
    }
    polyline(&pts, true)
}

#[test]
fn nested_nonzero_same_direction_is_one_ring() {
    let mut p = square(0.0, 30.0, false);
    p.extend(square(10.0, 10.0, false));
    let o = object(p, fill_stroke(FillRule::NonZero), CutOptions::default());
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 1, "{subs:?}");
    assert_size(subs[0].0.bounding_box(), 32.0, 32.0, 1e-6);
}

#[test]
fn nonzero_o_with_opposite_inner_keeps_its_hole() {
    let mut p = square(0.0, 30.0, false);
    p.extend(square(10.0, 10.0, true));
    let o = object(p, fill_stroke(FillRule::NonZero), CutOptions::default());
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 2, "{subs:?}");
    let mut sizes: Vec<f64> = subs.iter().map(|(p, _)| p.bounding_box().width()).collect();
    sizes.sort_by(f64::total_cmp);
    assert!(
        (sizes[0] - 8.0).abs() < 1e-6 && (sizes[1] - 32.0).abs() < 1e-6,
        "{sizes:?}"
    );
}

#[test]
fn square_caps_extend_the_strip() {
    let o = object(
        hline(),
        line_paint(3.0, MITER, Cap::Square),
        CutOptions::default(),
    );
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 1);
    assert_size(subs[0].0.bounding_box(), 53.0, 3.0, 1e-6);
}

#[test]
fn state_bounds_follow_the_cut_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import_line(&mut s);
    let st = run(&mut s, "document.state", json!({}));
    let b = &st["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .unwrap()["bounds_mm"];
    let h = b["y1"].as_f64().unwrap() - b["y0"].as_f64().unwrap();
    let w = b["x1"].as_f64().unwrap() - b["x0"].as_f64().unwrap();
    assert!((w - 50.0).abs() < 1e-3 && (h - 3.0).abs() < 1e-3, "{b}");
}

#[test]
fn set_cut_options_schema_has_threshold_range() {
    let c = flatcraft_engine::registry::get("object.set_cut_options").unwrap();
    let schema = serde_json::to_value(c.params_schema()).unwrap();
    let text = schema.to_string();
    assert!(text.contains("\"minimum\":0.05"), "{text}");
    assert!(text.contains("\"maximum\":10.0"), "{text}");
}

#[test]
fn project_open_repairs_bad_cut_options() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import_line(&mut s);
    let path = dir.path().join("p.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    let text = std::fs::read_to_string(&path).unwrap();
    let mut v: Value = serde_json::from_str(&text).unwrap();
    v["objects"][0]["cut"]["auto_threshold_mm"] = json!(99.0);
    std::fs::write(&path, v.to_string()).unwrap();
    let mut s2 = session(&dir);
    let r = run(&mut s2, "project.open", json!({ "path": path }));
    assert!(
        r["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("cut options")),
        "{r}"
    );
    assert_eq!(
        state_cut(&mut s2, id),
        json!({ "mode": "object_outlines", "strokes": "auto", "auto_threshold_mm": 0.5 })
    );
}

#[test]
fn cheap_bounds_match_the_cut_paths() {
    use flatcraft_engine::cutgeom::cut_bounds;
    let mut o_letter = letter_o();
    o_letter.extend(pentagram(15.0));
    let cases = [
        object(
            hline(),
            line_paint(3.0, MITER, Cap::Round),
            CutOptions::default(),
        ),
        object(
            hline(),
            line_paint(0.2, MITER, Cap::Butt),
            CutOptions::default(),
        ),
        object(
            ell(),
            line_paint(2.0, MITER, Cap::Square),
            CutOptions::default(),
        ),
        object(
            pentagram(15.0),
            line_paint(2.0, MITER, Cap::Butt),
            CutOptions::default(),
        ),
        object(
            kurbo::Circle::new((20.0, 20.0), 0.5).to_path(1e-6),
            line_paint(2.0, Join::Round, Cap::Butt),
            CutOptions::default(),
        ),
        object(
            o_letter,
            fill_stroke(FillRule::EvenOdd),
            CutOptions::default(),
        ),
        object(
            hline(),
            line_paint(3.0, MITER, Cap::Butt),
            CutOptions {
                mode: CutMode::AllLines,
                ..CutOptions::default()
            },
        ),
    ];
    for o in &cases {
        let exact = cut(o)
            .iter()
            .map(Shape::bounding_box)
            .reduce(|a, b| a.union(b))
            .unwrap();
        let cheap = cut_bounds(o).unwrap();
        for (a, b) in [
            (exact.x0, cheap.x0),
            (exact.y0, cheap.y0),
            (exact.x1, cheap.x1),
            (exact.y1, cheap.y1),
        ] {
            assert!((a - b).abs() < 2.0 * TOL, "{exact:?} vs {cheap:?}");
        }
    }
}

// ---------------------------------------------------------------- fill with a thin stroke (fix round 2)

fn fill_thin_stroke() -> Paint {
    Paint {
        fill: Some(FillRule::NonZero),
        stroke: stroke(0.2, MITER, Cap::Butt),
    }
}

#[test]
fn open_filled_triangle_with_thin_stroke_frees_the_fill() {
    let tri = polyline(&[(10.0, 10.0), (30.0, 10.0), (20.0, 30.0)], false);
    let o = object(tri, fill_thin_stroke(), CutOptions::default());
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 1, "{subs:?}");
    assert!(subs[0].1, "the ring must be closed");
    assert!((subs[0].0.area().abs() - 200.0).abs() < 1e-3);
}

#[test]
fn nested_nonzero_same_direction_with_thin_stroke_is_one_ring() {
    let mut p = square(0.0, 30.0, false);
    p.extend(square(10.0, 10.0, false));
    let o = object(p, fill_thin_stroke(), CutOptions::default());
    let subs = subpaths(&cut(&o));
    assert_eq!(subs.len(), 1, "{subs:?}");
    assert_size(subs[0].0.bounding_box(), 30.0, 30.0, 1e-6);
}

#[test]
fn job_counts_fills_without_area() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><path d="M50 50 H100"/><rect x="50" y="60" width="10" height="10"/></svg>"#;
    run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg, "name": "mixed" }),
    );
    let r = run(&mut s, "job.preview", json!({}));
    assert_eq!(r["toolpaths"], 1, "{r}");
    let warnings = r["warnings"].to_string();
    assert!(
        warnings.contains("1 filled paths have no area"),
        "{warnings}"
    );
}

// ---------------------------------------------------------------- thin lines on default fills (fix round 3)

/// Toolpaths of the job for one imported SVG, as (closed, length) pairs.
fn job_toolpaths(svg: &str) -> Vec<(bool, f64)> {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg, "name": "t" }),
    );
    run(&mut s, "job.preview", json!({}));
    let plan = &s.doc().last_job.as_ref().unwrap().plan;
    let mut out: Vec<(bool, f64)> = plan
        .toolpaths
        .iter()
        .map(|t| {
            let len: f64 = t.points.windows(2).map(|w| w[0].distance(w[1])).sum();
            (t.closed, len)
        })
        .collect();
    out.sort_by(|a, b| a.1.total_cmp(&b.1));
    out
}

fn svg200(body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200">{body}</svg>"#
    )
}

#[test]
fn thin_line_with_default_fill_is_cut() {
    let t = job_toolpaths(&svg200(
        r#"<path d="M50 50 H100" stroke="black" stroke-width="0.2"/>"#,
    ));
    assert_eq!(t.len(), 1, "{t:?}");
    assert!(!t[0].0 && (t[0].1 - 50.0).abs() < 1e-3, "{t:?}");
}

#[test]
fn thin_t_with_default_fill_is_two_lines() {
    let t = job_toolpaths(&svg200(
        r#"<path d="M50 50 H100 M75 50 V90" stroke="black" stroke-width="0.2"/>"#,
    ));
    assert_eq!(t.len(), 2, "{t:?}");
    assert!(t.iter().all(|(closed, _)| !closed), "{t:?}");
    assert!(
        (t[0].1 - 40.0).abs() < 1e-3 && (t[1].1 - 50.0).abs() < 1e-3,
        "{t:?}"
    );
}

#[test]
fn square_and_thin_line_subpath_are_both_cut() {
    let t = job_toolpaths(&svg200(
        r#"<path d="M50 50 H70 V70 H50 Z M80 50 H120" stroke="black" stroke-width="0.2"/>"#,
    ));
    assert_eq!(t.len(), 2, "{t:?}");
    assert!(!t[0].0 && (t[0].1 - 40.0).abs() < 1e-3, "{t:?}");
    // The closed square, 80 mm plus the overcut.
    assert!(t[1].0 && (t[1].1 - 80.0).abs() < 1.0, "{t:?}");
}

// ---------------------------------------------------------------- lines hidden under their stroke (fix round 4)

#[test]
fn jittered_thin_line_with_default_fill_is_one_line() {
    // A hand-drawn line: 50 mm long, every 0.5 mm a point 0.03 mm above or
    // below. Its default fill is a row of slivers under the 0.2 mm stroke.
    let mut d = String::from("M50 50");
    for i in 1..=100 {
        let y = if i % 2 == 0 { 50.03 } else { 49.97 };
        d.push_str(&format!(" L{} {y}", 50.0 + f64::from(i) * 0.5));
    }
    let t = job_toolpaths(&svg200(&format!(
        r#"<path d="{d}" stroke="black" stroke-width="0.2"/>"#
    )));
    assert_eq!(t.len(), 1, "{t:?}");
    assert!(!t[0].0, "the line must stay open: {t:?}");
    assert!((t[0].1 - 50.0).abs() < 1.0, "{t:?}");
}

#[test]
fn kinked_thin_line_with_default_fill_is_one_line() {
    // One 0.05 mm kink: the fill is a 50 mm sliver under the stroke. Cut
    // as a ring the knife would run over the same line twice.
    let t = job_toolpaths(&svg200(
        r#"<path d="M50 50 L75 50.05 L100 50" stroke="black" stroke-width="0.2"/>"#,
    ));
    assert_eq!(t.len(), 1, "{t:?}");
    assert!(!t[0].0, "the line must stay open: {t:?}");
    assert!((t[0].1 - 50.0).abs() < 0.01, "no double cut: {t:?}");
}

#[test]
fn fill_only_sliver_below_tolerance_is_dropped() {
    // A 50 x 0.008 mm fill has a mean width below the 0.01 mm tolerance.
    let sliver = polyline(
        &[(10.0, 10.0), (60.0, 10.0), (60.0, 10.008), (10.0, 10.008)],
        true,
    );
    let o = object(sliver, Paint::default(), CutOptions::default());
    assert!(cut(&o).is_empty(), "{:?}", cut(&o));

    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let svg = svg200(
        r#"<path d="M50 50 H100 V50.008 H50 Z"/><rect x="50" y="60" width="10" height="10"/>"#,
    );
    run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg, "name": "sliver" }),
    );
    let r = run(&mut s, "job.preview", json!({}));
    assert_eq!(r["toolpaths"], 1, "{r}");
    let warnings = r["warnings"].to_string();
    assert!(
        warnings.contains(
            "'sliver': 1 filled paths have no area or are thinner than the cut tolerance (0.01 mm) and are not cut."
        ),
        "{warnings}"
    );
}

#[test]
fn closed_hairline_is_cut_once() {
    // The same kinked line, ended with Z: the close runs back along the
    // line, so cut as drawn the knife would pass twice.
    let t = job_toolpaths(&svg200(
        r#"<path d="M50 50 L75 50.05 L100 50 Z" stroke="black" stroke-width="0.2"/>"#,
    ));
    assert_eq!(t.len(), 1, "{t:?}");
    assert!(!t[0].0, "the line must be open: {t:?}");
    assert!((t[0].1 - 50.0).abs() < 0.01, "{t:?}");
}

/// An open arc over a 30 mm chord with the given sagitta, filled, with a
/// 3 mm stroke forced to a centre line.
fn open_arc(sagitta: f64) -> Object {
    let mut arc = BezPath::new();
    arc.move_to((10.0, 50.0));
    arc.quad_to((25.0, 50.0 - 2.0 * sagitta), (40.0, 50.0));
    let cut = CutOptions {
        strokes: flatcraft_core::document::StrokeMode::CentreLine,
        ..CutOptions::default()
    };
    let paint = Paint {
        fill: Some(FillRule::NonZero),
        stroke: stroke(3.0, MITER, Cap::Butt),
    };
    object(arc, paint, cut)
}

#[test]
fn open_filled_arc_with_wide_centre_line_stroke_frees_the_fill() {
    // The implicit closing chord of an open subpath has no stroke, so the
    // fill counts as hidden only below half the stroke width. Mean width
    // here is about 2 mm: between 1.5 and 3.
    let subs = subpaths(&cut(&open_arc(3.0)));
    assert_eq!(subs.len(), 1, "{subs:?}");
    assert!(subs[0].1, "the ring must be closed: {subs:?}");
    assert!((subs[0].0.area().abs() - 60.0).abs() < 0.2, "{subs:?}");
}

#[test]
fn open_filled_arc_thinner_than_half_the_stroke_stays_a_line() {
    // Sagitta 2 mm: mean width about 1.3 mm, below half of the 3 mm
    // stroke, so the fill is hidden and the arc is cut as drawn.
    let subs = subpaths(&cut(&open_arc(2.0)));
    assert_eq!(subs.len(), 1, "{subs:?}");
    assert!(!subs[0].1, "{subs:?}");
}

// ---------------------------------------------------------------- closed hairlines and thin slots (R23)

#[test]
fn thin_closed_slot_stays_a_ring() {
    // 20 x 0.3 mm under a 0.4 mm stroke: hidden under the stroke, but
    // wide enough to be a real slot, so both sides are cut. Started on a
    // short side, so the close is a long side.
    let slot = polyline(
        &[(10.0, 10.3), (10.0, 10.0), (30.0, 10.0), (30.0, 10.3)],
        true,
    );
    let paint = Paint {
        fill: Some(FillRule::NonZero),
        stroke: stroke(0.4, MITER, Cap::Butt),
    };
    let subs = subpaths(&cut(&object(slot, paint, CutOptions::default())));
    assert_eq!(subs.len(), 1, "{subs:?}");
    assert!(subs[0].1, "the slot must stay closed: {subs:?}");
    assert!((length(&subs[0].0) - 40.6).abs() < 1e-6, "{subs:?}");
}

#[test]
fn hairline_spine_does_not_depend_on_start() {
    // A closed hairline, at most 0.07 mm wide.
    let ring = [
        (10.0, 10.0),
        (35.0, 10.05),
        (60.0, 10.0),
        (40.0, 9.98),
        (20.0, 9.98),
    ];
    let spine = |start: usize| {
        let mut pts = ring.to_vec();
        pts.rotate_left(start);
        let subs = subpaths(&cut(&object(
            polyline(&pts, true),
            fill_thin_stroke(),
            CutOptions::default(),
        )));
        assert_eq!(subs.len(), 1, "start {start}: {subs:?}");
        assert!(!subs[0].1, "start {start}: the spine must be open");
        vertices(&subs[0].0)
    };
    let first = spine(0);
    assert_eq!(first.first(), Some(&Point::new(10.0, 10.0)));
    assert_eq!(first.last(), Some(&Point::new(60.0, 10.0)));
    // Between the two sides: below the kink, above the lower side.
    assert!(
        first[1..first.len() - 1]
            .iter()
            .all(|p| p.y > 9.98 && p.y < 10.05),
        "{first:?}"
    );
    for start in 1..ring.len() {
        let other = spine(start);
        assert_eq!(other.len(), first.len(), "start {start}: {other:?}");
        for (a, b) in first.iter().zip(&other) {
            assert!(a.distance(*b) < TOL, "start {start}: {other:?}");
        }
    }
}

/// A closed wavy hairline over 50 mm, `width` wide, with `n` vertices on
/// its upper side and fewer on its lower side, and its centre line.
fn wavy_hairline(n: usize, width: f64) -> (BezPath, BezPath) {
    let centre = |x: f64| Point::new(10.0 + x, 50.0 + 2.0 * (x / 8.0).sin());
    let side = |count: usize, off: f64| -> Vec<Point> {
        (1..count)
            .map(|i| centre(50.0 * i as f64 / count as f64) + kurbo::Vec2::new(0.0, off))
            .collect()
    };
    let mut ring = BezPath::new();
    ring.move_to(centre(0.0));
    for p in side(n, -width / 2.0) {
        ring.line_to(p);
    }
    ring.line_to(centre(50.0));
    for p in side(n * 7 / 10, width / 2.0).into_iter().rev() {
        ring.line_to(p);
    }
    ring.close_path();
    let mut mid = BezPath::new();
    mid.move_to(centre(0.0));
    for i in 1..=4 * n {
        mid.line_to(centre(50.0 * i as f64 / (4 * n) as f64));
    }
    (ring, mid)
}

#[test]
fn hairline_spine_follows_the_middle() {
    let (ring, centre) = wavy_hairline(100, 0.08);
    let subs = subpaths(&cut(&object(
        ring,
        fill_thin_stroke(),
        CutOptions::default(),
    )));
    assert_eq!(subs.len(), 1, "{subs:?}");
    assert!(!subs[0].1, "the spine must be open");
    let spine = &subs[0].0;
    assert!((length(spine) - length(&centre)).abs() < 0.05);
    let worst = vertices(spine)
        .iter()
        .map(|p| dist(&centre, *p))
        .fold(0.0, f64::max);
    assert!(worst < 0.01, "spine is {worst} mm off the centre line");
}

#[test]
fn long_hairline_spine_is_fast() {
    // About 20000 vertices: comparing every pair takes about a second,
    // the hull a few milliseconds.
    let (ring, _) = wavy_hairline(11_765, 0.08);
    assert!(ring.elements().len() > 20_000);
    let o = object(ring, fill_thin_stroke(), CutOptions::default());
    let t0 = std::time::Instant::now();
    let subs = subpaths(&cut(&o));
    let took = t0.elapsed();
    assert_eq!(subs.len(), 1);
    assert!(!subs[0].1);
    assert!(took.as_millis() < 250, "{took:?}");
}
