//! The Objects panel (Task 64): the filter field over the list, the Notes
//! folded at the bottom with their first words, the right rail's job and
//! selection summary, and the left panel in a small window.

mod common;
use common::RunPlan;

use common::*;
use egui::accesskit::Role;
use egui::{Key, Rect};
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::objects;
use flatcraft::ui::prepare;
use flatcraft::ui::rails::{self, Dot};
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

fn snap(h: &mut AppHarness) -> prepare::Snapshot {
    let ctx = h.ctx.clone();
    prepare::snapshot(app_mut(h), &ctx)
}

/// The ids of the scene: a group "Badge" of "square" and "star", then
/// "circle" and "label" on their own.
struct Scene {
    square: u64,
    star: u64,
    circle: u64,
    badge: u64,
}

const NOTES: &str = "Blue mat, face down. Force 14 worked best on the glitter vinyl, two passes for the small letters.";

/// The scene, with the star selected.
fn scene(s: &SharedSession) -> Scene {
    let square = import_square(s);
    exec(
        s,
        "object.rename",
        json!({ "id": square, "name": "square" }),
    );
    let add = |name: &str, x: f64, y: f64| {
        let id = import_square(s);
        exec(
            s,
            "object.move",
            json!({ "ids": [id], "x_mm": x, "y_mm": y }),
        );
        exec(s, "object.rename", json!({ "id": id, "name": name }));
        id
    };
    let star = add("star", 100.0, 20.0);
    let circle = add("circle", 20.0, 120.0);
    add("label", 100.0, 120.0);
    let badge = exec(s, "object.group", json!({ "ids": [square, star] }))["group_id"]
        .as_u64()
        .unwrap();
    exec(s, "object.rename", json!({ "id": badge, "name": "Badge" }));
    exec(s, "object.select", json!({ "ids": [star] }));
    Scene {
        square,
        star,
        circle,
        badge,
    }
}

fn prepare_at(width: f32) -> (tempfile::TempDir, SharedSession, AppHarness, Scene) {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let sc = scene(&s);
    let mut h = harness_sized(&dir, s.clone(), [width, 900.0], 8, |a| a);
    to_prepare(&mut h);
    h.run_plan();
    (dir, s, h, sc)
}

/// Whether the Objects list shows the row `name` (a row is a button; the
/// inspector's heading with the same name is a label).
fn row(h: &AppHarness, name: &str) -> bool {
    h.query_by_role_and_label(Role::Button, name).is_some()
}

fn group_row(h: &AppHarness, name: &str) -> bool {
    h.query_by_label_contains(&format!("{name}, group of"))
        .is_some()
}

/// Types `text` into the filter field.
fn filter(h: &mut AppHarness, text: &str) {
    h.get_by_label(objects::FILTER_NAME).click();
    h.run_plan();
    h.get_by_label(objects::FILTER_NAME).type_text(text);
    h.run_plan();
}

/// The texts painted left of the canvas last frame.
fn left_texts(h: &AppHarness) -> Vec<String> {
    let c = app(h).ui.prepare.canvas_rect;
    painted(h)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) if t.pos.x < c.left() => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn search_filters_without_changing_selection() {
    let (_d, s, mut h, sc) = prepare_at(1400.0);
    assert!(row(&h, "circle") && row(&h, "label") && group_row(&h, "Badge"));
    filter(&mut h, "CIRC");
    // Case does not matter; only the match shows.
    assert!(row(&h, "circle"));
    assert!(!row(&h, "label"), "label does not match");
    assert!(!group_row(&h, "Badge"), "nothing in Badge matches");
    // The selection is what it was; the header tells that it is hidden.
    assert_eq!(selection(&s), vec![sc.star]);
    let note = objects::filter_note(1, 1);
    assert_eq!(note, "1 selected, 1 hidden by the filter");
    assert!(h.query_by_label(&note).is_some(), "the header says it");
    // A match that is selected is not hidden: no note.
    exec(&s, "object.select", json!({ "ids": [sc.circle] }));
    h.run_plan();
    assert!(h.query_by_label_contains("hidden by the filter").is_none());
    exec(&s, "object.select", json!({ "ids": [sc.star] }));
    h.run_plan();
    // A filter that matches nothing says so.
    h.get_by_label(objects::FILTER_NAME).type_text("zzz");
    h.run_plan();
    assert!(!row(&h, "circle"));
    assert!(h.query_by_label_contains(objects::NO_MATCH).is_some());
    // Escape in the field clears the filter, and only that.
    h.key_press(Key::Escape);
    h.run_plan();
    assert_eq!(field_text(&h, objects::FILTER_NAME), "");
    assert!(row(&h, "circle") && row(&h, "label") && group_row(&h, "Badge"));
    assert_eq!(selection(&s), vec![sc.star], "Escape kept the selection");
    assert!(h.query_by_label_contains("hidden by the filter").is_none());
    // The next Escape goes on as before: it clears the selection.
    h.key_press(Key::Escape);
    h.run_plan();
    assert!(selection(&s).is_empty());
}

#[test]
fn shift_click_ranges_over_the_rows_the_filter_shows() {
    let (_d, s, mut h, sc) = prepare_at(1400.0);
    // "b" matches Badge (its members show under it) and label, not
    // circle, which sits between star and label.
    filter(&mut h, "b");
    assert!(row(&h, "star") && row(&h, "label") && !row(&h, "circle"));
    let at = |h: &AppHarness, name: &str| h.get_by_role_and_label(Role::Button, name).rect();
    let p = at(&h, "star").center();
    click_at(&mut h, p);
    assert_eq!(selection(&s), vec![sc.star]);
    let p = at(&h, "label").center();
    let mods = egui::Modifiers::SHIFT;
    h.event(egui::Event::ModifiersChanged(mods));
    h.event(egui::Event::PointerMoved(p));
    h.run_plan();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: mods,
        });
        h.step();
    }
    h.run_plan();
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_plan();
    let got = selection(&s);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(
        got.contains(&sc.star) && !got.contains(&sc.circle),
        "{got:?}"
    );
}

#[test]
fn group_matches_show_members() {
    let (_d, s, mut h, sc) = prepare_at(1400.0);
    // Badge is folded: its members have no rows.
    assert!(!row(&h, "square") && !row(&h, "star"));
    filter(&mut h, "badge");
    assert!(group_row(&h, "Badge"));
    assert!(
        row(&h, "square") && row(&h, "star"),
        "the members stay under it"
    );
    assert!(!row(&h, "circle"));
    assert_eq!(selection(&s), vec![sc.star]);
    // A matching member shows with its group above it, not its siblings.
    h.key_press(Key::Escape);
    h.run_plan();
    filter(&mut h, "squ");
    assert!(group_row(&h, "Badge") && row(&h, "square"));
    assert!(!row(&h, "star"));
    // Clearing the filter folds Badge again: the filter never changed
    // which groups are open.
    h.key_press(Key::Escape);
    h.run_plan();
    assert!(!row(&h, "square"));
    let doc = s.lock().unwrap().doc().id();
    assert!(!app(&h).ui.prepare.open_groups.contains(&(doc, sc.badge)));
    let _ = sc.square;
}

#[test]
fn notes_fold_shows_first_words() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    scene(&s);
    exec(&s, "document.set_notes", json!({ "notes": NOTES }));
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 900.0], 8, |a| a);
    to_prepare(&mut h);
    h.run_plan();
    // Folded at first, the header shows the first words on its one line.
    assert!(h.query_by_label("Project notes").is_none(), "folded");
    let texts = left_texts(&h);
    assert!(
        texts.iter().any(|t| t.starts_with("Blue mat, face down.")),
        "{texts:?}"
    );
    // The whole text is in the hover card.
    h.ctx
        .global_style_mut(|st| st.interaction.tooltip_delay = 0.0);
    let at = h
        .get_by_role_and_label(Role::Button, "Notes")
        .rect()
        .center();
    h.hover_at(at);
    h.run_plan();
    h.run_plan();
    assert!(
        h.query_by_label_contains("two passes for the small letters")
            .is_some(),
        "the card holds all of it"
    );
    // The Notes sit at the bottom of the panel.
    let header = h.get_by_role_and_label(Role::Button, "Notes").rect();
    assert!(header.bottom() > 900.0 - 60.0, "{header:?}");
    // Open: the fields show, and the choice is kept in the preferences.
    click_at(&mut h, at);
    assert!(h.query_by_label("Project notes").is_some(), "open");
    assert_eq!(
        s.lock()
            .unwrap()
            .prefs
            .panel_sections
            .get(objects::NOTES_SECTION),
        Some(&true)
    );
    // Empty notes: the description stands in, else nothing.
    assert_eq!(
        objects::notes_summary("", "Kitchen labels"),
        "Kitchen labels"
    );
    assert_eq!(objects::notes_summary(" \n", " "), "");
    assert_eq!(
        objects::notes_summary("First line\nsecond line", "x"),
        "First line second line"
    );
}

#[test]
fn right_rail_shows_job_and_selection() {
    // Below 1100 pt the right panel is a rail.
    let (_d, s, mut h, sc) = prepare_at(1000.0);
    let sn = snap(&mut h);
    let job = flatcraft::ui::inspector::job::summary(&sn, Dot::Ready, false);
    let sum = rails::settings_summary(&sn, Dot::Ready);
    assert_eq!(sum.parts[0], "star: Object outlines");
    let plotter = job.plotter_short.clone().unwrap_or(job.plotter.clone());
    assert_eq!(
        &sum.parts[1..],
        [plotter, job.material, job.mat, job.strategy]
    );
    let rail = h.get_by_label_contains("Open the Cut and Job panel");
    let rect: Rect = rail.rect();
    assert!(rect.width() <= rails::RAIL_W + 0.5, "the rail stays narrow");
    // The rail reads the same text.
    assert!(h.query_by_label_contains("star: Object outlines").is_some());
    // Two objects in different modes, then nothing selected.
    exec(
        &s,
        "object.set_cut_options",
        json!({ "ids": [sc.circle], "mode": "all_lines" }),
    );
    exec(&s, "object.select", json!({ "ids": [sc.star, sc.circle] }));
    h.run_plan();
    assert_eq!(
        rails::settings_summary(&snap(&mut h), Dot::Ready).parts[0],
        "2 objects: mixed"
    );
    exec(&s, "object.select", json!({ "ids": [] }));
    h.run_plan();
    let sum = rails::settings_summary(&snap(&mut h), Dot::Ready);
    assert_eq!(sum.parts.len(), 4, "the job only: {:?}", sum.parts);
    // A click opens the panel.
    let c = h
        .get_by_label_contains("Open the Cut and Job panel")
        .rect()
        .center();
    click_at(&mut h, c);
    assert!(h.query_by_label("Collapse the Cut and Job panel").is_some());
}

#[test]
fn left_panel_reachable_at_900() {
    let (_d, _s, mut h, _) = prepare_at(900.0);
    // The Objects panel starts as a rail at 900 pt; a click opens it.
    let c = h
        .get_by_label_contains("Open the Objects panel")
        .rect()
        .center();
    click_at(&mut h, c);
    let window = Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(900.0, 900.0));
    let canvas = app(&h).ui.prepare.canvas_rect;
    let inside = |r: Rect, what: &str| {
        assert!(window.contains_rect(r), "{what} {r:?}");
        assert!(r.right() <= canvas.left() + 0.5, "{what} {r:?} {canvas:?}");
    };
    inside(h.get_by_label(objects::FILTER_NAME).rect(), "the filter");
    inside(
        h.get_by_role_and_label(Role::Button, "circle").rect(),
        "a row",
    );
    let notes = h.get_by_role_and_label(Role::Button, "Notes").rect();
    inside(notes, "the Notes header");
    click_at(&mut h, notes.center());
    for field in ["Project description", "Project notes"] {
        inside(h.get_by_label(field).rect(), field);
    }
    // The list stays reachable with the Notes open.
    inside(
        h.get_by_role_and_label(Role::Button, "label").rect(),
        "a row",
    );
}
