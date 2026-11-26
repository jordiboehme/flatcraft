//! The import report card in Prepare and the "why nothing cuts" card in
//! Cut: the engine's issues, and buttons that run the fixes' commands.

mod common;

use common::{
    AppHarness, app, app_mut, harness, import_square, session, to_prepare, wait_for_workers,
};
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::{Action, Workspace};
use flatcraft_core::document::ObjectId;
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

fn run(s: &SharedSession, cmd: &str, params: Value) -> Value {
    s.lock()
        .unwrap()
        .execute(cmd, params, Origin::User)
        .unwrap()
}

/// Drops `svg` (written to a file named `name.svg`) on the window, as a
/// person would, and lets the window settle.
fn drop_svg(h: &mut AppHarness, dir: &tempfile::TempDir, name: &str, svg: &str) {
    let path = dir.path().join(format!("{name}.svg"));
    std::fs::write(&path, svg).unwrap();
    let ctx = h.ctx.clone();
    app_mut(h).apply(&ctx, vec![Action::Dropped(vec![path])]);
    wait_for_workers(h);
}

fn page(body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100">{body}</svg>"#
    )
}

/// A 1 mm square: below the minimum feature size, with a Scale up fix.
const TINY: &str = r#"<rect x="20" y="20" width="1" height="1"/>"#;
const TINY_TEXT: &str = "'frame': 1 shape is smaller than 1.5 mm, the smallest shape this blade cuts cleanly. They may tear or not come off.";

fn start(dir: &tempfile::TempDir) -> (SharedSession, AppHarness) {
    let s = session(dir);
    let mut h = harness(dir, s.clone(), false);
    to_prepare(&mut h);
    (s, h)
}

#[test]
fn a_clean_import_shows_no_report() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mut h) = start(&dir);
    drop_svg(
        &mut h,
        &dir,
        "clean",
        &page(r#"<rect x="20" y="20" width="30" height="30"/>"#),
    );
    assert!(app(&h).ui.prepare.report.is_none());
    assert!(h.query_by_label("Import report").is_none());
}

#[test]
fn the_report_card_names_the_issue_and_its_fix_resolves_it() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = start(&dir);
    drop_svg(&mut h, &dir, "frame", &page(TINY));
    assert!(h.query_by_label("Import report").is_some());
    assert!(h.query_by_label(TINY_TEXT).is_some());
    h.get_by_role_and_label(Role::Button, "Scale up").click();
    wait_for_workers(&mut h);
    // The button ran the fix's command: the square is now big enough.
    let c = run(&s, "document.check", json!({}));
    assert_eq!(c["issues"], json!([]), "{c}");
    // Fixed: the line is gone, and with it the card.
    assert!(h.query_by_label(TINY_TEXT).is_none());
    assert!(app(&h).ui.prepare.report.is_none());
    assert!(h.query_by_label("Import report").is_none());
}

#[test]
fn the_report_buttons_run_exactly_the_engine_fixes() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = start(&dir);
    drop_svg(&mut h, &dir, "frame", &page(TINY));
    // The same import as an agent: the same fix.
    let agent = run(
        &s,
        "object.import_svg",
        json!({ "svg": page(TINY), "name": "frame" }),
    );
    let fix = &agent["report"]["issues"][0]["fixes"][0];
    let card = app(&h).ui.prepare.report.clone().unwrap();
    let gui = &card.issues[0].fixes[0];
    assert_eq!(gui.label, fix["label"]);
    assert_eq!(gui.command, fix["command"]);
    // The whole args, but for the object's id.
    let gui_id = card.ids[0];
    let agent_id = agent["id"].as_u64().unwrap();
    let mut args = gui.args.clone();
    assert_eq!(args["ids"], json!([gui_id]));
    args["ids"] = json!([agent_id]);
    assert_eq!(args, fix["args"]);
}

#[test]
fn a_file_bigger_than_the_mat_scales_to_fit_from_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = start(&dir);
    drop_svg(
        &mut h,
        &dir,
        "banner",
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="500mm" height="100mm" viewBox="0 0 500 100"><rect width="500" height="100"/></svg>"#,
    );
    assert!(h.query_by_label("Import report").is_some());
    h.get_by_role_and_label(Role::Button, "Scale to fit the mat")
        .click();
    wait_for_workers(&mut h);
    assert!(app(&h).ui.prepare.report.is_none());
    let c = run(&s, "document.check", json!({}));
    assert_eq!(c["issues"], json!([]));
}

#[test]
fn show_me_selects_the_object_and_moves_the_view_to_it() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = start(&dir);
    drop_svg(&mut h, &dir, "frame", &page(TINY));
    run(&s, "object.select", json!({ "ids": [] }));
    h.run();
    h.get_by_role_and_label(Role::Button, "Show me").click();
    wait_for_workers(&mut h);
    let id = s.lock().unwrap().doc().document.objects[0].id;
    let selection: Vec<ObjectId> = s.lock().unwrap().doc().selection.iter().copied().collect();
    assert_eq!(selection, vec![id]);
    // The object's centre is in the middle of the canvas.
    let p = &app(&h).ui.prepare;
    let b = s
        .lock()
        .unwrap()
        .doc()
        .document
        .get(id)
        .unwrap()
        .transform
        .center_mm;
    let at = p.view.mm_to_screen(b);
    let fit = flatcraft::ui::canvas::fit_rect(p.canvas_rect);
    assert!(at.distance(fit.center()) < 2.0, "{at:?} {:?}", fit.center());
}

#[test]
fn dismiss_hides_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mut h) = start(&dir);
    drop_svg(&mut h, &dir, "frame", &page(TINY));
    h.get_by_role_and_label(Role::Button, "Dismiss").click();
    h.run();
    assert!(app(&h).ui.prepare.report.is_none());
    assert!(h.query_by_label(TINY_TEXT).is_none());
}

#[test]
fn a_problem_that_the_file_has_stays_until_dismissed() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mut h) = start(&dir);
    drop_svg(
        &mut h,
        &dir,
        "dashed",
        &page(
            r#"<path d="M10 10 H90" fill="none" stroke="black" stroke-width="0.2" stroke-dasharray="2 1"/>"#,
        ),
    );
    let text = "1 stroke is dashed; its dashes are cut as a solid line.";
    assert!(h.query_by_label(text).is_some());
    // No automatic fix: the hint says what to do.
    assert!(
        h.query_by_label(
            "A knife cannot skip along a dash pattern yet. Turn the dashes into separate paths in your drawing app if you want a perforated line."
        )
        .is_some()
    );
    // A change to the document checks again, and the file's own issue stays.
    h.get_by_role_and_label(Role::Button, "Show me").click();
    wait_for_workers(&mut h);
    assert!(h.query_by_label(text).is_some());
}

// ---------------------------------------------------------------- Cut

fn to_cut(h: &mut AppHarness) {
    app_mut(h).ui.workspace = Workspace::Cut;
    h.run();
}

#[test]
fn cut_names_why_nothing_cuts_and_fixes_it() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    run(&s, "object.set_cut", json!({ "ids": [id], "cut": false }));
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    assert!(h.query_by_label("Nothing to cut").is_some());
    let text = "'square' is set to skip, so the knife leaves it out.";
    assert!(h.query_by_label(text).is_some());
    h.get_by_role_and_label(Role::Button, "Cut it").click();
    h.run();
    assert!(s.lock().unwrap().doc().document.objects[0].cut_enabled);
    assert!(matches!(app(&h).ui.cut.plan, Some(Ok(_))));
    assert!(h.query_by_label(text).is_none());
}

#[test]
fn every_reason_shows_with_its_own_fix() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = import_square(&s);
    run(
        &s,
        "object.set_visible",
        json!({ "ids": [a], "visible": false }),
    );
    run(&s, "object.set_cut", json!({ "ids": [b], "cut": false }));
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    let codes: Vec<&str> = app(&h)
        .ui
        .cut
        .issues
        .iter()
        .map(|i| i.code.as_str())
        .collect();
    assert_eq!(codes, ["hidden", "skipped"]);
    assert!(h.query_by_role_and_label(Role::Button, "Show it").is_some());
    assert!(h.query_by_role_and_label(Role::Button, "Cut it").is_some());
}

#[test]
fn an_empty_document_says_how_to_start() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s, false);
    to_cut(&mut h);
    assert!(
        h.query_by_label("The document is empty, so there is nothing to cut.")
            .is_some()
    );
    assert!(
        h.query_by_label(
            "Import an SVG or an image (Import in the ribbon, or drop a file on the window), or make test patterns."
        )
        .is_some()
    );
}

#[test]
fn an_object_off_the_mat_moves_back_from_cut() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    run(&s, "object.move", json!({ "ids": [id], "dx_mm": -200.0 }));
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    assert!(h.query_by_label("Houston, we have a problem").is_some());
    h.get_by_role_and_label(Role::Button, "Move onto the mat")
        .click();
    h.run();
    assert!(matches!(app(&h).ui.cut.plan, Some(Ok(_))));
}

const DASHED: &str = r#"<path d="M10 80 H90" fill="none" stroke="black" stroke-width="0.2" stroke-dasharray="2 1"/>"#;
const DASH_TEXT: &str = "1 stroke is dashed; its dashes are cut as a solid line.";

#[test]
fn the_most_severe_line_comes_first() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mut h) = start(&dir);
    // A dashed line (info, found in the file) and a tiny shape (warning).
    drop_svg(&mut h, &dir, "frame", &page(&format!("{DASHED}{TINY}")));
    let card = app(&h).ui.prepare.report.clone().unwrap();
    let order: Vec<&str> = card.all().iter().map(|i| i.code.as_str()).collect();
    assert_eq!(order, ["below_min_feature", "dashes_ignored"]);
    let top = |label: &str| h.get_by_label(label).rect().min.y;
    assert!(top(TINY_TEXT) < top(DASH_TEXT));
}

#[test]
fn a_reload_forgets_what_the_old_file_had() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = start(&dir);
    drop_svg(&mut h, &dir, "frame", &page(&format!("{DASHED}{TINY}")));
    assert!(h.query_by_label(DASH_TEXT).is_some());
    // The person fixes the file in their drawing app and reloads it.
    std::fs::write(
        dir.path().join("frame.svg"),
        page(&format!(
            r#"<path d="M10 80 H90" fill="none" stroke="black" stroke-width="0.2"/>{TINY}"#
        )),
    )
    .unwrap();
    let doc_id = s.lock().unwrap().doc().id();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::run(doc_id, "object.reload", json!({}))]);
    wait_for_workers(&mut h);
    assert!(h.query_by_label(DASH_TEXT).is_none());
    // The tiny shape is still there: the reload did not fix it.
    assert!(h.query_by_label(TINY_TEXT).is_some());
}

#[test]
fn a_reload_adds_what_the_new_file_has() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = start(&dir);
    drop_svg(&mut h, &dir, "frame", &page(TINY));
    assert!(h.query_by_label(DASH_TEXT).is_none());
    std::fs::write(
        dir.path().join("frame.svg"),
        page(&format!("{DASHED}{TINY}")),
    )
    .unwrap();
    let doc_id = s.lock().unwrap().doc().id();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::run(doc_id, "object.reload", json!({}))]);
    wait_for_workers(&mut h);
    assert!(h.query_by_label(DASH_TEXT).is_some());
    assert!(h.query_by_label(TINY_TEXT).is_some());
}

#[test]
fn an_object_a_hair_wider_than_the_mat_opens_cut_and_moves_on() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let a = run(&s, "document.state", json!({}))["cutting_area_mm"].clone();
    let w = a["x1"].as_f64().unwrap() - a["x0"].as_f64().unwrap();
    run(
        &s,
        "object.scale",
        json!({ "ids": [id], "width_mm": w + 5e-7, "keep_aspect": false }),
    );
    run(&s, "object.move", json!({ "ids": [id], "dy_mm": 1000.0 }));
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    h.get_by_role_and_label(Role::Button, "Move onto the mat")
        .click();
    h.run();
    assert!(matches!(app(&h).ui.cut.plan, Some(Ok(_))));
}

#[test]
fn a_job_that_cuts_shows_its_issues_with_fixes_in_cut() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    run(
        &s,
        "object.import_svg",
        json!({ "name": "lines", "svg": page(r#"<rect x="20" y="20" width="30" height="30"/><path d="M10 80 L90 80"/>"#) }),
    );
    let mut h = harness(&dir, s.clone(), false);
    to_cut(&mut h);
    h.get_by_role_and_label(Role::Tab, "Path").click();
    h.run();
    assert!(
        h.query_by_label(
            "'lines': 1 filled shape has no area or is thinner than 0.01 mm, so the cut mode Object outlines cuts nothing of it."
        )
        .is_some()
    );
    // The old warning that says the same is gone.
    assert!(
        h.query_by_label_contains("thinner than the cut tolerance")
            .is_none()
    );
    h.get_by_role_and_label(Role::Button, "Cut all lines")
        .click();
    h.run();
    let job = app(&h).ui.cut.job().unwrap();
    assert!(job.issues.is_empty());
}

/// Six issues in a short window: the card says how many lines are below,
/// and clicking that line scrolls until none are left.
#[test]
fn hidden_lines_are_counted_and_one_click_scrolls_to_them() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = common::harness_sized(&dir, s, [1400.0, 450.0], 8, |a| a);
    to_prepare(&mut h);
    let many = page(&format!(
        r#"<rect x="10" y="90" width="50" height="0.1"/><path d="M60 10 H90 V40 H60 V10.4" fill="none" stroke="black" stroke-width="0.2"/><circle cx="70" cy="70" r="0.4"/><path d="M10 80 L50 80"/>{DASHED}"#
    ));
    drop_svg(&mut h, &dir, "many", &many);
    let more = |h: &AppHarness| {
        h.query_all_by_label_contains("more below")
            .next()
            .and_then(|n| n.accesskit_node().value())
    };
    let first = more(&h).expect("a 'more below' line");
    let n: usize = first.split(' ').next().unwrap().parse().unwrap();
    assert!(n >= 1, "{first}");
    for _ in 0..10 {
        let Some(_) = more(&h) else { break };
        h.get_by_label_contains("more below").click();
        h.run();
        h.run();
    }
    assert_eq!(more(&h), None);
}

/// Two objects 0.3 mm apart: a hint (info, a dim dot), with Move apart
/// and Ignore. Ignoring it hides it for good, in one undo step; nothing
/// is moved on its own.
#[test]
fn a_near_touching_pair_is_a_hint_that_can_be_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = start(&dir);
    run(
        &s,
        "object.import_svg",
        json!({ "svg": page(r#"<rect x="20" y="20" width="30" height="30"/>"#), "name": "left" }),
    );
    drop_svg(
        &mut h,
        &dir,
        "right",
        &page(r#"<rect x="50.3" y="20" width="30" height="30"/>"#),
    );
    let text = "'left' and 'right' are only 0.3 mm apart. The strip between them is too thin to cut cleanly or to weed (peel the waste away), and the blade may tear it.";
    assert!(h.query_by_label(text).is_some());
    let card = app(&h).ui.prepare.report.clone().unwrap();
    assert!(
        card.all()
            .iter()
            .all(|i| i.severity == flatcraft_engine::report::Severity::Info)
    );
    assert!(
        h.query_by_role_and_label(Role::Button, "Move apart")
            .is_some()
    );
    let before = s.lock().unwrap().doc().document.objects[1].transform;
    let undo = common::undo_len(&s);
    h.get_by_role_and_label(Role::Button, "Ignore for this object pair")
        .click();
    wait_for_workers(&mut h);
    assert_eq!(common::undo_len(&s), undo + 1);
    assert_eq!(
        s.lock().unwrap().doc().document.objects[1].transform,
        before
    );
    assert!(h.query_by_label(text).is_none());
    assert!(app(&h).ui.prepare.report.is_none());
    let c = run(&s, "document.check", json!({}));
    assert_eq!(c["issues"], json!([]), "{c}");
}
