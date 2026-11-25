//! The context menu of the canvas and of the Objects panel, driven through
//! egui_kittest.

mod common;

use std::path::{Path, PathBuf};

use common::{
    AppHarness, FakePicker, app, app_mut, harness, harness_with, import_square, session,
    to_prepare, undo_len, wait_for_workers,
};
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::canvas::CanvasView;
use flatcraft::ui::context_menu::Item;
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

fn exec(s: &SharedSession, cmd: &str, params: Value) -> Value {
    s.lock()
        .unwrap()
        .execute(cmd, params, Origin::User)
        .unwrap()
}

fn state(s: &SharedSession) -> Value {
    exec(s, "document.state", json!({}))
}

fn object(s: &SharedSession, id: u64) -> Value {
    state(s)["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .cloned()
        .unwrap_or(Value::Null)
}

fn selection(s: &SharedSession) -> Vec<u64> {
    state(s)["selection"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect()
}

fn select(s: &SharedSession, ids: &[u64]) {
    exec(s, "object.select", json!({ "ids": ids }));
}

fn object_count(s: &SharedSession) -> usize {
    state(s)["objects"].as_array().unwrap().len()
}

/// A square of `side` mm in a file; the page is 200 mm.
fn write_square(path: &Path, side: f64) {
    std::fs::write(
        path,
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="20" y="20" width="{side}" height="{side}"/></svg>"#
        ),
    )
    .unwrap();
}

fn import_file(s: &SharedSession, path: &Path) -> u64 {
    exec(s, "object.import", json!({ "path": path }))["id"]
        .as_u64()
        .unwrap()
}

/// A black ring on white, to import as a picture.
fn write_ring(path: &Path) {
    let mut px = tiny_skia::Pixmap::new(200, 200).unwrap();
    px.fill(tiny_skia::Color::WHITE);
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::BLACK);
    let circle = tiny_skia::PathBuilder::from_circle(100.0, 100.0, 80.0).unwrap();
    px.fill_path(
        &circle,
        &paint,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::identity(),
        None,
    );
    px.save_png(path).unwrap();
}

/// `n` squares, 100 mm apart from the first at 45,45 mm.
fn squares(s: &SharedSession, n: usize) -> Vec<u64> {
    (0..n)
        .map(|i| {
            let id = import_square(s);
            exec(
                s,
                "object.move",
                json!({ "ids": [id], "dx_mm": 100.0 * i as f64, "dy_mm": 0.0 }),
            );
            id
        })
        .collect()
}

/// A view where 1 mm is 2 points and 145,45 mm (the second square) sits at
/// the canvas centre.
fn known_view(h: &mut AppHarness) -> CanvasView {
    let rect = app(h).ui.prepare.canvas_rect;
    let view = CanvasView {
        offset: rect.center().to_vec2() - Vec2::new(145.0, 45.0) * 2.0,
        px_per_mm: 2.0,
    };
    let p = &mut app_mut(h).ui.prepare;
    p.view = view;
    p.fit_pending = false;
    wait_for_workers(h);
    view
}

fn at_mm(view: &CanvasView, x: f64, y: f64) -> Pos2 {
    view.mm_to_screen(kurbo::Point::new(x, y))
}

fn press(h: &mut AppHarness, p: Pos2, button: PointerButton) {
    // The hit test reads the shapes the worker builds.
    wait_for_workers(h);
    h.event(Event::PointerMoved(p));
    h.run();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: p,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    h.run();
    h.run();
}

fn right_click(h: &mut AppHarness, p: Pos2) {
    press(h, p, PointerButton::Secondary);
}

fn left_click(h: &mut AppHarness, p: Pos2) {
    press(h, p, PointerButton::Primary);
}

fn menu_open(h: &AppHarness) -> bool {
    h.query_by_role(Role::Menu).is_some()
}

/// The accessible name of an item, with its key hint.
fn name(item: Item) -> String {
    match item.hint() {
        Some(hint) => format!("{} {hint}", item.label()),
        None => item.label().to_string(),
    }
}

const ALL: [Item; 23] = [
    Item::Duplicate,
    Item::Copies,
    Item::LinkCopies,
    Item::UnlinkCopies,
    Item::RotateLeft,
    Item::RotateRight,
    Item::MirrorHorizontal,
    Item::MirrorVertical,
    Item::Align,
    Item::Distribute,
    Item::Hide,
    Item::Show,
    Item::SkipCutting,
    Item::CutAgain,
    Item::Rename,
    Item::TraceAgain,
    Item::Reload,
    Item::Reconnect,
    Item::Delete,
    Item::Import,
    Item::SelectAll,
    Item::AutoArrange,
    Item::ZoomToFit,
];

/// The items the open menu shows, in order.
fn shown(h: &AppHarness) -> Vec<Item> {
    let menu = h.get_by_role(Role::Menu);
    let labels: Vec<String> = menu
        .children_recursive()
        .filter(|n| n.accesskit_node().role() == Role::Button)
        .filter_map(|n| n.accesskit_node().label())
        .collect();
    labels
        .iter()
        .map(|l| {
            *ALL.iter()
                .find(|i| &name(**i) == l || l.starts_with(&format!("{} ", i.label())))
                .unwrap_or_else(|| panic!("unknown menu entry {l:?}"))
        })
        .collect()
}

fn click_item(h: &mut AppHarness, item: Item) {
    h.get_by_role(Role::Menu).get_by_label(&name(item)).click();
    h.run();
}

/// Items with a submenu: opens it and clicks `entry`.
fn click_sub(h: &mut AppHarness, item: Item, entry: &str) {
    click_item(h, item);
    let sub = h
        .query_all_by_role(Role::Menu)
        .find(|m| m.query_by_label(entry).is_some())
        .expect("the submenu is open");
    sub.get_by_label(entry).click();
    h.run();
}

struct Rig {
    _dir: tempfile::TempDir,
    s: SharedSession,
    h: AppHarness,
    ids: Vec<u64>,
    view: CanvasView,
}

/// `n` squares in Prepare with the known view.
fn rig(n: usize) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let ids = squares(&s, n);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let view = known_view(&mut h);
    Rig {
        _dir: dir,
        s,
        h,
        ids,
        view,
    }
}

impl Rig {
    /// Right-clicks the centre of object `i` (selection untouched).
    fn right_click_object(&mut self, i: usize) {
        let p = at_mm(&self.view, 45.0 + 100.0 * i as f64, 45.0);
        right_click(&mut self.h, p);
    }

    fn items(&self) -> Vec<Item> {
        shown(&self.h)
    }
}

// ------------------------------------------------------------ selection

#[test]
fn right_click_selects_an_object_outside_the_selection() {
    let mut r = rig(2);
    select(&r.s, &[r.ids[0]]);
    r.h.run();
    r.right_click_object(1);
    assert!(menu_open(&r.h));
    assert_eq!(selection(&r.s), vec![r.ids[1]]);
}

#[test]
fn right_click_inside_the_selection_keeps_it() {
    let mut r = rig(3);
    select(&r.s, &[r.ids[0], r.ids[1]]);
    r.h.run();
    r.right_click_object(1);
    assert!(menu_open(&r.h));
    assert_eq!(selection(&r.s), vec![r.ids[0], r.ids[1]]);
    // The menu is the one of two objects.
    assert!(r.items().contains(&Item::Align));
}

#[test]
fn right_click_on_the_empty_mat_clears_the_selection() {
    let mut r = rig(1);
    select(&r.s, &[r.ids[0]]);
    r.h.run();
    right_click(&mut r.h, at_mm(&r.view, 200.0, 200.0));
    assert!(menu_open(&r.h));
    assert!(selection(&r.s).is_empty());
}

#[test]
fn right_click_uses_the_hit_test_of_the_left_click() {
    // A point 2 mm outside the object (3 points of slack at 2 points per
    // mm still reach it): both clicks find the object.
    let mut r = rig(1);
    let near = at_mm(&r.view, 45.0 + 25.0 + 1.0, 45.0);
    left_click(&mut r.h, near);
    assert_eq!(selection(&r.s), vec![r.ids[0]]);
    select(&r.s, &[]);
    r.h.run();
    right_click(&mut r.h, near);
    assert_eq!(selection(&r.s), vec![r.ids[0]]);
    // Far off: neither finds it.
    let far = at_mm(&r.view, 45.0 + 25.0 + 10.0, 45.0);
    select(&r.s, &[]);
    r.h.run();
    left_click(&mut r.h, far);
    assert!(selection(&r.s).is_empty());
    right_click(&mut r.h, far);
    assert!(selection(&r.s).is_empty());
}

#[test]
fn objects_panel_rows_use_the_same_rule() {
    let mut r = rig(2);
    select(&r.s, &[r.ids[0]]);
    r.h.run();
    // Two rows named "square": the second is the second object.
    let row =
        r.h.query_all_by_role_and_label(Role::Button, "square")
            .nth(1)
            .unwrap();
    row.click_secondary();
    r.h.run();
    r.h.run();
    assert!(menu_open(&r.h));
    assert_eq!(selection(&r.s), vec![r.ids[1]]);
    assert!(r.items().contains(&Item::Rename));
}

// ------------------------------------------------------------- item sets

use Item::*;

#[test]
fn the_menu_of_one_object() {
    let mut r = rig(1);
    r.right_click_object(0);
    assert_eq!(
        r.items(),
        vec![
            Duplicate,
            Copies,
            RotateLeft,
            RotateRight,
            MirrorHorizontal,
            MirrorVertical,
            Hide,
            SkipCutting,
            Rename,
            Delete
        ]
    );
}

#[test]
fn the_menu_of_two_objects() {
    let mut r = rig(2);
    select(&r.s, &r.ids.clone());
    r.h.run();
    r.right_click_object(0);
    // Two separate imports hold their own source: no Link copies.
    assert_eq!(
        r.items(),
        vec![
            Duplicate,
            RotateLeft,
            RotateRight,
            MirrorHorizontal,
            MirrorVertical,
            Align,
            Hide,
            SkipCutting,
            Delete
        ]
    );
}

#[test]
fn the_menu_of_three_objects_adds_distribute() {
    let mut r = rig(3);
    select(&r.s, &r.ids.clone());
    r.h.run();
    r.right_click_object(1);
    let items = r.items();
    assert!(
        items.contains(&Align) && items.contains(&Distribute),
        "{items:?}"
    );
    assert!(!items.contains(&Rename) && !items.contains(&Copies));
}

#[test]
fn the_menu_of_a_picture_offers_trace_again() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let png = dir.path().join("ring.png");
    write_ring(&png);
    let id = import_file(&s, &png);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let rect = app(&h).ui.prepare.canvas_rect;
    let bounds = object(&s, id)["bounds_mm"].clone();
    let cx = (bounds["x0"].as_f64().unwrap() + bounds["x1"].as_f64().unwrap()) / 2.0;
    let cy = (bounds["y0"].as_f64().unwrap() + bounds["y1"].as_f64().unwrap()) / 2.0;
    let view = CanvasView {
        offset: rect.center().to_vec2() - Vec2::new(cx as f32, cy as f32) * 2.0,
        px_per_mm: 2.0,
    };
    app_mut(&mut h).ui.prepare.view = view;
    app_mut(&mut h).ui.prepare.fit_pending = false;
    h.run();
    // The ring has a hole: click on its band.
    let x1 = bounds["x1"].as_f64().unwrap();
    right_click(&mut h, at_mm(&view, x1 - 2.0, cy));
    let items = shown(&h);
    assert!(items.contains(&TraceAgain), "{items:?}");
    // The Trace dialog keeps repainting while its preview builds.
    h.get_by_role(Role::Menu)
        .get_by_label(&name(TraceAgain))
        .click();
    h.step();
    h.step();
    assert!(app(&h).ui.trace.is_some());
}

#[test]
fn the_menu_of_a_linked_copy_offers_unlink_and_link() {
    let mut r = rig(1);
    let copies = exec(
        &r.s,
        "object.copies",
        json!({ "id": r.ids[0], "count": 2, "linked": true }),
    );
    let copy = copies["ids"][0].as_u64().unwrap();
    r.h.run();
    let view = r.view;
    // The copies sit to the right of the original, 55 mm apart.
    let b = object(&r.s, copy)["bounds_mm"].clone();
    let cx = (b["x0"].as_f64().unwrap() + b["x1"].as_f64().unwrap()) / 2.0;
    right_click(&mut r.h, at_mm(&view, cx, 45.0));
    let items = r.items();
    assert!(items.contains(&UnlinkCopies), "{items:?}");
    assert!(!items.contains(&LinkCopies), "{items:?}");
    click_item(&mut r.h, UnlinkCopies);
    assert!(object(&r.s, copy)["id"].is_u64());
    // Unlinked: the copy is on its own again.
    let st = r.s.lock().unwrap();
    assert!(
        st.doc()
            .document
            .get(flatcraft_core::document::ObjectId(copy))
            .unwrap()
            .link_group
            .is_none()
    );
}

#[test]
fn link_copies_is_offered_for_copies_of_one_source() {
    let mut r = rig(1);
    let d = exec(&r.s, "object.duplicate", json!({ "ids": [r.ids[0]] }));
    let dup = d["ids"][0].as_u64().unwrap();
    select(&r.s, &[r.ids[0], dup]);
    r.h.run();
    r.right_click_object(0);
    assert!(r.items().contains(&LinkCopies), "{:?}", r.items());
    click_item(&mut r.h, LinkCopies);
    let group = |id: u64| {
        r.s.lock()
            .unwrap()
            .doc()
            .document
            .get(flatcraft_core::document::ObjectId(id))
            .unwrap()
            .link_group
    };
    assert!(group(dup).is_some());
    assert_eq!(group(dup), group(r.ids[0]));
}

#[test]
fn the_menu_of_the_empty_mat() {
    let mut r = rig(2);
    right_click(&mut r.h, at_mm(&r.view, 200.0, 120.0));
    assert_eq!(r.items(), vec![Import, SelectAll, AutoArrange, ZoomToFit]);
    // No objects: nothing to select or arrange.
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    to_prepare(&mut h);
    let rect = app(&h).ui.prepare.canvas_rect;
    right_click(&mut h, rect.center());
    assert_eq!(shown(&h), vec![Import, ZoomToFit]);
}

#[test]
fn a_hidden_object_offers_show_and_a_skipped_one_cut_again() {
    let mut r = rig(1);
    exec(
        &r.s,
        "object.set_cut",
        json!({ "ids": [r.ids[0]], "cut": false }),
    );
    r.h.run();
    r.right_click_object(0);
    let items = r.items();
    assert!(items.contains(&CutAgain) && !items.contains(&SkipCutting));
    click_item(&mut r.h, CutAgain);
    assert_eq!(object(&r.s, r.ids[0])["cut_enabled"], true);
    // Hidden objects are reached through the Objects panel.
    exec(
        &r.s,
        "object.set_visible",
        json!({ "ids": [r.ids[0]], "visible": false }),
    );
    r.h.run();
    r.h.get_by_role_and_label(Role::Button, "square")
        .click_secondary();
    r.h.run();
    r.h.run();
    let items = r.items();
    assert!(items.contains(&Show) && !items.contains(&Hide), "{items:?}");
}

// ----------------------------------------------------------- the commands

#[test]
fn duplicate_and_delete_run_the_ribbon_commands() {
    let mut r = rig(1);
    r.right_click_object(0);
    click_item(&mut r.h, Duplicate);
    assert_eq!(object_count(&r.s), 2);
    // The copy is the selection; delete it from its own menu.
    let copy = selection(&r.s)[0];
    assert_ne!(copy, r.ids[0]);
    let b = object(&r.s, copy)["bounds_mm"].clone();
    let p = at_mm(
        &r.view,
        (b["x0"].as_f64().unwrap() + b["x1"].as_f64().unwrap()) / 2.0,
        (b["y0"].as_f64().unwrap() + b["y1"].as_f64().unwrap()) / 2.0,
    );
    right_click(&mut r.h, p);
    click_item(&mut r.h, Delete);
    assert_eq!(object_count(&r.s), 1);
}

#[test]
fn copies_opens_the_dialog() {
    let mut r = rig(1);
    r.right_click_object(0);
    click_item(&mut r.h, Copies);
    assert_eq!(app(&r.h).ui.prepare.copies.as_ref().unwrap().id, r.ids[0]);
    r.h.get_by_label("Make copies");
}

#[test]
fn rotate_and_mirror_run_the_ribbon_commands() {
    let mut r = rig(1);
    let rotation = |r: &Rig| {
        object(&r.s, r.ids[0])["transform"]["rotation_deg"]
            .as_f64()
            .unwrap()
    };
    r.right_click_object(0);
    click_item(&mut r.h, RotateRight);
    assert!((rotation(&r) - 90.0).abs() < 1e-6, "{}", rotation(&r));
    r.right_click_object(0);
    click_item(&mut r.h, RotateLeft);
    assert!(rotation(&r).abs() < 1e-6);
    r.right_click_object(0);
    click_item(&mut r.h, MirrorHorizontal);
    let mirrored = object(&r.s, r.ids[0])["transform"]["mirror"].clone();
    assert_ne!(mirrored, json!(null));
    assert!(!matches!(mirrored.as_str(), Some("none")), "{mirrored}");
    let undo = undo_len(&r.s);
    r.right_click_object(0);
    click_item(&mut r.h, MirrorVertical);
    assert!(undo_len(&r.s) > undo);
}

#[test]
fn align_and_distribute_run_the_ribbon_commands() {
    let mut r = rig(3);
    // Stagger the middle one, then align the three to the top.
    exec(
        &r.s,
        "object.move",
        json!({ "ids": [r.ids[1]], "dx_mm": 0.0, "dy_mm": 30.0 }),
    );
    select(&r.s, &r.ids.clone());
    r.h.run();
    let top = |r: &Rig, i: usize| object(&r.s, r.ids[i])["bounds_mm"]["y0"].as_f64().unwrap();
    assert!((top(&r, 1) - top(&r, 0)).abs() > 1.0);
    r.right_click_object(0);
    click_sub(&mut r.h, Align, "Top");
    assert!((top(&r, 1) - top(&r, 0)).abs() < 1e-6);
    assert!(!menu_open(&r.h));
    // Distribute: move the middle one sideways; the gaps become equal.
    exec(
        &r.s,
        "object.move",
        json!({ "ids": [r.ids[1]], "dx_mm": 30.0, "dy_mm": 0.0 }),
    );
    r.h.run();
    let left = |r: &Rig, i: usize| object(&r.s, r.ids[i])["bounds_mm"]["x0"].as_f64().unwrap();
    let uneven = (left(&r, 1) - left(&r, 0)) - (left(&r, 2) - left(&r, 1));
    assert!(uneven.abs() > 1.0);
    r.right_click_object(0);
    click_sub(&mut r.h, Distribute, "horizontally");
    let even = (left(&r, 1) - left(&r, 0)) - (left(&r, 2) - left(&r, 1));
    assert!(even.abs() < 1e-6, "{even}");
}

#[test]
fn hide_and_skip_cutting_run_the_panel_commands() {
    let mut r = rig(2);
    select(&r.s, &r.ids.clone());
    r.h.run();
    r.right_click_object(0);
    click_item(&mut r.h, SkipCutting);
    // The whole selection takes the new state, as the scissors do.
    for id in &r.ids {
        assert_eq!(object(&r.s, *id)["cut_enabled"], false);
    }
    r.right_click_object(0);
    click_item(&mut r.h, Hide);
    for id in &r.ids {
        assert_eq!(object(&r.s, *id)["visible"], false);
    }
}

#[test]
fn rename_asks_for_a_name_and_runs_the_command() {
    let mut r = rig(1);
    r.right_click_object(0);
    click_item(&mut r.h, Rename);
    assert!(app(&r.h).ui.prepare.rename.is_some());
    r.h.run();
    r.h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    r.h.run();
    r.h.get_by_label("Name").type_text("Lid");
    r.h.run();
    r.h.key_press(Key::Enter);
    r.h.run();
    assert_eq!(object(&r.s, r.ids[0])["name"], "Lid");
    assert!(app(&r.h).ui.prepare.rename.is_none());
    // Undoable.
    exec(&r.s, "edit.undo", json!({}));
    assert_eq!(object(&r.s, r.ids[0])["name"], "square");
}

#[test]
fn rename_cancel_changes_nothing() {
    let mut r = rig(1);
    r.right_click_object(0);
    click_item(&mut r.h, Rename);
    r.h.get_by_role_and_label(Role::Button, "Cancel").click();
    r.h.run();
    assert!(app(&r.h).ui.prepare.rename.is_none());
    assert_eq!(object(&r.s, r.ids[0])["name"], "square");
}

#[test]
fn reload_and_reconnect_follow_the_linked_file() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let file = dir.path().join("a.svg");
    write_square(&file, 50.0);
    let id = import_file(&s, &file);
    let picker = FakePicker::default();
    let p = picker.clone();
    let mut h = harness_with(&dir, s.clone(), move |app| app.with_file_picker(p));
    to_prepare(&mut h);
    let rect = app(&h).ui.prepare.canvas_rect;
    let b = object(&s, id)["bounds_mm"].clone();
    let (cx, cy) = (
        (b["x0"].as_f64().unwrap() + b["x1"].as_f64().unwrap()) / 2.0,
        (b["y0"].as_f64().unwrap() + b["y1"].as_f64().unwrap()) / 2.0,
    );
    let view = CanvasView {
        offset: rect.center().to_vec2() - Vec2::new(cx as f32, cy as f32) * 2.0,
        px_per_mm: 2.0,
    };
    app_mut(&mut h).ui.prepare.view = view;
    app_mut(&mut h).ui.prepare.fit_pending = false;
    h.run();
    let width = |s: &SharedSession| {
        let b = object(s, id)["bounds_mm"].clone();
        b["x1"].as_f64().unwrap() - b["x0"].as_f64().unwrap()
    };
    assert!((width(&s) - 50.0).abs() < 1e-3, "{}", width(&s));
    // The file grows; Reload reads it.
    write_square(&file, 80.0);
    right_click(&mut h, at_mm(&view, cx, cy));
    let items = shown(&h);
    assert!(
        items.contains(&Reload) && !items.contains(&Reconnect),
        "{items:?}"
    );
    click_item(&mut h, Reload);
    assert!((width(&s) - 80.0).abs() < 1e-3, "{}", width(&s));
    // The file goes away: Reconnect asks where it went.
    let moved: PathBuf = dir.path().join("moved.svg");
    std::fs::rename(&file, &moved).unwrap();
    h.event(Event::WindowFocused(false));
    h.run();
    h.event(Event::WindowFocused(true));
    wait_for_workers(&mut h);
    let b = object(&s, id)["bounds_mm"].clone();
    let (cx, cy) = (
        (b["x0"].as_f64().unwrap() + b["x1"].as_f64().unwrap()) / 2.0,
        (b["y0"].as_f64().unwrap() + b["y1"].as_f64().unwrap()) / 2.0,
    );
    picker.answers.lock().unwrap().push(moved.clone());
    right_click(&mut h, at_mm(&view, cx, cy));
    let items = shown(&h);
    assert!(
        items.contains(&Reconnect) && !items.contains(&Reload),
        "{items:?}"
    );
    click_item(&mut h, Reconnect);
    assert_eq!(picker.requests.lock().unwrap().len(), 1);
    let link = s
        .lock()
        .unwrap()
        .doc()
        .document
        .get(flatcraft_core::document::ObjectId(id))
        .unwrap()
        .link
        .clone()
        .unwrap();
    assert_eq!(link.path, moved);
}

#[test]
fn mat_items_import_select_all_arrange_and_fit() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let ids = squares(&s, 2);
    let picker = FakePicker::default();
    let p = picker.clone();
    let mut h = harness_with(&dir, s.clone(), move |app| app.with_file_picker(p));
    to_prepare(&mut h);
    let view = known_view(&mut h);
    let empty = at_mm(&view, 200.0, 120.0);
    // Select all.
    right_click(&mut h, empty);
    click_item(&mut h, SelectAll);
    assert_eq!(selection(&s), ids);
    // Auto arrange.
    let undo = undo_len(&s);
    right_click(&mut h, empty);
    click_item(&mut h, AutoArrange);
    assert!(undo_len(&s) > undo);
    // Zoom to fit changes the view.
    let view = known_view(&mut h);
    right_click(&mut h, at_mm(&view, 200.0, 120.0));
    click_item(&mut h, ZoomToFit);
    h.run();
    assert_ne!(app(&h).ui.prepare.view, view);
    // Import asks the picker.
    let view = app(&h).ui.prepare.view;
    right_click(&mut h, at_mm(&view, 300.0, 300.0));
    click_item(&mut h, Import);
    assert_eq!(picker.requests.lock().unwrap().len(), 1);
}

// ------------------------------------------------------ closing and keys

#[test]
fn escape_closes_the_menu_and_keeps_the_selection() {
    let mut r = rig(1);
    r.right_click_object(0);
    assert!(menu_open(&r.h));
    r.h.key_press(Key::Escape);
    r.h.run();
    assert!(!menu_open(&r.h));
    assert_eq!(selection(&r.s), vec![r.ids[0]]);
}

#[test]
fn a_click_outside_closes_the_menu() {
    let mut r = rig(1);
    r.right_click_object(0);
    assert!(menu_open(&r.h));
    left_click(&mut r.h, at_mm(&r.view, 200.0, 120.0));
    assert!(!menu_open(&r.h));
}

#[test]
fn delete_key_waits_while_the_menu_is_open() {
    let mut r = rig(1);
    r.right_click_object(0);
    assert!(menu_open(&r.h));
    r.h.key_press(Key::Delete);
    r.h.run();
    assert_eq!(object_count(&r.s), 1);
}

#[test]
fn cmd_d_duplicates_and_cmd_i_imports() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let picker = FakePicker::default();
    let p = picker.clone();
    let mut h = harness_with(&dir, s.clone(), move |app| app.with_file_picker(p));
    to_prepare(&mut h);
    select(&s, &[id]);
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.run();
    assert_eq!(object_count(&s), 2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::I);
    h.run();
    assert_eq!(picker.requests.lock().unwrap().len(), 1);
}

#[test]
fn menu_hints_come_from_the_shortcut_table() {
    for (item, what) in [
        (Duplicate, "Duplicate the selection"),
        (Delete, "Delete the selection"),
        (Import, "Import a design"),
        (SelectAll, "Select all objects"),
    ] {
        let row = flatcraft::ui::help::SHORTCUTS
            .iter()
            .find(|s| s.what == what)
            .unwrap();
        assert_eq!(
            item.hint().as_deref(),
            Some(row.keys().as_str()),
            "{item:?}"
        );
    }
    assert_eq!(Rename.hint(), None);
}

#[test]
fn no_em_dash_in_the_menu() {
    for item in ALL {
        assert!(!item.label().contains('\u{2014}'));
    }
}
