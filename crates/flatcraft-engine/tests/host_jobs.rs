//! Host knife compensation through the engine: the profile mapping, the
//! one guarded emit path, the shared send and the job warnings.

use std::path::Path;

use flatcraft_device::gpgl::quantize::device_grid;
use flatcraft_device::gpgl::{check_safe, decode};
use flatcraft_device::{ProfileStore, gpgl::CutSettings};
use flatcraft_engine::job::{self, JobInput, PLAN_MISMATCH};
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{CommandError, Origin, Session};
use flatcraft_path::{AppliedCompensation, ToolpathKind};
use flatcraft_sim::{SimSettings, measure_kept, simulate};
use serde_json::{Value, json};

struct Dirs {
    config: tempfile::TempDir,
    out: tempfile::TempDir,
}

fn dirs() -> Dirs {
    Dirs {
        config: tempfile::tempdir().unwrap(),
        out: tempfile::tempdir().unwrap(),
    }
}

fn session(d: &Dirs) -> Session {
    let profiles = ProfileStore::load(d.config.path()).unwrap();
    let prefs = Preferences {
        output_dir: Some(d.out.path().to_path_buf()),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, false, d.config.path().to_path_buf())
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

/// A user plotter preset `id` that inherits File Output, with `body`.
fn user_plotter(config: &Path, id: &str, body: &str) {
    let plotters = config.join("plotters");
    std::fs::create_dir_all(&plotters).unwrap();
    std::fs::write(
        plotters.join(format!("{id}.toml")),
        format!("id = \"{id}\"\nname = \"{id}\"\ninherits = \"file-output-gpgl-cameo2\"\n{body}"),
    )
    .unwrap();
}

/// Host compensation with offset `offset` and a blade of `blade` mm.
fn host_body(offset: f64, blade: f64) -> String {
    format!(
        "knife_compensation = {{ Host = {{ offset_mm = {offset:?}, pivot_threshold_deg = 0.0 }} }}\nblade_offset_mm = {blade:?}\n"
    )
}

/// A session on a user plotter `id` with host compensation.
fn host_session(d: &Dirs, id: &str, offset: f64, blade: f64, extra: &str) -> Session {
    user_plotter(
        d.config.path(),
        id,
        &format!("{}{extra}", host_body(offset, blade)),
    );
    let mut s = session(d);
    run(&mut s, "job.set_plotter", json!({ "id": id }));
    s
}

fn import(s: &mut Session, svg: &str, name: &str) -> u64 {
    run(s, "object.import_svg", json!({ "svg": svg, "name": name }))["id"]
        .as_u64()
        .unwrap()
}

const SQUARE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="20" y="20" width="30" height="30"/></svg>"#;

/// The M and D points of `bytes`, one list per `M`, as (y, x) device units.
fn sent_paths(bytes: &[u8]) -> Vec<Vec<(i64, i64)>> {
    let mut paths: Vec<Vec<(i64, i64)>> = Vec::new();
    for c in decode(bytes) {
        let (head, rest) = c.raw.split_at(1);
        if head != "M" && head != "D" {
            continue;
        }
        let n: Vec<i64> = rest.split(',').map(|t| t.parse().unwrap()).collect();
        let pts = n.chunks(2).map(|p| (p[0], p[1]));
        if head == "M" {
            paths.push(pts.collect());
        } else {
            paths.last_mut().unwrap().extend(pts);
        }
    }
    paths
}

/// What the plotter should get for the planned runs of `result`: each run
/// rounded through the one device transform, neighbours merged, runs that
/// collapse to one point left out.
fn planned_paths(input: &JobInput, result: &job::JobResult) -> Vec<Vec<(i64, i64)>> {
    let media = input.plotter.media_or_first(&input.job.media_id).unwrap().0;
    let g = device_grid(&input.plotter, media);
    result
        .plan
        .toolpaths
        .iter()
        .flat_map(|t| t.runs())
        .map(|r| {
            let mut units: Vec<(i64, i64)> = r
                .points
                .iter()
                .map(|&q| {
                    let u = g.rounded_units(q);
                    (u.y as i64, u.x as i64)
                })
                .collect();
            units.dedup();
            units
        })
        .filter(|u| u.len() >= 2)
        .collect()
}

fn warnings(r: &Value) -> Vec<String> {
    r["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn host_profile_job_sends_compensated_axis() {
    let d = dirs();
    let mut s = host_session(&d, "host", 0.3, 0.3, "");
    import(&mut s, SQUARE, "square");
    let input = JobInput::of(&s).unwrap();
    let result = job::build_input(&input).unwrap();
    assert_eq!(
        result.plan.compensation,
        AppliedCompensation::Host { offset_mm: 0.3 }
    );
    check_safe(&result.job.bytes).unwrap();
    let listing: Vec<&str> = result
        .job
        .listing
        .iter()
        .map(|l| l.command.as_str())
        .collect();
    assert!(listing.contains(&"FC0"), "{listing:?}");
    assert!(!listing.iter().any(|c| c.starts_with("FB") || *c == "TB72"));
    assert_eq!(listing.last(), Some(&"H"));

    // The first cut: its tip starts at corner P0 heading t; the axis runs
    // 0.3 mm ahead of the tip, so the plotter is sent P0 + 0.3 t.
    let t = &result.plan.toolpaths[0];
    assert_eq!(t.kind, ToolpathKind::Cut);
    let p0 = t.points[0];
    let dir = (t.points[1] - p0).normalize();
    let media = input.plotter.media_or_first(&input.job.media_id).unwrap().0;
    let g = device_grid(&input.plotter, media);
    let want = g.rounded_units(p0 + 0.3 * dir);
    let want = (want.y as i64, want.x as i64);
    let sent = sent_paths(&result.job.bytes);
    let tip = g.rounded_units(p0);
    let tip = (tip.y as i64, tip.x as i64);
    assert!(sent[0].contains(&want), "{want:?} not in {:?}", sent[0]);
    // It is exactly where the cut of the first segment starts: the axis
    // point before the first move that cuts segment 0 (after any swivel).
    let run0 = &t.runs()[0];
    let j = run0.seg.iter().position(|&k| k == 0).unwrap();
    let start = g.rounded_units(run0.points[j - 1]);
    assert_eq!((start.y as i64, start.x as i64), want);
    assert!(
        !sent[0].contains(&tip),
        "the tip corner itself is never sent"
    );
    // The bytes are exactly the planned device-grid points.
    assert_eq!(sent, planned_paths(&input, &result));
}

#[test]
fn firmware_job_commands_the_intended_points() {
    let d = dirs();
    let mut s = session(&d);
    import(&mut s, SQUARE, "square");
    let input = JobInput::of(&s).unwrap();
    let result = job::build_input(&input).unwrap();
    assert_eq!(result.plan.compensation, AppliedCompensation::Tip);
    assert!(result.plan.toolpaths.iter().all(|t| t.axis.is_none()));
    let listing: Vec<&str> = result
        .job
        .listing
        .iter()
        .map(|l| l.command.as_str())
        .collect();
    assert!(listing.contains(&"FC18"), "{listing:?}");
    // The tip path itself: the firmware compensates.
    assert_eq!(
        sent_paths(&result.job.bytes),
        planned_paths(&input, &result)
    );
    check_safe(&result.job.bytes).unwrap();
}

fn media_id(s: &Session) -> String {
    s.doc().document.job.media_id.clone()
}

fn settings(r: &job::JobResult) -> CutSettings {
    r.settings
}

#[test]
fn forged_plan_with_axis_under_firmware_is_refused() {
    let d = dirs();
    let mut s = session(&d);
    import(&mut s, SQUARE, "square");
    let input = JobInput::of(&s).unwrap();
    let result = job::build_input(&input).unwrap();
    let mut plan = result.plan.clone();
    assert_eq!(plan.compensation, AppliedCompensation::Tip);
    plan.toolpaths[0].axis = Some(plan.toolpaths[0].runs());
    let e = job::emit_plan(
        &plan,
        &input.objects,
        &input.plotter,
        &media_id(&s),
        &settings(&result),
    )
    .unwrap_err();
    assert_eq!(e, CommandError::Failed(PLAN_MISMATCH.into()));
    // The untouched plan still emits.
    job::emit_plan(
        &result.plan,
        &input.objects,
        &input.plotter,
        &media_id(&s),
        &settings(&result),
    )
    .unwrap();
}

#[test]
fn host_plan_missing_axis_is_refused() {
    let d = dirs();
    let mut s = host_session(&d, "host", 0.3, 0.3, "");
    import(&mut s, SQUARE, "square");
    let input = JobInput::of(&s).unwrap();
    let result = job::build_input(&input).unwrap();
    let mut plan = result.plan.clone();
    plan.toolpaths[0].axis = None;
    let e = job::emit_plan(
        &plan,
        &input.objects,
        &input.plotter,
        &media_id(&s),
        &settings(&result),
    )
    .unwrap_err();
    assert_eq!(e, CommandError::Failed(PLAN_MISMATCH.into()));
}

#[test]
fn host_axis_outside_media_is_refused_with_object_name() {
    let d = dirs();
    // A calibrated mat whose grid is the whole mat: the cutting area
    // reaches the mat's edge, and the axis runs ahead of the tip.
    let media = "media = [{ id = \"mat-12x12\", name = \"Flush mat\", kind = \"Mat\", color = \"#FBF4E4\", width_mm = 304.8, height_mm = 304.8, calibrated = true, grid = { origin_x_mm = 0.0, origin_y_mm = 0.0, width_mm = 304.8, height_mm = 304.8 } }]\n";
    let mut s = host_session(&d, "flush", 0.3, 0.3, media);
    let id = import(&mut s, SQUARE, "edge piece");
    run(
        &mut s,
        "object.move",
        json!({ "ids": [id], "x_mm": 0.0, "y_mm": 0.0 }),
    );
    let e = s
        .execute("job.preview", json!({}), Origin::User)
        .unwrap_err();
    let CommandError::Refused(text) = &e else {
        panic!("{e}")
    };
    assert!(text.contains("'edge piece'"), "{text}");
    assert!(text.contains("outside the mat"), "{text}");
    // Moved in by more than the offset, it builds.
    run(
        &mut s,
        "object.move",
        json!({ "ids": [id], "x_mm": 1.0, "y_mm": 1.0 }),
    );
    run(&mut s, "job.preview", json!({}));
}

/// A filled circle of radius `r` at `cx`, `cy`, and a big square.
fn circle_svg(r: f64) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="40" y="40" width="30" height="30"/><circle cx="20" cy="20" r="{r}"/></svg>"#
    )
}

#[test]
fn tight_curves_warn_in_every_mode() {
    let d = dirs();
    // Firmware, blade 0.25: a 0.1 mm circle is tighter, a 5 mm one is not.
    let mut s = session(&d);
    import(&mut s, &circle_svg(0.1), "dot");
    let w = warnings(&run(&mut s, "job.preview", json!({})));
    assert!(
        w.iter()
            .any(|w| w == "1 curve is tighter than 0.25 mm. It may come out rounded or torn."),
        "{w:?}"
    );
    let mut s = session(&d);
    import(&mut s, &circle_svg(5.0), "coin");
    let w = warnings(&run(&mut s, "job.preview", json!({})));
    assert!(!w.iter().any(|w| w.contains("tighter than")), "{w:?}");

    // Host, blade 0.3: the same warning, with the blade offset.
    let mut s = host_session(&d, "host", 0.3, 0.3, "");
    import(&mut s, &circle_svg(0.1), "dot");
    let w = warnings(&run(&mut s, "job.preview", json!({})));
    assert!(
        w.iter()
            .any(|w| w == "1 curve is tighter than 0.3 mm. It may come out rounded or torn."),
        "{w:?}"
    );

    // Two passes cut the same curve twice, but it is one curve.
    let mut s = session(&d);
    import(&mut s, &circle_svg(0.1), "dot");
    run(&mut s, "job.set_overrides", json!({ "passes": 2 }));
    let w = warnings(&run(&mut s, "job.preview", json!({})));
    assert!(
        w.iter().any(|w| w.starts_with("1 curve is tighter")),
        "{w:?}"
    );
}

#[test]
fn dropped_short_paths_warn() {
    let d = dirs();
    let mut s = session(&d);
    import(&mut s, SQUARE, "square");
    // 0.02 mm: under half a step, so the plan drops it.
    let short = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><path d="M60 60 L60.02 60" stroke="black" fill="none"/></svg>"#;
    // 0.03 mm from x = 9.98 mm: both ends round to one device unit, so the
    // emitter leaves it out.
    let speck = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><path d="M60 60 L60.03 60" stroke="black" fill="none"/></svg>"#;
    for (svg, x) in [(short, 70.0), (speck, 9.98)] {
        let id = import(&mut s, svg, "speck");
        run(
            &mut s,
            "object.set_cut_options",
            json!({ "ids": [id], "mode": "all_lines", "strokes": "centre_line", "auto_threshold_mm": 1.5 }),
        );
        run(
            &mut s,
            "object.move",
            json!({ "ids": [id], "x_mm": x, "y_mm": 20.0 }),
        );
    }
    // Two passes send the speck twice; it is still one path.
    run(&mut s, "job.set_overrides", json!({ "passes": 2 }));
    let w = warnings(&run(&mut s, "job.preview", json!({})));
    let job = s.doc().last_job().unwrap();
    assert_eq!(job.plan.dropped_short, 1);
    assert_eq!(job.job.collapsed.len(), 2, "both passes of the speck");
    // "Paths" counts what is cut: the speck's two passes are not.
    let planned = job.plan.toolpaths.len();
    assert_eq!(job.toolpaths_cut(), planned - 2);
    let r = run(&mut s, "job.preview", json!({}));
    assert_eq!(r["toolpaths"], planned - 2);
    let small: Vec<&String> = w.iter().filter(|w| w.contains("plotter step")).collect();
    assert_eq!(
        small,
        [
            "1 path is shorter than half a plotter step and is not cut.",
            "1 path fits inside one plotter step and is not cut.",
        ],
        "{w:?}"
    );
}

#[test]
fn offset_mismatch_warns() {
    let d = dirs();
    let mut s = host_session(&d, "differs", 0.3, 0.25, "");
    import(&mut s, SQUARE, "square");
    let w = warnings(&run(&mut s, "job.preview", json!({})));
    assert!(
        w.iter().any(|w| w
            == "Host compensation uses 0.3 mm, the blade offset is 0.25 mm. The simulator uses the blade offset."),
        "{w:?}"
    );
    let mut s = host_session(&d, "same", 0.3, 0.3, "");
    import(&mut s, SQUARE, "square");
    let w = warnings(&run(&mut s, "job.preview", json!({})));
    assert!(
        !w.iter().any(|w| w.contains("Host compensation uses")),
        "{w:?}"
    );
    // Firmware never compares.
    let mut s = session(&d);
    import(&mut s, SQUARE, "square");
    let w = warnings(&run(&mut s, "job.preview", json!({})));
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn set_strategy_is_undoable_and_saved() {
    let d = dirs();
    let mut s = session(&d);
    import(&mut s, SQUARE, "square");
    let strategy = |s: &mut Session| run(s, "document.state", json!({}))["job"]["strategy"].clone();
    assert_eq!(strategy(&mut s), "quality");
    let r = run(&mut s, "job.set_strategy", json!({ "strategy": "fast" }));
    assert_eq!(r["strategy"], "fast");
    assert_eq!(strategy(&mut s), "fast");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(strategy(&mut s), "quality");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(strategy(&mut s), "fast");
    for bad in ["keep_sheet", "slow"] {
        let e = s
            .execute("job.set_strategy", json!({ "strategy": bad }), Origin::User)
            .unwrap_err();
        assert!(matches!(e, CommandError::InvalidParams(_)), "{bad}: {e}");
    }
    let e = s
        .execute(
            "job.set_strategy",
            json!({ "strategy": "fast", "extra": 1 }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
    run(
        &mut s,
        "job.set_strategy",
        json!({ "strategy": "outlines_last" }),
    );
    let path = d.out.path().join("strategy.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    let mut s = session(&d);
    run(&mut s, "project.open", json!({ "path": path }));
    assert_eq!(strategy(&mut s), "outlines_last");
}

/// Task 10's check through the engine: host mode plans on the plotter's
/// device grid, the emitted M and D points are exactly the planned points,
/// and the device simulation of those points never pushes the blade. The
/// test sheet of `tools.test_patterns`, on a calibrated, mirrored plotter
/// with a grid origin off the unit lattice.
#[test]
fn host_engine_job_sends_the_planned_device_grid() {
    let d = dirs();
    let calibrated = "scale_x = 1.002\nscale_y = 0.998\nmedia = [{ id = \"mat-12x12\", name = \"Mat 12x12\", kind = \"Mat\", color = \"#FBF4E4\", width_mm = 304.8, height_mm = 330.2, calibrated = false, grid = { origin_x_mm = 0.37, origin_y_mm = 0.013, width_mm = 300.0, height_mm = 300.0 } }]\n[axis]\nmirror_x = true\nmirror_y = false\n";
    for (name, blade) in [("sheet-25", 0.25), ("sheet-30", 0.3)] {
        let mut s = host_session(&d, name, blade, blade, calibrated);
        // The real test sheet: V tips 15 to 170 deg, circles r 0.5 to 25,
        // squares, a round hole, an S curve, the F and the rulers.
        run(&mut s, "tools.test_patterns", json!({}));
        let input = JobInput::of(&s).unwrap();
        assert!(input.plotter.axis.mirror_x);
        let result = job::build_input(&input).unwrap();
        assert!(result.job.collapsed.is_empty(), "{name}");
        let want = planned_paths(&input, &result);
        assert_eq!(
            want.len(),
            result
                .plan
                .toolpaths
                .iter()
                .map(|t| t.runs().len())
                .sum::<usize>(),
            "{name}: no planned run collapses"
        );
        assert_eq!(sent_paths(&result.job.bytes), want, "{name}");
        check_safe(&result.job.bytes).unwrap();

        let media = input.plotter.media_or_first(&input.job.media_id).unwrap().0;
        let g = device_grid(&input.plotter, media);
        // Every planned point already lies on a device unit.
        for t in &result.plan.toolpaths {
            for r in t.runs() {
                for q in r.points {
                    let (u, v) = (g.to_units(q), g.rounded_units(q));
                    assert!(
                        (u.x - v.x).abs() < 1e-6 && (u.y - v.y).abs() < 1e-6,
                        "{name}: {q:?} is off the device grid"
                    );
                }
            }
        }
        let sim = SimSettings::device(input.plotter.blade_offset_mm, g);
        let trace = simulate(&result.plan.toolpaths, result.plan.home, &sim).unwrap();
        let (m, _) = measure_kept(&result.plan.toolpaths, &trace, &sim, &result.plan.kept);
        assert_eq!(m.pushed, 0, "{name}");
    }
}

/// A toolpath cut as several blade-down runs numbers them in its notes.
#[test]
fn several_runs_get_run_numbers_in_the_listing() {
    let d = dirs();
    // Lift and overshoot lifts the blade at sharp corners into the waste.
    let mut s = host_session(&d, "host", 0.3, 0.3, "");
    run(&mut s, "job.set_material", json!({ "id": "cardstock" }));
    import(&mut s, SQUARE, "square");
    let r = run(&mut s, "job.preview", json!({}));
    let notes: Vec<&str> = r["listing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["note"].as_str().unwrap())
        .collect();
    let job = s.doc().last_job().unwrap();
    let several = job
        .plan
        .toolpaths
        .iter()
        .any(|t| t.axis.as_ref().is_some_and(|r| r.len() > 1));
    assert!(several, "cardstock lifts at the square's corners");
    assert!(
        notes
            .iter()
            .any(|n| n.starts_with("object ") && n.ends_with(", pass 1 run 2")),
        "{notes:?}"
    );
    // A single run has no number.
    let mut s = host_session(&d, "host2", 0.3, 0.3, "");
    import(&mut s, SQUARE, "square");
    let r = run(&mut s, "job.preview", json!({}));
    assert!(
        !r["listing"].to_string().contains(" run "),
        "{}",
        r["listing"]
    );
}

/// People and agents read "plotter step", one word for one thing.
#[test]
fn no_user_facing_text_says_device_step() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = vec![root.join("README.md")];
    let mut dirs = vec![root.join("crates"), root.join("docs")];
    while let Some(dir) = dirs.pop() {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if p.is_dir() {
                // Plans and specs are not shown to anyone; tests may name
                // the old word to check it is gone.
                if !matches!(name.as_str(), "superpowers" | "target" | "tests") {
                    dirs.push(p);
                }
            } else if name.ends_with(".rs") || name.ends_with(".md") || name.ends_with(".toml") {
                files.push(p);
            }
        }
    }
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        assert!(
            !text.to_lowercase().contains("device step"),
            "{} says device step",
            f.display()
        );
    }
}
