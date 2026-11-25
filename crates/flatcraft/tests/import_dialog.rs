//! Import, drag-and-drop and the Trace dialog, driven through egui_kittest.

mod common;

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{
    AppHarness, FakePicker, app, app_mut, harness, harness_with, import_square, session,
    wait_for_source_check,
};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::Workspace;
use flatcraft_core::document::{ObjectId, Source, TraceSettings};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::json;

fn undo_len(s: &SharedSession) -> usize {
    s.lock().unwrap().doc().undo.undo_len()
}

fn to_prepare(h: &mut AppHarness) {
    app_mut(h).ui.workspace = Workspace::Prepare;
    wait_for_source_check(h);
}

/// A black ring (outer radius 80 px, hole 40 px) on white, 200 x 200 px.
fn write_ring(path: &Path) {
    let mut px = tiny_skia::Pixmap::new(200, 200).unwrap();
    px.fill(tiny_skia::Color::WHITE);
    let mut paint = tiny_skia::Paint::default();
    for (r, colour) in [
        (80.0, tiny_skia::Color::BLACK),
        (40.0, tiny_skia::Color::WHITE),
    ] {
        paint.set_color(colour);
        let circle = tiny_skia::PathBuilder::from_circle(100.0, 100.0, r).unwrap();
        px.fill_path(
            &circle,
            &paint,
            tiny_skia::FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );
    }
    px.save_png(path).unwrap();
}

/// Imports a ring PNG (selected afterwards) and returns its id.
fn import_ring(dir: &tempfile::TempDir, s: &SharedSession) -> u64 {
    let png = dir.path().join("ring.png");
    write_ring(&png);
    s.lock()
        .unwrap()
        .execute("object.import", json!({ "path": png }), Origin::User)
        .unwrap()["id"]
        .as_u64()
        .unwrap()
}

/// The object's placed paths as SVG path data, to compare geometry.
fn geometry(s: &SharedSession, id: u64) -> Vec<String> {
    s.lock()
        .unwrap()
        .doc()
        .document
        .get(ObjectId(id))
        .unwrap()
        .placed_paths()
        .iter()
        .map(|p| p.path.to_svg())
        .collect()
}

fn stored_settings(s: &SharedSession, id: u64) -> TraceSettings {
    match &s
        .lock()
        .unwrap()
        .doc()
        .document
        .get(ObjectId(id))
        .unwrap()
        .source
    {
        Source::Raster { trace, .. } => trace.clone(),
        _ => panic!("not a raster"),
    }
}

/// Clicks Trace. The preview worker repaints the window when it is done,
/// so this steps a fixed number of frames instead of `run`.
fn click_trace(h: &mut AppHarness) {
    h.get_by_role_and_label(Role::Button, "Trace").click();
    h.run_steps(3);
    assert!(app(h).ui.trace.is_some(), "the Trace dialog did not open");
}

/// Clicks Trace and waits for the first preview.
fn open_trace(h: &mut AppHarness) {
    click_trace(h);
    step_until(h, "first preview", has_preview);
}

/// Steps the window until `done` holds, at most 10 s of real time.
fn step_until(h: &mut AppHarness, what: &str, done: impl Fn(&AppHarness) -> bool) {
    let start = Instant::now();
    while !done(h) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out: {what}"
        );
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    h.run();
}

fn has_preview(h: &AppHarness) -> bool {
    app(h)
        .ui
        .trace
        .as_ref()
        .is_some_and(|d| d.preview.is_some() && !d.busy())
}

#[test]
fn trace_dialog_apply_retraces() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_ring(&dir, &s);
    // A stored dpi must survive the dialog, which has no field for it.
    s.lock()
        .unwrap()
        .execute(
            "object.retrace",
            json!({ "id": id, "settings": { "dpi_override": 150.0 } }),
            Origin::User,
        )
        .unwrap();
    let before = geometry(&s, id);
    let undo = undo_len(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    open_trace(&mut h);
    h.get_by_label("Invert").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Apply").click();
    h.run();
    assert!(app(&h).ui.toasts.is_empty(), "{:?}", app(&h).ui.toasts);
    assert!(app(&h).ui.trace.is_none(), "Apply closes the dialog");
    assert_eq!(undo_len(&s), undo + 1);
    assert_ne!(geometry(&s, id), before);
    let t = stored_settings(&s, id);
    assert!(t.invert);
    assert_eq!(t.dpi_override, Some(150.0));
    assert_eq!(
        t.min_feature_mm, None,
        "Auto minimum feature stays automatic"
    );
}

#[test]
fn trace_dialog_cancel_keeps_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_ring(&dir, &s);
    let before = geometry(&s, id);
    let undo = undo_len(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    open_trace(&mut h);
    h.get_by_label("Invert").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run();
    assert!(app(&h).ui.trace.is_none());
    assert_eq!(undo_len(&s), undo);
    assert_eq!(geometry(&s, id), before);
    assert!(!stored_settings(&s, id).invert);
}

#[test]
fn trace_dialog_escape_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_ring(&dir, &s);
    let before = geometry(&s, id);
    let undo = undo_len(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    open_trace(&mut h);
    h.get_by_label("Invert").click();
    h.run();
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(app(&h).ui.trace.is_none());
    assert_eq!(undo_len(&s), undo);
    assert_eq!(geometry(&s, id), before);
    // Escape closed the dialog and did not also clear the selection.
    assert!(s.lock().unwrap().doc().selection.contains(&ObjectId(id)));
}

#[test]
fn trace_dialog_starts_from_the_stored_settings() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_ring(&dir, &s);
    s.lock()
        .unwrap()
        .execute(
            "object.retrace",
            json!({ "id": id, "settings": { "threshold": 90, "smoothing": 2, "min_feature_mm": 0.5 } }),
            Origin::User,
        )
        .unwrap();
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    open_trace(&mut h);
    let d = app(&h).ui.trace.as_ref().unwrap();
    assert_eq!(d.settings.threshold, Some(90));
    assert_eq!(d.settings.smoothing, 2);
    assert_eq!(d.settings.min_feature_mm, Some(0.5));
}

#[test]
fn trace_preview_shows_threshold_and_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_ring(&dir, &s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    open_trace(&mut h);
    let d = app(&h).ui.trace.as_ref().unwrap();
    let p = d.preview.as_ref().unwrap();
    // The ring: an outline and a hole.
    assert_eq!(p.lines.len(), 2, "{:?}", p.lines.len());
    let status = d.status();
    assert!(status.contains("(Otsu)"), "{status}");
    assert!(status.contains(&p.threshold_used.to_string()), "{status}");
    assert!(
        status.contains("0 specks removed, 0 small shapes dropped"),
        "{status}"
    );
    h.get_by_label(&status);
}

#[test]
fn trace_preview_runs_off_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_ring(&dir, &s);
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let started_tx = Mutex::new(started_tx);
    let release_rx = Mutex::new(release_rx);
    let mut h = harness_with(&dir, s.clone(), move |a| {
        a.with_tracer(move |bytes, settings, blade| {
            let _ = started_tx.lock().unwrap().send(());
            let _ = release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10));
            flatcraft_import::trace::trace(bytes, settings, blade)
        })
    });
    to_prepare(&mut h);
    click_trace(&mut h);
    started_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the preview trace did not start");
    // The trace is running: the session is free and the window keeps drawing.
    assert!(s.try_lock().is_ok(), "the session is locked during a trace");
    h.run();
    assert!(app(&h).ui.trace.as_ref().unwrap().busy());
    release_tx.send(()).unwrap();
    step_until(&mut h, "preview after release", has_preview);
}

#[test]
fn trace_preview_drops_stale_results() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_ring(&dir, &s);
    let (started_tx, started_rx) = mpsc::channel::<bool>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let started_tx = Mutex::new(started_tx);
    let release_rx = Mutex::new(release_rx);
    let mut h = harness_with(&dir, s.clone(), move |a| {
        a.with_tracer(move |bytes, settings, blade| {
            let _ = started_tx.lock().unwrap().send(settings.invert);
            let _ = release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10));
            flatcraft_import::trace::trace(bytes, settings, blade)
        })
    });
    to_prepare(&mut h);
    click_trace(&mut h);
    assert_eq!(started_rx.recv_timeout(Duration::from_secs(10)), Ok(false));
    // A change while the first trace runs.
    h.get_by_label("Invert").click();
    h.run();
    h.step(); // past the 200 ms debounce
    release_tx.send(()).unwrap();
    assert_eq!(started_rx.recv_timeout(Duration::from_secs(10)), Ok(true));
    // The first result is stale and is never shown.
    h.run();
    assert!(app(&h).ui.trace.as_ref().unwrap().preview.is_none());
    release_tx.send(()).unwrap();
    step_until(&mut h, "fresh preview", has_preview);
    let d = app(&h).ui.trace.as_ref().unwrap();
    assert_eq!(d.preview.as_ref().unwrap().generation, d.generation());
}

#[test]
fn trace_dialog_closes_when_the_document_changes() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_ring(&dir, &s);
    let undo = undo_len(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    open_trace(&mut h);
    s.lock()
        .unwrap()
        .execute("document.new", json!({}), Origin::Agent)
        .unwrap();
    wait_for_source_check(&mut h);
    assert!(app(&h).ui.trace.is_none());
    let first = s.lock().unwrap().docs[0].undo.undo_len();
    assert_eq!(first, undo);
}

#[test]
fn trace_needs_exactly_one_image() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Button, "Trace").click();
    h.run();
    assert!(app(&h).ui.trace.is_none(), "an SVG cannot be traced");
}

// ---------------------------------------------------------------- import and drop

#[test]
fn import_uses_the_file_picker() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let svg = dir.path().join("square.svg");
    std::fs::write(&svg, common::SQUARE_SVG).unwrap();
    let picker = FakePicker::answering([svg]);
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker.clone()));
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Button, "Import").click();
    h.run();
    assert_eq!(s.lock().unwrap().doc().document.objects.len(), 1);
    let reqs = picker.requests.lock().unwrap();
    let names: Vec<&str> = reqs[0].filters.iter().map(|f| f.0.as_str()).collect();
    for name in ["SVG", "PNG", "JPEG", "WebP", "GIF"] {
        assert!(names.contains(&name), "{name} missing in {names:?}");
    }
    let svg = reqs[0].filters.iter().find(|f| f.0 == "SVG").unwrap();
    assert_eq!(svg.1, vec!["svg".to_string(), "svgz".to_string()]);
}

#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

fn drop_files(h: &mut AppHarness, paths: &[PathBuf]) {
    for p in paths {
        h.input_mut()
            .dropped_files
            .push(Arc::new(Dropped(p.clone())));
    }
    // A dropped project opens and starts a source check.
    wait_for_source_check(h);
}

#[test]
fn dropped_svg_is_imported() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = dir.path().join("a.svg");
    let b = dir.path().join("b.png");
    std::fs::write(&a, common::SQUARE_SVG).unwrap();
    write_ring(&b);
    let mut h = harness(&dir, s.clone(), false);
    let undo = undo_len(&s);
    drop_files(&mut h, &[a, b]);
    assert!(app(&h).ui.toasts.is_empty(), "{:?}", app(&h).ui.toasts);
    let names: Vec<String> = s
        .lock()
        .unwrap()
        .doc()
        .document
        .objects
        .iter()
        .map(|o| o.name.clone())
        .collect();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(undo_len(&s), undo + 1, "one undo step for the whole drop");
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
}

#[test]
fn dropped_files_that_fail_are_listed_in_one_toast() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let ok = dir.path().join("ok.svg");
    let bad = dir.path().join("bad.svg");
    let worse = dir.path().join("worse.svg");
    std::fs::write(&ok, common::SQUARE_SVG).unwrap();
    std::fs::write(&bad, "<svg").unwrap();
    std::fs::write(&worse, "not svg").unwrap();
    let mut h = harness(&dir, s.clone(), false);
    drop_files(&mut h, &[ok, bad, worse]);
    let toasts = &app(&h).ui.toasts;
    assert_eq!(toasts.len(), 1, "{toasts:?}");
    assert!(
        toasts[0].text.contains("bad.svg") && toasts[0].text.contains("worse.svg"),
        "{}",
        toasts[0].text
    );
    assert_eq!(s.lock().unwrap().doc().document.objects.len(), 1);
}

#[test]
fn dropped_project_opens() {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let project = dir.path().join("saved.flatcraft");
    {
        let s2 = session(&other);
        import_square(&s2);
        s2.lock()
            .unwrap()
            .execute("project.save", json!({ "path": project }), Origin::User)
            .unwrap();
    }
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    drop_files(&mut h, std::slice::from_ref(&project));
    assert!(app(&h).ui.toasts.is_empty(), "{:?}", app(&h).ui.toasts);
    let st = s.lock().unwrap();
    assert_eq!(st.doc().path.as_deref(), Some(project.as_path()));
    assert_eq!(st.doc().document.objects.len(), 1);
    drop(st);
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
}

#[test]
fn dropped_unsupported_file_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let txt = dir.path().join("notes.txt");
    let svg = dir.path().join("ok.svg");
    std::fs::write(&txt, "hello").unwrap();
    std::fs::write(&svg, common::SQUARE_SVG).unwrap();
    let mut h = harness(&dir, s.clone(), false);
    drop_files(&mut h, &[txt, svg]);
    let toasts = &app(&h).ui.toasts;
    assert_eq!(toasts.len(), 1, "{toasts:?}");
    assert!(toasts[0].text.contains("notes.txt"), "{}", toasts[0].text);
    assert!(!toasts[0].text.contains('\u{2014}'));
    assert_eq!(s.lock().unwrap().doc().document.objects.len(), 1);
}

#[test]
fn trace_dialog_is_accessible() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_ring(&dir, &s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    open_trace(&mut h);
    h.check_accessibility();
}

/// The ring with `n` black 2 x 2 px dots along the top edge.
fn write_ring_with_specks(path: &Path, n: u32) {
    write_ring(path);
    let mut px = tiny_skia::Pixmap::load_png(path).unwrap();
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::BLACK);
    for i in 0..n {
        let r = tiny_skia::Rect::from_xywh(10.0 + i as f32 * 30.0, 4.0, 2.0, 2.0).unwrap();
        px.fill_rect(r, &paint, tiny_skia::Transform::identity(), None);
    }
    px.save_png(path).unwrap();
}

#[test]
fn trace_status_counts_removed_specks() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let png = dir.path().join("specks.png");
    write_ring_with_specks(&png, 3);
    s.lock()
        .unwrap()
        .execute("object.import", json!({ "path": png }), Origin::User)
        .unwrap();
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    open_trace(&mut h);
    let status = app(&h).ui.trace.as_ref().unwrap().status();
    assert!(status.contains(", 3 specks removed,"), "{status}");
}

#[test]
fn unchecking_auto_starts_from_the_otsu_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_ring(&dir, &s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    open_trace(&mut h);
    let used = {
        let d = app(&h).ui.trace.as_ref().unwrap();
        assert_eq!(d.settings.threshold, None);
        d.preview.as_ref().unwrap().threshold_used
    };
    assert_eq!(app(&h).ui.trace.as_ref().unwrap().manual_threshold, used);
    h.get_by_label("Automatic threshold").click();
    h.run();
    assert_eq!(
        app(&h).ui.trace.as_ref().unwrap().settings.threshold,
        Some(used)
    );
}

#[test]
fn trace_status_says_when_a_newer_preview_is_coming() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_ring(&dir, &s);
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Mutex::new(release_rx);
    let calls = Arc::new(Mutex::new(0usize));
    let counter = calls.clone();
    let mut h = harness_with(&dir, s.clone(), move |a| {
        a.with_tracer(move |bytes, settings, blade| {
            let n = {
                let mut c = counter.lock().unwrap();
                *c += 1;
                *c
            };
            if n > 1 {
                let _ = release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10));
            }
            flatcraft_import::trace::trace(bytes, settings, blade)
        })
    });
    to_prepare(&mut h);
    open_trace(&mut h);
    h.get_by_label("Invert").click();
    h.run();
    h.step(); // past the 200 ms debounce
    let status = app(&h).ui.trace.as_ref().unwrap().status();
    assert!(status.ends_with(" (updating...)"), "{status}");
    release_tx.send(()).unwrap();
    step_until(&mut h, "second preview", has_preview);
    let status = app(&h).ui.trace.as_ref().unwrap().status();
    assert!(!status.contains("updating"), "{status}");
}

#[test]
fn trace_dialog_closes_when_the_image_changes() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_ring(&dir, &s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    open_trace(&mut h);
    // An agent retraces the image while the dialog is open.
    s.lock()
        .unwrap()
        .execute(
            "object.retrace",
            json!({ "id": id, "settings": { "smoothing": 1 } }),
            Origin::Agent,
        )
        .unwrap();
    let undo = undo_len(&s);
    h.run();
    assert!(app(&h).ui.trace.is_none());
    let toasts: Vec<&str> = app(&h).ui.toasts.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(toasts, ["The image changed, open Trace again."]);
    assert_eq!(undo_len(&s), undo);
}
