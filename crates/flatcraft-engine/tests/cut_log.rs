//! The cut log: every completed send appends an entry to
//! `<config dir>/cut-log.jsonl`; only the person records how it went.

use flatcraft_device::ProfileStore;
use flatcraft_engine::cutlog::{self, Target, Verdict};
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::registry;
use flatcraft_engine::session::{CommandError, Origin, Session};
use serde_json::{Value, json};

fn session(dir: &tempfile::TempDir) -> Session {
    let profiles = ProfileStore::load(dir.path()).unwrap();
    let prefs = Preferences {
        output_dir: Some(dir.path().join("output")),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, false, dir.path().to_path_buf())
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

const SQUARE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="20" y="20" width="50" height="50"/></svg>"#;

/// A document with one square on File Output and vinyl, force 12.
fn ready(dir: &tempfile::TempDir) -> Session {
    let mut s = session(dir);
    run(
        &mut s,
        "object.import_svg",
        json!({ "svg": SQUARE, "name": "square" }),
    );
    run(
        &mut s,
        "job.set_plotter",
        json!({ "id": "file-output-gpgl-cameo2" }),
    );
    run(&mut s, "job.set_material", json!({ "id": "vinyl" }));
    run(&mut s, "job.set_overrides", json!({ "force": 12 }));
    s
}

fn log_text(dir: &tempfile::TempDir) -> String {
    std::fs::read_to_string(dir.path().join(cutlog::FILE)).unwrap_or_default()
}

#[test]
fn a_completed_send_appends_an_entry() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready(&dir);
    let path = dir.path().join("coasters.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    let sent = run(&mut s, "job.send", json!({}));
    let log = cutlog::read(dir.path());
    assert_eq!(log.diagnostics, Vec::<String>::new());
    assert_eq!(log.entries.len(), 1);
    let e = &log.entries[0];
    assert_eq!(sent["cut_log_entry"], e.id.as_str());
    assert_eq!(e.document, "coasters");
    assert_eq!(e.path.as_deref(), Some(path.as_path()));
    assert_eq!(e.plotter_id, "file-output-gpgl-cameo2");
    assert_eq!(e.plotter, "File Output (GPGL Cameo 2)");
    assert_eq!(e.material_id, "vinyl");
    assert_eq!(e.material, "Vinyl");
    assert!(!e.mat_id.is_empty() && !e.mat.is_empty());
    assert_eq!(e.force, 12);
    assert!(e.speed >= 1 && e.passes >= 1);
    assert_eq!(e.strategy, s.doc().document.job.strategy);
    assert_eq!(e.size.bytes, sent["bytes_sent"].as_u64().unwrap() as usize);
    assert_eq!(
        e.size.toolpaths,
        sent["toolpaths"].as_u64().unwrap() as usize
    );
    assert_eq!(e.keep_mode.as_deref(), Some("pieces"));
    match &e.target {
        Target::File { files } => assert_eq!(files.len(), 3, "{files:?}"),
        other => panic!("{other:?}"),
    }
    assert!(e.date.ends_with('Z') && e.date.len() >= 20, "{}", e.date);
    assert_eq!(e.outcome, None);
    assert!(
        !cutlog::asks_after(&e.target),
        "File Output cut nothing yet"
    );

    // An agent's send is a completed send too.
    s.execute("job.send", json!({}), Origin::Agent).unwrap();
    assert_eq!(cutlog::read(dir.path()).entries.len(), 2);
    assert_eq!(s.cut_log().entries.len(), 2, "the cached log follows");
}

#[test]
fn a_send_that_fails_appends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    // Nothing to cut.
    assert!(s.execute("job.send", json!({}), Origin::User).is_err());
    assert_eq!(log_text(&dir), "");
}

#[test]
fn the_person_records_how_it_went_and_the_newest_outcome_counts() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready(&dir);
    run(&mut s, "job.send", json!({}));
    let id = s.cut_log().entries[0].id.clone();
    s.record_outcome(&id, Verdict::Bad, "did not cut through")
        .unwrap();
    s.record_outcome(&id, Verdict::Good, "force 14 did it")
        .unwrap();
    let log = cutlog::read(dir.path());
    let o = log.entries[0].outcome.as_ref().unwrap();
    assert_eq!(o.verdict, Verdict::Good);
    assert_eq!(o.note, "force 14 did it");
    assert_eq!(
        s.cut_log().entries[0].outcome.as_ref().unwrap().verdict,
        Verdict::Good
    );
    // Append-only: the first cut line and both outcomes are all there.
    assert_eq!(log_text(&dir).lines().count(), 3);
    assert!(s.record_outcome("nope", Verdict::Good, "").is_err());
    assert_eq!(log_text(&dir).lines().count(), 3);
}

#[test]
fn last_time_prefers_the_newest_cut_with_an_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready(&dir);
    run(&mut s, "job.send", json!({}));
    let first = s.cut_log().entries[0].id.clone();
    run(&mut s, "job.send", json!({}));
    let second = s.cut_log().entries[1].id.clone();
    // No outcome yet: the newest.
    assert_eq!(s.cut_log().last_time("vinyl").unwrap().id, second);
    s.record_outcome(&first, Verdict::Good, "").unwrap();
    assert_eq!(s.cut_log().last_time("vinyl").unwrap().id, first);
    // Another material has its own history.
    assert!(s.cut_log().last_time("cardstock").is_none());
}

#[test]
fn a_broken_line_is_skipped_with_a_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready(&dir);
    run(&mut s, "job.send", json!({}));
    // A hand edit, and a last line a crash cut short.
    let mut text = log_text(&dir);
    text.push_str("this is not json\n{\"kind\":\"cut\",\"id\":\"x\"");
    std::fs::write(dir.path().join(cutlog::FILE), text).unwrap();
    run(&mut s, "job.send", json!({}));
    let log = cutlog::read(dir.path());
    assert_eq!(log.entries.len(), 2, "both good cuts count");
    assert_eq!(log.diagnostics.len(), 2, "{:?}", log.diagnostics);
    assert!(
        log.diagnostics[0].contains("line 2"),
        "{:?}",
        log.diagnostics
    );
    assert!(
        log.diagnostics[1].contains("line 3"),
        "{:?}",
        log.diagnostics
    );
    let h = run(&mut s, "job.history", json!({}));
    assert_eq!(h["diagnostics"].as_array().unwrap().len(), 2);
}

#[test]
fn agents_read_the_history_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready(&dir);
    run(&mut s, "job.send", json!({}));
    run(&mut s, "job.set_material", json!({ "id": "cardstock" }));
    run(&mut s, "job.send", json!({}));
    let first = s.cut_log().entries[0].id.clone();
    s.record_outcome(&first, Verdict::Good, "clean").unwrap();
    let before = log_text(&dir);
    let h = s.execute("job.history", json!({}), Origin::Agent).unwrap();
    let entries = h["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["material_id"], "cardstock");
    assert_eq!(entries[1]["outcome"]["verdict"], "good");
    assert_eq!(entries[1]["outcome"]["note"], "clean");
    assert_eq!(entries[0]["outcome"], Value::Null);
    let h = s
        .execute(
            "job.history",
            json!({ "material_id": "vinyl", "limit": 5 }),
            Origin::Agent,
        )
        .unwrap();
    assert_eq!(h["entries"].as_array().unwrap().len(), 1);
    let h = s
        .execute("job.history", json!({ "limit": 1 }), Origin::Agent)
        .unwrap();
    assert_eq!(h["entries"].as_array().unwrap().len(), 1);
    assert_eq!(h["total"], 2);
    assert_eq!(log_text(&dir), before, "reading changes nothing");
}

#[test]
fn agents_cannot_write_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready(&dir);
    run(&mut s, "job.send", json!({}));
    let before = log_text(&dir);
    // No command records an outcome.
    for c in registry::all() {
        let schema = serde_json::to_value(c.params_schema()).unwrap().to_string();
        assert!(
            !schema.contains("verdict") && !c.id().contains("outcome"),
            "{} could record an outcome",
            c.id()
        );
    }
    let r = s.execute(
        "job.history",
        json!({ "outcome": { "verdict": "good" } }),
        Origin::Agent,
    );
    assert!(matches!(r, Err(CommandError::InvalidParams(_))), "{r:?}");
    // Nor can a file-writing command reach it.
    let log = dir.path().join(cutlog::FILE);
    let r = s.execute("project.save", json!({ "path": log }), Origin::Agent);
    assert!(matches!(r, Err(CommandError::Refused(_))), "{r:?}");
    let r = s.execute(
        "document.export",
        json!({ "path": log, "format": "svg", "scope": "all" }),
        Origin::Agent,
    );
    assert!(matches!(r, Err(CommandError::Refused(_))), "{r:?}");
    assert_eq!(log_text(&dir), before);
}

#[test]
fn the_history_names_files_but_never_gives_absolute_paths() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready(&dir);
    run(
        &mut s,
        "project.save",
        json!({ "path": dir.path().join("coasters.flatcraft") }),
    );
    run(&mut s, "job.send", json!({}));
    let h = s.execute("job.history", json!({}), Origin::Agent).unwrap();
    let e = &h["entries"][0];
    assert_eq!(e["path"], "coasters.flatcraft");
    let files = e["target"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    for f in files {
        let f = f.as_str().unwrap();
        assert!(!f.contains('/') && !f.contains('\\'), "{f}");
    }
    let text = h.to_string();
    assert!(!text.contains(&dir.path().display().to_string()), "{text}");
}

#[test]
fn the_cached_log_sees_a_send_from_another_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = ready(&dir);
    let mut b = ready(&dir);
    assert_eq!(a.cut_log().entries.len(), 0);
    run(&mut b, "job.send", json!({}));
    assert_eq!(a.cut_log().entries.len(), 1, "another session's send shows");
    run(&mut a, "job.send", json!({}));
    assert_eq!(b.cut_log().entries.len(), 2);
}

#[test]
fn agents_see_damaged_lines_without_their_content() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready(&dir);
    run(&mut s, "job.send", json!({}));
    let mut text = log_text(&dir);
    text.push_str("{\"kind\":\"cut\",\"id\":\"x\",\"date\":7,\"document\":\"/Users/someone/secret/plan.flatcraft\"}\n");
    text.push_str("{\"kind\":\"nope\",\"path\":\"/Users/someone/secret\"}\n");
    std::fs::write(dir.path().join(cutlog::FILE), text).unwrap();
    let h = s.execute("job.history", json!({}), Origin::Agent).unwrap();
    let out = h.to_string();
    assert!(!out.contains("/Users/someone"), "{out}");
    assert_eq!(
        h["diagnostics"],
        json!([
            "cut-log.jsonl line 2 was skipped (damaged)",
            "cut-log.jsonl line 3 was skipped (damaged)"
        ])
    );
    // The window keeps the detailed reasons.
    assert_eq!(cutlog::read(dir.path()).diagnostics.len(), 2);
}

#[test]
fn the_entry_records_the_keep_mode_of_the_cut_objects() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready(&dir);
    run(
        &mut s,
        "object.set_keep",
        json!({ "ids": [1], "keep": "stencil" }),
    );
    run(&mut s, "job.send", json!({}));
    let r = run(
        &mut s,
        "object.import_svg",
        json!({ "svg": SQUARE.replace("x=\"20\"", "x=\"120\""), "name": "other" }),
    );
    let other = r["id"].as_u64().unwrap();
    run(&mut s, "job.send", json!({}));
    // A skipped object does not count.
    run(
        &mut s,
        "object.set_cut",
        json!({ "ids": [other], "cut": false }),
    );
    run(&mut s, "job.send", json!({}));
    let log = cutlog::read(dir.path());
    let modes: Vec<Option<&str>> = log.entries.iter().map(|e| e.keep_mode.as_deref()).collect();
    assert_eq!(modes, [Some("stencil"), Some("mixed"), Some("stencil")]);
}
