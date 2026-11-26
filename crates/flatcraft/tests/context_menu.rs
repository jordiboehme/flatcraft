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

const ALL: [Item; 29] = [
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
    Item::Group,
    Item::Ungroup,
    Item::Hide,
    Item::Show,
    Item::SkipCutting,
    Item::CutAgain,
    Item::PickKeep,
    Item::Rename,
    Item::TraceAgain,
    Item::Reload,
    Item::Reconnect,
    Item::RevealSource,
    Item::CopySourcePath,
    Item::ExportSelected,
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

/// Whether the open menu shows `item` dimmed.
fn dimmed(h: &AppHarness, item: Item) -> bool {
    h.get_by_role(Role::Menu)
        .get_by_label(&name(item))
        .accesskit_node()
        .is_disabled()
}

/// The dimmed items of the open menu, in order.
fn dimmed_items(h: &AppHarness) -> Vec<Item> {
    shown(h).into_iter().filter(|i| dimmed(h, *i)).collect()
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
            LinkCopies,
            RotateLeft,
            RotateRight,
            MirrorHorizontal,
            MirrorVertical,
            Align,
            Distribute,
            Group,
            Ungroup,
            Hide,
            SkipCutting,
            PickKeep,
            Rename,
            ExportSelected,
            Delete
        ]
    );
    // Not now: shown dimmed, with the reason on hover.
    assert_eq!(
        dimmed_items(&r.h),
        vec![LinkCopies, Distribute, Group, Ungroup]
    );
}

#[test]
fn trace_again_with_several_objects_is_dimmed_and_link_follows_the_design() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let png = dir.path().join("ring.png");
    write_ring(&png);
    let pic = import_file(&s, &png);
    let square = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
    let st = &app(&h).ui.prepare;
    let flat = |ids: &[u64]| -> Vec<Item> {
        flatcraft::ui::context_menu::items(&snap, st, ids)
            .into_iter()
            .flatten()
            .collect()
    };
    // A picture with another object: Trace again is listed, dimmed.
    let both = [pic, square];
    assert!(flat(&both).contains(&TraceAgain));
    assert_eq!(
        TraceAgain.why(&snap, st, &both),
        Some("Select only one picture to trace it again.")
    );
    assert_eq!(TraceAgain.why(&snap, st, &[pic]), None);
    // Only vectors: it can never apply, so it is left out.
    assert!(!flat(&[square]).contains(&TraceAgain));
    // Link copies: one object says why; different designs can never be
    // linked, so it is left out.
    assert!(flat(&[square]).contains(&LinkCopies));
    assert!(LinkCopies.why(&snap, st, &[square]).is_some());
    assert!(!flat(&both).contains(&LinkCopies));
}

#[test]
fn the_menu_of_two_objects() {
    let mut r = rig(2);
    select(&r.s, &r.ids.clone());
    r.h.run();
    r.right_click_object(0);
    // Two imports of one file: the same bytes, so they can be linked.
    assert_eq!(
        r.items(),
        vec![
            Duplicate,
            Copies,
            LinkCopies,
            RotateLeft,
            RotateRight,
            MirrorHorizontal,
            MirrorVertical,
            Align,
            Distribute,
            Group,
            Ungroup,
            Hide,
            SkipCutting,
            PickKeep,
            Rename,
            ExportSelected,
            Delete
        ]
    );
    assert_eq!(
        dimmed_items(&r.h),
        vec![Copies, Distribute, Ungroup, Rename]
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
    // Rename and Copies are for one object: dimmed, not gone.
    assert_eq!(dimmed_items(&r.h), vec![Copies, Ungroup, Rename]);
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
    assert!(dimmed_items(&r.h).is_empty());
    // No objects: nothing to select or arrange, so those are dimmed.
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    to_prepare(&mut h);
    let rect = app(&h).ui.prepare.canvas_rect;
    right_click(&mut h, rect.center());
    assert_eq!(shown(&h), vec![Import, SelectAll, AutoArrange, ZoomToFit]);
    assert_eq!(dimmed_items(&h), vec![SelectAll, AutoArrange]);
}

#[test]
fn a_dimmed_item_says_why_and_does_nothing() {
    let mut r = rig(2);
    select(&r.s, &r.ids.clone());
    r.h.run();
    r.h.ctx
        .global_style_mut(|s| s.interaction.tooltip_delay = 0.0);
    r.right_click_object(0);
    assert!(dimmed(&r.h, Distribute));
    // Its card: the reason first, then what it does.
    let at =
        r.h.get_by_role(Role::Menu)
            .get_by_label(&name(Distribute))
            .rect()
            .center();
    r.h.hover_at(egui::pos2(at.x, at.y));
    r.h.run();
    r.h.run();
    r.h.get_by_label("Select 3 or more objects first.");
    r.h.get_by_label(Distribute.description());
    let undo = undo_len(&r.s);
    r.h.get_by_role(Role::Menu)
        .get_by_role_and_label(Role::Button, &name(Distribute))
        .click();
    r.h.run();
    assert_eq!(undo_len(&r.s), undo, "nothing ran");
    // The titles of ribbon actions come from the table.
    for item in [
        Duplicate,
        Copies,
        RotateLeft,
        MirrorVertical,
        Delete,
        AutoArrange,
    ] {
        assert_eq!(Some(item.label()), item.tool().map(|t| t.title));
    }
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
fn copies_opens_the_rectangular_pattern_panel() {
    let mut r = rig(1);
    r.right_click_object(0);
    click_item(&mut r.h, Copies);
    let p = app(&r.h).ui.prepare.panel.as_ref().unwrap();
    assert_eq!(
        p.kind,
        flatcraft::ui::arrange::PanelKind::RectangularPattern
    );
    r.h.get_by_role_and_label(egui::accesskit::Role::Button, "OK");
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
    click_sub(&mut r.h, Align, "Align top");
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
    click_sub(&mut r.h, Distribute, "Distribute horizontally");
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
        // Delete has two rows; a Mac shows the Backspace one.
        let want = if item == Delete && cfg!(target_os = "macos") {
            "Backspace".to_string()
        } else {
            row.keys()
        };
        assert_eq!(item.hint().as_deref(), Some(want.as_str()), "{item:?}");
    }
    assert_eq!(Rename.hint(), None);
}

#[test]
fn no_em_dash_in_the_menu() {
    for item in ALL {
        assert!(!item.label().contains('\u{2014}'));
        assert!(!item.description().contains('\u{2014}'), "{item:?}");
        assert!(!item.description().trim().is_empty(), "{item:?}");
    }
}

#[test]
fn shifted_keys_are_not_duplicate_or_import() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let picker = FakePicker::default();
    let p = picker.clone();
    let mut h = harness_with(&dir, s.clone(), move |app| app.with_file_picker(p));
    to_prepare(&mut h);
    select(&s, &[id]);
    h.run();
    for key in [Key::D, Key::I, Key::A] {
        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, key);
        h.run();
    }
    assert_eq!(object_count(&s), 1);
    assert!(picker.requests.lock().unwrap().is_empty());
}

#[test]
fn cmd_d_and_cmd_i_wait_while_a_text_field_has_the_keys() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let picker = FakePicker::default();
    let p = picker.clone();
    let mut h = harness_with(&dir, s.clone(), move |app| app.with_file_picker(p));
    to_prepare(&mut h);
    select(&s, &[id]);
    h.run();
    h.get_by_label("Search commands").click();
    h.run();
    for key in [Key::D, Key::I] {
        h.key_press_modifiers(Modifiers::COMMAND, key);
        h.run();
    }
    assert_eq!(object_count(&s), 1);
    assert!(picker.requests.lock().unwrap().is_empty());
}

fn save_and_reopen(dir: &tempfile::TempDir, s: &SharedSession) -> SharedSession {
    let path = dir.path().join("p.flatcraft");
    exec(s, "project.save", json!({ "path": path }));
    let fresh = session(dir);
    exec(&fresh, "project.open", json!({ "path": path }));
    fresh
}

#[test]
fn link_copies_is_offered_after_a_reopen_and_for_two_imports_of_one_file() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let d = exec(&s, "object.duplicate", json!({ "ids": [a] }));
    let dup = d["ids"][0].as_u64().unwrap();
    exec(
        &s,
        "object.move",
        json!({ "ids": [dup], "dx_mm": 100.0, "dy_mm": 0.0 }),
    );
    // Two imports of one file.
    let file = dir.path().join("a.svg");
    write_square(&file, 50.0);
    let f1 = import_file(&s, &file);
    let f2 = import_file(&s, &file);
    exec(
        &s,
        "object.move",
        json!({ "ids": [f1], "dx_mm": 0.0, "dy_mm": 100.0 }),
    );
    exec(
        &s,
        "object.move",
        json!({ "ids": [f2], "dx_mm": 100.0, "dy_mm": 100.0 }),
    );
    for (sess, pair) in [(s.clone(), [f1, f2]), (save_and_reopen(&dir, &s), [a, dup])] {
        select(&sess, &pair);
        let mut h = harness(&dir, sess.clone(), false);
        to_prepare(&mut h);
        let rect = app(&h).ui.prepare.canvas_rect;
        let b = object(&sess, pair[0])["bounds_mm"].clone();
        let view = CanvasView {
            offset: rect.center().to_vec2() - Vec2::new(45.0, 45.0) * 2.0,
            px_per_mm: 2.0,
        };
        app_mut(&mut h).ui.prepare.view = view;
        app_mut(&mut h).ui.prepare.fit_pending = false;
        wait_for_workers(&mut h);
        select(&sess, &pair);
        let cx = (b["x0"].as_f64().unwrap() + b["x1"].as_f64().unwrap()) / 2.0;
        let cy = (b["y0"].as_f64().unwrap() + b["y1"].as_f64().unwrap()) / 2.0;
        right_click(&mut h, at_mm(&view, cx, cy));
        let items = shown(&h);
        assert!(items.contains(&LinkCopies), "{pair:?}: {items:?}");
    }
}

#[test]
fn export_selected_opens_the_export_dialog_on_the_selection() {
    let mut r = rig(2);
    r.right_click_object(1);
    assert!(r.items().contains(&ExportSelected));
    assert!(!dimmed(&r.h, ExportSelected));
    click_item(&mut r.h, ExportSelected);
    r.h.run();
    let d = app(&r.h)
        .ui
        .export
        .clone()
        .expect("the Export dialog is open");
    assert_eq!(d.scope, flatcraft_engine::export::ExportScope::Selected);
    assert_eq!(selection(&r.s), vec![r.ids[1]]);
}

#[test]
fn export_selected_is_dimmed_for_hidden_objects_and_says_why() {
    let mut r = rig(1);
    exec(
        &r.s,
        "object.set_visible",
        json!({ "ids": [r.ids[0]], "visible": false }),
    );
    r.h.run();
    let ctx = r.h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut r.h), &ctx);
    let st = &app(&r.h).ui.prepare;
    assert_eq!(
        ExportSelected.why(&snap, st, &[r.ids[0]]),
        Some(flatcraft::ui::export_dialog::ALL_HIDDEN)
    );
    assert!(!ExportSelected.description().is_empty());
}

#[test]
fn auto_arrange_on_the_empty_mat_is_active_from_the_first_frame() {
    // Only a hidden object is selected. The empty-mat menu clears the
    // selection, so its Auto arrange packs every visible object: it must
    // never look dimmed, not even in the frame the menu opens.
    let mut r = rig(2);
    r.s.lock()
        .unwrap()
        .execute(
            "object.set_visible",
            json!({ "ids": [r.ids[0]], "visible": false }),
            Origin::User,
        )
        .unwrap();
    select(&r.s, &r.ids[..1]);
    r.h.run();
    wait_for_workers(&mut r.h);
    // The item is judged by what it will do (the cleared selection), not
    // by the selection the snapshot still holds.
    let ctx = r.h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut r.h), &ctx);
    assert_eq!(snap.selection.len(), 1);
    assert_eq!(
        AutoArrange.why(&snap, &app(&r.h).ui.prepare, &[]),
        None,
        "dimmed while the old selection is still in the snapshot"
    );
    let p = at_mm(&r.view, 200.0, 120.0);
    r.h.event(Event::PointerMoved(p));
    r.h.run();
    for pressed in [true, false] {
        r.h.event(Event::PointerButton {
            pos: p,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    // Frame by frame, up to the first one that shows the menu.
    let mut frames = 0;
    while !menu_open(&r.h) {
        r.h.step();
        frames += 1;
        assert!(frames < 10, "the menu does not open");
    }
    assert!(!dimmed(&r.h, AutoArrange), "dimmed in frame {frames}");
    r.h.run();
    assert!(!dimmed(&r.h, AutoArrange));
    // It packs the visible object, the hidden one stays.
    let undo = r.s.lock().unwrap().doc().undo.undo_len();
    click_item(&mut r.h, AutoArrange);
    wait_for_workers(&mut r.h);
    assert_eq!(r.s.lock().unwrap().doc().undo.undo_len(), undo + 1);
}
