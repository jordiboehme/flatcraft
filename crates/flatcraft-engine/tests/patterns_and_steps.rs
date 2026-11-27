//! The arrange panels' commands: object.circular_copies, object.duplicate
//! with a count and object.copies with its own gaps per axis.

use std::path::Path;

use flatcraft_core::document::ObjectId;
use flatcraft_device::ProfileStore;
use flatcraft_engine::commands::placed_bounds;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{CommandError, Origin, Session};
use kurbo::Point;
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

/// A 10 x 20 mm rectangle, moved so its centre is at `at`.
fn rect_at(s: &mut Session, at: Point) -> u64 {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="20mm" viewBox="0 0 10 20"><rect x="0" y="0" width="10" height="20"/></svg>"#;
    let id = run(s, "object.import_svg", json!({ "svg": svg, "name": "r" }))["id"]
        .as_u64()
        .unwrap();
    run(
        s,
        "object.move",
        json!({ "ids": [id], "center_mm": [at.x, at.y] }),
    );
    id
}

fn ids(v: &Value) -> Vec<u64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap())
        .collect()
}

fn centre(s: &Session, id: u64) -> Point {
    s.doc()
        .document
        .get(ObjectId(id))
        .unwrap()
        .transform
        .center_mm
}

fn rotation(s: &Session, id: u64) -> f64 {
    s.doc()
        .document
        .get(ObjectId(id))
        .unwrap()
        .transform
        .rotation_deg
}

fn close(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-9
}

#[test]
fn four_around_a_full_turn_in_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = rect_at(&mut s, Point::new(110.0, 100.0));
    let undo = s.doc().undo.undo_len();
    let r = run(
        &mut s,
        "object.circular_copies",
        json!({ "ids": [id], "count": 4, "center_mm": [100.0, 100.0] }),
    );
    assert_eq!(s.doc().undo.undo_len(), undo + 1);
    let new = ids(&r["ids"]);
    assert_eq!(new.len(), 3);
    let want = [
        (Point::new(100.0, 110.0), 90.0),
        (Point::new(90.0, 100.0), 180.0),
        (Point::new(100.0, 90.0), 270.0),
    ];
    for (n, (p, a)) in new.iter().zip(want) {
        assert!(close(centre(&s, *n), p), "{n}: {:?}", centre(&s, *n));
        assert_eq!(rotation(&s, *n), a);
    }
    assert_eq!(r["center_mm"], json!([100.0, 100.0]));
    let sel: Vec<u64> = s.doc().selection.iter().map(|i| i.0).collect();
    assert_eq!(sel, new, "the copies become the selection");
    // A copy at 90 degrees has the original's box, turned.
    let size = |id: u64| {
        placed_bounds(
            s.doc().document.get(ObjectId(id)).unwrap(),
            Default::default(),
        )
        .unwrap()
    };
    let (o, c) = (size(id), size(new[0]));
    assert!((o.width() - c.height()).abs() < 1e-6 && (o.height() - c.width()).abs() < 1e-6);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().document.objects.len(), 1);
}

#[test]
fn three_over_a_half_turn_without_turning_the_copies() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = rect_at(&mut s, Point::new(60.0, 50.0));
    let r = run(
        &mut s,
        "object.circular_copies",
        json!({
            "ids": [id],
            "count": 3,
            "total_deg": 180.0,
            "center_mm": [50.0, 50.0],
            "rotate_copies": false,
        }),
    );
    let new = ids(&r["ids"]);
    assert!(close(centre(&s, new[0]), Point::new(50.0, 60.0)));
    assert!(close(centre(&s, new[1]), Point::new(40.0, 50.0)));
    assert!(new.iter().all(|n| rotation(&s, *n) == 0.0));
}

#[test]
fn the_default_centre_is_the_centre_of_the_common_box() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = rect_at(&mut s, Point::new(50.0, 50.0));
    let b = rect_at(&mut s, Point::new(90.0, 50.0));
    run(&mut s, "object.select", json!({ "ids": [a, b] }));
    let r = run(&mut s, "object.circular_copies", json!({ "count": 2 }));
    assert_eq!(r["center_mm"], json!([70.0, 50.0]));
    // Half a turn about (70, 50) swaps the two places.
    let new = ids(&r["ids"]);
    assert_eq!(new.len(), 2);
    assert!(close(centre(&s, new[0]), Point::new(90.0, 50.0)));
    assert!(close(centre(&s, new[1]), Point::new(50.0, 50.0)));
}

#[test]
fn linked_circular_copies_follow_the_copies_rules() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = rect_at(&mut s, Point::new(60.0, 50.0));
    let r = run(
        &mut s,
        "object.circular_copies",
        json!({ "ids": [a], "count": 3, "center_mm": [50.0, 50.0], "linked": true }),
    );
    let new = ids(&r["ids"]);
    let doc = &s.doc().document;
    let g = doc.get(ObjectId(a)).unwrap().link_group;
    assert!(g.is_some());
    assert!(
        new.iter()
            .all(|n| doc.get(ObjectId(*n)).unwrap().link_group == g)
    );
    assert!(doc.link_groups_agree());
    // Shared: the scale follows; own: the rotation stays.
    run(&mut s, "object.scale", json!({ "ids": [a], "factor": 2.0 }));
    let o = s.doc().document.get(ObjectId(new[1])).unwrap();
    assert_eq!(o.transform.scale_x, 2.0);
    assert_eq!(rotation(&s, new[1]), 240.0);
    // Without linked, a copy of a linked copy is independent.
    let r = run(
        &mut s,
        "object.circular_copies",
        json!({ "ids": [new[0]], "count": 2 }),
    );
    let lone = ids(&r["ids"])[0];
    assert_eq!(
        s.doc().document.get(ObjectId(lone)).unwrap().link_group,
        None
    );
}

#[test]
fn circular_copies_outside_the_area_are_listed() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = rect_at(&mut s, Point::new(30.0, 30.0));
    let r = run(
        &mut s,
        "object.circular_copies",
        json!({ "ids": [a], "count": 2, "center_mm": [20.0, 20.0] }),
    );
    // Half a turn about (20, 20) puts it at (10, 10): off the cutting area
    // of an uncalibrated mat (5 mm inset) by its own half size.
    let new = ids(&r["ids"]);
    assert_eq!(ids(&r["outside_area"]), new);
    assert_eq!(r["warnings"].as_array().unwrap().len(), 1);
    // A hidden original's copies are not counted.
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [a], "visible": false }),
    );
    let r = run(
        &mut s,
        "object.circular_copies",
        json!({ "ids": [a], "count": 2, "center_mm": [20.0, 20.0] }),
    );
    assert!(ids(&r["outside_area"]).is_empty());
}

#[test]
fn bad_circular_params_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = rect_at(&mut s, Point::new(60.0, 50.0));
    let revision = s.doc().revision();
    for p in [
        json!({ "ids": [a], "count": 1 }),
        json!({ "ids": [a], "count": 502 }),
        json!({ "ids": [a], "count": 3, "total_deg": 0.0 }),
        json!({ "ids": [a], "count": 3, "total_deg": 400.0 }),
        json!({ "ids": [a], "count": 3, "rows": 2 }),
    ] {
        let e = s
            .execute("object.circular_copies", p.clone(), Origin::User)
            .unwrap_err();
        assert!(matches!(e, CommandError::InvalidParams(_)), "{p}: {e:?}");
    }
    let e = s
        .execute(
            "object.circular_copies",
            json!({ "ids": [a], "count": 3, "center_mm": [5.0e7, 0.0] }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "{e:?}");
    assert_eq!(s.doc().revision(), revision);
    assert_eq!(s.doc().document.objects.len(), 1);
}

#[test]
fn an_agent_makes_circular_copies() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = rect_at(&mut s, Point::new(110.0, 100.0));
    let r = s
        .execute(
            "object.circular_copies",
            json!({ "ids": [a], "count": 6, "center_mm": [100.0, 100.0] }),
            Origin::Agent,
        )
        .unwrap();
    assert_eq!(ids(&r["ids"]).len(), 5);
    assert_eq!(s.doc().document.objects.len(), 6);
    let schema = serde_json::to_value(
        flatcraft_engine::registry::get("object.circular_copies")
            .unwrap()
            .params_schema(),
    )
    .unwrap();
    assert_eq!(schema["required"], json!(["count"]));
}

#[test]
fn duplicate_with_a_count_steps_the_copies() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = rect_at(&mut s, Point::new(50.0, 50.0));
    let b = rect_at(&mut s, Point::new(80.0, 50.0));
    let undo = s.doc().undo.undo_len();
    let r = run(
        &mut s,
        "object.duplicate",
        json!({ "ids": [a, b], "offset_mm": [0.0, 25.0], "count": 3 }),
    );
    assert_eq!(s.doc().undo.undo_len(), undo + 1);
    let new = ids(&r["ids"]);
    assert_eq!(new.len(), 6);
    assert!(close(centre(&s, new[0]), Point::new(50.0, 75.0)));
    assert!(close(centre(&s, new[1]), Point::new(80.0, 75.0)));
    assert!(close(centre(&s, new[5]), Point::new(80.0, 125.0)));
    for p in [
        json!({ "ids": [a], "count": 0 }),
        json!({ "ids": [a, b], "count": 251 }),
    ] {
        assert!(s.execute("object.duplicate", p, Origin::User).is_err());
    }
}

#[test]
fn copies_take_a_gap_per_axis() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = rect_at(&mut s, Point::new(50.0, 50.0));
    let r = run(
        &mut s,
        "object.copies",
        json!({
            "id": a,
            "count": 3,
            "layout": { "grid": { "columns": 2 } },
            "gap_x_mm": 1.0,
            "gap_y_mm": 3.0,
        }),
    );
    let new = ids(&r["ids"]);
    // Laid out by the cut bounds of the original.
    let b = placed_bounds(
        s.doc().document.get(ObjectId(a)).unwrap(),
        Default::default(),
    )
    .unwrap();
    let (x, y) = (50.0 + b.width() + 1.0, 50.0 + b.height() + 3.0);
    assert!(close(centre(&s, new[0]), Point::new(x, 50.0)));
    assert!(close(centre(&s, new[1]), Point::new(50.0, y)));
    assert!(close(centre(&s, new[2]), Point::new(x, y)));
    let e = s
        .execute(
            "object.copies",
            json!({ "id": a, "count": 1, "gap_y_mm": -1.0 }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)));
}
