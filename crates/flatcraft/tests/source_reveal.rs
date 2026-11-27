//! Reveal source and Copy source path, in the object context menu and in
//! the macOS menu bar's Object menu.

mod common;
use common::RunPlan;

use std::path::Path;

use common::{
    AppHarness, FakeRevealer, app, app_mut, harness_with, import_square, session, to_prepare,
    wait_for_workers,
};
use flatcraft::ui::context_menu::{self, Item, SOURCE_MISSING, SOURCE_MIXED};
use flatcraft::ui::menu_model::{Cmd, MenuInput, MenuModel};
use flatcraft::ui::{Action, Workspace};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::json;

fn write_square(path: &Path) {
    std::fs::write(
        path,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="20" y="20" width="30" height="30"/></svg>"#,
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

struct Rig {
    dir: tempfile::TempDir,
    s: SharedSession,
    h: AppHarness,
    revealed: FakeRevealer,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let revealed = FakeRevealer::default();
    let r = revealed.clone();
    let mut h = harness_with(&dir, s.clone(), move |a| a.with_revealer(r));
    to_prepare(&mut h);
    Rig {
        dir,
        s,
        h,
        revealed,
    }
}

/// The context menu's items for `ids`, flattened, with their reasons.
fn items(r: &mut Rig, ids: &[u64]) -> Vec<(Item, Option<&'static str>)> {
    let ctx = r.h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut r.h), &ctx);
    context_menu::items(&snap, &app(&r.h).ui.prepare, ids)
        .into_iter()
        .flatten()
        .map(|i| (i, i.why(&snap, &app(&r.h).ui.prepare, ids)))
        .collect()
}

/// Runs `item` on `ids` as a click does, then applies the actions.
fn run(r: &mut Rig, item: Item, ids: &[u64]) {
    let ctx = r.h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut r.h), &ctx);
    let mut actions = Vec::new();
    context_menu::run_item(
        item,
        &snap,
        &mut app_mut(&mut r.h).ui.prepare,
        ids,
        &mut actions,
    );
    app_mut(&mut r.h).apply(&ctx, actions);
}

fn reason(list: &[(Item, Option<&'static str>)], item: Item) -> Option<Option<&'static str>> {
    list.iter().find(|(i, _)| *i == item).map(|(_, w)| *w)
}

fn model(r: &mut Rig) -> MenuModel {
    let ctx = r.h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut r.h), &ctx);
    let a = app(&r.h);
    MenuModel::build(&MenuInput {
        workspace: Workspace::Prepare,
        snap: Some(&snap),
        prepare: &a.ui.prepare,
        show_debug: false,
        can_undo: true,
        can_redo: false,
        text_focused: false,
        send_ready: false,
        recent: &[],
        modal: false,
    })
}

fn enabled(m: &MenuModel, cmd: &Cmd) -> bool {
    m.leaves()
        .into_iter()
        .find(|l| &l.cmd == cmd)
        .unwrap_or_else(|| panic!("{cmd:?}"))
        .enabled
}

fn select(s: &SharedSession, ids: &[u64]) {
    s.lock()
        .unwrap()
        .execute("object.select", json!({ "ids": ids }), Origin::User)
        .unwrap();
}

#[test]
fn a_linked_file_that_exists_is_revealed_and_copied() {
    let mut r = rig();
    let file = r.dir.path().join("a.svg");
    write_square(&file);
    let id = import_file(&r.s, &file);
    // Two linked copies of one file act on that file.
    let twin = import_file(&r.s, &file);
    let ids = [id, twin];
    let list = items(&mut r, &ids);
    assert_eq!(reason(&list, Item::RevealSource), Some(None));
    assert_eq!(reason(&list, Item::CopySourcePath), Some(None));
    run(&mut r, Item::RevealSource, &ids);
    assert_eq!(*r.revealed.0.lock().unwrap(), vec![file.clone()]);
    run(&mut r, Item::CopySourcePath, &ids);
    let toast = app(&r.h).ui.toasts.last().unwrap();
    assert_eq!(toast.text, "Path copied");
    assert_eq!(
        toast.detail.as_deref(),
        Some(file.display().to_string().as_str())
    );
    // The menu bar offers the same, through the same items.
    select(&r.s, &ids);
    r.h.run_plan();
    let m = model(&mut r);
    assert!(enabled(&m, &Cmd::RevealSource));
    assert!(enabled(&m, &Cmd::CopySourcePath));
    let ctx = r.h.ctx.clone();
    let action = Cmd::RevealSource.action(0, &[]).unwrap();
    assert_eq!(action, Action::Menu(Cmd::RevealSource));
    app_mut(&mut r.h).apply(&ctx, vec![action]);
    assert_eq!(r.revealed.0.lock().unwrap().len(), 2);
}

#[test]
fn a_missing_file_is_dimmed_with_its_reason() {
    let mut r = rig();
    let file = r.dir.path().join("gone.svg");
    write_square(&file);
    let id = import_file(&r.s, &file);
    std::fs::remove_file(&file).unwrap();
    // The source check (off the UI thread) finds it gone.
    r.h.event(egui::Event::WindowFocused(false));
    r.h.run_plan();
    r.h.event(egui::Event::WindowFocused(true));
    wait_for_workers(&mut r.h);
    let list = items(&mut r, &[id]);
    assert_eq!(
        reason(&list, Item::RevealSource),
        Some(Some(SOURCE_MISSING))
    );
    assert_eq!(
        reason(&list, Item::CopySourcePath),
        Some(Some(SOURCE_MISSING))
    );
    run(&mut r, Item::RevealSource, &[id]);
    assert!(r.revealed.0.lock().unwrap().is_empty());
    select(&r.s, &[id]);
    r.h.run_plan();
    let m = model(&mut r);
    assert!(!enabled(&m, &Cmd::RevealSource));
    assert!(!enabled(&m, &Cmd::CopySourcePath));
}

#[test]
fn an_object_without_a_file_never_shows_the_items() {
    let mut r = rig();
    let id = import_square(&r.s);
    let list = items(&mut r, &[id]);
    assert_eq!(reason(&list, Item::RevealSource), None);
    assert_eq!(reason(&list, Item::CopySourcePath), None);
    select(&r.s, &[id]);
    r.h.run_plan();
    let m = model(&mut r);
    assert!(!enabled(&m, &Cmd::RevealSource));
}

#[test]
fn objects_from_different_files_are_dimmed() {
    let mut r = rig();
    let a = r.dir.path().join("a.svg");
    let b = r.dir.path().join("b.svg");
    write_square(&a);
    write_square(&b);
    let ia = import_file(&r.s, &a);
    let ib = import_file(&r.s, &b);
    let plain = import_square(&r.s);
    let list = items(&mut r, &[ia, ib]);
    assert_eq!(reason(&list, Item::RevealSource), Some(Some(SOURCE_MIXED)));
    assert_eq!(
        reason(&list, Item::CopySourcePath),
        Some(Some(SOURCE_MIXED))
    );
    // An object without a file does not count: one file is left.
    let list = items(&mut r, &[ia, plain]);
    assert_eq!(reason(&list, Item::RevealSource), Some(None));
    select(&r.s, &[ia, ib]);
    r.h.run_plan();
    let m = model(&mut r);
    assert!(!enabled(&m, &Cmd::CopySourcePath));
}

#[test]
fn a_project_folder_that_moved_as_a_whole_still_finds_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let old = dir.path().join("old");
    std::fs::create_dir(&old).unwrap();
    let file = old.join("a.svg");
    write_square(&file);
    import_file(&s, &file);
    s.lock()
        .unwrap()
        .execute(
            "project.save",
            json!({ "path": old.join("p.flatcraft") }),
            Origin::User,
        )
        .unwrap();
    let new = dir.path().join("new");
    std::fs::rename(&old, &new).unwrap();
    s.lock()
        .unwrap()
        .execute(
            "project.open",
            json!({ "path": new.join("p.flatcraft") }),
            Origin::User,
        )
        .unwrap();
    let id = s.lock().unwrap().doc().document.objects[0].id.0;
    let revealed = FakeRevealer::default();
    let r = revealed.clone();
    let mut h = harness_with(&dir, s.clone(), move |a| a.with_revealer(r));
    to_prepare(&mut h);
    let mut r = Rig {
        dir,
        s,
        h,
        revealed,
    };
    let list = items(&mut r, &[id]);
    assert_eq!(reason(&list, Item::RevealSource), Some(None), "{list:?}");
    run(&mut r, Item::RevealSource, &[id]);
    let shown = r.revealed.0.lock().unwrap().clone();
    assert_eq!(shown.len(), 1);
    assert!(shown[0].ends_with("new/a.svg"), "{shown:?}");
}

#[test]
fn twelve_copies_of_one_file_touch_no_file_per_frame() {
    let mut r = rig();
    let file = r.dir.path().join("a.svg");
    write_square(&file);
    let id = import_file(&r.s, &file);
    r.s.lock()
        .unwrap()
        .execute(
            "object.copies",
            json!({ "id": id, "count": 11, "layout": { "grid": { "columns": 4 } }, "linked": true }),
            Origin::User,
        )
        .unwrap();
    let ids: Vec<u64> =
        r.s.lock()
            .unwrap()
            .doc()
            .document
            .objects
            .iter()
            .map(|o| o.id.0)
            .collect();
    assert_eq!(ids.len(), 12);
    select(&r.s, &ids);
    r.h.run_plan();
    let calls = flatcraft_engine::commands::sources::find_linked_calls();
    for _ in 0..10 {
        r.h.step();
        // What the context menu and the menu bar ask each frame.
        let list = items(&mut r, &ids);
        assert_eq!(reason(&list, Item::RevealSource), Some(None));
        let m = model(&mut r);
        assert!(enabled(&m, &Cmd::RevealSource));
    }
    let after = flatcraft_engine::commands::sources::find_linked_calls();
    assert_eq!(after, calls, "no file was looked at");
    // A click finds the file once.
    run(&mut r, Item::RevealSource, &ids);
    assert_eq!(
        flatcraft_engine::commands::sources::find_linked_calls(),
        calls + 1
    );
    assert_eq!(*r.revealed.0.lock().unwrap(), vec![file]);
}

#[test]
fn a_click_on_a_file_gone_since_the_last_check_says_so_and_copies_nothing() {
    let mut r = rig();
    let file = r.dir.path().join("a.svg");
    write_square(&file);
    let id = import_file(&r.s, &file);
    // Gone, and the source check has not run since.
    std::fs::remove_file(&file).unwrap();
    let list = items(&mut r, &[id]);
    assert_eq!(reason(&list, Item::CopySourcePath), Some(None));
    for item in [Item::RevealSource, Item::CopySourcePath] {
        run(&mut r, item, &[id]);
        let toast = app(&r.h).ui.toasts.last().unwrap();
        assert_eq!(toast.text, SOURCE_MISSING);
        assert_eq!(toast.kind, flatcraft::ui::toasts::ToastKind::Error);
    }
    assert!(r.revealed.0.lock().unwrap().is_empty());
    assert!(
        !app(&r.h).ui.toasts.iter().any(|t| t.text == "Path copied"),
        "no dead path was copied"
    );
}

/// A file manager that refuses at once, or later.
#[derive(Clone, Default)]
struct FailingRevealer {
    later: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    at_once: bool,
}

impl flatcraft::ui::files::Revealer for FailingRevealer {
    fn reveal(&mut self, _file: &Path) -> Result<(), String> {
        if self.at_once {
            return Err("no file manager".into());
        }
        self.later.lock().unwrap().push("Finder said no".into());
        Ok(())
    }

    fn failures(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.later.lock().unwrap())
    }
}

#[test]
fn a_reveal_that_fails_shows_why() {
    for at_once in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        let file = dir.path().join("a.svg");
        write_square(&file);
        let id = import_file(&s, &file);
        let fail = FailingRevealer {
            at_once,
            ..Default::default()
        };
        let mut h = harness_with(&dir, s.clone(), move |a| a.with_revealer(fail));
        to_prepare(&mut h);
        let mut r = Rig {
            dir,
            s,
            h,
            revealed: FakeRevealer::default(),
        };
        run(&mut r, Item::RevealSource, &[id]);
        r.h.run_plan();
        let texts: Vec<String> = app(&r.h).ui.toasts.iter().map(|t| t.text.clone()).collect();
        let want = if at_once {
            "no file manager"
        } else {
            "Finder said no"
        };
        assert!(
            texts.iter().any(|t| t.contains(want)),
            "{at_once}: {texts:?}"
        );
    }
}
