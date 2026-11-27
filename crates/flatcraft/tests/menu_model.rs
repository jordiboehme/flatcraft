//! The macOS menu bar as data: titles, order, keys and states of
//! `MenuModel`, built as a pure function and tested without NSApp. The
//! muda glue (`ui::menu`) is macOS only and has its own small tests.

mod common;
use common::RunPlan;

use std::path::PathBuf;

use common::{
    AppHarness, app, app_mut, harness, import_square, session, settle, to_prepare, undo_len,
};
use flatcraft::ui::help::{self, SHORTCUTS};
use flatcraft::ui::menu_model::{Cmd, Entry, Leaf, MenuInput, MenuModel, Sub};
use flatcraft::ui::tools::{self, Act, TOOLS};
use flatcraft::ui::{Action, Workspace};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::json;

fn select(s: &SharedSession, ids: &[u64]) {
    s.lock()
        .unwrap()
        .execute("object.select", json!({ "ids": ids }), Origin::User)
        .unwrap();
}

/// Three squares; the first `n` are selected. Returns the harness and ids.
fn with_selection(n: usize) -> (tempfile::TempDir, SharedSession, AppHarness, Vec<u64>) {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let ids: Vec<u64> = (0..3).map(|_| import_square(&s)).collect();
    select(&s, &ids[..n]);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.run_plan();
    (dir, s, h, ids)
}

fn model_of(h: &mut AppHarness, recent: &[PathBuf]) -> MenuModel {
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(h), &ctx);
    let a = app(h);
    MenuModel::build(&MenuInput {
        workspace: a.ui.workspace,
        snap: Some(&snap),
        prepare: &a.ui.prepare,
        show_debug: a.ui.show_debug,
        can_undo: true,
        can_redo: false,
        text_focused: false,
        send_ready: false,
        recent,
        modal: false,
    })
}

fn home_model(h: &AppHarness) -> MenuModel {
    let a = app(h);
    MenuModel::build(&MenuInput {
        workspace: Workspace::Home,
        snap: None,
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

fn leaf<'a>(m: &'a MenuModel, cmd: &Cmd) -> &'a Leaf {
    m.leaves()
        .into_iter()
        .find(|l| &l.cmd == cmd)
        .unwrap_or_else(|| panic!("{cmd:?}"))
}

fn tool<'a>(m: &'a MenuModel, id: &str) -> &'a Leaf {
    leaf(m, &Cmd::Tool(tools::by_id(id).unwrap().id))
}

fn menu<'a>(m: &'a MenuModel, id: &str) -> &'a Sub {
    m.menus.iter().find(|s| s.id == id).unwrap()
}

fn titles(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .map(|e| match e {
            Entry::Item(l) => l.title.clone(),
            Entry::Sub(s) => format!("{} >", s.title),
            Entry::Separator => "-".into(),
            Entry::Predef(p) => format!("{p:?}"),
        })
        .collect()
}

#[test]
fn the_menus_and_their_items_come_in_order() {
    let (_d, _s, mut h, _) = with_selection(1);
    let m = model_of(&mut h, &[]);
    let tops: Vec<&str> = m.menus.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(
        tops,
        [
            "FlatCraft",
            "File",
            "Edit",
            "Object",
            "View",
            "Window",
            "Help"
        ]
    );
    assert_eq!(
        titles(&menu(&m, "file").entries),
        [
            "New",
            "Open...",
            "Open Recent >",
            "-",
            "Save",
            "Save As...",
            "Export...",
            "Close",
            "-",
            "Import...",
            "Test patterns",
            "Send to File Output"
        ]
    );
    assert_eq!(
        titles(&menu(&m, "edit").entries),
        ["Undo", "Redo", "-", "Cut", "Copy", "Paste", "Select All"]
    );
    assert_eq!(
        titles(&menu(&m, "object").entries),
        [
            "Duplicate",
            "Copies...",
            "Rectangular pattern...",
            "Circular pattern...",
            "Transform >",
            "Arrange >",
            "-",
            "Group",
            "Ungroup",
            "-",
            "Bring to front",
            "Bring forward",
            "Send backward",
            "Send to back",
            "-",
            "Link copies",
            "Unlink",
            "-",
            "Skip cutting",
            "Hide",
            "Pick what stays...",
            "Rename",
            "-",
            "Trace...",
            "Reload",
            "Reconnect source...",
            flatcraft::ui::files::reveal_source_label(),
            "Copy source path",
            "-",
            "Delete"
        ]
    );
    let submenu = |id: &str| {
        let Some(Entry::Sub(sub)) = menu(&m, "object")
            .entries
            .iter()
            .find(|e| matches!(e, Entry::Sub(s) if s.id == id))
        else {
            panic!("no {id} submenu")
        };
        titles(&sub.entries)
    };
    assert_eq!(
        submenu("transform"),
        [
            "Move / Copy...",
            "Rotate...",
            "Rotate -90",
            "Rotate +90",
            "-",
            "Mirror horizontally",
            "Mirror vertically",
        ]
    );
    assert_eq!(
        submenu("arrange"),
        ["Align >", "Distribute >", "-", "Auto arrange..."]
    );
    assert_eq!(
        titles(&menu(&m, "view").entries),
        [
            "Prepare",
            "Cut",
            "-",
            "Zoom to fit",
            "Zoom 100%",
            "-",
            "Grid",
            "Rulers",
            "Show kept and waste",
            "-",
            "Objects panel",
            "Settings panel"
        ]
    );
    assert_eq!(
        titles(&menu(&m, "window").entries),
        ["Minimize", "Zoom", "-", "BringAllToFront"]
    );
    assert_eq!(
        titles(&menu(&m, "help").entries),
        [
            "User guide",
            "Keyboard shortcuts",
            "It didn't cut right...",
            "Report a problem",
            "Ask for your plotter",
            "About FlatCraft"
        ]
    );
}

/// Align is one submenu with the two options below it, named by the table.
#[test]
fn align_lists_both_options_with_the_six_directions() {
    let (_d, _s, mut h, _) = with_selection(2);
    let m = model_of(&mut h, &[]);
    let align = m.subs().into_iter().find(|s| s.id == "align").unwrap();
    assert_eq!(
        titles(&align.entries),
        ["To the selection >", "To the cutting area >"]
    );
    for s in m.subs().into_iter().filter(|s| s.id.starts_with("align-")) {
        assert_eq!(
            titles(&s.entries),
            [
                "Align left",
                "Align centre",
                "Align right",
                "Align top",
                "Align middle",
                "Align bottom"
            ]
        );
    }
}

#[test]
fn titles_and_reasons_come_from_the_action_table() {
    for n in 0..=3 {
        let (_d, _s, mut h, _) = with_selection(n);
        let ctx = h.ctx.clone();
        let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
        let m = model_of(&mut h, &[]);
        for l in m.leaves() {
            let Cmd::Tool(id) = l.cmd else { continue };
            let t = tools::by_id(id).unwrap();
            assert_eq!(l.title, t.title);
            assert_eq!(l.enabled, t.blocked(&snap).is_none(), "{id} with {n}");
        }
    }
}

#[test]
fn enabled_states_follow_the_selection() {
    // (item, enabled with 0, 1, 2 and 3 objects selected)
    let tool_rows: [(&str, [bool; 4]); 9] = [
        ("duplicate", [false, true, true, true]),
        ("copies", [false, true, false, false]),
        ("rotate", [false, true, true, true]),
        ("mirror.horizontal", [false, true, true, true]),
        ("align.left.selection", [false, false, true, true]),
        ("align.left.area", [false, true, true, true]),
        ("distribute.horizontal", [false, false, false, true]),
        ("auto_arrange.panel", [true, true, true, true]),
        ("delete", [false, true, true, true]),
    ];
    let cmd_rows: [(Cmd, [bool; 4]); 5] = [
        (Cmd::LinkCopies, [false, false, true, true]),
        (Cmd::Unlink, [false, false, false, false]),
        (Cmd::ToggleCut, [false, true, true, true]),
        (Cmd::ToggleHide, [false, true, true, true]),
        (Cmd::Rename, [false, true, false, false]),
    ];
    for n in 0..=3 {
        let (_d, _s, mut h, _) = with_selection(n);
        let m = model_of(&mut h, &[]);
        for (id, want) in &tool_rows {
            assert_eq!(tool(&m, id).enabled, want[n], "{id} with {n} selected");
        }
        for (cmd, want) in &cmd_rows {
            assert_eq!(leaf(&m, cmd).enabled, want[n], "{cmd:?} with {n} selected");
        }
        // The submenus dim when none of their items can run.
        let distribute = m.subs().into_iter().find(|s| s.id == "distribute").unwrap();
        assert_eq!(distribute.enabled, n == 3);
    }
}

#[test]
fn link_copies_and_unlink_follow_the_link_state() {
    let (_d, s, mut h, ids) = with_selection(2);
    s.lock()
        .unwrap()
        .execute(
            "object.link",
            json!({ "ids": [ids[0], ids[1]] }),
            Origin::User,
        )
        .unwrap();
    h.run_plan();
    let m = model_of(&mut h, &[]);
    assert!(!leaf(&m, &Cmd::LinkCopies).enabled, "they are linked");
    assert!(leaf(&m, &Cmd::Unlink).enabled);
}

#[test]
fn hide_and_skip_cutting_flip_their_titles() {
    let (_d, s, mut h, ids) = with_selection(1);
    let ids2 = json!({ "ids": [ids[0]] });
    let run = |c: &str, p: serde_json::Value| {
        s.lock().unwrap().execute(c, p, Origin::User).unwrap();
    };
    run(
        "object.set_visible",
        json!({ "ids": [ids[0]], "visible": false }),
    );
    run("object.set_cut", json!({ "ids": [ids[0]], "cut": false }));
    let _ = ids2;
    h.run_plan();
    let m = model_of(&mut h, &[]);
    assert_eq!(leaf(&m, &Cmd::ToggleHide).title, "Show");
    assert_eq!(leaf(&m, &Cmd::ToggleCut).title, "Cut again");
    assert!(leaf(&m, &Cmd::ToggleHide).enabled);
}

#[test]
fn view_items_show_their_state_as_check_marks() {
    let (_d, _s, mut h, _) = with_selection(0);
    {
        let p = &mut app_mut(&mut h).ui.prepare;
        p.show_grid = true;
        p.show_rulers = false;
        p.panels
            .set_collapsed(flatcraft::ui::rails::SidePanel::Objects, false);
        p.panels
            .set_collapsed(flatcraft::ui::rails::SidePanel::Settings, true);
    }
    let m = model_of(&mut h, &[]);
    assert_eq!(tool(&m, "view.grid").checked, Some(true));
    assert_eq!(tool(&m, "view.rulers").checked, Some(false));
    assert_eq!(tool(&m, "panel.objects").checked, Some(true));
    assert_eq!(tool(&m, "panel.settings").checked, Some(false));
    assert_eq!(tool(&m, "zoom.fit").checked, None);
    assert_eq!(
        leaf(&m, &Cmd::Workspace(Workspace::Prepare)).checked,
        Some(true)
    );
    assert_eq!(
        leaf(&m, &Cmd::Workspace(Workspace::Cut)).checked,
        Some(false)
    );
    app_mut(&mut h).ui.prepare.show_grid = false;
    let m2 = model_of(&mut h, &[]);
    assert_eq!(tool(&m2, "view.grid").checked, Some(false));
    assert_ne!(m, m2);
    assert_eq!(m.shape(), m2.shape(), "a state change keeps the items");
}

#[test]
fn home_has_no_document_items() {
    let (_d, _s, h, _) = with_selection(3);
    let m = home_model(&h);
    for l in m.leaves() {
        match l.cmd {
            Cmd::Tool(id) => assert!(!l.enabled, "{id}"),
            Cmd::LinkCopies | Cmd::Unlink | Cmd::ToggleCut | Cmd::ToggleHide | Cmd::Rename => {
                assert!(!l.enabled, "{:?}", l.cmd)
            }
            _ => {}
        }
    }
    assert!(leaf(&m, &Cmd::New).enabled && leaf(&m, &Cmd::Quit).enabled);
}

#[test]
fn the_cut_workspace_dims_the_prepare_items_and_enables_send() {
    let (_d, _s, mut h, _) = with_selection(2);
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
    let a = app(&h);
    let input = |send_ready| MenuInput {
        workspace: Workspace::Cut,
        snap: Some(&snap),
        prepare: &a.ui.prepare,
        show_debug: false,
        can_undo: true,
        can_redo: true,
        text_focused: false,
        send_ready,
        recent: &[],
        modal: false,
    };
    let m = MenuModel::build(&input(true));
    assert!(!tool(&m, "duplicate").enabled);
    assert!(!tool(&m, "zoom.fit").enabled);
    assert!(leaf(&m, &Cmd::SendToFile).enabled);
    assert_eq!(
        leaf(&m, &Cmd::Workspace(Workspace::Cut)).checked,
        Some(true)
    );
    let m = MenuModel::build(&input(false));
    assert!(!leaf(&m, &Cmd::SendToFile).enabled, "no job, no send");
}

#[test]
fn edit_items_work_for_text_fields() {
    let (_d, _s, h, _) = with_selection(1);
    let a = app(&h);
    let build = |text_focused: bool, can_undo: bool| {
        MenuModel::build(&MenuInput {
            workspace: Workspace::Home,
            snap: None,
            prepare: &a.ui.prepare,
            show_debug: false,
            can_undo,
            can_redo: false,
            text_focused,
            send_ready: false,
            recent: &[],
            modal: false,
        })
    };
    let m = build(false, false);
    // A dimmed item lets its key through to the window.
    for c in [Cmd::Undo, Cmd::Cut, Cmd::Copy, Cmd::Paste, Cmd::SelectAll] {
        assert!(!leaf(&m, &c).enabled, "{c:?}");
    }
    let m = build(false, true);
    assert!(leaf(&m, &Cmd::Undo).enabled && !leaf(&m, &Cmd::Redo).enabled);
    let m = build(true, false);
    for c in [
        Cmd::Undo,
        Cmd::Redo,
        Cmd::Cut,
        Cmd::Copy,
        Cmd::Paste,
        Cmd::SelectAll,
    ] {
        assert!(leaf(&m, &c).enabled, "{c:?}");
    }
}

#[test]
fn recent_projects_become_items_that_open_them() {
    let (_d, _s, mut h, _) = with_selection(0);
    let none = model_of(&mut h, &[]);
    let open_recent = |m: &MenuModel| {
        m.subs()
            .into_iter()
            .find(|s| s.id == "open-recent")
            .unwrap()
            .enabled
    };
    assert!(!open_recent(&none));
    assert!(!leaf(&none, &Cmd::NoRecent).enabled);
    let paths = vec![
        PathBuf::from("/x/a.flatcraft"),
        PathBuf::from("/x/b.flatcraft"),
    ];
    let some = model_of(&mut h, &paths);
    assert!(open_recent(&some));
    assert_eq!(leaf(&some, &Cmd::OpenRecent(1)).title, "b");
    assert_eq!(
        Cmd::OpenRecent(1).action(1, &some.recent),
        Some(Action::Open(paths[1].clone()))
    );
    assert_ne!(none.shape(), some.shape(), "the items changed");
}

#[test]
fn ids_are_unique_and_parse_back() {
    let (_d, _s, mut h, _) = with_selection(1);
    let paths = vec![PathBuf::from("/x/a.flatcraft")];
    let m = model_of(&mut h, &paths);
    let mut ids: Vec<String> = m.leaves().iter().map(|l| l.cmd.id()).collect();
    for l in m.leaves() {
        assert_eq!(Cmd::parse(&l.cmd.id()), Some(l.cmd.clone()), "{:?}", l.cmd);
    }
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n);
    assert_eq!(Cmd::parse("flatcraft.nonsense"), None);
    // Every action of the table that is listed is reachable.
    let listed = m
        .leaves()
        .iter()
        .filter(|l| matches!(l.cmd, Cmd::Tool(_)))
        .count();
    assert_eq!(listed, TOOLS.iter().filter(|t| in_menu(t)).count());
}

/// The actions the menu lists (the one-click Auto arrange and the Debug
/// panel stay in the ribbon).
fn in_menu(t: &tools::Tool) -> bool {
    !matches!(t.act, Act::AutoArrange | Act::DebugPanel)
}

#[test]
fn keys_are_the_table_keys_and_never_bare() {
    let (_d, _s, mut h, _) = with_selection(1);
    let m = model_of(&mut h, &[]);
    let accel = |c: &Cmd| leaf(&m, c).accel.map(|a| (a.key, a.shift));
    assert_eq!(accel(&Cmd::Save), Some((egui::Key::S, false)));
    assert_eq!(accel(&Cmd::SaveAs), Some((egui::Key::S, true)));
    assert_eq!(accel(&Cmd::Open), Some((egui::Key::O, false)));
    assert_eq!(accel(&Cmd::New), Some((egui::Key::N, false)));
    assert_eq!(accel(&Cmd::Close), Some((egui::Key::W, false)));
    assert_eq!(accel(&Cmd::Settings), Some((egui::Key::Comma, false)));
    assert_eq!(accel(&Cmd::Undo), Some((egui::Key::Z, false)));
    assert_eq!(accel(&Cmd::Redo), Some((egui::Key::Z, true)));
    assert_eq!(accel(&Cmd::SelectAll), Some((egui::Key::A, false)));
    assert_eq!(accel(&Cmd::Quit), Some((egui::Key::Q, false)));
    assert_eq!(accel(&Cmd::Tool("duplicate")), Some((egui::Key::D, false)));
    assert_eq!(accel(&Cmd::Tool("import")), Some((egui::Key::I, false)));
    // Delete, Backspace and the arrows would be taken from text fields.
    assert_eq!(accel(&Cmd::Tool("delete")), None);
    // Every accelerator is a key that a row of the table or the Edit and
    // Quit keys use.
    for l in m.leaves() {
        if let Some(a) = l.accel {
            assert!(
                !matches!(
                    a.key,
                    egui::Key::Backspace
                        | egui::Key::Delete
                        | egui::Key::Escape
                        | egui::Key::Space
                        | egui::Key::ArrowLeft
                        | egui::Key::ArrowRight
                        | egui::Key::ArrowUp
                        | egui::Key::ArrowDown
                ),
                "{:?}",
                l.cmd
            );
        }
    }
}

/// No key is handled twice: a row that the menu owns on macOS has an
/// accelerator, and a row that runs in the key handler has none.
#[test]
fn no_shortcut_is_a_menu_key_and_run_by_the_key_handler() {
    let (_d, _s, mut h, _) = with_selection(1);
    let m = model_of(&mut h, &[]);
    let accels: Vec<_> = m.leaves().iter().filter_map(|l| l.accel).collect();
    for s in SHORTCUTS.iter().filter(|s| s.run.is_some()) {
        let is_menu_key = accels.iter().any(|a| {
            Some(a.key) == s.key() && a.shift == s.modifiers.shift && a.alt == s.modifiers.alt
        });
        assert_eq!(
            is_menu_key,
            s.menu_on_macos,
            "{} {}: menu_on_macos must say whether the menu has the key",
            s.keys(),
            s.what
        );
        // And the key handler really skips the menu's rows on macOS.
        if cfg!(target_os = "macos") {
            let ctx = egui::Context::default();
            let mut actions = Vec::new();
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key: s.key().unwrap(),
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: s.modifiers,
                }],
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| help::run_shortcuts(ui, 1, &mut actions));
            out.textures_delta.clear();
            assert_eq!(actions.is_empty(), s.menu_on_macos, "{}", s.what);
        }
    }
}

#[test]
fn the_gate_stops_every_menu_item_but_quit_while_a_dialog_is_open() {
    let (_d, s, mut h, _) = with_selection(3);
    let paths = vec![PathBuf::from("/x/a.flatcraft")];
    let m = model_of(&mut h, &paths);
    let doc = s.lock().unwrap().doc().id();
    let all: Vec<Action> = m
        .leaves()
        .iter()
        .filter_map(|l| l.cmd.action(doc, &m.recent))
        .collect();
    assert!(all.len() > 60, "{}", all.len());
    assert_eq!(app(&h).menu_gate(all.clone()), all);
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::ShowAbout]);
    h.run_plan();
    assert_eq!(
        app(&h).menu_gate(all.clone()),
        vec![Action::Quit],
        "only Quit goes through"
    );
    // Every click of the table is an Action the gate sees (none is run
    // from the menu directly).
    assert!(all.iter().any(|a| matches!(a, Action::Menu(Cmd::Tool(_)))));
}

#[test]
fn menu_actions_run_the_same_things_as_the_ribbon() {
    let (_d, s, mut h, ids) = with_selection(1);
    let count = |s: &SharedSession| {
        s.lock()
            .unwrap()
            .execute("document.state", json!({}), Origin::User)
            .unwrap()["objects"]
            .as_array()
            .unwrap()
            .len()
    };
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Menu(Cmd::Tool("duplicate"))]);
    h.run_plan();
    assert_eq!(count(&s), 4, "Duplicate");
    // A dimmed item does nothing: Distribute needs 3 objects.
    app_mut(&mut h).apply(&ctx, vec![Action::Menu(Cmd::Tool("distribute.horizontal"))]);
    h.run_plan();
    assert_eq!(count(&s), 4);
    // Hide.
    select(&s, &[ids[0]]);
    h.run_plan();
    app_mut(&mut h).apply(&ctx, vec![Action::Menu(Cmd::ToggleHide)]);
    h.run_plan();
    let state = s
        .lock()
        .unwrap()
        .execute("document.state", json!({}), Origin::User)
        .unwrap();
    let hidden = state["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == ids[0])
        .unwrap()["visible"]
        .clone();
    assert_eq!(hidden, json!(false));
    // Grid is a switch.
    let before = app(&h).ui.prepare.show_grid;
    app_mut(&mut h).apply(&ctx, vec![Action::Menu(Cmd::Tool("view.grid"))]);
    assert_eq!(app(&h).ui.prepare.show_grid, !before);
    // Rename opens its dialog.
    app_mut(&mut h).apply(&ctx, vec![Action::Menu(Cmd::Rename)]);
    assert!(app(&h).ui.prepare.rename.is_some());
}

#[test]
fn menu_items_do_nothing_outside_prepare() {
    let (_d, s, mut h, _) = with_selection(1);
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Menu(Cmd::Tool("duplicate"))]);
    h.run_plan();
    let n = s
        .lock()
        .unwrap()
        .execute("document.state", json!({}), Origin::User)
        .unwrap()["objects"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(n, 3);
}

/// A click on Undo while a text field has the keyboard hands Cmd+Z to the
/// field (egui's text edit keeps its own undo history), so the document is
/// not touched.
#[test]
fn undo_in_a_text_field_undoes_the_text() {
    let ctx = egui::Context::default();
    let mut text = String::new();
    let id = egui::Id::unique("field");
    let frame = |ctx: &egui::Context, events: Vec<egui::Event>, text: &mut String| {
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let r = ui.add(egui::TextEdit::singleline(text).id(id));
            if !r.has_focus() {
                r.request_focus();
            }
        });
        out.textures_delta.clear();
    };
    frame(&ctx, vec![], &mut text);
    frame(&ctx, vec![egui::Event::Text("abc".into())], &mut text);
    assert_eq!(text, "abc");
    assert!(ctx.text_edit_focused());
    // What `run_menu` injects for Cmd+Z.
    let key = egui::Event::Key {
        key: egui::Key::Z,
        physical_key: Some(egui::Key::Z),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    };
    frame(&ctx, vec![key], &mut text);
    assert_eq!(text, "", "the text edit undid the typing");
}

use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::Unsaved;

fn rename_open(h: &mut AppHarness) {
    let ctx = h.ctx.clone();
    app_mut(h).apply(&ctx, vec![Action::Menu(Cmd::Rename)]);
    h.run_plan();
    h.run_plan();
    assert!(app(h).ui.prepare.rename.is_some());
    assert!(h.ctx.text_edit_focused(), "the name field has the focus");
}

const EDIT_CMDS: [Cmd; 6] = [
    Cmd::Undo,
    Cmd::Redo,
    Cmd::Cut,
    Cmd::Copy,
    Cmd::Paste,
    Cmd::SelectAll,
];

#[test]
fn the_edit_items_reach_a_text_field_in_a_dialog() {
    let (_d, s, mut h, _) = with_selection(1);
    rename_open(&mut h);
    let all: Vec<Action> = EDIT_CMDS.iter().cloned().map(Action::Menu).collect();
    // The dialog is open: the gate lets the Edit items through only while a
    // text field has the keys, and nothing else.
    assert_eq!(app(&h).menu_gate_with(all.clone(), true), all);
    assert!(app(&h).menu_gate_with(all.clone(), false).is_empty());
    let mixed = vec![Action::Menu(Cmd::Tool("delete")), Action::Menu(Cmd::Copy)];
    assert_eq!(
        app(&h).menu_gate_with(mixed, true),
        vec![Action::Menu(Cmd::Copy)]
    );
    // The model has the items enabled, so the menu takes the keys.
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
    let a = app(&h);
    let m = MenuModel::build(&MenuInput {
        workspace: a.ui.workspace,
        snap: Some(&snap),
        prepare: &a.ui.prepare,
        show_debug: false,
        can_undo: false,
        can_redo: false,
        text_focused: true,
        send_ready: false,
        recent: &[],
        modal: false,
    });
    for c in &EDIT_CMDS {
        assert!(leaf(&m, c).enabled, "{c:?}");
    }
    // Select All, typing and Undo act on the field, not on the document.
    let steps = undo_len(&s);
    app_mut(&mut h).apply(&ctx, vec![Action::Menu(Cmd::SelectAll)]);
    h.run_plan();
    h.event(egui::Event::Text("zz".into()));
    h.run_plan();
    fn name(h: &AppHarness) -> String {
        app(h).ui.prepare.rename.as_ref().unwrap().name.clone()
    }
    assert_eq!(name(&h), "zz", "Select All selected the name");
    app_mut(&mut h).apply(&ctx, vec![Action::Menu(Cmd::Undo)]);
    h.run_plan();
    assert_ne!(name(&h), "zz", "Undo undid the typing");
    assert_eq!(undo_len(&s), steps, "the document was not touched");
    // Cut, Copy and Paste ask the window for the clipboard.
    app_mut(&mut h).apply(
        &ctx,
        vec![
            Action::Menu(Cmd::Copy),
            Action::Menu(Cmd::Cut),
            Action::Menu(Cmd::Paste),
        ],
    );
    h.run_plan();
}

#[test]
fn a_text_field_keeps_the_keys_of_duplicate_import_and_delete() {
    let (_d, s, mut h, _) = with_selection(1);
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
    let a = app(&h);
    let build = |text_focused| {
        MenuModel::build(&MenuInput {
            workspace: Workspace::Prepare,
            snap: Some(&snap),
            prepare: &a.ui.prepare,
            show_debug: false,
            can_undo: true,
            can_redo: true,
            text_focused,
            send_ready: false,
            recent: &[],
            modal: false,
        })
    };
    let typing = build(true);
    let idle = build(false);
    for id in ["duplicate", "import", "delete"] {
        assert!(!tool(&typing, id).enabled, "{id} while typing");
    }
    assert!(tool(&idle, "duplicate").enabled);
    assert!(tool(&typing, "rotate").enabled, "no key, no clash");
    // And a late click does not duplicate behind the dialog's field.
    rename_open(&mut h);
    app_mut(&mut h).apply(&ctx, vec![Action::Menu(Cmd::Tool("duplicate"))]);
    h.run_plan();
    let n = s
        .lock()
        .unwrap()
        .execute("document.state", json!({}), Origin::User)
        .unwrap()["objects"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(n, 3);
}

#[test]
fn trace_reload_reconnect_and_test_patterns_are_listed_with_their_rules() {
    let (_d, _s, mut h, _) = with_selection(1);
    let m = model_of(&mut h, &[]);
    assert_eq!(tool(&m, "trace").title, "Trace...");
    assert!(!tool(&m, "trace").enabled, "a vector is not a picture");
    assert!(!tool(&m, "reload").enabled, "not imported from a file");
    assert!(!leaf(&m, &Cmd::Reconnect).enabled);
    assert!(tool(&m, "test_patterns").enabled);
    assert_eq!(tool(&m, "import").title, "Import...");
}

#[test]
fn close_is_dimmed_on_home() {
    let (_d, _s, h, _) = with_selection(0);
    assert!(!leaf(&home_model(&h), &Cmd::Close).enabled);
}

#[test]
fn redo_with_cmd_y_leaves_a_text_field_its_own_undo() {
    let ctx = egui::Context::default();
    let mut text = String::new();
    let id = egui::Id::unique("field");
    let mut run = |events: Vec<egui::Event>, field: bool| {
        let mut actions = Vec::new();
        let mut out = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                help::run_shortcuts(ui, 7, &mut actions);
                if field {
                    let r = ui.add(egui::TextEdit::singleline(&mut text).id(id));
                    if !r.has_focus() {
                        r.request_focus();
                    }
                }
            },
        );
        out.textures_delta.clear();
        actions
    };
    let key = |key, modifiers| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    run(vec![], true);
    run(vec![], true);
    assert!(ctx.text_edit_focused());
    // Cmd+Y and Cmd+Z in the field: no document undo or redo.
    let redo = key(egui::Key::Y, egui::Modifiers::COMMAND);
    assert!(run(vec![redo.clone()], true).is_empty());
    let undo = key(egui::Key::Z, egui::Modifiers::COMMAND);
    assert!(run(vec![undo], true).is_empty());
    // Without a field the key redoes the document.
    run(vec![], false);
    run(vec![], false);
    assert!(!ctx.text_edit_focused());
    assert_eq!(run(vec![redo], false), vec![Action::Redo(7)]);
}

fn pending_send(dir: &tempfile::TempDir) -> SharedSession {
    let s = session(dir);
    import_square(&s);
    {
        let mut g = s.lock().unwrap();
        g.execute(
            "job.set_plotter",
            json!({ "id": "silhouette-cameo-2" }),
            Origin::User,
        )
        .unwrap();
        g.execute("job.send", json!({}), Origin::Agent).unwrap_err();
        assert!(g.pending_send().is_some());
        g.doc_mut().dirty = false;
    }
    s
}

#[test]
fn quit_with_a_pending_send_asks_first() {
    let dir = tempfile::tempdir().unwrap();
    let s = pending_send(&dir);
    let mut h = harness(&dir, s.clone(), true);
    let ctx = h.ctx.clone();
    let got = common::capture(&mut h, |h| {
        app_mut(h).apply(&ctx, vec![Action::Quit]);
        h.run_plan();
    });
    assert_eq!(app(&h).ui.unsaved, Some(Unsaved::PendingSend));
    h.get_by_label("Quit while an agent waits?");
    h.get_by_label("An agent is waiting for your OK to send a cut.");
    assert!(!got.closes() && !app(&h).ui.closing);
    // Cancel keeps the window and the request.
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
        .click();
    h.run_plan();
    assert!(app(&h).ui.unsaved.is_none() && !app(&h).ui.closing);
    assert!(s.lock().unwrap().pending_send().is_some());
    // The window's close button asks the same way.
    app_mut(&mut h).apply(&ctx, vec![Action::Quit]);
    h.run_plan();
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Quit anyway")
        .click();
    h.run_plan();
    assert!(app(&h).ui.closing);
}

/// A document with a linked file that is gone, two squares, everything
/// selected, and a way to see what a click did.
struct Fixture {
    _dir: tempfile::TempDir,
    s: SharedSession,
    h: AppHarness,
    picker: common::FakePicker,
    revealed: common::FakeRevealer,
    recent: Vec<PathBuf>,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let file = dir.path().join("link.svg");
    std::fs::write(
        &file,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="40" y="60" width="20" height="20"/></svg>"#,
    )
    .unwrap();
    let linked = s
        .lock()
        .unwrap()
        .execute("object.import", json!({ "path": file }), Origin::User)
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    // The file changed since the import, so Reload has something to read.
    std::fs::write(
        &file,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="40" y="60" width="25" height="25"/></svg>"#,
    )
    .unwrap();
    let a = import_square(&s);
    let b = import_square(&s);
    // Not all selected: Select All has something to do.
    let _ = b;
    select(&s, &[linked, a]);
    let picker = common::FakePicker::default();
    let p = picker.clone();
    let revealed = common::FakeRevealer::default();
    let r = revealed.clone();
    let mut h = common::harness_with(&dir, s.clone(), |app| {
        app.with_file_picker(p).with_revealer(r)
    });
    to_prepare(&mut h);
    app_mut(&mut h).ui.prepare.sources.doc_id = s.lock().unwrap().doc().id();
    app_mut(&mut h).ui.prepare.sources.missing.push(linked);
    h.run_plan();
    // Not fitted and not at 100%, so both zoom items have something to do.
    app_mut(&mut h).ui.prepare.view.px_per_mm = 5.0;
    Fixture {
        _dir: dir,
        s,
        h,
        picker,
        revealed,
        recent: vec![PathBuf::from("/nowhere/a.flatcraft")],
    }
}

/// Everything a click can change, as one string.
fn effects(f: &Fixture) -> String {
    let a = app(&f.h);
    let doc =
        f.s.lock()
            .unwrap()
            .execute("document.state", json!({}), Origin::User)
            .unwrap();
    let docs = f.s.lock().unwrap().docs.len();
    let p = &a.ui.prepare;
    let requests = f.picker.requests.lock().unwrap().len();
    let revealed = f.revealed.0.lock().unwrap().len();
    format!(
        "{doc} {docs} {requests} {revealed} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
        a.ui.workspace,
        (
            p.rename.is_some(),
            p.panel.is_some(),
            p.fit_pending,
            p.keep_pick.is_some()
        ),
        (
            p.show_grid,
            p.show_rulers,
            p.show_kept_waste,
            p.panels.clone()
        ),
        a.ui.about.open,
        a.ui.show_shortcuts,
        a.ui.troubleshoot.open,
        a.ui.show_settings,
        (&a.ui.unsaved, &a.ui.export),
        a.ui.toasts.len(),
        a.injected.len(),
        p.view.px_per_mm,
    )
}

/// A click on every enabled item does something: an action, a dialog, a
/// request to the window or a changed state. A `Cmd` the click handler
/// forgets (as Reconnect once was) shows up here.
#[test]
fn every_enabled_menu_item_does_something_when_clicked() {
    let probe = fixture();
    let mut h = probe.h;
    let m = {
        let ctx = h.ctx.clone();
        let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
        let a = app(&h);
        MenuModel::build(&MenuInput {
            workspace: a.ui.workspace,
            snap: Some(&snap),
            prepare: &a.ui.prepare,
            show_debug: false,
            can_undo: true,
            can_redo: false,
            text_focused: false,
            send_ready: false,
            recent: &probe.recent,
            modal: false,
        })
    };
    let todo: Vec<Cmd> = m
        .leaves()
        .into_iter()
        // The workspace we are in is checked: choosing it again changes nothing.
        .filter(|l| l.enabled && l.cmd != Cmd::Workspace(Workspace::Prepare))
        .map(|l| l.cmd.clone())
        .collect();
    assert!(todo.len() > 40, "{}", todo.len());
    assert!(
        todo.contains(&Cmd::Reconnect),
        "the fixture has a broken link"
    );
    drop(h);
    for cmd in todo {
        let mut f = fixture();
        let before = effects(&f);
        let doc = f.s.lock().unwrap().doc().id();
        let action = cmd
            .action(doc, &f.recent)
            .unwrap_or_else(|| panic!("{cmd:?}"));
        let ctx = f.h.ctx.clone();
        // Some items start a worker (the hole finder of Pick on the mat)
        // that asks for a repaint when it is done: settle by the clock.
        // What the click asked of the platform (open a URL, close) is what
        // it sent while it ran, over every frame of the settling.
        let sent = common::capture(&mut f.h, |h| {
            app_mut(h).apply(&ctx, vec![action]);
            settle(h);
        });
        assert!(
            sent != common::Captured::default() || effects(&f) != before,
            "{cmd:?} did nothing"
        );
    }
}

#[test]
fn reconnect_asks_for_the_moved_file() {
    let mut f = fixture();
    let ctx = f.h.ctx.clone();
    app_mut(&mut f.h).apply(&ctx, vec![Action::Menu(Cmd::Reconnect)]);
    let requests = f.picker.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].title, "Reconnect file");
}
