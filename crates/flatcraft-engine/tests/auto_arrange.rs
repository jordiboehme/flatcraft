//! Auto arrange (`arrange.auto`): packing the selection around the
//! objects placed by hand.

use flatcraft_core::document::ObjectId;
use flatcraft_core::geometry::bounds;
use flatcraft_device::ProfileStore;
use flatcraft_engine::placement::{self, AutoObject};
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{Origin, Session};
use kurbo::{Point, Rect, Vec2};
use proptest::prelude::*;
use serde_json::{Value, json};

/// The cutting area of the default (uncalibrated) Cameo 2 mat.
const AREA: Rect = Rect::new(5.0, 5.0, 299.8, 299.8);

/// How much closer than the spacing still counts as the spacing (float
/// noise of usvg's f32 import and of the moves).
const TOL: f64 = 1e-4;

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

/// A `w` x `h` mm rectangle, moved so its box starts at `at`.
fn rect_at(s: &mut Session, w: f64, h: f64, at: Point) -> u64 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="400mm" height="400mm" viewBox="0 0 400 400"><rect x="0" y="0" width="{w}" height="{h}"/></svg>"#
    );
    let id = run(
        s,
        "object.import_svg",
        json!({ "svg": svg, "name": "rect" }),
    )["id"]
        .as_u64()
        .unwrap();
    let b = boxed(s, id);
    run(
        s,
        "object.move",
        json!({ "ids": [id], "dx_mm": at.x - b.x0, "dy_mm": at.y - b.y0 }),
    );
    id
}

fn boxed(s: &Session, id: u64) -> Rect {
    let o = s.doc().document.get(ObjectId(id)).unwrap();
    bounds(&o.placed_paths())
        .or_else(|| flatcraft_engine::commands::placed_frame(o))
        .unwrap()
}

/// A blank 300 x 300 px picture (about 25.4 mm at 300 dpi): it traces to
/// nothing, so it has no cut geometry, only its image. Its box starts at `at`.
fn blank_picture(s: &mut Session, dir: &tempfile::TempDir, at: Point) -> u64 {
    let path = dir.path().join("blank.png");
    image::GrayImage::from_pixel(300, 300, image::Luma([255]))
        .save(&path)
        .unwrap();
    let id = run(s, "object.import", json!({ "path": path }))["id"]
        .as_u64()
        .unwrap();
    let o = s.doc().document.get(ObjectId(id)).unwrap();
    assert!(
        o.placed_paths().is_empty(),
        "the blank picture traced to something"
    );
    let b = boxed(s, id);
    assert!(b.width() > 20.0 && b.height() > 20.0, "{b:?}");
    run(
        s,
        "object.move",
        json!({ "ids": [id], "dx_mm": at.x - b.x0, "dy_mm": at.y - b.y0 }),
    );
    id
}

fn ids(v: &Value) -> Vec<u64> {
    let mut out: Vec<u64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap())
        .collect();
    out.sort_unstable();
    out
}

/// The gap between two boxes: the larger of the gaps along x and y (0 or
/// less when they overlap).
fn gap(a: Rect, b: Rect) -> f64 {
    let gx = (b.x0 - a.x1).max(a.x0 - b.x1);
    let gy = (b.y0 - a.y1).max(a.y0 - b.y1);
    gx.max(gy)
}

fn within(r: Rect, area: Rect) -> bool {
    r.x0 >= area.x0 - TOL && r.y0 >= area.y0 - TOL && r.x1 <= area.x1 + TOL && r.y1 <= area.y1 + TOL
}

/// Three objects to pack, two placed by hand in the middle of the mat.
struct Scene {
    packed: Vec<u64>,
    fixed: Vec<u64>,
}

fn scene(s: &mut Session) -> Scene {
    let fixed = vec![
        rect_at(s, 60.0, 40.0, Point::new(20.0, 20.0)),
        rect_at(s, 80.0, 50.0, Point::new(120.0, 100.0)),
    ];
    let packed = vec![
        rect_at(s, 50.0, 30.0, Point::new(200.0, 220.0)),
        rect_at(s, 40.0, 60.0, Point::new(30.0, 200.0)),
        rect_at(s, 70.0, 20.0, Point::new(150.0, 250.0)),
    ];
    Scene { packed, fixed }
}

#[test]
fn the_selection_is_packed_around_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let sc = scene(&mut s);
    let fixed_before: Vec<Rect> = sc.fixed.iter().map(|&i| boxed(&s, i)).collect();
    run(&mut s, "object.select", json!({ "ids": sc.packed }));
    let r = run(&mut s, "arrange.auto", json!({ "spacing_mm": 5.0 }));
    assert_eq!(ids(&r["placed"]), sc.packed, "{r}");
    assert!(ids(&r["did_not_fit"]).is_empty(), "{r}");
    assert_eq!(ids(&r["fixed"]), sc.fixed, "{r}");
    // The others did not move at all.
    for (&i, b) in sc.fixed.iter().zip(&fixed_before) {
        assert_eq!(boxed(&s, i), *b);
    }
    let packed: Vec<Rect> = sc.packed.iter().map(|&i| boxed(&s, i)).collect();
    for (k, a) in packed.iter().enumerate() {
        assert!(within(*a, AREA), "{a:?}");
        for f in &fixed_before {
            assert!(gap(*a, *f) >= 5.0 - TOL, "{a:?} near {f:?}");
        }
        for b in &packed[k + 1..] {
            assert!(gap(*a, *b) >= 5.0 - TOL, "{a:?} near {b:?}");
        }
    }
    // The tallest goes first, at the top-left: the fixed box at 20,20
    // leaves room only to its right (x 85).
    let tallest = boxed(&s, sc.packed[1]);
    assert!((tallest.x0 - 85.0).abs() < TOL, "{tallest:?}");
    assert!((tallest.y0 - 5.0).abs() < TOL, "{tallest:?}");
}

#[test]
fn nothing_selected_packs_every_visible_object() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let sc = scene(&mut s);
    let hidden = rect_at(&mut s, 20.0, 20.0, Point::new(250.0, 20.0));
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [hidden], "visible": false }),
    );
    let hidden_before = boxed(&s, hidden);
    run(&mut s, "object.select", json!({ "ids": [] }));
    let r = run(&mut s, "arrange.auto", json!({}));
    let mut all = sc.packed.clone();
    all.extend(&sc.fixed);
    all.sort_unstable();
    assert_eq!(ids(&r["placed"]), all, "{r}");
    assert!(ids(&r["fixed"]).is_empty(), "{r}");
    // Hidden objects are neither packed nor packed around.
    assert_eq!(boxed(&s, hidden), hidden_before);
    // Tallest first at the top-left of the cutting area, as before.
    let first = boxed(&s, sc.packed[1]);
    assert!((first.x0 - AREA.x0).abs() < TOL && (first.y0 - AREA.y0).abs() < TOL);
}

#[test]
fn ids_win_over_the_selection() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let sc = scene(&mut s);
    // The selection says the fixed ones; ids say only the first packed one.
    run(&mut s, "object.select", json!({ "ids": sc.fixed }));
    let others: Vec<(u64, Rect)> = sc
        .packed
        .iter()
        .chain(&sc.fixed)
        .skip(1)
        .map(|&i| (i, boxed(&s, i)))
        .collect();
    let r = run(&mut s, "arrange.auto", json!({ "ids": [sc.packed[0]] }));
    assert_eq!(ids(&r["placed"]), vec![sc.packed[0]], "{r}");
    assert_eq!(ids(&r["fixed"]).len(), 4, "{r}");
    for (i, b) in others {
        assert_eq!(boxed(&s, i), b, "object {i} moved");
    }
    let a = boxed(&s, sc.packed[0]);
    assert!((a.y0 - AREA.y0).abs() < TOL, "{a:?}");
    // An unknown id is refused.
    let e = s
        .execute("arrange.auto", json!({ "ids": [9999] }), Origin::User)
        .unwrap_err();
    assert!(e.to_string().contains("9999"), "{e}");
}

#[test]
fn objects_that_do_not_fit_around_the_others_stay() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    // One big object placed by hand leaves a 40 mm strip at the bottom.
    let wall = rect_at(&mut s, 294.8, 250.0, Point::new(5.0, 5.0));
    let small = rect_at(&mut s, 50.0, 30.0, Point::new(300.0, 300.0));
    let tall = rect_at(&mut s, 50.0, 60.0, Point::new(320.0, 10.0));
    let tall_before = boxed(&s, tall);
    run(&mut s, "object.select", json!({ "ids": [small, tall] }));
    let r = run(&mut s, "arrange.auto", json!({ "spacing_mm": 5.0 }));
    assert_eq!(ids(&r["placed"]), vec![small], "{r}");
    assert_eq!(ids(&r["did_not_fit"]), vec![tall], "{r}");
    assert_eq!(ids(&r["fixed"]), vec![wall], "{r}");
    assert_eq!(boxed(&s, tall), tall_before, "it keeps its place");
    let b = boxed(&s, small);
    assert!(
        (b.y0 - 260.0).abs() < TOL && (b.x0 - 5.0).abs() < TOL,
        "{b:?}"
    );
}

#[test]
fn one_undo_step_and_the_same_result_every_time() {
    let dir = tempfile::tempdir().unwrap();
    let mut results = Vec::new();
    for _ in 0..2 {
        let mut s = session(&dir);
        let sc = scene(&mut s);
        run(&mut s, "object.select", json!({ "ids": sc.packed }));
        let before: Vec<Rect> = sc.packed.iter().map(|&i| boxed(&s, i)).collect();
        let undo = s.doc().undo.undo_len();
        run(&mut s, "arrange.auto", json!({}));
        assert_eq!(s.doc().undo.undo_len(), undo + 1, "one undo step");
        let after: Vec<Rect> = sc.packed.iter().map(|&i| boxed(&s, i)).collect();
        results.push(after.clone());
        run(&mut s, "edit.undo", json!({}));
        let back: Vec<Rect> = sc.packed.iter().map(|&i| boxed(&s, i)).collect();
        assert_eq!(back, before, "undo puts every object back");
    }
    // A fresh session with the same input: the same spots.
    assert_eq!(results[0], results[1]);
}

#[test]
fn lines_with_no_height_and_no_spacing_do_not_land_on_each_other() {
    let items: Vec<(u32, Rect)> = (0..3)
        .map(|k| (k, Rect::new(0.0, 50.0, 150.0, 50.0)))
        .collect();
    let (moves, missed) = placement::pack_around(&items, &[], AREA, 0.0);
    assert!(missed.is_empty());
    let placed: Vec<Rect> = moves
        .iter()
        .map(|(k, d)| items[*k as usize].1 + *d)
        .collect();
    for (k, a) in placed.iter().enumerate() {
        assert!(within(*a, AREA), "{a:?}");
        for b in &placed[k + 1..] {
            assert!(
                a.intersect(*b).width() <= 0.0 || a.y0 != b.y0,
                "{a:?} on {b:?}"
            );
        }
    }
    // A line as an obstacle is kept clear of too.
    let wall = Rect::new(5.0, 100.0, 299.8, 100.0);
    let item = [(0u32, Rect::new(0.0, 0.0, 50.0, 150.0))];
    let (moves, _) = placement::pack_around(&item, &[wall], AREA, 0.0);
    let b = item[0].1 + moves[0].1;
    assert!(b.y0 >= 100.0, "{b:?} crosses the line");
}

/// A box from random numbers, inside a 400 mm square around the mat.
fn any_box() -> impl Strategy<Value = Rect> {
    (
        -50.0..350.0f64,
        -50.0..350.0f64,
        0.0..120.0f64,
        0.0..120.0f64,
    )
        .prop_map(|(x, y, w, h)| Rect::new(x, y, x + w, y + h))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Every packed box lies inside the cutting area and keeps the spacing
    /// from every obstacle and every other packed box; the same input
    /// always gives the same result; every item is placed or missed.
    #[test]
    fn packed_boxes_keep_the_spacing_from_obstacles(
        obstacles in prop::collection::vec(any_box(), 0..8),
        items in prop::collection::vec(any_box(), 1..12),
        spacing in prop_oneof![Just(0.0), 0.0..20.0f64],
    ) {
        let keyed: Vec<(usize, Rect)> = items.iter().copied().enumerate().collect();
        let (moves, missed) = placement::pack_around(&keyed, &obstacles, AREA, spacing);
        prop_assert_eq!(moves.len() + missed.len(), items.len());
        let placed: Vec<Rect> = moves.iter().map(|(k, d)| items[*k] + *d).collect();
        for (k, a) in placed.iter().enumerate() {
            prop_assert!(within(*a, AREA), "{:?} outside", a);
            prop_assert!((a.width() - items[moves[k].0].width()).abs() < 1e-9);
            for o in &obstacles {
                prop_assert!(gap(*a, *o) >= spacing - 1e-6, "{:?} near obstacle {:?}", a, o);
            }
            for b in &placed[k + 1..] {
                prop_assert!(gap(*a, *b) >= spacing - 1e-6, "{:?} near {:?}", a, b);
            }
        }
        prop_assert_eq!(
            placement::pack_around(&keyed, &obstacles, AREA, spacing),
            (moves, missed)
        );
    }

    /// Only the chosen visible objects move; every other visible one with a
    /// box is fixed, hidden ones are neither.
    #[test]
    fn only_the_chosen_ones_are_packed(
        boxes in prop::collection::vec(any_box(), 1..10),
        flags in prop::collection::vec((any::<bool>(), any::<bool>()), 10),
    ) {
        let objects: Vec<AutoObject<usize>> = boxes
            .iter()
            .enumerate()
            .map(|(k, b)| AutoObject { key: k, visible: flags[k].0, bounds: Some(*b) })
            .collect();
        let chosen: Vec<usize> = (0..boxes.len()).filter(|k| flags[*k].1).collect();
        let plan = placement::auto_arrange(&objects, &chosen, AREA, 5.0);
        for (k, _) in &plan.moves {
            prop_assert!(objects[*k].visible);
            prop_assert!(chosen.is_empty() || chosen.contains(k));
        }
        for k in &plan.fixed {
            prop_assert!(objects[*k].visible && !chosen.is_empty() && !chosen.contains(k));
        }
        let all = plan.moves.len() + plan.did_not_fit.len() + plan.fixed.len();
        prop_assert_eq!(all, objects.iter().filter(|o| o.visible).count());
        for k in &plan.hidden {
            prop_assert!(!objects[*k].visible && chosen.contains(k));
        }
        let hidden_chosen = chosen.iter().filter(|k| !objects[**k].visible).count();
        prop_assert_eq!(plan.hidden.len(), hidden_chosen);
    }
}

#[test]
fn an_untraced_picture_is_packed_around_by_its_image() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    // The picture sits at the top-left, where the first packed object
    // would go.
    let pic = blank_picture(&mut s, &dir, Point::new(5.0, 5.0));
    let pic_box = boxed(&s, pic);
    let a = rect_at(&mut s, 40.0, 40.0, Point::new(200.0, 200.0));
    let b = rect_at(&mut s, 30.0, 20.0, Point::new(100.0, 250.0));
    run(&mut s, "object.select", json!({ "ids": [a, b] }));
    let r = run(&mut s, "arrange.auto", json!({ "spacing_mm": 5.0 }));
    assert_eq!(ids(&r["placed"]), vec![a, b], "{r}");
    assert_eq!(ids(&r["fixed"]), vec![pic], "{r}");
    assert_eq!(boxed(&s, pic), pic_box, "the picture did not move");
    for id in [a, b] {
        let bx = boxed(&s, id);
        assert!(
            gap(bx, pic_box) >= 5.0 - TOL,
            "{bx:?} near the picture {pic_box:?}"
        );
    }
}

#[test]
fn a_selected_untraced_picture_is_packed_by_its_image() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let wall = rect_at(&mut s, 100.0, 100.0, Point::new(5.0, 5.0));
    let pic = blank_picture(&mut s, &dir, Point::new(200.0, 200.0));
    let size = boxed(&s, pic).size();
    run(&mut s, "object.select", json!({ "ids": [pic] }));
    let r = run(&mut s, "arrange.auto", json!({ "spacing_mm": 5.0 }));
    assert_eq!(ids(&r["placed"]), vec![pic], "{r}");
    assert!(ids(&r["did_not_fit"]).is_empty(), "{r}");
    let b = boxed(&s, pic);
    assert!((b.width() - size.width).abs() < TOL && (b.height() - size.height).abs() < TOL);
    // Right of the wall, at the top.
    assert!(
        (b.x0 - 110.0).abs() < TOL && (b.y0 - 5.0).abs() < TOL,
        "{b:?}"
    );
    assert!(gap(b, boxed(&s, wall)) >= 5.0 - TOL);
}

#[test]
fn hidden_selected_objects_stay_and_are_listed() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = rect_at(&mut s, 40.0, 40.0, Point::new(200.0, 200.0));
    let h = rect_at(&mut s, 30.0, 30.0, Point::new(100.0, 100.0));
    let other = rect_at(&mut s, 30.0, 30.0, Point::new(250.0, 20.0));
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [h], "visible": false }),
    );
    let h_before = boxed(&s, h);
    run(&mut s, "object.select", json!({ "ids": [a, h] }));
    let r = run(&mut s, "arrange.auto", json!({}));
    assert_eq!(ids(&r["placed"]), vec![a], "{r}");
    assert_eq!(ids(&r["hidden"]), vec![h], "{r}");
    assert_eq!(ids(&r["fixed"]), vec![other], "{r}");
    assert!(ids(&r["did_not_fit"]).is_empty(), "{r}");
    assert_eq!(boxed(&s, h), h_before);
    // Nothing selected: hidden objects are skipped, not listed.
    run(&mut s, "object.select", json!({ "ids": [] }));
    let r = run(&mut s, "arrange.auto", json!({}));
    assert!(ids(&r["hidden"]).is_empty(), "{r}");
    assert_eq!(boxed(&s, h), h_before);
}

#[test]
fn heights_equal_but_for_float_noise_go_in_id_order() {
    // Key 2 is taller by 1e-12 mm: noise, not a real difference.
    let items = [
        (2u32, Rect::new(0.0, 0.0, 30.0, 20.0 + 1e-12)),
        (1u32, Rect::new(100.0, 100.0, 50.0 + 100.0, 120.0)),
    ];
    let (moves, missed) = placement::pack_around(&items, &[], AREA, 5.0);
    assert!(missed.is_empty());
    assert_eq!(moves[0].0, 1, "id order, not the noise");
    assert_eq!(moves[0].1, Point::new(5.0, 5.0) - Point::new(100.0, 100.0));
    assert_eq!(moves[1].0, 2);
    // The second sits right of the first, on the same row.
    assert!((items[0].1 + moves[1].1).x0 - 60.0 < 1e-9);
    // In the other order the result is the same.
    let swapped = [items[1], items[0]];
    assert_eq!(placement::pack_around(&swapped, &[], AREA, 5.0).0, moves);
}

#[test]
fn rotated_objects_are_packed_and_packed_around_by_their_box() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let fixed = rect_at(&mut s, 80.0, 30.0, Point::new(20.0, 20.0));
    let packed = rect_at(&mut s, 60.0, 20.0, Point::new(150.0, 200.0));
    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [fixed, packed], "degrees": 30.0, "about": "each" }),
    );
    let fixed_box = boxed(&s, fixed);
    let size = boxed(&s, packed).size();
    let angle = s
        .doc()
        .document
        .get(ObjectId(packed))
        .unwrap()
        .transform
        .rotation_deg;
    run(&mut s, "object.select", json!({ "ids": [packed] }));
    let r = run(&mut s, "arrange.auto", json!({ "spacing_mm": 5.0 }));
    assert_eq!(ids(&r["placed"]), vec![packed], "{r}");
    let b = boxed(&s, packed);
    assert_eq!(boxed(&s, fixed), fixed_box, "the obstacle did not move");
    assert!((b.width() - size.width).abs() < TOL && (b.height() - size.height).abs() < TOL);
    assert_eq!(
        s.doc()
            .document
            .get(ObjectId(packed))
            .unwrap()
            .transform
            .rotation_deg,
        angle,
        "still turned"
    );
    assert!(within(b, AREA), "{b:?}");
    assert!(gap(b, fixed_box) >= 5.0 - TOL, "{b:?} near {fixed_box:?}");
}

#[test]
fn a_chosen_object_without_a_box_does_not_fit() {
    let objects = [
        AutoObject {
            key: 1u8,
            visible: true,
            bounds: None,
        },
        AutoObject {
            key: 2,
            visible: true,
            bounds: Some(Rect::new(0.0, 0.0, 10.0, 10.0)),
        },
        AutoObject {
            key: 3,
            visible: true,
            bounds: None,
        },
    ];
    let plan = placement::auto_arrange(&objects, &[1, 2], AREA, 5.0);
    assert_eq!(plan.did_not_fit, vec![1]);
    assert_eq!(plan.moves, vec![(2, Vec2::new(5.0, 5.0))]);
    // Without a box there is nothing to keep clear of.
    assert!(plan.fixed.is_empty());
}
