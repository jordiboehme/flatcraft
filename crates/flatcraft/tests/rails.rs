//! Collapsible side panels with summary rails: collapse and open, the
//! summaries, the state dot, the automatic collapse by window width, what
//! the person chose by hand, and the preferences round trip.

mod common;

use common::*;
use egui::epaint::TextShape;
use egui::{Rect, Shape, vec2};
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::rails::{self, Dot, SidePanel};
use flatcraft::ui::{Workspace, prepare};
use flatcraft_engine::prefs::{PanelUser, Preferences};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

fn run(s: &SharedSession, cmd: &str, params: Value) {
    s.lock()
        .unwrap()
        .execute(cmd, params, Origin::User)
        .unwrap();
}

/// Four squares: ids 0..3. The first is selected.
fn four(width: f32) -> (tempfile::TempDir, SharedSession, AppHarness, Vec<u64>) {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let ids: Vec<u64> = (0..4).map(|_| import_square(&s)).collect();
    run(&s, "object.select", json!({ "ids": [ids[0]] }));
    let mut h = harness_sized(&dir, s.clone(), [width, 900.0], 8, |a| a);
    to_prepare(&mut h);
    h.run();
    (dir, s, h, ids)
}

fn snap(h: &mut AppHarness) -> prepare::Snapshot {
    let ctx = h.ctx.clone();
    prepare::snapshot(app_mut(h), &ctx)
}

fn resize(h: &mut AppHarness, width: f32) {
    h.set_size(vec2(width, 900.0));
    h.run();
}

fn has(h: &AppHarness, label: &str) -> bool {
    h.query_by_label_contains(label).is_some()
}

fn rail_open(h: &AppHarness, panel: &str) -> bool {
    has(h, &format!("Open the {panel} panel"))
}

fn collapse_button(h: &AppHarness, panel: &str) -> bool {
    has(h, &format!("Collapse the {panel} panel"))
}

#[test]
fn each_panel_collapses_and_opens() {
    let (_d, _s, mut h, _) = four(1400.0);
    for panel in ["Objects", "Cut and Job"] {
        assert!(collapse_button(&h, panel), "{panel} is open at 1400 pt");
        h.get_by_label(&format!("Collapse the {panel} panel"))
            .click();
        h.run();
        assert!(rail_open(&h, panel), "{panel} is a rail now");
        assert!(!collapse_button(&h, panel));
        h.get_by_label_contains(&format!("Open the {panel} panel"))
            .click();
        h.run();
        assert!(collapse_button(&h, panel), "{panel} is open again");
    }
    // The Cut workspace's stage panel.
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run();
    assert!(collapse_button(&h, "Cut stage"));
    h.get_by_label("Collapse the Cut stage panel").click();
    h.run();
    assert!(rail_open(&h, "Cut stage"));
    h.get_by_label_contains("Open the Cut stage panel").click();
    h.run();
    assert!(collapse_button(&h, "Cut stage"));
}

#[test]
fn objects_summary_counts_only_what_is_not_zero() {
    let (_d, s, mut h, ids) = four(1400.0);
    let sum = |h: &mut AppHarness| rails::objects_summary(&snap(h)).parts;
    assert_eq!(sum(&mut h), ["4 objects", "1 selected"]);
    run(&s, "object.select", json!({ "ids": [] }));
    h.run();
    assert_eq!(sum(&mut h), ["4 objects"]);
    run(
        &s,
        "object.set_cut",
        json!({ "ids": [ids[1]], "cut": false }),
    );
    run(
        &s,
        "object.set_visible",
        json!({ "ids": [ids[2]], "visible": false }),
    );
    run(&s, "object.select", json!({ "ids": [ids[0], ids[3]] }));
    h.run();
    assert_eq!(
        sum(&mut h),
        ["4 objects", "2 selected", "1 skipped", "1 hidden"]
    );
    let full = rails::objects_summary(&snap(&mut h)).full();
    assert_eq!(
        full,
        "4 objects \u{b7} 2 selected \u{b7} 1 skipped \u{b7} 1 hidden"
    );
}

#[test]
fn one_object_is_not_plural() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s, false);
    to_prepare(&mut h);
    assert_eq!(rails::objects_summary(&snap(&mut h)).parts[0], "1 object");
}

#[test]
fn settings_summary_names_the_selection_and_the_job() {
    let (_d, s, mut h, ids) = four(1400.0);
    let parts = |h: &mut AppHarness| rails::settings_summary(&snap(h), Dot::Ready).parts;
    run(
        &s,
        "job.set_overrides",
        json!({ "speed": 4, "force": 12, "passes": 2 }),
    );
    h.run();
    let p = parts(&mut h);
    assert_eq!(p[0], "1 selected: Object outlines");
    assert_eq!(&p[3..], ["Speed 4", "Force 12", "2 passes"]);
    // Same mode on two objects, then mixed.
    run(&s, "object.select", json!({ "ids": [ids[0], ids[1]] }));
    h.run();
    assert_eq!(parts(&mut h)[0], "2 selected: Object outlines");
    run(
        &s,
        "object.set_cut_options",
        json!({ "ids": [ids[1]], "mode": "all_lines" }),
    );
    h.run();
    assert_eq!(parts(&mut h)[0], "2 selected: mixed");
    // No selection: the job only.
    run(&s, "object.select", json!({ "ids": [] }));
    h.run();
    let p = parts(&mut h);
    assert!(!p[0].contains("selected"), "{p:?}");
    assert_eq!(&p[2..], ["Speed 4", "Force 12", "2 passes"]);
    run(&s, "job.set_overrides", json!({ "passes": 1 }));
    h.run();
    assert_eq!(parts(&mut h).last().unwrap(), "1 pass");
}

#[test]
fn stage_summary_names_the_stage() {
    let s = rails::stage_summary(flatcraft::ui::cut::Stage::Path, Dot::Ready);
    assert_eq!(s.parts, ["Stage 2 of 4", "Path"]);
    assert_eq!(s.full(), "Stage 2 of 4 \u{b7} Path \u{b7} Ready to cut");
}

fn both_dots(h: &mut AppHarness) -> (Dot, Dot) {
    let sn = snap(h);
    let prepare = rails::job_dot(&sn, None);
    let ws = app(h).ui.workspace;
    app_mut(h).ui.workspace = Workspace::Cut;
    h.run();
    let cut = rails::job_dot(&sn, Some(&app(h).ui.cut));
    app_mut(h).ui.workspace = ws;
    h.run();
    (prepare, cut)
}

#[test]
fn the_dot_says_ready_warning_or_nothing_to_cut() {
    let (_d, s, mut h, ids) = four(1400.0);
    assert_eq!(both_dots(&mut h), (Dot::Ready, Dot::Ready));
    // Hiding or skipping an object is the person's choice: still ready.
    run(
        &s,
        "object.set_visible",
        json!({ "ids": [ids[0]], "visible": false }),
    );
    run(
        &s,
        "object.set_cut",
        json!({ "ids": [ids[1]], "cut": false }),
    );
    h.run();
    assert_eq!(both_dots(&mut h), (Dot::Ready, Dot::Ready));
    run(
        &s,
        "object.set_visible",
        json!({ "ids": [ids[0]], "visible": true }),
    );
    run(
        &s,
        "object.set_cut",
        json!({ "ids": [ids[1]], "cut": true }),
    );
    // One object sticks out of the cutting area: a real problem.
    run(
        &s,
        "object.move",
        json!({ "ids": [ids[3]], "dx_mm": 5000.0 }),
    );
    h.run();
    assert_eq!(both_dots(&mut h), (Dot::Warning, Dot::Warning));
    // Skipping that object solves it.
    run(
        &s,
        "object.set_cut",
        json!({ "ids": [ids[3]], "cut": false }),
    );
    h.run();
    assert_eq!(both_dots(&mut h), (Dot::Ready, Dot::Ready));
    // Nothing left to cut.
    run(
        &s,
        "object.set_visible",
        json!({ "ids": [ids[0], ids[1], ids[2]], "visible": false }),
    );
    h.run();
    assert_eq!(both_dots(&mut h), (Dot::Nothing, Dot::Nothing));
}

#[test]
fn the_job_tells_choices_from_problems_and_nothing_to_cut() {
    let (_d, s, mut h, ids) = four(1400.0);
    run(
        &s,
        "object.set_visible",
        json!({ "ids": [ids[0]], "visible": false }),
    );
    h.run();
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run();
    let job = app(&h).ui.cut.job().unwrap();
    assert_eq!(job.warnings.len(), 1);
    assert_eq!(job.choice_warnings, 1);
    assert_eq!(job.problem_warnings(), 0);
    run(
        &s,
        "object.set_visible",
        json!({ "ids": [ids[1], ids[2], ids[3]], "visible": false }),
    );
    h.run();
    assert!(app(&h).ui.cut.plan.as_ref().unwrap().is_err());
    assert!(app(&h).ui.cut.nothing_to_cut);
    run(
        &s,
        "object.set_visible",
        json!({ "ids": [ids[1]], "visible": true }),
    );
    h.run();
    assert!(!app(&h).ui.cut.nothing_to_cut);
}

#[test]
fn small_windows_collapse_the_panels_on_their_own() {
    let (_d, _s, mut h, _) = four(1000.0);
    assert!(rail_open(&h, "Cut and Job"), "right: below 1100 pt");
    assert!(
        collapse_button(&h, "Objects"),
        "left: 1000 pt is above 960 pt"
    );
    resize(&mut h, 1050.0);
    assert!(rail_open(&h, "Cut and Job"));
    assert!(collapse_button(&h, "Objects"));
    resize(&mut h, 900.0);
    assert!(rail_open(&h, "Objects"), "left: below 960 pt");
    resize(&mut h, 1400.0);
    assert!(collapse_button(&h, "Cut and Job"), "they open again");
    assert!(collapse_button(&h, "Objects"));
}

#[test]
fn the_cut_stage_panel_follows_the_right_threshold() {
    let (_d, _s, mut h, _) = four(1000.0);
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run();
    assert!(rail_open(&h, "Cut stage"));
    resize(&mut h, 1400.0);
    assert!(collapse_button(&h, "Cut stage"));
}

#[test]
fn a_hand_set_state_wins_over_the_width() {
    let (_d, s, mut h, _) = four(1000.0);
    let prefs = |s: &SharedSession| s.lock().unwrap().prefs.clone();
    // Opened by hand in a narrow window: stays open at 900 and 1400 pt.
    h.get_by_label_contains("Open the Cut and Job panel")
        .click();
    h.run();
    assert_eq!(prefs(&s).panel_settings, PanelUser::Open);
    resize(&mut h, 900.0);
    assert!(collapse_button(&h, "Cut and Job"));
    resize(&mut h, 1400.0);
    assert!(collapse_button(&h, "Cut and Job"));
    // Collapsed by hand in a wide window: stays collapsed.
    h.get_by_label("Collapse the Objects panel").click();
    h.run();
    resize(&mut h, 1600.0);
    assert!(rail_open(&h, "Objects"));
    assert_eq!(prefs(&s).panel_objects, PanelUser::Collapsed);
}

#[test]
fn a_choice_the_width_would_make_anyway_follows_the_width_again() {
    let (_d, s, mut h, _) = four(1400.0);
    let prefs = |s: &SharedSession| s.lock().unwrap().prefs.clone();
    // Wide window: collapse, then open again: no choice is left.
    h.get_by_label("Collapse the Objects panel").click();
    h.run();
    assert_eq!(prefs(&s).panel_objects, PanelUser::Collapsed);
    h.get_by_label_contains("Open the Objects panel").click();
    h.run();
    assert_eq!(prefs(&s).panel_objects, PanelUser::None);
    resize(&mut h, 900.0);
    assert!(rail_open(&h, "Objects"), "it follows the width again");
    // Narrow window: open by hand, then collapse again: no choice either.
    h.get_by_label_contains("Open the Objects panel").click();
    h.run();
    assert_eq!(prefs(&s).panel_objects, PanelUser::Open);
    h.get_by_label("Collapse the Objects panel").click();
    h.run();
    assert_eq!(prefs(&s).panel_objects, PanelUser::None);
    resize(&mut h, 1400.0);
    assert!(collapse_button(&h, "Objects"));
    // The ribbon toggle does the same.
    let st = &mut app_mut(&mut h).ui.prepare.panels;
    st.toggle(SidePanel::Settings);
    st.toggle(SidePanel::Settings);
    assert_eq!(st.user(SidePanel::Settings), PanelUser::None);
}

#[test]
fn the_ribbon_toggles_act_on_the_same_state() {
    let (_d, _s, mut h, _) = four(1400.0);
    let toggle = |h: &mut AppHarness, id: &str| {
        let sn = snap(h);
        let tool = flatcraft::ui::tools::TOOLS
            .iter()
            .find(|t| t.id == id)
            .unwrap();
        let (mut dbg, mut actions) = (false, Vec::new());
        tool.run(&sn, &mut app_mut(h).ui.prepare, &mut dbg, &mut actions);
        let checked = tool.selected(&app(h).ui.prepare, false);
        h.run();
        checked
    };
    assert_eq!(toggle(&mut h, "panel.settings"), Some(false));
    assert!(rail_open(&h, "Cut and Job"));
    assert_eq!(toggle(&mut h, "panel.objects"), Some(false));
    assert!(rail_open(&h, "Objects"));
    // The rail's choice shows in the toggle too.
    h.get_by_label_contains("Open the Objects panel").click();
    h.run();
    assert!(collapse_button(&h, "Objects"));
    let tool = flatcraft::ui::tools::TOOLS
        .iter()
        .find(|t| t.id == "panel.objects")
        .unwrap();
    assert_eq!(tool.selected(&app(&h).ui.prepare, false), Some(true));
    // Narrow window: the toggle shows what is on screen, and a press opens.
    resize(&mut h, 900.0);
    app_mut(&mut h)
        .ui
        .prepare
        .panels
        .set_collapsed(SidePanel::Settings, false);
    app_mut(&mut h)
        .ui
        .prepare
        .panels
        .toggle(SidePanel::Settings);
    h.run();
    assert!(rail_open(&h, "Cut and Job"));
}

#[test]
fn the_choices_round_trip_in_the_preferences() {
    let (dir, s, mut h, _) = four(1400.0);
    h.get_by_label("Collapse the Cut and Job panel").click();
    h.run();
    let stored = Preferences::read(dir.path()).unwrap();
    assert_eq!(stored.panel_settings, PanelUser::Collapsed);
    assert_eq!(stored.panel_objects, PanelUser::None);
    // A new window starts as it was.
    let panels = rails::Panels::from_prefs(&stored);
    assert!(panels.collapsed(SidePanel::Settings));
    assert!(!panels.collapsed(SidePanel::Objects));
    drop(h);
    let h2 = harness(&dir, s, false);
    assert!(app(&h2).ui.prepare.panels.collapsed(SidePanel::Settings));
    // Older files without the keys read as "none".
    let p: Preferences = toml::from_str("theme = \"Dark\"").unwrap();
    assert_eq!(p.panel_objects, PanelUser::None);
}

#[test]
fn an_open_arrange_panel_makes_the_settings_panel_give_way() {
    use rails::auto_collapsed as auto;
    assert!(!auto(SidePanel::Settings, 1200.0, false));
    assert!(auto(SidePanel::Settings, 1200.0, true));
    assert!(!auto(SidePanel::Settings, 1400.0, true));
    assert!(auto(SidePanel::Settings, 900.0, true));
    // The others do not care.
    assert!(!auto(SidePanel::Objects, 1000.0, true));
    assert!(!auto(SidePanel::Stage, 1100.0, true));
}

fn rotated(h: &AppHarness) -> Vec<TextShape> {
    painted(h)
        .into_iter()
        .filter_map(|s| match s {
            Shape::Text(t) if t.angle.abs() > 1.0 => Some(t),
            _ => None,
        })
        .collect()
}

/// The text of one rail stays inside the rail's own layout: below the
/// arrow and the dot, above the bottom padding, inside its width.
fn assert_text_inside(h: &AppHarness, panel: &str, text: &TextShape, dot: bool) -> Rect {
    let rail = h
        .get_by_label_contains(&format!("Open the {panel} panel"))
        .rect();
    let lay = rails::layout(rail, dot);
    let b = text.visual_bounding_rect();
    let eps = 0.5;
    assert!(
        b.top() >= lay.arrow.bottom() - eps,
        "{b:?} under {:?}",
        lay.arrow
    );
    assert!(
        b.top() >= lay.text_top - eps,
        "{b:?} starts at {}",
        lay.text_top
    );
    if let Some(d) = lay.dot {
        assert!(b.top() > d.y + 4.5, "{b:?} clear of the dot at {d:?}");
    }
    assert!(
        b.bottom() <= lay.text_top + lay.text_len + eps,
        "{b:?} ends at {}",
        lay.text_top + lay.text_len
    );
    assert!(b.left() >= rail.left() - eps && b.right() <= rail.right() + eps);
    // Right under the arrow: the text block starts at the top.
    assert!(
        b.top() <= lay.text_top + eps,
        "{b:?} starts right under the arrow"
    );
    b
}

#[test]
fn rail_text_is_turned_and_stays_inside_the_rail() {
    let (_d, _s, h, _) = four(900.0);
    let shapes = rotated(&h);
    assert_eq!(shapes.len(), 2, "one text per rail");
    let left = shapes
        .iter()
        .find(|t| t.angle < 0.0)
        .expect("left rail reads bottom to top");
    let right = shapes
        .iter()
        .find(|t| t.angle > 0.0)
        .expect("right rail reads top to bottom");
    assert!(left.galley.text().starts_with("4 objects"));
    assert!(right.galley.text().starts_with("1 selected"));
    assert!(!left.galley.text().ends_with("..."), "it fits whole");
    assert_text_inside(&h, "Objects", left, false);
    assert_text_inside(&h, "Cut and Job", right, true);
}

#[test]
fn a_long_summary_ends_in_dots_and_never_runs_past_the_rail() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    for _ in 0..3 {
        import_square(&s);
    }
    run(&s, "object.select", json!({ "ids": [] }));
    // A short window: the summaries do not fit and are cut.
    let mut h = harness_sized(&dir, s, [900.0, 260.0], 8, |a| a);
    to_prepare(&mut h);
    h.run();
    let shapes = rotated(&h);
    assert_eq!(shapes.len(), 2);
    let right = shapes.iter().find(|t| t.angle > 0.0).unwrap();
    let text = right.galley.text();
    assert!(text.ends_with("..."), "{text}");
    let b = assert_text_inside(&h, "Cut and Job", right, true);
    let lay = rails::layout(
        h.get_by_label_contains("Open the Cut and Job panel").rect(),
        true,
    );
    // It uses the room it has, to within one character.
    assert!(
        b.height() > lay.text_len - 12.0,
        "{b:?} of {}",
        lay.text_len
    );
    let left = shapes.iter().find(|t| t.angle < 0.0).unwrap();
    assert_text_inside(&h, "Objects", left, false);
    // The tooltip and the name still hold everything.
    assert!(has(&h, "Speed"));
}

#[test]
fn an_open_arrange_panel_keeps_the_canvas_usable_at_900() {
    let (_d, _s, mut h, _) = four(900.0);
    let sn = snap(&mut h);
    flatcraft::ui::arrange::open(
        &mut app_mut(&mut h).ui.prepare,
        &sn,
        flatcraft::ui::arrange::PanelKind::MoveCopy,
    );
    h.run();
    assert!(app(&h).ui.prepare.panel.is_some());
    assert!(rail_open(&h, "Objects") && rail_open(&h, "Cut and Job"));
    let w = app(&h).ui.prepare.canvas_rect.width();
    assert!(w > 450.0, "the canvas is {w} pt wide");
    // At 1200 pt the Cut and Job panel gives way to the Arrange panel too.
    resize(&mut h, 1200.0);
    assert!(rail_open(&h, "Cut and Job"));
    assert!(app(&h).ui.prepare.canvas_rect.width() > 450.0);
}

#[test]
fn the_ribbon_toggle_knows_about_an_open_arrange_panel_in_the_same_frame() {
    let (_d, _s, mut h, _) = four(1200.0);
    let tool = flatcraft::ui::tools::TOOLS
        .iter()
        .find(|t| t.id == "panel.settings")
        .unwrap();
    assert_eq!(tool.selected(&app(&h).ui.prepare, false), Some(true));
    let sn = snap(&mut h);
    flatcraft::ui::arrange::open(
        &mut app_mut(&mut h).ui.prepare,
        &sn,
        flatcraft::ui::arrange::PanelKind::MoveCopy,
    );
    h.step();
    // The frame has set arrange_open before the ribbon asked.
    assert!(app(&h).ui.prepare.panels.arrange_open);
    assert_eq!(tool.selected(&app(&h).ui.prepare, false), Some(false));
}
