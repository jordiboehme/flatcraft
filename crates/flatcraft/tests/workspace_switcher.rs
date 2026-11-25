//! The compact workspace switcher and the flat rows of the Objects panel.

mod common;

use common::{AppHarness, app, app_mut, harness, import_square, painted, session, to_prepare};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::{Workspace, switcher};
use flatcraft_engine::session::Origin;
use serde_json::json;

fn texts(h: &AppHarness) -> Vec<String> {
    painted(h)
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}

fn open_list(h: &mut AppHarness) {
    h.get_by_role_and_label(Role::Button, "Workspace").click();
    h.run();
    h.run();
}

#[test]
fn the_closed_tile_is_narrow_and_as_tall_as_the_ribbon() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    for w in [Workspace::Home, Workspace::Prepare, Workspace::Cut] {
        app_mut(&mut h).ui.workspace = w;
        h.run();
        let r = h.get_by_role_and_label(Role::Button, "Workspace").rect();
        assert!(r.width() < 110.0, "{w:?} tile is {} pt wide", r.width());
        assert!(r.width() > 60.0, "{w:?} tile is {} pt wide", r.width());
        assert!((r.height() - switcher::TILE_HEIGHT).abs() < 0.5);
    }
    // Prepare: the tile is as tall as the tab row plus the ribbon panel.
    app_mut(&mut h).ui.workspace = Workspace::Prepare;
    h.run();
    let layout = app(&h).ui.prepare.ribbon_layout.unwrap();
    let tile = h.get_by_role_and_label(Role::Button, "Workspace").rect();
    assert!((tile.top() - layout.tab_row.top()).abs() < 0.5);
    assert!((tile.bottom() - layout.panel.bottom()).abs() < 0.5);
    // The closed tile says the name and "Workspace".
    let t = texts(&h);
    assert!(t.iter().any(|s| s == "Prepare"), "{t:?}");
    assert!(t.iter().any(|s| s == "Workspace"), "{t:?}");
}

#[test]
fn the_list_has_a_heading_rows_a_badge_and_a_footer() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    to_prepare(&mut h);
    open_list(&mut h);
    h.get_by_label("WORKSPACE");
    h.get_by_label(switcher::FOOTER);
    h.get_by_role_and_label(Role::Button, "Prepare");
    h.get_by_role_and_label(Role::Button, "Cut");
    let t = texts(&h);
    for want in [
        "Current",
        "Import, place and size your designs.",
        "Home \u{b7} Arrange \u{b7} View",
        "Choose plotter and material, check the path and send.",
        "Device & Material \u{b7} Path \u{b7} Simulate \u{b7} Send",
    ] {
        assert!(t.iter().any(|s| s == want), "missing {want:?} in {t:?}");
    }
    assert_eq!(t.iter().filter(|s| *s == "Current").count(), 1);
    assert_eq!(
        switcher::FOOTER,
        "Both workspaces work on the same document. Only the tools change."
    );
}

#[test]
fn the_current_badge_follows_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run();
    open_list(&mut h);
    let cut_row = h.get_by_role_and_label(Role::Button, "Cut").rect();
    let badge = painted(&h)
        .iter()
        .find_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text() == "Current" => Some(t.pos),
            _ => None,
        })
        .expect("a Current badge");
    assert!(cut_row.contains(badge), "{badge:?} not in {cut_row:?}");
}

#[test]
fn arrows_move_enter_picks_and_escape_closes() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    to_prepare(&mut h);
    open_list(&mut h);
    h.key_press(egui::Key::ArrowDown);
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    h.run();
    assert_eq!(app(&h).ui.workspace, Workspace::Cut);
    assert!(h.query_by_label("WORKSPACE").is_none());

    open_list(&mut h);
    h.key_press(egui::Key::ArrowUp);
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    h.run();
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);

    open_list(&mut h);
    h.key_press(egui::Key::ArrowDown);
    h.run();
    h.key_press(egui::Key::Escape);
    h.run();
    h.run();
    assert!(h.query_by_label("WORKSPACE").is_none());
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
}

#[test]
fn texts_come_from_the_stage_names() {
    for s in flatcraft::ui::cut::Stage::ALL {
        assert!(switcher::holds(Workspace::Cut).contains(s.name()));
    }
    for w in [Workspace::Home, Workspace::Prepare, Workspace::Cut] {
        assert!(!switcher::purpose(w).contains('\u{2014}'));
        assert!(!switcher::holds(w).contains('\u{2014}'));
    }
}

#[test]
fn object_rows_show_normal_skipped_and_hidden_and_toggle_independently() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = import_square(&s);
    let c = import_square(&s);
    {
        let mut g = s.lock().unwrap();
        g.execute(
            "object.set_cut",
            json!({ "ids": [b], "cut": false }),
            Origin::User,
        )
        .unwrap();
        g.execute(
            "object.set_visible",
            json!({ "ids": [c], "visible": false }),
            Origin::User,
        )
        .unwrap();
    }
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    // One of each state, each with its own names.
    assert_eq!(
        h.query_all_by_role_and_label(Role::Button, "Skip cutting square")
            .count(),
        2
    );
    assert_eq!(
        h.query_all_by_role_and_label(Role::Button, "Cut again square")
            .count(),
        1
    );
    assert_eq!(
        h.query_all_by_role_and_label(Role::Button, "Hide square")
            .count(),
        2
    );
    assert_eq!(
        h.query_all_by_role_and_label(Role::Button, "Show square")
            .count(),
        1
    );
    // The rows are comfortable to click and flat.
    for row in h.query_all_by_role_and_label(Role::Button, "square") {
        assert!(row.rect().height() >= 24.0, "{:?}", row.rect());
    }
    // Skipped keeps its eye, hidden keeps its scissors: the toggles do not
    // depend on each other.
    h.get_by_role_and_label(Role::Button, "Cut again square")
        .click();
    h.run();
    let g = s.lock().unwrap();
    let doc = &g.doc().document;
    assert!(
        doc.get(flatcraft_core::document::ObjectId(b))
            .unwrap()
            .cut_enabled
    );
    assert!(
        doc.get(flatcraft_core::document::ObjectId(b))
            .unwrap()
            .visible
    );
    assert!(
        doc.get(flatcraft_core::document::ObjectId(c))
            .unwrap()
            .cut_enabled
    );
    assert!(
        !doc.get(flatcraft_core::document::ObjectId(c))
            .unwrap()
            .visible
    );
    let _ = a;
}

#[test]
fn a_row_click_selects_and_a_double_click_renames() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.get_by_role_and_label(Role::Button, "square").click();
    h.run();
    assert!(
        s.lock()
            .unwrap()
            .doc()
            .selection
            .contains(&flatcraft_core::document::ObjectId(id))
    );
    h.ctx
        .options_mut(|o| o.input_options.max_double_click_delay = 1.0);
    let at = h
        .get_by_role_and_label(Role::Button, "square")
        .rect()
        .center();
    h.hover_at(at);
    h.step();
    for _ in 0..2 {
        h.drag_at(at);
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
    }
    for _ in 0..3 {
        h.step();
    }
    assert!(app(&h).ui.prepare.rename.is_some());
}

#[test]
fn in_cut_the_stage_panel_is_flush_with_the_tile() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run();
    h.run();
    let tile = h.get_by_role_and_label(Role::Button, "Workspace").rect();
    let border = app(&h).tokens.border;
    let panel = painted(&h)
        .iter()
        .find_map(|s| match s {
            egui::Shape::Rect(r)
                if r.stroke.color == border
                    && (r.rect.height() - switcher::TILE_HEIGHT).abs() < 1.0
                    && r.rect.width() > 300.0 =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .expect("the stage panel");
    assert!((panel.top() - tile.top()).abs() < 0.5, "{panel:?} {tile:?}");
    assert!((panel.bottom() - tile.bottom()).abs() < 0.5);
}
