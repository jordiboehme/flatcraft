//! The Settings panel: what is cut of the selected objects, then plotter,
//! mat, material and the speed, force and passes of the job.

use egui::{Response, RichText, Ui};
use flatcraft_core::document::{CutMode, KeepMode, StrokeMode, WeedLines};
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

/// The name of a cut mode in the Cut mode combo.
pub fn mode_name(m: CutMode) -> &'static str {
    match m {
        CutMode::AllLines => "All lines",
        CutMode::ObjectOutlines => "Object outlines",
        CutMode::MergedOutline => "Merged outline",
        CutMode::VisibleOutline => "Visible outline",
    }
}

/// The cut modes in the order the Cut mode combo lists them.
pub const CUT_MODES: [CutMode; 4] = [
    CutMode::ObjectOutlines,
    CutMode::AllLines,
    CutMode::MergedOutline,
    CutMode::VisibleOutline,
];

/// The short hint of a cut mode, on its entry in the Cut mode combo: what
/// it cuts and when to pick it.
pub fn mode_hint(m: CutMode) -> &'static str {
    match m {
        CutMode::ObjectOutlines => {
            "Cuts around every shape you can see, holes included, like the middle of an O. Shapes that overlap are each cut in full. The right pick for most designs."
        }
        CutMode::AllLines => {
            "Cuts every line exactly as drawn, along its middle, whatever its fill or thickness. Use it for line art, score lines or a drawing made of single strokes."
        }
        CutMode::MergedOutline => {
            "Welds overlapping shapes into one piece: one cut around all of them, none where they overlap. Every overlapping object must use this mode and the same Keep choice. The weld belongs to the object furthest back. Use it for joined script letters or a badge made of several shapes."
        }
        CutMode::VisibleOutline => {
            "Cuts only what you can see: where an object in front covers this one, that part is not cut. An edge two shapes share is cut once. Objects added later lie in front of earlier ones, and a skipped object still covers. Use it for layered stickers and knockouts."
        }
    }
}

/// The name of a keep mode in the Keep combo.
pub fn keep_name(k: KeepMode) -> &'static str {
    match k {
        KeepMode::Pieces => "Pieces",
        KeepMode::Stencil => "Stencil",
        KeepMode::Both => "Both",
    }
}

/// The short hint of a keep mode, on its entry in the Keep combo.
pub fn keep_hint(k: KeepMode) -> &'static str {
    match k {
        KeepMode::Pieces => {
            "Keep the shapes, like stickers. The sheet around them and their holes are waste."
        }
        KeepMode::Stencil => "Keep the sheet, like a stencil. The shapes fall out and are waste.",
        KeepMode::Both => {
            "Keep both sides of every cut: the shapes and the sheet around them. The blade never starts or overshoots there; it only turns on the spot."
        }
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
    combo_hinted(
        ui,
        label,
        setting,
        text,
        choices,
        &[],
        selected,
        current,
        enabled,
        None,
    )
}

/// [`combo`] with a short hint on each entry (`hints`, in the order of
/// `choices`; an entry without one has none), and `beside` drawn right of
/// the combo (an icon button, see [`icon_button_carded`]).
#[allow(clippy::too_many_arguments)]
fn combo_hinted<T: Clone + PartialEq>(
    ui: &mut Ui,
    label: &str,
    setting: Option<Setting>,
    text: &str,
    choices: &[(T, String)],
    hints: &[&str],
    selected: Option<&T>,
    current: Option<&T>,
    enabled: bool,
    beside: Option<&mut dyn FnMut(&mut Ui)>,
) -> Option<T> {
    let mut picked = None;
    painted_label(ui, label);
    let width = ui.available_width();
    match beside {
        None => combo_box(
            ui,
            label,
            setting,
            text,
            choices,
            hints,
            selected,
            current,
            enabled,
            width,
            &mut picked,
        ),
        Some(beside) => {
            // The button first, from the right; the combo takes what the
            // button leaves, so the row is exactly as wide as the column.
            let size = egui::vec2(width, ui.spacing().interact_size.y);
            let layout = egui::Layout::right_to_left(egui::Align::Center);
            ui.allocate_ui_with_layout(size, layout, |ui| {
                beside(ui);
                let width = ui.available_width();
                combo_box(
                    ui,
                    label,
                    setting,
                    text,
                    choices,
                    hints,
                    selected,
                    current,
                    enabled,
                    width,
                    &mut picked,
                );
            });
        }
    }
    ui.end_row();
    picked
}

/// The combo box of [`combo_hinted`], `width` wide.
#[allow(clippy::too_many_arguments)]
fn combo_box<T: Clone + PartialEq>(
    ui: &mut Ui,
    label: &str,
    setting: Option<Setting>,
    text: &str,
    choices: &[(T, String)],
    hints: &[&str],
    selected: Option<&T>,
    current: Option<&T>,
    enabled: bool,
    width: f32,
    picked: &mut Option<T>,
) {
    ui.add_enabled_ui(enabled, |ui| {
        let r = egui::ComboBox::from_id_salt(label)
            .selected_text(text)
            .width(width)
            .show_ui(ui, |ui| {
                for (i, (value, text)) in choices.iter().enumerate() {
                    let mut r = ui.selectable_label(selected == Some(value), text.as_str());
                    if let Some(hint) = hints.get(i) {
                        r = r.on_hover_text(*hint);
                    }
                    if r.clicked() && current != Some(value) {
                        *picked = Some(value.clone());
                    }
                }
            });
        set_name(ui, &r.response, label);
        if let Some(setting) = setting {
            let why = (!enabled).then_some("Select an object first.");
            super::groups::card(&r.response, label, setting.card(), why, false);
        }
    });
}

/// An icon-only button named `name` with the picker's hover card, dimmed
/// with `why` when it cannot run.
fn icon_button_carded(ui: &mut Ui, icon: Icon, name: &str, why: Option<&str>) -> Response {
    let size = super::ICON_SIZE;
    let tex = super::icons::texture(ui.ctx(), icon, size);
    let image = egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(size),
    ))
    .alt_text(name);
    let r = ui.add_enabled(why.is_none(), egui::Button::image(image));
    set_name(ui, &r, name);
    super::groups::card(&r, name, super::keep_pick::HINT, why, false);
    r
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
            let modes = CUT_MODES.map(|m| (m, mode_name(m).to_string()));
            let text = match (any, mode) {
                (false, _) => "",
                (true, Some(m)) => mode_name(m),
                (true, None) => "Mixed",
            };
            if let Some(m) = combo_hinted(
                ui,
                "Cut mode",
                Some(Setting::CutMode),
                text,
                &modes,
                &CUT_MODES.map(mode_hint),
                mode.as_ref(),
                mode.as_ref(),
                any,
                None,
            ) {
                run(
                    actions,
                    "object.set_cut_options",
                    json!({ "mode": m.key() }),
                );
            }
            let keep = common(selected.iter().map(|o| o.keep));
            let kinds = [KeepMode::Pieces, KeepMode::Stencil, KeepMode::Both];
            let choices = kinds.map(|k| (k, keep_name(k).to_string()));
            let text = match (any, keep) {
                (false, _) => "",
                (true, Some(k)) => keep_name(k),
                (true, None) => "Mixed",
            };
            let ids: Vec<u64> = snap.selection.iter().copied().collect();
            let mut pick_button = |ui: &mut Ui| {
                let why = super::keep_pick::why_not(snap, &ids);
                let r = icon_button_carded(ui, Icon::Pipette, super::keep_pick::BUTTON, why);
                if r.clicked() {
                    st.keep_pick = super::keep_pick::KeepPick::start(snap, &ids);
                }
            };
            if let Some(k) = combo_hinted(
                ui,
                "Keep",
                Some(Setting::Keep),
                text,
                &choices,
                &kinds.map(keep_hint),
                keep.as_ref(),
                keep.as_ref(),
                any,
                Some(&mut pick_button),
            ) {
                run(actions, "object.set_keep", json!({ "keep": k.key() }));
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
            .on_hover_text("Linked copies share the cut options, the keep mode and the scale. Unlink a copy in the Objects panel to change it alone.");
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
            if let Some(id) = super::materials::picker_row(ui, snap, t) {
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
                "The order this document cuts in. Quality cleans up the corners, Fast keeps the travel short, and Outlines last cuts every outline after the holes and inner pieces. New documents take it from Settings, Cutting.",
                None,
                false,
            );
            ui.end_row();
        });

    super::materials::skipped_note(ui, snap, t);
    super::materials::test_cut_hint(ui, snap, t, actions);
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
    weeding(ui, snap, st.sheet_kept, t, actions);
    if let Some(hint) = &material.blade_depth_hint {
        ui.add_space(t.spacing / 2.0);
        ui.label(RichText::new(format!("Blade depth: {hint}")).color(t.text_dim));
    }
    material_tips(ui, material, t);
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

/// The name of a weed lines choice.
pub fn weed_lines_name(l: WeedLines) -> &'static str {
    match l {
        WeedLines::Off => "Off",
        WeedLines::Horizontal => "Horizontal",
        WeedLines::Vertical => "Vertical",
        WeedLines::Both => "Both",
    }
}

/// The `job.set_weeding` value of a weed lines choice.
fn weed_lines_key(l: WeedLines) -> &'static str {
    match l {
        WeedLines::Off => "off",
        WeedLines::Horizontal => "horizontal",
        WeedLines::Vertical => "vertical",
        WeedLines::Both => "both",
    }
}

/// Why per object is dimmed.
pub const WEED_BOX_OFF: &str = "Turn on Weed box first.";
/// Why the box margin is dimmed.
pub const WEEDING_OFF: &str = "Turn on Weed box or Weed lines first.";
/// Why the weed line spacing is dimmed.
pub const WEED_LINES_OFF: &str = "Pick a direction for Weed lines first.";

/// A checkbox row of the weeding grid with its card. Returns the new value
/// when it was clicked.
fn weed_check(
    ui: &mut Ui,
    label: &str,
    on: bool,
    setting: Setting,
    enabled: bool,
    why: &str,
) -> Option<bool> {
    painted_label(ui, label);
    let mut v = on;
    let r = ui.add_enabled(enabled, egui::Checkbox::without_text(&mut v));
    set_name(ui, &r, label);
    super::groups::card(&r, label, setting.card(), (!enabled).then_some(why), false);
    ui.end_row();
    r.clicked().then_some(v)
}

/// A millimetre field of the weeding grid with its card. Returns the typed
/// value when it changed.
#[allow(clippy::too_many_arguments)]
fn weed_mm(
    ui: &mut Ui,
    label: &str,
    value: f64,
    (lo, hi): (f64, f64),
    setting: Setting,
    enabled: bool,
    why: &str,
) -> Option<f64> {
    painted_label(ui, label);
    let mut v = value;
    let spec = Number::new(Field::new(Kind::Length).range(lo, hi))
        .decimals(0, 2)
        .unit("mm");
    let (r, typed) = calc::number(ui, ("flatcraft-weed", label), enabled, &mut v, &spec);
    set_name(ui, &r, label);
    let text = format!(
        "{} From {lo} to {hi} mm. {}",
        setting.card(),
        calc::hint(Kind::Length)
    );
    super::groups::card(&r, label, &text, (!enabled).then_some(why), false);
    ui.end_row();
    (typed && v != value).then_some(v)
}

/// The weed box and weed lines of the job (`job.set_weeding`).
fn weeding(ui: &mut Ui, snap: &Snapshot, sheet_kept: bool, t: &Tokens, actions: &mut Vec<Action>) {
    use flatcraft_core::limits::{
        MAX_WEED_MARGIN_MM, MAX_WEED_SPACING_MM, MIN_WEED_MARGIN_MM, MIN_WEED_SPACING_MM,
    };
    let doc_id = snap.doc_id;
    let mut set = |params: Value| actions.push(Action::run(doc_id, "job.set_weeding", params));
    let w = snap.job.weed;
    ui.add_space(t.spacing / 2.0);
    let r = ui.label(RichText::new("Weeding").font(fonts::semibold(13.0)));
    super::groups::card(&r, "Weeding", WEEDING_HINT, None, false);
    // The settings stay usable: they apply again once no object keeps
    // the sheet.
    if sheet_kept {
        ui.label(RichText::new(SHEET_KEPT_NO_WEEDING).color(t.text_dim));
    }
    egui::Grid::new("flatcraft-weeding")
        .num_columns(2)
        .spacing([t.spacing, t.spacing * 0.75])
        .show(ui, |ui| {
            if let Some(v) = weed_check(ui, "Weed box", w.weed_box, Setting::WeedBox, true, "") {
                set(json!({ "box": v }));
            }
            if let Some(v) = weed_mm(
                ui,
                "Box margin",
                w.margin_mm,
                (MIN_WEED_MARGIN_MM, MAX_WEED_MARGIN_MM),
                Setting::WeedMargin,
                w.any(),
                WEEDING_OFF,
            ) {
                set(json!({ "margin_mm": v }));
            }
            if let Some(v) = weed_check(
                ui,
                "One box per object",
                w.per_object,
                Setting::WeedPerObject,
                w.weed_box,
                WEED_BOX_OFF,
            ) {
                set(json!({ "per_object": v }));
            }
            let choices = [
                WeedLines::Off,
                WeedLines::Horizontal,
                WeedLines::Vertical,
                WeedLines::Both,
            ]
            .map(|l| (l, weed_lines_name(l).to_string()));
            if let Some(l) = combo(
                ui,
                "Weed lines",
                Some(Setting::WeedLines),
                weed_lines_name(w.lines),
                &choices,
                Some(&w.lines),
                Some(&w.lines),
                true,
            ) {
                set(json!({ "lines": weed_lines_key(l) }));
            }
            if let Some(v) = weed_mm(
                ui,
                "Line spacing",
                w.spacing_mm,
                (MIN_WEED_SPACING_MM, MAX_WEED_SPACING_MM),
                Setting::WeedSpacing,
                w.lines != WeedLines::Off,
                WEED_LINES_OFF,
            ) {
                set(json!({ "spacing_mm": v }));
            }
        });
}

/// Why weeding cuts nothing while an object keeps the shared sheet: the
/// Weeding section, the Path legend's weed row and its weed path count.
pub const SHEET_KEPT_NO_WEEDING: &str =
    "An object set to Stencil or Both keeps the sheet on this mat, so there is nothing to weed.";

/// The guess of [`super::cut::sheet_kept_now`] without a current plan:
/// an object that is cut keeps the shared sheet (Keep is Stencil or
/// Both). The plan counts only objects with a closed cut.
pub fn sheet_kept(snap: &Snapshot) -> bool {
    snap.objects
        .iter()
        .any(|o| o.visible && o.cut_enabled && o.keep != KeepMode::Pieces)
}

/// The card of the Weeding heading.
pub const WEEDING_HINT: &str = "Weeding means peeling away the vinyl you do not need after the cut, so only your design stays on the backing. A weed box and weed lines are extra cuts through that waste only: it comes off in neat pieces instead of tearing. They are cut after the design and show dashed in the Path view.";

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

/// The heading of a material's built-in tips in the Job card.
pub const TIPS_HEADING: &str = "Material tips";

/// The built-in tips of the picked material, under a small heading.
/// Nothing shows when it has none.
fn material_tips(ui: &mut Ui, material: &flatcraft_device::MaterialProfile, t: &Tokens) {
    let Some(tips) = material
        .tips
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    ui.add_space(t.spacing / 2.0);
    let r = ui.label(
        RichText::new(TIPS_HEADING)
            .font(fonts::semibold(12.0))
            .color(t.text_dim),
    );
    super::groups::card(
        &r,
        TIPS_HEADING,
        &format!(
            "Tips that come with {}: which side goes up and how to weed. Your own notes go in Settings, in the presets.",
            material.name
        ),
        None,
        false,
    );
    ui.add(egui::Label::new(tips).wrap());
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
