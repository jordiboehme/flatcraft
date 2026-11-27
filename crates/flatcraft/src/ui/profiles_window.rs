//! The preset pages of the Settings window: plotter and material presets
//! and the mats of each plotter (see [`super::settings_window`], which
//! draws the frame around them).
//!
//! System presets are read-only; "Save as user preset..." makes a user
//! preset that inherits from them. The widgets only edit a draft in
//! [`ProfilesState`] and push [`Request`]s; [`apply`] writes presets under
//! the session lock, one short step each, so the MCP thread never sees half
//! a change. The Plotters and Mats pages edit the same plotter draft, and
//! "Save preset" stores both.

use super::tools::Setting;
use crate::ui::metrics;
use egui::{Color32, Response, RichText, Sense, Ui, vec2};
use flatcraft_core::numexpr::{Field, Kind as NumKind};
use flatcraft_device::profile::MAX_SIZE_MM;
use flatcraft_device::{
    CornerModeSpec, GpglDialect, Grid, KnifeCompensation, MatTack, MaterialGroup, MaterialOverride,
    MaterialProfile, Media, MediaField, MediaKind, PlotterProfile, PresetInfo, Protocol,
    RingDirection, TransportSpec,
};

use super::calc::{self, Number};
use super::settings::set_name;
use super::theme::Tokens;
use super::{Action, FlatCraftApp, Toast, canvas, files, fonts, lock};

/// Help text under the Theme combo.
#[cfg(target_os = "macos")]
pub const THEME_HELP: &str = "System follows your Mac's appearance.";
/// Help text under the Theme combo.
#[cfg(not(target_os = "macos"))]
pub const THEME_HELP: &str = "System follows your computer's appearance.";
/// Help text of the auto-reload switch.
pub const AUTO_RELOAD_HELP: &str = "When an imported file changes on disk, FlatCraft reloads it and keeps your placement and cut settings.";

/// Why a mat cannot be hidden or deleted.
pub const MAT_IN_USE: &str = "This mat is in use. Pick another mat in the job first.";

/// Why a field of a system preset is dimmed, on its hover card.
pub const SYSTEM_READ_ONLY: &str =
    "This is a system preset, so it cannot be changed. Save a user preset to change it.";

/// Why Save preset or Save as is dimmed while a field needs a fix.
pub const FIX_PROBLEMS_FIRST: &str =
    "A field needs a fix first: the red lines above say which one and why.";

/// Why Save preset is dimmed when nothing changed.
pub const NOTHING_TO_SAVE: &str = "Nothing changed yet, so there is nothing to save.";

/// Why Save preset is dimmed for a preset without a parent.
pub const NO_PARENT: &str = "This preset inherits from nothing, so edit it in its file. Save as user preset makes an editable copy.";

/// Why Copy mat is dimmed with no mat picked.
pub const PICK_A_MAT: &str = "No mat is picked. Pick one in the list first.";

/// Why Copy mat is dimmed while the picked mat is hidden.
pub const HIDDEN_NO_COPY: &str = "Hidden mats cannot be copied. Turn off Hidden first.";

/// Why a plotter override value is dimmed, on its hover card.
pub const OVERRIDE_OFF: &str = "The box on its left is off, so the material's own value applies. Tick the box to set a value for this plotter.";

/// Warning under "Allow agents to start cuts".
pub const AGENT_CUTS_WARNING: &str = "With this on, an agent can start a USB cut without asking you first. Turn it on only when the mat is loaded and you trust the agent. File Output never needs it.";

/// Plotter or material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Plotter,
    Material,
}

/// The preset being edited: the stored (resolved) profile and the edits.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft<T> {
    pub id: String,
    pub system: bool,
    /// The preset this one inherits from; a user preset saves against it.
    /// A user preset without `inherits` is read-only here: the store saves
    /// a user preset only as the difference to a parent, so Save stays
    /// disabled and the window says to edit its file instead.
    pub parent: Option<String>,
    pub saved: T,
    pub edit: T,
    /// `ProfileStore::modified_plotter_fields` or `modified_material_fields`
    /// of the stored preset.
    pub modified: Vec<String>,
}

impl<T: PartialEq> Draft<T> {
    pub fn changed(&self) -> bool {
        self.saved != self.edit
    }
}

/// The open "Save as user preset" dialog.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SaveAs {
    pub kind: Option<Kind>,
    pub name: String,
    pub error: Option<String>,
}

/// What the window shows besides the session.
#[derive(Debug, Default)]
pub struct ProfilesState {
    pub plotter: Option<Draft<PlotterProfile>>,
    /// The mats of the parent preset: the built-in mats of the selected plotter.
    pub parent_media: Vec<Media>,
    /// The selected mat id.
    pub mat: Option<String>,
    pub material: Option<Draft<MaterialProfile>>,
    /// Why the selected preset could not be loaded.
    pub load_error: Option<(Kind, String, String)>,
    /// Load this plotter at the start of the next frame.
    pub want_plotter: Option<String>,
    pub want_material: Option<String>,
    pub save_as: Option<SaveAs>,
    /// The preset waiting for "Yes, delete".
    pub confirm_delete: Option<(Kind, String, String)>,
    /// Waiting for "Discard your changes?".
    pub leave: Option<Leave>,
    /// "Discard" answered a close: the Settings window closes this frame.
    pub(crate) closing: bool,
    /// The row a search in the Settings window jumped to.
    pub(crate) highlight: Highlight,
    /// A message for a toast, from a widget that cannot reach the app.
    notice: Option<String>,
    /// "Save notes" of a system preset was clicked in a form, which has no
    /// way to the actions; the page sends it.
    notes_request: Option<Request>,
    /// The text of the hex colour field while it has the keyboard.
    hex: String,
    hex_editing: bool,
}

/// The row of a preset form that a search jumped to: its label, and
/// whether the form still has to scroll it into view.
#[derive(Debug, Default)]
pub(crate) struct Highlight {
    pub(crate) label: Option<&'static str>,
    pub(crate) scroll: std::cell::Cell<bool>,
}

/// What waits for "Discard your changes?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leave {
    /// Show this plotter preset.
    Plotter(String),
    /// Show this material preset.
    Material(String),
    /// Close the Settings window.
    Close,
}

/// What the window asks [`apply`] to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Store a user plotter preset that inherits from `parent`. `new` is
    /// true for "Save as user preset": an existing id is refused then.
    SavePlotter {
        profile: Box<PlotterProfile>,
        parent: String,
        new: bool,
    },
    SaveMaterial {
        profile: Box<MaterialProfile>,
        parent: String,
        new: bool,
    },
    Delete(Kind, String),
    /// Store the notes of a system preset beside it (the preset itself
    /// stays as it ships), or of a user preset without a parent in its
    /// file. `name` is for the toast.
    SaveNotes {
        kind: Kind,
        id: String,
        name: String,
        notes: String,
    },
    /// Ask for the File Output folder; the answer goes into the Settings
    /// window's staged preferences.
    ChooseOutputFolder,
    /// Read the user preset files again (after fixing a broken one).
    ReloadPresets,
}

/// What the window reads from the session, copied under one lock.
pub(crate) struct View {
    pub(crate) plotters: Vec<PresetInfo>,
    pub(crate) materials: Vec<PresetInfo>,
    /// (plotter id, mat id) pairs open jobs and new documents use.
    mats_in_use: Vec<(String, String)>,
    /// User preset files the store could not read.
    diagnostics: Vec<flatcraft_device::PresetDiagnostic>,
    /// The Settings default mat colours of the light and the dark theme:
    /// what a mat without a colour of its own is painted in.
    mat_default: ([u8; 3], [u8; 3]),
}

impl View {
    /// Ids of unreadable preset files of `kind` (their file stems).
    fn broken_ids(&self, kind: Kind) -> impl Iterator<Item = &str> {
        let folder = match kind {
            Kind::Plotter => "plotters",
            Kind::Material => "materials",
        };
        self.diagnostics.iter().filter_map(move |d| {
            let parent = d.path.parent()?.file_name()?.to_str()?;
            (parent == folder)
                .then(|| d.path.file_stem()?.to_str())
                .flatten()
        })
    }
}

/// The (plotter id, mat id) pairs the open documents use, plus the mat new
/// documents start on with the default device.
fn mats_in_use(s: &flatcraft_engine::session::Session) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = s
        .docs
        .iter()
        .map(|d| {
            (
                d.document.job.plotter_id.clone(),
                d.document.job.media_id.clone(),
            )
        })
        .collect();
    out.push((
        s.prefs.default_plotter.clone(),
        s.default_media(&s.prefs.default_plotter),
    ));
    out
}

/// The prefix of every id "Save as user preset" makes. No built-in preset
/// id starts with it, so a user preset never collides with one that ships
/// later.
pub const USER_ID_PREFIX: &str = "user-";

/// The id of a new user preset named `name`: [`USER_ID_PREFIX`] plus
/// [`slug`], with "-2", "-3" and so on when an id in `taken` has it.
/// Empty when the name has no letter or digit.
pub fn new_preset_id(name: &str, taken: &[&str]) -> String {
    let s = slug(name);
    if s.is_empty() {
        return s;
    }
    fresh_mat_id(&format!("{USER_ID_PREFIX}{s}"), taken)
}

/// A preset id from a name: lowercase letters and digits, words joined by
/// single hyphens ("My Cameo 2" -> "my-cameo-2").
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// `#RRGGBB` of a colour.
pub fn hex(c: [u8; 3]) -> String {
    flatcraft_device::hex_colour(c)
}

/// Why the hex field of a mat that follows the default is dimmed.
const MAT_COLOUR_DEFAULT_WHY: &str = "This mat follows the default colour from Settings. Pick Custom to give it a colour of its own.";

/// A colour from `#RRGGBB` (the `#` may be left out).
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim();
    let h = h.strip_prefix('#').unwrap_or(h);
    flatcraft_device::parse_hex_colour(&format!("#{h}")).ok()
}

/// The problem of a mat for a person ("The grid size must be above 0."),
/// and the field whose row shows it.
pub fn mat_problem(m: &Media) -> Option<(MediaField, String)> {
    let e = m.validate().err()?;
    Some((e.field, sentence(&e.reason)))
}

/// A check result as a sentence: capital first letter, full stop.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

/// Problems of a plotter draft that block Save.
pub fn plotter_problems(p: &PlotterProfile) -> Vec<String> {
    let mut out = Vec::new();
    let pos = |v: f64| v.is_finite() && v > 0.0;
    if p.name.trim().is_empty() {
        out.push("Give the preset a name.".into());
    }
    if !pos(p.units_per_mm) {
        out.push("Units per mm must be above 0.".into());
    }
    if !pos(p.scale_x) || !pos(p.scale_y) {
        out.push("The scale must be above 0.".into());
    }
    if !(p.blade_offset_mm.is_finite() && p.blade_offset_mm >= 0.0) {
        out.push("The blade offset must not be negative.".into());
    }
    if let Err(e) = p.validate_values() {
        out.push(sentence(&e));
    }
    if p.speed_range[0] > p.speed_range[1] || p.speed_range[0] == 0 {
        out.push("The speed range must start at 1 or more and not run backwards.".into());
    }
    if p.force_range[0] > p.force_range[1] || p.force_range[0] == 0 {
        out.push("The force range must start at 1 or more and not run backwards.".into());
    }
    if p.passes_range[0] > p.passes_range[1] || p.passes_range[0] == 0 {
        out.push("The passes range must start at 1 or more and not run backwards.".into());
    }
    if let TransportSpec::File { extension, .. } = &p.transport
        && extension.trim().is_empty()
    {
        out.push("File Output needs a file extension.".into());
    }
    if p.media.is_empty() {
        out.push("Keep at least one mat.".into());
    }
    if p.media.iter().any(|m| m.validate().is_err()) {
        out.push("A mat has a problem. Check the mats marked \"needs a fix\".".into());
    } else if let Err(e) = flatcraft_device::store::validate_media(&p.id, p) {
        out.push(e.to_string());
    }
    out
}

/// Problems of a material draft that block Save.
pub fn material_problems(m: &MaterialProfile) -> Vec<String> {
    let mut out = Vec::new();
    if m.name.trim().is_empty() {
        out.push("Give the preset a name.".into());
    }
    if m.speed == 0 || m.force == 0 {
        out.push("Speed and force must be 1 or more.".into());
    }
    if m.passes == 0 {
        out.push("Passes must be 1 or more.".into());
    }
    if let Err(e) = m.validate_values() {
        out.push(sentence(&e));
    }
    for o in m.overrides.values() {
        if o.speed == Some(0) || o.force == Some(0) || o.passes == Some(0) {
            out.push("Override values must be 1 or more.".into());
            break;
        }
    }
    out
}

/// The unreadable preset files and "Reload presets", over a preset page.
pub(crate) fn diagnostics(ui: &mut Ui, view: &View, t: &Tokens, actions: &mut Vec<Action>) {
    for d in &view.diagnostics {
        ui.label(
            RichText::new(format!("This preset file can't be read: {}", d.reason)).color(t.danger),
        )
        .on_hover_text(d.path.display().to_string());
    }
    if !view.diagnostics.is_empty()
        && ui
            .button("Reload presets")
            .on_hover_ui(|ui| {
                super::groups::card_ui(
                    ui,
                    "Reload presets",
                    "Fixed the file? This reads the preset folder again.",
                    None,
                )
            })
            .clicked()
    {
        actions.push(Action::Profiles(Request::ReloadPresets));
    }
}

/// Whether a preset draft holds edits that are not saved.
pub(crate) fn unsaved(st: &ProfilesState) -> bool {
    st.plotter.as_ref().is_some_and(Draft::changed)
        || st.material.as_ref().is_some_and(Draft::changed)
}

/// The Save as and Delete questions over the window.
pub(crate) fn dialogs(
    ctx: &egui::Context,
    st: &mut ProfilesState,
    view: &View,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    save_as_dialog(ctx, st, view, t, actions);
    delete_dialog(ctx, st, t, actions);
}

/// Forgets the open questions when the window closes.
pub(crate) fn closed(st: &mut ProfilesState) {
    st.closing = false;
    st.leave = None;
    st.save_as = None;
    st.confirm_delete = None;
    st.highlight = Highlight::default();
}

/// Takes the toast a widget asked for.
pub(crate) fn take_notice(st: &mut ProfilesState) -> Option<String> {
    st.notice.take()
}

/// "Discard your changes?" at the top of the window, for a preset switch
/// or Close while a draft has unsaved edits.
pub(crate) fn discard_banner(ui: &mut Ui, st: &mut ProfilesState, t: &Tokens) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, t.accent_cut))
        .corner_radius(t.radius)
        .inner_margin(egui::Margin::same(metrics::SPACE_8 as i8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Discard your changes?").color(t.warn_text));
                let r = ui.button("Discard");
                action_card(
                    &r,
                    "Discard",
                    "Drops the changes you did not save and goes on: to the preset you picked, or the window closes.",
                    None,
                );
                if r.clicked() {
                    match st.leave.take() {
                        Some(Leave::Plotter(id)) => st.want_plotter = Some(id),
                        Some(Leave::Material(id)) => st.want_material = Some(id),
                        Some(Leave::Close) => {
                            // Reload the stored presets, so the next open
                            // shows what is on disk.
                            st.want_plotter = st.plotter.as_ref().map(|d| d.id.clone());
                            st.want_material = st.material.as_ref().map(|d| d.id.clone());
                            for d in st.plotter.iter_mut() {
                                d.edit = d.saved.clone();
                            }
                            for d in st.material.iter_mut() {
                                d.edit = d.saved.clone();
                            }
                            st.closing = true;
                        }
                        None => {}
                    }
                }
                let r = ui.button("Keep editing");
                action_card(
                    &r,
                    "Keep editing",
                    "Stays on this preset with your changes, so you can save them first.",
                    None,
                );
                if r.clicked() {
                    st.leave = None;
                }
            });
        });
    ui.add_space(metrics::SPACE_4);
}

/// Copies the lists and preferences, and loads a newly selected preset,
/// under one lock.
pub(crate) fn load(app: &mut FlatCraftApp) -> View {
    let st = &mut app.ui.profiles;
    let s = lock(&app.session);
    let job = &s.doc().document.job;
    if st.plotter.is_none() && st.want_plotter.is_none() {
        st.want_plotter = Some(job.plotter_id.clone());
    }
    if st.material.is_none() && st.want_material.is_none() {
        st.want_material = Some(job.material_id.clone());
    }
    let plotters = s.profiles.plotters();
    let materials = s.profiles.materials();
    if let Some(id) = st.want_plotter.take() {
        let info = plotters.iter().find(|p| p.id == id);
        match (info, s.profiles.plotter_unchecked(&id)) {
            (Some(info), Ok(p)) => {
                let parent_media = match &info.inherits {
                    Some(parent) => s
                        .profiles
                        .plotter_unchecked(parent)
                        .map(|p| p.media)
                        .unwrap_or_default(),
                    None => p.media.clone(),
                };
                if !st
                    .mat
                    .as_ref()
                    .is_some_and(|m| p.media.iter().chain(&parent_media).any(|x| &x.id == m))
                {
                    st.mat = p.media.first().map(|m| m.id.clone());
                }
                st.parent_media = parent_media;
                st.plotter = Some(Draft {
                    id: id.clone(),
                    system: info.system,
                    parent: info.inherits.clone(),
                    saved: p.clone(),
                    edit: p,
                    modified: s.profiles.modified_plotter_fields(&id),
                });
                st.load_error = None;
            }
            (_, Err(e)) => {
                st.plotter = None;
                st.load_error = Some((Kind::Plotter, id, e.to_string()));
            }
            (None, Ok(_)) => st.plotter = None,
        }
    }
    if let Some(id) = st.want_material.take() {
        let info = materials.iter().find(|p| p.id == id);
        match (info, s.profiles.material_unchecked(&id)) {
            (Some(info), Ok(m)) => {
                st.material = Some(Draft {
                    id: id.clone(),
                    system: info.system,
                    parent: info.inherits.clone(),
                    saved: m.clone(),
                    edit: m,
                    modified: s.profiles.modified_material_fields(&id),
                });
                st.load_error = None;
            }
            (_, Err(e)) => {
                st.material = None;
                st.load_error = Some((Kind::Material, id, e.to_string()));
            }
            (None, Ok(_)) => st.material = None,
        }
    }
    View {
        mats_in_use: mats_in_use(&s),
        plotters,
        materials,
        diagnostics: s.profiles.diagnostics().to_vec(),
        mat_default: (s.prefs.mat_color_light, s.prefs.mat_color_dark),
    }
}

/// The name of the selected preset and where it comes from. The name is
/// painted, so the preset list keeps the only node with that name.
fn header(ui: &mut Ui, name: &str, parent: Option<&str>, system: bool, t: &Tokens) {
    let galley = ui
        .painter()
        .layout_no_wrap(name.to_string(), metrics::heading(), t.text);
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.left_top(), galley, t.text);
    let origin = match (parent, system) {
        (Some(p), true) => format!("System preset, based on {p}"),
        (None, true) => "System preset".to_string(),
        (Some(p), false) => format!("User preset, inherits from {p}"),
        (None, false) => "User preset".to_string(),
    };
    ui.label(RichText::new(origin).color(t.text_dim));
    ui.add_space(metrics::SPACE_4);
}

/// Text painted without an accessibility node, so the widget next to it
/// is the only node with that name. `dot` marks a modified field.
fn paint_text(ui: &mut Ui, text: &str, color: Color32, dot: Option<Color32>) -> egui::Rect {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let extra = if dot.is_some() {
        metrics::DOT_SLOT
    } else {
        0.0
    };
    let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(extra, 0.0), Sense::hover());
    ui.painter().galley(rect.left_top(), galley, color);
    if let Some(c) = dot {
        ui.painter()
            .circle_filled(egui::pos2(rect.right() - 4.0, rect.center().y), 3.5, c);
    }
    rect
}

/// Rows of a form with "modified" dots from the modified fields of the
/// stored preset.
struct Form<'a> {
    modified: &'a [String],
    t: &'a Tokens,
    /// The row a search jumped to.
    highlight: &'a Highlight,
    /// A system preset: every field is dimmed and its card says why.
    system: bool,
}

impl Form<'_> {
    fn is_modified(&self, key: &str) -> bool {
        self.modified
            .iter()
            .any(|m| m == key || m.strip_prefix(key).is_some_and(|r| r.starts_with('.')))
    }

    fn name(&self, label: &str, key: &str) -> String {
        if self.is_modified(key) {
            format!("{label} (modified)")
        } else {
            label.to_string()
        }
    }

    /// A label cell, the widget, its accessible name, end of row, with the
    /// hover card of `setting`. Every row has one.
    fn row(
        &self,
        ui: &mut Ui,
        label: &str,
        key: &str,
        setting: Setting,
        add: impl FnOnce(&mut Ui) -> Response,
    ) {
        self.row_dimmed(ui, label, key, setting, None, add);
    }

    /// Like [`Self::row`], for a widget that `reason` says is dimmed.
    fn row_dimmed(
        &self,
        ui: &mut Ui,
        label: &str,
        key: &str,
        setting: Setting,
        reason: Option<&str>,
        add: impl FnOnce(&mut Ui) -> Response,
    ) {
        let dot = self.is_modified(key).then_some(self.t.accent_prepare);
        let at = paint_text(ui, label, ui.visuals().text_color(), dot);
        let r = add(ui);
        set_name(ui, &r, &self.name(label, key));
        self.card(&r, label, setting, reason);
        self.mark(ui, label, at.union(r.rect));
        ui.end_row();
    }

    /// The hover card of an action on `r`. On a system preset it says why
    /// the action is dimmed; otherwise `reason` does, when there is one.
    fn card_text(&self, r: &Response, title: &str, text: &str, reason: Option<&str>) {
        let reason = if self.system {
            Some(SYSTEM_READ_ONLY)
        } else {
            reason
        };
        action_card(r, title, text, reason);
    }

    /// The hover card of `setting` on `r`. On a system preset it says why
    /// the field is dimmed; otherwise `reason` does, when there is one.
    fn card(&self, r: &Response, label: &str, setting: Setting, reason: Option<&str>) {
        let reason = if self.system {
            Some(SYSTEM_READ_ONLY)
        } else {
            reason
        };
        let text = match setting.number_kind() {
            Some(kind) => calc::with_hint(setting.card(), kind),
            None => setting.card().to_string(),
        };
        super::groups::card(r, label, &text, reason, false);
    }

    /// Outlines the row a search jumped to and scrolls it into view once.
    fn mark(&self, ui: &Ui, label: &str, rect: egui::Rect) {
        if self.highlight.label != Some(label) {
            return;
        }
        super::settings_window::paint_highlight(ui, rect, self.t);
        if self.highlight.scroll.replace(false) {
            ui.scroll_to_rect(rect, Some(egui::Align::Center));
        }
    }

    /// A checkbox in the widget column, with the hover card of `setting`.
    /// Every checkbox has one.
    fn check(&self, ui: &mut Ui, label: &str, key: &str, setting: Setting, v: &mut bool) {
        let dot = self.is_modified(key).then_some(self.t.accent_prepare);
        let at = paint_text(ui, "", ui.visuals().text_color(), dot);
        let r = ui.checkbox(v, label);
        set_name(ui, &r, &self.name(label, key));
        self.card(&r, label, setting, None);
        self.mark(ui, label, at.union(r.rect));
        ui.end_row();
    }
}

/// The hover card of an action button: what it does, and why it is dimmed
/// when `reason` says so.
fn action_card(r: &Response, title: &str, text: &str, reason: Option<&str>) {
    super::groups::card(r, title, text, reason, false);
}

/// A decimal field that takes a calculation in `kind`'s units (see
/// [`calc`]). The profile checks the ranges when it is saved.
fn num(ui: &mut Ui, on: bool, v: &mut f64, decimals: usize, kind: NumKind) -> Response {
    let salt = ui.next_auto_id();
    let spec = Number::new(Field::new(kind)).decimals(0, decimals);
    calc::number(ui, salt, on, v, &spec).0
}

/// A whole-number field that takes a calculation; it keeps to its type's
/// limits.
fn int<T: egui::emath::Numeric>(ui: &mut Ui, on: bool, v: &mut T) -> Response {
    let salt = ui.next_auto_id();
    calc::number(ui, salt, on, v, &Number::new(Field::plain())).0
}

/// A USB id: four hex digits, no calculation.
fn hex_id(ui: &mut Ui, v: &mut u16) -> Response {
    let salt = ui.next_auto_id();
    let spec = Number::hex(4).width(48.0);
    calc::number(ui, salt, true, v, &spec).0
}

/// A combo box of `choices`; sets `value` to the picked one.
fn combo<T: PartialEq + Clone>(
    ui: &mut Ui,
    salt: &str,
    value: &mut T,
    choices: &[(T, &str)],
) -> Response {
    let text = choices
        .iter()
        .find(|(v, _)| v == value)
        .map_or("", |(_, s)| *s);
    egui::ComboBox::from_id_salt(("flatcraft-profiles", salt))
        .selected_text(text)
        .width(180.0)
        .show_ui(ui, |ui| {
            for (v, label) in choices {
                ui.selectable_value(value, v.clone(), *label);
            }
        })
        .response
}

/// Two columns: the preset list and the editor.
fn preset_list(
    ui: &mut Ui,
    presets: &[PresetInfo],
    selected: Option<&str>,
    t: &Tokens,
) -> Option<String> {
    let mut picked = None;
    for (system, heading) in [(true, "System"), (false, "User")] {
        ui.label(
            RichText::new(heading)
                .font(metrics::section_title())
                .color(t.text_dim),
        );
        let mut any = false;
        for p in presets.iter().filter(|p| p.system == system) {
            any = true;
            let on = selected == Some(p.id.as_str());
            let r = ui.add(egui::Button::selectable(on, &p.name));
            action_card(
                &r,
                &p.name,
                "Shows this preset. If the open one has changes you did not save, FlatCraft asks first.",
                None,
            );
            if r.clicked() && !on {
                picked = Some(p.id.clone());
            }
        }
        if !any {
            ui.label(RichText::new("None yet. Save one from a system preset.").color(t.text_dim));
        }
        ui.add_space(metrics::SPACE_8);
    }
    picked
}

/// Save, Save as, Delete and Revert under an editor.
#[allow(clippy::too_many_arguments)]
fn preset_buttons(
    ui: &mut Ui,
    kind: Kind,
    id: &str,
    name: &str,
    system: bool,
    has_parent: bool,
    changed: bool,
    problems: &[String],
    t: &Tokens,
    save: impl FnOnce() -> Request,
    st_save_as: &mut Option<SaveAs>,
    st_delete: &mut Option<(Kind, String, String)>,
    revert: &mut bool,
    actions: &mut Vec<Action>,
) {
    for p in problems {
        ui.label(RichText::new(p).color(t.danger));
    }
    ui.horizontal(|ui| {
        if !system {
            // A value still being typed counts as a change: the click
            // waits for it (see `calc::click`).
            let typing = calc::editing(ui.ctx()).is_some();
            let ok = problems.is_empty() && (changed || typing) && has_parent;
            let r = ui.add_enabled(ok, egui::Button::new("Save preset"));
            let why = if !has_parent {
                Some(NO_PARENT)
            } else if !problems.is_empty() {
                Some(FIX_PROBLEMS_FIRST)
            } else if !changed {
                Some(NOTHING_TO_SAVE)
            } else {
                None
            };
            action_card(
                &r,
                "Save preset",
                "Writes your changes into this user preset's file.",
                why,
            );
            if calc::click(ui, &r) {
                actions.push(Action::Profiles(save()));
            }
        }
        // Save as writes too, so it waits for valid fields like Save.
        let r = ui.add_enabled(
            problems.is_empty(),
            egui::Button::new("Save as user preset..."),
        );
        action_card(
            &r,
            "Save as user preset",
            "Saves this preset under a new name as your own user preset. It inherits from this one, so everything you leave alone keeps up with it.",
            (!problems.is_empty()).then_some(FIX_PROBLEMS_FIRST),
        );
        if calc::click(ui, &r) {
            *st_save_as = Some(SaveAs {
                kind: Some(kind),
                name: format!("My {name}"),
                error: None,
            });
        }
        if changed {
            let r = ui.button("Revert");
            action_card(
                &r,
                "Revert",
                "Takes back every change you did not save yet.",
                None,
            );
            if r.clicked() {
                *revert = true;
            }
        }
        if !system {
            let r = ui.button("Delete preset");
            action_card(
                &r,
                "Delete preset",
                "Removes this user preset's file, after you say yes. A preset that an open document or the default uses is not deleted, and neither is one that other presets inherit from.",
                None,
            );
            if r.clicked() {
                *st_delete = Some((kind, id.to_string(), name.to_string()));
            }
        }
    });
    if system {
        ui.label(
            RichText::new(
                "System presets are read-only. Save a user preset to change it. It keeps up with this one for everything you leave alone.",
            )
            .color(t.text_dim),
        );
    } else if !has_parent {
        ui.label(
            RichText::new("This preset inherits from nothing, so edit it in its file.")
                .color(t.text_dim),
        );
    }
}

/// The plotter preset list in the left column. A pick with unsaved edits
/// asks first.
fn plotter_list(ui: &mut Ui, st: &mut ProfilesState, view: &View, t: &Tokens, height: f32) {
    ui.vertical(|ui| {
        ui.set_width(LIST_WIDTH);
        egui::ScrollArea::vertical()
            .id_salt("flatcraft-profiles-plotter-list")
            .max_height(height)
            .show(ui, |ui| {
                let sel = st.plotter.as_ref().map(|d| d.id.as_str());
                if let Some(id) = preset_list(ui, &view.plotters, sel, t) {
                    if st.plotter.as_ref().is_some_and(Draft::changed) {
                        st.leave = Some(Leave::Plotter(id));
                    } else {
                        st.want_plotter = Some(id);
                    }
                }
            });
    });
}

/// Width of the preset list column.
/// The preset list (a fixed size, see docs/design/ui-rules.md).
const LIST_WIDTH: f32 = 176.0;

/// The name, origin and Save buttons of the selected plotter preset, then
/// `body` with the draft. The Plotters and the Mats page share it.
fn plotter_page(
    ui: &mut Ui,
    st: &mut ProfilesState,
    view: &View,
    t: &Tokens,
    actions: &mut Vec<Action>,
    body: impl FnOnce(&mut Ui, &mut ProfilesState, &View, &Tokens),
) {
    let height = ui.available_height();
    ui.horizontal_top(|ui| {
        plotter_list(ui, st, view, t, height);
        ui.separator();
        let Some(d) = st.plotter.as_ref() else {
            if let Some((Kind::Plotter, id, e)) = &st.load_error {
                ui.label(RichText::new(format!("Cannot load {id}: {e}")).color(t.danger));
            }
            return;
        };
        let problems = plotter_problems(&d.edit);
        let (id, name, system, parent) = (
            d.id.clone(),
            d.saved.name.clone(),
            d.system,
            d.parent.clone(),
        );
        let edit = d.edit.clone();
        let changed = d.changed();
        let mut revert = false;
        ui.vertical(|ui| {
            header(ui, &name, parent.as_deref(), system, t);
            preset_buttons(
                ui,
                Kind::Plotter,
                &id,
                &name,
                system,
                parent.is_some(),
                changed,
                &problems,
                t,
                || Request::SavePlotter {
                    profile: Box::new(edit),
                    parent: parent.clone().unwrap_or_default(),
                    new: false,
                },
                &mut st.save_as,
                &mut st.confirm_delete,
                &mut revert,
                actions,
            );
            ui.separator();
            body(ui, st, view, t);
        });
        if revert && let Some(d) = st.plotter.as_mut() {
            d.edit = d.saved.clone();
        }
    });
}

/// The Plotters page: the preset list and the plotter form.
pub(crate) fn plotters_page(
    ui: &mut Ui,
    st: &mut ProfilesState,
    view: &View,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    plotter_page(ui, st, view, t, actions, |ui, st, _, t| {
        let Some(d) = st.plotter.as_mut() else { return };
        let hl = &st.highlight;
        let out = &mut st.notes_request;
        egui::ScrollArea::vertical()
            .id_salt("flatcraft-profiles-plotter-form")
            .max_height(ui.available_height())
            .show(ui, |ui| {
                plotter_form(ui, d, hl, t);
                *out = notes_field(ui, Kind::Plotter, d).or(out.take());
            });
    });
    if let Some(req) = st.notes_request.take() {
        actions.push(Action::Profiles(req));
    }
}

/// The Mats page: the preset list and the mats of the selected plotter
/// preset, which "Save preset" stores with it.
pub(crate) fn mats_page(
    ui: &mut Ui,
    st: &mut ProfilesState,
    view: &View,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    plotter_page(ui, st, view, t, actions, |ui, st, view, t| {
        let ProfilesState {
            plotter: Some(d),
            notice,
            parent_media,
            mat,
            hex,
            hex_editing,
            highlight,
            ..
        } = st
        else {
            return;
        };
        egui::ScrollArea::vertical()
            .id_salt("flatcraft-profiles-mats")
            .max_height(ui.available_height())
            .show(ui, |ui| {
                let in_use: Vec<&str> = view
                    .mats_in_use
                    .iter()
                    .filter(|(p, _)| *p == d.id)
                    .map(|(_, m)| m.as_str())
                    .collect();
                mats(
                    ui,
                    d,
                    &in_use,
                    notice,
                    parent_media,
                    mat,
                    hex,
                    hex_editing,
                    highlight,
                    if t.is_dark() {
                        view.mat_default.1
                    } else {
                        view.mat_default.0
                    },
                    t,
                );
                ui.add_space(metrics::SPACE_16);
                // A row search can jump to, like the form rows.
                let at = ui
                    .vertical(|ui| {
                        ui.label(RichText::new("Calibration").font(metrics::heading()));
                        ui.label(CALIBRATION_LATER);
                        ui.label(
                            RichText::new(
                                "The mat assistant, which finds where the printed grid of your mat starts, arrives there too.",
                            )
                            .color(t.text_dim),
                        );
                    })
                    .response
                    .rect;
                let f = Form {
                    modified: &[],
                    t,
                    highlight,
                    system: false,
                };
                f.mark(ui, "Calibration", at);
            });
    });
}

/// What the Mats page says about calibration.
pub const CALIBRATION_LATER: &str = "The calibration wizard arrives in milestone 3.";

fn plotter_form(ui: &mut Ui, d: &mut Draft<PlotterProfile>, highlight: &Highlight, t: &Tokens) {
    let f = Form {
        modified: &d.modified,
        t,
        highlight,
        system: d.system,
    };
    let p = &mut d.edit;
    ui.add_enabled_ui(!d.system, |ui| {
        egui::Grid::new("flatcraft-profiles-plotter")
            .num_columns(2)
            .spacing([metrics::SPACE_8, metrics::ROW_GAP])
            .min_row_height(metrics::CONTROL)
            .show(ui, |ui| {
                f.row(ui, "Name", "name", Setting::PresetName, |ui| {
                    ui.text_edit_singleline(&mut p.name)
                });
                f.row(ui, "Vendor", "vendor", Setting::Vendor, |ui| {
                    ui.text_edit_singleline(&mut p.vendor)
                });
                f.row(ui, "Model", "model", Setting::Model, |ui| {
                    ui.text_edit_singleline(&mut p.model)
                });
                f.row(ui, "Protocol", "protocol", Setting::Protocol, |ui| {
                    let mut v = 0;
                    combo(ui, "protocol", &mut v, &[(0, "GPGL")])
                });
                f.row(ui, "Dialect", "protocol", Setting::Dialect, |ui| {
                    let Protocol::Gpgl { dialect } = &mut p.protocol;
                    combo(
                        ui,
                        "dialect",
                        dialect,
                        &[
                            (GpglDialect::Cameo12, "Cameo 1 and 2"),
                            (GpglDialect::Cameo3Plus, "Cameo 3 and later"),
                        ],
                    )
                });
                let mut usb = matches!(p.transport, TransportSpec::Usb { .. });
                f.row(ui, "Transport", "transport", Setting::Transport, |ui| {
                    combo(ui, "transport", &mut usb, &[(true, "USB"), (false, "File")])
                });
                match (&mut p.transport, usb) {
                    (TransportSpec::Usb { vid, pid }, true) => {
                        f.row(
                            ui,
                            "USB vendor id",
                            "transport",
                            Setting::UsbVendorId,
                            |ui| hex_id(ui, vid),
                        );
                        f.row(
                            ui,
                            "USB product id",
                            "transport",
                            Setting::UsbProductId,
                            |ui| hex_id(ui, pid),
                        );
                    }
                    (
                        TransportSpec::File {
                            directory,
                            extension,
                        },
                        false,
                    ) => {
                        f.row(
                            ui,
                            "Output folder",
                            "transport",
                            Setting::OutputFolder,
                            |ui| {
                                let mut text = directory
                                    .as_ref()
                                    .map(|d| d.display().to_string())
                                    .unwrap_or_default();
                                let r = ui.add(
                                    egui::TextEdit::singleline(&mut text)
                                        .hint_text("The General page's folder"),
                                );
                                if r.changed() {
                                    *directory = (!text.trim().is_empty()).then(|| text.into());
                                }
                                r
                            },
                        );
                        f.row(
                            ui,
                            "File extension",
                            "transport",
                            Setting::FileExtension,
                            |ui| ui.text_edit_singleline(extension),
                        );
                    }
                    (_, true) => {
                        p.transport = TransportSpec::Usb {
                            vid: 0x0b4d,
                            pid: 0x112b,
                        }
                    }
                    (_, false) => {
                        p.transport = TransportSpec::File {
                            directory: None,
                            extension: "gpgl".into(),
                        }
                    }
                }
                f.row(
                    ui,
                    "Units per mm",
                    "units_per_mm",
                    Setting::UnitsPerMm,
                    |ui| num(ui, true, &mut p.units_per_mm, 3, NumKind::Plain),
                );
                f.check(
                    ui,
                    "Mirror x",
                    "axis.mirror_x",
                    Setting::MirrorX,
                    &mut p.axis.mirror_x,
                );
                f.check(
                    ui,
                    "Mirror y",
                    "axis.mirror_y",
                    Setting::MirrorY,
                    &mut p.axis.mirror_y,
                );
                f.row(ui, "Scale x", "scale_x", Setting::ScaleX, |ui| {
                    num(ui, true, &mut p.scale_x, 5, NumKind::Plain)
                });
                f.row(ui, "Scale y", "scale_y", Setting::ScaleY, |ui| {
                    num(ui, true, &mut p.scale_y, 5, NumKind::Plain)
                });
                knife(ui, &f, p);
                f.row(
                    ui,
                    "Ring direction",
                    "ring_direction",
                    Setting::RingDirection,
                    |ui| {
                        combo(
                            ui,
                            "ring-direction",
                            &mut p.ring_direction,
                            &[
                                (RingDirection::Ccw, "Counter-clockwise"),
                                (RingDirection::Cw, "Clockwise"),
                            ],
                        )
                    },
                );
                f.row(
                    ui,
                    "Blade offset (mm)",
                    "blade_offset_mm",
                    Setting::BladeOffset,
                    |ui| num(ui, true, &mut p.blade_offset_mm, 3, NumKind::Length),
                );
                f.row(ui, "Speed min", "speed_range", Setting::SpeedRange, |ui| {
                    int(ui, true, &mut p.speed_range[0])
                });
                f.row(ui, "Speed max", "speed_range", Setting::SpeedRange, |ui| {
                    int(ui, true, &mut p.speed_range[1])
                });
                f.row(ui, "Force min", "force_range", Setting::ForceRange, |ui| {
                    int(ui, true, &mut p.force_range[0])
                });
                f.row(ui, "Force max", "force_range", Setting::ForceRange, |ui| {
                    int(ui, true, &mut p.force_range[1])
                });
                f.row(
                    ui,
                    "Passes min",
                    "passes_range",
                    Setting::PassesRange,
                    |ui| int(ui, true, &mut p.passes_range[0]),
                );
                f.row(
                    ui,
                    "Passes max",
                    "passes_range",
                    Setting::PassesRange,
                    |ui| int(ui, true, &mut p.passes_range[1]),
                );
                let c = &mut p.capabilities;
                f.check(
                    ui,
                    "Track enhancing",
                    "capabilities.track_enhancing",
                    Setting::TrackEnhancingSupport,
                    &mut c.track_enhancing,
                );
                f.check(
                    ui,
                    "Registration marks",
                    "capabilities.registration_marks",
                    Setting::RegistrationMarks,
                    &mut c.registration_marks,
                );
                f.check(
                    ui,
                    "Auto blade",
                    "capabilities.auto_blade",
                    Setting::AutoBlade,
                    &mut c.auto_blade,
                );
                f.row(
                    ui,
                    "Flatten tolerance (mm)",
                    "flatten_tolerance_mm",
                    Setting::FlattenTolerance,
                    |ui| num(ui, true, &mut p.flatten_tolerance_mm, 4, NumKind::Length),
                );
            });
    });
}

fn knife(ui: &mut Ui, f: &Form<'_>, p: &mut PlotterProfile) {
    #[derive(Clone, Copy, PartialEq)]
    enum K {
        Firmware,
        Host,
        None,
    }
    let mut k = match p.knife_compensation {
        KnifeCompensation::Firmware { .. } => K::Firmware,
        KnifeCompensation::Host { .. } => K::Host,
        KnifeCompensation::None => K::None,
    };
    let before = k;
    f.row(
        ui,
        "Knife compensation",
        "knife_compensation",
        Setting::KnifeCompensation,
        |ui| {
            combo(
                ui,
                "knife",
                &mut k,
                &[
                    (K::Firmware, "Firmware"),
                    (K::Host, "Host"),
                    (K::None, "None"),
                ],
            )
        },
    );
    if k != before {
        p.knife_compensation = match k {
            K::Firmware => KnifeCompensation::Firmware { fc: 18 },
            // The pivot threshold defaults to 0: an arc at every corner (R2).
            K::Host => KnifeCompensation::Host {
                offset_mm: p.blade_offset_mm,
                pivot_threshold_deg: 0.0,
            },
            K::None => KnifeCompensation::None,
        };
    }
    match &mut p.knife_compensation {
        KnifeCompensation::Firmware { fc } => {
            f.row(
                ui,
                "FC value",
                "knife_compensation",
                Setting::FcValue,
                |ui| int(ui, true, fc),
            );
        }
        KnifeCompensation::Host {
            offset_mm,
            pivot_threshold_deg,
        } => {
            f.row(
                ui,
                "Host offset (mm)",
                "knife_compensation",
                Setting::HostOffset,
                |ui| num(ui, true, offset_mm, 3, NumKind::Length),
            );
            f.row(
                ui,
                "Pivot threshold (deg)",
                "knife_compensation",
                Setting::PivotThreshold,
                |ui| num(ui, true, pivot_threshold_deg, 1, NumKind::Angle),
            );
        }
        KnifeCompensation::None => {}
    }
}

/// An id not used by any mat in `taken`, from `base`.
fn fresh_mat_id(base: &str, taken: &[&str]) -> String {
    if !taken.contains(&base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken.contains(&id.as_str()))
        .unwrap_or_else(|| base.to_string())
}

/// The Mats section: list, add, copy, delete, the fields of the selected
/// mat and a small drawing of it.
#[allow(clippy::too_many_arguments)]
fn mats(
    ui: &mut Ui,
    d: &mut Draft<PlotterProfile>,
    in_use: &[&str],
    notice: &mut Option<String>,
    parent: &[Media],
    selected: &mut Option<String>,
    hex_text: &mut String,
    hex_editing: &mut bool,
    highlight: &Highlight,
    default_colour: [u8; 3],
    t: &Tokens,
) {
    let f = Form {
        modified: &d.modified,
        t,
        highlight,
        system: d.system,
    };
    ui.label(
        RichText::new("Measure twice, cut once. The grid's 0,0 is FlatCraft's 0,0.")
            .color(t.text_dim),
    );
    let system = d.system;
    let media = &mut d.edit.media;
    let builtin = |id: &str| parent.iter().any(|m| m.id == id);
    // Visible mats in their order, then the hidden built-in ones.
    let hidden: Vec<&Media> = parent
        .iter()
        .filter(|p| !media.iter().any(|m| m.id == p.id))
        .collect();
    // The mats side by side, so the form below stays in view.
    ui.horizontal_wrapped(|ui| {
        for (m, is_hidden) in media
            .iter()
            .map(|m| (m, false))
            .chain(hidden.iter().map(|m| (*m, true)))
        {
            let mut notes = Vec::new();
            if is_hidden {
                notes.push("hidden");
            }
            if m.validate().is_err() && !is_hidden {
                notes.push("needs a fix");
            }
            if f.is_modified(&format!("media.{}", m.id)) {
                notes.push("modified");
            }
            let label = if notes.is_empty() {
                m.name.clone()
            } else {
                format!("{} ({})", m.name, notes.join(", "))
            };
            let on = selected.as_deref() == Some(m.id.as_str());
            let r = ui.add(egui::Button::selectable(on, label.as_str()));
            action_card(&r, &label, "Shows this mat's fields below.", None);
            if r.clicked() {
                *selected = Some(m.id.clone());
                *hex_editing = false;
            }
        }
    });
    ui.add_enabled_ui(!system, |ui| {
        ui.horizontal(|ui| {
            let owned: Vec<String> = media
                .iter()
                .chain(parent.iter())
                .map(|m| m.id.clone())
                .collect();
            let taken: Vec<&str> = owned.iter().map(String::as_str).collect();
            let r = ui.button("Add mat");
            f.card_text(
                &r,
                "Add mat",
                "Adds a new mat to this plotter with a 12 by 12 inch grid. Then set its fields below to match your mat.",
                None,
            );
            if r.clicked() {
                let id = fresh_mat_id("my-mat", &taken);
                media.push(Media {
                    id: id.clone(),
                    name: "New mat".into(),
                    kind: MediaKind::Mat,
                    width_mm: 304.8,
                    height_mm: 330.2,
                    grid: Grid {
                        origin_x_mm: 0.0,
                        origin_y_mm: 0.0,
                        width_mm: 304.8,
                        height_mm: 304.8,
                    },
                    calibrated: false,
                    color: None,
                });
                *selected = Some(id);
            }
            let sel = selected
                .as_ref()
                .and_then(|s| media.iter().position(|m| &m.id == s));
            let r = ui.add_enabled(sel.is_some(), egui::Button::new("Copy mat"));
            f.card_text(
                &r,
                "Copy mat",
                "Adds a copy of the picked mat, so you can change it and keep the original.",
                match (selected.is_some(), sel.is_some()) {
                    (false, _) => Some(PICK_A_MAT),
                    // Picked, but not among the visible mats: a hidden
                    // built-in one.
                    (true, false) => Some(HIDDEN_NO_COPY),
                    (true, true) => None,
                },
            );
            if r.clicked()
                && let Some(i) = sel
            {
                let mut copy = media[i].clone();
                copy.id = fresh_mat_id(&format!("{}-copy", copy.id), &taken);
                copy.name = format!("{} copy", copy.name);
                *selected = Some(copy.id.clone());
                media.push(copy);
            }
            let own = sel.filter(|i| !builtin(&media[*i].id));
            // Only your own mats can be deleted; built-in ones are hidden.
            let delete = own.map(|i| {
                let r = ui.button("Delete mat");
                f.card_text(
                    &r,
                    "Delete mat",
                    "Removes this mat of your own. Built-in mats cannot be deleted, only hidden. A mat in use is not deleted: pick another mat in the job first.",
                    None,
                );
                (i, r.clicked())
            });
            if let Some((i, true)) = delete {
                if in_use.contains(&media[i].id.as_str()) {
                    *notice = Some(MAT_IN_USE.into());
                } else {
                    media.remove(i);
                    *selected = media.first().map(|m| m.id.clone());
                }
            }
        });
    });
    ui.add_space(metrics::SPACE_4);

    let Some(id) = selected.clone() else {
        return;
    };
    let is_builtin = builtin(&id);
    let at = media.iter().position(|m| m.id == id);
    ui.add_enabled_ui(!system, |ui| {
        if is_builtin {
            let mut hide = at.is_none();
            let r = ui.checkbox(&mut hide, "Hidden");
            set_name(ui, &r, "Hidden");
            f.card(&r, "Hidden", Setting::HiddenMat, None);
            if r.changed() {
                match (hide, at) {
                    (true, Some(_)) if in_use.contains(&id.as_str()) => {
                        *notice = Some(MAT_IN_USE.into());
                    }
                    (true, Some(i)) => {
                        media.remove(i);
                    }
                    (false, None) => {
                        if let Some(m) = parent.iter().find(|m| m.id == id) {
                            // Back where the parent lists it.
                            let rank = |x: &str| parent.iter().position(|p| p.id == x);
                            let mine = rank(&id);
                            let pos = media
                                .iter()
                                .position(|o| match (rank(&o.id), mine) {
                                    (Some(r), Some(me)) => r > me,
                                    _ => true,
                                })
                                .unwrap_or(media.len());
                            media.insert(pos, m.clone());
                        }
                    }
                    _ => {}
                }
            }
            ui.label(
                RichText::new("Built-in mats can be hidden, never deleted.").color(t.text_dim),
            );
        }
    });
    let Some(i) = media.iter().position(|m| m.id == id) else {
        if is_builtin {
            ui.label(
                RichText::new("This mat is hidden. Turn off Hidden to use it again.")
                    .color(t.text_dim),
            );
        }
        return;
    };
    // The dots mark the fields of the stored mat that differ from the
    // parent's mat with the same id (`media.<id>` in `modified_plotter_fields`).
    let mat_modified: Vec<String> = match (
        f.is_modified(&format!("media.{id}")),
        d.saved.media.iter().find(|m| m.id == id),
        parent.iter().find(|m| m.id == id),
    ) {
        (true, Some(s), Some(p)) => mat_differences(s, p),
        _ => Vec::new(),
    };
    let f = Form {
        modified: &mat_modified,
        t,
        highlight,
        system,
    };
    let m = &mut media[i];
    let problem = mat_problem(m);
    // The error goes on the row right under the field it names.
    let show_problem = |ui: &mut Ui, g: MediaField| {
        if let Some((pg, text)) = &problem
            && *pg == g
        {
            paint_text(ui, "", t.danger, None);
            ui.label(RichText::new(text).color(t.danger));
            ui.end_row();
        }
    };
    // The drawing goes beside the fields when there is room, else under them.
    let layout = if ui.available_width() >= 540.0 {
        egui::Layout::left_to_right(egui::Align::Min)
    } else {
        egui::Layout::top_down(egui::Align::Min)
    };
    ui.with_layout(layout, |ui| {
        ui.add_enabled_ui(!system, |ui| {
            egui::Grid::new("flatcraft-profiles-mat")
                .num_columns(2)
                .spacing([metrics::SPACE_8, metrics::ROW_GAP])
                .min_row_height(metrics::CONTROL)
                .show(ui, |ui| {
                    f.row(ui, "Mat name", "name", Setting::MatName, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(160.0))
                    });
                    f.row(ui, "Mat kind", "kind", Setting::MatKind, |ui| {
                        combo(
                            ui,
                            "mat-kind",
                            &mut m.kind,
                            &[(MediaKind::Mat, "Mat"), (MediaKind::NoMat, "No mat")],
                        )
                    });
                    f.row(
                        ui,
                        "Physical width (mm)",
                        "width",
                        Setting::MatWidth,
                        |ui| num(ui, true, &mut m.width_mm, 2, NumKind::Length),
                    );
                    show_problem(ui, MediaField::Width);
                    f.row(
                        ui,
                        "Physical height (mm)",
                        "height",
                        Setting::MatHeight,
                        |ui| num(ui, true, &mut m.height_mm, 2, NumKind::Length),
                    );
                    show_problem(ui, MediaField::Height);
                    f.row(
                        ui,
                        "Grid origin x (mm)",
                        "origin_x",
                        Setting::GridOriginX,
                        |ui| num(ui, true, &mut m.grid.origin_x_mm, 2, NumKind::Length),
                    );
                    show_problem(ui, MediaField::OriginX);
                    f.row(
                        ui,
                        "Grid origin y (mm)",
                        "origin_y",
                        Setting::GridOriginY,
                        |ui| num(ui, true, &mut m.grid.origin_y_mm, 2, NumKind::Length),
                    );
                    show_problem(ui, MediaField::OriginY);
                    f.row(
                        ui,
                        "Grid width (mm)",
                        "grid_width",
                        Setting::GridWidth,
                        |ui| num(ui, true, &mut m.grid.width_mm, 2, NumKind::Length),
                    );
                    show_problem(ui, MediaField::GridWidth);
                    f.row(
                        ui,
                        "Grid height (mm)",
                        "grid_height",
                        Setting::GridHeight,
                        |ui| num(ui, true, &mut m.grid.height_mm, 2, NumKind::Length),
                    );
                    show_problem(ui, MediaField::GridHeight);
                    f.check(
                        ui,
                        "Calibrated",
                        "calibrated",
                        Setting::Calibrated,
                        &mut m.calibrated,
                    );
                    f.row(ui, "Mat colour", "color", Setting::MatColour, |ui| {
                        ui.horizontal(|ui| {
                            let own = m.color.is_some();
                            if ui.selectable_label(!own, "Use default").clicked() {
                                m.color = None;
                            }
                            if ui.selectable_label(own, "Custom").clicked() && !own {
                                m.color = Some(default_colour);
                            }
                            let mut shown = m.color.unwrap_or(default_colour);
                            if own {
                                if ui.color_edit_button_srgb(&mut shown).changed() {
                                    m.color = Some(shown);
                                }
                            } else {
                                // The default, shown as a swatch.
                                let (r, _) = ui.allocate_exact_size(
                                    egui::Vec2::splat(metrics::ICON_COMPACT),
                                    Sense::hover(),
                                );
                                ui.painter().rect_filled(r, 3.0, canvas::rgb(shown));
                                ui.painter().rect_stroke(
                                    r,
                                    3.0,
                                    egui::Stroke::new(1.0, t.border),
                                    egui::StrokeKind::Inside,
                                );
                            }
                        })
                        .response
                    });
                    f.row_dimmed(
                        ui,
                        "Mat colour (hex)",
                        "color",
                        Setting::MatColourHex,
                        m.color.is_none().then_some(MAT_COLOUR_DEFAULT_WHY),
                        |ui| {
                            let own = m.color.is_some();
                            if !*hex_editing {
                                *hex_text = hex(m.color.unwrap_or(default_colour));
                            }
                            let r = ui.add_enabled(
                                own,
                                egui::TextEdit::singleline(hex_text).desired_width(90.0),
                            );
                            *hex_editing = r.has_focus();
                            if own
                                && (r.changed() || r.lost_focus())
                                && let Some(c) = parse_hex(hex_text)
                            {
                                m.color = Some(c);
                            }
                            r
                        },
                    );
                });
        });
        mat_drawing(ui, m, default_colour, t);
    });
    if m.width_mm > MAX_SIZE_MM || m.height_mm > MAX_SIZE_MM {
        ui.label(RichText::new(format!("Mats up to {MAX_SIZE_MM} mm.")).color(t.text_dim));
    }
}

/// The fields of mat `a` that differ from mat `b`, as the mat form keys them.
fn mat_differences(a: &Media, b: &Media) -> Vec<String> {
    let (ga, gb) = (&a.grid, &b.grid);
    [
        ("name", a.name != b.name),
        ("kind", a.kind != b.kind),
        ("width", a.width_mm != b.width_mm),
        ("height", a.height_mm != b.height_mm),
        ("origin_x", ga.origin_x_mm != gb.origin_x_mm),
        ("origin_y", ga.origin_y_mm != gb.origin_y_mm),
        ("grid_width", ga.width_mm != gb.width_mm),
        ("grid_height", ga.height_mm != gb.height_mm),
        ("calibrated", a.calibrated != b.calibrated),
        ("color", a.color != b.color),
    ]
    .into_iter()
    .filter(|(_, differs)| *differs)
    .map(|(k, _)| k.to_string())
    .collect()
}

/// The physical mat in its colour, the grid area and the origin, to scale.
fn mat_drawing(ui: &mut Ui, m: &Media, default_colour: [u8; 3], t: &Tokens) {
    let mat_colour = m.color.unwrap_or(default_colour);
    let size = vec2(150.0, 200.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let (w, h) = (m.width_mm, m.height_mm);
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return;
    }
    let scale = (f64::from(size.x) / w).min(f64::from(size.y) / h) as f32;
    let mat = egui::Rect::from_min_size(rect.min, vec2(w as f32 * scale, h as f32 * scale));
    let [r, g, b] = mat_colour;
    let painter = ui.painter_at(rect);
    painter.rect_filled(mat, 2.0, Color32::from_rgb(r, g, b));
    let [cr, cg, cb] = canvas::contrast_colour(mat_colour);
    let ink = Color32::from_rgb(cr, cg, cb);
    let gr = &m.grid;
    let grid = egui::Rect::from_min_size(
        mat.min + vec2(gr.origin_x_mm as f32, gr.origin_y_mm as f32) * scale,
        vec2(gr.width_mm as f32, gr.height_mm as f32) * scale,
    );
    let colour = if m.validate().is_ok() { ink } else { t.danger };
    // The grid is the cutting area: a light tint sets it off from the
    // margin of the physical mat.
    painter.rect_filled(grid, 0.0, ink.gamma_multiply(0.18));
    painter.rect_stroke(
        grid,
        0.0,
        egui::Stroke::new(1.0, colour),
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(grid.min, 3.0, t.accent_cut);
    painter.text(
        grid.min + vec2(5.0, 3.0),
        egui::Align2::LEFT_TOP,
        "0,0",
        fonts::medium(metrics::TEXT_SMALL),
        ink,
    );
}

/// The Materials page: the preset list and the material form.
pub(crate) fn materials_page(
    ui: &mut Ui,
    st: &mut ProfilesState,
    view: &View,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let height = ui.available_height();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(LIST_WIDTH);
            egui::ScrollArea::vertical()
                .id_salt("flatcraft-profiles-material-list")
                .max_height(height)
                .show(ui, |ui| {
                    let sel = st.material.as_ref().map(|d| d.id.as_str());
                    if let Some(id) = preset_list(ui, &view.materials, sel, t) {
                        if st.material.as_ref().is_some_and(Draft::changed) {
                            st.leave = Some(Leave::Material(id));
                        } else {
                            st.want_material = Some(id);
                        }
                    }
                });
        });
        ui.separator();
        let Some(d) = st.material.as_mut() else {
            if let Some((Kind::Material, id, e)) = &st.load_error {
                ui.label(RichText::new(format!("Cannot load {id}: {e}")).color(t.danger));
            }
            return;
        };
        let problems = material_problems(&d.edit);
        let mut revert = false;
        ui.vertical(|ui| {
            let (id, name, system, parent) = (
                d.id.clone(),
                d.saved.name.clone(),
                d.system,
                d.parent.clone(),
            );
            header(ui, &name, parent.as_deref(), system, t);
            let edit = d.edit.clone();
            preset_buttons(
                ui,
                Kind::Material,
                &id,
                &name,
                system,
                parent.is_some(),
                d.changed(),
                &problems,
                t,
                || Request::SaveMaterial {
                    profile: Box::new(edit),
                    parent: parent.clone().unwrap_or_default(),
                    new: false,
                },
                &mut st.save_as,
                &mut st.confirm_delete,
                &mut revert,
                actions,
            );
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("flatcraft-profiles-material-form")
                .max_height(ui.available_height())
                .show(ui, |ui| {
                    material_form(ui, d, &st.highlight, view, t);
                    if let Some(req) = notes_field(ui, Kind::Material, d) {
                        actions.push(Action::Profiles(req));
                    }
                });
        });
        if revert {
            d.edit = d.saved.clone();
        }
    });
}

/// Why "Save notes" is dimmed when the notes did not change.
pub const NOTES_UNCHANGED: &str = "The notes did not change, so there is nothing to save.";

/// The notes of a preset, under its form. They are always the person's
/// own, so the field is open on a system preset too: there "Save notes"
/// stores them beside the preset, which stays as it ships. On a user
/// preset "Save preset" stores them with the rest, and on one without a
/// parent (where Save preset is dimmed) "Save notes" writes them into its
/// file.
fn notes_field<T: PresetNotes + PartialEq>(
    ui: &mut Ui,
    kind: Kind,
    d: &mut Draft<T>,
) -> Option<Request> {
    ui.add_space(metrics::SPACE_8);
    ui.label(RichText::new("Notes").font(metrics::heading()));
    let mut text = d.edit.notes().cloned().unwrap_or_default();
    let r = ui.add(
        egui::TextEdit::multiline(&mut text)
            .hint_text("What worked, what to watch out for...")
            .desired_rows(4)
            .desired_width(f32::INFINITY),
    );
    set_name(ui, &r, "Preset notes");
    super::groups::card(&r, "Notes", Setting::PresetNotes.card(), None, false);
    if r.changed() {
        *d.edit.notes_mut() = (!text.is_empty()).then_some(text);
    }
    // A user preset with a parent saves its notes with "Save preset"; one
    // without a parent cannot use that, so it gets "Save notes" too.
    if !d.system && d.parent.is_some() {
        return None;
    }
    let changed = d.edit.notes() != d.saved.notes();
    let b = ui.add_enabled(changed, egui::Button::new("Save notes"));
    let what = if d.system {
        "Stores your notes on this system preset in your own settings folder. The preset itself stays as it ships."
    } else {
        "Stores your notes in this preset's file. Nothing else in it changes."
    };
    action_card(
        &b,
        "Save notes",
        what,
        (!changed).then_some(NOTES_UNCHANGED),
    );
    b.clicked().then(|| Request::SaveNotes {
        kind,
        id: d.id.clone(),
        name: d.saved.name().to_string(),
        notes: d.edit.notes().cloned().unwrap_or_default(),
    })
}

/// The notes of the draft were stored as `saved`: they count as saved now,
/// and every other edit of the draft stays as it is.
fn notes_saved<T: PresetNotes>(d: &mut Draft<T>, saved: Option<String>) {
    *d.saved.notes_mut() = saved.clone();
    *d.edit.notes_mut() = saved;
}

/// The notes of a plotter or material preset.
trait PresetNotes {
    fn notes(&self) -> Option<&String>;
    fn notes_mut(&mut self) -> &mut Option<String>;
    fn name(&self) -> &str;
}

impl PresetNotes for PlotterProfile {
    fn notes(&self) -> Option<&String> {
        self.notes.as_ref()
    }
    fn notes_mut(&mut self) -> &mut Option<String> {
        &mut self.notes
    }
    fn name(&self) -> &str {
        &self.name
    }
}

impl PresetNotes for MaterialProfile {
    fn notes(&self) -> Option<&String> {
        self.notes.as_ref()
    }
    fn notes_mut(&mut self) -> &mut Option<String> {
        &mut self.notes
    }
    fn name(&self) -> &str {
        &self.name
    }
}

fn material_form(
    ui: &mut Ui,
    d: &mut Draft<MaterialProfile>,
    highlight: &Highlight,
    view: &View,
    t: &Tokens,
) {
    let f = Form {
        modified: &d.modified,
        t,
        highlight,
        system: d.system,
    };
    let m = &mut d.edit;
    ui.add_enabled_ui(!d.system, |ui| {
        egui::Grid::new("flatcraft-profiles-material")
            .num_columns(2)
            .spacing([metrics::SPACE_8, metrics::ROW_GAP])
            .min_row_height(metrics::CONTROL)
            .show(ui, |ui| {
                f.row(ui, "Name", "name", Setting::PresetName, |ui| {
                    ui.text_edit_singleline(&mut m.name)
                });
                f.row(ui, "Speed", "speed", Setting::Speed, |ui| {
                    int(ui, true, &mut m.speed)
                });
                f.row(ui, "Force", "force", Setting::Force, |ui| {
                    int(ui, true, &mut m.force)
                });
                f.row(ui, "Passes", "passes", Setting::Passes, |ui| {
                    int(ui, true, &mut m.passes)
                });
                f.check(
                    ui,
                    "Reverse alternate passes",
                    "reverse_alternate_passes",
                    Setting::ReverseAlternatePasses,
                    &mut m.reverse_alternate_passes,
                );
                f.row(ui, "Overcut (mm)", "overcut_mm", Setting::Overcut, |ui| {
                    num(ui, true, &mut m.overcut_mm, 2, NumKind::Length)
                });
                f.check(
                    ui,
                    "Track enhancing",
                    "track_enhancing",
                    Setting::TrackEnhancing,
                    &mut m.track_enhancing,
                );
                corner_rows(ui, &f, m);
                f.check(
                    ui,
                    "Backing sheet",
                    "backing",
                    Setting::Backing,
                    &mut m.backing,
                );
                f.row(
                    ui,
                    "Blade depth hint",
                    "blade_depth_hint",
                    Setting::BladeDepthHint,
                    |ui| {
                        let mut text = m.blade_depth_hint.clone().unwrap_or_default();
                        let r = ui.text_edit_singleline(&mut text);
                        if r.changed() {
                            m.blade_depth_hint = (!text.is_empty()).then_some(text);
                        }
                        r
                    },
                );
                f.row(ui, "Mat tack", "mat_tack", Setting::MatTack, |ui| {
                    let choices = MatTack::ALL.map(|v| (v, v.label()));
                    combo(ui, "mat-tack", &mut m.mat_tack, &choices)
                });
                f.row(ui, "Picker group", "group", Setting::MaterialGroup, |ui| {
                    let choices = MaterialGroup::ALL.map(|v| (v, v.label()));
                    combo(ui, "material-group", &mut m.group, &choices)
                });
                if let Some(tips) = m.tips.as_deref().filter(|s| !s.trim().is_empty()) {
                    f.row(ui, "Tips", "tips", Setting::PresetTips, |ui| {
                        ui.add(egui::Label::new(tips.trim()).wrap())
                    });
                }
            });
        ui.add_space(metrics::SPACE_8);
        ui.label(RichText::new("Per-plotter overrides").font(metrics::heading()));
        ui.label(
            RichText::new("Values that differ on one plotter. The rest come from above.")
                .color(t.text_dim),
        );
        overrides(ui, &f, m, view, t);
    });
}

/// The corner mode, its overshoot and the minimum radius of a material.
/// The two corner modes as combo choices, without the overshoot length.
#[derive(Clone, Copy, PartialEq)]
enum CornerChoice {
    Arc,
    Lift,
}

impl CornerChoice {
    const CHOICES: [(CornerChoice, &'static str); 2] = [
        (CornerChoice::Arc, "Swivel (arc)"),
        (CornerChoice::Lift, "Lift and overshoot"),
    ];

    fn of(c: CornerModeSpec) -> Self {
        match c {
            CornerModeSpec::Arc => CornerChoice::Arc,
            CornerModeSpec::LiftOvershoot { .. } => CornerChoice::Lift,
        }
    }

    /// The corner mode of this choice; a switch to lift starts at the
    /// default overshoot.
    fn spec(self) -> CornerModeSpec {
        match self {
            CornerChoice::Arc => CornerModeSpec::Arc,
            CornerChoice::Lift => CornerModeSpec::LiftOvershoot {
                overshoot_mm: DEFAULT_CORNER_OVERSHOOT_MM,
            },
        }
    }
}

/// The overshoot a corner mode shows in its field: its own, or the default
/// (dimmed) while the blade swivels.
fn shown_overshoot(c: CornerModeSpec) -> f64 {
    match c {
        CornerModeSpec::LiftOvershoot { overshoot_mm } => overshoot_mm,
        CornerModeSpec::Arc => DEFAULT_CORNER_OVERSHOOT_MM,
    }
}

fn corner_rows(ui: &mut Ui, f: &Form<'_>, m: &mut MaterialProfile) {
    let mut c = CornerChoice::of(m.corner_mode);
    let before = c;
    f.row(
        ui,
        "Corner mode",
        "corner_mode",
        Setting::CornerMode,
        |ui| combo(ui, "corner-mode", &mut c, &CornerChoice::CHOICES),
    );
    if c != before {
        m.corner_mode = c.spec();
    }
    // The overshoot stays visible while the blade swivels, dimmed.
    let lift = matches!(m.corner_mode, CornerModeSpec::LiftOvershoot { .. });
    let mut shown = shown_overshoot(m.corner_mode);
    f.row_dimmed(
        ui,
        "Corner overshoot (mm)",
        "corner_mode",
        Setting::CornerOvershoot,
        (!lift).then_some(OVERSHOOT_OFF),
        |ui| num(ui, lift, &mut shown, 2, NumKind::Length),
    );
    if lift {
        m.corner_mode = CornerModeSpec::LiftOvershoot {
            overshoot_mm: shown,
        };
    }
    let label = "Minimum radius (mm)";
    let key = "min_radius_mm";
    let dot = f.is_modified(key).then_some(f.t.accent_prepare);
    let at = paint_text(ui, label, ui.visuals().text_color(), dot);
    let r = ui
        .horizontal(|ui| {
            let mut on = m.min_radius_mm.is_some();
            let r = ui.checkbox(&mut on, "Own value");
            set_name(ui, &r, &format!("{label} own value"));
            f.card(&r, label, Setting::MinRadiusOwn, None);
            if r.changed() {
                m.min_radius_mm = on.then_some(DEFAULT_MIN_RADIUS_MM);
            }
            let mut v = m.min_radius_mm.unwrap_or(DEFAULT_MIN_RADIUS_MM);
            let r = radius_field(ui, on.then_some(&mut v));
            set_name(ui, &r, &f.name(label, key));
            f.card(
                &r,
                label,
                Setting::MinRadius,
                (!on).then_some(MIN_RADIUS_OFF),
            );
            if on {
                m.min_radius_mm = Some(v);
            }
            r
        })
        .inner;
    f.mark(ui, label, at.union(r.rect));
    ui.end_row();
}

/// A minimum radius field: the number while it has one, otherwise a dimmed
/// "blade offset", because the plotter's blade offset applies (the material
/// form does not know which plotter that is).
fn radius_field(ui: &mut Ui, v: Option<&mut f64>) -> Response {
    match v {
        Some(v) => num(ui, true, v, 2, NumKind::Length),
        None => {
            let mut text = BLADE_OFFSET_TEXT.to_string();
            ui.add_enabled(
                false,
                egui::TextEdit::singleline(&mut text).desired_width(96.0),
            )
        }
    }
}

/// What a minimum radius field shows while the blade offset applies.
const BLADE_OFFSET_TEXT: &str = "blade offset";

/// The overshoot a material gets when it switches to lift and overshoot:
/// the usual 0.1 mm of vendor software.
const DEFAULT_CORNER_OVERSHOOT_MM: f64 = 0.1;
/// The minimum radius a material gets when it switches to its own value.
const DEFAULT_MIN_RADIUS_MM: f64 = 0.25;
/// Why the corner overshoot field is dimmed.
pub const OVERSHOOT_OFF: &str = "Corner mode is Swivel (arc), so the blade never overshoots. Pick Lift and overshoot to set one.";
/// Why the minimum radius field is dimmed.
pub const MIN_RADIUS_OFF: &str = "Own value is off, so the plotter's blade offset is the minimum radius. Tick Own value to set one for this material.";

fn overrides(ui: &mut Ui, f: &Form<'_>, m: &mut MaterialProfile, view: &View, t: &Tokens) {
    let name_of = |id: &str| {
        view.plotters
            .iter()
            .find(|p| p.id == id)
            .map_or_else(|| id.to_string(), |p| p.name.clone())
    };
    let base = (m.speed, m.force, m.passes, m.overcut_mm);
    let mut remove = None;
    egui::Grid::new("flatcraft-profiles-overrides")
        .num_columns(6)
        .spacing([metrics::SPACE_8, metrics::ROW_GAP])
            .min_row_height(metrics::CONTROL)
        .show(ui, |ui| {
            if m.overrides.is_empty() {
                return;
            }
            let dim = t.text_dim;
            for h in ["Plotter", "Speed", "Force", "Passes", "Overcut (mm)", ""] {
                paint_text(ui, h, dim, None);
            }
            ui.end_row();
            for (pid, o) in m.overrides.iter_mut() {
                let name = name_of(pid);
                let key = format!("overrides.{pid}");
                let dot = f.is_modified(&key).then_some(t.accent_prepare);
                paint_text(ui, &name, ui.visuals().text_color(), dot);
                cell(
                    ui,
                    f,
                    &format!("{name} speed"),
                    &format!("{key}.speed"),
                    Setting::Speed,
                    &mut o.speed,
                    base.0,
                );
                cell(
                    ui,
                    f,
                    &format!("{name} force"),
                    &format!("{key}.force"),
                    Setting::Force,
                    &mut o.force,
                    base.1,
                );
                cell(
                    ui,
                    f,
                    &format!("{name} passes"),
                    &format!("{key}.passes"),
                    Setting::Passes,
                    &mut o.passes,
                    base.2,
                );
                ui.horizontal(|ui| {
                    let label = format!("{name} overcut");
                    let mut on = o.overcut_mm.is_some();
                    let r = ui.checkbox(&mut on, "");
                    set_name(ui, &r, &format!("{label} set"));
                    f.card(&r, &label, Setting::OverrideSet, None);
                    if r.changed() {
                        o.overcut_mm = on.then_some(base.3);
                    }
                    let mut v = o.overcut_mm.unwrap_or(base.3);
                    let r = num(ui, on, &mut v, 2, NumKind::Length);
                    set_name(ui, &r, &f.name(&label, &format!("{key}.overcut_mm")));
                    f.card(&r, &label, Setting::Overcut, (!on).then_some(OVERRIDE_OFF));
                    if on {
                        o.overcut_mm = Some(v);
                    }
                });
                let r = ui.button(format!("Remove override for {name}"));
                f.card_text(
                    &r,
                    &format!("Remove override for {name}"),
                    "Removes this plotter's own values, so the material's values apply on it again.",
                    None,
                );
                if r.clicked() {
                    remove = Some(pid.clone());
                }
                ui.end_row();
            }
        });
    if let Some(pid) = remove {
        m.overrides.remove(&pid);
    }
    corner_overrides(ui, f, m, &name_of, t);
    let free: Vec<&PresetInfo> = view
        .plotters
        .iter()
        .filter(|p| !m.overrides.contains_key(&p.id))
        .collect();
    if !free.is_empty() {
        let mut add: Option<String> = None;
        let r = egui::ComboBox::from_id_salt("flatcraft-profiles-add-override")
            .selected_text("Add override for...")
            .width(220.0)
            .show_ui(ui, |ui| {
                for p in free {
                    if ui.selectable_label(false, &p.name).clicked() {
                        add = Some(p.id.clone());
                    }
                }
            });
        set_name(ui, &r.response, "Add override for");
        f.card(&r.response, "Add override for", Setting::AddOverride, None);
        if let Some(id) = add {
            m.overrides.insert(id, MaterialOverride::default());
        }
    }
}

/// The corner mode, overshoot and minimum radius of each plotter override,
/// as a second table under the first (one wide table would not fit).
fn corner_overrides(
    ui: &mut Ui,
    f: &Form<'_>,
    m: &mut MaterialProfile,
    name_of: &dyn Fn(&str) -> String,
    t: &Tokens,
) {
    if m.overrides.is_empty() {
        return;
    }
    let base_corner = m.corner_mode;
    // The material's own minimum radius; `None` is the blade offset.
    let base_radius = m.min_radius_mm;
    ui.add_space(metrics::SPACE_4);
    egui::Grid::new("flatcraft-profiles-corner-overrides")
        .num_columns(4)
        .spacing([metrics::SPACE_8, metrics::ROW_GAP])
        .min_row_height(metrics::CONTROL)
        .show(ui, |ui| {
            let dim = t.text_dim;
            for h in [
                "Plotter",
                "Corner mode",
                "Corner overshoot (mm)",
                "Minimum radius (mm)",
            ] {
                paint_text(ui, h, dim, None);
            }
            ui.end_row();
            for (pid, o) in m.overrides.iter_mut() {
                let name = name_of(pid);
                let key = format!("overrides.{pid}");
                let corner_key = format!("{key}.corner_mode");
                let radius_key = format!("{key}.min_radius_mm");
                let dot = (f.is_modified(&corner_key) || f.is_modified(&radius_key))
                    .then_some(t.accent_prepare);
                paint_text(ui, &name, ui.visuals().text_color(), dot);
                ui.horizontal(|ui| {
                    let label = format!("{name} corner mode");
                    let mut on = o.corner_mode.is_some();
                    let r = ui.checkbox(&mut on, "");
                    set_name(ui, &r, &format!("{label} set"));
                    f.card(&r, &label, Setting::OverrideSet, None);
                    if r.changed() {
                        o.corner_mode = on.then_some(base_corner);
                    }
                    let mut c = CornerChoice::of(o.corner_mode.unwrap_or(base_corner));
                    let before = c;
                    let r = ui
                        .add_enabled_ui(on, |ui| {
                            combo(
                                ui,
                                &format!("corner-mode-{pid}"),
                                &mut c,
                                &CornerChoice::CHOICES,
                            )
                        })
                        .inner;
                    set_name(ui, &r, &f.name(&label, &corner_key));
                    f.card(
                        &r,
                        &label,
                        Setting::CornerMode,
                        (!on).then_some(OVERRIDE_OFF),
                    );
                    if on && c != before {
                        o.corner_mode = Some(c.spec());
                    }
                });
                let label = format!("{name} corner overshoot");
                let lift = matches!(o.corner_mode, Some(CornerModeSpec::LiftOvershoot { .. }));
                let reason = if o.corner_mode.is_none() {
                    Some(OVERRIDE_OFF)
                } else if !lift {
                    Some(OVERSHOOT_OFF)
                } else {
                    None
                };
                let mut v = shown_overshoot(o.corner_mode.unwrap_or(base_corner));
                let r = num(ui, lift, &mut v, 2, NumKind::Length);
                set_name(ui, &r, &f.name(&label, &corner_key));
                f.card(&r, &label, Setting::CornerOvershoot, reason);
                if lift {
                    o.corner_mode = Some(CornerModeSpec::LiftOvershoot { overshoot_mm: v });
                }
                ui.horizontal(|ui| {
                    let label = format!("{name} minimum radius");
                    let mut on = o.min_radius_mm.is_some();
                    let r = ui.checkbox(&mut on, "");
                    set_name(ui, &r, &format!("{label} set"));
                    f.card(&r, &label, Setting::OverrideSet, None);
                    if r.changed() {
                        o.min_radius_mm =
                            on.then_some(base_radius.unwrap_or(DEFAULT_MIN_RADIUS_MM));
                    }
                    let r = if on {
                        let mut v = o.min_radius_mm.unwrap_or(DEFAULT_MIN_RADIUS_MM);
                        let r = num(ui, true, &mut v, 2, NumKind::Length);
                        o.min_radius_mm = Some(v);
                        r
                    } else {
                        // Dimmed: the material's own value, or the blade offset.
                        match base_radius {
                            Some(mut v) => num(ui, false, &mut v, 2, NumKind::Length),
                            None => radius_field(ui, None),
                        }
                    };
                    set_name(ui, &r, &f.name(&label, &radius_key));
                    f.card(
                        &r,
                        &label,
                        Setting::MinRadius,
                        (!on).then_some(OVERRIDE_OFF),
                    );
                });
                ui.end_row();
            }
        });
}

/// A checkbox ("<label> set") and a value that only counts while it is on.
fn cell<T: egui::emath::Numeric>(
    ui: &mut Ui,
    f: &Form<'_>,
    label: &str,
    key: &str,
    setting: Setting,
    v: &mut Option<T>,
    base: T,
) {
    ui.horizontal(|ui| {
        let mut on = v.is_some();
        let r = ui.checkbox(&mut on, "");
        set_name(ui, &r, &format!("{label} set"));
        f.card(&r, label, Setting::OverrideSet, None);
        if r.changed() {
            *v = on.then_some(base);
        }
        let mut x = v.unwrap_or(base);
        let r = int(ui, on, &mut x);
        set_name(ui, &r, &f.name(label, key));
        f.card(&r, label, setting, (!on).then_some(OVERRIDE_OFF));
        if on {
            *v = Some(x);
        }
    });
}

fn save_as_dialog(
    ctx: &egui::Context,
    st: &mut ProfilesState,
    view: &View,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let Some(dialog) = st.save_as.as_mut() else {
        return;
    };
    let Some(kind) = dialog.kind else {
        st.save_as = None;
        return;
    };
    let mut close = false;
    egui::Modal::new(egui::Id::unique("flatcraft-profiles-save-as")).show(ctx, |ui| {
        ui.set_width(metrics::SMALL_DIALOG_W);
        ui.label(RichText::new("Save as user preset").font(metrics::title()));
        ui.add_space(metrics::SPACE_4);
        let r = ui.text_edit_singleline(&mut dialog.name);
        set_name(ui, &r, "Preset name");
        action_card(
            &r,
            "Preset name",
            "The name of the new preset. FlatCraft makes its file name from it.",
            None,
        );
        if let Some(e) = &dialog.error {
            ui.label(RichText::new(e).color(t.danger));
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            let c = ui.button("Create");
            action_card(
                &c,
                "Create",
                "Saves the new user preset under this name. The name needs at least one letter or digit. Its file is named after it, starting with user-, so it never clashes with a built-in preset.",
                None,
            );
            if c.clicked() || enter {
                let name = dialog.name.trim().to_string();
                // Plotters and materials live in their own folders, so an
                // id only has to be free within its kind.
                let same_kind = match kind {
                    Kind::Plotter => &view.plotters,
                    Kind::Material => &view.materials,
                };
                // A file that can't be read keeps its id taken.
                let taken: Vec<&str> = same_kind
                    .iter()
                    .map(|p| p.id.as_str())
                    .chain(view.broken_ids(kind))
                    .collect();
                let id = new_preset_id(&name, &taken);
                if id.is_empty() {
                    dialog.error =
                        Some("Give the preset a name with at least one letter or digit.".into());
                } else if let Some(r) =
                    save_as_request(st_draft(kind, &st.plotter, &st.material), id, name)
                {
                    actions.push(Action::Profiles(r));
                    close = true;
                }
            }
            let r = ui.button("Cancel");
            action_card(&r, "Cancel", "Closes this dialog and saves nothing.", None);
            if r.clicked() {
                close = true;
            }
        });
    });
    if close {
        st.save_as = None;
    }
}

/// The draft to save from, as the plotter or material request input.
enum Source<'a> {
    Plotter(&'a Draft<PlotterProfile>),
    Material(&'a Draft<MaterialProfile>),
}

fn st_draft<'a>(
    kind: Kind,
    plotter: &'a Option<Draft<PlotterProfile>>,
    material: &'a Option<Draft<MaterialProfile>>,
) -> Option<Source<'a>> {
    match kind {
        Kind::Plotter => plotter.as_ref().map(Source::Plotter),
        Kind::Material => material.as_ref().map(Source::Material),
    }
}

fn save_as_request(src: Option<Source<'_>>, id: String, name: String) -> Option<Request> {
    Some(match src? {
        Source::Plotter(d) => {
            let mut p = d.edit.clone();
            p.id = id;
            p.name = name;
            p.inherits = Some(d.id.clone());
            Request::SavePlotter {
                profile: Box::new(p),
                parent: d.id.clone(),
                new: true,
            }
        }
        Source::Material(d) => {
            let mut m = d.edit.clone();
            m.id = id;
            m.name = name;
            m.inherits = Some(d.id.clone());
            Request::SaveMaterial {
                profile: Box::new(m),
                parent: d.id.clone(),
                new: true,
            }
        }
    })
}

fn delete_dialog(
    ctx: &egui::Context,
    st: &mut ProfilesState,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let Some((kind, id, name)) = st.confirm_delete.clone() else {
        return;
    };
    let mut close = false;
    egui::Modal::new(egui::Id::unique("flatcraft-profiles-delete")).show(ctx, |ui| {
        ui.set_width(metrics::SMALL_DIALOG_W);
        ui.label(RichText::new(format!("Delete {name}?")).font(metrics::title()));
        ui.label(RichText::new("Its file is removed. This cannot be undone.").color(t.text_dim));
        ui.add_space(metrics::SPACE_8);
        ui.horizontal(|ui| {
            let r = ui.button("Yes, delete");
            action_card(
                &r,
                "Yes, delete",
                "Removes the preset file for good. This cannot be undone.",
                None,
            );
            if r.clicked() {
                actions.push(Action::Profiles(Request::Delete(kind, id.clone())));
                close = true;
            }
            let r = ui.button("Keep it");
            action_card(
                &r,
                "Keep it",
                "Closes this question and keeps the preset.",
                None,
            );
            if r.clicked() {
                close = true;
            }
        });
    });
    if close {
        st.confirm_delete = None;
    }
}

/// Runs a request of the window. Every write happens under the session
/// lock in one short step.
pub fn apply(app: &mut FlatCraftApp, req: Request) {
    match req {
        Request::SavePlotter {
            profile,
            parent,
            new,
        } => {
            let result = {
                let mut s = lock(&app.session);
                let problems = plotter_problems(&profile);
                let drops_used = mats_in_use(&s)
                    .iter()
                    .any(|(p, m)| *p == profile.id && !profile.media.iter().any(|x| x.id == *m));
                if !problems.is_empty() {
                    Err(problems.join(" "))
                } else if drops_used {
                    Err(MAT_IN_USE.to_string())
                } else if new
                    && (s.profiles.plotters().iter().any(|p| p.id == profile.id)
                        || s.profiles.broken_plotter_file(&profile.id).is_some())
                {
                    Err(format!(
                        "A preset with the id '{}' already exists.",
                        profile.id
                    ))
                } else {
                    s.profiles
                        .save_user_plotter(&profile, &parent)
                        .map_err(|e| e.to_string())
                }
            };
            saved(app, Kind::Plotter, &profile.id, &profile.name, result);
        }
        Request::SaveMaterial {
            profile,
            parent,
            new,
        } => {
            let result = {
                let mut s = lock(&app.session);
                let problems = material_problems(&profile);
                if !problems.is_empty() {
                    Err(problems.join(" "))
                } else if new
                    && (s.profiles.materials().iter().any(|p| p.id == profile.id)
                        || s.profiles.broken_material_file(&profile.id).is_some())
                {
                    Err(format!(
                        "A preset with the id '{}' already exists.",
                        profile.id
                    ))
                } else {
                    s.profiles
                        .save_user_material(&profile, &parent)
                        .map_err(|e| e.to_string())
                }
            };
            saved(app, Kind::Material, &profile.id, &profile.name, result);
        }
        Request::Delete(kind, id) => delete(app, kind, &id),
        Request::SaveNotes {
            kind,
            id,
            name,
            notes,
        } => {
            let result = {
                let mut s = lock(&app.session);
                match kind {
                    Kind::Plotter => s.profiles.set_plotter_notes(&id, &notes),
                    Kind::Material => s.profiles.set_material_notes(&id, &notes),
                }
            };
            match result {
                Ok(()) => {
                    // Only the notes are stored: the draft keeps every
                    // other edit that is not saved yet.
                    let saved = {
                        let t = notes.trim_end();
                        (!t.trim().is_empty()).then(|| t.to_string())
                    };
                    let st = &mut app.ui.profiles;
                    match kind {
                        Kind::Plotter => {
                            if let Some(d) = st.plotter.as_mut().filter(|d| d.id == id) {
                                notes_saved(d, saved);
                            }
                        }
                        Kind::Material => {
                            if let Some(d) = st.material.as_mut().filter(|d| d.id == id) {
                                notes_saved(d, saved);
                            }
                        }
                    }
                    app.toast(Toast::info(format!("Saved the notes of {name}.")));
                }
                Err(e) => app.toast(Toast::error(format!(
                    "Cannot save the notes of {name}: {e}"
                ))),
            }
        }
        Request::ReloadPresets => {
            let result = lock(&app.session).profiles.reload();
            match result {
                Ok(()) => {
                    // Load the selected presets again from the new store,
                    // unless they hold unsaved edits.
                    let st = &mut app.ui.profiles;
                    if let Some(d) = st.plotter.as_ref().filter(|d| !d.changed()) {
                        st.want_plotter = Some(d.id.clone());
                    }
                    if let Some(d) = st.material.as_ref().filter(|d| !d.changed()) {
                        st.want_material = Some(d.id.clone());
                    }
                    app.toast(Toast::info("Presets reloaded."));
                }
                Err(e) => app.toast(Toast::error(format!("Cannot reload the presets: {e}"))),
            }
        }
        Request::ChooseOutputFolder => {
            let current = match &app.ui.settings.draft {
                Some(d) => d.output_dir.clone(),
                None => lock(&app.session).prefs.output_dir.clone(),
            };
            let req = files::FileRequest {
                title: "File Output folder".into(),
                directory: current,
                ..Default::default()
            };
            // Staged: Apply or OK stores it.
            if let Some(dir) = app.files.pick_folder(&req)
                && let Some(d) = app.ui.settings.draft.as_mut()
            {
                d.output_dir = Some(dir);
            }
        }
    }
}

fn saved(app: &mut FlatCraftApp, kind: Kind, id: &str, name: &str, result: Result<(), String>) {
    match result {
        Ok(()) => {
            let st = &mut app.ui.profiles;
            match kind {
                Kind::Plotter => st.want_plotter = Some(id.to_string()),
                Kind::Material => st.want_material = Some(id.to_string()),
            }
            app.toast(Toast::info(format!("Saved {name}.")));
        }
        Err(e) => app.toast(Toast::error(format!("Cannot save {name}: {e}"))),
    }
}

fn delete(app: &mut FlatCraftApp, kind: Kind, id: &str) {
    let result = {
        let mut s = lock(&app.session);
        let in_use = s.docs.iter().any(|d| {
            let job = &d.document.job;
            match kind {
                Kind::Plotter => job.plotter_id == id,
                Kind::Material => job.material_id == id,
            }
        }) || match kind {
            Kind::Plotter => s.prefs.default_plotter == id,
            Kind::Material => s.prefs.default_material == id,
        };
        if in_use {
            Err(
                "It is in use by an open document or as the default, so it was not deleted. Pick another preset there first."
                    .to_string(),
            )
        } else {
            match kind {
                Kind::Plotter => s.profiles.delete_user_plotter(id),
                Kind::Material => s.profiles.delete_user_material(id),
            }
            .map_err(|e| e.to_string())
        }
    };
    match result {
        Ok(()) => {
            let st = &mut app.ui.profiles;
            match kind {
                Kind::Plotter => {
                    st.plotter = None;
                    st.want_plotter = None;
                }
                Kind::Material => {
                    st.material = None;
                    st.want_material = None;
                }
            }
            app.toast(Toast::info(format!("Deleted {id}.")));
        }
        Err(e) => app.toast(Toast::error(format!("Cannot delete {id}: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_makes_kebab_ids() {
        assert_eq!(slug("My Cameo"), "my-cameo");
        assert_eq!(slug("  Cameo 2 -- fast!! "), "cameo-2-fast");
        assert_eq!(slug("!!!"), "");
        assert_eq!(new_preset_id("Vellum", &["vellum"]), "user-vellum");
        assert_eq!(
            new_preset_id("Vellum", &["user-vellum", "user-vellum-2"]),
            "user-vellum-3"
        );
        assert_eq!(new_preset_id("!!!", &[]), "");
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(parse_hex("#336699"), Some([0x33, 0x66, 0x99]));
        assert_eq!(parse_hex("336699"), Some([0x33, 0x66, 0x99]));
        assert_eq!(parse_hex("#3366"), None);
        assert_eq!(hex([0x33, 0x66, 0x99]), "#336699");
    }

    #[test]
    fn system_presets_pass_the_checks() {
        let dir = tempfile::tempdir().unwrap();
        let store = flatcraft_device::ProfileStore::load(dir.path()).unwrap();
        for p in store.plotters() {
            let p = store.plotter(&p.id).unwrap();
            assert_eq!(plotter_problems(&p), Vec::<String>::new(), "{}", p.id);
        }
        for m in store.materials() {
            let m = store.material(&m.id).unwrap();
            assert_eq!(material_problems(&m), Vec::<String>::new(), "{}", m.id);
        }
    }

    #[test]
    fn reverse_alternate_passes_has_a_hover_card() {
        let card = Setting::ReverseAlternatePasses.card();
        assert!(card.contains("every second pass"), "{card}");
        assert!(card.contains("lift"), "{card}");
        assert!(!card.contains('\u{2014}'), "{card}");
    }

    #[test]
    fn fresh_ids_skip_taken_ones() {
        assert_eq!(fresh_mat_id("my-mat", &["my-mat", "my-mat-2"]), "my-mat-3");
        assert_eq!(fresh_mat_id("a", &[]), "a");
    }
}
