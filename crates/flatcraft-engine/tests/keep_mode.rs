//! Keep mode per object (Task 29): what is kept after cutting. Pieces
//! keeps the shapes, Stencil the sheet, Both everything near the cuts.
//! The plan, the weeds and the cut log read it; linked copies and groups
//! share it as the brief says.

use flatcraft_core::document::{KeepMode, ObjectId};
use flatcraft_device::ProfileStore;
use flatcraft_engine::job::{self, JobInput};
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{Origin, Session};
use kurbo::Point;
use serde_json::{Value, json};

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

/// A 50 mm square at (`x`, 20) on a 200 mm page.
fn square(s: &mut Session, x: f64, name: &str) -> u64 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="{x}" y="20" width="50" height="50"/></svg>"#
    );
    run(s, "object.import_svg", json!({ "svg": svg, "name": name }))["id"]
        .as_u64()
        .unwrap()
}

fn keep(s: &Session, id: u64) -> KeepMode {
    s.doc().document.get(ObjectId(id)).unwrap().keep
}

#[test]
fn a_group_id_sets_every_member_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    let b = square(&mut s, 100.0, "b");
    let c = square(&mut s, 20.0, "c");
    let g = run(&mut s, "object.group", json!({ "ids": [a, b] }))["group_id"]
        .as_u64()
        .unwrap();
    let before = s.doc().undo.undo_len();
    let r = run(
        &mut s,
        "object.set_keep",
        json!({ "ids": [g], "keep": "both" }),
    );
    assert_eq!(s.doc().undo.undo_len(), before + 1);
    assert_eq!(r, json!({ "ids": [a, b], "keep": "both" }));
    assert_eq!((keep(&s, a), keep(&s, b)), (KeepMode::Both, KeepMode::Both));
    assert_eq!(keep(&s, c), KeepMode::Pieces);
}

#[test]
fn the_selection_is_the_default_and_bad_values_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    run(&mut s, "object.set_keep", json!({ "keep": "stencil" }));
    assert_eq!(keep(&s, a), KeepMode::Stencil);
    for bad in [
        json!({ "keep": "sheet" }),
        json!({}),
        json!({ "keep": "pieces", "mode": "x" }),
    ] {
        assert!(
            s.execute("object.set_keep", bad.clone(), Origin::User)
                .is_err(),
            "{bad}"
        );
    }
    run(&mut s, "object.select", json!({ "ids": [] }));
    assert!(
        s.execute("object.set_keep", json!({ "keep": "both" }), Origin::User)
            .is_err()
    );
}

#[test]
fn an_agent_sets_and_reads_it_like_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    let state = run(&mut s, "document.state", json!({}));
    assert_eq!(state["objects"][0]["keep"], "pieces");
    let by_agent = s
        .execute(
            "object.set_keep",
            json!({ "ids": [a], "keep": "stencil" }),
            Origin::Agent,
        )
        .unwrap();
    assert_eq!(by_agent, json!({ "ids": [a], "keep": "stencil" }));
    let state = s
        .execute("document.state", json!({}), Origin::Agent)
        .unwrap();
    assert_eq!(state["objects"][0]["keep"], "stencil");
}

#[test]
fn a_project_keeps_it_and_writes_only_what_is_not_pieces() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    let b = square(&mut s, 100.0, "b");
    run(
        &mut s,
        "object.set_keep",
        json!({ "ids": [b], "keep": "stencil" }),
    );
    let doc = s.doc().document.clone();
    let value = flatcraft_core::project::to_value(&doc).unwrap();
    let objects = value["objects"].as_array().unwrap();
    assert_eq!(objects[0].get("keep"), None, "Pieces is not written");
    assert_eq!(objects[1]["keep"], "stencil");
    let path = dir.path().join("keep.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    run(&mut s, "project.open", json!({ "path": path }));
    assert_eq!(keep(&s, a), KeepMode::Pieces);
    assert_eq!(keep(&s, b), KeepMode::Stencil);
}

#[test]
fn the_plan_classifies_by_the_keep_mode_and_replans_when_it_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    let inside = Point::new(45.0, 45.0);
    let outside = Point::new(100.0, 45.0);
    let pieces = JobInput::of(&s).unwrap();
    let plan = job::build_input(&pieces).unwrap().plan;
    assert!(plan.kept.is_kept(inside) && !plan.kept.is_kept(outside));

    run(
        &mut s,
        "object.set_keep",
        json!({ "ids": [a], "keep": "stencil" }),
    );
    let stencil = JobInput::of(&s).unwrap();
    assert_ne!(stencil, pieces, "a keep change is a new job input");
    let plan = job::build_input(&stencil).unwrap().plan;
    assert!(!plan.kept.is_kept(inside) && plan.kept.is_kept(outside));
    // The toolpath's keep side follows: a stencil keeps the outside.
    assert!(
        plan.toolpaths
            .iter()
            .all(|t| t.keep == flatcraft_path::KeepSide::Outside)
    );
}

#[test]
fn a_stencil_keeps_the_sheet_so_nothing_is_weeded() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    square(&mut s, 100.0, "b");
    run(
        &mut s,
        "job.set_weeding",
        json!({ "box": true, "lines": "both", "spacing_mm": 10.0 }),
    );
    let weeds = |s: &Session| {
        let r = job::build(s).unwrap();
        let n = r
            .plan
            .toolpaths
            .iter()
            .filter(|t| t.kind == flatcraft_path::ToolpathKind::Weed)
            .count();
        (n, r.plan.blocked_weed_boxes)
    };
    let (n, blocked) = weeds(&s);
    assert!(n > 1 && blocked == 0, "pieces are weeded: {n} {blocked}");
    // One stencil keeps the whole sheet (kept wins), so no weed path
    // may cut anywhere outside its shapes.
    run(
        &mut s,
        "object.set_keep",
        json!({ "ids": [a], "keep": "stencil" }),
    );
    assert_eq!(weeds(&s), (0, 1));
}
