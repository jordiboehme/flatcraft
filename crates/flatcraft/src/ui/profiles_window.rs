//! The Profiles window: plotter and material presets, the mats of each
//! plotter, and the app preferences. It opens from the Settings button and
//! the macOS Preferences menu (Cmd+,).
//!
//! System presets are read-only; "Save as user preset..." makes a user
//! preset that inherits from them. The widgets only edit a draft in
//! [`ProfilesState`] and push [`Request`]s; [`apply`] writes presets and
//! preferences under the session lock, one short step each, so the MCP
//! thread never sees half a change. No registry command changes the
//! preferences: this window is the only way (agent limit).

use std::path::PathBuf;

use egui::{Color32, Response, RichText, Sense, Ui, vec2};
use flatcraft_device::profile::MAX_SIZE_MM;
use flatcraft_device::{
    DEFAULT_MAT_COLOR, GpglDialect, Grid, KnifeCompensation, MaterialOverride, MaterialProfile,
    Media, MediaField, MediaKind, PlotterProfile, PresetInfo, Protocol, TransportSpec,
};
use flatcraft_engine::prefs::Theme;

use super::settings::set_name;
use super::theme::{self, Tokens};
use super::{Action, FlatCraftApp, Toast, canvas, files, fonts, lock};

/// Help text of the auto-reload switch.
pub const AUTO_RELOAD_HELP: &str = "When an imported file changes on disk, FlatCraft reloads it and keeps your placement and cut settings.";

/// Why a mat cannot be hidden or deleted.
pub const MAT_IN_USE: &str = "This mat is in use. Pick another mat in the job first.";

/// Warning under "Allow agents to start cuts".
pub const AGENT_CUTS_WARNING: &str = "With this on, an agent can start a USB cut without asking you first. Turn it on only when the mat is loaded and you trust the agent. File Output never needs it.";

/// The tabs of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Plotters,
    Materials,
    App,
    Calibration,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Plotters, Tab::Materials, Tab::App, Tab::Calibration];

    pub fn name(self) -> &'static str {
        match self {
            Tab::Plotters => "Plotters",
            Tab::Materials => "Materials",
            Tab::App => "App",
            Tab::Calibration => "Calibration",
        }
    }
}

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
    pub tab: Tab,
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
    /// Close the window at the end of this frame.
    closing: bool,
    /// A message for a toast, from a widget that cannot reach the app.
    notice: Option<String>,
    /// The text of the hex colour field while it has the keyboard.
    hex: String,
    hex_editing: bool,
}

/// What waits for "Discard your changes?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leave {
    /// Show this plotter preset.
    Plotter(String),
    /// Show this material preset.
    Material(String),
    /// Close the window.
    Close,
}

/// One change of the preferences. Each is stored right away.
#[derive(Debug, Clone, PartialEq)]
pub enum PrefsChange {
    AllowAgentCuts(bool),
    Theme(Theme),
    DefaultPlotter(String),
    DefaultMaterial(String),
    OutputDir(Option<PathBuf>),
    AutoReload(bool),
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
    Prefs(PrefsChange),
    ChooseOutputFolder,
}

/// What the window reads from the session, copied under one lock.
struct View {
    plotters: Vec<PresetInfo>,
    materials: Vec<PresetInfo>,
    allow_agent_cuts: bool,
    theme: Theme,
    default_plotter: String,
    default_material: String,
    output_dir: Option<PathBuf>,
    auto_reload: bool,
    /// (plotter id, mat id) pairs open jobs and new documents use.
    mats_in_use: Vec<(String, String)>,
    /// User preset files the store could not read.
    diagnostics: Vec<flatcraft_device::PresetDiagnostic>,
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
        flatcraft_engine::session::DEFAULT_MEDIA.to_string(),
    ));
    out
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
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// A colour from `#RRGGBB` (the `#` may be left out).
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim();
    let h = h.strip_prefix('#').unwrap_or(h);
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
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

/// Draws the window when it is open. Loads a newly selected preset first.
pub fn show(ctx: &egui::Context, app: &mut FlatCraftApp, actions: &mut Vec<Action>) {
    let view = load(app);
    let t = app.tokens;
    let st = &mut app.ui.profiles;
    let mut open = true;
    let screen = ctx.content_rect();
    egui::Window::new("Profiles")
        .id(egui::Id::unique("flatcraft-profiles"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([1080.0, 680.0])
        .default_pos(screen.center() - vec2(540.0, 360.0))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for tab in Tab::ALL {
                    let on = st.tab == tab;
                    let r = ui.add(egui::Button::selectable(on, tab.name()));
                    r.widget_info(|| {
                        egui::WidgetInfo::selected(egui::accesskit::Role::Tab, true, on, tab.name())
                    });
                    if r.clicked() {
                        st.tab = tab;
                    }
                }
            });
            ui.separator();
            for d in &view.diagnostics {
                ui.label(
                    RichText::new(format!(
                        "This preset file can't be read: {}",
                        d.reason
                    ))
                    .color(t.danger),
                )
                .on_hover_text(d.path.display().to_string());
            }
            if st.leave.is_some() {
                discard_banner(ui, st, &t);
            }
            match st.tab {
                Tab::Plotters => plotters_tab(ui, st, &view, &t, actions),
                Tab::Materials => materials_tab(ui, st, &view, &t, actions),
                Tab::App => app_tab(ui, &view, &t, actions),
                Tab::Calibration => {
                    ui.add_space(t.spacing);
                    ui.label("The calibration wizard arrives in milestone 3.");
                    ui.label(
                        RichText::new(
                            "The mat assistant, which finds where the printed grid of your mat starts, arrives there too.",
                        )
                        .color(t.text_dim),
                    );
                }
            }
        });
    save_as_dialog(ctx, st, &view, &t, actions);
    delete_dialog(ctx, st, &t, actions);
    let unsaved = st.plotter.as_ref().is_some_and(Draft::changed)
        || st.material.as_ref().is_some_and(Draft::changed);
    if !open && unsaved {
        st.leave = Some(Leave::Close);
    } else if !open || st.closing {
        st.closing = false;
        st.leave = None;
        st.save_as = None;
        st.confirm_delete = None;
        app.ui.show_profiles = false;
    }
    if let Some(n) = app.ui.profiles.notice.take() {
        app.toast(Toast::warning(n));
    }
}

/// "Discard your changes?" at the top of the window, for a preset switch
/// or Close while a draft has unsaved edits.
fn discard_banner(ui: &mut Ui, st: &mut ProfilesState, t: &Tokens) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, t.accent_cut))
        .corner_radius(t.radius)
        .inner_margin(egui::Margin::same(t.spacing as i8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Discard your changes?").color(t.accent_cut));
                if ui.button("Discard").clicked() {
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
                if ui.button("Keep editing").clicked() {
                    st.leave = None;
                }
            });
        });
    ui.add_space(t.spacing / 2.0);
}

/// Copies the lists and preferences, and loads a newly selected preset,
/// under one lock.
fn load(app: &mut FlatCraftApp) -> View {
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
        match (info, s.profiles.plotter(&id)) {
            (Some(info), Ok(p)) => {
                let parent_media = match &info.inherits {
                    Some(parent) => s
                        .profiles
                        .plotter(parent)
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
        match (info, s.profiles.material(&id)) {
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
        allow_agent_cuts: s.prefs.allow_agent_cuts,
        theme: s.prefs.theme,
        default_plotter: s.prefs.default_plotter.clone(),
        default_material: s.prefs.default_material.clone(),
        output_dir: s.prefs.output_dir.clone(),
        auto_reload: s.prefs.auto_reload,
        diagnostics: s.profiles.diagnostics().to_vec(),
    }
}

/// The name of the selected preset and where it comes from. The name is
/// painted, so the preset list keeps the only node with that name.
fn header(ui: &mut Ui, name: &str, parent: Option<&str>, system: bool, t: &Tokens) {
    let galley = ui
        .painter()
        .layout_no_wrap(name.to_string(), fonts::semibold(15.0), t.text);
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.left_top(), galley, t.text);
    let origin = match (parent, system) {
        (Some(p), true) => format!("System preset, based on {p}"),
        (None, true) => "System preset".to_string(),
        (Some(p), false) => format!("User preset, inherits from {p}"),
        (None, false) => "User preset".to_string(),
    };
    ui.label(RichText::new(origin).color(t.text_dim));
    ui.add_space(t.spacing / 2.0);
}

/// Text painted without an accessibility node, so the widget next to it
/// is the only node with that name. `dot` marks a modified field.
fn paint_text(ui: &mut Ui, text: &str, color: Color32, dot: Option<Color32>) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let extra = if dot.is_some() { 12.0 } else { 0.0 };
    let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(extra, 0.0), Sense::hover());
    ui.painter().galley(rect.left_top(), galley, color);
    if let Some(c) = dot {
        ui.painter()
            .circle_filled(egui::pos2(rect.right() - 4.0, rect.center().y), 3.5, c);
    }
}

/// Rows of a form with "modified" dots from the modified fields of the
/// stored preset.
struct Form<'a> {
    modified: &'a [String],
    t: &'a Tokens,
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

    /// A label cell, the widget, its accessible name, end of row.
    fn row(&self, ui: &mut Ui, label: &str, key: &str, add: impl FnOnce(&mut Ui) -> Response) {
        let dot = self.is_modified(key).then_some(self.t.accent_prepare);
        paint_text(ui, label, ui.visuals().text_color(), dot);
        let r = add(ui);
        set_name(ui, &r, &self.name(label, key));
        ui.end_row();
    }

    /// A checkbox in the widget column.
    fn check(&self, ui: &mut Ui, label: &str, key: &str, v: &mut bool) {
        let dot = self.is_modified(key).then_some(self.t.accent_prepare);
        paint_text(ui, "", ui.visuals().text_color(), dot);
        let r = ui.checkbox(v, label);
        set_name(ui, &r, &self.name(label, key));
        ui.end_row();
    }
}

fn num(v: &mut f64, decimals: usize) -> egui::DragValue<'_> {
    egui::DragValue::new(v)
        .speed(0.01)
        .max_decimals(decimals)
        .update_while_editing(false)
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
                .font(fonts::semibold(13.0))
                .color(t.text_dim),
        );
        let mut any = false;
        for p in presets.iter().filter(|p| p.system == system) {
            any = true;
            let on = selected == Some(p.id.as_str());
            if ui.add(egui::Button::selectable(on, &p.name)).clicked() && !on {
                picked = Some(p.id.clone());
            }
        }
        if !any {
            ui.label(RichText::new("None yet. Save one from a system preset.").color(t.text_dim));
        }
        ui.add_space(t.spacing);
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
            let ok = problems.is_empty() && changed && has_parent;
            if ui
                .add_enabled(ok, egui::Button::new("Save preset"))
                .clicked()
            {
                actions.push(Action::Profiles(save()));
            }
        }
        // Save as writes too, so it waits for valid fields like Save.
        if ui
            .add_enabled(
                problems.is_empty(),
                egui::Button::new("Save as user preset..."),
            )
            .clicked()
        {
            *st_save_as = Some(SaveAs {
                kind: Some(kind),
                name: format!("My {name}"),
                error: None,
            });
        }
        if changed && ui.button("Revert").clicked() {
            *revert = true;
        }
        if !system && ui.button("Delete preset").clicked() {
            *st_delete = Some((kind, id.to_string(), name.to_string()));
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

fn plotters_tab(
    ui: &mut Ui,
    st: &mut ProfilesState,
    view: &View,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let height = ui.available_height();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(190.0);
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
        ui.separator();
        let Some(d) = st.plotter.as_mut() else {
            if let Some((Kind::Plotter, id, e)) = &st.load_error {
                ui.label(RichText::new(format!("Cannot load {id}: {e}")).color(t.danger));
            }
            return;
        };
        let problems = plotter_problems(&d.edit);
        let mut revert = false;
        ui.vertical(|ui| {
            ui.set_width(360.0);
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
                Kind::Plotter,
                &id,
                &name,
                system,
                parent.is_some(),
                d.changed(),
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
            egui::ScrollArea::vertical()
                .id_salt("flatcraft-profiles-plotter-form")
                .max_height(ui.available_height())
                .show(ui, |ui| plotter_form(ui, d, t));
        });
        ui.separator();
        ui.vertical(|ui| {
            egui::ScrollArea::vertical()
                .id_salt("flatcraft-profiles-mats")
                .max_height(height)
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
                        &mut st.notice,
                        &st.parent_media,
                        &mut st.mat,
                        &mut st.hex,
                        &mut st.hex_editing,
                        t,
                    )
                });
        });
        if revert {
            d.edit = d.saved.clone();
        }
    });
}

fn plotter_form(ui: &mut Ui, d: &mut Draft<PlotterProfile>, t: &Tokens) {
    let f = Form {
        modified: &d.modified,
        t,
    };
    let p = &mut d.edit;
    ui.add_enabled_ui(!d.system, |ui| {
        egui::Grid::new("flatcraft-profiles-plotter")
            .num_columns(2)
            .spacing([t.spacing, t.spacing * 0.6])
            .show(ui, |ui| {
                f.row(ui, "Name", "name", |ui| {
                    ui.text_edit_singleline(&mut p.name)
                });
                f.row(ui, "Vendor", "vendor", |ui| {
                    ui.text_edit_singleline(&mut p.vendor)
                });
                f.row(ui, "Model", "model", |ui| {
                    ui.text_edit_singleline(&mut p.model)
                });
                f.row(ui, "Protocol", "protocol", |ui| {
                    let mut v = 0;
                    combo(ui, "protocol", &mut v, &[(0, "GPGL")])
                });
                f.row(ui, "Dialect", "protocol", |ui| {
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
                f.row(ui, "Transport", "transport", |ui| {
                    combo(ui, "transport", &mut usb, &[(true, "USB"), (false, "File")])
                });
                match (&mut p.transport, usb) {
                    (TransportSpec::Usb { vid, pid }, true) => {
                        f.row(ui, "USB vendor id", "transport", |ui| {
                            ui.add(egui::DragValue::new(vid).hexadecimal(4, false, true))
                        });
                        f.row(ui, "USB product id", "transport", |ui| {
                            ui.add(egui::DragValue::new(pid).hexadecimal(4, false, true))
                        });
                    }
                    (
                        TransportSpec::File {
                            directory,
                            extension,
                        },
                        false,
                    ) => {
                        f.row(ui, "Output folder", "transport", |ui| {
                            let mut text = directory
                                .as_ref()
                                .map(|d| d.display().to_string())
                                .unwrap_or_default();
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut text)
                                    .hint_text("The App tab's folder"),
                            );
                            if r.changed() {
                                *directory = (!text.trim().is_empty()).then(|| text.into());
                            }
                            r
                        });
                        f.row(ui, "File extension", "transport", |ui| {
                            ui.text_edit_singleline(extension)
                        });
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
                f.row(ui, "Units per mm", "units_per_mm", |ui| {
                    ui.add(num(&mut p.units_per_mm, 3))
                });
                f.check(ui, "Mirror x", "axis.mirror_x", &mut p.axis.mirror_x);
                f.check(ui, "Mirror y", "axis.mirror_y", &mut p.axis.mirror_y);
                f.row(ui, "Scale x", "scale_x", |ui| {
                    ui.add(num(&mut p.scale_x, 5).speed(0.0005))
                });
                f.row(ui, "Scale y", "scale_y", |ui| {
                    ui.add(num(&mut p.scale_y, 5).speed(0.0005))
                });
                knife(ui, &f, p);
                f.row(ui, "Blade offset (mm)", "blade_offset_mm", |ui| {
                    ui.add(num(&mut p.blade_offset_mm, 3))
                });
                f.row(ui, "Speed min", "speed_range", |ui| {
                    ui.add(egui::DragValue::new(&mut p.speed_range[0]).update_while_editing(false))
                });
                f.row(ui, "Speed max", "speed_range", |ui| {
                    ui.add(egui::DragValue::new(&mut p.speed_range[1]).update_while_editing(false))
                });
                f.row(ui, "Force min", "force_range", |ui| {
                    ui.add(egui::DragValue::new(&mut p.force_range[0]).update_while_editing(false))
                });
                f.row(ui, "Force max", "force_range", |ui| {
                    ui.add(egui::DragValue::new(&mut p.force_range[1]).update_while_editing(false))
                });
                f.row(ui, "Passes min", "passes_range", |ui| {
                    ui.add(egui::DragValue::new(&mut p.passes_range[0]).update_while_editing(false))
                });
                f.row(ui, "Passes max", "passes_range", |ui| {
                    ui.add(egui::DragValue::new(&mut p.passes_range[1]).update_while_editing(false))
                });
                let c = &mut p.capabilities;
                f.check(
                    ui,
                    "Track enhancing",
                    "capabilities.track_enhancing",
                    &mut c.track_enhancing,
                );
                f.check(
                    ui,
                    "Registration marks",
                    "capabilities.registration_marks",
                    &mut c.registration_marks,
                );
                f.check(
                    ui,
                    "Auto blade",
                    "capabilities.auto_blade",
                    &mut c.auto_blade,
                );
                f.row(ui, "Flatten tolerance (mm)", "flatten_tolerance_mm", |ui| {
                    ui.add(num(&mut p.flatten_tolerance_mm, 4).speed(0.001))
                });
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
    f.row(ui, "Knife compensation", "knife_compensation", |ui| {
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
    });
    if k != before {
        p.knife_compensation = match k {
            K::Firmware => KnifeCompensation::Firmware { fc: 18 },
            K::Host => KnifeCompensation::Host {
                offset_mm: p.blade_offset_mm,
                pivot_threshold_deg: 10.0,
            },
            K::None => KnifeCompensation::None,
        };
    }
    match &mut p.knife_compensation {
        KnifeCompensation::Firmware { fc } => {
            f.row(ui, "FC value", "knife_compensation", |ui| {
                ui.add(egui::DragValue::new(fc).update_while_editing(false))
            });
        }
        KnifeCompensation::Host {
            offset_mm,
            pivot_threshold_deg,
        } => {
            f.row(ui, "Host offset (mm)", "knife_compensation", |ui| {
                ui.add(num(offset_mm, 3))
            });
            f.row(ui, "Pivot threshold (deg)", "knife_compensation", |ui| {
                ui.add(num(pivot_threshold_deg, 1).speed(0.5))
            });
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
    t: &Tokens,
) {
    let f = Form {
        modified: &d.modified,
        t,
    };
    ui.label(RichText::new("Mats").font(fonts::semibold(15.0)));
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
        let r = ui.add(egui::Button::selectable(on, label));
        if r.clicked() {
            *selected = Some(m.id.clone());
            *hex_editing = false;
        }
    }
    ui.add_enabled_ui(!system, |ui| {
        ui.horizontal(|ui| {
            let owned: Vec<String> = media
                .iter()
                .chain(parent.iter())
                .map(|m| m.id.clone())
                .collect();
            let taken: Vec<&str> = owned.iter().map(String::as_str).collect();
            if ui.button("Add mat").clicked() {
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
                    color: DEFAULT_MAT_COLOR,
                });
                *selected = Some(id);
            }
            let sel = selected
                .as_ref()
                .and_then(|s| media.iter().position(|m| &m.id == s));
            if ui
                .add_enabled(sel.is_some(), egui::Button::new("Copy mat"))
                .clicked()
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
            if let Some(i) = own
                && ui.button("Delete mat").clicked()
            {
                if in_use.contains(&media[i].id.as_str()) {
                    *notice = Some(MAT_IN_USE.into());
                } else {
                    media.remove(i);
                    *selected = media.first().map(|m| m.id.clone());
                }
            }
        });
    });
    ui.add_space(t.spacing / 2.0);

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
    ui.horizontal_top(|ui| {
        ui.add_enabled_ui(!system, |ui| {
            egui::Grid::new("flatcraft-profiles-mat")
                .num_columns(2)
                .spacing([t.spacing, t.spacing * 0.6])
                .show(ui, |ui| {
                    f.row(ui, "Mat name", "name", |ui| {
                        ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(160.0))
                    });
                    f.row(ui, "Mat kind", "kind", |ui| {
                        combo(
                            ui,
                            "mat-kind",
                            &mut m.kind,
                            &[(MediaKind::Mat, "Mat"), (MediaKind::NoMat, "No mat")],
                        )
                    });
                    f.row(ui, "Physical width (mm)", "width", |ui| {
                        ui.add(num(&mut m.width_mm, 2))
                    });
                    show_problem(ui, MediaField::Width);
                    f.row(ui, "Physical height (mm)", "height", |ui| {
                        ui.add(num(&mut m.height_mm, 2))
                    });
                    show_problem(ui, MediaField::Height);
                    f.row(ui, "Grid origin x (mm)", "origin_x", |ui| {
                        ui.add(num(&mut m.grid.origin_x_mm, 2))
                    });
                    show_problem(ui, MediaField::OriginX);
                    f.row(ui, "Grid origin y (mm)", "origin_y", |ui| {
                        ui.add(num(&mut m.grid.origin_y_mm, 2))
                    });
                    show_problem(ui, MediaField::OriginY);
                    f.row(ui, "Grid width (mm)", "grid_width", |ui| {
                        ui.add(num(&mut m.grid.width_mm, 2))
                    });
                    show_problem(ui, MediaField::GridWidth);
                    f.row(ui, "Grid height (mm)", "grid_height", |ui| {
                        ui.add(num(&mut m.grid.height_mm, 2))
                    });
                    show_problem(ui, MediaField::GridHeight);
                    f.check(ui, "Calibrated", "calibrated", &mut m.calibrated);
                    f.row(ui, "Mat colour", "color", |ui| {
                        ui.color_edit_button_srgb(&mut m.color)
                    });
                    f.row(ui, "Mat colour (hex)", "color", |ui| {
                        if !*hex_editing {
                            *hex_text = hex(m.color);
                        }
                        let r = ui.add(egui::TextEdit::singleline(hex_text).desired_width(90.0));
                        *hex_editing = r.has_focus();
                        if (r.changed() || r.lost_focus())
                            && let Some(c) = parse_hex(hex_text)
                        {
                            m.color = c;
                        }
                        r
                    });
                });
        });
        mat_drawing(ui, m, t);
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
fn mat_drawing(ui: &mut Ui, m: &Media, t: &Tokens) {
    let size = vec2(150.0, 200.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let (w, h) = (m.width_mm, m.height_mm);
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return;
    }
    let scale = (f64::from(size.x) / w).min(f64::from(size.y) / h) as f32;
    let mat = egui::Rect::from_min_size(rect.min, vec2(w as f32 * scale, h as f32 * scale));
    let [r, g, b] = m.color;
    let painter = ui.painter_at(rect);
    painter.rect_filled(mat, 2.0, Color32::from_rgb(r, g, b));
    let [cr, cg, cb] = canvas::contrast_colour(m.color);
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
        fonts::medium(11.0),
        ink,
    );
}

fn materials_tab(
    ui: &mut Ui,
    st: &mut ProfilesState,
    view: &View,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let height = ui.available_height();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(190.0);
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
                .show(ui, |ui| material_form(ui, d, view, t));
        });
        if revert {
            d.edit = d.saved.clone();
        }
    });
}

fn material_form(ui: &mut Ui, d: &mut Draft<MaterialProfile>, view: &View, t: &Tokens) {
    let f = Form {
        modified: &d.modified,
        t,
    };
    let m = &mut d.edit;
    ui.add_enabled_ui(!d.system, |ui| {
        egui::Grid::new("flatcraft-profiles-material")
            .num_columns(2)
            .spacing([t.spacing, t.spacing * 0.6])
            .show(ui, |ui| {
                f.row(ui, "Name", "name", |ui| {
                    ui.text_edit_singleline(&mut m.name)
                });
                f.row(ui, "Speed", "speed", |ui| {
                    ui.add(egui::DragValue::new(&mut m.speed).update_while_editing(false))
                });
                f.row(ui, "Force", "force", |ui| {
                    ui.add(egui::DragValue::new(&mut m.force).update_while_editing(false))
                });
                f.row(ui, "Passes", "passes", |ui| {
                    ui.add(egui::DragValue::new(&mut m.passes).update_while_editing(false))
                });
                f.check(
                    ui,
                    "Reverse alternate passes",
                    "reverse_alternate_passes",
                    &mut m.reverse_alternate_passes,
                );
                f.row(ui, "Overcut (mm)", "overcut_mm", |ui| {
                    ui.add(num(&mut m.overcut_mm, 2))
                });
                f.check(
                    ui,
                    "Track enhancing",
                    "track_enhancing",
                    &mut m.track_enhancing,
                );
                f.row(ui, "Blade depth hint", "blade_depth_hint", |ui| {
                    let mut text = m.blade_depth_hint.clone().unwrap_or_default();
                    let r = ui.text_edit_singleline(&mut text);
                    if r.changed() {
                        m.blade_depth_hint = (!text.is_empty()).then_some(text);
                    }
                    r
                });
            });
        ui.add_space(t.spacing);
        ui.label(RichText::new("Per-plotter overrides").font(fonts::semibold(14.0)));
        ui.label(
            RichText::new("Values that differ on one plotter. The rest come from above.")
                .color(t.text_dim),
        );
        overrides(ui, &f, m, view, t);
    });
}

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
        .spacing([t.spacing, t.spacing * 0.6])
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
                    &mut o.speed,
                    base.0,
                );
                cell(
                    ui,
                    f,
                    &format!("{name} force"),
                    &format!("{key}.force"),
                    &mut o.force,
                    base.1,
                );
                cell(
                    ui,
                    f,
                    &format!("{name} passes"),
                    &format!("{key}.passes"),
                    &mut o.passes,
                    base.2,
                );
                ui.horizontal(|ui| {
                    let label = format!("{name} overcut");
                    let mut on = o.overcut_mm.is_some();
                    let r = ui.checkbox(&mut on, "");
                    set_name(ui, &r, &format!("{label} set"));
                    if r.changed() {
                        o.overcut_mm = on.then_some(base.3);
                    }
                    let mut v = o.overcut_mm.unwrap_or(base.3);
                    let r = ui.add_enabled(on, num(&mut v, 2));
                    set_name(ui, &r, &f.name(&label, &format!("{key}.overcut_mm")));
                    if on {
                        o.overcut_mm = Some(v);
                    }
                });
                if ui.button(format!("Remove override for {name}")).clicked() {
                    remove = Some(pid.clone());
                }
                ui.end_row();
            }
        });
    if let Some(pid) = remove {
        m.overrides.remove(&pid);
    }
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
        if let Some(id) = add {
            m.overrides.insert(id, MaterialOverride::default());
        }
    }
}

/// A checkbox ("<label> set") and a value that only counts while it is on.
fn cell<T: egui::emath::Numeric>(
    ui: &mut Ui,
    f: &Form<'_>,
    label: &str,
    key: &str,
    v: &mut Option<T>,
    base: T,
) {
    ui.horizontal(|ui| {
        let mut on = v.is_some();
        let r = ui.checkbox(&mut on, "");
        set_name(ui, &r, &format!("{label} set"));
        if r.changed() {
            *v = on.then_some(base);
        }
        let mut x = v.unwrap_or(base);
        let r = ui.add_enabled(on, egui::DragValue::new(&mut x).update_while_editing(false));
        set_name(ui, &r, &f.name(label, key));
        if on {
            *v = Some(x);
        }
    });
}

fn app_tab(ui: &mut Ui, view: &View, t: &Tokens, actions: &mut Vec<Action>) {
    let mut reqs: Vec<Request> = Vec::new();
    let dim = t.text_dim;
    ui.add_space(t.spacing / 2.0);
    let mut allow = view.allow_agent_cuts;
    if ui
        .checkbox(&mut allow, "Allow agents to start cuts")
        .changed()
    {
        reqs.push(Request::Prefs(PrefsChange::AllowAgentCuts(allow)));
    }
    let warn = if allow { t.accent_cut } else { dim };
    ui.label(RichText::new(AGENT_CUTS_WARNING).color(warn));
    ui.add_space(t.spacing);

    let mut reload = view.auto_reload;
    if ui
        .checkbox(&mut reload, "Reload changed files automatically")
        .changed()
    {
        reqs.push(Request::Prefs(PrefsChange::AutoReload(reload)));
    }
    ui.label(RichText::new(AUTO_RELOAD_HELP).color(dim));
    ui.add_space(t.spacing);

    egui::Grid::new("flatcraft-profiles-app")
        .num_columns(2)
        .spacing([t.spacing, t.spacing * 0.75])
        .show(ui, |ui| {
            paint_text(ui, "Theme", ui.visuals().text_color(), None);
            let mut theme = view.theme;
            let r = combo(
                ui,
                "theme",
                &mut theme,
                &[(Theme::Dark, "Dark"), (Theme::Light, "Light")],
            );
            set_name(ui, &r, "Theme");
            if theme != view.theme {
                reqs.push(Request::Prefs(PrefsChange::Theme(theme)));
            }
            ui.end_row();

            paint_text(ui, "Default device", ui.visuals().text_color(), None);
            let mut device = view.default_plotter.clone();
            let choices: Vec<(String, &str)> = view
                .plotters
                .iter()
                .map(|p| (p.id.clone(), p.name.as_str()))
                .collect();
            let r = combo(ui, "default-device", &mut device, &choices);
            set_name(ui, &r, "Default device");
            if device != view.default_plotter {
                reqs.push(Request::Prefs(PrefsChange::DefaultPlotter(device)));
            }
            ui.end_row();

            paint_text(ui, "Default material", ui.visuals().text_color(), None);
            let mut material = view.default_material.clone();
            let choices: Vec<(String, &str)> = view
                .materials
                .iter()
                .map(|m| (m.id.clone(), m.name.as_str()))
                .collect();
            let r = combo(ui, "default-material", &mut material, &choices);
            set_name(ui, &r, "Default material");
            if material != view.default_material {
                reqs.push(Request::Prefs(PrefsChange::DefaultMaterial(material)));
            }
            ui.end_row();

            paint_text(ui, "File Output folder", ui.visuals().text_color(), None);
            ui.vertical(|ui| {
                let shown = view.output_dir.as_ref().map_or_else(
                    || "Documents/FlatCraft Output (the default)".to_string(),
                    |d| d.display().to_string(),
                );
                ui.label(shown);
                ui.horizontal(|ui| {
                    if ui.button("Choose folder...").clicked() {
                        reqs.push(Request::ChooseOutputFolder);
                    }
                    if ui
                        .add_enabled(
                            view.output_dir.is_some(),
                            egui::Button::new("Use the default"),
                        )
                        .clicked()
                    {
                        reqs.push(Request::Prefs(PrefsChange::OutputDir(None)));
                    }
                });
            });
            ui.end_row();
        });
    ui.label(
        RichText::new(
            "New documents start with the default device and material. A File Output plotter profile with its own folder writes there instead.",
        )
        .color(dim),
    );
    actions.extend(reqs.into_iter().map(Action::Profiles));
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
        ui.set_width(360.0);
        ui.label(RichText::new("Save as user preset").font(fonts::semibold(15.0)));
        ui.add_space(t.spacing / 2.0);
        let r = ui.text_edit_singleline(&mut dialog.name);
        set_name(ui, &r, "Preset name");
        if let Some(e) = &dialog.error {
            ui.label(RichText::new(e).color(t.danger));
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            if ui.button("Create").clicked() || enter {
                let name = dialog.name.trim().to_string();
                let id = slug(&name);
                // Plotters and materials live in their own folders, so an
                // id only has to be free within its kind.
                let same_kind = match kind {
                    Kind::Plotter => &view.plotters,
                    Kind::Material => &view.materials,
                };
                let taken = same_kind.iter().any(|p| p.id == id);
                if id.is_empty() {
                    dialog.error =
                        Some("Give the preset a name with at least one letter or digit.".into());
                } else if taken {
                    dialog.error = Some(format!(
                        "A preset with the id '{id}' already exists. Pick another name."
                    ));
                } else if let Some(r) =
                    save_as_request(st_draft(kind, &st.plotter, &st.material), id, name)
                {
                    actions.push(Action::Profiles(r));
                    close = true;
                }
            }
            if ui.button("Cancel").clicked() {
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
        ui.set_width(360.0);
        ui.label(RichText::new(format!("Delete {name}?")).font(fonts::semibold(15.0)));
        ui.label(RichText::new("Its file is removed. This cannot be undone.").color(t.text_dim));
        ui.add_space(t.spacing);
        ui.horizontal(|ui| {
            if ui.button("Yes, delete").clicked() {
                actions.push(Action::Profiles(Request::Delete(kind, id.clone())));
                close = true;
            }
            if ui.button("Keep it").clicked() {
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
pub fn apply(app: &mut FlatCraftApp, ctx: &egui::Context, req: Request) {
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
                } else if new && s.profiles.plotters().iter().any(|p| p.id == profile.id) {
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
                } else if new && s.profiles.materials().iter().any(|p| p.id == profile.id) {
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
        Request::Prefs(change) => set_pref(app, ctx, change),
        Request::ChooseOutputFolder => {
            let current = lock(&app.session).prefs.output_dir.clone();
            let req = files::FileRequest {
                title: "File Output folder".into(),
                directory: current,
                ..Default::default()
            };
            if let Some(dir) = app.files.pick_folder(&req) {
                set_pref(app, ctx, PrefsChange::OutputDir(Some(dir)));
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

fn set_pref(app: &mut FlatCraftApp, ctx: &egui::Context, change: PrefsChange) {
    let theme = match &change {
        PrefsChange::Theme(t) => Some(*t),
        _ => None,
    };
    let result = {
        let mut s = lock(&app.session);
        let p = &mut s.prefs;
        match change {
            PrefsChange::AllowAgentCuts(v) => p.allow_agent_cuts = v,
            PrefsChange::Theme(v) => p.theme = v,
            PrefsChange::DefaultPlotter(v) => p.default_plotter = v,
            PrefsChange::DefaultMaterial(v) => p.default_material = v,
            PrefsChange::OutputDir(v) => p.output_dir = v,
            PrefsChange::AutoReload(v) => p.auto_reload = v,
        }
        s.prefs.save(&app.config_dir)
    };
    if let Some(theme) = theme {
        app.tokens = Tokens::for_theme(theme);
        theme::apply(ctx, &app.tokens);
    }
    if let Err(e) = result {
        app.toast(Toast::error(format!(
            "Cannot store the preferences: {e}. The change holds until FlatCraft closes."
        )));
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
    fn fresh_ids_skip_taken_ones() {
        assert_eq!(fresh_mat_id("my-mat", &["my-mat", "my-mat-2"]), "my-mat-3");
        assert_eq!(fresh_mat_id("a", &[]), "a");
    }
}
