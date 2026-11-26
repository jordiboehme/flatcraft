//! The import report and the reasons a job has nothing to cut, as agents
//! get them: stable codes, severities, object ids, plain messages and
//! fixes that are registry commands. Every offered fix is run as an agent
//! and must resolve its issue.

use flatcraft_device::ProfileStore;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::registry;
use flatcraft_engine::session::{Origin, Session};
use serde_json::{Value, json};

fn session(dir: &tempfile::TempDir, out: &tempfile::TempDir) -> Session {
    let profiles = ProfileStore::load(dir.path()).unwrap();
    let prefs = Preferences {
        output_dir: Some(out.path().to_path_buf()),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, false, dir.path().to_path_buf())
}

struct T {
    s: Session,
    _dir: tempfile::TempDir,
    _out: tempfile::TempDir,
}

fn t() -> T {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    T {
        s: session(&dir, &out),
        _dir: dir,
        _out: out,
    }
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

fn svg(body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100">{body}</svg>"#
    )
}

/// Imports SVG markup; returns the id and the whole result.
fn import(s: &mut Session, name: &str, body: &str) -> (u64, Value) {
    let r = run(
        s,
        "object.import_svg",
        json!({ "name": name, "svg": svg(body) }),
    );
    (r["id"].as_u64().unwrap(), r)
}

fn issues(v: &Value) -> Vec<Value> {
    v["issues"].as_array().cloned().unwrap_or_default()
}

fn codes(v: &Value) -> Vec<String> {
    issues(v)
        .iter()
        .map(|i| i["code"].as_str().unwrap().to_string())
        .collect()
}

fn issue(v: &Value, code: &str) -> Value {
    issues(v)
        .into_iter()
        .find(|i| i["code"] == code)
        .unwrap_or_else(|| panic!("no {code} in {v}"))
}

fn check(s: &mut Session) -> Value {
    run(s, "document.check", json!({}))
}

/// Runs a fix as an agent would: one `execute` of its command and args.
fn apply(s: &mut Session, fix: &Value) {
    let command = fix["command"].as_str().unwrap();
    assert!(registry::get(command).is_some(), "{command}");
    s.execute(command, fix["args"].clone(), Origin::Agent)
        .unwrap_or_else(|e| panic!("{command} {}: {e}", fix["args"]));
}

/// Every issue has the fields agents read, plain text without em-dashes
/// and fixes with a label, a description, a command and args.
fn well_formed(v: &Value) {
    for i in issues(v) {
        for key in [
            "code",
            "severity",
            "object_ids",
            "paths",
            "message",
            "hint",
            "fixes",
        ] {
            assert!(i.get(key).is_some(), "{key} missing in {i}");
        }
        assert!(
            ["info", "warning", "error"].contains(&i["severity"].as_str().unwrap()),
            "{i}"
        );
        let text = i.to_string();
        assert!(!text.contains('\u{2014}'), "{text}");
        assert!(!i["message"].as_str().unwrap().is_empty());
        assert!(!i["hint"].as_str().unwrap().is_empty());
        for f in i["fixes"].as_array().unwrap() {
            assert!(f["label"].as_str().is_some_and(|l| !l.is_empty()), "{f}");
            assert!(
                f["description"].as_str().is_some_and(|l| !l.is_empty()),
                "{f}"
            );
            assert!(f["args"].is_object(), "{f}");
        }
    }
}

const RECT: &str = r#"<rect x="20" y="20" width="30" height="30"/>"#;

// ---------------------------------------------------------------- import report

#[test]
fn a_clean_file_reports_its_shapes_and_no_issues() {
    let mut t = t();
    let (_, r) = import(
        &mut t.s,
        "clean",
        &format!(
            r#"{RECT}<path d="M10 80 L90 85" fill="none" stroke="black" stroke-width="0.26"/>"#
        ),
    );
    assert_eq!(r["report"]["shapes"], 2, "{r}");
    assert_eq!(issues(&r["report"]), Vec::<Value>::new(), "{r}");
    assert_eq!(issues(&check(&mut t.s)), Vec::<Value>::new());
}

#[test]
fn an_outline_with_a_gap_is_an_open_path_without_an_automatic_fix() {
    let mut t = t();
    let (id, r) = import(
        &mut t.s,
        "gap",
        r#"<path d="M10 10 H40 V40 H10 V10.4" fill="none" stroke="black" stroke-width="0.2"/>"#,
    );
    well_formed(&r["report"]);
    let i = issue(&r["report"], "open_paths");
    assert_eq!(i["severity"], "warning");
    assert_eq!(i["object_ids"], json!([id]));
    assert_eq!(i["paths"], 1);
    assert_eq!(
        i["message"],
        "'gap': 1 shape is not closed, so the knife cuts a line there and the piece stays on the sheet."
    );
    assert_eq!(i["fixes"], json!([]));
    assert!(i["hint"].as_str().unwrap().contains("Close the gaps"));
}

#[test]
fn filled_open_paths_cut_as_lines_are_fixed_by_object_outlines() {
    let mut t = t();
    let (id, r) = import(&mut t.s, "tri", r#"<path d="M10 10 L40 10 L25 40"/>"#);
    // Object outlines (the default) cut the edge of the fill: closed.
    assert_eq!(codes(&r["report"]), Vec::<String>::new(), "{r}");
    run(
        &mut t.s,
        "object.set_cut_options",
        json!({ "ids": [id], "mode": "all_lines" }),
    );
    let c = check(&mut t.s);
    well_formed(&c);
    let i = issue(&c, "open_paths");
    let fixes = i["fixes"].as_array().unwrap();
    assert_eq!(fixes.len(), 1);
    assert_eq!(fixes[0]["label"], "Use object outlines");
    assert_eq!(fixes[0]["command"], "object.set_cut_options");
    assert_eq!(
        fixes[0]["args"],
        json!({ "ids": [id], "mode": "object_outlines" })
    );
    apply(&mut t.s, &fixes[0]);
    assert!(!codes(&check(&mut t.s)).contains(&"open_paths".to_string()));
}

#[test]
fn a_stroke_cut_on_both_edges_below_the_minimum_feature_is_a_hairline() {
    let mut t = t();
    let (id, r) = import(
        &mut t.s,
        "frame",
        r#"<rect x="20" y="20" width="30" height="30" fill="none" stroke="black" stroke-width="1"/>"#,
    );
    well_formed(&r["report"]);
    let i = issue(&r["report"], "hairline");
    assert_eq!(i["severity"], "warning");
    assert_eq!(i["paths"], 1);
    assert_eq!(
        i["message"],
        "'frame': 1 line is cut along both edges but only 1 mm wide. A strip thinner than 1.5 mm tears."
    );
    let fix = &i["fixes"][0];
    assert_eq!(fix["label"], "Cut as single lines");
    assert_eq!(
        fix["args"],
        json!({ "ids": [id], "strokes": "centre_line" })
    );
    apply(&mut t.s, fix);
    assert_eq!(codes(&check(&mut t.s)), Vec::<String>::new());
}

#[test]
fn tiny_shapes_are_below_the_minimum_feature_and_scale_up_when_all_are_tiny() {
    let mut t = t();
    let dots = r#"<circle cx="20" cy="20" r="0.4"/><circle cx="23" cy="20" r="0.5"/>"#;
    let (id, r) = import(&mut t.s, "dots", dots);
    well_formed(&r["report"]);
    let i = issue(&r["report"], "below_min_feature");
    assert_eq!(i["paths"], 2);
    assert_eq!(
        i["message"],
        "'dots': 2 shapes are smaller than 1.5 mm, the smallest shape this blade cuts cleanly. They may tear or not come off."
    );
    let fix = &i["fixes"][0];
    assert_eq!(fix["label"], "Scale up");
    assert_eq!(fix["command"], "object.scale");
    assert_eq!(fix["args"]["ids"], json!([id]));
    apply(&mut t.s, fix);
    assert!(!codes(&check(&mut t.s)).contains(&"below_min_feature".to_string()));
    // Idempotent: the size is absolute.
    let scale = |s: &mut Session| {
        run(s, "document.state", json!({}))["objects"][0]["transform"]["scale_x"]
            .as_f64()
            .unwrap()
    };
    let before = scale(&mut t.s);
    apply(&mut t.s, fix);
    assert!((scale(&mut t.s) - before).abs() < 1e-9);
}

#[test]
fn a_tiny_shape_next_to_big_ones_has_no_automatic_fix() {
    let mut t = t();
    let (_, r) = import(
        &mut t.s,
        "mixed",
        &format!(r#"{RECT}<circle cx="70" cy="70" r="0.4"/>"#),
    );
    let i = issue(&r["report"], "below_min_feature");
    assert_eq!(i["paths"], 1);
    assert_eq!(i["fixes"], json!([]));
    assert!(
        i["hint"]
            .as_str()
            .unwrap()
            .contains("remove the tiny shapes")
    );
}

#[test]
fn a_fill_without_area_is_ignored_by_object_outlines_and_cut_by_all_lines() {
    let mut t = t();
    let (id, r) = import(
        &mut t.s,
        "lines",
        &format!(r#"{RECT}<path d="M10 80 L90 80"/>"#),
    );
    well_formed(&r["report"]);
    let i = issue(&r["report"], "fill_ignored");
    assert_eq!(i["severity"], "warning");
    assert_eq!(i["paths"], 1);
    assert_eq!(
        i["message"],
        "'lines': 1 filled shape has no area or is thinner than 0.01 mm, so the cut mode Object outlines cuts nothing of it."
    );
    let fix = &i["fixes"][0];
    assert_eq!(fix["label"], "Cut all lines");
    assert_eq!(fix["args"], json!({ "ids": [id], "mode": "all_lines" }));
    // job.preview names it too, and the plan still cuts the rectangle.
    let p = run(&mut t.s, "job.preview", json!({}));
    assert_eq!(p["ready"], true);
    assert_eq!(codes(&p), ["fill_ignored"]);
    apply(&mut t.s, fix);
    assert_eq!(codes(&check(&mut t.s)), Vec::<String>::new());
    let p = run(&mut t.s, "job.preview", json!({}));
    assert_eq!(p["issues"], json!([]));
    assert_eq!(p["toolpaths"], 2);
}

#[test]
fn a_file_larger_than_the_mat_scales_to_fit_in_one_click() {
    let mut t = t();
    let r = run(
        &mut t.s,
        "object.import_svg",
        json!({ "name": "big", "svg": r#"<svg xmlns="http://www.w3.org/2000/svg" width="500mm" height="100mm" viewBox="0 0 500 100"><rect x="0" y="0" width="500" height="100"/></svg>"# }),
    );
    let id = r["id"].as_u64().unwrap();
    well_formed(&r["report"]);
    assert_eq!(codes(&r["report"]), ["larger_than_area"]);
    let i = issue(&r["report"], "larger_than_area");
    assert_eq!(i["severity"], "error");
    assert!(
        i["message"]
            .as_str()
            .unwrap()
            .starts_with("'big' is 500 x 100 mm, larger than the cutting area ("),
        "{i}"
    );
    let fix = &i["fixes"][0];
    assert_eq!(fix["label"], "Scale to fit the mat");
    assert_eq!(fix["command"], "object.scale");
    assert_eq!(fix["args"]["anchor"], "top_left");
    apply(&mut t.s, fix);
    assert_eq!(codes(&check(&mut t.s)), Vec::<String>::new());
    let p = run(&mut t.s, "job.preview", json!({}));
    assert_eq!(p["ready"], true, "{p}");
    let _ = id;
}

#[test]
fn an_object_off_the_mat_moves_onto_it() {
    let mut t = t();
    let (id, _) = import(&mut t.s, "sq", RECT);
    run(
        &mut t.s,
        "object.move",
        json!({ "ids": [id], "dx_mm": -100.0 }),
    );
    let c = check(&mut t.s);
    well_formed(&c);
    let i = issue(&c, "outside_area");
    assert_eq!(i["severity"], "error");
    assert_eq!(i["message"], "'sq' is outside the cutting area.");
    // job.preview says the same, as data, instead of failing.
    let p = run(&mut t.s, "job.preview", json!({}));
    assert_eq!(p["ready"], false);
    assert_eq!(p["nothing_to_cut"], false);
    assert!(
        p["reason"]
            .as_str()
            .unwrap()
            .contains("outside the cutting area")
    );
    assert_eq!(codes(&p), ["outside_area"]);
    let fix = &issue(&p, "outside_area")["fixes"][0];
    assert_eq!(fix["label"], "Move onto the mat");
    assert_eq!(fix["command"], "object.move");
    apply(&mut t.s, fix);
    assert_eq!(codes(&check(&mut t.s)), Vec::<String>::new());
    assert_eq!(run(&mut t.s, "job.preview", json!({}))["ready"], true);
}

#[test]
fn a_shape_sticking_out_says_so() {
    let mut t = t();
    let (id, _) = import(&mut t.s, "sq", RECT);
    run(
        &mut t.s,
        "object.move",
        json!({ "ids": [id], "x_mm": -10.0 }),
    );
    let c = check(&mut t.s);
    assert_eq!(
        issue(&c, "outside_area")["message"],
        "'sq' sticks out of the cutting area."
    );
}

#[test]
fn document_check_takes_ids() {
    let mut t = t();
    let (a, _) = import(&mut t.s, "a", RECT);
    let (b, _) = import(&mut t.s, "b", r#"<circle cx="20" cy="20" r="0.4"/>"#);
    assert_eq!(
        codes(&run(&mut t.s, "document.check", json!({ "ids": [a] }))),
        Vec::<String>::new()
    );
    let r = run(&mut t.s, "document.check", json!({ "ids": [b] }));
    assert_eq!(codes(&r), ["below_min_feature"]);
    assert_eq!(r["shapes"], 1);
    assert!(
        t.s.execute("document.check", json!({ "ids": [999] }), Origin::Agent)
            .is_err()
    );
}

#[test]
fn importing_several_files_reports_each_object() {
    let mut t = t();
    let dir = tempfile::tempdir().unwrap();
    let clean = dir.path().join("clean.svg");
    let dots = dir.path().join("dots.svg");
    std::fs::write(&clean, svg(RECT)).unwrap();
    std::fs::write(&dots, svg(r#"<circle cx="20" cy="20" r="0.4"/>"#)).unwrap();
    let r = run(&mut t.s, "object.import", json!({ "paths": [clean, dots] }));
    let ids = r["imported"].as_array().unwrap().clone();
    assert_eq!(r["report"]["shapes"], 2, "{r}");
    assert_eq!(codes(&r["report"]), ["below_min_feature"]);
    assert_eq!(
        issue(&r["report"], "below_min_feature")["object_ids"],
        json!([ids[1]])
    );
    // One file with path: the same report.
    let one = run(&mut t.s, "object.import", json!({ "path": dots }));
    assert_eq!(codes(&one["report"]), ["below_min_feature"]);
}

#[test]
fn a_traced_image_reports_what_the_trace_did() {
    let mut t = t();
    // A white image: nothing to trace.
    let img = image::RgbImage::from_pixel(40, 40, image::Rgb([255, 255, 255]));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blank.png");
    img.save(&path).unwrap();
    let r = run(&mut t.s, "object.import", json!({ "path": path }));
    well_formed(&r["report"]);
    let i = issue(&r["report"], "nothing_traced");
    assert_eq!(i["severity"], "warning");
    assert_eq!(i["fixes"], json!([]));
    assert!(
        i["message"]
            .as_str()
            .unwrap()
            .starts_with("Nothing was traced")
    );
}

// ---------------------------------------------------------------- nothing to cut

fn preview(s: &mut Session) -> Value {
    let p = run(s, "job.preview", json!({}));
    well_formed(&p);
    p
}

#[test]
fn an_empty_document_has_nothing_to_cut_and_no_fix() {
    let mut t = t();
    let p = preview(&mut t.s);
    assert_eq!(p["ready"], false);
    assert_eq!(p["nothing_to_cut"], true);
    assert_eq!(p["toolpaths"], 0);
    assert_eq!(
        p["reason"],
        "Nothing to cut: no visible object has cut paths."
    );
    let i = issue(&p, "empty_document");
    assert_eq!(i["severity"], "error");
    assert_eq!(i["object_ids"], json!([]));
    assert_eq!(i["fixes"], json!([]));
    assert!(i["hint"].as_str().unwrap().contains("Import"));
    // job.send still refuses with the same words.
    assert!(t.s.execute("job.send", json!({}), Origin::User).is_err());
}

#[test]
fn hidden_and_skipped_objects_each_name_their_reason_and_fix() {
    let mut t = t();
    let (a, _) = import(&mut t.s, "a", RECT);
    let (b, _) = import(&mut t.s, "b", RECT);
    run(
        &mut t.s,
        "object.set_visible",
        json!({ "ids": [a], "visible": false }),
    );
    run(
        &mut t.s,
        "object.set_cut",
        json!({ "ids": [b], "cut": false }),
    );
    let p = preview(&mut t.s);
    assert_eq!(p["nothing_to_cut"], true);
    assert_eq!(codes(&p), ["hidden", "skipped"]);
    let hidden = issue(&p, "hidden");
    assert_eq!(
        hidden["message"],
        "'a' is hidden, and hidden objects are not cut."
    );
    assert_eq!(hidden["object_ids"], json!([a]));
    assert_eq!(
        hidden["fixes"][0]["args"],
        json!({ "ids": [a], "visible": true })
    );
    let skipped = issue(&p, "skipped");
    assert_eq!(
        skipped["message"],
        "'b' is set to skip, so the knife leaves it out."
    );
    apply(&mut t.s, &hidden["fixes"][0]);
    let p = preview(&mut t.s);
    assert_eq!(p["ready"], true, "{p}");
    run(
        &mut t.s,
        "object.set_visible",
        json!({ "ids": [a], "visible": false }),
    );
    apply(&mut t.s, &skipped["fixes"][0]);
    let p = preview(&mut t.s);
    assert_eq!(p["ready"], true, "{p}");
}

#[test]
fn only_fills_without_area_say_the_cut_mode_ignores_them() {
    let mut t = t();
    let (id, _) = import(&mut t.s, "line", r#"<path d="M10 80 L90 80"/>"#);
    let p = preview(&mut t.s);
    assert_eq!(p["nothing_to_cut"], true);
    assert_eq!(codes(&p), ["fill_ignored"]);
    let i = issue(&p, "fill_ignored");
    assert_eq!(i["severity"], "error");
    assert_eq!(i["object_ids"], json!([id]));
    apply(&mut t.s, &i["fixes"][0]);
    assert_eq!(preview(&mut t.s)["ready"], true);
}

#[test]
fn a_job_too_tiny_to_move_the_plotter_scales_up() {
    let mut t = t();
    let (id, _) = import(
        &mut t.s,
        "speck",
        r#"<path d="M60 60 L60.02 60" stroke="black" fill="none"/>"#,
    );
    run(
        &mut t.s,
        "object.set_cut_options",
        json!({ "ids": [id], "mode": "all_lines" }),
    );
    let p = preview(&mut t.s);
    assert_eq!(p["nothing_to_cut"], true);
    assert_eq!(codes(&p), ["too_small"]);
    let i = issue(&p, "too_small");
    assert_eq!(i["object_ids"], json!([id]));
    assert_eq!(i["fixes"][0]["command"], "object.scale");
    apply(&mut t.s, &i["fixes"][0]);
    assert_eq!(preview(&mut t.s)["ready"], true);
}

#[test]
fn a_ready_job_has_no_issues() {
    let mut t = t();
    import(&mut t.s, "sq", RECT);
    let p = preview(&mut t.s);
    assert_eq!(p["ready"], true);
    assert_eq!(p["nothing_to_cut"], false);
    assert_eq!(p["issues"], json!([]));
    assert!(p["toolpaths"].as_u64().unwrap() > 0);
}

#[test]
fn the_codes_are_documented_for_agents() {
    let desc = |id: &str| registry::get(id).unwrap().description();
    for code in flatcraft_engine::report::Code::ALL {
        assert!(
            desc("document.check").contains(code.as_str()),
            "{} is not in document.check",
            code.as_str()
        );
    }
    for id in ["object.import", "object.import_svg", "job.preview"] {
        assert!(desc(id).contains("document.check"), "{id}");
    }
}
