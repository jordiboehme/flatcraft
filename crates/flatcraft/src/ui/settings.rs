//! The Settings panel: what is cut of the selected objects, then plotter,
//! mat, material and the speed, force and passes of the job.

use egui::{Response, RichText, Ui};
use flatcraft_core::document::{CutMode, StrokeMode};
use flatcraft_core::numexpr::{Field, Kind};
use serde_json::{Value, json};

use super::calc::{self, Number};
use super::icons::Icon;
use super::prepare::{Eleven, Override, PrepareState, Snapshot};
use super::rails::{Side, SidePanel};
use super::theme::Tokens;
use super::tools::Setting;
use super::{Action, fonts, icon_button};

/// The `job.set_overrides` field of an override.
fn override_key(which: Override) -> &'static str {
    match which {
        Override::Speed => "speed",
        Override::Force => "force",
        Override::Passes => "passes",
    }
}

/// The speed hint after a typed speed above the plotter's maximum.
pub const ELEVEN: &str = "These go to eleven.";

/// The speed hint shows while the clamped attempt is still the speed of
/// this document, plotter and material. On the frame of the attempt the
/// override command has not run yet, so it shows from the next frame on.
fn eleven_shown(st: &PrepareState, snap: &Snapshot) -> bool {
    st.speed_eleven.as_ref().is_some_and(|e| e.current(snap))
}

/// Sets the accessible name of a widget, replacing the one it gave itself.
pub fn set_name(ui: &Ui, resp: &Response, name: &str) {
    ui.ctx().accesskit_node_builder(resp.id, |node| {
        node.clear_labelled_by();
        node.set_label(name);
    });
}

/// A row label that is only painted, so the widget it names (which gets
/// the name through [`set_name`]) is the only accessible node with it.
pub fn painted_label(ui: &mut Ui, text: &str) {
    let color = ui.visuals().text_color();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    ui.painter().galley(rect.left_top(), galley, color);
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

/// A labelled combo box. Returns the picked value when it differs from
/// `current`; `selected` is the entry shown as selected (usually the same).
#[allow(clippy::too_many_arguments)]
fn combo<T: Clone + PartialEq>(
    ui: &mut Ui,
    label: &str,
    setting: Option<Setting>,
    text: &str,
    choices: &[(T, String)],
    selected: Option<&T>,
    current: Option<&T>,
    enabled: bool,
) -> Option<T> {
    let mut picked = None;
    painted_label(ui, label);
    ui.add_enabled_ui(enabled, |ui| {
        let r = egui::ComboBox::from_id_salt(label)
            .selected_text(text)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for (value, text) in choices {
                    if ui
                        .selectable_label(selected == Some(value), text.as_str())
                        .clicked()
                        && current != Some(value)
                    {
                        picked = Some(value.clone());
                    }
                }
            });
        set_name(ui, &r.response, label);
        if let Some(setting) = setting {
            let why = (!enabled).then_some("Select an object first.");
            super::groups::card(&r.response, label, setting.card(), why, false);
        }
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

    if super::rails::header(ui, SidePanel::Settings, Side::Right, "Cut") {
        st.panels.set_collapsed(SidePanel::Settings, true);
    }
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
            if let Some(m) = combo(
                ui,
                "Cut mode",
                Some(Setting::CutMode),
                text,
                &modes,
                mode.as_ref(),
                mode.as_ref(),
                any,
            ) {
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
            if let Some(m) = combo(
                ui,
                "Strokes",
                Some(Setting::Strokes),
                text,
                &choices,
                strokes.as_ref(),
                strokes.as_ref(),
                any,
            ) {
                let v = match m {
                    StrokeMode::Auto => "auto",
                    StrokeMode::CentreLine => "centre_line",
                    StrokeMode::Outline => "outline",
                };
                run(actions, "object.set_cut_options", json!({ "strokes": v }));
            }
        });
    if let Some(n) = snap.linked_reach() {
        ui.label(RichText::new(format!("Applies to {n} linked copies")).color(t.text_dim))
            .on_hover_text("Linked copies share the cut options and the scale. Unlink a copy in the Objects panel to change it alone.");
    }

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
                None,
                name,
                &plotters,
                Some(&snap.job.plotter_id),
                Some(&snap.job.plotter_id),
                true,
            ) {
                run(actions, "job.set_plotter", json!({ "id": id }));
            }
            // A mat the plotter no longer lists: its first mat is used and
            // shown as selected, marked "(fallback)"; picking it makes it
            // the document's own mat.
            let fallback = snap.mat_warning.is_some();
            let shown = snap.mat.as_ref().map(|m| m.id.clone());
            let mark = |id: &str, name: &str| {
                if fallback && shown.as_deref() == Some(id) {
                    format!("{name} (fallback)")
                } else {
                    name.to_string()
                }
            };
            let mats: Vec<(String, String)> = snap
                .plotter
                .iter()
                .flat_map(|p| &p.media)
                .map(|m| (m.id.clone(), mark(&m.id, &m.name)))
                .collect();
            let name = snap
                .mat
                .as_ref()
                .map_or(snap.job.media_id.clone(), |m| mark(&m.id, &m.name));
            if let Some(id) = combo(
                ui,
                "Mat",
                None,
                &name,
                &mats,
                shown.as_ref(),
                Some(&snap.job.media_id),
                true,
            ) {
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
                None,
                name,
                &materials,
                Some(&snap.job.material_id),
                Some(&snap.job.material_id),
                true,
            ) {
                run(actions, "job.set_material", json!({ "id": id }));
            }
            // The strategy shows here; the Settings window sets the one
            // new documents start with.
            painted_label(ui, "Strategy");
            let name = super::settings_window::strategy_name(snap.job.strategy);
            let r = ui.label(name);
            super::groups::card(
                &r,
                "Strategy",
                "The order this document cuts in. Quality cleans up the corners, Fast keeps the travel short, and Keep sheet cuts the outlines last. New documents take it from Settings, Cutting.",
                None,
                false,
            );
            ui.end_row();
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
                    [plotter.passes_range[0].max(1), plotter.passes_range[1]],
                ),
            ];
            for (which, label, set, base, range) in rows {
                let value = set.unwrap_or(base);
                let modified = set.is_some_and(|v| v != base);
                ui.horizontal(|ui| {
                    painted_label(ui, label);
                    if modified {
                        let (r, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter()
                            .circle_filled(r.center(), 3.5, t.accent_prepare);
                    }
                });
                let mut v = f64::from(value);
                // A typed speed (a number or a calculation) above the
                // maximum is clamped, and the reader notes the attempt.
                // Force and passes refuse a value outside their range.
                let typed_over = std::cell::Cell::new(None::<bool>);
                let max = f64::from(range[1]);
                let field = Field::plain().integer().range(f64::from(range[0]), max);
                let spec = if which == Override::Speed {
                    Number::new(field).clamp()
                } else {
                    Number::new(field)
                };
                let (resp, typed) = calc::number_seen(
                    ui,
                    ("flatcraft-job", override_key(which)),
                    true,
                    &mut v,
                    &spec,
                    |n| {
                        if which == Override::Speed {
                            typed_over.set(Some(n.round() > max));
                        }
                    },
                );
                if let Some(over) = typed_over.get() {
                    st.speed_eleven = over.then(|| Eleven {
                        doc_id: snap.doc_id,
                        plotter_id: snap.job.plotter_id.clone(),
                        material_id: snap.job.material_id.clone(),
                        speed: range[1],
                    });
                }
                let eleven = which == Override::Speed && eleven_shown(st, snap);
                let name = if modified {
                    format!("{label} (modified)")
                } else {
                    label.to_string()
                };
                set_name(ui, &resp, &name);
                let setting = match which {
                    Override::Speed => Setting::Speed,
                    Override::Force => Setting::Force,
                    Override::Passes => Setting::Passes,
                };
                let what = format!(
                    "{} From {} to {}. The material says {base}. {}",
                    setting.card(),
                    range[0],
                    range[1],
                    calc::hint(Kind::Plain)
                );
                if eleven {
                    super::groups::card(&resp, label, &what, Some(ELEVEN), false);
                } else {
                    super::groups::card(&resp, label, &what, None, false);
                }
                if typed {
                    let n = v.round() as u32;
                    if n != value {
                        run(
                            actions,
                            "job.set_overrides",
                            json!({ override_key(which): n }),
                        );
                    }
                }
                if modified {
                    let reset = format!("Reset {}", label.to_lowercase());
                    if icon_button(ui, Icon::RotateLeft, &reset, true).clicked() {
                        // null removes this override only.
                        run(
                            actions,
                            "job.set_overrides",
                            json!({ override_key(which): null }),
                        );
                    }
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
    if eleven_shown(st, snap) {
        ui.label(RichText::new(ELEVEN).color(t.accent_cut));
    }
    if let Some(hint) = &material.blade_depth_hint {
        ui.add_space(t.spacing / 2.0);
        ui.label(RichText::new(format!("Blade depth: {hint}")).color(t.text_dim));
    }
    preset_notes(
        ui,
        "Plotter notes",
        &plotter.name,
        plotter.notes.as_deref(),
        t,
    );
    preset_notes(
        ui,
        "Material notes",
        &material.name,
        material.notes.as_deref(),
        t,
    );
    last_time(ui, snap, t, actions);
}

/// The Job card's "Last time with this material" line from the cut log
/// (the newest rated cut with this material, else the newest), and a
/// button that asks how the newest cut nobody rated yet went.
fn last_time(ui: &mut Ui, snap: &Snapshot, t: &Tokens, actions: &mut Vec<Action>) {
    use super::outcome;
    if let Some(e) = &snap.last_time {
        ui.add_space(t.spacing / 2.0);
        let text = outcome::last_time_text(e);
        let r = ui.add(egui::Label::new(RichText::new(&text).color(t.text_dim)).wrap());
        super::groups::card(
            &r,
            "Last time",
            &format!(
                "From the cut log on this computer: the newest cut with {} that you rated, else the newest cut with it. The cut: {}.",
                e.material,
                outcome::cut_text(e)
            ),
            None,
            false,
        );
    }
    if let Some(e) = &snap.to_rate {
        let label = outcome::rate_label(e);
        let b = ui.button(&label);
        super::groups::card(
            &b,
            &label,
            &format!("{} The cut: {}.", outcome::ASK_HINT, outcome::cut_text(e)),
            None,
            false,
        );
        if b.clicked() {
            actions.push(Action::AskOutcome(e.id.clone()));
        }
    }
}

/// The person's notes on the picked plotter or material, under a small
/// heading. Nothing shows when the preset has none.
fn preset_notes(ui: &mut Ui, heading: &str, preset: &str, notes: Option<&str>, t: &Tokens) {
    let Some(notes) = notes.map(str::trim_end).filter(|n| !n.trim().is_empty()) else {
        return;
    };
    ui.add_space(t.spacing / 2.0);
    let r = ui.label(
        RichText::new(heading)
            .font(fonts::semibold(12.0))
            .color(t.text_dim),
    );
    super::groups::card(
        &r,
        heading,
        &format!("Your notes on {preset}. Change them in Settings, in the presets of {preset}."),
        None,
        false,
    );
    ui.add(egui::Label::new(notes).wrap());
}
