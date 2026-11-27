//! The cut geometry cache: keyed on the document and profile revisions,
//! shared by the job, the report and the window.

use std::path::Path;
use std::sync::Arc;

use flatcraft_device::ProfileStore;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{Origin, Session};
use serde_json::{Value, json};

struct Dirs {
    config: tempfile::TempDir,
    out: tempfile::TempDir,
}

fn dirs() -> Dirs {
    Dirs {
        config: tempfile::tempdir().unwrap(),
        out: tempfile::tempdir().unwrap(),
    }
}

fn session(d: &Dirs) -> Session {
    let profiles = ProfileStore::load(d.config.path()).unwrap();
    let prefs = Preferences {
        output_dir: Some(d.out.path().to_path_buf()),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, false, d.config.path().to_path_buf())
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

fn import(s: &mut Session, svg: &str, name: &str) -> u64 {
    run(s, "object.import_svg", json!({ "svg": svg, "name": name }))["id"]
        .as_u64()
        .unwrap()
}

fn svg(body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100">{body}</svg>"#
    )
}

const SQUARE: &str = r#"<rect x="20" y="20" width="30" height="30"/>"#;

fn user_plotter(config: &Path, body: &str) {
    let plotters = config.join("plotters");
    std::fs::create_dir_all(&plotters).unwrap();
    std::fs::write(
        plotters.join("custom.toml"),
        format!(
            "id = \"custom\"\nname = \"Custom\"\ninherits = \"file-output-gpgl-cameo2\"\n{body}"
        ),
    )
    .unwrap();
}

/// The bytes of the job file `job.send` wrote.
fn sent_bytes(r: &Value) -> Vec<u8> {
    let file = r["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .find(|f| !f.ends_with(".txt") && !f.ends_with(".jsonl"))
        .unwrap()
        .to_string();
    std::fs::read(file).unwrap()
}

#[test]
fn cut_geometry_is_cached_until_an_edit() {
    let d = dirs();
    let mut s = session(&d);
    let id = import(&mut s, &svg(SQUARE), "square");
    let a = s.cut_geometry().unwrap();
    let b = s.cut_geometry().unwrap();
    assert!(Arc::ptr_eq(&a, &b), "the same build twice");
    run(&mut s, "object.move", json!({ "ids": [id], "dx_mm": 5.0 }));
    let c = s.cut_geometry().unwrap();
    assert!(!Arc::ptr_eq(&a, &c), "a move builds again");
    let moved = c.pieces(flatcraft_core::document::ObjectId(id))[0]
        .path
        .elements()[0]
        .end_point()
        .unwrap();
    let before = a.pieces(flatcraft_core::document::ObjectId(id))[0]
        .path
        .elements()[0]
        .end_point()
        .unwrap();
    assert!(
        (moved.x - before.x - 5.0).abs() < 1e-9,
        "{moved:?} {before:?}"
    );
}

#[test]
fn only_changed_objects_are_cut_again() {
    let d = dirs();
    let mut s = session(&d);
    let a = import(&mut s, &svg(SQUARE), "a");
    let b = import(&mut s, &svg(SQUARE), "b");
    run(&mut s, "object.move", json!({ "ids": [b], "dx_mm": 40.0 }));
    let first = s.cut_geometry().unwrap();
    run(&mut s, "object.move", json!({ "ids": [a], "dy_mm": 5.0 }));
    let second = s.cut_geometry().unwrap();
    let (a, b) = (
        flatcraft_core::document::ObjectId(a),
        flatcraft_core::document::ObjectId(b),
    );
    assert!(Arc::ptr_eq(&first.per_object[&b], &second.per_object[&b]));
    assert!(!Arc::ptr_eq(&first.per_object[&a], &second.per_object[&a]));
}

#[test]
fn undo_invalidates_cut_geometry_and_last_sim() {
    let d = dirs();
    let mut s = session(&d);
    let id = import(&mut s, &svg(SQUARE), "square");
    let before = sent_bytes(&run(&mut s, "job.send", json!({})));
    let key = s.doc().cut_cache_key().unwrap();
    run(&mut s, "object.move", json!({ "ids": [id], "dx_mm": 7.0 }));
    let moved = sent_bytes(&run(&mut s, "job.send", json!({})));
    assert_ne!(before, moved);
    run(&mut s, "edit.undo", json!({}));
    let _ = s.cut_geometry().unwrap();
    let after = s.doc().cut_cache_key().unwrap();
    assert_ne!(key, after, "undo moves the key on");
    assert!(after.doc_revision > key.doc_revision);
    let again = sent_bytes(&run(&mut s, "job.send", json!({})));
    assert_eq!(before, again, "the pre-move geometry is cut again");
    // The last simulation follows the same key.
    run(&mut s, "job.simulate", json!({}));
    let sim = s.last_sim().expect("a fresh simulation").clone();
    run(&mut s, "job.simulate", json!({}));
    assert!(
        Arc::ptr_eq(s.last_sim().unwrap(), &sim),
        "an unchanged document reuses it"
    );
    run(&mut s, "edit.redo", json!({}));
    assert!(s.last_sim().is_none(), "redo makes it stale");
    run(&mut s, "edit.undo", json!({}));
    assert!(s.last_sim().is_none(), "undo makes it stale too");
}

#[test]
fn profile_edit_invalidates_cut_geometry() {
    let d = dirs();
    user_plotter(d.config.path(), "flatten_tolerance_mm = 0.01\n");
    let mut s = session(&d);
    run(&mut s, "job.set_plotter", json!({ "id": "custom" }));
    import(
        &mut s,
        &svg(r#"<circle cx="40" cy="40" r="20"/>"#),
        "circle",
    );
    let a = s.cut_geometry().unwrap();
    assert_eq!(a.tolerance_mm, 0.01);
    user_plotter(d.config.path(), "flatten_tolerance_mm = 0.05\n");
    s.profiles.reload().unwrap();
    let b = s.cut_geometry().unwrap();
    assert!(!Arc::ptr_eq(&a, &b));
    assert_eq!(b.tolerance_mm, 0.05);
    assert_eq!(
        s.doc().cut_cache_key().unwrap().profiles_revision,
        s.profiles.revision()
    );
}

/// A command that reads the cut geometry half way (the import report)
/// sees its own change, although the revision only moves after it.
#[test]
fn a_report_inside_a_command_sees_the_new_object() {
    let d = dirs();
    let mut s = session(&d);
    import(&mut s, &svg(SQUARE), "square");
    let _ = s.cut_geometry().unwrap();
    let r = run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg(r#"<rect x="60" y="60" width="0.5" height="0.5"/>"#), "name": "dot" }),
    );
    let codes: Vec<&str> = r["report"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"below_min_feature"), "{r}");
}

#[test]
fn labels_follow_pieces() {
    let d = dirs();
    let mut s = session(&d);
    // Path 0 is a ring with a counter (two pieces); path 1 a square; path
    // 2 carries the label.
    let body = r#"<path d="M5 5 H35 V35 H5 Z M15 15 V25 H25 V15 Z"/><rect x="40" y="5" width="10" height="10"/><g id="tag"><rect x="60" y="5" width="10" height="10"/></g>"#;
    let id = import(&mut s, &svg(body), "labels");
    let g = s.cut_geometry().unwrap();
    let pieces = g.pieces(flatcraft_core::document::ObjectId(id));
    assert!(pieces.len() > 3, "{}", pieces.len());
    let label = s.doc().document.objects[0].geometry[2].label.clone();
    assert_eq!(label.as_deref(), Some("tag"));
    let tagged: Vec<_> = pieces.iter().filter(|p| p.source == Some(2)).collect();
    assert!(!tagged.is_empty());
    assert!(tagged.iter().all(|p| p.label == label));
    let r = run(&mut s, "job.preview", json!({}));
    let notes: Vec<&str> = r["listing"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|l| l["note"].as_str())
        .collect();
    let with_label: Vec<&&str> = notes.iter().filter(|n| n.contains("[tag]")).collect();
    assert!(!with_label.is_empty(), "{notes:?}");
    assert!(
        with_label.iter().all(|n| n.contains("path 2 ")),
        "{notes:?}"
    );
    assert!(notes.iter().any(|n| n.contains("path 0,")), "{notes:?}");
    assert!(!notes.iter().any(|n| n.contains("path 3")), "{notes:?}");
}

// ---------------------------------------------------------------- minimum feature size

fn warnings(v: &Value) -> Vec<String> {
    v["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap().to_string())
        .collect()
}

fn codes(v: &Value) -> Vec<String> {
    v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["code"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn fill_only_hairline_sliver_is_dropped_with_warning() {
    let d = dirs();
    let mut s = session(&d);
    // Blade 0.25 mm: the narrowest cut feature is 0.125 mm.
    let thin = import(
        &mut s,
        &svg(
            r#"<rect x="20" y="20" width="50" height="0.1"/><rect x="20" y="40" width="10" height="10"/>"#,
        ),
        "thin",
    );
    let g = s.cut_geometry().unwrap();
    let id = flatcraft_core::document::ObjectId(thin);
    assert_eq!(g.pieces(id).len(), 1, "only the square is cut");
    assert_eq!(g.facts(id).slivers, 1);
    assert_eq!(g.facts(id).no_area, 0);
    let r = run(&mut s, "job.preview", json!({}));
    assert!(
        warnings(&r).contains(
            &"'thin': 1 filled sliver is thinner than 0.125 mm and is not cut.".to_string()
        ),
        "{r}"
    );
    assert_eq!(codes(&r), ["thin_slivers"], "{r}");
    let c = run(&mut s, "document.check", json!({}));
    assert_eq!(codes(&c), ["thin_slivers"], "{c}");
    assert_eq!(
        c["issues"][0]["severity"], "warning",
        "a sliver never blocks"
    );

    let mut s = session(&d);
    let wide = import(
        &mut s,
        &svg(r#"<rect x="20" y="20" width="50" height="0.2"/>"#),
        "wide",
    );
    let g = s.cut_geometry().unwrap();
    let id = flatcraft_core::document::ObjectId(wide);
    assert_eq!(g.pieces(id).len(), 1, "a 0.2 mm strip is kept");
    assert_eq!(g.facts(id).slivers, 0);
}

/// An object of slivers only: nothing to cut, and the reason is named.
#[test]
fn an_object_of_slivers_says_why_nothing_cuts() {
    let d = dirs();
    let mut s = session(&d);
    import(
        &mut s,
        &svg(r#"<rect x="20" y="20" width="50" height="0.1"/>"#),
        "thin",
    );
    let r = run(&mut s, "job.preview", json!({}));
    assert_eq!(r["ready"], false, "{r}");
    assert_eq!(codes(&r), ["thin_slivers"], "{r}");
    assert_eq!(r["issues"][0]["severity"], "error");
}

/// A hand-drawn line exported as a fill: out along a line and back with a
/// little jitter. Its fill is a chain of tiny lobes, which used to be cut
/// as micro loops; now they are slivers.
#[test]
fn jittered_fill_line_no_longer_micro_loops() {
    let d = dirs();
    let mut s = session(&d);
    let mut path = String::from("M10 50 L60 50");
    for k in 0..=50 {
        let x = 60.0 - f64::from(k);
        let y = if k % 2 == 0 { 50.04 } else { 49.96 };
        path.push_str(&format!(" L{x} {y}"));
    }
    path.push_str(" Z");
    let id = import(&mut s, &svg(&format!(r#"<path d="{path}"/>"#)), "jitter");
    let g = s.cut_geometry().unwrap();
    let id = flatcraft_core::document::ObjectId(id);
    assert!(g.pieces(id).is_empty(), "{:?}", g.pieces(id).len());
    assert!(g.facts(id).slivers > 0);
}

/// Ruling M2-P3: the hairline limit follows the minimum feature width.
/// A closed stroked strip 0.11 mm wide (above the old 0.1 mm limit, below
/// half the blade offset) is one spine cut, not a ring.
#[test]
fn a_closed_strip_below_the_minimum_width_is_one_spine_cut() {
    let d = dirs();
    let mut s = session(&d);
    let id = import(
        &mut s,
        &svg(
            r#"<path d="M10 50 H60 V50.11 H10 Z" fill="black" stroke="black" stroke-width="0.05"/>"#,
        ),
        "strip",
    );
    let g = s.cut_geometry().unwrap();
    let pieces = g.pieces(flatcraft_core::document::ObjectId(id));
    assert_eq!(pieces.len(), 1);
    let els = pieces[0].path.elements();
    assert!(
        !els.contains(&kurbo::PathEl::ClosePath),
        "a spine, not a ring: {els:?}"
    );
    assert!(
        (flatcraft_engine::cutgeom::hairline_mm(0.01, 0.125) - 0.125).abs() < 1e-12,
        "structural: max(t, min_width)"
    );
}

/// The decision rule of the Task 13 addition: at the default cut options
/// and the default blade, no stroke width is both cut around its outline
/// and reported as a hairline. With strokes forced to outline, the ones
/// under the minimum feature size are reported.
#[test]
fn no_default_stroke_is_outlined_and_a_hairline() {
    let min_feature = flatcraft_engine::report::min_feature_mm(0.25);
    for w in [0.1, 0.5, 1.0, 1.2, 1.49, 1.5, 2.0, 3.0] {
        let d = dirs();
        let mut s = session(&d);
        let id = import(
            &mut s,
            &svg(&format!(
                r#"<rect x="20" y="20" width="30" height="30" fill="none" stroke="black" stroke-width="{w}"/>"#
            )),
            "frame",
        );
        let c = run(&mut s, "document.check", json!({}));
        let g = s.cut_geometry().unwrap();
        let outlined = g.pieces(flatcraft_core::document::ObjectId(id)).len() > 1;
        let hairline = codes(&c).contains(&"hairline".to_string());
        assert!(!(outlined && hairline), "{w} mm: {c}");
        run(
            &mut s,
            "object.set_cut_options",
            json!({ "ids": [id], "strokes": "outline" }),
        );
        let c = run(&mut s, "document.check", json!({}));
        let hairline = codes(&c).contains(&"hairline".to_string());
        assert_eq!(hairline, w < min_feature, "{w} mm outlined: {c}");
    }
    // Structural: the default threshold is the default blade's minimum
    // feature size.
    assert_eq!(
        flatcraft_core::document::DEFAULT_AUTO_THRESHOLD_MM,
        min_feature
    );
}

// ---------------------------------------------------------------- stroked spurs

/// The open pieces of the cut of a single filled path with a thin stroke,
/// and the total length of each kind.
fn spur_cut(d: &str) -> (Vec<kurbo::BezPath>, Vec<kurbo::BezPath>) {
    let dirs = dirs();
    let mut s = session(&dirs);
    let id = import(
        &mut s,
        &svg(&format!(
            r#"<path d="{d}" fill="black" stroke="black" stroke-width="0.2"/>"#
        )),
        "spur",
    );
    let g = s.cut_geometry().unwrap();
    let pieces = g.pieces(flatcraft_core::document::ObjectId(id));
    let (rings, open): (Vec<_>, Vec<_>) = pieces
        .iter()
        .map(|p| p.path.clone())
        .partition(|p| p.elements().contains(&kurbo::PathEl::ClosePath));
    (rings, open)
}

fn length(paths: &[kurbo::BezPath]) -> f64 {
    use kurbo::ParamCurveArclen as _;
    paths
        .iter()
        .flat_map(|p| p.segments())
        .map(|s| s.arclen(1e-6))
        .sum()
}

#[test]
fn stroked_spur_in_closed_subpath_is_cut() {
    // As the brief gives it: the tail is a subpath of its own.
    let (rings, open) = spur_cut("M10 10 H30 V30 H10 Z M10 10 L0 0");
    assert_eq!(rings.len(), 1);
    assert!((length(&open) - 200f64.sqrt()).abs() < 0.05, "{open:?}");
    // The tail drawn inside the subpath that encloses the area: out and
    // back along the same line, cut once.
    let (rings, open) = spur_cut("M10 10 L0 0 L10 10 H30 V30 H10 Z");
    assert_eq!(rings.len(), 1);
    assert!((length(&open) - 200f64.sqrt()).abs() < 0.05, "{open:?}");
    // A tail that ends inside the fill is cut into it.
    let (rings, open) = spur_cut("M10 10 L20 20 L10 10 H30 V30 H10 Z");
    assert_eq!(rings.len(), 1);
    assert!((length(&open) - 200f64.sqrt()).abs() < 0.05, "{open:?}");
}

#[test]
fn spur_on_the_fill_edge_is_not_cut_twice() {
    for d in [
        "M10 10 H30 V30 H10 Z",
        "M10 10 H30 V30 H10 V10",
        // A circle: its flattened stroke runs along the flattened edge.
        "M40 20 A20 20 0 1 1 0 20 A20 20 0 1 1 40 20 Z",
        // An edge drawn twice in one subpath.
        "M10 10 H30 H10 H30 V30 H10 Z",
    ] {
        let (rings, open) = spur_cut(d);
        assert_eq!(rings.len(), 1, "{d}");
        assert!(open.is_empty(), "{d}: {open:?}");
    }
}

/// A long outline of curves with a tail at its start: the clip hands back
/// the tail joined to the closing edge, a hair off the fill's own edge.
/// Only the tail is cut.
#[test]
fn a_tail_on_a_curvy_outline_is_cut_alone() {
    let mut d = String::from("M10 10 L5 5 L10 10");
    for k in 0..100 {
        let x = 10.0 + f64::from(k) * 2.8;
        let y = if k % 2 == 0 { 6.0 } else { 14.0 };
        d.push_str(&format!(" Q{} {y} {} 10", x + 1.4, x + 2.8));
    }
    d.push_str(" V200");
    for k in 0..100 {
        let x = 290.0 - f64::from(k) * 2.8;
        let y = if k % 2 == 0 { 196.0 } else { 204.0 };
        d.push_str(&format!(" Q{} {y} {} 200", x - 1.4, x - 2.8));
    }
    d.push_str(" Z");
    let (rings, open) = spur_cut(&d);
    assert_eq!(rings.len(), 1);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!((length(&open) - 50f64.sqrt()).abs() < 0.05, "{open:?}");
}

/// A sliver is not cut, so it never counts for the box of what is cut: no
/// off-the-mat error, no space in the layout, the job plans.
#[test]
fn an_uncut_sliver_takes_no_space() {
    let d = dirs();
    let mut s = session(&d);
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="500mm" height="500mm" viewBox="0 0 500 500"><rect x="20" y="20" width="20" height="20"/><rect x="20" y="50" width="400" height="0.1"/></svg>"#;
    let id = run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg, "name": "square and sliver" }),
    )["id"]
        .as_u64()
        .unwrap();
    let state = run(&mut s, "document.state", json!({}));
    let b = &state["objects"][0]["bounds_mm"];
    let width = b["x1"].as_f64().unwrap() - b["x0"].as_f64().unwrap();
    let height = b["y1"].as_f64().unwrap() - b["y0"].as_f64().unwrap();
    assert!((width - 20.0).abs() < 0.01, "{b}");
    assert!((height - 20.0).abs() < 0.01, "{b}");
    let c = run(&mut s, "document.check", json!({}));
    assert_eq!(codes(&c), ["thin_slivers"], "{c}");
    let p = run(&mut s, "job.preview", json!({}));
    assert_eq!(p["ready"], true, "{p}");
    // Auto arrange packs the square, not the sliver.
    run(&mut s, "arrange.auto", json!({ "ids": [id] }));
    let c = run(&mut s, "document.check", json!({}));
    assert_eq!(codes(&c), ["thin_slivers"], "{c}");
    // The export has the square only.
    let out = d.out.path().join("x.svg");
    run(
        &mut s,
        "document.export",
        json!({ "path": out, "format": "svg" }),
    );
    let text = std::fs::read_to_string(&out).unwrap();
    assert_eq!(text.matches("<path").count(), 1, "{text}");
}

// ---------------------------------------------------------------- compound slivers

/// An even-odd frame 0.1 mm wide: outer square 10..30, inner 10.1..29.9.
const FRAME_D: &str = "M10 10 H30 V30 H10 Z M10.1 10.1 V29.9 H29.9 V10.1 Z";

/// Imports one path; returns the pieces, the job preview and the check.
fn compound(d: &str, stroke: Option<f64>) -> (Vec<kurbo::BezPath>, Value, Value) {
    let dirs = dirs();
    let mut s = session(&dirs);
    let paint = match stroke {
        Some(w) => format!(r#"fill="black" stroke="black" stroke-width="{w}""#),
        None => r#"fill="black""#.to_string(),
    };
    let id = import(
        &mut s,
        &svg(&format!(r#"<path fill-rule="evenodd" d="{d}" {paint}/>"#)),
        "frame",
    );
    let g = s.cut_geometry().unwrap();
    let pieces = g
        .pieces(flatcraft_core::document::ObjectId(id))
        .iter()
        .map(|p| p.path.clone())
        .collect();
    let p = run(&mut s, "job.preview", json!({}));
    let c = run(&mut s, "document.check", json!({}));
    (pieces, p, c)
}

fn closed(p: &kurbo::BezPath) -> bool {
    p.elements().contains(&kurbo::PathEl::ClosePath)
}

/// True if `p` runs in the middle of the 0.1 mm frame: every point
/// 0.05 mm (plus a little) inside the outer square.
fn on_frame_middle(p: &kurbo::BezPath) -> bool {
    p.elements().iter().filter_map(|e| e.end_point()).all(|q| {
        let inset = [q.x - 10.0, 30.0 - q.x, q.y - 10.0, 30.0 - q.y]
            .into_iter()
            .fold(f64::INFINITY, f64::min);
        (inset - 0.05).abs() < 0.01
    })
}

/// A stroked frame thinner than the knife's narrowest feature, alone in
/// its path: cut once, along its middle, and nothing says "not cut".
#[test]
fn a_stroked_compound_sliver_is_cut_once_along_its_middle() {
    let (pieces, p, c) = compound(FRAME_D, Some(0.2));
    assert_eq!(pieces.len(), 1, "{pieces:?}");
    assert!(
        closed(&pieces[0]) && on_frame_middle(&pieces[0]),
        "{pieces:?}"
    );
    assert_eq!(p["ready"], true, "{p}");
    let text = p["warnings"].to_string();
    assert!(!text.contains("not cut"), "{text}");
    assert!(!text.contains("no cut paths"), "{text}");
    assert_eq!(codes(&c), Vec::<String>::new(), "{c}");
}

/// The same frame beside a square in the same path: the square's ring and
/// the frame's middle, never the frame's two edges (the tear case).
#[test]
fn a_stroked_compound_sliver_beside_a_shape_is_not_cut_twice() {
    let d = format!("{FRAME_D} M40 10 H60 V30 H40 Z");
    let (pieces, p, c) = compound(&d, Some(0.2));
    assert_eq!(pieces.len(), 2, "{pieces:?}");
    assert!(pieces.iter().all(closed), "no open pieces: {pieces:?}");
    let frame: Vec<_> = pieces
        .iter()
        .filter(|q| kurbo::Shape::bounding_box(*q).x1 < 35.0)
        .collect();
    assert_eq!(frame.len(), 1, "{pieces:?}");
    assert!(on_frame_middle(frame[0]), "{frame:?}");
    let text = p["warnings"].to_string();
    assert!(!text.contains("not cut"), "{text}");
    assert_eq!(codes(&c), Vec::<String>::new(), "{c}");
}

/// A 0.6 mm stroke under the Auto threshold: the frame is cut once along
/// its middle, and the report says exactly that.
#[test]
fn a_wide_stroked_compound_sliver_is_cut_once_and_says_so() {
    let (pieces, p, c) = compound(FRAME_D, Some(0.6));
    assert_eq!(pieces.len(), 1, "{pieces:?}");
    assert!(on_frame_middle(&pieces[0]), "{pieces:?}");
    assert!(!p["warnings"].to_string().contains("not cut"), "{p}");
    assert_eq!(codes(&c), ["strokes_cut_once"], "{c}");
}

/// Without a stroke the frame is a sliver: dropped, and said so.
#[test]
fn a_fill_only_compound_sliver_is_dropped_with_the_warning() {
    let d = format!("{FRAME_D} M40 10 H60 V30 H40 Z");
    let (pieces, p, c) = compound(&d, None);
    assert_eq!(pieces.len(), 1, "only the square: {pieces:?}");
    assert!(
        p["warnings"]
            .to_string()
            .contains("'frame': 1 filled sliver is thinner than 0.125 mm and is not cut."),
        "{p}"
    );
    assert_eq!(codes(&c), ["thin_slivers"], "{c}");
}

/// A fill-only frame of two wide subpaths, 0.05 mm wide and reaching far
/// off the mat, beside a square in the same path: the frame is a sliver
/// that is not cut, so no box counts it, not the report's, not align's.
#[test]
fn a_compound_sliver_off_the_mat_takes_no_space() {
    let d = dirs();
    let mut s = session(&d);
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="500mm" height="500mm" viewBox="0 0 500 500"><path fill-rule="evenodd" d="M20 20 H40 V40 H20 Z M20 60 H420 V70 H20 Z M20.05 60.05 V69.95 H419.95 V60.05 Z"/></svg>"#;
    let id = run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg, "name": "square and frame" }),
    )["id"]
        .as_u64()
        .unwrap();
    let c = run(&mut s, "document.check", json!({}));
    assert_eq!(codes(&c), ["thin_slivers"], "{c}");
    let box_of = |s: &mut Session| {
        let st = run(s, "document.state", json!({}));
        let b = st["objects"][0]["bounds_mm"].clone();
        (
            b["x0"].as_f64().unwrap(),
            b["x1"].as_f64().unwrap() - b["x0"].as_f64().unwrap(),
            b["y1"].as_f64().unwrap() - b["y0"].as_f64().unwrap(),
        )
    };
    let (_, w, h) = box_of(&mut s);
    assert!(
        (w - 20.0).abs() < 0.01 && (h - 20.0).abs() < 0.01,
        "{w} {h}"
    );
    assert_eq!(run(&mut s, "job.preview", json!({}))["ready"], true);
    let st = run(&mut s, "document.state", json!({}));
    let x0 = st["cutting_area_mm"]["x1"].as_f64().unwrap();
    run(
        &mut s,
        "arrange.align",
        json!({ "ids": [id], "mode": "right", "relative_to": "cutting_area" }),
    );
    let (left, w, _) = box_of(&mut s);
    assert!(
        (left + w - x0).abs() < 1e-6,
        "the square's right edge on the area's"
    );
    let c = run(&mut s, "document.check", json!({}));
    assert_eq!(codes(&c), ["thin_slivers"], "{c}");
}

/// A command's box and the cut geometry's box are measured with the same
/// knife: the active plotter's tolerance and narrowest feature, so both
/// are equal, also for a plotter with a coarser tolerance.
#[test]
fn the_command_box_is_the_cut_geometry_box() {
    let d = dirs();
    user_plotter(d.config.path(), "flatten_tolerance_mm = 0.05\n");
    let mut s = session(&d);
    run(&mut s, "job.set_plotter", json!({ "id": "custom" }));
    // A curve and a sliver in one path: the box is the curve's pieces,
    // flattened with the plotter's tolerance.
    let id = import(
        &mut s,
        &svg(r#"<path d="M30 50 C40 13 63 21 70 50 Z M10 90 H90 V90.1 H10 Z"/>"#),
        "curve and sliver",
    );
    let cached = s
        .cut_geometry()
        .unwrap()
        .bounds(flatcraft_core::document::ObjectId(id))
        .unwrap();
    let st = run(&mut s, "document.state", json!({}));
    let b = &st["objects"][0]["bounds_mm"];
    for (got, want) in [
        (b["x0"].as_f64().unwrap(), cached.x0),
        (b["y0"].as_f64().unwrap(), cached.y0),
        (b["x1"].as_f64().unwrap(), cached.x1),
        (b["y1"].as_f64().unwrap(), cached.y1),
    ] {
        assert!((got - want).abs() < 1e-9, "{b} {cached:?}");
    }
    assert!(
        cached.width() < 40.0,
        "the sliver does not count: {cached:?}"
    );
}
