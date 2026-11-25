//! The Objects panel: one row per object with its kind, name, a mark for
//! linked copies, file status and an eye to show or hide it and scissors to skip its cut. Below the
//! list, the details of a single selected object with its linked copies
//! and its linked file.

use egui::{RichText, Ui};
use serde_json::json;

use super::icons::{self, Icon};
use super::prepare::{ObjView, PrepareState, Snapshot};
use super::theme::Tokens;
use super::{Action, ICON_SIZE, fonts, icon_button, icon_button_hint};

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
    st: &PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let doc_id = snap.doc_id;
    let src = &st.sources;
    ui.label(RichText::new("Objects").font(fonts::semibold(14.0)));
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
                ui.horizontal(|ui| {
                    let (icon, kind) = if o.raster {
                        (Icon::ImageFile, "Traced image")
                    } else {
                        (Icon::SvgFile, "SVG")
                    };
                    badge(ui, icon, kind, t.text_dim);
                    let selected = snap.selection.contains(&o.id);
                    let name = if o.visible {
                        RichText::new(&o.name)
                    } else {
                        RichText::new(&o.name).color(t.text_dim)
                    };
                    let row = ui.add(egui::Button::selectable(selected, name));
                    if o.link_group.is_some() {
                        badge(ui, Icon::Linked, "Linked copy", t.text_dim);
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
                    if src.changed.contains(&o.id) {
                        badge(ui, Icon::Reload, "Source file changed", t.accent_prepare);
                    } else if src.missing.contains(&o.id) {
                        badge(ui, Icon::BrokenLink, "Source file not found", t.danger);
                    } else if src.unreadable.contains(&o.id) {
                        badge(ui, Icon::BrokenLink, "Source file can't be read", t.danger);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (icon, name) = if o.visible {
                            (Icon::Eye, format!("Hide {}", o.name))
                        } else {
                            (Icon::EyeOff, format!("Show {}", o.name))
                        };
                        if icon_button(ui, icon, &name, true).clicked() {
                            actions.push(Action::run(
                                doc_id,
                                "object.set_visible",
                                json!({ "ids": [o.id], "visible": !o.visible }),
                            ));
                        }
                        let (hint, name) = if o.cut_enabled {
                            ("Skip cutting", format!("Skip cutting {}", o.name))
                        } else {
                            ("Cut again", format!("Cut again {}", o.name))
                        };
                        if icon_button_hint(ui, Icon::Cut, &name, true, hint).clicked() {
                            actions.push(toggle_cut(snap, o));
                        }
                    });
                });
            }
        });

    let mut selected = snap.selected();
    if let (Some(o), None) = (selected.next(), selected.next()) {
        ui.separator();
        details(ui, o, st, snap, t, actions);
    }
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
