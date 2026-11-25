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
    // the 20 mm square's corner app (0,0) = device y 12 mm, x 10 mm = D240,200 units (y first),
    // reached from the path start point M640,200; Z is the physical mat
    assert!(text.contains("M640,200\x03D240,200,240,600,"), "{text}");
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
    assert_eq!(m["color"], "#8FA8B8");
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
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["objects"][0]["source"]["Raster"]["trace"]["min_feature_mm"] = json!(5.0);
    std::fs::write(&path, v.to_string()).unwrap();
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
    use base64::Engine as _;
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["objects"][0]["source"]["Svg"]["bytes"] =
        json!(base64::engine::general_purpose::STANDARD.encode(one));
    std::fs::write(&path, v.to_string()).unwrap();
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
    use base64::Engine as _;
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="10" y="10" width="10" height="20"/></svg>"#;
    let bytes = base64::engine::general_purpose::STANDARD.encode(svg);
    let text = format!(
        r#"{{"format_version":1,"objects":[{{"id":1,"name":"box","source":{{"Svg":{{"name":"box.svg","bytes":"{bytes}"}}}},"transform":{transform},"visible":true{extra}}}],"job":{{"plotter_id":"cameo2","material_id":"cardstock","media_id":"a4","speed":null,"force":null,"passes":null}},"next_id":2}}"#
    );
    let path = dir.path().join("hand.flatcraft");
    std::fs::write(&path, text).unwrap();
    path
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
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let png = flatcraft_engine::preview::render_png(
        &s,
        flatcraft_engine::preview::PreviewKind::Canvas,
        512,
    )
    .unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    // a corner of the physical mat, outside the grid
    let (w, h) = img.dimensions();
    let px = img.get_pixel(w / 2, h - 3);
    assert_eq!(&px.0[..3], &[0x8F, 0xA8, 0xB8]);
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
    let e = s
        .execute(
            "object.scale",
            json!({ "ids": [a, b], "width_mm": 50.0, "keep_aspect": false, "group": true }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
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
