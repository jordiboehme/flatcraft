//! One Layout tab for what used to be Home and Arrange (Task 44): the
//! tabs, the group order, search targets, saved names and the folding.

mod common;

use common::*;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::collapse::{self, Level};
use flatcraft::ui::prepare::RibbonTab;
use flatcraft::ui::ribbon::{self, SearchTarget};
use flatcraft::ui::tools::{self, GroupId};

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
fn a_saved_home_or_arrange_tab_loads_as_layout() {
    assert_eq!(RibbonTab::from_saved("Home"), Some(RibbonTab::Layout));
    assert_eq!(RibbonTab::from_saved("Arrange"), Some(RibbonTab::Layout));
    assert_eq!(RibbonTab::from_saved("Layout"), Some(RibbonTab::Layout));
    assert_eq!(RibbonTab::from_saved("View"), Some(RibbonTab::View));
    assert_eq!(RibbonTab::from_saved("Nope"), None);
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
fn linked_files_folds_first_at_900_pt() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness_sized(&dir, s, [900.0, 600.0], 8, |app| app);
    to_prepare(&mut h);
    h.run();
    assert_ne!(collapse::level(&h.ctx, ribbon::bar_id()), Some(Level::Full));
    // Linked files is a tile; it is the last group, so the first to fold.
    assert!(
        h.query_by_role_and_label(Role::Button, "Linked files")
            .is_some()
    );
}
