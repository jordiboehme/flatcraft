//! Number fields take a calculation (`100/3`, `2in`, `+=10`): one test per
//! kind of field (length, angle, whole number, percent), each with an
//! input it cannot read. Agents still send plain numbers.

mod common;

use common::{AppHarness, app, harness, import_square, session, to_prepare, undo_len};
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

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

fn rotation(s: &SharedSession, id: u64) -> f64 {
    s.lock()
        .unwrap()
        .doc()
        .document
        .get(flatcraft_core::document::ObjectId(id))
        .unwrap()
        .transform
        .rotation_deg
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

/// A 50 mm square at 20,20, selected, in Prepare on the Layout tab.
fn square(dir: &tempfile::TempDir) -> (SharedSession, u64, AppHarness) {
    let s = session(dir);
    let id = import_square(&s);
    let mut h = harness(dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.run();
    (s, id, h)
}

/// Clicks into the field, selects its text, types `text` and presses
/// `key` to leave it.
fn enter(h: &mut AppHarness, label: &str, text: &str, key: Key) {
    h.get_by_label(label).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    for c in text.chars() {
        h.event(Event::Text(c.to_string()));
    }
    h.run();
    h.key_press(key);
    h.run();
    h.run();
}

fn fields(h: &AppHarness) -> flatcraft::ui::prepare::Fields {
    app(h).ui.prepare.fields.clone()
}

fn has_focus(h: &AppHarness, label: &str) -> bool {
    h.get_by_label(label).is_focused()
}

#[test]
fn a_length_field_takes_a_calculation_with_units() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h) = square(&dir);
    let before = undo_len(&s);
    enter(&mut h, "Width (mm)", "50/2 + 1in", Key::Tab);
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    assert!(near(bounds(&s, id).width(), 50.4), "{:?}", bounds(&s, id));
    assert_eq!(fields(&h).width, "50.40", "the field shows the result");
    // A relative edit counts from the value of now.
    let x0 = bounds(&s, id).x0;
    enter(&mut h, "X (mm)", "+=1cm", Key::Enter);
    assert!(near(bounds(&s, id).x0, x0 + 10.0), "{:?}", bounds(&s, id));
}

#[test]
fn a_length_field_it_cannot_read_says_why_and_escape_restores() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h) = square(&dir);
    let before = undo_len(&s);
    enter(&mut h, "Width (mm)", "2in+", Key::Tab);
    assert_eq!(undo_len(&s), before, "nothing applied");
    assert!(near(bounds(&s, id).width(), 50.0));
    let msg = "Can't read '2in+': something is missing after +.";
    h.get_by_label(msg);
    assert_eq!(fields(&h).width, "2in+", "the text stays to fix it");
    assert!(has_focus(&h, "Width (mm)"), "the field keeps the keys");
    // Escape puts the old value back and the message goes.
    h.key_press(Key::Escape);
    h.run();
    h.run();
    assert_eq!(fields(&h).width, "50.00");
    assert!(h.query_by_label(msg).is_none());
    assert_eq!(undo_len(&s), before);
}

#[test]
fn a_fixed_text_applies_and_the_message_goes() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h) = square(&dir);
    enter(&mut h, "Height (mm)", "10/0", Key::Enter);
    h.get_by_label("Can't use '10/0': you can't divide by 0.");
    assert!(near(bounds(&s, id).height(), 50.0));
    // Still in the field: fix the text and leave it.
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.event(Event::Text("10/4".into()));
    h.run();
    h.key_press(Key::Enter);
    h.run();
    h.run();
    assert!(near(bounds(&s, id).height(), 2.5), "{:?}", bounds(&s, id));
    assert!(fields(&h).error.is_none());
}

#[test]
fn an_angle_field_takes_degrees_and_a_turn_counts_once() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h) = square(&dir);
    enter(&mut h, "Angle (degrees)", "360/8 + 45deg", Key::Enter);
    assert!(near(rotation(&s, id), 90.0));
    enter(&mut h, "Angle (degrees)", "+=300\u{00b0}", Key::Enter);
    assert!(near(rotation(&s, id), 30.0), "{}", rotation(&s, id));
    assert_eq!(fields(&h).angle, "30.00");
    // A length unit does not fit an angle.
    let before = undo_len(&s);
    enter(&mut h, "Angle (degrees)", "2in", Key::Enter);
    assert_eq!(undo_len(&s), before);
    h.get_by_label("Can't read '2in': in does not fit here: use deg or \u{00b0}.");
}

#[test]
fn a_percent_field_scales_by_a_calculation() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h) = square(&dir);
    enter(&mut h, "Scale (%)", "*=3/2", Key::Enter);
    assert!(near(bounds(&s, id).width(), 75.0), "{:?}", bounds(&s, id));
    // 0 % is no size at all.
    let before = undo_len(&s);
    enter(&mut h, "Scale (%)", "100-100", Key::Enter);
    assert_eq!(undo_len(&s), before);
    h.get_by_label("0 is too small: give a value above 0 %.");
}

#[test]
fn a_whole_number_field_rounds_and_says_what_it_cannot_read() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let passes = |s: &SharedSession| s.lock().unwrap().doc().document.job.passes;
    h.get_by_role_and_label(Role::SpinButton, "Passes").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.event(Event::Text("5/2".into()));
    h.run();
    h.key_press(Key::Enter);
    h.run();
    h.run();
    assert_eq!(passes(&s), Some(3), "2.5 rounds to 3");
    h.get_by_role_and_label(Role::SpinButton, "Passes (modified)")
        .click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.event(Event::Text("2*".into()));
    h.run();
    h.key_press(Key::Enter);
    h.run();
    h.run();
    assert_eq!(passes(&s), Some(3), "nothing applied");
    h.get_by_label("Can't read '2*': something is missing after *.");
}

#[test]
fn agents_still_send_plain_numbers() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let r = s.lock().unwrap().execute(
        "object.move",
        json!({ "ids": [id], "x_mm": "1in" }),
        Origin::Agent,
    );
    assert!(r.is_err(), "a calculation is no number for an agent");
    assert!(near(bounds(&s, id).x0, 20.0));
}

#[test]
fn a_text_it_cannot_read_left_behind_by_a_tab_switch_says_so_in_a_toast() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h) = square(&dir);
    let before = undo_len(&s);
    enter(&mut h, "Width (mm)", "(50", Key::Tab);
    assert!(fields(&h).error.is_some());
    h.get_by_role_and_label(Role::Tab, "View").click();
    h.run();
    h.run();
    assert_eq!(undo_len(&s), before);
    assert!(near(bounds(&s, id).width(), 50.0));
    assert!(fields(&h).error.is_none() && fields(&h).editing.is_none());
    let toast = app(&h).ui.toasts.last().cloned().unwrap();
    assert_eq!(
        toast.text,
        "Can't read '(50': a ) is missing. Nothing changed."
    );
    assert!(
        !h.ctx.egui_wants_keyboard_input(),
        "no field keeps the keys"
    );
}
