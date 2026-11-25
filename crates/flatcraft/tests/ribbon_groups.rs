//! The ribbon's groups (Task 37): group buttons with their lists, pins for
//! every group, hover cards, dimmed actions with their reason, the Align
//! option, and the Auto arrange panel.

mod common;

use common::*;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::groups::{FOOTER, PIN_TIP};
use flatcraft::ui::icons::{self, Icon};
use flatcraft::ui::tools::{self, Act};
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::json;

fn select(s: &SharedSession, ids: &[u64]) {
    s.lock()
        .unwrap()
        .execute("object.select", json!({ "ids": ids }), Origin::User)
        .unwrap();
}

fn pin(s: &SharedSession, pins: &[&str]) {
    let pins: Vec<String> = pins.iter().map(|p| p.to_string()).collect();
    s.lock()
        .unwrap()
        .update_prefs(|p| p.ribbon_pins = pins)
        .unwrap();
}

fn stored(h: &AppHarness) -> Vec<String> {
    Preferences::load(&app(h).config_dir).ribbon_pins
}

fn document_bytes(s: &SharedSession) -> String {
    let g = s.lock().unwrap();
    serde_json::to_string(&flatcraft_core::project::to_value(&g.doc().document).unwrap()).unwrap()
}

/// A harness in Prepare whose cards show without a delay.
fn rig(dir: &tempfile::TempDir, s: SharedSession) -> AppHarness {
    let mut h = harness(dir, s, false);
    to_prepare(&mut h);
    h.ctx
        .global_style_mut(|st| st.interaction.tooltip_delay = 0.0);
    h
}

fn show_tab(h: &mut AppHarness, tab: &str) {
    h.get_by_role_and_label(Role::Tab, tab).click();
    h.run();
}

fn hover(h: &mut AppHarness, label: &str) {
    let at = h.get_by_label(label).rect().center();
    h.hover_at(egui::pos2(at.x, at.y));
    h.run();
    h.run();
}

fn disabled(h: &AppHarness, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

/// The textures painted last frame (icons are image meshes or textured
/// rects).
fn textures(h: &AppHarness) -> Vec<egui::TextureId> {
    painted(h)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::Mesh(m) => Some(m.texture_id),
            egui::Shape::Rect(r) => r.brush.as_ref().map(|b| b.fill_texture_id),
            _ => None,
        })
        .collect()
}

fn hover_button(h: &mut AppHarness, label: &str) {
    let at = h.get_by_role_and_label(Role::Button, label).rect().center();
    h.hover_at(egui::pos2(at.x, at.y));
    h.run();
    h.run();
}

#[test]
fn the_chevron_group_button_opens_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = rig(&dir, s);
    assert!(h.query_by_label(FOOTER).is_none());
    h.get_by_role_and_label(Role::Button, "Edit").click();
    h.run();
    h.get_by_label(FOOTER);
    for t in tools::GroupId::Edit.tools() {
        h.get_by_label(&t.row_name());
    }
    // The shortcut hint comes from the shortcut table.
    let dup = h.get_by_label("Duplicate (Edit)");
    assert!(dup.rect().width() > 100.0);
    h.get_by_role_and_label(Role::Button, "Edit").click();
    h.run();
    assert!(
        h.query_by_label(FOOTER).is_none(),
        "a second click closes it"
    );
}

#[test]
fn pinning_two_actions_in_a_row_keeps_the_list_open() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = rig(&dir, s);
    show_tab(&mut h, "Arrange");
    h.get_by_role_and_label(Role::Button, "Arrange").click();
    h.run();
    for (row, toggle, button) in [
        (
            "Circular pattern... (Arrange)",
            "Pin Circular pattern",
            "Circular pattern",
        ),
        (
            "Distribute vertically (Arrange)",
            "Pin Distribute vertically",
            "Distribute vertically",
        ),
    ] {
        hover(&mut h, row);
        // The toggle says what it does.
        hover(&mut h, toggle);
        h.get_by_label(PIN_TIP);
        h.get_by_label(toggle).click();
        h.run();
        h.get_by_label(FOOTER);
        // The ribbon shows it at once.
        h.get_by_role_and_label(Role::Button, button);
    }
    let pins = stored(&h);
    assert!(pins.contains(&"pattern.circular".to_string()), "{pins:?}");
    assert!(
        pins.contains(&"distribute.vertical".to_string()),
        "{pins:?}"
    );
    // A pinned row keeps its (un)pin toggle visible without hovering.
    h.hover_at(egui::pos2(1.0, 1.0));
    h.run();
    h.get_by_label("Unpin Circular pattern");
}

#[test]
fn cards_show_the_description_of_actions_and_groups() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = rig(&dir, s);
    show_tab(&mut h, "Arrange");
    let rotate = tools::by_act(Act::RotateLeft);
    hover(&mut h, "Rotate -90");
    h.get_by_label(rotate.description);
    let group = tools::GroupId::Arrange.def();
    hover_button(&mut h, "Arrange");
    h.get_by_label(group.description);
    // A list row's card too.
    h.get_by_role_and_label(Role::Button, "Arrange").click();
    h.run();
    let mirror = tools::by_act(Act::MirrorVertical);
    hover(&mut h, &mirror.row_name());
    h.get_by_label(mirror.description);
}

#[test]
fn unpinning_the_last_pin_leaves_the_group_as_a_tile_that_opens_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = rig(&dir, s);
    let reload = tools::by_act(Act::Reload);
    h.get_by_role_and_label(Role::Button, "Reload");
    h.get_by_role_and_label(Role::Button, "Linked files")
        .click();
    h.run();
    hover(&mut h, &reload.row_name());
    h.get_by_label("Unpin Reload").click();
    h.run();
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(
        h.query_by_role_and_label(Role::Button, "Reload").is_none(),
        "unpinned"
    );
    // The group never disappears: its tile opens the list.
    h.get_by_role_and_label(Role::Button, "Linked files")
        .click();
    h.run();
    h.get_by_label(&reload.row_name());
    h.get_by_label(FOOTER);
}

#[test]
fn a_dimmed_row_says_why_and_does_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = rig(&dir, s.clone());
    show_tab(&mut h, "Arrange");
    h.get_by_role_and_label(Role::Button, "Arrange").click();
    h.run();
    let row = "Distribute horizontally (Arrange)";
    assert!(disabled(&h, row), "visible but dimmed");
    hover(&mut h, row);
    // The reason first, as a sentence, then what it does.
    h.get_by_label("Select 3 or more objects first.");
    h.get_by_label(tools::by_act(Act::DistributeHorizontal).description);
    let before = document_bytes(&s);
    h.get_by_label(row).click();
    h.run();
    assert_eq!(document_bytes(&s), before, "nothing ran");
    // The pin toggle of a dimmed row still works.
    h.get_by_label("Pin Distribute horizontally").click();
    h.run();
    assert!(stored(&h).contains(&"distribute.horizontal".to_string()));
}

#[test]
fn a_pinned_dimmed_action_shows_its_reason_and_does_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    pin(&s, &["distribute.vertical"]);
    let mut h = rig(&dir, s.clone());
    show_tab(&mut h, "Arrange");
    let name = "Distribute vertically";
    assert!(disabled(&h, name));
    hover(&mut h, name);
    h.get_by_label("Select 3 or more objects first.");
    let before = document_bytes(&s);
    h.get_by_role_and_label(Role::Button, name).click();
    h.run();
    assert_eq!(document_bytes(&s), before);
}

#[test]
fn the_align_option_switches_the_rows_and_their_icons() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = import_square(&s);
    select(&s, &[a, b]);
    let mut h = rig(&dir, s);
    show_tab(&mut h, "Arrange");
    h.get_by_role_and_label(Role::Button, "Arrange").click();
    h.run();
    let tex =
        |h: &AppHarness, icon: Icon| icons::texture(&h.ctx, icon, flatcraft::ui::ICON_SIZE).id();
    let plain = tex(&h, Icon::AlignLeft);
    let framed = tex(&h, Icon::AlignLeftArea);
    h.run();
    assert!(textures(&h).contains(&plain));
    assert!(!textures(&h).contains(&framed));
    hover(&mut h, "Align left (Arrange)");
    h.get_by_label("Pin Align left · selection");
    h.get_by_label("To the cutting area").click();
    h.run();
    // At once: the rows are the cutting-area actions, with the framed icon.
    assert!(textures(&h).contains(&framed));
    hover(&mut h, "Align left (Arrange)");
    // Its card names both parts.
    h.get_by_label("Align left · cutting area");
    h.get_by_label(
        tools::by_act(Act::Align(tools::AlignMode::Left, tools::AlignTo::Area)).description,
    );
    h.get_by_label("Pin Align left · cutting area").click();
    h.run();
    assert!(stored(&h).contains(&"align.left.area".to_string()));
    // The pin is named with both parts on the ribbon.
    h.get_by_role_and_label(Role::Button, "Align left · cutting area");
}

#[test]
fn auto_arrange_spacing_round_trips_and_is_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let _b = import_square(&s);
    select(&s, &[a]);
    let mut h = rig(&dir, s.clone());
    show_tab(&mut h, "Arrange");
    h.get_by_role_and_label(Role::Button, "Arrange").click();
    h.run();
    h.get_by_label("Auto arrange... (Arrange)").click();
    h.run();
    let p = app(&h).ui.prepare.panel.clone().expect("the panel opens");
    assert_eq!(p.kind, flatcraft::ui::arrange::PanelKind::AutoArrange);
    assert_eq!(p.values.spacing_mm, 5.0, "the stored spacing");
    app_mut(&mut h)
        .ui
        .prepare
        .panel
        .as_mut()
        .unwrap()
        .values
        .spacing_mm = 2.5;
    h.run();
    // The preview is where the objects land.
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
    let panel = app(&h).ui.prepare.panel.clone().unwrap();
    let ghosts = flatcraft::ui::arrange::ghosts(&snap, &panel);
    assert_eq!(ghosts.len(), 2);
    let before = document_bytes(&s);
    let undo = undo_len(&s);
    h.get_by_role_and_label(Role::Button, "OK").click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), undo + 1, "one step");
    assert_eq!(
        Preferences::load(&app(&h).config_dir).auto_arrange_spacing_mm,
        2.5
    );
    let state = s
        .lock()
        .unwrap()
        .execute("document.state", json!({}), Origin::User)
        .unwrap();
    for (id, aff) in ghosts {
        let o = state["objects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == id)
            .unwrap();
        let c = &o["transform"]["center_mm"];
        let ghost = aff * kurbo::Point::ZERO;
        assert!(
            (c[0].as_f64().unwrap() - ghost.x).abs() < 1e-9,
            "{c} vs {ghost:?}"
        );
        assert!((c[1].as_f64().unwrap() - ghost.y).abs() < 1e-9);
    }
    // The pinned one-click Auto arrange runs with the stored spacing, and
    // its card says which.
    let arranged = document_bytes(&s);
    s.lock()
        .unwrap()
        .execute("edit.undo", json!({}), Origin::User)
        .unwrap();
    assert_eq!(document_bytes(&s), before);
    pin(&s, &["auto_arrange"]);
    h.run();
    hover(&mut h, "Auto arrange");
    h.get_by_label("Auto arrange · 2.5 mm");
    let undo = undo_len(&s);
    h.get_by_role_and_label(Role::Button, "Auto arrange")
        .click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), undo + 1);
    assert_eq!(document_bytes(&s), arranged, "the same as the panel's OK");
}

#[test]
fn escape_closes_a_group_list_and_keeps_the_selection() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = rig(&dir, s.clone());
    show_tab(&mut h, "Arrange");
    h.get_by_role_and_label(Role::Button, "Arrange").click();
    h.run();
    h.get_by_label(FOOTER);
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(h.query_by_label(FOOTER).is_none(), "the list closed");
    let selection: Vec<u64> = s
        .lock()
        .unwrap()
        .doc()
        .selection
        .iter()
        .map(|i| i.0)
        .collect();
    assert_eq!(selection, vec![a], "the selection stays");
}

#[test]
fn without_a_cutting_area_the_actions_that_need_one_are_dimmed() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    // The plotter of the document is gone: no mat, no cutting area.
    s.lock().unwrap().doc_mut().document.job.plotter_id = "gone".into();
    pin(
        &s,
        &[
            "auto_arrange",
            "auto_arrange.panel",
            "align.left.area",
            "test_patterns",
        ],
    );
    let mut h = rig(&dir, s.clone());
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
    assert!(snap.area.is_none());
    for act in [
        Act::AutoArrange,
        Act::AutoArrangePanel,
        Act::TestPatterns,
        Act::Align(tools::AlignMode::Left, tools::AlignTo::Area),
    ] {
        assert_eq!(
            tools::by_act(act).blocked(&snap),
            Some(tools::NO_AREA),
            "{act:?}"
        );
    }
    // To the selection still needs no area, only 2 objects.
    assert_ne!(
        tools::by_act(Act::Align(
            tools::AlignMode::Left,
            tools::AlignTo::Selection
        ))
        .blocked(&snap),
        Some(tools::NO_AREA)
    );
    // On the ribbon: dimmed, the reason on hover, a click does nothing.
    assert!(disabled(&h, "Test patterns"));
    show_tab(&mut h, "Arrange");
    for name in [
        "Auto arrange",
        "Auto arrange...",
        "Align left · cutting area",
    ] {
        assert!(disabled(&h, name), "{name}");
    }
    hover_button(&mut h, "Auto arrange");
    h.get_by_label(tools::NO_AREA);
    let before = document_bytes(&s);
    h.get_by_role_and_label(Role::Button, "Auto arrange")
        .click();
    h.run();
    assert_eq!(document_bytes(&s), before);
    // In the list too.
    h.get_by_role_and_label(Role::Button, "Arrange").click();
    h.run();
    assert!(disabled(&h, "Auto arrange (Arrange)"));
}

/// Whether a selected-looking fill is painted over `r`.
fn looks_on(h: &AppHarness, r: egui::Rect) -> bool {
    let on = h.ctx.global_style().visuals.selection.bg_fill;
    painted(h).into_iter().any(|s| match s {
        egui::Shape::Rect(x) => x.fill == on && x.rect.intersects(r.shrink(2.0)),
        _ => false,
    })
}

#[test]
fn a_dimmed_switch_that_is_on_does_not_look_active() {
    // Icon-only (900 pt): Keep aspect is on and dimmed with 3 objects,
    // Each object on and dimmed with 1 object.
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let ids: Vec<u64> = (0..3).map(|_| import_square(&s)).collect();
    select(&s, &ids);
    let mut h = harness_sized(&dir, s.clone(), [900.0, 600.0], 8, |app| app);
    to_prepare(&mut h);
    show_tab(&mut h, "Arrange");
    let keep = h.get_by_role_and_label(Role::CheckBox, "Keep aspect");
    assert!(keep.accesskit_node().is_disabled());
    assert_eq!(
        keep.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True),
        "it says it is on"
    );
    let r = keep.rect();
    assert!(!looks_on(&h, r), "but does not look active");
    select(&s, &[ids[0]]);
    app_mut(&mut h).ui.prepare.fields.each = true;
    h.run();
    let each = h.get_by_role_and_label(Role::CheckBox, "Each object");
    assert!(each.accesskit_node().is_disabled());
    let r = each.rect();
    assert!(!looks_on(&h, r));
    // Enabled and on, it does look on.
    select(&s, &ids);
    h.run();
    let r = h
        .get_by_role_and_label(Role::CheckBox, "Each object")
        .rect();
    assert!(looks_on(&h, r), "an active switch keeps its look");
}

#[test]
fn the_pin_toggle_is_reachable_by_keyboard() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = rig(&dir, s);
    show_tab(&mut h, "Arrange");
    h.get_by_role_and_label(Role::Button, "Arrange").click();
    h.run();
    // Nothing hovered: an unpinned row shows no toggle.
    h.hover_at(egui::pos2(1.0, 1.0));
    h.run();
    assert!(h.query_by_label("Pin Circular pattern").is_none());
    h.get_by_label("Circular pattern... (Arrange)").focus();
    h.run();
    h.run();
    h.key_press(egui::Key::Tab);
    h.run();
    let toggle = h.get_by_label("Pin Circular pattern");
    assert!(
        toggle.accesskit_node().is_focused(),
        "Tab moved to the toggle"
    );
    h.key_press(egui::Key::Enter);
    h.run();
    assert!(stored(&h).contains(&"pattern.circular".to_string()));
}
