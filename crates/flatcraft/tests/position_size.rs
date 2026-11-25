//! The Position and size group (X, Y, W, H, %, Keep aspect) and the
//! selection handles on the canvas: scaling follows Keep aspect, Shift
//! flips it for one drag, Option/Alt scales from the centre, and the fields
//! apply when they lose the focus.

mod common;

use common::{AppHarness, app, app_mut, harness, import_square, session, to_prepare, undo_len};
use egui::accesskit::Role;
use egui::{Key, Modifiers, Pos2, Vec2};
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::canvas::CanvasView;
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

/// The placed box of an object, as `document.state` reports it.
fn bounds(s: &SharedSession, id: u64) -> kurbo::Rect {
    let st = s
        .lock()
        .unwrap()
        .execute("document.state", json!({}), Origin::User)
        .unwrap();
    let o: &Value = st["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .unwrap();
    let b = &o["bounds_mm"];
    let f = |k: &str| b[k].as_f64().unwrap();
    kurbo::Rect::new(f("x0"), f("y0"), f("x1"), f("y1"))
}

fn run(s: &SharedSession, id: &str, params: Value) {
    s.lock().unwrap().execute(id, params, Origin::User).unwrap();
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

/// One square, 50 mm at 20,20, and the rig around its centre.
fn square(dir: &tempfile::TempDir) -> (SharedSession, u64, AppHarness, CanvasView) {
    let s = session(dir);
    let id = import_square(&s);
    let (h, v) = rig(dir, s.clone(), kurbo::Point::new(45.0, 45.0));
    (s, id, h, v)
}

/// Presses at `from` and moves to `to` in steps, one frame each; the
/// button stays down.
fn drag_to(h: &mut AppHarness, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    for t in [0.25, 0.5, 0.75, 1.0] {
        h.hover_at(from + (to - from) * t);
        h.run();
    }
}

fn drop_at(h: &mut AppHarness, to: Pos2) {
    h.drop_at(to);
    h.run();
}

/// A whole drag with `modifiers` held from start to end.
fn drag(h: &mut AppHarness, from: Pos2, to: Pos2, modifiers: Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    drag_to(h, from, to);
    drop_at(h, to);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

fn mm(v: &CanvasView, x: f64, y: f64) -> Pos2 {
    v.mm_to_screen(kurbo::Point::new(x, y))
}

fn set_keep(h: &mut AppHarness, on: bool) {
    app_mut(h).ui.prepare.fields.keep_aspect = on;
    h.run();
}

fn fields(h: &AppHarness) -> flatcraft::ui::prepare::Fields {
    app(h).ui.prepare.fields.clone()
}

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

// ---------------------------------------------------------------- handles

#[test]
fn a_corner_drag_without_keep_aspect_stretches_both_ways() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    set_keep(&mut h, false);
    let before = undo_len(&s);
    drag(
        &mut h,
        mm(&v, 70.0, 70.0),
        mm(&v, 95.0, 80.0),
        Modifiers::NONE,
    );
    assert_eq!(undo_len(&s), before + 1);
    let b = bounds(&s, id);
    assert!(near(b.x0, 20.0) && near(b.y0, 20.0), "{b:?}");
    assert!(near(b.width(), 75.0) && near(b.height(), 60.0), "{b:?}");
}

#[test]
fn a_corner_drag_with_keep_aspect_keeps_the_shape() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    drag(
        &mut h,
        mm(&v, 70.0, 70.0),
        mm(&v, 95.0, 80.0),
        Modifiers::NONE,
    );
    let b = bounds(&s, id);
    // The pointer projected on the diagonal: 1 + (25 + 10) / 100.
    assert!(near(b.x0, 20.0) && near(b.y0, 20.0), "{b:?}");
    assert!(near(b.width(), 67.5) && near(b.height(), 67.5), "{b:?}");
}

#[test]
fn a_side_drag_without_keep_aspect_changes_one_side() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    set_keep(&mut h, false);
    // The right handle, 20 mm out.
    drag(
        &mut h,
        mm(&v, 70.0, 45.0),
        mm(&v, 90.0, 45.0),
        Modifiers::NONE,
    );
    let b = bounds(&s, id);
    assert!(near(b.x0, 20.0) && near(b.y0, 20.0), "{b:?}");
    assert!(near(b.width(), 70.0) && near(b.height(), 50.0), "{b:?}");
}

#[test]
fn a_side_drag_with_keep_aspect_grows_about_the_opposite_side() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    // The bottom handle, 20 mm down.
    drag(
        &mut h,
        mm(&v, 45.0, 70.0),
        mm(&v, 45.0, 90.0),
        Modifiers::NONE,
    );
    let b = bounds(&s, id);
    assert!(near(b.y0, 20.0) && near(b.center().x, 45.0), "{b:?}");
    assert!(near(b.width(), 70.0) && near(b.height(), 70.0), "{b:?}");
}

#[test]
fn shift_flips_keep_aspect_for_one_drag() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    // Keep aspect on, Shift held: it stretches.
    drag(
        &mut h,
        mm(&v, 70.0, 70.0),
        mm(&v, 95.0, 80.0),
        Modifiers::SHIFT,
    );
    let b = bounds(&s, id);
    assert!(near(b.width(), 75.0) && near(b.height(), 60.0), "{b:?}");
    assert!(fields(&h).keep_aspect, "the switch itself stays on");
    // Keep aspect off, Shift held: it keeps the shape.
    set_keep(&mut h, false);
    drag(
        &mut h,
        mm(&v, 95.0, 80.0),
        mm(&v, 105.0, 90.0),
        Modifiers::SHIFT,
    );
    let n = bounds(&s, id);
    assert!(near(n.width() / n.height(), 75.0 / 60.0), "{n:?}");
    assert!(n.width() > 75.0 + 1.0, "{n:?}");
    assert!(!fields(&h).keep_aspect);
}

#[test]
fn option_scales_from_the_centre() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    // The bottom-right corner 10 mm out along the diagonal: from the
    // centre, half-size 25 grows to 35.
    drag(
        &mut h,
        mm(&v, 70.0, 70.0),
        mm(&v, 80.0, 80.0),
        Modifiers::ALT,
    );
    let b = bounds(&s, id);
    assert!(
        near(b.center().x, 45.0) && near(b.center().y, 45.0),
        "{b:?}"
    );
    assert!(near(b.width(), 70.0) && near(b.height(), 70.0), "{b:?}");
}

#[test]
fn the_fields_follow_a_drag() {
    let dir = tempfile::tempdir().unwrap();
    let (_s, _id, mut h, v) = square(&dir);
    // Moving: X and Y follow.
    let from = mm(&v, 45.0, 45.0);
    let to = mm(&v, 55.0, 50.0);
    drag_to(&mut h, from, to);
    h.run();
    let f = fields(&h);
    assert_eq!((f.x.as_str(), f.y.as_str()), ("30.00", "25.00"));
    drop_at(&mut h, to);
    // Scaling the right side out by 10 mm: W follows, X stays.
    set_keep(&mut h, false);
    let from = mm(&v, 80.0, 50.0);
    let to = mm(&v, 90.0, 50.0);
    drag_to(&mut h, from, to);
    h.run();
    let f = fields(&h);
    assert_eq!((f.x.as_str(), f.width.as_str()), ("30.00", "60.00"));
    assert_eq!(f.height, "50.00");
    drop_at(&mut h, to);
}

#[test]
fn the_fields_follow_a_nudge() {
    let dir = tempfile::tempdir().unwrap();
    let (_s, _id, mut h, v) = square(&dir);
    // A click on the empty canvas gives the keys back to the window.
    let _ = v;
    h.key_press(Key::ArrowRight);
    h.run();
    h.run();
    assert_eq!(fields(&h).x, "21.00");
}

// ---------------------------------------------------------------- fields

#[test]
fn tab_applies_the_width() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, _) = square(&dir);
    let before = undo_len(&s);
    type_text(&mut h, "Width (mm)", "80");
    assert_eq!(undo_len(&s), before, "nothing yet while typing");
    h.key_press(Key::Tab);
    h.run();
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    let b = bounds(&s, id);
    assert!(near(b.width(), 80.0), "{b:?}");
}

#[test]
fn shift_tab_applies_x() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, _) = square(&dir);
    type_text(&mut h, "Y (mm)", "33");
    h.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
    h.run();
    let b = bounds(&s, id);
    assert!(near(b.y0, 33.0) && near(b.x0, 20.0), "{b:?}");
    // Now in X: type and Shift-Tab out again.
    type_text(&mut h, "X (mm)", "12.5");
    h.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
    h.run();
    let b = bounds(&s, id);
    assert!(near(b.x0, 12.5) && near(b.y0, 33.0), "{b:?}");
}

#[test]
fn a_click_elsewhere_applies_the_height() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    type_text(&mut h, "Height (mm)", "25");
    // The empty mat, away from the square.
    let p = mm(&v, 150.0, 150.0);
    h.hover_at(p);
    h.run();
    h.event(egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.event(egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    h.run();
    let b = bounds(&s, id);
    assert!(near(b.height(), 25.0), "{b:?}");
}

#[test]
fn escape_puts_the_shown_value_back() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, _) = square(&dir);
    let before = undo_len(&s);
    type_text(&mut h, "Width (mm)", "80");
    h.key_press(Key::Escape);
    h.run();
    h.run();
    assert_eq!(undo_len(&s), before);
    assert!(near(bounds(&s, id).width(), 50.0));
    assert_eq!(fields(&h).width, "50.00");
    assert!(
        !s.lock().unwrap().doc().selection.is_empty(),
        "Escape only left the field"
    );
}

#[test]
fn tabbing_through_unchanged_fields_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _id, mut h, _) = square(&dir);
    let before = undo_len(&s);
    h.get_by_label("X (mm)").click();
    h.run();
    for _ in 0..5 {
        h.key_press(Key::Tab);
        h.run();
    }
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(undo_len(&s), before, "no undo step for an unchanged value");
}

#[test]
fn each_applied_value_is_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, _) = square(&dir);
    let before = undo_len(&s);
    type_text(&mut h, "X (mm)", "10");
    h.key_press(Key::Tab);
    h.run();
    // Now in Y.
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.get_by_label("Y (mm)").type_text("15");
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(undo_len(&s), before + 2);
    let b = bounds(&s, id);
    assert!(near(b.x0, 10.0) && near(b.y0, 15.0), "{b:?}");
}

#[test]
fn a_focused_field_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, _) = square(&dir);
    type_text(&mut h, "Width (mm)", "77");
    // An agent scales the square while the person types.
    run(&s, "object.scale", json!({ "ids": [id], "factor": 2.0 }));
    h.run();
    assert_eq!(fields(&h).width, "77");
    // The other fields show the new size.
    assert_eq!(fields(&h).height, "100.00");
    h.key_press(Key::Enter);
    h.run();
    assert!(near(bounds(&s, id).width(), 77.0));
}

#[test]
fn x_and_y_are_the_corner_of_the_turned_shape() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    run(&s, "object.rotate", json!({ "ids": [id], "degrees": 30.0 }));
    let b = bounds(&s, id);
    let (mut h, _) = rig(&dir, s.clone(), kurbo::Point::new(45.0, 45.0));
    let f = fields(&h);
    assert_eq!(f.x, format!("{:.2}", b.x0));
    assert_eq!(f.y, format!("{:.2}", b.y0));
    type_text(&mut h, "X (mm)", "5");
    h.key_press(Key::Enter);
    h.run();
    let n = bounds(&s, id);
    assert!(near(n.x0, 5.0) && near(n.y0, b.y0), "{n:?}");
    assert!(near(n.width(), b.width()), "only moved");
}

#[test]
fn x_moves_a_multi_selection_as_one() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = import_square(&s);
    run(&s, "object.move", json!({ "ids": [b], "dx_mm": 60.0 }));
    run(&s, "object.select", json!({ "ids": [a, b] }));
    let (mut h, _) = rig(&dir, s.clone(), kurbo::Point::new(75.0, 45.0));
    assert_eq!(fields(&h).x, "20.00");
    assert_eq!(fields(&h).width, "110.00");
    let before = undo_len(&s);
    type_text(&mut h, "X (mm)", "0");
    h.key_press(Key::Tab);
    h.run();
    assert_eq!(undo_len(&s), before + 1);
    let (na, nb) = (bounds(&s, a), bounds(&s, b));
    assert!(near(na.x0, 0.0) && near(nb.x0, 60.0), "{na:?} {nb:?}");
}

#[test]
fn a_field_tabbed_into_shows_the_new_value() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _id, mut h, _) = square(&dir);
    // Keep aspect is on: W 80 makes H 80 too, and H has the focus now.
    type_text(&mut h, "Width (mm)", "80");
    h.key_press(Key::Tab);
    h.run();
    h.run();
    assert_eq!(fields(&h).height, "80.00");
    // Leaving H unchanged applies nothing more.
    let before = undo_len(&s);
    h.key_press(Key::Tab);
    h.run();
    assert_eq!(undo_len(&s), before);
}
