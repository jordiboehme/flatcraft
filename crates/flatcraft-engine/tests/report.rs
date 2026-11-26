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
    // By default a stroke under the minimum feature size is cut once.
    assert!(
        !codes(&r["report"]).contains(&"hairline".to_string()),
        "{r}"
    );
    outline_strokes(&mut t.s, id);
    let r = json!({ "report": check(&mut t.s) });
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
    let (b, _) = import(&mut t.s, "b", r#"<circle cx="80" cy="80" r="0.4"/>"#);
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
    std::fs::write(&dots, svg(r#"<circle cx="80" cy="80" r="0.4"/>"#)).unwrap();
    let r = run(&mut t.s, "object.import", json!({ "paths": [clean, dots] }));
    let ids = r["imported"].as_array().unwrap().clone();
    assert_eq!(r["report"]["shapes"], 2, "{r}");
    assert_eq!(codes(&r["report"]), ["below_min_feature"]);
    assert_eq!(
        issue(&r["report"], "below_min_feature")["object_ids"],
        json!([ids[1]])
    );
    // One file with path: the same report, and the new dot lies on the
    // first one.
    let one = run(&mut t.s, "object.import", json!({ "path": dots }));
    assert_eq!(
        codes(&one["report"]),
        ["below_min_feature", "objects_overlap"]
    );
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

// ---------------------------------------------------------------- fix round 1

fn area(s: &mut Session) -> [f64; 4] {
    let a = &run(s, "document.state", json!({}))["cutting_area_mm"];
    [
        a["x0"].as_f64().unwrap(),
        a["y0"].as_f64().unwrap(),
        a["x1"].as_f64().unwrap(),
        a["y1"].as_f64().unwrap(),
    ]
}

/// An object a hair (5e-7 mm) wider or taller than the cutting area, off
/// the mat: it counts as fitting, and the move fix clamps it in without a
/// panic.
#[test]
fn an_object_a_hair_wider_than_the_area_moves_without_a_panic() {
    for wide in [true, false] {
        let mut t = t();
        let (id, _) = import(&mut t.s, "sq", RECT);
        let [x0, y0, x1, y1] = area(&mut t.s);
        let size = if wide {
            json!({ "ids": [id], "width_mm": x1 - x0 + 5e-7, "keep_aspect": false })
        } else {
            json!({ "ids": [id], "height_mm": y1 - y0 + 5e-7, "keep_aspect": false })
        };
        run(&mut t.s, "object.scale", size);
        let off = if wide {
            json!({ "ids": [id], "dy_mm": 1000.0 })
        } else {
            json!({ "ids": [id], "dx_mm": 1000.0 })
        };
        run(&mut t.s, "object.move", off);
        let c = check(&mut t.s);
        assert_eq!(codes(&c), ["outside_area"], "{c}");
        let p = preview(&mut t.s);
        assert_eq!(p["ready"], false);
        assert_eq!(codes(&p), ["outside_area"]);
        let e =
            t.s.execute("job.send", json!({}), Origin::User)
                .unwrap_err();
        assert_eq!(
            e.to_string(),
            "Refused: These objects are outside the cutting area: sq. Move or scale them, or pick a bigger mat."
        );
        apply(&mut t.s, &issue(&c, "outside_area")["fixes"][0]);
        assert_eq!(codes(&check(&mut t.s)), Vec::<String>::new());
        assert_eq!(preview(&mut t.s)["ready"], true);
    }
}

/// Applies the first offered fix until none is offered; returns the steps.
fn fix_until_done(s: &mut Session, limit: usize) -> usize {
    for step in 0..=limit {
        let c = check(s);
        let p = run(s, "job.preview", json!({}));
        let fix = issues(&c)
            .into_iter()
            .chain(issues(&p))
            .flat_map(|i| i["fixes"].as_array().cloned().unwrap_or_default())
            .next();
        match fix {
            Some(f) => apply(s, &f),
            None => return step,
        }
    }
    panic!("fixes did not end within {limit} steps: {}", check(s));
}

const TRIANGLE: &str = r#"<path d="M10 10 L40 10 L25 40"/>"#;
const FLAT: &str = r#"<path d="M10 80 L90 80"/>"#;

#[test]
fn open_paths_and_fill_ignored_never_undo_each_other() {
    for mode in ["all_lines", "object_outlines"] {
        let mut t = t();
        let (id, _) = import(&mut t.s, "mix", &format!("{TRIANGLE}{FLAT}"));
        run(
            &mut t.s,
            "object.set_cut_options",
            json!({ "ids": [id], "mode": mode }),
        );
        let c = check(&mut t.s);
        // The fix that would raise the other issue is withheld, with a hint.
        let code = if mode == "all_lines" {
            "open_paths"
        } else {
            "fill_ignored"
        };
        let i = issue(&c, code);
        assert_eq!(i["fixes"], json!([]), "{i}");
        assert!(i["hint"].as_str().unwrap().contains("own object"), "{i}");
        assert!(fix_until_done(&mut t.s, 8) <= 8);
    }
}

/// An object 1.5e-6 mm wider (or taller) than the area: more than the
/// slack, so it is larger than the area and "Scale to fit" resolves it.
#[test]
fn an_object_just_over_the_slack_scales_to_fit() {
    for wide in [true, false] {
        let mut t = t();
        hair_off_the_mat(&mut t.s, wide, 1.5e-6);
        let c = check(&mut t.s);
        assert_eq!(codes(&c), ["larger_than_area"], "{c}");
        let p = preview(&mut t.s);
        assert_eq!(codes(&p), ["larger_than_area"], "{p}");
        // Scale to fit, then (it is off the mat) move onto it.
        assert!(fix_until_done(&mut t.s, 4) <= 2);
        assert_eq!(codes(&check(&mut t.s)), Vec::<String>::new());
        assert_eq!(preview(&mut t.s)["ready"], true);
    }
}

/// Imports a square, makes it `over` mm wider (or taller) than the area
/// and moves it off the mat the other way.
fn hair_off_the_mat(s: &mut Session, wide: bool, over: f64) -> u64 {
    let (id, _) = import(s, "sq", RECT);
    let [x0, y0, x1, y1] = area(s);
    let size = if wide {
        json!({ "ids": [id], "width_mm": x1 - x0 + over, "keep_aspect": false })
    } else {
        json!({ "ids": [id], "height_mm": y1 - y0 + over, "keep_aspect": false })
    };
    run(s, "object.scale", size);
    let off = if wide {
        json!({ "ids": [id], "dy_mm": 1000.0 })
    } else {
        json!({ "ids": [id], "dx_mm": 1000.0 })
    };
    run(s, "object.move", off);
    id
}

#[test]
fn a_group_too_large_for_the_mat_gets_one_scale_to_fit_in_preview() {
    let mut t = t();
    let r = run(
        &mut t.s,
        "object.import_svg",
        json!({ "name": "big", "svg": r#"<svg xmlns="http://www.w3.org/2000/svg" width="500mm" height="100mm" viewBox="0 0 500 100"><rect width="500" height="100"/></svg>"# }),
    );
    let id = r["id"].as_u64().unwrap();
    run(
        &mut t.s,
        "object.copies",
        json!({ "id": id, "count": 2, "linked": true }),
    );
    let p = preview(&mut t.s);
    let large: Vec<Value> = issues(&p)
        .into_iter()
        .filter(|i| i["code"] == "larger_than_area")
        .collect();
    assert_eq!(large.len(), 1, "{p}");
    assert_eq!(large[0]["object_ids"].as_array().unwrap().len(), 3);
}

#[test]
fn offered_fixes_always_come_to_an_end() {
    // Objects a hair over the area, between the slacks and beyond.
    for over in [5e-7, 1e-6, 1.5e-6, 2.5e-6] {
        for wide in [true, false] {
            let mut t = t();
            hair_off_the_mat(&mut t.s, wide, over);
            assert!(fix_until_done(&mut t.s, 10) <= 10, "{over} {wide}");
            assert_eq!(
                codes(&check(&mut t.s)),
                Vec::<String>::new(),
                "{over} {wide}"
            );
        }
    }
    let bodies = [
        format!("{TRIANGLE}{FLAT}"),
        format!("{TRIANGLE}{FLAT}{FRAME}"),
        format!(
            r#"{TRIANGLE}<path d="M60 10 H90 V40 H60 V10.4" fill="none" stroke="black" stroke-width="0.2"/>"#
        ),
        r#"<circle cx="10" cy="10" r="0.3"/><circle cx="95" cy="95" r="0.3"/>"#.to_string(),
        format!("{FRAME}{FLAT}"),
    ];
    for body in &bodies {
        for mode in ["all_lines", "object_outlines"] {
            let mut t = t();
            let (id, _) = import(&mut t.s, "mix", body);
            run(
                &mut t.s,
                "object.set_cut_options",
                json!({ "ids": [id], "mode": mode }),
            );
            run(
                &mut t.s,
                "object.move",
                json!({ "ids": [id], "dx_mm": -150.0 }),
            );
            assert!(fix_until_done(&mut t.s, 10) <= 10, "{body} {mode}");
        }
    }
}

const FRAME: &str =
    r#"<rect x="60" y="60" width="30" height="30" fill="none" stroke="black" stroke-width="1"/>"#;

/// Cuts every stroke of object `id` around its outline: the person's own
/// choice, so a thin stroke becomes a hairline the report names.
fn outline_strokes(s: &mut Session, id: u64) {
    run(
        s,
        "object.set_cut_options",
        json!({ "ids": [id], "strokes": "outline" }),
    );
}

#[test]
fn object_outlines_is_offered_for_filled_shapes_even_with_gaps() {
    let mut t = t();
    let (id, _) = import(
        &mut t.s,
        "both",
        &format!(
            r#"{TRIANGLE}<path d="M60 10 H90 V40 H60 V10.4" fill="none" stroke="black" stroke-width="0.2"/>"#
        ),
    );
    run(
        &mut t.s,
        "object.set_cut_options",
        json!({ "ids": [id], "mode": "all_lines" }),
    );
    let i = issue(&check(&mut t.s), "open_paths");
    assert_eq!(i["paths"], 2);
    let fix = &i["fixes"][0];
    assert_eq!(fix["label"], "Use object outlines");
    assert!(
        fix["description"]
            .as_str()
            .unwrap()
            .contains("1 shape with a gap stay")
    );
    apply(&mut t.s, fix);
    let i = issue(&check(&mut t.s), "open_paths");
    assert_eq!(i["paths"], 1);
    assert_eq!(i["fixes"], json!([]));
}

#[test]
fn reload_relink_and_retrace_return_the_report() {
    let mut t = t();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("frame.svg");
    std::fs::write(&file, svg(RECT)).unwrap();
    let id = run(&mut t.s, "object.import", json!({ "path": file }))["id"]
        .as_u64()
        .unwrap();
    outline_strokes(&mut t.s, id);
    // The file changes: now it has a dashed line and a hairline frame.
    let dashed = format!(
        r#"<path d="M10 10 H90" fill="none" stroke="black" stroke-width="0.2" stroke-dasharray="2 1"/>{FRAME}"#
    );
    std::fs::write(&file, svg(&dashed)).unwrap();
    let r = run(&mut t.s, "object.reload", json!({ "ids": [id] }));
    well_formed(&r["report"]);
    assert_eq!(codes(&r["report"]), ["dashes_ignored", "hairline"], "{r}");
    // Relink to another copy of the file.
    let other = dir.path().join("moved.svg");
    std::fs::write(&other, svg(RECT)).unwrap();
    let r = run(
        &mut t.s,
        "object.relink",
        json!({ "id": id, "path": other }),
    );
    assert_eq!(r["report"]["issues"], json!([]), "{r}");
    assert_eq!(r["report"]["shapes"], 1);
    // Retrace a blank image.
    let img = image::RgbImage::from_pixel(40, 40, image::Rgb([255, 255, 255]));
    let png = dir.path().join("blank.png");
    img.save(&png).unwrap();
    let rid = run(&mut t.s, "object.import", json!({ "path": png }))["id"]
        .as_u64()
        .unwrap();
    let r = run(
        &mut t.s,
        "object.retrace",
        json!({ "id": rid, "settings": { "threshold": 100 } }),
    );
    assert_eq!(codes(&r["report"]), ["nothing_traced"], "{r}");
}

#[test]
fn a_fix_of_linked_copies_names_every_member() {
    let mut t = t();
    let (id, _) = import(&mut t.s, "frame", FRAME);
    outline_strokes(&mut t.s, id);
    let r = run(
        &mut t.s,
        "object.copies",
        json!({ "id": id, "count": 2, "linked": true }),
    );
    let _ = r;
    let ids: Vec<u64> = run(&mut t.s, "document.state", json!({}))["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids.len(), 3);
    let c = check(&mut t.s);
    // One issue for the group, not one per copy.
    let hair: Vec<Value> = issues(&c)
        .into_iter()
        .filter(|i| i["code"] == "hairline")
        .collect();
    assert_eq!(hair.len(), 1, "{c}");
    assert_eq!(hair[0]["object_ids"], json!(ids));
    let fix = &hair[0]["fixes"][0];
    assert!(
        fix["description"]
            .as_str()
            .unwrap()
            .ends_with("This object and its linked copies change together.")
    );
    apply(&mut t.s, fix);
    for o in run(&mut t.s, "document.state", json!({}))["objects"]
        .as_array()
        .unwrap()
    {
        assert_eq!(o["cut"]["strokes"], "centre_line");
    }
    assert!(!codes(&check(&mut t.s)).contains(&"hairline".to_string()));
}

#[test]
fn a_job_that_cannot_be_cut_still_carries_its_warnings() {
    let mut t = t();
    let (a, _) = import(&mut t.s, "a", RECT);
    run(
        &mut t.s,
        "object.set_visible",
        json!({ "ids": [a], "visible": false }),
    );
    let p = preview(&mut t.s);
    assert_eq!(p["ready"], false);
    assert_eq!(p["warnings"], json!(["'a' is hidden and will not be cut."]));
}

/// Each fix is exactly one undo step, and undoing it brings the issue back.
#[test]
fn each_fix_is_one_undo_step() {
    let undo = |s: &Session| s.doc().undo.undo_len();
    type Setup = Box<dyn Fn(&mut Session)>;
    let cases: Vec<(&str, Setup)> = vec![
        (
            "hairline",
            Box::new(|s| {
                let (id, _) = import(s, "frame", FRAME);
                outline_strokes(s, id);
            }),
        ),
        (
            "fill_ignored",
            Box::new(|s| {
                import(s, "lines", &format!("{RECT}{FLAT}"));
            }),
        ),
        (
            "below_min_feature",
            Box::new(|s| {
                import(s, "dot", r#"<circle cx="20" cy="20" r="0.4"/>"#);
            }),
        ),
        (
            "outside_area",
            Box::new(|s| {
                let (id, _) = import(s, "sq", RECT);
                run(s, "object.move", json!({ "ids": [id], "dx_mm": -100.0 }));
            }),
        ),
        (
            "larger_than_area",
            Box::new(|s| {
                run(
                    s,
                    "object.import_svg",
                    json!({ "name": "big", "svg": r#"<svg xmlns="http://www.w3.org/2000/svg" width="500mm" height="100mm" viewBox="0 0 500 100"><rect width="500" height="100"/></svg>"# }),
                );
            }),
        ),
    ];
    for (code, setup) in cases {
        let mut t = t();
        setup(&mut t.s);
        let fix = issue(&check(&mut t.s), code)["fixes"][0].clone();
        let before = undo(&t.s);
        apply(&mut t.s, &fix);
        assert_eq!(undo(&t.s), before + 1, "{code}");
        assert!(
            !codes(&check(&mut t.s)).contains(&code.to_string()),
            "{code}"
        );
        run(&mut t.s, "edit.undo", json!({}));
        assert!(
            codes(&check(&mut t.s)).contains(&code.to_string()),
            "{code}"
        );
    }
    // Nothing-to-cut fixes too.
    let mut t = t();
    let (a, _) = import(&mut t.s, "a", RECT);
    run(
        &mut t.s,
        "object.set_cut",
        json!({ "ids": [a], "cut": false }),
    );
    let fix = issue(&preview(&mut t.s), "skipped")["fixes"][0].clone();
    let before = undo(&t.s);
    apply(&mut t.s, &fix);
    assert_eq!(undo(&t.s), before + 1);
}

// ---------------------------------------------------------------- pairs of objects

/// Two 30 mm squares side by side, `gap` mm apart; returns their ids.
fn two_squares(s: &mut Session, gap: f64) -> (u64, u64) {
    let (a, _) = import(s, "left", RECT);
    let (b, _) = import(
        s,
        "right",
        &format!(
            r#"<rect x="{}" y="20" width="30" height="30"/>"#,
            50.0 + gap
        ),
    );
    (a, b)
}

fn pair_codes(v: &Value) -> Vec<String> {
    codes(v)
        .into_iter()
        .filter(|c| c.starts_with("objects_"))
        .collect()
}

#[test]
fn objects_closer_than_the_minimum_feature_are_a_hint() {
    let mut t = self::t();
    let (a, b) = two_squares(&mut t.s, 0.3);
    let c = check(&mut t.s);
    well_formed(&c);
    let i = issue(&c, "objects_close");
    assert_eq!(i["severity"], "info");
    assert_eq!(i["object_ids"], json!([a, b]));
    assert_eq!(
        i["message"],
        "'left' and 'right' are only 0.3 mm apart. The strip between them is too thin to cut cleanly or to weed (peel the waste away), and the blade may tear it."
    );
    let labels: Vec<&str> = i["fixes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["Move apart", "Ignore for this object pair"]);
    assert_eq!(
        i["hint"],
        "Close shaves can be part of the design. If they are not, move the objects at least 1.5 mm apart, the narrowest strip this blade cuts cleanly. If they are, ignore this hint for the pair."
    );
    // The import report of the second object names the pair too.
    let mut t = self::t();
    let (_, _) = import(&mut t.s, "left", RECT);
    let (_, r) = import(
        &mut t.s,
        "right",
        r#"<rect x="50.3" y="20" width="30" height="30"/>"#,
    );
    assert_eq!(pair_codes(&r["report"]), ["objects_close"], "{r}");

    let mut t = self::t();
    two_squares(&mut t.s, 2.0);
    assert_eq!(pair_codes(&check(&mut t.s)), Vec::<String>::new());
}

#[test]
fn overlapping_objects_are_a_hint() {
    let mut t = self::t();
    let (a, b) = two_squares(&mut t.s, -5.0);
    let c = check(&mut t.s);
    well_formed(&c);
    let i = issue(&c, "objects_overlap");
    assert_eq!(i["severity"], "info");
    assert_eq!(i["object_ids"], json!([a, b]));
    assert_eq!(
        i["message"],
        "'left' and 'right' overlap, so the knife cuts through both."
    );
    assert_eq!(i["fixes"][0]["label"], "Ignore for this object pair");
    assert_eq!(pair_codes(&c), ["objects_overlap"]);
    // One inside the other, no line crossing: an overlap too.
    let mut t = self::t();
    import(&mut t.s, "big", RECT);
    import(
        &mut t.s,
        "small",
        r#"<rect x="30" y="30" width="5" height="5"/>"#,
    );
    assert_eq!(pair_codes(&check(&mut t.s)), ["objects_overlap"]);
}

#[test]
fn move_apart_clears_the_hint_in_one_undo_step() {
    let mut t = self::t();
    let (_, b) = two_squares(&mut t.s, 0.3);
    let c = check(&mut t.s);
    let fix = issue(&c, "objects_close")["fixes"][0].clone();
    assert_eq!(fix["command"], "object.move");
    assert_eq!(fix["args"]["ids"], json!([b]));
    let undo = t.s.doc().undo.undo_len();
    apply(&mut t.s, &fix);
    assert_eq!(t.s.doc().undo.undo_len(), undo + 1);
    assert_eq!(pair_codes(&check(&mut t.s)), Vec::<String>::new());
    // Absolute: running it again changes nothing.
    apply(&mut t.s, &fix);
    assert_eq!(t.s.doc().undo.undo_len(), undo + 1);
    run(&mut t.s, "edit.undo", json!({}));
    assert_eq!(pair_codes(&check(&mut t.s)), ["objects_close"]);
}

#[test]
fn move_apart_is_not_offered_off_the_mat() {
    let mut t = self::t();
    let [_, _, x1, _] = area(&mut t.s);
    let (a, _) = import(&mut t.s, "left", RECT);
    let (b, _) = import(&mut t.s, "right", RECT);
    // b against the right edge of the cutting area, a 0.3 mm to its left.
    run(
        &mut t.s,
        "object.move",
        json!({ "ids": [b], "x_mm": x1 - 30.0 }),
    );
    run(
        &mut t.s,
        "object.move",
        json!({ "ids": [a], "x_mm": x1 - 60.3 }),
    );
    let c = check(&mut t.s);
    let i = issue(&c, "objects_close");
    let labels: Vec<&str> = i["fixes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["Ignore for this object pair"], "{c}");
}

#[test]
fn ignoring_a_pair_is_kept_in_the_project() {
    let mut t = self::t();
    let (a, b) = two_squares(&mut t.s, -5.0);
    let c = check(&mut t.s);
    let fix = issue(&c, "objects_overlap")["fixes"][0].clone();
    assert_eq!(fix["command"], "document.ignore_issue");
    let undo = t.s.doc().undo.undo_len();
    apply(&mut t.s, &fix);
    assert_eq!(t.s.doc().undo.undo_len(), undo + 1, "one undo step");
    assert!(t.s.doc().dirty);
    assert_eq!(pair_codes(&check(&mut t.s)), Vec::<String>::new());
    // Saved and opened again, the pair stays ignored.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pair.flatcraft");
    run(&mut t.s, "project.save", json!({ "path": path }));
    let mut t2 = self::t();
    run(&mut t2.s, "project.open", json!({ "path": path }));
    assert_eq!(pair_codes(&check(&mut t2.s)), Vec::<String>::new());
    assert!(t2.s.doc().document.is_ignored("objects_overlap", b, a));
    // Undo brings the hint back; ignore false too.
    run(&mut t.s, "edit.undo", json!({}));
    assert_eq!(pair_codes(&check(&mut t.s)), ["objects_overlap"]);
    run(&mut t.s, "edit.redo", json!({}));
    run(
        &mut t.s,
        "document.ignore_issue",
        json!({ "code": "objects_overlap", "object_ids": [b, a], "ignore": false }),
    );
    assert_eq!(pair_codes(&check(&mut t.s)), ["objects_overlap"]);
    // Only pair hints, only objects.
    for bad in [
        json!({ "code": "hairline", "object_ids": [a, b] }),
        json!({ "code": "objects_close", "object_ids": [a, a] }),
        json!({ "code": "objects_close", "object_ids": [a, 999] }),
    ] {
        assert!(
            t.s.execute("document.ignore_issue", bad.clone(), Origin::Agent)
                .is_err(),
            "{bad}"
        );
    }
}

#[test]
fn overlapping_objects_plan_and_send_normally() {
    let mut t = self::t();
    two_squares(&mut t.s, -5.0);
    let p = run(&mut t.s, "job.preview", json!({}));
    assert_eq!(p["ready"], true, "{p}");
    assert_eq!(p["issues"], json!([]), "{p}");
    let r = run(&mut t.s, "job.send", json!({}));
    assert!(r["bytes_sent"].as_u64().unwrap() > 0, "{r}");
    let mut t = self::t();
    two_squares(&mut t.s, 0.3);
    let p = run(&mut t.s, "job.preview", json!({}));
    assert_eq!(p["ready"], true, "{p}");
    assert_eq!(p["issues"], json!([]), "{p}");
}

#[test]
fn a_pair_in_one_group_is_a_pair_and_moves_alone() {
    let mut t = self::t();
    let (a, b) = two_squares(&mut t.s, 0.3);
    run(&mut t.s, "object.group", json!({ "ids": [a, b] }));
    let c = check(&mut t.s);
    let fix = issue(&c, "objects_close")["fixes"][0].clone();
    assert_eq!(
        fix["args"]["ids"],
        json!([b]),
        "only the second object moves"
    );
    apply(&mut t.s, &fix);
    assert_eq!(pair_codes(&check(&mut t.s)), Vec::<String>::new());

    // In different groups, the second one's group moves with it.
    let mut t = self::t();
    let (a, b) = two_squares(&mut t.s, 0.3);
    let (c3, _) = import(
        &mut t.s,
        "far",
        r#"<rect x="85" y="20" width="5" height="5"/>"#,
    );
    let g = run(&mut t.s, "object.group", json!({ "ids": [b, c3] }))["group_id"].clone();
    let c = check(&mut t.s);
    let fix = issue(&c, "objects_close")["fixes"][0].clone();
    assert!(g.is_u64(), "{g}");
    assert_eq!(fix["args"]["ids"], json!([g]));
    assert!(
        fix["description"]
            .as_str()
            .unwrap()
            .starts_with("Moves 'right' and the rest of its group"),
        "{fix}"
    );
    apply(&mut t.s, &fix);
    assert_eq!(pair_codes(&check(&mut t.s)), Vec::<String>::new());
    let _ = a;
}

/// Hidden and skipped objects are not on the sheet: no pair hints.
#[test]
fn hidden_and_skipped_objects_form_no_pairs() {
    let mut t = self::t();
    let (a, b) = two_squares(&mut t.s, -5.0);
    run(
        &mut t.s,
        "object.set_visible",
        json!({ "ids": [a], "visible": false }),
    );
    assert_eq!(pair_codes(&check(&mut t.s)), Vec::<String>::new());
    run(
        &mut t.s,
        "object.set_visible",
        json!({ "ids": [a], "visible": true }),
    );
    run(
        &mut t.s,
        "object.set_cut",
        json!({ "ids": [b], "cut": false }),
    );
    assert_eq!(pair_codes(&check(&mut t.s)), Vec::<String>::new());
}

/// A stroke wide enough to show as a band, but under the Auto threshold,
/// is cut once along its middle: the report says so, as a hint.
#[test]
fn strokes_cut_once_lose_their_width_out_loud() {
    let frame = |w: f64| {
        format!(
            r#"<rect x="20" y="20" width="30" height="30" fill="none" stroke="black" stroke-width="{w}"/>"#
        )
    };
    let mut t = self::t();
    let (id, r) = import(&mut t.s, "frame", &frame(1.0));
    well_formed(&r["report"]);
    let i = issue(&r["report"], "strokes_cut_once");
    assert_eq!(i["severity"], "info");
    assert_eq!(i["object_ids"], json!([id]));
    assert_eq!(
        i["message"],
        "'frame': 1 stroke is thinner than 1.5 mm, so it is cut once along its middle. Outline it if you want the stroke width as vinyl."
    );
    assert_eq!(i["fixes"], json!([]));
    run(
        &mut t.s,
        "object.set_cut_options",
        json!({ "ids": [id], "strokes": "outline" }),
    );
    let c = check(&mut t.s);
    assert!(!codes(&c).contains(&"strokes_cut_once".to_string()), "{c}");
    for w in [0.3, 2.0] {
        let mut t = self::t();
        let (_, r) = import(&mut t.s, "frame", &frame(w));
        assert!(
            !codes(&r["report"]).contains(&"strokes_cut_once".to_string()),
            "{w}: {r}"
        );
    }
}
