//! Side panels keep their width while the pointer moves: content that
//! asks for a little more room than the panel has must never make a
//! resizable panel grow frame by frame.

mod common;
use common::RunPlan;

use common::*;
use egui::accesskit::Role;
use egui::{Event, Pos2, Rect};
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::Workspace;
use flatcraft::ui::cut::Stage;
use flatcraft_engine::session::Origin;
use serde_json::json;

/// The canvas of the workspace that shows: the side panels sit left and
/// right of it, so its edges move when a panel changes its width.
fn canvas(h: &AppHarness) -> Rect {
    let a = app(h);
    match a.ui.workspace {
        Workspace::Cut => a.ui.cut.canvas_rect,
        _ => a.ui.prepare.canvas_rect,
    }
}

/// Moves the pointer across the window, over the panels and the canvas,
/// for 120 frames, and checks that the side panels keep their width (the
/// canvas keeps its left and right edges).
fn sweep(h: &mut AppHarness, what: &str) {
    let first = canvas(h);
    let w = h.ctx.content_rect().width();
    for i in 0..120 {
        // Every other move over the right panels, where the bug was.
        let x = if i % 2 == 0 {
            w - 300.0 + (i as f32 * 37.0) % 290.0
        } else {
            20.0 + (i as f32 * 97.0) % (w - 40.0)
        };
        let y = 160.0 + (i as f32 * 53.0) % 700.0;
        h.event(Event::PointerMoved(Pos2::new(x, y)));
        h.step();
        let now = canvas(h);
        assert!(
            (now.left() - first.left()).abs() <= 0.5 && (now.right() - first.right()).abs() <= 0.5,
            "{what}: the canvas went from {first:?} to {now:?} after {i} frames"
        );
    }
    h.run_plan();
}

/// Two squares, the first selected, every panel section open (the panels
/// are taller than the window, so they scroll), at `ppp` pixels per point.
fn window(ppp: f32) -> (tempfile::TempDir, AppHarness) {
    window_with(ppp, None)
}

/// [`window`]; with `knife`, the job's plotter is a File Output preset
/// with that knife compensation (the host one is simulated, the firmware
/// one is not).
fn window_with(ppp: f32, knife: Option<&str>) -> (tempfile::TempDir, AppHarness) {
    window_sized(ppp, knife, 1400.0)
}

/// [`window_with`] `width` points wide. Below 1400 pt every side panel is
/// held open, so the panels share the narrow window.
fn window_sized(ppp: f32, knife: Option<&str>, width: f32) -> (tempfile::TempDir, AppHarness) {
    let dir = tempfile::tempdir().unwrap();
    if let Some(knife) = knife {
        let plotters = dir.path().join("profiles").join("plotters");
        std::fs::create_dir_all(&plotters).unwrap();
        std::fs::write(
            plotters.join("host.toml"),
            format!(
                "id = \"host\"\nname = \"host\"\ninherits = \"file-output-gpgl-cameo2\"\nknife_compensation = {knife}\nblade_offset_mm = 0.3\n"
            ),
        )
        .unwrap();
    }
    let s = session(&dir);
    if knife.is_some() {
        s.lock()
            .unwrap()
            .execute("job.set_plotter", json!({ "id": "host" }), Origin::User)
            .unwrap();
    }
    {
        use flatcraft::ui::inspector::{job, object};
        let mut st = s.lock().unwrap();
        for id in [
            job::MACHINE,
            job::VALUES,
            job::ORDER,
            job::WEEDING,
            job::TIPS,
            job::ADVANCED,
            object::CUT,
            object::POSITION,
            object::ARRANGE,
            object::SOURCE,
            object::HINTS,
        ] {
            st.prefs.panel_sections.insert(id.into(), true);
        }
        if width < 1400.0 {
            use flatcraft_engine::prefs::PanelUser;
            st.prefs.panel_objects = PanelUser::Open;
            st.prefs.panel_settings = PanelUser::Open;
            st.prefs.panel_stage = PanelUser::Open;
        }
    }
    import_square(&s);
    import_square(&s);
    s.lock()
        .unwrap()
        .execute("object.select", json!({ "ids": [1] }), Origin::User)
        .unwrap();
    let mut h = harness_sized(&dir, s, [width, 900.0], 8, |a| a);
    h.set_pixels_per_point(ppp);
    h.set_size(egui::vec2(width, 900.0));
    h.run_plan();
    (dir, h)
}

#[test]
fn the_cut_stage_panel_keeps_its_width_while_the_pointer_moves() {
    for ppp in [1.0, 2.0] {
        let (_d, mut h) = window(ppp);
        app_mut(&mut h).ui.workspace = Workspace::Cut;
        wait_for_workers(&mut h);
        for stage in Stage::ALL {
            app_mut(&mut h).ui.cut.stage = stage;
            wait_for_workers(&mut h);
            sweep(&mut h, &format!("{} at {ppp}x", stage.name()));
        }
    }
}

#[test]
fn the_prepare_panels_keep_their_width_while_the_pointer_moves() {
    for ppp in [1.0, 2.0] {
        prepare_panels(ppp);
    }
}

#[test]
fn the_panels_share_a_900_pt_window_without_growing() {
    // Objects, Cut and Job and the docked Arrange panel, all open.
    prepare_panels_at(1.0, 900.0);
    // The Cut stage panel.
    let (_d, mut h) = window_sized(1.0, None, 900.0);
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    wait_for_workers(&mut h);
    for stage in Stage::ALL {
        app_mut(&mut h).ui.cut.stage = stage;
        wait_for_workers(&mut h);
        sweep(&mut h, &format!("{} at 900 pt", stage.name()));
    }
}

fn prepare_panels(ppp: f32) {
    prepare_panels_at(ppp, 1400.0);
}

fn prepare_panels_at(ppp: f32, width: f32) {
    let (_d, mut h) = window_sized(ppp, None, width);
    to_prepare(&mut h);
    wait_for_workers(&mut h);
    sweep(&mut h, "Prepare");
    // With the docked Arrange panel open.
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.run_plan();
    let tool = flatcraft::ui::tools::TOOLS
        .iter()
        .find(|t| t.title == "Rotate...")
        .unwrap();
    h.get_by_role_and_label(Role::Button, tool.group.def().title)
        .click();
    h.run_plan();
    h.get_by_label(&tool.row_name()).click();
    h.run_plan();
    assert!(app(&h).ui.prepare.panel.is_some());
    // Held open below 1400 pt: the panels share the window.
    if width < 1400.0 {
        for panel in ["Objects", "Cut and Job"] {
            let name = format!("Collapse the {panel} panel");
            assert!(h.query_by_label(&name).is_some(), "{panel} is open");
        }
    }
    sweep(&mut h, "Arrange");
}

/// Moves the pointer over the stage panel and the canvas for 400 frames,
/// as a person does for a while before reaching for the bar. (A panel
/// that grew 2 pt per frame took the whole window by then.)
fn wander(h: &mut AppHarness) {
    for i in 0..400 {
        let x = 600.0 + (i as f32 * 37.0) % 790.0;
        let y = 160.0 + (i as f32 * 53.0) % 600.0;
        h.event(Event::PointerMoved(Pos2::new(x, y)));
        h.step();
    }
    h.run_plan();
}

/// A click at `at` in two frames, without waiting for the window to
/// settle after it: playback keeps asking for frames until it ends, so a
/// wait would only return once it stopped.
fn press(h: &mut AppHarness, at: Pos2) {
    h.drag_at(at);
    h.step();
    h.drop_at(at);
    h.step();
}

/// The rect of the control `name` in the bottom bar, which must lie
/// under the canvas, never under the stage panel (that would take its
/// clicks), and leave the canvas most of the window.
fn bar_control(h: &AppHarness, name: &str) -> Rect {
    let r = h.get_by_label(name).rect();
    let c = app(h).ui.cut.canvas_rect;
    assert!(c.width() > 600.0, "the canvas keeps its room: {c:?}");
    assert!(
        r.width() > 4.0 && r.left() >= c.left() && r.right() <= c.right() && r.top() >= c.bottom(),
        "{name} at {r:?}, canvas {c:?}"
    );
    r
}

#[test]
fn the_playback_bar_takes_clicks_after_the_pointer_moved() {
    let host = "{ Host = { offset_mm = 0.3, pivot_threshold_deg = 0.0 } }";
    let (_d, mut h) = window_with(1.0, Some(host));
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    wait_for_workers(&mut h);
    // A while on the first step, where the panel once grew.
    wander(&mut h);
    // Simulate: the run lands and its numbers show.
    app_mut(&mut h).ui.cut.stage = Stage::Simulate;
    wait_for_workers(&mut h);
    wander(&mut h);
    assert!(
        h.query_by_label("Time (estimate)").is_some(),
        "the simulation landed"
    );
    let total = app(&h).ui.cut.sim.clock.total_mm;
    assert!(total > 0.0);
    // The time slider takes a click: the head goes there.
    let slider = bar_control(&h, "Time");
    click_at(&mut h, slider.left_center() + egui::vec2(8.0, 0.0));
    let at = app(&h).ui.cut.sim.clock.t_mm;
    assert!(at < total, "the click moved the time: {at} of {total}");
    // A speed button and Play take clicks; playback moves on.
    let p = bar_control(&h, "Speed 4x").center();
    click_at(&mut h, p);
    assert_eq!(app(&h).ui.cut.sim.clock.speed, 4.0);
    let p = bar_control(&h, "Play").center();
    press(&mut h, p);
    assert!(
        app(&h).ui.cut.sim.clock.playing.is_some(),
        "Play took the click"
    );
    let first = app(&h).ui.cut.sim.clock.t_mm;
    for _ in 0..3 {
        h.step();
    }
    assert!(app(&h).ui.cut.sim.clock.t_mm > first, "playback moves on");
    let p = bar_control(&h, "Stop").center();
    press(&mut h, p);
    assert!(
        app(&h).ui.cut.sim.clock.playing.is_none(),
        "Stop took the click"
    );
    h.run_plan();
    // Path: the same bar for the toolpath.
    app_mut(&mut h).ui.cut.stage = Stage::Path;
    wait_for_workers(&mut h);
    wander(&mut h);
    let p = bar_control(&h, "Play").center();
    press(&mut h, p);
    assert!(
        app(&h).ui.cut.timeline.playing.is_some(),
        "Play took the click"
    );
    let p = bar_control(&h, "Stop").center();
    press(&mut h, p);
    assert!(app(&h).ui.cut.timeline.playing.is_none());
    h.run_plan();
    let slider = bar_control(&h, "Toolpath");
    click_at(&mut h, slider.left_center() + egui::vec2(8.0, 0.0));
    let tl = &app(&h).ui.cut.timeline;
    assert!(tl.visible() < tl.total, "{} of {}", tl.visible(), tl.total);
}
