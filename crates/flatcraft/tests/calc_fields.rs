//! Number fields take a calculation (`100/3`, `2in`, `+=10`): one test per
//! kind of field (length, angle, whole number, percent), each with an
//! input it cannot read. Agents still send plain numbers.

mod common;
use common::RunPlan;

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
    h.run_plan();
    (s, id, h)
}

/// Clicks into the field, selects its text, types `text` and presses
/// `key` to leave it.
fn enter(h: &mut AppHarness, label: &str, text: &str, key: Key) {
    h.get_by_label(label).click();
    h.run_plan();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_plan();
    for c in text.chars() {
        h.event(Event::Text(c.to_string()));
    }
    h.run_plan();
    h.key_press(key);
    h.run_plan();
    h.run_plan();
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
fn enter_with_a_text_it_cannot_read_keeps_it_and_escape_restores() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h) = square(&dir);
    let before = undo_len(&s);
    enter(&mut h, "Width (mm)", "2in+", Key::Enter);
    assert_eq!(undo_len(&s), before, "nothing applied");
    assert!(near(bounds(&s, id).width(), 50.0));
    let msg = "Can't read '2in+': something is missing after +. Press Esc to go back.";
    h.get_by_label(msg);
    assert_eq!(fields(&h).width, "2in+", "the text stays to fix it");
    assert!(has_focus(&h, "Width (mm)"), "the field keeps the keys");
    // Escape puts the old value back and the message goes.
    h.key_press(Key::Escape);
    h.run_plan();
    h.run_plan();
    assert_eq!(fields(&h).width, "50.00");
    assert!(h.query_by_label(msg).is_none());
    assert_eq!(undo_len(&s), before);
    assert!(app(&h).ui.toasts.is_empty(), "Esc is no error");
}

#[test]
fn tab_with_a_text_it_cannot_read_restores_and_says_so_in_a_toast() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h) = square(&dir);
    let before = undo_len(&s);
    enter(&mut h, "Width (mm)", "2in+", Key::Tab);
    assert_eq!(undo_len(&s), before, "nothing applied");
    assert!(near(bounds(&s, id).width(), 50.0));
    assert_eq!(fields(&h).width, "50.00", "the old value is back");
    assert!(fields(&h).error.is_none());
    let toast = app(&h).ui.toasts.last().cloned().unwrap();
    assert_eq!(
        toast.text,
        "Can't read '2in+': something is missing after +. Nothing changed."
    );
    // The focus went on with Tab and stays there.
    for _ in 0..3 {
        h.run_plan();
    }
    assert!(!has_focus(&h, "Width (mm)"));
    assert!(has_focus(&h, "Height (mm)"));
}

#[test]
fn a_click_on_another_field_after_a_bad_width_moves_the_focus_there() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _id, mut h) = square(&dir);
    let passes = |s: &SharedSession| s.lock().unwrap().doc().document.job.passes;
    // Passes is in the Job panel, over the selection.
    common::show_job(&mut h);
    // Typed, not left yet.
    h.get_by_label("Width (mm)").click();
    h.run_plan();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_plan();
    h.event(Event::Text("2in+".into()));
    h.run_plan();
    h.get_by_label("Passes").click();
    h.run_plan();
    for _ in 0..3 {
        h.run_plan();
    }
    assert!(has_focus(&h, "Passes"), "no focus is taken back");
    assert!(!has_focus(&h, "Width (mm)"));
    assert_eq!(fields(&h).width, "50.00");
    let toast = app(&h).ui.toasts.last().cloned().unwrap();
    assert!(toast.text.ends_with("Nothing changed."), "{}", toast.text);
    // And Passes works as usual.
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_plan();
    h.event(Event::Text("2".into()));
    h.run_plan();
    h.key_press(Key::Enter);
    h.run_plan();
    assert_eq!(passes(&s), Some(2));
}

#[test]
fn a_selection_change_drops_a_bad_text_and_a_good_one_stays_with_its_objects() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = import_square(&s);
    s.lock()
        .unwrap()
        .execute(
            "object.move",
            json!({ "ids": [b], "x_mm": 100.0 }),
            Origin::User,
        )
        .unwrap();
    let select = |id: u64| {
        s.lock()
            .unwrap()
            .execute("object.select", json!({ "ids": [id] }), Origin::Agent)
            .unwrap();
    };
    select(a);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.run_plan();
    // A text it cannot read, kept by Enter; then another selection.
    enter(&mut h, "X (mm)", "abc", Key::Enter);
    assert!(fields(&h).error.is_some());
    let before = undo_len(&s);
    select(b);
    h.run_plan();
    h.run_plan();
    assert!(fields(&h).editing.is_none() && fields(&h).error.is_none());
    assert_eq!(fields(&h).x, "100.00", "it shows the new selection");
    assert!(!has_focus(&h, "X (mm)"));
    let toast = app(&h).ui.toasts.last().cloned().unwrap();
    assert!(toast.text.ends_with("Nothing changed."), "{}", toast.text);
    assert_eq!(undo_len(&s), before);
    // A good text typed for a, then b is selected: it stays with a, and
    // a relative edit counts from a's value.
    let typed_for_a_then_b = |h: &mut AppHarness, text: &str| {
        select(a);
        h.run_plan();
        h.get_by_label("X (mm)").click();
        h.run_plan();
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.run_plan();
        h.event(Event::Text(text.into()));
        h.run_plan();
        select(b);
        h.run_plan();
        h.key_press(Key::Enter);
        h.run_plan();
        h.run_plan();
    };
    typed_for_a_then_b(&mut h, "+=5");
    assert!(near(bounds(&s, a).x0, 25.0), "{:?}", bounds(&s, a));
    assert!(near(bounds(&s, b).x0, 100.0), "{:?}", bounds(&s, b));
    typed_for_a_then_b(&mut h, "40");
    assert!(near(bounds(&s, a).x0, 40.0), "{:?}", bounds(&s, a));
    assert!(near(bounds(&s, b).x0, 100.0), "{:?}", bounds(&s, b));
}

#[test]
fn a_fixed_text_applies_and_the_message_goes() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h) = square(&dir);
    enter(&mut h, "Height (mm)", "10/0", Key::Enter);
    h.get_by_label("Can't use '10/0': you can't divide by 0. Press Esc to go back.");
    assert!(near(bounds(&s, id).height(), 50.0));
    // Still in the field: fix the text and leave it.
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_plan();
    h.event(Event::Text("10/4".into()));
    h.run_plan();
    h.key_press(Key::Enter);
    h.run_plan();
    h.run_plan();
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
    h.get_by_label(
        "Can't read '2in': in does not fit here: use deg or \u{00b0}. Press Esc to go back.",
    );
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
    h.get_by_label("0 is too small: give a value above 0 %. Press Esc to go back.");
}

#[test]
fn a_whole_number_field_rounds_and_says_what_it_cannot_read() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    common::select_none(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let passes = |s: &SharedSession| s.lock().unwrap().doc().document.job.passes;
    h.get_by_label("Passes").click();
    h.run_plan();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_plan();
    h.event(Event::Text("5/2".into()));
    h.run_plan();
    h.key_press(Key::Enter);
    h.run_plan();
    h.run_plan();
    assert_eq!(passes(&s), Some(3), "2.5 rounds to 3");
    h.get_by_label("Passes (modified)").click();
    h.run_plan();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_plan();
    h.event(Event::Text("2*".into()));
    h.run_plan();
    h.key_press(Key::Enter);
    h.run_plan();
    h.run_plan();
    assert_eq!(passes(&s), Some(3), "nothing applied");
    h.get_by_label("Can't read '2*': something is missing after *. Press Esc to go back.");
    // Force refuses a value above its range (only Speed clamps).
    h.key_press(Key::Escape);
    h.run_plan();
    h.get_by_label("Force").click();
    h.run_plan();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_plan();
    h.event(Event::Text("40".into()));
    h.run_plan();
    h.key_press(Key::Tab);
    h.run_plan();
    let force = s.lock().unwrap().doc().document.job.force;
    assert_eq!(force, None, "refused");
    let toast = app(&h).ui.toasts.last().cloned().unwrap();
    assert_eq!(
        toast.text,
        "40 is too large: give at most 33. Nothing changed."
    );
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
    enter(&mut h, "Width (mm)", "(50", Key::Enter);
    assert!(fields(&h).error.is_some());
    h.get_by_role_and_label(Role::Tab, "View").click();
    h.run_plan();
    h.run_plan();
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

#[test]
fn a_job_field_follows_the_click_rules() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _id, mut h) = square(&dir);
    let before = undo_len(&s);
    common::show_job(&mut h);
    common::check_click_rules(&mut h, "Passes");
    assert_eq!(undo_len(&s), before);
}
