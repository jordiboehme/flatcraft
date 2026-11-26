//! Multi-selection editing: group rotate and mirror about the common centre
//! of the selection, `about: each`, and nudges that coalesce into one undo
//! step (`object.move` with `coalesce_key`).

use std::path::Path;

use flatcraft_core::document::ObjectId;
use flatcraft_core::transform::Transform;
use flatcraft_device::ProfileStore;
use flatcraft_engine::commands::placed_bounds;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::registry;
use flatcraft_engine::session::{CommandError, Origin, Session};
use kurbo::{Point, Rect, Vec2};
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

/// A 40 x 20 mm rectangle (not square, so a turn shows) at x, y.
fn rect_at(s: &mut Session, x: f64, y: f64) -> u64 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="300mm" height="300mm" viewBox="0 0 300 300"><rect x="{x}" y="{y}" width="40" height="20"/></svg>"#
    );
    run(
        s,
        "object.import_svg",
        json!({ "svg": svg, "name": "rect" }),
    )["id"]
        .as_u64()
        .unwrap()
}

fn transform(s: &Session, id: u64) -> Transform {
    s.doc().document.get(ObjectId(id)).unwrap().transform
}

fn bounds(s: &Session, id: u64) -> Rect {
    placed_bounds(s.doc().document.get(ObjectId(id)).unwrap(), 0.125).unwrap()
}

fn common(s: &Session, ids: &[u64]) -> Rect {
    ids.iter()
        .map(|i| bounds(s, *i))
        .reduce(|a, b| a.union(b))
        .unwrap()
}

/// The mean of the objects' centres: the pivot of a group rotate or
/// mirror (Ruling M2-P11).
fn mean(s: &Session, ids: &[u64]) -> Point {
    let n = ids.len() as f64;
    let sum = ids.iter().fold(Vec2::ZERO, |acc, i| {
        acc + transform(s, *i).center_mm.to_vec2()
    });
    (sum / n).to_point()
}

fn undo_len(s: &Session) -> usize {
    s.doc().undo.undo_len()
}

/// Import reads the SVG in f32, so its numbers are off by about 1e-6 mm.
fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-5
}

fn near_rect(a: Rect, b: Rect) -> bool {
    near(Point::new(a.x0, a.y0), Point::new(b.x0, b.y0))
        && near(Point::new(a.x1, a.y1), Point::new(b.x1, b.y1))
}

/// Two rectangles side by side with a gap: their centres at (40, 30) and
/// (120, 30), the common box 20..140 x 20..40, its centre (80, 30).
fn two(s: &mut Session) -> (u64, u64) {
    (rect_at(s, 20.0, 20.0), rect_at(s, 100.0, 20.0))
}

// ---------------------------------------------------------------- rotate

#[test]
fn group_rotate_turns_the_centres_around_the_common_centre() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let (a, b) = two(&mut s);
    // The common centre: the mean of the two centres.
    let c = mean(&s, &[a, b]);
    assert!(near(c, Point::new(80.0, 30.0)), "{c:?}");
    let before = undo_len(&s);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a, b], "degrees": 90.0 }),
    );
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    // Clockwise on screen with y down: left of the centre goes up.
    let (ta, tb) = (transform(&s, a), transform(&s, b));
    assert!(near(ta.center_mm, Point::new(80.0, -10.0)), "{ta:?}");
    assert!(near(tb.center_mm, Point::new(80.0, 70.0)), "{tb:?}");
    assert!((ta.rotation_deg - 90.0).abs() < 1e-9, "{ta:?}");
    assert!((tb.rotation_deg - 90.0).abs() < 1e-9, "{tb:?}");
    // The common box turned as one: 20 wide, 120 high, same centre.
    let after = common(&s, &[a, b]);
    assert!(
        near_rect(after, Rect::new(70.0, -30.0, 90.0, 90.0)),
        "{after:?}"
    );
    run(&mut s, "edit.undo", json!({}));
    assert!(near(transform(&s, a).center_mm, Point::new(40.0, 30.0)));
}

#[test]
fn group_is_the_default_and_the_selection_works_too() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let (a, b) = two(&mut s);
    run(&mut s, "object.select", json!({ "ids": [a, b] }));
    run(
        &mut s,
        "object.rotate",
        json!({ "degrees": 180.0, "about": "group" }),
    );
    // Half a turn about (80, 30) swaps the two centres.
    assert!(near(transform(&s, a).center_mm, Point::new(120.0, 30.0)));
    assert!(near(transform(&s, b).center_mm, Point::new(40.0, 30.0)));
}

#[test]
fn each_object_turns_around_its_own_centre() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let (a, b) = two(&mut s);
    let (ca, cb) = (transform(&s, a).center_mm, transform(&s, b).center_mm);
    let before = undo_len(&s);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a, b], "degrees": 90.0, "about": "each" }),
    );
    assert_eq!(undo_len(&s), before + 1);
    let (ta, tb) = (transform(&s, a), transform(&s, b));
    assert_eq!(ta.center_mm, ca);
    assert_eq!(tb.center_mm, cb);
    assert_eq!(ta.rotation_deg, 90.0);
    assert_eq!(tb.rotation_deg, 90.0);
}

#[test]
fn one_object_turns_the_same_either_way() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    // A thick stroke on the right edge only: the cut outline reaches out
    // on that side, so the placed bounds' centre is not the object's
    // centre. One object still turns about its own centre.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="300mm" height="300mm" viewBox="0 0 300 300"><rect x="20" y="20" width="40" height="20"/><line x1="60" y1="20" x2="60" y2="40" stroke="black" stroke-width="10"/></svg>"#;
    let a = run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg, "name": "lopsided" }),
    )["id"]
        .as_u64()
        .unwrap();
    let start = transform(&s, a);
    let off = bounds(&s, a).center() - start.center_mm;
    assert!(off.hypot() > 0.5, "the fixture is lopsided: {off:?}");
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a], "degrees": 33.0, "about": "group" }),
    );
    let group = transform(&s, a);
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a], "degrees": 33.0, "about": "each" }),
    );
    assert_eq!(transform(&s, a), group);
    assert_eq!(group.center_mm, start.center_mm);
    // A mirror with about each flips about the object's own centre; with
    // about group (the default) its cut box stays in place.
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [a], "axis": "horizontal", "about": "each" }),
    );
    assert_eq!(transform(&s, a).center_mm, start.center_mm);
    run(&mut s, "edit.undo", json!({}));
    let before = bounds(&s, a);
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [a], "axis": "horizontal" }),
    );
    let after = bounds(&s, a);
    assert!(
        (after.x0 - before.x0).abs() < 1e-9 && (after.x1 - before.x1).abs() < 1e-9,
        "{after:?} vs {before:?}"
    );
    assert!((after.y0 - before.y0).abs() < 1e-9 && (after.y1 - before.y1).abs() < 1e-9);
}

#[test]
fn group_rotate_keeps_the_objects_rigid() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let (a, b) = two(&mut s);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a], "degrees": 30.0, "about": "each" }),
    );
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [b], "axis": "horizontal" }),
    );
    let d = (transform(&s, b).center_mm - transform(&s, a).center_mm).hypot();
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a, b], "degrees": 45.0 }),
    );
    let (ta, tb) = (transform(&s, a), transform(&s, b));
    assert!(((tb.center_mm - ta.center_mm).hypot() - d).abs() < 1e-9);
    assert!((ta.rotation_deg - 75.0).abs() < 1e-9, "{ta:?}");
    assert!((tb.rotation_deg - 45.0).abs() < 1e-9, "{tb:?}");
    assert!(tb.mirror && !ta.mirror);
}

#[test]
fn a_group_rotate_that_could_not_reopen_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let (a, b) = two(&mut s);
    // b near the far corner of the 1 km limit: an eighth of a turn about
    // the middle of a and b puts b's centre past it.
    run(
        &mut s,
        "object.move",
        json!({ "ids": [b], "dx_mm": 998_000.0, "dy_mm": 998_000.0 }),
    );
    let before = undo_len(&s);
    let ta = transform(&s, a);
    let e = s
        .execute(
            "object.rotate",
            json!({ "ids": [a, b], "degrees": 45.0 }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "{e:?}");
    assert_eq!(undo_len(&s), before);
    assert_eq!(transform(&s, a), ta);
}

#[test]
fn linked_copies_keep_their_own_rotation_after_a_group_rotate() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = rect_at(&mut s, 20.0, 20.0);
    let r = run(
        &mut s,
        "object.copies",
        json!({ "id": a, "count": 2, "linked": true }),
    );
    let copies: Vec<u64> = r["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    let (b, c) = (copies[0], copies[1]);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [c], "degrees": 30.0 }),
    );
    let tc = transform(&s, c);
    let before = undo_len(&s);
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [a, b], "degrees": 90.0 }),
    );
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [a, b], "axis": "vertical" }),
    );
    assert_eq!(undo_len(&s), before + 2);
    assert_eq!(transform(&s, c), tc, "the copy outside the selection");
    let (ta, tb) = (transform(&s, a), transform(&s, b));
    // 90 then a vertical mirror: 180 - 90.
    assert!((ta.rotation_deg - 90.0).abs() < 1e-9, "{ta:?}");
    assert!(ta.mirror && tb.mirror && !tc.mirror);
    let g = |id: u64| s.doc().document.get(ObjectId(id)).unwrap().link_group;
    assert!(
        g(a).is_some() && g(a) == g(b) && g(b) == g(c),
        "still linked"
    );
}

// ---------------------------------------------------------------- mirror

#[test]
fn group_mirror_reflects_the_centres_across_the_common_centre() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = rect_at(&mut s, 20.0, 20.0);
    let b = rect_at(&mut s, 100.0, 60.0);
    let c = common(&s, &[a, b]);
    let m = mean(&s, &[a, b]);
    let (ca, cb) = (transform(&s, a).center_mm, transform(&s, b).center_mm);
    let before = undo_len(&s);
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [a, b], "axis": "horizontal" }),
    );
    assert_eq!(undo_len(&s), before + 1);
    let (ta, tb) = (transform(&s, a), transform(&s, b));
    let cx = m.x;
    assert!(
        near(ta.center_mm, Point::new(2.0 * cx - ca.x, ca.y)),
        "{ta:?}"
    );
    assert!(
        near(tb.center_mm, Point::new(2.0 * cx - cb.x, cb.y)),
        "{tb:?}"
    );
    assert!(ta.mirror && tb.mirror);
    assert!(near_rect(common(&s, &[a, b]), c), "the common box stays");
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [a, b], "axis": "vertical" }),
    );
    let (ta, tb) = (transform(&s, a), transform(&s, b));
    let cy = m.y;
    assert!(
        near(ta.center_mm, Point::new(2.0 * cx - ca.x, 2.0 * cy - ca.y)),
        "{ta:?}"
    );
    assert!(
        near(tb.center_mm, Point::new(2.0 * cx - cb.x, 2.0 * cy - cb.y)),
        "{tb:?}"
    );
    // Mirrored twice: a half turn, not mirrored.
    assert!(
        !ta.mirror && (ta.rotation_deg - 180.0).abs() < 1e-9,
        "{ta:?}"
    );
}

#[test]
fn each_mirror_keeps_the_centres() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let (a, b) = two(&mut s);
    let (ca, cb) = (transform(&s, a).center_mm, transform(&s, b).center_mm);
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [a, b], "axis": "horizontal", "about": "each" }),
    );
    let (ta, tb) = (transform(&s, a), transform(&s, b));
    assert_eq!((ta.center_mm, tb.center_mm), (ca, cb));
    assert!(ta.mirror && tb.mirror);
    // A mirror of 0 degrees stores 0, not -0.
    assert!(ta.rotation_deg.is_sign_positive(), "{ta:?}");
}

// ---------------------------------------------------------------- coalesce

fn nudge(s: &mut Session, ids: &[u64], dx: f64, key: &str) -> Result<Value, CommandError> {
    s.execute(
        "object.move",
        json!({ "ids": ids, "dx_mm": dx, "coalesce_key": key }),
        Origin::User,
    )
}

#[test]
fn moves_with_one_coalesce_key_are_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let (a, b) = two(&mut s);
    let start = transform(&s, a).center_mm;
    let before = undo_len(&s);
    for _ in 0..5 {
        nudge(&mut s, &[a, b], 1.0, "k1").unwrap();
    }
    assert_eq!(undo_len(&s), before + 1, "one step for the burst");
    assert!(near(
        transform(&s, a).center_mm,
        start + Vec2::new(5.0, 0.0)
    ));
    assert!(s.doc().dirty);
    run(&mut s, "edit.undo", json!({}));
    assert!(
        near(transform(&s, a).center_mm, start),
        "undo goes back before the burst"
    );
    run(&mut s, "edit.redo", json!({}));
    assert!(near(
        transform(&s, a).center_mm,
        start + Vec2::new(5.0, 0.0)
    ));
}

#[test]
fn another_key_other_ids_or_a_change_in_between_start_a_new_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let (a, b) = two(&mut s);
    let n0 = undo_len(&s);
    nudge(&mut s, &[a], 1.0, "k1").unwrap();
    nudge(&mut s, &[a], 1.0, "k2").unwrap();
    assert_eq!(undo_len(&s), n0 + 2, "another key");
    nudge(&mut s, &[a, b], 1.0, "k2").unwrap();
    assert_eq!(undo_len(&s), n0 + 3, "other ids");
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [b], "degrees": 10.0 }),
    );
    nudge(&mut s, &[a, b], 1.0, "k2").unwrap();
    assert_eq!(undo_len(&s), n0 + 5, "a rotate in between");
    // An undo in between: the next nudge is a step of its own.
    run(&mut s, "edit.undo", json!({}));
    let n = undo_len(&s);
    nudge(&mut s, &[a, b], 1.0, "k2").unwrap();
    assert_eq!(undo_len(&s), n + 1, "an undo in between");
    // A plain move never merges.
    let n = undo_len(&s);
    run(
        &mut s,
        "object.move",
        json!({ "ids": [a, b], "dx_mm": 1.0 }),
    );
    assert_eq!(undo_len(&s), n + 1);
    nudge(&mut s, &[a, b], 1.0, "k2").unwrap();
    assert_eq!(undo_len(&s), n + 2);
}

#[test]
fn a_burst_that_starts_with_no_change_or_a_refusal_still_records_a_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = rect_at(&mut s, 20.0, 20.0);
    let start = transform(&s, a).center_mm;
    let n0 = undo_len(&s);
    nudge(&mut s, &[a], 0.0, "k").unwrap();
    assert_eq!(undo_len(&s), n0, "nothing changed");
    nudge(&mut s, &[a], 1.0, "k").unwrap();
    assert_eq!(undo_len(&s), n0 + 1, "the first real move records a step");
    // A refused move (past 1 km) starts nothing either.
    assert!(nudge(&mut s, &[a], 2e6, "k2").is_err());
    nudge(&mut s, &[a], 1.0, "k2").unwrap();
    assert_eq!(undo_len(&s), n0 + 2);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(near(transform(&s, a).center_mm, start));
}

#[test]
fn a_burst_that_ends_where_it_started_leaves_no_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = rect_at(&mut s, 20.0, 20.0);
    // As if just saved.
    s.doc_mut().dirty = false;
    let n0 = undo_len(&s);
    nudge(&mut s, &[a], 1.0, "k").unwrap();
    assert!(s.doc().dirty);
    nudge(&mut s, &[a], -1.0, "k").unwrap();
    assert_eq!(undo_len(&s), n0, "back where it started: nothing to undo");
    assert!(!s.doc().dirty, "and nothing to save");
    // The next move of that key is a step of its own again.
    nudge(&mut s, &[a], 1.0, "k").unwrap();
    assert_eq!(undo_len(&s), n0 + 1);
}

#[test]
fn agents_use_about_and_coalesce_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let (a, b) = two(&mut s);
    let n0 = undo_len(&s);
    for _ in 0..2 {
        s.execute(
            "object.move",
            json!({ "ids": [a], "dy_mm": 1.0, "coalesce_key": "agent" }),
            Origin::Agent,
        )
        .unwrap();
    }
    assert_eq!(undo_len(&s), n0 + 1);
    s.execute(
        "object.rotate",
        json!({ "ids": [a, b], "degrees": 90.0, "about": "each" }),
        Origin::Agent,
    )
    .unwrap();
    for id in ["object.rotate", "object.mirror"] {
        let d = registry::get(id).unwrap().description();
        for word in [
            "about",
            "group",
            "each",
            "default",
            "mm",
            "grid",
            "Linked copies",
        ] {
            assert!(d.contains(word), "{id} description lacks {word:?}: {d}");
        }
        assert!(!d.contains('\u{2014}'), "{id} has an em-dash");
    }
    let d = registry::get("object.move").unwrap().description();
    assert!(d.contains("coalesce_key") && d.contains("undo step"), "{d}");
}

#[test]
fn a_save_ends_the_burst() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = rect_at(&mut s, 20.0, 20.0);
    nudge(&mut s, &[a], 1.0, "k").unwrap();
    let path = dir.path().join("p.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    assert!(!s.doc().dirty);
    let saved = undo_len(&s);
    nudge(&mut s, &[a], -1.0, "k").unwrap();
    // The file holds +1, the document is back at 0: unsaved, and the move
    // after the save is a step of its own.
    assert!(s.doc().dirty, "differs from the file");
    assert_eq!(undo_len(&s), saved + 1);
}

#[test]
fn a_burst_back_at_its_start_keeps_redo() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = rect_at(&mut s, 20.0, 20.0);
    let start = transform(&s, a).center_mm;
    run(&mut s, "object.move", json!({ "ids": [a], "dy_mm": 5.0 }));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().undo.can_redo());
    nudge(&mut s, &[a], 1.0, "k").unwrap();
    nudge(&mut s, &[a], -1.0, "k").unwrap();
    assert!(
        s.doc().undo.can_redo(),
        "a burst that changed nothing keeps redo"
    );
    run(&mut s, "edit.redo", json!({}));
    assert!(near(
        transform(&s, a).center_mm,
        start + Vec2::new(0.0, 5.0)
    ));
}

/// Objects of different sizes, one turned, so the centre of the common
/// box differs from the mean of the centres and moves when they turn.
fn uneven(s: &mut Session) -> Vec<u64> {
    let a = rect_at(s, 20.0, 20.0);
    let b = rect_at(s, 150.0, 90.0);
    run(s, "object.scale", json!({ "ids": [b], "factor": 2.5 }));
    run(s, "object.rotate", json!({ "ids": [b], "degrees": 20.0 }));
    let c = rect_at(s, 60.0, 200.0);
    vec![a, b, c]
}

#[test]
fn group_rotate_there_and_back_puts_everything_back() {
    for theta in [30.0, 37.0] {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(dir.path());
        let ids = uneven(&mut s);
        let before: Vec<Transform> = ids.iter().map(|i| transform(&s, *i)).collect();
        run(
            &mut s,
            "object.rotate",
            json!({ "ids": ids, "degrees": theta }),
        );
        run(
            &mut s,
            "object.rotate",
            json!({ "ids": ids, "degrees": -theta }),
        );
        for (id, t) in ids.iter().zip(&before) {
            let now = transform(&s, *id);
            assert!(
                (now.center_mm - t.center_mm).hypot() < 1e-9,
                "{theta}: {id} walked {:?}",
                now.center_mm - t.center_mm
            );
            let dr = (now.rotation_deg - t.rotation_deg).rem_euclid(360.0);
            assert!(dr.min(360.0 - dr) < 1e-9, "{theta}: {id} {now:?}");
        }
    }
}

#[test]
fn the_group_pivot_is_the_mean_of_the_centres() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let ids = uneven(&mut s);
    let centres: Vec<Point> = ids.iter().map(|i| transform(&s, *i).center_mm).collect();
    let n = centres.len() as f64;
    let mean = Point::new(
        centres.iter().map(|c| c.x).sum::<f64>() / n,
        centres.iter().map(|c| c.y).sum::<f64>() / n,
    );
    assert!(
        (common(&s, &ids).center() - mean).hypot() > 1.0,
        "the fixture tells them apart"
    );
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": ids, "degrees": 90.0 }),
    );
    for (k, id) in ids.iter().enumerate() {
        let d = centres[k] - mean;
        let want = mean + Vec2::new(-d.y, d.x);
        assert!((transform(&s, *id).center_mm - want).hypot() < 1e-9, "{id}");
    }
    // A mirror flips across the middle of the combined box and keeps the
    // box in place, so the design stays where it is on the mat.
    let boxed = common(&s, &ids);
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": ids, "axis": "horizontal" }),
    );
    let after = common(&s, &ids);
    for (a, b) in [
        (after.x0, boxed.x0),
        (after.x1, boxed.x1),
        (after.y0, boxed.y0),
        (after.y1, boxed.y1),
    ] {
        assert!(
            (a - b).abs() < 1e-9,
            "a mirror keeps the box: {after:?} vs {boxed:?}"
        );
    }
}
