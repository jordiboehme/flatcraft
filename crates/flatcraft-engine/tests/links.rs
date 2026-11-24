//! Linked sources: import records the file, reload, relink, unlink and the
//! change check. A missing or broken file never breaks a project.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use flatcraft_core::document::{Object, ObjectId, Source};
use flatcraft_device::ProfileStore;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{CommandError, Origin, Session};
use kurbo::Point;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn session(dir: &Path) -> Session {
    let profiles = ProfileStore::load(dir).unwrap();
    // Never the real Documents folder.
    let prefs = Preferences {
        output_dir: Some(dir.join("output")),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, false, dir.to_path_buf())
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

/// A 200 x 200 mm page with a 50 x 50 mm square at (20, 20); `corner` adds
/// a 5 mm square at the page's top-left after it.
fn svg(corner: bool) -> String {
    let extra = if corner {
        r#"<rect x="0" y="0" width="5" height="5"/>"#
    } else {
        ""
    };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="20" y="20" width="50" height="50"/>{extra}</svg>"#
    )
}

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

/// Rewrites a file and moves its modification time forward, so the change
/// check sees it even on file systems with coarse timestamps.
fn change(path: &Path, bytes: impl AsRef<[u8]>) {
    write(path, bytes);
    bump_mtime(path);
}

fn bump_mtime(path: &Path) {
    let f = std::fs::File::options().write(true).open(path).unwrap();
    f.set_modified(SystemTime::now() + Duration::from_secs(60))
        .unwrap();
}

fn import(s: &mut Session, path: &Path) -> u64 {
    run(s, "object.import", json!({ "path": path }))["id"]
        .as_u64()
        .unwrap()
}

fn obj(s: &Session, id: u64) -> &Object {
    s.doc().document.get(ObjectId(id)).unwrap()
}

fn bytes_of(o: &Object) -> &[u8] {
    match &o.source {
        Source::Svg { bytes, .. } | Source::Raster { bytes, .. } => bytes,
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn ids(v: &Value) -> Vec<u64> {
    let mut out: Vec<u64> = v
        .as_array()
        .unwrap_or_else(|| panic!("not a list: {v}"))
        .iter()
        .map(|x| x.as_u64().unwrap())
        .collect();
    out.sort_unstable();
    out
}

/// The placed points of the first path of an object.
fn first_path_points(o: &Object) -> Vec<Point> {
    o.placed_paths()[0]
        .path
        .elements()
        .iter()
        .filter_map(|e| e.end_point())
        .collect()
}

fn assert_points_close(a: &[Point], b: &[Point]) {
    assert_eq!(a.len(), b.len());
    for (p, q) in a.iter().zip(b) {
        assert!((*p - *q).hypot() < 1e-6, "{p:?} vs {q:?}");
    }
}

/// A 300 x 300 px PNG, white with black squares (x, y, size) in px.
fn png(path: &Path, squares: &[(u32, u32, u32)]) {
    let mut img = image::GrayImage::from_pixel(300, 300, image::Luma([255]));
    for &(x0, y0, size) in squares {
        for y in y0..y0 + size {
            for x in x0..x0 + size {
                img.put_pixel(x, y, image::Luma([0]));
            }
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    img.save(path).unwrap();
}

// ---------------------------------------------------------------- import

#[test]
fn import_records_the_link() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut s = session(root);
    let project = root.join("proj").join("p.flatcraft");
    std::fs::create_dir_all(project.parent().unwrap()).unwrap();
    run(&mut s, "project.save", json!({ "path": project }));
    let file = root.join("proj").join("art").join("a.svg");
    write(&file, svg(false));
    let id = import(&mut s, &file);

    let link = obj(&s, id).link.clone().expect("a link");
    assert_eq!(link.path, std::path::absolute(&file).unwrap());
    assert!(link.path.is_absolute());
    assert_eq!(link.relative, Some(PathBuf::from("art").join("a.svg")));
    assert_eq!(link.sha256, hex_sha256(svg(false).as_bytes()));
    let modified = link.modified.expect("modification time");
    assert!(
        modified.ends_with('Z') && modified.contains('T'),
        "{modified}"
    );

    let state = run(&mut s, "document.state", json!({}));
    assert_eq!(
        state["objects"][0]["link"]["relative"],
        json!("art/a.svg"),
        "{state}"
    );

    // Without a project path there is no relative path.
    run(&mut s, "document.new", json!({}));
    let id = import(&mut s, &file);
    let link = obj(&s, id).link.clone().unwrap();
    assert_eq!(link.relative, None);

    // SVG given as text has no file.
    let r = run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg(false), "name": "text" }),
    );
    assert!(obj(&s, r["id"].as_u64().unwrap()).link.is_none());
    let state = run(&mut s, "document.state", json!({}));
    assert!(state["objects"][1]["link"].is_null(), "{state}");
}

// ---------------------------------------------------------------- reload

#[test]
fn reload_picks_up_changes_and_keeps_transform() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let file = dir.path().join("a.svg");
    write(&file, svg(false));
    let id = import(&mut s, &file);
    run(
        &mut s,
        "object.move",
        json!({ "ids": [id], "dx_mm": 40.0, "dy_mm": 30.0 }),
    );
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [id], "degrees": 30.0 }),
    );
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [id], "factor": 1.5 }),
    );
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [id], "axis": "horizontal" }),
    );
    run(
        &mut s,
        "object.set_cut_options",
        json!({ "ids": [id], "mode": "all_lines", "auto_threshold_mm": 1.0 }),
    );
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [id], "visible": false }),
    );
    let before = obj(&s, id).clone();
    let square_before = first_path_points(&before);

    change(&file, svg(true));
    let r = run(&mut s, "object.reload", json!({ "ids": [id] }));
    assert_eq!(ids(&r["reloaded"]), vec![id], "{r}");
    assert_eq!(ids(&r["missing"]), Vec::<u64>::new());
    assert_eq!(ids(&r["unchanged"]), Vec::<u64>::new());

    let o = obj(&s, id);
    assert_eq!(o.geometry.len(), 2, "the new corner shape is there");
    assert_eq!(bytes_of(o), svg(true).as_bytes());
    let t = (o.transform, before.transform);
    assert_eq!(
        (t.0.scale_x, t.0.scale_y, t.0.rotation_deg, t.0.mirror),
        (t.1.scale_x, t.1.scale_y, t.1.rotation_deg, t.1.mirror)
    );
    assert_eq!(o.cut, before.cut);
    assert_eq!((o.name.as_str(), o.visible), ("a", false));
    // The unchanged square stays where it was on the mat.
    assert_points_close(&first_path_points(o), &square_before);
    let link = o.link.clone().unwrap();
    assert_eq!(link.sha256, hex_sha256(svg(true).as_bytes()));

    // Nothing changed since: unchanged.
    let r = run(&mut s, "object.reload", json!({ "ids": [] }));
    assert_eq!(ids(&r["unchanged"]), vec![id], "{r}");
    assert_eq!(ids(&r["reloaded"]), Vec::<u64>::new());
}

#[test]
fn reload_raster_retraces_with_stored_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let file = dir.path().join("pic.png");
    png(&file, &[(50, 50, 200)]);
    let id = import(&mut s, &file);
    run(
        &mut s,
        "object.retrace",
        json!({ "id": id, "settings": { "invert": true } }),
    );
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [id], "degrees": 30.0 }),
    );
    let frame_corner = |o: &Object| o.transform.affine(o.local_bounds) * o.anchor_frame().origin();
    let corner_before = frame_corner(obj(&s, id));
    // Inverted: the white frame around one hole.
    assert_eq!(obj(&s, id).geometry.len(), 2);

    // Two black squares now: the white frame has two holes.
    png(&file, &[(50, 50, 80), (170, 170, 80)]);
    bump_mtime(&file);
    let r = run(&mut s, "object.reload", json!({ "ids": [id] }));
    assert_eq!(ids(&r["reloaded"]), vec![id], "{r}");
    let o = obj(&s, id);
    assert_eq!(o.geometry.len(), 3, "traced inverted again");
    match &o.source {
        Source::Raster { trace, bytes, .. } => {
            assert!(trace.invert, "the stored trace settings are kept");
            assert_eq!(bytes, &std::fs::read(&file).unwrap());
        }
        other => panic!("{other:?}"),
    }
    assert!((frame_corner(o) - corner_before).hypot() < 1e-6);
    assert_eq!(o.transform.rotation_deg, 30.0);
}

#[test]
fn reload_is_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = dir.path().join("a.svg");
    let b = dir.path().join("b.svg");
    write(&a, svg(false));
    write(&b, svg(false));
    let ia = import(&mut s, &a);
    let ib = import(&mut s, &b);
    change(&a, svg(true));
    change(&b, svg(true));

    let r = run(&mut s, "object.reload", json!({ "ids": [] }));
    assert_eq!(ids(&r["reloaded"]), vec![ia, ib], "{r}");
    assert_eq!(obj(&s, ia).geometry.len(), 2);
    assert_eq!(obj(&s, ib).geometry.len(), 2);

    run(&mut s, "edit.undo", json!({}));
    for id in [ia, ib] {
        let o = obj(&s, id);
        assert_eq!(o.geometry.len(), 1);
        assert_eq!(bytes_of(o), svg(false).as_bytes());
        assert_eq!(
            o.link.as_ref().unwrap().sha256,
            hex_sha256(svg(false).as_bytes())
        );
    }
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(obj(&s, ia).geometry.len(), 2);
    assert_eq!(obj(&s, ib).geometry.len(), 2);
}

// ---------------------------------------------------------------- check_sources

#[test]
fn check_sources_reports_changed_and_missing() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let [a, b, c] = ["a", "b", "c"].map(|n| dir.path().join(format!("{n}.svg")));
    for f in [&a, &b, &c] {
        write(f, svg(false));
    }
    let [ia, ib, ic] = [&a, &b, &c].map(|f| import(&mut s, f));
    run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg(false), "name": "unlinked" }),
    );
    run(
        &mut s,
        "project.save",
        json!({ "path": dir.path().join("p.flatcraft") }),
    );

    bump_mtime(&a); // touched, same content: not changed
    change(&b, svg(true));
    std::fs::remove_file(&c).unwrap();
    let r = run(&mut s, "document.check_sources", json!({}));
    assert_eq!(ids(&r["changed"]), vec![ib], "{r}");
    assert_eq!(ids(&r["missing"]), vec![ic], "{r}");
    assert!(!ids(&r["changed"]).contains(&ia), "only touched");
    // Not mutating: still saved, nothing to undo for it.
    assert!(!s.doc().dirty);
    assert!(
        s.execute("document.check_sources", json!({ "x": 1 }), Origin::User)
            .is_err()
    );
}

// ---------------------------------------------------------------- relink / unlink

#[test]
fn relink_points_to_a_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let old = dir.path().join("old").join("a.svg");
    write(&old, svg(false));
    let id = import(&mut s, &old);
    std::fs::remove_file(&old).unwrap();
    let r = run(&mut s, "object.reload", json!({ "ids": [id] }));
    assert_eq!(ids(&r["missing"]), vec![id]);

    let new = dir.path().join("new").join("renamed.svg");
    write(&new, svg(true));
    let r = run(&mut s, "object.relink", json!({ "id": id, "path": new }));
    assert_eq!(ids(&r["reloaded"]), vec![id], "{r}");
    assert_eq!(r["also_found"], json!([]));
    let o = obj(&s, id);
    assert_eq!(o.geometry.len(), 2);
    assert_eq!(o.link.as_ref().unwrap().path, new);
    let r = run(&mut s, "document.check_sources", json!({}));
    assert_eq!(ids(&r["missing"]), Vec::<u64>::new(), "{r}");

    // Wrong input is refused and changes nothing.
    let png_file = dir.path().join("x.png");
    png(&png_file, &[(10, 10, 100)]);
    for (path, want) in [
        (dir.path().join("x.bmp"), "unsupported"),
        (dir.path().join("nope.svg"), "nope.svg"),
        (png_file, "SVG"),
    ] {
        if path.extension().unwrap() == "bmp" {
            write(&path, b"bmp");
        }
        let e = s
            .execute(
                "object.relink",
                json!({ "id": id, "path": path }),
                Origin::Agent,
            )
            .unwrap_err();
        assert!(e.to_string().contains(want), "{e}");
        assert!(!e.to_string().contains('\u{2014}'), "{e}");
    }
    assert_eq!(obj(&s, id).link.as_ref().unwrap().path, new);
}

#[test]
fn relink_finds_siblings_in_the_new_folder() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let old = dir.path().join("old");
    let new = dir.path().join("new");
    write(&old.join("a.svg"), svg(false));
    write(&old.join("b.svg"), svg(false));
    let ia = import(&mut s, &old.join("a.svg"));
    let ib = import(&mut s, &old.join("b.svg"));
    std::fs::rename(&old, &new).unwrap();
    let r = run(&mut s, "document.check_sources", json!({}));
    assert_eq!(ids(&r["missing"]), vec![ia, ib]);

    let r = run(
        &mut s,
        "object.relink",
        json!({ "id": ia, "path": new.join("a.svg") }),
    );
    assert_eq!(
        r["also_found"],
        json!([{ "id": ib, "path": new.join("b.svg") }]),
        "{r}"
    );
    // Same content as before: relinked, nothing to rebuild.
    assert_eq!(ids(&r["unchanged"]), vec![ia], "{r}");
    run(
        &mut s,
        "object.relink",
        json!({ "id": ib, "path": new.join("b.svg") }),
    );
    let r = run(&mut s, "document.check_sources", json!({}));
    assert_eq!(ids(&r["missing"]), Vec::<u64>::new());
}

#[test]
fn unlink_keeps_the_object() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let file = dir.path().join("a.svg");
    write(&file, svg(false));
    let id = import(&mut s, &file);
    let before = obj(&s, id).clone();
    let r = run(&mut s, "object.unlink", json!({ "ids": [id] }));
    assert_eq!(ids(&r["ids"]), vec![id]);
    let o = obj(&s, id);
    assert!(o.link.is_none());
    assert_eq!(o.geometry, before.geometry);
    assert_eq!(bytes_of(o), bytes_of(&before));

    change(&file, svg(true));
    let r = run(&mut s, "object.reload", json!({ "ids": [] }));
    assert_eq!(ids(&r["reloaded"]), Vec::<u64>::new(), "{r}");
    let r = run(&mut s, "object.reload", json!({ "ids": [id] }));
    assert_eq!(ids(&r["reloaded"]), Vec::<u64>::new(), "{r}");
    assert!(
        r["warnings"][0]
            .as_str()
            .unwrap()
            .contains("no linked file"),
        "{r}"
    );
    assert_eq!(obj(&s, id).geometry.len(), 1);

    // The two reloads changed nothing, so the next undo is the unlink.
    run(&mut s, "edit.undo", json!({}));
    assert!(obj(&s, id).link.is_some());
}

// ---------------------------------------------------------------- never break a project

#[test]
fn missing_file_keeps_embedded_copy() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let file = dir.path().join("a.svg");
    write(&file, svg(false));
    let id = import(&mut s, &file);
    let project = dir.path().join("p.flatcraft");
    run(&mut s, "project.save", json!({ "path": project }));
    std::fs::remove_file(&file).unwrap();

    let mut s = session(dir.path());
    let r = run(&mut s, "project.open", json!({ "path": project }));
    assert_eq!(r["warnings"], json!([]), "{r}");
    let o = obj(&s, id);
    assert_eq!(o.geometry.len(), 1);
    assert_eq!(bytes_of(o), svg(false).as_bytes());
    let r = run(&mut s, "job.send", json!({ "name": "missing" }));
    assert!(r.is_object(), "{r}");
}

#[test]
fn deleted_or_renamed_file_never_breaks_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let deleted = dir.path().join("deleted.svg");
    let renamed = dir.path().join("renamed.svg");
    let kept = dir.path().join("kept.png");
    write(&deleted, svg(false));
    write(&renamed, svg(true));
    png(&kept, &[(50, 50, 200)]);
    let id_deleted = import(&mut s, &deleted);
    let id_renamed = import(&mut s, &renamed);
    let id_kept = import(&mut s, &kept);
    run(
        &mut s,
        "object.move",
        json!({ "ids": [id_kept], "dx_mm": 120.0, "dy_mm": 120.0 }),
    );
    run(
        &mut s,
        "object.set_cut_options",
        json!({ "ids": [id_renamed], "mode": "all_lines" }),
    );
    let project = dir.path().join("p.flatcraft");
    run(&mut s, "project.save", json!({ "path": project }));
    let before: Vec<Object> = [id_deleted, id_renamed]
        .iter()
        .map(|&i| obj(&s, i).clone())
        .collect();

    std::fs::remove_file(&deleted).unwrap();
    std::fs::rename(&renamed, dir.path().join("something else.svg")).unwrap();

    let mut s = session(dir.path());
    let r = run(&mut s, "project.open", json!({ "path": project }));
    assert_eq!(r["objects"], 3, "{r}");
    run(
        &mut s,
        "object.move",
        json!({ "ids": [id_kept], "dx_mm": 1.0 }),
    );
    run(&mut s, "project.save", json!({}));
    run(&mut s, "project.save", json!({}));
    let r = run(&mut s, "document.check_sources", json!({}));
    assert_eq!(ids(&r["missing"]), vec![id_deleted, id_renamed], "{r}");
    assert_eq!(ids(&r["changed"]), Vec::<u64>::new(), "{r}");
    let r = run(&mut s, "object.reload", json!({ "ids": [] }));
    assert_eq!(ids(&r["missing"]), vec![id_deleted, id_renamed], "{r}");
    assert_eq!(ids(&r["unchanged"]), vec![id_kept], "{r}");
    for b in &before {
        let o = obj(&s, b.id.0);
        assert_eq!(bytes_of(o), bytes_of(b));
        assert_eq!(o.geometry, b.geometry);
        assert_eq!(o.transform, b.transform);
        assert_eq!(o.cut, b.cut);
        assert!(o.link.is_some(), "a missing file keeps its link");
    }
    let r = run(&mut s, "job.send", json!({ "name": "never-broken" }));
    assert!(r.is_object(), "{r}");
    run(&mut s, "project.save", json!({}));

    let mut s = session(dir.path());
    let r = run(&mut s, "project.open", json!({ "path": project }));
    assert_eq!(r["objects"], 3, "{r}");
    assert_eq!(r["warnings"], json!([]), "{r}");
}

#[test]
fn failed_reload_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let broken = dir.path().join("broken.svg");
    let huge = dir.path().join("huge.svg");
    let bad_png = dir.path().join("bad.png");
    let good = dir.path().join("good.svg");
    for f in [&broken, &huge, &good] {
        write(f, svg(false));
    }
    png(&bad_png, &[(50, 50, 200)]);
    let [ib, ih, ip, ig] = [&broken, &huge, &bad_png, &good].map(|f| import(&mut s, f));
    let before: Vec<Object> = [ib, ih, ip].iter().map(|&i| obj(&s, i).clone()).collect();

    change(&broken, "this is not an svg");
    // Sparse: larger than the 50 MB import cap without writing 50 MB.
    std::fs::File::options()
        .write(true)
        .open(&huge)
        .unwrap()
        .set_len(51 * 1024 * 1024)
        .unwrap();
    bump_mtime(&huge);
    change(&bad_png, b"\x89PNG but not really");
    change(&good, svg(true));

    let r = run(&mut s, "object.reload", json!({ "ids": [] }));
    assert_eq!(ids(&r["reloaded"]), vec![ig], "{r}");
    assert_eq!(ids(&r["failed"]), vec![ib, ih, ip], "{r}");
    let warnings: Vec<&str> = r["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap())
        .collect();
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(
        warnings.iter().any(|w| w.contains("broken")),
        "{warnings:?}"
    );
    assert!(warnings.iter().any(|w| w.contains("50 MB")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("bad")), "{warnings:?}");
    assert!(warnings.iter().all(|w| !w.contains('\u{2014}')));
    for b in &before {
        let o = obj(&s, b.id.0);
        assert_eq!(bytes_of(o), bytes_of(b));
        assert_eq!(o.geometry, b.geometry);
        assert_eq!(o.transform, b.transform);
        assert_eq!(o.link, b.link);
    }
    assert_eq!(obj(&s, ig).geometry.len(), 2);
}

#[test]
fn relative_path_survives_moving_the_project_folder() {
    let dir = tempfile::tempdir().unwrap();
    let one = dir.path().join("one");
    let two = dir.path().join("two");
    let mut s = session(dir.path());
    std::fs::create_dir_all(&one).unwrap();
    run(
        &mut s,
        "project.save",
        json!({ "path": one.join("p.flatcraft") }),
    );
    write(&one.join("art").join("a.svg"), svg(false));
    let id = import(&mut s, &one.join("art").join("a.svg"));
    run(&mut s, "project.save", json!({}));
    std::fs::rename(&one, &two).unwrap();

    let mut s = session(dir.path());
    run(
        &mut s,
        "project.open",
        json!({ "path": two.join("p.flatcraft") }),
    );
    let r = run(&mut s, "document.check_sources", json!({}));
    assert_eq!(ids(&r["missing"]), Vec::<u64>::new(), "{r}");
    change(&two.join("art").join("a.svg"), svg(true));
    let r = run(&mut s, "object.reload", json!({ "ids": [] }));
    assert_eq!(ids(&r["reloaded"]), vec![id], "{r}");
    let link = obj(&s, id).link.clone().unwrap();
    assert_eq!(link.path, two.join("art").join("a.svg"));

    // Saving into another folder keeps pointing at the same file.
    let three = dir.path().join("three");
    std::fs::create_dir_all(&three).unwrap();
    run(
        &mut s,
        "project.save",
        json!({ "path": three.join("p.flatcraft") }),
    );
    let link = obj(&s, id).link.clone().unwrap();
    assert_eq!(link.path, two.join("art").join("a.svg"));
    assert_eq!(
        link.relative,
        Some(PathBuf::from("..").join("two").join("art").join("a.svg"))
    );
}

#[test]
fn auto_reload_is_off_by_default() {
    assert!(!Preferences::default().auto_reload);
    let p: Preferences = toml::from_str("").unwrap();
    assert!(!p.auto_reload);
}

#[test]
fn reload_refuses_unknown_ids() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let e = s
        .execute("object.reload", json!({ "ids": [42] }), Origin::Agent)
        .unwrap_err();
    assert!(matches!(e, CommandError::NotFound(_)), "{e}");
}

// ---------------------------------------------------------------- review fixes

#[test]
fn save_as_then_undo_never_picks_a_decoy() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    let mut s = session(dir.path());
    std::fs::create_dir_all(&a).unwrap();
    run(
        &mut s,
        "project.save",
        json!({ "path": a.join("p.flatcraft") }),
    );
    write(&a.join("art").join("x.svg"), svg(false));
    let id = import(&mut s, &a.join("art").join("x.svg"));
    run(&mut s, "object.move", json!({ "ids": [id], "dx_mm": 1.0 }));
    // A decoy with the same relative path next to the new project file.
    write(&b.join("art").join("x.svg"), svg(true));
    bump_mtime(&b.join("art").join("x.svg"));
    run(
        &mut s,
        "project.save",
        json!({ "path": b.join("p.flatcraft") }),
    );
    // Undo the move: the snapshot holds the link made against a/.
    run(&mut s, "edit.undo", json!({}));
    let r = run(&mut s, "document.check_sources", json!({}));
    assert_eq!(ids(&r["changed"]), Vec::<u64>::new(), "{r}");
    assert_eq!(ids(&r["missing"]), Vec::<u64>::new(), "{r}");
    let r = run(&mut s, "object.reload", json!({ "ids": [] }));
    assert_eq!(ids(&r["reloaded"]), Vec::<u64>::new(), "{r}");
    assert_eq!(obj(&s, id).geometry.len(), 1, "never the decoy");
    let link = obj(&s, id).link.clone().unwrap();
    assert_eq!(link.path, a.join("art").join("x.svg"));

    // Without the original the link reports missing, never the decoy.
    std::fs::remove_file(a.join("art").join("x.svg")).unwrap();
    let r = run(&mut s, "document.check_sources", json!({}));
    assert_eq!(ids(&r["missing"]), vec![id], "{r}");
    let r = run(&mut s, "object.reload", json!({ "ids": [] }));
    assert_eq!(ids(&r["missing"]), vec![id], "{r}");
    assert_eq!(obj(&s, id).geometry.len(), 1);
}

#[test]
fn check_sources_reports_unreadable_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let huge = dir.path().join("huge.svg");
    let gone = dir.path().join("dir.svg");
    write(&huge, svg(false));
    write(&gone, svg(false));
    let ih = import(&mut s, &huge);
    let id = import(&mut s, &gone);
    std::fs::File::options()
        .write(true)
        .open(&huge)
        .unwrap()
        .set_len(51 * 1024 * 1024)
        .unwrap();
    bump_mtime(&huge);
    std::fs::remove_file(&gone).unwrap();
    std::fs::create_dir(&gone).unwrap(); // not a regular file
    let r = run(&mut s, "document.check_sources", json!({}));
    assert_eq!(ids(&r["unreadable"]), vec![ih, id], "{r}");
    assert_eq!(ids(&r["changed"]), Vec::<u64>::new(), "{r}");
    assert_eq!(ids(&r["missing"]), Vec::<u64>::new(), "{r}");
}

/// Saves, then opens the project in a fresh session: no undo history, clean.
fn reopened(dir: &Path, s: &mut Session) -> Session {
    let project = dir.join("p.flatcraft");
    run(s, "project.save", json!({ "path": project }));
    let mut s = session(dir);
    run(&mut s, "project.open", json!({ "path": project }));
    assert!(!s.doc().undo.can_undo() && !s.doc().dirty);
    s
}

#[test]
fn reload_without_changes_records_no_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let [missing, broken, same] =
        ["missing", "broken", "same"].map(|n| dir.path().join(format!("{n}.svg")));
    for f in [&missing, &broken, &same] {
        write(f, svg(false));
    }
    let [im, ib, is] = [&missing, &broken, &same].map(|f| import(&mut s, f));
    let mut s = reopened(dir.path(), &mut s);
    std::fs::remove_file(&missing).unwrap();
    change(&broken, "not an svg");
    bump_mtime(&same);

    let r = run(&mut s, "object.reload", json!({ "ids": [im, ib] }));
    assert_eq!(ids(&r["missing"]), vec![im], "{r}");
    assert_eq!(ids(&r["failed"]), vec![ib], "{r}");
    assert!(!s.doc().undo.can_undo(), "nothing changed, no undo step");
    assert!(!s.doc().dirty);

    let old = obj(&s, is).link.clone().unwrap();
    let r = run(&mut s, "object.reload", json!({ "ids": [is] }));
    assert_eq!(ids(&r["unchanged"]), vec![is], "{r}");
    assert!(
        !s.doc().undo.can_undo(),
        "an mtime refresh is no user change"
    );
    assert!(!s.doc().dirty);
    let new = obj(&s, is).link.clone().unwrap();
    assert_ne!(new.modified, old.modified, "the mtime is refreshed");
    assert_eq!((new.path, new.sha256), (old.path, old.sha256));
}

#[test]
fn a_link_to_another_file_type_is_never_read() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let file = dir.path().join("a.svg");
    write(&file, svg(false));
    let id = import(&mut s, &file);
    let project = dir.path().join("p.flatcraft");
    run(&mut s, "project.save", json!({ "path": project }));
    // A crafted project points the link at a file that is no SVG or image.
    let secret = dir.path().join("secret.txt");
    write(&secret, svg(true));
    let text = std::fs::read_to_string(&project).unwrap();
    let mut v: Value = serde_json::from_str(&text).unwrap();
    v["objects"][0]["link"]["path"] = json!(secret);
    v["objects"][0]["link"]
        .as_object_mut()
        .unwrap()
        .remove("relative");
    std::fs::write(&project, v.to_string()).unwrap();

    let mut s = session(dir.path());
    run(&mut s, "project.open", json!({ "path": project }));
    let r = run(&mut s, "document.check_sources", json!({}));
    assert_eq!(ids(&r["missing"]), vec![id], "{r}");
    let r = run(&mut s, "object.reload", json!({ "ids": [] }));
    assert_eq!(ids(&r["missing"]), vec![id], "{r}");
    assert_eq!(bytes_of(obj(&s, id)), svg(false).as_bytes());
}

#[test]
fn a_relative_link_path_resolves_against_the_project_folder() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let file = dir.path().join("only-here.svg");
    write(&file, svg(false));
    let id = import(&mut s, &file);
    let project = dir.path().join("p.flatcraft");
    run(&mut s, "project.save", json!({ "path": project }));
    let text = std::fs::read_to_string(&project).unwrap();
    let mut v: Value = serde_json::from_str(&text).unwrap();
    v["objects"][0]["link"]["path"] = json!("only-here.svg");
    v["objects"][0]["link"]
        .as_object_mut()
        .unwrap()
        .remove("relative");
    std::fs::write(&project, v.to_string()).unwrap();
    change(&file, svg(true));

    let mut s = session(dir.path());
    run(&mut s, "project.open", json!({ "path": project }));
    let r = run(&mut s, "object.reload", json!({ "ids": [] }));
    assert_eq!(ids(&r["reloaded"]), vec![id], "{r}");
}

#[cfg(unix)]
#[test]
fn a_fifo_never_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("pipe.svg");
    let ok = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let mut s = session(dir.path());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = s.execute("object.import", json!({ "path": fifo }), Origin::Agent);
        let _ = tx.send(r.map_err(|e| e.to_string()));
    });
    let r = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("importing a FIFO blocked");
    assert!(r.is_err(), "{r:?}");
}

#[cfg(unix)]
#[test]
fn reload_and_check_of_a_fifo_never_block() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let file = dir.path().join("pipe.svg");
    write(&file, svg(false));
    let id = import(&mut s, &file);
    std::fs::remove_file(&file).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&file)
            .status()
            .unwrap()
            .success()
    );
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let check = run(&mut s, "document.check_sources", json!({}));
        let reload = run(&mut s, "object.reload", json!({ "ids": [] }));
        let _ = tx.send((check, reload));
    });
    let (check, reload) = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("a FIFO blocked");
    assert_eq!(ids(&check["unreadable"]), vec![id], "{check}");
    assert_eq!(ids(&reload["failed"]), vec![id], "{reload}");
}

#[test]
fn reload_with_a_new_image_size_keeps_scale_and_frame() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let file = dir.path().join("pic.png");
    png(&file, &[(30, 30, 100)]);
    let id = import(&mut s, &file);
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [id], "factor": 1.5 }),
    );
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [id], "degrees": 90.0 }),
    );
    let before = obj(&s, id).clone();
    let square = |o: &Object| first_path_points(o);
    let corner = |o: &Object| o.transform.affine(o.local_bounds) * o.anchor_frame().origin();

    // Twice the pixels; the square keeps its pixel position.
    let mut img = image::GrayImage::from_pixel(600, 400, image::Luma([255]));
    for y in 30..130 {
        for x in 30..130 {
            img.put_pixel(x, y, image::Luma([0]));
        }
    }
    img.save(&file).unwrap();
    bump_mtime(&file);
    let r = run(&mut s, "object.reload", json!({ "ids": [id] }));
    assert_eq!(ids(&r["reloaded"]), vec![id], "{r}");
    let o = obj(&s, id);
    assert!((o.anchor_frame().width() - 2.0 * before.anchor_frame().width()).abs() < 1e-6);
    assert_eq!(
        (o.transform.scale_x, o.transform.rotation_deg),
        (before.transform.scale_x, before.transform.rotation_deg)
    );
    assert!((corner(o) - corner(&before)).hypot() < 1e-6);
    let (a, b) = (square(o), square(&before));
    let bbox = |p: &[Point]| {
        p.iter()
            .fold(kurbo::Rect::from_points(p[0], p[0]), |r, q| r.union_pt(*q))
    };
    let (a, b) = (bbox(&a), bbox(&b));
    assert!(
        (a.x0 - b.x0).abs() < 1e-6 && (a.y0 - b.y0).abs() < 1e-6,
        "{a:?} {b:?}"
    );
}

#[test]
fn reload_with_a_new_svg_page_size_keeps_the_page_corner() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let file = dir.path().join("a.svg");
    write(&file, svg(false));
    let id = import(&mut s, &file);
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [id], "factor": 0.5 }),
    );
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [id], "axis": "vertical" }),
    );
    let before = obj(&s, id).clone();
    change(
        &file,
        svg(true).replace(
            r#"width="200mm" height="200mm" viewBox="0 0 200 200""#,
            r#"width="300mm" height="250mm" viewBox="0 0 300 250""#,
        ),
    );
    let r = run(&mut s, "object.reload", json!({ "ids": [id] }));
    assert_eq!(ids(&r["reloaded"]), vec![id], "{r}");
    let o = obj(&s, id);
    assert!((o.anchor_frame().width() - 300.0).abs() < 1e-3);
    assert_eq!(o.transform.scale_x, before.transform.scale_x);
    assert_eq!(o.transform.mirror, before.transform.mirror);
    assert_points_close(&first_path_points(o), &first_path_points(&before));
}

#[test]
fn relink_to_a_broken_file_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let r = run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg(false), "name": "text" }),
    );
    let id = r["id"].as_u64().unwrap();
    let before = obj(&s, id).clone();
    let broken = dir.path().join("broken.svg");
    write(&broken, "not an svg");
    let e = s
        .execute(
            "object.relink",
            json!({ "id": id, "path": broken }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::Failed(_)), "{e}");
    let o = obj(&s, id);
    assert!(o.link.is_none(), "still unlinked");
    assert_eq!(o.geometry, before.geometry);
    assert_eq!(bytes_of(o), bytes_of(&before));
}

#[test]
fn a_folder_with_the_file_name_is_never_taken_for_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("p");
    let q = dir.path().join("q");
    let mut s = session(dir.path());
    std::fs::create_dir_all(&p).unwrap();
    run(
        &mut s,
        "project.save",
        json!({ "path": p.join("p.flatcraft") }),
    );
    write(&p.join("art").join("x.svg"), svg(false));
    let id = import(&mut s, &p.join("art").join("x.svg"));
    run(&mut s, "project.save", json!({}));
    // A copy of the project whose art/x.svg is a folder.
    std::fs::create_dir_all(q.join("art").join("x.svg")).unwrap();
    std::fs::copy(p.join("p.flatcraft"), q.join("p.flatcraft")).unwrap();

    let mut s = session(dir.path());
    run(
        &mut s,
        "project.open",
        json!({ "path": q.join("p.flatcraft") }),
    );
    let r = run(&mut s, "document.check_sources", json!({}));
    assert_eq!(ids(&r["unreadable"]), Vec::<u64>::new(), "{r}");
    assert_eq!(ids(&r["missing"]), Vec::<u64>::new(), "{r}");
    run(
        &mut s,
        "project.save",
        json!({ "path": dir.path().join("p2.flatcraft") }),
    );
    let link = obj(&s, id).link.clone().unwrap();
    assert_eq!(link.path, p.join("art").join("x.svg"), "the real file");
}
