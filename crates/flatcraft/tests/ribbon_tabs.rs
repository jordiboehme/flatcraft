//! The ribbon's look (Task 40): folder tabs that open into the ribbon
//! panel, one tools row at the same place on every tab, big buttons for
//! every pinned action and compact switches inside their group.

mod common;

use common::*;
use egui::Rect;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::groups::{BIG_HEIGHT, COMPACT_HEIGHT, ROW_HEIGHT};
use flatcraft::ui::prepare::RibbonTab;
use flatcraft::ui::ribbon::Layout;
use flatcraft::ui::tools::{self, GroupId};
use flatcraft_engine::prefs::RIBBON_PIN_IDS;
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::json;

const WIDTHS: [f32; 4] = [900.0, 1100.0, 1400.0, 1920.0];

fn rig(dir: &tempfile::TempDir, s: SharedSession, width: f32) -> AppHarness {
    let height = if width <= 900.0 { 600.0 } else { 900.0 };
    let mut h = harness_sized(dir, s, [width, height], 8, |app| app);
    to_prepare(&mut h);
    h
}

fn select_all(s: &SharedSession, ids: &[u64]) {
    s.lock()
        .unwrap()
        .execute("object.select", json!({ "ids": ids }), Origin::User)
        .unwrap();
}

fn show_tab(h: &mut AppHarness, tab: RibbonTab) {
    h.get_by_role_and_label(Role::Tab, tab.name()).click();
    h.run();
}

fn layout(h: &AppHarness) -> Layout {
    app(h)
        .ui
        .prepare
        .ribbon_layout
        .expect("the ribbon was drawn")
}

fn node_rect(h: &AppHarness, role: Role, name: &str) -> Rect {
    let found: Vec<_> = h
        .query_all_by_role(role)
        .filter(|n| n.accesskit_node().label().as_deref() == Some(name))
        .collect();
    assert_eq!(found.len(), 1, "{name}");
    found[0].rect()
}

#[test]
fn the_tools_row_has_the_same_top_and_height_on_every_tab() {
    for width in WIDTHS {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        let a = import_square(&s);
        select_all(&s, &[a]);
        let mut h = rig(&dir, s, width);
        let mut rows = Vec::new();
        for tab in RibbonTab::ALL {
            show_tab(&mut h, tab);
            let l = layout(&h);
            assert_eq!(l.tools.height(), ROW_HEIGHT, "{width} {tab:?}");
            // The group buttons and tiles end at the bottom of the row.
            for g in tools::GROUPS
                .iter()
                .filter(|g| g.tab == tab && g.id != GroupId::Size)
            {
                let r = node_rect(&h, Role::Button, g.title);
                assert!(
                    (r.bottom() - l.tools.bottom()).abs() < 0.5,
                    "{width} {tab:?}: {} ends at {} not {}",
                    g.title,
                    r.bottom(),
                    l.tools.bottom()
                );
            }
            // The tools start right under the tab row: no extra gap.
            assert!(
                l.tools.top() - l.tab_row.bottom() <= 4.0,
                "{width} {tab:?}: {l:?}"
            );
            rows.push((l.tab_row, l.tools, l.panel));
        }
        assert!(
            rows.windows(2).all(|w| w[0] == w[1]),
            "{width}: the rows move between the tabs: {rows:?}"
        );
    }
}

#[test]
fn the_active_tab_touches_the_ribbon_panel() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = rig(&dir, s, 1400.0);
    for tab in RibbonTab::ALL {
        show_tab(&mut h, tab);
        let l = layout(&h);
        let r = node_rect(&h, Role::Tab, tab.name());
        // The tab's own rect ends where the panel starts, and the painted
        // folder tab reaches over the panel's top border.
        assert_eq!(r.bottom(), l.panel.top(), "{tab:?}");
        assert!(l.active_tab.bottom() > l.panel.top(), "{tab:?}: {l:?}");
        assert!(l.active_tab.left() >= l.panel.left() && l.active_tab.right() <= l.panel.right());
        // Inactive tabs sit on the same line.
        for other in RibbonTab::ALL.into_iter().filter(|o| *o != tab) {
            assert_eq!(
                node_rect(&h, Role::Tab, other.name()).bottom(),
                l.panel.top()
            );
        }
    }
}

#[test]
fn every_pinned_action_on_home_is_a_big_button() {
    for width in WIDTHS {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        let a = import_square(&s);
        select_all(&s, &[a]);
        let h = rig(&dir, s, width);
        let l = layout(&h);
        let mut seen = 0;
        for t in tools::TOOLS
            .iter()
            .filter(|t| t.group.def().tab == RibbonTab::Home)
        {
            let found: Vec<Rect> = h
                .query_all_by_role(Role::Button)
                .filter(|n| n.accesskit_node().label().as_deref() == Some(&t.button_name()))
                .map(|n| n.rect())
                .collect();
            for r in found {
                seen += 1;
                assert_eq!(r.height(), BIG_HEIGHT, "{width}: {}", t.id);
                assert_eq!(
                    r.top(),
                    l.tools.top(),
                    "{width}: {} is not at the top",
                    t.id
                );
            }
        }
        // At 900 pt groups may be tiles; at 1400 pt and wider every pin shows.
        if width >= 1400.0 {
            assert_eq!(seen, 7, "{width}");
        }
    }
}

#[test]
fn the_switches_are_compact_and_inside_their_group() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let a = import_square(&s);
    let b = import_square(&s);
    select_all(&s, &[a, b]);
    // Every pin: the Arrange group is at its widest.
    s.lock()
        .unwrap()
        .update_prefs(|p| p.ribbon_pins = RIBBON_PIN_IDS.iter().map(|s| s.to_string()).collect())
        .unwrap();
    for width in [1400.0, 1920.0] {
        let mut h = rig(&dir, s.clone(), width);
        show_tab(&mut h, RibbonTab::Arrange);
        let l = layout(&h);
        let group = node_rect(&h, Role::Button, "Arrange");
        let each = node_rect(&h, Role::CheckBox, "Each object");
        assert_eq!(each.height(), COMPACT_HEIGHT);
        assert!(
            each.left() >= group.left() && each.right() <= group.right(),
            "{width}: Each object {each:?} is outside its group {group:?}"
        );
        // Centred in the row of pins.
        let pins_mid = l.tools.top() + BIG_HEIGHT / 2.0;
        assert!(
            (each.center().y - pins_mid).abs() <= 0.5,
            "{width}: {each:?}"
        );
        let keep = node_rect(&h, Role::CheckBox, "Keep aspect");
        assert_eq!(keep.height(), COMPACT_HEIGHT);
        assert!(
            (keep.center().y - pins_mid).abs() <= 0.5,
            "{width}: {keep:?}"
        );
    }
}
