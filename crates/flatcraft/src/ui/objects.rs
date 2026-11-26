//! The Objects panel: one row per object with its kind, name, a mark for
//! linked copies, file status, an eye to show or hide it and scissors to
//! skip its cut. The two toggles are quiet until the row is hovered, and
//! clear when the object is hidden or skipped. Below the list, the details of a single selected object with its linked copies
//! and its linked file, then the project's notes.

use egui::{Rect, RichText, Ui};
use serde_json::json;

use super::context_menu;
use super::icons::{self, Icon};
use super::prepare::{ObjView, PrepareState, Snapshot};
use super::rails::{Side, SidePanel};
use super::theme::Tokens;
use super::{Action, ICON_SIZE, fonts};

/// A small icon with an accessible name.
fn badge(ui: &mut Ui, icon: Icon, name: &str, colour: egui::Color32) {
    let tex = icons::texture_colored(ui.ctx(), icon, ICON_SIZE * 0.8, colour);
    ui.add(
        egui::Image::from_texture(egui::load::SizedTexture::new(
            tex.id(),
            egui::Vec2::splat(ICON_SIZE * 0.8),
        ))
        .alt_text(name),
    )
    .on_hover_text(name);
}

pub fn show(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    if super::rails::header(ui, SidePanel::Objects, Side::Left, "Objects") {
        st.panels.set_collapsed(SidePanel::Objects, true);
    }
    ui.add_space(t.spacing / 2.0);
    // One scroll area for the whole panel, so the Notes below the list stay
    // reachable in a short window. The list keeps its own, at most 60 % of
    // the panel's height.
    let list_h = ui.available_height() * 0.6;
    egui::ScrollArea::vertical()
        .id_salt("flatcraft-objects-panel")
        .auto_shrink([false, false])
        .show(ui, |ui| {
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

            let mut selected = snap.selected();
            if let (Some(o), None) = (selected.next(), selected.next()) {
                ui.separator();
                details(ui, o, st, snap, t, actions);
            } else if let [unit] = snap.units().as_slice()
                && snap.is_group(*unit)
            {
                ui.separator();
                group_details(ui, snap, *unit, t);
            }
            ui.separator();
            notes(ui, st, snap, t, actions);
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
fn notes(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    if section_toggle(ui, "Notes", st.notes_open, NOTES_HINT) {
        st.notes_open = !st.notes_open;
    }
    if !st.notes_open {
        return;
    }
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
    ui.add_space(t.spacing / 2.0);
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
const INDENT: f32 = 16.0;

/// A row dragged in the list: the objects or groups it moves into or out
/// of a group.
#[derive(Debug, Clone)]
struct RowDrag {
    ids: Vec<u64>,
}

/// The card of a group row.
pub const GROUP_ROW_HINT: &str = "A group: its objects act as one. Click to select the whole group, click the arrow to see its members. Drag a row onto a group to put it in, or below the list to take it out. Double-click to rename.";

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
    let hover = row
        .dnd_hover_payload::<RowDrag>()
        .is_some_and(|p| p.ids != me);
    if let Some(p) = row.dnd_release_payload::<RowDrag>()
        && p.ids != me
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
    left.set_right(rect.right() - 2.0 * TOGGLE - 6.0);
    // The fold arrow, a button of its own.
    let arrow = egui::Rect::from_center_size(
        egui::pos2(left.left() + 6.0 + ICON_SIZE * 0.4, rect.center().y),
        egui::Vec2::splat(ICON_SIZE),
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
    let fold = fold.on_hover_text(if open {
        "Hide the members of this group."
    } else {
        "Show the members of this group."
    });
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
    let tex = icons::texture_colored(ui.ctx(), chevron, ICON_SIZE * 0.8, t.text_dim);
    ui.painter().image(
        tex.id(),
        egui::Rect::from_center_size(arrow.center(), egui::Vec2::splat(ICON_SIZE * 0.8)),
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    let mut text = left;
    text.set_left(arrow.right() + 2.0);
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
            egui::pos2(rect.right() - 2.0 - TOGGLE * (1.5 - i), rect.center().y),
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
    super::groups::card(&row, &name, GROUP_ROW_HINT, None, false);
    if row.clicked() && !fold.clicked() {
        let shift = ui.input(|i| i.modifiers.shift);
        let mut ids: Vec<u64> = if shift {
            snap.selection.iter().copied().collect()
        } else {
            Vec::new()
        };
        for l in &leaves {
            if !ids.contains(l) {
                ids.push(*l);
            }
        }
        // A nested group is picked inside its parent group.
        st.entered = snap
            .tree
            .parent(flatcraft_core::document::ObjectId(id))
            .map(|g| (doc_id, g.0));
        actions.push(Action::run(doc_id, "object.select", json!({ "ids": ids })));
    }
    if row.double_clicked() {
        st.rename = Some(super::prepare::RenameDialog { doc_id, id, name });
    }
    if row.secondary_clicked() {
        st.context = Some(context_menu::target(snap, Some(leaves.clone()), actions));
    }
    row.context_menu(|ui| context_menu::show(ui, snap, st, actions));
}

/// What the panel says about one selected group.
fn group_details(ui: &mut Ui, snap: &Snapshot, id: u64, t: &Tokens) {
    let name = snap.name_of(id).unwrap_or_default();
    ui.label(RichText::new(&name).font(fonts::medium(14.0)));
    let n = snap.leaves_of(id).len();
    if let Some(b) = snap.box_of(&snap.leaves_of(id)) {
        ui.label(
            RichText::new(format!(
                "Group of {n} objects, {:.1} x {:.1} mm",
                b.width(),
                b.height()
            ))
            .color(t.text_dim),
        );
    }
    ui.label(
        RichText::new(
            "It moves, turns and gets packed as one. Double-click a member on the mat to change it alone.",
        )
        .color(t.text_dim),
    );
}

/// The height of a row: comfortable to click.
const ROW_HEIGHT: f32 = 28.0;
/// The side of a toggle button.
const TOGGLE: f32 = 22.0;

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
        left.set_left(rect.left() + depth as f32 * INDENT + ICON_SIZE);
    }
    left.set_right(rect.right() - 2.0 * TOGGLE - 6.0);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(left.shrink2(egui::vec2(6.0, 0.0)))
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
                badge(ui, Icon::Reload, "Source file changed", t.accent_prepare);
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
            egui::pos2(rect.right() - 2.0 - TOGGLE * (1.5 - i), rect.center().y),
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

    if row.clicked() {
        let shift = ui.input(|i| i.modifiers.shift);
        let ids: Vec<u64> = if shift {
            let mut ids: Vec<u64> = snap.selection.iter().copied().collect();
            if !ids.contains(&o.id) {
                ids.push(o.id);
            }
            ids
        } else {
            vec![o.id]
        };
        // A member picked in the list is picked inside its group.
        st.entered = o.parent.map(|g| (doc_id, g));
        actions.push(Action::run(doc_id, "object.select", json!({ "ids": ids })));
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
            let tex = icons::texture_colored(ui.ctx(), icon, 14.0, colour);
            let tint = egui::Color32::WHITE.gamma_multiply(alpha);
            let img = Rect::from_center_size(rect.center(), egui::Vec2::splat(14.0));
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

/// Name, kind, linked copies and linked file of one object.
fn details(
    ui: &mut Ui,
    o: &ObjView,
    st: &PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let doc_id = snap.doc_id;
    let src = &st.sources;
    ui.label(RichText::new(&o.name).font(fonts::medium(14.0)));
    if let Some(b) = o.bounds() {
        ui.label(
            RichText::new(format!("{:.1} x {:.1} mm", b.width(), b.height())).color(t.text_dim),
        );
    }
    if let Some(g) = o.link_group {
        let n = snap.group_size(g);
        ui.label(
            RichText::new(format!(
                "Linked copy, one of {n}. It shares the source, the cut options and the scale."
            ))
            .color(t.text_dim),
        );
        if ui
            .button("Unlink copy")
            .on_hover_text("Make this copy independent. The others stay linked.")
            .clicked()
        {
            actions.push(Action::run(
                doc_id,
                "object.unlink_copies",
                json!({ "ids": [o.id] }),
            ));
        }
    }
    let Some(path) = &o.link else {
        ui.label(RichText::new("Not linked to a file").color(t.text_dim));
        return;
    };
    ui.label(RichText::new("Linked file").color(t.text_dim));
    ui.add(egui::Label::new(path.display().to_string()).wrap());
    if src.missing.contains(&o.id) {
        ui.colored_label(
            t.danger,
            "The file is gone. The project keeps its own copy.",
        );
    } else if src.unreadable.contains(&o.id) {
        ui.colored_label(t.danger, "File can't be read");
    } else if src.changed.contains(&o.id) {
        ui.colored_label(t.accent_prepare, "The file has changed since the import.");
    }
    ui.horizontal(|ui| {
        if src.broken(o.id) && ui.button("Reconnect file...").clicked() {
            actions.push(Action::Reconnect { doc_id, id: o.id });
        }
        if ui
            .button("Unlink file")
            .on_hover_text("Forget the file. The object stays as it is.")
            .clicked()
        {
            actions.push(Action::run(
                doc_id,
                "object.unlink",
                json!({ "ids": [o.id] }),
            ));
        }
    });
}
