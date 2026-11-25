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
    h.get_by_role_and_label(Role::Tab, "Layout").click();
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
fn scissors_toggle_skips_and_restores_cutting() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let cut = || {
        s.lock()
            .unwrap()
            .doc()
            .document
            .get(ObjectId(id))
            .unwrap()
            .cut_enabled
    };
    h.get_by_role_and_label(Role::Button, "Skip cutting square")
        .click();
    h.run();
    assert!(!cut());
    // Still shown.
    assert!(
        s.lock()
            .unwrap()
            .doc()
            .document
            .get(ObjectId(id))
            .unwrap()
            .visible
    );
    h.get_by_role_and_label(Role::Button, "Cut again square")
        .click();
    h.run();
    assert!(cut());
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
    let colour = Color32::from_rgb(0xFB, 0xF4, 0xE4);
    assert_eq!(mat["color"], "#FBF4E4");
    let hit = painted(&h).iter().any(|shape| match shape {
        Shape::Rect(r) => r.fill == colour && r.rect.contains(at),
        _ => false,
    });
    assert!(hit, "no mat-coloured rect at {at:?}");
}

#[test]
fn mat_margin_is_a_darker_shade_and_the_area_is_the_mat_colour() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let paper = flatcraft_device::PAPER_COLOR;
    let [mr, mg, mb] = flatcraft_core::colour::margin_colour(paper);
    let margin = Color32::from_rgb(mr, mg, mb);
    let shapes = painted(&h);
    let fills: Vec<_> = shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Rect(r) => Some((r.fill, r.rect)),
            _ => None,
        })
        .collect();
    let mat = fills.iter().find(|(c, _)| *c == margin).expect("margin").1;
    let area = fills
        .iter()
        .find(|(c, _)| *c == Color32::from_rgb(paper[0], paper[1], paper[2]))
        .expect("area")
        .1;
    assert!(mat.contains_rect(area) && mat.area() > area.area());
    // The cutting area carries no tint and no dimming overlay of its own.
    assert!(
        !fills
            .iter()
            .any(|(c, r)| *c == Color32::from_black_alpha(110) && r.intersects(area.shrink(2.0)))
    );
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
    h.get_by_role_and_label(Role::Button, "Untitled 1 \u{2022}")
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
    h.get_by_role_and_label(Role::Tab, "Layout").click();
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
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.run();
    select(&s, &[a, b]);
    h.run();
    open_arrange(&mut h);
    assert!(disabled(&h, "Distribute horizontally (Arrange)"));
    assert!(disabled(&h, "Distribute vertically (Arrange)"));
    select(&s, &[a, b, c]);
    h.run();
    assert!(!disabled(&h, "Distribute vertically (Arrange)"));
    let before = undo_len(&s);
    h.get_by_label("Distribute horizontally (Arrange)").click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), before + 1);
    let x = |id| bounds(&s, id)["x0"].as_f64().unwrap();
    let x1 = |id| bounds(&s, id)["x1"].as_f64().unwrap();
    assert!(((x(b) - x1(a)) - (x(c) - x1(b))).abs() < 1e-9);
}

/// Opens the Arrange drop-down of the Arrange tab.
fn open_arrange(h: &mut AppHarness) {
    h.get_by_role_and_label(Role::Button, "Arrange").click();
    h.run();
}

/// Opens the panel of `label` ("Rotate...") from the Arrange drop-down.
fn open_panel(h: &mut AppHarness, label: &str) {
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.run();
    open_arrange(h);
    h.get_by_label(&format!("{label} (Arrange)")).click();
    h.run();
}

fn panel_kind(h: &AppHarness) -> Option<flatcraft::ui::arrange::PanelKind> {
    app(h).ui.prepare.panel.as_ref().map(|p| p.kind)
}

fn values(h: &mut AppHarness) -> &mut flatcraft::ui::arrange::PanelValues {
    &mut app_mut(h).ui.prepare.panel.as_mut().unwrap().values
}

fn revision(s: &SharedSession) -> u64 {
    s.lock().unwrap().doc().revision()
}

fn object_count(s: &SharedSession) -> usize {
    s.lock().unwrap().doc().document.objects.len()
}

/// The accessible name of the panels' Cancel button, with its key hint.
fn cancel_name() -> String {
    let hint = flatcraft::ui::help::hint(flatcraft::ui::help::PrepareKey::ClearSelection);
    format!("Cancel {hint}")
}

fn click_ok(h: &mut AppHarness) {
    h.get_by_role_and_label(Role::Button, "OK").click();
    h.run();
    wait_for_workers(h);
}

#[test]
fn copies_opens_the_rectangular_pattern_panel_and_ok_is_one_step() {
    use flatcraft::ui::arrange::PanelKind;
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    select(&s, &[a, b]);
    h.run();
    assert!(disabled(&h, "Copies"), "one object at a time");
    select(&s, &[a]);
    h.run();
    h.get_by_role_and_label(Role::Button, "Copies").click();
    h.run();
    assert_eq!(panel_kind(&h), Some(PanelKind::RectangularPattern));
    assert!(
        !flatcraft::ui::modal_open(app(&h)),
        "a docked panel is no dialog"
    );
    h.get_by_label("Linked copies");
    h.get_by_role_and_label(Role::SpinButton, "Columns").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run();
    h.event(Event::Text("4".into()));
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(values(&mut h).rect.columns, 4);
    let before = undo_len(&s);
    click_ok(&mut h);
    assert_eq!(undo_len(&s), before + 1);
    // 4 columns, 2 rows: the original and 7 copies.
    assert_eq!(object_count(&s), 2 + 7);
    assert!(app(&h).ui.prepare.panel.is_none(), "OK closes the panel");
    // The original is cell 0, so copy 4 starts row 2.
    let copies = selection(&s);
    let (o, c4) = (bounds(&s, a), bounds(&s, copies[3]));
    let f = |v: &Value, k: &str| v[k].as_f64().unwrap();
    assert!((f(&c4, "x0") - f(&o, "x0")).abs() < 1e-9, "{o} {c4}");
    assert!(
        (f(&c4, "y0") - (f(&o, "y1") + 5.0)).abs() < 1e-9,
        "{o} {c4}"
    );
}

#[test]
fn panels_remember_the_last_values_that_ran() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Button, "Copies").click();
    h.run();
    {
        let r = &mut values(&mut h).rect;
        r.columns = 2;
        r.rows = 5;
        r.gap_x_mm = 1.5;
        r.gap_y_mm = 2.5;
    }
    click_ok(&mut h);
    select(&s, &[a]);
    h.run();
    h.get_by_role_and_label(Role::Button, "Copies").click();
    h.run();
    let r = values(&mut h).rect.clone();
    assert_eq!(
        (r.columns, r.rows, r.gap_x_mm, r.gap_y_mm),
        (2, 5, 1.5, 2.5)
    );
    // Cancel keeps the last values that ran, not the edits.
    values(&mut h).rect.columns = 7;
    h.get_by_role_and_label(Role::Button, &cancel_name())
        .click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Copies").click();
    h.run();
    assert_eq!(values(&mut h).rect.columns, 2);
}

/// The active document as the project file stores it, plus the
/// selection: what Cancel must leave exactly as it was.
fn document_bytes(s: &SharedSession) -> (String, Vec<u64>) {
    let g = s.lock().unwrap();
    let doc = flatcraft_core::project::to_value(&g.doc().document).unwrap();
    let sel = g.doc().selection.iter().map(|i| i.0).collect();
    (serde_json::to_string(&doc).unwrap(), sel)
}

#[test]
fn cancel_close_and_escape_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = square_at(&s, 60.0);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    for (label, ids) in [
        ("Move / Copy...", vec![a, b]),
        ("Rotate...", vec![a, b]),
        ("Rectangular pattern...", vec![a]),
        ("Circular pattern...", vec![a, b]),
        ("Auto arrange...", vec![a, b]),
    ] {
        select(&s, &ids);
        h.run();
        let (before, rev, undo) = (document_bytes(&s), revision(&s), undo_len(&s));
        for how in [cancel_name().as_str(), "Close panel", "escape"] {
            open_panel(&mut h, label);
            assert!(app(&h).ui.prepare.panel.is_some(), "{label}");
            // Edits that OK would use: Cancel drops them.
            {
                let v = values(&mut h);
                v.move_copy.copy = true;
                v.move_copy.count = 4;
                v.rotate.degrees = 33.0;
                v.rotate.each = true;
                v.rect.columns = 5;
                v.circular.count = 7;
                v.spacing_mm = 12.0;
            }
            h.run();
            if how == "escape" {
                h.key_press(egui::Key::Escape);
            } else {
                h.get_by_label(how).click();
            }
            h.run();
            wait_for_workers(&mut h);
            assert!(app(&h).ui.prepare.panel.is_none(), "{label} {how}");
            assert_eq!(document_bytes(&s), before, "{label} {how}");
            assert_eq!(revision(&s), rev, "{label} {how}");
            assert_eq!(undo_len(&s), undo, "{label} {how}");
            assert!(!app(&h).ui.prepare.fields.each, "{label} {how}");
            let stored = flatcraft_engine::prefs::Preferences::load(&app(&h).config_dir);
            assert_eq!(stored.auto_arrange_spacing_mm, 5.0, "{label} {how}");
        }
    }
}

#[test]
fn escape_in_a_panel_field_closes_the_panel_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    open_panel(&mut h, "Move / Copy...");
    h.get_by_role_and_label(Role::SpinButton, "X offset")
        .click();
    h.run();
    assert!(h.ctx.egui_wants_keyboard_input(), "the field has the keys");
    let before = document_bytes(&s);
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(app(&h).ui.prepare.panel.is_none(), "one press closes it");
    assert_eq!(document_bytes(&s), before, "and keeps the selection");
    // Delete afterwards works on the canvas again: the field let go.
    assert!(!h.ctx.egui_wants_keyboard_input());
}

#[test]
fn search_reaches_arrange_commands_that_are_not_on_the_tab() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = square_at(&s, 60.0);
    let c = square_at(&s, 200.0);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let search = |h: &mut AppHarness, query: &str, hit: &str| {
        h.get_by_label("Search commands").click();
        h.run();
        h.get_by_label("Search commands").type_text(query);
        h.run();
        h.get_by_role_and_label(Role::Button, hit).click();
        h.run();
        h.run();
    };
    // Rotate opens its panel.
    select(&s, &[a]);
    h.run();
    search(&mut h, "rotate", "Rotate... (search)");
    assert_eq!(
        panel_kind(&h),
        Some(flatcraft::ui::arrange::PanelKind::Rotate)
    );
    h.get_by_role_and_label(Role::Button, &cancel_name())
        .click();
    h.run();
    // Mirror: unpinned, search runs it from the table all the same.
    let pins = |h: &mut AppHarness, keep: &[&str]| {
        let keep: Vec<String> = keep.iter().map(|k| k.to_string()).collect();
        s.lock()
            .unwrap()
            .update_prefs(|p| p.ribbon_pins = keep)
            .unwrap();
        h.run();
    };
    pins(&mut h, &[]);
    assert!(
        h.query_by_role_and_label(Role::Button, "Mirror horizontally")
            .is_none()
    );
    let undo = undo_len(&s);
    search(&mut h, "mirror", "Mirror horizontally (search)");
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), undo + 1, "mirrored");
    // Align: each option is its own hit.
    let undo = undo_len(&s);
    search(
        &mut h,
        "align right cutting",
        "Align right · cutting area (search)",
    );
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), undo + 1, "aligned");
    // Distribute with one object cannot run: its list opens, with the
    // row dimmed.
    search(&mut h, "distrib", "Distribute horizontally (search)");
    assert_eq!(
        app(&h).ui.prepare.tab,
        flatcraft::ui::prepare::RibbonTab::Layout
    );
    assert!(disabled(&h, "Distribute horizontally (Arrange)"));
    h.key_press(egui::Key::Escape);
    h.run();
    // With three objects it runs.
    s.lock()
        .unwrap()
        .execute(
            "object.move",
            json!({ "ids": [b], "dx_mm": 30.0 }),
            Origin::User,
        )
        .unwrap();
    select(&s, &[a, b, c]);
    h.run();
    let undo = undo_len(&s);
    search(&mut h, "distrib", "Distribute horizontally (search)");
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), undo + 1, "distributed");
}

#[test]
fn keys_typed_into_a_panel_field_leave_the_document_alone() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    open_panel(&mut h, "Move / Copy...");
    h.get_by_role_and_label(Role::SpinButton, "X offset")
        .click();
    h.run();
    assert!(h.ctx.egui_wants_keyboard_input(), "the field has the keys");
    let (rev, centre) = (revision(&s), object_centre(&s, a));
    for key in [
        egui::Key::Delete,
        egui::Key::Backspace,
        egui::Key::ArrowLeft,
        egui::Key::ArrowDown,
    ] {
        h.key_press(key);
        h.run();
    }
    wait_for_workers(&mut h);
    assert_eq!(revision(&s), rev);
    assert_eq!(object_count(&s), 1);
    assert_eq!(object_centre(&s, a), centre);
    assert!(app(&h).ui.prepare.panel.is_some());
}

#[test]
fn move_copy_moves_or_copies_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    // Pinned by default.
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Move / Copy").click();
    h.run();
    let c = object_centre(&s, a);
    {
        let m = &mut values(&mut h).move_copy;
        m.dx_mm = 12.5;
        m.dy_mm = -4.0;
    }
    let before = undo_len(&s);
    click_ok(&mut h);
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(object_centre(&s, a), c + kurbo::Vec2::new(12.5, -4.0));
    // Copy: the original stays, copy k is k offsets away.
    h.get_by_role_and_label(Role::Button, "Move / Copy").click();
    h.run();
    h.get_by_label("Copy").click();
    h.run();
    {
        let m = &mut values(&mut h).move_copy;
        m.dx_mm = 0.0;
        m.dy_mm = 30.0;
        m.count = 3;
    }
    let c = object_centre(&s, a);
    let before = undo_len(&s);
    click_ok(&mut h);
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(object_centre(&s, a), c);
    let copies = selection(&s);
    assert_eq!(copies.len(), 3);
    assert!((object_centre(&s, copies[2]) - (c + kurbo::Vec2::new(0.0, 90.0))).hypot() < 1e-9);
}

#[test]
fn rotate_panel_turns_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = square_at(&s, 60.0);
    select(&s, &[a, b]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let cb = object_centre(&s, b);
    open_panel(&mut h, "Rotate...");
    values(&mut h).rotate.degrees = 180.0;
    let before = undo_len(&s);
    click_ok(&mut h);
    assert_eq!(undo_len(&s), before + 1);
    assert!((object_centre(&s, a) - cb).hypot() < 1e-5, "group turn");
    // Each about its own centre, then Cancel: the Each object switch and
    // its preference stay as they were.
    let rev = revision(&s);
    open_panel(&mut h, "Rotate...");
    h.get_by_label("Each about its own centre").click();
    h.run();
    h.get_by_role_and_label(Role::Button, &cancel_name())
        .click();
    h.run();
    assert!(!app(&h).ui.prepare.fields.each);
    assert!(
        !s.lock().unwrap().prefs.rotate_each,
        "Cancel stores nothing"
    );
    assert_eq!(revision(&s), rev);
    // OK turns each about its own centre and stores the switch.
    open_panel(&mut h, "Rotate...");
    h.get_by_label("Each about its own centre").click();
    h.run();
    let (na, nb) = (object_centre(&s, a), object_centre(&s, b));
    click_ok(&mut h);
    assert_eq!((object_centre(&s, a), object_centre(&s, b)), (na, nb));
    assert!(app(&h).ui.prepare.fields.each);
    assert!(s.lock().unwrap().prefs.rotate_each, "stored");
}

#[test]
fn circular_panel_previews_exactly_what_ok_makes() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let rev = revision(&s);
    open_panel(&mut h, "Circular pattern...");
    {
        let c = &mut values(&mut h).circular;
        c.count = 2;
        c.own_centre = true;
        c.center_mm = [120.0, 120.0];
    }
    h.run();
    let one = ghost_points(&h);
    values(&mut h).circular.count = 4;
    h.run();
    let three = ghost_points(&h);
    assert!(!one.is_empty(), "the ghost is drawn");
    assert_eq!(three.len(), 3 * one.len(), "one ghost per copy");
    assert_eq!(revision(&s), rev, "the preview changes nothing");
    let ghosts = current_ghosts(&mut h);
    assert_eq!(ghosts.len(), 3);
    // Each ghost is drawn where its copy will be: its dashes surround the
    // copy's centre on screen.
    let view = app(&h).ui.prepare.view;
    for (_, g) in &ghosts {
        let c = view.mm_to_screen(*g * kurbo::Point::ZERO);
        let near: Vec<&egui::Pos2> = three.iter().filter(|p| p.distance(c) < 60.0).collect();
        let r = egui::Rect::from_points(&near.iter().map(|p| **p).collect::<Vec<_>>());
        assert!(r.expand(1.0).contains(c), "no ghost around {c:?}");
    }
    let before = undo_len(&s);
    click_ok(&mut h);
    assert_eq!(undo_len(&s), before + 1);
    let copies = selection(&s);
    assert_eq!(copies.len(), 3);
    for ((_, g), id) in ghosts.iter().zip(&copies) {
        let at = *g * kurbo::Point::ZERO;
        assert!((at - object_centre(&s, *id)).hypot() < 1e-9, "{at:?}");
        let turned = *g * kurbo::Point::new(1.0, 0.0) - at;
        let deg = turned.y.atan2(turned.x).to_degrees().rem_euclid(360.0);
        let rot = s
            .lock()
            .unwrap()
            .doc()
            .document
            .get(ObjectId(*id))
            .unwrap()
            .transform
            .rotation_deg;
        assert!((deg - rot).abs() < 1e-6, "{deg} vs {rot}");
    }
}

/// The end points of the ghost preview's dashes painted last frame.
fn ghost_points(h: &AppHarness) -> Vec<egui::Pos2> {
    let mat = state(&app(h).session)["mat"]["color"]
        .as_str()
        .unwrap()
        .to_owned();
    let byte = |k: usize| u8::from_str_radix(&mat[1 + 2 * k..3 + 2 * k], 16).unwrap();
    let colour = flatcraft::ui::canvas::ghost_colour([byte(0), byte(1), byte(2)]);
    painted(h)
        .into_iter()
        .filter_map(|s| match s {
            Shape::LineSegment { points, stroke } if stroke.color == colour => Some(points),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The ghosts of the open panel, as the canvas draws them.
fn current_ghosts(h: &mut AppHarness) -> Vec<(u64, kurbo::Affine)> {
    let ctx = h.ctx.clone();
    let app = app_mut(h);
    let snap = flatcraft::ui::prepare::snapshot(app, &ctx);
    let p = app.ui.prepare.panel.as_ref().unwrap();
    flatcraft::ui::arrange::ghosts(&snap, p)
}

#[test]
fn rectangular_panel_previews_exactly_what_ok_makes() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Button, "Copies").click();
    h.run();
    {
        let r = &mut values(&mut h).rect;
        r.columns = 3;
        r.rows = 2;
        r.gap_x_mm = 2.5;
        r.gap_y_mm = 7.0;
    }
    h.run();
    assert!(!ghost_points(&h).is_empty());
    let ghosts = current_ghosts(&mut h);
    assert_eq!(ghosts.len(), 5);
    click_ok(&mut h);
    let copies = selection(&s);
    assert_eq!(copies.len(), 5);
    for ((_, g), id) in ghosts.iter().zip(&copies) {
        let at = *g * kurbo::Point::ZERO;
        assert!((at - object_centre(&s, *id)).hypot() < 1e-9, "{at:?}");
    }
}

#[test]
fn pins_round_trip_through_the_preferences() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.run();
    // The default pins: two big buttons, four directions.
    for label in [
        "Move / Copy",
        "Rectangular pattern",
        "Rotate -90",
        "Rotate +90",
        "Mirror horizontally",
        "Mirror vertically",
    ] {
        h.get_by_role_and_label(Role::Button, label);
    }
    assert!(h.query_by_label("Circular pattern").is_none());
    open_arrange(&mut h);
    h.get_by_label(flatcraft::ui::groups::FOOTER);
    let hover = |h: &mut AppHarness, row: &str| {
        let at = h.get_by_label(row).rect().center();
        h.hover_at(egui::pos2(at.x, at.y));
        h.run();
        h.run();
    };
    hover(&mut h, "Circular pattern... (Arrange)");
    h.get_by_label("Pin Circular pattern").click();
    h.run();
    let stored =
        |h: &AppHarness| flatcraft_engine::prefs::Preferences::load(&app(h).config_dir).ribbon_pins;
    assert!(
        stored(&h).contains(&"pattern.circular".to_string()),
        "{:?}",
        stored(&h)
    );
    // The list is still open and the ribbon shows the new pin at once.
    h.get_by_label(flatcraft::ui::groups::FOOTER);
    h.get_by_role_and_label(Role::Button, "Circular pattern");
    // Unpin one Mirror direction from its row, pin an Align direction;
    // the list stays open meanwhile.
    for (row, toggle) in [
        ("Mirror vertically (Arrange)", "Unpin Mirror vertically"),
        ("Align top (Arrange)", "Pin Align top · selection"),
    ] {
        hover(&mut h, row);
        h.get_by_label(toggle).click();
        h.run();
    }
    let mut want = flatcraft_engine::prefs::default_ribbon_pins();
    want.retain(|p| p != "mirror.vertical");
    want.extend(["pattern.circular".into(), "align.top.selection".into()]);
    assert_eq!(stored(&h), want);
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(
        h.query_by_role_and_label(Role::Button, "Mirror vertically")
            .is_none()
    );
    h.get_by_role_and_label(Role::Button, "Align top · selection");
}

#[test]
fn search_finds_copies_and_distribute() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    for (query, hit) in [
        ("distrib", "Distribute horizontally (search)"),
        ("copies", "Copies... (search)"),
    ] {
        h.get_by_label("Search commands").click();
        h.run();
        h.get_by_label("Search commands").type_text(query);
        h.run();
        h.get_by_role_and_label(Role::Button, hit).click();
        h.run();
    }
    assert_eq!(
        panel_kind(&h),
        Some(flatcraft::ui::arrange::PanelKind::RectangularPattern),
        "copies opens the panel"
    );
    assert_eq!(
        app(&h).ui.prepare.tab,
        flatcraft::ui::prepare::RibbonTab::Layout
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
fn rectangular_pattern_makes_linked_copies() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Button, "Copies").click();
    h.run();
    {
        let r = &mut values(&mut h).rect;
        r.columns = 2;
        r.rows = 2;
    }
    h.get_by_label("Linked copies").click();
    h.run();
    assert!(values(&mut h).rect.linked);
    let before = undo_len(&s);
    click_ok(&mut h);
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
    // The switch is remembered with the other values.
    select(&s, &[a]);
    h.run();
    h.get_by_role_and_label(Role::Button, "Copies").click();
    h.run();
    assert!(values(&mut h).rect.linked);
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
    h.get_by_role_and_label(Role::Tab, "Layout").click();
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
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.run();
    let (ca, cb) = (object_centre(&s, a), object_centre(&s, b));
    // Group by default: half a turn swaps the two centres.
    let rotate = |h: &mut AppHarness, degrees: f64| {
        open_panel(h, "Rotate...");
        values(h).rotate.degrees = degrees;
        click_ok(h);
    };
    rotate(&mut h, 180.0);
    assert!((object_centre(&s, a) - cb).hypot() < 1e-5, "group turn");
    // Both Mirror directions are pinned by default: one click each.
    let mirror = |h: &mut AppHarness, item: &str| {
        h.get_by_role_and_label(Role::Button, item).click();
        h.run();
    };
    mirror(&mut h, "Mirror horizontally");
    assert!((object_centre(&s, a) - ca).hypot() < 1e-5, "group mirror");
    // Each object: the centres stay.
    h.get_by_label("Each object").click();
    h.run();
    let (ca, cb) = (object_centre(&s, a), object_centre(&s, b));
    rotate(&mut h, 90.0);
    mirror(&mut h, "Mirror vertically");
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
    h.get_by_role_and_label(Role::Tab, "Layout").click();
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
    h.get_by_role_and_label(Role::Tab, "Layout").click();
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
    h.get_by_role_and_label(Role::Tab, "Layout").click();
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
    h.get_by_role_and_label(Role::Tab, "Layout").click();
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

/// Every direction pinned: one click on each runs exactly its command,
/// one undo step with the same result as the command run directly.
#[test]
fn each_pinned_direction_runs_its_command_in_one_click() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = square_at(&s, 60.0);
    let c = square_at(&s, 200.0);
    for (id, dy) in [(b, 25.0), (c, 70.0)] {
        s.lock()
            .unwrap()
            .execute(
                "object.move",
                json!({ "ids": [id], "dy_mm": dy }),
                Origin::User,
            )
            .unwrap();
    }
    select(&s, &[a, b, c]);
    s.lock()
        .unwrap()
        .update_prefs(|p| {
            p.ribbon_pins = flatcraft::ui::tools::TOOLS
                .iter()
                .filter(|t| t.style == flatcraft::ui::tools::Style::Small)
                .map(|t| t.id.to_string())
                .collect();
        })
        .unwrap();
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.run();
    let ids = json!([a, b, c]);
    let align = |mode: &str, to: &str| {
        (
            "arrange.align",
            json!({ "ids": ids, "mode": mode, "relative_to": to }),
        )
    };
    // The list shows the other option: a pin runs with its own.
    app_mut(&mut h).ui.prepare.fields.align_to_area = true;
    let cases = [
        (
            "Rotate -90",
            (
                "object.rotate",
                json!({ "ids": ids, "degrees": -90.0, "about": "group" }),
            ),
        ),
        (
            "Rotate +90",
            (
                "object.rotate",
                json!({ "ids": ids, "degrees": 90.0, "about": "group" }),
            ),
        ),
        (
            "Mirror horizontally",
            (
                "object.mirror",
                json!({ "ids": ids, "axis": "horizontal", "about": "group" }),
            ),
        ),
        (
            "Mirror vertically",
            (
                "object.mirror",
                json!({ "ids": ids, "axis": "vertical", "about": "group" }),
            ),
        ),
        ("Align left · selection", align("left", "selection")),
        ("Align centre · selection", align("hcenter", "selection")),
        ("Align right · selection", align("right", "selection")),
        ("Align top · selection", align("top", "selection")),
        ("Align middle · selection", align("vcenter", "selection")),
        ("Align bottom · selection", align("bottom", "selection")),
        ("Align left · cutting area", align("left", "cutting_area")),
        (
            "Align centre · cutting area",
            align("hcenter", "cutting_area"),
        ),
        ("Align right · cutting area", align("right", "cutting_area")),
        ("Align top · cutting area", align("top", "cutting_area")),
        (
            "Align middle · cutting area",
            align("vcenter", "cutting_area"),
        ),
        (
            "Align bottom · cutting area",
            align("bottom", "cutting_area"),
        ),
        (
            "Distribute horizontally",
            (
                "arrange.distribute",
                json!({ "ids": ids, "axis": "horizontal" }),
            ),
        ),
        (
            "Distribute vertically",
            (
                "arrange.distribute",
                json!({ "ids": ids, "axis": "vertical" }),
            ),
        ),
    ];
    for (label, (cmd, params)) in cases {
        let before = document_bytes(&s);
        let undo = undo_len(&s);
        h.get_by_role_and_label(Role::Button, label).click();
        h.run();
        wait_for_workers(&mut h);
        assert_eq!(undo_len(&s), undo + 1, "{label}: one step");
        let clicked = document_bytes(&s);
        assert_ne!(clicked, before, "{label} changed something");
        let mut g = s.lock().unwrap();
        g.execute("edit.undo", json!({}), Origin::User).unwrap();
        g.execute(cmd, params, Origin::User).unwrap();
        drop(g);
        assert_eq!(document_bytes(&s), clicked, "{label} ran {cmd}");
        // Back to the staggered layout for the next one.
        s.lock()
            .unwrap()
            .execute("edit.undo", json!({}), Origin::User)
            .unwrap();
        h.run();
    }
}

#[test]
fn escape_in_the_search_box_leaves_the_panel_open() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    open_panel(&mut h, "Move / Copy...");
    h.get_by_label("Search commands").click();
    h.run();
    assert!(h.ctx.egui_wants_keyboard_input(), "search has the keys");
    let before = document_bytes(&s);
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(app(&h).ui.prepare.panel.is_some(), "the panel stays");
    assert!(!h.ctx.egui_wants_keyboard_input(), "search let go");
    assert_eq!(document_bytes(&s), before, "the selection stays too");
    // With no focus, Escape closes the panel.
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(app(&h).ui.prepare.panel.is_none());
}
