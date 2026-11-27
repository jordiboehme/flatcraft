//! The cross-object cut modes (Task 17): merged outline welds overlapping
//! objects into one shape, visible outline leaves out what objects in
//! front cover.

use flatcraft_core::document::{
    CutMode, CutOptions, Document, JobSettings, KeepMode, ObjectId, Source,
};
use flatcraft_core::geometry::{Cap, CutPath, Join, Paint, StrokeInfo};
use flatcraft_device::ProfileStore;
use flatcraft_device::gpgl::check_safe;
use flatcraft_engine::cutgeom::{CutGeometry, document_cut_geometry};
use flatcraft_engine::job::{self, JobInput};
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{Origin, Session};
use kurbo::{BezPath, ParamCurveArclen, PathEl, Point, Rect, Shape};
use proptest::prelude::*;
use serde_json::{Value, json};

const TOL: f64 = 0.01;

fn min_width() -> f64 {
    flatcraft_engine::report::min_width_mm(0.25)
}

fn job_settings() -> JobSettings {
    JobSettings {
        plotter_id: "cameo2".into(),
        material_id: "cardstock".into(),
        media_id: "a4".into(),
        speed: None,
        force: None,
        passes: None,
        strategy: Default::default(),
        weed: Default::default(),
    }
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((x0, y0));
    p.line_to((x1, y0));
    p.line_to((x1, y1));
    p.line_to((x0, y1));
    p.close_path();
    p
}

fn filled(path: BezPath) -> CutPath {
    CutPath {
        path,
        color: None,
        label: None,
        paint: Paint::default(),
    }
}

/// A thin stroke without fill: cut along its centre line.
fn line(points: &[(f64, f64)]) -> CutPath {
    let mut p = BezPath::new();
    p.move_to(points[0]);
    for q in &points[1..] {
        p.line_to(*q);
    }
    CutPath {
        path: p,
        color: None,
        label: None,
        paint: Paint {
            fill: None,
            stroke: Some(StrokeInfo {
                width_mm: 0.3,
                join: Join::Round,
                cap: Cap::Round,
            }),
        },
    }
}

/// Adds an object whose transform places its paths where they are.
fn add(doc: &mut Document, name: &str, paths: Vec<CutPath>, mode: CutMode) -> ObjectId {
    let id = doc.add(
        name.into(),
        Source::Svg {
            name: format!("{name}.svg"),
            bytes: Vec::new().into(),
        },
        paths,
    );
    let o = doc.get_mut(id).unwrap();
    o.transform.center_mm = o.local_bounds.center();
    o.cut = CutOptions {
        mode,
        ..CutOptions::default()
    };
    id
}

fn geometry(doc: &Document) -> CutGeometry {
    document_cut_geometry(doc, TOL, min_width())
}

fn length(p: &BezPath) -> f64 {
    p.segments().map(|s| s.arclen(1e-9)).sum()
}

fn closed(p: &BezPath) -> bool {
    p.elements().last() == Some(&PathEl::ClosePath)
}

fn vertices(p: &BezPath) -> Vec<Point> {
    p.elements().iter().filter_map(|e| e.end_point()).collect()
}

/// The signed area of every closed subpath of `p`, summed.
fn signed_area(p: &BezPath) -> f64 {
    p.area()
}

fn total_length(g: &CutGeometry, ids: &[ObjectId]) -> f64 {
    ids.iter()
        .flat_map(|id| g.pieces(*id))
        .map(|p| length(&p.path))
        .sum()
}

// ---------------------------------------------------------------- merged

#[test]
fn merged_overlapping_objects_cut_one_ring() {
    let mut doc = Document::new(job_settings());
    let back = add(
        &mut doc,
        "back",
        vec![filled(rect(0.0, 0.0, 20.0, 20.0))],
        CutMode::MergedOutline,
    );
    let front = add(
        &mut doc,
        "front",
        vec![filled(rect(10.0, 10.0, 30.0, 30.0))],
        CutMode::MergedOutline,
    );
    let g = geometry(&doc);
    let pieces = g.pieces(back);
    assert_eq!(pieces.len(), 1, "{pieces:?}");
    assert!(closed(&pieces[0].path));
    assert!(
        (signed_area(&pieces[0].path).abs() - 700.0).abs() < 1e-6,
        "{}",
        signed_area(&pieces[0].path)
    );
    // A weld of several objects names no source path.
    assert_eq!((pieces[0].source, &pieces[0].label), (None, &None));
    assert!(g.pieces(front).is_empty());
    assert_eq!(g.merged_into.get(&front), Some(&back));
    // The backmost object's box holds the whole weld.
    let b = g.bounds(back).unwrap();
    assert!(b.contains(Point::new(29.9, 29.9)), "{b:?}");
}

#[test]
fn merged_keeps_letter_holes() {
    let mut doc = Document::new(job_settings());
    // An "O": a 30 mm square with a 10 mm counter (even-odd).
    let mut o = rect(0.0, 0.0, 30.0, 30.0);
    o.extend(rect(10.0, 10.0, 20.0, 20.0));
    let letter = CutPath {
        paint: Paint {
            fill: Some(flatcraft_core::geometry::FillRule::EvenOdd),
            stroke: None,
        },
        ..filled(o)
    };
    let back = add(&mut doc, "O", vec![letter], CutMode::MergedOutline);
    // A tab over the right edge that does not reach the counter.
    add(
        &mut doc,
        "tab",
        vec![filled(rect(25.0, 5.0, 40.0, 25.0))],
        CutMode::MergedOutline,
    );
    let g = geometry(&doc);
    let pieces = g.pieces(back);
    assert_eq!(pieces.len(), 2, "{pieces:?}");
    let mut areas: Vec<f64> = pieces.iter().map(|p| signed_area(&p.path).abs()).collect();
    areas.sort_by(f64::total_cmp);
    assert!((areas[0] - 100.0).abs() < 1e-6, "{areas:?}");
    // 900 + 15 x 20 - 5 x 20 overlap.
    assert!((areas[1] - 1100.0).abs() < 1e-6, "{areas:?}");
}

#[test]
fn merged_drops_inner_centre_lines() {
    let mut doc = Document::new(job_settings());
    let back = add(
        &mut doc,
        "back",
        vec![filled(rect(0.0, 0.0, 20.0, 20.0))],
        CutMode::MergedOutline,
    );
    // The front object has a stroked line from inside the weld to 20 mm
    // past its edge.
    add(
        &mut doc,
        "front",
        vec![
            filled(rect(10.0, 10.0, 30.0, 30.0)),
            line(&[(5.0, 5.0), (50.0, 5.0)]),
        ],
        CutMode::MergedOutline,
    );
    let g = geometry(&doc);
    let open: Vec<_> = g.pieces(back).iter().filter(|p| !closed(&p.path)).collect();
    assert_eq!(open.len(), 1, "{:?}", g.pieces(back));
    assert!((length(&open[0].path) - 30.0).abs() < 1e-6);
    let xs: Vec<f64> = vertices(&open[0].path).iter().map(|p| p.x).collect();
    assert!(xs.iter().all(|x| *x >= 20.0 - 1e-9), "{xs:?}");
    // The line came from the front object: it has no source in the back.
    assert_eq!(open[0].source, None);
}

#[test]
fn merged_does_not_weld_with_other_modes() {
    let mut doc = Document::new(job_settings());
    let back = add(
        &mut doc,
        "back",
        vec![filled(rect(0.0, 0.0, 20.0, 20.0))],
        CutMode::MergedOutline,
    );
    let front = add(
        &mut doc,
        "front",
        vec![filled(rect(10.0, 10.0, 30.0, 30.0))],
        CutMode::ObjectOutlines,
    );
    let g = geometry(&doc);
    for id in [back, front] {
        let p = g.pieces(id);
        assert_eq!(p.len(), 1);
        assert!((signed_area(&p[0].path).abs() - 400.0).abs() < 1e-6);
    }
    assert!(g.merged_into.is_empty());
}

#[test]
fn merged_only_welds_the_same_keep_mode() {
    let mut doc = Document::new(job_settings());
    let back = add(
        &mut doc,
        "back",
        vec![filled(rect(0.0, 0.0, 20.0, 20.0))],
        CutMode::MergedOutline,
    );
    let front = add(
        &mut doc,
        "front",
        vec![filled(rect(10.0, 10.0, 30.0, 30.0))],
        CutMode::MergedOutline,
    );
    doc.get_mut(front).unwrap().keep = KeepMode::Stencil;
    let g = geometry(&doc);
    assert_eq!(g.pieces(back).len(), 1);
    assert_eq!(g.pieces(front).len(), 1);
    assert!(g.merged_into.is_empty());
}

#[test]
fn merged_welds_transitively_and_skips_hidden_and_skipped_objects() {
    let mut doc = Document::new(job_settings());
    let a = add(
        &mut doc,
        "a",
        vec![filled(rect(0.0, 0.0, 20.0, 20.0))],
        CutMode::MergedOutline,
    );
    // c overlaps b only, b overlaps a: one group of three.
    let c = add(
        &mut doc,
        "c",
        vec![filled(rect(30.0, 0.0, 50.0, 20.0))],
        CutMode::MergedOutline,
    );
    let b = add(
        &mut doc,
        "b",
        vec![filled(rect(15.0, 5.0, 35.0, 15.0))],
        CutMode::MergedOutline,
    );
    let g = geometry(&doc);
    assert_eq!(g.pieces(a).len(), 1);
    assert_eq!(g.merged_into.get(&b), Some(&a));
    assert_eq!(g.merged_into.get(&c), Some(&a));
    // A hidden or skipped bridge splits the group again.
    for hide in [true, false] {
        let mut d = doc.clone();
        if hide {
            d.get_mut(b).unwrap().visible = false;
        } else {
            d.get_mut(b).unwrap().cut_enabled = false;
        }
        let g = geometry(&d);
        assert!(g.merged_into.is_empty(), "hide {hide}");
        for id in [a, b, c] {
            assert_eq!(g.pieces(id).len(), 1, "hide {hide}");
        }
    }
}

// ---------------------------------------------------------------- visible

/// H4: a back square (0..20) with a front square (10..30) over its corner.
fn h4(front_mode: CutMode) -> (Document, ObjectId, ObjectId) {
    let mut doc = Document::new(job_settings());
    let back = add(
        &mut doc,
        "back",
        vec![filled(rect(0.0, 0.0, 20.0, 20.0))],
        CutMode::VisibleOutline,
    );
    let front = add(
        &mut doc,
        "front",
        vec![filled(rect(10.0, 10.0, 30.0, 30.0))],
        front_mode,
    );
    (doc, back, front)
}

fn assert_h4_back(g: &CutGeometry, back: ObjectId) {
    let pieces = g.pieces(back);
    assert_eq!(pieces.len(), 1, "{pieces:?}");
    let p = &pieces[0].path;
    assert!(!closed(p));
    assert!((length(p) - 60.0).abs() < 1e-6, "{}", length(p));
    let mut v = vertices(p);
    if v[0].x > v[0].y {
        v.reverse();
    }
    let want = [
        (10.0, 20.0),
        (0.0, 20.0),
        (0.0, 0.0),
        (20.0, 0.0),
        (20.0, 10.0),
    ];
    assert_eq!(v.len(), want.len(), "{v:?}");
    for (a, b) in v.iter().zip(want) {
        assert!(a.distance(Point::new(b.0, b.1)) < 1e-6, "{v:?}");
    }
}

#[test]
fn visible_front_object_hides_back_corner() {
    let (doc, back, front) = h4(CutMode::VisibleOutline);
    let g = geometry(&doc);
    let f = g.pieces(front);
    assert_eq!(f.len(), 1);
    assert!(closed(&f[0].path), "nothing covers the front ring");
    assert!((length(&f[0].path) - 80.0).abs() < 1e-6);
    assert_h4_back(&g, back);
    assert_eq!(g.pieces(back)[0].source, Some(0));
}

#[test]
fn object_outlines_front_object_still_covers() {
    for mode in [CutMode::ObjectOutlines, CutMode::AllLines] {
        let (doc, back, front) = h4(mode);
        let g = geometry(&doc);
        assert_h4_back(&g, back);
        assert_eq!(g.pieces(front).len(), 1);
    }
}

#[test]
fn coincident_edge_is_cut_once() {
    // Side by side: the shared edge x = 20 is cut by the front square only.
    let mut doc = Document::new(job_settings());
    let back = add(
        &mut doc,
        "back",
        vec![filled(rect(0.0, 0.0, 20.0, 20.0))],
        CutMode::VisibleOutline,
    );
    let front = add(
        &mut doc,
        "front",
        vec![filled(rect(20.0, 0.0, 40.0, 20.0))],
        CutMode::VisibleOutline,
    );
    let g = geometry(&doc);
    assert!((total_length(&g, &[back, front]) - 140.0).abs() < 1e-6);
    for p in g.pieces(back) {
        let v = vertices(&p.path);
        for w in v.windows(2) {
            let on_edge = (w[0].x - 20.0).abs() < 1e-9 && (w[1].x - 20.0).abs() < 1e-9;
            assert!(!on_edge, "the back cuts the shared edge: {v:?}");
        }
    }
    // Overlapping with edges on the same lines: the parts of the back's
    // top and bottom edges along the front are left to the front.
    let mut doc = Document::new(job_settings());
    let back = add(
        &mut doc,
        "back",
        vec![filled(rect(0.0, 0.0, 20.0, 20.0))],
        CutMode::VisibleOutline,
    );
    let front = add(
        &mut doc,
        "front",
        vec![filled(rect(10.0, 0.0, 30.0, 20.0))],
        CutMode::VisibleOutline,
    );
    let g = geometry(&doc);
    assert!((total_length(&g, &[back]) - 40.0).abs() < 1e-6);
    assert!((total_length(&g, &[front]) - 80.0).abs() < 1e-6);
}

#[test]
fn hidden_objects_do_not_cover_and_skipped_ones_do() {
    let (mut doc, back, front) = h4(CutMode::ObjectOutlines);
    doc.get_mut(front).unwrap().visible = false;
    let g = geometry(&doc);
    assert_eq!(g.pieces(back).len(), 1);
    assert!(closed(&g.pieces(back)[0].path));
    let (mut doc, back, front) = h4(CutMode::ObjectOutlines);
    doc.get_mut(front).unwrap().cut_enabled = false;
    assert_h4_back(&geometry(&doc), back);
}

#[test]
fn later_paths_of_one_object_cover_its_earlier_ones() {
    let mut doc = Document::new(job_settings());
    let id = add(
        &mut doc,
        "two",
        vec![
            filled(rect(0.0, 0.0, 20.0, 20.0)),
            filled(rect(10.0, 10.0, 30.0, 30.0)),
        ],
        CutMode::VisibleOutline,
    );
    let g = geometry(&doc);
    let pieces = g.pieces(id);
    assert_eq!(pieces.len(), 2, "{pieces:?}");
    let back = pieces.iter().find(|p| p.source == Some(0)).unwrap();
    assert!((length(&back.path) - 60.0).abs() < 1e-6);
    let front = pieces.iter().find(|p| p.source == Some(1)).unwrap();
    assert!(closed(&front.path));
}

#[test]
fn a_fully_covered_object_is_listed_as_covered() {
    let mut doc = Document::new(job_settings());
    let back = add(
        &mut doc,
        "back",
        vec![filled(rect(10.0, 10.0, 20.0, 20.0))],
        CutMode::VisibleOutline,
    );
    add(
        &mut doc,
        "front",
        vec![filled(rect(0.0, 0.0, 30.0, 30.0))],
        CutMode::ObjectOutlines,
    );
    let g = geometry(&doc);
    assert!(g.pieces(back).is_empty());
    assert!(g.covered.contains(&back));
}

// ---------------------------------------------------------------- cache

fn objects(doc: &Document) -> Vec<job::JobObject> {
    let tops = doc.tree().top_of_each();
    doc.objects
        .iter()
        .map(|o| job::JobObject {
            id: o.id,
            name: o.name.clone(),
            visible: o.visible,
            cut_enabled: o.cut_enabled,
            transform: o.transform,
            local_bounds: o.local_bounds,
            cut: o.cut,
            keep: o.keep,
            geometry: o.geometry.clone(),
            link_group: o.link_group,
            unit: tops.get(&o.id).copied().unwrap_or(o.id),
        })
        .collect()
}

fn build(
    doc: &Document,
    previous: Option<&std::sync::Arc<CutGeometry>>,
) -> std::sync::Arc<CutGeometry> {
    flatcraft_engine::cutgeom::objects_cut_geometry(&objects(doc), TOL, min_width(), previous)
}

#[test]
fn an_edit_to_a_front_object_rebuilds_the_outline_behind_it() {
    let (mut doc, back, front) = h4(CutMode::ObjectOutlines);
    let first = build(&doc, None);
    assert_h4_back(&first, back);
    // Unchanged: the same build.
    let again = build(&doc, Some(&first));
    assert!(std::sync::Arc::ptr_eq(&first, &again));
    // Moved away: the back ring is whole again.
    doc.get_mut(front).unwrap().transform.center_mm.x += 50.0;
    let moved = build(&doc, Some(&first));
    assert_eq!(moved.pieces(back).len(), 1);
    assert!(closed(&moved.pieces(back)[0].path));
    // Hidden: the same, with no change to any shape or cut option.
    let (mut doc, back, front) = h4(CutMode::ObjectOutlines);
    let first = build(&doc, None);
    doc.get_mut(front).unwrap().visible = false;
    let hidden = build(&doc, Some(&first));
    assert!(closed(&hidden.pieces(back)[0].path));
    // Turned around: the old front is behind now and the old back in front.
    let (mut doc, back, front) = h4(CutMode::VisibleOutline);
    let first = build(&doc, None);
    doc.objects.reverse();
    let swapped = build(&doc, Some(&first));
    assert!(closed(&swapped.pieces(back)[0].path));
    assert!(!closed(&swapped.pieces(front)[0].path));
}

#[test]
fn a_keep_change_splits_a_weld_on_the_next_build() {
    let mut doc = Document::new(job_settings());
    let back = add(
        &mut doc,
        "back",
        vec![filled(rect(0.0, 0.0, 20.0, 20.0))],
        CutMode::MergedOutline,
    );
    let front = add(
        &mut doc,
        "front",
        vec![filled(rect(10.0, 10.0, 30.0, 30.0))],
        CutMode::MergedOutline,
    );
    let first = build(&doc, None);
    assert!(first.pieces(front).is_empty());
    doc.get_mut(front).unwrap().keep = KeepMode::Stencil;
    let split = build(&doc, Some(&first));
    assert_eq!(split.pieces(front).len(), 1);
    assert_eq!(split.pieces(back).len(), 1);
}

// ---------------------------------------------------------------- properties

/// The seed of every property run in CI (as `shapes::CI_SEED` in
/// flatcraft-path): CI draws the same cases on every run.
const CI_SEED: u64 = 0x5eed_f1a7_c4af_7016;

fn ci_config(cases: u32) -> ProptestConfig {
    ProptestConfig {
        cases,
        rng_seed: proptest::test_runner::RngSeed::Fixed(CI_SEED),
        ..ProptestConfig::default()
    }
}

/// Rectangles on a half-millimetre grid, so edges often coincide.
fn rects() -> impl Strategy<Value = Vec<(f64, f64, f64, f64)>> {
    proptest::collection::vec((0u32..40, 0u32..40, 2u32..30, 2u32..30), 2..5).prop_map(|v| {
        v.into_iter()
            .map(|(x, y, w, h)| {
                let (x, y, w, h) = (
                    f64::from(x) / 2.0,
                    f64::from(y) / 2.0,
                    f64::from(w) / 2.0,
                    f64::from(h) / 2.0,
                );
                (x, y, x + w, y + h)
            })
            .collect()
    })
}

/// [`rects`], each shifted by 0, 0.004 or 0.008 mm in x and y, so edges
/// also come within the join tolerance of each other without meeting.
fn jittered_rects() -> impl Strategy<Value = Vec<(f64, f64, f64, f64)>> {
    (rects(), proptest::collection::vec((0u8..3, 0u8..3), 4)).prop_map(|(r, j)| {
        r.into_iter()
            .zip(j)
            .map(|((x0, y0, x1, y1), (jx, jy))| {
                let (dx, dy) = (f64::from(jx) * 0.004, f64::from(jy) * 0.004);
                (x0 + dx, y0 + dy, x1 + dx, y1 + dy)
            })
            .collect()
    })
}

fn doc_of(rects: &[(f64, f64, f64, f64)], mode: CutMode) -> (Document, Vec<ObjectId>) {
    let mut doc = Document::new(job_settings());
    let ids = rects
        .iter()
        .enumerate()
        .map(|(k, r)| {
            add(
                &mut doc,
                &format!("r{k}"),
                vec![filled(rect(r.0, r.1, r.2, r.3))],
                mode,
            )
        })
        .collect();
    (doc, ids)
}

/// Every straight segment of every piece.
fn segments(g: &CutGeometry, ids: &[ObjectId]) -> Vec<(Point, Point)> {
    let mut out = Vec::new();
    for p in ids.iter().flat_map(|id| g.pieces(*id)) {
        for s in p.path.segments() {
            if let kurbo::PathSeg::Line(l) = s {
                out.push((l.p0, l.p1));
            }
        }
    }
    out
}

fn check_cut_once(rects: &[(f64, f64, f64, f64)]) {
    let (doc, ids) = doc_of(rects, CutMode::VisibleOutline);
    let g = geometry(&doc);
    let segs = segments(&g, &ids);
    // Nothing seen is left uncut: a point on a rectangle's edge that no
    // rectangle in front covers (inside or on its edge) lies on a cut.
    let near = |p: Point| {
        segs.iter().any(|(a, b)| {
            let d = *b - *a;
            let t = ((p - *a).dot(d) / d.hypot2()).clamp(0.0, 1.0);
            (*a + d * t).distance(p) <= 0.011
        })
    };
    for (k, r) in rects.iter().enumerate() {
        let corners = [(r.0, r.1), (r.2, r.1), (r.2, r.3), (r.0, r.3)];
        for e in 0..4 {
            let (a, b) = (corners[e], corners[(e + 1) % 4]);
            for m in 0..40 {
                let t = (f64::from(m) + 0.5) / 40.0;
                let p = Point::new(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                let hidden = rects[k + 1..]
                    .iter()
                    .any(|f| p.x >= f.0 && p.x <= f.2 && p.y >= f.1 && p.y <= f.3);
                assert!(
                    hidden || near(p),
                    "{p:?} of rect {k} is not cut ({rects:?})"
                );
            }
        }
    }
    // No two cuts run along each other, on one line or a hair apart
    // (within the join tolerance); short crossings where a cut meets a
    // front edge do not count.
    for i in 0..segs.len() {
        for j in i + 1..segs.len() {
            let shared = shared_length_within(segs[i], segs[j], 0.011);
            assert!(
                shared < 0.035,
                "cut twice: {:?} and {:?} share {shared} mm ({rects:?})",
                segs[i],
                segs[j]
            );
        }
    }
}

fn check_merged_area(rects: &[(f64, f64, f64, f64)]) {
    let (doc, ids) = doc_of(rects, CutMode::MergedOutline);
    let g = geometry(&doc);
    // The cut area: outer rings minus holes, which run the other way.
    let mut cut = 0.0;
    for p in ids.iter().flat_map(|id| g.pieces(*id)) {
        cut += signed_area(&p.path);
    }
    // The union sampled at the centres of a quarter-millimetre grid: exact
    // for rectangles on the half-millimetre grid.
    let step = 0.25;
    let mut inside = 0usize;
    for i in 0..(40.0 / step) as usize {
        for j in 0..(40.0 / step) as usize {
            let (x, y) = ((i as f64 + 0.5) * step, (j as f64 + 0.5) * step);
            if rects
                .iter()
                .any(|r| x > r.0 && x < r.2 && y > r.1 && y < r.3)
            {
                inside += 1;
            }
        }
    }
    let sampled = inside as f64 * step * step;
    assert!(
        (cut.abs() - sampled).abs() < 1e-6,
        "cut {cut}, sampled {sampled} ({rects:?})"
    );
}

proptest! {
    #![proptest_config(ci_config(48))]

    #[test]
    fn visible_outline_never_cuts_an_edge_twice(r in jittered_rects()) {
        check_cut_once(&r);
    }

    #[test]
    fn merged_area_matches_sampled_union(r in rects()) {
        check_merged_area(&r);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    #[ignore = "random cases; run with --ignored"]
    fn visible_outline_never_cuts_an_edge_twice_full(r in jittered_rects()) {
        check_cut_once(&r);
    }

    #[test]
    #[ignore = "random cases; run with --ignored"]
    fn merged_area_matches_sampled_union_full(r in rects()) {
        check_merged_area(&r);
    }
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

/// A filled square of side `w` at (`x`, `y`) on a 200 mm page.
fn square(s: &mut Session, x: f64, y: f64, w: f64, name: &str) -> u64 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="{x}" y="{y}" width="{w}" height="{w}"/></svg>"#
    );
    run(s, "object.import_svg", json!({ "svg": svg, "name": name }))["id"]
        .as_u64()
        .unwrap()
}

fn set_mode(s: &mut Session, ids: &[u64], mode: &str) {
    run(
        s,
        "object.set_cut_options",
        json!({ "ids": ids, "mode": mode }),
    );
}

#[test]
fn visible_outline_job_sends() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let back = square(&mut s, 20.0, 20.0, 40.0, "back");
    let front = square(&mut s, 40.0, 40.0, 40.0, "front");
    set_mode(&mut s, &[back], "visible_outline");
    let input = JobInput::of(&s).unwrap();
    let built = job::build_input(&input).unwrap();
    assert!(
        built.warnings.iter().all(|w| !w.contains("kept")),
        "{:?}",
        built.warnings
    );
    // The back is cut as one open path, the front as a ring.
    let g = s.cut_geometry().unwrap();
    assert_eq!(g.pieces(ObjectId(back)).len(), 1);
    assert!(!closed(&g.pieces(ObjectId(back))[0].path));
    assert!(closed(&g.pieces(ObjectId(front))[0].path));
    let r = run(&mut s, "job.send", json!({ "name": "visible" }));
    let file = r["files"][0].as_str().unwrap().to_string();
    let bytes = std::fs::read(file).unwrap();
    assert!(bytes.ends_with(b"H\x03"));
    check_safe(&bytes).unwrap();
    // 160 mm of front ring and 120 mm of the back's visible edge.
    let cut = s.doc().last_job().unwrap().plan.cut_mm;
    assert!(cut > 279.0, "{cut}");
}

#[test]
fn merged_job_sends_one_ring_with_one_weed_box_per_object() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let back = square(&mut s, 20.0, 20.0, 40.0, "back");
    let front = square(&mut s, 40.0, 40.0, 40.0, "front");
    set_mode(&mut s, &[back, front], "merged_outline");
    run(
        &mut s,
        "job.set_weeding",
        json!({ "box": true, "per_object": true }),
    );
    let input = JobInput::of(&s).unwrap();
    let built = job::build_input(&input).unwrap();
    let design: Vec<_> = built
        .plan
        .toolpaths
        .iter()
        .filter(|t| t.kind == flatcraft_path::plan::ToolpathKind::Cut)
        .collect();
    assert_eq!(design.len(), 1, "one welded ring");
    assert_eq!(design[0].source.object, ObjectId(back));
    assert!(
        built.warnings.iter().all(|w| !w.contains("front")),
        "nothing about the welded front object: {:?}",
        built.warnings
    );
    let boxes = built
        .plan
        .toolpaths
        .iter()
        .filter(|t| t.source.object == flatcraft_path::weed::WEED_BOX_OBJECT)
        .count();
    assert_eq!(boxes, 1, "one box around the weld");
    let r = run(&mut s, "job.send", json!({ "name": "merged" }));
    let bytes = std::fs::read(r["files"][0].as_str().unwrap()).unwrap();
    check_safe(&bytes).unwrap();
    // The union of two 40 mm squares shifted by 20 in both directions:
    // perimeter 4 x 40 + 4 x 20 = 240 mm, plus the box.
    let cut: f64 = design
        .iter()
        .map(|t| flatcraft_path::clean::length(&t.points))
        .sum();
    assert!((cut - 240.0).abs() < 1.0, "{cut}");
}

#[test]
fn new_cut_modes_round_trip_through_save_and_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, 20.0, 40.0, "a");
    let b = square(&mut s, 40.0, 40.0, 40.0, "b");
    set_mode(&mut s, &[a], "merged_outline");
    set_mode(&mut s, &[b], "visible_outline");
    let path = dir.path().join("modes.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    let mut s2 = session(&dir);
    run(&mut s2, "project.open", json!({ "path": path }));
    let doc = &s2.doc().document;
    assert_eq!(doc.format_version, 1);
    assert_eq!(
        doc.get(ObjectId(a)).unwrap().cut.mode,
        CutMode::MergedOutline
    );
    assert_eq!(
        doc.get(ObjectId(b)).unwrap().cut.mode,
        CutMode::VisibleOutline
    );
    let st = run(&mut s2, "document.state", json!({}));
    let modes: Vec<&str> = st["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["cut"]["mode"].as_str().unwrap())
        .collect();
    assert_eq!(modes, ["merged_outline", "visible_outline"]);
}

// ---------------------------------------------------------------- hints

fn issues(s: &mut Session) -> Vec<Value> {
    run(s, "document.check", json!({}))["issues"]
        .as_array()
        .unwrap()
        .clone()
}

fn fix_labels(issue: &Value) -> Vec<String> {
    issue["fixes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["label"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn the_overlap_hint_offers_merge_and_the_fix_clears_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, 20.0, 40.0, "a");
    let b = square(&mut s, 40.0, 40.0, 40.0, "b");
    let all = issues(&mut s);
    let overlap = all
        .iter()
        .find(|i| i["code"] == "objects_overlap")
        .expect("overlap hint");
    assert_eq!(
        fix_labels(overlap),
        ["Merge into one outline", "Ignore for this object pair"]
    );
    let fix = &overlap["fixes"][0];
    assert_eq!(
        fix["args"],
        json!({ "ids": [a, b], "mode": "merged_outline" })
    );
    let before = s.doc().undo.undo_len();
    run(&mut s, "object.set_cut_options", fix["args"].clone());
    assert_eq!(s.doc().undo.undo_len(), before + 1, "one undo step");
    assert_eq!(issues(&mut s), Vec::<Value>::new(), "the hint is gone");
    // Undo brings both modes back, and the hint.
    run(&mut s, "edit.undo", json!({}));
    assert!(
        issues(&mut s)
            .iter()
            .any(|i| i["code"] == "objects_overlap")
    );
}

#[test]
fn merge_is_not_offered_when_it_would_not_weld() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, 20.0, 40.0, "a");
    square(&mut s, 40.0, 40.0, 40.0, "b");
    // Another keep mode: they would not weld.
    run(
        &mut s,
        "object.set_keep",
        json!({ "ids": [a], "keep": "stencil" }),
    );
    let all = issues(&mut s);
    let overlap = all.iter().find(|i| i["code"] == "objects_overlap").unwrap();
    assert_eq!(fix_labels(overlap), ["Ignore for this object pair"]);

    // Only close (0.3 mm apart): merging does not bridge a gap.
    let mut s = session(&dir);
    square(&mut s, 20.0, 20.0, 40.0, "a");
    square(&mut s, 60.3, 20.0, 40.0, "b");
    let all = issues(&mut s);
    let close = all.iter().find(|i| i["code"] == "objects_close").unwrap();
    assert!(
        !fix_labels(close)
            .iter()
            .any(|l| l == "Merge into one outline"),
        "{close}"
    );
}

/// Its open pieces are no open paths, and its cut stops at the front
/// object's edge, so the knife does not cut through both: no hint at all.
#[test]
fn a_visible_outline_behind_raises_no_issue() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let back = square(&mut s, 20.0, 20.0, 40.0, "back");
    square(&mut s, 40.0, 40.0, 40.0, "front");
    set_mode(&mut s, &[back], "visible_outline");
    assert_eq!(issues(&mut s), Vec::<Value>::new());
}

#[test]
fn merge_is_never_offered_for_a_visible_outline() {
    // The front object is in visible outline: nothing covers it, so both
    // rings are cut through each other, but merging would undo the mode.
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    square(&mut s, 20.0, 20.0, 40.0, "back");
    let front = square(&mut s, 40.0, 40.0, 40.0, "front");
    set_mode(&mut s, &[front], "visible_outline");
    let all = issues(&mut s);
    let overlap = all.iter().find(|i| i["code"] == "objects_overlap").unwrap();
    assert_eq!(fix_labels(overlap), ["Ignore for this object pair"]);
}

#[test]
fn what_is_seen_of_a_visible_outline_is_kept_and_weeded_around() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let back = square(&mut s, 20.0, 20.0, 40.0, "back");
    square(&mut s, 40.0, 40.0, 40.0, "front");
    set_mode(&mut s, &[back], "visible_outline");
    run(
        &mut s,
        "job.set_weeding",
        json!({ "box": true, "lines": "both", "spacing_mm": 5 }),
    );
    let input = JobInput::of(&s).unwrap();
    let plan = job::build_input(&input).unwrap().plan;
    // The seen part of the back square and the front square are kept,
    // the sheet around them is waste.
    assert!(plan.kept.is_kept(Point::new(25.0, 25.0)));
    assert!(plan.kept.is_kept(Point::new(55.0, 30.0)));
    assert!(plan.kept.is_kept(Point::new(70.0, 70.0)));
    assert!(!plan.kept.is_kept(Point::new(10.0, 10.0)));
    // Weed lines run in the waste only: none crosses the back's seen part.
    let seen = |p: &Point| {
        p.x > 20.0 + 1e-6
            && p.x < 60.0 - 1e-6
            && p.y > 20.0 + 1e-6
            && p.y < 60.0 - 1e-6
            && !(p.x > 40.0 && p.y > 40.0)
    };
    let weeds: Vec<_> = plan
        .toolpaths
        .iter()
        .filter(|t| t.kind == flatcraft_path::plan::ToolpathKind::Weed)
        .collect();
    assert!(!weeds.is_empty());
    for t in weeds {
        for w in t.points.windows(2) {
            for k in 0..=10 {
                let p = w[0].lerp(w[1], f64::from(k) / 10.0);
                assert!(!seen(&p), "a weed line crosses the back at {p:?}");
            }
        }
    }
}

#[test]
fn a_covered_object_keeps_the_box_of_its_outlines() {
    // The box of a cross-object mode is the box of its outlines.
    let mut doc = Document::new(job_settings());
    let back = add(
        &mut doc,
        "back",
        vec![filled(rect(0.0, 0.0, 20.0, 20.0))],
        CutMode::VisibleOutline,
    );
    add(
        &mut doc,
        "front",
        vec![filled(rect(10.0, 10.0, 30.0, 30.0))],
        CutMode::ObjectOutlines,
    );
    let g = geometry(&doc);
    let b = g.bounds(back).unwrap();
    let want = Rect::new(0.0, 0.0, 20.0, 20.0);
    let off = (b.x0 - want.x0).abs()
        + (b.y0 - want.y0).abs()
        + (b.x1 - want.x1).abs()
        + (b.y1 - want.y1).abs();
    assert!(off < 1e-9, "{b:?}");
}

// ---------------------------------------------------------------- order

fn order(s: &Session) -> Vec<u64> {
    s.doc().document.objects.iter().map(|o| o.id.0).collect()
}

fn reorder(s: &mut Session, ids: &[u64], to: &str) -> Value {
    run(s, "object.reorder", json!({ "ids": ids, "to": to }))
}

#[test]
fn reordering_changes_the_visible_outline() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let back = square(&mut s, 20.0, 20.0, 40.0, "back");
    let front = square(&mut s, 40.0, 40.0, 40.0, "front");
    set_mode(&mut s, &[back, front], "visible_outline");
    let open = |s: &Session, id: u64| {
        let g = s.cut_geometry().unwrap();
        !closed(&g.pieces(ObjectId(id))[0].path)
    };
    assert!(open(&s, back) && !open(&s, front));
    let before = s.doc().undo.undo_len();
    let r = reorder(&mut s, &[back], "front");
    assert_eq!(r["changed"], true);
    assert_eq!(r["order"], json!([front, back]));
    assert_eq!(s.doc().undo.undo_len(), before + 1, "one undo step");
    assert!(
        !open(&s, back) && open(&s, front),
        "the old back is in front now"
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(order(&s), [back, front]);
    assert!(open(&s, back) && !open(&s, front));
}

#[test]
fn reorder_steps_one_object_and_changes_nothing_at_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 10.0, 10.0, 10.0, "a");
    let b = square(&mut s, 30.0, 10.0, 10.0, "b");
    let c = square(&mut s, 50.0, 10.0, 10.0, "c");
    reorder(&mut s, &[a], "forward");
    assert_eq!(order(&s), [b, a, c]);
    reorder(&mut s, &[a], "forward");
    assert_eq!(order(&s), [b, c, a]);
    let before = s.doc().undo.undo_len();
    let r = reorder(&mut s, &[a], "front");
    assert_eq!(r["changed"], false);
    assert_eq!(s.doc().undo.undo_len(), before, "no step for no change");
    reorder(&mut s, &[a], "backward");
    assert_eq!(order(&s), [b, a, c]);
    reorder(&mut s, &[c], "back");
    assert_eq!(order(&s), [c, b, a]);
    // Two picked together keep their order and step together.
    reorder(&mut s, &[c, b], "forward");
    assert_eq!(order(&s), [a, c, b]);
}

#[test]
fn a_group_moves_and_is_passed_as_one_block() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 10.0, 10.0, 10.0, "a");
    let c = square(&mut s, 30.0, 10.0, 10.0, "c");
    let b = square(&mut s, 50.0, 10.0, 10.0, "b");
    let d = square(&mut s, 70.0, 10.0, 10.0, "d");
    let g = run(&mut s, "object.group", json!({ "ids": [a, b] }))["group_id"]
        .as_u64()
        .unwrap();
    // The group goes to the front as one block, in its own order.
    reorder(&mut s, &[g], "front");
    assert_eq!(order(&s), [c, d, a, b]);
    // A step forward of d passes the whole group.
    reorder(&mut s, &[d], "forward");
    assert_eq!(order(&s), [c, a, b, d]);
    // A step back of the group passes c.
    reorder(&mut s, &[g], "backward");
    assert_eq!(order(&s), [a, b, c, d]);
    // A member picked on its own moves on its own.
    reorder(&mut s, &[a], "forward");
    assert_eq!(order(&s), [b, a, c, d]);
    let st = run(&mut s, "document.state", json!({}));
    let listed: Vec<u64> = st["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_u64().unwrap())
        .collect();
    assert_eq!(listed, order(&s), "document.state lists the stacking order");
}

// ---------------------------------------------------------------- scale

/// `n` squares of 8 mm on a grid, each overlapping its right and lower
/// neighbours by 1 mm, all in `mode`.
fn sheet(n: usize, mode: CutMode) -> Document {
    let mut doc = Document::new(job_settings());
    let side = (n as f64).sqrt().ceil() as usize;
    for k in 0..n {
        let (x, y) = ((k % side) as f64 * 7.0, (k / side) as f64 * 7.0);
        add(
            &mut doc,
            &format!("s{k}"),
            vec![filled(rect(x, y, x + 8.0, y + 8.0))],
            mode,
        );
    }
    doc
}

#[test]
#[ignore = "timing table; run with --ignored --nocapture"]
fn cross_object_pass_timing() {
    for mode in [CutMode::MergedOutline, CutMode::VisibleOutline] {
        for n in [100, 200, 400] {
            let doc = sheet(n, mode);
            let start = std::time::Instant::now();
            let first = build(&doc, None);
            eprintln!("TIMING {mode:?} n={n}: first build {:?}", start.elapsed());
            // Move one object: the cross pass runs again on cached solos.
            let mut moved = doc.clone();
            moved.objects[0].transform.center_mm.x += 0.5;
            let start = std::time::Instant::now();
            let again = build(&moved, Some(&first));
            let t = start.elapsed();
            eprintln!(
                "TIMING {mode:?} n={n}: cross pass {t:?} ({} pieces on 0)",
                again.pieces(moved.objects[0].id).len()
            );
        }
    }
}

#[test]
fn a_step_inside_a_group_passes_one_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 10.0, 10.0, 10.0, "a");
    let b = square(&mut s, 30.0, 10.0, 10.0, "b");
    let e = square(&mut s, 50.0, 10.0, 10.0, "e");
    let c = square(&mut s, 70.0, 10.0, 10.0, "c");
    run(&mut s, "object.group", json!({ "ids": [a, b, e] }));
    // One step among the three members, never out of the group.
    reorder(&mut s, &[a], "forward");
    assert_eq!(order(&s), [b, a, e, c]);
    reorder(&mut s, &[a], "forward");
    assert_eq!(order(&s), [b, e, a, c]);
    let r = reorder(&mut s, &[a], "forward");
    assert_eq!(r["changed"], false, "the front of its group");
    reorder(&mut s, &[a], "backward");
    assert_eq!(order(&s), [b, a, e, c]);
}

#[test]
fn a_step_inside_nested_groups_passes_a_sibling_group_as_a_whole() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 10.0, 10.0, 10.0, "a");
    let b = square(&mut s, 30.0, 10.0, 10.0, "b");
    let e = square(&mut s, 50.0, 10.0, 10.0, "e");
    let f = square(&mut s, 70.0, 10.0, 10.0, "f");
    let c = square(&mut s, 90.0, 10.0, 10.0, "c");
    // Outer group {a, inner {b, e}, f}.
    let inner = run(&mut s, "object.group", json!({ "ids": [b, e] }))["group_id"]
        .as_u64()
        .unwrap();
    run(&mut s, "object.group", json!({ "ids": [a, inner, f] }));
    // a steps over the inner group as one sibling.
    reorder(&mut s, &[a], "forward");
    assert_eq!(order(&s), [b, e, a, f, c]);
    // b steps inside the inner group only.
    reorder(&mut s, &[b], "forward");
    assert_eq!(order(&s), [e, b, a, f, c]);
    assert_eq!(reorder(&mut s, &[b], "forward")["changed"], false);
    // The inner group is at the back of the outer group already; a step
    // forward passes a, as a whole.
    assert_eq!(reorder(&mut s, &[inner], "backward")["changed"], false);
    reorder(&mut s, &[inner], "forward");
    assert_eq!(order(&s), [a, e, b, f, c]);
    // f steps back over the inner group as a whole.
    reorder(&mut s, &[f], "backward");
    assert_eq!(order(&s), [a, f, e, b, c]);
}

// ---------------------------------------------------------------- review fixes

#[test]
fn a_near_coincident_edge_is_cut_once() {
    // The front square sits 0.004 mm lower than the back's top edge: the
    // back's top edge along it lies just outside the front, within the
    // join tolerance. Only the front cuts it.
    let mut doc = Document::new(job_settings());
    let back = add(
        &mut doc,
        "back",
        vec![filled(rect(0.0, 0.0, 20.0, 20.0))],
        CutMode::VisibleOutline,
    );
    let front = add(
        &mut doc,
        "front",
        vec![filled(rect(10.0, 0.004, 30.0, 20.004))],
        CutMode::ObjectOutlines,
    );
    let g = geometry(&doc);
    let segs = segments(&g, &[back, front]);
    for i in 0..segs.len() {
        for j in i + 1..segs.len() {
            let shared = shared_length_within(segs[i], segs[j], 0.011);
            assert!(
                shared < 0.035,
                "{:?} and {:?} share {shared}",
                segs[i],
                segs[j]
            );
        }
    }
    // The back still meets the front: nothing of its edge outside the
    // band is lost.
    assert!(
        (total_length(&g, &[back]) - 40.0).abs() < 0.05,
        "{}",
        total_length(&g, &[back])
    );
}

/// The length two segments share when they lie on one line, or within
/// `off` of it.
fn shared_length_within(a: (Point, Point), b: (Point, Point), off: f64) -> f64 {
    let d = a.1 - a.0;
    let len = d.hypot();
    if len == 0.0 {
        return 0.0;
    }
    let dist = |p: Point| d.cross(p - a.0).abs() / len;
    if dist(b.0) > off || dist(b.1) > off {
        return 0.0;
    }
    let t = |p: Point| d.dot(p - a.0) / len;
    let (b0, b1) = (t(b.0).min(t(b.1)), t(b.0).max(t(b.1)));
    (b1.min(len) - b0.max(0.0)).max(0.0)
}

#[test]
fn a_front_sliver_that_is_not_cut_covers_nothing() {
    // A filled strip 0.1 mm wide: below half the blade offset, so it is
    // not cut, and it must not cover the back's edge either.
    let mut doc = Document::new(job_settings());
    let back = add(
        &mut doc,
        "back",
        vec![filled(rect(0.0, 0.0, 20.0, 20.0))],
        CutMode::VisibleOutline,
    );
    let sliver = add(
        &mut doc,
        "sliver",
        vec![filled(rect(19.95, -5.0, 20.05, 25.0))],
        CutMode::ObjectOutlines,
    );
    let g = geometry(&doc);
    assert!(g.pieces(sliver).is_empty());
    assert_eq!(g.pieces(back).len(), 1);
    assert!(closed(&g.pieces(back)[0].path), "{:?}", g.pieces(back));
}

#[test]
fn a_seen_sliver_is_dropped_with_the_warning() {
    // The front covers all of the back but a 0.1 mm strip on the right.
    let mut doc = Document::new(job_settings());
    let back = add(
        &mut doc,
        "back",
        vec![filled(rect(0.0, 0.0, 20.0, 20.0))],
        CutMode::VisibleOutline,
    );
    add(
        &mut doc,
        "front",
        vec![filled(rect(-5.0, -5.0, 19.9, 25.0))],
        CutMode::ObjectOutlines,
    );
    let g = geometry(&doc);
    assert!(g.pieces(back).is_empty(), "{:?}", g.pieces(back));
    assert_eq!(g.facts(back).slivers, 1);
    assert!(
        g.warnings.iter().any(|w| w.contains("'back'")),
        "{:?}",
        g.warnings
    );
    assert!(!g.regions.contains_key(&back), "no kept outline for it");
}

#[test]
fn a_stroked_seen_sliver_is_cut_once_along_its_middle() {
    let mut doc = Document::new(job_settings());
    let mut plate = filled(rect(0.0, 0.0, 20.0, 20.0));
    plate.paint.stroke = Some(StrokeInfo {
        width_mm: 0.3,
        join: Join::Round,
        cap: Cap::Round,
    });
    let back = add(&mut doc, "back", vec![plate], CutMode::VisibleOutline);
    add(
        &mut doc,
        "front",
        vec![filled(rect(-5.0, -5.0, 19.9, 25.0))],
        CutMode::ObjectOutlines,
    );
    let g = geometry(&doc);
    let pieces = g.pieces(back);
    assert_eq!(pieces.len(), 1, "{pieces:?}");
    // Along the middle of the strip, x = 19.95 (a blunt end starts at a
    // corner, half the width off the middle).
    let v = vertices(&pieces[0].path);
    for p in &v {
        assert!((p.x - 19.95).abs() <= 0.05 + 1e-6, "{v:?}");
    }
    for p in &v[1..v.len() - 1] {
        assert!((p.x - 19.95).abs() < 1e-3, "{v:?}");
    }
    assert_eq!(g.facts(back).slivers, 0);
}

#[test]
fn a_thin_seen_strip_is_named_in_a_hint() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let back = square(&mut s, 20.0, 20.0, 40.0, "back");
    // The front leaves a 0.2 mm strip of the back on the right.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="10" y="10" width="49.8" height="60"/></svg>"#;
    run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg, "name": "front" }),
    );
    set_mode(&mut s, &[back], "visible_outline");
    let all = issues(&mut s);
    let hint = all
        .iter()
        .find(|i| i["code"] == "objects_close")
        .unwrap_or_else(|| panic!("{all:?}"));
    let msg = hint["message"].as_str().unwrap();
    assert!(
        msg.contains("'front' leaves only a 0.2 mm strip of 'back'"),
        "{msg}"
    );
    assert_eq!(fix_labels(hint), ["Ignore for this object pair"]);
}

#[test]
fn a_front_line_without_area_keeps_the_overlap_hint() {
    // A centre-line stroke covers nothing: the back is cut through it.
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let back = square(&mut s, 20.0, 20.0, 40.0, "back");
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><path d="M10 40 H80" fill="none" stroke="black" stroke-width="0.3"/></svg>"#;
    run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg, "name": "line" }),
    );
    set_mode(&mut s, &[back], "visible_outline");
    assert!(
        issues(&mut s)
            .iter()
            .any(|i| i["code"] == "objects_overlap"),
        "{:?}",
        issues(&mut s)
    );
}

#[test]
fn the_overlap_hint_names_the_merged_outline() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let w = square(&mut s, 20.0, 20.0, 40.0, "W");
    let m = square(&mut s, 50.0, 20.0, 40.0, "M");
    set_mode(&mut s, &[w, m], "merged_outline");
    // X overlaps M only.
    square(&mut s, 85.0, 30.0, 20.0, "X");
    let all = issues(&mut s);
    let overlap = all
        .iter()
        .find(|i| i["code"] == "objects_overlap")
        .unwrap_or_else(|| panic!("{all:?}"));
    let msg = overlap["message"].as_str().unwrap();
    assert!(
        msg.starts_with("The merged outline of 'W' and 'X'"),
        "{msg}"
    );
}
