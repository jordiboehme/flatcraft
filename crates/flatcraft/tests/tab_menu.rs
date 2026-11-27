//! The right-click menu of a document tab.

mod common;
use common::RunPlan;

use common::{FakeRevealer, app, app_mut, copied, harness_with, session};
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::Unsaved;
use flatcraft::ui::app_bar::SAVE_FIRST;
use flatcraft::ui::files::reveal_menu_label;
use flatcraft_engine::session::Origin;
use serde_json::json;

/// Two documents: "Stickers" saved, then an untitled one (active).
fn rig() -> (
    tempfile::TempDir,
    flatcraft_mcp::SharedSession,
    common::AppHarness,
    FakeRevealer,
    std::path::PathBuf,
) {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let path = dir.path().join("Stickers.flatcraft");
    {
        let mut st = s.lock().unwrap();
        st.execute("project.save", json!({ "path": path }), Origin::User)
            .unwrap();
        st.execute("document.new", json!({}), Origin::User).unwrap();
    }
    let revealed = FakeRevealer::default();
    let r = revealed.clone();
    let h = harness_with(&dir, s.clone(), move |a| a.with_revealer(r));
    h.ctx
        .global_style_mut(|st| st.interaction.tooltip_delay = 0.0);
    (dir, s, h, revealed, path)
}

fn open_menu(h: &mut common::AppHarness, tab: &str) {
    h.get_by_role_and_label(Role::Button, tab).click_secondary();
    h.run_plan();
}

#[test]
fn reveal_and_copy_path_act_on_the_tab_file() {
    let (_dir, _s, mut h, revealed, path) = rig();
    open_menu(&mut h, "Stickers");
    h.get_by_role_and_label(Role::Button, reveal_menu_label())
        .click();
    h.run_plan();
    assert_eq!(*revealed.0.lock().unwrap(), std::slice::from_ref(&path));
    open_menu(&mut h, "Stickers");
    h.get_by_role_and_label(Role::Button, "Copy path").click();
    h.step();
    assert_eq!(copied(&h), Some(path.display().to_string()));
}

#[test]
fn an_untitled_tab_dims_reveal_and_copy_with_the_reason() {
    let (_dir, _s, mut h, revealed, _path) = rig();
    open_menu(&mut h, "Untitled 2");
    for item in [reveal_menu_label(), "Copy path"] {
        let node = h.get_by_role_and_label(Role::Button, item);
        assert!(node.accesskit_node().is_disabled(), "{item}");
        let at = node.rect().center();
        h.hover_at(at);
        h.run_plan();
        h.run_plan();
        h.get_by_label(SAVE_FIRST);
    }
    assert!(revealed.0.lock().unwrap().is_empty());
}

#[test]
fn close_closes_the_tab() {
    let (_dir, s, mut h, _r, _path) = rig();
    open_menu(&mut h, "Stickers");
    h.get_by_role_and_label(Role::Button, "Close").click();
    h.run_plan();
    assert_eq!(s.lock().unwrap().docs.len(), 1);
    assert!(s.lock().unwrap().doc().path.is_none());
}

#[test]
fn close_other_tabs_asks_for_each_dirty_document() {
    let (_dir, s, mut h, _r, _path) = rig();
    // Two more documents: one dirty, one clean. Stickers is dirty too.
    {
        let mut st = s.lock().unwrap();
        st.execute("document.new", json!({}), Origin::User).unwrap();
        st.doc_mut().dirty = true;
        st.execute("document.new", json!({}), Origin::User).unwrap();
        st.docs[0].dirty = true;
    }
    h.run_plan();
    let keep = s.lock().unwrap().docs[1].id();
    let (stickers, dirty) = {
        let st = s.lock().unwrap();
        (st.docs[0].id(), st.docs[2].id())
    };
    open_menu(&mut h, "Untitled 2");
    h.get_by_role_and_label(Role::Button, "Close other tabs")
        .click();
    h.run_plan();
    // In tab order: Stickers is asked about first, nothing closed yet.
    assert_eq!(app(&h).ui.unsaved, Some(Unsaved::Close(stickers)));
    assert_eq!(s.lock().unwrap().docs.len(), 4);
    h.get_by_role_and_label(Role::Button, "Don't Save").click();
    h.run_plan();
    assert_eq!(app(&h).ui.unsaved, Some(Unsaved::Close(dirty)));
    h.get_by_role_and_label(Role::Button, "Don't Save").click();
    h.run_plan();
    assert_eq!(app(&h).ui.unsaved, None);
    let st = s.lock().unwrap();
    assert_eq!(st.docs.len(), 1);
    assert_eq!(st.doc().id(), keep);
}

#[test]
fn cancel_stops_close_other_tabs() {
    let (_dir, s, mut h, _r, _path) = rig();
    {
        let mut st = s.lock().unwrap();
        st.execute("document.new", json!({}), Origin::User).unwrap();
        st.doc_mut().dirty = true;
        st.docs[0].dirty = true;
    }
    h.run_plan();
    let keep = s.lock().unwrap().docs[1].id();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![flatcraft::ui::Action::CloseOthers(keep)]);
    h.run_plan();
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run_plan();
    assert_eq!(app(&h).ui.unsaved, None);
    assert_eq!(s.lock().unwrap().docs.len(), 3, "nothing more was closed");
    assert!(app(&h).ui.close_queue.is_empty());
}

#[test]
fn no_tab_menu_while_a_dialog_is_open() {
    let (_dir, s, mut h, _r, _path) = rig();
    s.lock().unwrap().docs[0].dirty = true;
    let id = s.lock().unwrap().docs[0].id();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![flatcraft::ui::Action::Close(id)]);
    h.run_plan();
    h.get_by_role_and_label(Role::Button, "Stickers •")
        .click_secondary();
    h.run_plan();
    assert!(h.query_by_label("Close other tabs").is_none());
}
