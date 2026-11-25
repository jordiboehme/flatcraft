//! Rotating on the canvas and in the Angle field: the rotate zone just
//! outside a corner, Cmd (macOS) or Ctrl on a corner handle, Shift for 15
//! degree steps, and the Angle field of Position and size.

mod common;

use common::{AppHarness, app, app_mut, harness, import_square, session, to_prepare, undo_len};
use egui::accesskit::Role;
use egui::{CursorIcon, Key, Modifiers, Pos2, Vec2};
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::canvas::{self, CanvasView};
use flatcraft::ui::help::{self, Os, PrepareKey};
use flatcraft_core::document::ObjectId;
use flatcraft_core::transform::Transform;
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

fn run(s: &SharedSession, id: &str, params: Value) {
    s.lock().unwrap().execute(id, params, Origin::User).unwrap();
}

fn transform(s: &SharedSession, id: u64) -> Transform {
    s.lock()
        .unwrap()
        .doc()
        .document
        .get(ObjectId(id))
        .unwrap()
        .transform
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

/// Prepare on the Layout tab, with 1 mm = 2 pt and `mm` at the canvas
/// centre.
fn rig(dir: &tempfile::TempDir, s: SharedSession, mm: kurbo::Point) -> (AppHarness, CanvasView) {
    let mut h = harness(dir, s, false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.run();
    let rect = app(&h).ui.prepare.canvas_rect;
    let view = CanvasView {
        offset: rect.center().to_vec2() - Vec2::new(mm.x as f32, mm.y as f32) * 2.0,
        px_per_mm: 2.0,
    };
    let p = &mut app_mut(&mut h).ui.prepare;
    p.view = view;
    p.fit_pending = false;
    h.run();
    (h, view)
}

/// One square, 50 mm at 20,20 (centre 45,45), and the rig around it.
fn square(dir: &tempfile::TempDir) -> (SharedSession, u64, AppHarness, CanvasView) {
    let s = session(dir);
    let id = import_square(&s);
    let (h, v) = rig(dir, s.clone(), kurbo::Point::new(45.0, 45.0));
    (s, id, h, v)
}

/// Two squares, 10 mm apart (centres 45,45 and 105,45), both selected.
fn two_squares(dir: &tempfile::TempDir) -> (SharedSession, u64, u64, AppHarness, CanvasView) {
    let s = session(dir);
    let a = import_square(&s);
    let b = import_square(&s);
    run(&s, "object.move", json!({ "ids": [b], "dx_mm": 60.0 }));
    run(&s, "object.select", json!({ "ids": [a, b] }));
    let (h, v) = rig(dir, s.clone(), kurbo::Point::new(75.0, 45.0));
    (s, a, b, h, v)
}

fn mm(v: &CanvasView, x: f64, y: f64) -> Pos2 {
    v.mm_to_screen(kurbo::Point::new(x, y))
}

/// A point in the rotate zone off the bottom-right corner `corner` (mm):
/// 12 pt out along the diagonal.
fn zone(v: &CanvasView, corner: kurbo::Point) -> Pos2 {
    v.mm_to_screen(corner) + Vec2::splat(12.0 / 2f32.sqrt())
}

/// `p` turned by `deg` clockwise about `pivot`, on screen.
fn turned(p: Pos2, pivot: Pos2, deg: f32) -> Pos2 {
    let (sin, cos) = deg.to_radians().sin_cos();
    let d = p - pivot;
    pivot + Vec2::new(d.x * cos - d.y * sin, d.x * sin + d.y * cos)
}

/// Presses at `from` and turns the pointer about `pivot` by `deg` in
/// steps, one frame each; the button stays down.
fn turn_to(h: &mut AppHarness, from: Pos2, pivot: Pos2, deg: f32) {
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    for t in [0.25, 0.5, 0.75, 1.0] {
        h.hover_at(turned(from, pivot, deg * t));
        h.run();
    }
}

/// A whole turn with `modifiers` held from start to end.
fn turn(h: &mut AppHarness, from: Pos2, pivot: Pos2, deg: f32, modifiers: Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.run();
    turn_to(h, from, pivot, deg);
    h.drop_at(turned(from, pivot, deg));
    h.run();
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

fn snap(h: &mut AppHarness) -> flatcraft::ui::prepare::Snapshot {
    flatcraft::ui::prepare::snapshot(app_mut(h), &egui::Context::default())
}

fn fields(h: &AppHarness) -> flatcraft::ui::prepare::Fields {
    app(h).ui.prepare.fields.clone()
}

fn cursor(h: &AppHarness) -> CursorIcon {
    h.output().platform_output.cursor_icon
}

/// The modifiers that turn a corner handle drag into a rotate here.
fn rotate_keys() -> Modifiers {
    help::drag_modifiers(Os::CURRENT, PrepareKey::RotateOnCorner)
}

const ANGLE: &str = "Angle (degrees)";

/// Clicks into the field, selects its text and types `text`, without
/// leaving it.
fn type_text(h: &mut AppHarness, label: &str, text: &str) {
    h.get_by_label(label).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.get_by_label(label).type_text(text);
    h.run();
}

// ---------------------------------------------------------------- zone

#[test]
fn the_rotate_zone_turns_one_object_about_its_centre() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    let before = undo_len(&s);
    let pivot = mm(&v, 45.0, 45.0);
    let from = zone(&v, kurbo::Point::new(70.0, 70.0));
    turn_to(&mut h, from, pivot, 90.0);
    // Live: the field shows the angle, the cursor is the painted rotate
    // cursor, nothing is sent yet.
    assert_eq!(fields(&h).angle, "90.00");
    assert_eq!(cursor(&h), CursorIcon::None);
    assert_eq!(undo_len(&s), before);
    h.drop_at(turned(from, pivot, 90.0));
    h.run();
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    let t = transform(&s, id);
    assert!(near(t.rotation_deg, 90.0), "{t:?}");
    assert!(
        near(t.center_mm.x, 45.0) && near(t.center_mm.y, 45.0),
        "{t:?}"
    );
    h.run();
    assert_eq!(fields(&h).angle, "90.00");
}

#[test]
fn turning_the_other_way_counts_down_from_360() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    let pivot = mm(&v, 45.0, 45.0);
    turn(
        &mut h,
        zone(&v, kurbo::Point::new(70.0, 70.0)),
        pivot,
        -30.0,
        Modifiers::NONE,
    );
    assert!(near(transform(&s, id).rotation_deg, 330.0));
    assert_eq!(fields(&h).angle, "330.00");
}

#[test]
fn hovering_the_rotate_zone_shows_the_rotate_cursor_and_its_card() {
    let dir = tempfile::tempdir().unwrap();
    let (_s, _id, mut h, v) = square(&dir);
    h.hover_at(zone(&v, kurbo::Point::new(70.0, 70.0)));
    h.run();
    h.run();
    assert_eq!(cursor(&h), CursorIcon::None, "the painted rotate cursor");
    assert!(app(&h).ui.prepare.rotate_cursor.is_some());
    // On the handle itself: the resize cursor.
    h.hover_at(mm(&v, 70.0, 70.0));
    h.run();
    h.run();
    assert_eq!(cursor(&h), CursorIcon::ResizeNwSe);
    assert!(app(&h).ui.prepare.rotate_cursor.is_none());
    // Inside the box near the corner: no rotate zone (that is a move).
    h.hover_at(mm(&v, 70.0, 70.0) - Vec2::splat(9.0));
    h.run();
    assert_ne!(cursor(&h), CursorIcon::None);
}

#[test]
fn the_handle_wins_inside_its_own_hit_area() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    // 6 pt out along the diagonal: still the handle's pick area, outside
    // the box. A drag there resizes.
    let corner = mm(&v, 70.0, 70.0);
    let from = corner + Vec2::splat(6.0 / 2f32.sqrt());
    assert!(from.distance(corner) < canvas::HANDLE_PICK);
    let to = from + Vec2::splat(20.0);
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    h.hover_at(to);
    h.run();
    h.drop_at(to);
    h.run();
    let t = transform(&s, id);
    assert!(near(t.rotation_deg, 0.0), "{t:?}");
    assert!(t.scale_x > 1.1, "{t:?}");
}

#[test]
fn a_click_in_the_rotate_zone_keeps_the_selection() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    run(&s, "object.select", json!({ "ids": [id] }));
    h.run();
    let at = zone(&v, kurbo::Point::new(70.0, 70.0));
    h.hover_at(at);
    h.run();
    // Press and release on the spot: a click.
    h.drag_at(at);
    h.run();
    h.drop_at(at);
    h.run();
    h.run();
    assert_eq!(s.lock().unwrap().doc().selection.len(), 1);
}

// ---------------------------------------------------------------- keys

#[test]
fn cmd_or_ctrl_on_a_corner_rotates_instead_of_resizing() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    let before = undo_len(&s);
    // Holding the key over a corner shows the rotate cursor.
    h.event(egui::Event::ModifiersChanged(rotate_keys()));
    h.hover_at(mm(&v, 70.0, 70.0));
    h.run();
    h.run();
    assert_eq!(cursor(&h), CursorIcon::None);
    turn(
        &mut h,
        mm(&v, 70.0, 70.0),
        mm(&v, 45.0, 45.0),
        45.0,
        rotate_keys(),
    );
    assert_eq!(undo_len(&s), before + 1);
    let t = transform(&s, id);
    assert!(near(t.rotation_deg, 45.0), "{t:?}");
    assert!(near(t.scale_x, 1.0), "{t:?}");
}

#[test]
fn shift_snaps_one_object_to_15_degree_steps() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    run(&s, "object.rotate", json!({ "ids": [id], "degrees": 10.0 }));
    h.run();
    // The corner moved with the turn; take the zone of the new box.
    let b = canvas::selection_box(&snap(&mut h)).unwrap();
    // 10 + 52 = 62 snaps to 60.
    turn(
        &mut h,
        zone(&v, kurbo::Point::new(b.x1, b.y1)),
        mm(&v, 45.0, 45.0),
        52.0,
        Modifiers::SHIFT,
    );
    assert!(near(transform(&s, id).rotation_deg, 60.0));
}

#[test]
fn shift_snaps_the_turn_of_several_objects() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a, b, mut h, v) = two_squares(&dir);
    run(&s, "object.rotate", json!({ "ids": [a], "degrees": 10.0 }));
    run(&s, "object.select", json!({ "ids": [a, b] }));
    h.run();
    let bx = canvas::selection_box(&snap(&mut h)).unwrap();
    let before = undo_len(&s);
    // A turn of 52 snaps to 45, about the mean of the centres (75, 45).
    turn(
        &mut h,
        zone(&v, kurbo::Point::new(bx.x1, bx.y1)),
        mm(&v, 75.0, 45.0),
        52.0,
        Modifiers::SHIFT,
    );
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    assert!(near(transform(&s, a).rotation_deg, 55.0));
    assert!(near(transform(&s, b).rotation_deg, 45.0));
    // The layout turned about the common centre.
    let cb = transform(&s, b).center_mm;
    let r = 30.0 / 2f64.sqrt();
    assert!(near(cb.x, 75.0 + r) && near(cb.y, 45.0 + r), "{cb:?}");
}

#[test]
fn a_turn_back_to_the_start_adds_no_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _id, mut h, v) = square(&dir);
    let before = undo_len(&s);
    let from = zone(&v, kurbo::Point::new(70.0, 70.0));
    turn_to(&mut h, from, mm(&v, 45.0, 45.0), 40.0);
    h.hover_at(from);
    h.run();
    h.drop_at(from);
    h.run();
    assert_eq!(undo_len(&s), before);
}

#[test]
fn a_turned_object_locks_free_scaling() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    turn(
        &mut h,
        zone(&v, kurbo::Point::new(70.0, 70.0)),
        mm(&v, 45.0, 45.0),
        30.0,
        Modifiers::NONE,
    );
    assert!(near(transform(&s, id).rotation_deg, 30.0));
    assert!(canvas::aspect_lock(&snap(&mut h), &[id]).is_some());
}

// ---------------------------------------------------------------- field

#[test]
fn the_angle_field_turns_one_object_to_the_typed_angle() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, _) = square(&dir);
    run(&s, "object.rotate", json!({ "ids": [id], "degrees": 10.0 }));
    h.run();
    assert_eq!(fields(&h).angle, "10.00");
    let before = undo_len(&s);
    type_text(&mut h, ANGLE, "-30");
    h.key_press(Key::Tab);
    h.run();
    h.run();
    assert_eq!(undo_len(&s), before + 1);
    let t = transform(&s, id);
    assert!(near(t.rotation_deg, 330.0), "{t:?}");
    assert!(
        near(t.center_mm.x, 45.0) && near(t.center_mm.y, 45.0),
        "{t:?}"
    );
    assert_eq!(fields(&h).angle, "330.00");
}

#[test]
fn escape_puts_the_angle_back() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, _) = square(&dir);
    let before = undo_len(&s);
    type_text(&mut h, ANGLE, "45");
    h.key_press(Key::Escape);
    h.run();
    h.run();
    assert_eq!(undo_len(&s), before);
    assert!(near(transform(&s, id).rotation_deg, 0.0));
    assert_eq!(fields(&h).angle, "0.00");
}

#[test]
fn the_angle_field_shows_mixed_and_sets_every_object() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a, b, mut h, _) = two_squares(&dir);
    // Equal angles: the common one.
    assert_eq!(fields(&h).angle, "0.00");
    run(&s, "object.rotate", json!({ "ids": [a], "degrees": 20.0 }));
    run(&s, "object.select", json!({ "ids": [a, b] }));
    h.run();
    h.run();
    assert_eq!(fields(&h).angle, "", "mixed angles show the Mixed hint");
    let (ca, cb) = (transform(&s, a).center_mm, transform(&s, b).center_mm);
    let before = undo_len(&s);
    type_text(&mut h, ANGLE, "45");
    h.key_press(Key::Enter);
    h.run();
    h.run();
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    for (id, c) in [(a, ca), (b, cb)] {
        let t = transform(&s, id);
        assert!(near(t.rotation_deg, 45.0), "{t:?}");
        assert!(
            near(t.center_mm.x, c.x) && near(t.center_mm.y, c.y),
            "{t:?}"
        );
    }
    assert_eq!(fields(&h).angle, "45.00");
}

#[test]
fn the_angle_field_follows_an_agent() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, _) = square(&dir);
    run(
        &s,
        "object.rotate",
        json!({ "ids": [id], "angle_deg": 123.5 }),
    );
    h.run();
    h.run();
    assert_eq!(fields(&h).angle, "123.50");
}

#[test]
fn tab_from_the_percent_field_reaches_the_angle() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _id, mut h, _) = square(&dir);
    let before = undo_len(&s);
    h.get_by_label("Scale (%)").click();
    h.run();
    let editing = |h: &AppHarness| fields(h).editing.map(|e| e.field);
    let mut reached = false;
    for _ in 0..3 {
        h.key_press(Key::Tab);
        h.run();
        if editing(&h) == Some(flatcraft::ui::prepare::SizeField::Angle) {
            reached = true;
            break;
        }
    }
    assert!(reached, "Tab reaches the Angle field");
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(undo_len(&s), before);
}

#[test]
fn the_rotate_card_opens_next_to_the_pointer() {
    let dir = tempfile::tempdir().unwrap();
    let (_s, _id, mut h, v) = square(&dir);
    let at = zone(&v, kurbo::Point::new(70.0, 70.0));
    h.hover_at(at);
    for _ in 0..4 {
        h.run();
    }
    let card = h.get_by_label(&help::rotate_card(Os::CURRENT)).rect();
    let canvas = app(&h).ui.prepare.canvas_rect;
    // On the canvas, not up over the ribbon, and close to the pointer.
    assert!(card.top() >= canvas.top(), "{card:?} {canvas:?}");
    assert!(card.distance_to_pos(at) < 60.0, "{card:?} {at:?}");
}
