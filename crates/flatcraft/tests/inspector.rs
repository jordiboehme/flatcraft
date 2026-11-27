//! The inspector on the right of Prepare: the job line, the Job panel's
//! sections with their summaries, the change dot and reset, the open
//! sections remembered in the preferences, and the Escape order.

mod common;
use common::RunPlan;

use common::*;
use egui::accesskit::Role;
use egui::{Key, Shape};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::inspector::{self, InspectorView, blocks, job, object};
use flatcraft::ui::prepare;
use flatcraft_engine::prefs::{PanelUser, Preferences};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

fn run(s: &SharedSession, id: &str, params: Value) -> Value {
    s.lock().unwrap().execute(id, params, Origin::User).unwrap()
}

fn snap(h: &mut AppHarness) -> prepare::Snapshot {
    let ctx = h.ctx.clone();
    prepare::snapshot(app_mut(h), &ctx)
}

fn view(h: &AppHarness) -> InspectorView {
    app(h).ui.prepare.inspector
}

fn has(h: &AppHarness, label: &str) -> bool {
    h.query_by_label(label).is_some()
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

/// A Prepare window `w` points wide with the right panel open and nothing
/// selected; `setup` runs on the session first.
fn window(
    dir: &tempfile::TempDir,
    w: f32,
    setup: impl FnOnce(&SharedSession),
) -> (SharedSession, AppHarness) {
    let s = session(dir);
    s.lock().unwrap().prefs.panel_settings = PanelUser::Open;
    import_square(&s);
    run(&s, "object.select", json!({ "ids": [] }));
    setup(&s);
    let mut h = harness_sized(dir, s.clone(), [w, 900.0], 8, |a| a);
    to_prepare(&mut h);
    wait_for_workers(&mut h);
    (s, h)
}

fn job_line(h: &AppHarness) -> egui_kittest::Node<'_> {
    h.get_by_label_contains("Job: ")
}

#[test]
fn job_line_shows_plotter_material_mat_strategy() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = window(&dir, 1400.0, |s| {
        run(s, "job.set_material", json!({ "id": "vinyl" }));
    });
    let sn = snap(&mut h);
    let (plotter, material, mat) = (
        sn.plotter.as_ref().unwrap().name.clone(),
        sn.material.as_ref().unwrap().name.clone(),
        sn.mat.as_ref().unwrap().name.clone(),
    );
    let name = format!("Job: {plotter} · {material} · {mat} · Quality");
    assert_eq!(
        job_line(&h).accesskit_node().label().as_deref(),
        Some(&*name)
    );
    assert_eq!(job_line(&h).accesskit_node().role(), Role::Button);
    // It follows the document in the same frame.
    run(&s, "job.set_strategy", json!({ "strategy": "fast" }));
    h.step();
    h.get_by_label_contains("· Fast");
    // It is one line, cut short with its card holding all of it.
    assert!((job_line(&h).rect().height() - blocks::JOB_LINE_H).abs() < 0.5);
    let sn = snap(&mut h);
    let dot = flatcraft::ui::rails::job_dot(&sn, None);
    let card = blocks::job_line_card(&job::summary(&sn, dot, false));
    assert!(card.contains(&plotter) && card.contains(&material) && card.contains(&mat));
    assert!(!card.contains('\u{2014}'));
}

#[test]
fn job_line_click_shows_job_panel_with_a_selection_and_selection_change_returns() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = window(&dir, 1400.0, |_| {});
    // Nothing selected: the Job panel, no cut options. A click on the job
    // line changes nothing then: the Job panel is already there.
    assert!(has(&h, "Plotter") && !has(&h, "Cut mode"));
    job_line(&h).click();
    h.run_plan();
    assert_eq!(view(&h), InspectorView::Selection);
    let first = selection_all(&s)[0];
    let second = import_square(&s);
    run(&s, "object.select", json!({ "ids": [first] }));
    h.run_plan();
    // A selection: the object inspector, the job is behind the job line.
    assert!(has(&h, "Cut mode") && !has(&h, "Plotter"));
    assert_eq!(view(&h), InspectorView::Selection);
    job_line(&h).click();
    h.run_plan();
    assert_eq!(view(&h), InspectorView::Job);
    assert!(!has(&h, "Cut mode"), "only the Job panel");
    assert!(has(&h, "Plotter") && has(&h, "Speed"));
    assert_eq!(selection(&s), vec![first], "the selection stays");
    // A second click goes back.
    job_line(&h).click();
    h.run_plan();
    assert_eq!(view(&h), InspectorView::Selection);
    assert!(has(&h, "Cut mode"));
    // Another selection goes back too, in the same frame.
    job_line(&h).click();
    h.run_plan();
    assert_eq!(view(&h), InspectorView::Job);
    run(&s, "object.select", json!({ "ids": [second] }));
    h.step();
    assert_eq!(view(&h), InspectorView::Selection);
    assert!(has(&h, "Cut mode"));
    // So does another document.
    job_line(&h).click();
    h.run_plan();
    run(&s, "document.new", json!({}));
    h.step();
    assert_eq!(view(&h), InspectorView::Selection);
    // It is UI state only: no undo step, nothing in the document.
    assert_eq!(undo_len(&s), 0);
}

fn selection_all(s: &SharedSession) -> Vec<u64> {
    let state = run(s, "document.state", json!({}));
    state["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_u64().unwrap())
        .collect()
}

/// The change dots painted last frame, in the change colour.
fn change_dots(h: &AppHarness) -> usize {
    let colour = app(h).tokens.changed;
    painted(h)
        .iter()
        .filter(|s| matches!(s, Shape::Circle(c) if c.fill == colour))
        .count()
}

#[test]
fn changed_force_shows_dot_and_reset_is_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = window(&dir, 1400.0, |_| {});
    assert_eq!(change_dots(&h), 0);
    assert!(!has(&h, "Force (modified)"));
    assert!(!has(&h, "Reset force"));
    let material = s.lock().unwrap().doc().document.job.material_id.clone();
    let base = snap(&mut h).material.unwrap().force;
    let other = if base == 10 { 12 } else { 10 };
    run(&s, "job.set_overrides", json!({ "force": other }));
    h.run_plan();
    h.get_by_label("Force (modified)");
    assert_eq!(change_dots(&h), 1);
    // The cards name the source and its value.
    let reset = h.get_by_role_and_label(Role::Button, "Reset force");
    reset.hover();
    h.run_plan();
    let text = format!("Puts back {base} from the material");
    assert!(
        h.query_by_label_contains(&text).is_some(),
        "the reset card says {text}"
    );
    let undo = undo_len(&s);
    h.get_by_role_and_label(Role::Button, "Reset force").click();
    h.run_plan();
    assert_eq!(undo_len(&s), undo + 1, "one undo step");
    let job = s.lock().unwrap().doc().document.job.clone();
    assert_eq!(job.force, None, "the material value again");
    assert_eq!(job.material_id, material);
    assert!(!has(&h, "Force (modified)"));
    assert!(!has(&h, "Reset force"));
    assert_eq!(change_dots(&h), 0);
    // An override equal to the material value is no change.
    run(&s, "job.set_overrides", json!({ "force": base }));
    h.run_plan();
    assert_eq!(change_dots(&h), 0);
    assert!(!has(&h, "Force (modified)"));
}

#[test]
fn sections_remember_open_state() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = window(&dir, 1400.0, |s| {
        // An id this version does not know is ignored.
        s.lock()
            .unwrap()
            .prefs
            .panel_sections
            .insert("nope.gone".into(), true);
    });
    let undo = undo_len(&s);
    // Folded by default: Weeding, Material tips and Advanced.
    assert!(!has(&h, "Weed box"));
    assert!(!has(&h, "Overcut"));
    assert!(has(&h, "Speed"));
    h.get_by_label("Weeding").click();
    h.run_plan();
    assert!(has(&h, "Weed box"));
    h.get_by_label("Cut values").click();
    h.run_plan();
    assert!(!has(&h, "Speed"));
    let stored = Preferences::load(dir.path()).panel_sections;
    assert_eq!(stored.get(job::WEEDING), Some(&true));
    assert_eq!(stored.get(job::VALUES), Some(&false));
    assert_eq!(stored.get("nope.gone"), Some(&true), "kept, not used");
    // Another window stored a section meanwhile: a fold here keeps it.
    s.lock()
        .unwrap()
        .update_prefs(|p| {
            p.panel_sections.insert(job::ORDER.into(), false);
        })
        .unwrap();
    h.get_by_label("Weeding").click();
    h.run_plan();
    h.get_by_label("Weeding").click();
    h.run_plan();
    let stored = Preferences::load(dir.path()).panel_sections;
    assert_eq!(stored.get(job::ORDER), Some(&false), "the other window's");
    assert_eq!(stored.get(job::WEEDING), Some(&true));
    assert_eq!(
        s.lock().unwrap().prefs.panel_sections.get(job::WEEDING),
        Some(&true)
    );
    // No undo step: it is no change to the document.
    assert_eq!(undo_len(&s), undo);
    // A new window opens them as they were left.
    drop(h);
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 900.0], 8, |a| a);
    to_prepare(&mut h);
    assert!(has(&h, "Weed box"));
    assert!(!has(&h, "Speed"));
    // The headers say whether they are open.
    assert_eq!(
        h.get_by_label("Weeding")
            .accesskit_node()
            .data()
            .is_expanded(),
        Some(true)
    );
    assert_eq!(
        h.get_by_label("Cut values")
            .accesskit_node()
            .data()
            .is_expanded(),
        Some(false)
    );
}

/// The painted text that equals `text`, if any.
fn painted_text(h: &AppHarness, text: &str) -> bool {
    painted(h).iter().any(|s| match s {
        Shape::Text(t) => t.galley.text() == text,
        _ => false,
    })
}

#[test]
fn section_summary_is_one_line_and_height_is_stable() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = window(&dir, 1400.0, |_| {});
    let rects = |h: &AppHarness| -> Vec<egui::Rect> {
        ["Weeding", flatcraft::ui::settings::TIPS_HEADING, "Advanced"]
            .map(|l| h.get_by_label(l).rect())
            .to_vec()
    };
    let before = rects(&h);
    for r in &before {
        assert!((r.height() - blocks::HEADER_H).abs() < 0.5, "{r:?}");
    }
    assert!(painted_text(&h, "off"));
    // A change of a folded section's value changes only its summary.
    run(
        &s,
        "job.set_weeding",
        json!({ "box": true, "lines": "both" }),
    );
    h.run_plan();
    assert!(painted_text(&h, "box · lines both"));
    assert_eq!(rects(&h), before);
    // A summary longer than the room is cut short on its one line.
    run(&s, "job.set_weeding", json!({ "per_object": true }));
    let narrow = 900.0;
    h.set_size(egui::vec2(narrow, 900.0));
    h.run_plan();
    let r = h.get_by_label("Weeding").rect();
    assert!((r.height() - blocks::HEADER_H).abs() < 0.5, "{r:?}");
    let summary = job::weeding_summary(&snap(&mut h), false);
    assert_eq!(summary, "box per object · lines both");
    assert!(!summary.contains('\n'));
}

/// Opens the right panel if it is folded to its rail (at 900 pt it
/// starts folded).
fn open_right_panel(h: &mut AppHarness) {
    if let Some(rail) = h.query_by_label_contains("Open the Cut and Job panel") {
        rail.click();
        h.run_plan();
    }
}

/// Scrolls the inspector down with the mouse wheel over `at`.
fn wheel_down(h: &mut AppHarness, at: egui::Pos2) {
    h.hover_at(at);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -120.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Trackpad,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_plan();
}

/// Every setting is in its section, at most one fold away, and can be
/// scrolled into view inside the window. egui only names the widgets it
/// lays out in view, so the person's way is followed: scroll, open the
/// section, scroll on.
fn reachable(h: &mut AppHarness, settings: &[(&str, &str)]) {
    let window = h.ctx.content_rect();
    for &(section, label) in settings {
        let title = job::SECTIONS
            .iter()
            .chain(&object::SECTIONS)
            .find(|(id, _)| *id == section)
            .unwrap()
            .1;
        let mut tries = 0;
        while h.query_by_label(label).is_none() {
            tries += 1;
            assert!(tries < 30, "{label} is not reachable");
            let header = h.get_by_label(title);
            let expanded = header.accesskit_node().data().is_expanded();
            // Over the inspector, away from the header and its card.
            let at = egui::pos2(job_line(h).rect().center().x, window.height() * 0.55);
            if expanded == Some(false) {
                header.scroll_to_me();
                h.run_plan();
                h.get_by_label(title).click();
                h.run_plan();
            } else {
                wheel_down(h, at);
            }
        }
        h.get_by_label(label).scroll_to_me();
        h.run_plan();
        let r = h.get_by_label(label).rect();
        assert!(window.contains_rect(r), "{label} at {r:?} in {window:?}");
        // The inspector's column, not under the canvas.
        assert!(r.left() > window.width() * 0.5, "{label} at {r:?}");
        assert_card(h, label);
    }
}

/// Hovering the widget named `label` shows its card, titled `label`.
fn assert_card(h: &mut AppHarness, label: &str) {
    let before = h.query_all_by_label(label).count();
    h.get_by_label(label).hover();
    h.run_plan();
    assert!(
        h.query_all_by_label(label).count() > before,
        "{label} shows no card"
    );
    // Away again, so the card covers nothing the next step clicks.
    h.hover_at(egui::pos2(1.0, 1.0));
    h.run_plan();
}

#[test]
fn every_job_setting_is_reachable() {
    for w in [900.0, 1920.0] {
        let dir = tempfile::tempdir().unwrap();
        let (s, mut h) = window(&dir, w, |s| {
            s.lock().unwrap().prefs.panel_settings = PanelUser::None;
        });
        open_right_panel(&mut h);
        // Every section header has its card too.
        for (_, title) in job::SECTIONS {
            h.get_by_label(title).scroll_to_me();
            h.run_plan();
            assert_card(&mut h, title);
        }
        reachable(&mut h, &job::SETTINGS);
        // With a selection the job is one click on the job line away.
        let id = selection_all(&s)[0];
        run(&s, "object.select", json!({ "ids": [id] }));
        h.run_plan();
        assert!(!has(&h, "Plotter"));
        job_line(&h).click();
        h.run_plan();
        reachable(&mut h, &job::SETTINGS);
    }
}

#[test]
fn every_job_row_and_section_has_a_plain_card() {
    for text in [
        job::PLOTTER_CARD,
        job::MAT_CARD,
        job::STRATEGY_CARD,
        inspector::SUBTITLES[0],
        inspector::SUBTITLES[1],
        job::COMPARE_CARD,
    ] {
        assert!(!text.is_empty());
        assert!(!text.contains('\u{2014}'), "{text}");
        assert!(text.ends_with('.'), "{text}");
    }
}

#[test]
fn escape_order_card_then_job_view_then_canvas() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = window(&dir, 1400.0, |_| {});
    let id = selection_all(&s)[0];
    run(&s, "object.select", json!({ "ids": [id] }));
    h.run_plan();
    // A tool card (the Arrange panel), the Job view and the Lasso tool.
    let sn = snap(&mut h);
    flatcraft::ui::arrange::open(
        &mut app_mut(&mut h).ui.prepare,
        &sn,
        flatcraft::ui::arrange::PanelKind::MoveCopy,
    );
    h.run_plan();
    job_line(&h).click();
    h.run_plan();
    app_mut(&mut h).ui.prepare.select_tool = prepare::SelectTool::Lasso;
    h.run_plan();
    assert!(app(&h).ui.prepare.panel.is_some());
    assert_eq!(view(&h), InspectorView::Job);
    // 1: the card cancels; nothing else changes.
    h.key_press(Key::Escape);
    h.run_plan();
    assert!(app(&h).ui.prepare.panel.is_none());
    assert_eq!(view(&h), InspectorView::Job);
    assert_eq!(app(&h).ui.prepare.select_tool, prepare::SelectTool::Lasso);
    assert_eq!(selection(&s), vec![id]);
    // 2: the Job view goes back to the selection.
    h.key_press(Key::Escape);
    h.run_plan();
    assert_eq!(view(&h), InspectorView::Selection);
    assert_eq!(app(&h).ui.prepare.select_tool, prepare::SelectTool::Lasso);
    assert_eq!(selection(&s), vec![id]);
    // 3: then the canvas has it as before: the Lasso tool goes back to
    // Select, and the next one clears the selection.
    h.key_press(Key::Escape);
    h.run_plan();
    assert_eq!(app(&h).ui.prepare.select_tool, prepare::SelectTool::Select);
    assert_eq!(selection(&s), vec![id]);
    h.key_press(Key::Escape);
    h.run_plan();
    assert!(selection(&s).is_empty());
    // With nothing selected the Job panel is the inspector anyway: Escape
    // is not taken by a Job view.
    job_line(&h).click();
    h.run_plan();
    let sn = snap(&mut h);
    assert!(!inspector::job_over_selection(&app(&h).ui.prepare, &sn));
}

#[test]
fn escape_cancels_a_lasso_in_progress_before_the_card_and_the_job_view() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = window(&dir, 1400.0, |_| {});
    let id = selection_all(&s)[0];
    run(&s, "object.select", json!({ "ids": [id] }));
    h.run_plan();
    let sn = snap(&mut h);
    flatcraft::ui::arrange::open(
        &mut app_mut(&mut h).ui.prepare,
        &sn,
        flatcraft::ui::arrange::PanelKind::MoveCopy,
    );
    h.run_plan();
    job_line(&h).click();
    h.run_plan();
    app_mut(&mut h).ui.prepare.select_tool = prepare::SelectTool::Lasso;
    h.run_plan();
    // A lasso drawn on the empty mat, still held.
    let c = app(&h).ui.prepare.canvas_rect.center() + egui::vec2(60.0, 60.0);
    let path = [
        c,
        c + egui::vec2(30.0, 0.0),
        c + egui::vec2(30.0, 30.0),
        c + egui::vec2(0.0, 30.0),
    ];
    h.hover_at(path[0]);
    h.run_plan();
    h.drag_at(path[0]);
    h.run_plan();
    for p in &path[1..] {
        h.hover_at(*p);
        h.run_plan();
    }
    assert!(matches!(
        app(&h).ui.prepare.drag,
        Some(prepare::Drag::Frame { .. })
    ));
    // 1: the lasso in progress goes (and the Lasso tool with it); the
    // card and the Job view stay. A held gesture comes first, so the
    // card never closes under a running drag.
    h.key_press(Key::Escape);
    h.run_plan();
    assert!(app(&h).ui.prepare.drag.is_none());
    assert_eq!(app(&h).ui.prepare.select_tool, prepare::SelectTool::Select);
    assert!(app(&h).ui.prepare.panel.is_some());
    assert_eq!(view(&h), InspectorView::Job);
    h.drop_at(path[3]);
    h.run_plan();
    assert_eq!(selection(&s), vec![id]);
    // 2: the card.
    h.key_press(Key::Escape);
    h.run_plan();
    assert!(app(&h).ui.prepare.panel.is_none());
    assert_eq!(view(&h), InspectorView::Job);
    // 3: the Job view.
    h.key_press(Key::Escape);
    h.run_plan();
    assert_eq!(view(&h), InspectorView::Selection);
    assert_eq!(selection(&s), vec![id]);
    // 4: the canvas clears the selection.
    h.key_press(Key::Escape);
    h.run_plan();
    assert!(selection(&s).is_empty());
}

/// The texts painted inside the job line, last frame.
fn job_line_texts(h: &AppHarness) -> Vec<String> {
    let r = job_line(h).rect();
    painted(h)
        .iter()
        .filter_map(|s| match s {
            Shape::Text(t) if r.contains(t.pos + egui::vec2(1.0, 1.0)) => {
                Some(t.galley.text().to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn job_line_always_paints_the_mat_and_the_strategy() {
    let dir = tempfile::tempdir().unwrap();
    // The longest material names, on a USB plotter and on File Output.
    for plotter in ["silhouette-cameo-2", "file-output-gpgl-cameo2"] {
        let (s, mut h) = window(&dir, 1400.0, |s| {
            run(s, "job.set_plotter", json!({ "id": plotter }));
            run(s, "job.set_material", json!({ "id": "cardstock-heavy" }));
        });
        // The panel has its default width.
        assert!(job_line(&h).rect().width() < 320.0);
        let texts = job_line_texts(&h);
        let sn = snap(&mut h);
        let mat = sn.mat.as_ref().unwrap().name.clone();
        assert!(texts.contains(&mat), "{texts:?}");
        assert!(texts.contains(&"Quality".to_string()), "{texts:?}");
        // A USB plotter shows its model, File Output the word File.
        let short = if plotter == "silhouette-cameo-2" {
            "Cameo 2"
        } else {
            job::FILE_SHORT
        };
        assert!(texts.contains(&short.to_string()), "{texts:?}");
        // The plotter and the material are cut short, each on its own.
        assert!(
            texts.iter().any(|t| t.starts_with("Cardstock")),
            "{texts:?}"
        );
        // A change of either shows at once.
        run(
            &s,
            "job.set_strategy",
            json!({ "strategy": "outlines_last" }),
        );
        let other = sn
            .plotter
            .as_ref()
            .unwrap()
            .media
            .iter()
            .find(|m| m.name != mat)
            .unwrap()
            .clone();
        run(&s, "job.set_media", json!({ "id": other.id }));
        h.run_plan();
        let texts = job_line_texts(&h);
        assert!(texts.contains(&"Outlines last".to_string()), "{texts:?}");
        assert!(texts.contains(&other.name), "{texts:?}");
        // The full text stays in the name (and the card).
        let name = job_line(&h).accesskit_node().label().unwrap_or_default();
        assert!(
            name.contains("Cardstock (heavy, about 250-300 g/m2)"),
            "{name}"
        );
        let full = &sn.plotter.as_ref().unwrap().name;
        assert!(name.contains(full.as_str()), "{name}");
    }
}

#[test]
fn the_inspector_keeps_its_width() {
    let dir = tempfile::tempdir().unwrap();
    let (_s, mut h) = window(&dir, 1400.0, |s| {
        run(s, "job.set_material", json!({ "id": "cardstock-heavy" }));
    });
    let first = job_line(&h).rect();
    for _ in 0..10 {
        h.step();
    }
    assert_eq!(job_line(&h).rect(), first, "no growth frame by frame");
    // Narrower and wider windows: the line fits its panel each time.
    for w in [1100.0, 1920.0, 1400.0] {
        h.set_size(egui::vec2(w, 900.0));
        h.run_plan();
        let r = job_line(&h).rect();
        for _ in 0..5 {
            h.step();
        }
        assert_eq!(job_line(&h).rect(), r, "stable at {w}");
        assert!(r.right() <= w, "inside the window at {w}");
    }
}

#[test]
fn escape_in_a_job_panel_field_only_leaves_the_field() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = window(&dir, 1400.0, |_| {});
    let id = selection_all(&s)[0];
    run(&s, "object.select", json!({ "ids": [id] }));
    h.run_plan();
    job_line(&h).click();
    h.run_plan();
    h.get_by_label("Force").click();
    h.run_plan();
    assert!(h.get_by_label("Force").is_focused());
    // 1: the field lets go; the Job view and the selection stay.
    h.key_press(Key::Escape);
    h.run_plan();
    assert!(!h.get_by_label("Force").is_focused());
    assert_eq!(view(&h), InspectorView::Job);
    assert_eq!(selection(&s), vec![id]);
    // 2: the Job view goes back.
    h.key_press(Key::Escape);
    h.run_plan();
    assert_eq!(view(&h), InspectorView::Selection);
    assert_eq!(selection(&s), vec![id]);
}

// ---------------------------------------------------------------- object inspector

/// The nodes named `label` in the inspector's column (right of the
/// canvas, below the ribbon): the ribbon and the Objects panel have some
/// of the same names.
fn in_inspector<'a>(h: &'a AppHarness, label: &'a str) -> Vec<egui_kittest::Node<'a>> {
    let left = app(h).ui.prepare.canvas_rect.right();
    let top = job_line(h).rect().top();
    h.query_all_by_label(label)
        .filter(|n| n.rect().left() >= left && n.rect().top() >= top)
        .collect()
}

fn the(h: &AppHarness, label: &str) -> egui::Rect {
    let found = in_inspector(h, label);
    assert_eq!(found.len(), 1, "{label} in the inspector");
    found[0].rect()
}

/// The shown value of the combo (or field) named `label`.
fn value(h: &AppHarness, label: &str) -> String {
    let found = in_inspector(h, label);
    assert_eq!(found.len(), 1, "{label} in the inspector");
    found[0].accesskit_node().value().unwrap_or_default()
}

fn rename(s: &SharedSession, id: u64, name: &str) {
    run(s, "object.rename", json!({ "id": id, "name": name }));
}

/// Two squares, "alpha" at 20,20 and "beta" at 100,20, nothing selected.
fn two_squares(s: &SharedSession) -> (u64, u64) {
    let a = selection_all(s)[0];
    let b = import_square(s);
    run(s, "object.move", json!({ "ids": [b], "x_mm": 100.0 }));
    rename(s, a, "alpha");
    rename(s, b, "beta");
    run(s, "object.select", json!({ "ids": [] }));
    (a, b)
}

#[test]
fn inspector_follows_selection_in_the_same_frame() {
    let dir = tempfile::tempdir().unwrap();
    let ids = std::cell::Cell::new((0, 0));
    let (s, mut h) = window(&dir, 1400.0, |s| {
        let (a, b) = two_squares(s);
        run(
            s,
            "object.set_keep",
            json!({ "ids": [b], "keep": "stencil" }),
        );
        ids.set((a, b));
    });
    let (a, b) = ids.get();
    let line = job_line(&h).rect();
    let job_heading = the(&h, "Job");
    run(&s, "object.select", json!({ "ids": [a] }));
    h.step();
    assert_eq!(the(&h, "alpha"), job_heading, "the heading stays put");
    assert_eq!(value(&h, "Keep"), "Pieces");
    // Another selection: the heading and the values follow in one frame.
    run(&s, "object.select", json!({ "ids": [b] }));
    h.step();
    assert!(in_inspector(&h, "alpha").is_empty());
    the(&h, "beta");
    assert_eq!(value(&h, "Keep"), "Stencil");
    // Back to nothing: the Job panel, in the same place.
    run(&s, "object.select", json!({ "ids": [] }));
    h.step();
    assert_eq!(the(&h, "Job"), job_heading);
    assert!(in_inspector(&h, "Keep").is_empty());
    assert_eq!(job_line(&h).rect(), line, "the job line never moves");
    // A group: named as one; a member of an entered group: its name and
    // its group.
    let g = run(&s, "object.group", json!({ "ids": [a, b] }))["group_id"]
        .as_u64()
        .unwrap();
    rename(&s, g, "Badge");
    run(&s, "object.select", json!({ "ids": [a, b] }));
    h.step();
    the(&h, "Group 'Badge'");
    let doc = s.lock().unwrap().doc().id();
    app_mut(&mut h).ui.prepare.entered = Some((doc, g));
    run(&s, "object.select", json!({ "ids": [b] }));
    h.step();
    the(&h, "beta · in group Badge");
    assert_eq!(value(&h, "Keep"), "Stencil");
}

#[test]
fn mixed_selection_shows_mixed() {
    let dir = tempfile::tempdir().unwrap();
    let ids = std::cell::Cell::new((0, 0));
    let (s, mut h) = window(&dir, 1400.0, |s| {
        let (a, b) = two_squares(s);
        run(
            s,
            "object.set_keep",
            json!({ "ids": [b], "keep": "stencil" }),
        );
        run(s, "object.rotate", json!({ "ids": [b], "angle_deg": 30.0 }));
        run(s, "object.select", json!({ "ids": [a, b] }));
        ids.set((a, b));
    });
    the(&h, "2 objects");
    assert_eq!(value(&h, "Keep"), "Mixed");
    // What they share shows as it is.
    assert_eq!(value(&h, "Cut mode"), "Object outlines");
    assert_eq!(value(&h, "Strokes"), "Auto");
    // Folded, the Cut summary says it too.
    h.get_by_label("Cut").click();
    h.run_plan();
    assert!(painted_text(&h, "Object outlines · Mixed"));
    h.get_by_label("Cut").click();
    h.run_plan();
    // Position and size: the common box, and no one angle.
    h.get_by_label("Position and size").click();
    h.run_plan();
    assert_eq!(value(&h, "X"), "20.00");
    assert_eq!(value(&h, "Angle"), "", "turned different ways");
    assert!(painted_text(&h, "Mixed"));
    // A change applies to both, one step to undo.
    let undo = undo_len(&s);
    let (a, b) = ids.get();
    h.get_by_label("Keep").click();
    h.run_plan();
    h.get_by_label("Both").click();
    h.run_plan();
    assert_eq!(undo_len(&s), undo + 1);
    let keep = |id: u64| {
        s.lock()
            .unwrap()
            .doc()
            .document
            .get(flatcraft_core::document::ObjectId(id))
            .unwrap()
            .keep
    };
    assert_eq!(keep(a), flatcraft_core::document::KeepMode::Both);
    assert_eq!(keep(b), flatcraft_core::document::KeepMode::Both);
    assert_eq!(value(&h, "Keep"), "Both");
}

#[test]
fn object_cut_mode_offers_merged_and_visible() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = window(&dir, 1400.0, |s| {
        let id = selection_all(s)[0];
        run(s, "object.select", json!({ "ids": [id] }));
    });
    let id = selection_all(&s)[0];
    let mode = |s: &SharedSession| {
        s.lock()
            .unwrap()
            .doc()
            .document
            .get(flatcraft_core::document::ObjectId(id))
            .unwrap()
            .cut
            .mode
    };
    for (entry, want) in [
        (
            "Merged outline",
            flatcraft_core::document::CutMode::MergedOutline,
        ),
        (
            "Visible outline",
            flatcraft_core::document::CutMode::VisibleOutline,
        ),
    ] {
        let undo = undo_len(&s);
        h.get_by_label("Cut mode").click();
        h.run_plan();
        h.get_by_label(entry).click();
        h.run_plan();
        assert_eq!(mode(&s), want);
        assert_eq!(undo_len(&s), undo + 1, "one step to undo");
        assert_eq!(value(&h, "Cut mode"), entry);
    }
    // Strokes do not apply to All lines: dimmed, and its card says why.
    run(
        &s,
        "object.set_cut_options",
        json!({ "ids": [id], "mode": "all_lines" }),
    );
    h.run_plan();
    let strokes = in_inspector(&h, "Strokes");
    assert!(strokes[0].accesskit_node().is_disabled());
    h.get_by_label("Strokes").hover();
    h.run_plan();
    assert!(
        h.query_by_label(object::STROKES_ALL_LINES).is_some(),
        "the card says why"
    );
}

#[test]
fn hint_card_fix_runs_existing_command() {
    let dir = tempfile::tempdir().unwrap();
    let ids = std::cell::Cell::new((0, 0));
    let (s, mut h) = window(&dir, 1400.0, |s| {
        let (a, b) = two_squares(s);
        // 0.3 mm apart: too close to cut cleanly.
        run(
            s,
            "object.move",
            json!({ "ids": [b], "x_mm": 70.3, "y_mm": 20.0 }),
        );
        run(s, "object.select", json!({ "ids": [a, b] }));
        ids.set((a, b));
    });
    let (a, b) = ids.get();
    let ignore = "Ignore for this object pair";
    the(&h, ignore);
    the(&h, "Move apart");
    // Ignore runs document.ignore_issue: one undo step, stored with the
    // document, and the hint is gone.
    let undo = undo_len(&s);
    h.get_by_label(ignore).click();
    h.run_plan();
    assert_eq!(undo_len(&s), undo + 1);
    let ignored = s.lock().unwrap().doc().document.ignored_pairs.clone();
    assert_eq!(ignored.len(), 1);
    assert_eq!(ignored[0].ids, [a.min(b), a.max(b)]);
    assert!(h.query_by_label("Move apart").is_none());
    assert!(painted_text(&h, object::NO_HINTS));
    // Undo brings it back; Move apart runs the fix's object.move.
    run(&s, "edit.undo", json!({}));
    h.run_plan();
    let before = s
        .lock()
        .unwrap()
        .doc()
        .document
        .get(flatcraft_core::document::ObjectId(b))
        .unwrap()
        .transform
        .center_mm;
    h.get_by_label("Move apart").click();
    h.run_plan();
    let after = s
        .lock()
        .unwrap()
        .doc()
        .document
        .get(flatcraft_core::document::ObjectId(b))
        .unwrap()
        .transform
        .center_mm;
    assert_ne!(before, after, "moved apart");
    assert!(h.query_by_label("Move apart").is_none());
}

/// Every setting of `settings` is in its object inspector section, at
/// most one fold away, can be scrolled into view in the inspector's
/// column, and shows its card on hover.
fn reachable_objects(h: &mut AppHarness, settings: &[(&str, &str)]) {
    let window = h.ctx.content_rect();
    for &(section, label) in settings {
        let title = object::SECTIONS
            .iter()
            .find(|(id, _)| *id == section)
            .unwrap()
            .1;
        let mut tries = 0;
        while in_inspector(h, label).is_empty() {
            tries += 1;
            assert!(tries < 30, "{label} is not reachable");
            let header = h.get_by_label(title);
            let expanded = header.accesskit_node().data().is_expanded();
            let at = egui::pos2(job_line(h).rect().center().x, window.height() * 0.55);
            if expanded == Some(false) {
                header.scroll_to_me();
                h.run_plan();
                h.get_by_label(title).click();
                h.run_plan();
            } else {
                wheel_down(h, at);
            }
        }
        in_inspector(h, label)[0].scroll_to_me();
        h.run_plan();
        let r = the(h, label);
        assert!(window.contains_rect(r), "{label} at {r:?} in {window:?}");
        // Its card shows on hover.
        let before = h.query_all_by_label(label).count();
        in_inspector(h, label)[0].hover();
        h.run_plan();
        assert!(
            h.query_all_by_label(label).count() > before,
            "{label} at {r:?} in {window:?} shows no card"
        );
        h.hover_at(egui::pos2(1.0, 1.0));
        h.run_plan();
    }
}

#[test]
fn every_object_setting_is_reachable() {
    for w in [900.0, 1920.0] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("logo.svg");
        std::fs::write(&file, common::SQUARE_SVG).unwrap();
        let (s, mut h) = window(&dir, w, |s| {
            s.lock().unwrap().prefs.panel_settings = PanelUser::None;
            let id = run(s, "object.import", json!({ "path": file }))["id"]
                .as_u64()
                .unwrap();
            run(s, "object.select", json!({ "ids": [id] }));
        });
        wait_for_workers(&mut h);
        open_right_panel(&mut h);
        // Every section header has its card, titled as it reads.
        for (_, name) in object::SECTIONS {
            let title = if name == object::ARRANGE_NAME {
                "Arrange"
            } else {
                name
            };
            h.get_by_label(name).scroll_to_me();
            h.run_plan();
            let before = h.query_all_by_label(title).count();
            h.get_by_label(name).hover();
            h.run_plan();
            assert!(
                h.query_all_by_label(title).count() > before,
                "{name} shows no card"
            );
            h.hover_at(egui::pos2(1.0, 1.0));
            h.run_plan();
        }
        reachable_objects(&mut h, &object::SETTINGS);
        // The Source actions of a linked file.
        let sn = snap(&mut h);
        let items = object::source_items(&sn, &app(&h).ui.prepare);
        assert!(items.len() >= 3, "{items:?}");
        let names: Vec<String> = items.iter().map(|i| i.text(&sn, &[])).collect();
        let source: Vec<(&str, &str)> =
            names.iter().map(|n| (object::SOURCE, n.as_str())).collect();
        reachable_objects(&mut h, &source);
        // The common path: speed and force, one click on the job line.
        job_line(&h).click();
        h.run_plan();
        reachable(&mut h, &[(job::VALUES, "Speed"), (job::VALUES, "Force")]);
        drop(s);
    }
}

#[test]
fn tab_switch_never_shows_other_document_values() {
    let dir = tempfile::tempdir().unwrap();
    let docs = std::cell::Cell::new((0, 0));
    let (_s, mut h) = window(&dir, 1400.0, |s| {
        // The first document: alpha and beta 0.3 mm apart, a hint, alpha
        // kept as a stencil.
        let (a, b) = two_squares(s);
        run(
            s,
            "object.move",
            json!({ "ids": [b], "x_mm": 70.3, "y_mm": 20.0 }),
        );
        run(
            s,
            "object.set_keep",
            json!({ "ids": [a], "keep": "stencil" }),
        );
        run(s, "object.select", json!({ "ids": [a, b] }));
        let first = s.lock().unwrap().doc().id();
        // The second: one square, "gamma", alone.
        run(s, "document.new", json!({}));
        let c = import_square(s);
        rename(s, c, "gamma");
        run(s, "object.select", json!({ "ids": [c] }));
        let second = s.lock().unwrap().doc().id();
        docs.set((first, second));
    });
    let (first, second) = docs.get();
    let ctx = h.ctx.clone();
    for _ in 0..2 {
        app_mut(&mut h).apply(&ctx, vec![flatcraft::ui::Action::Switch(first)]);
        h.step();
        the(&h, "2 objects");
        assert_eq!(value(&h, "Keep"), "Mixed");
        the(&h, "Move apart");
        app_mut(&mut h).apply(&ctx, vec![flatcraft::ui::Action::Switch(second)]);
        h.step();
        assert!(in_inspector(&h, "2 objects").is_empty());
        the(&h, "gamma");
        assert_eq!(value(&h, "Keep"), "Pieces");
        assert!(
            h.query_by_label("Move apart").is_none(),
            "no hint of the other document"
        );
    }
}

#[test]
fn compare_link_opens_the_strategy_comparison() {
    let dir = tempfile::tempdir().unwrap();
    let (_s, mut h) = window(&dir, 1400.0, |_| {});
    h.get_by_label(job::COMPARE).click();
    h.run_plan();
    assert_eq!(app(&h).ui.workspace, flatcraft::ui::Workspace::Cut);
    assert_eq!(app(&h).ui.cut.stage, flatcraft::ui::cut::Stage::Simulate);
}

#[test]
fn position_fields_in_the_inspector_apply_like_the_ribbon() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = window(&dir, 1400.0, |s| {
        let id = selection_all(s)[0];
        run(s, "object.select", json!({ "ids": [id] }));
        s.lock()
            .unwrap()
            .prefs
            .panel_sections
            .insert(object::POSITION.into(), true);
    });
    let id = selection_all(&s)[0];
    let bounds = |h: &mut AppHarness| snap(h).box_of(&[id]).unwrap();
    assert_eq!(value(&h, "X"), "20.00");
    let undo = undo_len(&s);
    // A calculation, applied when the field loses the focus by Enter.
    in_inspector(&h, "X")[0].click();
    h.run_plan();
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.run_plan();
    h.event(egui::Event::Text("20+15".into()));
    h.run_plan();
    // The ribbon's X shows what is typed while the inspector edits it.
    assert_eq!(app(&h).ui.prepare.fields.x, "20+15");
    h.key_press(Key::Enter);
    h.run_plan();
    let b = bounds(&mut h);
    assert!((b.x0 - 35.0).abs() < 1e-6, "{b:?}");
    assert_eq!(undo_len(&s), undo + 1, "one step to undo");
    assert_eq!(value(&h, "X"), "35.00");
    // The ribbon's field shows it too.
    assert_eq!(field_text(&h, "X (mm)"), "35.00");
    // Esc puts the shown value back.
    in_inspector(&h, "Width")[0].click();
    h.run_plan();
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.run_plan();
    h.event(egui::Event::Text("99".into()));
    h.run_plan();
    h.key_press(Key::Escape);
    h.run_plan();
    assert_eq!(undo_len(&s), undo + 1);
    assert_eq!(value(&h, "Width"), "50.00");
    // The ribbon's View tab does not end an edit in the inspector.
    in_inspector(&h, "Y")[0].click();
    h.run_plan();
    app_mut(&mut h).ui.prepare.tab = prepare::RibbonTab::View;
    h.run_plan();
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.run_plan();
    h.event(egui::Event::Text("40".into()));
    h.run_plan();
    h.key_press(Key::Tab);
    h.run_plan();
    let b = bounds(&mut h);
    assert!((b.y0 - 40.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn every_object_section_has_a_plain_card() {
    for text in [
        object::CUT_CARD,
        object::POSITION_CARD,
        object::ARRANGE_CARD,
        object::SOURCE_CARD,
        object::HINTS_CARD,
        object::HEADING_CARD,
        object::STROKES_ALL_LINES,
        object::NO_HINTS,
        &object::on_the_card(1),
        &object::on_the_card(2),
    ] {
        assert!(!text.is_empty());
        assert!(!text.contains('\u{2014}'), "{text}");
        assert!(text.ends_with('.'), "{text}");
    }
}
