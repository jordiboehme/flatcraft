//! Home's Start page: the New cards, the recent projects and the samples.

mod common;

use common::{
    FakePicker, SQUARE_SVG, app, harness_sized, harness_with, session, wait_for_source_check,
};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::Workspace;

#[test]
fn new_project_card_starts_an_empty_project_in_prepare() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness_with(&dir, s.clone(), |a| a);
    h.get_by_role_and_label(Role::Button, "New project").click();
    wait_for_source_check(&mut h);
    let st = s.lock().unwrap();
    assert_eq!(st.docs.len(), 1, "the empty start document is used");
    assert!(st.doc().document.objects.is_empty());
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
}

#[test]
fn import_card_asks_first_and_imports_into_a_new_project() {
    let dir = tempfile::tempdir().unwrap();
    let svg = dir.path().join("star.svg");
    std::fs::write(&svg, SQUARE_SVG).unwrap();
    let s = session(&dir);
    // The first document has content, so the import gets a new one.
    s.lock().unwrap().doc_mut().dirty = true;
    let picker = FakePicker::answering([svg.clone()]);
    let requests = picker.requests.clone();
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker));
    h.get_by_role_and_label(Role::Button, "Import design...")
        .click();
    wait_for_source_check(&mut h);
    {
        let st = s.lock().unwrap();
        assert_eq!(st.docs.len(), 2);
        assert_eq!(st.doc().document.objects.len(), 1);
        assert!(st.docs[0].document.objects.is_empty());
    }
    assert_eq!(requests.lock().unwrap()[0].title, "Import");
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);

    // Cancelled: no new project, Home stays.
    h.get_by_role_and_label(Role::Button, "Home").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Import design...")
        .click();
    h.run();
    assert_eq!(s.lock().unwrap().docs.len(), 2);
    assert_eq!(app(&h).ui.workspace, Workspace::Home);
}

#[test]
fn open_card_opens_the_file_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let path = dir.path().join("coasters.flatcraft");
    s.lock()
        .unwrap()
        .execute(
            "project.save",
            serde_json::json!({ "path": path }),
            flatcraft_engine::session::Origin::User,
        )
        .unwrap();
    let picker = FakePicker::answering([path.clone()]);
    let requests = picker.requests.clone();
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker));
    h.get_by_role_and_label(Role::Button, "Open...").click();
    wait_for_source_check(&mut h);
    assert_eq!(requests.lock().unwrap()[0].title, "Open");
    assert_eq!(
        s.lock().unwrap().doc().path.as_deref(),
        Some(path.as_path())
    );
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
}

#[test]
fn test_patterns_card_makes_the_test_sheet_in_a_new_project() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness_with(&dir, s.clone(), |a| a);
    h.get_by_role_and_label(Role::Button, "Test patterns")
        .click();
    wait_for_source_check(&mut h);
    let st = s.lock().unwrap();
    assert_eq!(st.doc().document.objects.len(), 1);
    assert_eq!(st.doc().document.objects[0].name, "Test patterns");
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
    assert!(app(&h).ui.toasts.is_empty(), "{:?}", app(&h).ui.toasts);
}

#[test]
fn the_new_cards_wrap_at_the_smallest_window() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness_sized(&dir, session(&dir), [900.0, 600.0], 8, |a| a);
    let new = h.get_by_role_and_label(Role::Button, "New project").rect();
    let import = h
        .get_by_role_and_label(Role::Button, "Import design...")
        .rect();
    let open = h.get_by_role_and_label(Role::Button, "Open...").rect();
    // Two columns: Open starts the second row.
    assert!((new.top() - import.top()).abs() < 0.5);
    assert!(open.top() > new.bottom());
    assert!((open.left() - new.left()).abs() < 0.5);
    assert!(import.right() <= 900.0);
}
