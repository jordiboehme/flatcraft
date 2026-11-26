//! Group and Ungroup in the window (Task 57): a click picks the whole
//! group, a double-click or Cmd-click (Ctrl-click) one member, Escape
//! leaves the group; Cmd+G and Shift+Cmd+G; the Objects panel's group rows;
//! the ribbon and context menu items with their dimmed reasons.

mod common;

use common::{AppHarness, app, app_mut, harness, import_square, session, to_prepare, undo_len};
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::canvas::CanvasView;
use flatcraft::ui::context_menu::{self, Item};
use flatcraft::ui::prepare::Snapshot;
use flatcraft::ui::tools::{self, Act};
use flatcraft_core::document::ObjectId;
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

fn exec(s: &SharedSession, id: &str, params: Value) -> Value {
    s.lock().unwrap().execute(id, params, Origin::User).unwrap()
}

fn selection(s: &SharedSession) -> Vec<u64> {
    s.lock()
        .unwrap()
        .doc()
        .selection
        .iter()
        .map(|i| i.0)
        .collect()
}

fn snap(h: &mut AppHarness) -> Snapshot {
    flatcraft::ui::prepare::snapshot(app_mut(h), &egui::Context::default())
}

/// Three squares 100 mm apart (centres 45, 145, 245 at y 45); the first
/// two in a group.
fn grouped(dir: &tempfile::TempDir) -> (SharedSession, [u64; 3], u64) {
    let s = session(dir);
    let ids: Vec<u64> = (0..3)
        .map(|i| {
            let id = import_square(&s);
            exec(
                &s,
                "object.move",
                json!({ "ids": [id], "dx_mm": 100.0 * i as f64 }),
            );
            id
        })
        .collect();
    let g = exec(&s, "object.group", json!({ "ids": [ids[0], ids[1]] }))["group_id"]
        .as_u64()
        .unwrap();
    exec(&s, "object.select", json!({ "ids": [] }));
    (s, [ids[0], ids[1], ids[2]], g)
}

/// Prepare with 1 mm = 2 points and 145,45 mm at the canvas centre.
fn rig(dir: &tempfile::TempDir, s: SharedSession) -> (AppHarness, CanvasView) {
    let mut h = harness(dir, s, false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.run();
    let rect = app(&h).ui.prepare.canvas_rect;
    let view = CanvasView {
        offset: rect.center().to_vec2() - Vec2::new(145.0, 45.0) * 2.0,
        px_per_mm: 2.0,
    };
    let p = &mut app_mut(&mut h).ui.prepare;
    p.view = view;
    p.fit_pending = false;
    h.run();
    (h, view)
}

fn at(v: &CanvasView, x: f64, y: f64) -> Pos2 {
    v.mm_to_screen(kurbo::Point::new(x, y))
}

/// `clicks` clicks at `p`, with
/// `modifiers` held.
fn click(h: &mut AppHarness, p: Pos2, clicks: usize, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(p));
    h.run();
    for _ in 0..clicks {
        for pressed in [true, false] {
            h.event(Event::PointerButton {
                pos: p,
                button: PointerButton::Primary,
                pressed,
                modifiers,
            });
            // One frame per press and release: egui sees each.
            h.step();
        }
    }
    h.run();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

#[test]
fn a_click_picks_the_group_and_a_double_click_one_member() {
    let dir = tempfile::tempdir().unwrap();
    let (s, [a, b, c], g) = grouped(&dir);
    let (mut h, v) = rig(&dir, s.clone());
    // A click on a member selects the whole group.
    click(&mut h, at(&v, 45.0, 45.0), 1, Modifiers::NONE);
    assert_eq!(selection(&s), vec![a, b]);
    assert_eq!(app(&h).ui.prepare.entered, None);
    // A double-click (the harness's clock is too coarse to send one)
    // goes in: the member alone. It picks as Cmd-click does.
    let sn = snap(&mut h);
    assert_eq!(
        flatcraft::ui::canvas::pick(&sn, &app(&h).ui.prepare, b, true),
        (vec![b], Some(g))
    );
    // Cmd-click (Ctrl-click) goes in: the member alone.
    click(&mut h, at(&v, 145.0, 45.0), 1, Modifiers::COMMAND);
    assert_eq!(selection(&s), vec![b]);
    let doc_id = s.lock().unwrap().doc().id();
    assert_eq!(app(&h).ui.prepare.entered, Some((doc_id, g)));
    // Inside the group, a click on the other member picks it alone.
    click(&mut h, at(&v, 45.0, 45.0), 1, Modifiers::NONE);
    assert_eq!(selection(&s), vec![a]);
    // Escape leaves the group: the whole group is selected again.
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(selection(&s), vec![a, b]);
    assert_eq!(app(&h).ui.prepare.entered, None);
    // Go in again, then click outside the group.
    click(&mut h, at(&v, 145.0, 45.0), 1, Modifiers::COMMAND);
    assert_eq!(selection(&s), vec![b]);
    // A click outside the group leaves it.
    click(&mut h, at(&v, 245.0, 45.0), 1, Modifiers::NONE);
    assert_eq!(selection(&s), vec![c]);
    assert_eq!(app(&h).ui.prepare.entered, None);
}

#[test]
fn cmd_g_groups_and_shift_cmd_g_ungroups() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = import_square(&s);
    exec(&s, "object.move", json!({ "ids": [b], "dx_mm": 100.0 }));
    exec(&s, "object.select", json!({ "ids": [a, b] }));
    let (mut h, _) = rig(&dir, s.clone());
    let steps = undo_len(&s);
    h.key_press_modifiers(Modifiers::COMMAND, Key::G);
    h.run();
    assert_eq!(s.lock().unwrap().doc().document.groups.len(), 1);
    assert_eq!(undo_len(&s), steps + 1, "one undo step");
    let shift_cmd = Modifiers {
        shift: true,
        ..Modifiers::COMMAND
    };
    h.key_press_modifiers(shift_cmd, Key::G);
    h.run();
    assert!(s.lock().unwrap().doc().document.groups.is_empty());
    assert_eq!(undo_len(&s), steps + 2);
}

#[test]
fn group_and_ungroup_say_why_they_are_dimmed_and_what_they_do() {
    let dir = tempfile::tempdir().unwrap();
    let (s, [a, b, c], _) = grouped(&dir);
    let (mut h, _) = rig(&dir, s.clone());
    let group = tools::by_act(Act::Group);
    let ungroup = tools::by_act(Act::Ungroup);
    // One loose object: nothing to group or ungroup.
    exec(&s, "object.select", json!({ "ids": [c] }));
    let sn = snap(&mut h);
    assert_eq!(group.blocked(&sn), Some("Select 2 or more objects first."));
    assert_eq!(ungroup.blocked(&sn), Some("Select a group first."));
    // A whole group: one object, so Group is dimmed; Ungroup dissolves.
    exec(&s, "object.select", json!({ "ids": [a, b] }));
    let sn = snap(&mut h);
    let ids: Vec<u64> = sn.selection.iter().copied().collect();
    assert_eq!(group.blocked(&sn), Some("Select 2 or more objects first."));
    assert_eq!(ungroup.blocked(&sn), None);
    assert_eq!(ungroup.live_title(&sn, &ids), "Ungroup");
    // The group and a loose object: Group can run.
    exec(&s, "object.select", json!({ "ids": [a, b, c] }));
    let sn = snap(&mut h);
    assert_eq!(group.blocked(&sn), None);
    // One member picked inside: it leaves the group.
    exec(&s, "object.select", json!({ "ids": [a] }));
    let sn = snap(&mut h);
    assert_eq!(ungroup.live_title(&sn, &[a]), "Remove 1 object from group");
    // The context menu lists both, Ungroup with what it does.
    let items: Vec<Item> = context_menu::items(&sn, &app(&h).ui.prepare, &[a])
        .into_iter()
        .flatten()
        .collect();
    assert!(items.contains(&Item::Group) && items.contains(&Item::Ungroup));
    assert_eq!(Item::Ungroup.text(&sn, &[a]), "Remove 1 object from group");
}

#[test]
fn ungroup_names_two_members() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let ids: Vec<u64> = (0..3).map(|_| import_square(&s)).collect();
    exec(&s, "object.group", json!({ "ids": ids }));
    let (mut h, _) = rig(&dir, s.clone());
    let sn = snap(&mut h);
    let ungroup = tools::by_act(Act::Ungroup);
    assert_eq!(
        ungroup.live_title(&sn, &ids[..2]),
        "Remove 2 objects from group"
    );
}

#[test]
fn the_objects_panel_folds_a_group_open() {
    let dir = tempfile::tempdir().unwrap();
    let (s, [a, _, _], g) = grouped(&dir);
    let (mut h, _) = rig(&dir, s.clone());
    // The group is one row; its members are folded away.
    assert!(h.query_by_label("Group 1, group of 2 objects").is_some());
    assert!(
        h.query_by_role_and_label(Role::Button, "Hide square")
            .is_some()
    );
    let rows_before = h.query_all_by_label("Hide square").count();
    h.get_by_role_and_label(Role::Button, "Open Group 1")
        .click();
    h.run();
    let doc_id = s.lock().unwrap().doc().id();
    assert!(app(&h).ui.prepare.open_groups.contains(&(doc_id, g)));
    assert_eq!(h.query_all_by_label("Hide square").count(), rows_before + 2);
    // A click on the group row selects the whole group.
    h.get_by_label("Group 1, group of 2 objects").click();
    h.run();
    assert_eq!(selection(&s).len(), 2);
    assert!(selection(&s).contains(&a));
    // Hiding the group hides both members.
    h.get_by_role_and_label(Role::Button, "Hide Group 1")
        .click();
    h.run();
    let guard = s.lock().unwrap();
    let doc = &guard.doc().document;
    let hidden = doc.objects.iter().filter(|o| !o.visible).count();
    assert_eq!(hidden, 2);
    assert!(doc.group_node(ObjectId(g)).is_some());
}
