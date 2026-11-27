//! The cut log in the window: the Job card's "Last time" line and the
//! "How did it go?" question, which only the person answers.

mod common;
use common::RunPlan;

use common::*;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::Workspace;
use flatcraft::ui::outcome::{self, OutcomeDialog};
use flatcraft_engine::cutlog::{self, Target, Verdict};
use flatcraft_engine::session::Origin;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A square on vinyl, sent once to File Output from the Cut workspace.
fn sent_once() -> (tempfile::TempDir, flatcraft_mcp::SharedSession, AppHarness) {
    let (dir, s, h, _) = sent_once_with_clock();
    (dir, s, h)
}

/// [`sent_once`] with the window's clock in the test's hand, so the
/// notices of the sends stay up while the test clicks.
fn sent_once_with_clock() -> (
    tempfile::TempDir,
    flatcraft_mcp::SharedSession,
    AppHarness,
    Arc<Mutex<Instant>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    s.lock()
        .unwrap()
        .execute("job.set_material", json!({ "id": "vinyl" }), Origin::User)
        .unwrap();
    let now = Arc::new(Mutex::new(Instant::now()));
    let clock = now.clone();
    let mut h = harness_with(&dir, s.clone(), move |a| {
        a.with_clock(move || *clock.lock().unwrap())
    });
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    settle(&mut h);
    h.get_by_role_and_label(Role::Tab, "Send").click();
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Send to File Output")
        .click();
    settle(&mut h);
    settle(&mut h);
    (dir, s, h, now)
}

/// True while a notice ("Sent ...") shows.
fn notice_shows(h: &AppHarness) -> bool {
    !app(h).ui.toasts.is_empty()
}

/// The node of `label` inside the open question.
fn in_dialog<'a>(h: &'a AppHarness, label: &'a str) -> egui_kittest::Node<'a> {
    h.root()
        .children_recursive()
        .filter(|n| n.accesskit_node().role() == Role::Dialog)
        .find(|d| d.query_by_label("It went well").is_some())
        .expect("the question is open")
        .get_by_role_and_label(Role::Button, label)
}

/// The label of the Job card's button for the newest unrated cut.
fn rate_button(dir: &tempfile::TempDir) -> String {
    let log = cutlog::read(dir.path());
    outcome::rate_label(log.to_rate("vinyl").unwrap())
}

fn dialog_open(h: &AppHarness) -> bool {
    app(h).ui.outcome.is_some()
        && h.query_by_role_and_label(Role::Button, "It went well")
            .is_some()
}

#[test]
fn a_file_output_send_is_logged_and_asks_nothing() {
    let (dir, _s, mut h) = sent_once();
    let log = cutlog::read(dir.path());
    assert_eq!(log.entries.len(), 1);
    assert_eq!(log.entries[0].material_id, "vinyl");
    assert!(!dialog_open(&h), "no question after File Output");
    assert!(app(&h).ui.outcome.is_none());

    // The Job card recalls it.
    h.get_by_role_and_label(Role::Tab, "Device & Material")
        .click();
    h.run_plan();
    let line = outcome::last_time_text(&log.entries[0]);
    assert!(line.starts_with("Last time with this material: "), "{line}");
    assert!(line.ends_with(" - not rated yet"), "{line}");
    assert!(!line.contains('\u{2014}'));
    h.get_by_label(&line);
}

#[test]
fn the_person_answers_from_the_job_card() {
    let (dir, s, mut h) = sent_once();
    h.get_by_role_and_label(Role::Tab, "Device & Material")
        .click();
    h.run_plan();
    h.get_by_role_and_label(Role::Button, &rate_button(&dir))
        .click();
    h.run_plan();
    assert!(dialog_open(&h));
    assert!(
        in_dialog(&h, "Save").accesskit_node().is_disabled(),
        "a thumb first"
    );
    h.get_by_role_and_label(Role::Button, "It went well")
        .click();
    h.run_plan();
    h.get_by_label("Outcome note").click();
    h.run_plan();
    h.get_by_label("Outcome note").type_text("clean edges");
    h.run_plan();
    in_dialog(&h, "Save").click();
    h.run_plan();
    h.run_plan();
    assert!(!dialog_open(&h));
    let log = cutlog::read(dir.path());
    let o = log.entries[0].outcome.as_ref().unwrap();
    assert_eq!(o.verdict, Verdict::Good);
    assert_eq!(o.note, "clean edges");
    let line = outcome::last_time_text(&s.lock().unwrap().cut_log().entries[0]);
    assert!(line.ends_with(" - good: clean edges"), "{line}");
    h.get_by_label(&line);
}

#[test]
fn not_now_records_nothing() {
    let (dir, _s, mut h) = sent_once();
    let before = std::fs::read_to_string(dir.path().join(cutlog::FILE)).unwrap();
    h.get_by_role_and_label(Role::Tab, "Device & Material")
        .click();
    h.run_plan();
    h.get_by_role_and_label(Role::Button, &rate_button(&dir))
        .click();
    h.run_plan();
    h.get_by_role_and_label(Role::Button, "It did not go well")
        .click();
    h.run_plan();
    h.get_by_role_and_label(Role::Button, "Not now").click();
    h.run_plan();
    assert!(!dialog_open(&h));
    assert_eq!(
        std::fs::read_to_string(dir.path().join(cutlog::FILE)).unwrap(),
        before
    );
}

#[test]
fn only_a_cut_on_a_plotter_asks_right_after_the_send() {
    let (dir, _s, _h) = sent_once();
    let mut e = cutlog::read(dir.path()).entries[0].clone();
    assert!(outcome::after_send(&e).is_none());
    e.target = Target::Usb;
    let d = outcome::after_send(&e).unwrap();
    assert_eq!(d, OutcomeDialog::about(&e));
    assert!(d.about.contains("Adhesive vinyl"), "{}", d.about);
}

/// A calibration cut (an offset test series) is never asked about, even
/// on a plotter, and its line says what it was.
#[test]
fn a_calibration_cut_is_not_asked_about() {
    let (dir, _s, _h) = sent_once();
    let mut e = cutlog::read(dir.path()).entries[0].clone();
    e.target = Target::Usb;
    e.calibration = Some(cutlog::Calibration::OffsetSeries {
        offsets_mm: vec![0.25, 0.3],
    });
    assert!(outcome::after_send(&e).is_none());
    assert!(
        outcome::cut_text(&e).contains(", offset test series, "),
        "{}",
        outcome::cut_text(&e)
    );
}

/// Should the question open for a calibration cut, nothing in it can be
/// picked and Save says why; the session refuses its rating too.
#[test]
fn a_calibration_cut_cannot_be_rated() {
    let (dir, s, mut h) = sent_once();
    let mut e = cutlog::read(dir.path()).entries[0].clone();
    e.calibration = Some(cutlog::Calibration::OffsetSeries {
        offsets_mm: vec![0.25, 0.3],
    });
    let d = OutcomeDialog::about(&e);
    assert!(d.calibration);
    app_mut(&mut h).ui.outcome = Some(d);
    h.run_plan();
    assert!(dialog_open(&h));
    for label in ["It went well", "It did not go well", "Save"] {
        assert!(
            in_dialog(&h, label).accesskit_node().is_disabled(),
            "{label} is dimmed"
        );
    }
    assert!(!cutlog::CALIBRATION_NOT_RATED.contains('\u{2014}'));

    // A real calibration entry: the session refuses its rating.
    let series = cutlog::CutEntry {
        id: "calib-1".into(),
        ..e
    };
    let line = serde_json::json!({ "kind": "cut" });
    let mut text = serde_json::to_value(&series).unwrap();
    text.as_object_mut()
        .unwrap()
        .extend(line.as_object().unwrap().clone());
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.path().join(cutlog::FILE))
        .unwrap();
    use std::io::Write as _;
    writeln!(f, "{text}").unwrap();
    drop(f);
    assert_eq!(
        s.lock()
            .unwrap()
            .record_outcome("calib-1", Verdict::Good, ""),
        Err(cutlog::CALIBRATION_NOT_RATED.to_string())
    );
}

#[test]
fn rate_asks_about_the_newest_unrated_cut_while_the_line_shows_the_rated_one() {
    let (dir, s, mut h, _now) = sent_once_with_clock();
    // A second send while the first one's notice shows: it must not cover
    // the button. Then rate the first one.
    assert!(notice_shows(&h));
    h.get_by_role_and_label(Role::Button, "Send to File Output")
        .click();
    settle(&mut h);
    settle(&mut h);
    let (older, newer) = {
        let log = cutlog::read(dir.path());
        assert_eq!(log.entries.len(), 2);
        (log.entries[0].clone(), log.entries[1].clone())
    };
    s.lock()
        .unwrap()
        .record_outcome(&older.id, Verdict::Good, "clean")
        .unwrap();
    h.get_by_role_and_label(Role::Tab, "Device & Material")
        .click();
    settle(&mut h);
    let log = s.lock().unwrap().cut_log();
    // The line keeps the rule: the newest rated cut.
    h.get_by_label(&outcome::last_time_text(log.get(&older.id).unwrap()));
    // The button asks about the newer, unrated cut, and says which.
    let label = outcome::rate_label(&newer);
    assert!(label.starts_with("Rate the cut of 20"), "{label}");
    // In the 900 pt window the button sits low in the stage panel; the
    // notices of the sends show over the canvas, not over it.
    assert!(notice_shows(&h));
    h.get_by_role_and_label(Role::Button, &label).click();
    settle(&mut h);
    assert_eq!(app(&h).ui.outcome.as_ref().unwrap().entry, newer.id);
}

#[test]
fn the_day_follows_the_time_zone_offset() {
    let at = "2026-11-26T23:30:00.000000000Z";
    assert_eq!(cutlog::day_at(at, 3600), "2026-11-27", "UTC+1");
    assert_eq!(cutlog::day_at(at, 0), "2026-11-26");
    assert_eq!(cutlog::day_at("2026-03-01T00:10:00Z", -3600), "2026-02-28");
    assert_eq!(cutlog::day_at("2024-03-01T00:10:00Z", -3600), "2024-02-29");
    assert_eq!(cutlog::day_at("2026-12-31T23:30:00Z", 3600), "2027-01-01");
    assert_eq!(cutlog::day_at("not a date", 0), "not a date");
}

#[test]
fn the_local_day_uses_the_offset_of_that_instant() {
    use jiff::tz::{Offset, TimeZone};
    let plus_1 = TimeZone::fixed(Offset::from_hours(1).unwrap());
    assert_eq!(
        outcome::day_in("2026-11-26T23:30:00.000000000Z", &plus_1),
        "2026-11-27"
    );
    assert_eq!(
        outcome::day_in("2026-11-26T23:30:00.000000000Z", &TimeZone::UTC),
        "2026-11-26"
    );
    // A zone with summer time: +1 in winter, +2 in summer.
    let berlin = TimeZone::get("Europe/Berlin").unwrap();
    assert_eq!(
        outcome::day_in("2026-11-26T22:30:00Z", &berlin),
        "2026-11-26"
    );
    assert_eq!(
        outcome::day_in("2026-11-26T23:30:00Z", &berlin),
        "2026-11-27"
    );
    assert_eq!(
        outcome::day_in("2026-07-01T22:30:00Z", &berlin),
        "2026-07-02"
    );
    assert_eq!(outcome::day_in("not a date", &berlin), "not a date");
}
