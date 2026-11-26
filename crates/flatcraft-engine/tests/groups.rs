//! Group and Ungroup (Task 57): a group acts as one object for selection,
//! moves, transforms, arrange and the weed box per object, while every
//! member keeps its own geometry and place.

use flatcraft_core::document::ObjectId;
use flatcraft_device::ProfileStore;
use flatcraft_engine::job;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{CommandError, Origin, Session};
use flatcraft_path::ToolpathKind;
use kurbo::{Point, Rect};
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

fn fails(s: &mut Session, id: &str, params: Value) -> CommandError {
    s.execute(id, params, Origin::User)
        .expect_err(&format!("{id} should fail"))
}

/// A `w` x `h` mm rectangle, placed with its top-left at (x, y).
fn rect(s: &mut Session, name: &str, x: f64, y: f64, w: f64, h: f64) -> u64 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}mm" height="{h}mm" viewBox="0 0 {w} {h}"><path d="M 0 0 H {w} V {h} H 0 Z M {a} {a} H {b} V {c} H {a} Z"/></svg>"#,
        a = 1.0,
        b = w - 1.0,
        c = h - 1.0,
    );
    let id = run(s, "object.import_svg", json!({ "svg": svg, "name": name }))["id"]
        .as_u64()
        .unwrap();
    run(
        s,
        "object.move",
        json!({ "ids": [id], "x_mm": x, "y_mm": y }),
    );
    id
}

/// Every placed path point of every object, by id: what is cut.
fn placed(s: &Session) -> Vec<(u64, Vec<Point>)> {
    s.doc()
        .document
        .objects
        .iter()
        .map(|o| {
            let pts = o
                .placed_paths()
                .iter()
                .flat_map(|p| {
                    p.path
                        .elements()
                        .iter()
                        .filter_map(|e| e.end_point())
                        .collect::<Vec<_>>()
                })
                .collect();
            (o.id.0, pts)
        })
        .collect()
}

fn bounds(s: &mut Session, id: u64) -> Rect {
    let state = run(s, "document.state", json!({}));
    let o = state["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .unwrap()
        .clone();
    let b = &o["bounds_mm"];
    Rect::new(
        b["x0"].as_f64().unwrap(),
        b["y0"].as_f64().unwrap(),
        b["x1"].as_f64().unwrap(),
        b["y1"].as_f64().unwrap(),
    )
}

fn group(s: &mut Session, ids: &[u64]) -> u64 {
    run(s, "object.group", json!({ "ids": ids }))["group_id"]
        .as_u64()
        .unwrap()
}

fn parent(s: &Session, id: u64) -> Option<u64> {
    let doc = &s.doc().document;
    match doc.get(ObjectId(id)) {
        Some(o) => o.parent.map(|p| p.0),
        None => doc
            .group_node(ObjectId(id))
            .and_then(|g| g.parent)
            .map(|p| p.0),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-5
}

fn close_rect(a: Rect, b: Rect) -> bool {
    close(a.x0, b.x0) && close(a.y0, b.y0) && close(a.x1, b.x1) && close(a.y1, b.y1)
}

#[test]
fn group_and_ungroup_keep_every_point_exactly() {
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 10.0, 10.0, 20.0, 10.0);
    let b = rect(&mut s, "b", 40.0, 10.0, 10.0, 30.0);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [b], "degrees": 17.0 }),
    );
    let before = placed(&s);
    let g = group(&mut s, &[a, b]);
    assert_eq!(placed(&s), before, "grouping moves nothing");
    assert_eq!(parent(&s, a), Some(g));
    assert_eq!(parent(&s, b), Some(g));
    // The group's id is new, from the object counter.
    assert!(g > a.max(b));
    let state = run(&mut s, "document.state", json!({}));
    assert_eq!(state["groups"][0]["id"], g);
    assert_eq!(state["groups"][0]["name"], "Group 1");
    assert_eq!(state["groups"][0]["members"], json!([a, b]));
    assert_eq!(state["objects"][0]["group"], g);
    let reply = run(&mut s, "object.ungroup", json!({ "ids": [g] }));
    assert_eq!(reply["dissolved"], json!([g]));
    assert_eq!(placed(&s), before, "ungrouping moves nothing");
    assert!(s.doc().document.groups.is_empty());
    assert_eq!(parent(&s, a), None);
    // Each is one undo step.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(parent(&s, a), Some(g));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(parent(&s, a), None);
    assert!(s.doc().document.groups.is_empty());
    assert_eq!(placed(&s), before);
}

#[test]
fn group_needs_two_units_and_ungroup_a_group() {
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 10.0, 10.0, 20.0, 10.0);
    let b = rect(&mut s, "b", 40.0, 10.0, 10.0, 30.0);
    let e = fails(&mut s, "object.group", json!({ "ids": [a] }));
    assert!(
        e.to_string().contains("Select 2 or more objects first."),
        "{e}"
    );
    let e = fails(&mut s, "object.ungroup", json!({ "ids": [a] }));
    assert!(e.to_string().contains("Select a group first."), "{e}");
    let g = group(&mut s, &[a, b]);
    // A whole group counts as one: grouping it alone is refused.
    let e = fails(&mut s, "object.group", json!({ "ids": [g] }));
    assert!(e.to_string().contains("2 or more"), "{e}");
    let e = fails(&mut s, "object.group", json!({ "ids": [a, b] }));
    assert!(e.to_string().contains("2 or more"), "{e}");
}

#[test]
fn nested_groups_and_releasing_members() {
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 10.0, 10.0, 10.0, 10.0);
    let b = rect(&mut s, "b", 30.0, 10.0, 10.0, 10.0);
    let c = rect(&mut s, "c", 50.0, 10.0, 10.0, 10.0);
    let e = rect(&mut s, "e", 70.0, 10.0, 10.0, 10.0);
    let inner = group(&mut s, &[a, b]);
    let outer = group(&mut s, &[inner, c, e]);
    assert_eq!(parent(&s, inner), Some(outer));
    assert_eq!(parent(&s, a), Some(inner));
    let before = placed(&s);

    // Selecting a group id selects all its objects.
    let sel = run(&mut s, "object.select", json!({ "ids": [outer] }));
    assert_eq!(sel["selection"], json!([a, b, c, e]));

    // Ungroup of the whole outer group goes one level: the inner stays.
    run(&mut s, "object.ungroup", json!({ "ids": [outer] }));
    assert_eq!(parent(&s, inner), None);
    assert_eq!(parent(&s, c), None);
    assert_eq!(parent(&s, a), Some(inner));
    run(&mut s, "edit.undo", json!({}));

    // Release one member of the outer group: only it leaves.
    let reply = run(&mut s, "object.ungroup", json!({ "ids": [c] }));
    assert_eq!(reply["released"], json!([c]));
    assert_eq!(parent(&s, c), None);
    assert_eq!(parent(&s, e), Some(outer));
    assert_eq!(placed(&s), before);
    run(&mut s, "edit.undo", json!({}));

    // A nested release goes one level up, into the outer group.
    run(&mut s, "object.ungroup", json!({ "ids": [a] }));
    assert_eq!(parent(&s, a), Some(outer));
    // The inner group was left with one member: it dissolved.
    assert!(s.doc().document.group_node(ObjectId(inner)).is_none());
    assert_eq!(parent(&s, b), Some(outer));
    assert_eq!(placed(&s), before);
    run(&mut s, "edit.undo", json!({}));

    // Release all but one: the group dissolves, its last member (the
    // inner group) stays a group at the top level.
    run(&mut s, "object.ungroup", json!({ "ids": [c, e] }));
    assert!(s.doc().document.group_node(ObjectId(outer)).is_none());
    assert_eq!(parent(&s, inner), None);
    assert_eq!(parent(&s, a), Some(inner));
    assert_eq!(placed(&s), before);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(parent(&s, inner), Some(outer));
}

#[test]
fn a_group_moves_turns_and_mirrors_as_one() {
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 10.0, 10.0, 20.0, 10.0);
    let b = rect(&mut s, "b", 40.0, 30.0, 10.0, 10.0);
    let lone = rect(&mut s, "lone", 100.0, 10.0, 10.0, 10.0);
    let g = group(&mut s, &[a, b]);
    // Move by the group id.
    run(
        &mut s,
        "object.move",
        json!({ "ids": [g], "dx_mm": 5.0, "dy_mm": 1.0 }),
    );
    let ba = bounds(&mut s, a);
    assert!(close_rect(ba, Rect::new(15.0, 11.0, 35.0, 21.0)), "{ba:?}");
    assert!(close_rect(
        bounds(&mut s, b),
        Rect::new(45.0, 31.0, 55.0, 41.0)
    ));
    // Rotate about each: the group turns about its own box centre, as one
    // piece, and the lone object about its own centre.
    let box_before = bounds(&mut s, a).union(bounds(&mut s, b));
    let lone_before = bounds(&mut s, lone);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [g, lone], "degrees": 90.0, "about": "each" }),
    );
    let box_after = bounds(&mut s, a).union(bounds(&mut s, b));
    assert!(close(box_after.center().x, box_before.center().x));
    assert!(close(box_after.center().y, box_before.center().y));
    assert!(close(box_after.width(), box_before.height()));
    assert!(close_rect(
        bounds(&mut s, lone),
        Rect::from_center_size(lone_before.center(), (10.0, 10.0))
    ));
    assert_eq!(
        s.doc()
            .document
            .group_node(ObjectId(g))
            .unwrap()
            .rotation_deg,
        90.0
    );
    // The Angle field: an absolute angle turns the group as one piece
    // until its own angle is the angle.
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [g], "angle_deg": 0.0 }),
    );
    let back = bounds(&mut s, a).union(bounds(&mut s, b));
    assert!(close(back.width(), box_before.width()), "{back:?}");
    assert!(close(back.center().x, box_before.center().x));
    // Mirror about each: the group flips across its own box, which stays.
    let ra = bounds(&mut s, a);
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [g], "axis": "horizontal", "about": "each" }),
    );
    let flipped = bounds(&mut s, a).union(bounds(&mut s, b));
    assert!(close_rect(flipped, back));
    let fa = bounds(&mut s, a);
    // a was on the left of the group, now on the right.
    assert!(
        close(fa.x1, back.x1 - (ra.x0 - back.x0)),
        "{fa:?} {ra:?} {back:?}"
    );
    // Scale about each: the group's box doubles about its centre.
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [g, lone], "factor": 2.0 }),
    );
    let big = bounds(&mut s, a).union(bounds(&mut s, b));
    assert!(close(big.width(), 2.0 * back.width()));
    assert!(close(big.center().x, back.center().x));
    // The duplicate of a group is a new group with copies of its members.
    let dup = run(&mut s, "object.duplicate", json!({ "ids": [g] }));
    let copy = dup["ids"][0].as_u64().unwrap();
    let doc = &s.doc().document;
    assert!(doc.group_node(ObjectId(copy)).is_some());
    assert_eq!(doc.tree().leaves_of(ObjectId(copy)).len(), 2);
    assert_eq!(s.doc().selection.len(), 2);
    // Hide and delete by the group id.
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [copy], "visible": false }),
    );
    let hidden = s
        .doc()
        .document
        .objects
        .iter()
        .filter(|o| !o.visible)
        .count();
    assert_eq!(hidden, 2);
    run(&mut s, "object.delete", json!({ "ids": [copy] }));
    assert!(s.doc().document.group_node(ObjectId(copy)).is_none());
    assert_eq!(s.doc().document.objects.len(), 3);
}

#[test]
fn align_and_distribute_treat_a_group_as_one_box() {
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 10.0, 10.0, 10.0, 10.0);
    let b = rect(&mut s, "b", 30.0, 40.0, 10.0, 10.0);
    let c = rect(&mut s, "c", 80.0, 20.0, 10.0, 10.0);
    let g = group(&mut s, &[a, b]);
    run(
        &mut s,
        "arrange.align",
        json!({ "ids": [g, c], "mode": "top" }),
    );
    // The group's box top meets c's top; a and b keep their offsets.
    let ga = bounds(&mut s, a);
    let gb = bounds(&mut s, b);
    let gc = bounds(&mut s, c);
    assert!(close(ga.y0, gc.y0));
    assert!(close(gb.y0 - ga.y0, 30.0), "members keep their layout");
    // Distribute needs 3 units: a group and one object are 2.
    let e = fails(
        &mut s,
        "arrange.distribute",
        json!({ "ids": [g, c], "axis": "horizontal" }),
    );
    assert!(e.to_string().contains("at least 3"), "{e}");
}

#[test]
fn auto_arrange_packs_a_group_as_one_item() {
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 100.0, 100.0, 10.0, 10.0);
    let b = rect(&mut s, "b", 115.0, 100.0, 10.0, 10.0);
    let c = rect(&mut s, "c", 150.0, 150.0, 20.0, 30.0);
    let g = group(&mut s, &[a, b]);
    let before_gap = bounds(&mut s, b).x0 - bounds(&mut s, a).x1;
    run(&mut s, "object.select", json!({ "ids": [] }));
    let reply = run(
        &mut s,
        "arrange.auto",
        json!({ "ids": [] , "spacing_mm": 5.0 }),
    );
    let placed: Vec<u64> = reply["placed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert!(placed.contains(&g), "{reply}");
    assert!(placed.contains(&c), "{reply}");
    let ba = bounds(&mut s, a);
    let bb = bounds(&mut s, b);
    assert!(
        close(bb.x0 - ba.x1, before_gap),
        "the group keeps its layout"
    );
    assert!(close(bb.y0, ba.y0));
    // A member picked inside the group packs alone; the rest of the group
    // stays exactly where it is.
    let b_before = bounds(&mut s, b);
    let reply = run(
        &mut s,
        "arrange.auto",
        json!({ "ids": [a], "spacing_mm": 5.0 }),
    );
    assert!(
        !reply["placed"].as_array().unwrap().contains(&json!(g)),
        "{reply}"
    );
    assert_eq!(bounds(&mut s, b), b_before, "the rest of the group stays");
}

#[test]
fn the_weed_box_per_object_puts_one_box_around_a_group() {
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 20.0, 20.0, 10.0, 10.0);
    let b = rect(&mut s, "b", 60.0, 20.0, 10.0, 10.0);
    let _c = rect(&mut s, "c", 120.0, 20.0, 10.0, 10.0);
    run(
        &mut s,
        "job.set_weeding",
        json!({ "box": true, "per_object": true, "margin_mm": 3.0 }),
    );
    let boxes = |s: &Session| {
        job::build(s)
            .unwrap()
            .plan
            .toolpaths
            .iter()
            .filter(|t| {
                t.kind == ToolpathKind::Weed
                    && t.closed
                    && t.pass == 1
                    && t.source.object == flatcraft_path::weed::WEED_BOX_OBJECT
            })
            .count()
    };
    assert_eq!(boxes(&s), 3);
    group(&mut s, &[a, b]);
    assert_eq!(boxes(&s), 2, "one box for the group, one for c");
}

#[test]
fn groups_survive_save_and_open() {
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 10.0, 10.0, 10.0, 10.0);
    let b = rect(&mut s, "b", 30.0, 10.0, 10.0, 10.0);
    let c = rect(&mut s, "c", 50.0, 10.0, 10.0, 10.0);
    let inner = group(&mut s, &[a, b]);
    let outer = group(&mut s, &[inner, c]);
    run(
        &mut s,
        "object.rename",
        json!({ "id": outer, "name": "Badge" }),
    );
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [outer], "degrees": 30.0 }),
    );
    let before = placed(&s);
    let path = d.out.path().join("groups.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    let mut s2 = session(&d);
    run(&mut s2, "project.open", json!({ "path": path }));
    let doc = &s2.doc().document;
    assert_eq!(doc.groups.len(), 2);
    let o = doc.group_node(ObjectId(outer)).unwrap();
    assert_eq!(o.name, "Badge");
    assert!(close(o.rotation_deg, 30.0));
    assert_eq!(parent(&s2, inner), Some(outer));
    assert_eq!(parent(&s2, a), Some(inner));
    assert_eq!(placed(&s2), before);
    // A new id never names an existing group.
    let d2 = rect(&mut s2, "d", 80.0, 10.0, 10.0, 10.0);
    assert!(d2 > outer);
}

#[test]
fn linked_copies_inside_a_group_still_link() {
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 10.0, 10.0, 10.0, 10.0);
    let copies = run(
        &mut s,
        "object.copies",
        json!({ "id": a, "count": 1, "linked": true }),
    );
    let copy = copies["ids"][0].as_u64().unwrap();
    let g = group(&mut s, &[a, copy]);
    // Scaling one member alone (picked inside the group) scales its
    // linked copy too.
    run(&mut s, "object.scale", json!({ "ids": [a], "factor": 2.0 }));
    let doc = &s.doc().document;
    let sx = |id: u64| doc.get(ObjectId(id)).unwrap().transform.scale_x;
    assert_eq!(sx(a), sx(copy));
    assert_eq!(sx(a), 2.0);
    assert!(doc.link_groups_agree());
    assert!(doc.group_node(ObjectId(g)).is_some());
    // Copies of a whole group make a new group.
    let reply = run(&mut s, "object.copies", json!({ "id": g, "count": 2 }));
    let ids = reply["ids"].as_array().unwrap();
    assert_eq!(ids.len(), 2);
    for id in ids {
        assert!(
            s.doc()
                .document
                .group_node(ObjectId(id.as_u64().unwrap()))
                .is_some()
        );
    }
}

#[test]
fn move_to_group_takes_objects_in_and_out() {
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 10.0, 10.0, 10.0, 10.0);
    let b = rect(&mut s, "b", 30.0, 10.0, 10.0, 10.0);
    let c = rect(&mut s, "c", 50.0, 10.0, 10.0, 10.0);
    let g = group(&mut s, &[a, b]);
    let before = placed(&s);
    run(
        &mut s,
        "object.move_to_group",
        json!({ "ids": [c], "group_id": g }),
    );
    assert_eq!(parent(&s, c), Some(g));
    run(&mut s, "object.move_to_group", json!({ "ids": [c] }));
    assert_eq!(parent(&s, c), None);
    // Taking one of two out dissolves the group.
    run(&mut s, "object.move_to_group", json!({ "ids": [a] }));
    assert!(s.doc().document.groups.is_empty());
    assert_eq!(placed(&s), before);
    let e = fails(
        &mut s,
        "object.move_to_group",
        json!({ "ids": [a], "group_id": b }),
    );
    assert!(e.to_string().contains("not a group"), "{e}");
}

#[test]
fn a_broken_group_tree_in_a_file_is_mended() {
    let mut doc = flatcraft_core::document::Document::new(flatcraft_core::document::JobSettings {
        plotter_id: String::new(),
        material_id: String::new(),
        media_id: String::new(),
        speed: None,
        force: None,
        passes: None,
        strategy: Default::default(),
        weed: Default::default(),
    });
    let src = || flatcraft_core::document::Source::Svg {
        name: "x".into(),
        bytes: Vec::new().into(),
    };
    let a = doc.add("a".into(), src(), Vec::new());
    let b = doc.add("b".into(), src(), Vec::new());
    // A loop, a missing parent and a group of one.
    doc.groups = vec![
        flatcraft_core::nodes::GroupNode {
            id: ObjectId(10),
            name: "x".into(),
            parent: Some(ObjectId(11)),
            rotation_deg: 0.0,
        },
        flatcraft_core::nodes::GroupNode {
            id: ObjectId(11),
            name: "y".into(),
            parent: Some(ObjectId(10)),
            rotation_deg: 0.0,
        },
    ];
    doc.get_mut(a).unwrap().parent = Some(ObjectId(10));
    doc.get_mut(b).unwrap().parent = Some(ObjectId(99));
    assert!(!doc.groups_valid());
    doc.normalize_groups();
    assert!(doc.groups_valid());
    assert!(doc.groups.is_empty(), "{:?}", doc.groups);
    assert!(doc.objects.iter().all(|o| o.parent.is_none()));
    assert!(doc.next_id > 11);
}

#[test]
fn a_hidden_member_is_not_part_of_the_group_box() {
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 10.0, 10.0, 10.0, 10.0);
    let b = rect(&mut s, "b", 50.0, 10.0, 10.0, 10.0);
    let g = group(&mut s, &[a, b]);
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [a], "visible": false }),
    );
    // The X field puts the box of what is shown: b's left edge.
    run(&mut s, "object.move", json!({ "ids": [g], "x_mm": 100.0 }));
    assert!(close(bounds(&mut s, b).x0, 100.0));
    assert!(close(bounds(&mut s, a).x0, 60.0), "a moves along");
    // Each: the group turns about the centre of b alone.
    let before = bounds(&mut s, b);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [g], "degrees": 90.0, "about": "each" }),
    );
    let after = bounds(&mut s, b);
    assert!(close(after.center().x, before.center().x));
    assert!(close(after.center().y, before.center().y));
}

/// The common box of the objects `ids`.
fn box_of(s: &mut Session, ids: &[u64]) -> Rect {
    ids.iter()
        .map(|id| bounds(s, *id))
        .reduce(|x, y| x.union(y))
        .unwrap()
}

fn scale_of(s: &Session, id: u64) -> f64 {
    s.doc()
        .document
        .get(ObjectId(id))
        .unwrap()
        .transform
        .scale_x
}

#[test]
fn linked_copies_in_groups_scale_once() {
    let d = dirs();
    let mut s = session(&d);
    // G = {A, B}, L a linked copy of A outside G.
    let a = rect(&mut s, "a", 10.0, 10.0, 10.0, 10.0);
    let b = rect(&mut s, "b", 30.0, 10.0, 10.0, 10.0);
    let l = run(
        &mut s,
        "object.copies",
        json!({ "id": a, "count": 1, "linked": true }),
    )["ids"][0]
        .as_u64()
        .unwrap();
    let g = group(&mut s, &[a, b]);
    let l_centre = s
        .doc()
        .document
        .get(ObjectId(l))
        .unwrap()
        .transform
        .center_mm;
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [g, l], "factor": 2.0 }),
    );
    assert_eq!(scale_of(&s, a), 2.0);
    assert_eq!(scale_of(&s, l), 2.0);
    assert_eq!(scale_of(&s, b), 2.0);
    assert!(s.doc().document.link_groups_agree());
    let lc = s
        .doc()
        .document
        .get(ObjectId(l))
        .unwrap()
        .transform
        .center_mm;
    assert!(
        close(lc.x, l_centre.x) && close(lc.y, l_centre.y),
        "L keeps its centre"
    );
    run(&mut s, "edit.undo", json!({}));

    // Two groups that share a linked design: G1 = {A, B}, G2 = {L, C}.
    let c = rect(&mut s, "c", 120.0, 10.0, 10.0, 10.0);
    run(&mut s, "object.ungroup", json!({ "ids": [g] }));
    let g1 = group(&mut s, &[a, b]);
    let g2 = group(&mut s, &[l, c]);
    let (b1, b2) = (box_of(&mut s, &[a, b]), box_of(&mut s, &[l, c]));
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [g1, g2], "factor": 2.0 }),
    );
    for id in [a, b, l, c] {
        assert_eq!(scale_of(&s, id), 2.0, "object {id}");
    }
    assert!(s.doc().document.link_groups_agree());
    // Each group scales about its own box centre, measured before anything
    // scaled: neither slides.
    for (ids, before) in [([a, b], b1), ([l, c], b2)] {
        let after = box_of(&mut s, &ids);
        let want = Rect::from_center_size(before.center(), before.size() * 2.0);
        assert!(close_rect(after, want), "{before:?} -> {after:?}");
    }
    // As one piece (the canvas handles) too.
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "object.scale",
        json!({ "ids": [g1, g2], "factor": 2.0, "group": true }),
    );
    for id in [a, b, l, c] {
        assert_eq!(scale_of(&s, id), 2.0, "object {id}");
    }
}

#[test]
fn rotate_and_mirror_apply_once_to_linked_copies_in_groups() {
    // Rotation and mirror are each copy's own (not shared), so a turn of
    // two groups with a shared design turns every object once.
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 10.0, 10.0, 10.0, 10.0);
    let b = rect(&mut s, "b", 30.0, 10.0, 10.0, 10.0);
    let l = run(
        &mut s,
        "object.copies",
        json!({ "id": a, "count": 1, "linked": true }),
    )["ids"][0]
        .as_u64()
        .unwrap();
    let c = rect(&mut s, "c", 120.0, 10.0, 10.0, 10.0);
    let g1 = group(&mut s, &[a, b]);
    let g2 = group(&mut s, &[l, c]);
    let (b1, b2) = (box_of(&mut s, &[a, b]), box_of(&mut s, &[l, c]));
    // A quarter turn of each group about its own box centre keeps it.
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [g1, g2], "degrees": 90.0, "about": "each" }),
    );
    for (ids, before) in [([a, b], b1), ([l, c], b2)] {
        let after = box_of(&mut s, &ids).center();
        let c0 = before.center();
        assert!(
            close(after.x, c0.x) && close(after.y, c0.y),
            "{c0:?} -> {after:?}"
        );
    }
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [g1, g2], "degrees": -60.0, "about": "each" }),
    );
    let (m1, m2) = (box_of(&mut s, &[a, b]), box_of(&mut s, &[l, c]));
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [g1, g2], "axis": "horizontal", "about": "each" }),
    );
    // Each group mirrors across its own box, which stays.
    assert!(close_rect(box_of(&mut s, &[a, b]), m1));
    assert!(close_rect(box_of(&mut s, &[l, c]), m2));
    for id in [a, b, l, c] {
        let t = s.doc().document.get(ObjectId(id)).unwrap().transform;
        assert!(
            close(t.rotation_deg, 330.0),
            "object {id}: {}",
            t.rotation_deg
        );
        assert!(t.mirror, "object {id}");
    }
}

/// A minimal call of each registry command that takes object ids, for
/// `id`. Every command with an `ids`, `id` or `follow` parameter must be
/// listed (the test fails for one that is not).
fn id_call(cmd: &str, id: u64) -> Option<Value> {
    Some(match cmd {
        "object.select"
        | "object.delete"
        | "object.duplicate"
        | "object.link"
        | "object.unlink_copies"
        | "object.group"
        | "object.ungroup"
        | "object.move_to_group"
        | "document.check"
        | "object.reload"
        | "object.unlink"
        | "arrange.auto" => json!({ "ids": [id] }),
        "object.set_visible" => json!({ "ids": [id], "visible": true }),
        "object.set_cut" => json!({ "ids": [id], "cut": true }),
        "object.rename" => json!({ "id": id, "name": "x" }),
        "object.copies" => json!({ "id": id, "count": 1 }),
        "object.circular_copies" => json!({ "ids": [id], "count": 2 }),
        "object.move" => json!({ "ids": [id], "dx_mm": 1.0 }),
        "object.scale" => json!({ "ids": [id], "factor": 1.5 }),
        "object.rotate" => json!({ "ids": [id], "degrees": 10.0 }),
        "object.mirror" => {
            json!({ "ids": [id], "axis": "horizontal", "mirrored": true, "follow": [id] })
        }
        "object.set_cut_options" => json!({ "ids": [id], "mode": "all_lines" }),
        "object.set_keep" => json!({ "ids": [id], "keep": "stencil" }),
        // A point on the sheet, away from every object of the test.
        "object.keep_at" => json!({ "ids": [id], "points": [{ "x_mm": 150.0, "y_mm": 150.0 }] }),
        "arrange.align" => json!({ "ids": [id], "mode": "left" }),
        "arrange.distribute" => json!({ "ids": [id], "axis": "horizontal" }),
        "document.export" => return None,
        // One object by nature (see `one_object_commands`).
        "object.retrace" | "object.relink" => return None,
        _ => return None,
    })
}

#[test]
fn every_command_that_takes_object_ids_takes_a_group_id() {
    let one_object = ["object.retrace", "object.relink"];
    let d = dirs();
    for c in flatcraft_engine::registry::all() {
        let schema = serde_json::to_value(c.params_schema()).unwrap();
        let props = &schema["properties"];
        // Object ids are integers (profile ids are text).
        let int = |v: &Value| {
            v.get("type") == Some(&json!("integer"))
                || v.pointer("/items/type") == Some(&json!("integer"))
        };
        let takes = ["ids", "id", "follow"]
            .iter()
            .any(|k| props.get(*k).is_some_and(int));
        if !takes || one_object.contains(&c.id()) {
            continue;
        }
        let mut s = session(&d);
        let a = rect(&mut s, "a", 10.0, 10.0, 10.0, 10.0);
        let b = rect(&mut s, "b", 30.0, 10.0, 10.0, 10.0);
        let c3 = rect(&mut s, "c", 50.0, 10.0, 10.0, 10.0);
        let g = group(&mut s, &[a, b]);
        let params =
            id_call(c.id(), g).unwrap_or_else(|| panic!("{} takes ids: add it to id_call", c.id()));
        let params = if c.id() == "arrange.distribute" {
            json!({ "ids": [g, c3, rect(&mut s, "e", 90.0, 10.0, 10.0, 10.0)], "axis": "horizontal" })
        } else if c.id() == "object.group" {
            json!({ "ids": [g, c3] })
        } else {
            params
        };
        match s.execute(c.id(), params, Origin::User) {
            Ok(_) => {}
            Err(e) => {
                let text = e.to_string();
                assert!(
                    !text.contains(&format!("{g}")) && !matches!(e, CommandError::NotFound(_)),
                    "{} refuses a group id: {text}",
                    c.id()
                );
            }
        }
    }
}

#[test]
fn one_object_commands_say_a_group_is_a_group() {
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 10.0, 10.0, 10.0, 10.0);
    let b = rect(&mut s, "b", 30.0, 10.0, 10.0, 10.0);
    let g = group(&mut s, &[a, b]);
    let e = fails(&mut s, "object.retrace", json!({ "id": g, "settings": {} }));
    assert!(e.to_string().contains("is a group"), "{e}");
}

#[test]
fn mirrored_flips_a_whole_group_when_any_member_differs() {
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 10.0, 10.0, 20.0, 10.0);
    let b = rect(&mut s, "b", 40.0, 10.0, 10.0, 10.0);
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [a], "axis": "horizontal", "about": "each" }),
    );
    let g = group(&mut s, &[a, b]);
    let before = bounds(&mut s, a).union(bounds(&mut s, b));
    let ra = bounds(&mut s, a);
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [g], "axis": "horizontal", "mirrored": true }),
    );
    // Both flipped across the group's box: a was left, is now right.
    let doc = &s.doc().document;
    assert!(!doc.get(ObjectId(a)).unwrap().transform.mirror);
    assert!(doc.get(ObjectId(b)).unwrap().transform.mirror);
    let after = bounds(&mut s, a).union(bounds(&mut s, b));
    assert!(close_rect(after, before));
    assert!(close(bounds(&mut s, a).x1, before.x1 - (ra.x0 - before.x0)));
}

#[test]
fn a_hand_edited_group_tree_is_mended_on_open() {
    use flatcraft_core::project::Archive;
    let d = dirs();
    let mut s = session(&d);
    let a = rect(&mut s, "a", 10.0, 10.0, 10.0, 10.0);
    let b = rect(&mut s, "b", 30.0, 10.0, 10.0, 10.0);
    let c = rect(&mut s, "c", 50.0, 10.0, 10.0, 10.0);
    group(&mut s, &[a, b]);
    let path = d.out.path().join("p.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    let mut archive = Archive::read(&path).unwrap();
    let v = &mut archive.project;
    // A loop, a dangling parent, a group whose id is an object's, and a
    // group without members.
    v["groups"] = json!([
        { "id": 90, "name": "loop a", "parent": 91 },
        { "id": 91, "name": "loop b", "parent": 90 },
        { "id": a, "name": "clash" },
        { "id": 92, "name": "empty" },
    ]);
    v["objects"][0]["group"] = json!(90);
    v["objects"][0]["parent"] = json!(90);
    v["objects"][1]["parent"] = json!(77);
    v["objects"][2]["parent"] = json!(90);
    let p2 = d.out.path().join("edited.flatcraft");
    archive.write(&p2).unwrap();
    let mut s2 = session(&d);
    run(&mut s2, "project.open", json!({ "path": p2 }));
    let doc = &s2.doc().document;
    assert!(doc.groups_valid());
    // 90 keeps a and c (its loop broken); the rest is gone.
    assert_eq!(doc.groups.len(), 1, "{:?}", doc.groups);
    assert_eq!(doc.groups[0].id, ObjectId(90));
    assert_eq!(doc.get(ObjectId(a)).unwrap().parent, Some(ObjectId(90)));
    assert_eq!(doc.get(ObjectId(c)).unwrap().parent, Some(ObjectId(90)));
    assert_eq!(doc.get(ObjectId(b)).unwrap().parent, None);
    assert!(doc.next_id > 92);
}
