//! The Objects panel (Task 64): the filter field over the list, the Notes
//! folded at the bottom with their first words, the right rail's job and
//! selection summary, and the left panel in a small window.

mod common;
use common::RunPlan;

use common::*;
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, Rect, Vec2};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
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
    // The header counts what the list shows: circle of 4 objects.
    let note = objects::filter_note(1, 4);
    assert_eq!(note, "1 of 4 shown");
    assert!(h.query_by_label(&note).is_some(), "the header says it");
    // Whole on its line at the default panel width, not cut short.
    let texts = left_texts(&h);
    assert!(texts.contains(&note), "{texts:?}");
    // Its card says the selected star is not shown.
    assert_eq!(
        objects::filter_note_card(3, 4, 1),
        "3 of 4 objects shown. 1 selected object is not shown because of the filter. Clear the filter to see it."
    );
    assert!(objects::filter_note_card(2, 4, 2).ends_with(
        "2 selected objects are not shown because of the filter. Clear the filter to see them."
    ));
    h.ctx
        .global_style_mut(|st| st.interaction.tooltip_delay = 0.0);
    let at = h.get_by_label(&note).rect().center();
    h.hover_at(at);
    h.run_plan();
    h.run_plan();
    assert!(
        h.query_by_label_contains("1 selected object is not shown because of the filter")
            .is_some()
    );
    // The pointer leaves, the card closes.
    let away = app(&h).ui.prepare.canvas_rect.center();
    h.hover_at(away);
    h.run_plan();
    // A match that is selected is not left out: the card does not say
    // so, the note stays.
    exec(&s, "object.select", json!({ "ids": [sc.circle] }));
    h.run_plan();
    assert!(h.query_by_label(&note).is_some());
    assert!(!objects::filter_note_card(1, 4, 0).contains("selected"));
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
    // No filter, no note.
    assert!(h.query_by_label_contains(" shown").is_none());
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

/// Sets the filter as if typed, without giving the field the keys.
fn set_filter(h: &mut AppHarness, s: &SharedSession, text: &str) {
    let doc = s.lock().unwrap().doc().id();
    app_mut(h).ui.prepare.objects_filter = Some((doc, text.to_string()));
    h.run_plan();
}

#[test]
fn typing_in_the_filter_leaves_tools_and_selection_alone() {
    let (_d, s, mut h, sc) = prepare_at(1400.0);
    let count = s.lock().unwrap().doc().document.objects.len();
    h.get_by_label(objects::FILTER_NAME).click();
    h.run_plan();
    // V and L switch the tools, Delete and Backspace remove the selection:
    // not while the filter has the keys.
    for (key, text) in [(Key::V, "v"), (Key::L, "l")] {
        h.key_press(key);
        h.event(Event::Text(text.into()));
        h.run_plan();
    }
    assert_eq!(field_text(&h, objects::FILTER_NAME), "vl");
    h.key_press(Key::Delete);
    h.run_plan();
    h.key_press(Key::Backspace);
    h.run_plan();
    assert_eq!(field_text(&h, objects::FILTER_NAME), "v");
    let st = &app(&h).ui.prepare;
    assert_eq!(st.select_tool, prepare::SelectTool::Select);
    assert_eq!(selection(&s), vec![sc.star]);
    assert_eq!(s.lock().unwrap().doc().document.objects.len(), count);
}

/// The filter is about the list only. Cmd+A and the lasso pick on the mat,
/// so they reach objects whose rows the filter hides, and an object that
/// is selected and hidden stays selected through Cmd+A. The filter itself
/// never selects or deselects anything.
#[test]
fn select_all_and_lasso_pick_on_the_mat_whatever_the_filter() {
    let (_d, s, mut h, sc) = prepare_at(1400.0);
    set_filter(&mut h, &s, "circ");
    assert!(!row(&h, "label") && !group_row(&h, "Badge"));
    assert_eq!(selection(&s), vec![sc.star], "the filter selected nothing");
    // Cmd+A: every object, the hidden star among them.
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_plan();
    let all = selection(&s);
    assert_eq!(all.len(), 4, "{all:?}");
    assert!(all.contains(&sc.star), "the hidden star stays selected");
    assert!(h.query_by_label(&objects::filter_note(1, 4)).is_some());
    // The lasso around the star (100..150 x 20..70 mm), whose row is
    // hidden: it picks the star's group Badge, as on any mat.
    let rect = app(&h).ui.prepare.canvas_rect;
    let view = flatcraft::ui::canvas::CanvasView {
        offset: rect.center().to_vec2() - Vec2::new(145.0, 45.0) * 2.0,
        px_per_mm: 2.0,
    };
    let p = &mut app_mut(&mut h).ui.prepare;
    p.view = view;
    p.fit_pending = false;
    p.select_tool = prepare::SelectTool::Lasso;
    h.run_plan();
    let at = |x: f64, y: f64| view.mm_to_screen(kurbo::Point::new(x, y));
    let path = [
        at(92.0, 12.0),
        at(158.0, 12.0),
        at(158.0, 78.0),
        at(92.0, 78.0),
        at(92.0, 14.0),
    ];
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
    h.drop_at(path[4]);
    h.run_plan();
    let mut got = selection(&s);
    got.sort_unstable();
    assert_eq!(got, [sc.square, sc.star]);
    // Still filtered: the lasso changed the selection, not the filter.
    assert_eq!(field_text(&h, objects::FILTER_NAME), "circ");
}

#[test]
fn the_folded_objects_rail_says_filtered() {
    let (_d, s, mut h, _) = prepare_at(1400.0);
    set_filter(&mut h, &s, "circ");
    h.get_by_label("Collapse the Objects panel").click();
    h.run_plan();
    let rail = h.get_by_label_contains("Open the Objects panel");
    let name = rail.accesskit_node().label().unwrap_or_default();
    assert!(name.contains(rails::FILTERED), "{name}");
    let sum = rails::objects_summary_filtered(&snap(&mut h), "circ");
    assert_eq!(sum.parts[1], rails::FILTERED, "right after the count");
    assert!(rails::filtered_card("circ").contains("\"circ\""));
    // Its card explains it.
    h.ctx
        .global_style_mut(|st| st.interaction.tooltip_delay = 0.0);
    let at = h
        .get_by_label_contains("Open the Objects panel")
        .rect()
        .center();
    h.hover_at(at);
    h.run_plan();
    h.run_plan();
    assert!(
        h.query_by_label_contains("The list is filtered by")
            .is_some()
    );
    // No filter, no word.
    assert!(
        !rails::objects_summary_filtered(&snap(&mut h), " ")
            .parts
            .iter()
            .any(|p| p == rails::FILTERED)
    );
}

#[test]
fn a_filter_field_that_is_not_drawn_never_takes_an_escape() {
    let (_d, s, mut h, _) = prepare_at(1400.0);
    set_filter(&mut h, &s, "circ");
    // The Objects panel as a rail, then the Cut workspace: the field is
    // not drawn, so it cannot have had the keys.
    for step in ["rail", "cut"] {
        if step == "rail" {
            h.get_by_label("Collapse the Objects panel").click();
        } else {
            app_mut(&mut h).ui.workspace = flatcraft::ui::Workspace::Cut;
        }
        h.run_plan();
        app_mut(&mut h).ui.prepare.objects_filter_keys = true;
        h.run_plan();
        assert!(!app(&h).ui.prepare.objects_filter_keys, "{step}");
    }
    to_prepare(&mut h);
    h.run_plan();
    // Escape now clears the selection, as always, and keeps the filter.
    h.key_press(Key::Escape);
    h.run_plan();
    assert!(selection(&s).is_empty());
    let doc = s.lock().unwrap().doc().id();
    assert_eq!(
        app(&h).ui.prepare.objects_filter,
        Some((doc, "circ".to_string()))
    );
}

#[test]
fn filtered_survives_a_long_rail_summary_cut_short() {
    // Many counts and a short window: the rail text is cut short, and
    // "filtered" still shows, right after the count.
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let sc = scene(&s);
    exec(
        &s,
        "object.select",
        json!({ "ids": [sc.star, sc.circle, sc.square] }),
    );
    exec(
        &s,
        "object.set_visible",
        json!({ "ids": [sc.circle], "visible": false }),
    );
    exec(
        &s,
        "object.set_cut",
        json!({ "ids": [sc.square], "cut": false }),
    );
    let mut h = harness_sized(&dir, s.clone(), [900.0, 300.0], 8, |a| a);
    to_prepare(&mut h);
    h.run_plan();
    set_filter(&mut h, &s, "circ");
    let left = painted(&h)
        .into_iter()
        .find_map(|sh| match sh {
            egui::Shape::Text(t) if t.angle < 0.0 => Some(t.galley.text().to_string()),
            _ => None,
        })
        .expect("the left rail's text");
    assert!(left.ends_with("..."), "cut short: {left}");
    assert!(left.starts_with("4 objects · filtered"), "{left}");
}
