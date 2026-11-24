//! The app shell, driven through egui_kittest.

use std::sync::{Arc, Mutex};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::{self, FlatCraftApp, Workspace, fonts};
use flatcraft_core::document::ObjectId;
use flatcraft_device::ProfileStore;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{Origin, Session};
use flatcraft_mcp::SharedSession;
use serde_json::json;

fn session(dir: &tempfile::TempDir) -> SharedSession {
    let profiles = ProfileStore::load(&dir.path().join("profiles")).unwrap();
    // Never the real Documents folder.
    let prefs = Preferences {
        output_dir: Some(dir.path().join("output")),
        ..Preferences::default()
    };
    Arc::new(Mutex::new(Session::new(
        profiles,
        prefs,
        true,
        dir.path().to_path_buf(),
    )))
}

/// A harness around `ui::shell`. The app is built with the harness context
/// after the fonts are in, as `main` does with eframe's context.
fn harness(
    dir: &tempfile::TempDir,
    s: SharedSession,
    agent: bool,
) -> Harness<'static, Option<FlatCraftApp>> {
    let mut h = Harness::builder()
        .with_size([1400.0, 900.0])
        .with_context_setup(fonts::install)
        .build_ui_state(
            |ui, app: &mut Option<FlatCraftApp>| {
                if let Some(app) = app {
                    ui::shell(ui, app);
                }
            },
            None,
        );
    let ctx = h.ctx.clone();
    fonts::install(&ctx);
    let app = FlatCraftApp::new(&ctx, s, dir.path().to_path_buf());
    *h.state_mut() = Some(if agent { app.with_agent() } else { app });
    h.run();
    h
}

fn app<'a>(h: &'a Harness<'_, Option<FlatCraftApp>>) -> &'a FlatCraftApp {
    h.state().as_ref().unwrap()
}

#[test]
fn switcher_changes_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    h.check_accessibility();
    h.get_by_role_and_label(Role::Button, "Workspace").click();
    h.run();
    // The open popup and its cards have accessible names too.
    h.check_accessibility();
    h.get_by_role_and_label(Role::Button, "Prepare");
    h.get_by_role_and_label(Role::Button, "Cut").click();
    h.run();
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
    h.run();
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
    h.run();
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
    let mut h = harness(&dir, s.clone(), false);
    assert_eq!(app(&h).ui.workspace, Workspace::Home);
    h.get_by_role_and_label(Role::Button, "New");
    h.get_by_role_and_label(Role::Button, "Open");
    h.get_by_role_and_label(Role::Button, "coasters.flatcraft");
    // A missing recent project gives a toast, not a panic.
    h.get_by_role_and_label(Role::Button, "coasters.flatcraft")
        .click();
    h.run();
    assert_eq!(app(&h).ui.toasts.len(), 1);
}

#[test]
fn home_new_reuses_the_empty_start_document() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    h.get_by_role_and_label(Role::Button, "New").click();
    h.run();
    assert_eq!(s.lock().unwrap().docs.len(), 1);
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);

    // A document with content stays; New opens a second one.
    h.get_by_role_and_label(Role::Button, "Home").click();
    h.run();
    s.lock().unwrap().doc_mut().dirty = true;
    h.get_by_role_and_label(Role::Button, "New").click();
    h.run();
    assert_eq!(s.lock().unwrap().docs.len(), 2);
}
