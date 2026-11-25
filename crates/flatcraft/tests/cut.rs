//! The Cut workspace, driven through egui_kittest.

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{AppHarness, FakePicker, app, app_mut, harness, harness_with, import_square, session};
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::Workspace;
use flatcraft::ui::cut::{Stage, mm_text};
use flatcraft::ui::files::Revealer;
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
    h.run();
}

fn stage(h: &mut AppHarness, name: &str) {
    h.get_by_role_and_label(Role::Tab, name).click();
    h.run();
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
    h.run();
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
    assert!(toast.text.contains(".gpgl.txt"), "{}", toast.text);
    let gpgl = toast.reveal.clone().unwrap();
    assert_eq!(gpgl.extension().unwrap(), "gpgl");
    let label = flatcraft::ui::files::reveal_label();
    h.get_by_label(label).click();
    h.run();
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
    h.run();
    assert!(h.query_by_label("No device (File Output)").is_some());
    assert!(h.query_by_label("home").is_none());
    h.get_by_role_and_label(Role::Button, "Send to File Output")
        .click();
    h.run();
    h.run();
    let rows = &app(&h).ui.debug.log_rows;
    assert!(!rows.is_empty());
    assert_eq!(rows.last().unwrap().meaning, "home");
    assert!(h.query_by_label("home").is_some());
    // The listing of the current job.
    h.get_by_role_and_label(Role::Tab, "Listing").click();
    h.run();
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

#[test]
fn simulate_stage_is_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s, false);
    to_cut(&mut h);
    let node = h.get_by_role_and_label(Role::Tab, "Simulate");
    assert!(node.accesskit_node().is_disabled());
    node.click();
    h.run();
    assert_eq!(app(&h).ui.cut.stage, Stage::Device);
    assert!(h.query_by_role_and_label(Role::Tab, "Path").is_some());
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
        h.query_by_label_contains("These objects are outside the cutting area: square.")
            .is_some()
    );
    assert!(h.query_by_label_contains("Refused").is_none());
    stage(&mut h, "Send");
    let send = h.get_by_role_and_label(Role::Button, "Send to File Output");
    assert!(send.accesskit_node().is_disabled());
    send.click();
    h.run();
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
    h.run();
    assert_eq!(app(&h).ui.cut.visible_toolpaths(), 1);
    assert!(h.query_by_label("1 of 3").is_some());
    // A changed job starts over with all toolpaths shown.
    run(&s, "job.set_overrides", json!({ "passes": 2 }));
    h.run();
    assert_eq!(app(&h).ui.cut.visible_toolpaths(), 6);
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
    h.run();
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

const QUESTION: &str = "An agent wants to start a cut on Silhouette Cameo 2. Allow?";

#[test]
fn agent_confirmation_dialog_allows() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    agent_request(&s);
    let mut h = harness(&dir, s.clone(), true);
    // The dialog shows in every workspace, Home included.
    assert!(h.query_by_label(QUESTION).is_some());
    h.get_by_role_and_label(Role::Button, "Allow").click();
    h.run();
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
    h.run();
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
    let line = format!("1 path, {} cut", mm_text(snapshot.summary.cut_mm));
    assert!(h.query_by_label(&line).is_some(), "{line}");
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
    h.run();
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
    h.run();
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
    h.run();
    h.get_by_role_and_label(Role::Tab, "Capture").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Open capture...")
        .click();
    h.run();
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
    h.run();
    h.get_by_role_and_label(Role::Tab, "Capture").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Open capture...")
        .click();
    h.run();
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
    h.run();
    h.get_by_role_and_label(Role::Tab, "Listing").click();
    h.run();
    assert!(
        h.query_by_label("Preview of the current job (.gpgl.txt)")
            .is_some()
    );
    let last = h.get_by_role_and_label(Role::RadioButton, "Last sent");
    assert!(last.accesskit_node().is_disabled());
    h.get_by_role_and_label(Role::Button, "Send to File Output")
        .click();
    h.run();
    // The document changes after the send: the preview follows it, the
    // last sent listing does not.
    run(&s, "job.set_overrides", json!({ "speed": 2 }));
    h.run();
    h.get_by_role_and_label(Role::RadioButton, "Last sent")
        .click();
    h.run();
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
    h.run();
    assert!(h.query_by_label("!2\t# speed 2").is_some());
}
