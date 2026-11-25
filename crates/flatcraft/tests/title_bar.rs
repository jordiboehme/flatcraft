//! The app bar as the macOS title bar and the document tabs (Task 40).

mod common;

use common::*;
use egui::accesskit::Role;
use egui::{Pos2, Rect};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::app_bar::{self, BAR_HEIGHT, DIRTY, TRAFFIC_LIGHTS};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::json;

/// egui_kittest draws the app 8 pt in from the window's corner; eframe
/// draws it at the corner.
const ORIGIN: f32 = 8.0;

fn three_documents(s: &SharedSession) {
    // Untitled 1 has unsaved changes; 2 and 3 are new and clean.
    import_square(s);
    for _ in 0..2 {
        s.lock()
            .unwrap()
            .execute("document.new", json!({}), Origin::User)
            .unwrap();
    }
}

fn rig(dir: &tempfile::TempDir, s: SharedSession, title_bar: bool) -> AppHarness {
    let mut h = harness_with(dir, s, |app| app.with_title_bar(title_bar));
    h.run();
    h
}

fn rect(h: &AppHarness, role: Role, name: &str) -> Rect {
    h.get_by_role_and_label(role, name).rect()
}

/// The viewport commands of the last frame.
fn commands(h: &AppHarness) -> Vec<egui::ViewportCommand> {
    h.output()
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map(|v| v.commands.clone())
        .unwrap_or_default()
}

fn starts_drag(h: &AppHarness) -> bool {
    commands(h)
        .iter()
        .any(|c| matches!(c, egui::ViewportCommand::StartDrag))
}

/// Presses at `at`, moves 30 pt to the right in steps and reports whether
/// any frame asked the window to start a drag.
fn drag_from(h: &mut AppHarness, at: Pos2) -> bool {
    let mut started = false;
    h.hover_at(at);
    h.step();
    h.drag_at(at);
    h.step();
    started |= starts_drag(h);
    for dx in [5.0, 10.0, 20.0, 30.0] {
        h.hover_at(at + egui::vec2(dx, 0.0));
        h.step();
        started |= starts_drag(h);
    }
    h.drop_at(at + egui::vec2(30.0, 0.0));
    h.step();
    started |= starts_drag(h);
    h.run();
    started
}

#[test]
fn a_document_with_unsaved_changes_shows_a_dot() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    three_documents(&s);
    let h = rig(&dir, s, true);
    let dirty = format!("Untitled 1 {DIRTY}");
    h.get_by_role_and_label(Role::Button, &dirty);
    h.get_by_role_and_label(Role::Button, "Untitled 2");
    assert!(h.query_by_label("Untitled 1 *").is_none());
    assert!(
        h.query_all_by_role(Role::Button)
            .filter_map(|n| n.accesskit_node().label())
            .all(|l| !l.ends_with(" *")),
        "no tab uses the old star"
    );
}

#[test]
fn the_close_button_sits_inside_its_tab() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    three_documents(&s);
    let mut h = rig(&dir, s.clone(), true);
    for name in [
        format!("Untitled 1 {DIRTY}"),
        "Untitled 2".into(),
        "Untitled 3".into(),
    ] {
        let tab = rect(&h, Role::Button, &name);
        let base = name.trim_end_matches(&format!(" {DIRTY}")).to_string();
        let close = rect(&h, Role::Button, &format!("Close {base}"));
        assert!(tab.contains_rect(close), "{name}: {close:?} not in {tab:?}");
        assert!(tab.height() <= BAR_HEIGHT);
    }
    // The close button closes its own tab, not the active one.
    let before = s.lock().unwrap().docs.len();
    h.get_by_role_and_label(Role::Button, "Close Untitled 2")
        .click();
    h.run();
    assert_eq!(s.lock().unwrap().docs.len(), before - 1);
    assert!(h.query_by_label("Untitled 2").is_none());
}

#[test]
fn a_long_name_is_cut_and_the_tooltip_has_it_whole() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    for _ in 0..9 {
        s.lock()
            .unwrap()
            .execute("document.new", json!({}), Origin::User)
            .unwrap();
    }
    // The active (last) document gets the long name.
    import_square(&s);
    // The tab leaves out ".flatcraft".
    let name = "A very long project name that goes on and on and on";
    let path = dir.path().join(format!("{name}.flatcraft"));
    s.lock()
        .unwrap()
        .execute("project.save", json!({ "path": path }), Origin::User)
        .unwrap();
    let mut h = harness_sized(&dir, s, [1100.0, 800.0], 8, |a| a.with_title_bar(true));
    h.ctx
        .global_style_mut(|st| st.interaction.tooltip_delay = 0.0);
    h.run();
    // The accessible name stays whole while the painted name is cut.
    let r = rect(&h, Role::Button, name);
    let font = egui::FontId::proportional(14.0);
    let whole = h.ctx.fonts_mut(|f| {
        f.layout_no_wrap(name.into(), font, egui::Color32::WHITE)
            .size()
            .x
    });
    assert!(r.width() < whole, "{r:?} is not narrower than {whole}");
    let close = rect(&h, Role::Button, &format!("Close {name}"));
    assert!(r.contains_rect(close));
    h.hover_at(r.center());
    h.run();
    h.run();
    assert!(
        h.query_all_by_label(name).count() >= 2,
        "the tooltip shows the whole name"
    );
}

#[test]
fn dragging_empty_bar_space_moves_the_window_and_buttons_do_not() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    three_documents(&s);
    let mut h = rig(&dir, s.clone(), true);
    // Empty space: between "+" and Settings.
    let plus = rect(&h, Role::Button, "New document");
    let settings = rect(&h, Role::Button, "Settings");
    let empty = egui::pos2((plus.right() + settings.left()) / 2.0, plus.center().y);
    assert!(
        drag_from(&mut h, empty),
        "the empty bar starts a window drag"
    );
    // Inside the traffic-light room too.
    assert!(drag_from(
        &mut h,
        egui::pos2(ORIGIN + TRAFFIC_LIGHTS / 2.0, ORIGIN + BAR_HEIGHT / 2.0)
    ));
    // A drag that starts on a button or a tab never moves the window.
    for name in ["Home", "Save", "New document", "Untitled 2", "Settings"] {
        let at = rect(&h, Role::Button, name).center();
        assert!(!drag_from(&mut h, at), "{name} started a window drag");
    }
    // A click on a button still works.
    let docs = s.lock().unwrap().docs.len();
    h.get_by_role_and_label(Role::Button, "New document")
        .click();
    h.run();
    assert_eq!(s.lock().unwrap().docs.len(), docs + 1);
    // Under a system title bar the bar does not move the window.
    let dir = tempfile::tempdir().unwrap();
    let mut h = rig(&dir, session(&dir), false);
    let plus = rect(&h, Role::Button, "New document");
    let settings = rect(&h, Role::Button, "Settings");
    let empty = egui::pos2((plus.right() + settings.left()) / 2.0, plus.center().y);
    assert!(!drag_from(&mut h, empty));
}

/// Two quick clicks at `at` with the title-bar setting `action`; the
/// viewport commands of those frames.
fn double_click(
    action: app_bar::DoubleClick,
    at: impl Fn(&AppHarness) -> Pos2,
) -> Vec<egui::ViewportCommand> {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    three_documents(&s);
    let mut h = harness_with(&dir, s, move |a| {
        a.with_title_bar(true).with_double_click(action)
    });
    h.run();
    // The harness takes a quarter second per frame and hands the second
    // click a frame later: allow that much time between the clicks.
    h.ctx
        .options_mut(|o| o.input_options.max_double_click_delay = 1.0);
    let at = at(&h);
    h.hover_at(at);
    h.step();
    for _ in 0..2 {
        h.drag_at(at);
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
    }
    let mut out = Vec::new();
    for _ in 0..3 {
        h.step();
        out.extend(commands(&h));
    }
    out
}

fn empty_space(h: &AppHarness) -> Pos2 {
    let plus = rect(h, Role::Button, "New document");
    egui::pos2(plus.right() + 40.0, plus.center().y)
}

fn window_commands(c: &[egui::ViewportCommand]) -> usize {
    c.iter()
        .filter(|c| {
            matches!(
                c,
                egui::ViewportCommand::Maximized(_) | egui::ViewportCommand::Minimized(_)
            )
        })
        .count()
}

#[test]
fn a_double_click_on_empty_space_does_what_the_system_says() {
    use app_bar::DoubleClick;
    let c = double_click(DoubleClick::Zoom, empty_space);
    assert!(c.contains(&egui::ViewportCommand::Maximized(true)), "{c:?}");
    let c = double_click(DoubleClick::Minimize, empty_space);
    assert!(c.contains(&egui::ViewportCommand::Minimized(true)), "{c:?}");
    let c = double_click(DoubleClick::Nothing, empty_space);
    assert_eq!(window_commands(&c), 0, "{c:?}");
}

#[test]
fn a_double_click_on_a_button_or_a_tab_does_not_zoom() {
    use app_bar::DoubleClick;
    // Not Save: it would open the real save dialog for an untitled document.
    for name in ["Home", "Undo", "New document", "Untitled 2", "Settings"] {
        let c = double_click(DoubleClick::Zoom, |h| rect(h, Role::Button, name).center());
        assert_eq!(window_commands(&c), 0, "{name}: {c:?}");
    }
}

#[test]
fn the_title_bar_leaves_the_traffic_lights_free() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    three_documents(&s);
    let h = rig(&dir, s.clone(), true);
    let bar = Rect::from_min_max(
        egui::pos2(0.0, 0.0),
        egui::pos2(f32::INFINITY, ORIGIN + BAR_HEIGHT),
    );
    let mut left = f32::INFINITY;
    for n in h
        .query_all_by_role(Role::Button)
        .chain(h.query_all_by_role(Role::Label))
    {
        let r = n.rect();
        if bar.contains_rect(r) {
            left = left.min(r.left());
        }
    }
    assert!(left >= ORIGIN + TRAFFIC_LIGHTS, "a widget starts at {left}");
    // Every widget of the bar is inside its 32 pt, so the traffic lights
    // sit in the middle of the bar.
    for name in ["Home", "Save", "Settings", "New document", "Untitled 2"] {
        let r = rect(&h, Role::Button, name);
        assert!(
            r.top() >= ORIGIN && r.bottom() <= ORIGIN + BAR_HEIGHT,
            "{name} {r:?}"
        );
        assert!(
            (r.center().y - (ORIGIN + BAR_HEIGHT / 2.0)).abs() <= 0.5,
            "{name} {r:?}"
        );
    }
    // Under a system title bar the bar starts at the left edge.
    let dir = tempfile::tempdir().unwrap();
    let h = rig(&dir, session(&dir), false);
    h.get_by_label("FlatCraft");
    assert!(h.get_by_label("FlatCraft").rect().left() < ORIGIN + TRAFFIC_LIGHTS);
}

/// Structural: the options as built. The real window was checked by hand
/// on macOS 27.2.1 (see the Task 40 report).
#[test]
fn the_viewport_options_of_the_macos_title_bar() {
    let mac = app_bar::viewport(true);
    assert_eq!(mac.fullsize_content_view, Some(true));
    assert_eq!(mac.titlebar_shown, Some(false));
    assert_eq!(mac.title_shown, Some(false));
    // The traffic lights stay.
    assert_ne!(mac.titlebar_buttons_shown, Some(false));
    let other = app_bar::viewport(false);
    assert_eq!(other.fullsize_content_view, None);
    assert_eq!(other.titlebar_shown, None);
    assert_eq!(other.title_shown, None);
    for v in [mac, other] {
        assert_eq!(v.title.as_deref(), Some("FlatCraft"));
        assert_eq!(
            v.min_inner_size,
            Some(egui::Vec2::from(flatcraft::ui::MIN_WINDOW))
        );
    }
}

#[test]
fn the_bar_explains_itself_on_hover() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    three_documents(&s);
    let mut h = rig(&dir, s, true);
    h.ctx
        .global_style_mut(|st| st.interaction.tooltip_delay = 0.0);
    let hover = |h: &mut AppHarness, role: Role, name: &str| {
        let at = rect(h, role, name).center();
        h.hover_at(at);
        h.run();
        h.run();
    };
    // The dot is explained on a tab with unsaved changes.
    hover(&mut h, Role::Button, &format!("Untitled 1 {DIRTY}"));
    h.get_by_label(app_bar::TAB_DIRTY_HINT);
    // A dimmed Redo says why.
    hover(&mut h, Role::Button, "Redo");
    h.get_by_label("There is nothing to redo. Redo brings back what Undo took back.");
    for name in ["Home", "Save", "New document", "Close Untitled 2"] {
        hover(&mut h, Role::Button, name);
        assert!(
            h.query_all_by_label(name).count() >= 2,
            "{name} has a card with its title"
        );
    }
}
