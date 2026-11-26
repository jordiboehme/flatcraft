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
