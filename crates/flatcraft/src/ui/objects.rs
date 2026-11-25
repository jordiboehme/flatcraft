//! The Objects panel: one flat row per object with its kind, name, a mark for
//! linked copies, file status, an eye to show or hide it and scissors to
//! skip its cut. The two toggles are quiet until the row is hovered, and
//! clear when the object is hidden or skipped. Below the list, the details of a single selected object with its linked copies
//! and its linked file.

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
    if snap.objects.is_empty() {
        ui.label(
            RichText::new("Nothing here yet. Import a design to get started.").color(t.text_dim),
        );
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .max_height(ui.available_height() * 0.6)
        .show(ui, |ui| {
            for o in &snap.objects {
                object_row(ui, st, snap, o, t, actions);
            }
        });

    let mut selected = snap.selected();
    if let (Some(o), None) = (selected.next(), selected.next()) {
        ui.separator();
        details(ui, o, st, snap, t, actions);
    }
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
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let doc_id = snap.doc_id;
    let selected = snap.selection.contains(&o.id);
    let (rect, row) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::click(),
    );
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
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            t.accent_prepare.gamma_multiply(0.22)
        } else if over {
            t.surface
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, t.radius, fill);
    }
    // The kind, the name and the marks.
    let mut left = rect;
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
        st.context = Some(context_menu::target(snap, Some(o.id), actions));
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
