//! Keep mode in the window (Task 29): the Keep combo in the Settings
//! panel, and the kept and waste overlay (always on in Cut, a View toggle
//! in Prepare).

mod common;

use common::{
    AppHarness, app, app_mut, harness, import_square, painted, session, to_prepare, undo_len,
    wait_for_workers,
};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::Workspace;
use flatcraft::ui::menu_model::Cmd;
use flatcraft::ui::tools::Setting;
use flatcraft_core::document::{KeepMode, ObjectId};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

fn run(s: &SharedSession, cmd: &str, params: Value) -> Value {
    s.lock()
        .unwrap()
        .execute(cmd, params, Origin::User)
        .unwrap()
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

/// True when the text was painted last frame.
fn drawn(h: &AppHarness, text: &str) -> bool {
    painted(h).iter().any(|s| match s {
        egui::Shape::Text(t) => t.galley.text() == text,
        _ => false,
    })
}

/// True when the overlay's texture was painted last frame.
fn overlay_drawn(h: &AppHarness) -> bool {
    let Some(id) = app(h).ui.cut.kept.texture_id() else {
        return false;
    };
    painted(h).iter().any(|s| match s {
        egui::Shape::Mesh(m) => m.texture_id == id,
        _ => false,
    })
}

#[test]
fn the_keep_combo_sets_the_selection_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = import_square(&s);
    run(&s, "object.select", json!({ "ids": [a] }));
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    assert_eq!(h.query_all_by_label("Keep").count(), 1);
    let before = undo_len(&s);
    h.get_by_role_and_label(Role::ComboBox, "Keep").click();
    h.run();
    h.get_by_label("Stencil").click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(keep(&s, a), KeepMode::Stencil);
    assert_eq!(keep(&s, b), KeepMode::Pieces);
    assert!(drawn(&h, "Stencil"), "the combo shows the new mode");
}

#[test]
fn a_mixed_selection_shows_mixed() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = import_square(&s);
    run(&s, "object.set_keep", json!({ "ids": [b], "keep": "both" }));
    run(&s, "object.select", json!({ "ids": [a, b] }));
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    assert!(drawn(&h, "Mixed"));
    run(&s, "object.select", json!({ "ids": [b] }));
    h.run();
    assert!(!drawn(&h, "Mixed") && drawn(&h, "Both"));
}

#[test]
fn the_keep_card_explains_the_modes_in_plain_words() {
    let card = Setting::Keep.card();
    for word in ["Pieces", "stickers", "Stencil", "stencil", "Both", "waste"] {
        assert!(card.contains(word), "{word}: {card}");
    }
    assert!(!card.contains('\u{2014}'));
    for k in [KeepMode::Pieces, KeepMode::Stencil, KeepMode::Both] {
        let hint = flatcraft::ui::settings::keep_hint(k);
        assert!(!hint.is_empty() && !hint.contains('\u{2014}'));
    }
}

#[test]
fn prepare_shows_kept_and_waste_only_when_switched_on() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    h.run();
    assert!(!app(&h).ui.prepare.show_kept_waste, "off by default");
    assert!(!overlay_drawn(&h));
    let ctx = h.ctx.clone();
    let doc = s.lock().unwrap().doc().id();
    let toggle = Cmd::Tool("view.kept").action(doc, &[]).unwrap();
    app_mut(&mut h).apply(&ctx, vec![toggle.clone()]);
    h.run();
    assert!(app(&h).ui.prepare.show_kept_waste);
    assert!(overlay_drawn(&h), "the overlay is drawn");
    let made = app(&h).ui.cut.kept.made;
    for _ in 0..3 {
        h.step();
    }
    assert_eq!(app(&h).ui.cut.kept.made, made, "made once per plan");
    // A new plan makes a new mask.
    run(&s, "object.set_keep", json!({ "keep": "stencil" }));
    h.run();
    assert_eq!(app(&h).ui.cut.kept.made, made + 1);
    app_mut(&mut h).apply(&ctx, vec![toggle]);
    h.run();
    assert!(!overlay_drawn(&h), "switched off again");
}

#[test]
fn cut_always_shows_kept_and_waste() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    assert!(!app(&h).ui.prepare.show_kept_waste);
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run();
    assert!(overlay_drawn(&h));
    h.get_by_role_and_label(Role::Tab, "Path").click();
    h.run();
    assert!(overlay_drawn(&h));
    h.get_by_label(flatcraft::ui::cut::LEGEND_KEPT.0);
    h.get_by_label(flatcraft::ui::cut::LEGEND_WASTE.0);
}
