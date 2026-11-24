//! The app bar: name, Home, File, Save, Undo, Redo and the document tabs on
//! the left; the agent pill, Settings, Help and the version on the right.

use std::time::Instant;

use egui::{RichText, Ui};
use serde_json::json;

use super::icons::Icon;
use super::{Action, FlatCraftApp, Workspace, display_name, fonts, icon_button, lock};

/// One open document as the tabs show it.
struct Tab {
    id: u64,
    name: String,
    dirty: bool,
}

pub fn show(ui: &mut Ui, app: &FlatCraftApp, actions: &mut Vec<Action>) {
    let t = app.tokens;
    let (tabs, active, can_undo, can_redo) = {
        let s = lock(&app.session);
        let tabs: Vec<Tab> = s
            .docs
            .iter()
            .map(|d| Tab {
                id: d.id(),
                name: d.path.as_deref().map(display_name).unwrap_or_else(|| {
                    let n = app.ui.untitled.get(&d.id()).copied().unwrap_or(1);
                    format!("Untitled {n}")
                }),
                dirty: d.dirty,
            })
            .collect();
        let doc = s.doc();
        (tabs, doc.id(), doc.undo.can_undo(), doc.undo.can_redo())
    };
    let agent = app.agent_status(Instant::now());
    // The two sides are separate closures; the right one collects here.
    let mut right_actions = Vec::new();

    egui::Panel::top("flatcraft-app-bar")
        .frame(
            egui::Frame::new()
                .fill(t.panel)
                .inner_margin(egui::Margin::symmetric(
                    t.spacing as i8,
                    (t.spacing / 2.0) as i8,
                )),
        )
        .show(ui, |ui| {
            egui::Sides::new().show(
                ui,
                |ui| {
                    ui.label(RichText::new("FlatCraft").font(fonts::semibold(16.0)));
                    ui.add_space(t.spacing);
                    if icon_button(ui, Icon::Home, "Home", true).clicked() {
                        actions.push(Action::SetWorkspace(Workspace::Home));
                    }
                    ui.menu_button("File", |ui| {
                        if ui.button("New").clicked() {
                            actions.push(Action::NewProject);
                        }
                        if ui.button("Open").clicked() {
                            actions.push(Action::OpenDialog);
                        }
                        if ui.button("Save").clicked() {
                            actions.push(Action::Save { doc_id: active });
                        }
                        if ui.button("Save As").clicked() {
                            actions.push(Action::SaveAs { doc_id: active });
                        }
                    });
                    if icon_button(ui, Icon::Save, "Save", true).clicked() {
                        actions.push(Action::Save { doc_id: active });
                    }
                    if icon_button(ui, Icon::Undo, "Undo", can_undo).clicked() {
                        actions.push(Action::Execute("edit.undo", json!({})));
                    }
                    if icon_button(ui, Icon::Redo, "Redo", can_redo).clicked() {
                        actions.push(Action::Execute("edit.redo", json!({})));
                    }
                    ui.separator();
                    for tab in &tabs {
                        let label = if tab.dirty {
                            format!("{} *", tab.name)
                        } else {
                            tab.name.clone()
                        };
                        if ui
                            .add(egui::Button::selectable(tab.id == active, label))
                            .clicked()
                            && tab.id != active
                        {
                            actions.push(Action::Switch(tab.id));
                        }
                        let close = format!("Close {}", tab.name);
                        if icon_button(ui, Icon::Close, &close, true).clicked() {
                            actions.push(Action::Close(tab.id));
                        }
                    }
                    if icon_button(ui, Icon::Plus, "New document", true).clicked() {
                        actions.push(Action::NewDocument);
                    }
                },
                |ui| {
                    let actions = &mut right_actions;
                    ui.label(
                        RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).color(t.text_dim),
                    );
                    if icon_button(ui, Icon::Help, "Help", true).clicked() {
                        actions.push(Action::Help);
                    }
                    if icon_button(ui, Icon::Settings, "Settings", true).clicked() {
                        actions.push(Action::ShowProfiles);
                    }
                    if let Some(status) = agent {
                        let color = if status == "Agent connected" {
                            t.accent_prepare
                        } else {
                            t.text_dim
                        };
                        egui::Frame::new()
                            .stroke(egui::Stroke::new(1.0, color))
                            .corner_radius(t.radius * 2)
                            .inner_margin(egui::Margin::symmetric(t.spacing as i8, 2))
                            .show(ui, |ui| {
                                ui.label(RichText::new(status).color(color));
                            });
                    }
                },
            );
        });
    actions.extend(right_actions);
}
