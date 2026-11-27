//! Tool cards: a tool with settings opens as a card on top of the
//! inspector, its fields built from the command's parameters (see
//! `ui::inspector::tool_card`). OK is one undo step, Cancel changes
//! nothing, another selection cancels the card, and at 900 pt the card and
//! the inspector share the right column.

mod common;
use common::RunPlan;

use common::*;
use egui::accesskit::Role;
use egui::{Event, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::arrange::{self, KINDS, PanelKind};
use flatcraft::ui::inspector::tool_card::{self, FieldKind, Source};
use flatcraft::ui::{prepare, tools};
use flatcraft_engine::commands::meta;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

const EM_DASH: char = '\u{2014}';

fn run(s: &SharedSession, id: &str, params: Value) -> Value {
    s.lock().unwrap().execute(id, params, Origin::User).unwrap()
}

fn select(s: &SharedSession, ids: &[u64]) {
    run(s, "object.select", json!({ "ids": ids }));
}

fn selection_ids(s: &SharedSession) -> Vec<u64> {
    document(s).1
}

fn snap(h: &mut AppHarness) -> prepare::Snapshot {
    let ctx = h.ctx.clone();
    prepare::snapshot(app_mut(h), &ctx)
}

/// Imports the square and moves it by `dx`, `dy` mm.
fn square_at(s: &SharedSession, dx: f64, dy: f64) -> u64 {
    let id = import_square(s);
    run(
        s,
        "object.move",
        json!({ "ids": [id], "dx_mm": dx, "dy_mm": dy }),
    );
    id
}

/// Three squares, the first two overlapping a little, the third apart.
fn three(s: &SharedSession) -> [u64; 3] {
    [
        square_at(s, 0.0, 0.0),
        square_at(s, 30.0, 10.0),
        square_at(s, 160.0, 40.0),
    ]
}

/// The document as the project file stores it, plus the selection.
fn document(s: &SharedSession) -> (String, Vec<u64>) {
    let g = s.lock().unwrap();
    let doc = flatcraft_core::project::to_value(&g.doc().document).unwrap();
    let sel = g.doc().selection.iter().map(|i| i.0).collect();
    (serde_json::to_string(&doc).unwrap(), sel)
}

fn centres(s: &SharedSession) -> Vec<(u64, f64, f64)> {
    let g = s.lock().unwrap();
    g.doc()
        .document
        .objects
        .iter()
        .map(|o| (o.id.0, o.transform.center_mm.x, o.transform.center_mm.y))
        .collect()
}

fn open(h: &mut AppHarness, kind: PanelKind) {
    let sn = snap(h);
    arrange::open(&mut app_mut(h).ui.prepare, &sn, kind);
    h.run_plan();
}

fn cancel_name() -> String {
    let hint = flatcraft::ui::help::hint(flatcraft::ui::help::PrepareKey::ClearSelection);
    format!("Cancel {hint}")
}

/// Types `text` into the number field `label` and leaves it with Enter.
fn type_into(h: &mut AppHarness, label: &str, text: &str) {
    h.get_by_label(label).click();
    h.run_plan();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_plan();
    h.event(Event::Text(text.into()));
    h.run_plan();
    h.key_press(egui::Key::Enter);
    h.run_plan();
}

fn window(dir: &tempfile::TempDir, s: SharedSession, w: f32) -> AppHarness {
    let mut h = harness_sized(dir, s, [w, 900.0], 8, |a| a);
    to_prepare(&mut h);
    wait_for_workers(&mut h);
    h
}

#[test]
fn every_tool_param_has_label_unit_range_and_card() {
    let prefs = toml::to_string(&Preferences::default()).unwrap();
    for kind in KINDS {
        for text in [kind.description(), kind.title_card(), kind.title()] {
            assert!(!text.is_empty() && !text.contains(EM_DASH), "{kind:?}");
        }
        assert!(!kind.fields().is_empty(), "{kind:?}");
        for f in kind.fields() {
            let s = tool_card::spec(f).unwrap_or_else(|| panic!("{kind:?}: unknown field"));
            let what = format!("{kind:?} {} {}", s.owner, s.key);
            assert!(!s.label.is_empty() && !s.label.contains(EM_DASH), "{what}");
            let card = s.card.unwrap_or_else(|| panic!("{what}: no card text"));
            assert!(card.len() > 20, "{what}: a card in plain words");
            assert!(!tool_card::card_text(&s).contains(EM_DASH), "{what}");
            match &s.kind {
                FieldKind::Number { min, max } | FieldKind::Integer { min, max } => {
                    assert!(min.is_some() || max.is_some(), "{what}: no range");
                    assert_ne!(s.unit, meta::Unit::None, "{what}: no unit");
                }
                FieldKind::Choice(labels) => {
                    assert_eq!(labels.len(), 2, "{what}");
                    assert!(labels.iter().all(|l| !l.is_empty() && !l.contains(EM_DASH)));
                }
                FieldKind::Switch => {}
            }
            if let Source::Param { command, path } = f.source {
                assert!(meta::param_info(command, path).is_some(), "{what}");
            }
            if let Some(name) = f.special {
                assert!(tool_card::special(name).is_some(), "{what}: no {name}");
            }
            if let Some(key) = f.pref {
                assert!(
                    prefs.lines().any(|l| l.starts_with(&format!("{key} ="))),
                    "{what}: {key} is no Preferences key"
                );
            }
        }
    }
    // One card text per engine parameter and per window-only field, and
    // no text without its field.
    for m in meta::PARAMS {
        assert!(tools::field_card(m.command, m.path).is_some(), "{m:?}");
    }
    let window: Vec<(&str, &str)> = KINDS
        .iter()
        .flat_map(|k| k.fields())
        .filter_map(|f| match f.source {
            Source::Window(w) => Some((w.owner, w.id)),
            Source::Param { .. } => None,
        })
        .collect();
    for (owner, key, text) in tools::FIELD_CARDS {
        assert!(!text.contains(EM_DASH), "{owner} {key}");
        assert!(
            meta::meta(owner, key).is_some() || window.contains(&(*owner, *key)),
            "{owner} {key}: a card without its field"
        );
    }
    for t in [
        tool_card::CLOSE_CARD,
        tool_card::OK_CARD,
        tool_card::CANCEL_CARD,
    ] {
        assert!(!t.contains(EM_DASH));
    }
}

#[test]
fn every_card_field_shows_with_its_card_and_dimmed_reason() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let [a, b, _] = three(&s);
    let mut h = window(&dir, s.clone(), 1400.0);
    for kind in KINDS {
        let ids = if kind == PanelKind::RectangularPattern {
            vec![a]
        } else {
            vec![a, b]
        };
        select(&s, &ids);
        h.run_plan();
        open(&mut h, kind);
        for f in kind.fields() {
            let spec = tool_card::spec(f).unwrap();
            let node = h.get_by_label(spec.label);
            node.hover();
            h.run_plan();
            let card = spec.card.unwrap();
            assert!(
                h.query_by_label_contains(&card[..20]).is_some(),
                "{kind:?} {}: its card shows",
                spec.label
            );
        }
        h.get_by_role_and_label(Role::Button, &cancel_name())
            .click();
        h.run_plan();
        assert!(app(&h).ui.prepare.panel.is_none());
    }
    // A field that does not apply is dimmed and says why.
    select(&s, &[a, b]);
    h.run_plan();
    open(&mut h, PanelKind::MoveCopy);
    assert!(
        h.get_by_label("Number of copies")
            .accesskit_node()
            .is_disabled(),
        "copies wait for Copy"
    );
    h.get_by_label("Number of copies").hover();
    h.run_plan();
    assert!(h.query_by_label_contains("Turn on Copy first").is_some());
    h.get_by_label("Copy").click();
    h.run_plan();
    assert!(
        !h.get_by_label("Number of copies")
            .accesskit_node()
            .is_disabled()
    );
}

/// Auto arrange from its card moves the objects exactly as `arrange.auto`
/// with the same objects and spacing, as the old panel did.
#[test]
fn auto_arrange_card_matches_old_panel_result() {
    // The same three squares in two sessions.
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let [a, b, _] = three(&s);
    select(&s, &[a, b]);
    let other_dir = tempfile::tempdir().unwrap();
    let direct = session(&other_dir);
    let [da, db, _] = three(&direct);
    assert_eq!(centres(&s), centres(&direct));
    run(
        &direct,
        "arrange.auto",
        json!({ "ids": [da, db], "spacing_mm": 7.5 }),
    );
    let mut h = window(&dir, s.clone(), 1400.0);
    open(&mut h, PanelKind::AutoArrange);
    type_into(&mut h, "Spacing", "7.5");
    assert_eq!(
        app(&h).ui.prepare.panel.as_ref().unwrap().values.spacing_mm,
        7.5
    );
    // The preview shows where they land: the moves of the direct run.
    let sn = snap(&mut h);
    let p = app(&h).ui.prepare.panel.clone().unwrap();
    let ghosts = arrange::ghosts(&sn, &p);
    assert_eq!(ghosts.len(), 2);
    h.get_by_role_and_label(Role::Button, "OK").click();
    h.run_plan();
    wait_for_workers(&mut h);
    assert!(app(&h).ui.prepare.panel.is_none());
    assert_eq!(centres(&s), centres(&direct));
    for (id, g) in ghosts {
        let at = g * kurbo::Point::ZERO;
        let c = centres(&s).into_iter().find(|c| c.0 == id).unwrap();
        assert!((at.x - c.1).abs() < 1e-9 && (at.y - c.2).abs() < 1e-9);
    }
    // OK stored the spacing for the next Auto arrange.
    assert_eq!(
        Preferences::load(&app(&h).config_dir).auto_arrange_spacing_mm,
        7.5
    );
}

#[test]
fn card_cancel_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let [a, b, _] = three(&s);
    let mut h = window(&dir, s.clone(), 1400.0);
    for kind in KINDS {
        let ids = if kind == PanelKind::RectangularPattern {
            vec![a]
        } else {
            vec![a, b]
        };
        select(&s, &ids);
        h.run_plan();
        let (before, undo) = (document(&s), undo_len(&s));
        let stored = Preferences::load(&app(&h).config_dir);
        for how in [cancel_name(), tool_card::CLOSE.to_string()] {
            open(&mut h, kind);
            // Change the first number field, as a person would.
            let first = kind
                .fields()
                .iter()
                .filter_map(tool_card::spec)
                .find(|s| matches!(s.kind, FieldKind::Number { .. } | FieldKind::Integer { .. }))
                .unwrap();
            type_into(&mut h, first.label, "3");
            assert!(app(&h).ui.prepare.panel.is_some(), "{kind:?}");
            h.get_by_label(&how).click();
            h.run_plan();
            wait_for_workers(&mut h);
            assert!(app(&h).ui.prepare.panel.is_none(), "{kind:?} {how}");
            assert_eq!(document(&s), before, "{kind:?} {how}");
            assert_eq!(undo_len(&s), undo, "{kind:?} {how}");
            let now = Preferences::load(&app(&h).config_dir);
            assert_eq!(
                (now.rotate_each, now.auto_arrange_spacing_mm),
                (stored.rotate_each, stored.auto_arrange_spacing_mm),
                "{kind:?} {how}"
            );
        }
    }
}

#[test]
fn card_ok_is_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let [a, b, _] = three(&s);
    let mut h = window(&dir, s.clone(), 1400.0);
    for kind in KINDS {
        let ids = if kind == PanelKind::RectangularPattern {
            vec![a]
        } else {
            vec![a, b]
        };
        select(&s, &ids);
        h.run_plan();
        let (before, undo) = (document(&s).0, undo_len(&s));
        open(&mut h, kind);
        h.get_by_role_and_label(Role::Button, "OK").click();
        h.run_plan();
        wait_for_workers(&mut h);
        assert!(app(&h).ui.prepare.panel.is_none(), "{kind:?}: OK closes it");
        assert_eq!(undo_len(&s), undo + 1, "{kind:?}: one step");
        assert_ne!(document(&s).0, before, "{kind:?} changed something");
        run(&s, "edit.undo", json!({}));
        h.run_plan();
        assert_eq!(document(&s).0, before, "{kind:?}: one undo takes it back");
    }
}

#[test]
fn selection_change_with_card_open_never_applies_to_new_selection() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let [a, b, c] = three(&s);
    let mut h = window(&dir, s.clone(), 1400.0);
    for kind in KINDS {
        select(&s, &[a]);
        h.run_plan();
        open(&mut h, kind);
        assert!(app(&h).ui.prepare.panel.is_some(), "{kind:?}");
        let before = document(&s).0;
        // An agent, a click on the canvas or the Objects list picks other
        // objects: the card goes, nothing changes.
        select(&s, &[b, c]);
        h.run_plan();
        assert!(app(&h).ui.prepare.panel.is_none(), "{kind:?}");
        assert!(h.query_by_role_and_label(Role::Button, "OK").is_none());
        h.key_press(egui::Key::Enter);
        h.run_plan();
        wait_for_workers(&mut h);
        assert_eq!(document(&s).0, before, "{kind:?}");
    }
    // A click on another object on the canvas does the same.
    select(&s, &[a]);
    h.run_plan();
    open(&mut h, PanelKind::Rotate);
    let before = document(&s).0;
    let at = centres(&s).into_iter().find(|o| o.0 == c).unwrap();
    let view = app(&h).ui.prepare.view;
    let p = view.mm_to_screen(kurbo::Point::new(at.1, at.2));
    click_at(&mut h, p);
    assert_eq!(selection_ids(&s), vec![c]);
    assert!(app(&h).ui.prepare.panel.is_none());
    assert_eq!(document(&s).0, before);
}

#[test]
fn card_and_inspector_share_column_at_900() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let [a, b, _] = three(&s);
    select(&s, &[a, b]);
    let mut h = window(&dir, s.clone(), 900.0);
    // At 900 pt the right panel is a rail until a card opens.
    assert!(
        h.query_by_label_contains("Open the Cut and Job panel")
            .is_some()
    );
    for kind in KINDS {
        let ids = if kind == PanelKind::RectangularPattern {
            vec![a]
        } else {
            vec![a, b]
        };
        select(&s, &ids);
        h.run_plan();
        open(&mut h, kind);
        assert!(
            h.query_by_label("Collapse the Cut and Job panel").is_some(),
            "{kind:?}: the panel opens for the card"
        );
        let card = app(&h).ui.prepare.panel_rect.expect("the card is drawn");
        let canvas = app(&h).ui.prepare.canvas_rect;
        // One column: the card sits right of the canvas, the inspector
        // under it in the same column.
        assert!(
            card.left() >= canvas.right(),
            "{kind:?}: {card:?} {canvas:?}"
        );
        let heading = h.get_by_label("Cut mode").rect();
        assert!(
            heading.top() >= card.bottom(),
            "{kind:?}: the inspector below"
        );
        assert!(
            heading.left() >= card.left() - 1.0 && heading.left() < card.right(),
            "{kind:?}: one column"
        );
        let job = h.get_by_label_contains("Job: ").rect();
        assert!(job.bottom() <= card.top(), "{kind:?}: the job line on top");
        // The buttons are in sight.
        let ok = h.get_by_role_and_label(Role::Button, "OK").rect();
        assert!(ok.bottom() <= 900.0, "{kind:?}");
        assert!(canvas.width() > 450.0, "{kind:?}: the canvas keeps room");
        h.get_by_role_and_label(Role::Button, &cancel_name())
            .click();
        h.run_plan();
    }
    // The card gone, the panel folds back to its rail: the choice was
    // never stored.
    assert!(
        h.query_by_label_contains("Open the Cut and Job panel")
            .is_some()
    );
    let stored = Preferences::load(&app(&h).config_dir);
    assert_eq!(stored.panel_settings, Preferences::default().panel_settings);
}

#[test]
fn folding_the_panel_cancels_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let [a, b, _] = three(&s);
    select(&s, &[a, b]);
    let mut h = window(&dir, s.clone(), 1400.0);
    let before = document(&s);
    open(&mut h, PanelKind::Rotate);
    h.get_by_label("Collapse the Cut and Job panel").click();
    h.run_plan();
    assert!(app(&h).ui.prepare.panel.is_none());
    assert!(
        h.query_by_label_contains("Open the Cut and Job panel")
            .is_some()
    );
    assert_eq!(document(&s), before);
}

fn pin(s: &SharedSession, pins: &[&str]) {
    let pins: Vec<String> = pins.iter().map(|p| p.to_string()).collect();
    s.lock()
        .unwrap()
        .update_prefs(|p| p.ribbon_pins = pins)
        .unwrap();
}

/// Auto arrange runs now and has a card: one split button. The main part
/// runs it, the chevron part on its right opens the card, and there is no
/// second, icon-only button beside it.
#[test]
fn auto_arrange_is_one_split_button() {
    let main = tools::by_act(tools::Act::AutoArrange).button_name();
    let more = tools::by_act(tools::Act::AutoArrangePanel).button_name();
    for pins in [
        &["auto_arrange"][..],
        &["auto_arrange", "auto_arrange.panel"][..],
        &["auto_arrange.panel"][..],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        let [a, b, _] = three(&s);
        select(&s, &[a, b]);
        pin(&s, pins);
        let mut h = window(&dir, s.clone(), 1400.0);
        h.get_by_role_and_label(Role::Tab, "Layout").click();
        h.run_plan();
        assert_eq!(
            h.query_all_by_role_and_label(Role::Button, &main).count(),
            1,
            "{pins:?}"
        );
        assert_eq!(
            h.query_all_by_role_and_label(Role::Button, &more).count(),
            1,
            "{pins:?}"
        );
        let m = h.get_by_role_and_label(Role::Button, &main).rect();
        let c = h.get_by_role_and_label(Role::Button, &more).rect();
        // One button: the chevron touches the main part and is as tall.
        assert!((c.left() - m.right()).abs() < 0.5, "{pins:?}: {m:?} {c:?}");
        assert!((c.top() - m.top()).abs() < 0.5 && (c.height() - m.height()).abs() < 0.5);
        assert!(c.width() < m.width() / 2.0, "{pins:?}: a narrow chevron");
        // The chevron opens the card and changes nothing.
        let before = (document(&s), undo_len(&s));
        click_at(&mut h, c.center());
        let p = app(&h).ui.prepare.panel.as_ref().map(|p| p.kind);
        assert_eq!(p, Some(PanelKind::AutoArrange), "{pins:?}");
        assert_eq!((document(&s), undo_len(&s)), before);
        h.get_by_role_and_label(Role::Button, &cancel_name())
            .click();
        h.run_plan();
        // The main part runs Auto arrange at once: one step, no card.
        click_at(&mut h, m.center());
        wait_for_workers(&mut h);
        assert!(app(&h).ui.prepare.panel.is_none(), "{pins:?}");
        assert_eq!(undo_len(&s), before.1 + 1, "{pins:?}");
    }
}

#[test]
fn the_auto_arrange_card_row_has_no_pin_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    three(&s);
    let mut h = window(&dir, s.clone(), 1400.0);
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.run_plan();
    h.get_by_role_and_label(Role::Button, "Arrange").click();
    h.run_plan();
    let t = tools::by_act(tools::Act::AutoArrangePanel);
    h.get_by_label(&t.row_name()).hover();
    h.run_plan();
    let auto = tools::by_act(tools::Act::AutoArrange);
    h.get_by_label(&auto.row_name()).hover();
    h.run_plan();
    assert!(h.query_by_label_contains("Pin Auto arrange...").is_none());
    assert!(
        h.query_all_by_label_contains("Pin Auto arrange").count() == 1
            || h.query_all_by_label_contains("Unpin Auto arrange").count() == 1
    );
}

/// The Auto arrange row unpins the split button whichever of its two ids
/// an old pin file holds, and pinning adds only its own id.
#[test]
fn unpinning_auto_arrange_removes_the_split_button() {
    let main = tools::by_act(tools::Act::AutoArrange);
    let more = tools::by_act(tools::Act::AutoArrangePanel).button_name();
    for pins in [
        &["auto_arrange", "auto_arrange.panel", "group"][..],
        &["auto_arrange.panel", "group"][..],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        three(&s);
        pin(&s, pins);
        let mut h = window(&dir, s.clone(), 1400.0);
        h.get_by_role_and_label(Role::Tab, "Layout").click();
        h.run_plan();
        assert!(h.query_by_role_and_label(Role::Button, &more).is_some());
        h.get_by_role_and_label(Role::Button, "Arrange").click();
        h.run_plan();
        h.get_by_label(&main.row_name()).hover();
        h.run_plan();
        h.get_by_label("Unpin Auto arrange").click();
        h.run_plan();
        let stored = s.lock().unwrap().prefs.ribbon_pins.clone();
        assert_eq!(stored, vec!["group".to_string()], "{pins:?}");
        h.key_press(egui::Key::Escape);
        h.run_plan();
        assert!(
            h.query_by_role_and_label(Role::Button, &more).is_none(),
            "{pins:?}"
        );
        // Pinning it again adds only its own id.
        h.get_by_role_and_label(Role::Button, "Arrange").click();
        h.run_plan();
        h.get_by_label(&main.row_name()).hover();
        h.run_plan();
        h.get_by_label("Pin Auto arrange").click();
        h.run_plan();
        let stored = s.lock().unwrap().prefs.ribbon_pins.clone();
        assert!(stored.contains(&"auto_arrange".to_string()), "{pins:?}");
        assert!(
            !stored.contains(&"auto_arrange.panel".to_string()),
            "{pins:?}"
        );
    }
}

/// In a short window the card's fields scroll inside it, so its title
/// and OK stay in sight.
#[test]
fn the_card_buttons_stay_in_sight_in_a_short_window() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let [a, b, _] = three(&s);
    select(&s, &[a, b]);
    let mut h = harness_sized(&dir, s.clone(), [900.0, 500.0], 8, |a| a);
    to_prepare(&mut h);
    wait_for_workers(&mut h);
    open(&mut h, PanelKind::CircularPattern);
    let ok = h.get_by_role_and_label(Role::Button, "OK").rect();
    assert!(ok.bottom() <= 500.0, "{ok:?}");
    let close = h.get_by_label(tool_card::CLOSE).rect();
    assert!(close.top() >= 0.0, "{close:?}");
}
