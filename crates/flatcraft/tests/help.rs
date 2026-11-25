//! The Help menu, the shortcuts sheet and the shortcut table.

mod common;

use common::{AppHarness, app, app_mut, harness, import_square, session, to_prepare};
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::help::{self, HelpItem, SHORTCUTS};
use flatcraft::ui::{Action, about};

fn opened(h: &AppHarness) -> Vec<String> {
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

/// Opens the Help popup and clicks an item; the next frame's output is
/// what the click asked for.
fn pick(h: &mut AppHarness, item: HelpItem) {
    h.get_by_role_and_label(Role::Button, "Help").click();
    h.run();
    h.get_by_label(item.label()).click();
    h.step();
}

#[test]
fn the_help_menu_lists_its_items() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    h.get_by_role_and_label(Role::Button, "Help").click();
    h.run();
    for item in HelpItem::ALL {
        h.get_by_label(item.label());
    }
}

#[test]
fn user_guide_opens_the_readme() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    pick(&mut h, HelpItem::UserGuide);
    assert_eq!(
        opened(&h),
        vec!["https://github.com/jordiboehme/flatcraft#readme".to_string()]
    );
}

#[test]
fn ask_for_your_plotter_opens_the_plotter_form() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    pick(&mut h, HelpItem::AskPlotter);
    assert_eq!(
        opened(&h),
        vec![
            "https://github.com/jordiboehme/flatcraft/issues/new?template=plotter_support.yml"
                .to_string()
        ]
    );
}

#[test]
fn report_a_problem_fills_in_version_build_and_system() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    pick(&mut h, HelpItem::ReportProblem);
    let urls = opened(&h);
    assert_eq!(urls.len(), 1, "{urls:?}");
    let url = &urls[0];
    assert!(
        url.starts_with(
            "https://github.com/jordiboehme/flatcraft/issues/new?template=bug_report.yml&"
        ),
        "{url}"
    );
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        url.contains(&format!("version={version}%20%28{}%29", about::BUILD)),
        "{url}"
    );
    assert!(
        url.contains(&format!("system={}", help::encode(&help::system()))),
        "{url}"
    );
    // The ids are the form's own field ids.
    let form = include_str!("../../../.github/ISSUE_TEMPLATE/bug_report.yml");
    assert!(form.contains("id: version") && form.contains("id: system"));
}

#[test]
fn about_item_opens_the_dialog_on_every_platform() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    pick(&mut h, HelpItem::About);
    h.run();
    assert!(app(&h).ui.about.open);
    assert!(opened(&h).is_empty());
}

#[test]
fn shortcuts_item_opens_the_sheet_and_close_ends_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    pick(&mut h, HelpItem::Shortcuts);
    h.run();
    assert!(app(&h).ui.show_shortcuts);
    h.get_by_label("Keyboard shortcuts");
    let cmd = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    };
    h.get_by_label(&format!("{cmd}+Shift+S"));
    h.get_by_label("Save As");
    // Keys do not reach the app behind the sheet.
    h.get_by_role_and_label(Role::Button, "Close").click();
    h.run();
    assert!(!app(&h).ui.show_shortcuts);
    assert!(h.query_by_label("Select all objects").is_none());
}

#[test]
fn escape_closes_the_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    app_mut(&mut h).ui.show_shortcuts = true;
    h.run();
    h.key_press(Key::Escape);
    h.run();
    assert!(!app(&h).ui.show_shortcuts);
}

#[test]
fn the_menu_waits_while_the_sheet_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    let all = || HelpItem::ALL.map(HelpItem::action).to_vec();
    assert_eq!(app(&h).menu_gate(all()), all());
    app_mut(&mut h).ui.show_shortcuts = true;
    assert_eq!(app(&h).menu_gate(all()), Vec::<Action>::new());
}

#[test]
fn the_sheet_shows_the_platform_modifier() {
    let save = SHORTCUTS.iter().find(|s| s.what == "Save").unwrap();
    let want = if cfg!(target_os = "macos") {
        "Cmd+S"
    } else {
        "Ctrl+S"
    };
    assert_eq!(save.keys(), want);
}

/// Every global shortcut in the table has a handler: pressing it makes the
/// key handler produce its action. (On macOS the menu bar owns three of
/// them, so those are skipped there.)
#[test]
fn every_global_shortcut_has_a_handler() {
    for s in &SHORTCUTS {
        let Some(run) = s.run else { continue };
        if s.menu_on_macos && cfg!(target_os = "macos") {
            continue;
        }
        let ctx = egui::Context::default();
        let mut actions = Vec::new();
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: s.key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: s.modifiers,
            }],
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| help::run_shortcuts(ui, 7, &mut actions));
        out.textures_delta.clear();
        assert_eq!(actions, vec![run(7)], "{} {}", s.keys(), s.what);
    }
}

/// Every Prepare key in the table does what the sheet says.
#[test]
fn every_prepare_key_does_what_the_sheet_says() {
    for s in SHORTCUTS.iter().filter(|s| s.prepare.is_some()) {
        let dir = tempfile::tempdir().unwrap();
        let sess = session(&dir);
        let id = import_square(&sess);
        import_square(&sess);
        let select = |ids: Vec<u64>| {
            sess.lock()
                .unwrap()
                .execute(
                    "object.select",
                    serde_json::json!({ "ids": ids }),
                    flatcraft_engine::session::Origin::User,
                )
                .unwrap();
        };
        let selection = || sess.lock().unwrap().doc().selection.len();
        let objects = || sess.lock().unwrap().doc().document.objects.len();
        select(vec![id]);
        let picker = common::FakePicker::default();
        let p = picker.clone();
        let mut h = common::harness_with(&dir, sess.clone(), move |app| app.with_file_picker(p));
        to_prepare(&mut h);
        let undo_before = common::undo_len(&sess);
        let who = format!("{} {}", s.keys(), s.what);
        h.key_press_modifiers(s.modifiers, s.key);
        h.run();
        match s.key {
            Key::A => assert_eq!(selection(), 2, "{who}"),
            Key::D => assert_eq!(objects(), 3, "{who}"),
            Key::I => assert_eq!(picker.requests.lock().unwrap().len(), 1, "{who}"),
            // Held while dragging; see `space_drag_pans_the_canvas`.
            Key::Space => continue,
            Key::ArrowLeft => assert!(common::undo_len(&sess) > undo_before, "{who}"),
            Key::Delete | Key::Backspace => assert_eq!(objects(), 1, "{who}"),
            Key::Escape => assert_eq!(selection(), 0, "{who}"),
            other => panic!("no check for {other:?}"),
        }
    }
}

#[test]
fn no_em_dash_in_the_help_texts() {
    for item in HelpItem::ALL {
        assert!(!item.label().contains('\u{2014}'));
    }
    for s in &SHORTCUTS {
        assert!(!s.what.contains('\u{2014}'));
    }
    let _ = Modifiers::NONE;
}

/// Global and Prepare rows are told apart by exactly one of the two fields.
#[test]
fn every_row_is_global_or_prepare() {
    for s in &SHORTCUTS {
        assert!(s.run.is_some() != s.prepare.is_some(), "{}", s.what);
    }
}

/// The arrow keys of the Nudge row are the keys the handler reads.
#[test]
fn the_nudge_row_lists_the_nudge_keys() {
    let row = SHORTCUTS
        .iter()
        .find(|s| s.prepare == Some(help::PrepareKey::Nudge))
        .unwrap();
    assert!(help::NUDGE_KEYS.contains(&row.key));
    assert_eq!(help::NUDGE_KEYS.len(), 4);
}

#[test]
fn the_sheet_lists_the_space_drag_pan_and_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    app_mut(&mut h).ui.show_shortcuts = true;
    h.run();
    h.get_by_label("Space");
    h.get_by_label("Pan the canvas (hold it and drag, or drag with the middle button)");
    h.get_by_label("Settings...");
    assert_eq!(help::pan_key(), Key::Space);
}

/// Holding the pan key and dragging moves the view, not the object.
#[test]
fn space_drag_pans_the_canvas() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness(&dir, s, false);
    to_prepare(&mut h);
    let before = app(&h).ui.prepare.view;
    let from = app(&h).ui.prepare.canvas_rect.center();
    h.hover_at(from);
    h.run();
    h.event(egui::Event::Key {
        key: help::pan_key(),
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    h.drag_at(from);
    h.run();
    for t in [0.5, 1.0] {
        h.hover_at(from + egui::vec2(60.0, 20.0) * t);
        h.run();
    }
    h.drop_at(from + egui::vec2(60.0, 20.0));
    h.run();
    let moved = app(&h).ui.prepare.view.offset - before.offset;
    assert!(
        (moved.x - 60.0).abs() < 1.0 && (moved.y - 20.0).abs() < 1.0,
        "{moved:?}"
    );
}
