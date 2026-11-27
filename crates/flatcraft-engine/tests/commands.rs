//! Command registry tests on a headless session.

use flatcraft_core::document::ObjectId;
use flatcraft_core::geometry::bounds;
use flatcraft_device::ProfileStore;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::registry;
use flatcraft_engine::session::{CommandError, Origin, Session};
use kurbo::{PathEl, Point, Rect};
use serde_json::{Value, json};

const EPS: f64 = 1e-9;

fn session(dir: &tempfile::TempDir) -> Session {
    let profiles = ProfileStore::load(dir.path()).unwrap();
    // Never the real Documents folder.
    let prefs = Preferences {
        output_dir: Some(dir.path().join("output")),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, false, dir.path().to_path_buf())
}

fn rect_svg(w: f64, h: f64) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="20" y="20" width="{w}" height="{h}"/></svg>"#
    )
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

fn import(s: &mut Session, w: f64, h: f64) -> u64 {
    let r = run(
        s,
        "object.import_svg",
        json!({ "svg": rect_svg(w, h), "name": "rect" }),
    );
    r["id"].as_u64().unwrap()
}

fn placed_bounds(s: &Session, id: u64) -> Rect {
    let o = s.doc().document.get(ObjectId(id)).unwrap();
    bounds(&o.placed_paths()).unwrap()
}

/// usvg works in f32, so imported coordinates carry about 1e-6 mm of noise.
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn import_then_state_reports_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 50.0, 50.0);
    let state = run(&mut s, "document.state", json!({}));
    let objects = state["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 1);
    let o = &objects[0];
    assert_eq!(o["id"].as_u64(), Some(id));
    assert_eq!(o["name"], "rect");
    assert_eq!(o["kind"], "svg");
    assert_eq!(o["visible"], true);
    let b = &o["bounds_mm"];
    let w = b["x1"].as_f64().unwrap() - b["x0"].as_f64().unwrap();
    assert!(close(w, 50.0), "{b}");
    assert!(state["cutting_area_mm"].is_object(), "{state}");
    assert_eq!(state["dirty"], true);
    assert!(state["path"].is_null());
    assert_eq!(state["job"]["plotter_id"], "file-output-gpgl-cameo2");
}

#[test]
fn import_file_by_extension() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let svg = dir.path().join("Badge.SVG");
    std::fs::write(&svg, rect_svg(30.0, 30.0)).unwrap();
    let r = run(&mut s, "object.import", json!({ "path": svg }));
    let id = r["id"].as_u64().unwrap();
    assert_eq!(s.doc().document.get(ObjectId(id)).unwrap().name, "Badge");

    let bmp = dir.path().join("photo.bmp");
    std::fs::write(&bmp, b"not really").unwrap();
    let e = s
        .execute("object.import", json!({ "path": bmp }), Origin::User)
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
    assert!(e.to_string().contains(".svg"), "{e}");
}

#[test]
fn move_scale_rotate_mirror_update_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let sq = import(&mut s, 50.0, 50.0);

    run(
        &mut s,
        "object.move",
        json!({ "ids": [sq], "dx_mm": 10.0, "dy_mm": -5.0 }),
    );
    let b = placed_bounds(&s, sq);
    assert!(close(b.x0, 30.0) && close(b.y0, 15.0), "{b:?}");

    run(
        &mut s,
        "object.move",
        json!({ "ids": [sq], "center_mm": [100.0, 100.0] }),
    );
    let b = placed_bounds(&s, sq);
    assert!(close(b.center().x, 100.0) && close(b.center().y, 100.0));

    run(
        &mut s,
        "object.scale",
        json!({ "ids": [sq], "width_mm": 100.0, "keep_aspect": true }),
    );
    let b = placed_bounds(&s, sq);
    assert!(close(b.width(), 100.0) && close(b.height(), 100.0), "{b:?}");
    assert!(close(b.center().x, 100.0), "center anchor stays: {b:?}");

    // Anchor top_left keeps the top-left corner.
    let before = placed_bounds(&s, sq);
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [sq], "factor": 0.5, "anchor": "top_left" }),
    );
    let b = placed_bounds(&s, sq);
    assert!(close(b.x0, before.x0) && close(b.y0, before.y0), "{b:?}");
    assert!(close(b.width(), 50.0));

    // Non-uniform: only height, keep_aspect false.
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [sq], "height_mm": 20.0, "keep_aspect": false, "anchor": "bottom_right" }),
    );
    let b2 = placed_bounds(&s, sq);
    assert!(
        close(b2.width(), 50.0) && close(b2.height(), 20.0),
        "{b2:?}"
    );
    assert!(close(b2.x1, b.x1) && close(b2.y1, b.y1), "{b2:?}");

    let r = import(&mut s, 20.0, 10.0);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [r], "degrees": 90.0 }),
    );
    let b = placed_bounds(&s, r);
    assert!(close(b.width(), 10.0) && close(b.height(), 20.0), "{b:?}");
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [r], "degrees": -180.0 }),
    );
    let t = s.doc().document.get(ObjectId(r)).unwrap().transform;
    assert!(close(t.rotation_deg, 270.0), "{t:?}");
}

fn pts(els: &[PathEl]) -> Vec<Point> {
    els.iter()
        .flat_map(|e| match *e {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => vec![p],
            PathEl::QuadTo(a, b) => vec![a, b],
            PathEl::CurveTo(a, b, d) => vec![a, b, d],
            PathEl::ClosePath => vec![],
        })
        .collect()
}

#[test]
fn mirror_at_30_degrees() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 20.0, 10.0);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [id], "degrees": 30.0 }),
    );
    let obj = s.doc().document.get(ObjectId(id)).unwrap().clone();
    let c = obj.transform.center_mm;
    let before = obj.placed_paths();

    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [id], "axis": "horizontal" }),
    );
    let obj2 = s.doc().document.get(ObjectId(id)).unwrap();
    assert!((obj2.transform.rotation_deg - 330.0).abs() < EPS);
    assert!(obj2.transform.mirror != obj.transform.mirror);
    let after = obj2.placed_paths();

    let reflect = |p: Point| Point::new(2.0 * c.x - p.x, p.y);
    assert_eq!(before.len(), after.len());
    for (b, a) in before.iter().zip(&after) {
        let pb = pts(b.path.elements());
        let pa = pts(a.path.elements());
        assert_eq!(pb.len(), pa.len());
        for (p, q) in pb.iter().zip(&pa) {
            let r = reflect(*p);
            assert!(
                (r.x - q.x).abs() < EPS && (r.y - q.y).abs() < EPS,
                "{r:?} vs {q:?}"
            );
        }
    }

    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [id], "axis": "vertical" }),
    );
    let obj3 = s.doc().document.get(ObjectId(id)).unwrap();
    let t = obj3.transform;
    // 180 - 330 = -150 -> 210.
    assert!((t.rotation_deg - 210.0).abs() < EPS, "{t:?}");
    assert_eq!(t.mirror, obj.transform.mirror);
    // Vertical: the outline is the previous one reflected across the
    // horizontal line through the centre.
    let c2 = obj3.transform.center_mm;
    let flipped = obj3.placed_paths();
    assert_eq!(after.len(), flipped.len());
    for (b, a) in after.iter().zip(&flipped) {
        let pb = pts(b.path.elements());
        let pa = pts(a.path.elements());
        assert_eq!(pb.len(), pa.len());
        for (p, q) in pb.iter().zip(&pa) {
            let r = Point::new(p.x, 2.0 * c2.y - p.y);
            assert!(
                (r.x - q.x).abs() < EPS && (r.y - q.y).abs() < EPS,
                "{r:?} vs {q:?}"
            );
        }
    }
}

#[test]
fn scale_swaps_axes_at_90_and_refuses_shear() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 20.0, 10.0);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [id], "degrees": 90.0 }),
    );
    // Placed box is 10 wide, 20 high. Make it 40 wide, keep the height.
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [id], "width_mm": 40.0, "keep_aspect": false }),
    );
    let b = placed_bounds(&s, id);
    assert!(close(b.width(), 40.0) && close(b.height(), 20.0), "{b:?}");
    let t = s.doc().document.get(ObjectId(id)).unwrap().transform;
    assert!(close(t.scale_x, 1.0) && close(t.scale_y, 4.0), "{t:?}");

    let other = import(&mut s, 20.0, 10.0);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [other], "degrees": 30.0 }),
    );
    let before = s.doc().document.get(ObjectId(other)).unwrap().transform;
    let e = s
        .execute(
            "object.scale",
            json!({ "ids": [other], "width_mm": 40.0, "keep_aspect": false }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
    assert_eq!(
        s.doc().document.get(ObjectId(other)).unwrap().transform,
        before
    );
}

#[test]
fn job_arrange_and_visibility_are_undoable() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 20.0, 20.0);
    s.doc_mut().dirty = false;

    run(&mut s, "job.set_material", json!({ "id": "vinyl" }));
    assert!(s.doc().dirty);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().document.job.material_id, "copy-paper");

    s.doc_mut().dirty = false;
    let before = placed_bounds(&s, id);
    run(
        &mut s,
        "arrange.align",
        json!({ "ids": [id], "mode": "right", "relative_to": "cutting_area" }),
    );
    assert!(s.doc().dirty);
    assert!(close(placed_bounds(&s, id).x1, 299.8));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(placed_bounds(&s, id), before);

    s.doc_mut().dirty = false;
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [id], "visible": false }),
    );
    assert!(s.doc().dirty);
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().document.get(ObjectId(id)).unwrap().visible);
}

#[test]
fn set_cut_skips_several_objects_and_undoes_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = import(&mut s, 50.0, 50.0);
    let b = import(&mut s, 50.0, 50.0);
    s.doc_mut().dirty = false;
    // An agent may use it.
    let r = s
        .execute(
            "object.set_cut",
            json!({ "ids": [a, b], "cut": false }),
            Origin::Agent,
        )
        .unwrap();
    assert_eq!(r["cut"], false);
    assert!(s.doc().dirty);
    let cut = |s: &Session, id: u64| s.doc().document.get(ObjectId(id)).unwrap().cut_enabled;
    assert!(!cut(&s, a) && !cut(&s, b));
    // Still visible, and document.state says so.
    let state = run(&mut s, "document.state", json!({}));
    assert_eq!(state["objects"][0]["visible"], true);
    assert_eq!(state["objects"][0]["cut_enabled"], false);
    run(&mut s, "edit.undo", json!({}));
    assert!(cut(&s, a) && cut(&s, b));
    // An empty list means the selection.
    run(&mut s, "object.select", json!({ "ids": [b] }));
    run(&mut s, "object.set_cut", json!({ "ids": [], "cut": false }));
    assert!(cut(&s, a) && !cut(&s, b));
    let e = s
        .execute(
            "object.set_cut",
            json!({ "ids": [999], "cut": false }),
            Origin::Agent,
        )
        .unwrap_err();
    assert!(!e.to_string().contains('\u{2014}'));
}

#[test]
fn rename_changes_one_name_and_undoes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = import(&mut s, 50.0, 50.0);
    let b = import(&mut s, 50.0, 50.0);
    let name = |s: &Session, id: u64| s.doc().document.get(ObjectId(id)).unwrap().name.clone();
    let old = name(&s, b);
    let r = s
        .execute(
            "object.rename",
            json!({ "id": a, "name": "  Lid  " }),
            Origin::Agent,
        )
        .unwrap();
    assert_eq!(r["name"], "Lid");
    assert_eq!(name(&s, a), "Lid");
    assert_eq!(name(&s, b), old);
    run(&mut s, "edit.undo", json!({}));
    assert_ne!(name(&s, a), "Lid");
    for bad in ["", "   ", "two\nlines"] {
        let e = s
            .execute(
                "object.rename",
                json!({ "id": a, "name": bad }),
                Origin::Agent,
            )
            .unwrap_err();
        assert!(!e.to_string().contains('\u{2014}'));
    }
    // The cap is in characters, not bytes.
    let long = "\u{e4}".repeat(200);
    run(&mut s, "object.rename", json!({ "id": a, "name": long }));
    let too_long = "x".repeat(201);
    assert!(
        s.execute(
            "object.rename",
            json!({ "id": a, "name": too_long }),
            Origin::Agent
        )
        .is_err()
    );
    // The same name again is no undo step.
    let steps = s.doc().undo.undo_len();
    run(&mut s, "object.rename", json!({ "id": a, "name": long }));
    run(
        &mut s,
        "object.rename",
        json!({ "id": a, "name": format!(" {long} ") }),
    );
    assert_eq!(s.doc().undo.undo_len(), steps);
    assert!(
        s.execute(
            "object.rename",
            json!({ "id": 999, "name": "x" }),
            Origin::Agent
        )
        .is_err()
    );
}

#[test]
fn import_svgz_file() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/svg/mm.svg");
    let plain = std::fs::read(fixture).unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&plain).unwrap();
    let path = dir.path().join("mm.SVGZ");
    std::fs::write(&path, gz.finish().unwrap()).unwrap();
    let r = run(&mut s, "object.import", json!({ "path": path }));
    let id = r["id"].as_u64().unwrap();
    let b = placed_bounds(&s, id);
    assert!(close(b.width(), 50.0) && close(b.height(), 20.0), "{b:?}");
    assert_eq!(s.doc().document.get(ObjectId(id)).unwrap().name, "mm");
}

#[test]
fn error_texts_have_no_em_dash() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 20.0, 10.0);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [id], "degrees": 30.0 }),
    );
    run(&mut s, "object.select", json!({ "ids": [] }));
    let png = dir.path().join("a.png");
    let cases = [
        ("object.teleport", json!({})),
        ("document.state", json!({ "x": 1 })),
        ("object.move", json!({ "ids": [999], "dx_mm": 1.0 })),
        ("object.move", json!({ "ids": [id] })),
        (
            "object.move",
            json!({ "ids": [id], "dx_mm": 1.0, "center_mm": [0.0, 0.0] }),
        ),
        ("object.delete", json!({ "ids": [] })),
        ("object.scale", json!({ "ids": [id] })),
        ("object.scale", json!({ "ids": [id], "factor": -1.0 })),
        (
            "object.scale",
            json!({ "ids": [id], "height_mm": 5.0, "keep_aspect": false }),
        ),
        ("object.import", json!({ "path": png })),
        (
            "object.import",
            json!({ "path": dir.path().join("missing.svg") }),
        ),
        ("object.import_svg", json!({ "svg": "<svg", "name": "x" })),
        ("document.switch", json!({ "index": 9 })),
        ("document.close", json!({ "index": 0 })),
        ("project.save", json!({})),
        (
            "project.save",
            json!({ "path": dir.path().join("a.flatcraft") }),
        ),
        (
            "project.open",
            json!({ "path": dir.path().join("nope.flatcraft") }),
        ),
        ("edit.redo", json!({})),
        ("profiles.get", json!({ "id": "nope" })),
        ("job.set_media", json!({ "id": "a4" })),
        ("job.set_overrides", json!({ "speed": 11 })),
        ("job.set_overrides", json!({ "force": 0 })),
        ("job.set_overrides", json!({ "passes": 0 })),
        ("arrange.auto", json!({ "spacing_mm": -1.0 })),
        (
            "arrange.distribute",
            json!({ "ids": [id], "axis": "horizontal" }),
        ),
        ("arrange.distribute", json!({ "axis": "vertical" })),
        ("object.copies", json!({ "id": id, "count": 0 })),
        (
            "object.copies",
            json!({ "id": id, "count": 2, "gap_mm": -5.0 }),
        ),
        ("object.link", json!({ "ids": [id] })),
    ];
    for (cmd, params) in cases {
        let e = s
            .execute(cmd, params.clone(), Origin::Agent)
            .expect_err(&format!("{cmd} {params} should fail"));
        let text = e.to_string();
        assert!(!text.contains('\u{2014}'), "em-dash in {cmd}: {text}");
        assert!(!text.trim().is_empty());
    }
}

#[test]
fn undo_restores_previous_transform() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 50.0, 50.0);
    let before = s.doc().document.get(ObjectId(id)).unwrap().transform;
    run(
        &mut s,
        "object.move",
        json!({ "ids": [id], "dx_mm": 7.0, "dy_mm": 3.0 }),
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(
        s.doc().document.get(ObjectId(id)).unwrap().transform,
        before
    );
    // Undo the import too, then nothing is left to undo.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().document.objects.is_empty());
    let e = s.execute("edit.undo", json!({}), Origin::User).unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "{e}");
}

#[test]
fn redo_reapplies() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 50.0, 50.0);
    run(
        &mut s,
        "object.move",
        json!({ "ids": [id], "dx_mm": 7.0, "dy_mm": 3.0 }),
    );
    let moved = s.doc().document.get(ObjectId(id)).unwrap().transform;
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc().document.get(ObjectId(id)).unwrap().transform, moved);
    assert!(
        !s.doc()
            .document
            .get(ObjectId(id))
            .unwrap()
            .geometry
            .is_empty(),
        "redo keeps geometry"
    );
}

#[test]
fn failed_command_leaves_no_trace() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 50.0, 50.0);
    let before = s.doc().document.get(ObjectId(id)).unwrap().transform;
    // Second id is unknown: nothing may move, and no undo entry is added.
    let e = s
        .execute(
            "object.move",
            json!({ "ids": [id, 999], "dx_mm": 5.0 }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::NotFound(_)), "{e}");
    assert!(e.to_string().contains("999"), "{e}");
    assert_eq!(
        s.doc().document.get(ObjectId(id)).unwrap().transform,
        before
    );
    run(&mut s, "edit.undo", json!({}));
    assert!(
        s.doc().document.objects.is_empty(),
        "undo removed the import"
    );
}

#[test]
fn unknown_fields_are_invalid_params() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let e = s
        .execute("document.state", json!({ "bogus": 1 }), Origin::Agent)
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
}

#[test]
fn empty_ids_use_selection() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = import(&mut s, 10.0, 10.0);
    let b = import(&mut s, 10.0, 10.0);
    run(&mut s, "object.select", json!({ "ids": [b] }));
    let ba = placed_bounds(&s, a);
    run(&mut s, "object.move", json!({ "ids": [], "dx_mm": 5.0 }));
    assert_eq!(placed_bounds(&s, a), ba);
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [], "visible": false }),
    );
    assert!(!s.doc().document.get(ObjectId(b)).unwrap().visible);
    run(&mut s, "object.select", json!({ "ids": [] }));
    let e = s
        .execute("object.delete", json!({ "ids": [] }), Origin::User)
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
    run(&mut s, "object.delete", json!({ "ids": [a] }));
    assert!(s.doc().document.get(ObjectId(a)).is_none());
}

#[test]
fn duplicate_offsets_and_new_ids() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 20.0, 20.0);
    let r = run(&mut s, "object.duplicate", json!({ "ids": [id] }));
    let new_ids: Vec<u64> = r["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert_eq!(new_ids.len(), 1);
    assert_ne!(new_ids[0], id);
    let a = placed_bounds(&s, id);
    let b = placed_bounds(&s, new_ids[0]);
    assert!(close(b.x0 - a.x0, 10.0) && close(b.y0 - a.y0, 10.0));
    let r = run(
        &mut s,
        "object.duplicate",
        json!({ "ids": [id], "offset_mm": [0.0, 30.0] }),
    );
    let c = r["ids"][0].as_u64().unwrap();
    assert!(c != id && c != new_ids[0]);
    assert!(close(placed_bounds(&s, c).y0 - a.y0, 30.0));
}

#[test]
fn align_left_to_cutting_area() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = import(&mut s, 20.0, 20.0);
    let b = import(&mut s, 40.0, 10.0);
    run(&mut s, "object.move", json!({ "ids": [b], "dx_mm": 60.0 }));
    run(
        &mut s,
        "arrange.align",
        json!({ "ids": [a, b], "mode": "left", "relative_to": "cutting_area" }),
    );
    assert!(close(placed_bounds(&s, a).x0, 5.0));
    assert!(close(placed_bounds(&s, b).x0, 5.0));

    run(&mut s, "object.move", json!({ "ids": [b], "dx_mm": 60.0 }));
    run(
        &mut s,
        "arrange.align",
        json!({ "ids": [a, b], "mode": "right", "relative_to": "selection" }),
    );
    assert!(close(placed_bounds(&s, a).x1, placed_bounds(&s, b).x1));
}

/// A user plotter whose 12x12 mat is calibrated: grid origin 10/12, grid 200 x 150 mm.
fn calibrated_session(dir: &tempfile::TempDir) -> Session {
    let d = dir.path().join("plotters");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("grid.toml"),
        "id = \"grid\"\nname = \"Grid\"\ninherits = \"file-output-gpgl-cameo2\"\n\
         media = [{ id = \"mat-12x12\", name = \"My mat\", kind = \"Mat\", width_mm = 304.8, \
         height_mm = 330.2, calibrated = true, \
         grid = { origin_x_mm = 10.0, origin_y_mm = 12.0, width_mm = 200.0, height_mm = 150.0 } }]\n",
    )
    .unwrap();
    let mut s = session(dir);
    run(&mut s, "job.set_plotter", json!({ "id": "grid" }));
    s
}

#[test]
fn align_left_uses_grid_area() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = calibrated_session(&dir);
    let a = import(&mut s, 20.0, 20.0);
    run(&mut s, "object.move", json!({ "ids": [a], "dx_mm": 60.0 }));
    run(
        &mut s,
        "arrange.align",
        json!({ "ids": [a], "mode": "left", "relative_to": "cutting_area" }),
    );
    // calibrated: the grid's 0,0 is the left edge, no safety margin
    assert!(close(placed_bounds(&s, a).x0, 0.0));
    run(
        &mut s,
        "arrange.align",
        json!({ "ids": [a], "mode": "right", "relative_to": "cutting_area" }),
    );
    assert!(close(placed_bounds(&s, a).x1, 200.0));
    let state = run(&mut s, "document.state", json!({}));
    assert_eq!(state["cutting_area_mm"]["y1"].as_f64(), Some(150.0));
}

#[test]
fn job_adds_the_grid_origin() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = calibrated_session(&dir);
    let a = import(&mut s, 20.0, 20.0);
    run(
        &mut s,
        "arrange.align",
        json!({ "ids": [a], "mode": "left", "relative_to": "cutting_area" }),
    );
    run(
        &mut s,
        "arrange.align",
        json!({ "ids": [a], "mode": "top", "relative_to": "cutting_area" }),
    );
    let built = flatcraft_engine::job::build(&s).unwrap();
    let text = String::from_utf8_lossy(&built.job.bytes).into_owned();
    // the 20 mm square's corner app (0,0) = device y 12 mm, x 10 mm = M240,200 units (y first),
    // the corner nearest the home, cut counter-clockwise (down the left side
    // first) and closed with the 0.5 mm overcut; Z is the physical mat
    assert!(
        text.contains("M240,200\x03D640,200,640,600,240,600,240,200,250,200\x03"),
        "{text}"
    );
    assert!(text.contains("Z6604,6096"), "{text}");
}

#[test]
fn document_state_reports_mat() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let state = run(&mut s, "document.state", json!({}));
    let m = &state["mat"];
    assert_eq!(m["id"], "mat-12x12");
    assert_eq!(m["name"], "Mat 12x12");
    assert_eq!(m["kind"], "mat");
    assert_eq!(m["width_mm"].as_f64(), Some(304.8));
    assert_eq!(m["height_mm"].as_f64(), Some(330.2));
    assert_eq!(m["calibrated"], false);
    assert_eq!(m["color"], "#FBF4E4");
    assert_eq!(m["grid"]["origin_x_mm"].as_f64(), Some(0.0));
    assert_eq!(m["grid"]["origin_y_mm"].as_f64(), Some(0.0));
    assert_eq!(m["grid"]["width_mm"].as_f64(), Some(304.8));
    assert_eq!(m["grid"]["height_mm"].as_f64(), Some(304.8));
    run(&mut s, "job.set_media", json!({ "id": "no-mat" }));
    let state = run(&mut s, "document.state", json!({}));
    assert_eq!(state["mat"]["kind"], "no_mat");
}

#[test]
fn auto_arrange_packs_without_overlap() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let sizes = [
        (80.0, 40.0),
        (120.0, 60.0),
        (30.0, 90.0),
        (60.0, 60.0),
        (100.0, 20.0),
        (45.0, 70.0),
    ];
    let ids: Vec<u64> = sizes.iter().map(|&(w, h)| import(&mut s, w, h)).collect();
    // An import selects the new object; with nothing selected all are packed.
    run(&mut s, "object.select", json!({ "ids": [] }));
    let r = run(&mut s, "arrange.auto", json!({}));
    assert_eq!(r["placed"].as_array().unwrap().len(), 6, "{r}");
    assert!(r["did_not_fit"].as_array().unwrap().is_empty(), "{r}");
    let area = Rect::new(5.0, 5.0, 299.8, 299.8);
    let rects: Vec<Rect> = ids.iter().map(|&i| placed_bounds(&s, i)).collect();
    for (i, a) in rects.iter().enumerate() {
        assert!(
            a.x0 >= area.x0 - 1e-6
                && a.y0 >= area.y0 - 1e-6
                && a.x1 <= area.x1 + 1e-6
                && a.y1 <= area.y1 + 1e-6,
            "{a:?} outside"
        );
        for b in &rects[i + 1..] {
            let o = a.intersect(*b);
            assert!(
                o.width() <= 1e-6 || o.height() <= 1e-6,
                "{a:?} overlaps {b:?}"
            );
        }
    }
    // Wider than the cutting area: does not fit and stays where it is.
    let big = import(&mut s, 300.0, 100.0);
    let before = placed_bounds(&s, big);
    run(&mut s, "object.select", json!({ "ids": [] }));
    let r = run(&mut s, "arrange.auto", json!({ "spacing_mm": 5.0 }));
    assert_eq!(r["did_not_fit"], json!([big]), "{r}");
    assert_eq!(placed_bounds(&s, big), before);
}

#[test]
fn project_roundtrip_rebuilds_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 50.0, 25.0);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [id], "degrees": 45.0 }),
    );
    let b = placed_bounds(&s, id);
    let path = dir.path().join("work").join("p.flatcraft");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    run(&mut s, "project.save", json!({ "path": path }));
    assert!(!s.doc().dirty);
    // Save again without a path uses the remembered one.
    run(&mut s, "project.save", json!({}));

    let mut s2 = session(&dir);
    let r = run(&mut s2, "project.open", json!({ "path": path }));
    assert!(r["warnings"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(s2.doc().path.as_deref(), Some(path.as_path()));
    assert!(!s2.doc().dirty);
    let b2 = placed_bounds(&s2, id);
    assert!(
        close(b.x0, b2.x0) && close(b.y1, b2.y1) && close(b.width(), b2.width()),
        "{b:?} vs {b2:?}"
    );
}

#[test]
fn save_without_any_path_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let e = s
        .execute("project.save", json!({}), Origin::User)
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
}

#[test]
fn agent_cannot_save_into_config_dir() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let inside = dir.path().join("sneaky.flatcraft");
    let e = s
        .execute("project.save", json!({ "path": inside }), Origin::Agent)
        .unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "{e}");
    assert!(!inside.exists());
    let nested = dir
        .path()
        .join("plotters")
        .join("..")
        .join("preferences.toml");
    let e = s
        .execute("project.save", json!({ "path": nested }), Origin::Agent)
        .unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "{e}");
    // The user may.
    run(&mut s, "project.save", json!({ "path": inside }));
    assert!(inside.exists());
}

/// Saves follow a link at the target path to the file it names, so an
/// agent's save through a link into the config dir must be refused, also
/// when the link points to a file that does not exist yet.
#[cfg(unix)]
#[test]
fn agent_cannot_save_through_a_link_into_the_config_dir() {
    let config = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let mut s = session(&config);
    let prefs = config.path().join("preferences.toml");
    std::fs::write(&prefs, "theme = \"Light\"\n").unwrap();
    let to_prefs = elsewhere.path().join("innocent.flatcraft");
    std::os::unix::fs::symlink(&prefs, &to_prefs).unwrap();
    let new_file = config.path().join("profiles").join("planted.toml");
    let dangling = elsewhere.path().join("dangling.flatcraft");
    std::os::unix::fs::symlink(&new_file, &dangling).unwrap();
    let chained = elsewhere.path().join("chained.flatcraft");
    std::os::unix::fs::symlink(&dangling, &chained).unwrap();
    for path in [&to_prefs, &dangling, &chained] {
        let e = s
            .execute("project.save", json!({ "path": path }), Origin::Agent)
            .unwrap_err();
        assert!(
            matches!(e, CommandError::Refused(_)),
            "{}: {e}",
            path.display()
        );
    }
    assert_eq!(
        std::fs::read_to_string(&prefs).unwrap(),
        "theme = \"Light\"\n"
    );
    assert!(!new_file.exists());
}

/// The guard looks through a linked folder above the file, through a
/// relative link, and also guards `tools.test_patterns` `svg_path`.
#[cfg(unix)]
#[test]
fn agent_writes_through_folder_and_relative_links_are_refused() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir_all(config.join("profiles")).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    let profiles = ProfileStore::load(&config).unwrap();
    let prefs = Preferences {
        output_dir: Some(root.path().join("output")),
        ..Preferences::default()
    };
    let mut s = Session::new(profiles, prefs, false, config.clone());
    let prefs_file = config.join("preferences.toml");
    std::fs::write(&prefs_file, "theme = \"Light\"\n").unwrap();
    // A folder link: elsewhere/folder -> config/profiles.
    let folder = elsewhere.join("folder");
    symlink(config.join("profiles"), &folder).unwrap();
    let via_folder = folder.join("planted.flatcraft");
    // A relative link: elsewhere/rel.flatcraft -> ../config/preferences.toml.
    let relative = elsewhere.join("rel.flatcraft");
    symlink("../config/preferences.toml", &relative).unwrap();
    let svg_link = elsewhere.join("sheet.svg");
    symlink("../config/preferences.toml", &svg_link).unwrap();
    let svg_in_folder = folder.join("sheet.svg");
    let attempts = [
        ("project.save", json!({ "path": via_folder })),
        ("project.save", json!({ "path": relative })),
        (
            "tools.test_patterns",
            json!({ "add_to_document": false, "svg_path": svg_link }),
        ),
        (
            "tools.test_patterns",
            json!({ "add_to_document": false, "svg_path": svg_in_folder }),
        ),
    ];
    for (id, params) in attempts {
        let e = s.execute(id, params.clone(), Origin::Agent).unwrap_err();
        assert!(matches!(e, CommandError::Refused(_)), "{id} {params}: {e}");
    }
    assert_eq!(
        std::fs::read_to_string(&prefs_file).unwrap(),
        "theme = \"Light\"\n"
    );
    assert!(!config.join("profiles").join("planted.flatcraft").exists());
    assert!(!config.join("profiles").join("sheet.svg").exists());
    // The user may, and writes through the links.
    run(&mut s, "project.save", json!({ "path": via_folder }));
    assert!(config.join("profiles").join("planted.flatcraft").exists());
    run(
        &mut s,
        "tools.test_patterns",
        json!({ "add_to_document": false, "svg_path": svg_link }),
    );
    assert!(
        std::fs::read_to_string(&prefs_file)
            .unwrap()
            .contains("<svg")
    );
    assert!(
        std::fs::symlink_metadata(&svg_link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link stays a link"
    );
}

/// A `..` after a folder that does not exist cannot be resolved by the
/// file system; the guard must not fall back to the lexical path, which
/// would walk through a linked folder into the config dir.
#[cfg(unix)]
#[test]
fn agent_writes_through_unresolvable_dot_dot_are_refused() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    let profiles = ProfileStore::load(&config).unwrap();
    let prefs = Preferences {
        output_dir: Some(root.path().join("output")),
        ..Preferences::default()
    };
    let mut s = Session::new(profiles, prefs, false, config.clone());
    let prefs_file = config.join("preferences.toml");
    std::fs::write(&prefs_file, "theme = \"Light\"\n").unwrap();
    symlink(&config, elsewhere.join("cfglink")).unwrap();
    symlink(
        "../nonexist/../config/rel.flatcraft",
        elsewhere.join("rel.flatcraft"),
    )
    .unwrap();
    symlink("../nonexist/../config/rel.svg", elsewhere.join("rel.svg")).unwrap();
    let attempts = [
        (
            "project.save",
            json!({ "path": elsewhere.join("cfglink/nonexist/../preferences.toml") }),
        ),
        (
            "project.save",
            json!({ "path": elsewhere.join("rel.flatcraft") }),
        ),
        (
            "tools.test_patterns",
            json!({ "add_to_document": false, "svg_path": elsewhere.join("cfglink/nonexist/../sheet.svg") }),
        ),
        (
            "tools.test_patterns",
            json!({ "add_to_document": false, "svg_path": elsewhere.join("rel.svg") }),
        ),
    ];
    let listing = || {
        let mut v: Vec<_> = std::fs::read_dir(&config)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        v.sort();
        v
    };
    let before = listing();
    for (id, params) in attempts {
        let r = s.execute(id, params.clone(), Origin::Agent);
        assert!(r.is_err(), "{id} {params}: {r:?}");
        assert_eq!(
            std::fs::read_to_string(&prefs_file).unwrap(),
            "theme = \"Light\"\n",
            "{id} {params}"
        );
        assert_eq!(listing(), before, "{id} {params}");
    }
}

#[test]
fn documents_new_switch_close() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    import(&mut s, 10.0, 10.0);
    run(&mut s, "document.new", json!({}));
    assert_eq!(s.docs.len(), 2);
    assert_eq!(s.active, 1);
    assert!(s.doc().document.objects.is_empty());
    run(&mut s, "document.switch", json!({ "index": 0 }));
    assert_eq!(s.doc().document.objects.len(), 1);
    let e = s
        .execute("document.switch", json!({ "index": 5 }), Origin::User)
        .unwrap_err();
    assert!(matches!(e, CommandError::NotFound(_)), "{e}");
    let e = s
        .execute("document.close", json!({ "index": 0 }), Origin::Agent)
        .unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "dirty: {e}");
    run(
        &mut s,
        "document.close",
        json!({ "index": 0, "discard_changes": true }),
    );
    assert_eq!(s.docs.len(), 1);
    assert!(s.doc().document.objects.is_empty());
    run(&mut s, "document.close", json!({ "index": 0 }));
    assert_eq!(
        s.docs.len(),
        1,
        "closing the last document opens a fresh one"
    );
}

#[test]
fn profiles_and_job_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let r = run(&mut s, "profiles.list", json!({}));
    assert!(r["plotters"].as_array().unwrap().len() >= 2, "{r}");
    assert!(r["materials"].as_array().unwrap().len() >= 2, "{r}");
    let p = run(
        &mut s,
        "profiles.get",
        json!({ "id": "silhouette-cameo-2" }),
    );
    assert_eq!(p["speed_range"], json!([1, 10]));
    let m = run(&mut s, "profiles.get", json!({ "id": "vinyl" }));
    assert_eq!(m["id"], "vinyl");
    let e = s
        .execute("profiles.get", json!({ "id": "nope" }), Origin::User)
        .unwrap_err();
    assert!(matches!(e, CommandError::NotFound(_)), "{e}");

    run(&mut s, "job.set_material", json!({ "id": "vinyl" }));
    run(&mut s, "job.set_media", json!({ "id": "mat-12x24" }));
    let state = run(&mut s, "document.state", json!({}));
    assert_eq!(state["job"]["material_id"], "vinyl");
    assert!(close(
        state["cutting_area_mm"]["y1"].as_f64().unwrap(),
        604.6
    ));
    let e = s
        .execute("job.set_media", json!({ "id": "a4" }), Origin::User)
        .unwrap_err();
    assert!(matches!(e, CommandError::NotFound(_)), "{e}");
    run(
        &mut s,
        "job.set_plotter",
        json!({ "id": "silhouette-cameo-2" }),
    );
    assert_eq!(s.doc().document.job.plotter_id, "silhouette-cameo-2");

    run(
        &mut s,
        "job.set_overrides",
        json!({ "speed": 3, "force": 20, "passes": 2 }),
    );
    let job = &s.doc().document.job;
    assert_eq!(
        (job.speed, job.force, job.passes),
        (Some(3), Some(20), Some(2))
    );
    run(&mut s, "job.set_overrides", json!({ "clear": true }));
    let job = &s.doc().document.job;
    assert_eq!((job.speed, job.force, job.passes), (None, None, None));
}

#[test]
fn set_overrides_validates_range() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let e = s
        .execute("job.set_overrides", json!({ "speed": 11 }), Origin::User)
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
    let e = s
        .execute("job.set_overrides", json!({ "force": 34 }), Origin::User)
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
    assert_eq!(s.doc().document.job.speed, None);
}

#[test]
fn unknown_command_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let e = s
        .execute("object.teleport", json!({}), Origin::Agent)
        .unwrap_err();
    assert!(matches!(e, CommandError::UnknownCommand(_)), "{e}");
    assert!(e.to_string().contains("object.teleport"), "{e}");
}

#[test]
fn every_command_has_schema_and_description() {
    let all = registry::all();
    assert!(all.len() >= 26, "{}", all.len());
    let mut ids = std::collections::BTreeSet::new();
    for c in all {
        assert!(ids.insert(c.id()), "duplicate id {}", c.id());
        assert!(!c.description().trim().is_empty(), "{}", c.id());
        assert!(!c.title().trim().is_empty(), "{}", c.id());
        assert!(
            !c.description().contains('\u{2014}'),
            "em-dash in {}",
            c.id()
        );
        let schema = serde_json::to_value(c.params_schema()).unwrap();
        let text = serde_json::to_string(&schema).unwrap();
        assert!(
            !text.contains('\u{2014}'),
            "em-dash in schema of {}",
            c.id()
        );
        assert!(schema.is_object(), "{}", c.id());
        assert_eq!(schema["type"], "object", "{}: {schema}", c.id());
        assert!(registry::get(c.id()).is_some());
    }
    assert!(registry::get("nope").is_none());
}

#[test]
fn preferences_roundtrip_and_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let p = Preferences::load(dir.path());
    assert_eq!(p, Preferences::default());
    assert!(!p.allow_agent_cuts);
    let mut q = p.clone();
    q.allow_agent_cuts = true;
    q.recent_projects.push(dir.path().join("x.flatcraft"));
    q.save(dir.path()).unwrap();
    assert_eq!(Preferences::load(dir.path()), q);
    std::fs::write(
        dir.path().join("preferences.toml"),
        "allow_agent_cuts = 'nope",
    )
    .unwrap();
    assert_eq!(Preferences::load(dir.path()), Preferences::default());
}

fn hline(s: &mut Session, len: f64) -> u64 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><path d="M20 20 h{len}" stroke="black" fill="none"/></svg>"#
    );
    let r = run(
        s,
        "object.import_svg",
        json!({ "svg": svg, "name": "line" }),
    );
    r["id"].as_u64().unwrap()
}

#[test]
fn auto_arrange_wraps_zero_height_rows() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let ids: Vec<u64> = (0..3).map(|_| hline(&mut s, 150.0)).collect();
    for &i in &ids {
        assert!(close(placed_bounds(&s, i).height(), 0.0));
    }
    run(&mut s, "object.select", json!({ "ids": [] }));
    let r = run(&mut s, "arrange.auto", json!({}));
    assert_eq!(r["placed"].as_array().unwrap().len(), 3, "{r}");
    let area = Rect::new(5.0, 5.0, 299.8, 299.8);
    for &i in &ids {
        let b = placed_bounds(&s, i);
        assert!(
            b.x0 >= area.x0 - 1e-6 && b.x1 <= area.x1 + 1e-6 && b.y1 <= area.y1 + 1e-6,
            "{b:?} outside the cutting area"
        );
    }
}

// ---------------------------------------------------------------- raster

/// A 300 dpi style image: 300 x 300 px, white with a black 200 x 200 px square.
fn square_png(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let mut img = image::GrayImage::from_pixel(300, 300, image::Luma([255]));
    for y in 50..250 {
        for x in 50..250 {
            img.put_pixel(x, y, image::Luma([0]));
        }
    }
    let path = dir.path().join("Square.PNG");
    img.save(&path).unwrap();
    path
}

#[test]
fn raster_import_traces_and_reports() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let path = square_png(&dir);
    let r = run(&mut s, "object.import", json!({ "path": path }));
    let id = r["id"].as_u64().unwrap();
    assert_eq!(r["dropped_small"], 0);
    assert_eq!(r["despeckled"], 0);
    assert!(r["threshold_used"].as_u64().unwrap() > 0, "{r}");
    let o = s.doc().document.get(ObjectId(id)).unwrap();
    assert_eq!(o.name, "Square");
    assert!(matches!(
        o.source,
        flatcraft_core::document::Source::Raster { .. }
    ));
    let b = placed_bounds(&s, id);
    assert!((b.width() - 16.93).abs() < 0.1, "{b:?}");
    let state = run(&mut s, "document.state", json!({}));
    assert_eq!(state["objects"][0]["kind"], "raster");
}

#[test]
fn retrace_keeps_transform() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let r = run(&mut s, "object.import", json!({ "path": square_png(&dir) }));
    let id = r["id"].as_u64().unwrap();
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [id], "degrees": 30.0 }),
    );
    run(
        &mut s,
        "object.move",
        json!({ "ids": [id], "dx_mm": 7.0, "dy_mm": 3.0 }),
    );
    let before = s.doc().document.get(ObjectId(id)).unwrap().transform;

    let r = run(
        &mut s,
        "object.retrace",
        json!({ "id": id, "settings": { "invert": true } }),
    );
    assert_eq!(r["dropped_small"], 0);
    assert_eq!(r["despeckled"], 0);
    assert!(r["threshold_used"].is_u64(), "{r}");
    let o = s.doc().document.get(ObjectId(id)).unwrap();
    assert_transform_close(&o.transform, &before);
    // Inverted: the white frame, two paths instead of one.
    assert_eq!(o.geometry.len(), 2);
    match &o.source {
        flatcraft_core::document::Source::Raster { trace, .. } => assert!(trace.invert),
        other => panic!("{other:?}"),
    }

    run(&mut s, "edit.undo", json!({}));
    let o = s.doc().document.get(ObjectId(id)).unwrap();
    assert_eq!(o.geometry.len(), 1);
}

#[test]
fn retrace_refuses_bad_input() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let svg = import(&mut s, 20.0, 10.0);
    let r = run(&mut s, "object.import", json!({ "path": square_png(&dir) }));
    let png = r["id"].as_u64().unwrap();
    for (params, want) in [
        (json!({ "id": svg, "settings": {} }), "raster"),
        (json!({ "id": 999, "settings": {} }), "999"),
        (
            json!({ "id": png, "settings": { "smoothing": 11 } }),
            "smoothing",
        ),
        (
            json!({ "id": png, "settings": { "dpi_override": 0.0 } }),
            "dpi",
        ),
        (json!({ "id": png, "settings": { "bogus": 1 } }), "bogus"),
    ] {
        let e = s
            .execute("object.retrace", params.clone(), Origin::Agent)
            .expect_err(&format!("{params}"));
        let text = e.to_string();
        assert!(text.contains(want), "{params}: {text}");
        assert!(!text.contains('\u{2014}'), "{text}");
    }
}

#[test]
fn project_open_retraces_rasters() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let r = run(&mut s, "object.import", json!({ "path": square_png(&dir) }));
    let id = r["id"].as_u64().unwrap();
    let saved = dir.path().join("r.flatcraft");
    run(&mut s, "project.save", json!({ "path": saved }));
    let r = run(&mut s, "project.open", json!({ "path": saved }));
    assert_eq!(r["warnings"].as_array().map(Vec::len), Some(0), "{r}");
    let b = placed_bounds(&s, id);
    assert!((b.width() - 16.93).abs() < 0.1, "{b:?}");
}

fn assert_transform_close(
    a: &flatcraft_core::transform::Transform,
    b: &flatcraft_core::transform::Transform,
) {
    assert!((a.center_mm - b.center_mm).hypot() < 1e-9, "{a:?} vs {b:?}");
    assert_eq!(
        (a.scale_x, a.scale_y, a.rotation_deg, a.mirror),
        (b.scale_x, b.scale_y, b.rotation_deg, b.mirror)
    );
}

#[test]
fn retrace_keeps_the_image_frame_when_bounds_change_asymmetrically() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    // A big square plus a 30 px dot at the top-left; the dot is 2.5 mm.
    let mut img = image::GrayImage::from_pixel(300, 300, image::Luma([255]));
    for (x0, y0, size) in [(100u32, 100u32, 150u32), (10, 10, 30)] {
        for y in y0..y0 + size {
            for x in x0..x0 + size {
                img.put_pixel(x, y, image::Luma([0]));
            }
        }
    }
    let path = dir.path().join("two.png");
    img.save(&path).unwrap();
    let id = run(&mut s, "object.import", json!({ "path": path }))["id"]
        .as_u64()
        .unwrap();
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [id], "degrees": 30.0 }),
    );
    let square_before = {
        let o = s.doc().document.get(ObjectId(id)).unwrap();
        assert_eq!(o.geometry.len(), 2);
        let p = o.placed_paths();
        // The big square is the larger of the two.
        p.iter()
            .map(|c| kurbo::Shape::bounding_box(&c.path))
            .max_by(|a, b| a.area().total_cmp(&b.area()))
            .unwrap()
    };
    run(
        &mut s,
        "object.retrace",
        json!({ "id": id, "settings": { "min_feature_mm": 5.0 } }),
    );
    let o = s.doc().document.get(ObjectId(id)).unwrap();
    assert_eq!(o.geometry.len(), 1);
    let after = kurbo::Shape::bounding_box(&o.placed_paths()[0].path);
    assert!(
        (after.x0 - square_before.x0).abs() < 1e-6,
        "{after:?} {square_before:?}"
    );
    assert!(
        (after.y0 - square_before.y0).abs() < 1e-6,
        "{after:?} {square_before:?}"
    );
    assert!((after.x1 - square_before.x1).abs() < 1e-6);
}

/// An all-black image: traced as is, it is one square; inverted, nothing.
fn framed_square_png(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let img = image::GrayImage::from_pixel(300, 300, image::Luma([0]));
    let path = dir.path().join("framed.png");
    img.save(&path).unwrap();
    path
}

#[test]
fn retrace_from_an_empty_trace_keeps_the_image_frame() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = run(
        &mut s,
        "object.import",
        json!({ "path": framed_square_png(&dir) }),
    )["id"]
        .as_u64()
        .unwrap();
    run(
        &mut s,
        "object.move",
        json!({ "ids": [id], "dx_mm": 40.0, "dy_mm": 30.0 }),
    );
    let before = placed_bounds(&s, id);
    let r = run(
        &mut s,
        "object.retrace",
        json!({ "id": id, "settings": { "invert": true } }),
    );
    assert!(
        r["warnings"].to_string().contains("nothing was traced"),
        "{r}"
    );
    run(
        &mut s,
        "object.retrace",
        json!({ "id": id, "settings": {} }),
    );
    let after = placed_bounds(&s, id);
    assert!(
        (after.x0 - before.x0).abs() < 1e-6 && (after.y0 - before.y0).abs() < 1e-6,
        "{after:?} {before:?}"
    );
}

#[test]
fn an_image_that_traces_to_nothing_is_placed_in_the_cutting_area() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let blank = dir.path().join("blank.png");
    image::GrayImage::from_pixel(50, 50, image::Luma([255]))
        .save(&blank)
        .unwrap();
    let id = run(&mut s, "object.import", json!({ "path": blank }))["id"]
        .as_u64()
        .unwrap();
    // Traced again with a shape, the image frame sits where an image
    // with shapes would have gone: at the top-left of the cutting area.
    let r = run(
        &mut s,
        "object.retrace",
        json!({ "id": id, "settings": { "invert": true } }),
    );
    let area = run(&mut s, "document.state", json!({}))["cutting_area_mm"].clone();
    let b = placed_bounds(&s, id);
    assert!(
        (b.x0 - area["x0"].as_f64().unwrap()).abs() < 1e-6
            && (b.y0 - area["y0"].as_f64().unwrap()).abs() < 1e-6,
        "{b:?} {area} {r}"
    );
}

/// The trace of a saved image can differ when the project is opened
/// (other trace settings, another blade offset, a newer tracer). The image
/// frame stays where it was, as object.retrace and object.reload keep it.
#[test]
fn project_open_keeps_the_image_frame_when_the_trace_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    // A big square plus a 30 px dot at the top-left; the dot is 2.5 mm.
    let mut img = image::GrayImage::from_pixel(300, 300, image::Luma([255]));
    for (x0, y0, size) in [(100u32, 100u32, 150u32), (10, 10, 30)] {
        for y in y0..y0 + size {
            for x in x0..x0 + size {
                img.put_pixel(x, y, image::Luma([0]));
            }
        }
    }
    let png = dir.path().join("two.png");
    img.save(&png).unwrap();
    let id = run(&mut s, "object.import", json!({ "path": png }))["id"]
        .as_u64()
        .unwrap();
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [id], "degrees": 30.0 }),
    );
    let biggest = |o: &flatcraft_core::document::Object| {
        o.placed_paths()
            .iter()
            .map(|c| kurbo::Shape::bounding_box(&c.path))
            .max_by(|a, b| a.area().total_cmp(&b.area()))
            .unwrap()
    };
    let before = biggest(s.doc().document.get(ObjectId(id)).unwrap());
    let path = dir.path().join("p.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));

    // The dot is dropped when the project is opened.
    flatcraft_core::project::Archive::edit(&path, |a| {
        let v = &mut a.project;
        v["objects"][0]["source"]["Raster"]["trace"]["min_feature_mm"] = json!(5.0);
    })
    .unwrap();
    let mut s2 = session(&dir);
    run(&mut s2, "project.open", json!({ "path": path }));
    let o = s2.doc().document.get(ObjectId(id)).unwrap();
    assert_eq!(o.geometry.len(), 1);
    let after = biggest(o);
    for (a, b) in [
        (after.x0, before.x0),
        (after.y0, before.y0),
        (after.x1, before.x1),
        (after.y1, before.y1),
    ] {
        assert!((a - b).abs() < 1e-6, "{after:?} {before:?}");
    }
}

/// The same for an SVG: the page frame stays put when the shapes change
/// (here: a project file whose embedded SVG lost a shape).
#[test]
fn project_open_keeps_the_svg_page_frame_when_the_shapes_change() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let two = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="10" y="10" width="5" height="5"/><rect x="50" y="40" width="30" height="20"/></svg>"#;
    let one = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="50" y="40" width="30" height="20"/></svg>"#;
    let id = run(
        &mut s,
        "object.import_svg",
        json!({ "svg": two, "name": "two" }),
    )["id"]
        .as_u64()
        .unwrap();
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [id], "degrees": 30.0 }),
    );
    let o = s.doc().document.get(ObjectId(id)).unwrap();
    let before = kurbo::Shape::bounding_box(&o.placed_paths()[1].path);
    let path = dir.path().join("p.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    flatcraft_core::project::Archive::edit(&path, |a| {
        let file = a.add_source(&svg_source(one));
        a.project["objects"][0]["source"]["Svg"]["file"] = json!(file);
    })
    .unwrap();
    let mut s2 = session(&dir);
    run(&mut s2, "project.open", json!({ "path": path }));
    let o = s2.doc().document.get(ObjectId(id)).unwrap();
    assert_eq!(o.geometry.len(), 1);
    let after = kurbo::Shape::bounding_box(&o.placed_paths()[0].path);
    for (a, b) in [
        (after.x0, before.x0),
        (after.y0, before.y0),
        (after.x1, before.x1),
        (after.y1, before.y1),
    ] {
        assert!((a - b).abs() < 1e-6, "{after:?} {before:?}");
    }
}

/// A project file with one SVG object; `extra` is spliced into the
/// object (for example saved bounds) and `transform` replaces the default.
fn hand_written_project(
    dir: &tempfile::TempDir,
    extra: &str,
    transform: &str,
) -> std::path::PathBuf {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="10" y="10" width="10" height="20"/></svg>"#;
    let mut archive = flatcraft_core::project::Archive {
        project: Value::Null,
        sources: Default::default(),
        thumbnail: None,
    };
    let file = archive.add_source(&svg_source(svg));
    let text = format!(
        r#"{{"format_version":1,"objects":[{{"id":1,"name":"box","source":{{"Svg":{{"name":"box.svg","file":"{file}"}}}},"transform":{transform},"visible":true{extra}}}],"job":{{"plotter_id":"cameo2","material_id":"cardstock","media_id":"a4","speed":null,"force":null,"passes":null}},"next_id":2}}"#
    );
    archive.project = serde_json::from_str(&text).unwrap();
    let path = dir.path().join("hand.flatcraft");
    archive.write(&path).unwrap();
    path
}

/// An SVG source with these bytes.
fn svg_source(svg: &str) -> flatcraft_core::document::Source {
    flatcraft_core::document::Source::Svg {
        name: "box.svg".into(),
        bytes: svg.as_bytes().into(),
    }
}

const PLAIN_TRANSFORM: &str = r#"{"center_mm":{"x":50.0,"y":60.0},"scale_x":1.0,"scale_y":1.0,"rotation_deg":0.0,"mirror":false}"#;

/// Projects from before the saved bounds: the transform is relative to
/// the centre of the rebuilt shapes, as it always was.
#[test]
fn project_open_without_saved_bounds_keeps_the_old_placement() {
    let dir = tempfile::tempdir().unwrap();
    let path = hand_written_project(&dir, "", PLAIN_TRANSFORM);
    let mut s = session(&dir);
    let r = run(&mut s, "project.open", json!({ "path": path }));
    assert!(r["warnings"].as_array().unwrap().is_empty(), "{r}");
    let b = placed_bounds(&s, 1);
    assert!(
        close(b.x0, 45.0) && close(b.y0, 50.0) && close(b.x1, 55.0) && close(b.y1, 70.0),
        "{b:?}"
    );
}

/// Saved bounds that cannot be right are not used: the object opens as
/// in an older project, with a warning.
#[test]
fn project_open_ignores_broken_saved_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let extra = r#","local_bounds_mm":{"x0":10.0,"y0":10.0,"x1":20.0,"y1":30.0},"frame_mm":{"x0":1e308,"y0":0.0,"x1":100.0,"y1":100.0}"#;
    let path = hand_written_project(&dir, extra, PLAIN_TRANSFORM);
    let mut s = session(&dir);
    let r = run(&mut s, "project.open", json!({ "path": path }));
    let w = r["warnings"].to_string();
    assert!(w.contains("box: the saved position data is broken"), "{w}");
    let b = placed_bounds(&s, 1);
    assert!(
        close(b.x0, 45.0) && close(b.y0, 50.0) && close(b.x1, 55.0) && close(b.y1, 70.0),
        "{b:?}"
    );
}

#[test]
fn project_open_refuses_a_broken_transform() {
    let dir = tempfile::tempdir().unwrap();
    for transform in [
        r#"{"center_mm":{"x":50.0,"y":60.0},"scale_x":0.0,"scale_y":1.0,"rotation_deg":0.0,"mirror":false}"#,
        // NaN, as JSON writes it
        r#"{"center_mm":{"x":null,"y":60.0},"scale_x":1.0,"scale_y":1.0,"rotation_deg":0.0,"mirror":false}"#,
        r#"{"center_mm":{"x":50.0,"y":60.0},"scale_x":1.0,"scale_y":1.0,"rotation_deg":null,"mirror":false}"#,
    ] {
        let path = hand_written_project(&dir, "", transform);
        let mut s = session(&dir);
        let e = s
            .execute("project.open", json!({ "path": path }), Origin::User)
            .unwrap_err();
        let text = e.to_string();
        assert!(
            text.contains("'box'") && text.contains("transform"),
            "{text}"
        );
        assert_eq!(s.doc().document.objects.len(), 0);
    }
}

const TOO_LARGE: &str = "That would put an object more than 1 km from the mat, or make it over 1 km in size. Undo it, or move or scale it back.";

/// project.open refuses a placement beyond 1e6 mm, so no command may
/// make one: group scaling by 1000 stops before it overflows.
#[test]
fn group_scale_is_refused_before_the_object_could_not_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = import(&mut s, 10.0, 10.0);
    let b = import(&mut s, 20.0, 10.0);
    let mut refused = None;
    for k in 0..400 {
        match s.execute(
            "object.scale",
            json!({ "ids": [a, b], "factor": 1000.0, "group": true }),
            Origin::User,
        ) {
            Ok(_) => {}
            Err(e) => {
                refused = Some((k, e));
                break;
            }
        }
    }
    let (k, e) = refused.expect("never refused");
    assert!(k <= 3, "refused only after {k} steps");
    assert!(matches!(e, CommandError::Refused(_)), "{e}");
    assert!(e.to_string().contains(TOO_LARGE), "{e}");
    // The refused step changed nothing; the project still saves and opens.
    let path = dir.path().join("big.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    let mut s2 = session(&dir);
    run(&mut s2, "project.open", json!({ "path": path }));
}

#[test]
fn a_move_beyond_1_km_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 10.0, 10.0);
    let before = placed_bounds(&s, id);
    let e = s
        .execute(
            "object.move",
            json!({ "ids": [id], "center_mm": [2e6, 50.0] }),
            Origin::User,
        )
        .unwrap_err();
    assert!(e.to_string().contains(TOO_LARGE), "{e}");
    assert_eq!(placed_bounds(&s, id), before);
}

/// A wrapped error text joins after a colon in lower case.
#[test]
fn project_open_warnings_join_wrapped_errors_in_lower_case() {
    let dir = tempfile::tempdir().unwrap();
    let path = hand_written_project(&dir, "", PLAIN_TRANSFORM);
    flatcraft_core::project::Archive::edit(&path, |a| {
        let file = a.add_source(&svg_source("<not svg>"));
        a.project["objects"][0]["source"]["Svg"]["file"] = json!(file);
    })
    .unwrap();
    let mut s = session(&dir);
    let r = run(&mut s, "project.open", json!({ "path": path }));
    let w = r["warnings"][0].as_str().unwrap().to_string();
    assert!(
        w.starts_with("box: cannot rebuild the shape: the file could not be parsed"),
        "{w}"
    );
}

#[test]
fn big_raster_reports_the_downscale_and_wording_follows_invert() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let mut img = image::GrayImage::from_pixel(6000, 4000, image::Luma([255]));
    for y in 1000..2000 {
        for x in 1000..3000 {
            img.put_pixel(x, y, image::Luma([0]));
        }
    }
    let path = dir.path().join("big.png");
    img.save(&path).unwrap();
    let r = run(&mut s, "object.import", json!({ "path": path }));
    assert_eq!(r["downscaled_from"], json!([6000, 4000]));
    let w = r["warnings"].to_string();
    assert!(
        w.contains("traced at a lower resolution (6000 x 4000 px)"),
        "{w}"
    );
    let id = r["id"].as_u64().unwrap();
    let b = placed_bounds(&s, id);
    assert!((b.width() - 169.33).abs() < 0.6, "{b:?}");

    // An all-white image traces to nothing; inverted, an all-black one does.
    let blank = dir.path().join("blank.png");
    image::GrayImage::from_pixel(50, 50, image::Luma([255]))
        .save(&blank)
        .unwrap();
    let r = run(&mut s, "object.import", json!({ "path": blank }));
    assert!(r["warnings"].to_string().contains("no dark shape"), "{r}");
    let black = dir.path().join("black.png");
    image::GrayImage::from_pixel(50, 50, image::Luma([0]))
        .save(&black)
        .unwrap();
    let r = run(&mut s, "object.import", json!({ "path": black }));
    let id = r["id"].as_u64().unwrap();
    let r = run(
        &mut s,
        "object.retrace",
        json!({ "id": id, "settings": { "invert": true } }),
    );
    assert!(r["warnings"].to_string().contains("no light shape"), "{r}");
}

#[test]
fn a_raster_larger_than_the_cutting_area_warns_at_import() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    // 4000 px at 300 dpi: a bar about 330 mm long, longer than any 12 inch mat.
    let mut img = image::GrayImage::from_pixel(4000, 300, image::Luma([255]));
    for y in 100..200 {
        for x in 50..3950 {
            img.put_pixel(x, y, image::Luma([0]));
        }
    }
    let path = dir.path().join("long.png");
    img.save(&path).unwrap();
    let r = run(&mut s, "object.import", json!({ "path": path }));
    let w = r["warnings"].to_string();
    assert!(w.contains("larger than the cutting area"), "{w}");
    // A small image gives no such warning.
    let r = run(&mut s, "object.import", json!({ "path": square_png(&dir) }));
    assert!(!r["warnings"].to_string().contains("cutting area"), "{r}");
}

#[test]
fn an_svg_larger_than_the_cutting_area_warns_at_import() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let r = run(
        &mut s,
        "object.import_svg",
        json!({ "svg": rect_svg(400.0, 20.0), "name": "long" }),
    );
    let w = r["warnings"].to_string();
    assert!(
        w.contains("The shapes are 400 x 20 mm, larger than the cutting area"),
        "{w}"
    );
    let r = run(
        &mut s,
        "object.import_svg",
        json!({ "svg": rect_svg(40.0, 20.0), "name": "short" }),
    );
    assert!(!r["warnings"].to_string().contains("cutting area"), "{r}");
}

/// A retrace at a lower dpi makes the shapes 30 times larger: that warns.
/// Retracing the same again does not warn a second time.
#[test]
fn a_retrace_that_grows_past_the_cutting_area_warns() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = run(&mut s, "object.import", json!({ "path": square_png(&dir) }))["id"]
        .as_u64()
        .unwrap();
    let r = run(
        &mut s,
        "object.retrace",
        json!({ "id": id, "settings": { "dpi_override": 10.0 } }),
    );
    assert!(
        r["warnings"]
            .to_string()
            .contains("larger than the cutting area"),
        "{r}"
    );
    let r = run(
        &mut s,
        "object.retrace",
        json!({ "id": id, "settings": { "dpi_override": 10.0 } }),
    );
    assert!(!r["warnings"].to_string().contains("cutting area"), "{r}");
}

#[test]
fn oversized_image_error_names_the_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    // IHDR claims 30000 x 30000; a stub IDAT follows.
    let mut png: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let chunk = |kind: &[u8; 4], data: &[u8]| {
        let mut c = flate2::Crc::new();
        c.update(kind);
        c.update(data);
        let mut v = (data.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(data);
        v.extend_from_slice(&c.sum().to_be_bytes());
        v
    };
    let mut ihdr = 30000u32.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&30000u32.to_be_bytes());
    ihdr.extend_from_slice(&[1, 0, 0, 0, 0]);
    png.extend(chunk(b"IHDR", &ihdr));
    png.extend(chunk(b"IDAT", &[0x78, 0x9c, 0x63, 0x00]));
    png.extend(chunk(b"IEND", &[]));
    let path = dir.path().join("bomb.png");
    std::fs::write(&path, png).unwrap();
    let e = s
        .execute("object.import", json!({ "path": path }), Origin::User)
        .unwrap_err();
    let text = e.to_string();
    assert!(
        text.contains("30000 x 30000 pixels, at most 16384 per side"),
        "{text}"
    );
    assert!(!text.contains("50 MB"), "{text}");
}

#[test]
fn save_by_doc_id_writes_that_document() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    import(&mut s, 10.0, 10.0);
    let first = s.doc().id();
    run(&mut s, "document.new", json!({}));
    assert_eq!(s.active, 1);
    let path = dir.path().join("first.flatcraft");
    let r = run(
        &mut s,
        "project.save",
        json!({ "doc_id": first, "path": path }),
    );
    assert_eq!(r["path"], json!(path));
    assert_eq!(s.active, 1, "saving does not switch documents");
    assert_eq!(s.docs[0].path.as_deref(), Some(path.as_path()));
    assert!(!s.docs[0].dirty);
    assert!(s.docs[1].path.is_none());
    let saved = flatcraft_core::project::load(&path).unwrap();
    assert_eq!(saved.objects.len(), 1, "the first document was saved");
    // Without a path it uses that document's own path.
    run(&mut s, "project.save", json!({ "doc_id": first }));
}

#[test]
fn switch_and_close_by_doc_id() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = s.doc().id();
    run(&mut s, "document.new", json!({}));
    let b = s.doc().id();
    run(&mut s, "document.new", json!({}));
    let c = s.doc().id();
    run(&mut s, "document.switch", json!({ "doc_id": a }));
    assert_eq!(s.doc().id(), a);
    // Closing a shifts the indexes; closing b by id still closes b.
    run(&mut s, "document.close", json!({ "doc_id": a }));
    run(&mut s, "document.close", json!({ "doc_id": b }));
    let ids: Vec<u64> = s.docs.iter().map(|d| d.id()).collect();
    assert_eq!(ids, [c]);
    let st = run(&mut s, "document.state", json!({}));
    assert_eq!(st["doc_id"], json!(c));
    assert_eq!(st["doc_ids"], json!([c]));
    for (cmd, params) in [
        ("document.switch", json!({ "doc_id": a })),
        ("document.close", json!({ "doc_id": a })),
        (
            "project.save",
            json!({ "doc_id": a, "path": dir.path().join("x.flatcraft") }),
        ),
    ] {
        let e = s.execute(cmd, params, Origin::User).unwrap_err();
        assert_eq!(
            e,
            CommandError::NotFound("That document is no longer open.".into()),
            "{cmd}"
        );
    }
    for params in [json!({}), json!({ "index": 0, "doc_id": c })] {
        let e = s
            .execute("document.switch", params, Origin::User)
            .unwrap_err();
        assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
    }
}

#[test]
fn pristine_document() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    assert!(s.doc().is_pristine());
    import(&mut s, 10.0, 10.0);
    assert!(!s.doc().is_pristine());
    run(&mut s, "edit.undo", json!({}));
    assert!(!s.doc().is_pristine(), "it has a redo step");
}

#[test]
fn undo_and_redo_by_doc_id() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 10.0, 10.0);
    let first = s.doc().id();
    run(&mut s, "document.new", json!({}));
    run(&mut s, "edit.undo", json!({ "doc_id": first }));
    assert_eq!(s.active, 1, "undo does not switch documents");
    assert!(s.docs[0].document.get(ObjectId(id)).is_none());
    run(&mut s, "edit.redo", json!({ "doc_id": first }));
    assert!(s.docs[0].document.get(ObjectId(id)).is_some());
    let e = s
        .execute("edit.undo", json!({ "doc_id": 999_999 }), Origin::User)
        .unwrap_err();
    assert_eq!(
        e,
        CommandError::NotFound("That document is no longer open.".into())
    );
}

#[test]
fn preview_fills_the_mat_with_its_color() {
    use flatcraft_core::colour::{contrast_colour, margin_colour};
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let png = flatcraft_engine::preview::render_png(
        &s,
        flatcraft_engine::preview::PreviewKind::Canvas,
        512,
    )
    .unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    let paper = flatcraft_device::PAPER_COLOR;
    // A corner of the physical mat, outside the grid: the darker margin.
    let (w, h) = img.dimensions();
    let px = img.get_pixel(w / 2, h - 3);
    assert_eq!(&px.0[..3], &margin_colour(paper));
    // Somewhere in the cutting area between two grid lines (the grid is
    // 10 mm; 512 px for 330 mm is about 1.55 px per mm): the mat colour.
    let inside = (w / 4..w * 3 / 4)
        .flat_map(|x| (h / 4..h * 3 / 4).map(move |y| (x, y)))
        .filter(|&(x, y)| img.get_pixel(x, y).0[..3] == paper)
        .count();
    assert!(inside > 1000, "{inside} paper pixels in the cutting area");
    // The grid lines are dark on paper (the canvas's contrast rule).
    let dark = contrast_colour(paper);
    let darker = (w / 4..w * 3 / 4)
        .flat_map(|x| (h / 4..h * 3 / 4).map(move |y| (x, y)))
        .any(|(x, y)| {
            let p = img.get_pixel(x, y).0;
            (0..3).all(|c| p[c] < paper[c]) && p[0].abs_diff(dark[0]) < 200
        });
    assert!(darker, "no grid line pixels");
}

#[test]
fn raster_import_reports_removed_specks() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let path = square_png(&dir);
    let mut img = image::open(&path).unwrap().into_luma8();
    // Three 2 x 2 px dots, far below the 0.5 mm2 despeckle default.
    for i in 0..3 {
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            img.put_pixel(10 + i * 40 + dx, 10 + dy, image::Luma([0]));
        }
    }
    img.save(&path).unwrap();
    let r = run(&mut s, "object.import", json!({ "path": path }));
    assert_eq!(r["despeckled"], 3, "{r}");
    assert_eq!(r["dropped_small"], 0, "{r}");
}

#[test]
fn a_command_that_changes_nothing_records_no_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 20.0, 20.0);
    run(
        &mut s,
        "project.save",
        json!({ "path": dir.path().join("p.flatcraft") }),
    );
    let undo = s.doc().undo.undo_len();
    let revision = s.doc().revision();
    for (command, params) in [
        (
            "object.move",
            json!({ "ids": [id], "dx_mm": 0.0, "dy_mm": 0.0 }),
        ),
        (
            "object.set_visible",
            json!({ "ids": [id], "visible": true }),
        ),
        ("object.scale", json!({ "ids": [id], "factor": 1.0 })),
        (
            "job.set_material",
            json!({ "id": s.doc().document.job.material_id.clone() }),
        ),
    ] {
        run(&mut s, command, params);
        assert_eq!(
            s.doc().undo.undo_len(),
            undo,
            "{command} recorded an undo step"
        );
        assert!(!s.doc().dirty, "{command} marked the document as changed");
        assert_eq!(
            s.doc().revision(),
            revision,
            "{command} raised the revision"
        );
    }
    run(&mut s, "object.move", json!({ "ids": [id], "dx_mm": 1.0 }));
    assert_eq!(s.doc().undo.undo_len(), undo + 1);
    assert!(s.doc().dirty);
}

#[test]
fn null_clears_one_override() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(
        &mut s,
        "job.set_overrides",
        json!({ "speed": 5, "force": 10, "passes": 2 }),
    );
    let r = run(&mut s, "job.set_overrides", json!({ "force": null }));
    assert_eq!(r, json!({ "speed": 5, "force": null, "passes": 2 }));
    let job = &s.doc().document.job;
    assert_eq!((job.speed, job.force, job.passes), (Some(5), None, Some(2)));
    // A missing field stays as it is.
    let r = run(&mut s, "job.set_overrides", json!({ "passes": 3 }));
    assert_eq!(r, json!({ "speed": 5, "force": null, "passes": 3 }));
}

#[test]
fn passes_follow_the_plotter_range() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("plotters")).unwrap();
    std::fs::write(
        dir.path().join("plotters/many-passes.toml"),
        "id = \"many-passes\"\nname = \"Many passes\"\ninherits = \"file-output-gpgl-cameo2\"\npasses_range = [1, 30]\n",
    )
    .unwrap();
    let mut s = session(&dir);
    let r = run(&mut s, "job.set_overrides", json!({ "passes": 20 }));
    assert_eq!(r["passes"], 20);
    let e = s
        .execute("job.set_overrides", json!({ "passes": 21 }), Origin::User)
        .unwrap_err();
    assert!(e.to_string().contains("1 to 20"), "{e}");
    run(&mut s, "job.set_plotter", json!({ "id": "many-passes" }));
    let r = run(&mut s, "job.set_overrides", json!({ "passes": 25 }));
    assert_eq!(r["passes"], 25);
    // Back on a plotter with fewer passes, the override is cleared.
    let r = run(
        &mut s,
        "job.set_plotter",
        json!({ "id": "file-output-gpgl-cameo2" }),
    );
    assert_eq!(r["cleared_overrides"], json!(["passes"]));
    assert_eq!(s.doc().document.job.passes, None);
}

#[test]
fn a_missing_mat_falls_back_to_the_first_one_with_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    import(&mut s, 20.0, 20.0);
    // An edited project file, or a mat hidden in the Profiles window.
    s.doc_mut().document.job.media_id = "gone".into();
    let st = run(&mut s, "document.state", json!({}));
    assert_eq!(st["job"]["media_id"], "gone");
    assert_eq!(st["mat"]["id"], "mat-12x12", "{st}");
    assert!(st["cutting_area_mm"].is_object(), "{st}");
    let warnings = st["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{st}");
    let w = warnings[0].as_str().unwrap();
    assert!(w.contains("'gone'") && w.contains("Mat 12x12"), "{w}");
    let r = run(&mut s, "job.preview", json!({}));
    assert!(
        r["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x.as_str() == Some(w)),
        "{r}"
    );
    run(
        &mut s,
        "arrange.align",
        json!({ "ids": [], "mode": "left", "relative_to": "cutting_area" }),
    )
    .as_object()
    .unwrap();
}

#[test]
fn a_headless_session_rereads_changed_preferences() {
    use std::time::{Duration, Instant, SystemTime};
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("output");
    let stamp = SystemTime::now() + Duration::from_secs(60);
    // Every version keeps File Output in the tempdir.
    let write = |allow: &str, at: SystemTime| {
        let path = dir.path().join(flatcraft_engine::prefs::FILE_NAME);
        let text = format!("output_dir = {:?}\n{allow}\n", out.to_str().unwrap());
        std::fs::write(&path, text).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    };
    write("allow_agent_cuts = false", stamp);
    let mut s = session(&dir);
    s.watch_prefs_file();
    let t0 = Instant::now();
    assert!(!s.refresh_prefs(t0), "nothing changed");
    write("allow_agent_cuts = true", stamp + Duration::from_secs(60));
    assert!(
        !s.refresh_prefs(t0 + Duration::from_millis(500)),
        "checked at most once per second"
    );
    assert!(!s.prefs.allow_agent_cuts);
    assert!(s.refresh_prefs(t0 + Duration::from_millis(1100)));
    assert!(s.prefs.allow_agent_cuts);
    assert_eq!(s.prefs.output_dir.as_deref(), Some(out.as_path()));
    // Same time, other size (a coarse clock): still seen.
    write(
        "allow_agent_cuts = false\nauto_reload = true",
        stamp + Duration::from_secs(60),
    );
    assert!(s.refresh_prefs(t0 + Duration::from_millis(2200)));
    assert!(s.prefs.auto_reload);
    // A broken file keeps what was read last, not the defaults.
    write("allow_agent_cuts = [", stamp + Duration::from_secs(120));
    assert!(!s.refresh_prefs(t0 + Duration::from_millis(3300)));
    assert!(s.prefs.auto_reload);
}

fn state_bounds(s: &mut Session, id: u64) -> Rect {
    let st = run(s, "document.state", json!({}));
    let o = st["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .unwrap()["bounds_mm"]
        .clone();
    let f = |k: &str| o[k].as_f64().unwrap();
    Rect::new(f("x0"), f("y0"), f("x1"), f("y1"))
}

#[test]
fn group_scale_keeps_the_layout_about_one_anchor() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = import(&mut s, 10.0, 10.0);
    let b = import(&mut s, 20.0, 10.0);
    run(
        &mut s,
        "object.move",
        json!({ "ids": [b], "dx_mm": 30.0, "dy_mm": 15.0 }),
    );
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [b], "degrees": 90.0 }),
    );
    let (ra, rb) = (state_bounds(&mut s, a), state_bounds(&mut s, b));
    let group = ra.union(rb);
    let undo = s.doc().undo.undo_len();
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [a, b], "factor": 2.0, "anchor": "top_left", "group": true }),
    );
    assert_eq!(s.doc().undo.undo_len(), undo + 1, "one undo step");
    let anchor = Point::new(group.x0, group.y0);
    let expect = |r: Rect| {
        let p0 = anchor + (Point::new(r.x0, r.y0) - anchor) * 2.0;
        let p1 = anchor + (Point::new(r.x1, r.y1) - anchor) * 2.0;
        Rect::from_points(p0, p1)
    };
    for (id, before) in [(a, ra), (b, rb)] {
        let now = state_bounds(&mut s, id);
        let want = expect(before);
        for (x, y) in [
            (now.x0, want.x0),
            (now.y0, want.y0),
            (now.x1, want.x1),
            (now.y1, want.y1),
        ] {
            assert!(close(x, y), "object {id}: {now:?} != {want:?}");
        }
    }
    // Width and height set the group box.
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [a, b], "width_mm": 100.0, "anchor": "center", "group": true }),
    );
    let group2 = state_bounds(&mut s, a).union(state_bounds(&mut s, b));
    assert!(close(group2.width(), 100.0), "{group2:?}");
    // Both are straight (0 and 90 degrees): only the width changes.
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [a, b], "width_mm": 50.0, "keep_aspect": false, "group": true }),
    );
    let group3 = state_bounds(&mut s, a).union(state_bounds(&mut s, b));
    assert!(close(group3.width(), 50.0), "{group3:?}");
    assert!(close(group3.height(), group2.height()), "{group3:?}");
}

#[test]
fn a_material_with_too_many_passes_is_refused_at_job_build() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("materials")).unwrap();
    std::fs::write(
        dir.path().join("materials/many.toml"),
        "id = \"many\"\nname = \"Many\"\ninherits = \"vinyl\"\npasses = 25\n",
    )
    .unwrap();
    let mut s = session(&dir);
    import(&mut s, 20.0, 20.0);
    run(&mut s, "job.set_material", json!({ "id": "many" }));
    let e = s
        .execute("job.preview", json!({}), Origin::User)
        .unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "{e}");
    let text = e.to_string();
    assert!(text.contains("25") && text.contains("1 to 20"), "{text}");
    // An override inside the range is fine.
    run(&mut s, "job.set_overrides", json!({ "passes": 2 }));
    run(&mut s, "job.preview", json!({}));
}

// ---------------------------------------------------------------- preferences written by two sessions

fn prefs_session(dir: &tempfile::TempDir) -> Session {
    let profiles = ProfileStore::load(dir.path()).unwrap();
    let prefs = Preferences::load(dir.path());
    Session::new(profiles, prefs, true, dir.path().to_path_buf())
}

#[test]
fn a_preference_change_survives_another_sessions_later_save() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = prefs_session(&dir);
    let mut b = prefs_session(&dir);
    a.update_prefs(|p| p.theme = flatcraft_engine::prefs::Theme::Light)
        .unwrap();
    // B still holds the old copy and stores a recent project.
    let recent = dir.path().join("badge.flatcraft");
    b.update_prefs(|p| p.recent_projects.insert(0, recent.clone()))
        .unwrap();
    let stored = Preferences::load(dir.path());
    assert_eq!(stored.theme, flatcraft_engine::prefs::Theme::Light);
    assert_eq!(stored.recent_projects, std::slice::from_ref(&recent));
    assert_eq!(b.prefs, stored, "B gets the fresh copy");
    // And the other way round.
    a.update_prefs(|p| p.auto_reload = true).unwrap();
    let stored = Preferences::load(dir.path());
    assert_eq!(stored.recent_projects, [recent]);
    assert!(stored.auto_reload);
}

#[test]
fn an_unreadable_preferences_file_turns_agent_cuts_off() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preferences.toml");
    let stamp = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let write = |text: &str, at: std::time::SystemTime| {
        std::fs::write(&path, text).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    };
    write("allow_agent_cuts = true\nauto_reload = true\n", stamp);
    let mut s = prefs_session(&dir);
    s.watch_prefs_file();
    assert!(s.prefs.allow_agent_cuts);
    write(
        "allow_agent_cuts = [",
        stamp + std::time::Duration::from_secs(60),
    );
    let t0 = std::time::Instant::now() + std::time::Duration::from_secs(5);
    assert!(!s.refresh_prefs(t0));
    assert!(!s.prefs.allow_agent_cuts, "fails closed");
    assert!(s.prefs.auto_reload, "the rest stays as read last");
}

/// A broken file is reported once per change of the file, not at every
/// check, and the report waits for the window to take it.
#[test]
fn a_broken_preferences_file_is_reported_once() {
    use std::time::{Duration, Instant, SystemTime};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(flatcraft_engine::prefs::FILE_NAME);
    let write = |text: &str, secs: u64| {
        std::fs::write(&path, text).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(secs))
            .unwrap();
    };
    let mut s = prefs_session(&dir);
    s.watch_prefs_file();
    assert_eq!(s.take_prefs_warning(), None);
    write("allow_agent_cuts = [", 60);
    let t0 = Instant::now() + Duration::from_secs(5);
    assert!(!s.refresh_prefs(t0));
    let w = s.take_prefs_warning().expect("a warning");
    assert!(w.contains("preferences.toml"), "{w}");
    assert!(!w.contains('\n'), "one line for the window: {w}");
    assert!(w.ends_with("until the file is fixed."), "{w}");
    for n in 1..4 {
        assert!(!s.refresh_prefs(t0 + Duration::from_millis(1100 * n)));
        assert_eq!(s.take_prefs_warning(), None, "the same file, no news");
    }
    // Broken in another way: a new report.
    write("allow_agent_cuts = [[", 120);
    assert!(!s.refresh_prefs(t0 + Duration::from_secs(10)));
    assert!(s.take_prefs_warning().is_some());
    // Fixed: read, and no report.
    write("auto_reload = true\n", 180);
    assert!(s.refresh_prefs(t0 + Duration::from_secs(20)));
    assert!(s.prefs.auto_reload);
    assert_eq!(s.take_prefs_warning(), None);
}

/// The start-up warning for the window is one line; the parser's details
/// go to stderr.
#[test]
fn the_start_up_preferences_warning_is_one_line() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(flatcraft_engine::prefs::FILE_NAME),
        "allow_agent_cuts = [",
    )
    .unwrap();
    let (prefs, w) = Preferences::load_reporting(dir.path());
    assert_eq!(prefs, Preferences::default());
    let w = w.expect("a warning");
    assert!(w.contains("preferences.toml is broken"), "{w}");
    assert!(!w.contains('\n'), "{w}");
    assert!(w.ends_with("Using default preferences."), "{w}");
}

/// A broken file put back exactly as it was read last (same time and size,
/// like `cp -p` of a backup) is read again, so agent cuts come back.
#[test]
fn an_exact_restore_after_a_broken_file_is_read_again() {
    use std::time::{Duration, Instant, SystemTime};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(flatcraft_engine::prefs::FILE_NAME);
    let good = "allow_agent_cuts = true\n";
    let at = SystemTime::now() - Duration::from_secs(3600);
    let write = |text: &str, at: SystemTime| {
        std::fs::write(&path, text).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    };
    write(good, at);
    let mut s = prefs_session(&dir);
    s.watch_prefs_file();
    assert!(s.prefs.allow_agent_cuts);
    write("allow_agent_cuts = [", at + Duration::from_secs(60));
    let t0 = Instant::now() + Duration::from_secs(5);
    assert!(!s.refresh_prefs(t0));
    assert!(!s.prefs.allow_agent_cuts, "fails closed");
    write(good, at);
    assert!(s.refresh_prefs(t0 + Duration::from_millis(1100)));
    assert!(s.prefs.allow_agent_cuts, "the restored file counts again");
}

/// A read that fails while the file itself stays the same (time and size)
/// is tried again at the next check, not only after the next change.
#[cfg(unix)]
#[test]
fn a_failed_preferences_read_is_retried() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant, SystemTime};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(flatcraft_engine::prefs::FILE_NAME);
    let mut s = prefs_session(&dir);
    s.watch_prefs_file();
    std::fs::write(&path, "auto_reload = true\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(60))
        .unwrap();
    let mode =
        |m: u32| std::fs::set_permissions(&path, std::fs::Permissions::from_mode(m)).unwrap();
    mode(0o000);
    if std::fs::read(&path).is_ok() {
        // Root reads anyway; nothing to test here.
        return;
    }
    let t0 = Instant::now() + Duration::from_secs(5);
    assert!(!s.refresh_prefs(t0), "the read fails");
    assert!(!s.prefs.auto_reload);
    // Readable again; chmod changes neither the time nor the size.
    mode(0o644);
    assert!(
        s.refresh_prefs(t0 + Duration::from_millis(1100)),
        "the next check reads the file again"
    );
    assert!(s.prefs.auto_reload);
}

fn backups(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("preferences.toml.bak-")
        })
        .collect();
    v.sort();
    v
}

#[test]
fn a_broken_preferences_file_is_kept_as_a_backup_before_a_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preferences.toml");
    let broken = "theme = \"Light\"\nallow_agent_cuts = [\n";
    std::fs::write(&path, broken).unwrap();
    let mut s = prefs_session(&dir);
    s.update_prefs(|p| p.auto_reload = true).unwrap();
    let saved = backups(dir.path());
    assert_eq!(saved.len(), 1, "{saved:?}");
    // The window hears about the copy, not only stderr.
    let w = s.take_prefs_warning().expect("a warning about the copy");
    let copy = saved[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(w.contains(&copy), "{w}");
    assert_eq!(std::fs::read_to_string(&saved[0]).unwrap(), broken);
    let name = saved[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(!name.contains(':'), "{name}");
    assert!(Preferences::read(dir.path()).unwrap().auto_reload);
    // The file is fine now, so the next save keeps no copy.
    s.update_prefs(|p| p.auto_reload = false).unwrap();
    assert_eq!(backups(dir.path()).len(), 1);
    assert_eq!(s.take_prefs_warning(), None, "no copy, no warning");
    // A plain save backs up a broken file too.
    std::fs::write(&path, broken).unwrap();
    Preferences::default().save(dir.path()).unwrap();
    assert_eq!(backups(dir.path()).len(), 2);
}

#[test]
fn new_documents_start_on_a_mat_the_default_plotter_has() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let base = s.prefs.default_plotter.clone();
    assert_eq!(s.doc().document.job.media_id, "mat-12x12");
    let mut p = s.profiles.plotter(&base).unwrap();
    p.id = "no-square-mat".into();
    p.name = "No square mat".into();
    p.media.retain(|m| m.id != "mat-12x12");
    let first = p.media[0].id.clone();
    s.profiles.save_user_plotter(&p, &base).unwrap();
    s.prefs.default_plotter = "no-square-mat".into();
    run(&mut s, "document.new", json!({}));
    assert_eq!(s.doc().document.job.media_id, first);
    let st = run(&mut s, "document.state", json!({}));
    assert!(st["warnings"].as_array().unwrap().is_empty(), "{st}");
}

#[test]
fn reopening_warns_when_an_svg_rebuilds_to_other_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    import(&mut s, 30.0, 20.0);
    let path = dir.path().join("output").join("fonts.flatcraft");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    run(&mut s, "project.save", json!({ "path": path }));
    let r = run(&mut s, "project.open", json!({ "path": path }));
    let warned = |r: &Value| {
        r["warnings"].as_array().unwrap().iter().any(|w| {
            w.as_str()
                .unwrap()
                .contains("looks different on this computer")
        })
    };
    assert!(!warned(&r), "same bounds, no warning: {r}");
    // As if text in the SVG came out in another font here.
    flatcraft_core::project::Archive::edit(&path, |a| {
        let v = &mut a.project;
        let x1 = v["objects"][0]["local_bounds_mm"]["x1"].as_f64().unwrap();
        v["objects"][0]["local_bounds_mm"]["x1"] = json!(x1 + 0.1);
    })
    .unwrap();
    let r = run(&mut s, "project.open", json!({ "path": path }));
    let w: Vec<&str> = r["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        w.contains(&"rect looks different on this computer (missing fonts?)"),
        "{w:?}"
    );
}

// ---------------------------------------------------------------- distribute and copies

/// The placed bounds the arrange commands work with.
fn cmd_bounds(s: &Session, id: u64) -> Rect {
    let o = s.doc().document.get(ObjectId(id)).unwrap();
    flatcraft_engine::commands::placed_bounds(o, Default::default()).unwrap()
}

/// Imports a w x h rectangle and moves its top-left corner to (x0, y0).
fn rect_at(s: &mut Session, w: f64, h: f64, x0: f64, y0: f64) -> u64 {
    let id = import(s, w, h);
    let b = cmd_bounds(s, id);
    run(
        s,
        "object.move",
        json!({ "ids": [id], "dx_mm": x0 - b.x0, "dy_mm": y0 - b.y0 }),
    );
    id
}

fn undo_len(s: &Session) -> usize {
    s.doc().undo.undo_len()
}

#[test]
fn distribute_horizontal_makes_equal_gaps_with_uneven_widths() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = rect_at(&mut s, 10.0, 10.0, 20.0, 30.0);
    let b = rect_at(&mut s, 30.0, 20.0, 50.0, 60.0);
    let c = rect_at(&mut s, 20.0, 15.0, 130.0, 40.0);
    let d = rect_at(&mut s, 40.0, 10.0, 200.0, 90.0);
    let (ya, yb, yc, yd) = (
        cmd_bounds(&s, a).y0,
        cmd_bounds(&s, b).y0,
        cmd_bounds(&s, c).y0,
        cmd_bounds(&s, d).y0,
    );
    let (first, last) = (cmd_bounds(&s, a), cmd_bounds(&s, d));
    let before = undo_len(&s);
    // Shuffled: the order comes from the bounds centres.
    let r = run(
        &mut s,
        "arrange.distribute",
        json!({ "ids": [c, a, d, b], "axis": "horizontal" }),
    );
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    let order: Vec<u64> = r["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert_eq!(order, vec![a, b, c, d]);
    // The ends stay exactly where they were.
    assert_eq!(cmd_bounds(&s, a), first);
    assert_eq!(cmd_bounds(&s, d), last);
    let bs: Vec<Rect> = order.iter().map(|id| cmd_bounds(&s, *id)).collect();
    let gaps: Vec<f64> = bs.windows(2).map(|w| w[1].x0 - w[0].x1).collect();
    let gap = r["gap_mm"].as_f64().unwrap();
    for g in &gaps {
        assert!((g - gap).abs() < EPS, "gaps {gaps:?}, gap_mm {gap}");
    }
    // 220 mm span, 100 mm of objects, 3 gaps.
    assert!((gap - 40.0).abs() < 1e-4, "{gap}");
    // Only x moves.
    for (id, y) in [(a, ya), (b, yb), (c, yc), (d, yd)] {
        assert_eq!(cmd_bounds(&s, id).y0, y);
    }
    run(&mut s, "edit.undo", json!({}));
    assert!(close(cmd_bounds(&s, b).x0, 50.0) && close(cmd_bounds(&s, c).x0, 130.0));
}

#[test]
fn distribute_vertical_uses_the_selection() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = rect_at(&mut s, 10.0, 12.0, 20.0, 20.0);
    let b = rect_at(&mut s, 10.0, 33.0, 60.0, 150.0);
    let c = rect_at(&mut s, 10.0, 7.0, 40.0, 50.0);
    let (top, bottom) = (cmd_bounds(&s, a), cmd_bounds(&s, b));
    run(&mut s, "object.select", json!({ "ids": [a, b, c] }));
    let before = undo_len(&s);
    let r = run(&mut s, "arrange.distribute", json!({ "axis": "vertical" }));
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(cmd_bounds(&s, a), top);
    assert_eq!(cmd_bounds(&s, b), bottom);
    let mid = cmd_bounds(&s, c);
    let gap = r["gap_mm"].as_f64().unwrap();
    assert!((mid.y0 - top.y1 - gap).abs() < EPS, "{mid:?} {gap}");
    assert!((bottom.y0 - mid.y1 - gap).abs() < EPS, "{mid:?} {gap}");
    assert!(close(mid.x0, 40.0), "x stays");
}

#[test]
fn distribute_ties_keep_the_given_order_and_wide_objects_overlap_evenly() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = rect_at(&mut s, 10.0, 10.0, 20.0, 20.0);
    let b = rect_at(&mut s, 60.0, 10.0, 30.0, 40.0);
    let c = rect_at(&mut s, 60.0, 10.0, 30.0, 60.0);
    let d = rect_at(&mut s, 10.0, 10.0, 70.0, 80.0);
    // b and c share their centre: c was named first, so it comes first.
    let r = run(
        &mut s,
        "arrange.distribute",
        json!({ "ids": [a, c, b, d], "axis": "horizontal" }),
    );
    assert_eq!(r["ids"], json!([a, c, b, d]));
    let gap = r["gap_mm"].as_f64().unwrap();
    // 60 mm span, 140 mm of objects: the gaps are negative.
    assert!((gap - (-80.0 / 3.0)).abs() < 1e-4, "{gap}");
    let bs: Vec<Rect> = [a, c, b, d].iter().map(|id| cmd_bounds(&s, *id)).collect();
    for w in bs.windows(2) {
        assert!((w[1].x0 - w[0].x1 - gap).abs() < EPS, "{bs:?}");
    }
}

#[test]
fn distribute_refuses_fewer_than_three() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = rect_at(&mut s, 10.0, 10.0, 20.0, 20.0);
    let b = rect_at(&mut s, 10.0, 10.0, 80.0, 20.0);
    let before = undo_len(&s);
    let (ba, bb) = (cmd_bounds(&s, a), cmd_bounds(&s, b));
    for params in [
        json!({ "ids": [a, b], "axis": "horizontal" }),
        json!({ "ids": [a, a, b], "axis": "vertical" }),
    ] {
        let e = s
            .execute("arrange.distribute", params, Origin::Agent)
            .unwrap_err();
        assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
        assert!(e.to_string().contains("at least 3 objects"), "{e}");
    }
    run(&mut s, "object.select", json!({ "ids": [a, b] }));
    let e = s
        .execute(
            "arrange.distribute",
            json!({ "axis": "horizontal" }),
            Origin::User,
        )
        .unwrap_err();
    assert!(e.to_string().contains("2 are selected"), "{e}");
    assert_eq!(undo_len(&s), before);
    assert_eq!((cmd_bounds(&s, a), cmd_bounds(&s, b)), (ba, bb));
}

#[test]
fn distribute_of_evenly_spaced_objects_is_no_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let ids = [
        rect_at(&mut s, 10.0, 10.0, 20.0, 20.0),
        rect_at(&mut s, 10.0, 10.0, 50.0, 20.0),
        rect_at(&mut s, 10.0, 10.0, 80.0, 20.0),
    ];
    run(
        &mut s,
        "arrange.distribute",
        json!({ "ids": ids, "axis": "horizontal" }),
    );
    let before = undo_len(&s);
    run(
        &mut s,
        "arrange.distribute",
        json!({ "ids": ids, "axis": "horizontal" }),
    );
    assert_eq!(undo_len(&s), before);
}

fn ids_of(r: &Value) -> Vec<u64> {
    r["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect()
}

#[test]
fn copies_in_a_row_start_right_of_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = rect_at(&mut s, 50.0, 20.0, 20.0, 30.0);
    let orig = cmd_bounds(&s, id);
    let before = undo_len(&s);
    let r = run(
        &mut s,
        "object.copies",
        json!({ "id": id, "count": 3, "layout": "row" }),
    );
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    let ids = ids_of(&r);
    assert_eq!(ids.len(), 3);
    assert_eq!(s.doc().document.objects.len(), 4);
    for (i, c) in ids.iter().enumerate() {
        let b = cmd_bounds(&s, *c);
        let k = (i + 1) as f64;
        // Default gap 5 mm. The width carries usvg's f32 noise.
        let step = orig.width() + 5.0;
        assert!((b.x0 - (orig.x0 + k * step)).abs() < EPS, "{i}: {b:?}");
        assert!((b.y0 - orig.y0).abs() < EPS, "{i}: {b:?}");
    }
    let sel: Vec<u64> = s.doc().selection.iter().map(|i| i.0).collect();
    assert_eq!(sel, ids, "the copies are selected");
    assert_eq!(cmd_bounds(&s, id), orig, "the original stays");
    assert_eq!(r["outside_area"], json!([]));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().document.objects.len(), 1);
}

#[test]
fn copies_in_a_grid_fill_rows_then_go_down() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = rect_at(&mut s, 30.0, 20.0, 10.0, 10.0);
    let orig = cmd_bounds(&s, id);
    let before = undo_len(&s);
    let r = run(
        &mut s,
        "object.copies",
        json!({ "id": id, "count": 5, "layout": { "grid": { "columns": 3 } }, "gap_mm": 2.0 }),
    );
    assert_eq!(undo_len(&s), before + 1);
    let ids = ids_of(&r);
    // The original is cell 0 of the grid.
    let cells = [(1, 0), (2, 0), (0, 1), (1, 1), (2, 1)];
    assert_eq!(ids.len(), cells.len());
    for (c, (col, row)) in ids.iter().zip(cells) {
        let b = cmd_bounds(&s, *c);
        let (sx, sy) = (orig.width() + 2.0, orig.height() + 2.0);
        assert!((b.x0 - (orig.x0 + col as f64 * sx)).abs() < EPS, "{b:?}");
        assert!((b.y0 - (orig.y0 + row as f64 * sy)).abs() < EPS, "{b:?}");
    }
}

#[test]
fn copies_outside_the_cutting_area_are_added_and_named() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = rect_at(&mut s, 50.0, 50.0, 20.0, 20.0);
    let r = run(&mut s, "object.copies", json!({ "id": id, "count": 6 }));
    let ids = ids_of(&r);
    assert_eq!(ids.len(), 6);
    let outside: Vec<u64> = r["outside_area"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    // The cutting area ends at 299.8 mm: copies 5 and 6 start at 295 and 350.
    assert_eq!(outside, ids[4..].to_vec(), "{r}");
    let warnings = r["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{r}");
    assert!(
        warnings[0]
            .as_str()
            .unwrap()
            .starts_with("2 copies are outside the cutting area"),
        "{r}"
    );
}

#[test]
fn copies_refuse_bad_params() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 20.0, 20.0);
    let before = undo_len(&s);
    // false is accepted (true too, see tests/linked_copies.rs).
    run(
        &mut s,
        "object.copies",
        json!({ "id": id, "count": 1, "linked": false }),
    );
    run(&mut s, "edit.undo", json!({}));
    for params in [
        json!({ "id": id, "count": 0 }),
        json!({ "id": id, "count": 501 }),
        json!({ "id": id, "count": 2, "gap_mm": -1.0 }),
        json!({ "id": id, "count": 2, "layout": { "grid": { "columns": 0 } } }),
        json!({ "id": id, "count": 2, "layout": { "grid": { "columns": 2, "rows": 2 } } }),
        json!({ "id": id, "count": 2, "layout": "column" }),
        json!({ "id": id, "count": 2, "spacing_mm": 3.0 }),
    ] {
        let e = s
            .execute("object.copies", params.clone(), Origin::Agent)
            .expect_err(&format!("{params}"));
        assert!(matches!(e, CommandError::InvalidParams(_)), "{params}: {e}");
    }
    let e = s
        .execute(
            "object.copies",
            json!({ "id": 999, "count": 2 }),
            Origin::Agent,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::NotFound(_)), "{e}");
    assert_eq!(undo_len(&s), before);
    assert_eq!(s.doc().document.objects.len(), 1);
}

#[test]
fn copies_that_would_land_too_far_away_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 20.0, 20.0);
    let before = undo_len(&s);
    let e = s
        .execute(
            "object.copies",
            json!({ "id": id, "count": 500, "gap_mm": 5000.0 }),
            Origin::Agent,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "{e}");
    assert!(e.to_string().contains("1 km"), "{e}");
    assert_eq!(undo_len(&s), before);
    assert_eq!(s.doc().document.objects.len(), 1);
}

/// Rotates object `id` by `deg` about its centre.
fn rotate(s: &mut Session, id: u64, deg: f64) {
    run(s, "object.rotate", json!({ "ids": [id], "degrees": deg }));
}

#[test]
fn distribute_and_copies_use_the_bounds_of_rotated_objects() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = rect_at(&mut s, 30.0, 10.0, 20.0, 20.0);
    let b = rect_at(&mut s, 50.0, 20.0, 90.0, 60.0);
    let c = rect_at(&mut s, 20.0, 40.0, 220.0, 30.0);
    rotate(&mut s, a, 30.0);
    rotate(&mut s, b, -45.0);
    rotate(&mut s, c, 72.0);
    let r = run(
        &mut s,
        "arrange.distribute",
        json!({ "ids": [a, b, c], "axis": "horizontal" }),
    );
    let gap = r["gap_mm"].as_f64().unwrap();
    let bs: Vec<Rect> = ids_of(&r).iter().map(|id| cmd_bounds(&s, *id)).collect();
    for w in bs.windows(2) {
        assert!((w[1].x0 - w[0].x1 - gap).abs() < EPS, "{bs:?} {gap}");
    }

    let orig = cmd_bounds(&s, b);
    let r = run(
        &mut s,
        "object.copies",
        json!({ "id": b, "count": 3, "layout": { "grid": { "columns": 2 } }, "gap_mm": 4.0 }),
    );
    let ids = ids_of(&r);
    let first = cmd_bounds(&s, ids[0]);
    let below = cmd_bounds(&s, ids[1]);
    assert!((first.x0 - orig.x1 - 4.0).abs() < EPS, "{orig:?} {first:?}");
    assert!((below.y0 - orig.y1 - 4.0).abs() < EPS, "{orig:?} {below:?}");
    assert!((below.x0 - orig.x0).abs() < EPS, "{orig:?} {below:?}");
}

#[test]
fn copies_description_names_the_refusal_and_the_shared_link() {
    let d = registry::get("object.copies").unwrap().description();
    assert!(d.contains("refuses the whole call"), "{d}");
    assert!(d.contains("file link"), "{d}");
}

/// A reopened .svgz keeps its text: the stored bytes are still gzipped,
/// and the rebuild must load the fonts for them anyway. The text is in
/// IBM Plex Sans, which is always in the font database.
#[test]
fn a_reopened_svgz_keeps_its_text() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="40mm" viewBox="0 0 100 40"><text x="5" y="30" font-family="IBM Plex Sans" font-size="24">Hi</text></svg>"#;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(svg.as_bytes()).unwrap();
    let file = dir.path().join("hi.svgz");
    std::fs::write(&file, gz.finish().unwrap()).unwrap();
    let r = run(&mut s, "object.import", json!({ "path": file }));
    let id = ObjectId(r["id"].as_u64().unwrap());
    let paths = |s: &Session| s.doc().document.get(id).unwrap().geometry.len();
    // IBM Plex Sans ships with FlatCraft, so the text draws everywhere.
    assert_eq!(r["warnings"], json!([]), "{r}");
    let before = paths(&s);
    assert!(before > 0, "the text was drawn");
    let project = dir.path().join("hi.flatcraft");
    run(&mut s, "project.save", json!({ "path": project }));
    run(&mut s, "document.new", json!({}));
    let opened = run(&mut s, "project.open", json!({ "path": project }));
    assert_eq!(opened["warnings"], json!([]), "{opened}");
    assert_eq!(paths(&s), before, "the text is still there");
    let reloaded = run(&mut s, "object.reload", json!({ "ids": [id.0] }));
    assert_eq!(reloaded["warnings"], json!([]), "{reloaded}");
    assert_eq!(paths(&s), before);
    // The Home picture's path: load without opening, then draw.
    let doc = flatcraft_engine::commands::project::load_rebuilt(&s.profiles, &project).unwrap();
    assert_eq!(doc.objects[0].geometry.len(), before);
    let plotter = s.profiles.plotter(&doc.job.plotter_id).unwrap();
    let png = flatcraft_engine::preview::render_document_png(&plotter, &doc, 128).unwrap();
    assert_eq!(&png[1..4], b"PNG");
}

#[test]
fn move_to_x_or_y_sets_the_top_left_corner() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 40.0, 20.0);
    let before = state_bounds(&mut s, id);
    // X alone keeps Y.
    run(&mut s, "object.move", json!({ "ids": [id], "x_mm": 12.5 }));
    let b = state_bounds(&mut s, id);
    assert!(close(b.x0, 12.5) && close(b.y0, before.y0), "{b:?}");
    run(&mut s, "object.move", json!({ "ids": [id], "y_mm": 7.0 }));
    let b = state_bounds(&mut s, id);
    assert!(close(b.x0, 12.5) && close(b.y0, 7.0), "{b:?}");
    // A rotated object: the box of the rotated shape.
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [id], "degrees": 30.0 }),
    );
    run(
        &mut s,
        "object.move",
        json!({ "ids": [id], "x_mm": 30.0, "y_mm": 40.0 }),
    );
    let b = state_bounds(&mut s, id);
    assert!(close(b.x0, 30.0) && close(b.y0, 40.0), "{b:?}");
    // Not with a relative move or a centre.
    for bad in [
        json!({ "ids": [id], "x_mm": 1.0, "dx_mm": 1.0 }),
        json!({ "ids": [id], "y_mm": 1.0, "center_mm": [0.0, 0.0] }),
    ] {
        assert!(matches!(
            s.execute("object.move", bad, Origin::User),
            Err(CommandError::InvalidParams(_))
        ));
    }
}

#[test]
fn move_to_x_moves_a_multi_selection_as_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = import(&mut s, 20.0, 20.0);
    let b = import(&mut s, 20.0, 20.0);
    run(&mut s, "object.move", json!({ "ids": [b], "dx_mm": 50.0 }));
    let (ba, bb) = (state_bounds(&mut s, a), state_bounds(&mut s, b));
    let undo = s.doc().undo.undo_len();
    run(&mut s, "object.move", json!({ "ids": [a, b], "x_mm": 5.0 }));
    assert_eq!(s.doc().undo.undo_len(), undo + 1, "one undo step");
    let (na, nb) = (state_bounds(&mut s, a), state_bounds(&mut s, b));
    assert!(close(na.x0, 5.0), "{na:?}");
    assert!(close(nb.x0 - na.x0, bb.x0 - ba.x0), "the layout stays");
    assert!(close(na.y0, ba.y0) && close(nb.y0, bb.y0));
}

#[test]
fn a_side_anchor_keeps_the_middle_of_that_side() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 40.0, 20.0);
    let b = state_bounds(&mut s, id);
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [id], "factor": 2.0, "anchor": "left" }),
    );
    let n = state_bounds(&mut s, id);
    assert!(
        close(n.x0, b.x0) && close(n.center().y, b.center().y),
        "{n:?}"
    );
    assert!(close(n.width(), 80.0), "{n:?}");
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [id], "factor": 0.5, "anchor": "bottom" }),
    );
    let m = state_bounds(&mut s, id);
    assert!(
        close(m.y1, n.y1) && close(m.center().x, n.center().x),
        "{m:?}"
    );
}

#[test]
fn a_straight_group_scales_width_and_height_apart() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = import(&mut s, 40.0, 20.0);
    let b = import(&mut s, 40.0, 20.0);
    let c = import(&mut s, 30.0, 10.0);
    run(&mut s, "object.move", json!({ "ids": [b], "dx_mm": 60.0 }));
    run(&mut s, "object.move", json!({ "ids": [c], "dy_mm": 50.0 }));
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [b], "degrees": 90.0, "about": "each" }),
    );
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [c], "degrees": 180.0, "about": "each" }),
    );
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [c], "axis": "horizontal", "about": "each" }),
    );
    let ids = [a, b, c];
    let olds: Vec<Rect> = ids.iter().map(|i| state_bounds(&mut s, *i)).collect();
    let all = olds.iter().copied().reduce(|x, y| x.union(y)).unwrap();
    let undo = s.doc().undo.undo_len();
    run(
        &mut s,
        "object.scale",
        json!({
            "ids": ids,
            "width_mm": all.width() * 2.0,
            "height_mm": all.height() * 0.5,
            "keep_aspect": false,
            "anchor": "top_left",
            "group": true,
        }),
    );
    assert_eq!(s.doc().undo.undo_len(), undo + 1);
    for (id, old) in ids.iter().zip(&olds) {
        let n = state_bounds(&mut s, *id);
        let want = Rect::new(
            all.x0 + (old.x0 - all.x0) * 2.0,
            all.y0 + (old.y0 - all.y0) * 0.5,
            all.x0 + (old.x1 - all.x0) * 2.0,
            all.y0 + (old.y1 - all.y0) * 0.5,
        );
        assert!(
            close(n.x0, want.x0)
                && close(n.y0, want.y0)
                && close(n.x1, want.x1)
                && close(n.y1, want.y1),
            "{id}: {n:?} want {want:?}"
        );
    }
    // One object turned off a multiple of 90 degrees locks the group.
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a], "degrees": 30.0, "about": "each" }),
    );
    let err = s.execute(
        "object.scale",
        json!({ "ids": ids, "width_mm": 300.0, "height_mm": 50.0, "keep_aspect": false, "group": true }),
        Origin::User,
    );
    assert!(
        matches!(err, Err(CommandError::InvalidParams(_))),
        "{err:?}"
    );
}

#[test]
fn execute_in_runs_in_another_document_and_keeps_the_active_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 40.0, 20.0);
    let first = s.doc().id();
    run(&mut s, "document.new", json!({}));
    let second = s.doc().id();
    s.execute_in(
        first,
        "object.move",
        json!({ "ids": [id], "x_mm": 5.0 }),
        Origin::User,
    )
    .unwrap();
    assert_eq!(s.doc().id(), second, "the active document stays");
    assert!(s.doc().undo.undo_len() == 0);
    let a = &s.docs[s.doc_index(first).unwrap()];
    assert_eq!(a.undo.undo_len(), 2, "the import and the move");
    assert!(
        s.execute_in(
            999,
            "object.move",
            json!({ "ids": [id], "x_mm": 1.0 }),
            Origin::User
        )
        .is_err()
    );
}

#[test]
fn a_group_free_scale_refuses_linked_copies_turned_two_ways() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = import(&mut s, 40.0, 20.0);
    let r = run(
        &mut s,
        "object.copies",
        json!({ "id": a, "count": 1, "linked": true }),
    );
    let b = r["ids"][0].as_u64().unwrap();
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [b], "degrees": 90.0, "about": "each" }),
    );
    let undo = s.doc().undo.undo_len();
    let (ba, bb) = (state_bounds(&mut s, a), state_bounds(&mut s, b));
    let err = s.execute(
        "object.scale",
        json!({ "ids": [a, b], "width_mm": 200.0, "height_mm": 30.0, "keep_aspect": false, "group": true }),
        Origin::User,
    );
    match err {
        Err(CommandError::InvalidParams(m)) => assert!(m.contains("linked copy"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.doc().undo.undo_len(), undo, "nothing changed");
    assert_eq!(state_bounds(&mut s, a), ba);
    assert_eq!(state_bounds(&mut s, b), bb);
    // Turned the same way, they stretch together.
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a], "degrees": 90.0, "about": "each" }),
    );
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [a, b], "width_mm": 200.0, "keep_aspect": false, "group": true }),
    );
}

#[test]
fn rotate_to_an_absolute_angle_turns_each_about_its_centre() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = import(&mut s, 20.0, 10.0);
    let b = import(&mut s, 30.0, 10.0);
    run(&mut s, "object.move", json!({ "ids": [b], "dx_mm": 60.0 }));
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a], "degrees": 30.0 }),
    );
    let centre = |s: &Session, id: u64| s.doc().document.get(ObjectId(id)).unwrap().transform;
    let (ca, cb) = (centre(&s, a).center_mm, centre(&s, b).center_mm);
    let steps = s.doc().undo.undo_len();
    // -45 is 315 clockwise; both get it, each about its own centre.
    let r = run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a, b], "angle_deg": -45.0 }),
    );
    assert_eq!(r["objects"].as_array().unwrap().len(), 2);
    assert_eq!(s.doc().undo.undo_len(), steps + 1);
    for (id, c) in [(a, ca), (b, cb)] {
        let t = centre(&s, id);
        assert!(close(t.rotation_deg, 315.0), "{t:?}");
        assert!(
            close(t.center_mm.x, c.x) && close(t.center_mm.y, c.y),
            "{t:?}"
        );
    }
    // The same angle again changes nothing and adds no undo step.
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a, b], "angle_deg": 315.0 }),
    );
    assert_eq!(s.doc().undo.undo_len(), steps + 1);
    // about: each is allowed.
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a], "angle_deg": 370.0, "about": "each" }),
    );
    assert!(close(centre(&s, a).rotation_deg, 10.0));
}

#[test]
fn rotate_needs_exactly_one_of_degrees_and_angle() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = import(&mut s, 20.0, 10.0);
    for params in [
        json!({ "ids": [a] }),
        json!({ "ids": [a], "degrees": 10.0, "angle_deg": 20.0 }),
        json!({ "ids": [a], "angle_deg": 20.0, "about": "group" }),
        json!({ "ids": [a], "angle_deg": "north" }),
    ] {
        let e = s
            .execute("object.rotate", params.clone(), Origin::User)
            .unwrap_err();
        assert!(matches!(e, CommandError::InvalidParams(_)), "{params}: {e}");
    }
    // Relative still works, with the default about.
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a], "degrees": 10.0 }),
    );
    let t = s.doc().document.get(ObjectId(a)).unwrap().transform;
    assert!(close(t.rotation_deg, 10.0));
}

#[test]
fn any_finite_angle_is_reduced_by_full_turns() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = import(&mut s, 20.0, 10.0);
    let angle = |s: &Session| {
        s.doc()
            .document
            .get(ObjectId(a))
            .unwrap()
            .transform
            .rotation_deg
    };
    for (given, want) in [
        (735.0, 15.0),
        (-90.0, 270.0),
        (1080.0, 0.0),
        (359.999, 359.999),
        (-0.0, 0.0),
        (1e9, 280.0),
        (-1e9, 80.0),
    ] {
        run(
            &mut s,
            "object.rotate",
            json!({ "ids": [a], "angle_deg": given }),
        );
        assert!(close(angle(&s), want), "{given}: {}", angle(&s));
        assert!(angle(&s).is_sign_positive(), "{given}");
    }
    // Relative turns too.
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a], "angle_deg": 0.0 }),
    );
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a], "degrees": 735.0 }),
    );
    assert!(close(angle(&s), 15.0));
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a], "degrees": -1e9 }),
    );
    assert!(close(angle(&s), 95.0), "{}", angle(&s));
}

#[test]
fn angles_too_large_to_be_exact_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = import(&mut s, 20.0, 10.0);
    let steps = s.doc().undo.undo_len();
    for params in [
        json!({ "ids": [a], "angle_deg": 1e10 }),
        json!({ "ids": [a], "angle_deg": -1.5e9 }),
        json!({ "ids": [a], "degrees": 1e10 }),
    ] {
        let e = s
            .execute("object.rotate", params.clone(), Origin::User)
            .unwrap_err();
        assert!(matches!(e, CommandError::InvalidParams(_)), "{params}: {e}");
        assert!(e.to_string().contains("1000000000"), "{e}");
    }
    assert_eq!(s.doc().undo.undo_len(), steps);
}

/// JSON has no NaN or infinity, so the engine checks them on the typed
/// value itself.
#[test]
fn a_non_finite_angle_is_refused() {
    use flatcraft_engine::commands::object::check_angle;
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(check_angle(v).is_err(), "{v}");
    }
    assert!(check_angle(1e9).is_ok());
}

// ---------------------------------------------------------------- project file thumbnail

/// A save, also an agent's, puts a 512 px picture of the mat into the
/// project file.
#[test]
fn project_save_embeds_a_thumbnail_that_decodes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    import(&mut s, 30.0, 20.0);
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("pic.flatcraft");
    s.execute("project.save", json!({ "path": path }), Origin::Agent)
        .unwrap();
    let png = flatcraft_core::project::read_thumbnail(&path)
        .unwrap()
        .expect("a thumbnail");
    let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap();
    assert_eq!(img.width().max(img.height()), 512);
    // The project itself is a zip with the FlatCraft mimetype.
    let raw = std::fs::read(&path).unwrap();
    assert_eq!(&raw[0..4], b"PK\x03\x04");
    assert_eq!(&raw[30..38], b"mimetype");
}

/// How long the 512 px thumbnail takes in a save of 500 traced objects.
/// A measurement, not a check: run it in release mode with
/// `cargo nextest run --release -p flatcraft-engine --run-ignored only thumbnail_timing`.
#[test]
#[ignore = "timing measurement"]
fn thumbnail_timing_for_500_traced_objects() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let mut img = image::GrayImage::from_pixel(600, 600, image::Luma([255]));
    for row in 0..6 {
        for col in 0..6 {
            for y in 0..60 {
                for x in 0..60 {
                    let (x0, y0) = (20 + col * 95, 20 + row * 95);
                    let inside = (x as i32 - 30).pow(2) + (y as i32 - 30).pow(2) < 900;
                    if inside {
                        img.put_pixel(x0 + x, y0 + y, image::Luma([0]));
                    }
                }
            }
        }
    }
    let png = dir.path().join("dots.png");
    img.save(&png).unwrap();
    let id = run(&mut s, "object.import", json!({ "path": png }))["id"]
        .as_u64()
        .unwrap();
    run(
        &mut s,
        "object.copies",
        json!({ "id": id, "count": 499, "gap_mm": 1.0, "layout": { "grid": { "columns": 25 } } }),
    );
    let doc = s.doc().document.clone();
    assert_eq!(doc.objects.len(), 500);
    let paths: usize = doc.objects.iter().map(|o| o.geometry.len()).sum();
    let t = std::time::Instant::now();
    let placed: usize = doc.objects.iter().map(|o| o.placed_paths().len()).sum();
    let transform = t.elapsed();
    let mut empty = doc.clone();
    empty.objects.clear();
    let t = std::time::Instant::now();
    flatcraft_engine::commands::project::thumbnail_png(&s.profiles, &empty).unwrap();
    let mat = t.elapsed();
    eprintln!("TIMING placing {placed} paths {transform:?}, empty mat {mat:?}");
    let t = std::time::Instant::now();
    let pic = flatcraft_engine::commands::project::thumbnail_png(&s.profiles, &doc).unwrap();
    let render = t.elapsed();
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("big.flatcraft");
    let t = std::time::Instant::now();
    run(&mut s, "project.save", json!({ "path": path }));
    let save = t.elapsed();
    eprintln!(
        "TIMING 500 objects, {paths} paths: thumbnail {render:?} ({} bytes), whole project.save {save:?}",
        pic.len()
    );
}

/// The slow part of a save runs without the session lock: a change made
/// meanwhile is not in the file and keeps the document dirty.
#[test]
fn a_change_during_an_unlocked_save_keeps_the_document_dirty() {
    use flatcraft_engine::commands::project::{SaveParams, begin_save, finish_save};
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = import(&mut s, 10.0, 10.0);
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("p.flatcraft");
    let params: SaveParams = serde_json::from_value(json!({ "path": path })).unwrap();
    let job = begin_save(&mut s, params, Origin::User).unwrap();
    // Without the lock: someone moves the object.
    run(
        &mut s,
        "object.move",
        json!({ "ids": [id], "dx_mm": 20.0, "dy_mm": 0.0 }),
    );
    let written = job.write();
    finish_save(&mut s, job, written).unwrap();
    assert_eq!(s.doc().path.as_deref(), Some(path.as_path()));
    assert!(s.doc().dirty, "the move is not in the file");
    let saved = flatcraft_core::project::load(&path).unwrap();
    let t = saved.get(ObjectId(id)).unwrap().transform;
    let now = s.doc().document.get(ObjectId(id)).unwrap().transform;
    assert!((now.center_mm.x - t.center_mm.x - 20.0).abs() < 1e-6);

    // Without a change in between, the save leaves it clean.
    let session = std::sync::Mutex::new(s);
    flatcraft_engine::commands::project::save_unlocked(&session, json!({}), Origin::User).unwrap();
    assert!(!session.lock().unwrap().doc().dirty);
}
