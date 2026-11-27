//! The cut options of the selected objects (cut mode, keep and strokes),
//! shown as the inspector's interim Selection section until the object
//! inspector takes them over, and the texts and helpers the Job panel
//! (`inspector::job`) shares with the rest of the window.

use egui::{Response, RichText, Ui};
use flatcraft_core::document::{CutMode, KeepMode, StrokeMode, WeedLines};
use serde_json::{Value, json};

use super::Action;
use super::icons::Icon;
use super::prepare::{PrepareState, Snapshot};
use super::theme::Tokens;
use super::tools::Setting;

/// The speed hint after a typed speed above the plotter's maximum.
pub const ELEVEN: &str = "These go to eleven.";

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
            "Cuts only what you can see: where an object in front covers this one, that part is not cut. An edge two shapes share is cut once. Objects lower in the Objects list lie in front (Bring to front and Send to back in Arrange change that), and a skipped object still covers. Use it for layered stickers and knockouts."
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

/// The summary of the Selection section: how many are selected and their
/// cut mode and keep choice ("Mixed" when they differ).
pub fn selection_summary(snap: &Snapshot) -> String {
    let selected: Vec<_> = snap.selected().collect();
    let mode = common(selected.iter().map(|o| o.cut.mode)).map_or("Mixed", mode_name);
    let keep = common(selected.iter().map(|o| o.keep)).map_or("Mixed", keep_name);
    format!("{} selected · {mode} · {keep}", selected.len())
}

/// The interim Selection section: cut mode, keep (with the picker) and
/// strokes of the selected objects.
pub fn selection(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let summary = selection_summary(snap);
    super::inspector::blocks::section(
        ui,
        super::inspector::SELECTION,
        "Selection",
        "What is cut of the selected objects, and which side of the cut you keep. Each change is one step to undo.",
        &summary,
        true,
        |ui| selection_body(ui, st, snap, t, actions),
        None,
    );
}

fn selection_body(
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
    let selected: Vec<_> = snap.selected().collect();
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

/// Why per object is dimmed.
pub const WEED_BOX_OFF: &str = "Turn on Weed box first.";
/// Why the box margin is dimmed.
pub const WEEDING_OFF: &str = "Turn on Weed box or Weed lines first.";
/// Why the weed line spacing is dimmed.
pub const WEED_LINES_OFF: &str = "Pick a direction for Weed lines first.";

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

/// The title of the Job panel's Material tips section.
pub const TIPS_HEADING: &str = "Material tips";
