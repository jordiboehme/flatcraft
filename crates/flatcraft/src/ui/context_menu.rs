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
//! `ribbon` and `objects`), and an item that is a ribbon action takes its
//! title, description and reason from the table in [`tools`]. An item
//! that does not apply right now is shown dimmed, and its hover card says
//! why; an item that can never apply to what was clicked (Trace again on
//! vectors only, Reload without a file, Unlink on objects that are not
//! linked, Link copies on different designs) is left out. Only such properties of the objects hide an item. Key hints come from
//! [`help::SHORTCUTS`].

use egui::Ui;
use serde_json::json;

use super::groups;
use super::help::{self, PrepareKey};
use super::prepare::{PrepareState, RenameDialog, Snapshot};
use super::tools::{self, Act, AlignTo, Tool};
use super::{Action, arrange, objects, ribbon};

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
    /// Show the linked file in the file manager.
    RevealSource,
    /// Copy the linked file's path.
    CopySourcePath,
    /// Export the objects' cut shapes (the Export dialog on the selection).
    ExportSelected,
    Delete,
    // The empty mat.
    Import,
    SelectAll,
    AutoArrange,
    ZoomToFit,
}

impl Item {
    /// The ribbon action the item is, if it is one.
    pub fn tool(self) -> Option<&'static Tool> {
        let act = match self {
            Item::Duplicate => Act::Duplicate,
            Item::Copies => Act::Copies,
            Item::RotateLeft => Act::RotateLeft,
            Item::RotateRight => Act::RotateRight,
            Item::MirrorHorizontal => Act::MirrorHorizontal,
            Item::MirrorVertical => Act::MirrorVertical,
            Item::Delete => Act::Delete,
            Item::Import => Act::Import,
            Item::AutoArrange => Act::AutoArrange,
            Item::ZoomToFit => Act::ZoomFit,
            _ => return None,
        };
        Some(tools::by_act(act))
    }

    /// Its text: the table's title for a ribbon action.
    pub fn label(self) -> &'static str {
        if let Some(t) = self.tool() {
            return t.title;
        }
        match self {
            Item::LinkCopies => "Link copies",
            Item::UnlinkCopies => "Unlink",
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
            Item::RevealSource => super::files::reveal_source_label(),
            Item::CopySourcePath => "Copy source path",
            Item::SelectAll => "Select all",
            Item::ExportSelected => "Export selected...",
            _ => unreachable!("the other items are ribbon actions"),
        }
    }

    /// What it does, for its hover card: the table's description for a
    /// ribbon action.
    pub fn description(self) -> &'static str {
        if let Some(t) = self.tool() {
            return t.description;
        }
        match self {
            Item::LinkCopies => {
                "Make these copies of one design linked: they share the source, the cut options and the scale. Each keeps its own place."
            }
            Item::UnlinkCopies => {
                "Make these objects independent again, so you can change one without the others."
            }
            Item::Align => {
                "Line up the objects with each other or with the cutting area, the part of the mat the blade can reach."
            }
            Item::Distribute => {
                "Equal gaps between 3 or more objects. The two outer objects stay where they are."
            }
            Item::Hide => "Hide the objects on the canvas. Hidden objects are not cut.",
            Item::Show => "Show the hidden objects again, so they are cut.",
            Item::SkipCutting => {
                "Keep the objects on the mat but leave them out of the cut, for example a print you only trace around."
            }
            Item::CutAgain => "Cut these objects again with the rest.",
            Item::Rename => "Give the object a name you will recognise in the Objects panel.",
            Item::TraceAgain => "Trace the picture again with other settings.",
            Item::Reload => "Read the linked file again after you changed it in another app.",
            Item::Reconnect => "The linked file moved or is gone. Show FlatCraft where it is now.",
            Item::RevealSource => {
                "Opens the folder of the design file this object was imported from, with the file selected."
            }
            Item::CopySourcePath => {
                "Copies the full path of the design file this object was imported from."
            }
            Item::SelectAll => "Select every visible object on the mat.",
            Item::ExportSelected => super::export_dialog::SELECTED_HINT,
            _ => unreachable!("the other items are ribbon actions"),
        }
    }

    /// The key hint, from the shortcut table.
    pub fn hint(self) -> Option<String> {
        let key = match self {
            Item::SelectAll => PrepareKey::SelectAll,
            _ => self.tool()?.shortcut()?,
        };
        Some(help::hint(key))
    }

    /// Why the item cannot run on `ids` now, as a sentence that says what
    /// to do; `None` when it can. The ribbon actions give the table's
    /// reason, so the ribbon and the menu say the same.
    pub fn why(self, snap: &Snapshot, st: &PrepareState, ids: &[u64]) -> Option<&'static str> {
        let n = ids.len();
        match self {
            Item::Rename => (n > 1).then_some("Select only one object to rename it."),
            Item::TraceAgain => (n > 1).then_some("Select only one picture to trace it again."),
            Item::LinkCopies => (n < 2).then_some("Select 2 or more copies of one design first."),
            Item::Align => None,
            Item::Distribute => tools::by_act(Act::DistributeHorizontal).blocked_for(snap, n),
            Item::SelectAll => snap
                .objects
                .is_empty()
                .then_some("Import something first. There is nothing to select yet."),
            Item::RevealSource | Item::CopySourcePath => source_file(snap, st, ids)?.err(),
            Item::ExportSelected => super::export_dialog::selected_why(snap, ids),
            _ => self.tool()?.blocked_for(snap, n),
        }
    }
}

/// Why Reveal source and Copy source path are dimmed: the file is gone.
pub const SOURCE_MISSING: &str = "The file was moved or deleted. Use Reconnect source.";
/// Why they are dimmed: the objects come from several files.
pub const SOURCE_MIXED: &str =
    "The selected objects come from different files. Select objects from one file.";

/// The linked file of the objects `ids`, for Reveal source and Copy
/// source path: `None` when none of them is linked to a file (the items
/// are left out); the reason when the file is gone (the source check,
/// which runs off the UI thread, found it missing) or the linked objects
/// come from different files; else the id of a linked object, whose file
/// [`source_path`] finds on a click. Objects that are not linked do not
/// count. Touches no file, so the menu bar can ask every frame.
pub fn source_file(
    snap: &Snapshot,
    st: &PrepareState,
    ids: &[u64],
) -> Option<Result<u64, &'static str>> {
    let linked: Vec<(u64, &std::path::PathBuf)> = ids
        .iter()
        .filter_map(|id| Some((*id, &snap.object(*id)?.source_link.as_ref()?.path)))
        .collect();
    let (first, path) = *linked.first()?;
    if linked.iter().any(|(_, p)| *p != path) {
        return Some(Err(SOURCE_MIXED));
    }
    if linked.iter().any(|(id, _)| st.sources.broken(*id)) {
        return Some(Err(SOURCE_MISSING));
    }
    Some(Ok(first))
}

/// Where the linked file of object `id` is now, as Reload finds it (also
/// after the project folder moved as a whole); `None` when it is in
/// neither place. Looks at the disk: only for a click.
pub fn source_path(snap: &Snapshot, id: u64) -> Option<std::path::PathBuf> {
    let o = snap.object(id)?;
    let link = o.source_link.as_ref()?;
    flatcraft_engine::commands::sources::find_linked_file(link, o.raster, snap.project.as_deref())
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
/// is the empty mat. Objects that are gone are ignored. Items that do not
/// apply right now are listed too ([`Item::why`] dims them); items that
/// can never apply to these objects are not.
pub fn items(snap: &Snapshot, st: &PrepareState, ids: &[u64]) -> Vec<Vec<Item>> {
    let objs: Vec<_> = ids.iter().filter_map(|id| snap.object(*id)).collect();
    let n = objs.len();
    let mut groups: Vec<Vec<Item>> = Vec::new();
    if n == 0 {
        return vec![vec![
            Item::Import,
            Item::SelectAll,
            Item::AutoArrange,
            Item::ZoomToFit,
        ]];
    }

    let mut g = vec![Item::Duplicate, Item::Copies];
    let all_one_group =
        objs[0].link_group.is_some() && objs.iter().all(|o| o.link_group == objs[0].link_group);
    // Linked already: Unlink instead. Objects of different designs can
    // never be linked: left out. One object: listed, dimmed.
    if !all_one_group && objs.iter().all(|o| o.same_source(objs[0])) {
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

    groups.push(vec![Item::Align, Item::Distribute]);

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
    g.push(Item::Rename);
    // Only pictures can be traced; with several objects it is dimmed.
    if objs.iter().any(|o| o.raster) {
        g.push(Item::TraceAgain);
    }
    let src = &st.sources;
    if objs.iter().any(|o| o.link.is_some() && !src.broken(o.id)) {
        g.push(Item::Reload);
    }
    if objs.iter().any(|o| o.link.is_some() && src.broken(o.id)) {
        g.push(Item::Reconnect);
    }
    if objs.iter().any(|o| o.link.is_some()) {
        g.push(Item::RevealSource);
        g.push(Item::CopySourcePath);
    }
    groups.push(g);

    groups.push(vec![Item::ExportSelected]);
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

/// Draws one item; true when it was used (the menu closes). An item
/// that cannot run now is dimmed and its card gives the reason.
fn draw(
    ui: &mut Ui,
    item: Item,
    snap: &Snapshot,
    st: &mut PrepareState,
    ids: &[u64],
    actions: &mut Vec<Action>,
) -> bool {
    let doc_id = snap.doc_id;
    let why = item.why(snap, st, ids);
    match item {
        Item::Align | Item::Distribute if why.is_none() => {
            let r = ui.menu_button(item.label(), |ui| {
                if item == Item::Align {
                    let to_area = &mut st.fields.align_to_area;
                    ui.radio_value(to_area, false, AlignTo::Selection.label());
                    ui.radio_value(to_area, true, AlignTo::Area.label());
                    ui.separator();
                }
                let to = AlignTo::of(st.fields.align_to_area);
                let rows = tools::TOOLS.iter().filter(|t| match t.act {
                    Act::Align(_, o) => item == Item::Align && o == to,
                    Act::DistributeHorizontal | Act::DistributeVertical => item == Item::Distribute,
                    _ => false,
                });
                for t in rows {
                    let blocked = t.blocked_for(snap, ids.len());
                    let r = ui.add_enabled(blocked.is_none(), egui::Button::new(t.title));
                    groups::card(&r, &t.full_title(), t.description, blocked, true);
                    if r.clicked() {
                        actions.push(sub_action(t, doc_id, ids));
                        ui.close();
                    }
                }
            });
            groups::card(&r.response, item.label(), item.description(), None, true);
            return false;
        }
        _ => {}
    }
    let mut button = egui::Button::new(item.label());
    if let Some(hint) = item.hint() {
        button = button.shortcut_text(hint);
    }
    let r = ui.add_enabled(why.is_none(), button);
    let title = match item.tool() {
        Some(t) => t.card_title(snap),
        None => item.label().to_string(),
    };
    groups::card(&r, &title, item.description(), why, true);
    if !r.clicked() {
        return false;
    }
    run_item(item, snap, st, ids, actions)
}

/// Runs one item on `ids`: pushes its action or opens its panel or dialog.
/// The context menu and the macOS menu bar both use it. True when it ran
/// (a submenu never does).
pub fn run_item(
    item: Item,
    snap: &Snapshot,
    st: &mut PrepareState,
    ids: &[u64],
    actions: &mut Vec<Action>,
) -> bool {
    let doc_id = snap.doc_id;
    let each = st.fields.each;
    let first = ids.first().and_then(|id| snap.object(*id));
    match item {
        Item::Duplicate => actions.push(ribbon::duplicate_action(doc_id, ids)),
        Item::Copies => {
            // The panel works on the selection, which is `ids` by now
            // (see `target`).
            arrange::open(st, snap, arrange::PanelKind::RectangularPattern);
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
        Item::RevealSource | Item::CopySourcePath => {
            let Some(Ok(id)) = source_file(snap, st, ids) else {
                return false;
            };
            // The source check may not have seen the file go yet: never
            // reveal or copy a dead path.
            let Some(path) = source_path(snap, id) else {
                actions.push(Action::Error(SOURCE_MISSING.to_string()));
                return true;
            };
            actions.push(if item == Item::RevealSource {
                Action::Reveal(path)
            } else {
                Action::CopyPath(path)
            });
        }
        Item::ExportSelected => actions.push(Action::ExportDialog {
            doc_id,
            selected: true,
        }),
        Item::Delete => actions.push(ribbon::delete_action(doc_id, ids)),
        Item::Import => actions.push(Action::Import { doc_id }),
        Item::SelectAll => actions.push(select_all_action(snap)),
        Item::AutoArrange => actions.push(tools::auto_arrange_action(snap)),
        Item::ZoomToFit => st.fit_pending = true,
        // Dimmed submenus: a click does nothing.
        Item::Align | Item::Distribute => return false,
    }
    true
}

/// The command of an Align or Distribute row of a submenu, on `ids`.
fn sub_action(t: &Tool, doc_id: u64, ids: &[u64]) -> Action {
    match t.act {
        Act::Align(mode, to) => {
            ribbon::align_action(doc_id, ids, mode.param(), to == AlignTo::Area)
        }
        Act::DistributeVertical => ribbon::distribute_action(doc_id, ids, "vertical"),
        _ => ribbon::distribute_action(doc_id, ids, "horizontal"),
    }
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
