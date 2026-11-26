//! The Settings window: pages, staged changes, Restore page defaults,
//! search and the theme preview, driven through egui_kittest.

mod common;

use common::{AppHarness, app, app_mut, harness_sized, harness_steps, session, to_prepare};
use egui::accesskit::{Role, Toggled};
use egui::{Modifiers, Rect};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::rails::SidePanel;
use flatcraft::ui::settings_window::{self, CLOSE_NAME, Page, ROWS, SEARCH_NAME};
use flatcraft::ui::theme::Tokens;
use flatcraft_core::document::CutStrategy;
use flatcraft_engine::prefs::{PanelUser, Preferences, Theme};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::json;

fn harness(dir: &tempfile::TempDir, s: &SharedSession) -> AppHarness {
    harness_steps(dir, s.clone(), 8, |app| app)
}

fn open(h: &mut AppHarness, page: &str) {
    h.get_by_role_and_label(Role::Button, "Settings").click();
    h.run();
    h.get_by_role_and_label(Role::Tab, page).click();
    h.run();
}

fn click(h: &mut AppHarness, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

fn disabled(h: &AppHarness, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

fn stored(dir: &tempfile::TempDir) -> Preferences {
    Preferences::load(dir.path())
}

fn is_open(h: &AppHarness) -> bool {
    app(h).ui.show_settings
}

/// The controls of every row of the window (every setting the old
/// Settings and Profiles window had, and the new ones), by row id. The
/// test fails when a row has no entry here, so a new setting needs one.
const REACHABLE: [(&str, &[&str]); 24] = [
    ("output_dir", &["Choose folder...", "Use the default"]),
    ("auto_reload", &["Reload changed files automatically"]),
    ("theme", &["Theme: Dark", "Theme: Light", "Theme: System"]),
    (
        "panel_objects",
        &[
            "Objects panel: Auto",
            "Objects panel: Open",
            "Objects panel: Collapsed",
        ],
    ),
    ("panel_settings", &["Cut and Job panel: Auto"]),
    ("panel_stage", &["Cut stage panel: Auto"]),
    ("ribbon_pins", &["Reset pins"]),
    ("default_plotter", &["Default device"]),
    ("default_material", &["Default material"]),
    (
        "default_strategy",
        &[
            "Cut strategy: Quality",
            "Cut strategy: Fast",
            "Cut strategy: Outlines last",
        ],
    ),
    (
        "rotate_each",
        &["Turn and scale each object about its own centre"],
    ),
    (
        "auto_arrange_spacing_mm",
        &[
            "Auto arrange spacing",
            "Less Auto arrange spacing",
            "More Auto arrange spacing",
        ],
    ),
    ("allow_agent_cuts", &["Allow agents to start cuts"]),
    (
        "mcp",
        &[
            "Copy the Claude Code command",
            "Copy the Claude Desktop config",
        ],
    ),
    ("shortcuts", &["Select all objects"]),
    (
        "plotter.knife",
        &["Silhouette Cameo 2", "Knife compensation"],
    ),
    ("plotter.blade_offset", &["Blade offset (mm)"]),
    ("plotter.units", &["Units per mm", "Save as user preset..."]),
    ("material.speed", &["Vinyl", "Speed"]),
    ("material.force", &["Force"]),
    ("material.passes", &["Passes"]),
    ("material.overcut", &["Overcut (mm)"]),
    (
        "mat.origin",
        &["Mat name", "Grid origin x (mm)", "Mat colour (hex)"],
    ),
    (
        "mat.calibration",
        &[flatcraft::ui::profiles_window::CALIBRATION_LATER],
    ),
];

#[test]
fn every_setting_is_reachable() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "General");
    // Every row has its controls listed, and every listed id is a row.
    for r in ROWS {
        assert!(
            REACHABLE.iter().any(|(id, _)| *id == r.id),
            "row {} has no controls in REACHABLE",
            r.id
        );
    }
    for (id, names) in REACHABLE {
        let page = settings_window::row(id).page;
        h.get_by_role_and_label(Role::Tab, page.name()).click();
        h.run();
        for name in names {
            assert!(
                h.query_by_label(name).is_some(),
                "{name} ({id}) is not on the {} page",
                page.name()
            );
        }
    }
    assert_eq!(REACHABLE.len(), ROWS.len());
}

#[test]
fn cancel_discards_apply_keeps_and_ok_keeps_and_closes() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);

    // Cancel drops the change.
    open(&mut h, "General");
    click(&mut h, "Reload changed files automatically");
    click(&mut h, "Cancel");
    assert!(!is_open(&h));
    assert!(!s.lock().unwrap().prefs.auto_reload);
    assert!(!stored(&dir).auto_reload);
    // Opening again shows the stored value, not the dropped one.
    open(&mut h, "General");
    let node = h.get_by_label("Reload changed files automatically");
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::False));

    // Apply stores it and keeps the window open.
    assert!(disabled(&h, "Apply"));
    click(&mut h, "Reload changed files automatically");
    assert!(!disabled(&h, "Apply"));
    click(&mut h, "Apply");
    assert!(is_open(&h));
    assert!(stored(&dir).auto_reload);
    assert!(disabled(&h, "Apply"));

    // OK stores it and closes.
    h.get_by_role_and_label(Role::Tab, "Agents").click();
    h.run();
    click(&mut h, "Allow agents to start cuts");
    click(&mut h, "OK");
    assert!(!is_open(&h));
    assert!(stored(&dir).allow_agent_cuts);
    assert!(stored(&dir).auto_reload);
    // The other preferences are kept.
    assert_eq!(stored(&dir).output_dir, Some(dir.path().join("output")));
}

#[test]
fn the_close_button_and_escape_drop_the_changes() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    for how in ["x", "escape"] {
        open(&mut h, "Agents");
        click(&mut h, "Allow agents to start cuts");
        if how == "x" {
            click(&mut h, CLOSE_NAME);
        } else {
            h.key_press(egui::Key::Escape);
            h.run();
        }
        assert!(!is_open(&h), "{how}");
        assert!(!s.lock().unwrap().prefs.allow_agent_cuts, "{how}");
    }
}

#[test]
fn restore_page_defaults_only_touches_that_page() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    s.lock()
        .unwrap()
        .update_prefs(|p| p.theme = Theme::Dark)
        .unwrap();
    let mut h = harness(&dir, &s);
    open(&mut h, "General");
    click(&mut h, "Reload changed files automatically");
    h.get_by_role_and_label(Role::Tab, "Cutting").click();
    h.run();
    click(&mut h, "Cut strategy: Fast");
    h.get_by_role_and_label(Role::Tab, "Appearance").click();
    h.run();
    assert!(!disabled(&h, "Restore page defaults"));
    click(&mut h, "Restore page defaults");
    assert!(disabled(&h, "Restore page defaults"));
    click(&mut h, "OK");
    let p = stored(&dir);
    assert_eq!(p.theme, Theme::System);
    // The staged edits of the other pages are kept.
    assert!(p.auto_reload);
    assert_eq!(p.default_strategy, CutStrategy::Fast);
    assert_eq!(p.output_dir, Some(dir.path().join("output")));
}

#[test]
fn a_preset_page_has_no_page_defaults_and_apply_stores_the_other_pages() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "General");
    click(&mut h, "Reload changed files automatically");
    h.get_by_role_and_label(Role::Tab, "Plotters").click();
    h.run();
    assert!(disabled(&h, "Restore page defaults"));
    // Apply stores the staged preferences; the preset keeps its own Save.
    click(&mut h, "Apply");
    assert!(stored(&dir).auto_reload);
    assert!(!dir.path().join("profiles").exists());
}

#[test]
fn ok_with_an_unsaved_preset_asks_first() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "General");
    click(&mut h, "Reload changed files automatically");
    h.get_by_role_and_label(Role::Tab, "Plotters").click();
    h.run();
    click(&mut h, "Silhouette Cameo 2");
    click(&mut h, "Save as user preset...");
    h.get_by_label("Preset name").type_text(" X");
    h.run();
    click(&mut h, "Create");
    let edit = &mut app_mut(&mut h).ui.profiles.plotter.as_mut().unwrap().edit;
    edit.scale_x = 1.5;
    h.run();
    click(&mut h, "OK");
    assert!(is_open(&h));
    h.get_by_label("Discard your changes?");
    // Moving between the Plotters and the Mats page keeps the edit.
    h.get_by_role_and_label(Role::Tab, "Mats").click();
    h.run();
    assert_eq!(
        app(&h).ui.profiles.plotter.as_ref().unwrap().edit.scale_x,
        1.5
    );
    click(&mut h, "Discard");
    assert!(!is_open(&h));
    // OK stored the staged preference after the question.
    assert!(stored(&dir).auto_reload);
}

#[test]
fn the_theme_previews_live_and_cancel_takes_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    s.lock()
        .unwrap()
        .update_prefs(|p| p.theme = Theme::Dark)
        .unwrap();
    let mut h = harness(&dir, &s);
    assert_eq!(app(&h).tokens, Tokens::dark());
    open(&mut h, "Appearance");
    click(&mut h, "Theme: Light");
    // Shown at once, not stored.
    assert_eq!(app(&h).tokens, Tokens::light());
    assert_eq!(stored(&dir).theme, Theme::Dark);
    assert_eq!(s.lock().unwrap().prefs.theme, Theme::Dark);
    click(&mut h, "Cancel");
    assert_eq!(app(&h).tokens, Tokens::dark());
    assert_eq!(stored(&dir).theme, Theme::Dark);
}

#[test]
fn search_finds_a_setting_by_a_word_from_its_card() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "General");
    // Cmd+F (Ctrl+F) puts the keyboard into the search field.
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::F);
    h.run();
    h.get_by_label(SEARCH_NAME).type_text("Voltron");
    h.run();
    // "Voltron" is only in the card of Each object.
    click(&mut h, "Each object (Cutting)");
    let st = &app(&h).ui.settings;
    assert_eq!(st.page, Page::Cutting);
    assert_eq!(st.highlight.map(|(id, _)| id), Some("rotate_each"));
    assert!(st.search.is_empty());
    h.get_by_label("Turn and scale each object about its own centre");

    // Enter takes the best result, also on a preset page.
    h.get_by_label(SEARCH_NAME).click();
    h.run();
    h.get_by_label(SEARCH_NAME).type_text("pressure");
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    let st = &app(&h).ui.settings;
    assert_eq!(st.page, Page::Materials);
    assert_eq!(st.highlight.map(|(id, _)| id), Some("material.force"));
}

#[test]
fn search_unfolds_a_folded_section() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Agents");
    click(&mut h, "Agent cuts section");
    assert!(h.query_by_label("Allow agents to start cuts").is_none());
    h.get_by_label(SEARCH_NAME).click();
    h.run();
    h.get_by_label(SEARCH_NAME).type_text("trust");
    h.run();
    click(&mut h, "Agent cuts (Agents)");
    h.get_by_label("Allow agents to start cuts");
}

#[test]
fn applied_panels_and_each_object_take_effect_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Layout");
    click(&mut h, "Objects panel: Collapsed");
    h.get_by_role_and_label(Role::Tab, "Cutting").click();
    h.run();
    click(&mut h, "Turn and scale each object about its own centre");
    click(&mut h, "More Auto arrange spacing");
    click(&mut h, "OK");
    let a = app(&h);
    assert_eq!(
        a.ui.prepare.panels.user(SidePanel::Objects),
        PanelUser::Collapsed
    );
    assert!(a.ui.prepare.fields.each);
    let p = stored(&dir);
    assert_eq!(p.panel_objects, PanelUser::Collapsed);
    assert!(p.rotate_each);
    assert_eq!(p.auto_arrange_spacing_mm, 5.5);
    // The ribbon pins reset to the defaults of a new install.
    s.lock()
        .unwrap()
        .update_prefs(|p| p.ribbon_pins.clear())
        .unwrap();
    open(&mut h, "Layout");
    click(&mut h, "Reset pins");
    click(&mut h, "OK");
    assert_eq!(
        stored(&dir).ribbon_pins,
        flatcraft_engine::prefs::default_ribbon_pins()
    );
}

#[test]
fn the_default_strategy_starts_new_documents_and_the_panel_shows_it() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Cutting");
    click(&mut h, "Cut strategy: Outlines last");
    click(&mut h, "OK");
    assert_eq!(stored(&dir).default_strategy, CutStrategy::OutlinesLast);
    s.lock()
        .unwrap()
        .execute("document.new", json!({}), Origin::User)
        .unwrap();
    assert_eq!(
        s.lock().unwrap().doc().document.job.strategy,
        CutStrategy::OutlinesLast
    );
    to_prepare(&mut h);
    // The Cut and Job panel shows the strategy of the document.
    h.get_by_label("Outlines last");
}

#[test]
fn the_window_fits_at_900_by_600() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness_sized(&dir, s.clone(), [900.0, 600.0], 8, |app| app);
    h.get_by_role_and_label(Role::Button, "Settings").click();
    h.run();
    let window: Rect = h
        .ctx
        .memory(|m| m.area_rect(egui::Id::unique("flatcraft-settings")))
        .expect("the Settings window is drawn");
    let screen = h.ctx.content_rect();
    assert!(screen.contains_rect(window), "{window:?} in {screen:?}");
    for page in Page::ALL {
        h.get_by_role_and_label(Role::Tab, page.name()).click();
        h.run();
        let mut names: Vec<&str> = vec![
            CLOSE_NAME,
            SEARCH_NAME,
            "Restore page defaults",
            "Cancel",
            "Apply",
            "OK",
        ];
        names.extend(Page::ALL.iter().map(|p| p.name()));
        for name in names {
            let node = h
                .query_by_role_and_label(Role::Tab, name)
                .or_else(|| h.query_by_label(name))
                .unwrap_or_else(|| panic!("{name} missing on {}", page.name()));
            let r = node.rect();
            assert!(
                window.contains_rect(r),
                "{name} at {r:?} is outside the window {window:?} on {}",
                page.name()
            );
        }
        // Nothing of the page runs past the window's right edge: pages
        // scroll down, never sideways.
        for shape in common::painted(&h) {
            let r = shape.visual_bounding_rect();
            let inside = r.is_finite()
                && r.min.x >= window.left()
                && r.min.x < window.right()
                && r.min.y >= window.top()
                && r.max.y <= window.bottom();
            if inside {
                assert!(
                    r.max.x <= window.right() + 1.0,
                    "a shape at {r:?} runs past the window {window:?} on {}",
                    page.name()
                );
            }
        }
    }
}

#[test]
fn opening_settings_never_changes_a_stored_value() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    // Above what the stepper offers, as an older build or a hand edit
    // may have stored it.
    s.lock()
        .unwrap()
        .update_prefs(|p| p.auto_arrange_spacing_mm = 180.0)
        .unwrap();
    let mut h = harness(&dir, &s);
    for value in [80.0, 180.0] {
        s.lock()
            .unwrap()
            .update_prefs(|p| p.auto_arrange_spacing_mm = value)
            .unwrap();
        open(&mut h, "Cutting");
        h.get_by_label("Auto arrange spacing");
        let st = &app(&h).ui.settings;
        assert_eq!(st.draft.as_ref().unwrap().auto_arrange_spacing_mm, value);
        assert!(!st.dirty(), "{value}");
        assert!(disabled(&h, "Apply"), "{value}");
        click(&mut h, "OK");
        assert_eq!(stored(&dir).auto_arrange_spacing_mm, value);
    }
}

#[test]
fn cancel_with_unsaved_preset_edits_keeps_everything_until_discard() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "General");
    click(&mut h, "Reload changed files automatically");
    h.get_by_role_and_label(Role::Tab, "Plotters").click();
    h.run();
    click(&mut h, "Silhouette Cameo 2");
    click(&mut h, "Save as user preset...");
    h.get_by_label("Preset name").type_text(" X");
    h.run();
    click(&mut h, "Create");
    app_mut(&mut h)
        .ui
        .profiles
        .plotter
        .as_mut()
        .unwrap()
        .edit
        .scale_x = 1.5;
    h.run();
    for how in ["cancel", "escape"] {
        click(&mut h, "Cancel");
        h.get_by_label("Discard your changes?");
        // The staged preference waits for the answer.
        assert!(app(&h).ui.settings.dirty(), "{how}");
        if how == "cancel" {
            click(&mut h, "Keep editing");
        } else {
            // Escape on the question means Keep editing.
            h.key_press(egui::Key::Escape);
            h.run();
        }
        assert!(is_open(&h), "{how}");
        assert!(h.query_by_label("Discard your changes?").is_none(), "{how}");
        assert!(app(&h).ui.settings.dirty(), "{how}");
        assert_eq!(
            app(&h).ui.profiles.plotter.as_ref().unwrap().edit.scale_x,
            1.5
        );
    }
    // Discard after Cancel drops both.
    click(&mut h, "Cancel");
    click(&mut h, "Discard");
    assert!(!is_open(&h));
    assert!(!stored(&dir).auto_reload);
}

#[test]
fn command_f_alone_gives_the_search_field_the_keyboard() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "General");
    // Without Cmd+F, typed text goes nowhere.
    h.event(egui::Event::Text("theme".into()));
    h.run();
    assert!(app(&h).ui.settings.search.is_empty());
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::F);
    h.run();
    // Raw key events, not aimed at any widget.
    h.event(egui::Event::Text("theme".into()));
    h.run();
    assert_eq!(app(&h).ui.settings.search, "theme");
    h.get_by_label("Theme (Appearance)");
}

#[test]
fn search_scrolls_to_the_calibration_note() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness_sized(&dir, s.clone(), [900.0, 600.0], 8, |app| app);
    h.get_by_role_and_label(Role::Button, "Settings").click();
    h.run();
    let window: Rect = h
        .ctx
        .memory(|m| m.area_rect(egui::Id::unique("flatcraft-settings")))
        .unwrap();
    h.get_by_label(SEARCH_NAME).click();
    h.run();
    h.get_by_label(SEARCH_NAME).type_text("wizard");
    h.run();
    click(&mut h, "Calibration (Mats)");
    assert_eq!(app(&h).ui.settings.page, Page::Mats);
    let note = h
        .get_by_label(flatcraft::ui::profiles_window::CALIBRATION_LATER)
        .rect();
    // Far below the mat form at this size: in view only after the scroll.
    assert!(window.contains_rect(note), "{note:?} not in {window:?}");
    let t = app(&h).tokens;
    let outlined = common::painted(&h).iter().any(|shape| match shape {
        egui::Shape::Rect(r) => r.stroke.color == t.accent_prepare && r.rect.contains_rect(note),
        _ => false,
    });
    assert!(outlined, "the calibration note is not highlighted");
}

#[test]
fn the_panel_choices_are_explained_once() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Layout");
    assert_eq!(
        h.query_all_by_label(settings_window::PANELS_HELP).count(),
        1
    );
    assert!(h.query_by_label_contains("Stays open at every").is_none());
}

/// The Keyboard page lists the drag modifiers from the same table as the
/// Help sheet, with this system's key names.
#[test]
fn the_keyboard_page_lists_the_drag_modifiers() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Shortcuts");
    let (option, cmd) = if cfg!(target_os = "macos") {
        ("Option+drag", "Cmd+drag")
    } else {
        ("Alt+drag", "Ctrl+drag")
    };
    h.get_by_label(option);
    h.get_by_label(cmd);
    h.get_by_label("While dragging a corner handle: rotate instead of resize");
    h.get_by_label("Drag just outside a corner of the selection: rotate it");
    h.get_by_label("While rotating: turn in 15 degree steps");
}

fn click_at(h: &mut AppHarness, at: egui::Pos2) {
    h.drag_at(at);
    h.run();
    h.drop_at(at);
    h.run();
}

/// A number field of the window: a calculation counts, a double-click
/// selects the whole value, and a text it cannot read says why.
#[test]
fn a_number_field_takes_a_calculation_and_a_double_click_selects_it_all() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Cutting");
    let r = h.get_by_label("Auto arrange spacing").rect();
    // On the "5" of "5.0", which a word selection would take alone.
    let at = egui::pos2(r.left() + 6.0, r.center().y);
    h.hover_at(at);
    h.run();
    // The first click turns the drag value into a text field (egui
    // selects it all); the second places the cursor.
    click_at(&mut h, at);
    h.ctx
        .options_mut(|o| o.input_options.max_double_click_delay = 2.0);
    for _ in 0..8 {
        h.step();
    }
    click_at(&mut h, at);
    for _ in 0..8 {
        h.step();
    }
    click_at(&mut h, at);
    click_at(&mut h, at);
    h.run();
    // With only "5" selected this would read "7,.0", which has a dot and
    // a comma; the whole value replaced reads "7,", that is 7.
    h.event(egui::Event::Text("7,".into()));
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    click(&mut h, "OK");
    assert_eq!(stored(&dir).auto_arrange_spacing_mm, 7.0);
}

#[test]
fn a_number_field_refuses_a_value_outside_its_range_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Cutting");
    h.get_by_label("Auto arrange spacing").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run();
    h.event(egui::Event::Text("5in".into()));
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    let (_, hi) = flatcraft_engine::prefs::AUTO_SPACING_RANGE_MM.into_inner();
    h.get_by_label(&format!(
        "127 is too large: give at most {hi} mm. Press Esc to go back."
    ));
    click(&mut h, "OK");
    assert_eq!(stored(&dir).auto_arrange_spacing_mm, 5.0);
}

#[test]
fn ok_counts_a_typed_value_and_restores_one_it_cannot_read() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    let type_without_leaving = |h: &mut AppHarness, text: &str| {
        h.get_by_label("Auto arrange spacing").click();
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
        h.run();
        h.event(egui::Event::Text(text.into()));
        h.run();
    };
    // Typed, still in the field: OK counts it first.
    open(&mut h, "Cutting");
    type_without_leaving(&mut h, "2*3");
    click(&mut h, "OK");
    for _ in 0..3 {
        h.run();
    }
    assert!(!is_open(&h));
    assert_eq!(stored(&dir).auto_arrange_spacing_mm, 6.0);
    // A text it cannot read: the old value stays, a toast says why.
    open(&mut h, "Cutting");
    type_without_leaving(&mut h, "2*");
    click(&mut h, "OK");
    for _ in 0..3 {
        h.run();
    }
    assert!(!is_open(&h));
    assert_eq!(stored(&dir).auto_arrange_spacing_mm, 6.0);
    let toast = app(&h).ui.toasts.last().cloned().unwrap();
    assert_eq!(
        toast.text,
        "Can't read '2*': something is missing after *. Nothing changed."
    );
}

#[test]
fn the_stepper_field_follows_the_click_rules_and_esc_keeps_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Cutting");
    common::check_click_rules(&mut h, "Auto arrange spacing");
    assert!(is_open(&h), "Esc in the field only puts the value back");
    // Esc with no field being typed into closes the window as before.
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(!is_open(&h));
}
