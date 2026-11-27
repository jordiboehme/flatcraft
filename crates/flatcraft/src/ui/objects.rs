//! The Objects panel: one row per object with its kind, name, a mark for
//! linked copies, file status, an eye to show or hide it and scissors to
//! skip its cut. The two toggles are quiet until the row is hovered, and
//! clear when the object is hidden or skipped. Below the list, the
//! project's notes. What a selected object is (its name, size, file and
//! linked copies) is in the inspector on the right.

use egui::{Rect, RichText, Ui};
use serde_json::json;

use super::context_menu;
use super::icons::{self, Icon};
use super::prepare::{ObjView, PrepareState, Snapshot};
use super::rails::{Side, SidePanel};
use super::theme::Tokens;
use super::{Action, ICON_SIZE, fonts, metrics};

/// A small icon with an accessible name.
fn badge(ui: &mut Ui, icon: Icon, name: &str, colour: egui::Color32) {
    badge_with(ui, icon, name, name, colour);
}

/// [`badge`] whose hover text is `card`.
fn badge_with(ui: &mut Ui, icon: Icon, name: &str, card: &str, colour: egui::Color32) {
    let tex = icons::texture_colored(ui.ctx(), icon, metrics::ICON_COMPACT, colour);
    ui.add(
        egui::Image::from_texture(egui::load::SizedTexture::new(
            tex.id(),
            egui::Vec2::splat(metrics::ICON_COMPACT),
        ))
        .alt_text(name),
    )
    .on_hover_text(card);
}

pub fn show(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    super::inspector::blocks::panel_spacing(ui);
    if super::rails::header(ui, SidePanel::Objects, Side::Left, "Objects") {
        st.panels.set_collapsed(SidePanel::Objects, true);
    }
    // One scroll area for the whole panel, so the Notes below the list stay
    // reachable in a short window. The list keeps its own, at most 60 % of
    // the panel's height.
    let list_h = ui.available_height() * 0.6;
    egui::ScrollArea::vertical()
        .id_salt("flatcraft-objects-panel")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            super::inspector::blocks::panel_spacing(ui);
            if snap.objects.is_empty() {
                ui.label(
                    RichText::new("Nothing here yet. Import a design to get started.")
                        .color(t.text_dim),
                );
            }
            egui::ScrollArea::vertical()
                .id_salt("flatcraft-objects-list")
                .auto_shrink([false, true])
                .max_height(list_h)
                .show(ui, |ui| {
                    tree_rows(ui, st, snap, t, actions, None, 0);
                    top_level_drop(ui, snap, t, actions);
                });
            notes(ui, st, snap, actions);
        });
}

/// The card of the Notes heading in the Objects panel.
pub const NOTES_HINT: &str = "Shows or hides the notes of this project: a one-line description and what to remember the next time you cut it.";
/// The card of the description field.
pub const DESCRIPTION_HINT: &str = "One line that says what this project is. The Start page shows it under the name, and so does the tab's card. It is saved in the project file.";
/// The card of the notes field.
pub const NOTES_FIELD_HINT: &str = "Things to remember the next time you cut this: the mat, which side goes up, a force that worked. They are saved in the project file and show again in the Cut workspace, right above the send button. Undo takes back what you typed.";

/// A heading that opens and closes a section: a chevron and the title. Its
/// accessible name is `title`. Returns true when clicked.
pub fn section_toggle(ui: &mut Ui, title: &str, open: bool, hint: &str) -> bool {
    let icon = if open {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    let tex = icons::texture(ui.ctx(), icon, ICON_SIZE * 0.8);
    let image = egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(ICON_SIZE * 0.8),
    ));
    let resp = ui.add(
        egui::Button::image_and_text(image, RichText::new(title).font(fonts::semibold(14.0)))
            .frame(false),
    );
    super::settings::set_name(ui, &resp, title);
    let state = if open {
        "Click to fold it away."
    } else {
        "Click to open it."
    };
    super::groups::card(&resp, title, &format!("{hint} {state}"), None, false);
    resp.clicked()
}

/// The Notes section: the project's one-line description and its notes.
/// Each visit to a field is one undo step: the typing goes in as it
/// happens (`document.set_notes` with a coalesce key per visit), so the
/// document is unsaved at once and Undo takes the whole visit back.
fn notes(ui: &mut Ui, st: &mut PrepareState, snap: &Snapshot, actions: &mut Vec<Action>) {
    st.notes_open = super::inspector::blocks::toggle_header(ui, "Notes", NOTES_HINT, st.notes_open);
    if !st.notes_open {
        return;
    }
    ui.spacing_mut().item_spacing.y = metrics::SPACE_4;
    let doc_id = snap.doc_id;
    super::settings::painted_label(ui, "Description");
    let mut description = snap.description.clone();
    let r = ui.add(
        egui::TextEdit::singleline(&mut description)
            .hint_text("What is this project?")
            .desired_width(f32::INFINITY),
    );
    super::settings::set_name(ui, &r, "Project description");
    super::groups::card(&r, "Description", DESCRIPTION_HINT, None, false);
    if r.gained_focus() {
        st.notes_burst += 1;
    }
    if r.changed() {
        actions.push(Action::run(
            doc_id,
            "document.set_notes",
            json!({
                "description": description,
                "coalesce_key": format!("description-{}", st.notes_burst),
            }),
        ));
    }
    ui.add_space(metrics::SPACE_4);
    super::settings::painted_label(ui, "Notes");
    let mut notes = snap.notes.clone();
    let r = ui.add(
        egui::TextEdit::multiline(&mut notes)
            .hint_text("Which mat, which side up, a force that worked...")
            .desired_rows(4)
            .desired_width(f32::INFINITY),
    );
    super::settings::set_name(ui, &r, "Project notes");
    super::groups::card(&r, "Notes", NOTES_FIELD_HINT, None, false);
    if r.gained_focus() {
        st.notes_burst += 1;
    }
    if r.changed() {
        actions.push(Action::run(
            doc_id,
            "document.set_notes",
            json!({
                "notes": notes,
                "coalesce_key": format!("notes-{}", st.notes_burst),
            }),
        ));
    }
}

/// How far a member's row is indented per group level.
const INDENT: f32 = metrics::INDENT;

/// A row dragged in the list: the objects or groups it moves into or out
/// of a group.
#[derive(Debug, Clone)]
struct RowDrag {
    ids: Vec<u64>,
}

/// The card of a group row.
pub const GROUP_ROW_HINT: &str = "A group: its objects act as one. Click to select the whole group, click the arrow to see its members. Drag a row onto a group to put it in, or below the list to take it out. Double-click to rename.";

/// How clicks pick rows, with the keys of `os` (see [`row_click`]).
pub fn list_keys_hint(os: super::help::Os) -> String {
    let cmd = super::help::modifier_names(egui::Modifiers::COMMAND, os);
    format!(
        "{cmd}+click adds a row to the selection or takes it out; Shift+click selects every row from the last one you clicked, and {cmd}+Shift+click adds those rows."
    )
}

static LIST_KEYS: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| list_keys_hint(super::help::Os::CURRENT));

/// The group row's sentence on changing one member.
pub const GROUP_MEMBER_HINT: &str = "Double-click a member on the mat to change it alone.";

/// The card of the mark on a row whose file changed.
pub const SOURCE_CHANGED_CARD: &str = "Source file changed: the file has changed since the import. Use Reload source in the context menu or in Source in the inspector to bring the change in.";

/// The card of an object row.
pub const OBJECT_ROW_HINT: &str = "An object on the mat. Click to select it. Double-click to rename. The eye hides it, the scissors skip it when cutting.";

/// The rows of the children of `parent` (the top level for `None`), in
/// order, with the members of open groups under their group.
fn tree_rows(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
    parent: Option<u64>,
    depth: usize,
) {
    let parent_id = parent.map(flatcraft_core::document::ObjectId);
    for node in snap.tree.children(parent_id) {
        let id = node.0;
        if snap.is_group(id) {
            group_row(ui, st, snap, id, depth, t, actions);
            if st.open_groups.contains(&(snap.doc_id, id)) {
                tree_rows(ui, st, snap, t, actions, Some(id), depth + 1);
            }
        } else if let Some(o) = snap.object(id) {
            object_row(ui, st, snap, o, depth, t, actions);
        }
    }
}

/// The rows of the list in order (objects and groups), the members of
/// open groups under their group, as [`tree_rows`] draws them.
pub fn list_rows(snap: &Snapshot, st: &PrepareState) -> Vec<u64> {
    fn walk(snap: &Snapshot, st: &PrepareState, parent: Option<u64>, out: &mut Vec<u64>) {
        for node in snap
            .tree
            .children(parent.map(flatcraft_core::document::ObjectId))
        {
            let id = node.0;
            if snap.is_group(id) {
                out.push(id);
                if st.open_groups.contains(&(snap.doc_id, id)) {
                    walk(snap, st, Some(id), out);
                }
            } else if snap.object(id).is_some() {
                out.push(id);
            }
        }
    }
    let mut out = Vec::new();
    walk(snap, st, None, &mut out);
    out
}

/// A click on the row of `node` (an object or a group), as lists do in
/// Finder, Explorer and Illustrator's layers: a plain click selects the
/// row, Cmd+click (Ctrl+click) flips it in or out of the selection, and
/// Shift+click selects every row from the last plain or Cmd click to this
/// one (Cmd+Shift adds the range). A group row stands for all its objects;
/// a row inside a group is picked inside it. Hidden objects in the range
/// are selected too: their rows are in the list.
fn row_click(
    st: &mut PrepareState,
    snap: &Snapshot,
    node: u64,
    mods: egui::Modifiers,
    actions: &mut Vec<Action>,
) {
    use flatcraft_core::document::ObjectId;
    let doc_id = snap.doc_id;
    let parent = |id: u64| snap.tree.parent(ObjectId(id)).map(|g| g.0);
    let rows = list_rows(snap, st);
    let anchor = st
        .list_anchor
        .filter(|(d, a)| *d == doc_id && rows.contains(a))
        .map(|(_, a)| a);
    let ids: Vec<u64> = if mods.shift
        && let Some(anchor) = anchor
    {
        let (i, j) = (
            rows.iter().position(|r| *r == anchor).unwrap_or(0),
            rows.iter().position(|r| *r == node).unwrap_or(0),
        );
        let range = &rows[i.min(j)..=i.max(j)];
        // Inside a group when every row of the range is in it.
        let inside = parent(node).filter(|g| range.iter().all(|r| parent(*r) == Some(*g)));
        st.entered = inside.map(|g| (doc_id, g));
        // Cmd+Shift (Ctrl+Shift) adds the range to the selection, as in
        // Finder; Shift alone replaces it.
        let mut ids: Vec<u64> = if mods.command {
            snap.selection.iter().copied().collect()
        } else {
            Vec::new()
        };
        for r in range {
            for l in snap.leaves_of(*r) {
                if !ids.contains(&l) {
                    ids.push(l);
                }
            }
        }
        ids
    } else {
        st.list_anchor = Some((doc_id, node));
        st.entered = parent(node).map(|g| (doc_id, g));
        let leaves = snap.leaves_of(node);
        if mods.command {
            let current: Vec<ObjectId> = snap.selection.iter().map(|i| ObjectId(*i)).collect();
            let unit: Vec<ObjectId> = leaves.into_iter().map(ObjectId).collect();
            flatcraft_engine::lasso::combine(
                &current,
                &[unit],
                flatcraft_engine::lasso::Mode::Toggle,
            )
            .into_iter()
            .map(|i| i.0)
            .collect()
        } else {
            leaves
        }
    };
    actions.push(Action::run(doc_id, "object.select", json!({ "ids": ids })));
}

/// What the drop zone under the list says.
const DROP_OUT: &str = "Drop here to take it out of its group";

/// The room under the list: a row dropped there leaves its groups for
/// the top level.
fn top_level_drop(ui: &mut Ui, snap: &Snapshot, t: &Tokens, actions: &mut Vec<Action>) {
    if !egui::DragAndDrop::has_payload_of_type::<RowDrag>(ui.ctx()) {
        return;
    }
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::hover(),
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Label, true, DROP_OUT));
    if resp.dnd_hover_payload::<RowDrag>().is_some() {
        ui.painter().rect_stroke(
            rect,
            t.radius,
            egui::Stroke::new(1.5, t.accent_prepare),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        DROP_OUT,
        egui::TextStyle::Small.resolve(ui.style()),
        t.text_dim,
    );
    super::groups::card(
        &resp,
        DROP_OUT,
        "Let go here to take the row out of its group, to the top of the list. It stays where it is on the mat and acts on its own again.",
        None,
        false,
    );
    if let Some(p) = resp.dnd_release_payload::<RowDrag>() {
        actions.push(Action::run(
            snap.doc_id,
            "object.move_to_group",
            json!({ "ids": p.ids }),
        ));
    }
}

/// Makes `row` a drag source for `ids` and a drop target: a row dropped
/// on a group goes into it, on an object into that object's group.
/// Returns true while something is held over it.
fn drag_and_drop(
    row: &egui::Response,
    snap: &Snapshot,
    ids: Vec<u64>,
    into: Option<u64>,
    actions: &mut Vec<Action>,
) -> bool {
    let me = ids.clone();
    row.dnd_set_drag_payload(RowDrag { ids });
    // A row never drops onto itself or onto a row inside it (a group onto
    // one of its own members): the panel does not offer what the engine
    // would refuse.
    let fits = |p: &RowDrag| {
        !p.ids.iter().any(|d| {
            me.iter().any(|m| {
                snap.tree.within(
                    flatcraft_core::document::ObjectId(*m),
                    flatcraft_core::document::ObjectId(*d),
                )
            })
        })
    };
    let hover = row.dnd_hover_payload::<RowDrag>().is_some_and(|p| fits(&p));
    if let Some(p) = row.dnd_release_payload::<RowDrag>()
        && fits(&p)
    {
        actions.push(Action::run(
            snap.doc_id,
            "object.move_to_group",
            json!({ "ids": p.ids, "group_id": into }),
        ));
    }
    hover
}

/// A group's row: the fold arrow, the group icon, its name and size, and
/// the eye and scissors for all its objects.
fn group_row(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    id: u64,
    depth: usize,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let doc_id = snap.doc_id;
    let leaves = snap.leaves_of(id);
    let name = snap.name_of(id).unwrap_or_default();
    let selected = !leaves.is_empty() && leaves.iter().all(|l| snap.selection.contains(l));
    let open = st.open_groups.contains(&(doc_id, id));
    let (rect, row) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::click_and_drag(),
    );
    let label = format!("{name}, group of {} objects", leaves.len());
    row.widget_info(|| {
        egui::WidgetInfo::selected(egui::accesskit::Role::Button, true, selected, label.clone())
    });
    let over = row.contains_pointer();
    let objs: Vec<&ObjView> = leaves.iter().filter_map(|l| snap.object(*l)).collect();
    let visible = objs.iter().any(|o| o.visible);
    let cut = objs.iter().any(|o| o.cut_enabled);
    let quiet = !(visible && cut);
    let hover = drag_and_drop(&row, snap, vec![id], Some(id), actions);
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            t.accent_prepare.gamma_multiply(0.22)
        } else if over {
            t.surface
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, t.radius, fill);
        if hover {
            ui.painter().rect_stroke(
                rect,
                t.radius,
                egui::Stroke::new(1.5, t.accent_prepare),
                egui::StrokeKind::Inside,
            );
        }
    }
    let mut left = rect;
    left.set_left(rect.left() + depth as f32 * INDENT);
    left.set_right(rect.right() - 2.0 * TOGGLE - metrics::SPACE_8);
    // The fold arrow, a button of its own.
    let arrow = egui::Rect::from_center_size(
        egui::pos2(
            left.left() + ROW_INSET + metrics::ICON_COMPACT / 2.0,
            rect.center().y,
        ),
        egui::Vec2::splat(metrics::ICON_BUTTON),
    );
    let fold = ui.interact(
        arrow,
        ui.make_persistent_id(("group-fold", id)),
        egui::Sense::click(),
    );
    let fold_name = if open {
        format!("Fold {name}")
    } else {
        format!("Open {name}")
    };
    fold.widget_info(|| {
        egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, fold_name.clone())
    });
    super::groups::card(
        &fold,
        &fold_name,
        if open {
            "Folds the group away, so the list shows it as one row. Its members stay as they are."
        } else {
            "Shows the members of this group under it, so you can pick, hide or rename one alone."
        },
        None,
        false,
    );
    if fold.clicked() {
        if open {
            st.open_groups.remove(&(doc_id, id));
        } else {
            st.open_groups.insert((doc_id, id));
        }
    }
    let chevron = if open {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    let tex = icons::texture_colored(ui.ctx(), chevron, metrics::ICON_COMPACT, t.text_dim);
    ui.painter().image(
        tex.id(),
        egui::Rect::from_center_size(arrow.center(), egui::Vec2::splat(metrics::ICON_COMPACT)),
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    let mut text = left;
    text.set_left(arrow.right() + metrics::SPACE_2);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(text.shrink2(egui::vec2(0.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            badge(ui, Icon::Group, "Group", t.text_dim);
            let colour = if quiet { t.text_dim } else { t.text };
            ui.add(
                egui::Label::new(RichText::new(&name).color(colour))
                    .truncate()
                    .selectable(false),
            );
            ui.add(
                egui::Label::new(RichText::new(format!("{}", leaves.len())).color(t.text_dim))
                    .selectable(false),
            );
        },
    );
    let toggle_rect = |i: f32| {
        egui::Rect::from_center_size(
            egui::pos2(
                rect.right() - metrics::SPACE_2 - TOGGLE * (1.5 - i),
                rect.center().y,
            ),
            egui::Vec2::splat(TOGGLE),
        )
    };
    let (hint, toggle_name) = if cut {
        ("Skip cutting", format!("Skip cutting {name}"))
    } else {
        ("Cut again", format!("Cut again {name}"))
    };
    if toggle(
        ui,
        toggle_rect(0.0),
        id,
        Icon::Cut,
        &toggle_name,
        hint,
        !cut,
        over,
        selected,
        t,
    )
    .clicked()
    {
        actions.push(Action::run(
            doc_id,
            "object.set_cut",
            json!({ "ids": [id], "cut": !cut }),
        ));
    }
    let (icon, hint, toggle_name) = if visible {
        (Icon::Eye, "Hide", format!("Hide {name}"))
    } else {
        (Icon::EyeOff, "Show", format!("Show {name}"))
    };
    if toggle(
        ui,
        toggle_rect(1.0),
        id,
        icon,
        &toggle_name,
        hint,
        !visible,
        over,
        selected,
        t,
    )
    .clicked()
    {
        actions.push(set_visible_action(doc_id, &[id], !visible));
    }
    super::groups::card(
        &row,
        &name,
        &format!(
            "{GROUP_ROW_HINT} {GROUP_MEMBER_HINT} {}",
            LIST_KEYS.as_str()
        ),
        None,
        false,
    );
    if row.clicked() && !fold.clicked() {
        let mods = ui.input(|i| i.modifiers);
        row_click(st, snap, id, mods, actions);
    }
    if row.double_clicked() {
        st.rename = Some(super::prepare::RenameDialog { doc_id, id, name });
    }
    if row.secondary_clicked() {
        st.context = Some(context_menu::target(snap, Some(leaves.clone()), actions));
    }
    row.context_menu(|ui| context_menu::show(ui, snap, st, actions));
}

/// The height of a row: a panel row, as in the inspector.
const ROW_HEIGHT: f32 = metrics::ROW;
/// The side of a toggle button: a compact icon button.
const TOGGLE: f32 = metrics::ICON_BUTTON;
/// Room between a row's edge (its highlight) and its content.
const ROW_INSET: f32 = metrics::SPACE_4;

/// One row: a highlight when selected, the kind icon and the name, marks,
/// and the two toggles on the right.
fn object_row(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    o: &ObjView,
    depth: usize,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let doc_id = snap.doc_id;
    let selected = snap.selection.contains(&o.id);
    // Drag sense only where a row can go somewhere: with groups.
    let sense = if snap.tree.is_flat() {
        egui::Sense::click()
    } else {
        egui::Sense::click_and_drag()
    };
    let (rect, row) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), sense);
    let name_text = o.name.clone();
    row.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::accesskit::Role::Button,
            true,
            selected,
            name_text.clone(),
        )
    });
    let over = row.contains_pointer();
    let quiet = !(o.visible && o.cut_enabled);
    let hover = !snap.tree.is_flat() && drag_and_drop(&row, snap, vec![o.id], o.parent, actions);
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            t.accent_prepare.gamma_multiply(0.22)
        } else if over {
            t.surface
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, t.radius, fill);
        if hover {
            ui.painter().rect_stroke(
                rect,
                t.radius,
                egui::Stroke::new(1.5, t.accent_prepare),
                egui::StrokeKind::Inside,
            );
        }
    }
    // The kind, the name and the marks; members sit under their group,
    // past its fold arrow.
    let mut left = rect;
    if depth > 0 {
        left.set_left(rect.left() + depth as f32 * INDENT + metrics::ICON_COMPACT);
    }
    left.set_right(rect.right() - 2.0 * TOGGLE - metrics::SPACE_8);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(left.shrink2(egui::vec2(ROW_INSET, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            let (icon, kind) = if o.raster {
                (Icon::ImageFile, "Traced image")
            } else {
                (Icon::SvgFile, "SVG")
            };
            badge(ui, icon, kind, t.text_dim);
            let colour = if quiet { t.text_dim } else { t.text };
            ui.add(
                egui::Label::new(RichText::new(&o.name).color(colour))
                    .truncate()
                    .selectable(false),
            );
            if o.link_group.is_some() {
                badge(ui, Icon::Linked, "Linked copy", t.text_dim);
            }
            let src = &st.sources;
            if src.changed.contains(&o.id) {
                badge_with(
                    ui,
                    Icon::Reload,
                    "Source file changed",
                    SOURCE_CHANGED_CARD,
                    t.accent_prepare,
                );
            } else if src.missing.contains(&o.id) {
                badge(ui, Icon::BrokenLink, "Source file not found", t.danger);
            } else if src.unreadable.contains(&o.id) {
                badge(ui, Icon::BrokenLink, "Source file can't be read", t.danger);
            }
        },
    );
    // The toggles come after the row, so they win their clicks.
    let toggle_rect = |i: f32| {
        egui::Rect::from_center_size(
            egui::pos2(
                rect.right() - metrics::SPACE_2 - TOGGLE * (1.5 - i),
                rect.center().y,
            ),
            egui::Vec2::splat(TOGGLE),
        )
    };
    let (hint, name) = if o.cut_enabled {
        ("Skip cutting", format!("Skip cutting {}", o.name))
    } else {
        ("Cut again", format!("Cut again {}", o.name))
    };
    if toggle(
        ui,
        toggle_rect(0.0),
        o.id,
        Icon::Cut,
        &name,
        hint,
        !o.cut_enabled,
        over,
        selected,
        t,
    )
    .clicked()
    {
        actions.push(toggle_cut(snap, o));
    }
    let (icon, hint, name) = if o.visible {
        (Icon::Eye, "Hide", format!("Hide {}", o.name))
    } else {
        (Icon::EyeOff, "Show", format!("Show {}", o.name))
    };
    if toggle(
        ui,
        toggle_rect(1.0),
        o.id,
        icon,
        &name,
        hint,
        !o.visible,
        over,
        selected,
        t,
    )
    .clicked()
    {
        actions.push(set_visible_action(doc_id, &[o.id], !o.visible));
    }

    super::groups::card(
        &row,
        &o.name,
        &format!("{OBJECT_ROW_HINT} {}", LIST_KEYS.as_str()),
        None,
        false,
    );
    if row.clicked() {
        let mods = ui.input(|i| i.modifiers);
        row_click(st, snap, o.id, mods, actions);
    }
    if row.double_clicked() {
        st.rename = Some(super::prepare::RenameDialog {
            doc_id,
            id: o.id,
            name: o.name.clone(),
        });
    }
    if row.secondary_clicked() {
        st.context = Some(context_menu::target(snap, Some(vec![o.id]), actions));
    }
    row.context_menu(|ui| context_menu::show(ui, snap, st, actions));
}

/// A small icon toggle. Quiet (faint) until the row is hovered, clear when
/// `on` (the object is skipped or hidden), with a hover fill of its own.
#[allow(clippy::too_many_arguments)]
fn toggle(
    ui: &mut Ui,
    rect: egui::Rect,
    object: u64,
    icon: Icon,
    name: &str,
    hint: &str,
    on: bool,
    row_hovered: bool,
    row_selected: bool,
    t: &Tokens,
) -> egui::Response {
    let id = ui.make_persistent_id(("object-toggle", object, name));
    let resp = ui.interact(rect, id, egui::Sense::click());
    let name = name.to_owned();
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, name.clone())
    });
    let resp = resp.on_hover_text(hint);
    if ui.is_rect_visible(rect) {
        let loud = on || resp.hovered();
        let alpha = if loud {
            1.0
        } else if row_hovered || row_selected || resp.has_focus() {
            0.55
        } else {
            0.0
        };
        let painter = ui.painter();
        if resp.hovered() {
            painter.rect_filled(rect, t.radius, t.border);
        } else if on {
            painter.rect_filled(rect, t.radius, t.warn.gamma_multiply(0.18));
        }
        if resp.has_focus() {
            ui.painter().rect_stroke(
                rect,
                t.radius,
                egui::Stroke::new(1.5, t.accent_prepare),
                egui::StrokeKind::Inside,
            );
        }
        if alpha > 0.0 {
            let colour = if on { t.warn } else { t.text };
            let tex = icons::texture_colored(ui.ctx(), icon, metrics::ICON_COMPACT, colour);
            let tint = egui::Color32::WHITE.gamma_multiply(alpha);
            let img =
                Rect::from_center_size(rect.center(), egui::Vec2::splat(metrics::ICON_COMPACT));
            ui.painter().image(
                tex.id(),
                img,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                tint,
            );
            if icon == Icon::Cut && on {
                // Struck through: skipped, at a glance.
                let r = img.shrink(1.0);
                ui.painter().line_segment(
                    [r.left_bottom(), r.right_top()],
                    egui::Stroke::new(1.5, t.warn),
                );
            }
        }
    }
    resp
}

/// The `object.set_visible` of the eye button and the Hide and Show items.
pub fn set_visible_action(doc_id: u64, ids: &[u64], visible: bool) -> Action {
    Action::run(
        doc_id,
        "object.set_visible",
        json!({ "ids": ids, "visible": visible }),
    )
}

/// The action that flips whether `o` is cut: skip it if it is cut, cut it
/// again if it is skipped. When `o` is part of the selection the whole
/// selection gets the new state, else only `o`. The Objects panel and the
/// context menu both use it.
pub fn toggle_cut(snap: &Snapshot, o: &ObjView) -> Action {
    let ids: Vec<u64> = if snap.selection.contains(&o.id) {
        snap.selection.iter().copied().collect()
    } else {
        vec![o.id]
    };
    Action::run(
        snap.doc_id,
        "object.set_cut",
        json!({ "ids": ids, "cut": !o.cut_enabled }),
    )
}
