//! The Prepare workspace, driven through egui_kittest.

mod common;

use std::path::Path;

use common::{AppHarness, FakePicker, app, app_mut, harness, harness_with, import_square, session};
use egui::accesskit::Role;
use egui::{Color32, Event, Modifiers, Pos2, Shape, Vec2};
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::Workspace;
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

fn undo_len(s: &SharedSession) -> usize {
    s.lock().unwrap().doc().undo.undo_len()
}

fn to_prepare(h: &mut AppHarness) {
    app_mut(h).ui.workspace = Workspace::Prepare;
    h.run();
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

fn focus_window(h: &mut AppHarness) {
    h.event(Event::WindowFocused(false));
    h.run();
    h.event(Event::WindowFocused(true));
    h.run();
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
    // The reset button puts the material value back.
    h.get_by_role_and_label(Role::Button, "Reset speed").click();
    h.run();
    assert_eq!(s.lock().unwrap().doc().document.job.speed, None);
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

/// Every shape painted last frame, with nested lists flattened.
fn painted(h: &AppHarness) -> Vec<Shape> {
    fn walk(shape: &Shape, out: &mut Vec<Shape>) {
        match shape {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            other => out.push(other.clone()),
        }
    }
    let mut out = Vec::new();
    for c in &h.output().shapes {
        walk(&c.shape, &mut out);
    }
    out
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
    let a = import_file(&s, &old.join("a.svg"));
    let b = import_file(&s, &old.join("b.svg"));
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
    h.get_by_label("2 source files not found");
    assert_eq!(h.query_all_by_label("Source file not found").count(), 2);
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
    // b's file is in the same new folder: one click reconnects it too.
    h.get_by_label("1 more file found in that folder");
    h.get_by_role_and_label(Role::Button, "Reconnect them too")
        .click();
    h.run();
    assert_eq!(link(b), new.join("b.svg"));
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
    h.get_by_role_and_label(Role::Button, "Unlink").click();
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
fn trace_button_retraces_the_selected_image() {
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
    h.get_by_role_and_label(Role::Button, "Trace").click();
    h.run();
    assert!(app(&h).ui.toasts.is_empty(), "{:?}", app(&h).ui.toasts);
    assert_eq!(undo_len(&s), before + 1);
}
