//! The Settings panel: what is cut of the selected objects, then plotter,
//! mat, material and the speed, force and passes of the job.

use egui::{Response, RichText, Ui};
use flatcraft_core::document::{CutMode, StrokeMode};
use serde_json::{Value, json};

use super::icons::Icon;
use super::prepare::{Override, PrepareState, Snapshot};
use super::theme::Tokens;
use super::{Action, fonts, icon_button};

/// Largest pass count `job.set_overrides` accepts.
pub const MAX_PASSES: u32 = 20;

/// Sets the accessible name of a widget, replacing the one it gave itself.
pub fn set_name(ui: &Ui, resp: &Response, name: &str) {
    ui.ctx().accesskit_node_builder(resp.id, |node| {
        node.clear_labelled_by();
        node.set_label(name);
    });
}

/// The value all selected objects share, or `None` when they differ.
fn common<T: PartialEq + Copy>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let first = values.next()?;
    values.all(|v| v == first).then_some(first)
}

fn mode_name(m: CutMode) -> &'static str {
    match m {
        CutMode::AllLines => "All lines",
        CutMode::ObjectOutlines => "Object outlines",
    }
}

fn strokes_name(m: StrokeMode) -> &'static str {
    match m {
        StrokeMode::Auto => "Auto",
        StrokeMode::CentreLine => "Centre line",
        StrokeMode::Outline => "Outline",
    }
}

/// A labelled combo box. Returns the picked value when it changed.
fn combo<T: Clone + PartialEq>(
    ui: &mut Ui,
    label: &str,
    current: &str,
    choices: &[(T, String)],
    selected: Option<&T>,
    enabled: bool,
) -> Option<T> {
    let mut picked = None;
    ui.label(label);
    ui.add_enabled_ui(enabled, |ui| {
        let r = egui::ComboBox::from_id_salt(label)
            .selected_text(current)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for (value, text) in choices {
                    if ui
                        .selectable_label(selected == Some(value), text.as_str())
                        .clicked()
                        && selected != Some(value)
                    {
                        picked = Some(value.clone());
                    }
                }
            });
        set_name(ui, &r.response, label);
    });
    ui.end_row();
    picked
}

pub fn show(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let doc_id = snap.doc_id;
    let run = |actions: &mut Vec<Action>, cmd: &'static str, params: Value| {
        actions.push(Action::run(doc_id, cmd, params));
    };

    ui.label(RichText::new("Cut").font(fonts::semibold(14.0)));
    let selected: Vec<_> = snap.selected().collect();
    if selected.is_empty() {
        ui.label(RichText::new("Select an object to choose what is cut.").color(t.text_dim));
    }
    egui::Grid::new("flatcraft-cut-options")
        .num_columns(2)
        .spacing([t.spacing, t.spacing * 0.75])
        .show(ui, |ui| {
            let any = !selected.is_empty();
            let mode = common(selected.iter().map(|o| o.cut.mode));
            let modes =
                [CutMode::ObjectOutlines, CutMode::AllLines].map(|m| (m, mode_name(m).to_string()));
            let text = match (any, mode) {
                (false, _) => "",
                (true, Some(m)) => mode_name(m),
                (true, None) => "Mixed",
            };
            if let Some(m) = combo(ui, "Cut mode", text, &modes, mode.as_ref(), any) {
                let v = match m {
                    CutMode::AllLines => "all_lines",
                    CutMode::ObjectOutlines => "object_outlines",
                };
                run(actions, "object.set_cut_options", json!({ "mode": v }));
            }
            let strokes = common(selected.iter().map(|o| o.cut.strokes));
            let choices = [
                StrokeMode::Auto,
                StrokeMode::CentreLine,
                StrokeMode::Outline,
            ]
            .map(|m| (m, strokes_name(m).to_string()));
            let text = match (any, strokes) {
                (false, _) => "",
                (true, Some(m)) => strokes_name(m),
                (true, None) => "Mixed",
            };
            if let Some(m) = combo(ui, "Strokes", text, &choices, strokes.as_ref(), any) {
                let v = match m {
                    StrokeMode::Auto => "auto",
                    StrokeMode::CentreLine => "centre_line",
                    StrokeMode::Outline => "outline",
                };
                run(actions, "object.set_cut_options", json!({ "strokes": v }));
            }
        });

    ui.separator();
    job(ui, st, snap, t, actions);
}

/// Plotter, mat, material and the speed, force and passes of the job. The
/// Cut workspace shows the same controls in its first stage.
pub fn job(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let doc_id = snap.doc_id;
    let run = |actions: &mut Vec<Action>, cmd: &'static str, params: Value| {
        actions.push(Action::run(doc_id, cmd, params));
    };
    ui.label(RichText::new("Job").font(fonts::semibold(14.0)));
    egui::Grid::new("flatcraft-job")
        .num_columns(2)
        .spacing([t.spacing, t.spacing * 0.75])
        .show(ui, |ui| {
            let plotters: Vec<(String, String)> = snap
                .plotters
                .iter()
                .map(|p| (p.id.clone(), p.name.clone()))
                .collect();
            let name = snap
                .plotter
                .as_ref()
                .map_or(snap.job.plotter_id.as_str(), |p| p.name.as_str());
            if let Some(id) = combo(
                ui,
                "Plotter",
                name,
                &plotters,
                Some(&snap.job.plotter_id),
                true,
            ) {
                run(actions, "job.set_plotter", json!({ "id": id }));
            }
            let mats: Vec<(String, String)> = snap
                .plotter
                .iter()
                .flat_map(|p| &p.media)
                .map(|m| (m.id.clone(), m.name.clone()))
                .collect();
            let name = snap
                .mat
                .as_ref()
                .map_or(snap.job.media_id.as_str(), |m| m.name.as_str());
            if let Some(id) = combo(ui, "Mat", name, &mats, Some(&snap.job.media_id), true) {
                run(actions, "job.set_media", json!({ "id": id }));
            }
            let materials: Vec<(String, String)> = snap
                .materials
                .iter()
                .map(|m| (m.id.clone(), m.name.clone()))
                .collect();
            let name = snap
                .material
                .as_ref()
                .map_or(snap.job.material_id.as_str(), |m| m.name.as_str());
            if let Some(id) = combo(
                ui,
                "Material",
                name,
                &materials,
                Some(&snap.job.material_id),
                true,
            ) {
                run(actions, "job.set_material", json!({ "id": id }));
            }
        });

    let (Some(plotter), Some(material)) = (&snap.plotter, &snap.material) else {
        return;
    };
    ui.add_space(t.spacing / 2.0);
    egui::Grid::new("flatcraft-overrides")
        .num_columns(3)
        .spacing([t.spacing, t.spacing * 0.75])
        .show(ui, |ui| {
            let job = &snap.job;
            let rows = [
                (
                    Override::Speed,
                    "Speed",
                    job.speed.map(u32::from),
                    u32::from(material.speed),
                    [
                        u32::from(plotter.speed_range[0]),
                        u32::from(plotter.speed_range[1]),
                    ],
                ),
                (
                    Override::Force,
                    "Force",
                    job.force.map(u32::from),
                    u32::from(material.force),
                    [
                        u32::from(plotter.force_range[0]),
                        u32::from(plotter.force_range[1]),
                    ],
                ),
                (
                    Override::Passes,
                    "Passes",
                    job.passes,
                    material.passes,
                    [1, MAX_PASSES],
                ),
            ];
            for (which, label, set, base, range) in rows {
                let value = set.unwrap_or(base);
                let modified = set.is_some_and(|v| v != base);
                ui.horizontal(|ui| {
                    ui.label(label);
                    if modified {
                        let (r, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter()
                            .circle_filled(r.center(), 3.5, t.accent_prepare);
                    }
                });
                let mut v = match st.live_override {
                    Some((w, x)) if w == which => x,
                    _ => f64::from(value),
                };
                let resp = ui.add(
                    egui::DragValue::new(&mut v)
                        .range(f64::from(range[0])..=f64::from(range[1]))
                        .speed(0.1)
                        .max_decimals(0)
                        .update_while_editing(false),
                );
                let name = if modified {
                    format!("{label} (modified)")
                } else {
                    label.to_string()
                };
                set_name(ui, &resp, &name);
                let resp = resp.on_hover_text(format!(
                    "{label}: {} to {}. The material says {base}.",
                    range[0], range[1]
                ));
                // One command when a drag ends or a typed value lands,
                // never one per frame of a drag.
                if resp.dragged() {
                    st.live_override = Some((which, v));
                }
                let done = resp.drag_stopped() || (resp.changed() && !resp.dragged());
                if done {
                    if matches!(st.live_override, Some((w, _)) if w == which) {
                        st.live_override = None;
                    }
                    let n = v.round() as u32;
                    if n != value {
                        let key = match which {
                            Override::Speed => "speed",
                            Override::Force => "force",
                            Override::Passes => "passes",
                        };
                        run(actions, "job.set_overrides", json!({ key: n }));
                    }
                }
                if modified {
                    let reset = format!("Reset {}", label.to_lowercase());
                    if icon_button(ui, Icon::RotateLeft, &reset, true).clicked() {
                        // Clear all overrides and send the others back.
                        let mut p = json!({ "clear": true });
                        for (w, key, set) in [
                            (Override::Speed, "speed", job.speed.map(u32::from)),
                            (Override::Force, "force", job.force.map(u32::from)),
                            (Override::Passes, "passes", job.passes),
                        ] {
                            if w != which
                                && let Some(x) = set
                            {
                                p[key] = json!(x);
                            }
                        }
                        run(actions, "job.set_overrides", p);
                    }
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
    if let Some(hint) = &material.blade_depth_hint {
        ui.add_space(t.spacing / 2.0);
        ui.label(RichText::new(format!("Blade depth: {hint}")).color(t.text_dim));
    }
}
