//! The Help menu, the shortcuts sheet and the shortcut table.

mod common;
use common::RunPlan;

use common::{AppHarness, app, app_mut, harness, import_square, session, to_prepare};
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::help::{self, HelpItem, SHORTCUTS};
use flatcraft::ui::prepare::SelectTool;
use flatcraft::ui::{Action, about};

/// Opens the Help popup, clicks an item and returns what the click asked of
/// the platform (see `common::capture`).
fn pick(h: &mut AppHarness, item: HelpItem) -> common::Captured {
    h.get_by_role_and_label(Role::Button, "Help").click();
    h.run_plan();
    common::capture(h, |h| {
        h.get_by_label(item.label()).click();
        h.step();
    })
}

#[test]
fn the_help_menu_lists_its_items() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    h.get_by_role_and_label(Role::Button, "Help").click();
    h.run_plan();
    for item in HelpItem::ALL {
        h.get_by_label(item.label());
    }
}

#[test]
fn user_guide_opens_the_readme() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    let got = pick(&mut h, HelpItem::UserGuide);
    assert_eq!(
        got.urls(),
        vec!["https://github.com/jordiboehme/flatcraft#readme".to_string()]
    );
}

#[test]
fn ask_for_your_plotter_opens_the_plotter_form() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    let got = pick(&mut h, HelpItem::AskPlotter);
    assert_eq!(
        got.urls(),
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
    let urls = pick(&mut h, HelpItem::ReportProblem).urls();
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
    let got = pick(&mut h, HelpItem::About);
    h.run_plan();
    assert!(app(&h).ui.about.open);
    assert!(got.urls().is_empty());
}

#[test]
fn shortcuts_item_opens_the_sheet_and_close_ends_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    pick(&mut h, HelpItem::Shortcuts);
    h.run_plan();
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
    h.run_plan();
    assert!(!app(&h).ui.show_shortcuts);
    assert!(h.query_by_label("Select all objects").is_none());
}

#[test]
fn escape_closes_the_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    app_mut(&mut h).ui.show_shortcuts = true;
    h.run_plan();
    h.key_press(Key::Escape);
    h.run_plan();
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
/// key handler produce its action. (On macOS the menu bar owns some of
/// them, and some rows exist only there, so those are skipped.)
#[test]
fn every_global_shortcut_has_a_handler() {
    for s in &SHORTCUTS {
        let Some(run) = s.run else { continue };
        if !s.here() || (s.menu_on_macos && cfg!(target_os = "macos")) {
            continue;
        }
        let ctx = egui::Context::default();
        let mut actions = Vec::new();
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: s.key().expect("a global row has a key"),
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
        // Held while dragging a handle; see `tests/position_size.rs`.
        let Some(key) = s.key() else { continue };
        h.key_press_modifiers(s.modifiers, key);
        h.run_plan();
        match key {
            Key::A => assert_eq!(selection(), 2, "{who}"),
            Key::D => assert_eq!(objects(), 3, "{who}"),
            Key::I => assert_eq!(picker.requests.lock().unwrap().len(), 1, "{who}"),
            // Held while dragging; see `space_drag_pans_the_canvas`.
            Key::Space => continue,
            Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown => {
                assert!(common::undo_len(&sess) > undo_before, "{who}")
            }
            Key::Delete | Key::Backspace => assert_eq!(objects(), 1, "{who}"),
            Key::Escape => assert_eq!(selection(), 0, "{who}"),
            Key::L => assert_eq!(app(&h).ui.prepare.select_tool, SelectTool::Lasso, "{who}"),
            // From the Lasso tool back to Select.
            Key::V => {
                app_mut(&mut h).ui.prepare.select_tool = SelectTool::Lasso;
                h.key_press_modifiers(s.modifiers, key);
                h.run_plan();
                assert_eq!(app(&h).ui.prepare.select_tool, SelectTool::Select, "{who}")
            }
            // One object: nothing to group or ungroup, nothing changes
            // (tests/groups.rs presses them on a real group).
            Key::G => assert_eq!(common::undo_len(&sess), undo_before, "{who}"),
            // The selected object is the one at the back: front and
            // forward move it (one step), back and backward change nothing.
            Key::CloseBracket => assert_eq!(common::undo_len(&sess), undo_before + 1, "{who}"),
            Key::OpenBracket => assert_eq!(common::undo_len(&sess), undo_before, "{who}"),
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

/// The table has a row for each arrow key, and they are the nudge keys.
#[test]
fn the_nudge_rows_are_the_four_arrows() {
    let keys: Vec<Key> = SHORTCUTS
        .iter()
        .filter(|s| s.prepare == Some(help::PrepareKey::Nudge))
        .filter_map(|s| s.key())
        .collect();
    assert_eq!(
        keys,
        [
            Key::ArrowLeft,
            Key::ArrowRight,
            Key::ArrowUp,
            Key::ArrowDown
        ]
    );
    assert!(keys.iter().all(|k| help::is_nudge_key(*k)));
    assert!(!help::is_nudge_key(Key::A));
}

#[test]
fn the_delete_hint_names_the_key_of_the_platform() {
    let hint = help::hint(help::PrepareKey::Delete);
    if cfg!(target_os = "macos") {
        // The bundled font has no delete-key glyph, so it is spelled out.
        assert_eq!(hint, "Backspace");
    } else {
        assert_eq!(hint, "Del");
    }
}

#[test]
fn the_sheet_lists_the_space_drag_pan_and_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    app_mut(&mut h).ui.show_shortcuts = true;
    h.run_plan();
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
    h.run_plan();
    h.event(egui::Event::Key {
        key: help::pan_key(),
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.run_plan();
    h.drag_at(from);
    h.run_plan();
    for t in [0.5, 1.0] {
        h.hover_at(from + egui::vec2(60.0, 20.0) * t);
        h.run_plan();
    }
    h.drop_at(from + egui::vec2(60.0, 20.0));
    h.run_plan();
    let moved = app(&h).ui.prepare.view.offset - before.offset;
    assert!(
        (moved.x - 60.0).abs() < 1.0 && (moved.y - 20.0).abs() < 1.0,
        "{moved:?}"
    );
}

/// The drag modifiers (Shift, Option or Alt, Cmd or Ctrl, and the rotate
/// zone) are rows of the table, and the canvas asks the table for them.
#[test]
fn the_sheet_lists_the_handle_modifiers() {
    use help::Os;
    use help::PrepareKey::{
        FlipKeepAspect, Lasso, RotateOnCorner, RotateZone, ScaleFromCentre, SelectBox, SnapAngle,
        ToggleInSelection,
    };
    let drags: Vec<_> = SHORTCUTS.iter().filter(|s| s.is_drag()).collect();
    assert_eq!(drags.len(), 8);
    let (option, cmd) = if cfg!(target_os = "macos") {
        ("Option+drag", "Cmd+drag")
    } else {
        ("Alt+drag", "Ctrl+drag")
    };
    let keys: Vec<String> = drags.iter().map(|s| s.keys()).collect();
    assert_eq!(
        keys,
        [
            "drag",
            "Shift+drag",
            option,
            cmd,
            "Shift+drag",
            "drag",
            option,
            "Shift+drag"
        ]
        .map(String::from)
    );
    // The rows hold the modifiers of this system's table.
    for s in &drags {
        let action = s.prepare.unwrap();
        assert_eq!(
            s.modifiers,
            help::drag_modifiers(Os::CURRENT, action),
            "{}",
            s.what
        );
    }
    assert!(help::held(Modifiers::SHIFT, FlipKeepAspect));
    assert!(!help::held(Modifiers::NONE, FlipKeepAspect));
    assert!(!help::held(Modifiers::SHIFT, ScaleFromCentre));
    assert!(help::held(Modifiers::ALT, ScaleFromCentre));
    assert!(help::held(Modifiers::SHIFT, SnapAngle));
    let rotate = help::drag_modifiers(Os::CURRENT, RotateOnCorner);
    assert!(help::held(rotate, RotateOnCorner));
    assert!(!help::held(Modifiers::ALT, RotateOnCorner));
    // The rotate zone and the box need no key: they are never "held".
    assert!(!help::held(Modifiers::NONE, RotateZone));
    assert!(!help::held(Modifiers::NONE, SelectBox));
    assert!(help::held(Modifiers::ALT, Lasso));
    assert!(!help::held(Modifiers::SHIFT, Lasso));
    assert!(help::held(Modifiers::SHIFT, ToggleInSelection));
    // A drag row is never taken for a key press.
    assert!(drags.iter().all(|s| s.run.is_none()));
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    app_mut(&mut h).ui.show_shortcuts = true;
    h.run_plan();
    assert_eq!(h.get_all_by_label("Shift+drag").count(), 3);
    assert_eq!(h.get_all_by_label("drag").count(), 2);
    assert_eq!(h.get_all_by_label(option).count(), 2);
    h.get_by_label("Drag on the empty mat: select what lies fully inside the box");
    h.get_by_label(
        "While clicking, or drawing a box or lasso: flip objects in or out of the selection",
    );
    h.get_by_label(cmd);
    h.get_by_label("While dragging a handle: scale from the centre");
    h.get_by_label("Drag just outside a corner of the selection: rotate it");
    h.get_by_label("While dragging a corner handle: rotate instead of resize");
    h.get_by_label("While rotating: turn in 15 degree steps");
}

/// One table per system names the drag modifiers. The tests read all
/// three, whatever system runs them.
#[test]
fn each_system_has_its_own_drag_modifiers() {
    use help::Os;
    use help::PrepareKey::{
        FlipKeepAspect, RotateOnCorner, RotateZone, ScaleFromCentre, SnapAngle,
    };
    let name = |os, k| help::modifier_names(help::drag_modifiers(os, k), os);
    assert_eq!(name(Os::MacOs, RotateOnCorner), "Cmd");
    assert_eq!(name(Os::Windows, RotateOnCorner), "Ctrl");
    assert_eq!(name(Os::Linux, RotateOnCorner), "Ctrl");
    assert_eq!(name(Os::MacOs, ScaleFromCentre), "Option");
    assert_eq!(name(Os::Windows, ScaleFromCentre), "Alt");
    assert_eq!(name(Os::Linux, ScaleFromCentre), "Alt");
    for os in Os::ALL {
        assert_eq!(name(os, FlipKeepAspect), "Shift");
        assert_eq!(name(os, SnapAngle), "Shift");
        assert_eq!(name(os, RotateZone), "");
        // Scale from the centre and rotate never share a key.
        assert_ne!(
            help::drag_modifiers(os, ScaleFromCentre),
            help::drag_modifiers(os, RotateOnCorner)
        );
    }
    // The Mac table uses the Command key itself, the others Ctrl.
    assert!(help::drag_modifiers(Os::MacOs, RotateOnCorner).mac_cmd);
    assert!(help::drag_modifiers(Os::Windows, RotateOnCorner).ctrl);
    assert!(help::drag_modifiers(Os::Linux, RotateOnCorner).ctrl);
}

/// The hover cards of the handles, the rotate zone and the Angle field
/// name the keys of each system, from the same table.
#[test]
fn the_cards_name_the_keys_of_each_system() {
    use help::Os;
    for os in Os::ALL {
        let cards = [
            help::handle_card(os),
            help::rotate_card(os),
            help::angle_card(os),
        ];
        for c in &cards {
            assert!(!c.contains('\u{2014}'), "{c}");
        }
        let (centre, rotate, other) = match os {
            Os::MacOs => ("Option", "Cmd", ["Alt", "Ctrl"]),
            _ => ("Alt", "Ctrl", ["Option", "Cmd"]),
        };
        assert!(
            cards[0].contains(centre) && cards[0].contains(rotate),
            "{}",
            cards[0]
        );
        assert!(cards[0].contains("Shift"), "{}", cards[0]);
        assert!(
            cards[1].contains(rotate) && cards[1].contains("15 degree"),
            "{}",
            cards[1]
        );
        assert!(
            cards[2].contains(rotate) && cards[2].contains("clockwise"),
            "{}",
            cards[2]
        );
        for c in &cards {
            for word in other {
                assert!(!c.contains(word), "{os:?}: {c}");
            }
        }
        // Linux desktops often take Alt-drag for moving windows.
        assert_eq!(
            cards[0].contains("desktop settings"),
            os == Os::Linux,
            "{}",
            cards[0]
        );
    }
}

#[test]
fn the_sheet_shows_calculations_and_its_examples_are_right() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    pick(&mut h, HelpItem::Shortcuts);
    h.run_plan();
    h.get_by_label(help::CALC_TITLE);
    h.get_by_label("3/4in + 1cm");
    use flatcraft_core::numexpr::{Kind, eval};
    for (typed, what) in help::CALC_EXAMPLES {
        assert!(!what.contains('\u{2014}') && !typed.contains('\u{2014}'));
        let kind = if what.contains("degrees") {
            Kind::Angle
        } else {
            Kind::Length
        };
        let v = eval(typed, kind, Some(5.0)).unwrap();
        // The first number of the text is the result (+= adds to 5 here).
        let first: String = what
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let want: f64 = if typed.starts_with("+=") {
            15.0
        } else {
            first.parse().unwrap()
        };
        assert!((v - want).abs() < 0.01, "{typed}: {v} vs {what}");
    }
}

/// Every item of the app bar's Help menu has a hover card that says what
/// it does (the native macOS menu cannot show cards).
#[test]
fn every_help_item_has_a_hover_card() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    for item in HelpItem::ALL {
        assert!(!item.hint().is_empty(), "{item:?}");
        assert!(!item.hint().contains('\u{2014}'), "{item:?}");
        h.get_by_role_and_label(Role::Button, "Help").click();
        h.run_plan();
        h.get_by_label(item.label()).hover();
        h.run_plan();
        h.get_by_label(item.hint());
        h.key_press(Key::Escape);
        h.run_plan();
    }
}
