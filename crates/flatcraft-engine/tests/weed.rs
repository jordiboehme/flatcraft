//! Weed box and weed lines (Task 51) through the engine: the command, the
//! document state, the job listing, the project file and the export.

use flatcraft_device::ProfileStore;
use flatcraft_engine::export;
use flatcraft_engine::job::{self, JobInput};
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{CommandError, Origin, Session};
use flatcraft_path::ToolpathKind;
use kurbo::Point;
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

/// Two labels with a letter O in one of them.
const LABELS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><path fill-rule="evenodd" d="M 20 20 H 80 V 60 H 20 Z M 30 30 H 70 V 50 H 30 Z"/><rect x="100" y="20" width="60" height="40"/><circle cx="60" cy="120" r="20"/></svg>"#;

fn with_labels(s: &mut Session) {
    run(
        s,
        "object.import_svg",
        json!({ "svg": LABELS, "name": "labels" }),
    );
}

fn weeding_on(s: &mut Session) -> Value {
    run(
        s,
        "job.set_weeding",
        json!({ "box": true, "margin_mm": 4, "lines": "both", "spacing_mm": 10 }),
    )
}

#[test]
fn set_weeding_is_one_undo_step_and_shows_in_the_state() {
    let d = dirs();
    let mut s = session(&d);
    with_labels(&mut s);
    let state = run(&mut s, "document.state", json!({}));
    assert_eq!(
        state["job"]["weed"],
        json!({ "box": false, "margin_mm": 3.0, "per_object": false, "lines": "off", "spacing_mm": 20.0 })
    );
    assert_eq!(state["job"]["strategy"], "quality");
    let reply = weeding_on(&mut s);
    assert_eq!(
        reply,
        json!({ "box": true, "margin_mm": 4.0, "per_object": false, "lines": "both", "spacing_mm": 10.0 })
    );
    // Left-out fields stay.
    let reply = run(&mut s, "job.set_weeding", json!({ "per_object": true }));
    assert_eq!(reply["lines"], "both");
    assert_eq!(reply["per_object"], true);
    run(&mut s, "edit.undo", json!({}));
    let state = run(&mut s, "document.state", json!({}));
    assert_eq!(state["job"]["weed"]["per_object"], false);
    assert_eq!(state["job"]["weed"]["box"], true);
}

#[test]
fn set_weeding_refuses_values_out_of_range() {
    let d = dirs();
    let mut s = session(&d);
    for params in [
        json!({ "margin_mm": 0.5 }),
        json!({ "margin_mm": 51 }),
        json!({ "spacing_mm": 4 }),
        json!({ "spacing_mm": 301 }),
        json!({ "lines": "diagonal" }),
        json!({ "colour": "red" }),
    ] {
        let r = s.execute("job.set_weeding", params.clone(), Origin::Agent);
        assert!(
            matches!(r, Err(CommandError::InvalidParams(_))),
            "{params}: {r:?}"
        );
    }
    let state = run(&mut s, "document.state", json!({}));
    assert_eq!(state["job"]["weed"]["margin_mm"], 3.0);
}

#[test]
fn the_listing_names_weed_paths_after_the_design() {
    let d = dirs();
    let mut s = session(&d);
    with_labels(&mut s);
    let before = job::build(&s).unwrap();
    assert!(
        before
            .plan
            .toolpaths
            .iter()
            .all(|t| t.kind == ToolpathKind::Cut)
    );
    weeding_on(&mut s);
    let built = job::build(&s).unwrap();
    let kinds: Vec<ToolpathKind> = built.plan.toolpaths.iter().map(|t| t.kind).collect();
    let first_weed = kinds.iter().position(|k| *k == ToolpathKind::Weed).unwrap();
    assert!(kinds[first_weed..].iter().all(|k| *k == ToolpathKind::Weed));
    assert_eq!(first_weed, before.plan.toolpaths.len());
    let preview = run(&mut s, "job.preview", json!({}));
    let notes: Vec<String> = preview["listing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["note"].as_str().unwrap_or("").to_string())
        .filter(|n| !n.is_empty())
        .collect();
    assert!(
        notes.iter().any(|n| n == "weed line 1, pass 1"),
        "{notes:?}"
    );
    // The box is the last path; only the trailer (home) follows it.
    let last_path = notes.iter().rev().find(|n| *n != "home");
    assert_eq!(last_path.map(String::as_str), Some("weed box 1, pass 1"));
    assert!(preview["toolpaths"].as_u64().unwrap() > before.plan.toolpaths.len() as u64);
}

#[test]
fn a_box_that_does_not_fit_warns() {
    let d = dirs();
    let mut s = session(&d);
    with_labels(&mut s);
    weeding_on(&mut s);
    // Push the labels into the top left corner of the cutting area: no
    // room for a box with 1 mm to spare.
    let state = run(&mut s, "document.state", json!({}));
    let id = state["objects"][0]["id"].as_u64().unwrap();
    let area = &state["cutting_area_mm"];
    let (x, y) = (area["x0"].as_f64().unwrap(), area["y0"].as_f64().unwrap());
    run(
        &mut s,
        "object.move",
        json!({ "ids": [id], "x_mm": x + 0.5, "y_mm": y + 0.5 }),
    );
    let built = job::build(&s).unwrap();
    assert!(
        built
            .warnings
            .iter()
            .any(|w| w.starts_with("The weed box does not fit")),
        "{:?}",
        built.warnings
    );
    assert!(
        !built
            .plan
            .toolpaths
            .iter()
            .any(|t| t.kind == ToolpathKind::Weed && t.closed)
    );
}

#[test]
fn weeding_survives_save_and_open() {
    let d = dirs();
    let mut s = session(&d);
    with_labels(&mut s);
    weeding_on(&mut s);
    let path = d.out.path().join("labels.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    let mut s2 = session(&d);
    run(&mut s2, "project.open", json!({ "path": path }));
    let state = run(&mut s2, "document.state", json!({}));
    assert_eq!(state["job"]["weed"]["lines"], "both");
    assert_eq!(state["job"]["weed"]["margin_mm"], 4.0);
}

/// The segments of a shape, each with its ends in a fixed order.
fn segments(points: &[Point], closed: bool) -> Vec<(Point, Point)> {
    let mut pts = points.to_vec();
    if closed && pts.first() != pts.last() {
        pts.push(pts[0]);
    }
    pts.windows(2)
        .filter(|w| w[0].distance(w[1]) > 1e-9)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            if (a.x, a.y) <= (b.x, b.y) {
                (a, b)
            } else {
                (b, a)
            }
        })
        .collect()
}

fn same(mut a: Vec<(Point, Point)>, mut b: Vec<(Point, Point)>) -> bool {
    let key = |s: &(Point, Point)| (s.0.x, s.0.y, s.1.x, s.1.y);
    a.sort_by(|x, y| key(x).partial_cmp(&key(y)).unwrap());
    b.sort_by(|x, y| key(x).partial_cmp(&key(y)).unwrap());
    a.len() == b.len()
        && a.iter()
            .zip(&b)
            .all(|(x, y)| x.0.distance(y.0) < 1e-6 && x.1.distance(y.1) < 1e-6)
}

#[test]
fn the_export_holds_the_weed_paths_the_plan_cuts() {
    let d = dirs();
    let mut s = session(&d);
    with_labels(&mut s);
    let input = JobInput::of(&s).unwrap();
    assert!(
        export::collect(&input, None, false)
            .unwrap()
            .weeds
            .is_empty()
    );
    weeding_on(&mut s);
    let input = JobInput::of(&s).unwrap();
    let doc = export::collect(&input, None, false).unwrap();
    assert!(doc.weeds.iter().any(|w| w.closed));
    assert!(doc.weeds.iter().filter(|w| !w.closed).count() > 3);
    let built = job::build(&s).unwrap();
    let planned: Vec<(Point, Point)> = built
        .plan
        .toolpaths
        .iter()
        .filter(|t| t.kind == ToolpathKind::Weed && t.pass == 1)
        .flat_map(|t| segments(t.lap(), t.closed))
        .collect();
    let exported: Vec<(Point, Point)> = doc
        .weeds
        .iter()
        .flat_map(|w| segments(&w.points, w.closed))
        .collect();
    assert!(same(planned, exported));

    // And in the files: the SVG has its weed group, the DXF its layer.
    let svg = String::from_utf8(export::write(&doc, export::ExportFormat::Svg)).unwrap();
    assert!(svg.contains("<g id=\"weed\""), "{svg}");
    assert_eq!(
        svg.matches("<path ").count(),
        doc.shape_count(),
        "one path per shape"
    );
    let dxf = String::from_utf8(export::write(&doc, export::ExportFormat::Dxf)).unwrap();
    assert!(dxf.contains("  8\nWEED\n"));
}
