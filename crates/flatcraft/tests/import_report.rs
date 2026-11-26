//! The import report card in Prepare and the "why nothing cuts" card in
//! Cut: the engine's issues, and buttons that run the fixes' commands.

mod common;

use common::{
    AppHarness, app, app_mut, harness, import_square, session, to_prepare, wait_for_workers,
};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::{Action, Workspace};
use flatcraft_core::document::{ObjectId, StrokeMode};
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

const FRAME: &str =
    r#"<rect x="20" y="20" width="30" height="30" fill="none" stroke="black" stroke-width="1"/>"#;
const HAIRLINE_TEXT: &str = "'frame': 1 line is cut along both edges but only 1 mm wide. A strip thinner than 1.5 mm tears.";

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
    drop_svg(&mut h, &dir, "frame", &page(FRAME));
    assert!(h.query_by_label("Import report").is_some());
    assert!(h.query_by_label(HAIRLINE_TEXT).is_some());
    h.get_by_role_and_label(Role::Button, "Cut as single lines")
        .click();
    wait_for_workers(&mut h);
    // The button ran the fix's command: the stroke is now one cut.
    let strokes = s.lock().unwrap().doc().document.objects[0].cut.strokes;
    assert_eq!(strokes, StrokeMode::CentreLine);
    // Fixed: the line is gone, and with it the card.
    assert!(h.query_by_label(HAIRLINE_TEXT).is_none());
    assert!(app(&h).ui.prepare.report.is_none());
    assert!(h.query_by_label("Import report").is_none());
}

#[test]
fn the_report_buttons_run_exactly_the_engine_fixes() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut h) = start(&dir);
    drop_svg(&mut h, &dir, "frame", &page(FRAME));
    // The same import as an agent: the same fix.
    let agent = run(
        &s,
        "object.import_svg",
        json!({ "svg": page(FRAME), "name": "frame" }),
    );
    let fix = &agent["report"]["issues"][0]["fixes"][0];
    let card = app(&h).ui.prepare.report.clone().unwrap();
    let gui = &card.issues[0].fixes[0];
    assert_eq!(gui.label, fix["label"]);
    assert_eq!(gui.command, fix["command"]);
    assert_eq!(gui.args["strokes"], fix["args"]["strokes"]);
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
    drop_svg(&mut h, &dir, "frame", &page(FRAME));
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
    drop_svg(&mut h, &dir, "frame", &page(FRAME));
    h.get_by_role_and_label(Role::Button, "Dismiss").click();
    h.run();
    assert!(app(&h).ui.prepare.report.is_none());
    assert!(h.query_by_label(HAIRLINE_TEXT).is_none());
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
