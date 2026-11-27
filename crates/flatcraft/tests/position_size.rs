//! The Position and size group (X, Y, W, H, %, Keep aspect) and the
//! selection handles on the canvas: scaling follows Keep aspect, Shift
//! flips it for one drag, Option/Alt scales from the centre, and the fields
//! apply when they lose the focus.

mod common;
use common::RunPlan;

use common::{
    AppHarness, app, app_mut, click_at, double_click_at, harness, import_square, session,
    to_prepare, undo_len,
};
use egui::accesskit::Role;
use egui::{Key, Modifiers, Pos2, Vec2};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
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
    h.run_plan();
    let rect = app(&h).ui.prepare.canvas_rect;
    let view = CanvasView {
        offset: rect.center().to_vec2() - Vec2::new(mm.x as f32, mm.y as f32) * 2.0,
        px_per_mm: 2.0,
    };
    let p = &mut app_mut(&mut h).ui.prepare;
    p.view = view;
    p.fit_pending = false;
    h.run_plan();
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
    h.run_plan();
    h.drag_at(from);
    h.run_plan();
    for t in [0.25, 0.5, 0.75, 1.0] {
        h.hover_at(from + (to - from) * t);
        h.run_plan();
    }
}

fn drop_at(h: &mut AppHarness, to: Pos2) {
    h.drop_at(to);
    h.run_plan();
}

/// A whole drag with `modifiers` held from start to end.
fn drag(h: &mut AppHarness, from: Pos2, to: Pos2, modifiers: Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    drag_to(h, from, to);
    drop_at(h, to);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_plan();
}

fn mm(v: &CanvasView, x: f64, y: f64) -> Pos2 {
    v.mm_to_screen(kurbo::Point::new(x, y))
}

fn set_keep(h: &mut AppHarness, on: bool) {
    app_mut(h).ui.prepare.fields.keep_aspect = on;
    h.run_plan();
}

fn fields(h: &AppHarness) -> flatcraft::ui::prepare::Fields {
    app(h).ui.prepare.fields.clone()
}

/// Clicks into the field, selects its text and types `text`, without
/// leaving it.
fn type_text(h: &mut AppHarness, label: &str, text: &str) {
    h.get_by_label(label).click();
    h.run_plan();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_plan();
    h.get_by_label(label).type_text(text);
    h.run_plan();
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
    h.run_plan();
    let f = fields(&h);
    assert_eq!((f.x.as_str(), f.y.as_str()), ("30.00", "25.00"));
    drop_at(&mut h, to);
    // Scaling the right side out by 10 mm: W follows, X stays.
    set_keep(&mut h, false);
    let from = mm(&v, 80.0, 50.0);
    let to = mm(&v, 90.0, 50.0);
    drag_to(&mut h, from, to);
    h.run_plan();
    let f = fields(&h);
    assert_eq!((f.x.as_str(), f.width.as_str()), ("30.00", "60.00"));
    assert_eq!(f.height, "50.00");
    drop_at(&mut h, to);
}

#[test]
fn the_fields_follow_a_nudge() {
    let dir = tempfile::tempdir().unwrap();
    let (_s, _id, mut h, _) = square(&dir);
    // No field has the keys, so the arrow nudges the square by 1 mm.
    h.key_press(Key::ArrowRight);
    h.run_plan();
    h.run_plan();
    assert_eq!(fields(&h).x, "21.00");
}

#[test]
fn option_without_keep_aspect_stretches_from_the_centre() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, v) = square(&dir);
    set_keep(&mut h, false);
    // From the centre: half-sizes 25 grow to 35 and 30.
    drag(
        &mut h,
        mm(&v, 70.0, 70.0),
        mm(&v, 80.0, 75.0),
        Modifiers::ALT,
    );
    let b = bounds(&s, id);
    assert!(
        near(b.center().x, 45.0) && near(b.center().y, 45.0),
        "{b:?}"
    );
    assert!(near(b.width(), 70.0) && near(b.height(), 60.0), "{b:?}");
}

/// Two straight squares, 10 mm apart, both selected; the rig around them.
fn two_squares(dir: &tempfile::TempDir) -> (SharedSession, u64, u64, AppHarness, CanvasView) {
    let s = session(dir);
    let a = import_square(&s);
    let b = import_square(&s);
    run(&s, "object.move", json!({ "ids": [b], "dx_mm": 60.0 }));
    run(&s, "object.select", json!({ "ids": [a, b] }));
    let (h, v) = rig(dir, s.clone(), kurbo::Point::new(75.0, 45.0));
    (s, a, b, h, v)
}

#[test]
fn a_side_drag_stretches_two_straight_objects() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a, b, mut h, v) = two_squares(&dir);
    set_keep(&mut h, false);
    let before = undo_len(&s);
    // The right handle of the common box, 110 mm wide, out to 135 mm.
    drag(
        &mut h,
        mm(&v, 130.0, 45.0),
        mm(&v, 155.0, 45.0),
        Modifiers::NONE,
    );
    assert_eq!(undo_len(&s), before + 1);
    let (na, nb) = (bounds(&s, a), bounds(&s, b));
    let f = 135.0 / 110.0;
    assert!(near(na.x0, 20.0) && near(nb.x1, 155.0), "{na:?} {nb:?}");
    assert!(
        near(na.width(), 50.0 * f) && near(nb.x0, 20.0 + 60.0 * f),
        "{na:?} {nb:?}"
    );
    assert!(
        near(na.height(), 50.0) && near(nb.height(), 50.0),
        "{na:?} {nb:?}"
    );
}

#[test]
fn a_typed_width_stretches_two_straight_objects() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a, b, mut h, _) = two_squares(&dir);
    set_keep(&mut h, false);
    type_text(&mut h, "Width (mm)", "220");
    h.key_press(Key::Enter);
    h.run_plan();
    let (na, nb) = (bounds(&s, a), bounds(&s, b));
    assert!(near(nb.x1 - na.x0, 220.0), "{na:?} {nb:?}");
    assert!(
        near(na.height(), 50.0) && near(nb.height(), 50.0),
        "{na:?} {nb:?}"
    );
    // With Keep aspect on, it keeps the shape as before.
    set_keep(&mut h, true);
    type_text(&mut h, "Width (mm)", "110");
    h.key_press(Key::Enter);
    h.run_plan();
    let (na, nb) = (bounds(&s, a), bounds(&s, b));
    assert!(near(nb.x1 - na.x0, 110.0), "{na:?} {nb:?}");
    assert!(near(na.height(), 25.0), "{na:?}");
}

#[test]
fn a_turned_object_locks_keep_aspect_for_the_selection() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a, _b, mut h, _) = two_squares(&dir);
    run(
        &s,
        "object.rotate",
        json!({ "ids": [a], "degrees": 30.0, "about": "each" }),
    );
    set_keep(&mut h, false);
    let node = h.get_by_role_and_label(Role::CheckBox, "Keep aspect");
    let n = egui_kittest::kittest::NodeT::accesskit_node(&node);
    assert!(n.is_disabled());
    assert_eq!(n.toggled(), Some(egui::accesskit::Toggled::True));
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
    h.run_plan();
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
    h.run_plan();
    let b = bounds(&s, id);
    assert!(near(b.y0, 33.0) && near(b.x0, 20.0), "{b:?}");
    // Now in X: type and Shift-Tab out again.
    type_text(&mut h, "X (mm)", "12.5");
    h.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
    h.run_plan();
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
    h.run_plan();
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
    h.run_plan();
    h.run_plan();
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
    h.run_plan();
    h.run_plan();
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
    h.run_plan();
    let editing = |h: &AppHarness| fields(h).editing.map(|e| e.field);
    let mut seen = vec![editing(&h)];
    for _ in 0..4 {
        h.key_press(Key::Tab);
        h.run_plan();
        seen.push(editing(&h));
    }
    use flatcraft::ui::prepare::SizeField::*;
    assert_eq!(
        seen,
        [Some(X), Some(Y), Some(Width), Some(Height), Some(Percent)],
        "the focus moved through every field"
    );
    h.key_press(Key::Escape);
    h.run_plan();
    assert_eq!(undo_len(&s), before, "no undo step for an unchanged value");
}

#[test]
fn a_typed_value_applies_at_once_when_the_tab_changes() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, _) = square(&dir);
    let before = undo_len(&s);
    type_text(&mut h, "Width (mm)", "80");
    h.get_by_role_and_label(Role::Tab, "View").click();
    h.run_plan();
    assert_eq!(undo_len(&s), before + 1);
    assert!(near(bounds(&s, id).width(), 80.0));
    assert!(fields(&h).editing.is_none());
}

#[test]
fn a_tab_applies_to_the_selection_of_now() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a, b, mut h, _) = two_squares(&dir);
    run(&s, "object.select", json!({ "ids": [a] }));
    h.run_plan();
    h.get_by_label("Width (mm)").click();
    h.run_plan();
    // An agent selects the other square while the field has the focus.
    run(&s, "object.select", json!({ "ids": [b] }));
    h.run_plan();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_plan();
    h.get_by_label("Width (mm)").type_text("80");
    h.run_plan();
    h.key_press(Key::Tab);
    h.run_plan();
    assert!(near(bounds(&s, a).width(), 50.0), "the old selection stays");
    assert!(near(bounds(&s, b).width(), 80.0));
}

#[test]
fn each_applied_value_is_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, _) = square(&dir);
    let before = undo_len(&s);
    type_text(&mut h, "X (mm)", "10");
    h.key_press(Key::Tab);
    h.run_plan();
    // Now in Y.
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_plan();
    h.get_by_label("Y (mm)").type_text("15");
    h.run_plan();
    h.key_press(Key::Enter);
    h.run_plan();
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
    h.run_plan();
    assert_eq!(fields(&h).width, "77");
    // The other fields show the new size.
    assert_eq!(fields(&h).height, "100.00");
    h.key_press(Key::Enter);
    h.run_plan();
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
    h.run_plan();
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
    h.run_plan();
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
    h.run_plan();
    h.run_plan();
    assert_eq!(fields(&h).height, "80.00");
    // Leaving H unchanged applies nothing more.
    let before = undo_len(&s);
    h.key_press(Key::Tab);
    h.run_plan();
    assert_eq!(undo_len(&s), before);
}

#[test]
fn a_typed_value_applies_to_its_document_when_the_document_changes() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h, _) = square(&dir);
    let first = s.lock().unwrap().doc().id();
    let before = undo_len(&s);
    type_text(&mut h, "Width (mm)", "80");
    h.get_by_role_and_label(Role::Button, "New document")
        .click();
    h.run_plan();
    h.run_plan();
    let g = s.lock().unwrap();
    assert_eq!(g.docs.len(), 2);
    assert_ne!(g.doc().id(), first, "the new document is active");
    let a = &g.docs[g.doc_index(first).unwrap()];
    assert_eq!(
        a.undo.undo_len(),
        before + 1,
        "one undo step in its document"
    );
    assert!(g.docs[1].document.objects.is_empty());
    let o = a
        .document
        .get(flatcraft_core::document::ObjectId(id))
        .unwrap();
    let b = flatcraft_engine::commands::placed_bounds(o, Default::default()).unwrap();
    drop(g);
    assert!(near(b.width(), 80.0), "{b:?}");
    assert!(fields(&h).editing.is_none());
}

/// The width of the square in the saved project file.
fn saved_width(path: &std::path::Path) -> f64 {
    let doc = flatcraft_engine::commands::project::load_rebuilt(
        &flatcraft_device::ProfileStore::load(&path.parent().unwrap().join("profiles")).unwrap(),
        path,
    )
    .unwrap();
    flatcraft_engine::commands::placed_bounds(&doc.objects[0], Default::default())
        .unwrap()
        .width()
}

fn saving_rig(dir: &tempfile::TempDir) -> (SharedSession, AppHarness, std::path::PathBuf) {
    let s = session(dir);
    import_square(&s);
    let path = dir.path().join("sized.flatcraft");
    let picker = common::FakePicker::answering([path.clone()]);
    let mut h = common::harness_with(dir, s.clone(), |a| a.with_file_picker(picker.clone()));
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.run_plan();
    (s, h, path)
}

#[test]
fn a_typed_value_counts_before_a_save_click() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h, path) = saving_rig(&dir);
    type_text(&mut h, "Width (mm)", "80");
    h.get_all_by_role_and_label(Role::Button, "Save")
        .next()
        .unwrap()
        .click();
    h.run_plan();
    assert!(near(saved_width(&path), 80.0));
    assert!(!s.lock().unwrap().doc().dirty, "saved after the change");
}

#[test]
fn a_typed_value_counts_before_cmd_s() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h, path) = saving_rig(&dir);
    type_text(&mut h, "Width (mm)", "80");
    if cfg!(target_os = "macos") {
        // The menu bar owns Cmd+S there; it sends this action.
        let doc_id = s.lock().unwrap().doc().id();
        let ctx = h.ctx.clone();
        app_mut(&mut h).apply(&ctx, vec![flatcraft::ui::Action::Save { doc_id }]);
    } else {
        h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    }
    h.run_plan();
    assert!(near(saved_width(&path), 80.0));
    assert!(!s.lock().unwrap().doc().dirty, "saved after the change");
}

#[test]
fn a_typed_value_counts_before_closing_the_tab() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h, path) = saving_rig(&dir);
    // Saved, so the document is clean before the typing.
    run(&s, "project.save", json!({ "path": path }));
    h.run_plan();
    assert!(!s.lock().unwrap().doc().dirty);
    let id = s.lock().unwrap().doc().document.objects[0].id.0;
    type_text(&mut h, "Width (mm)", "80");
    h.get_by_role_and_label(Role::Button, "Close sized").click();
    h.run_plan();
    h.get_by_label("Save changes to sized?");
    assert!(s.lock().unwrap().doc().dirty);
    assert!(app(&h).ui.toasts.is_empty(), "no error toast");
    assert!(near(bounds(&s, id).width(), 80.0));
}

#[test]
fn linked_copies_turned_two_ways_lock_keep_aspect_with_their_reason() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let r = s
        .lock()
        .unwrap()
        .execute(
            "object.copies",
            json!({ "id": a, "count": 1, "linked": true }),
            Origin::User,
        )
        .unwrap();
    let b = r["ids"][0].as_u64().unwrap();
    run(
        &s,
        "object.rotate",
        json!({ "ids": [b], "degrees": 90.0, "about": "each" }),
    );
    run(&s, "object.select", json!({ "ids": [a, b] }));
    let (mut h, _) = rig(&dir, s.clone(), kurbo::Point::new(75.0, 45.0));
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
    let why = flatcraft::ui::canvas::aspect_lock(&snap, &[a, b]).expect("locked");
    assert!(
        why.starts_with("Linked copies here are turned different ways"),
        "{why}"
    );
    assert!(why.contains("Unlink copies"), "it says what to do: {why}");
    assert!(!flatcraft::ui::canvas::free_aspect(&snap, &[a, b]));
    // Each copy alone may stretch.
    assert!(flatcraft::ui::canvas::free_aspect(&snap, &[b]));
    let node = h.get_by_role_and_label(Role::CheckBox, "Keep aspect");
    assert!(egui_kittest::kittest::NodeT::accesskit_node(&node).is_disabled());
}

/// Keep aspect is a lock: a click opens it (off) and closes it (on); its
/// accessible name and state follow.
#[test]
fn the_keep_aspect_lock_opens_and_closes() {
    let dir = tempfile::tempdir().unwrap();
    let (_s, _id, mut h, _) = square(&dir);
    let toggled = |h: &AppHarness| {
        h.get_by_role_and_label(Role::CheckBox, "Keep aspect")
            .accesskit_node()
            .toggled()
    };
    assert_eq!(toggled(&h), Some(egui::accesskit::Toggled::True));
    h.get_by_role_and_label(Role::CheckBox, "Keep aspect")
        .click();
    h.run_plan();
    assert!(!fields(&h).keep_aspect);
    assert_eq!(toggled(&h), Some(egui::accesskit::Toggled::False));
    h.get_by_role_and_label(Role::CheckBox, "Keep aspect")
        .click();
    h.run_plan();
    assert!(fields(&h).keep_aspect);
}

// ---------------------------------------------------------------- selecting

/// Presses and releases the primary button at `at`.

#[test]
fn a_double_click_selects_the_whole_value() {
    let dir = tempfile::tempdir().unwrap();
    let (_s, _id, mut h, _) = square(&dir);
    assert_eq!(fields(&h).width, "50.00");
    // On the "50" part, which a word selection would take alone.
    let r = h.get_by_label("Width (mm)").rect();
    let at = Pos2::new(r.left() + 8.0, r.center().y);
    h.hover_at(at);
    h.run_plan();
    // A test frame lasts longer than a real double-click may take. The
    // click on the Layout tab is long past, so this is not a triple click.
    h.ctx
        .options_mut(|o| o.input_options.max_double_click_delay = 2.0);
    for _ in 0..8 {
        h.step();
    }
    double_click_at(&mut h, at);
    h.run_plan();
    h.get_by_label("Width (mm)").type_text("7");
    h.run_plan();
    assert_eq!(fields(&h).width, "7", "typing replaced the whole value");
}

#[test]
fn a_single_click_places_the_cursor_without_a_selection() {
    let dir = tempfile::tempdir().unwrap();
    let (_s, _id, mut h, _) = square(&dir);
    let r = h.get_by_label("Width (mm)").rect();
    // Past the end of the text: the cursor goes to the end.
    let at = Pos2::new(r.right() - 2.0, r.center().y);
    h.hover_at(at);
    h.run_plan();
    click_at(&mut h, at);
    h.run_plan();
    h.get_by_label("Width (mm)").type_text("5");
    h.run_plan();
    assert_eq!(
        fields(&h).width,
        "50.005",
        "the value stays, the digit is added"
    );
}

#[test]
fn tab_into_a_field_selects_the_whole_value() {
    let dir = tempfile::tempdir().unwrap();
    let (_s, _id, mut h, _) = square(&dir);
    h.get_by_label("X (mm)").click();
    h.run_plan();
    h.key_press(Key::Tab);
    h.run_plan();
    h.run_plan();
    h.get_by_label("Y (mm)").type_text("9");
    h.run_plan();
    assert_eq!(fields(&h).y, "9");
    // Shift-Tab back into X too.
    h.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
    h.run_plan();
    h.run_plan();
    h.get_by_label("X (mm)").type_text("4");
    h.run_plan();
    assert_eq!(fields(&h).x, "4");
}

/// The bracket is part of the Keep aspect toggle: a click on one of its
/// arms toggles it. A click into W still goes to W.
#[test]
fn a_click_on_the_bracket_toggles_keep_aspect() {
    let dir = tempfile::tempdir().unwrap();
    let (_s, _id, mut h, _) = square(&dir);
    let keep = h
        .get_by_role_and_label(Role::CheckBox, "Keep aspect")
        .rect();
    let w = h.get_by_label("Width (mm)").rect();
    // The upper arm: level with W, inside the toggle.
    let arm = Pos2::new(keep.right() - 3.0, w.center().y);
    assert!(fields(&h).keep_aspect);
    h.hover_at(arm);
    h.run_plan();
    click_at(&mut h, arm);
    h.run_plan();
    assert!(!fields(&h).keep_aspect, "the arm toggled it");
    // A click into W focuses W and leaves Keep aspect as it is.
    let into_w = Pos2::new(w.left() + 4.0, w.center().y);
    h.hover_at(into_w);
    h.run_plan();
    click_at(&mut h, into_w);
    h.run_plan();
    assert!(!fields(&h).keep_aspect);
    assert_eq!(
        fields(&h).editing.map(|e| e.field),
        Some(flatcraft::ui::prepare::SizeField::Width)
    );
}
