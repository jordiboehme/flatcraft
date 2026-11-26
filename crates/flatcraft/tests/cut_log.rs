//! The cut log in the window: the Job card's "Last time" line and the
//! "How did it go?" question, which only the person answers.

mod common;

use common::*;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::Workspace;
use flatcraft::ui::outcome::{self, OutcomeDialog};
use flatcraft_engine::cutlog::{self, Target, Verdict};
use flatcraft_engine::session::Origin;
use serde_json::json;

/// A square on vinyl, sent once to File Output from the Cut workspace.
fn sent_once() -> (tempfile::TempDir, flatcraft_mcp::SharedSession, AppHarness) {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    s.lock()
        .unwrap()
        .execute("job.set_material", json!({ "id": "vinyl" }), Origin::User)
        .unwrap();
    // Tall enough that the Job card's rate button, under the material's
    // tips, is clear of the bottom edge.
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 1200.0], 4, |a| a);
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run();
    h.get_by_role_and_label(Role::Tab, "Send").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Send to File Output")
        .click();
    h.run();
    h.run();
    (dir, s, h)
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
    h.run();
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
    h.run();
    h.get_by_role_and_label(Role::Button, &rate_button(&dir))
        .click();
    h.run();
    assert!(dialog_open(&h));
    assert!(
        in_dialog(&h, "Save").accesskit_node().is_disabled(),
        "a thumb first"
    );
    h.get_by_role_and_label(Role::Button, "It went well")
        .click();
    h.run();
    h.get_by_label("Outcome note").click();
    h.run();
    h.get_by_label("Outcome note").type_text("clean edges");
    h.run();
    in_dialog(&h, "Save").click();
    h.run();
    h.run();
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
    h.run();
    h.get_by_role_and_label(Role::Button, &rate_button(&dir))
        .click();
    h.run();
    h.get_by_role_and_label(Role::Button, "It did not go well")
        .click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Not now").click();
    h.run();
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

#[test]
fn rate_asks_about_the_newest_unrated_cut_while_the_line_shows_the_rated_one() {
    let (dir, s, mut h) = sent_once();
    // A second send, then rate the first one.
    h.get_by_role_and_label(Role::Button, "Send to File Output")
        .click();
    h.run();
    h.run();
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
    h.run();
    let log = s.lock().unwrap().cut_log();
    // The line keeps the rule: the newest rated cut.
    h.get_by_label(&outcome::last_time_text(log.get(&older.id).unwrap()));
    // The button asks about the newer, unrated cut, and says which.
    let label = outcome::rate_label(&newer);
    assert!(label.starts_with("Rate the cut of 20"), "{label}");
    h.get_by_role_and_label(Role::Button, &label).click();
    h.run();
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
