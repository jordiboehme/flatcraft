//! "Pick what stays" (Task 59): the keep picker on the Prepare canvas sets
//! the keep mode from the part people click as staying.

mod common;

use common::{
    AppHarness, app, app_mut, harness, painted, session, to_prepare, undo_len, wait_for_workers,
    wait_until,
};
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::Workspace;
use flatcraft::ui::context_menu::Item;
use flatcraft::ui::keep_pick::{self, KeepPick};
use flatcraft::ui::kept::Mask;
use flatcraft_core::document::{KeepMode, ObjectId};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

fn run(s: &SharedSession, cmd: &str, params: Value) -> Value {
    s.lock()
        .unwrap()
        .execute(cmd, params, Origin::User)
        .unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn keep(s: &SharedSession, id: u64) -> KeepMode {
    s.lock()
        .unwrap()
        .doc()
        .document
        .get(ObjectId(id))
        .unwrap()
        .keep
}

/// A 40 x 50 mm O with a 20 x 30 mm middle at (`x`, 20) on the page.
fn letter_o(s: &SharedSession, x: f64) -> u64 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><path fill-rule="evenodd" d="M{x} 20 h40 v50 h-40 Z M{} 30 h20 v30 h-20 Z"/></svg>"#,
        x + 10.0
    );
    run(s, "object.import_svg", json!({ "svg": svg, "name": "O" }))["id"]
        .as_u64()
        .unwrap()
}

/// A 30 mm square at (`x`, 20) on the page.
fn square(s: &SharedSession, x: f64) -> u64 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="{x}" y="20" width="30" height="30"/></svg>"#
    );
    run(
        s,
        "object.import_svg",
        json!({ "svg": svg, "name": "square" }),
    )["id"]
        .as_u64()
        .unwrap()
}

/// The bounds of `id` in mat mm.
fn bounds(s: &SharedSession, id: u64) -> kurbo::Rect {
    let state = run(s, "document.state", json!({}));
    let o = state["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .unwrap()
        .clone();
    let b = &o["bounds_mm"];
    let v = |k: &str| b[k].as_f64().unwrap();
    kurbo::Rect::new(v("x0"), v("y0"), v("x1"), v("y1"))
}

/// The points of an O: on its stroke, in its middle, and on the sheet.
struct OPoints {
    stroke: kurbo::Point,
    middle: kurbo::Point,
    sheet: kurbo::Point,
}

fn o_points(b: kurbo::Rect) -> OPoints {
    OPoints {
        stroke: kurbo::Point::new(b.x0 + 5.0, b.center().y),
        middle: b.center(),
        // Below the O: copies of it go to its right.
        sheet: kurbo::Point::new(b.center().x, b.y1 + 15.0),
    }
}

fn screen(h: &AppHarness, p: kurbo::Point) -> Pos2 {
    app(h).ui.prepare.view.mm_to_screen(p)
}

fn pick(h: &AppHarness) -> &KeepPick {
    app(h).ui.prepare.keep_pick.as_ref().expect("picking")
}

/// Runs frames until the picker's masks are built.
fn wait_for_masks(h: &mut AppHarness) {
    wait_until(h, |a| {
        a.ui.prepare
            .keep_pick
            .as_ref()
            .is_some_and(KeepPick::pending)
            || a.ui.cut.kept.pending()
    });
}

fn hover(h: &mut AppHarness, p: kurbo::Point) {
    let at = screen(h, p);
    h.event(Event::PointerMoved(at));
    wait_for_masks(h);
}

/// A left click at `p` (mat mm), one frame for the press and one for the
/// release.
fn click(h: &mut AppHarness, p: kurbo::Point) {
    let at = screen(h, p);
    h.event(Event::PointerMoved(at));
    h.run();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    wait_for_masks(h);
}

/// True when the text was painted last frame.
fn drawn(h: &AppHarness, text: &str) -> bool {
    painted(h).iter().any(|s| match s {
        egui::Shape::Text(t) => t.galley.text() == text,
        _ => false,
    })
}

/// The texture ids painted last frame.
fn textures(h: &AppHarness) -> Vec<egui::TextureId> {
    painted(h)
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Mesh(m) => Some(m.texture_id),
            _ => None,
        })
        .collect()
}

/// Prepare with the objects `ids` selected, picking started from the
/// button next to the Keep combo, masks built.
fn picking(dir: &tempfile::TempDir, s: &SharedSession, ids: &[u64]) -> AppHarness {
    run(s, "object.select", json!({ "ids": ids }));
    let mut h = harness(dir, s.clone(), false);
    to_prepare(&mut h);
    wait_for_workers(&mut h);
    h.get_by_label(keep_pick::BUTTON).click();
    wait_for_masks(&mut h);
    assert!(app(&h).ui.prepare.keep_pick.is_some(), "picking started");
    h
}

#[test]
fn each_mark_gives_its_mode_and_nothing_marked_dims_done() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let o = letter_o(&s, 20.0);
    let p = o_points(bounds(&s, o));
    let mut h = picking(&dir, &s, &[o]);
    // Nothing marked yet: Done is dimmed and the bar says what to do.
    assert!(drawn(&h, keep_pick::NOTHING_MARKED));
    assert!(h.get_by_label("Done").accesskit_node().is_disabled());
    let steps = [
        (p.stroke, Some(KeepMode::Pieces)),
        (p.sheet, Some(KeepMode::Both)),
        (p.stroke, Some(KeepMode::Stencil)),
        (p.sheet, None),
    ];
    for (at, want) in steps {
        click(&mut h, at);
        assert_eq!(pick(&h).result(), want, "after a click at {at:?}");
        assert!(drawn(&h, keep_pick::result_text(want)), "{want:?}");
        assert_eq!(
            !h.get_by_label("Done").accesskit_node().is_disabled(),
            want.is_some()
        );
    }
    // Nothing was set while picking.
    assert_eq!(keep(&s, o), KeepMode::Pieces);
    // The middle of the O is part of the sheet.
    click(&mut h, p.middle);
    assert_eq!(pick(&h).result(), Some(KeepMode::Stencil));
}

#[test]
fn done_sets_the_mode_in_one_undo_step_and_cancel_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let o = letter_o(&s, 20.0);
    let p = o_points(bounds(&s, o));
    let mut h = picking(&dir, &s, &[o]);
    click(&mut h, p.sheet);
    let before = undo_len(&s);
    h.get_by_label("Done").click();
    h.run();
    wait_for_workers(&mut h);
    assert!(app(&h).ui.prepare.keep_pick.is_none(), "Done ends picking");
    assert_eq!(keep(&s, o), KeepMode::Stencil);
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    run(&s, "edit.undo", json!({}));
    assert_eq!(keep(&s, o), KeepMode::Pieces);

    // Cancel: nothing changes.
    h.get_by_label(keep_pick::BUTTON).click();
    wait_for_masks(&mut h);
    click(&mut h, p.sheet);
    let before = undo_len(&s);
    h.get_by_label_contains("Cancel").click();
    h.run();
    assert!(app(&h).ui.prepare.keep_pick.is_none());
    assert_eq!((keep(&s, o), undo_len(&s)), (KeepMode::Pieces, before));

    // Escape cancels too, and keeps the selection.
    h.get_by_label(keep_pick::BUTTON).click();
    wait_for_masks(&mut h);
    click(&mut h, p.stroke);
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(app(&h).ui.prepare.keep_pick.is_none());
    assert_eq!((keep(&s, o), undo_len(&s)), (KeepMode::Pieces, before));
    assert_eq!(
        s.lock()
            .unwrap()
            .doc()
            .selection
            .iter()
            .map(|i| i.0)
            .collect::<Vec<_>>(),
        vec![o]
    );
}

#[test]
fn another_selection_or_workspace_cancels_and_clicks_select_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let o = letter_o(&s, 20.0);
    let other = square(&s, 120.0);
    let p = o_points(bounds(&s, o));
    let mut h = picking(&dir, &s, &[o]);
    // A click on the other object marks the sheet and selects nothing.
    click(&mut h, bounds(&s, other).center());
    assert_eq!(pick(&h).result(), Some(KeepMode::Stencil));
    assert_eq!(
        s.lock()
            .unwrap()
            .doc()
            .selection
            .iter()
            .map(|i| i.0)
            .collect::<Vec<_>>(),
        vec![o]
    );
    // A click on the bar itself marks nothing.
    let bar = h.get_by_label(keep_pick::TITLE).rect();
    let before = (pick(&h).shape, pick(&h).sheet);
    h.event(Event::PointerMoved(bar.center()));
    h.run();
    h.get_by_label(keep_pick::TITLE).click();
    h.run();
    assert_eq!((pick(&h).shape, pick(&h).sheet), before);
    // Another selection (an agent) cancels.
    run(&s, "object.select", json!({ "ids": [other] }));
    h.run();
    assert!(app(&h).ui.prepare.keep_pick.is_none());
    assert_eq!(keep(&s, o), KeepMode::Pieces);
    // Leaving the workspace cancels.
    run(&s, "object.select", json!({ "ids": [o] }));
    h.run();
    h.get_by_label(keep_pick::BUTTON).click();
    wait_for_masks(&mut h);
    click(&mut h, p.stroke);
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run();
    assert!(app(&h).ui.prepare.keep_pick.is_none());
    assert_eq!(keep(&s, o), KeepMode::Pieces);
}

#[test]
fn hovering_tells_the_shape_from_the_sheet_and_a_letter_with_a_hole_explains_its_middle() {
    for (material, backing) in [("vinyl", true), ("copy-paper", false)] {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        run(&s, "job.set_material", json!({ "id": material }));
        let o = letter_o(&s, 20.0);
        let p = o_points(bounds(&s, o));
        let mut h = picking(&dir, &s, &[o]);
        hover(&mut h, p.sheet);
        let hit = pick(&h).hover.expect("the sheet");
        assert_eq!(
            (hit.region, hit.hole),
            (flatcraft_path::Region::Sheet, false)
        );
        assert!(!pick(&h).hole_seen);
        assert!(drawn(&h, &keep_pick::hover_text(Some(hit), false)));
        hover(&mut h, p.middle);
        let hit = pick(&h).hover.expect("the middle");
        assert_eq!(
            (hit.region, hit.hole),
            (flatcraft_path::Region::Sheet, true)
        );
        assert!(pick(&h).hole_seen, "the bar explains the middle");
        assert!(drawn(&h, keep_pick::hole_text(None, backing)), "{material}");
        hover(&mut h, p.stroke);
        let hit = pick(&h).hover.expect("the shape");
        assert_eq!(hit.region, flatcraft_path::Region::Shape);
        // The text follows the mode the marks give.
        for (at, mode) in [(p.stroke, KeepMode::Pieces), (p.stroke, KeepMode::Stencil)] {
            if mode == KeepMode::Stencil {
                click(&mut h, at);
                click(&mut h, p.sheet);
            } else {
                click(&mut h, at);
            }
            assert_eq!(pick(&h).result(), Some(mode));
            assert!(
                drawn(&h, keep_pick::hole_text(Some(mode), backing)),
                "{mode:?}"
            );
        }
        // On paper the stencil hint is the report's.
        if !backing {
            assert_eq!(
                keep_pick::hole_text(Some(KeepMode::Stencil), false),
                flatcraft_engine::report::STENCIL_ISLAND_HINT
            );
        }
    }
    // A square has no hole: nothing to explain.
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let q = square(&s, 20.0);
    let mut h = picking(&dir, &s, &[q]);
    hover(&mut h, bounds(&s, q).center());
    assert!(!pick(&h).hole_seen);
}

#[test]
fn the_overlay_follows_the_marks_from_masks_the_worker_built() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let o = letter_o(&s, 20.0);
    // A linked copy outside the selection changes with it.
    let copy = run(
        &s,
        "object.copies",
        json!({ "id": o, "count": 1, "linked": true }),
    )["ids"][0]
        .as_u64()
        .unwrap();
    let p = o_points(bounds(&s, o));
    let mut h = picking(&dir, &s, &[o]);
    for (clicks, mode) in [
        (vec![p.stroke], KeepMode::Pieces),
        (vec![p.sheet], KeepMode::Both),
        (vec![p.stroke], KeepMode::Stencil),
    ] {
        for at in clicks {
            click(&mut h, at);
        }
        wait_for_masks(&mut h);
        h.run();
        let pk = pick(&h);
        assert_eq!(pk.result(), Some(mode));
        let cache = pk.preview(mode);
        assert_eq!(cache.built_on.as_deref(), Some(flatcraft::ui::kept::WORKER));
        let tex = cache.texture_id().expect("drawn");
        assert!(
            textures(&h).contains(&tex),
            "{mode:?}: its overlay is drawn"
        );
    }
    // Every mask was built once: switching marks reuses them.
    let made: Vec<usize> = pick(&h).previews.iter().map(|c| c.made).collect();
    assert_eq!(made, vec![1, 1, 1]);
    // The preview equals the plan's own overlay after Done.
    let ctx = h.ctx.clone();
    let job = app(&h).ui.cut.job().unwrap();
    let area = job.plan.kept.sheet();
    let v = pick(&h).variant(KeepMode::Stencil);
    let preview = app_mut(&mut h)
        .ui
        .prepare
        .keep_pick
        .as_mut()
        .unwrap()
        .previews[1]
        .mask_of(&ctx, &job, area, &v)
        .unwrap();
    h.get_by_label("Done").click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(
        (keep(&s, o), keep(&s, copy)),
        (KeepMode::Stencil, KeepMode::Stencil)
    );
    let after = app(&h).ui.cut.job().unwrap();
    assert!(!std::sync::Arc::ptr_eq(&job, &after), "a new plan");
    let done = Mask::of(&after.plan.kept, area, flatcraft::ui::kept::MAX_TEXELS).unwrap();
    assert_eq!(*preview, done, "the preview was what Done keeps");
}

#[test]
fn the_window_and_an_agent_pick_the_same_mode_for_the_same_points() {
    let marks = |p: &OPoints| {
        [
            vec![p.stroke],
            vec![p.middle],
            vec![p.sheet],
            vec![p.stroke, p.sheet],
            vec![p.middle, p.stroke],
        ]
    };
    for k in 0..5 {
        // The window.
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        let o = letter_o(&s, 20.0);
        let p = o_points(bounds(&s, o));
        let points = marks(&p)[k].clone();
        let mut h = picking(&dir, &s, &[o]);
        for at in &points {
            click(&mut h, *at);
        }
        h.get_by_label("Done").click();
        h.run();
        wait_for_workers(&mut h);
        let window = keep(&s, o);
        // An agent, on a fresh copy of the document.
        let dir2 = tempfile::tempdir().unwrap();
        let s2 = session(&dir2);
        let o2 = letter_o(&s2, 20.0);
        let pts: Vec<Value> = points
            .iter()
            .map(|q| json!({ "x_mm": q.x, "y_mm": q.y }))
            .collect();
        let r = s2
            .lock()
            .unwrap()
            .execute(
                "object.keep_at",
                json!({ "ids": [o2], "points": pts }),
                Origin::Agent,
            )
            .unwrap();
        assert_eq!(r["keep"], json!(window.key()), "points {points:?}");
        assert_eq!(keep(&s2, o2), window);
    }
}

#[test]
fn a_group_picks_its_members_and_the_context_menu_starts_picking() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = square(&s, 20.0);
    let b = square(&s, 70.0);
    let g = run(&s, "object.group", json!({ "ids": [a, b] }))["group_id"]
        .as_u64()
        .unwrap();
    run(&s, "object.select", json!({ "ids": [g] }));
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    wait_for_workers(&mut h);
    // Right-click on a member: the menu offers the picker.
    let at = screen(&h, bounds(&s, b).center());
    h.event(Event::PointerMoved(at));
    h.run();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    h.run();
    h.run();
    h.get_by_label(Item::PickKeep.label()).click();
    wait_for_masks(&mut h);
    let ids: Vec<u64> = pick(&h).ids.iter().copied().collect();
    assert_eq!(ids, vec![a, b]);
    // Inside b is the shape of the group.
    click(&mut h, bounds(&s, b).center());
    assert_eq!(pick(&h).result(), Some(KeepMode::Pieces));
    click(&mut h, bounds(&s, a).center());
    assert_eq!(
        pick(&h).result(),
        None,
        "a is the same shape: unmarked again"
    );
    click(&mut h, kurbo::Point::new(bounds(&s, b).x1 + 20.0, 30.0));
    h.get_by_label("Done").click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(
        (keep(&s, a), keep(&s, b)),
        (KeepMode::Stencil, KeepMode::Stencil)
    );
}

#[test]
fn the_button_is_dimmed_with_a_reason_and_the_cards_say_what_it_does() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let o = letter_o(&s, 20.0);
    run(&s, "object.select", json!({ "ids": [] }));
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    wait_for_workers(&mut h);
    assert!(
        h.get_by_label(keep_pick::BUTTON)
            .accesskit_node()
            .is_disabled(),
        "nothing selected"
    );
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
    assert_eq!(
        keep_pick::why_not(&snap, &[]),
        Some(keep_pick::NOTHING_SELECTED)
    );
    run(&s, "object.set_cut", json!({ "ids": [o], "cut": false }));
    run(&s, "object.select", json!({ "ids": [o] }));
    h.run();
    assert!(
        h.get_by_label(keep_pick::BUTTON)
            .accesskit_node()
            .is_disabled(),
        "skipped"
    );
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
    assert_eq!(keep_pick::why_not(&snap, &[o]), Some(keep_pick::NOT_CUT));
    run(&s, "object.set_cut", json!({ "ids": [o], "cut": true }));
    h.run();
    assert!(
        !h.get_by_label(keep_pick::BUTTON)
            .accesskit_node()
            .is_disabled()
    );
    // The Settings panel keeps its width with the button in it.
    let canvas = app(&h).ui.prepare.canvas_rect;
    for _ in 0..5 {
        h.step();
    }
    assert_eq!(app(&h).ui.prepare.canvas_rect, canvas);
    // Plain words, no em-dash, and the Keep card points to the picker.
    let mut texts = vec![
        keep_pick::HINT.to_string(),
        keep_pick::NOT_CUT.to_string(),
        keep_pick::NO_PLAN.to_string(),
        keep_pick::CANCEL_HINT.to_string(),
        keep_pick::POINT_AT.to_string(),
    ];
    for m in [
        None,
        Some(KeepMode::Pieces),
        Some(KeepMode::Stencil),
        Some(KeepMode::Both),
    ] {
        texts.push(keep_pick::result_text(m).to_string());
        for backing in [true, false] {
            texts.push(keep_pick::hole_text(m, backing).to_string());
        }
    }
    for t in &texts {
        assert!(!t.contains('\u{2014}'), "{t}");
    }
    assert!(keep_pick::result_text(Some(KeepMode::Pieces)).contains("stickers"));
    let card = flatcraft::ui::tools::Setting::Keep.card();
    assert!(card.contains("pipette") && card.contains("click the part that stays"));
    assert_eq!(Item::PickKeep.description(), keep_pick::HINT);
}

#[test]
fn the_menu_bar_offers_the_picker_with_the_same_rules() {
    use flatcraft::ui::menu_model::{Cmd, object_item};
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let o = letter_o(&s, 20.0);
    run(&s, "object.select", json!({ "ids": [] }));
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    wait_for_workers(&mut h);
    let ctx = h.ctx.clone();
    let offered = |h: &mut AppHarness, ids: &[u64]| {
        let snap = flatcraft::ui::prepare::snapshot(app_mut(h), &ctx);
        object_item(&Cmd::PickKeep, &snap, &app(h).ui.prepare, ids)
    };
    assert_eq!(offered(&mut h, &[]), None, "nothing selected");
    assert_eq!(offered(&mut h, &[o]), Some(Item::PickKeep));
    run(&s, "object.set_cut", json!({ "ids": [o], "cut": false }));
    h.run();
    assert_eq!(offered(&mut h, &[o]), None, "skipped");
    // A click on the menu item starts picking on the selection.
    run(&s, "object.set_cut", json!({ "ids": [o], "cut": true }));
    run(&s, "object.select", json!({ "ids": [o] }));
    h.run();
    let doc = s.lock().unwrap().doc().id();
    let action = Cmd::PickKeep.action(doc, &[]).unwrap();
    app_mut(&mut h).apply(&ctx, vec![action]);
    wait_for_masks(&mut h);
    let ids: Vec<u64> = pick(&h).ids.iter().copied().collect();
    assert_eq!(ids, vec![o]);
}

#[test]
fn the_bar_says_when_another_object_keeps_the_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let o = letter_o(&s, 20.0);
    let p = o_points(bounds(&s, o));
    // Without a stencil on the mat: no note.
    let mut h = picking(&dir, &s, &[o]);
    click(&mut h, p.stroke);
    assert_eq!(pick(&h).result(), Some(KeepMode::Pieces));
    assert!(!drawn(&h, keep_pick::SHEET_KEPT_ELSEWHERE));
    drop(h);
    // A stencil square elsewhere keeps the sheet: the bar says so.
    let frame = square(&s, 140.0);
    run(
        &s,
        "object.set_keep",
        json!({ "ids": [frame], "keep": "stencil" }),
    );
    let mut h = picking(&dir, &s, &[o]);
    click(&mut h, p.stroke);
    assert_eq!(pick(&h).result(), Some(KeepMode::Pieces));
    assert!(drawn(&h, keep_pick::result_text(Some(KeepMode::Pieces))));
    assert!(drawn(&h, keep_pick::SHEET_KEPT_ELSEWHERE));
    // The overlay agrees: the sheet next to the O is kept.
    let job = app(&h).ui.cut.job().unwrap();
    assert!(job.plan.kept.is_kept(p.sheet));
}

#[test]
fn a_linked_copy_of_the_selection_is_the_shape_too() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let o = letter_o(&s, 20.0);
    let copy = run(
        &s,
        "object.copies",
        json!({ "id": o, "count": 1, "linked": true }),
    )["ids"][0]
        .as_u64()
        .unwrap();
    run(
        &s,
        "object.set_keep",
        json!({ "ids": [o], "keep": "stencil" }),
    );
    let c = o_points(bounds(&s, copy));
    let mut h = picking(&dir, &s, &[o]);
    // The copy's stroke is the shape, its middle a hole.
    hover(&mut h, c.middle);
    let hit = pick(&h).hover.expect("the copy's middle");
    assert_eq!(
        (hit.region, hit.hole),
        (flatcraft_path::Region::Sheet, true)
    );
    hover(&mut h, c.stroke);
    let hit = pick(&h).hover.expect("the copy's stroke");
    assert_eq!(hit.region, flatcraft_path::Region::Shape);
    assert_eq!(hit.object, Some(ObjectId(copy)));
    click(&mut h, c.stroke);
    assert_eq!(pick(&h).result(), Some(KeepMode::Pieces));
    h.get_by_label("Done").click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(
        (keep(&s, o), keep(&s, copy)),
        (KeepMode::Pieces, KeepMode::Pieces)
    );
}

#[test]
fn an_undo_while_picking_refreshes_the_linked_copies() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = letter_o(&s, 20.0);
    let b = run(&s, "object.copies", json!({ "id": a, "count": 1 }))["ids"][0]
        .as_u64()
        .unwrap();
    run(&s, "object.link", json!({ "ids": [a, b] }));
    let mut h = picking(&dir, &s, &[a]);
    let both: Vec<ObjectId> = vec![ObjectId(a), ObjectId(b)];
    assert_eq!(pick(&h).reach().iter().copied().collect::<Vec<_>>(), both);
    // Undo "Link copies": the selection stays, the picker stays and only
    // a is reached now.
    run(&s, "edit.undo", json!({}));
    h.step();
    wait_for_masks(&mut h);
    assert!(app(&h).ui.prepare.keep_pick.is_some(), "still picking");
    assert_eq!(
        pick(&h).reach().iter().copied().collect::<Vec<_>>(),
        vec![ObjectId(a)]
    );
    // b is no longer the shape: a click on it is the sheet.
    click(&mut h, o_points(bounds(&s, b)).stroke);
    assert_eq!(pick(&h).result(), Some(KeepMode::Stencil));
}

#[test]
fn weeding_says_why_it_cuts_nothing_while_another_object_keeps_the_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let o = letter_o(&s, 20.0);
    run(
        &s,
        "job.set_weeding",
        json!({ "box": true, "lines": "both" }),
    );
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    wait_for_workers(&mut h);
    let text = flatcraft::ui::settings::SHEET_KEPT_NO_WEEDING;
    assert!(!drawn(&h, text), "Pieces only: weeding cuts");
    assert_eq!(why_now(&mut h), None);
    run(
        &s,
        "object.set_keep",
        json!({ "ids": [o], "keep": "stencil" }),
    );
    h.run();
    assert!(drawn(&h, text), "the Weeding section says why");
    assert_eq!(why_now(&mut h), Some(text));
    // The settings stay usable.
    assert!(!h.get_by_label("Weed box").accesskit_node().is_disabled());
    // And the plan has no weed path.
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run();
    assert_eq!(
        flatcraft::ui::cut::weed_count(&app(&h).ui.cut.job().unwrap()),
        0
    );
    // Weeding off says that first.
    run(
        &s,
        "job.set_weeding",
        json!({ "box": false, "lines": "off" }),
    );
    h.run();
    assert_eq!(why_now(&mut h), Some(flatcraft::ui::cut::WEEDING_OFF));
}

/// Why the weed row is dimmed now, by the plan when the app has a current
/// one for the snapshot, as the window asks it.
fn why_now(h: &mut AppHarness) -> Option<&'static str> {
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot_with(app_mut(h), &ctx, true);
    app_mut(h).ui.cut.update(&snap);
    let kept = flatcraft::ui::cut::sheet_kept_now(&app(h).ui.cut, &snap);
    flatcraft::ui::cut::weed_why(&snap, kept)
}

#[test]
fn an_open_line_stencil_does_not_stop_weeding_as_the_plan_says() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><path d="M20 100 H90" fill="none" stroke="black" stroke-width="0.1"/></svg>"#;
    let score = run(
        &s,
        "object.import_svg",
        json!({ "svg": svg, "name": "score" }),
    )["id"]
        .as_u64()
        .unwrap();
    run(
        &s,
        "object.set_keep",
        json!({ "ids": [score], "keep": "stencil" }),
    );
    square(&s, 20.0);
    run(
        &s,
        "job.set_weeding",
        json!({ "box": true, "lines": "both" }),
    );
    let mut h = harness(&dir, s.clone(), false);
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run();
    // The score lines have no closed cut, so the plan keeps no sheet and
    // weeds around the square.
    let job = app(&h).ui.cut.job().unwrap();
    assert!(!job.plan.kept.sheet_kept_besides(|_| false));
    assert!(flatcraft::ui::cut::weed_count(&job) > 0);
    assert_eq!(why_now(&mut h), None, "the plan's rule, not the guess");
    // Without a current plan the document's guess is used.
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
    assert!(flatcraft::ui::settings::sheet_kept(&snap));
    assert!(flatcraft::ui::cut::sheet_kept_now(&app(&h).ui.cut, &snap));
    assert!(!app(&h).ui.prepare.sheet_kept, "the window used the plan");
    assert!(!drawn(&h, flatcraft::ui::settings::SHEET_KEPT_NO_WEEDING));
}

#[test]
fn notices_leave_the_pick_bar_free() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let o = letter_o(&s, 20.0);
    let mut h = picking(&dir, &s, &[o]);
    for i in 0..12 {
        app_mut(&mut h).toast(flatcraft::ui::Toast::error(format!("error {i}")));
    }
    common::settle(&mut h);
    let stack = h
        .ctx
        .memory(|m| m.area_rect(egui::Id::unique("flatcraft-toasts")))
        .unwrap();
    let bar = app(&h)
        .ui
        .prepare
        .keep_pick
        .as_ref()
        .unwrap()
        .bar_rect()
        .unwrap();
    assert!(!stack.intersects(bar), "{stack:?} over {bar:?}");
    // The bar's buttons still take a click.
    h.get_by_label_contains("Cancel").click();
    common::settle(&mut h);
    assert!(app(&h).ui.prepare.keep_pick.is_none());
}
