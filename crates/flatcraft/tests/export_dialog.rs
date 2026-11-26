//! The Export dialog, File > Export... in the app bar and the menu bar,
//! and the toasts of an export, driven through egui_kittest.

mod common;

use std::path::PathBuf;

use common::{
    AppHarness, FakePicker, app, app_mut, harness, harness_with, import_square, session, to_prepare,
};
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::export_dialog::{self, ExportDialog};
use flatcraft::ui::files::filter;
use flatcraft::ui::menu_model::{Cmd, MenuInput, MenuModel};
use flatcraft::ui::toasts::{Toast, ToastKind};
use flatcraft::ui::{Action, Workspace};
use flatcraft_engine::export::{ExportFormat, ExportScope};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

fn exec(s: &SharedSession, cmd: &str, params: Value) -> Value {
    s.lock()
        .unwrap()
        .execute(cmd, params, Origin::User)
        .unwrap()
}

fn doc_id(s: &SharedSession) -> u64 {
    s.lock().unwrap().doc().id()
}

/// Prepare with `n` squares and a picker that answers `answers`.
fn rig(
    n: usize,
    answers: Vec<PathBuf>,
) -> (
    tempfile::TempDir,
    SharedSession,
    AppHarness,
    FakePicker,
    Vec<u64>,
) {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let ids: Vec<u64> = (0..n).map(|_| import_square(&s)).collect();
    exec(&s, "object.select", json!({ "ids": [] }));
    let picker = FakePicker::answering(answers);
    let p = picker.clone();
    let mut h = harness_with(&dir, s.clone(), move |app| app.with_file_picker(p));
    to_prepare(&mut h);
    h.run();
    (dir, s, h, picker, ids)
}

fn open_from_app_bar(h: &mut AppHarness) {
    h.get_by_label("File").click();
    h.run();
    h.get_by_label(flatcraft::ui::menu_model::EXPORT_TITLE)
        .click();
    h.run();
    h.run();
}

fn dialog_open(h: &AppHarness) -> bool {
    h.query_by_label_contains(export_dialog::TITLE).is_some()
}

fn disabled(h: &AppHarness, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

fn click_export(h: &mut AppHarness) {
    // The dialog's primary button (the File menu is closed by now).
    h.get_by_role_and_label(Role::Button, export_dialog::EXPORT)
        .click();
    h.run();
    h.run();
}

fn last_toast(h: &AppHarness) -> Option<Toast> {
    app(h).ui.toasts.last().cloned()
}

// ---------------------------------------------------------------- dialog

#[test]
fn file_export_writes_the_chosen_format_with_the_document_name() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.dxf");
    let (_d, _s, mut h, picker, _) = rig(1, vec![out.clone()]);
    open_from_app_bar(&mut h);
    assert!(dialog_open(&h));
    // A dialog owns the keyboard.
    assert!(flatcraft::ui::modal_open(app(&h)));
    // Nothing selected: All objects, and Selected objects says why not.
    assert!(disabled(&h, export_dialog::SELECTED));
    h.get_by_label("DXF").click();
    h.run();
    assert_eq!(
        app(&h).ui.export.as_ref().unwrap().format,
        ExportFormat::Dxf
    );
    click_export(&mut h);
    assert!(!dialog_open(&h));
    let reqs = picker.requests.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].title, "Export");
    assert_eq!(reqs[0].filters, vec![filter("DXF", &["dxf"])]);
    assert_eq!(reqs[0].file_name.as_deref(), Some("Untitled 1.dxf"));
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains("AC1009"), "{text}");
    let toast = last_toast(&h).unwrap();
    assert_eq!(toast.kind, ToastKind::Success);
    assert_eq!(toast.detail.as_deref(), Some("Wrote 1 shape to out.dxf"));
    assert_eq!(toast.reveal.as_deref(), Some(out.as_path()));
}

#[test]
fn the_default_name_follows_the_project_file() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("Star Badge.flatcraft");
    let out = dir.path().join("x.pdf");
    let (_d, s, mut h, picker, _) = rig(1, vec![out.clone()]);
    exec(&s, "project.save", json!({ "path": project }));
    h.run();
    let doc = doc_id(&s);
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(
        &ctx,
        vec![Action::Export(ExportDialog {
            doc_id: doc,
            format: ExportFormat::Pdf,
            scope: ExportScope::All,
            include_skipped: false,
        })],
    );
    let reqs = picker.requests.lock().unwrap().clone();
    assert_eq!(reqs[0].file_name.as_deref(), Some("Star Badge.pdf"));
    assert_eq!(reqs[0].directory.as_deref(), Some(dir.path()));
    assert!(std::fs::read(&out).unwrap().starts_with(b"%PDF-"));
}

#[test]
fn cancel_in_the_save_dialog_writes_nothing() {
    let (_d, _s, mut h, picker, _) = rig(1, vec![]);
    open_from_app_bar(&mut h);
    let before = app(&h).ui.toasts.len();
    click_export(&mut h);
    assert_eq!(picker.requests.lock().unwrap().len(), 1);
    assert_eq!(app(&h).ui.toasts.len(), before);
}

#[test]
fn a_failed_write_shows_an_error_toast_with_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("missing-folder").join("x.svg");
    let (_d, _s, mut h, _p, _) = rig(1, vec![out.clone()]);
    open_from_app_bar(&mut h);
    click_export(&mut h);
    let toast = last_toast(&h).unwrap();
    assert_eq!(toast.kind, ToastKind::Error, "{toast:?}");
    assert!(toast.text.contains("x.svg"), "{}", toast.text);
    assert!(!out.exists());
}

#[test]
fn the_scope_starts_on_the_selection_and_can_take_all() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("x.svg");
    let (_d, s, mut h, _p, ids) = rig(2, vec![out.clone(), out.clone()]);
    exec(&s, "object.select", json!({ "ids": [ids[0]] }));
    h.run();
    open_from_app_bar(&mut h);
    assert_eq!(
        app(&h).ui.export.as_ref().unwrap().scope,
        ExportScope::Selected
    );
    click_export(&mut h);
    let svg = std::fs::read_to_string(&out).unwrap();
    assert_eq!(svg.matches("<g ").count(), 1);

    open_from_app_bar(&mut h);
    h.get_by_label(export_dialog::ALL).click();
    h.run();
    click_export(&mut h);
    let svg = std::fs::read_to_string(&out).unwrap();
    assert_eq!(svg.matches("<g ").count(), 2);
}

#[test]
fn skipped_objects_dim_export_until_the_box_is_ticked() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("x.svg");
    let (_d, s, mut h, _p, ids) = rig(1, vec![out.clone()]);
    exec(
        &s,
        "object.set_cut",
        json!({ "ids": [ids[0]], "cut": false }),
    );
    h.run();
    open_from_app_bar(&mut h);
    assert!(disabled(&h, export_dialog::EXPORT));
    assert!(h.query_by_label(export_dialog::ALL_SKIPPED).is_some());
    assert!(!disabled(&h, export_dialog::INCLUDE_SKIPPED));
    h.get_by_label(export_dialog::INCLUDE_SKIPPED).click();
    h.run();
    assert!(!disabled(&h, export_dialog::EXPORT));
    click_export(&mut h);
    assert!(out.exists());
}

#[test]
fn the_include_box_is_dimmed_when_nothing_is_skipped() {
    let (_d, _s, mut h, _p, _) = rig(1, vec![]);
    open_from_app_bar(&mut h);
    assert!(disabled(&h, export_dialog::INCLUDE_SKIPPED));
    assert!(!disabled(&h, export_dialog::EXPORT));
}

#[test]
fn escape_closes_the_dialog() {
    let (_d, _s, mut h, _p, _) = rig(1, vec![]);
    open_from_app_bar(&mut h);
    assert!(dialog_open(&h));
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(!dialog_open(&h));
}

// ---------------------------------------------------------------- menus

fn model(h: &mut AppHarness, workspace: Workspace) -> MenuModel {
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(h), &ctx);
    let a = app(h);
    MenuModel::build(&MenuInput {
        workspace,
        snap: (workspace != Workspace::Home).then_some(&snap),
        prepare: &a.ui.prepare,
        show_debug: false,
        can_undo: false,
        can_redo: false,
        text_focused: false,
        send_ready: false,
        recent: &[],
        modal: false,
    })
}

fn export_enabled(m: &MenuModel) -> bool {
    m.leaves()
        .into_iter()
        .find(|l| l.cmd == Cmd::Export)
        .expect("File > Export...")
        .enabled
}

#[test]
fn the_menu_bar_has_file_export() {
    let (_d, s, mut h, _p, ids) = rig(1, vec![]);
    let m = model(&mut h, Workspace::Prepare);
    let leaf = m
        .leaves()
        .into_iter()
        .find(|l| l.cmd == Cmd::Export)
        .unwrap();
    assert_eq!(leaf.title, "Export...");
    assert!(leaf.enabled);
    assert_eq!(Cmd::parse(&Cmd::Export.id()), Some(Cmd::Export));
    let doc = doc_id(&s);
    assert_eq!(
        Cmd::Export.action(doc, &[]),
        Some(Action::ExportDialog { doc_id: doc })
    );
    assert!(export_enabled(&model(&mut h, Workspace::Cut)));
    assert!(!export_enabled(&model(&mut h, Workspace::Home)));
    // Only hidden objects: nothing to export.
    exec(
        &s,
        "object.set_visible",
        json!({ "ids": [ids[0]], "visible": false }),
    );
    h.run();
    assert!(!export_enabled(&model(&mut h, Workspace::Prepare)));
    // A skipped object can still be exported with the box.
    exec(
        &s,
        "object.set_visible",
        json!({ "ids": [ids[0]], "visible": true }),
    );
    exec(
        &s,
        "object.set_cut",
        json!({ "ids": [ids[0]], "cut": false }),
    );
    h.run();
    assert!(export_enabled(&model(&mut h, Workspace::Prepare)));
}

#[test]
fn the_menu_bar_item_opens_the_dialog() {
    let (_d, s, mut h, _p, _) = rig(1, vec![]);
    let doc = doc_id(&s);
    let action = Cmd::Export.action(doc, &[]).unwrap();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![action]);
    h.run();
    assert!(dialog_open(&h));
}

#[test]
fn the_app_bar_export_is_dimmed_without_objects_and_on_home() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.run();
    h.get_by_label("File").click();
    h.run();
    assert!(disabled(&h, flatcraft::ui::menu_model::EXPORT_TITLE));
    // From Home, the action says what to do instead of opening.
    app_mut(&mut h).ui.workspace = Workspace::Home;
    let doc = doc_id(&s);
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::ExportDialog { doc_id: doc }]);
    assert!(app(&h).ui.export.is_none());
    assert_eq!(
        last_toast(&h).unwrap().text,
        export_dialog::NO_DOCUMENT.to_string()
    );
}

#[test]
fn export_selected_opens_on_the_selection() {
    let (_d, s, mut h, _p, ids) = rig(2, vec![]);
    exec(&s, "object.select", json!({ "ids": [ids[1]] }));
    h.run();
    let doc = doc_id(&s);
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::ExportDialog { doc_id: doc }]);
    h.run();
    assert_eq!(
        app(&h).ui.export.as_ref().unwrap().scope,
        ExportScope::Selected
    );
    assert!(!disabled(&h, export_dialog::SELECTED));
}

// ---------------------------------------------------------------- cards

fn hover_button(h: &mut AppHarness, label: &str) {
    h.hover_at(egui::pos2(1.0, 1.0));
    h.run();
    let at = h.get_by_role_and_label(Role::Button, label).rect().center();
    h.hover_at(at);
    for _ in 0..6 {
        h.run();
    }
}

#[test]
fn a_dimmed_export_says_why_on_hover() {
    let (_d, s, mut h, _p, ids) = rig(1, vec![]);
    exec(
        &s,
        "object.set_cut",
        json!({ "ids": [ids[0]], "cut": false }),
    );
    h.run();
    open_from_app_bar(&mut h);
    assert!(disabled(&h, export_dialog::EXPORT));
    // The warning line under the controls says it once; the card again.
    assert_eq!(h.query_all_by_label(export_dialog::ALL_SKIPPED).count(), 1);
    assert!(h.query_by_label(export_dialog::EXPORT_HINT).is_none());
    hover_button(&mut h, export_dialog::EXPORT);
    assert_eq!(h.query_all_by_label(export_dialog::ALL_SKIPPED).count(), 2);
    assert!(h.query_by_label(export_dialog::EXPORT_HINT).is_some());
}

#[test]
fn cancel_has_a_card() {
    let (_d, _s, mut h, _p, _) = rig(1, vec![]);
    open_from_app_bar(&mut h);
    hover_button(&mut h, "Cancel");
    assert!(h.query_by_label(export_dialog::CANCEL_HINT).is_some());
    assert!(dialog_open(&h));
}
