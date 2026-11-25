//! Linked copies: copies that share the source, the geometry, the trace
//! settings, the cut options, the scale and the file link. Each copy keeps
//! its own position, rotation and mirror (and its name and visibility).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use flatcraft_core::document::{Object, ObjectId, Source};
use flatcraft_device::ProfileStore;
use flatcraft_engine::commands::placed_bounds;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::registry;
use flatcraft_engine::session::{CommandError, Origin, Session};
use kurbo::Point;
use serde_json::{Value, json};

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
/// a 5 mm square at the page's top-left after it. `pad` adds a comment of
/// that many bytes, to make the source large.
fn svg(corner: bool, pad: usize) -> String {
    let extra = if corner {
        r#"<rect x="0" y="0" width="5" height="5"/>"#
    } else {
        ""
    };
    let comment = if pad > 0 {
        format!("<!-- {} -->", "x".repeat(pad))
    } else {
        String::new()
    };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200">{comment}<rect x="20" y="20" width="50" height="50"/>{extra}</svg>"#
    )
}

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

/// Rewrites a file and moves its modification time forward.
fn change(path: &Path, bytes: impl AsRef<[u8]>) {
    write(path, bytes);
    let f = std::fs::File::options().write(true).open(path).unwrap();
    f.set_modified(SystemTime::now() + Duration::from_secs(60))
        .unwrap();
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

fn import_file(s: &mut Session, path: &Path) -> u64 {
    run(s, "object.import", json!({ "path": path }))["id"]
        .as_u64()
        .unwrap()
}

fn import_text(s: &mut Session) -> u64 {
    run(
        s,
        "object.import_svg",
        json!({ "svg": svg(false, 0), "name": "sq" }),
    )["id"]
        .as_u64()
        .unwrap()
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

/// `count` linked copies of `id`; returns the original first, then the
/// copies.
fn linked(s: &mut Session, id: u64, count: u32) -> Vec<u64> {
    let r = run(
        s,
        "object.copies",
        json!({ "id": id, "count": count, "linked": true }),
    );
    let mut out = vec![id];
    out.extend(
        r["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap()),
    );
    out
}

fn obj(s: &Session, id: u64) -> &Object {
    s.doc().document.get(ObjectId(id)).unwrap()
}

fn bytes_of(o: &Object) -> &Arc<[u8]> {
    match &o.source {
        Source::Svg { bytes, .. } | Source::Raster { bytes, .. } => bytes,
    }
}

/// Every member shares one geometry and one source `Arc`, and the shared
/// fields are equal.
fn assert_shared(s: &Session, members: &[u64]) {
    let first = obj(s, members[0]);
    assert!(first.link_group.is_some(), "{} is not linked", members[0]);
    for &m in &members[1..] {
        let o = obj(s, m);
        assert_eq!(o.link_group, first.link_group, "group of {m}");
        assert!(Arc::ptr_eq(&o.geometry, &first.geometry), "geometry of {m}");
        assert!(Arc::ptr_eq(bytes_of(o), bytes_of(first)), "source of {m}");
        assert_eq!(o.source, first.source, "source of {m}");
        assert_eq!(o.cut, first.cut, "cut of {m}");
        assert_eq!(o.link, first.link, "link of {m}");
        assert_eq!(o.local_bounds, first.local_bounds, "bounds of {m}");
        assert_eq!(o.frame, first.frame, "frame of {m}");
        assert_eq!(o.transform.scale_x, first.transform.scale_x, "scale of {m}");
        assert_eq!(o.transform.scale_y, first.transform.scale_y, "scale of {m}");
    }
}

fn centre(s: &Session, id: u64) -> Point {
    obj(s, id).transform.center_mm
}

fn bounds_centre(s: &Session, id: u64) -> Point {
    placed_bounds(obj(s, id)).unwrap().center()
}

fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-6
}

fn undo_len(s: &Session) -> usize {
    s.doc().undo.undo_len()
}

// ---------------------------------------------------------------- copies

#[test]
fn linked_copies_share_one_source_and_one_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = import_text(&mut s);
    let before = undo_len(&s);
    let all = linked(&mut s, id, 3);
    assert_eq!(all.len(), 4);
    assert_eq!(undo_len(&s), before + 1);
    assert_shared(&s, &all);
    // document.state names the group, so agents can see it.
    let state = run(&mut s, "document.state", json!({}));
    let groups: Vec<&Value> = state["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| &o["link_group"])
        .collect();
    assert!(groups[0].is_u64(), "{state}");
    assert!(groups.iter().all(|g| *g == groups[0]), "{state}");
    // Copies of a linked copy join the same group.
    let more = linked(&mut s, all[2], 2);
    let mut every = all.clone();
    every.extend(&more[1..]);
    assert_shared(&s, &every);
}

// ---------------------------------------------------------------- per copy

#[test]
fn rotate_mirror_and_move_one_change_only_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = import_text(&mut s);
    let all = linked(&mut s, id, 2);
    let before: Vec<_> = all.iter().map(|i| obj(&s, *i).transform).collect();
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [all[1]], "degrees": 30.0 }),
    );
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [all[1]], "axis": "horizontal" }),
    );
    run(
        &mut s,
        "object.move",
        json!({ "ids": [all[1]], "dx_mm": 3.0, "dy_mm": 4.0 }),
    );
    let t = obj(&s, all[1]).transform;
    assert!((t.rotation_deg - 330.0).abs() < 1e-9, "{t:?}");
    assert!(t.mirror);
    assert!(near(
        t.center_mm,
        before[1].center_mm + kurbo::Vec2::new(3.0, 4.0)
    ));
    for k in [0, 2] {
        assert_eq!(obj(&s, all[k]).transform, before[k], "copy {k} changed");
    }
    assert_shared(&s, &all);
}

// ---------------------------------------------------------------- scale

#[test]
fn scale_one_scales_all_and_each_keeps_its_centre() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = import_text(&mut s);
    let all = linked(&mut s, id, 2);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [all[1]], "degrees": 30.0 }),
    );
    let centres: Vec<Point> = all.iter().map(|i| centre(&s, *i)).collect();
    let target = bounds_centre(&s, all[0]);
    let before = undo_len(&s);
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [all[0]], "factor": 2.0 }),
    );
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    for &m in &all {
        let t = obj(&s, m).transform;
        assert!(
            (t.scale_x - 2.0).abs() < 1e-12 && (t.scale_y - 2.0).abs() < 1e-12,
            "{t:?}"
        );
    }
    assert!(near(bounds_centre(&s, all[0]), target));
    assert!(near(centre(&s, all[1]), centres[1]));
    assert!(near(centre(&s, all[2]), centres[2]));
    assert!((obj(&s, all[1]).transform.rotation_deg - 30.0).abs() < 1e-9);
    assert_shared(&s, &all);

    // Two members of one group in one call: the group scales once.
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [all[0], all[2]], "factor": 2.0 }),
    );
    for &m in &all {
        assert!((obj(&s, m).transform.scale_x - 4.0).abs() < 1e-12);
    }

    // A size given for a rotated member: that member gets the size.
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [all[1]], "width_mm": 80.0 }),
    );
    let b = placed_bounds(obj(&s, all[1])).unwrap();
    assert!((b.width() - 80.0).abs() < 1e-4, "{b:?}");
    assert_shared(&s, &all);
}

#[test]
fn group_scale_scales_each_member_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = import_text(&mut s);
    let all = linked(&mut s, id, 2);
    let outside = centre(&s, all[2]);
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [all[0], all[1]], "factor": 0.5, "group": true }),
    );
    for &m in &all {
        let t = obj(&s, m).transform;
        assert!((t.scale_x - 0.5).abs() < 1e-12, "{m}: {t:?}");
    }
    assert!(near(centre(&s, all[2]), outside));
    assert_shared(&s, &all);
}

// ---------------------------------------------------------------- cut options

#[test]
fn set_cut_options_on_one_applies_to_all() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = import_text(&mut s);
    let all = linked(&mut s, id, 2);
    let other = import_text(&mut s);
    let before = undo_len(&s);
    let r = run(
        &mut s,
        "object.set_cut_options",
        json!({ "ids": [all[1]], "mode": "all_lines" }),
    );
    assert_eq!(undo_len(&s), before + 1);
    let reported: Vec<u64> = r["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_u64().unwrap())
        .collect();
    let mut sorted = reported.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, all, "{r}");
    for &m in &all {
        assert_eq!(
            obj(&s, m).cut.mode,
            flatcraft_core::document::CutMode::AllLines
        );
    }
    assert_eq!(
        obj(&s, other).cut.mode,
        flatcraft_core::document::CutMode::ObjectOutlines
    );
    assert_shared(&s, &all);
}

// ---------------------------------------------------------------- undo

#[test]
fn undo_and_redo_across_a_group_edit() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = import_text(&mut s);
    let all = linked(&mut s, id, 2);
    run(
        &mut s,
        "object.set_cut_options",
        json!({ "ids": [all[0]], "strokes": "outline" }),
    );
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [all[2]], "factor": 3.0 }),
    );
    run(&mut s, "edit.undo", json!({}));
    for &m in &all {
        assert_eq!(obj(&s, m).transform.scale_x, 1.0);
        assert_eq!(
            obj(&s, m).cut.strokes,
            flatcraft_core::document::StrokeMode::Outline
        );
    }
    run(&mut s, "edit.undo", json!({}));
    for &m in &all {
        assert_eq!(
            obj(&s, m).cut.strokes,
            flatcraft_core::document::StrokeMode::Auto
        );
    }
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "edit.redo", json!({}));
    for &m in &all {
        assert!((obj(&s, m).transform.scale_x - 3.0).abs() < 1e-12);
    }
    assert_shared(&s, &all);
    // Undo of the copies takes them all away.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().document.objects.len(), 1);
    assert_eq!(obj(&s, id).link_group, None);
}

// ---------------------------------------------------------------- retrace

#[test]
fn retrace_of_one_retraces_all_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let file = dir.path().join("dots.png");
    png(&file, &[(50, 50, 60), (150, 150, 100)]);
    let id = import_file(&mut s, &file);
    let all = linked(&mut s, id, 2);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [all[2]], "degrees": 90.0 }),
    );
    let paths_before = obj(&s, all[0]).geometry.len();
    let before = undo_len(&s);
    run(
        &mut s,
        "object.retrace",
        json!({ "id": all[1], "settings": { "invert": true } }),
    );
    assert_eq!(undo_len(&s), before + 1);
    for &m in &all {
        let Source::Raster { trace, .. } = &obj(&s, m).source else {
            panic!("not raster");
        };
        assert!(trace.invert, "{m}");
    }
    assert_ne!(obj(&s, all[0]).geometry.len(), paths_before);
    assert!((obj(&s, all[2]).transform.rotation_deg - 90.0).abs() < 1e-9);
    assert_shared(&s, &all);
    run(&mut s, "edit.undo", json!({}));
    for &m in &all {
        let Source::Raster { trace, .. } = &obj(&s, m).source else {
            panic!("not raster");
        };
        assert!(!trace.invert, "{m}");
    }
}

// ---------------------------------------------------------------- linked files

#[test]
fn reload_of_the_shared_source_updates_all_in_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let file = dir.path().join("art.svg");
    write(&file, svg(false, 0));
    let id = import_file(&mut s, &file);
    let all = linked(&mut s, id, 2);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [all[1]], "degrees": 90.0 }),
    );
    // The big square stays where it was on every copy (anchored to the page).
    let squares: Vec<Vec<Point>> = all.iter().map(|i| first_path_points(obj(&s, *i))).collect();
    change(&file, svg(true, 0));
    let check = run(&mut s, "document.check_sources", json!({}));
    assert_eq!(ids(&check["changed"]), all, "{check}");

    let before = undo_len(&s);
    let r = run(&mut s, "object.reload", json!({ "ids": [all[2]] }));
    assert_eq!(ids(&r["reloaded"]), all, "{r}");
    assert_eq!(undo_len(&s), before + 1);
    assert_shared(&s, &all);
    for (k, &m) in all.iter().enumerate() {
        assert_eq!(obj(&s, m).geometry.len(), 2, "{m}");
        let now = first_path_points(obj(&s, m));
        for (p, q) in now.iter().zip(&squares[k]) {
            assert!(near(*p, *q), "copy {k} moved: {p:?} vs {q:?}");
        }
    }
    run(&mut s, "edit.undo", json!({}));
    for &m in &all {
        assert_eq!(obj(&s, m).geometry.len(), 1, "{m}");
    }
    assert_shared(&s, &all);
    run(&mut s, "edit.redo", json!({}));
    // Reloading everything reloads the group once and reports all members.
    change(&file, svg(false, 0));
    let r = run(&mut s, "object.reload", json!({}));
    assert_eq!(ids(&r["reloaded"]), all, "{r}");
    assert_shared(&s, &all);
}

#[test]
fn relink_and_unlink_file_apply_to_all_members() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let file = dir.path().join("a/art.svg");
    write(&file, svg(false, 0));
    let id = import_file(&mut s, &file);
    let all = linked(&mut s, id, 2);
    let moved = dir.path().join("b/art.svg");
    write(&moved, svg(true, 0));
    std::fs::remove_file(&file).unwrap();
    let before = undo_len(&s);
    let r = run(
        &mut s,
        "object.relink",
        json!({ "id": all[1], "path": moved }),
    );
    assert_eq!(undo_len(&s), before + 1);
    // Other members are no siblings to relink: they are relinked already.
    assert_eq!(r["also_found"], json!([]), "{r}");
    for &m in &all {
        assert_eq!(obj(&s, m).link.as_ref().unwrap().path, moved);
        assert_eq!(obj(&s, m).geometry.len(), 2);
    }
    assert_shared(&s, &all);

    run(&mut s, "object.unlink", json!({ "ids": [all[0]] }));
    for &m in &all {
        assert!(obj(&s, m).link.is_none(), "{m} still has a file");
    }
    assert_shared(&s, &all);
}

// ---------------------------------------------------------------- link / unlink copies

#[test]
fn unlink_copies_then_edit_one_leaves_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = import_text(&mut s);
    let all = linked(&mut s, id, 2);
    let before = undo_len(&s);
    let r = run(&mut s, "object.unlink_copies", json!({ "ids": [all[1]] }));
    assert_eq!(ids(&r["ids"]), vec![all[1]], "{r}");
    assert_eq!(undo_len(&s), before + 1, "unlinking is an undo step");
    assert_eq!(obj(&s, all[1]).link_group, None);
    // It keeps its own copy of the shared fields, cheaply.
    assert!(Arc::ptr_eq(
        &obj(&s, all[1]).geometry,
        &obj(&s, all[0]).geometry
    ));
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [all[1]], "factor": 2.0 }),
    );
    run(
        &mut s,
        "object.set_cut_options",
        json!({ "ids": [all[1]], "mode": "all_lines" }),
    );
    for k in [0, 2] {
        assert_eq!(obj(&s, all[k]).transform.scale_x, 1.0);
        assert_eq!(
            obj(&s, all[k]).cut.mode,
            flatcraft_core::document::CutMode::ObjectOutlines
        );
    }
    assert_shared(&s, &[all[0], all[2]]);
    // A group of one is no group.
    run(&mut s, "object.unlink_copies", json!({ "ids": [all[2]] }));
    assert_eq!(obj(&s, all[0]).link_group, None);
    assert_eq!(obj(&s, all[2]).link_group, None);
}

#[test]
fn removing_a_member_keeps_the_rest_linked() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = import_text(&mut s);
    let all = linked(&mut s, id, 2);
    run(&mut s, "object.delete", json!({ "ids": [all[0]] }));
    assert_shared(&s, &all[1..]);
    run(&mut s, "object.delete", json!({ "ids": [all[1]] }));
    assert_eq!(
        obj(&s, all[2]).link_group,
        None,
        "a group of one is no group"
    );
    run(&mut s, "edit.undo", json!({}));
    assert_shared(&s, &all[1..]);
}

#[test]
fn duplicate_of_a_linked_copy_stays_linked() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = import_text(&mut s);
    let all = linked(&mut s, id, 1);
    let r = run(&mut s, "object.duplicate", json!({ "ids": [all[1]] }));
    let dup = r["ids"][0].as_u64().unwrap();
    assert_shared(&s, &[all[0], all[1], dup]);
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [all[0]], "factor": 2.0 }),
    );
    assert_eq!(obj(&s, dup).transform.scale_x, 2.0);
    // Copies without linked stay independent, even of a linked copy.
    let r = run(&mut s, "object.copies", json!({ "id": all[1], "count": 1 }));
    let plain = r["ids"][0].as_u64().unwrap();
    assert_eq!(obj(&s, plain).link_group, None);
}

#[test]
fn link_makes_copies_of_one_source_share_the_first_ones_fields() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = import_text(&mut s);
    let b = import_text(&mut s);
    let c = import_text(&mut s);
    run(&mut s, "object.scale", json!({ "ids": [a], "factor": 2.0 }));
    run(
        &mut s,
        "object.set_cut_options",
        json!({ "ids": [a], "mode": "all_lines" }),
    );
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [c], "degrees": 45.0 }),
    );
    run(&mut s, "object.move", json!({ "ids": [b], "dx_mm": 60.0 }));
    let centre_b = centre(&s, b);
    let before = undo_len(&s);
    let r = run(&mut s, "object.link", json!({ "ids": [a, b, c] }));
    assert_eq!(ids(&r["ids"]), vec![a, b, c], "{r}");
    assert_eq!(undo_len(&s), before + 1);
    assert_shared(&s, &[a, b, c]);
    assert_eq!(obj(&s, b).transform.scale_x, 2.0);
    assert_eq!(
        obj(&s, c).cut.mode,
        flatcraft_core::document::CutMode::AllLines
    );
    assert!((obj(&s, c).transform.rotation_deg - 45.0).abs() < 1e-9);
    assert!(near(centre(&s, b), centre_b), "b keeps its centre");
    // Linking them again changes nothing and records nothing.
    run(&mut s, "object.link", json!({ "ids": [c, a] }));
    assert_eq!(undo_len(&s), before + 1);
}

#[test]
fn link_refuses_different_sources_and_single_objects() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = import_text(&mut s);
    let other = run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg(true, 0), "name": "other" }),
    )["id"]
        .as_u64()
        .unwrap();
    let before = undo_len(&s);
    let e = s
        .execute("object.link", json!({ "ids": [a, other] }), Origin::Agent)
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
    assert!(e.to_string().contains("same source"), "{e}");
    let e = s
        .execute("object.link", json!({ "ids": [a] }), Origin::Agent)
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
    assert_eq!(undo_len(&s), before);
    assert_eq!(obj(&s, a).link_group, None);
}

// ---------------------------------------------------------------- project file

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).unwrap().len()
}

#[test]
fn save_and_open_keep_the_group_and_store_the_source_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let big = svg(false, 100_000);
    let id = run(
        &mut s,
        "object.import_svg",
        json!({ "svg": big, "name": "big" }),
    )["id"]
        .as_u64()
        .unwrap();
    let one = dir.path().join("one.flatcraft");
    run(&mut s, "project.save", json!({ "path": one }));
    let all = linked(&mut s, id, 9);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [all[3]], "degrees": 30.0 }),
    );
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [all[4]], "axis": "vertical" }),
    );
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [all[5]], "factor": 0.5 }),
    );
    let transforms: Vec<_> = all.iter().map(|i| obj(&s, *i).transform).collect();
    let ten = dir.path().join("ten.flatcraft");
    run(&mut s, "project.save", json!({ "path": ten }));
    assert!(
        file_len(&ten) < file_len(&one) + 20_000,
        "the source is stored more than once: {} vs {}",
        file_len(&ten),
        file_len(&one)
    );
    let text = std::fs::read_to_string(&ten).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    // Older builds must say "newer FlatCraft", not "broken file".
    assert_eq!(value["format_version"], 2);
    let one_value: Value = serde_json::from_str(&std::fs::read_to_string(&one).unwrap()).unwrap();
    assert_eq!(one_value["format_version"], 1, "no group, no new format");

    let mut s2 = session(dir.path());
    let r = run(&mut s2, "project.open", json!({ "path": ten }));
    assert_eq!(r["warnings"], json!([]), "{r}");
    assert_shared(&s2, &all);
    for (k, &m) in all.iter().enumerate() {
        let t = obj(&s2, m).transform;
        // Opening anchors the rebuilt shapes to the page, which may move
        // the centre by float noise (as for any object).
        assert!(near(t.center_mm, transforms[k].center_mm), "copy {k}");
        assert_eq!(
            (t.scale_x, t.scale_y, t.rotation_deg, t.mirror),
            (
                transforms[k].scale_x,
                transforms[k].scale_y,
                transforms[k].rotation_deg,
                transforms[k].mirror
            ),
            "transform of copy {k}"
        );
    }
    // The group still works after opening.
    run(
        &mut s2,
        "object.set_cut_options",
        json!({ "ids": [all[0]], "mode": "all_lines" }),
    );
    assert_shared(&s2, &all);
}

#[test]
fn open_refuses_a_member_whose_group_is_missing_and_keeps_a_members_own_fields() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = import_text(&mut s);
    let all = linked(&mut s, id, 2);
    let path = dir.path().join("p.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

    // A member that carries its own cut options keeps them and leaves the
    // group; nothing on open overwrites design data.
    let mut edited = value.clone();
    edited["objects"][1]["cut"] = json!({ "mode": "all_lines" });
    edited["objects"][1]["name"] = json!("odd");
    let p2 = dir.path().join("edited.flatcraft");
    write(&p2, serde_json::to_string(&edited).unwrap());
    let mut s2 = session(dir.path());
    let r = run(&mut s2, "project.open", json!({ "path": p2 }));
    assert_eq!(r["warnings"], json!([UNLINKED_WARNING]), "{r}");
    assert_eq!(
        obj(&s2, all[1]).cut.mode,
        flatcraft_core::document::CutMode::AllLines
    );
    assert_eq!(obj(&s2, all[1]).link_group, None);
    assert_shared(&s2, &[all[0], all[2]]);

    // A member whose group is gone has no source: refused, by name.
    value["link_groups"] = json!([]);
    value["objects"][0]["name"] = json!("Lonely");
    let p3 = dir.path().join("broken.flatcraft");
    write(&p3, serde_json::to_string(&value).unwrap());
    let mut s3 = session(dir.path());
    let e = s3
        .execute("project.open", json!({ "path": p3 }), Origin::User)
        .unwrap_err();
    assert!(matches!(e, CommandError::Failed(_)), "{e}");
    assert!(e.to_string().contains("Lonely"), "{e}");
}

/// The open warning for the member named "odd" that left its group.
const UNLINKED_WARNING: &str = "odd: it no longer matched its linked copies, so it was unlinked";

/// A saved project with three linked copies, written by hand the long way:
/// no `link_groups`, every member carries its own fields. The second
/// member is named "odd".
fn hand_written(dir: &Path) -> (Vec<u64>, Value) {
    let mut s = session(dir);
    let id = import_text(&mut s);
    let all = linked(&mut s, id, 2);
    let path = dir.join("p.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let g = v["link_groups"][0].clone();
    v.as_object_mut().unwrap().remove("link_groups");
    for o in v["objects"].as_array_mut().unwrap() {
        for k in ["source", "cut", "link", "local_bounds_mm", "frame_mm"] {
            if let Some(x) = g.get(k) {
                o[k] = x.clone();
            }
        }
        for k in ["scale_x", "scale_y"] {
            o["transform"][k] = g[k].clone();
        }
    }
    v["objects"][1]["name"] = json!("odd");
    (all, v)
}

fn open_value(dir: &Path, v: &Value) -> (Session, Value) {
    let p = dir.join("hand.flatcraft");
    write(&p, serde_json::to_string(v).unwrap());
    let mut s = session(dir);
    let r = run(&mut s, "project.open", json!({ "path": p }));
    (s, r)
}

#[test]
fn open_without_link_groups_keeps_matching_members_linked() {
    let dir = tempfile::tempdir().unwrap();
    let (all, v) = hand_written(dir.path());
    let (s, r) = open_value(dir.path(), &v);
    assert_eq!(r["warnings"], json!([]), "{r}");
    assert_shared(&s, &all);
}

#[test]
fn open_unlinks_a_member_whose_own_cut_differs() {
    let dir = tempfile::tempdir().unwrap();
    let (all, mut v) = hand_written(dir.path());
    v["objects"][1]["cut"]["mode"] = json!("all_lines");
    let (s, r) = open_value(dir.path(), &v);
    assert_eq!(r["warnings"], json!([UNLINKED_WARNING]), "{r}");
    assert_eq!(obj(&s, all[1]).link_group, None);
    assert_eq!(
        obj(&s, all[1]).cut.mode,
        flatcraft_core::document::CutMode::AllLines
    );
    for k in [0, 2] {
        assert_eq!(
            obj(&s, all[k]).cut.mode,
            flatcraft_core::document::CutMode::ObjectOutlines
        );
    }
    assert_shared(&s, &[all[0], all[2]]);
}

#[test]
fn open_unlinks_a_member_whose_own_source_differs() {
    let dir = tempfile::tempdir().unwrap();
    let (all, mut v) = hand_written(dir.path());
    use base64::Engine;
    let circle = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="40mm" viewBox="0 0 40 40"><circle cx="20" cy="20" r="15"/></svg>"#;
    v["objects"][1]["source"]["Svg"]["bytes"] =
        json!(base64::engine::general_purpose::STANDARD.encode(circle));
    let (s, r) = open_value(dir.path(), &v);
    // Only the bytes were swapped, so the saved bounds are the square's and
    // open also says the circle looks different.
    assert_eq!(r["warnings"][0], UNLINKED_WARNING, "{r}");
    assert_eq!(obj(&s, all[1]).link_group, None);
    // Each keeps its own design: the circle is 30 mm, the squares 50 mm.
    assert_eq!(&**bytes_of(obj(&s, all[1])), circle.as_bytes());
    let w = |id| placed_bounds(obj(&s, id)).unwrap().width();
    assert!((w(all[1]) - 30.0).abs() < 1e-3, "{}", w(all[1]));
    for k in [0, 2] {
        assert!((w(all[k]) - 50.0).abs() < 1e-3, "{}", w(all[k]));
    }
    assert_shared(&s, &[all[0], all[2]]);
}

#[test]
fn scaling_that_pushes_a_linked_copy_past_1_km_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = import_text(&mut s);
    let all = linked(&mut s, id, 1);
    // The copy sits near the 1 km limit; its 200 mm page still fits.
    run(
        &mut s,
        "object.move",
        json!({ "ids": [all[1]], "dx_mm": 999_700.0 }),
    );
    let before: Vec<_> = all.iter().map(|i| obj(&s, *i).transform).collect();
    let undo = undo_len(&s);
    let e = s
        .execute(
            "object.scale",
            json!({ "ids": [all[0]], "factor": 20.0 }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "{e}");
    assert!(e.to_string().contains("1 km"), "{e}");
    for (k, &m) in all.iter().enumerate() {
        assert_eq!(obj(&s, m).transform, before[k]);
    }
    assert_eq!(undo_len(&s), undo);
}

// ---------------------------------------------------------------- agents

#[test]
fn agents_use_the_same_commands() {
    for id in ["object.copies", "object.link", "object.unlink_copies"] {
        let c = registry::get(id).unwrap_or_else(|| panic!("{id} missing"));
        let d = c.description();
        assert!(d.contains("mm"), "{id}: {d}");
        assert!(d.contains("grid"), "{id}: {d}");
        for word in ["position", "rotation", "mirror", "scale", "cut options"] {
            assert!(d.contains(word), "{id} does not name {word}: {d}");
        }
        assert!(!d.contains('\u{2014}'), "em-dash in {id}");
    }
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = import_text(&mut s);
    let r = s
        .execute(
            "object.copies",
            json!({ "id": a, "count": 2, "linked": true }),
            Origin::Agent,
        )
        .unwrap();
    let copies = ids(&r["ids"]);
    s.execute(
        "object.unlink_copies",
        json!({ "ids": [copies[0]] }),
        Origin::Agent,
    )
    .unwrap();
    s.execute(
        "object.link",
        json!({ "ids": [a, copies[0]] }),
        Origin::Agent,
    )
    .unwrap();
    let all = vec![a, copies[0], copies[1]];
    assert_shared(&s, &all);
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

#[test]
fn relink_items_relink_a_group_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let file = dir.path().join("a/art.svg");
    write(&file, svg(false, 0));
    let id = import_file(&mut s, &file);
    let all = linked(&mut s, id, 2);
    let one = dir.path().join("b/art.svg");
    let two = dir.path().join("c/art.svg");
    write(&one, svg(true, 0));
    write(&two, svg(false, 0));
    let r = run(
        &mut s,
        "object.relink",
        json!({ "items": [{ "id": all[0], "path": one }, { "id": all[2], "path": two }] }),
    );
    // Every member once; the second item would move the group again.
    assert_eq!(ids(&r["reloaded"]), all, "{r}");
    let failed = r["failed"].as_array().unwrap();
    assert_eq!(failed.len(), 1, "{r}");
    assert_eq!(failed[0]["id"], all[2]);
    assert!(
        failed[0]["reason"]
            .as_str()
            .unwrap()
            .contains("linked copy"),
        "{r}"
    );
    for &m in &all {
        assert_eq!(obj(&s, m).link.as_ref().unwrap().path, one);
    }
}
