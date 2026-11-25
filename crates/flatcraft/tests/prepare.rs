//! The Prepare workspace, driven through egui_kittest.

mod common;

use std::path::Path;

use common::{
    AppHarness, FakePicker, app, app_mut, harness, harness_with, import_square, painted, session,
    to_prepare, undo_len, wait_for_source_check, wait_for_workers, worker_delay,
};
use egui::accesskit::Role;
use egui::{Color32, Event, Modifiers, Pos2, Shape, Vec2};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::canvas::CanvasView;
use flatcraft_core::document::ObjectId;
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

fn state(s: &SharedSession) -> Value {
    s.lock()
        .unwrap()
        .execute("document.state", json!({}), Origin::User)
        .unwrap()
}

fn bounds(s: &SharedSession, id: u64) -> Value {
    let st = state(s);
    st["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .unwrap()["bounds_mm"]
        .clone()
}

/// A view where 1 mm is 2 points and `mm` sits at the canvas centre.
fn known_view(h: &mut AppHarness, mm: kurbo::Point) -> CanvasView {
    let rect = app(h).ui.prepare.canvas_rect;
    let view = CanvasView {
        offset: rect.center().to_vec2() - Vec2::new(mm.x as f32, mm.y as f32) * 2.0,
        px_per_mm: 2.0,
    };
    let p = &mut app_mut(h).ui.prepare;
    p.view = view;
    p.fit_pending = false;
    h.run();
    view
}

/// Presses at `from`, moves in steps to `to` and releases, one frame each.
fn drag(h: &mut AppHarness, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    for t in [0.25, 0.5, 0.75, 1.0] {
        h.hover_at(from + (to - from) * t);
        h.run();
    }
    h.drop_at(to);
    h.run();
}

fn type_into(h: &mut AppHarness, label: &str, text: &str) {
    h.get_by_label(label).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run();
    h.get_by_label(label).type_text(text);
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
}

/// Focuses the window, which starts a source check, and waits until the
/// check is done.
fn focus_window(h: &mut AppHarness) {
    h.event(Event::WindowFocused(false));
    h.run();
    h.event(Event::WindowFocused(true));
    wait_for_workers(h);
}

#[test]
fn canvas_view_roundtrip() {
    let view = CanvasView {
        offset: Vec2::new(123.5, -40.25),
        px_per_mm: 3.7,
    };
    for p in [
        kurbo::Point::new(0.0, 0.0),
        kurbo::Point::new(152.4, 330.2),
        kurbo::Point::new(-12.5, 7.125),
    ] {
        let back = view.screen_to_mm(view.mm_to_screen(p));
        assert!((back - p).hypot() < 1e-4, "{p:?} -> {back:?}");
    }
}

#[test]
fn scale_field_sets_width() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Arrange").click();
    h.run();
    type_into(&mut h, "Width (mm)", "100");
    let b = bounds(&s, id);
    let w = b["x1"].as_f64().unwrap() - b["x0"].as_f64().unwrap();
    assert!((w - 100.0).abs() < 1e-6, "width {w}");
}

#[test]
fn eye_toggle_hides_object() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Button, "Hide square").click();
    h.run();
    assert!(
        !s.lock()
            .unwrap()
            .doc()
            .document
            .get(ObjectId(id))
            .unwrap()
            .visible
    );
    h.get_by_role_and_label(Role::Button, "Show square");
}

#[test]
fn settings_override_marks_modified() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    assert!(h.query_by_label("Speed (modified)").is_none());
    h.get_by_role_and_label(Role::SpinButton, "Speed")
        .set_value_accesskit(3.0);
    h.run();
    assert_eq!(s.lock().unwrap().doc().document.job.speed, Some(3));
    h.get_by_label("Speed (modified)");
    s.lock()
        .unwrap()
        .execute("job.set_overrides", json!({ "force": 9 }), Origin::User)
        .unwrap();
    h.run();
    let undo = undo_len(&s);
    // The reset button puts the material value back, for speed only.
    h.get_by_role_and_label(Role::Button, "Reset speed").click();
    h.run();
    let job = s.lock().unwrap().doc().document.job.clone();
    assert_eq!((job.speed, job.force), (None, Some(9)));
    assert_eq!(undo_len(&s), undo + 1);
    assert!(h.query_by_label("Speed (modified)").is_none());
}

#[test]
fn search_runs_auto_arrange() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let before = undo_len(&s);
    h.get_by_label("Search commands").click();
    h.run();
    h.get_by_label("Search commands").type_text("Auto-arrange");
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(undo_len(&s), before + 1);
}

#[test]
fn drag_move_is_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let start = s
        .lock()
        .unwrap()
        .doc()
        .document
        .get(ObjectId(id))
        .unwrap()
        .transform
        .center_mm;
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let view = known_view(&mut h, start);
    let before = undo_len(&s);
    let from = view.mm_to_screen(start);
    drag(&mut h, from, from + Vec2::new(40.0, 20.0));
    assert_eq!(undo_len(&s), before + 1);
    let end = s
        .lock()
        .unwrap()
        .doc()
        .document
        .get(ObjectId(id))
        .unwrap()
        .transform
        .center_mm;
    let moved = end - start;
    assert!(
        (moved.x - 20.0).abs() < 1e-3 && (moved.y - 10.0).abs() < 1e-3,
        "{moved:?}"
    );
}

#[test]
fn corner_drag_scales_about_the_opposite_corner() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let b = bounds(&s, id);
    let (x0, y0, x1, y1) = (
        b["x0"].as_f64().unwrap(),
        b["y0"].as_f64().unwrap(),
        b["x1"].as_f64().unwrap(),
        b["y1"].as_f64().unwrap(),
    );
    let view = known_view(&mut h, kurbo::Point::new((x0 + x1) / 2.0, (y0 + y1) / 2.0));
    let before = undo_len(&s);
    // Bottom-right handle out by 25 mm along the diagonal: 50 -> 75 mm.
    let from = view.mm_to_screen(kurbo::Point::new(x1, y1));
    drag(&mut h, from, from + Vec2::new(50.0, 50.0));
    assert_eq!(undo_len(&s), before + 1);
    let b = bounds(&s, id);
    let near = |v: &Value, want: f64| (v.as_f64().unwrap() - want).abs() < 1e-3;
    assert!(near(&b["x0"], x0) && near(&b["y0"], y0), "{b}");
    assert!(
        near(&b["x1"], x0 + 75.0) && near(&b["y1"], y0 + 75.0),
        "{b}"
    );
}

#[test]
fn mat_colour_fills_canvas() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let st = state(&s);
    let mat = &st["mat"];
    let ox = mat["grid"]["origin_x_mm"].as_f64().unwrap();
    let oy = mat["grid"]["origin_y_mm"].as_f64().unwrap();
    let centre = kurbo::Point::new(
        mat["width_mm"].as_f64().unwrap() / 2.0 - ox,
        mat["height_mm"].as_f64().unwrap() / 2.0 - oy,
    );
    let at = app(&h).ui.prepare.view.mm_to_screen(centre);
    let colour = Color32::from_rgb(0x8F, 0xA8, 0xB8);
    assert_eq!(mat["color"], "#8FA8B8");
    let hit = painted(&h).iter().any(|shape| match shape {
        Shape::Rect(r) => r.fill == colour && r.rect.contains(at),
        _ => false,
    });
    assert!(hit, "no mat-coloured rect at {at:?}");
}

#[test]
fn uncalibrated_pill_only_for_mats() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_label("This mat is not calibrated yet");
    s.lock()
        .unwrap()
        .execute("job.set_media", json!({ "id": "no-mat" }), Origin::User)
        .unwrap();
    h.run();
    assert!(h.query_by_label("This mat is not calibrated yet").is_none());
}

/// An SVG file with one `size` mm square at 10,10.
fn write_square(path: &Path, size: f64) {
    std::fs::write(
        path,
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="10" y="10" width="{size}" height="{size}"/></svg>"#
        ),
    )
    .unwrap();
}

fn import_file(s: &SharedSession, path: &Path) -> u64 {
    s.lock()
        .unwrap()
        .execute("object.import", json!({ "path": path }), Origin::User)
        .unwrap()["id"]
        .as_u64()
        .unwrap()
}

fn width(s: &SharedSession, id: u64) -> f64 {
    let b = bounds(s, id);
    b["x1"].as_f64().unwrap() - b["x0"].as_f64().unwrap()
}

#[test]
fn changed_source_shows_reload_pill_and_reload_runs() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let file = dir.path().join("square.svg");
    write_square(&file, 30.0);
    let id = import_file(&s, &file);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    assert!(h.query_by_label("1 source file changed").is_none());
    write_square(&file, 45.5);
    focus_window(&mut h);
    h.get_by_label("1 source file changed");
    h.get_by_label("Source file changed");
    let before = undo_len(&s);
    h.get_by_role_and_label(Role::Button, "Reload changed files")
        .click();
    h.run();
    assert!((width(&s, id) - 45.5).abs() < 1e-6, "{}", width(&s, id));
    assert_eq!(undo_len(&s), before + 1);
    assert!(h.query_by_label("1 source file changed").is_none());
}

#[test]
fn missing_source_shows_reconnect_and_relink_works() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let old = dir.path().join("old");
    std::fs::create_dir(&old).unwrap();
    write_square(&old.join("a.svg"), 20.0);
    write_square(&old.join("b.svg"), 25.0);
    write_square(&old.join("c.svg"), 30.0);
    let a = import_file(&s, &old.join("a.svg"));
    let b = import_file(&s, &old.join("b.svg"));
    let c = import_file(&s, &old.join("c.svg"));
    s.lock()
        .unwrap()
        .execute("object.select", json!({ "ids": [a] }), Origin::User)
        .unwrap();
    let new = dir.path().join("new");
    std::fs::rename(&old, &new).unwrap();

    let picker = FakePicker::answering([new.join("a.svg")]);
    let p = picker.clone();
    let mut h = harness_with(&dir, s.clone(), move |app| app.with_file_picker(p));
    to_prepare(&mut h);
    focus_window(&mut h);
    h.get_by_label("3 source files not found");
    assert_eq!(h.query_all_by_label("Source file not found").count(), 3);
    // The details of the selected object show its last known path.
    h.get_by_label(&old.join("a.svg").display().to_string());
    h.get_by_role_and_label(Role::Button, "Reconnect...")
        .click();
    h.run();
    assert_eq!(picker.requests.lock().unwrap().len(), 1);
    let link = |id: u64| {
        s.lock()
            .unwrap()
            .doc()
            .document
            .get(ObjectId(id))
            .unwrap()
            .link
            .as_ref()
            .unwrap()
            .path
            .clone()
    };
    assert_eq!(link(a), new.join("a.svg"));
    // b's and c's files are in the same new folder: one click, one undo
    // step reconnects them too.
    h.get_by_label("2 more files found in that folder");
    let undo = undo_len(&s);
    h.get_by_role_and_label(Role::Button, "Reconnect them too")
        .click();
    h.run();
    assert_eq!(link(b), new.join("b.svg"));
    assert_eq!(link(c), new.join("c.svg"));
    assert_eq!(undo_len(&s), undo + 1);
    assert!(h.query_by_label_contains("not found").is_none());
}

#[test]
fn unlink_forgets_the_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let file = dir.path().join("gone.svg");
    write_square(&file, 20.0);
    let id = import_file(&s, &file);
    std::fs::remove_file(&file).unwrap();
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    focus_window(&mut h);
    h.get_by_label("1 source file not found");
    h.get_by_role_and_label(Role::Button, "Unlink file").click();
    h.run();
    let st = s.lock().unwrap();
    assert!(st.doc().document.get(ObjectId(id)).unwrap().link.is_none());
    drop(st);
    assert!(h.query_by_label("1 source file not found").is_none());
}

#[test]
fn auto_reload_reloads_only_changed() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    s.lock().unwrap().prefs.auto_reload = true;
    let changed = dir.path().join("changed.svg");
    let gone = dir.path().join("gone.svg");
    write_square(&changed, 20.0);
    write_square(&gone, 20.0);
    let c = import_file(&s, &changed);
    let g = import_file(&s, &gone);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    write_square(&changed, 33.0);
    std::fs::remove_file(&gone).unwrap();
    focus_window(&mut h);
    assert!((width(&s, c) - 33.0).abs() < 1e-6);
    assert_eq!(app(&h).ui.prepare.sources.auto_reloaded, [c]);
    assert!(h.query_by_label("1 source file changed").is_none());
    h.get_by_label("1 source file not found");
    assert_eq!(app(&h).ui.prepare.sources.missing, [g]);
}

#[test]
fn source_checks_are_debounced() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let file = dir.path().join("square.svg");
    write_square(&file, 30.0);
    import_file(&s, &file);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    focus_window(&mut h);
    assert!(h.query_by_label("1 source file changed").is_none());
    // A second focus within 2 s does not check again.
    write_square(&file, 45.5);
    focus_window(&mut h);
    assert!(h.query_by_label("1 source file changed").is_none());
    assert!(app(&h).ui.prepare.sources.check_pending);
}

#[test]
fn objects_panel_click_selects_and_shift_click_adds() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let sel = |s: &SharedSession| -> Vec<u64> {
        s.lock()
            .unwrap()
            .doc()
            .selection
            .iter()
            .map(|i| i.0)
            .collect()
    };
    let rows = h.get_all_by_role_and_label(Role::Button, "square");
    let rects: Vec<_> = rows.map(|n| n.rect()).collect();
    assert_eq!(rects.len(), 2);
    h.get_all_by_role_and_label(Role::Button, "square")
        .next()
        .unwrap()
        .click();
    h.run();
    assert_eq!(sel(&s), [a]);
    h.get_all_by_role_and_label(Role::Button, "square")
        .nth(1)
        .unwrap()
        .click_modifiers(Modifiers::SHIFT);
    h.run();
    assert_eq!(sel(&s), [a, b]);
}

#[test]
fn out_of_area_pill_counts_objects() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    assert!(
        h.query_by_label_contains("outside the cutting area")
            .is_none()
    );
    s.lock()
        .unwrap()
        .execute(
            "object.move",
            json!({ "ids": [id], "dx_mm": 500.0 }),
            Origin::User,
        )
        .unwrap();
    h.run();
    h.get_by_label("1 object is outside the cutting area");
}

#[test]
fn prepare_is_accessible() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.check_accessibility();
}

#[test]
fn commands_skip_a_document_that_is_no_longer_active() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let chosen = s.lock().unwrap().doc().id();
    // An agent opens a document between the drag and its command.
    s.lock()
        .unwrap()
        .execute("document.new", json!({}), Origin::Agent)
        .unwrap();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(
        &ctx,
        vec![flatcraft::ui::Action::run(
            chosen,
            "object.move",
            json!({ "ids": [id], "dx_mm": 5.0 }),
        )],
    );
    let st = s.lock().unwrap();
    assert!(st.docs[1].document.objects.is_empty());
    assert_eq!(st.docs[0].undo.undo_len(), 1, "only the import");
    drop(st);
    assert_eq!(app(&h).ui.toasts.len(), 1);
}

#[test]
fn typed_speed_is_one_command() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let before = undo_len(&s);
    h.get_by_role_and_label(Role::SpinButton, "Speed").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run();
    for c in ["1", "0"] {
        h.event(Event::Text(c.into()));
        h.run();
    }
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(s.lock().unwrap().doc().document.job.speed, Some(10));
    assert_eq!(undo_len(&s), before + 1);
}

#[test]
fn clicking_a_search_hit_runs_it() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_label("Search commands").click();
    h.run();
    h.get_by_label("Search commands").type_text("dupl");
    h.run();
    let before = s.lock().unwrap().doc().document.objects.len();
    h.get_by_role_and_label(Role::Button, "Duplicate (search)")
        .click();
    h.run();
    assert_eq!(s.lock().unwrap().doc().document.objects.len(), before + 1);
}

#[test]
fn trace_button_opens_the_dialog_and_apply_retraces() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut px = tiny_skia::Pixmap::new(200, 200).unwrap();
    px.fill(tiny_skia::Color::WHITE);
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::BLACK);
    px.fill_rect(
        tiny_skia::Rect::from_xywh(50.0, 50.0, 100.0, 100.0).unwrap(),
        &paint,
        tiny_skia::Transform::identity(),
        None,
    );
    let png = dir.path().join("block.png");
    px.save_png(&png).unwrap();
    import_file(&s, &png);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let before = undo_len(&s);
    // Trace opens the dialog; Apply is one retrace.
    h.get_by_role_and_label(Role::Button, "Trace").click();
    h.run_steps(3);
    assert!(app(&h).ui.trace.is_some());
    assert_eq!(undo_len(&s), before);
    // Settings that change the result: an Apply that changes nothing
    // records no undo step.
    h.get_by_label("Invert").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Apply").click();
    h.run_steps(3);
    assert!(app(&h).ui.toasts.is_empty(), "{:?}", app(&h).ui.toasts);
    assert_eq!(undo_len(&s), before + 1);
}

// ---------------------------------------------------------------- review round 1

fn object_centre(s: &SharedSession, id: u64) -> kurbo::Point {
    s.lock()
        .unwrap()
        .doc()
        .document
        .get(ObjectId(id))
        .unwrap()
        .transform
        .center_mm
}

#[test]
fn drag_is_cancelled_when_the_document_changes() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let start = object_centre(&s, id);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let view = known_view(&mut h, start);
    let before_undo = undo_len(&s);
    let from = view.mm_to_screen(start);
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    h.hover_at(from + Vec2::new(10.0, 0.0));
    h.run();
    // Mid-drag an agent opens a document with an object of the same id.
    let b_id = {
        s.lock()
            .unwrap()
            .execute("document.new", json!({}), Origin::Agent)
            .unwrap();
        import_square(&s)
    };
    assert_eq!(b_id, id, "the same object id in the new document");
    let b_start = object_centre(&s, b_id);
    let b_undo = undo_len(&s);
    for t in [20.0, 40.0] {
        h.hover_at(from + Vec2::new(t, 20.0));
        // The new document starts a source check.
        wait_for_source_check(&mut h);
    }
    h.drop_at(from + Vec2::new(40.0, 20.0));
    h.run();
    assert_eq!(object_centre(&s, b_id), b_start, "the new document moved");
    assert_eq!(undo_len(&s), b_undo);
    let st = s.lock().unwrap();
    let a = &st.docs[0];
    assert_eq!(
        a.document.get(ObjectId(id)).unwrap().transform.center_mm,
        start
    );
    assert_eq!(a.undo.undo_len(), before_undo);
}

#[test]
fn switching_back_restores_link_status() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let file = dir.path().join("gone.svg");
    write_square(&file, 20.0);
    import_file(&s, &file);
    std::fs::remove_file(&file).unwrap();
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    focus_window(&mut h);
    h.get_by_label("1 source file not found");
    h.get_by_role_and_label(Role::Button, "New document")
        .click();
    wait_for_source_check(&mut h);
    assert!(h.query_by_label("1 source file not found").is_none());
    h.get_by_role_and_label(Role::Button, "Untitled 1 *")
        .click();
    wait_for_workers(&mut h);
    h.get_by_label("1 source file not found");
    h.get_by_label("Source file not found");
}

#[test]
fn agent_switching_in_a_loop_does_not_check_each_time() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let file = dir.path().join("gone.svg");
    write_square(&file, 20.0);
    import_file(&s, &file);
    std::fs::remove_file(&file).unwrap();
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    focus_window(&mut h);
    h.get_by_label("1 source file not found");
    let a = s.lock().unwrap().doc().id();
    s.lock()
        .unwrap()
        .execute("document.new", json!({}), Origin::Agent)
        .unwrap();
    // The new document is checked once.
    wait_for_workers(&mut h);
    let first = app(&h).ui.prepare.sources.last_check;
    assert!(first.is_some());
    for _ in 0..3 {
        s.lock()
            .unwrap()
            .execute("document.switch", json!({ "doc_id": a }), Origin::Agent)
            .unwrap();
        h.run();
        // The status of the document comes back without a new check.
        h.get_by_label("1 source file not found");
        s.lock()
            .unwrap()
            .execute("document.switch", json!({ "index": 1 }), Origin::Agent)
            .unwrap();
        h.run();
    }
    assert_eq!(
        app(&h).ui.prepare.sources.last_check,
        first,
        "switching within the check interval reuses the last result"
    );
}

#[test]
fn shapes_survive_a_document_switch() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let doc = s.lock().unwrap().doc().id();
    let shape = app(&h).ui.prepare.cache.shape(doc, id).unwrap();
    s.lock()
        .unwrap()
        .execute("document.new", json!({}), Origin::User)
        .unwrap();
    wait_for_source_check(&mut h);
    s.lock()
        .unwrap()
        .execute("document.switch", json!({ "doc_id": doc }), Origin::User)
        .unwrap();
    wait_for_source_check(&mut h);
    let again = app(&h).ui.prepare.cache.shape(doc, id).unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&shape, &again),
        "the shape was not rebuilt"
    );
}

/// An SVG path with `n` short segments.
fn zigzag_svg(n: usize) -> String {
    let mut d = String::from("M 0 0");
    for i in 0..n {
        d.push_str(&format!(" L {} {}", i as f64 * 0.01, (i % 2) as f64));
    }
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="10mm" viewBox="0 0 100 10"><path d="{d}" fill="none" stroke="black" stroke-width="0.1"/></svg>"#
    )
}

#[test]
fn huge_shapes_are_built_off_the_ui_thread() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = s
        .lock()
        .unwrap()
        .execute(
            "object.import_svg",
            json!({ "svg": zigzag_svg(9000), "name": "zigzag" }),
            Origin::User,
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let mut h = harness(&dir, s.clone(), false);
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let gate = std::sync::Mutex::new(gate);
    app_mut(&mut h).ui.prepare.cache.set_builder(move |input| {
        let _ = gate.lock().unwrap().recv();
        flatcraft::ui::prepare::build_shape(input)
    });
    to_prepare(&mut h);
    let doc = s.lock().unwrap().doc().id();
    // The frame finished although the shape is still being built.
    assert_eq!(app(&h).ui.prepare.cache.building(), 1);
    assert!(app(&h).ui.prepare.cache.shape(doc, id).is_none());
    release.send(()).unwrap();
    wait_built(&mut h, doc, id);
    assert_eq!(app(&h).ui.prepare.cache.building(), 0);
    let shape = app(&h).ui.prepare.cache.shape(doc, id).unwrap();
    assert!(!shape.lines.is_empty());
}

#[test]
fn a_missing_mat_still_shows_a_cutting_area() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    s.lock().unwrap().doc_mut().document.job.media_id = "gone".into();
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_label_contains("The mat 'gone' is not available");
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &egui::Context::default());
    assert_eq!(snap.mat.as_ref().map(|m| m.id.as_str()), Some("mat-12x12"));
    assert!(snap.area.is_some());
}

#[test]
fn corner_drag_scales_a_multi_selection_as_a_group() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = import_square(&s);
    {
        let mut g = s.lock().unwrap();
        g.execute(
            "object.move",
            json!({ "ids": [b], "dx_mm": 60.0, "dy_mm": 0.0 }),
            Origin::User,
        )
        .unwrap();
        g.execute("object.select", json!({ "ids": [a, b] }), Origin::User)
            .unwrap();
    }
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let f = |v: &Value, k: &str| v[k].as_f64().unwrap();
    let (ba, bb) = (bounds(&s, a), bounds(&s, b));
    let (x0, y0) = (f(&ba, "x0"), f(&ba, "y0"));
    let (x1, y1) = (f(&bb, "x1"), f(&bb, "y1"));
    let view = known_view(&mut h, kurbo::Point::new((x0 + x1) / 2.0, (y0 + y1) / 2.0));
    let before = undo_len(&s);
    // The bottom-right handle of the group box, out to twice its size.
    let from = view.mm_to_screen(kurbo::Point::new(x1, y1));
    let to = view.mm_to_screen(kurbo::Point::new(
        x0 + 2.0 * (x1 - x0),
        y0 + 2.0 * (y1 - y0),
    ));
    drag(&mut h, from, to);
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    let near = |got: f64, want: f64| (got - want).abs() < 1e-3;
    let (na, nb) = (bounds(&s, a), bounds(&s, b));
    assert!(near(f(&na, "x0"), x0) && near(f(&na, "y0"), y0), "{na}");
    // b keeps its place in the layout: twice as far from the anchor.
    assert!(near(f(&nb, "x0"), x0 + 2.0 * (f(&bb, "x0") - x0)), "{nb}");
    assert!(near(f(&nb, "x1"), x0 + 2.0 * (x1 - x0)), "{nb}");
}

#[test]
fn settings_rows_have_one_accessible_node_each() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    for label in [
        "Cut mode", "Strokes", "Plotter", "Mat", "Material", "Speed", "Force", "Passes",
    ] {
        assert_eq!(h.query_all_by_label(label).count(), 1, "{label}");
    }
}

#[test]
fn source_checks_hash_on_a_worker_thread() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let file = dir.path().join("square.svg");
    write_square(&file, 30.0);
    import_file(&s, &file);
    let ui_thread = std::thread::current().id();
    let threads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = threads.clone();
    let mut h = harness_with(&dir, s.clone(), move |app| {
        app.with_source_check(move |session| {
            let me = std::thread::current();
            seen.lock()
                .unwrap()
                .push((me.id(), me.name().map(str::to_string)));
            std::thread::sleep(worker_delay());
            flatcraft_engine::commands::sources::check_unlocked(session)
        })
    });
    to_prepare(&mut h);
    write_square(&file, 45.5);
    focus_window(&mut h);
    h.get_by_label("1 source file changed");
    let threads = threads.lock().unwrap();
    assert!(!threads.is_empty());
    assert!(
        threads.iter().all(|(t, _)| *t != ui_thread),
        "hashing ran on the UI thread"
    );
    assert!(
        threads
            .iter()
            .all(|(_, name)| name.as_deref() == Some("source-check")),
        "hashing ran on the source-check worker: {threads:?}"
    );
}

fn zigzag(s: &SharedSession) -> u64 {
    s.lock()
        .unwrap()
        .execute(
            "object.import_svg",
            json!({ "svg": zigzag_svg(9000), "name": "zigzag" }),
            Origin::User,
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap()
}

fn wait_built(h: &mut AppHarness, doc: u64, id: u64) {
    wait_for_workers(h);
    assert!(
        app(h).ui.prepare.cache.shape(doc, id).is_some(),
        "never built"
    );
}

#[test]
fn the_shape_worker_builds_only_the_newest_shape() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = zigzag(&s);
    let mut h = harness(&dir, s.clone(), false);
    let builds = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let gate = std::sync::Mutex::new(gate);
    let count = builds.clone();
    // The builder says when it is inside the first build, so the scales
    // below cannot overtake the worker picking that build up.
    let (started_tx, started) = std::sync::mpsc::channel::<()>();
    let started_tx = std::sync::Mutex::new(started_tx);
    app_mut(&mut h).ui.prepare.cache.set_builder(move |input| {
        let _ = started_tx.lock().unwrap().send(());
        let _ = gate.lock().unwrap().recv();
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        flatcraft::ui::prepare::build_shape(input)
    });
    to_prepare(&mut h);
    let doc = s.lock().unwrap().doc().id();
    // Two more scales while the first build waits inside the builder.
    started
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the worker never started the first build");
    for f in [1.5, 1.2] {
        s.lock()
            .unwrap()
            .execute(
                "object.scale",
                json!({ "ids": [id], "factor": f }),
                Origin::User,
            )
            .unwrap();
        h.run();
    }
    for _ in 0..3 {
        let _ = release.send(());
    }
    wait_built(&mut h, doc, id);
    assert_eq!(
        builds.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the first shape and the newest one, not the one in between"
    );
}

#[test]
fn a_dead_shape_worker_falls_back_to_building_inline() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = zigzag(&s);
    let mut h = harness(&dir, s.clone(), false);
    app_mut(&mut h)
        .ui
        .prepare
        .cache
        .set_builder(|_| panic!("the worker dies"));
    to_prepare(&mut h);
    let doc = s.lock().unwrap().doc().id();
    let start = std::time::Instant::now();
    loop {
        s.lock()
            .unwrap()
            .execute(
                "object.scale",
                json!({ "ids": [id], "factor": 1.01 }),
                Origin::User,
            )
            .unwrap();
        h.run();
        if app(&h).ui.prepare.cache.shape(doc, id).is_some() {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "never built"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(app(&h).ui.prepare.cache.building(), 0);
}

#[test]
fn the_mat_combo_shows_the_fallback_mat() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    s.lock().unwrap().doc_mut().document.job.media_id = "gone".into();
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_label("Mat").click();
    h.run();
    let item = h.get_by_label("Mat 12x12 (fallback)");
    let n = item.accesskit_node();
    // egui reports a selectable label's state as toggled.
    assert_eq!(n.toggled(), Some(egui::accesskit::Toggled::True));
    // Picking it makes it the document's own mat.
    h.get_by_label("Mat 12x12 (fallback)").click();
    h.run();
    assert_eq!(s.lock().unwrap().doc().document.job.media_id, "mat-12x12");
    assert!(h.query_by_label_contains("is not available").is_none());
}

#[test]
fn pills_stay_clear_of_the_cutting_area_corner() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let a = &state(&s)["cutting_area_mm"];
    let (x0, y0) = (a["x0"].as_f64().unwrap(), a["y0"].as_f64().unwrap());
    let corner =
        app(&h)
            .ui
            .prepare
            .view
            .rect_to_screen(kurbo::Rect::new(x0, y0, x0 + 50.0, y0 + 50.0));
    let pill = h.get_by_label("This mat is not calibrated yet").rect();
    assert!(!pill.intersects(corner), "{pill:?} covers {corner:?}");
    let canvas = app(&h).ui.prepare.canvas_rect;
    assert!(canvas.contains_rect(pill));
    // Stacked from the bottom-left.
    assert!(pill.min.y > canvas.center().y, "{pill:?} in {canvas:?}");
    assert!(pill.min.x < canvas.center().x, "{pill:?} in {canvas:?}");
}

fn wheel(unit: egui::MouseWheelUnit, source: egui::MouseWheelSource, delta: Vec2) -> Event {
    Event::MouseWheel {
        unit,
        delta,
        phase: egui::TouchPhase::Move,
        source,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn trackpad_scroll_pans_and_wheel_zooms() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let view = known_view(&mut h, kurbo::Point::new(100.0, 100.0));
    let at = app(&h).ui.prepare.canvas_rect.center() + Vec2::new(30.0, 40.0);
    h.hover_at(at);
    h.run();
    // Two fingers on a trackpad: pan, no zoom.
    h.event(wheel(
        egui::MouseWheelUnit::Point,
        egui::MouseWheelSource::Trackpad,
        Vec2::new(30.0, -12.0),
    ));
    h.run();
    let v = app(&h).ui.prepare.view;
    assert_eq!(v.px_per_mm, view.px_per_mm);
    assert!((v.offset - view.offset - Vec2::new(30.0, -12.0)).length() < 1e-3);
    // A mouse wheel notch: zoom around the pointer.
    let mm = v.screen_to_mm(at);
    h.event(wheel(
        egui::MouseWheelUnit::Line,
        egui::MouseWheelSource::Wheel,
        Vec2::new(0.0, 1.0),
    ));
    h.run();
    let w = app(&h).ui.prepare.view;
    assert!(w.px_per_mm > v.px_per_mm, "{w:?}");
    assert!((w.screen_to_mm(at) - mm).hypot() < 1e-3);
}

#[test]
fn pinch_and_cmd_scroll_zoom_around_the_pointer() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let view = known_view(&mut h, kurbo::Point::new(100.0, 100.0));
    let at = app(&h).ui.prepare.canvas_rect.center() + Vec2::new(-20.0, 10.0);
    h.hover_at(at);
    h.run();
    let mm = view.screen_to_mm(at);
    h.event(Event::Zoom(1.5));
    h.run();
    let v = app(&h).ui.prepare.view;
    assert!((v.px_per_mm - view.px_per_mm * 1.5).abs() < 1e-4, "{v:?}");
    assert!((v.screen_to_mm(at) - mm).hypot() < 1e-3);
    h.event_modifiers(
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, 40.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Trackpad,
            modifiers: Modifiers::COMMAND,
        },
        Modifiers::COMMAND,
    );
    for _ in 0..5 {
        h.run();
    }
    let w = app(&h).ui.prepare.view;
    assert!(w.px_per_mm > v.px_per_mm, "{w:?}");
    assert!((w.screen_to_mm(at) - mm).hypot() < 1e-2);
}

#[test]
fn search_reload_uses_the_selection() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let fa = dir.path().join("a.svg");
    let fb = dir.path().join("b.svg");
    write_square(&fa, 20.0);
    write_square(&fb, 20.0);
    let a = import_file(&s, &fa);
    let b = import_file(&s, &fb);
    s.lock()
        .unwrap()
        .execute("object.select", json!({ "ids": [a] }), Origin::User)
        .unwrap();
    write_square(&fa, 30.0);
    write_square(&fb, 30.0);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_label("Search commands").click();
    h.run();
    h.get_by_label("Search commands").type_text("Reload");
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    assert!((width(&s, a) - 30.0).abs() < 1e-4);
    assert!((width(&s, b) - 20.0).abs() < 1e-4, "b was reloaded too");
}

#[test]
fn keep_aspect_is_fixed_for_several_objects() {
    use egui_kittest::kittest::NodeT as _;
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Arrange").click();
    h.run();
    app_mut(&mut h).ui.prepare.fields.keep_aspect = false;
    s.lock()
        .unwrap()
        .execute("object.select", json!({ "ids": [a, b] }), Origin::User)
        .unwrap();
    h.run();
    let node = h.get_by_role_and_label(Role::CheckBox, "Keep aspect");
    let n = node.accesskit_node();
    assert!(n.is_disabled());
    assert_eq!(n.toggled(), Some(egui::accesskit::Toggled::True));
}

fn object_paths(h: &AppHarness, screen: egui::Rect) -> Vec<egui::epaint::PathShape> {
    painted(h)
        .into_iter()
        .filter_map(|s| match s {
            Shape::Path(p) => Some(p),
            _ => None,
        })
        .filter(|p| {
            let r = egui::Rect::from_points(&p.points);
            (r.min - screen.min).length() < 2.0 && (r.max - screen.max).length() < 2.0
        })
        .collect()
}

fn screen_bounds(h: &AppHarness, s: &SharedSession, id: u64) -> egui::Rect {
    let b = bounds(s, id);
    let r = kurbo::Rect::new(
        b["x0"].as_f64().unwrap(),
        b["y0"].as_f64().unwrap(),
        b["x1"].as_f64().unwrap(),
        b["y1"].as_f64().unwrap(),
    );
    app(h).ui.prepare.view.rect_to_screen(r)
}

#[test]
fn hidden_objects_are_not_drawn() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let r = screen_bounds(&h, &s, id);
    assert_eq!(object_paths(&h, r).len(), 1);
    s.lock()
        .unwrap()
        .execute(
            "object.set_visible",
            json!({ "ids": [id], "visible": false }),
            Origin::User,
        )
        .unwrap();
    h.run();
    assert!(object_paths(&h, r).is_empty());
}

/// A plotter "test" with a calibrated dark mat whose grid starts 10,20 mm
/// into the physical mat.
fn dark_mat_session(dir: &tempfile::TempDir) -> SharedSession {
    let plotters = dir.path().join("profiles").join("plotters");
    std::fs::create_dir_all(&plotters).unwrap();
    std::fs::write(
        plotters.join("test.toml"),
        r##"id = "test"
name = "Test"
inherits = "file-output-gpgl-cameo2"
media = [
  { id = "dark", name = "Dark", kind = "Mat", color = "#202020", width_mm = 320.0, height_mm = 340.0, calibrated = true, grid = { origin_x_mm = 10.0, origin_y_mm = 20.0, width_mm = 300.0, height_mm = 300.0 } },
]
"##,
    )
    .unwrap();
    let s = session(dir);
    {
        let mut st = s.lock().unwrap();
        st.execute("job.set_plotter", json!({ "id": "test" }), Origin::User)
            .unwrap();
        st.execute("job.set_media", json!({ "id": "dark" }), Origin::User)
            .unwrap();
    }
    s
}

#[test]
fn rulers_start_at_the_grid_origin() {
    let dir = tempfile::tempdir().unwrap();
    let s = dark_mat_session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let view = app(&h).ui.prepare.view;
    let rect = app(&h).ui.prepare.canvas_rect;
    let zero = view.mm_to_screen(kurbo::Point::ORIGIN);
    let mat_left = view.mm_to_screen(kurbo::Point::new(-10.0, -20.0));
    assert!((zero.x - mat_left.x).abs() > 5.0);
    let texts: Vec<_> = painted(&h)
        .into_iter()
        .filter_map(|s| match s {
            Shape::Text(t) => Some(t),
            _ => None,
        })
        .collect();
    let top_zero = texts.iter().any(|t| {
        t.galley.text() == "0"
            && (t.pos.x - (zero.x + 2.0)).abs() < 0.5
            && t.pos.y < rect.min.y + 18.0
    });
    let left_zero = texts.iter().any(|t| {
        t.galley.text() == "0"
            && (t.pos.y - (zero.y + 2.0)).abs() < 0.5
            && t.pos.x < rect.min.x + 18.0
    });
    assert!(top_zero && left_zero, "no 0 label at the grid origin");
}

#[test]
fn objects_contrast_with_a_dark_mat() {
    let dir = tempfile::tempdir().unwrap();
    let s = dark_mat_session(&dir);
    let id = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let r = screen_bounds(&h, &s, id);
    let paths = object_paths(&h, r);
    assert_eq!(paths.len(), 1);
    let egui::epaint::ColorMode::Solid(c) = paths[0].stroke.color else {
        panic!("not a solid stroke");
    };
    let mat = [0x20, 0x20, 0x20];
    assert_ne!([c.r(), c.g(), c.b()], mat);
    assert!(flatcraft::ui::canvas::contrast_ratio([c.r(), c.g(), c.b()], mat) >= 3.0);
}

#[test]
fn no_source_check_during_a_drag() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let file = dir.path().join("gone.svg");
    write_square(&file, 20.0);
    let id = import_file(&s, &file);
    std::fs::remove_file(&file).unwrap();
    let start = object_centre(&s, id);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let view = known_view(&mut h, start);
    let from = view.mm_to_screen(start);
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    h.hover_at(from + Vec2::new(20.0, 0.0));
    h.run();
    h.event(Event::WindowFocused(true));
    h.hover_at(from + Vec2::new(30.0, 0.0));
    h.run();
    assert!(app(&h).ui.prepare.drag.is_some(), "the drag is still on");
    assert!(app(&h).ui.prepare.sources.missing.is_empty());
    assert!(app(&h).ui.prepare.sources.check_pending);
    h.drop_at(from + Vec2::new(30.0, 0.0));
    wait_for_workers(&mut h);
    assert_eq!(app(&h).ui.prepare.sources.missing, [id]);
}

// ---------------------------------------------------------------- distribute and copies

fn select(s: &SharedSession, ids: &[u64]) {
    s.lock()
        .unwrap()
        .execute("object.select", json!({ "ids": ids }), Origin::User)
        .unwrap();
}

/// Imports the square and moves it right by `dx` mm.
fn square_at(s: &SharedSession, dx: f64) -> u64 {
    let id = import_square(s);
    s.lock()
        .unwrap()
        .execute(
            "object.move",
            json!({ "ids": [id], "dx_mm": dx }),
            Origin::User,
        )
        .unwrap();
    id
}

fn disabled(h: &AppHarness, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

#[test]
fn distribute_buttons_need_three_objects_and_are_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = square_at(&s, 0.0);
    let b = square_at(&s, 60.0);
    let c = square_at(&s, 200.0);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Arrange").click();
    h.run();
    h.get_by_label("Distribute");
    select(&s, &[a, b]);
    h.run();
    assert!(disabled(&h, "Distribute horizontally"));
    assert!(disabled(&h, "Distribute vertically"));
    select(&s, &[a, b, c]);
    h.run();
    assert!(!disabled(&h, "Distribute vertically"));
    let before = undo_len(&s);
    h.get_by_label("Distribute horizontally").click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), before + 1);
    let x = |id| bounds(&s, id)["x0"].as_f64().unwrap();
    let x1 = |id| bounds(&s, id)["x1"].as_f64().unwrap();
    assert!(((x(b) - x1(a)) - (x(c) - x1(b))).abs() < 1e-9);
}

#[test]
fn copies_dialog_makes_copies_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    select(&s, &[a, b]);
    h.run();
    assert!(disabled(&h, "Copies..."), "one object at a time");
    select(&s, &[a]);
    h.run();
    h.get_by_role_and_label(Role::Button, "Copies...").click();
    h.run();
    assert!(app(&h).ui.prepare.copies.is_some());
    assert!(!disabled(&h, "Linked copies"));
    assert!(disabled(&h, "Columns"), "only for a grid");
    h.get_by_label("Grid").click();
    h.run();
    assert!(!disabled(&h, "Columns"));
    h.get_by_role_and_label(Role::SpinButton, "Number of copies")
        .click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run();
    h.event(Event::Text("7".into()));
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    let before = undo_len(&s);
    h.get_by_role_and_label(Role::Button, "Make copies").click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(s.lock().unwrap().doc().document.objects.len(), 2 + 7);
    assert!(app(&h).ui.prepare.copies.is_none(), "the dialog closes");
    // Default 4 columns: the original is cell 0, so copy 4 starts row 2.
    let copies: Vec<u64> = s
        .lock()
        .unwrap()
        .doc()
        .selection
        .iter()
        .map(|i| i.0)
        .collect();
    let (o, c4) = (bounds(&s, a), bounds(&s, copies[3]));
    let f = |v: &Value, k: &str| v[k].as_f64().unwrap();
    assert!((f(&c4, "x0") - f(&o, "x0")).abs() < 1e-9, "{o} {c4}");
    assert!(
        (f(&c4, "y0") - (f(&o, "y1") + 5.0)).abs() < 1e-9,
        "{o} {c4}"
    );
}

#[test]
fn copies_dialog_remembers_the_last_settings() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Button, "Copies...").click();
    h.run();
    {
        let d = app_mut(&mut h).ui.prepare.copies.as_mut().unwrap();
        d.count = 9;
        d.grid = true;
        d.columns = 2;
        d.gap_mm = 1.5;
    }
    h.get_by_role_and_label(Role::Button, "Make copies").click();
    h.run();
    wait_for_workers(&mut h);
    select(&s, &[a]);
    h.run();
    h.get_by_role_and_label(Role::Button, "Copies...").click();
    h.run();
    let d = app(&h).ui.prepare.copies.clone().unwrap();
    assert_eq!((d.count, d.grid, d.columns, d.gap_mm), (9, true, 2, 1.5));
    // Cancel keeps the last settings that were used, not the edits.
    app_mut(&mut h).ui.prepare.copies.as_mut().unwrap().count = 2;
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Copies...").click();
    h.run();
    assert_eq!(app(&h).ui.prepare.copies.as_ref().unwrap().count, 9);
}

#[test]
fn copies_dialog_cancel_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Button, "Copies...").click();
    h.run();
    let before = undo_len(&s);
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run();
    assert!(app(&h).ui.prepare.copies.is_none());
    assert_eq!(undo_len(&s), before);
}

#[test]
fn search_finds_copies_and_distribute() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    // Copies last: its dialog is modal.
    for (query, hit) in [
        ("distrib", "Distribute (search)"),
        ("copies", "Copies (search)"),
    ] {
        h.get_by_label("Search commands").click();
        h.run();
        h.get_by_label("Search commands").type_text(query);
        h.run();
        h.get_by_role_and_label(Role::Button, hit).click();
        h.run();
    }
    assert!(
        app(&h).ui.prepare.copies.is_some(),
        "copies opens the dialog"
    );
    assert_eq!(
        app(&h).ui.prepare.tab,
        flatcraft::ui::prepare::RibbonTab::Arrange
    );
}

fn link_group(s: &SharedSession, id: u64) -> Option<u64> {
    s.lock()
        .unwrap()
        .doc()
        .document
        .get(ObjectId(id))
        .unwrap()
        .link_group
        .map(|g| g.0)
}

/// `count` linked copies of `id`; the original first, then the copies.
fn linked_copies(s: &SharedSession, id: u64, count: u32) -> Vec<u64> {
    let r = s
        .lock()
        .unwrap()
        .execute(
            "object.copies",
            json!({ "id": id, "count": count, "linked": true }),
            Origin::User,
        )
        .unwrap();
    let mut out = vec![id];
    out.extend(
        r["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap()),
    );
    out
}

#[test]
fn copies_dialog_makes_linked_copies() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Button, "Copies...").click();
    h.run();
    h.get_by_label("Linked copies").click();
    h.run();
    assert!(app(&h).ui.prepare.copies.as_ref().unwrap().linked);
    let before = undo_len(&s);
    h.get_by_role_and_label(Role::Button, "Make copies").click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), before + 1);
    let ids: Vec<u64> = s
        .lock()
        .unwrap()
        .doc()
        .document
        .objects
        .iter()
        .map(|o| o.id.0)
        .collect();
    assert_eq!(ids.len(), 4);
    let g = link_group(&s, a);
    assert!(g.is_some());
    assert!(ids.iter().all(|i| link_group(&s, *i) == g));
    // The switch is remembered with the other settings.
    select(&s, &[a]);
    h.run();
    h.get_by_role_and_label(Role::Button, "Copies...").click();
    h.run();
    assert!(app(&h).ui.prepare.copies.as_ref().unwrap().linked);
}

#[test]
fn objects_panel_marks_linked_copies_and_unlinks_one() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let lone = import_square(&s);
    let all = linked_copies(&s, a, 2);
    select(&s, &[all[1]]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    // Three marks for four rows: the lone square has none.
    assert_eq!(h.get_all_by_label("Linked copy").count(), 3);
    assert_eq!(link_group(&s, lone), None);
    h.get_by_label("Linked copy, one of 3. It shares the source, the cut options and the scale.");
    let before = undo_len(&s);
    h.get_by_role_and_label(Role::Button, "Unlink copy").click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(link_group(&s, all[1]), None);
    assert!(link_group(&s, all[0]).is_some());
    assert_eq!(link_group(&s, all[0]), link_group(&s, all[2]));
    assert_eq!(h.get_all_by_label("Linked copy").count(), 2);
}

#[test]
fn settings_say_how_many_linked_copies_an_edit_changes() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let lone = import_square(&s);
    let all = linked_copies(&s, a, 2);
    select(&s, &[all[2]]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_label("Applies to 3 linked copies");
    select(&s, &[lone]);
    h.run();
    assert!(h.query_by_label("Applies to 3 linked copies").is_none());
}

#[test]
fn canvas_marks_a_selection_with_linked_copies() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let lone = import_square(&s);
    linked_copies(&s, a, 1);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_label("Linked copies selected");
    select(&s, &[lone]);
    h.run();
    assert!(h.query_by_label("Linked copies selected").is_none());
}

#[test]
fn arrange_tab_says_scaling_reaches_linked_copies() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let lone = import_square(&s);
    linked_copies(&s, a, 2);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Arrange").click();
    h.run();
    h.get_by_label("Scales all 3 linked copies");
    select(&s, &[lone]);
    h.run();
    assert!(h.query_by_label("Scales all 3 linked copies").is_none());
}

#[test]
fn changed_file_of_linked_copies_counts_once() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let file = dir.path().join("square.svg");
    write_square(&file, 30.0);
    let id = import_file(&s, &file);
    let all = linked_copies(&s, id, 2);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    write_square(&file, 45.5);
    focus_window(&mut h);
    h.get_by_label("1 source file changed");
    h.get_by_role_and_label(Role::Button, "Reload changed files")
        .click();
    h.run();
    for m in all {
        assert!((width(&s, m) - 45.5).abs() < 1e-6, "{}", width(&s, m));
    }
}

#[test]
fn cut_mode_on_a_linked_copy_changes_every_member_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let lone = import_square(&s);
    let all = linked_copies(&s, a, 2);
    select(&s, &[all[1]]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let before = undo_len(&s);
    h.get_by_role_and_label(Role::ComboBox, "Cut mode").click();
    h.run();
    h.get_by_label("All lines").click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), before + 1);
    let mode = |id: u64| {
        s.lock()
            .unwrap()
            .doc()
            .document
            .get(ObjectId(id))
            .unwrap()
            .cut
            .mode
    };
    for m in all {
        assert_eq!(mode(m), flatcraft_core::document::CutMode::AllLines, "{m}");
    }
    assert_eq!(
        mode(lone),
        flatcraft_core::document::CutMode::ObjectOutlines
    );
}

// ---------------------------------------------------------------- multi-selection

fn selection(s: &SharedSession) -> Vec<u64> {
    s.lock()
        .unwrap()
        .doc()
        .selection
        .iter()
        .map(|i| i.0)
        .collect()
}

/// A harness on a clock the test moves (`advance`).
fn clocked(
    dir: &tempfile::TempDir,
    s: &SharedSession,
) -> (
    AppHarness,
    std::sync::Arc<std::sync::Mutex<std::time::Instant>>,
) {
    let now = std::sync::Arc::new(std::sync::Mutex::new(std::time::Instant::now()));
    let clock = now.clone();
    let h = harness_with(dir, s.clone(), move |a| {
        a.with_clock(move || *clock.lock().unwrap())
    });
    (h, now)
}

fn advance(now: &std::sync::Mutex<std::time::Instant>, ms: u64) {
    *now.lock().unwrap() += std::time::Duration::from_millis(ms);
}

fn press(h: &mut AppHarness, modifiers: Modifiers, key: egui::Key) {
    h.key_press_modifiers(modifiers, key);
    h.run();
}

#[test]
fn arrow_keys_nudge_the_selection_by_1_10_and_0_1_mm() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = square_at(&s, 60.0);
    select(&s, &[a, b]);
    let (mut h, now) = clocked(&dir, &s);
    to_prepare(&mut h);
    let (ca, cb) = (object_centre(&s, a), object_centre(&s, b));
    let before = undo_len(&s);
    press(&mut h, Modifiers::NONE, egui::Key::ArrowRight);
    advance(&now, 1000);
    press(&mut h, Modifiers::SHIFT, egui::Key::ArrowDown);
    advance(&now, 1000);
    press(&mut h, Modifiers::ALT, egui::Key::ArrowLeft);
    advance(&now, 1000);
    press(&mut h, Modifiers::NONE, egui::Key::ArrowUp);
    let want = kurbo::Vec2::new(1.0 - 0.1, 10.0 - 1.0);
    for (id, c) in [(a, ca), (b, cb)] {
        let d = object_centre(&s, id) - c;
        assert!((d - want).hypot() < 1e-9, "{id}: {d:?}");
    }
    assert_eq!(
        undo_len(&s),
        before + 4,
        "presses 1 s apart are steps of their own"
    );
}

#[test]
fn a_key_burst_is_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let (mut h, now) = clocked(&dir, &s);
    to_prepare(&mut h);
    let start = object_centre(&s, a);
    let before = undo_len(&s);
    for _ in 0..3 {
        press(&mut h, Modifiers::NONE, egui::Key::ArrowRight);
        advance(&now, 100);
    }
    assert_eq!(undo_len(&s), before + 1, "presses within 500 ms");
    assert!((object_centre(&s, a) - start - kurbo::Vec2::new(3.0, 0.0)).hypot() < 1e-9);
    // A key held down repeats: one step, however long it is held.
    advance(&now, 600);
    h.key_down(egui::Key::ArrowDown);
    h.run();
    for _ in 0..3 {
        advance(&now, 700);
        h.key_down(egui::Key::ArrowDown);
        h.run();
    }
    h.key_up(egui::Key::ArrowDown);
    h.run();
    assert_eq!(undo_len(&s), before + 2, "a pause, then a held key");
    assert!((object_centre(&s, a) - start - kurbo::Vec2::new(3.0, 4.0)).hypot() < 1e-9);
    let doc = s.lock().unwrap().doc().id();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![flatcraft::ui::Action::Undo(doc)]);
    h.run();
    assert!((object_centre(&s, a) - start - kurbo::Vec2::new(3.0, 0.0)).hypot() < 1e-9);
}

#[test]
fn arrows_are_ignored_in_a_text_field() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let start = object_centre(&s, a);
    h.get_by_label("Search commands").click();
    h.run();
    press(&mut h, Modifiers::NONE, egui::Key::ArrowRight);
    press(&mut h, Modifiers::COMMAND, egui::Key::A);
    assert_eq!(object_centre(&s, a), start);
    assert_eq!(
        selection(&s),
        [a],
        "Cmd+A selects the text, not the objects"
    );
}

#[test]
fn arrows_move_an_object_selected_on_the_canvas() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let start = object_centre(&s, a);
    let view = known_view(&mut h, start);
    let at = view.mm_to_screen(start);
    h.hover_at(at);
    h.run();
    h.drag_at(at);
    h.run();
    h.drop_at(at);
    h.run();
    assert_eq!(selection(&s), [a]);
    press(&mut h, Modifiers::NONE, egui::Key::ArrowLeft);
    assert!((object_centre(&s, a) - start + kurbo::Vec2::new(1.0, 0.0)).hypot() < 1e-9);
}

#[test]
fn cmd_a_selects_all_visible_objects() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = square_at(&s, 60.0);
    let hidden = square_at(&s, 120.0);
    s.lock()
        .unwrap()
        .execute(
            "object.set_visible",
            json!({ "ids": [hidden], "visible": false }),
            Origin::User,
        )
        .unwrap();
    select(&s, &[]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    press(&mut h, Modifiers::COMMAND, egui::Key::A);
    assert_eq!(selection(&s), [a, b]);
}

#[test]
fn a_frame_on_the_empty_canvas_selects_what_it_touches() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = square_at(&s, 60.0);
    let c = square_at(&s, 120.0);
    let f = |v: &Value, k: &str| v[k].as_f64().unwrap();
    let (ba, bb, bc) = (bounds(&s, a), bounds(&s, b), bounds(&s, c));
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let view = known_view(&mut h, kurbo::Point::new(f(&bb, "x0"), f(&bb, "y0")));
    let pt = |x: f64, y: f64| view.mm_to_screen(kurbo::Point::new(x, y));
    // From above-left of a to the middle of b: a and b, not c.
    let (top, mid) = (f(&ba, "y0") - 5.0, (f(&bb, "y0") + f(&bb, "y1")) / 2.0);
    drag(
        &mut h,
        pt(f(&ba, "x0") - 5.0, top),
        pt((f(&bb, "x0") + f(&bb, "x1")) / 2.0, mid),
    );
    assert_eq!(selection(&s), [a, b]);
    assert!(app(&h).ui.prepare.drag.is_none());
    // Shift adds c.
    let around_c = (pt(f(&bc, "x0") - 2.0, top), pt(f(&bc, "x1") + 2.0, mid));
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    drag(&mut h, around_c.0, around_c.1);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    assert_eq!(selection(&s), [a, b, c]);
    // Without Shift the frame replaces the selection.
    drag(&mut h, around_c.0, around_c.1);
    assert_eq!(selection(&s), [c]);
    // A frame that touches nothing clears it.
    drag(
        &mut h,
        pt(f(&bc, "x1") + 10.0, top),
        pt(f(&bc, "x1") + 30.0, mid),
    );
    assert!(selection(&s).is_empty());
}

#[test]
fn drag_scale_of_a_linked_copy_previews_every_member() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let all = linked_copies(&s, a, 1);
    let b = all[1];
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let f = |v: &Value, k: &str| v[k].as_f64().unwrap();
    let ba = bounds(&s, a);
    let view = known_view(&mut h, kurbo::Point::new(f(&ba, "x1"), f(&ba, "y1")));
    let cb = object_centre(&s, b);
    let bb = bounds(&s, b);
    // Bottom-right handle of a out by 25 mm along the diagonal: x1.5.
    let from = view.mm_to_screen(kurbo::Point::new(f(&ba, "x1"), f(&ba, "y1")));
    let to = from + Vec2::new(50.0, 50.0);
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    for t in [0.5, 1.0] {
        h.hover_at(from + (to - from) * t);
        h.run();
    }
    let grow = |x: f64, c: f64| c + (x - c) * 1.5;
    let want = view.rect_to_screen(kurbo::Rect::new(
        grow(f(&bb, "x0"), cb.x),
        grow(f(&bb, "y0"), cb.y),
        grow(f(&bb, "x1"), cb.x),
        grow(f(&bb, "y1"), cb.y),
    ));
    assert!(
        !object_paths(&h, want).is_empty(),
        "the linked copy grows about its own centre in the preview"
    );
    h.drop_at(to);
    h.run();
    // The engine agrees with the preview.
    let got = screen_bounds(&h, &s, b);
    assert!((got.min - want.min).length() < 1.0 && (got.max - want.max).length() < 1.0);
}

#[test]
fn rotate_and_mirror_turn_a_multi_selection_as_a_group_or_each_object() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = square_at(&s, 60.0);
    select(&s, &[a, b]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Arrange").click();
    h.run();
    let (ca, cb) = (object_centre(&s, a), object_centre(&s, b));
    // Group by default: half a turn swaps the two centres.
    h.get_by_label("Rotate +90").click();
    h.run();
    h.get_by_label("Rotate +90").click();
    h.run();
    assert!((object_centre(&s, a) - cb).hypot() < 1e-5, "group turn");
    h.get_by_label("Mirror horizontally").click();
    h.run();
    assert!((object_centre(&s, a) - ca).hypot() < 1e-5, "group mirror");
    // Each object: the centres stay.
    h.get_by_label("Each object").click();
    h.run();
    let (ca, cb) = (object_centre(&s, a), object_centre(&s, b));
    h.get_by_label("Rotate +90").click();
    h.run();
    h.get_by_label("Mirror vertically").click();
    h.run();
    assert_eq!((object_centre(&s, a), object_centre(&s, b)), (ca, cb));
    let rot = |id: u64| {
        s.lock()
            .unwrap()
            .doc()
            .document
            .get(ObjectId(id))
            .unwrap()
            .transform
    };
    assert!(rot(a).mirror == rot(b).mirror);
    // One object: the switch is off, it has nothing to choose.
    select(&s, &[a]);
    h.run();
    assert!(disabled(&h, "Each object"));
}

#[test]
fn typed_size_of_a_multi_selection_sizes_the_common_box() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = square_at(&s, 60.0);
    select(&s, &[a, b]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Arrange").click();
    h.run();
    let f = |v: &Value, k: &str| v[k].as_f64().unwrap();
    // 110 mm wide (two 50 mm squares, 10 mm apart): to 220 mm.
    type_into(&mut h, "Width (mm)", "220");
    let (na, nb) = (bounds(&s, a), bounds(&s, b));
    let width = f(&nb, "x1") - f(&na, "x0");
    assert!((width - 220.0).abs() < 1e-3, "common width {width}");
    // The gap grows with the group: the layout stays.
    assert!(
        (f(&nb, "x0") - f(&na, "x1") - 20.0).abs() < 1e-3,
        "{na} {nb}"
    );
}

#[test]
fn typed_percent_of_a_multi_selection_scales_the_group() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = square_at(&s, 60.0);
    select(&s, &[a, b]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Arrange").click();
    h.run();
    let f = |v: &Value, k: &str| v[k].as_f64().unwrap();
    let (oa, ob) = (bounds(&s, a), bounds(&s, b));
    let before = undo_len(&s);
    // The ribbon sends group: true for several objects. 110 mm wide at
    // 50 %: 55 mm, the gap shrinks with it, one undo step.
    type_into(&mut h, "Scale (%)", "50");
    assert_eq!(undo_len(&s), before + 1);
    let (na, nb) = (bounds(&s, a), bounds(&s, b));
    let width = f(&nb, "x1") - f(&na, "x0");
    assert!((width - 55.0).abs() < 1e-3, "common width {width}");
    assert!(
        (f(&nb, "x0") - f(&na, "x1") - 5.0).abs() < 1e-3,
        "{na} {nb}"
    );
    let centre = |a: &Value, b: &Value| (f(a, "x0") + f(b, "x1")) / 2.0;
    assert!(
        (centre(&na, &nb) - centre(&oa, &ob)).abs() < 1e-3,
        "about the group centre"
    );
}

#[test]
fn each_object_is_remembered_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = square_at(&s, 60.0);
    select(&s, &[a, b]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Arrange").click();
    h.run();
    h.get_by_label("Each object").click();
    h.run();
    assert!(
        s.lock().unwrap().prefs.rotate_each,
        "stored in the preferences"
    );
    drop(h);
    // A new window on the stored preferences starts with it on.
    let prefs = flatcraft_engine::prefs::Preferences::load(dir.path());
    assert!(prefs.rotate_each, "written to the file");
    let s2 = session(&dir);
    s2.lock().unwrap().prefs = prefs;
    let a = import_square(&s2);
    let b = square_at(&s2, 60.0);
    select(&s2, &[a, b]);
    let mut h = harness(&dir, s2.clone(), false);
    to_prepare(&mut h);
    assert!(app(&h).ui.prepare.fields.each);
    h.get_by_role_and_label(Role::Tab, "Arrange").click();
    h.run();
    assert_eq!(
        h.get_by_label("Each object").accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    // Off again is stored too.
    h.get_by_label("Each object").click();
    h.run();
    assert!(!flatcraft_engine::prefs::Preferences::load(dir.path()).rotate_each);
}
