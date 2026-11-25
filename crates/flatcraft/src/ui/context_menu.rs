//! The context menu of the canvas and of the rows of the Objects panel.
//!
//! Right-clicking an object that is not selected selects it first (as in
//! Finder); right-clicking inside the selection keeps it. The menu then
//! works on the objects in `PrepareState::context`, which [`target`] sets
//! on the click, so its items are right in the frame of the click, before
//! the selection command has run.
//!
//! The menu holds no logic of its own: every item pushes the same
//! [`Action`] as the ribbon or the Objects panel (see the helpers in
//! `ribbon` and `objects`). Items that do not apply are left out. Key
//! hints come from [`help::SHORTCUTS`].

use egui::Ui;
use serde_json::json;

use super::help::{self, PrepareKey};
use super::prepare::{PrepareState, RenameDialog, Snapshot};
use super::{Action, objects, ribbon};

/// One item of the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Duplicate,
    Copies,
    LinkCopies,
    UnlinkCopies,
    RotateLeft,
    RotateRight,
    MirrorHorizontal,
    MirrorVertical,
    /// A submenu.
    Align,
    /// A submenu.
    Distribute,
    Hide,
    Show,
    SkipCutting,
    CutAgain,
    Rename,
    TraceAgain,
    Reload,
    Reconnect,
    Delete,
    // The empty mat.
    Import,
    SelectAll,
    AutoArrange,
    ZoomToFit,
}

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Item::Duplicate => "Duplicate",
            Item::Copies => "Copies...",
            Item::LinkCopies => "Link copies",
            Item::UnlinkCopies => "Unlink",
            Item::RotateLeft => "Rotate -90",
            Item::RotateRight => "Rotate +90",
            Item::MirrorHorizontal => "Mirror horizontally",
            Item::MirrorVertical => "Mirror vertically",
            Item::Align => "Align",
            Item::Distribute => "Distribute",
            Item::Hide => "Hide",
            Item::Show => "Show",
            Item::SkipCutting => "Skip cutting",
            Item::CutAgain => "Cut again",
            Item::Rename => "Rename",
            Item::TraceAgain => "Trace again...",
            Item::Reload => "Reload source",
            Item::Reconnect => "Reconnect source...",
            Item::Delete => "Delete",
            Item::Import => "Import...",
            Item::SelectAll => "Select all",
            Item::AutoArrange => "Auto arrange",
            Item::ZoomToFit => "Zoom to fit",
        }
    }

    /// The key hint, from the shortcut table.
    pub fn hint(self) -> Option<String> {
        let key = match self {
            Item::Duplicate => PrepareKey::Duplicate,
            Item::Delete => PrepareKey::Delete,
            Item::Import => PrepareKey::Import,
            Item::SelectAll => PrepareKey::SelectAll,
            _ => return None,
        };
        Some(help::hint(key))
    }
}

/// The objects a right-click works on. A click on an object outside the
/// selection selects it; a click inside the selection keeps it; a click on
/// the empty mat clears the selection. Pushes the select command when the
/// selection changes.
pub fn target(snap: &Snapshot, clicked: Option<u64>, actions: &mut Vec<Action>) -> Vec<u64> {
    let selection: Vec<u64> = snap.selection.iter().copied().collect();
    match clicked {
        Some(id) if snap.selection.contains(&id) => selection,
        Some(id) => {
            actions.push(Action::run(
                snap.doc_id,
                "object.select",
                json!({ "ids": [id] }),
            ));
            vec![id]
        }
        None => {
            if !selection.is_empty() {
                actions.push(Action::run(
                    snap.doc_id,
                    "object.select",
                    json!({ "ids": [] }),
                ));
            }
            Vec::new()
        }
    }
}

/// The items for `ids`, in groups (a separator goes between groups). No ids
/// is the empty mat. Objects that are gone are ignored.
pub fn items(snap: &Snapshot, st: &PrepareState, ids: &[u64]) -> Vec<Vec<Item>> {
    let objs: Vec<_> = ids.iter().filter_map(|id| snap.object(*id)).collect();
    let n = objs.len();
    let mut groups: Vec<Vec<Item>> = Vec::new();
    if n == 0 {
        let mut g = vec![Item::Import];
        if !snap.objects.is_empty() {
            g.push(Item::SelectAll);
            g.push(Item::AutoArrange);
        }
        g.push(Item::ZoomToFit);
        return vec![g];
    }

    let mut g = vec![Item::Duplicate];
    if n == 1 {
        g.push(Item::Copies);
    }
    let all_one_group =
        objs[0].link_group.is_some() && objs.iter().all(|o| o.link_group == objs[0].link_group);
    if n >= 2 && !all_one_group && objs.iter().all(|o| o.same_source(objs[0])) {
        g.push(Item::LinkCopies);
    }
    if objs.iter().any(|o| o.link_group.is_some()) {
        g.push(Item::UnlinkCopies);
    }
    groups.push(g);

    groups.push(vec![
        Item::RotateLeft,
        Item::RotateRight,
        Item::MirrorHorizontal,
        Item::MirrorVertical,
    ]);

    let mut g = Vec::new();
    if n >= 2 {
        g.push(Item::Align);
    }
    if n >= 3 {
        g.push(Item::Distribute);
    }
    groups.push(g);

    let mut g = vec![if objs.iter().any(|o| o.visible) {
        Item::Hide
    } else {
        Item::Show
    }];
    g.push(if objs[0].cut_enabled {
        Item::SkipCutting
    } else {
        Item::CutAgain
    });
    if n == 1 {
        g.push(Item::Rename);
        if objs[0].raster {
            g.push(Item::TraceAgain);
        }
    }
    let src = &st.sources;
    if objs.iter().any(|o| o.link.is_some() && !src.broken(o.id)) {
        g.push(Item::Reload);
    }
    if objs.iter().any(|o| o.link.is_some() && src.broken(o.id)) {
        g.push(Item::Reconnect);
    }
    groups.push(g);

    groups.push(vec![Item::Delete]);
    groups.retain(|g| !g.is_empty());
    groups
}

/// Draws the menu for the objects in `st.context` into the popup `ui`.
pub fn show(ui: &mut Ui, snap: &Snapshot, st: &mut PrepareState, actions: &mut Vec<Action>) {
    // Objects that are gone (an undo, an agent) drop out.
    let ids: Vec<u64> = st
        .context
        .iter()
        .flatten()
        .copied()
        .filter(|id| snap.object(*id).is_some())
        .collect();
    ui.set_min_width(190.0);
    let groups = items(snap, st, &ids);
    for (i, group) in groups.iter().enumerate() {
        if i > 0 {
            ui.separator();
        }
        for item in group {
            if draw(ui, *item, snap, st, &ids, actions) {
                ui.close();
            }
        }
    }
}

/// Draws one item; true when it was used (the menu closes).
fn draw(
    ui: &mut Ui,
    item: Item,
    snap: &Snapshot,
    st: &mut PrepareState,
    ids: &[u64],
    actions: &mut Vec<Action>,
) -> bool {
    let doc_id = snap.doc_id;
    let each = st.fields.each;
    match item {
        Item::Align => {
            ui.menu_button(item.label(), |ui| {
                ribbon::align_items(ui, &mut st.fields.align_to_area, doc_id, ids, actions);
            });
            return false;
        }
        Item::Distribute => {
            ui.menu_button(item.label(), |ui| {
                for (name, axis) in ribbon::DISTRIBUTE {
                    if ui.button(name.trim_start_matches("Distribute ")).clicked() {
                        actions.push(ribbon::distribute_action(doc_id, ids, axis));
                        ui.close();
                    }
                }
            });
            return false;
        }
        _ => {}
    }
    let mut button = egui::Button::new(item.label());
    if let Some(hint) = item.hint() {
        button = button.shortcut_text(hint);
    }
    if !ui.add(button).clicked() {
        return false;
    }
    let first = ids.first().and_then(|id| snap.object(*id));
    match item {
        Item::Duplicate => actions.push(ribbon::duplicate_action(doc_id, ids)),
        Item::Copies => {
            if let Some(d) = ribbon::copies_dialog_for(snap, ids, st.copies_last) {
                st.copies = Some(d);
            }
        }
        Item::LinkCopies => actions.push(Action::run(doc_id, "object.link", json!({ "ids": ids }))),
        Item::UnlinkCopies => actions.push(Action::run(
            doc_id,
            "object.unlink_copies",
            json!({ "ids": ids }),
        )),
        Item::RotateLeft => actions.push(ribbon::rotate_action(doc_id, ids, -90.0, each)),
        Item::RotateRight => actions.push(ribbon::rotate_action(doc_id, ids, 90.0, each)),
        Item::MirrorHorizontal => {
            actions.push(ribbon::mirror_action(doc_id, ids, "horizontal", each));
        }
        Item::MirrorVertical => actions.push(ribbon::mirror_action(doc_id, ids, "vertical", each)),
        Item::Hide => actions.push(objects::set_visible_action(doc_id, ids, false)),
        Item::Show => actions.push(objects::set_visible_action(doc_id, ids, true)),
        Item::SkipCutting | Item::CutAgain => {
            if let Some(o) = first {
                actions.push(objects::toggle_cut(snap, o));
            }
        }
        Item::Rename => {
            if let Some(o) = first {
                st.rename = Some(RenameDialog {
                    doc_id,
                    id: o.id,
                    name: o.name.clone(),
                });
            }
        }
        Item::TraceAgain => {
            if let Some(id) = ribbon::single_raster_of(snap, ids) {
                actions.push(Action::TraceDialog { doc_id, id });
            }
        }
        Item::Reload => {
            if let Some(ids) = ribbon::reload_ids_of(snap, &st.sources, ids) {
                actions.push(ribbon::reload_action(doc_id, ids));
            }
        }
        Item::Reconnect => {
            if let Some(o) = ids.iter().find(|id| st.sources.broken(**id)) {
                actions.push(Action::Reconnect { doc_id, id: *o });
            }
        }
        Item::Delete => actions.push(ribbon::delete_action(doc_id, ids)),
        Item::Import => actions.push(Action::Import { doc_id }),
        Item::SelectAll => actions.push(select_all_action(snap)),
        Item::AutoArrange => actions.push(Action::run(doc_id, "arrange.auto", json!({}))),
        Item::ZoomToFit => st.fit_pending = true,
        Item::Align | Item::Distribute => unreachable!("submenus return above"),
    }
    true
}

/// The `object.select` of Select all: every visible object. Cmd+A and the
/// menu both use it.
pub fn select_all_action(snap: &Snapshot) -> Action {
    let ids: Vec<u64> = snap
        .objects
        .iter()
        .filter(|o| o.visible)
        .map(|o| o.id)
        .collect();
    Action::run(snap.doc_id, "object.select", json!({ "ids": ids }))
}
