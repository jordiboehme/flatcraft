//! Selecting on the canvas: the box (fully inside), the lasso (touching),
//! Shift to flip, the Select/Lasso toggle and its keys, Escape, and the
//! same rule as `selection.set_in_polygon`.

mod common;
use common::RunPlan;

use common::{AppHarness, app, app_mut, harness, import_square, session, to_prepare, undo_len};
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::canvas::{self, CanvasView};
use flatcraft::ui::prepare::{Drag, SelectTool, Snapshot};
use flatcraft_engine::lasso::Lasso;
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

/// Three 50 mm squares 100 mm apart: boxes x 20-70, 120-170 and 220-270,
/// y 20-70. Nothing selected.
fn three(dir: &tempfile::TempDir) -> (SharedSession, [u64; 3]) {
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
    exec(&s, "object.select", json!({ "ids": [] }));
    (s, [ids[0], ids[1], ids[2]])
}

/// Prepare with 1 mm = 2 points and 145,45 mm at the canvas centre.
fn rig(dir: &tempfile::TempDir, s: SharedSession) -> (AppHarness, CanvasView) {
    let mut h = harness(dir, s, false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.run_plan();
    let rect = app(&h).ui.prepare.canvas_rect;
    let view = CanvasView {
        offset: rect.center().to_vec2() - Vec2::new(145.0, 45.0) * 2.0,
        px_per_mm: 2.0,
    };
    let p = &mut app_mut(&mut h).ui.prepare;
    p.view = view;
    p.fit_pending = false;
    h.run_plan();
    (h, view)
}

fn at(v: &CanvasView, x: f64, y: f64) -> Pos2 {
    v.mm_to_screen(kurbo::Point::new(x, y))
}

/// Holds `mods`, presses at the first point, moves through the others in
/// small steps and lets go at the last.
fn trace(h: &mut AppHarness, path: &[Pos2], mods: Modifiers) {
    h.event(Event::ModifiersChanged(mods));
    h.hover_at(path[0]);
    h.run_plan();
    h.drag_at(path[0]);
    h.run_plan();
    for w in path.windows(2) {
        for t in [0.25, 0.5, 0.75, 1.0] {
            h.hover_at(w[0] + (w[1] - w[0]) * t);
            h.run_plan();
        }
    }
    h.drop_at(*path.last().unwrap());
    h.run_plan();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_plan();
}

fn click(h: &mut AppHarness, p: Pos2, mods: Modifiers) {
    h.event(Event::ModifiersChanged(mods));
    h.event(Event::PointerMoved(p));
    h.run_plan();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed,
            modifiers: mods,
        });
        h.step();
    }
    h.run_plan();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_plan();
}

/// A lasso from above the first square, around it and into the left half
/// of the second, back under the first.
fn over_a_into_b(v: &CanvasView) -> Vec<Pos2> {
    [
        (10.0, 10.0),
        (140.0, 10.0),
        (140.0, 80.0),
        (10.0, 80.0),
        (10.0, 12.0),
    ]
    .map(|(x, y)| at(v, x, y))
    .to_vec()
}

#[test]
fn option_drag_on_the_empty_mat_draws_a_lasso_that_selects_what_it_touches() {
    let dir = tempfile::tempdir().unwrap();
    let (s, [a, b, _]) = three(&dir);
    let (mut h, v) = rig(&dir, s.clone());
    let undo = undo_len(&s);
    let path = over_a_into_b(&v);
    // While it is drawn the canvas holds a lasso with its points.
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.hover_at(path[0]);
    h.run_plan();
    h.drag_at(path[0]);
    h.run_plan();
    for p in &path[1..3] {
        h.hover_at(*p);
        h.run_plan();
    }
    match &app(&h).ui.prepare.drag {
        Some(Drag::Frame {
            lasso: Some(points),
            ..
        }) => assert!(points.len() >= 2, "{points:?}"),
        other => panic!("no lasso: {other:?}"),
    }
    h.drop_at(path[2]);
    h.run_plan();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_plan();
    // The whole path: a is enclosed, b touched, c apart.
    trace(&mut h, &path, Modifiers::ALT);
    assert_eq!(selection(&s), [a, b]);
    assert!(app(&h).ui.prepare.drag.is_none());
    // The same drag without Option is a box: only a lies fully inside.
    trace(&mut h, &[path[0], path[2]], Modifiers::NONE);
    assert_eq!(selection(&s), [a]);
    // Selecting adds no undo step.
    assert_eq!(undo_len(&s), undo);
}

#[test]
fn the_lasso_tool_lassos_from_over_an_object_and_l_and_v_switch_it() {
    let dir = tempfile::tempdir().unwrap();
    let (s, [a, b, _]) = three(&dir);
    let (mut h, v) = rig(&dir, s.clone());
    h.key_press(Key::L);
    h.run_plan();
    assert_eq!(app(&h).ui.prepare.select_tool, SelectTool::Lasso);
    let before = exec(&s, "document.state", json!({}))["objects"][0]["bounds_mm"].clone();
    // From inside a, out over b's left part and back: a lasso, no move.
    let path =
        [(45.0, 45.0), (130.0, 45.0), (130.0, 60.0), (45.0, 60.0)].map(|(x, y)| at(&v, x, y));
    trace(&mut h, &path, Modifiers::NONE);
    assert_eq!(selection(&s), [a, b]);
    let after = exec(&s, "document.state", json!({}))["objects"][0]["bounds_mm"].clone();
    assert_eq!(before, after, "nothing moved");
    // The toggle on the Layout tab shows it, and V switches back.
    h.get_by_label("Lasso");
    h.key_press(Key::V);
    h.run_plan();
    assert_eq!(app(&h).ui.prepare.select_tool, SelectTool::Select);
    // The Lasso switch of the ribbon picks the tool too.
    h.get_by_role_and_label(Role::CheckBox, "Lasso").click();
    h.run_plan();
    assert_eq!(app(&h).ui.prepare.select_tool, SelectTool::Lasso);
    h.get_by_role_and_label(Role::CheckBox, "Select").click();
    h.run_plan();
    assert_eq!(app(&h).ui.prepare.select_tool, SelectTool::Select);
}

#[test]
fn shift_flips_objects_with_the_lasso_the_box_and_a_click() {
    let dir = tempfile::tempdir().unwrap();
    let (s, [a, b, c]) = three(&dir);
    let (mut h, v) = rig(&dir, s.clone());
    exec(&s, "object.select", json!({ "ids": [a] }));
    h.run_plan();
    // Shift+Option lasso over a and part of b: a leaves, b joins.
    trace(
        &mut h,
        &over_a_into_b(&v),
        Modifiers::ALT | Modifiers::SHIFT,
    );
    assert_eq!(selection(&s), [b]);
    // Shift+click c: it joins; again: it leaves.
    click(&mut h, at(&v, 245.0, 45.0), Modifiers::SHIFT);
    assert_eq!(selection(&s), [b, c]);
    click(&mut h, at(&v, 245.0, 45.0), Modifiers::SHIFT);
    assert_eq!(selection(&s), [b]);
    // Shift+box around b and c: b leaves, c joins.
    trace(
        &mut h,
        &[at(&v, 110.0, 10.0), at(&v, 280.0, 80.0)],
        Modifiers::SHIFT,
    );
    assert_eq!(selection(&s), [c]);
    // A Shift+click on the empty mat changes nothing.
    click(&mut h, at(&v, 95.0, 100.0), Modifiers::SHIFT);
    assert_eq!(selection(&s), [c]);
}

#[test]
fn escape_cancels_a_lasso_then_leaves_the_lasso_tool() {
    let dir = tempfile::tempdir().unwrap();
    let (s, [a, ..]) = three(&dir);
    let (mut h, v) = rig(&dir, s.clone());
    exec(&s, "object.select", json!({ "ids": [a] }));
    app_mut(&mut h).ui.prepare.select_tool = SelectTool::Lasso;
    h.run_plan();
    let path = over_a_into_b(&v);
    h.hover_at(path[0]);
    h.run_plan();
    h.drag_at(path[0]);
    h.run_plan();
    for p in &path[1..4] {
        h.hover_at(*p);
        h.run_plan();
    }
    assert!(app(&h).ui.prepare.drag.is_some());
    h.key_press(Key::Escape);
    h.run_plan();
    assert!(app(&h).ui.prepare.drag.is_none(), "cancelled");
    h.hover_at(path[4]);
    h.run_plan();
    h.drop_at(path[4]);
    h.run_plan();
    // Nothing changed, the tool stays.
    assert_eq!(selection(&s), [a]);
    assert_eq!(app(&h).ui.prepare.select_tool, SelectTool::Lasso);
    // Escape again: back to the Select tool, the selection stays.
    h.key_press(Key::Escape);
    h.run_plan();
    assert_eq!(app(&h).ui.prepare.select_tool, SelectTool::Select);
    assert_eq!(selection(&s), [a]);
    // Then Escape clears the selection as always.
    h.key_press(Key::Escape);
    h.run_plan();
    assert!(selection(&s).is_empty());
}

/// A press held still until egui calls it a drag (by the harness's
/// clock), a move of `by` and a release.
fn held_drag(h: &mut AppHarness, from: Pos2, by: Vec2, mods: Modifiers) {
    h.event(Event::ModifiersChanged(mods));
    h.hover_at(from);
    h.run_plan();
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: mods,
    });
    for _ in 0..600 {
        h.step();
        if app(h).ui.prepare.drag.is_some() {
            break;
        }
    }
    assert!(app(h).ui.prepare.drag.is_some(), "the press became a drag");
    h.hover_at(from + by);
    h.step();
    h.event(Event::PointerButton {
        pos: from + by,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: mods,
    });
    h.run_plan();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_plan();
}

#[test]
fn a_lasso_or_box_under_2_points_is_a_click() {
    let dir = tempfile::tempdir().unwrap();
    let (s, [a, ..]) = three(&dir);
    let (mut h, v) = rig(&dir, s.clone());
    let empty = at(&v, 95.0, 100.0);
    for tool in [SelectTool::Lasso, SelectTool::Select] {
        exec(&s, "object.select", json!({ "ids": [a] }));
        app_mut(&mut h).ui.prepare.select_tool = tool;
        h.run_plan();
        // With Shift a click on the empty mat keeps the selection.
        held_drag(&mut h, empty, Vec2::new(1.0, 0.5), Modifiers::SHIFT);
        assert_eq!(selection(&s), [a], "{tool:?}");
        // Without, it clears it, as a click there does.
        held_drag(&mut h, empty, Vec2::new(1.0, 0.5), Modifiers::NONE);
        assert!(selection(&s).is_empty(), "{tool:?}");
    }
    // A tiny lasso on an object picks it, as a click.
    app_mut(&mut h).ui.prepare.select_tool = SelectTool::Lasso;
    held_drag(
        &mut h,
        at(&v, 45.0, 45.0),
        Vec2::new(0.5, 0.5),
        Modifiers::NONE,
    );
    assert_eq!(selection(&s), [a]);
}

#[test]
fn a_group_is_lassoed_whole_and_inside_it_its_members() {
    let dir = tempfile::tempdir().unwrap();
    let (s, [a, b, c]) = three(&dir);
    exec(&s, "object.group", json!({ "ids": [a, b] }));
    exec(&s, "object.select", json!({ "ids": [] }));
    let (mut h, v) = rig(&dir, s.clone());
    // Touching one member picks the whole group.
    let around_a =
        [(10.0, 10.0), (80.0, 10.0), (80.0, 80.0), (10.0, 80.0)].map(|(x, y)| at(&v, x, y));
    trace(&mut h, &around_a, Modifiers::ALT);
    assert_eq!(selection(&s), [a, b]);
    // A box around one member: the group is not fully inside.
    trace(&mut h, &[around_a[0], around_a[2]], Modifiers::NONE);
    assert!(selection(&s).is_empty());
    // Go into the group (Cmd-click a member); a lasso over b and c picks
    // the member b alone, nothing outside the group.
    click(&mut h, at(&v, 45.0, 45.0), Modifiers::COMMAND);
    assert_eq!(selection(&s), [a]);
    let over_b_c =
        [(110.0, 10.0), (280.0, 10.0), (280.0, 80.0), (110.0, 80.0)].map(|(x, y)| at(&v, x, y));
    trace(&mut h, &over_b_c, Modifiers::ALT);
    assert_eq!(selection(&s), [b]);
    let _ = c;
}

#[test]
fn hidden_objects_are_never_lassoed() {
    let dir = tempfile::tempdir().unwrap();
    let (s, [a, b, c]) = three(&dir);
    exec(
        &s,
        "object.set_visible",
        json!({ "ids": [b], "visible": false }),
    );
    let (mut h, v) = rig(&dir, s.clone());
    let all = [(10.0, 10.0), (280.0, 10.0), (280.0, 80.0), (10.0, 80.0)].map(|(x, y)| at(&v, x, y));
    trace(&mut h, &all, Modifiers::ALT);
    assert_eq!(selection(&s), [a, c]);
    trace(&mut h, &[all[0], all[2]], Modifiers::NONE);
    assert_eq!(selection(&s), [a, c]);
}

/// The canvas and `selection.set_in_polygon` pick the same objects for
/// the same shape: boxes and free shapes, inside and touching, at the top
/// level and inside a group.
#[test]
fn the_canvas_and_the_agent_command_pick_alike() {
    let dir = tempfile::tempdir().unwrap();
    let (s, [a, b, c]) = three(&dir);
    // A turned, scaled fourth square and a group, so the outlines are not
    // all boxes.
    let d = import_square(&s);
    exec(
        &s,
        "object.move",
        json!({ "ids": [d], "dx_mm": 40.0, "dy_mm": 80.0 }),
    );
    exec(&s, "object.rotate", json!({ "ids": [d], "degrees": 30.0 }));
    let g = exec(&s, "object.group", json!({ "ids": [b, c] }))["group_id"]
        .as_u64()
        .unwrap();
    exec(&s, "object.select", json!({ "ids": [] }));
    let (mut h, _) = rig(&dir, s.clone());
    let shapes: Vec<Vec<(f64, f64)>> = vec![
        vec![(0.0, 0.0), (300.0, 0.0), (300.0, 200.0), (0.0, 200.0)],
        vec![(10.0, 10.0), (80.0, 10.0), (80.0, 80.0), (10.0, 80.0)],
        vec![(30.0, 30.0), (60.0, 30.0), (45.0, 60.0)],
        vec![(50.0, 60.0), (130.0, 100.0), (90.0, 180.0), (60.0, 90.0)],
        vec![
            (100.0, 0.0),
            (300.0, 0.0),
            (300.0, 75.0),
            (215.0, 75.0),
            (215.0, 30.0),
            (100.0, 30.0),
        ],
        vec![(65.0, 65.0), (125.0, 65.0), (125.0, 130.0), (65.0, 130.0)],
    ];
    let doc_id = s.lock().unwrap().doc().id();
    for within in [None, Some(g)] {
        app_mut(&mut h).ui.prepare.entered = within.map(|g| (doc_id, g));
        if within.is_some() {
            exec(&s, "object.select", json!({ "ids": [b] }));
        }
        h.run_plan();
        let sn = snap(&mut h);
        let st = &app(&h).ui.prepare;
        for pts in &shapes {
            let l = Lasso::new(pts.iter().map(|(x, y)| kurbo::Point::new(*x, *y))).unwrap();
            for touch in [false, true] {
                let ui: Vec<u64> = canvas::lasso_pick(&sn, st, &l, touch)
                    .ids
                    .iter()
                    .map(|i| i.0)
                    .collect();
                let mut p = json!({ "points_mm": pts.iter().map(|(x, y)| [x, y]).collect::<Vec<_>>(), "touch": touch });
                if let Some(g) = within {
                    p["group"] = json!(g);
                }
                let out = exec(&s, "selection.set_in_polygon", p);
                let agent: Vec<u64> = out["picked"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap())
                    .collect();
                assert_eq!(ui, agent, "{pts:?} touch {touch} in {within:?}");
            }
        }
    }
    let _ = (a, d);
}

/// The cards of the two tools name the keys of each system.
#[test]
fn the_tool_cards_name_the_keys_of_each_system() {
    use flatcraft::ui::help::{self, Os};
    for os in Os::ALL {
        let (select, lasso) = (help::select_card(os), help::lasso_card(os));
        let alt = if os == Os::MacOs { "Option" } else { "Alt" };
        for c in [&select, &lasso] {
            assert!(!c.contains('\u{2014}'), "{c}");
            assert!(c.contains(alt) && c.contains("Shift"), "{c}");
        }
        assert!(
            select.contains("fully inside") && select.contains("Key: V"),
            "{select}"
        );
        assert!(
            lasso.contains("touches") && lasso.contains("Key: L"),
            "{lasso}"
        );
        assert!(lasso.contains("Esc"), "{lasso}");
        assert_eq!(
            lasso.contains("desktop settings"),
            os == Os::Linux,
            "{lasso}"
        );
    }
}

/// Stickers on a backing sheet: the Lasso tool, started over the sheet,
/// picks the two stickers it goes around and leaves the sheet.
#[test]
fn a_lasso_on_a_backing_sheet_picks_the_stickers_only() {
    let dir = tempfile::tempdir().unwrap();
    let (s, [a, b, _]) = three(&dir);
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="300mm" height="300mm" viewBox="0 0 300 300"><rect x="5" y="5" width="285" height="85"/></svg>"#;
    let sheet = exec(
        &s,
        "object.import_svg",
        json!({ "svg": svg, "name": "sheet" }),
    )["id"]
        .as_u64()
        .unwrap();
    exec(
        &s,
        "object.reorder",
        json!({ "ids": [sheet], "to": "back" }),
    );
    exec(&s, "object.select", json!({ "ids": [] }));
    let (mut h, v) = rig(&dir, s.clone());
    app_mut(&mut h).ui.prepare.select_tool = SelectTool::Lasso;
    let around_ab =
        [(12.0, 12.0), (178.0, 12.0), (178.0, 80.0), (12.0, 80.0)].map(|(x, y)| at(&v, x, y));
    trace(&mut h, &around_ab, Modifiers::NONE);
    assert_eq!(selection(&s), [a, b]);
}
