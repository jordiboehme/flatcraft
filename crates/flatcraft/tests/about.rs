//! The About dialog and the easter eggs, driven through egui_kittest.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{AppHarness, app, app_mut, harness, harness_with, import_square, session, to_prepare};
use egui::accesskit::Role;
use egui::{Event, Modifiers};
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::about::{self, QUOTES};
use flatcraft::ui::rng::XorShift;
use flatcraft::ui::{Action, NOTHING_TO_UNDO, PUNCH_IT, PUNCH_IT_ODDS};

fn act(h: &mut AppHarness, action: Action) {
    let ctx = h.ctx.clone();
    app_mut(h).apply(&ctx, vec![action]);
    h.run();
}

fn open_about(h: &mut AppHarness) {
    act(h, Action::ShowAbout);
    assert!(app(h).ui.about.open);
}

fn toasts(h: &AppHarness) -> Vec<String> {
    app(h).ui.toasts.iter().map(|t| t.text.clone()).collect()
}

/// A harness whose clock only moves when the test moves it.
fn with_fake_clock(dir: &tempfile::TempDir) -> (AppHarness, Arc<Mutex<Instant>>) {
    let now = Arc::new(Mutex::new(Instant::now()));
    let c = now.clone();
    let h = harness_with(dir, session(dir), move |a| {
        a.with_clock(move || *c.lock().unwrap())
    });
    (h, now)
}

#[test]
fn about_shows_the_credits() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    open_about(&mut h);
    h.get_by_label(&about::version_line());
    assert!(about::version_line().starts_with(&format!("Version {} (", env!("CARGO_PKG_VERSION"))));
    for text in [
        about::MOTTO,
        about::TAGLINE,
        about::PITCH,
        about::BUILT_WITH,
        about::INSPIRED_BY,
        about::MADE_WITH,
        about::MADE_FOR,
        "❤️",
    ] {
        h.get_by_label(text);
    }
    assert_eq!(about::MOTTO, "Super massive stability stuff");
    assert!(about::MADE_FOR.contains("Jordi Böhme"));
    assert!(about::MADE_FOR.contains("MPL-2.0"));
    h.get_by_role_and_label(Role::Link, about::LICENSES);
    // The quote is one of the list, with its author.
    let i = app(&h).ui.about.quote.unwrap();
    let (quote, author) = QUOTES[i];
    h.get_by_label(&format!("\u{201c}{quote}\u{201d}"));
    h.get_by_label(&format!("- {author}"));
}

#[test]
fn the_tagline_is_the_readme_tagline() {
    let readme = include_str!("../../../README.md");
    let flat = readme.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains(&format!("<strong>{}</strong>", about::TAGLINE)));
    assert!(flat.contains(about::PITCH), "README and About differ");
}

#[test]
fn the_quotes_are_word_for_word() {
    assert_eq!(
        QUOTES,
        [
            ("Every defect gets respect.", "Laura Kampf"),
            ("Failure is always an option.", "Adam Savage"),
            ("Super massive stability stuff.", "Felix Schelhasse"),
            (
                "Maybe a toothbrush helmet isn't the answer, but at least you're asking the question.",
                "Simone Giertz, TED 2018"
            ),
            (
                "You want to make something that the very next morning the minute you wake up you want to run to your shop and look at it.",
                "Jimmy DiResta, Making It interview 2017"
            ),
        ]
    );
}

/// Clicks a button and returns the URLs the next frame asked to open.
fn opened_urls(h: &mut AppHarness, button: &str) -> Vec<String> {
    h.get_by_role_and_label(Role::Button, button).click();
    h.step();
    h.output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::OpenUrl(u) => Some(u.url.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_buttons_open_the_right_links() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    open_about(&mut h);
    for (button, url) in [
        ("Support Me", "https://ko-fi.com/V7V31T6CL9"),
        ("PayPal", "https://paypal.me/jordiboehmelopez"),
        ("GitHub", "https://github.com/jordiboehme/flatcraft"),
    ] {
        assert_eq!(
            opened_urls(&mut h, button),
            vec![url.to_string()],
            "{button}"
        );
        h.run();
    }
}

#[test]
fn licenses_link_shows_the_list_and_back_returns() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    open_about(&mut h);
    h.get_by_role_and_label(Role::Link, about::LICENSES).click();
    h.run();
    h.get_by_label("Third-party licenses");
    h.get_by_label(about::THIRD_PARTY);
    assert!(about::THIRD_PARTY.contains("egui 0.37.0 - MIT OR Apache-2.0"));
    h.get_by_role_and_label(Role::Button, "Back").click();
    h.run();
    assert!(h.query_by_label("Third-party licenses").is_none());
    h.get_by_label(about::TAGLINE);
}

#[test]
fn escape_closes_the_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    open_about(&mut h);
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(!app(&h).ui.about.open);
    assert!(h.query_by_label(about::TAGLINE).is_none());
}

#[test]
fn every_opening_shows_another_quote() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    let mut last = None;
    for _ in 0..12 {
        open_about(&mut h);
        let q = app(&h).ui.about.quote.unwrap();
        assert_ne!(Some(q), last);
        h.get_by_label(&format!("\u{201c}{}\u{201d}", QUOTES[q].0));
        last = Some(q);
        app_mut(&mut h).ui.about.close();
        h.run();
    }
}

#[test]
fn help_opens_about_except_on_macos() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    h.get_by_role_and_label(Role::Button, "Help").click();
    h.run();
    // macOS has the About item in the app menu; Help opens the repository.
    assert_eq!(app(&h).ui.about.open, !cfg!(target_os = "macos"));
}

#[test]
fn five_banner_clicks_run_the_knife_then_end_of_line() {
    let dir = tempfile::tempdir().unwrap();
    let (mut h, now) = with_fake_clock(&dir);
    open_about(&mut h);
    // Clicks elsewhere in the dialog do not count.
    for _ in 0..3 {
        h.get_by_label(about::TAGLINE).click();
        h.run();
    }
    assert_eq!(app(&h).ui.about.banner_clicks, 0);
    for n in 1..about::BANNER_CLICKS {
        h.get_by_label(about::BANNER_NAME).click();
        h.run();
        assert_eq!(app(&h).ui.about.banner_clicks, n);
        assert!(app(&h).ui.about.knife.is_none());
    }
    // Time alone does nothing.
    *now.lock().unwrap() += Duration::from_secs(10);
    h.run();
    assert!(h.query_by_label(about::END_OF_LINE).is_none());
    h.get_by_label(about::BANNER_NAME).click();
    // The knife asks for frames while it moves, so step instead of settling.
    h.run_steps(2);
    assert!(app(&h).ui.about.knife.is_some());
    assert!(h.query_by_label(about::END_OF_LINE).is_none());
    *now.lock().unwrap() += about::KNIFE_TIME / 2;
    h.run_steps(2);
    assert!(h.query_by_label(about::END_OF_LINE).is_none());
    *now.lock().unwrap() += about::KNIFE_TIME;
    // Done: no more frames are asked for, so the harness settles.
    h.run();
    h.get_by_label(about::END_OF_LINE);
    // Closing and opening again starts over.
    app_mut(&mut h).ui.about.close();
    h.run();
    open_about(&mut h);
    assert!(h.query_by_label(about::END_OF_LINE).is_none());
    assert_eq!(app(&h).ui.about.banner_clicks, 0);
}

#[test]
fn undo_with_nothing_to_undo_says_great_scott() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    let doc = s.lock().unwrap().doc().id();
    act(&mut h, Action::Redo(doc));
    assert!(!toasts(&h).iter().any(|t| t == NOTHING_TO_UNDO));
    app_mut(&mut h).ui.toasts.clear();
    act(&mut h, Action::Undo(doc));
    assert_eq!(toasts(&h), vec![NOTHING_TO_UNDO.to_string()]);
    app_mut(&mut h).ui.toasts.clear();
    // With a change to undo, Undo undoes it and says nothing.
    import_square(&s);
    assert_eq!(common::undo_len(&s), 1);
    act(&mut h, Action::Undo(doc));
    assert_eq!(common::undo_len(&s), 0);
    assert!(toasts(&h).is_empty(), "{:?}", toasts(&h));
}

#[test]
fn punch_it_follows_the_seeded_rolls() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let seed = 11;
    let mut h = harness_with(&dir, s.clone(), move |a| a.with_seed(seed));
    let doc = s.lock().unwrap().doc().id();
    // Saving and other toasts do not roll.
    act(&mut h, Action::Undo(doc));
    act(&mut h, Action::Redo(doc));
    let mut rolls = XorShift::new(seed);
    let mut punched = 0;
    for _ in 0..20 {
        app_mut(&mut h).ui.toasts.clear();
        act(&mut h, Action::Send { doc_id: doc });
        let texts = toasts(&h);
        assert_eq!(texts.len(), 1, "{texts:?}");
        if rolls.one_in(PUNCH_IT_ODDS) {
            punched += 1;
            assert_eq!(texts[0], PUNCH_IT);
        } else {
            assert!(texts[0].starts_with("Sent!"), "{}", texts[0]);
        }
    }
    // The seed gives both kinds of toast.
    assert!((1..20).contains(&punched), "{punched}");
}

#[test]
fn a_new_apps_first_send_is_the_usual_toast() {
    // The fixed default seed keeps other tests' send toasts as they are.
    assert!(!XorShift::default().one_in(PUNCH_IT_ODDS));
}

fn type_speed(h: &mut AppHarness, label: &str, text: &str) {
    h.get_by_role_and_label(Role::SpinButton, label).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run();
    for c in text.chars() {
        h.event(Event::Text(c.to_string()));
        h.run();
    }
    h.key_press(egui::Key::Enter);
    h.run();
}

#[test]
fn speed_above_the_maximum_goes_to_eleven() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let eleven = flatcraft::ui::settings::ELEVEN;
    // At the maximum: no joke.
    type_speed(&mut h, "Speed", "10");
    assert_eq!(s.lock().unwrap().doc().document.job.speed, Some(10));
    assert!(h.query_by_label(eleven).is_none());
    // Force and passes above their maximum: clamped, no joke.
    type_speed(&mut h, "Force", "99");
    assert_eq!(s.lock().unwrap().doc().document.job.force, Some(33));
    assert!(h.query_by_label(eleven).is_none());
    // Above the maximum: clamped, and the joke.
    type_speed(&mut h, "Speed (modified)", "11");
    assert_eq!(s.lock().unwrap().doc().document.job.speed, Some(10));
    h.get_by_label(eleven);
    // The next speed within the range ends it.
    type_speed(&mut h, "Speed (modified)", "4");
    assert_eq!(s.lock().unwrap().doc().document.job.speed, Some(4));
    assert!(h.query_by_label(eleven).is_none());
}

#[test]
fn the_banner_spans_the_dialog_and_the_buttons_are_centred() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    open_about(&mut h);
    let banner = h.get_by_label(about::BANNER_NAME).rect();
    assert!((banner.width() - 560.0).abs() < 1.0, "{banner:?}");
    assert!((banner.height() - 140.0).abs() < 1.0, "{banner:?}");
    let left = h.get_by_role_and_label(Role::Button, "Support Me").rect();
    let right = h.get_by_role_and_label(Role::Button, "GitHub").rect();
    let gap_left = left.left() - banner.left();
    let gap_right = banner.right() - right.right();
    assert!((gap_left - gap_right).abs() < 2.0, "{gap_left} {gap_right}");
}

#[test]
fn keys_do_not_reach_prepare_while_about_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    s.lock()
        .unwrap()
        .execute(
            "object.select",
            serde_json::json!({ "ids": [id] }),
            flatcraft_engine::session::Origin::User,
        )
        .unwrap();
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    open_about(&mut h);
    let before = common::undo_len(&s);
    h.key_press(egui::Key::Delete);
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert_eq!(common::undo_len(&s), before);
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(!app(&h).ui.about.open);
    assert_eq!(common::undo_len(&s), before);
}
