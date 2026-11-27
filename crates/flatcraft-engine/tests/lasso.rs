//! `selection.set_in_polygon`: the agent's box and lasso, with the same
//! rule as the canvas (`flatcraft_engine::lasso`).

use std::path::Path;

use flatcraft_core::document::ObjectId;
use flatcraft_device::ProfileStore;
use flatcraft_engine::commands::{knife_of, placed_bounds};
use flatcraft_engine::lasso::{self, DocumentOutlines, Lasso};
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{CommandError, Origin, Session};
use kurbo::{Point, Rect};
use serde_json::{Value, json};

fn session(dir: &Path) -> Session {
    let profiles = ProfileStore::load(dir).unwrap();
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

/// A `w` x `w` mm square at x, y.
fn square(s: &mut Session, x: f64, y: f64, w: f64) -> u64 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="300mm" height="300mm" viewBox="0 0 300 300"><rect x="{x}" y="{y}" width="{w}" height="{w}"/></svg>"#
    );
    run(
        s,
        "object.import_svg",
        json!({ "svg": svg, "name": "square" }),
    )["id"]
        .as_u64()
        .unwrap()
}

fn bounds(s: &Session, id: u64) -> Rect {
    placed_bounds(s.doc().document.get(ObjectId(id)).unwrap(), knife_of(s)).unwrap()
}

fn selection(s: &Session) -> Vec<u64> {
    s.doc().selection.iter().map(|i| i.0).collect()
}

/// The corners of `r` grown by `by` mm, as `points_mm`.
fn around(r: Rect, by: f64) -> Value {
    let r = r.inflate(by, by);
    json!([[r.x0, r.y0], [r.x1, r.y0], [r.x1, r.y1], [r.x0, r.y1]])
}

fn select_in(s: &mut Session, params: Value) -> Value {
    run(s, "selection.set_in_polygon", params)
}

/// Three squares side by side: a, b and c.
fn three(dir: &Path) -> (Session, [u64; 3]) {
    let mut s = session(dir);
    let ids = [0.0, 30.0, 60.0].map(|x| square(&mut s, 20.0 + x, 20.0, 20.0));
    run(&mut s, "object.select", json!({ "ids": [] }));
    (s, ids)
}

#[test]
fn fully_inside_by_default_and_touching_with_touch() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, [a, b, c]) = three(dir.path());
    let (ba, bb) = (bounds(&s, a), bounds(&s, b));
    // From around a to the middle of b.
    let r = Rect::new(ba.x0 - 2.0, ba.y0 - 2.0, bb.center().x, ba.y1 + 2.0);
    let out = select_in(&mut s, json!({ "points_mm": around(r, 0.0) }));
    assert_eq!(out["selection"], json!([a]));
    let out = select_in(
        &mut s,
        json!({ "points_mm": around(r, 0.0), "touch": true }),
    );
    assert_eq!(out["selection"], json!([a, b]));
    assert_eq!(out["picked"], json!([a, b]));
    assert!(!selection(&s).contains(&c));
}

#[test]
fn modes_replace_add_remove_and_toggle() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, [a, b, c]) = three(dir.path());
    let (pa, pb, pc) = (
        around(bounds(&s, a), 1.0),
        around(bounds(&s, b), 1.0),
        around(bounds(&s, c), 1.0),
    );
    select_in(&mut s, json!({ "points_mm": pa }));
    assert_eq!(selection(&s), [a]);
    select_in(&mut s, json!({ "points_mm": pb, "mode": "add" }));
    assert_eq!(selection(&s), [a, b]);
    select_in(&mut s, json!({ "points_mm": pa, "mode": "remove" }));
    assert_eq!(selection(&s), [b]);
    // Toggle: b leaves, c joins.
    let both = {
        let r = bounds(&s, b).union(bounds(&s, c));
        around(r, 1.0)
    };
    select_in(&mut s, json!({ "points_mm": both, "mode": "toggle" }));
    assert_eq!(selection(&s), [c]);
    select_in(&mut s, json!({ "points_mm": pc, "mode": "replace" }));
    assert_eq!(selection(&s), [c]);
}

#[test]
fn a_group_is_picked_whole_and_inside_a_group_its_members() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, [a, b, c]) = three(dir.path());
    let g = run(&mut s, "object.group", json!({ "ids": [a, b] }))["group_id"]
        .as_u64()
        .unwrap();
    let pa = around(bounds(&s, a), 1.0);
    // Fully inside: a alone is not all of the group.
    let out = select_in(&mut s, json!({ "points_mm": pa }));
    assert_eq!(out["selection"], json!([]));
    // Touching a member picks the whole group.
    let out = select_in(&mut s, json!({ "points_mm": pa, "touch": true }));
    assert_eq!(out["selection"], json!([a, b]));
    // All of the group inside: the whole group.
    let ab = around(bounds(&s, a).union(bounds(&s, b)), 1.0);
    let out = select_in(&mut s, json!({ "points_mm": ab }));
    assert_eq!(out["selection"], json!([a, b]));
    // Inside the group its members count one by one, and nothing outside
    // it is picked (c is in the shape too).
    let all = around(bounds(&s, a).union(bounds(&s, c)), 1.0);
    let out = select_in(&mut s, json!({ "points_mm": pa, "group": g }));
    assert_eq!(out["selection"], json!([a]));
    let out = select_in(&mut s, json!({ "points_mm": all, "group": g }));
    assert_eq!(out["selection"], json!([a, b]));
    // A group id that is no group is refused.
    let e = s
        .execute(
            "selection.set_in_polygon",
            json!({ "points_mm": pa, "group": c }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
}

#[test]
fn hidden_objects_are_never_picked() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, [a, b, c]) = three(dir.path());
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [b], "visible": false }),
    );
    let all = around(bounds(&s, a).union(bounds(&s, c)), 1.0);
    let out = select_in(&mut s, json!({ "points_mm": all, "touch": true }));
    assert_eq!(out["selection"], json!([a, c]));
}

#[test]
fn a_shape_without_area_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = three(dir.path());
    for points in [
        json!([[0, 0], [10, 0]]),
        json!([[0, 0], [10, 0], [20, 0]]),
        json!([]),
    ] {
        let e = s
            .execute(
                "selection.set_in_polygon",
                json!({ "points_mm": points }),
                Origin::User,
            )
            .unwrap_err();
        assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
    }
}

#[test]
fn the_command_changes_no_document_and_adds_no_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, [a, ..]) = three(dir.path());
    let undo = s.doc().undo.undo_len();
    let rev = s.doc().revision();
    let centres: Vec<_> = s
        .doc()
        .document
        .objects
        .iter()
        .map(|o| o.transform)
        .collect();
    let pa = around(bounds(&s, a), 1.0);
    select_in(&mut s, json!({ "points_mm": pa, "touch": true }));
    assert_eq!(selection(&s), [a]);
    assert_eq!(s.doc().undo.undo_len(), undo);
    assert_eq!(s.doc().revision(), rev);
    let after: Vec<_> = s
        .doc()
        .document
        .objects
        .iter()
        .map(|o| o.transform)
        .collect();
    assert_eq!(after, centres);
    let c = flatcraft_engine::registry::get("selection.set_in_polygon").unwrap();
    assert!(!c.mutates_document());
}

/// 2000 small squares: a lasso over a few of them tests the outline of
/// only those near it; the box test skips the rest.
#[test]
fn the_box_test_skips_far_objects_among_2000() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    // Four blocks of 500 squares (2 mm, 1 mm apart), 50 to a row.
    for block in 0..4 {
        let id = square(&mut s, 10.0, 10.0 + 40.0 * block as f64, 2.0);
        run(
            &mut s,
            "object.copies",
            json!({ "id": id, "count": 499, "layout": { "grid": { "columns": 50 } }, "gap_mm": 1.0 }),
        );
    }
    assert_eq!(s.doc().document.objects.len(), 2000);
    let first = s.doc().document.objects[0].id;
    let b = bounds(&s, first.0);
    // Around the first 3 x 3 squares of the first block.
    let r = Rect::new(b.x0 - 0.5, b.y0 - 0.5, b.x0 + 8.5, b.y0 + 8.5);
    let shape = Lasso::rect(r).unwrap();
    let doc = &s.doc().document;
    let tree = doc.tree();
    let outlines = DocumentOutlines::new(doc, knife_of(&s));
    let picked = lasso::pick(&tree, &outlines, &shape, false, None);
    assert_eq!(picked.ids.len(), 9);
    assert!(picked.outline_tests <= 9, "{}", picked.outline_tests);
    let touched = lasso::pick(&tree, &outlines, &shape, true, None);
    assert_eq!(touched.ids.len(), 9);
    assert!(touched.outline_tests <= 16, "{}", touched.outline_tests);
    // A free shape: a triangle over the corner takes fewer.
    let tri = Lasso::new([
        Point::new(b.x0 - 0.5, b.y0 - 0.5),
        Point::new(b.x0 + 12.0, b.y0 - 0.5),
        Point::new(b.x0 - 0.5, b.y0 + 12.0),
    ])
    .unwrap();
    let picked = lasso::pick(&tree, &outlines, &tri, false, None);
    assert!(
        picked.ids.len() < 16 && !picked.ids.is_empty(),
        "{:?}",
        picked.ids
    );
    assert!(picked.outline_tests <= 16, "{}", picked.outline_tests);
}

/// Stickers on a backing sheet: a lasso around two of them picks those
/// two, not the sheet it was drawn inside.
#[test]
fn a_lasso_inside_a_backing_sheet_leaves_the_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let sheet = square(&mut s, 10.0, 10.0, 150.0);
    let a = square(&mut s, 30.0, 30.0, 20.0);
    let b = square(&mut s, 70.0, 30.0, 20.0);
    let c = square(&mut s, 110.0, 30.0, 20.0);
    let ab = around(bounds(&s, a).union(bounds(&s, b)), 3.0);
    let out = select_in(&mut s, json!({ "points_mm": ab, "touch": true }));
    assert_eq!(out["selection"], json!([a, b]));
    assert!(!selection(&s).contains(&sheet) && !selection(&s).contains(&c));
}
