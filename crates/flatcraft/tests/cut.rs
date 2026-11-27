//! The Cut workspace, driven through egui_kittest.

mod common;
use common::RunPlan;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{
    AppHarness, FakePicker, app, app_mut, harness, harness_with, import_square, session,
    wait_for_source_check,
};
use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::Workspace;
use flatcraft::ui::cut::{Stage, mm_text, stage_status};
use flatcraft::ui::files::Revealer;
use flatcraft::ui::icons::{Icon, Palette};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

fn run(s: &SharedSession, cmd: &str, params: Value) -> Value {
    s.lock()
        .unwrap()
        .execute(cmd, params, Origin::User)
        .unwrap()
}

fn to_cut(h: &mut AppHarness) {
    app_mut(h).ui.workspace = Workspace::Cut;
    h.run_plan();
}

fn stage(h: &mut AppHarness, name: &str) {
    h.get_by_role_and_label(Role::Tab, name).click();
    h.run_plan();
}

fn files_in(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|r| {
            r.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Records revealed files instead of opening the file manager.
#[derive(Clone, Default)]
struct FakeRevealer(Arc<Mutex<Vec<PathBuf>>>);

impl Revealer for FakeRevealer {
    fn reveal(&mut self, file: &std::path::Path) -> Result<(), String> {
        self.0.lock().unwrap().push(file.to_path_buf());
        Ok(())
    }
}

#[test]
fn send_writes_file_from_stage_4() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let revealed = FakeRevealer::default();
    let r = revealed.clone();
    let mut h = harness_with(&dir, s.clone(), move |a| a.with_revealer(r));
    to_cut(&mut h);
    stage(&mut h, "Send");
    h.get_by_role_and_label(Role::Button, "Send to File Output")
        .click();
    h.run_plan();
    let out = dir.path().join("output");
    let names = files_in(&out);
    assert!(
        names.iter().any(|n| n.ends_with(".gpgl")),
        "no .gpgl in {names:?}"
    );
    // The file holds exactly the job the Cut workspace previewed.
    let gpgl_name = names.iter().find(|n| n.ends_with(".gpgl")).unwrap();
    let written = std::fs::read(out.join(gpgl_name)).unwrap();
    assert_eq!(written, app(&h).ui.cut.job().unwrap().job.bytes);
    let toast = app(&h).ui.toasts.last().cloned().unwrap();
    assert!(toast.text.starts_with("Sent!"), "{}", toast.text);
    let detail = toast.detail.clone().unwrap_or_default();
    assert!(detail.contains(".gpgl.txt"), "{detail}");
    let gpgl = toast.reveal.clone().unwrap();
    assert_eq!(gpgl.extension().unwrap(), "gpgl");
    let label = flatcraft::ui::files::reveal_label();
    h.get_by_label(label).click();
    h.run_plan();
    assert_eq!(*revealed.0.lock().unwrap(), vec![gpgl]);
}

#[test]
fn sent_bytes_show_in_the_debug_byte_log() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    stage(&mut h, "Send");
    h.get_by_role_and_label(Role::Button, "Debug panel").click();
    h.run_plan();
    assert!(h.query_by_label("No device (File Output)").is_some());
    assert!(h.query_by_label("home").is_none());
    h.get_by_role_and_label(Role::Button, "Send to File Output")
        .click();
    h.run_plan();
    h.run_plan();
    let rows = &app(&h).ui.debug.log_rows;
    assert!(!rows.is_empty());
    assert_eq!(rows.last().unwrap().meaning, "home");
    assert!(h.query_by_label("home").is_some());
    // The listing of the current job.
    h.get_by_role_and_label(Role::Tab, "Listing").click();
    h.run_plan();
    assert!(
        h.query_by_label("FC18\t# blade offset 18 (firmware compensation)")
            .is_some()
    );
    let job = app(&h).ui.cut.job().unwrap();
    assert!(
        job.job
            .listing
            .iter()
            .any(|l| l.note.contains("'square', path 0, pass 1"))
    );
}

/// Every text painted last frame.
fn painted_texts(h: &AppHarness) -> Vec<String> {
    common::painted(h)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
            _ => None,
        })
        .collect()
}

/// The status line under each stage, as painted on the bar.
fn status_lines(h: &mut AppHarness) -> Vec<String> {
    let texts = painted_texts(h);
    let ctx = h.ctx.clone();
    let snap = flatcraft::ui::prepare::snapshot(app_mut(h), &ctx);
    let want: Vec<String> = Stage::ALL
        .iter()
        .map(|s| stage_status(*s, &app(h).ui.cut, &snap))
        .collect();
    for w in &want {
        assert!(texts.contains(w), "{w:?} not painted in {texts:?}");
    }
    want
}

#[test]
fn every_stage_shows_its_icon_and_title() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s, false);
    to_cut(&mut h);
    assert_eq!(
        Stage::ALL.map(Stage::icon),
        [Icon::Plotter, Icon::RouteTravel, Icon::Play, Icon::Send]
    );
    let textures: Vec<egui::TextureId> = common::painted(&h)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::Mesh(m) => Some(m.texture_id),
            egui::Shape::Rect(r) => Some(r.fill_texture_id()),
            _ => None,
        })
        .collect();
    let texts = painted_texts(&h);
    let tokens = app(&h).tokens;
    for stage in Stage::ALL {
        // In the Cut colours: ink accent on the selected stage.
        let pal = Palette::cut(&tokens, stage == Stage::Device);
        let icon = flatcraft::ui::icons::texture_with(&h.ctx, stage.icon(), 24.0, &pal).id();
        assert!(textures.contains(&icon), "{stage:?}: no 24 pt icon");
        let title = format!("{}  {}", stage.number(), stage.name());
        assert!(texts.contains(&title), "{title:?} not in {texts:?}");
        // The accessible name stays the stage's name, a tab.
        let node = h.get_by_role_and_label(Role::Tab, stage.name());
        let want = if stage == Stage::Device {
            Toggled::True
        } else {
            Toggled::False
        };
        assert_eq!(node.accesskit_node().toggled(), Some(want), "{stage:?}");
    }
}

#[test]
fn status_lines_say_what_is_set_or_missing() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    // A fresh document: nothing to cut yet.
    to_cut(&mut h);
    assert_eq!(
        status_lines(&mut h),
        [
            "File Output · Copy paper",
            "Nothing to cut",
            "Nothing to cut",
            "Nothing to cut"
        ]
    );
    // A planned job, on file and then on USB, with two passes.
    import_square(&s);
    h.run_plan();
    assert_eq!(
        status_lines(&mut h),
        [
            "File Output · Copy paper",
            "Quality · 1 pass",
            "Not simulated",
            "File · ready"
        ]
    );
    run(&s, "job.set_plotter", json!({ "id": "silhouette-cameo-2" }));
    run(&s, "job.set_overrides", json!({ "passes": 2 }));
    run(&s, "job.set_weeding", json!({ "box": true }));
    h.run_plan();
    assert_eq!(
        status_lines(&mut h),
        [
            "Cameo 2 · Copy paper",
            "Quality · 2 passes · weed",
            "Not simulated",
            "USB · ready"
        ]
    );
    // The plotter preset is gone.
    s.lock().unwrap().doc_mut().document.job.plotter_id = "gone".into();
    h.run_plan();
    assert_eq!(
        status_lines(&mut h),
        [
            "Pick a plotter",
            "Pick a plotter",
            "Pick a plotter",
            "Pick a plotter"
        ]
    );
}

#[test]
fn simulate_stage_opens_and_explains_itself() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s, false);
    h.ctx
        .global_style_mut(|st| st.interaction.tooltip_delay = 0.0);
    to_cut(&mut h);
    let node = h.get_by_role_and_label(Role::Tab, "Simulate");
    assert!(!node.accesskit_node().is_disabled());
    node.click();
    h.run_plan();
    assert_eq!(app(&h).ui.cut.stage, Stage::Simulate);
    let hint = Stage::Simulate.hint();
    assert!(hint.contains("drag knife"), "{hint}");
    let at = h
        .get_by_role_and_label(Role::Tab, "Simulate")
        .rect()
        .center();
    h.hover_at(at);
    wait_for_label(&mut h, hint);
    // An enabled stage shows what it does.
    let at = h.get_by_role_and_label(Role::Tab, "Send").rect().center();
    h.hover_at(at);
    wait_for_label(&mut h, Stage::Send.hint());
}

/// Steps frames until `label` shows; fails after a few seconds, never
/// after a count of frames.
fn wait_for_label(h: &mut AppHarness, label: &str) {
    let start = Instant::now();
    while h.query_by_label(label).is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{label:?} did not show"
        );
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Where the stage tabs are, left to right.
fn tab_rects(h: &AppHarness) -> Vec<egui::Rect> {
    Stage::ALL
        .iter()
        .map(|s| h.get_by_role_and_label(Role::Tab, s.name()).rect())
        .collect()
}

#[test]
fn the_bar_does_not_flicker_while_the_job_is_planned_again() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    let before = tab_rects(&h);
    // A replan that keeps the status lines: the bar does not move in any
    // frame.
    run(
        &s,
        "object.move",
        json!({ "ids": [id], "dx_mm": 5.0, "dy_mm": 0.0 }),
    );
    for _ in 0..4 {
        h.step();
        assert_eq!(tab_rects(&h), before);
    }
    // A replan with a longer status line: until the new plan lands the
    // bar keeps the old line (no "Planning..." blip), the frame it lands
    // has the new layout, and the frames after it keep it.
    run(&s, "job.set_overrides", json!({ "passes": 2 }));
    let start = std::time::Instant::now();
    loop {
        h.step();
        if painted_texts(&h).contains(&"Quality · 2 passes".to_string()) {
            break;
        }
        assert_eq!(
            tab_rects(&h),
            before,
            "the bar keeps its layout while planning"
        );
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the plan did not land"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let after = tab_rects(&h);
    assert_ne!(after, before, "the Path step grew");
    let debug = h.get_by_role_and_label(Role::Button, "Debug panel").rect();
    for r in &after {
        assert!(r.max.x <= debug.min.x, "{r:?}");
    }
    for _ in 0..4 {
        h.step();
        assert_eq!(tab_rects(&h), after);
    }
}

#[test]
fn stage_texts_have_no_em_dash() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    use flatcraft::ui::simulate as sim;
    let mut texts: Vec<String> = [
        sim::SIMULATING,
        sim::SIMULATING_CARD,
        sim::FIRMWARE_CANVAS,
        sim::COMPARE_CARD,
        sim::GRADE_CARD,
        sim::LEGEND_HEAD.1,
        sim::LEGEND_GOOD.1,
        sim::LEGEND_WARN.1,
        sim::LEGEND_BAD.1,
        sim::LEGEND_WASTE_MOVE.1,
        sim::LEGEND_ALIGNMENT.1,
        sim::LEGEND_RING.1,
        sim::LEGEND_COMMANDED.1,
        flatcraft::ui::timeline::SIM_SLIDER_CARD,
        flatcraft::ui::timeline::SIM_SPEED_CARD,
        flatcraft::ui::timeline::SIM_PLAY_CARD,
        flatcraft::ui::timeline::SIM_NOTHING,
    ]
    .map(String::from)
    .to_vec();
    for stage in Stage::ALL {
        texts.extend([stage.name().into(), stage.title(), stage.hint().into()]);
    }
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    texts.extend(status_lines(&mut h));
    import_square(&s);
    h.run_plan();
    texts.extend(status_lines(&mut h));
    for t in texts {
        assert!(!t.contains('\u{2014}'), "{t}");
    }
}

#[test]
fn out_of_area_disables_send() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    run(
        &s,
        "object.move",
        json!({ "ids": [id], "dx_mm": 1000.0, "dy_mm": 0.0 }),
    );
    let mut h = harness(&dir, s, false);
    to_cut(&mut h);
    stage(&mut h, "Path");
    assert!(
        h.query_by_label("'square' is outside the cutting area.")
            .is_some()
    );
    assert!(h.query_by_label_contains("Refused").is_none());
    stage(&mut h, "Send");
    let send = h.get_by_role_and_label(Role::Button, "Send to File Output");
    assert!(send.accesskit_node().is_disabled());
    send.click();
    h.run_plan();
    assert!(files_in(&dir.path().join("output")).is_empty());
}

/// Three separate 20 mm squares.
const THREE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="10" y="10" width="20" height="20"/><rect x="60" y="10" width="20" height="20"/><rect x="110" y="10" width="20" height="20"/></svg>"#;

#[test]
fn timeline_limits_drawn_paths() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    run(
        &s,
        "object.import_svg",
        json!({ "svg": THREE_SVG, "name": "three" }),
    );
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    stage(&mut h, "Path");
    assert_eq!(app(&h).ui.cut.timeline.total, 3);
    assert_eq!(app(&h).ui.cut.visible_toolpaths(), 3);
    assert!(h.query_by_label("3 of 3").is_some());
    app_mut(&mut h).ui.cut.timeline.index = 1;
    h.run_plan();
    assert_eq!(app(&h).ui.cut.visible_toolpaths(), 1);
    assert!(h.query_by_label("1 of 3").is_some());
    // A changed job starts over with all toolpaths shown.
    // Two passes on closed rings are chained laps: still three toolpaths.
    run(&s, "job.set_overrides", json!({ "passes": 2 }));
    h.run_plan();
    assert_eq!(app(&h).ui.cut.timeline.total, 3);
    assert_eq!(app(&h).ui.cut.visible_toolpaths(), 3);
    assert!(h.query_by_label("3 of 3").is_some());
}

#[test]
fn path_stage_shows_stats() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let preview = run(&s, "job.preview", json!({}));
    let mut h = harness(&dir, s, false);
    to_cut(&mut h);
    stage(&mut h, "Path");
    for label in ["Paths", "Cut length", "Travel length"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    let cut = mm_text(preview["cut_mm"].as_f64().unwrap());
    let travel = mm_text(preview["travel_mm"].as_f64().unwrap());
    assert!(h.query_by_label(&cut).is_some(), "{cut}");
    assert!(h.query_by_label(&travel).is_some(), "{travel}");
    assert_eq!(preview["toolpaths"], 1);
    assert_eq!(app(&h).ui.cut.visible_toolpaths(), 1);
}

/// A path too small for one plotter step is not cut, so "Paths" leaves it
/// out, as job.preview and the warning do.
#[test]
fn paths_count_leaves_out_what_is_not_cut() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    // 0.03 mm from x = 9.98 mm: both ends round to one plotter step.
    let speck = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><path d="M60 60 L60.03 60" stroke="black" fill="none"/></svg>"#;
    let id = run(
        &s,
        "object.import_svg",
        json!({ "svg": speck, "name": "speck" }),
    )["id"]
        .clone();
    run(
        &s,
        "object.set_cut_options",
        json!({ "ids": [id], "mode": "all_lines", "strokes": "centre_line", "auto_threshold_mm": 1.5 }),
    );
    run(
        &s,
        "object.move",
        json!({ "ids": [id], "x_mm": 9.98, "y_mm": 20.0 }),
    );
    let preview = run(&s, "job.preview", json!({}));
    assert_eq!(preview["toolpaths"], 1);
    let planned = s
        .lock()
        .unwrap()
        .doc()
        .last_job()
        .unwrap()
        .plan
        .toolpaths
        .len();
    assert_eq!(planned, 2, "the plan still holds the speck");
    let mut h = harness(&dir, s, false);
    to_cut(&mut h);
    stage(&mut h, "Path");
    let job = app(&h).ui.cut.job().expect("a built job");
    assert_eq!(job.toolpaths_cut(), 1);
    assert!(
        h.query_by_label("1 path fits inside one plotter step and is not cut.")
            .is_some()
    );
    // The stats show 1 path, not the plan's 2.
    assert!(h.query_by_label("Paths").is_some());
    assert!(h.query_by_label("1").is_some());
    assert!(h.query_by_label("2").is_none());
}

/// "Weed paths" counts like "Paths": a weed path the emitter leaves out
/// is not cut, so it is not counted.
#[test]
fn weed_count_leaves_out_what_is_not_cut() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    run(
        &s,
        "job.set_weeding",
        json!({ "box": true, "lines": "both" }),
    );
    run(&s, "job.preview", json!({}));
    let mut job = s.lock().unwrap().doc().last_job().unwrap().clone();
    let weeds = flatcraft::ui::cut::weed_count(&job);
    assert!(weeds > 0);
    let all = job.toolpaths_cut();
    // Firmware compensation: one run per toolpath, so the run index is the
    // toolpath index.
    let first_weed = job
        .plan
        .toolpaths
        .iter()
        .position(|t| t.kind == flatcraft_path::ToolpathKind::Weed)
        .unwrap();
    job.job.collapsed = vec![first_weed];
    assert_eq!(flatcraft::ui::cut::weed_count(&job), weeds - 1);
    assert_eq!(job.toolpaths_cut(), all - 1);
}

#[test]
fn device_stage_shows_the_transport() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    let out = dir.path().join("output");
    let text = format!("File Output → {}", out.display());
    assert!(h.query_by_label(&text).is_some(), "{text}");
    run(&s, "job.set_plotter", json!({ "id": "silhouette-cameo-2" }));
    h.run_plan();
    assert!(h.query_by_label("USB 0b4d:112b").is_some());
    stage(&mut h, "Send");
    assert!(
        h.query_by_role_and_label(Role::Button, "Send to Cameo 2")
            .is_some()
    );
}

/// The USB plotter, a square, and an agent's send waiting for the person.
fn agent_request(s: &SharedSession) -> u64 {
    import_square(s);
    run(s, "job.set_plotter", json!({ "id": "silhouette-cameo-2" }));
    let mut g = s.lock().unwrap();
    let e = g.execute("job.send", json!({}), Origin::Agent).unwrap_err();
    assert!(e.to_string().contains("Waiting for confirmation"), "{e}");
    g.pending_send().unwrap().id
}

const QUESTION: &str = "Allow the agent to cut on Silhouette Cameo 2?";

#[test]
fn agent_confirmation_dialog_allows() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    agent_request(&s);
    let mut h = harness(&dir, s.clone(), true);
    // The dialog shows in every workspace, Home included.
    assert!(h.query_by_label(QUESTION).is_some());
    h.get_by_role_and_label(Role::Button, "Allow").click();
    h.run_plan();
    assert!(s.lock().unwrap().pending_send().is_none());
    assert!(h.query_by_label(QUESTION).is_none());
    let toast = app(&h).ui.toasts.last().cloned().unwrap();
    assert_eq!(toast.text, "USB cutting arrives in milestone 3.");
}

#[test]
fn agent_confirmation_dialog_denies() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    agent_request(&s);
    let mut h = harness(&dir, s.clone(), true);
    to_cut(&mut h);
    h.get_by_role_and_label(Role::Button, "Deny").click();
    h.run_plan();
    assert!(s.lock().unwrap().pending_send().is_none());
    assert!(h.query_by_label(QUESTION).is_none());
    assert!(files_in(&dir.path().join("output")).is_empty());
}

#[test]
fn allow_sends_the_snapshot_not_the_current_document() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = agent_request(&s);
    let snapshot = s.lock().unwrap().pending_send().cloned().unwrap();
    // An independent build of the document as the agent asked.
    let built = flatcraft_engine::job::build(&s.lock().unwrap()).unwrap();
    assert_eq!(built.job.bytes, snapshot.job.bytes);
    // The agent empties the document and raises the force after asking.
    let ids: Vec<u64> = {
        let g = s.lock().unwrap();
        g.doc().document.objects.iter().map(|o| o.id.0).collect()
    };
    s.lock()
        .unwrap()
        .execute("object.delete", json!({ "ids": ids }), Origin::Agent)
        .unwrap();
    s.lock()
        .unwrap()
        .execute("job.set_overrides", json!({ "force": 30 }), Origin::Agent)
        .unwrap();
    let mut h = harness(&dir, s.clone(), true);
    // The dialog shows the snapshot, not the empty document.
    let line = format!("1 path, {} cut,", mm_text(snapshot.summary.cut_mm));
    assert!(h.query_by_label_contains(&line).is_some(), "{line}");
    assert!(h.query_by_label_contains("force 20").is_none());
    {
        let g = s.lock().unwrap();
        let p = g.pending_send().unwrap();
        assert_eq!(p.id, id);
        assert_eq!(p.job.bytes, built.job.bytes);
        // The document now builds to something else (here: nothing).
        assert!(flatcraft_engine::job::build(&g).is_err());
    }
    h.get_by_role_and_label(Role::Button, "Allow").click();
    h.run_plan();
    // A rebuild would say "Nothing to cut"; the snapshot reaches the USB
    // transport, which M1 does not have yet.
    let toast = app(&h).ui.toasts.last().cloned().unwrap();
    assert_eq!(toast.text, "USB cutting arrives in milestone 3.");
    assert!(s.lock().unwrap().pending_send().is_none());
    assert!(files_in(&dir.path().join("output")).is_empty());
}

#[test]
fn agent_request_expires_after_five_minutes() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    agent_request(&s);
    let asked = s.lock().unwrap().pending_send().unwrap().requested_at;
    let now = Arc::new(Mutex::new(asked + Duration::from_secs(299)));
    let clock = now.clone();
    let mut h = harness_with(&dir, s.clone(), move |a| {
        a.with_clock(move || *clock.lock().unwrap())
    });
    assert!(h.query_by_label(QUESTION).is_some());
    *now.lock().unwrap() = asked + Duration::from_secs(300);
    h.run_plan();
    assert!(h.query_by_label(QUESTION).is_none());
    assert!(s.lock().unwrap().pending_send().is_none());
    let toast = app(&h).ui.toasts.last().cloned().unwrap();
    assert!(
        toast.text.contains("expired after 5 minutes"),
        "{}",
        toast.text
    );
    // The real clock is monotonic.
    assert!(Instant::now() >= asked);
}

#[test]
fn capture_viewer_shows_decoded_records() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let fixture = dir.path().join("cameo.jsonl");
    std::fs::write(
        &fixture,
        concat!(
            r#"{"t":0.0,"dir":"out","ep":"0x01","hex":"1b04"}"#,
            "\n",
            r#"{"t":0.25,"dir":"out","ep":"0x01","hex":"4803"}"#,
            "\n",
            r#"{"t":0.5,"dir":"in","ep":"0x82","hex":"3003","dev":7}"#,
            "\n",
        ),
    )
    .unwrap();
    let picker = FakePicker::answering([fixture]);
    let requests = picker.requests.clone();
    let mut h = harness_with(&dir, s, move |a| a.with_file_picker(picker));
    to_cut(&mut h);
    h.get_by_role_and_label(Role::Button, "Debug panel").click();
    h.run_plan();
    h.get_by_role_and_label(Role::Tab, "Capture").click();
    h.run_plan();
    h.get_by_role_and_label(Role::Button, "Open capture...")
        .click();
    h.run_plan();
    let req = requests.lock().unwrap()[0].clone();
    assert_eq!(req.filters[0].1, vec!["jsonl".to_string()]);
    for label in ["init", "home", "reply 0", "4803", "0.250 s", "cameo.jsonl"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    let rows = &app(&h).ui.debug.capture.as_ref().unwrap().rows;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].dir, "in");
}

#[test]
fn a_broken_capture_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let fixture = dir.path().join("broken.jsonl");
    std::fs::write(&fixture, "{\"t\":0}\n").unwrap();
    let picker = FakePicker::answering([fixture]);
    let mut h = harness_with(&dir, s, move |a| a.with_file_picker(picker));
    to_cut(&mut h);
    h.get_by_role_and_label(Role::Button, "Debug panel").click();
    h.run_plan();
    h.get_by_role_and_label(Role::Tab, "Capture").click();
    h.run_plan();
    h.get_by_role_and_label(Role::Button, "Open capture...")
        .click();
    h.run_plan();
    let toast = app(&h).ui.toasts.last().cloned().unwrap();
    assert!(toast.text.contains("broken.jsonl"), "{}", toast.text);
    assert!(toast.text.contains("line 1"), "{}", toast.text);
    assert!(app(&h).ui.debug.capture.is_none());
}

#[test]
fn debug_listing_shows_preview_and_last_sent() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    stage(&mut h, "Send");
    h.get_by_role_and_label(Role::Button, "Debug panel").click();
    h.run_plan();
    h.get_by_role_and_label(Role::Tab, "Listing").click();
    h.run_plan();
    assert!(
        h.query_by_label("Preview of the current job (.gpgl.txt)")
            .is_some()
    );
    h.get_by_role_and_label(Role::RadioButton, "Last sent")
        .click();
    h.run_plan();
    assert!(h.query_by_label("Nothing sent yet.").is_some());
    h.get_by_role_and_label(Role::Button, "Send to File Output")
        .click();
    h.run_plan();
    // The document changes after the send: the preview follows it, the
    // last sent listing does not.
    run(&s, "job.set_overrides", json!({ "speed": 2 }));
    h.run_plan();
    h.get_by_role_and_label(Role::RadioButton, "Last sent")
        .click();
    h.run_plan();
    let gpgl = files_in(&dir.path().join("output"))
        .into_iter()
        .find(|n| n.ends_with(".gpgl"))
        .unwrap();
    assert!(
        h.query_by_label(&format!("Last sent: {gpgl}.txt"))
            .is_some()
    );
    assert!(h.query_by_label("!5\t# speed 5").is_some());
    h.get_by_role_and_label(Role::RadioButton, "Preview")
        .click();
    h.run_plan();
    assert!(h.query_by_label("!2\t# speed 2").is_some());
}

fn open_last_sent(h: &mut AppHarness) {
    h.get_by_role_and_label(Role::Button, "Debug panel").click();
    h.run_plan();
    h.get_by_role_and_label(Role::Tab, "Listing").click();
    h.run_plan();
    h.get_by_role_and_label(Role::RadioButton, "Last sent")
        .click();
    h.run_plan();
}

fn sent_listing_name(dir: &tempfile::TempDir) -> String {
    let gpgl = files_in(&dir.path().join("output"))
        .into_iter()
        .find(|n| n.ends_with(".gpgl"))
        .unwrap();
    format!("Last sent: {gpgl}.txt")
}

#[test]
fn a_send_made_while_the_debug_panel_is_closed_shows_later() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    stage(&mut h, "Send");
    h.get_by_role_and_label(Role::Button, "Send to File Output")
        .click();
    h.run_plan();
    // A preview after the send does not hide it.
    run(&s, "job.preview", json!({}));
    h.run_plan();
    open_last_sent(&mut h);
    assert!(h.query_by_label(&sent_listing_name(&dir)).is_some());
}

#[test]
fn last_sent_belongs_to_the_active_document() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let first = s.lock().unwrap().doc().id();
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    stage(&mut h, "Send");
    h.get_by_role_and_label(Role::Button, "Send to File Output")
        .click();
    h.run_plan();
    let sent = sent_listing_name(&dir);
    open_last_sent(&mut h);
    assert!(h.query_by_label(&sent).is_some());
    run(&s, "document.new", json!({}));
    wait_for_source_check(&mut h);
    assert!(h.query_by_label("Nothing sent yet.").is_some());
    assert!(h.query_by_label(&sent).is_none());
    run(&s, "document.switch", json!({ "doc_id": first }));
    wait_for_source_check(&mut h);
    assert!(h.query_by_label(&sent).is_some());
}

#[test]
fn skipped_objects_are_left_out_of_the_cut_preview() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    run(
        &s,
        "object.import_svg",
        json!({ "svg": THREE_SVG, "name": "three" }),
    );
    let extra = run(
        &s,
        "object.import_svg",
        json!({ "svg": THREE_SVG, "name": "spare" }),
    );
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    stage(&mut h, "Path");
    assert_eq!(app(&h).ui.cut.timeline.total, 6);
    run(
        &s,
        "object.set_cut",
        json!({ "ids": [extra["id"]], "cut": false }),
    );
    h.run_plan();
    assert_eq!(app(&h).ui.cut.timeline.total, 3);
}

#[test]
fn cut_moves_are_orange_and_travel_is_faint() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s, false);
    to_cut(&mut h);
    stage(&mut h, "Path");
    // The test harness runs the dark theme, so a mat without a colour of
    // its own has the dark default.
    let default_mat = flatcraft_device::DEFAULT_MAT_COLOR_DARK;
    let [r, g, b] = flatcraft_core::colour::cut_colour(default_mat);
    let orange = egui::Color32::from_rgb(r, g, b);
    let shapes = common::painted(&h);
    let cut_lines = shapes
        .iter()
        .filter(|s| matches!(s, egui::Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(orange)))
        .count();
    assert!(cut_lines >= 1, "no cut line in the cut orange");
}

#[test]
fn weeding_controls_set_the_job_and_dim_what_does_not_apply() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    // The Weeding section of the Job panel starts folded.
    s.lock()
        .unwrap()
        .prefs
        .panel_sections
        .insert("job.weeding".into(), true);
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    // Each weeding change starts the kept and waste worker; it asks for a
    // repaint when done, which on a busy machine comes after a few frames.
    // Settle by clock, not by a count of frames.
    common::settle(&mut h);
    let disabled =
        |h: &AppHarness, label: &str| h.get_by_label(label).accesskit_node().is_disabled();
    // Off by default: the box margin, per object and the spacing are dimmed.
    assert!(disabled(&h, "Box margin"));
    assert!(disabled(&h, "One box per object"));
    assert!(disabled(&h, "Line spacing"));
    assert!(!disabled(&h, "Weed box"));
    let before = common::undo_len(&s);
    h.get_by_label("Weed box").click();
    common::settle(&mut h);
    let state = run(&s, "document.state", json!({}));
    assert_eq!(state["job"]["weed"]["box"], true);
    assert_eq!(common::undo_len(&s), before + 1, "one undo step");
    assert!(!disabled(&h, "Box margin"));
    assert!(!disabled(&h, "One box per object"));
    // The lines combo picks a direction; then the spacing applies.
    h.get_by_label("Weed lines").click();
    common::settle(&mut h);
    h.get_by_label("Both").click();
    common::settle(&mut h);
    let state = run(&s, "document.state", json!({}));
    assert_eq!(state["job"]["weed"]["lines"], "both");
    assert!(!disabled(&h, "Line spacing"));
    // The margin also sets where the lines stop: with the box off and the
    // lines on it stays enabled; with both off it is dimmed.
    run(&s, "job.set_weeding", json!({ "box": false }));
    common::settle(&mut h);
    assert!(!disabled(&h, "Box margin"));
    assert!(disabled(&h, "One box per object"));
    run(&s, "job.set_weeding", json!({ "lines": "off" }));
    common::settle(&mut h);
    assert!(disabled(&h, "Box margin"));
}

#[test]
fn path_stage_counts_and_dashes_weed_paths() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    stage(&mut h, "Path");
    let dashes = |h: &AppHarness| {
        common::painted(h)
            .iter()
            .filter(|s| matches!(s, egui::Shape::LineSegment { .. }))
            .count()
    };
    // Weeding off: the stat and the legend row stay visible, dimmed.
    let off = |h: &AppHarness, label: &str| h.get_by_label(label).accesskit_node().is_disabled();
    h.get_by_label(flatcraft::ui::cut::LEGEND_TRAVEL.0);
    assert!(off(&h, "Weed paths"));
    assert!(off(&h, flatcraft::ui::cut::LEGEND_WEED.0));
    assert!(!off(&h, flatcraft::ui::cut::LEGEND_TRAVEL.0));
    let plain = dashes(&h);
    run(
        &s,
        "job.set_weeding",
        json!({ "box": true, "margin_mm": 10, "lines": "both", "spacing_mm": 20 }),
    );
    h.run_plan();
    assert!(!off(&h, "Weed paths"));
    assert!(!off(&h, flatcraft::ui::cut::LEGEND_WEED.0));
    // The weed box is drawn in dashes: many more line segments, in the
    // weed blue of the theme on this mat.
    assert!(dashes(&h) > plain + 20, "{} vs {plain}", dashes(&h));
    let t = app(&h).tokens;
    let blue = flatcraft::ui::canvas::weed_rgb(flatcraft_device::DEFAULT_MAT_COLOR_DARK, &t);
    let blue_dashes = common::painted(&h)
        .iter()
        .filter(|s| matches!(s, egui::Shape::LineSegment { stroke, .. } if stroke.color == blue))
        .count();
    assert!(blue_dashes > 20, "{blue_dashes} blue dashes");
}

/// One source: the window builds the cut geometry outside the session
/// lock and stores it in the document's cache; the engine then reads that
/// same build, and a move makes both build again.
#[test]
fn the_window_and_the_engine_share_the_cut_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    let cached = s.lock().unwrap().doc().cached_cut_geometry().unwrap();
    let engine = s.lock().unwrap().cut_geometry().unwrap();
    assert!(Arc::ptr_eq(&cached, &engine));
    run(&s, "object.move", json!({ "ids": [id], "dx_mm": 3.0 }));
    let before = s.lock().unwrap().doc().cached_cut_geometry().unwrap();
    assert!(
        Arc::ptr_eq(&cached, &before),
        "the move itself built nothing"
    );
    h.run_plan();
    let moved = s.lock().unwrap().doc().cached_cut_geometry().unwrap();
    assert!(!Arc::ptr_eq(&cached, &moved), "the window built it again");
    assert!(Arc::ptr_eq(
        &moved,
        &s.lock().unwrap().cut_geometry().unwrap()
    ));
}
