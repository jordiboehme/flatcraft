//! Rules for the window tests themselves, so a busy machine cannot make
//! them flaky:
//!
//! - No bare `Harness::run` (`run`, `try_run`, `run_ok`): it fails when a
//!   worker lands within its few frames. Use `run_plan` or
//!   `common::wait_until`, which wait for the worker's event. A line that
//!   truly needs it says why in a `// run:` comment on it or just above.
//! - No click with a wait between press and release: egui counts a press
//!   held longer than 0.8 s (about three test frames) as no click, and a
//!   wait steps as many frames as a worker needs. Use `common::click_at`.
//!   A press that is meant to be held says so in a `// held:` comment.

/// Lines (1-based) of `src` that call `Harness::run` without a `// run:`
/// comment on the line or the line above.
fn bare_runs(src: &str) -> Vec<usize> {
    let lines: Vec<&str> = src.lines().collect();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let code = line.split("//").next().unwrap_or_default();
        let calls = [".run()", ".try_run()", ".run_ok()"]
            .iter()
            .any(|c| code.contains(c));
        let allowed =
            line.contains("// run:") || (i > 0 && lines[i - 1].trim_start().starts_with("// run:"));
        if calls && !allowed {
            found.push(i + 1);
        }
    }
    found
}

/// Lines (1-based) of `src` where a press at a point is followed by a wait
/// and then a release at the same point, with no move in between, unless
/// the press line or the line above has a `// held:` comment.
fn held_clicks(src: &str) -> Vec<usize> {
    let lines: Vec<&str> = src.lines().collect();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(at) = line.find(".drag_at(") else {
            continue;
        };
        let arg_start = at + ".drag_at(".len();
        let Some(len) = line[arg_start..].find(");") else {
            continue;
        };
        let arg = &line[arg_start..arg_start + len];
        let allowed = line.contains("// held:")
            || (i > 0 && lines[i - 1].trim_start().starts_with("// held:"));
        if allowed {
            continue;
        }
        let mut waited = false;
        for next in lines.iter().skip(i + 1).take(12) {
            if next.contains(".hover_at(") || next.contains(".drag_at(") {
                break;
            }
            if next.contains(".drop_at(") {
                if waited && next.contains(&format!(".drop_at({arg})")) {
                    found.push(i + 1);
                }
                break;
            }
            if ["run_plan(", "wait_until(", "wait_for_", "settle("]
                .iter()
                .any(|w| next.contains(w))
            {
                waited = true;
            }
        }
    }
    found
}

#[test]
fn the_rules_find_what_they_look_for() {
    let src = "h.run();\n\
               r.h.run_ok();\n\
               // run: no worker in this harness\n\
               h.run();\n\
               h.run(); // run: no worker\n\
               h.run_plan();\n\
               // h.run();\n";
    assert_eq!(bare_runs(src), vec![1, 2]);
    let src = "h.drag_at(at);\n\
               h.run_plan();\n\
               h.drop_at(at);\n\
               h.drag_at(p);\n\
               h.step();\n\
               h.drop_at(p);\n\
               h.drag_at(from);\n\
               h.run_plan();\n\
               h.hover_at(to);\n\
               h.run_plan();\n\
               h.drop_at(to);\n\
               // held: a long press is the point\n\
               h.drag_at(at);\n\
               h.run_plan();\n\
               h.drop_at(at);\n";
    assert_eq!(held_clicks(src), vec![1]);
}

#[test]
fn window_tests_wait_by_event_and_click_in_fixed_frames() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut files: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.push(root.join("common/mod.rs"));
    assert!(files.len() > 30, "scanned {} files", files.len());
    let mut bad = Vec::new();
    for f in &files {
        let name = f
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if name == "test_rules.rs" {
            continue;
        }
        let src = std::fs::read_to_string(f).unwrap();
        for l in bare_runs(&src) {
            bad.push(format!(
                "{name}:{l}: Harness::run (use run_plan or wait_until)"
            ));
        }
        for l in held_clicks(&src) {
            bad.push(format!(
                "{name}:{l}: a wait between press and release (use click_at)"
            ));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
