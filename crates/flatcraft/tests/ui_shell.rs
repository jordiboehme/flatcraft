//! The app shell, driven through egui_kittest.

mod common;
use common::RunPlan;

use common::{
    FakePicker, app, app_mut, capture, harness, harness_with, import_square, session,
    wait_for_source_check,
};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::{Action, Workspace};
use flatcraft_core::document::ObjectId;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::Origin;
use serde_json::json;

#[test]
fn switcher_changes_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    h.check_accessibility();
    h.get_by_role_and_label(Role::Button, "Workspace").click();
    h.run_plan();
    // The open popup and its cards have accessible names too.
    h.check_accessibility();
    h.get_by_role_and_label(Role::Button, "Prepare");
    h.get_by_role_and_label(Role::Button, "Cut").click();
    h.run_plan();
    assert_eq!(app(&h).ui.workspace, Workspace::Cut);
}

#[test]
fn undo_button_calls_edit_undo() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let (id, before) = {
        let mut s = s.lock().unwrap();
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="20" y="20" width="50" height="50"/></svg>"#;
        let r = s
            .execute(
                "object.import_svg",
                json!({ "svg": svg, "name": "square" }),
                Origin::User,
            )
            .unwrap();
        let id = r["id"].as_u64().unwrap();
        let before = s.doc().document.get(ObjectId(id)).unwrap().transform;
        s.execute(
            "object.move",
            json!({ "ids": [id], "dx_mm": 7.0, "dy_mm": 3.0 }),
            Origin::User,
        )
        .unwrap();
        assert_ne!(
            s.doc().document.get(ObjectId(id)).unwrap().transform,
            before
        );
        (id, before)
    };
    let mut h = harness(&dir, s.clone(), false);
    h.get_by_role_and_label(Role::Button, "Undo").click();
    h.run_plan();
    let s = s.lock().unwrap();
    assert_eq!(
        s.doc().document.get(ObjectId(id)).unwrap().transform,
        before
    );
}

#[test]
fn new_tab_adds_document() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    h.get_by_role_and_label(Role::Button, "New document")
        .click();
    h.run_plan();
    assert_eq!(s.lock().unwrap().docs.len(), 2);
}

#[test]
fn agent_pill_only_with_agent() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(&dir, session(&dir), false);
    // The shell is drawn: the app bar is there, the pill is not.
    h.get_by_role_and_label(Role::Button, "Settings");
    assert!(h.query_by_label_contains("Agent").is_none());
    drop(h);

    let h = harness(&dir, session(&dir), true);
    h.get_by_label("Agent idle");
    assert!(h.query_by_label("Agent connected").is_none());
}

#[test]
fn agent_pill_shows_recent_activity() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    s.lock()
        .unwrap()
        .execute("document.state", json!({}), Origin::Agent)
        .unwrap();
    let h = harness(&dir, s, true);
    h.get_by_label("Agent connected");
}

#[test]
fn home_shows_new_open_and_recent_projects() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    s.lock().unwrap().prefs.recent_projects = vec![dir.path().join("coasters.flatcraft")];
    let h = harness(&dir, s.clone(), false);
    assert_eq!(app(&h).ui.workspace, Workspace::Home);
    h.get_by_role_and_label(Role::Button, "New project");
    h.get_by_role_and_label(Role::Button, "Open...");
    // A missing recent project offers Locate and Remove, not a dead row.
    assert!(
        h.query_by_role_and_label(Role::Button, "Open coasters")
            .is_none()
    );
    h.get_by_role_and_label(Role::Button, "Locate...");
    h.get_by_role_and_label(Role::Button, "Remove from list");
}

#[test]
fn home_new_reuses_the_empty_start_document() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    h.get_by_role_and_label(Role::Button, "New project").click();
    h.run_plan();
    assert_eq!(s.lock().unwrap().docs.len(), 1);
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);

    // A document with content stays; New opens a second one.
    h.get_by_role_and_label(Role::Button, "Home").click();
    h.run_plan();
    s.lock().unwrap().doc_mut().dirty = true;
    h.get_by_role_and_label(Role::Button, "New project").click();
    wait_for_source_check(&mut h);
    assert_eq!(s.lock().unwrap().docs.len(), 2);
}

#[test]
fn save_targets_the_document_chosen_before_the_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    // The person picks Save As on this document ...
    let chosen = s.lock().unwrap().doc().id();
    // ... and while the dialog is open, an agent opens another document.
    s.lock()
        .unwrap()
        .execute("document.new", json!({}), Origin::Agent)
        .unwrap();
    let path = dir.path().join("chosen.flatcraft");
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(
        &ctx,
        vec![Action::SaveTo {
            doc_id: chosen,
            path: path.clone(),
        }],
    );
    let saved = flatcraft_core::project::load(&path).unwrap();
    assert_eq!(saved.objects.len(), 1, "the chosen document was saved");
    let st = s.lock().unwrap();
    assert_eq!(st.docs[0].path.as_deref(), Some(path.as_path()));
    assert!(st.docs[1].path.is_none());
    assert_eq!(st.active, 1, "saving does not switch documents");
    // A successful save lands in the recent projects, on disk too.
    assert_eq!(st.prefs.recent_projects, std::slice::from_ref(&path));
    let stored = Preferences::load(dir.path());
    assert_eq!(stored.recent_projects, [path]);
}

#[test]
fn open_adds_to_recent_projects() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let path = dir.path().join("coasters.flatcraft");
    s.lock()
        .unwrap()
        .execute("project.save", json!({ "path": path }), Origin::User)
        .unwrap();
    let mut h = harness(&dir, session(&dir), false);
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Open(path.clone())]);
    common::wait_for_long(&mut h);
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
    assert_eq!(Preferences::load(dir.path()).recent_projects, [path]);
}

#[test]
fn a_recent_project_save_keeps_what_another_window_stored() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let path = dir.path().join("coasters.flatcraft");
    s.lock()
        .unwrap()
        .execute("project.save", json!({ "path": path }), Origin::User)
        .unwrap();
    let mut h = harness(&dir, session(&dir), false);
    // Another FlatCraft window turns on auto reload after this one started.
    let other = session(&dir);
    other
        .lock()
        .unwrap()
        .update_prefs(|p| p.auto_reload = true)
        .unwrap();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Open(path.clone())]);
    common::wait_for_long(&mut h);
    let stored = Preferences::load(dir.path());
    assert_eq!(stored.recent_projects, [path]);
    assert!(stored.auto_reload, "the other window's change survives");
}

#[test]
fn close_targets_the_document_by_id() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    let (keep, close) = {
        let mut st = s.lock().unwrap();
        st.execute("document.new", json!({}), Origin::User).unwrap();
        (st.docs[0].id(), st.docs[1].id())
    };
    // An agent closes the first document; the index of `close` shifts.
    s.lock()
        .unwrap()
        .execute("document.close", json!({ "index": 0 }), Origin::Agent)
        .unwrap();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Close(close)]);
    let ids: Vec<u64> = s.lock().unwrap().docs.iter().map(|d| d.id()).collect();
    assert!(!ids.contains(&close) && !ids.contains(&keep), "{ids:?}");
}

#[test]
fn closing_a_dirty_tab_offers_discard() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    h.get_by_role_and_label(Role::Button, "Close Untitled 1")
        .click();
    h.run_plan();
    assert_eq!(s.lock().unwrap().docs[0].document.objects.len(), 1);
    h.get_by_label("Save changes to Untitled 1?");
    // Cancel keeps it.
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run_plan();
    assert!(h.query_by_label("Save changes to Untitled 1?").is_none());
    assert_eq!(s.lock().unwrap().docs[0].document.objects.len(), 1);
    // Discard closes it.
    h.get_by_role_and_label(Role::Button, "Close Untitled 1")
        .click();
    h.run_plan();
    h.get_by_role_and_label(Role::Button, "Don't Save").click();
    h.run_plan();
    let st = s.lock().unwrap();
    assert_eq!(st.docs.len(), 1);
    assert!(st.doc().document.objects.is_empty(), "a fresh document");
}

#[test]
fn closing_a_dirty_tab_can_save_it_first() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let path = dir.path().join("kept.flatcraft");
    let picker = FakePicker::answering([path.clone()]);
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker.clone()));
    h.get_by_role_and_label(Role::Button, "Close Untitled 1")
        .click();
    h.run_plan();
    // The ribbon has a Save button too; the dialog is drawn last.
    h.get_all_by_role_and_label(Role::Button, "Save")
        .last()
        .unwrap()
        .click();
    h.run_plan();
    assert_eq!(
        flatcraft_core::project::load(&path).unwrap().objects.len(),
        1
    );
    assert!(s.lock().unwrap().doc().document.objects.is_empty());
}

#[test]
fn quit_with_unsaved_work_asks_first_and_cancel_keeps_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    let ctx = h.ctx.clone();
    let got = capture(&mut h, |h| {
        app_mut(h).apply(&ctx, vec![Action::Quit]);
        h.run_plan();
    });
    h.get_by_label("Save changes to Untitled 1?");
    assert!(!got.closes());
    let got = capture(&mut h, |h| {
        h.get_by_role_and_label(Role::Button, "Cancel").click();
        h.run_plan();
    });
    assert!(h.query_by_label("Save changes to Untitled 1?").is_none());
    assert!(!got.closes());
    assert!(!app(&h).ui.closing);
    assert_eq!(s.lock().unwrap().docs[0].document.objects.len(), 1);
}

#[test]
fn quit_without_unsaved_work_closes_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Quit]);
    assert!(app(&h).ui.unsaved.is_none());
    assert!(app(&h).ui.closing);
}

#[test]
fn discard_on_quit_closes_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    s.lock()
        .unwrap()
        .execute("document.new", json!({}), Origin::User)
        .unwrap();
    let s2 = s.clone();
    import_square(&s2);
    let mut h = harness(&dir, s.clone(), false);
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Quit]);
    h.run_plan();
    // One document after the other, each by its name.
    h.get_by_label("Save changes to Untitled 1?");
    h.get_by_role_and_label(Role::Button, "Don't Save").click();
    h.run_plan();
    assert!(
        !app(&h).ui.closing,
        "the second document is still asked about"
    );
    h.get_by_label("Save changes to Untitled 2?");
    h.get_by_role_and_label(Role::Button, "Don't Save").click();
    h.run_plan();
    assert!(app(&h).ui.closing, "the window closes");
    assert!(app(&h).ui.unsaved.is_none());
    // The Close command comes back as a close request, which now goes ahead.
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    let got = capture(&mut h, |h| h.step());
    assert!(!got.viewport.contains(&egui::ViewportCommand::CancelClose));
}

#[test]
fn the_window_close_button_asks_too() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s, false);
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    let got = capture(&mut h, |h| h.step());
    assert!(got.viewport.contains(&egui::ViewportCommand::CancelClose));
    h.run_plan();
    h.get_by_label("Save changes to Untitled 1?");
}

#[test]
fn save_all_asks_for_a_path_per_untitled_document() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let path = dir.path().join("saved.flatcraft");
    let picker = FakePicker::answering([path.clone()]);
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker.clone()));
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Quit]);
    h.run_plan();
    // The app bar has a Save button too; the dialog is drawn last.
    h.get_all_by_role_and_label(Role::Button, "Save")
        .last()
        .unwrap()
        .click();
    h.run_plan();
    assert_eq!(picker.requests.lock().unwrap().len(), 1);
    assert_eq!(
        flatcraft_core::project::load(&path).unwrap().objects.len(),
        1
    );
    assert!(!s.lock().unwrap().doc().dirty);
    assert!(app(&h).ui.closing, "the window closes");
}

#[test]
fn a_cancelled_save_all_keeps_the_app_open() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let picker = FakePicker::default();
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker.clone()));
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Quit]);
    h.run_plan();
    // The app bar has a Save button too; the dialog is drawn last.
    let got = capture(&mut h, |h| {
        h.get_all_by_role_and_label(Role::Button, "Save")
            .last()
            .unwrap()
            .click();
        h.run_plan();
    });
    assert!(!got.closes());
    assert!(!app(&h).ui.closing);
    assert!(s.lock().unwrap().doc().dirty);
}

#[test]
fn untitled_names_stay_with_their_document() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    for _ in 0..2 {
        h.get_by_role_and_label(Role::Button, "New document")
            .click();
        h.run_plan();
    }
    h.get_by_role_and_label(Role::Button, "Close Untitled 2")
        .click();
    h.run_plan();
    h.get_by_role_and_label(Role::Button, "Untitled 1");
    h.get_by_role_and_label(Role::Button, "Untitled 3");
    assert!(h.query_by_label("Untitled 2").is_none());
}

/// Cmd+N: the key handler's on Windows and Linux; on macOS the menu bar has
/// the key and its New item runs the same action.
fn cmd_n(h: &mut common::AppHarness) {
    if cfg!(target_os = "macos") {
        let ctx = h.ctx.clone();
        let n = flatcraft::ui::menu_model::Cmd::New;
        app_mut(h).apply(&ctx, vec![n.action(0, &[]).unwrap()]);
    } else {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::N);
    }
}

#[test]
fn cmd_n_and_home_new_reuse_the_empty_start_document() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    cmd_n(&mut h);
    h.run_plan();
    assert_eq!(s.lock().unwrap().docs.len(), 1);
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
    s.lock().unwrap().doc_mut().dirty = true;
    cmd_n(&mut h);
    wait_for_source_check(&mut h);
    assert_eq!(s.lock().unwrap().docs.len(), 2);
}

#[test]
fn undo_targets_the_document_clicked() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    let chosen = s.lock().unwrap().doc().id();
    // Between the click and the apply, an agent opens another document.
    s.lock()
        .unwrap()
        .execute("document.new", json!({}), Origin::Agent)
        .unwrap();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Undo(chosen)]);
    let st = s.lock().unwrap();
    assert!(
        st.docs[0].document.objects.is_empty(),
        "the import was undone"
    );
    assert!(app(&h).ui.toasts.is_empty(), "{:?}", app(&h).ui.toasts);
}

#[test]
fn home_cards_line_up_under_the_title() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(&dir, session(&dir), false);
    let new = h.get_by_role_and_label(Role::Button, "New project").rect();
    let patterns = h
        .get_by_role_and_label(Role::Button, "Test patterns")
        .rect();
    let title = h.get_by_label("Start").rect();
    assert!(
        (new.left() - title.left()).abs() < 2.0,
        "card {new:?}, title {title:?}"
    );
    // 1400 pt: the four cards share one row.
    assert!((new.top() - patterns.top()).abs() < 0.5);
}

#[test]
fn save_as_suggests_the_current_file_name() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let h = harness(&dir, s.clone(), false);
    let id = s.lock().unwrap().doc().id();
    assert_eq!(
        app(&h).save_as_suggestion(id),
        ("Untitled 1.flatcraft".to_string(), None)
    );
    let path = dir.path().join("coasters.flatcraft");
    s.lock().unwrap().doc_mut().path = Some(path);
    assert_eq!(
        app(&h).save_as_suggestion(id),
        (
            "coasters.flatcraft".to_string(),
            Some(dir.path().to_path_buf())
        )
    );
}

/// A preferences file that breaks while the window runs is named in a
/// warning toast, once, not only on stderr.
#[test]
fn a_broken_preferences_file_shows_a_toast_once() {
    use flatcraft::ui::toasts::ToastKind;
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    s.lock().unwrap().watch_prefs_file();
    let path = dir.path().join(flatcraft_engine::prefs::FILE_NAME);
    std::fs::write(&path, "allow_agent_cuts = [").unwrap();
    let mut h = harness(&dir, s, false);
    h.run_plan();
    let warnings = |h: &common::AppHarness| {
        app(h)
            .ui
            .toasts
            .iter()
            .filter(|t| t.kind == ToastKind::Warning)
            .map(|t| t.text.clone())
            .collect::<Vec<_>>()
    };
    let shown = warnings(&h);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(shown[0].contains("preferences.toml"), "{shown:?}");
    // A second later the file is still broken: no second toast.
    let later = std::time::Instant::now() + std::time::Duration::from_secs(2);
    app_mut(&mut h).clock = std::sync::Arc::new(move || later);
    h.run_plan();
    assert_eq!(warnings(&h).len(), 1);
}

/// The warning from loading the preferences at start reaches the window.
#[test]
fn the_start_up_preferences_warning_shows_a_toast() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    s.lock()
        .unwrap()
        .note_prefs_warning("preferences.toml is broken. Using default preferences.".into());
    let mut h = harness(&dir, s, false);
    h.run_plan();
    assert!(
        app(&h)
            .ui
            .toasts
            .iter()
            .any(|t| t.text.contains("Using default preferences")),
        "{:?}",
        app(&h).ui.toasts
    );
}

/// Storing the recent list re-reads the preferences file; a theme another
/// window stored meanwhile reaches this window's colours too.
#[test]
fn a_theme_read_back_by_a_preference_save_is_applied() {
    use flatcraft::ui::theme::Tokens;
    use flatcraft_engine::prefs::Theme;
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let path = dir.path().join("coasters.flatcraft");
    s.lock()
        .unwrap()
        .execute("project.save", json!({ "path": path }), Origin::User)
        .unwrap();
    let mut h = harness(&dir, session(&dir), false);
    assert_eq!(app(&h).tokens, Tokens::for_theme(Theme::Dark, None));
    session(&dir)
        .lock()
        .unwrap()
        .update_prefs(|p| p.theme = Theme::Light)
        .unwrap();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Open(path)]);
    h.run_plan();
    assert_eq!(app(&h).tokens, Tokens::for_theme(Theme::Light, None));
}

/// Storing a preference over a broken file keeps a copy of it, and the
/// window says where.
#[test]
fn a_backup_of_broken_preferences_shows_a_toast() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let path = dir.path().join("coasters.flatcraft");
    s.lock()
        .unwrap()
        .execute("project.save", json!({ "path": path }), Origin::User)
        .unwrap();
    let mut h = harness(&dir, session(&dir), false);
    std::fs::write(
        dir.path().join(flatcraft_engine::prefs::FILE_NAME),
        "allow_agent_cuts = [",
    )
    .unwrap();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Open(path)]);
    h.run_plan();
    assert!(
        app(&h)
            .ui
            .toasts
            .iter()
            .any(|t| t.text.contains("keeps a copy as")),
        "{:?}",
        app(&h).ui.toasts
    );
}

/// Toasts age by the window's clock, so a test can let one expire.
#[test]
fn toasts_expire_by_the_window_clock() {
    use flatcraft::ui::toasts::Toast;
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let now = std::sync::Arc::new(std::sync::Mutex::new(std::time::Instant::now()));
    let clock = now.clone();
    let mut h = harness_with(&dir, s, move |a| {
        a.with_clock(move || *clock.lock().unwrap())
    });
    app_mut(&mut h).toast(Toast::info("Great Scott!"));
    h.run_plan();
    assert_eq!(app(&h).ui.toasts.len(), 1);
    *now.lock().unwrap() += flatcraft::ui::toasts::LIFETIME;
    h.run_plan();
    assert!(
        app(&h).ui.toasts.is_empty(),
        "expired after 5 s on the clock"
    );
}

/// With System the colours follow the operating system's appearance, live:
/// the viewport input changes between frames and no restart is needed.
#[test]
fn the_system_theme_switches_the_tokens_live() {
    use flatcraft::ui::theme::Tokens;
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    assert_eq!(
        s.lock().unwrap().prefs.theme,
        flatcraft_engine::prefs::Theme::System
    );
    let mut h = harness(&dir, s, false);
    h.input_mut().system_theme = Some(egui::Theme::Light);
    h.run_plan();
    assert_eq!(app(&h).tokens, Tokens::light());
    assert_eq!(h.ctx.theme(), egui::Theme::Light);
    h.input_mut().system_theme = Some(egui::Theme::Dark);
    h.run_plan();
    assert_eq!(app(&h).tokens, Tokens::dark());
    assert_eq!(h.ctx.theme(), egui::Theme::Dark);
}

/// Dark and Light ignore the system.
#[test]
fn a_fixed_theme_ignores_the_system() {
    use flatcraft::ui::theme::Tokens;
    use flatcraft_engine::prefs::Theme;
    for (pref, want) in [
        (Theme::Dark, Tokens::dark()),
        (Theme::Light, Tokens::light()),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        s.lock().unwrap().update_prefs(|p| p.theme = pref).unwrap();
        import_square(&s);
        let mut h = harness(&dir, s, false);
        for sys in [egui::Theme::Light, egui::Theme::Dark] {
            h.input_mut().system_theme = Some(sys);
            h.run_plan();
            assert_eq!(app(&h).tokens, want, "{pref:?} under system {sys:?}");
        }
    }
}

#[test]
fn tabs_and_dialogs_name_a_project_without_its_extension() {
    use flatcraft::ui::project_name;
    use std::path::Path;
    assert_eq!(project_name(Path::new("/a/Stickers.flatcraft")), "Stickers");
    assert_eq!(
        project_name(Path::new("/a/my.design.flatcraft")),
        "my.design"
    );
    assert_eq!(project_name(Path::new("/a/LOUD.FLATCRAFT")), "LOUD");
    assert_eq!(project_name(Path::new("/a/notes.svg")), "notes.svg");
    assert_eq!(project_name(Path::new("/a/.flatcraft")), ".flatcraft");

    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let path = dir.path().join("Stickers.flatcraft");
    {
        let mut st = s.lock().unwrap();
        st.execute("project.save", json!({ "path": path }), Origin::User)
            .unwrap();
        st.execute("document.new", json!({}), Origin::User).unwrap();
    }
    let mut h = harness(&dir, s.clone(), false);
    h.get_by_role_and_label(Role::Button, "Stickers");
    assert!(h.query_by_label("Stickers.flatcraft").is_none());
    h.get_by_role_and_label(Role::Button, "Close Stickers");
    h.get_by_role_and_label(Role::Button, "Untitled 2");
    // The unsaved-changes dialog uses the same name.
    s.lock().unwrap().docs[0].dirty = true;
    let id = s.lock().unwrap().docs[0].id();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Close(id)]);
    h.run_plan();
    h.get_by_label("Save changes to Stickers?");
}
