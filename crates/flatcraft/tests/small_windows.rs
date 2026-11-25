//! Small windows: the ribbon, the Cut bar and the app bar at 900, 1100,
//! 1400 and 1920 points. Nothing is cut off or out of reach.

mod common;

use common::*;
use egui::accesskit::Role;
use egui::{Rect, Shape};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::collapse::{self, Level};
use flatcraft::ui::{self, app_bar, ribbon};
use flatcraft_engine::prefs::ARRANGE_PIN_IDS;
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::json;

const WIDTHS: [f32; 4] = [900.0, 1100.0, 1400.0, 1920.0];

fn sized(dir: &tempfile::TempDir, s: SharedSession, width: f32) -> AppHarness {
    let height = if width <= 900.0 { 600.0 } else { 900.0 };
    let mut h = harness_sized(dir, s, [width, height], 8, |app| app);
    to_prepare(&mut h);
    h
}

fn select(s: &SharedSession, ids: &[u64]) {
    s.lock()
        .unwrap()
        .execute("object.select", json!({ "ids": ids }), Origin::User)
        .unwrap();
}

fn pin(s: &SharedSession, pins: &[&str]) {
    let pins: Vec<String> = pins.iter().map(|p| p.to_string()).collect();
    s.lock()
        .unwrap()
        .update_prefs(|p| p.arrange_pins = pins)
        .unwrap();
}

fn search_rect(h: &AppHarness) -> Rect {
    h.get_by_label("Search commands").rect()
}

fn window(h: &AppHarness) -> Rect {
    h.ctx.content_rect()
}

/// The names of a tab's tools, with the role to find each by.
fn tools(tab: &str, pins: &[&str]) -> Vec<(Role, String)> {
    let b = |n: &str| (Role::Button, n.to_string());
    match tab {
        "Home" => [
            "Import",
            "Trace",
            "Test patterns",
            "Duplicate",
            "Copies...",
            "Delete",
            "Reload",
        ]
        .map(b)
        .to_vec(),
        "Arrange" => {
            let mut v = vec![
                (Role::TextInput, "Width (mm)".to_string()),
                (Role::TextInput, "Height (mm)".to_string()),
                (Role::CheckBox, "Keep aspect".to_string()),
                (Role::TextInput, "Scale (%)".to_string()),
                b("Arrange"),
            ];
            for cmd in ui::arrange::ArrangeCmd::ALL {
                if pins.contains(&cmd.id()) {
                    v.push(b(cmd.label().trim_end_matches(if cmd.big() {
                        "..."
                    } else {
                        ""
                    })));
                }
            }
            v.push((Role::CheckBox, "Each object".to_string()));
            v
        }
        _ => [
            "Zoom to fit",
            "Zoom 100%",
            "Grid",
            "Rulers",
            "Objects panel",
            "Settings panel",
            "Debug panel",
        ]
        .map(b)
        .to_vec(),
    }
}

/// Whether the node is shown whole: inside the ribbon's margin (8 pt, the
/// bar clips there) and clear of the search box.
fn on_ribbon(h: &AppHarness, r: Rect) -> bool {
    let inside = window(h).shrink2(egui::vec2(7.5, 0.0));
    inside.contains_rect(r) && !search_rect(h).intersects(r.shrink(0.5))
}

fn find<'a>(h: &'a AppHarness, role: Role, name: &str) -> Vec<egui_kittest::Node<'a>> {
    h.query_all_by_role(role)
        .filter(|n| n.accesskit_node().label().as_deref() == Some(name))
        .collect()
}

/// Checks every tool of the current tab: shown whole on the ribbon, or
/// listed under "»". Returns how many were in the list.
fn check_tab(h: &mut AppHarness, tab: &str, pins: &[&str], width: f32) -> usize {
    let mut listed = Vec::new();
    for (role, name) in tools(tab, pins) {
        let found = find(h, role, &name);
        assert!(found.len() <= 1, "{width}: {name} twice");
        match found.first() {
            Some(n) => assert!(
                on_ribbon(h, n.rect()),
                "{width} {tab}: {name} is cut off at {:?} (search at {:?})",
                n.rect(),
                search_rect(h)
            ),
            None => listed.push((role, name)),
        }
    }
    if listed.is_empty() {
        assert!(
            h.query_by_label(collapse::MORE).is_none(),
            "{width} {tab}: a » with nothing in it"
        );
        return 0;
    }
    let more = h.get_by_label(collapse::MORE);
    assert!(on_ribbon(h, more.rect()), "{width} {tab}: » is cut off");
    more.click();
    h.run();
    for (role, name) in &listed {
        let found = find(h, *role, name);
        assert_eq!(
            found.len(),
            1,
            "{width} {tab}: {name} is neither shown nor listed"
        );
        let r = found[0].rect();
        assert!(
            window(h).contains_rect(r),
            "{width} {tab}: {name} listed off-screen"
        );
    }
    // Close the list.
    h.get_by_label(collapse::MORE).click();
    h.run();
    listed.len()
}

fn show_tab(h: &mut AppHarness, tab: &str) {
    h.get_by_role_and_label(Role::Tab, tab).click();
    h.run();
}

#[test]
fn every_ribbon_command_is_reachable_at_every_width() {
    let all: Vec<&str> = ARRANGE_PIN_IDS.to_vec();
    let defaults = flatcraft_engine::prefs::default_arrange_pins();
    let defaults: Vec<&str> = defaults.iter().map(String::as_str).collect();
    for width in WIDTHS {
        for pins in [&defaults[..], &all[..], &[][..]] {
            let dir = tempfile::tempdir().unwrap();
            let s = session(&dir);
            let a = import_square(&s);
            let b = import_square(&s);
            select(&s, &[a, b]);
            pin(&s, pins);
            let mut h = sized(&dir, s.clone(), width);
            let mut listed = 0;
            for tab in ["Home", "Arrange", "View"] {
                show_tab(&mut h, tab);
                assert!(
                    window(&h).contains_rect(search_rect(&h)),
                    "{width}: search off-screen"
                );
                listed += check_tab(&mut h, tab, pins, width);
            }
            if width >= 1920.0 {
                assert_eq!(listed, 0, "{width}: nothing needs the » list");
            }
        }
    }
}

#[test]
fn a_listed_tool_works_and_closes_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    pin(&s, &ARRANGE_PIN_IDS);
    select(&s, &[a]);
    let mut h = sized(&dir, s.clone(), 900.0);
    show_tab(&mut h, "Arrange");
    assert!(
        matches!(
            collapse::level(&h.ctx, ribbon::bar_id()),
            Some(Level::Overflow(_))
        ),
        "every pin does not fit 900 pt"
    );
    h.get_by_label(collapse::MORE).click();
    h.run();
    let undo = undo_len(&s);
    h.get_by_role_and_label(Role::Button, "Mirror vertically")
        .click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), undo + 1, "mirrored from the list");
    assert!(
        h.query_by_role_and_label(Role::Button, "Mirror vertically")
            .is_none(),
        "the list closed"
    );
}

#[test]
fn search_opens_the_arrange_drop_down_on_a_collapsed_ribbon() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    pin(&s, &ARRANGE_PIN_IDS);
    select(&s, &[a]);
    let mut h = sized(&dir, s.clone(), 900.0);
    show_tab(&mut h, "Home");
    h.get_by_label("Search commands").click();
    h.run();
    h.get_by_label("Search commands").type_text("mirror");
    h.run();
    h.get_by_role_and_label(Role::Button, "Mirror (search)")
        .click();
    h.run();
    assert!(
        matches!(
            collapse::level(&h.ctx, ribbon::bar_id()),
            Some(Level::Overflow(_))
        ),
        "the Arrange tab is collapsed at 900 pt with every pin"
    );
    let undo = undo_len(&s);
    h.get_by_label("Mirror horizontally (Arrange)").click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), undo + 1, "mirrored from the drop-down");
}

#[test]
fn resizing_settles_without_flipping() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    pin(&s, &ARRANGE_PIN_IDS);
    let mut h = sized(&dir, s, 1920.0);
    show_tab(&mut h, "Arrange");
    let wide = collapse::level(&h.ctx, ribbon::bar_id());
    let mut seen = Vec::new();
    for w in [
        1920.0, 1600.0, 1300.0, 1100.0, 1000.0, 900.0, 1000.0, 1300.0, 1920.0,
    ] {
        h.set_size(egui::vec2(w, 900.0));
        // `run` fails when the frames keep asking for more.
        h.run();
        let level = collapse::level(&h.ctx, ribbon::bar_id());
        h.step();
        assert_eq!(
            collapse::level(&h.ctx, ribbon::bar_id()),
            level,
            "{w}: stable"
        );
        seen.push((w, level));
    }
    assert_eq!(seen.last().unwrap().1, wide, "{seen:?}");
    assert_eq!(seen[0].1, Some(Level::Full), "{seen:?}");
    assert_ne!(seen[5].1, Some(Level::Full), "{seen:?}");
}

/// Every text painted last frame with the clip rect it was painted in.
fn texts(h: &AppHarness) -> Vec<(String, Rect, Rect)> {
    fn walk(shape: &Shape, clip: Rect, out: &mut Vec<(String, Rect, Rect)>) {
        match shape {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            Shape::Text(t) => out.push((
                t.galley.job.text.clone(),
                t.galley.rect.translate(t.pos.to_vec2()),
                clip,
            )),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in &h.output().shapes {
        walk(&c.shape, c.clip_rect, &mut out);
    }
    out
}

/// Asserts that `value` is painted, and whole.
fn shown_whole(h: &AppHarness, value: &str, width: f32) {
    let hits: Vec<_> = texts(h).into_iter().filter(|t| t.0 == value).collect();
    assert!(
        !hits.is_empty(),
        "{width}: {value} is not painted: {:?}",
        texts(h)
            .iter()
            .map(|t| t.0.clone())
            .filter(|t| t.chars().any(|c| c.is_ascii_digit()))
            .collect::<Vec<_>>()
    );
    for (_, r, clip) in hits {
        assert!(
            clip.expand(0.5).contains_rect(r),
            "{width}: {value} is cut: text {r:?}, clip {clip:?}"
        );
    }
}

#[test]
fn no_number_field_cuts_its_value() {
    for width in WIDTHS {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        let a = import_square(&s);
        s.lock()
            .unwrap()
            .execute(
                "object.scale",
                json!({ "ids": [a], "width_mm": 1250.25, "keep_aspect": false }),
                Origin::User,
            )
            .unwrap();
        select(&s, &[a]);
        let mut h = sized(&dir, s.clone(), width);
        show_tab(&mut h, "Arrange");
        wait_for_workers(&mut h);
        shown_whole(&h, "1250.25", width);
        shown_whole(&h, "50.00", width);
        shown_whole(&h, "100", width);
        // The panel fields, with their longest values.
        app_mut(&mut h).ui.prepare.panel_last.move_copy.dx_mm = -9999.99;
        app_mut(&mut h).ui.prepare.panel_last.rotate.degrees = -359.99;
        let ctx = h.ctx.clone();
        let snap = flatcraft::ui::prepare::snapshot(app_mut(&mut h), &ctx);
        flatcraft::ui::arrange::open(
            &mut app_mut(&mut h).ui.prepare,
            &snap,
            flatcraft::ui::arrange::PanelKind::MoveCopy,
        );
        h.run();
        shown_whole(&h, "-9999.99", width);
        flatcraft::ui::arrange::open(
            &mut app_mut(&mut h).ui.prepare,
            &snap,
            flatcraft::ui::arrange::PanelKind::Rotate,
        );
        h.run();
        shown_whole(&h, "-359.99", width);
    }
}

#[test]
fn the_cut_bar_fits_at_every_width() {
    for width in WIDTHS {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        import_square(&s);
        let mut h = sized(&dir, s, width);
        app_mut(&mut h).ui.workspace = ui::Workspace::Cut;
        h.run();
        let w = window(&h);
        let debug = h.get_by_label("Debug panel").rect();
        assert!(w.contains_rect(debug), "{width}: Debug panel off-screen");
        for name in ["Device & Material", "Path", "Simulate", "Send"] {
            let found = find(&h, Role::Tab, name);
            let r = match found.first() {
                Some(n) => n.rect(),
                None => panic!("{width}: {name} missing"),
            };
            assert!(
                w.contains_rect(r) && r.max.x <= debug.min.x,
                "{width}: {name} cut off at {r:?}"
            );
        }
    }
}

fn new_documents(s: &SharedSession, n: usize) {
    for _ in 0..n {
        s.lock()
            .unwrap()
            .execute("document.new", json!({}), Origin::User)
            .unwrap();
    }
}

#[test]
fn the_active_tab_stays_visible_with_12_documents() {
    for width in WIDTHS {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        new_documents(&s, 11);
        let names: Vec<String> = (1..=12).map(|n| format!("Untitled {n}")).collect();
        let mut h = sized(&dir, s.clone(), width);
        let settings = h.get_by_label("Settings").rect();
        let help = h.get_by_label("Help").rect();
        let version = h
            .get_by_label(concat!("v", env!("CARGO_PKG_VERSION")))
            .rect();
        for r in [settings, help, version] {
            assert!(window(&h).contains_rect(r), "{width}: {r:?} off-screen");
        }
        // The left side ends before Settings.
        let left_of_settings = |h: &AppHarness, name: &str| {
            if let Some(n) = h.query_by_label(name) {
                let r = n.rect();
                assert!(
                    window(h).contains_rect(r) && r.max.x <= settings.min.x,
                    "{width}: {name} at {r:?}, Settings at {settings:?}"
                );
            }
        };
        // Each document in turn is the active one.
        for (i, name) in names.iter().enumerate() {
            let id = s.lock().unwrap().docs[i].id();
            s.lock()
                .unwrap()
                .execute("document.switch", json!({ "doc_id": id }), Origin::User)
                .ok();
            app_mut(&mut h).ui.workspace = ui::Workspace::Prepare;
            h.run();
            let active = s.lock().unwrap().doc().id();
            assert_eq!(active, id, "switched");
            h.get_by_label("New document");
            left_of_settings(&h, "New document");
            left_of_settings(&h, app_bar::ALL_DOCUMENTS);
            let tab = h.get_by_role_and_label(Role::Button, name);
            let r = tab.rect();
            assert!(
                window(&h).contains_rect(r) && r.max.x <= settings.min.x,
                "{width}: active tab {name} at {r:?}"
            );
        }
        // Every document is a tab or in the list.
        let missing: Vec<&String> = names
            .iter()
            .filter(|n| h.query_by_role_and_label(Role::Button, n).is_none())
            .collect();
        if missing.is_empty() {
            assert!(h.query_by_label(app_bar::ALL_DOCUMENTS).is_none());
            continue;
        }
        h.get_by_label(app_bar::ALL_DOCUMENTS).click();
        h.run();
        for n in &names {
            h.get_by_label(&format!("{n} (documents)"));
        }
        // Picking a hidden one from the list makes it active and shows
        // its tab.
        let hidden = missing[0].clone();
        h.get_by_label(&format!("{hidden} (documents)")).click();
        h.run();
        assert!(h.query_by_role_and_label(Role::Button, &hidden).is_some());
        assert!(
            h.query_by_label(&format!("{hidden} (documents)")).is_none(),
            "list closed"
        );
    }
}
