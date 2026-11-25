//! Reveal source and Copy source path, in the object context menu and in
//! the macOS menu bar's Object menu.

mod common;

use std::path::Path;

use common::{
    AppHarness, FakeRevealer, app, app_mut, harness_with, import_square, session, to_prepare,
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
        .map(|i| (i, i.why(&snap, ids)))
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
    r.h.run();
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
    r.h.run();
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
    r.h.run();
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
    r.h.run();
    let m = model(&mut r);
    assert!(!enabled(&m, &Cmd::CopySourcePath));
}
