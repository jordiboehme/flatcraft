//! One Layout tab for what used to be Home and Arrange (Task 44): the
//! tabs, the group order, search targets, saved names and the folding.

mod common;
use common::RunPlan;

use common::*;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::collapse::{self, Level, Mode};
use flatcraft::ui::prepare::RibbonTab;
use flatcraft::ui::ribbon::{self, SearchTarget};
use flatcraft::ui::tools::{self, GroupId};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::json;

fn select(s: &SharedSession, ids: &[u64]) {
    s.lock()
        .unwrap()
        .execute("object.select", json!({ "ids": ids }), Origin::User)
        .unwrap();
}

#[test]
fn the_ribbon_has_a_layout_tab_and_a_view_tab() {
    assert_eq!(RibbonTab::ALL, [RibbonTab::Layout, RibbonTab::View]);
    assert_eq!(RibbonTab::Layout.name(), "Layout");
    assert_eq!(RibbonTab::default(), RibbonTab::Layout);
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness_sized(&dir, s, [1400.0, 900.0], 8, |app| app);
    to_prepare(&mut h);
    assert!(h.query_by_role_and_label(Role::Tab, "Layout").is_some());
    assert!(h.query_by_role_and_label(Role::Tab, "View").is_some());
    assert!(h.query_by_role_and_label(Role::Tab, "Home").is_none());
    assert!(h.query_by_role_and_label(Role::Tab, "Arrange").is_none());
}

#[test]
fn the_groups_of_layout_run_add_edit_size_transform_arrange_linked_files() {
    let layout: Vec<GroupId> = tools::groups_of(RibbonTab::Layout).map(|g| g.id).collect();
    assert_eq!(
        layout,
        [
            GroupId::Add,
            GroupId::Edit,
            GroupId::Size,
            GroupId::Transform,
            GroupId::Arrange,
            GroupId::Files
        ]
    );
    assert_eq!(GroupId::Transform.def().title, "Transform");
    let view: Vec<GroupId> = tools::groups_of(RibbonTab::View).map(|g| g.id).collect();
    assert_eq!(view, [GroupId::Zoom, GroupId::Show, GroupId::Panels]);
    assert_eq!(GroupId::Files.def().title, "Linked files");
}

#[test]
fn search_hits_for_add_edit_size_and_arrange_land_on_layout() {
    for id in [
        "object.retrace",
        "object.copies",
        "object.scale",
        "object.rotate",
        "object.mirror",
        "arrange.align",
        "arrange.distribute",
        "object.circular_copies",
    ] {
        assert_eq!(
            ribbon::search_target(id, 1),
            Some(SearchTarget::Tab(RibbonTab::Layout)),
            "{id}"
        );
    }
}

#[test]
fn pins_stay_with_their_groups() {
    let move_copy = tools::TOOLS
        .iter()
        .find(|t| t.id == "move_copy")
        .expect("Move / Copy");
    assert_eq!(move_copy.group, GroupId::Transform);
}

#[test]
fn each_action_is_in_its_group_in_order() {
    let ids = |g: GroupId| -> Vec<&str> { g.tools().map(|t| t.id).collect() };
    assert_eq!(ids(GroupId::Add), ["import", "trace", "test_patterns"]);
    assert_eq!(
        ids(GroupId::Edit),
        [
            "duplicate",
            "copies",
            "pattern.rectangular",
            "pattern.circular",
            "delete"
        ]
    );
    assert_eq!(
        ids(GroupId::Transform),
        [
            "move_copy",
            "rotate",
            "rotate.left",
            "rotate.right",
            "mirror.horizontal",
            "mirror.vertical"
        ]
    );
    let mut arrange: Vec<String> = Vec::new();
    for to in ["selection", "area"] {
        for m in ["left", "centre", "right", "top", "middle", "bottom"] {
            arrange.push(format!("align.{m}.{to}"));
        }
    }
    for id in [
        "distribute.horizontal",
        "distribute.vertical",
        "auto_arrange",
        "auto_arrange.panel",
        "group",
        "ungroup",
        "order.front",
        "order.forward",
        "order.backward",
        "order.back",
    ] {
        arrange.push(id.to_string());
    }
    assert_eq!(ids(GroupId::Arrange), arrange);
    assert_eq!(ids(GroupId::Files), ["reload"]);
    assert!(GroupId::Size.tools().next().is_none());
    // Every action of the table is in exactly one of the groups above or
    // on the View tab.
    let layout: usize = tools::groups_of(RibbonTab::Layout)
        .map(|g| g.id.tools().count())
        .sum();
    let view: usize = tools::groups_of(RibbonTab::View)
        .map(|g| g.id.tools().count())
        .sum();
    assert_eq!(layout + view, tools::TOOLS.len());
}

#[test]
fn the_layout_fold_order_is_the_table() {
    // Linked files goes first, then Arrange (one-click helpers, also on
    // the context menu and the keys), so Transform, the fields and the
    // labels of Edit and Add stay longest. Edit (Duplicate and Delete
    // have keys) becomes a tile before Transform.
    let names: Vec<&str> = tools::groups_of(RibbonTab::Layout)
        .map(|g| g.title)
        .collect();
    let order: Vec<String> = tools::fold_table(RibbonTab::Layout)
        .into_iter()
        .map(|(i, m)| format!("{} {m:?}", names[i]))
        .collect();
    assert_eq!(
        order,
        [
            "Linked files Icons",
            "Linked files Collapsed",
            "Arrange Icons",
            "Arrange Collapsed",
            "Transform Icons",
            "Position and size Icons",
            "Edit Icons",
            "Add Icons",
            "Edit Collapsed",
            "Transform Collapsed",
            "Add Collapsed",
        ]
    );
    // Every group goes icon-only before it becomes a tile, and the steps
    // are 1, 2, 3 and so on, each used once: the ladder never skips or
    // repeats a state.
    for tab in RibbonTab::ALL {
        let mut ranks: Vec<u8> = Vec::new();
        for g in tools::groups_of(tab) {
            ranks.push(g.fold.icons);
            if let Some(tile) = g.fold.tile {
                assert!(g.fold.icons < tile, "{}", g.title);
                ranks.push(tile);
            }
        }
        ranks.sort();
        let want: Vec<u8> = (1..=ranks.len() as u8).collect();
        assert_eq!(ranks, want, "{tab:?}");
    }
    // The View tab folds from the right, as every bar did before.
    let keep = vec![false; 3];
    assert_eq!(tools::fold_table(RibbonTab::View), collapse::steps(&keep));
}

#[test]
fn at_1400_pt_no_group_of_layout_is_folded() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness_sized(&dir, s, [1400.0, 900.0], 8, |app| app);
    to_prepare(&mut h);
    h.run_plan();
    assert_eq!(collapse::level(&h.ctx, ribbon::bar_id()), Some(Level::Full));
}

#[test]
fn linked_files_folds_first() {
    for (width, height) in [(900.0, 600.0), (1100.0, 900.0)] {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        let a = import_square(&s);
        select(&s, &[a]);
        let mut h = harness_sized(&dir, s, [width, height], 8, |app| app);
        to_prepare(&mut h);
        h.run_plan();
        let level = collapse::level(&h.ctx, ribbon::bar_id());
        // The modes of Add, Edit, Size (keep), Transform, Arrange and
        // Linked files.
        let table = tools::fold_table(RibbonTab::Layout);
        let level = level.expect("the ribbon was drawn");
        let modes = collapse::modes_in(&table, 6, level);
        let mode = |g: GroupId| {
            let i = tools::groups_of(RibbonTab::Layout)
                .position(|d| d.id == g)
                .unwrap();
            modes[i]
        };
        assert_eq!(
            mode(GroupId::Files),
            Mode::Collapsed,
            "{width}: Linked files is a tile"
        );
        assert_ne!(
            mode(GroupId::Size),
            Mode::Collapsed,
            "{width}: Size is never a tile"
        );
        if width == 1100.0 {
            // Add keeps its labels; Edit keeps its buttons.
            assert_eq!(mode(GroupId::Add), Mode::Full, "{width}: Add");
            assert_ne!(mode(GroupId::Edit), Mode::Collapsed, "{width}: Edit");
            // Transform is one of the most used groups: it stays on the
            // bar. Arrange folds to its tile before it.
            assert_ne!(mode(GroupId::Transform), Mode::Collapsed, "{width}");
            assert_eq!(mode(GroupId::Arrange), Mode::Collapsed, "{width}");
            assert!(matches!(level, Level::Folded(_)), "{level:?}");
        }
        // Linked files is a tile, so its Reload button is not on the ribbon.
        assert!(
            h.query_by_role_and_label(Role::Button, "Reload").is_none(),
            "{width}: Reload is on the ribbon"
        );
    }
}
