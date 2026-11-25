//! One Layout tab for what used to be Home and Arrange (Task 44): the
//! tabs, the group order, search targets, saved names and the folding.

mod common;

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
fn the_groups_of_layout_run_add_edit_size_arrange_linked_files() {
    let layout: Vec<GroupId> = tools::groups_of(RibbonTab::Layout).map(|g| g.id).collect();
    assert_eq!(
        layout,
        [
            GroupId::Add,
            GroupId::Edit,
            GroupId::Size,
            GroupId::Arrange,
            GroupId::Files
        ]
    );
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
    assert_eq!(move_copy.group, GroupId::Arrange);
}

#[test]
fn at_1400_pt_no_group_of_layout_is_folded() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness_sized(&dir, s, [1400.0, 900.0], 8, |app| app);
    to_prepare(&mut h);
    h.run();
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
        h.run();
        let level = collapse::level(&h.ctx, ribbon::bar_id());
        // The modes of Add, Edit, Size (keep), Arrange and Linked files.
        let keep = [false, false, true, false, false];
        let level = level.expect("the ribbon was drawn");
        let modes = collapse::modes(&keep, level);
        assert_eq!(modes[4], Mode::Collapsed, "{width}: Linked files is a tile");
        assert_ne!(modes[2], Mode::Collapsed, "{width}: Size is never a tile");
        if width == 1100.0 {
            // Arrange is one of the most used groups: it stays on the bar.
            assert_ne!(modes[3], Mode::Collapsed, "{width}: {level:?}");
            assert!(matches!(level, Level::Folded(_)), "{level:?}");
        }
        // Linked files is a tile, so its Reload button is not on the ribbon.
        assert!(
            h.query_by_role_and_label(Role::Button, "Reload").is_none(),
            "{width}: Reload is on the ribbon"
        );
    }
}
