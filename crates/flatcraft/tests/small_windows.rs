//! Small windows: the ribbon, the Cut bar and the app bar at 900, 1100,
//! 1400 and 1920 points. Nothing is cut off or out of reach: the ribbon's
//! groups collapse from the right to their tiles, whose lists hold every
//! action.

mod common;

use common::*;
use egui::accesskit::Role;
use egui::{Rect, Shape};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::collapse::{self, Level};
use flatcraft::ui::tools::{self, GroupId, Tool};
use flatcraft::ui::{self, app_bar, ribbon};
use flatcraft_engine::prefs::RIBBON_PIN_IDS;
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
        .update_prefs(|p| p.ribbon_pins = pins)
        .unwrap();
}

fn search_rect(h: &AppHarness) -> Rect {
    h.get_by_label("Search commands").rect()
}

fn window(h: &AppHarness) -> Rect {
    h.ctx.content_rect()
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

/// The fields and switches of a tab that are not actions.
fn fields(tab: &str) -> Vec<(Role, &'static str)> {
    match tab {
        "Layout" => vec![
            (Role::TextInput, "X (mm)"),
            (Role::TextInput, "Y (mm)"),
            (Role::TextInput, "Width (mm)"),
            (Role::TextInput, "Height (mm)"),
            (Role::CheckBox, "Keep aspect"),
            (Role::TextInput, "Scale (%)"),
        ],
        _ => Vec::new(),
    }
}

fn groups_of(tab: &str) -> Vec<GroupId> {
    tools::GROUPS
        .iter()
        .filter(|g| g.tab.name() == tab && g.id != GroupId::Size)
        .map(|g| g.id)
        .collect()
}

/// Checks every action of the current tab: each group's button or tile is
/// shown whole, a pinned button that is shown is whole, and the group's
/// list has a row for every action (both Align options). Nothing is
/// listed under a "»". Returns how many groups were tiles.
fn check_tab(h: &mut AppHarness, tab: &str, width: f32) -> usize {
    assert!(
        h.query_by_label(collapse::MORE).is_none(),
        "{width} {tab}: the ribbon needs no » list"
    );
    for (role, name) in fields(tab) {
        let found = find(h, role, name);
        assert_eq!(found.len(), 1, "{width} {tab}: {name}");
        assert!(
            on_ribbon(h, found[0].rect()),
            "{width} {tab}: {name} is cut off"
        );
    }
    let mut tiles = 0;
    for g in groups_of(tab) {
        let def = g.def();
        let found = find(h, Role::Button, def.title);
        assert_eq!(found.len(), 1, "{width} {tab}: group {}", def.title);
        let r = found[0].rect();
        assert!(
            on_ribbon(h, r),
            "{width} {tab}: {} is cut off at {r:?}",
            def.title
        );
        let pinned_shown = g
            .tools()
            .filter(|t| !find(h, Role::Button, &t.button_name()).is_empty())
            .count();
        if pinned_shown == 0 {
            tiles += 1;
        }
        for t in g.tools() {
            let found = find(h, Role::Button, &t.button_name());
            assert!(found.len() <= 1, "{width}: {} twice", t.button_name());
            for n in found {
                assert!(
                    on_ribbon(h, n.rect()),
                    "{width} {tab}: {} is cut off",
                    t.button_name()
                );
            }
        }
        // The list holds every action.
        h.get_by_role_and_label(Role::Button, def.title).click();
        h.run();
        for to_area in [false, true] {
            if g == GroupId::Arrange {
                let option = if to_area {
                    "To the cutting area"
                } else {
                    "To the selection"
                };
                h.get_by_label(option).click();
                h.run();
            }
            let want: Vec<&Tool> = g
                .tools()
                .filter(|t| match t.option() {
                    Some(o) => o == tools::AlignTo::of(to_area),
                    None => true,
                })
                .collect();
            for t in want {
                assert!(
                    find(h, Role::Button, &t.button_name()).len() <= 1,
                    "{width}: {} twice with the list open",
                    t.button_name()
                );
                let rows = find(h, Role::Button, &t.row_name());
                assert_eq!(rows.len(), 1, "{width} {tab}: no row {}", t.row_name());
                assert!(
                    window(h).contains_rect(rows[0].rect()),
                    "{width} {tab}: {} listed off-screen",
                    t.row_name()
                );
            }
        }
        // "Each object" is on the tab, or in the list while the group is a
        // tile.
        if g == GroupId::Arrange {
            let each = find(h, Role::CheckBox, "Each object");
            assert_eq!(each.len(), 1, "{width} {tab}: Each object");
            assert!(window(h).contains_rect(each[0].rect()));
        }
        h.key_press(egui::Key::Escape);
        h.run();
    }
    tiles
}

fn show_tab(h: &mut AppHarness, tab: &str) {
    h.get_by_role_and_label(Role::Tab, tab).click();
    h.run();
}

#[test]
fn every_ribbon_action_is_reachable_at_every_width() {
    let all: Vec<&str> = RIBBON_PIN_IDS.to_vec();
    let defaults = flatcraft_engine::prefs::default_ribbon_pins();
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
            for tab in ["Layout", "View"] {
                show_tab(&mut h, tab);
                assert!(
                    window(&h).contains_rect(search_rect(&h)),
                    "{width}: search off-screen"
                );
                let tiles = check_tab(&mut h, tab, width);
                if width >= 1920.0 && !pins.is_empty() && pins.len() < all.len() {
                    assert_eq!(tiles, 0, "{width} {tab}: nothing collapses");
                }
            }
        }
    }
}

#[test]
fn the_default_pins_are_the_buttons_the_ribbon_had() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = sized(&dir, s, 1920.0);
    let want = [
        (
            "Layout",
            &[
                "Import",
                "Trace",
                "Test patterns",
                "Duplicate",
                "Copies",
                "Delete",
                "Reload",
                "Move / Copy",
                "Rectangular pattern",
                "Rotate -90",
                "Rotate +90",
                "Mirror horizontally",
                "Mirror vertically",
            ][..],
        ),
        (
            "View",
            &[
                "Zoom to fit",
                "Zoom 100%",
                "Grid",
                "Rulers",
                "Objects panel",
                "Settings panel",
                "Debug panel",
            ][..],
        ),
    ];
    for (tab, names) in want {
        show_tab(&mut h, tab);
        let groups = groups_of(tab);
        let mut shown: Vec<String> = tools::TOOLS
            .iter()
            .filter(|t| groups.contains(&t.group))
            .map(|t| t.button_name())
            .filter(|n| !find(&h, Role::Button, n).is_empty())
            .collect();
        shown.sort();
        let mut names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        names.sort();
        assert_eq!(shown, names, "{tab}");
    }
}

#[test]
fn a_listed_action_works_and_closes_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    pin(&s, &RIBBON_PIN_IDS);
    select(&s, &[a]);
    let mut h = sized(&dir, s.clone(), 900.0);
    show_tab(&mut h, "Layout");
    assert!(
        matches!(
            collapse::level(&h.ctx, ribbon::bar_id()),
            Some(Level::Folded(_))
        ),
        "every pin does not fit 900 pt"
    );
    assert!(
        h.query_by_role_and_label(Role::Button, "Mirror vertically")
            .is_none(),
        "the Arrange group is a tile"
    );
    h.get_by_role_and_label(Role::Button, "Arrange").click();
    h.run();
    let undo = undo_len(&s);
    h.get_by_label("Mirror vertically (Arrange)").click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), undo + 1, "mirrored from the list");
    assert!(
        h.query_by_label("Mirror vertically (Arrange)").is_none(),
        "the list closed"
    );
}

#[test]
fn search_opens_a_list_on_a_collapsed_ribbon() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    pin(&s, &RIBBON_PIN_IDS);
    select(&s, &[a]);
    let mut h = sized(&dir, s.clone(), 900.0);
    show_tab(&mut h, "Layout");
    h.get_by_label("Search commands").click();
    h.run();
    h.get_by_label("Search commands").type_text("distribute");
    h.run();
    // One object: Distribute cannot run, so search opens its list.
    h.get_by_role_and_label(Role::Button, "Distribute vertically (search)")
        .click();
    h.run();
    assert!(
        matches!(
            collapse::level(&h.ctx, ribbon::bar_id()),
            Some(Level::Folded(_))
        ),
        "the Layout tab is collapsed at 900 pt with every pin"
    );
    assert!(
        h.get_by_label("Distribute vertically (Arrange)")
            .accesskit_node()
            .is_disabled()
    );
    let undo = undo_len(&s);
    h.get_by_label("Mirror horizontally (Arrange)").click();
    h.run();
    wait_for_workers(&mut h);
    assert_eq!(undo_len(&s), undo + 1, "mirrored from the list");
}

#[test]
fn the_first_frame_after_a_tab_switch_already_fits() {
    // Every level change is measured exactly, so one frame (with its three
    // passes) settles: no pass that cannot be discarded is too wide.
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    pin(&s, &RIBBON_PIN_IDS);
    select(&s, &[a]);
    let mut h = sized(&dir, s, 900.0);
    show_tab(&mut h, "Layout");
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    h.step();
    let level = collapse::level(&h.ctx, ribbon::bar_id());
    assert!(matches!(level, Some(Level::Folded(_))), "{level:?}");
    for g in groups_of("Layout") {
        let r = h.get_by_role_and_label(Role::Button, g.def().title).rect();
        assert!(on_ribbon(&h, r), "{} at {r:?}", g.def().title);
    }
    h.step();
    assert_eq!(
        collapse::level(&h.ctx, ribbon::bar_id()),
        level,
        "the next frame keeps it"
    );
}

#[test]
fn resizing_settles_without_flipping() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    pin(&s, &RIBBON_PIN_IDS);
    let mut h = sized(&dir, s, 1920.0);
    show_tab(&mut h, "Layout");
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

#[test]
fn a_field_that_shrinks_back_lets_the_ribbon_grow_again() {
    // Long values widen the W and H fields and the bar collapses further;
    // once they are short again, the bar measures again and ends where it
    // was, never stuck icon-only.
    let mut changed = 0;
    for width in WIDTHS {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        let a = import_square(&s);
        select(&s, &[a]);
        pin(&s, &RIBBON_PIN_IDS);
        let mut h = sized(&dir, s.clone(), width);
        show_tab(&mut h, "Layout");
        wait_for_workers(&mut h);
        let before = collapse::level(&h.ctx, ribbon::bar_id());
        let size = |h: &mut AppHarness, mm: f64| {
            for key in ["width_mm", "height_mm"] {
                s.lock()
                    .unwrap()
                    .execute(
                        "object.scale",
                        json!({ "ids": [a], key: mm, "keep_aspect": false }),
                        Origin::User,
                    )
                    .unwrap();
            }
            h.run();
            wait_for_workers(h);
        };
        size(&mut h, 123456.75);
        if collapse::level(&h.ctx, ribbon::bar_id()) != before {
            changed += 1;
        }
        size(&mut h, 50.0);
        assert_eq!(
            collapse::level(&h.ctx, ribbon::bar_id()),
            before,
            "{width}: back where it was once the text is short"
        );
    }
    // At the narrowest window where the Layout tab is still Full, the
    // long values make it step down, and short ones bring it back.
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    select(&s, &[a]);
    let mut h = sized(&dir, s.clone(), 1400.0);
    show_tab(&mut h, "Layout");
    let mut w = 1400.0;
    while w > 600.0 {
        h.set_size(egui::vec2(w - 10.0, 900.0));
        h.run();
        if collapse::level(&h.ctx, ribbon::bar_id()) != Some(Level::Full) {
            break;
        }
        w -= 10.0;
    }
    // A fresh window of that width (the bar steps up only with room to
    // spare, so going back by 10 pt would not show Full again).
    let mut h = sized(&dir, s.clone(), w);
    show_tab(&mut h, "Layout");
    wait_for_workers(&mut h);
    assert_eq!(
        collapse::level(&h.ctx, ribbon::bar_id()),
        Some(Level::Full),
        "{w}"
    );
    for (mm, full) in [(123456.75, false), (50.0, true)] {
        for key in ["width_mm", "height_mm"] {
            s.lock()
                .unwrap()
                .execute(
                    "object.scale",
                    json!({ "ids": [a], key: mm, "keep_aspect": false }),
                    Origin::User,
                )
                .unwrap();
        }
        h.run();
        wait_for_workers(&mut h);
        if !full {
            changed += 1;
        }
        assert_eq!(
            collapse::level(&h.ctx, ribbon::bar_id()) == Some(Level::Full),
            full,
            "{w} pt with {mm} mm"
        );
    }
    assert!(changed > 0);
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
        show_tab(&mut h, "Layout");
        wait_for_workers(&mut h);
        shown_whole(&h, "1250.25", width);
        shown_whole(&h, "50.00", width);
        shown_whole(&h, "100.00", width);
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
        // From 900 pt up every step shows its title and status line, and
        // the bar stays as it is.
        let level = collapse::level(&h.ctx, ui::cut::bar_id());
        assert_eq!(level, Some(Level::Full), "{width}");
        h.step();
        assert_eq!(collapse::level(&h.ctx, ui::cut::bar_id()), level, "{width}");
        shown_whole(&h, "2  Path", width);
        shown_whole(&h, "File · ready", width);
    }
}

#[test]
fn the_cut_bar_folds_to_numbers_then_to_a_list() {
    // Narrower than a window can be, to walk the whole fold ladder.
    for (width, want) in [(600.0, Level::Folded(1)), (480.0, Level::Folded(2))] {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        import_square(&s);
        let mut h = harness_sized(&dir, s, [width, 600.0], 8, |app| app);
        app_mut(&mut h).ui.workspace = ui::Workspace::Cut;
        h.run();
        let level = collapse::level(&h.ctx, ui::cut::bar_id());
        assert_eq!(level, Some(want), "{width}");
        h.step();
        assert_eq!(collapse::level(&h.ctx, ui::cut::bar_id()), level, "{width}");
        if want == Level::Folded(1) {
            // Icon and number only.
            shown_whole(&h, "4", width);
            assert!(texts(&h).iter().all(|t| t.0 != "File · ready"));
        } else {
            assert!(find(&h, Role::Tab, "Send").is_empty(), "{width}");
            h.get_by_label(collapse::MORE).click();
            h.run();
        }
        // A folded step says its status in its card, in plain words; a
        // dimmed one says it once, in its reason.
        if want == Level::Folded(1) {
            h.ctx
                .global_style_mut(|st| st.interaction.tooltip_delay = 0.0);
            let wait = |h: &mut AppHarness, said: &str| {
                let start = std::time::Instant::now();
                while h.query_by_label_contains(said).is_none() {
                    assert!(
                        start.elapsed() < std::time::Duration::from_secs(10),
                        "{said:?} did not show"
                    );
                    h.step();
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            };
            let at = h.get_by_role_and_label(Role::Tab, "Send").rect().center();
            h.hover_at(at);
            wait(&mut h, "Ready to write the job to a file.");
            let at = h
                .get_by_role_and_label(Role::Tab, "Simulate")
                .rect()
                .center();
            h.hover_at(at);
            wait(&mut h, ui::cut::SIMULATE_HINT);
            h.get_by_label(ui::cut::Stage::Simulate.hint());
            assert!(
                h.query_by_label_contains("Simulate is not ready yet")
                    .is_none(),
                "the reason is said twice"
            );
        }
        // Every stage is reachable and works.
        h.get_by_role_and_label(Role::Tab, "Send").click();
        h.run();
        assert_eq!(app(&h).ui.cut.stage, ui::cut::Stage::Send, "{width}");
    }
}

#[test]
fn the_agent_pill_leaves_room_for_the_active_tab() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    new_documents(&s, 11);
    let mut h = harness_sized(&dir, s.clone(), [900.0, 600.0], 8, |app| app.with_agent());
    to_prepare(&mut h);
    let pill = h.get_by_label("Agent idle").rect();
    let settings = h.get_by_label("Settings").rect();
    assert!(window(&h).contains_rect(pill), "pill off-screen");
    for i in [0, 6, 11] {
        let id = s.lock().unwrap().docs[i].id();
        s.lock()
            .unwrap()
            .execute("document.switch", json!({ "doc_id": id }), Origin::User)
            .ok();
        h.run();
        let name = format!("Untitled {}", i + 1);
        let tab = h.get_by_role_and_label(Role::Button, &name).rect();
        let right = pill.min.x.min(settings.min.x);
        assert!(
            window(&h).contains_rect(tab) && tab.max.x <= right,
            "{name} at {tab:?}, the right side starts at {right}"
        );
        for n in ["New document", app_bar::ALL_DOCUMENTS] {
            let r = h.get_by_label(n).rect();
            assert!(r.max.x <= right, "{n} at {r:?}");
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
