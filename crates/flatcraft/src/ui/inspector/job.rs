//! The Job panel: what this cut uses. Machine and material (plotter, mat,
//! material and the test cut hint), the cut values (speed, force and
//! passes, with a change dot against the material), the order, and the
//! folded Weeding, Material tips and Advanced sections. Prepare shows it
//! when nothing is selected (or the job line asks for it), and the Cut
//! workspace in its first stage.

use egui::{Response, RichText, Ui};
use flatcraft_core::document::{CutStrategy, WeedLines};
use flatcraft_core::numexpr::{Field, Kind};
use serde_json::{Value, json};

use super::super::calc::{self, Number};
use super::super::prepare::{Eleven, Override, PrepareState, Snapshot};
use super::super::rails::Dot;
use super::super::settings::set_name;
use super::super::settings::{
    ELEVEN, SHEET_KEPT_NO_WEEDING, TIPS_HEADING, WEED_BOX_OFF, WEED_LINES_OFF, WEEDING_HINT,
    WEEDING_OFF, weed_lines_name,
};
use super::super::settings_window::strategy_name;
use super::super::theme::Tokens;
use super::super::tools::Setting;
use super::super::{Action, groups, materials, metrics, outcome};
use super::blocks::{self, JobSummary, SourceValue};

/// The ids of the Job panel's sections.
pub const MACHINE: &str = "job.machine";
pub const VALUES: &str = "job.values";
pub const ORDER: &str = "job.order";
pub const WEEDING: &str = "job.weeding";
pub const TIPS: &str = "job.tips";
pub const ADVANCED: &str = "job.advanced";

/// The titles of the sections, in the panel's order, with their ids.
pub const SECTIONS: [(&str, &str); 6] = [
    (MACHINE, "Machine and material"),
    (VALUES, "Cut values"),
    (ORDER, "Order"),
    (WEEDING, "Weeding"),
    (TIPS, TIPS_HEADING),
    (ADVANCED, "Advanced"),
];

/// Every setting of the Job panel, as (section id, accessible name), for
/// the reachability test: each one is in its section, at most one fold
/// away.
pub const SETTINGS: [(&str, &str); 15] = [
    (MACHINE, "Plotter"),
    (MACHINE, "Mat"),
    (MACHINE, "Material"),
    (VALUES, "Speed"),
    (VALUES, "Force"),
    (VALUES, "Passes"),
    (ORDER, "Strategy"),
    (WEEDING, "Weed box"),
    (WEEDING, "Box margin"),
    (WEEDING, "One box per object"),
    (WEEDING, "Weed lines"),
    (WEEDING, "Line spacing"),
    (ADVANCED, "Blade depth"),
    (ADVANCED, "Overcut"),
    (ADVANCED, "Track enhancing"),
];

/// The job line's summary of `snap`, with the state dot `dot`; `open`
/// says the Job panel shows over a selection.
pub fn summary(snap: &Snapshot, dot: Dot, open: bool) -> JobSummary {
    JobSummary {
        plotter: snap
            .plotter
            .as_ref()
            .map_or(snap.job.plotter_id.clone(), |p| p.name.clone()),
        plotter_short: snap.plotter.as_ref().and_then(plotter_short),
        material: snap
            .material
            .as_ref()
            .map_or(snap.job.material_id.clone(), |m| m.name.clone()),
        mat: snap
            .mat
            .as_ref()
            .map_or(snap.job.media_id.clone(), |m| m.name.clone()),
        strategy: strategy_name(snap.job.strategy).to_string(),
        dot,
        selection: !snap.selection.is_empty(),
        open,
    }
}

/// The word the job line shows for a File Output plotter.
pub const FILE_SHORT: &str = "File";

/// The plotter's short name for the job line: the model of a USB plotter
/// ("Cameo 2"), "File" for a File Output preset. The full name is in the
/// line's hover card.
pub fn plotter_short(p: &flatcraft_device::PlotterProfile) -> Option<String> {
    use flatcraft_device::TransportSpec;
    let model = p.model.trim();
    match p.transport {
        TransportSpec::File { .. } => Some(FILE_SHORT.to_string()),
        TransportSpec::Usb { .. } => {
            (!model.is_empty() && model.len() < p.name.len()).then(|| model.to_string())
        }
    }
}

/// The `job.set_overrides` field of an override.
fn override_key(which: Override) -> &'static str {
    match which {
        Override::Speed => "speed",
        Override::Force => "force",
        Override::Passes => "passes",
    }
}

/// The speed hint shows while the clamped attempt is still the speed of
/// this document, plotter and material. On the frame of the attempt the
/// override command has not run yet, so it shows from the next frame on.
fn eleven_shown(st: &PrepareState, snap: &Snapshot) -> bool {
    st.speed_eleven.as_ref().is_some_and(|e| e.current(snap))
}

/// "1 pass", "3 passes".
fn passes_word(n: u32) -> String {
    if n == 1 {
        "1 pass".into()
    } else {
        format!("{n} passes")
    }
}

/// A combo box `width` wide with an entry per choice (and its hint).
/// Returns the combo and the picked value when it differs from `current`.
/// Its text is cut short when it does not fit, so the panel never grows.
#[allow(clippy::too_many_arguments)]
pub fn combo<T: Clone + PartialEq>(
    ui: &mut Ui,
    salt: &str,
    text: &str,
    choices: &[(T, String)],
    hints: &[&str],
    selected: Option<&T>,
    current: Option<&T>,
    enabled: bool,
) -> (Response, Option<T>) {
    let mut picked = None;
    let width = ui.available_width();
    let r = ui
        .add_enabled_ui(enabled, |ui| {
            egui::ComboBox::from_id_salt(salt)
                .selected_text(text)
                .width(width)
                .truncate()
                .show_ui(ui, |ui| {
                    for (i, (value, text)) in choices.iter().enumerate() {
                        let mut r = ui.selectable_label(selected == Some(value), text.as_str());
                        if let Some(hint) = hints.get(i) {
                            r = r.on_hover_text(*hint);
                        }
                        if r.clicked() && current != Some(value) {
                            picked = Some(value.clone());
                        }
                    }
                })
                .response
        })
        .inner;
    (r, picked)
}

/// The card of the Plotter row.
pub const PLOTTER_CARD: &str = "The machine that cuts this document. It sets the speed and force ranges and how the job is sent. Pick a File Output preset to write a file instead of cutting.";
/// The card of the Mat row.
pub const MAT_CARD: &str = "The carrier sheet the material sits on, or no mat. Its printed grid is where FlatCraft's 0,0 is, and the grid is the area that can be cut.";
/// The card of the Strategy row.
pub const STRATEGY_CARD: &str = "The order this document cuts in. Quality cleans up the corners, Fast keeps the travel short, and Outlines last cuts every outline after the holes and inner pieces. New documents take it from Settings, Cutting.";

/// The Job panel's sections.
pub fn panel(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    machine(ui, snap, t, actions);
    values(ui, st, snap, t, actions);
    order(ui, snap, actions);
    weeding(ui, snap, st.sheet_kept, t, actions);
    tips(ui, snap, t);
    advanced(ui, snap, t);
}

fn run(actions: &mut Vec<Action>, snap: &Snapshot, cmd: &'static str, params: Value) {
    actions.push(Action::run(snap.doc_id, cmd, params));
}

/// Machine and material: plotter, mat, material, the skipped preset note
/// and the test cut hint.
fn machine(ui: &mut Ui, snap: &Snapshot, t: &Tokens, actions: &mut Vec<Action>) {
    let plotter = snap
        .plotter
        .as_ref()
        .map_or(snap.job.plotter_id.as_str(), |p| p.name.as_str());
    let material = snap
        .material
        .as_ref()
        .map_or(snap.job.material_id.as_str(), |m| m.name.as_str());
    // A mat the plotter no longer lists: its first mat is used and shown
    // as selected, marked "(fallback)"; picking it makes it the
    // document's own mat.
    let fallback = snap.mat_warning.is_some();
    let shown = snap.mat.as_ref().map(|m| m.id.clone());
    let mark = |id: &str, name: &str| {
        if fallback && shown.as_deref() == Some(id) {
            format!("{name} (fallback)")
        } else {
            name.to_string()
        }
    };
    let mat_name = snap
        .mat
        .as_ref()
        .map_or(snap.job.media_id.clone(), |m| mark(&m.id, &m.name));
    let summary = format!("{plotter} · {mat_name} · {material}");
    blocks::section(
        ui,
        MACHINE,
        "Machine and material",
        "The plotter that cuts, the mat the material sits on, and the material. The material gives the starting speed, force and passes.",
        &summary,
        true,
        |ui| {
            let plotters: Vec<(String, String)> = snap
                .plotters
                .iter()
                .map(|p| (p.id.clone(), p.name.clone()))
                .collect();
            blocks::labels(ui, &["Plotter", "Mat", "Material"]);
            let mut picked = None;
            blocks::row(ui, "Plotter", PLOTTER_CARD, None, |ui| {
                let (r, p) = combo(
                    ui,
                    "Plotter",
                    plotter,
                    &plotters,
                    &[],
                    Some(&snap.job.plotter_id),
                    Some(&snap.job.plotter_id),
                    true,
                );
                picked = p;
                r
            });
            if let Some(id) = picked {
                run(actions, snap, "job.set_plotter", json!({ "id": id }));
            }
            let mats: Vec<(String, String)> = snap
                .plotter
                .iter()
                .flat_map(|p| &p.media)
                .map(|m| (m.id.clone(), mark(&m.id, &m.name)))
                .collect();
            let mut picked = None;
            blocks::row(ui, "Mat", MAT_CARD, None, |ui| {
                let (r, p) = combo(
                    ui,
                    "Mat",
                    &mat_name,
                    &mats,
                    &[],
                    shown.as_ref(),
                    Some(&snap.job.media_id),
                    true,
                );
                picked = p;
                r
            });
            if let Some(id) = picked {
                run(actions, snap, "job.set_media", json!({ "id": id }));
            }
            let mut picked = None;
            blocks::row(ui, "Material", &materials::picker_card(snap), None, |ui| {
                let (r, p) = materials::picker(ui, snap, t);
                picked = p;
                r
            });
            if let Some(id) = picked {
                run(actions, snap, "job.set_material", json!({ "id": id }));
            }
            materials::skipped_note(ui, snap, t);
            materials::test_cut_hint(ui, snap, t, actions);
        },
        None,
    );
}

/// Cut values: speed, force and passes, each with the change dot and the
/// reset arrow against the material, and the cut log's last time.
fn values(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let summary = match &snap.material {
        Some(m) => {
            let job = &snap.job;
            let changed = job.speed.is_some_and(|v| v != m.speed)
                || job.force.is_some_and(|v| v != m.force)
                || job.passes.is_some_and(|v| v != m.passes);
            let mut s = format!(
                "Speed {} · Force {} · {}",
                job.speed.unwrap_or(m.speed),
                job.force.unwrap_or(m.force),
                passes_word(job.passes.unwrap_or(m.passes))
            );
            if changed {
                s.push_str(" · changed");
            }
            s
        }
        None => "no material".into(),
    };
    blocks::section(
        ui,
        VALUES,
        "Cut values",
        "How the blade cuts this material: speed, force (how hard the blade presses) and passes (how often it cuts each line). They start at the material's values; a changed one gets an orange dot and an arrow that puts the material's value back.",
        &summary,
        true,
        |ui| {
            let (Some(plotter), Some(material)) = (&snap.plotter, &snap.material) else {
                blocks::note(
                    ui,
                    "Pick a plotter and a material first: they give the cut values.",
                    t.text_dim,
                );
                return;
            };
            let job = &snap.job;
            blocks::labels(ui, &["Speed", "Force", "Passes"]);
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
                let changed = set.is_some_and(|v| v != base);
                let source = SourceValue {
                    label: format!("the material {}", material.name),
                    value: base.to_string(),
                    changed,
                    // null removes this override only.
                    reset: Action::run(
                        snap.doc_id,
                        "job.set_overrides",
                        json!({ override_key(which): null }),
                    ),
                };
                let setting = match which {
                    Override::Speed => Setting::Speed,
                    Override::Force => Setting::Force,
                    Override::Passes => Setting::Passes,
                };
                let card = format!(
                    "{} From {} to {}. {}",
                    setting.card(),
                    range[0],
                    range[1],
                    calc::hint(Kind::Plain)
                );
                let eleven = which == Override::Speed && eleven_shown(st, snap);
                let mut v = f64::from(value);
                let mut typed = false;
                // A typed speed (a number or a calculation) above the
                // maximum is clamped, and the reader notes the attempt.
                // Force and passes refuse a value outside their range.
                let typed_over = std::cell::Cell::new(None::<bool>);
                let max = f64::from(range[1]);
                let r = blocks::row_dimmed(
                    ui,
                    label,
                    &card,
                    eleven.then_some(ELEVEN),
                    Some(source),
                    |ui| {
                        let field = Field::plain().integer().range(f64::from(range[0]), max);
                        let spec = Number::new(field).width(ui.available_width());
                        let spec = if which == Override::Speed {
                            spec.clamp()
                        } else {
                            spec
                        };
                        let (resp, t) = calc::number_seen(
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
                        typed = t;
                        resp
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
                if typed {
                    let n = v.round() as u32;
                    if n != value {
                        run(
                            actions,
                            snap,
                            "job.set_overrides",
                            json!({ override_key(which): n }),
                        );
                    }
                }
                actions.extend(r.reset);
            }
            if eleven_shown(st, snap) {
                blocks::note(ui, ELEVEN, t.warn_text);
            }
            last_time(ui, snap, t, actions);
        },
        None,
    );
}

/// Order: the strategy of this document (`job.set_strategy`).
fn order(ui: &mut Ui, snap: &Snapshot, actions: &mut Vec<Action>) {
    let current = snap.job.strategy;
    blocks::section(
        ui,
        ORDER,
        "Order",
        "In which order the shapes are cut. The plan in the Cut workspace follows it.",
        strategy_name(current),
        true,
        |ui| {
            let all = [
                CutStrategy::Quality,
                CutStrategy::Fast,
                CutStrategy::OutlinesLast,
            ];
            let choices = all.map(|s| (s, strategy_name(s).to_string()));
            let hints = [
                "Clean corners first: the blade starts on corners that go to waste, turns little and travels short.",
                "Short travel first. Use it for simple shapes when time counts.",
                "Like Quality, but every outline comes after the holes and inner pieces, so the sheet holds together until the end.",
            ];
            blocks::labels(ui, &["Strategy"]);
            let mut picked = None;
            blocks::row(ui, "Strategy", STRATEGY_CARD, None, |ui| {
                let (r, p) = combo(
                    ui,
                    "Strategy",
                    strategy_name(current),
                    &choices,
                    &hints,
                    Some(&current),
                    Some(&current),
                    true,
                );
                picked = p;
                r
            });
            if let Some(s) = picked {
                run(
                    actions,
                    snap,
                    "job.set_strategy",
                    json!({ "strategy": flatcraft_engine::simulate::strategy_name(s) }),
                );
            }
            blocks::actions_row(ui, "", COMPARE_CARD, |ui| {
                let r = ui.link(COMPARE);
                set_name(ui, &r, COMPARE);
                groups::card(&r, COMPARE, COMPARE_CARD, None, false);
                if r.clicked() {
                    actions.push(Action::CompareStrategies);
                }
            });
        },
        None,
    );
}

/// The Order section's link to the strategy comparison, and its card.
pub const COMPARE: &str = "Compare strategies";
pub const COMPARE_CARD: &str = "Opens the Cut workspace at Simulate, where this job is simulated with each cut order side by side: the grade, the worst miss, the travel and the time. Pick one there. Nothing changes until you do.";

/// The `job.set_weeding` value of a weed lines choice.
fn weed_lines_key(l: WeedLines) -> &'static str {
    match l {
        WeedLines::Off => "off",
        WeedLines::Horizontal => "horizontal",
        WeedLines::Vertical => "vertical",
        WeedLines::Both => "both",
    }
}

/// The summary of the Weeding section.
pub fn weeding_summary(snap: &Snapshot, sheet_kept: bool) -> String {
    let w = snap.job.weed;
    let mut parts = Vec::new();
    if w.weed_box {
        parts.push(if w.per_object {
            "box per object".to_string()
        } else {
            "box".to_string()
        });
    }
    if w.lines != WeedLines::Off {
        parts.push(format!("lines {}", weed_lines_name(w.lines).to_lowercase()));
    }
    if parts.is_empty() {
        parts.push("off".into());
    } else if sheet_kept {
        parts.push("nothing to weed".into());
    }
    parts.join(" · ")
}

/// A millimetre field of the weeding rows. Returns the typed value when it
/// changed.
fn weed_mm(
    ui: &mut Ui,
    label: &str,
    value: f64,
    (lo, hi): (f64, f64),
    setting: Setting,
    why: Option<&str>,
) -> Option<f64> {
    let mut v = value;
    let mut typed = false;
    let card = format!(
        "{} From {lo} to {hi} mm. {}",
        setting.card(),
        calc::hint(Kind::Length)
    );
    blocks::row_dimmed(ui, label, &card, why, None, |ui| {
        // The field fills the value column, less the unit after it.
        let unit = ui
            .painter()
            .layout_no_wrap("mm".into(), metrics::body(), egui::Color32::PLACEHOLDER)
            .size()
            .x;
        let w = ui.available_width() - unit - ui.spacing().item_spacing.x;
        let spec = Number::new(Field::new(Kind::Length).range(lo, hi))
            .decimals(0, 2)
            .unit("mm")
            .width(w);
        let (r, t) = calc::number(ui, ("flatcraft-weed", label), why.is_none(), &mut v, &spec);
        typed = t;
        r
    });
    (typed && v != value).then_some(v)
}

/// A checkbox row of the weeding rows. Returns the new value when it was
/// clicked.
fn weed_check(
    ui: &mut Ui,
    label: &str,
    on: bool,
    setting: Setting,
    why: Option<&str>,
) -> Option<bool> {
    let mut v = on;
    let r = blocks::row_dimmed(ui, label, setting.card(), why, None, |ui| {
        ui.add_enabled(why.is_none(), egui::Checkbox::without_text(&mut v))
    });
    r.widget.clicked().then_some(v)
}

/// Weeding: the weed box and weed lines of the job (`job.set_weeding`).
fn weeding(ui: &mut Ui, snap: &Snapshot, sheet_kept: bool, t: &Tokens, actions: &mut Vec<Action>) {
    use flatcraft_core::limits::{
        MAX_WEED_MARGIN_MM, MAX_WEED_SPACING_MM, MIN_WEED_MARGIN_MM, MIN_WEED_SPACING_MM,
    };
    let w = snap.job.weed;
    let summary = weeding_summary(snap, sheet_kept);
    blocks::section(
        ui,
        WEEDING,
        "Weeding",
        WEEDING_HINT,
        &summary,
        false,
        |ui| {
            let mut set = |params: Value| run(actions, snap, "job.set_weeding", params);
            blocks::labels(
                ui,
                &[
                    "Weed box",
                    "Box margin",
                    "One box per object",
                    "Weed lines",
                    "Line spacing",
                ],
            );
            // The settings stay usable: they apply again once no object
            // keeps the sheet.
            if sheet_kept {
                blocks::note(ui, SHEET_KEPT_NO_WEEDING, t.text_dim);
            }
            if let Some(v) = weed_check(ui, "Weed box", w.weed_box, Setting::WeedBox, None) {
                set(json!({ "box": v }));
            }
            if let Some(v) = weed_mm(
                ui,
                "Box margin",
                w.margin_mm,
                (MIN_WEED_MARGIN_MM, MAX_WEED_MARGIN_MM),
                Setting::WeedMargin,
                (!w.any()).then_some(WEEDING_OFF),
            ) {
                set(json!({ "margin_mm": v }));
            }
            if let Some(v) = weed_check(
                ui,
                "One box per object",
                w.per_object,
                Setting::WeedPerObject,
                (!w.weed_box).then_some(WEED_BOX_OFF),
            ) {
                set(json!({ "per_object": v }));
            }
            let all = [
                WeedLines::Off,
                WeedLines::Horizontal,
                WeedLines::Vertical,
                WeedLines::Both,
            ];
            let choices = all.map(|l| (l, weed_lines_name(l).to_string()));
            let mut picked = None;
            blocks::row(ui, "Weed lines", Setting::WeedLines.card(), None, |ui| {
                let (r, p) = combo(
                    ui,
                    "Weed lines",
                    weed_lines_name(w.lines),
                    &choices,
                    &[],
                    Some(&w.lines),
                    Some(&w.lines),
                    true,
                );
                picked = p;
                r
            });
            if let Some(l) = picked {
                set(json!({ "lines": weed_lines_key(l) }));
            }
            if let Some(v) = weed_mm(
                ui,
                "Line spacing",
                w.spacing_mm,
                (MIN_WEED_SPACING_MM, MAX_WEED_SPACING_MM),
                Setting::WeedSpacing,
                (w.lines == WeedLines::Off).then_some(WEED_LINES_OFF),
            ) {
                set(json!({ "spacing_mm": v }));
            }
        },
        None,
    );
}

/// The trimmed text of an optional note, when it has one.
fn some_text(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

/// The first words of a text for a one-line summary: up to six words,
/// then "...".
fn first_words(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= 6 {
        words.join(" ")
    } else {
        format!("{}...", words[..6].join(" "))
    }
}

/// The summary of the Material tips section: the first words of the
/// material's tips, and whether there are notes of your own.
pub fn tips_summary(snap: &Snapshot) -> String {
    let mut parts = Vec::new();
    if let Some(tips) = snap
        .material
        .as_ref()
        .and_then(|m| some_text(m.tips.as_deref()))
    {
        parts.push(first_words(tips));
    }
    let notes = snap
        .plotter
        .as_ref()
        .is_some_and(|p| some_text(p.notes.as_deref()).is_some())
        || snap
            .material
            .as_ref()
            .is_some_and(|m| some_text(m.notes.as_deref()).is_some());
    if notes {
        parts.push("your notes".into());
    }
    if parts.is_empty() {
        "none for this material".into()
    } else {
        parts.join(" · ")
    }
}

/// Material tips: the built-in tips of the material, and the person's
/// notes on the plotter and the material.
fn tips(ui: &mut Ui, snap: &Snapshot, t: &Tokens) {
    let name = snap
        .material
        .as_ref()
        .map_or(snap.job.material_id.as_str(), |m| m.name.as_str());
    let card = format!(
        "Tips that come with {name}: which side goes up and how to weed, and your own notes on the plotter and the material. Your notes go in Settings, in the presets."
    );
    blocks::section(
        ui,
        TIPS,
        TIPS_HEADING,
        &card,
        &tips_summary(snap),
        false,
        |ui| {
            let mut any = false;
            if let Some(tips) = snap
                .material
                .as_ref()
                .and_then(|m| some_text(m.tips.as_deref()))
            {
                ui.add(egui::Label::new(tips).wrap());
                any = true;
            }
            if let Some(p) = &snap.plotter {
                any |= preset_notes(ui, "Plotter notes", &p.name, p.notes.as_deref(), t);
            }
            if let Some(m) = &snap.material {
                any |= preset_notes(ui, "Material notes", &m.name, m.notes.as_deref(), t);
            }
            if !any {
                blocks::note(
                    ui,
                    "No tips for this material, and no notes yet. Add your own notes in Settings, in the presets.",
                    t.text_dim,
                );
            }
        },
        None,
    );
}

/// The person's notes on the picked plotter or material, under a small
/// heading. Nothing shows when the preset has none. Returns whether it
/// showed.
fn preset_notes(ui: &mut Ui, heading: &str, preset: &str, notes: Option<&str>, t: &Tokens) -> bool {
    let Some(notes) = notes.map(str::trim_end).filter(|n| !n.trim().is_empty()) else {
        return false;
    };
    ui.add_space(metrics::SPACE_4);
    let r = ui.label(
        RichText::new(heading)
            .font(super::super::fonts::semibold(12.0))
            .color(t.text_dim),
    );
    super::super::groups::card(
        &r,
        heading,
        &format!("Your notes on {preset}. Change them in Settings, in the presets of {preset}."),
        None,
        false,
    );
    ui.add(egui::Label::new(notes).wrap());
    true
}

/// Advanced: the blade depth, the overcut and track enhancing of the
/// material. They show here; they are changed in the material preset.
fn advanced(ui: &mut Ui, snap: &Snapshot, t: &Tokens) {
    let summary = match &snap.material {
        Some(m) => {
            let mut parts = Vec::new();
            if let Some(hint) = some_text(m.blade_depth_hint.as_deref()) {
                parts.push(hint.to_string());
            }
            parts.push(format!("overcut {} mm", m.overcut_mm));
            parts.push(format!(
                "track {}",
                if m.track_enhancing { "on" } else { "off" }
            ));
            parts.join(" · ")
        }
        None => "no material".into(),
    };
    blocks::section(
        ui,
        ADVANCED,
        "Advanced",
        "Values of the material you rarely need: the blade depth to set by hand, the overcut and track enhancing. They come from the material preset; change them in Settings, in the presets.",
        &summary,
        false,
        |ui| {
            let Some(m) = &snap.material else {
                blocks::note(ui, "Pick a material first.", t.text_dim);
                return;
            };
            let from = format!(
                "It comes from the material {}. Change it in Settings, in the presets of {}.",
                m.name, m.name
            );
            let depth = m.blade_depth_hint.as_deref().unwrap_or("No hint");
            blocks::labels(ui, &["Blade depth", "Overcut", "Track enhancing"]);
            blocks::value_row(
                ui,
                "Blade depth",
                &format!("{} {from}", Setting::BladeDepthHint.card()),
                depth,
            );
            blocks::value_row(
                ui,
                "Overcut",
                &format!("{} {from}", Setting::Overcut.card()),
                &format!("{} mm", m.overcut_mm),
            );
            blocks::value_row(
                ui,
                "Track enhancing",
                &format!("{} {from}", Setting::TrackEnhancing.card()),
                if m.track_enhancing { "On" } else { "Off" },
            );
        },
        None,
    );
}

/// The "Last time with this material" line from the cut log (the newest
/// rated cut with this material, else the newest), and a button that asks
/// how the newest cut nobody rated yet went.
fn last_time(ui: &mut Ui, snap: &Snapshot, t: &Tokens, actions: &mut Vec<Action>) {
    if let Some(e) = &snap.last_time {
        ui.add_space(metrics::SPACE_4);
        let text = outcome::last_time_text(e);
        let r = ui.add(egui::Label::new(RichText::new(&text).color(t.text_dim)).wrap());
        super::super::groups::card(
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
        super::super::groups::card(
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
