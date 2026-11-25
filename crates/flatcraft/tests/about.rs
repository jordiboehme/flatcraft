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
        about::PITCH_LINES[0],
        about::PITCH_LINES[1],
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
            ("Words are hard.", "Evan & Katelyn"),
            ("Don't glue yourself to the table.", "Joel Telling"),
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
        ("Ko-fi", "https://ko-fi.com/V7V31T6CL9"),
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
fn the_pitch_is_two_centred_lines() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    open_about(&mut h);
    let first = h.get_by_label(about::PITCH_LINES[0]).rect();
    let second = h.get_by_label(about::PITCH_LINES[1]).rect();
    assert!(second.top() >= first.bottom() - 1.0, "{first:?} {second:?}");
    assert!((first.center().x - second.center().x).abs() < 2.0);
}

#[test]
fn the_version_label_opens_about() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    assert!(!app(&h).ui.about.open);
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let rect = h.get_by_label(&version).rect();
    h.hover_at(rect.center());
    h.run();
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand
    );
    h.get_by_label(&version).click();
    h.run();
    assert!(app(&h).ui.about.open);
    h.get_by_label(about::TAGLINE);
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
    let (run, end) = {
        let k = app(&h).ui.about.knife.as_ref().unwrap();
        (k.run_secs, k.end_secs())
    };
    let started = *now.lock().unwrap();
    let at = |s: f32| started + Duration::from_secs_f32(s);
    assert!(h.query_by_label(about::END_OF_LINE).is_none());
    *now.lock().unwrap() = at(run / 2.0);
    h.run_steps(2);
    assert!(h.query_by_label(about::END_OF_LINE).is_none());
    // Home: "End of line." fades in and stays.
    *now.lock().unwrap() = at(run + 1.0);
    h.run_steps(2);
    h.get_by_label(about::END_OF_LINE);
    // Five more clicks restart the knife.
    for _ in 0..about::BANNER_CLICKS {
        h.get_by_label(about::BANNER_NAME).click();
        h.run_steps(2);
    }
    assert!(h.query_by_label(about::END_OF_LINE).is_none());
    let restarted = app(&h).ui.about.knife.as_ref().unwrap().started;
    assert_eq!(restarted, at(run + 1.0));
    // After the fade-out the banner is back to normal: no knife, no text,
    // and no more frames are asked for, so the harness settles.
    *now.lock().unwrap() = restarted + Duration::from_secs_f32(end + 0.1);
    h.run();
    assert!(app(&h).ui.about.knife.is_none());
    assert!(h.query_by_label(about::END_OF_LINE).is_none());
    // A new knife after it finished.
    *now.lock().unwrap() = at(run + 1.0) + Duration::from_secs_f32(end + 0.1);
    for _ in 0..about::BANNER_CLICKS {
        h.get_by_label(about::BANNER_NAME).click();
        h.run_steps(2);
    }
    assert!(app(&h).ui.about.knife.is_some());
    *now.lock().unwrap() += Duration::from_secs_f32(run + 1.0);
    h.run_steps(2);
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
    let left = h.get_by_role_and_label(Role::Button, "Ko-fi").rect();
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

fn speed(s: &flatcraft_mcp::SharedSession) -> Option<u8> {
    s.lock().unwrap().doc().document.job.speed
}

/// A square in Prepare with the speed typed above the maximum.
fn eleven_shown(dir: &tempfile::TempDir) -> (AppHarness, flatcraft_mcp::SharedSession) {
    let s = session(dir);
    import_square(&s);
    let mut h = harness(dir, s.clone(), false);
    to_prepare(&mut h);
    type_speed(&mut h, "Speed", "4");
    type_speed(&mut h, "Speed (modified)", "11");
    assert_eq!(speed(&s), Some(10));
    h.get_by_label(flatcraft::ui::settings::ELEVEN);
    (h, s)
}

#[test]
fn reset_speed_ends_eleven() {
    let dir = tempfile::tempdir().unwrap();
    let (mut h, s) = eleven_shown(&dir);
    h.get_by_role_and_label(Role::Button, "Reset speed").click();
    h.run();
    assert_eq!(speed(&s), None);
    assert!(h.query_by_label(flatcraft::ui::settings::ELEVEN).is_none());
}

#[test]
fn undo_ends_eleven_and_redo_does_not_bring_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let (mut h, s) = eleven_shown(&dir);
    let doc = s.lock().unwrap().doc().id();
    act(&mut h, Action::Undo(doc));
    assert_eq!(speed(&s), Some(4));
    assert!(h.query_by_label(flatcraft::ui::settings::ELEVEN).is_none());
    act(&mut h, Action::Redo(doc));
    assert_eq!(speed(&s), Some(10));
    assert!(h.query_by_label(flatcraft::ui::settings::ELEVEN).is_none());
}

#[test]
fn an_agents_override_ends_eleven() {
    let dir = tempfile::tempdir().unwrap();
    let (mut h, s) = eleven_shown(&dir);
    s.lock()
        .unwrap()
        .execute(
            "job.set_overrides",
            serde_json::json!({ "speed": 6 }),
            flatcraft_engine::session::Origin::Agent,
        )
        .unwrap();
    h.run();
    assert!(h.query_by_label(flatcraft::ui::settings::ELEVEN).is_none());
}

#[test]
fn a_document_switch_ends_eleven() {
    let dir = tempfile::tempdir().unwrap();
    let (mut h, s) = eleven_shown(&dir);
    let first = s.lock().unwrap().doc().id();
    act(&mut h, Action::NewDocument);
    assert_ne!(s.lock().unwrap().doc().id(), first);
    assert!(h.query_by_label(flatcraft::ui::settings::ELEVEN).is_none());
    act(&mut h, Action::Switch(first));
    common::wait_for_source_check(&mut h);
    assert_eq!(speed(&s), Some(10));
    assert!(h.query_by_label(flatcraft::ui::settings::ELEVEN).is_none());
}

#[test]
fn dragging_the_speed_past_the_maximum_is_no_joke() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let from = h
        .get_by_role_and_label(Role::SpinButton, "Speed")
        .rect()
        .center();
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    for dx in [50.0, 150.0, 300.0, 400.0] {
        h.hover_at(from + egui::vec2(dx, 0.0));
        h.run();
    }
    h.drop_at(from + egui::vec2(400.0, 0.0));
    h.run();
    assert_eq!(speed(&s), Some(10));
    assert!(h.query_by_label(flatcraft::ui::settings::ELEVEN).is_none());
}

/// A seed whose first roll is "Punch it.".
fn punching_seed() -> u64 {
    (1..)
        .find(|&seed| XorShift::new(seed).one_in(PUNCH_IT_ODDS))
        .unwrap()
}

#[test]
fn a_failed_send_never_says_punch_it() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let seed = punching_seed();
    let mut h = harness_with(&dir, s.clone(), move |a| a.with_seed(seed));
    let doc = s.lock().unwrap().doc().id();
    // Nothing to cut: the send fails.
    act(&mut h, Action::Send { doc_id: doc });
    let texts = toasts(&h);
    assert_eq!(texts.len(), 1, "{texts:?}");
    assert_ne!(texts[0], PUNCH_IT);
    assert!(!texts[0].starts_with("Sent"), "{}", texts[0]);
    // The failed send did not use the roll: the first real send gets it.
    app_mut(&mut h).ui.toasts.clear();
    import_square(&s);
    act(&mut h, Action::Send { doc_id: doc });
    assert_eq!(toasts(&h), vec![PUNCH_IT.to_string()]);
}

fn selected_square(dir: &tempfile::TempDir) -> (AppHarness, flatcraft_mcp::SharedSession) {
    let s = session(dir);
    let id = import_square(&s);
    s.lock()
        .unwrap()
        .execute(
            "object.select",
            serde_json::json!({ "ids": [id] }),
            flatcraft_engine::session::Origin::User,
        )
        .unwrap();
    let mut h = harness(dir, s.clone(), false);
    to_prepare(&mut h);
    (h, s)
}

fn objects(s: &flatcraft_mcp::SharedSession) -> usize {
    s.lock().unwrap().doc().document.objects.len()
}

fn selection(s: &flatcraft_mcp::SharedSession) -> usize {
    s.lock().unwrap().doc().selection.len()
}

#[test]
fn keys_do_not_reach_prepare_under_the_unsaved_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let (mut h, s) = selected_square(&dir);
    let doc = s.lock().unwrap().doc().id();
    act(&mut h, Action::Close(doc));
    let question = "Save changes to Untitled 1?";
    h.get_by_label(question);
    h.key_press(egui::Key::Delete);
    h.run();
    assert_eq!(objects(&s), 1);
    assert_eq!(selection(&s), 1);
    // Escape cancels the dialog, and the selection stays.
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(h.query_by_label(question).is_none());
    assert!(app(&h).ui.unsaved.is_none());
    assert_eq!((objects(&s), selection(&s)), (1, 1));
}

#[test]
fn keys_do_not_reach_prepare_under_the_pending_send_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let other = import_square(&s);
    {
        let mut g = s.lock().unwrap();
        g.execute(
            "object.select",
            serde_json::json!({ "ids": [id] }),
            flatcraft_engine::session::Origin::User,
        )
        .unwrap();
        g.execute(
            "job.set_plotter",
            serde_json::json!({ "id": "silhouette-cameo-2" }),
            flatcraft_engine::session::Origin::User,
        )
        .unwrap();
        g.execute(
            "job.send",
            serde_json::json!({}),
            flatcraft_engine::session::Origin::Agent,
        )
        .unwrap_err();
        assert!(g.pending_send().is_some());
    }
    let _ = other;
    let mut h = harness(&dir, s.clone(), true);
    to_prepare(&mut h);
    h.key_press(egui::Key::Delete);
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run();
    assert_eq!((objects(&s), selection(&s)), (2, 1));
    assert!(s.lock().unwrap().pending_send().is_some());
}

#[test]
fn eleven_shows_when_the_material_speed_is_already_the_maximum() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    {
        let mut g = s.lock().unwrap();
        let parent = g.doc().document.job.material_id.clone();
        let mut m = g.profiles.material(&parent).unwrap();
        m.id = "full-throttle".into();
        m.name = "Full throttle".into();
        m.speed = 10;
        m.overrides.clear();
        g.profiles.save_user_material(&m, &parent).unwrap();
        g.execute(
            "job.set_material",
            serde_json::json!({ "id": "full-throttle" }),
            flatcraft_engine::session::Origin::User,
        )
        .unwrap();
    }
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    type_speed(&mut h, "Speed", "11");
    // Clamped to the material's own speed: no override, no command.
    assert_eq!(speed(&s), None);
    h.get_by_label(flatcraft::ui::settings::ELEVEN);
}

#[test]
fn the_menu_waits_while_a_dialog_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    let doc = s.lock().unwrap().doc().id();
    let menu = || {
        vec![
            Action::ShowAbout,
            Action::ShowSettings,
            Action::Undo(doc),
            Action::Redo(doc),
            Action::Quit,
        ]
    };
    assert_eq!(app(&h).menu_gate(menu()), menu());
    open_about(&mut h);
    assert_eq!(app(&h).menu_gate(menu()), vec![Action::Quit]);
    app_mut(&mut h).ui.about.close();
    h.run();
    import_square(&s);
    act(&mut h, Action::Close(doc));
    assert!(app(&h).ui.unsaved.is_some());
    assert_eq!(app(&h).menu_gate(menu()), vec![Action::Quit]);
}
