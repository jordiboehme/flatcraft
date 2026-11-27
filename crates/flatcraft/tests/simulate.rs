//! The Simulate stage of the Cut workspace, driven through egui_kittest.

mod common;
use common::RunPlan;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{AppHarness, app, app_mut, harness, painted, session};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::Workspace;
use flatcraft::ui::cut::{Stage, stage_status};
use flatcraft::ui::simulate::{self as sim, quality_color, tip_colour};
use flatcraft::ui::theme::Tokens;
use flatcraft_engine::session::{Origin, UnlockedHook};
use flatcraft_mcp::SharedSession;
use flatcraft_sim::END_TO_END_LIMIT_MM;
use serde_json::{Value, json};

fn run(s: &SharedSession, cmd: &str, params: Value) -> Value {
    s.lock()
        .unwrap()
        .execute(cmd, params, Origin::User)
        .unwrap()
}

/// A session with a plotter preset `id` that inherits File Output with
/// the knife compensation `knife` and a blade offset of 0.3 mm, picked
/// for the job.
fn plotter_session(dir: &tempfile::TempDir, id: &str, knife: &str) -> SharedSession {
    let plotters = dir.path().join("profiles").join("plotters");
    std::fs::create_dir_all(&plotters).unwrap();
    std::fs::write(
        plotters.join(format!("{id}.toml")),
        format!(
            "id = \"{id}\"\nname = \"{id}\"\ninherits = \"file-output-gpgl-cameo2\"\nknife_compensation = {knife}\nblade_offset_mm = 0.3\n"
        ),
    )
    .unwrap();
    let s = session(dir);
    run(&s, "job.set_plotter", json!({ "id": id }));
    s
}

const HOST: &str = "{ Host = { offset_mm = 0.3, pivot_threshold_deg = 0.0 } }";
const PEN: &str = "\"None\"";

fn import(s: &SharedSession, svg: &str) {
    run(
        s,
        "object.import_svg",
        json!({ "svg": svg, "name": "shape" }),
    );
}

fn to_simulate(h: &mut AppHarness) {
    app_mut(h).ui.workspace = Workspace::Cut;
    h.run_plan();
    h.get_by_role_and_label(Role::Tab, "Simulate").click();
    h.run_plan();
    assert_eq!(app(h).ui.cut.stage, Stage::Simulate);
}

fn simulate_status(h: &mut AppHarness) -> String {
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(h), &ctx);
    stage_status(Stage::Simulate, &app(h).ui.cut, &snap)
}

fn painted_texts(h: &AppHarness) -> Vec<String> {
    painted(h)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn simulate_stage_runs_and_shows_grade() {
    let dir = tempfile::tempdir().unwrap();
    let s = plotter_session(&dir, "host", HOST);
    common::import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_simulate(&mut h);
    assert!(h.query_by_label("Good").is_some(), "no grade shown");
    assert!(h.query_by_label("100 % of the line on target").is_some());
    assert!(h.query_by_label("Time (estimate)").is_some());
    assert!(h.query_by_label(sim::COMPARE).is_some());
    assert_eq!(simulate_status(&mut h), "Good · 100 % on target");
    // The run is the document's last simulation, so an agent reads the
    // same grade.
    let stored = s.lock().unwrap().last_sim().cloned().expect("stored");
    assert_eq!(stored.grade(), Some(flatcraft_sim::Grade::Good));
    // Painted on the mat in the mat's green.
    let mat = flatcraft::ui::canvas::mat_rgb(&{
        let ctx = h.ctx.clone();
        flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx)
    });
    let green = tip_colour(mat, 0.0).unwrap();
    let has_green = painted(&h).iter().any(|s| match s {
        egui::Shape::Path(p) => p.stroke.color == egui::epaint::ColorMode::Solid(green),
        _ => false,
    });
    assert!(has_green, "no tip drawn in green");
}

#[test]
fn firmware_mode_shows_commanded_path_note() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    // File Output steers the blade in its firmware (FC18).
    common::import_square(&s);
    let mut h = harness(&dir, s, false);
    to_simulate(&mut h);
    // Nothing is graded; one line says why.
    assert!(h.query_by_label(sim::NOT_GRADED).is_some());
    assert!(h.query_by_label(sim::FIRMWARE_LINE).is_some());
    assert!(h.query_by_label("Commanded path").is_some());
    assert!(painted_texts(&h).contains(&sim::FIRMWARE_CANVAS.to_string()));
    assert_eq!(simulate_status(&mut h), "Not graded");
    // The sent path plays: the timeline is enabled.
    use egui_kittest::kittest::NodeT as _;
    assert!(!h.get_by_label("Time").accesskit_node().is_disabled());
    assert!(!h.get_by_label("Play").accesskit_node().is_disabled());
}

#[test]
fn firmware_mode_plays_the_sent_path() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    common::import_square(&s);
    let mut h = harness(&dir, s, false);
    to_simulate(&mut h);
    // The clock runs over the sent path: its travel and its cuts.
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
    let cur = app(&h).ui.cut.sim.current(&snap).cloned().unwrap().unwrap();
    assert!(cur.run.result.is_none(), "firmware mode");
    let c = cur.run.commanded;
    let clock = app(&h).ui.cut.sim.clock;
    assert!((clock.total_mm - (c.travel_mm + c.cut_mm)).abs() < 1e-9);
    assert_eq!(clock.t_mm, clock.total_mm, "a new run shows all of it");
    let (all, total) = app(&h).ui.cut.sim.drawn_down;
    assert_eq!((all, total), (c.segments, c.segments));
    // Play starts from the start; frames draw more of the path.
    h.get_by_label("Play").click();
    h.step();
    h.step();
    assert!(app(&h).ui.cut.sim.clock.playing.is_some());
    let first = app(&h).ui.cut.sim.drawn_down.0;
    assert!(first < total, "{first} of {total}");
    let start = Instant::now();
    while app(&h).ui.cut.sim.drawn_down.0 <= first {
        assert!(start.elapsed() < Duration::from_secs(10), "playback stuck");
        h.step();
    }
    // Scrubbing halfway draws the first part of the path only.
    h.get_by_label("Stop").click();
    h.step();
    h.step();
    app_mut(&mut h).ui.cut.sim.clock.t_mm = c.travel_mm.min(clock.total_mm) / 4.0;
    h.step();
    assert!(app(&h).ui.cut.sim.drawn_down.0 < total);
    // The end of the slider is the end of the last move: 100 % drawn
    // exactly there, and not a hair before.
    let total_mm = app(&h).ui.cut.sim.clock.total_mm;
    app_mut(&mut h).ui.cut.sim.clock.t_mm = total_mm;
    h.step();
    assert_eq!(app(&h).ui.cut.sim.drawn_down, (total, total));
    app_mut(&mut h).ui.cut.sim.clock.t_mm = total_mm - 0.01;
    h.step();
    assert_eq!(app(&h).ui.cut.sim.drawn_down.0, total - 1);
    // Played to the end, it stops with all of it drawn.
    app_mut(&mut h).ui.cut.sim.clock.speed = 4.0;
    h.get_by_label("Play").click();
    let start = Instant::now();
    loop {
        h.step();
        let clock = app(&h).ui.cut.sim.clock;
        if clock.playing.is_none() && clock.t_mm >= clock.total_mm {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "playback stuck");
    }
    h.step();
    assert_eq!(app(&h).ui.cut.sim.drawn_down, (total, total));
    // No grade and no deviation colours: nothing green on the mat.
    let mat = flatcraft::ui::canvas::mat_rgb(&snap);
    let colours: Vec<_> = (0..3)
        .map(|i| tip_colour(mat, [0.0, 0.1, 1.0][i]).unwrap())
        .collect();
    let graded = painted(&h).iter().any(|s| match s {
        egui::Shape::Path(p) => colours
            .iter()
            .any(|c| p.stroke.color == egui::epaint::ColorMode::Solid(*c)),
        _ => false,
    });
    assert!(!graded, "a deviation colour is painted");
    for word in ["Good", "Fair", "Poor"] {
        assert!(h.query_by_label(word).is_none(), "{word}");
    }
    assert!(h.query_by_label(sim::NOT_GRADED).is_some());
}

#[test]
fn timeline_scrubs_simulation_time() {
    let dir = tempfile::tempdir().unwrap();
    let s = plotter_session(&dir, "host", HOST);
    common::import_square(&s);
    let mut h = harness(&dir, s, false);
    to_simulate(&mut h);
    let clock = app(&h).ui.cut.sim.clock;
    assert!(clock.total_mm > 0.0);
    assert_eq!(clock.t_mm, clock.total_mm, "a new run shows all of it");
    let (all, total) = app(&h).ui.cut.sim.drawn_down;
    assert_eq!(all, total);
    app_mut(&mut h).ui.cut.sim.clock.t_mm = clock.total_mm / 2.0;
    h.run_plan();
    let (drawn, total) = app(&h).ui.cut.sim.drawn_down;
    let share = drawn as f64 / total as f64;
    assert!((0.4..=0.6).contains(&share), "{drawn} of {total}");
    // The Path stage keeps its toolpath index.
    assert_eq!(app(&h).ui.cut.timeline.index, app(&h).ui.cut.timeline.total);
    // Play runs from the start at the chosen speed.
    h.get_by_label("Speed 4x").click();
    h.run_plan();
    assert_eq!(app(&h).ui.cut.sim.clock.speed, 4.0);
}

#[test]
fn quality_color_thresholds() {
    for t in [Tokens::light(), Tokens::dark()] {
        let l = END_TO_END_LIMIT_MM as f32;
        assert_eq!(quality_color(0.0, &t), Some(t.quality_good));
        assert_eq!(quality_color(l, &t), Some(t.quality_good));
        assert_eq!(quality_color(l * 1.01, &t), Some(t.quality_warn));
        assert_eq!(quality_color(2.0 * l, &t), Some(t.quality_warn));
        assert_eq!(quality_color(2.0 * l * 1.01, &t), Some(t.quality_bad));
        assert_eq!(quality_color(f32::NAN, &t), None);
    }
    // On the mat: the engine preview's colours, none for NaN.
    let paper = [0xFB, 0xF4, 0xE4];
    let [g, a, r] = flatcraft_core::colour::deviation_colours(paper)
        .map(|c| egui::Color32::from_rgb(c[0], c[1], c[2]));
    let l = END_TO_END_LIMIT_MM as f32;
    assert_eq!(tip_colour(paper, l), Some(g));
    assert_eq!(tip_colour(paper, l * 1.5), Some(a));
    assert_eq!(tip_colour(paper, l * 3.0), Some(r));
    assert_eq!(tip_colour(paper, f32::NAN), None);
}

/// Blocks every simulation until `release` sends; a test that waits on a
/// worker never hangs: the gate opens by itself after a while.
fn gate(s: &SharedSession) -> mpsc::Sender<()> {
    gate_with_entry(s).0
}

/// [`gate`], plus a receiver that gets a message each time a simulation
/// reaches the gate, so a test waits for that event, not for wall time.
fn gate_with_entry(s: &SharedSession) -> (mpsc::Sender<()>, mpsc::Receiver<()>) {
    let (tx, rx) = mpsc::channel::<()>();
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let rx = Arc::new(Mutex::new(rx));
    let entered_tx = Mutex::new(entered_tx);
    s.lock().unwrap().unlocked_hook = Some(UnlockedHook(Arc::new(move |id| {
        if id == "job.simulate" {
            let _ = entered_tx.lock().unwrap().send(());
            let _ = rx.lock().unwrap().recv_timeout(Duration::from_secs(60));
        }
    })));
    (tx, entered_rx)
}

#[test]
fn simulation_runs_off_the_ui_thread() {
    let dir = tempfile::tempdir().unwrap();
    let s = plotter_session(&dir, "host", HOST);
    // One square: the gate holds the simulation, so it needs no size to be
    // slow, and after the gate it ends quickly on a loaded machine too.
    common::import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run_plan();
    let (release, entered) = gate_with_entry(&s);
    h.get_by_role_and_label(Role::Tab, "Simulate").click();
    // Step until the worker holds the simulation at the gate. The event
    // ends the wait, not a time: a loaded machine only takes more frames.
    let mut frames = 0;
    while entered.try_recv().is_err() {
        h.step();
        frames += 1;
        assert!(frames < 100_000, "the simulation never started");
        std::thread::sleep(Duration::from_millis(1));
    }
    // The worker holds the simulation and the frames come back anyway:
    // run in the frame, a step would wait for the gate.
    h.step();
    h.step();
    assert!(app(&h).ui.cut.sim.simulating());
    assert!(painted_texts(&h).contains(&sim::SIMULATING.to_string()));
    assert_eq!(simulate_status(&mut h), sim::SIMULATING);
    release.send(()).unwrap();
    h.run_plan();
    assert!(!app(&h).ui.cut.sim.simulating());
    assert!(h.query_by_label("Worst miss").is_some());
}

#[test]
fn a_change_while_simulating_never_shows_the_old_run() {
    let dir = tempfile::tempdir().unwrap();
    let s = plotter_session(&dir, "host", HOST);
    let id = common::import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run_plan();
    let release = gate(&s);
    h.get_by_role_and_label(Role::Tab, "Simulate").click();
    // The click lands in the first frame, the run starts in the second.
    h.step();
    h.step();
    assert!(app(&h).ui.cut.sim.simulating());
    run(
        &s,
        "object.move",
        json!({ "ids": [id], "dx_mm": 5.0, "dy_mm": 0.0 }),
    );
    release.send(()).unwrap();
    release.send(()).unwrap();
    h.run_plan();
    // The run of the moved square is the one shown and stored.
    let rev = s.lock().unwrap().doc().revision();
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
    let cur = app(&h).ui.cut.sim.current(&snap).cloned().unwrap().unwrap();
    assert_eq!(cur.run.key.doc_revision, rev);
    assert!(app(&h).ui.cut.sim.computed >= 2);
    assert_eq!(s.lock().unwrap().last_sim().unwrap().key.doc_revision, rev);
}

#[test]
fn worst_spots_are_ringed_and_centre_the_view() {
    let dir = tempfile::tempdir().unwrap();
    // A knife with no compensation cuts the corners short.
    let s = plotter_session(&dir, "pen", PEN);
    common::import_square(&s);
    let mut h = harness(&dir, s, false);
    to_simulate(&mut h);
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
    let cur = app(&h).ui.cut.sim.current(&snap).cloned().unwrap().unwrap();
    assert!(
        !cur.spots.is_empty(),
        "an uncompensated square has bad spots"
    );
    // The grade comes with the reason that decided it, the score after it
    // as the share on target: never "Poor - score 98".
    let m = &cur.run.result.as_ref().unwrap().metrics;
    let line = sim::grade_line(m);
    assert!(line.contains(": "), "{line}");
    assert!(h.query_by_label(&line).is_some(), "{line} not shown");
    assert!(h.query_by_label(&sim::on_target_text(m.score)).is_some());
    let status = simulate_status(&mut h);
    assert_eq!(status, sim::grade_short(m));
    assert!(!status.contains("score"), "{status}");
    // The pen mode warning says what is simulated.
    assert!(h.query_by_label_contains("no knife compensation").is_some());
    let at = cur.spots[0].at;
    let ring = h.get_by_label(&sim::spot_name(0)).rect().center();
    h.get_by_label(&sim::spot_name(0)).click();
    h.run_plan();
    let view = app(&h).ui.cut.view;
    let rect = app(&h).ui.cut.canvas_rect;
    let now = view.mm_to_screen(at);
    assert!(
        now.distance(rect.center()) < 1.0,
        "{now:?} vs {:?}",
        rect.center()
    );
    assert!(now.distance(ring) > 1.0 || ring.distance(rect.center()) < 1.0);
    assert!(view.px_per_mm >= 8.0);
}

#[test]
fn compare_strategies_lists_all_three_and_uses_one() {
    let dir = tempfile::tempdir().unwrap();
    let s = plotter_session(&dir, "host", HOST);
    import(
        &s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="10" y="10" width="40" height="40"/><rect x="20" y="20" width="10" height="10"/><rect x="80" y="10" width="20" height="20"/></svg>"#,
    );
    let mut h = harness(&dir, s.clone(), false);
    to_simulate(&mut h);
    h.get_by_label(sim::COMPARE).click();
    h.run_plan();
    for name in ["Quality", "Fast", "Outlines last"] {
        assert!(h.query_by_label(name).is_some(), "{name} row");
    }
    assert!(h.query_by_label("Use Fast").is_some());
    h.get_by_label("Use Fast").click();
    h.run_plan();
    assert_eq!(
        s.lock().unwrap().doc().document.job.strategy,
        flatcraft_core::document::CutStrategy::Fast
    );
    // The table was for the old state: it is gone, and the new strategy
    // is simulated.
    assert!(h.query_by_label("Use Fast").is_none());
    assert_eq!(
        s.lock().unwrap().last_sim().unwrap().strategy,
        flatcraft_core::document::CutStrategy::Fast
    );
}

#[test]
fn the_stage_bar_never_shows_an_old_grade_while_simulating() {
    let dir = tempfile::tempdir().unwrap();
    let s = plotter_session(&dir, "host", HOST);
    let id = common::import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_simulate(&mut h);
    assert!(simulate_status(&mut h).starts_with("Good"));
    let release = gate(&s);
    run(
        &s,
        "object.move",
        json!({ "ids": [id], "dx_mm": 5.0, "dy_mm": 0.0 }),
    );
    h.step();
    h.step();
    assert!(app(&h).ui.cut.sim.simulating());
    // The new state may grade differently: the bar says it is working.
    assert_eq!(simulate_status(&mut h), sim::SIMULATING);
    assert!(!painted_texts(&h).iter().any(|t| t.starts_with("Good")));
    release.send(()).unwrap();
    h.run_plan();
    assert!(simulate_status(&mut h).starts_with("Good"));
}

#[test]
fn a_run_of_another_document_is_never_drawn() {
    let dir = tempfile::tempdir().unwrap();
    let s = plotter_session(&dir, "host", HOST);
    common::import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_simulate(&mut h);
    let release = gate(&s);
    run(&s, "document.new", json!({}));
    run(&s, "job.set_plotter", json!({ "id": "host" }));
    common::import_square(&s);
    h.step();
    h.step();
    assert!(app(&h).ui.cut.sim.simulating());
    assert!(
        app(&h).ui.cut.sim.previous.is_none(),
        "the faded run belongs to the other document"
    );
    release.send(()).unwrap();
    h.run_plan();
}

#[test]
fn use_in_the_comparison_is_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let s = plotter_session(&dir, "host", HOST);
    common::import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_simulate(&mut h);
    let before = common::undo_len(&s);
    h.get_by_label(sim::COMPARE).click();
    h.run_plan();
    h.get_by_label("Use Fast").click();
    h.run_plan();
    assert_eq!(common::undo_len(&s), before + 1);
    run(&s, "edit.undo", json!({}));
    h.run_plan();
    assert_eq!(
        s.lock().unwrap().doc().document.job.strategy,
        flatcraft_core::document::CutStrategy::Quality
    );
}

#[test]
fn play_draws_the_cut_as_it_goes() {
    let dir = tempfile::tempdir().unwrap();
    let s = plotter_session(&dir, "host", HOST);
    common::import_square(&s);
    let mut h = harness(&dir, s, false);
    to_simulate(&mut h);
    let total = app(&h).ui.cut.sim.drawn_down.1;
    h.get_by_label("Play").click();
    // The click lands, then playback starts from the start.
    h.step();
    h.step();
    let clock = app(&h).ui.cut.sim.clock;
    assert!(clock.playing.is_some());
    let first = app(&h).ui.cut.sim.drawn_down.0;
    assert!(first < total, "{first} of {total}");
    // Frames advance the input time: the drawn part grows.
    let start = Instant::now();
    while app(&h).ui.cut.sim.drawn_down.0 <= first {
        assert!(start.elapsed() < Duration::from_secs(10), "playback stuck");
        h.step();
    }
    assert!(app(&h).ui.cut.sim.clock.t_mm > clock.t_mm);
    // Stop keeps the place.
    h.get_by_label("Stop").click();
    h.step();
    h.step();
    let at = app(&h).ui.cut.sim.clock.t_mm;
    h.step();
    assert_eq!(app(&h).ui.cut.sim.clock.t_mm, at);
    assert!(app(&h).ui.cut.sim.clock.playing.is_none());
}

/// Where the stage tabs are, left to right.
fn tab_rects(h: &AppHarness) -> Vec<egui::Rect> {
    Stage::ALL
        .iter()
        .map(|s| h.get_by_role_and_label(Role::Tab, s.name()).rect())
        .collect()
}

#[test]
fn the_bar_keeps_its_layout_while_simulating() {
    let dir = tempfile::tempdir().unwrap();
    let s = plotter_session(&dir, "host", HOST);
    let id = common::import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_simulate(&mut h);
    let good = simulate_status(&mut h);
    assert!(good.starts_with("Good"), "{good}");
    let before = tab_rects(&h);
    let release = gate(&s);
    run(
        &s,
        "object.move",
        json!({ "ids": [id], "dx_mm": 5.0, "dy_mm": 0.0 }),
    );
    // Every frame of the run: "Simulating..." in the width of the line
    // before, so no step moves.
    let mut simulating = 0;
    for _ in 0..6 {
        h.step();
        if painted_texts(&h).contains(&sim::SIMULATING.to_string()) {
            simulating += 1;
        }
        assert_eq!(tab_rects(&h), before);
    }
    assert!(simulating > 0, "the run was never shown as running");
    release.send(()).unwrap();
    h.run_plan();
    assert_eq!(simulate_status(&mut h), good);
    assert_eq!(tab_rects(&h), before);
}

/// Starts a comparison whose other strategies wait for a gate, goes Home
/// while it runs, opens the gate and waits until it ended there.
fn compare_ending_on_home(dir: &tempfile::TempDir) -> (SharedSession, u64, AppHarness) {
    let s = plotter_session(dir, "host", HOST);
    let id = common::import_square(&s);
    let mut h = harness(dir, s.clone(), false);
    to_simulate(&mut h);
    // The other strategies wait for the gate, like a slow machine. The
    // document's own strategy was simulated above and is reused.
    let others = flatcraft_engine::simulate::STRATEGIES.len() - 1;
    let (release, gate) = mpsc::channel::<()>();
    let (entered_tx, entered) = mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let entered_tx = Mutex::new(entered_tx);
    app_mut(&mut h).ui.cut.sim.set_computer(move |input| {
        let _ = entered_tx.lock().unwrap().send(());
        let _ = gate.lock().unwrap().recv_timeout(Duration::from_secs(60));
        flatcraft_engine::simulate::compute(input)
    });
    h.get_by_label(sim::COMPARE).click();
    let mut frames = 0;
    while entered.try_recv().is_err() {
        h.step();
        frames += 1;
        assert!(frames < 100_000, "the comparison never started");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(app(&h).ui.cut.sim.busy());
    // Home while it runs; it ends there.
    h.get_by_role_and_label(Role::Button, "Home").click();
    h.step();
    assert_eq!(app(&h).ui.workspace, Workspace::Home);
    for _ in 0..others {
        release.send(()).unwrap();
    }
    common::wait_until(&mut h, |a| a.ui.cut.sim.busy());
    assert_eq!(app(&h).ui.workspace, Workspace::Home);
    (s, id, h)
}

#[test]
fn a_comparison_that_ends_on_home_frees_the_worker() {
    let dir = tempfile::tempdir().unwrap();
    let (_s, _id, mut h) = compare_ending_on_home(&dir);
    // Back in Simulate, the comparison of the unchanged document shows.
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run_plan();
    assert_eq!(app(&h).ui.cut.stage, Stage::Simulate);
    assert!(h.query_by_label("Use Fast").is_some());
}

#[test]
fn a_comparison_kept_on_home_is_dropped_when_the_document_changed() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id, mut h) = compare_ending_on_home(&dir);
    // The square moves while Home shows: the comparison is of an old state.
    run(
        &s,
        "object.move",
        json!({ "ids": [id], "dx_mm": 5.0, "dy_mm": 0.0 }),
    );
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run_plan();
    assert_eq!(app(&h).ui.cut.stage, Stage::Simulate);
    assert!(h.query_by_label("Use Fast").is_none(), "no stale rows");
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
    assert!(app(&h).ui.cut.sim.compared(&snap).is_none());
    // The button asks again.
    h.get_by_label(sim::COMPARE);
}

/// Where the Simulate sections' headers are.
fn section_rects(h: &AppHarness) -> Vec<egui::Rect> {
    ["Numbers", "Hints", "Worst spots", "Strategies", "Legend"]
        .iter()
        .map(|t| h.get_by_role_and_label(Role::Button, t).rect())
        .collect()
}

#[test]
fn the_sections_stay_in_place_while_a_run_goes() {
    let dir = tempfile::tempdir().unwrap();
    let s = plotter_session(&dir, "host", HOST);
    let id = common::import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_simulate(&mut h);
    let landed = section_rects(&h);
    let (release, entered) = gate_with_entry(&s);
    // An edit: the run of the new state waits at the gate.
    run(
        &s,
        "object.move",
        json!({ "ids": [id], "dx_mm": 5.0, "dy_mm": 0.0 }),
    );
    h.step();
    assert_eq!(section_rects(&h), landed, "the edit");
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    for _ in 0..4 {
        h.step();
        assert!(app(&h).ui.cut.sim.simulating());
        // The run before, faded, under "Simulating...".
        assert!(painted_texts(&h).contains(&sim::SIMULATING.to_string()));
        assert_eq!(section_rects(&h), landed, "the run");
    }
    release.send(()).unwrap();
    h.run_plan();
    assert!(!app(&h).ui.cut.sim.simulating());
    assert_eq!(section_rects(&h), landed, "landed");
}
