//! Collapsible side panels. Every side panel (the Prepare Objects panel,
//! the Prepare Cut and Job panel and the Cut workspace's stage panel) has
//! a chevron in its header that collapses it into a [`RAIL_W`] wide rail.
//! The rail is one button: a click anywhere on it opens the panel again.
//! It shows the arrow, a state dot (right rails), and a one-line summary
//! turned on its side, starting right under the arrow. The left rail reads
//! bottom to top, the right rails top to bottom. A summary that is too long
//! ends in "..." (the tooltip holds all of it), measured so it never runs
//! into the dot or the arrow.
//!
//! A panel also collapses on its own when the window is narrow
//! ([`auto_collapsed`]). What the person chose by hand ([`PanelUser`]) wins
//! over the width until the person uses the opposite control, and is kept
//! in the preferences.

use egui::epaint::TextShape;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, vec2};
use flatcraft_core::document::CutMode;
use flatcraft_engine::prefs::{PanelUser, Preferences};

use super::cut::{CutState, Stage};
use super::fonts;
use super::groups;
use super::icons::{self, Icon};
use super::prepare::Snapshot;
use super::settings::set_name;
use super::theme::Tokens;

/// The width of a rail, in points.
pub const RAIL_W: f32 = 28.0;
/// The right panels collapse on their own below this window width.
pub const RIGHT_BELOW: f32 = 1100.0;
/// The Objects panel collapses on its own below this window width.
pub const LEFT_BELOW: f32 = 960.0;
/// The width of a docked Arrange panel. While one is open, the Cut and Job
/// panel collapses on its own below [`RIGHT_BELOW`] plus this width, so the
/// canvas keeps its room (about 570 pt at 900 pt, where it had about 100).
pub const ARRANGE_W: f32 = 270.0;

/// Which side panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidePanel {
    /// Prepare, left.
    Objects,
    /// Prepare, right: Cut and Job.
    Settings,
    /// Cut, right: the stage panel.
    Stage,
}

impl SidePanel {
    fn index(self) -> usize {
        match self {
            SidePanel::Objects => 0,
            SidePanel::Settings => 1,
            SidePanel::Stage => 2,
        }
    }

    /// The name of the panel in the words of the tooltips.
    pub fn title(self) -> &'static str {
        match self {
            SidePanel::Objects => "Objects",
            SidePanel::Settings => "Cut and Job",
            SidePanel::Stage => "Cut stage",
        }
    }

    fn what(self) -> &'static str {
        match self {
            SidePanel::Objects => "the list of everything on the mat and the project's notes",
            SidePanel::Settings => {
                "what is cut and how: cut mode, plotter, mat, material, speed and force"
            }
            SidePanel::Stage => "the settings and numbers of the current step",
        }
    }
}

/// Whether the window is too narrow for the panel, so it collapses on its
/// own. `arrange_open`: a docked Arrange panel takes [`ARRANGE_W`] on the
/// right, so the Cut and Job panel gives way earlier.
pub fn auto_collapsed(kind: SidePanel, window_w: f32, arrange_open: bool) -> bool {
    match kind {
        SidePanel::Objects => window_w < LEFT_BELOW,
        SidePanel::Settings => window_w < RIGHT_BELOW + if arrange_open { ARRANGE_W } else { 0.0 },
        SidePanel::Stage => window_w < RIGHT_BELOW,
    }
}

/// The collapse state of the three panels: what the person chose, and the
/// window the panels follow when they chose nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct Panels {
    users: [PanelUser; 3],
    saved: [PanelUser; 3],
    /// The window width of this frame, in points.
    pub window_w: f32,
    /// A docked Arrange panel is open (Prepare).
    pub arrange_open: bool,
}

impl Default for Panels {
    fn default() -> Self {
        Self {
            users: [PanelUser::None; 3],
            saved: [PanelUser::None; 3],
            window_w: 1400.0,
            arrange_open: false,
        }
    }
}

impl Panels {
    /// The choices stored in the preferences.
    pub fn from_prefs(p: &Preferences) -> Self {
        let users = [p.panel_objects, p.panel_settings, p.panel_stage];
        Self {
            users,
            saved: users,
            ..Self::default()
        }
    }

    pub fn user(&self, kind: SidePanel) -> PanelUser {
        self.users[kind.index()]
    }

    /// Whether the panel shows as a rail now.
    pub fn collapsed(&self, kind: SidePanel) -> bool {
        match self.user(kind) {
            PanelUser::Collapsed => true,
            PanelUser::Open => false,
            PanelUser::None => auto_collapsed(kind, self.window_w, self.arrange_open),
        }
    }

    /// The person collapsed (the chevron) or opened (the rail) the panel. The
    /// choice is kept only when it differs from what the window width
    /// gives.
    pub fn set_collapsed(&mut self, kind: SidePanel, collapsed: bool) {
        // A choice the window width would make anyway is no choice: the
        // panel follows the width again.
        let auto = auto_collapsed(kind, self.window_w, self.arrange_open);
        self.users[kind.index()] = if collapsed == auto {
            PanelUser::None
        } else if collapsed {
            PanelUser::Collapsed
        } else {
            PanelUser::Open
        };
    }

    /// The ribbon toggle: the opposite of what shows now.
    pub fn toggle(&mut self, kind: SidePanel) {
        let now = self.collapsed(kind);
        self.set_collapsed(kind, !now);
    }

    /// The choices when they changed since they were last stored; the
    /// caller stores them in the preferences.
    pub fn take_changed(&mut self) -> Option<[PanelUser; 3]> {
        (self.users != self.saved).then(|| {
            self.saved = self.users;
            self.users
        })
    }

    /// Takes the stored choices of `p` (the Settings window applied them),
    /// keeping the window width. They count as stored, so nothing is
    /// written back.
    pub fn adopt(&mut self, p: &Preferences) {
        self.users = [p.panel_objects, p.panel_settings, p.panel_stage];
        self.saved = self.users;
    }

    /// Stores the choices in a preferences value.
    pub fn write(users: [PanelUser; 3], p: &mut Preferences) {
        [p.panel_objects, p.panel_settings, p.panel_stage] = users;
    }
}

// ---------------------------------------------------------------- state dot

/// How the job stands, for the dot of a right rail; on the left rail,
/// that the project has notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dot {
    Ready,
    Warning,
    Nothing,
    /// The project has notes (the Objects rail).
    Notes,
}

impl Dot {
    /// What the dot says.
    pub fn label(self) -> &'static str {
        match self {
            Dot::Ready => "Ready to cut",
            Dot::Warning => "The job has warnings",
            Dot::Nothing => "Nothing to cut",
            Dot::Notes => "Has notes",
        }
    }

    fn colour(self, t: &Tokens) -> Color32 {
        match self {
            Dot::Ready => t.ok,
            Dot::Warning => t.warn,
            Dot::Nothing => t.text_dim,
            Dot::Notes => t.accent_prepare,
        }
    }
}

/// The one rule for the state dot, used by the Prepare and the Cut rails.
///
/// Grey: no visible object is set to be cut ("Nothing to cut"). Amber: a
/// real problem of the job: no plotter or material, a plotter or mat
/// problem, a cut object outside the cutting area, or a warning of the
/// job builder that is not the person's own choice. A hidden or a skipped
/// object is the person's choice and never turns it amber. Green:
/// otherwise.
///
/// The job is not built for the dot. Prepare reads what its snapshot
/// already knows; the Cut workspace passes its plan (`cut`), which is the
/// job builder's own answer (`JobResult::problem_warnings`,
/// `BuildFailure::nothing_to_cut`) and also sees problems the snapshot
/// cannot, such as a filled path too thin to cut.
pub fn job_dot(snap: &Snapshot, cut: Option<&CutState>) -> Dot {
    if let Some(cut) = cut
        && let Some(plan) = &cut.plan
    {
        return match plan {
            Ok(job) if job.problem_warnings() == 0 => Dot::Ready,
            Ok(_) => Dot::Warning,
            Err(_) if cut.nothing_to_cut => Dot::Nothing,
            Err(_) => Dot::Warning,
        };
    }
    if !snap.objects.iter().any(|o| o.visible && o.cut_enabled) {
        return Dot::Nothing;
    }
    if snap.plotter.is_none()
        || snap.material.is_none()
        || snap.profile_problem.is_some()
        || snap.mat_warning.is_some()
        || snap.outside_area() > 0
    {
        return Dot::Warning;
    }
    Dot::Ready
}

// ---------------------------------------------------------------- summaries

/// What a rail says: the parts of its text, the first one in semibold.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub parts: Vec<String>,
    pub dot: Option<Dot>,
}

impl Summary {
    /// The parts joined with " · ", and the dot's words after them. The
    /// tooltip and the accessible name hold this.
    pub fn full(&self) -> String {
        let mut text = self.parts.join(" · ");
        if let Some(d) = self.dot {
            if !text.is_empty() {
                text.push_str(" · ");
            }
            text.push_str(d.label());
        }
        text
    }
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The Objects rail: "<n> objects", then the counts that are not zero,
/// and a dot when the project has notes (a description alone is none).
pub fn objects_summary(snap: &Snapshot) -> Summary {
    let mut parts = vec![count(snap.objects.len(), "object", "objects")];
    let selected = snap.selection.len();
    let skipped = snap.objects.iter().filter(|o| !o.cut_enabled).count();
    let hidden = snap.objects.iter().filter(|o| !o.visible).count();
    for (n, what) in [
        (selected, "selected"),
        (skipped, "skipped"),
        (hidden, "hidden"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {what}"));
        }
    }
    let dot = (!snap.notes.trim().is_empty()).then_some(Dot::Notes);
    Summary { parts, dot }
}

fn mode_word(m: CutMode) -> &'static str {
    match m {
        CutMode::AllLines => "All lines",
        CutMode::ObjectOutlines => "Object outlines",
    }
}

/// The Cut and Job rail: the selection with its cut mode (or "mixed"),
/// then plotter, material, speed, force and passes.
pub fn settings_summary(snap: &Snapshot, dot: Dot) -> Summary {
    let mut parts = Vec::new();
    let selected: Vec<_> = snap.selected().collect();
    if let Some(first) = selected.first() {
        let mode = if selected.iter().all(|o| o.cut.mode == first.cut.mode) {
            mode_word(first.cut.mode)
        } else {
            "mixed"
        };
        parts.push(format!("{} selected: {mode}", selected.len()));
    }
    parts.push(
        snap.plotter
            .as_ref()
            .map_or(snap.job.plotter_id.clone(), |p| p.name.clone()),
    );
    parts.push(
        snap.material
            .as_ref()
            .map_or(snap.job.material_id.clone(), |m| m.name.clone()),
    );
    if let Some(m) = &snap.material {
        parts.push(format!("Speed {}", snap.job.speed.unwrap_or(m.speed)));
        parts.push(format!("Force {}", snap.job.force.unwrap_or(m.force)));
        parts.push(count(
            snap.job.passes.unwrap_or(m.passes) as usize,
            "pass",
            "passes",
        ));
    }
    Summary {
        parts,
        dot: Some(dot),
    }
}

/// The Cut workspace rail: the stage and its name.
pub fn stage_summary(stage: Stage, dot: Dot) -> Summary {
    Summary {
        parts: vec![
            format!("Stage {} of {}", stage.number(), Stage::ALL.len()),
            stage.name().to_string(),
        ],
        dot: Some(dot),
    }
}

// ---------------------------------------------------------------- drawing

/// The side a panel sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

const PAD: f32 = 6.0;
const DOT: f32 = 9.0;
const ARROW: f32 = 16.0;
const GAP: f32 = 8.0;
const FONT: f32 = 12.0;

/// One run of rail text.
struct Run {
    text: String,
    format: TextFormat,
}

fn runs(parts: &[String], t: &Tokens) -> Vec<Run> {
    let mut out = Vec::new();
    let format = |font, color| TextFormat {
        font_id: font,
        color,
        ..Default::default()
    };
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.push(Run {
                text: " \u{b7} ".into(),
                format: format(egui::FontId::proportional(FONT), t.text_dim),
            });
        }
        let font = if i == 0 {
            fonts::semibold(FONT)
        } else {
            egui::FontId::proportional(FONT)
        };
        out.push(Run {
            text: p.clone(),
            format: format(font, t.text),
        });
    }
    out
}

/// The text of the first `n` characters of `runs`, with "..." after it
/// when it is cut.
fn job_of(runs: &[Run], n: usize, total: usize) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut left = n;
    let mut last = None;
    for r in runs {
        if left == 0 {
            break;
        }
        let len = r.text.chars().count();
        let take = len.min(left);
        let text: String = r.text.chars().take(take).collect();
        job.append(&text, 0.0, r.format.clone());
        last = Some(r.format.clone());
        left -= take;
    }
    if n < total {
        let f = last.unwrap_or_else(|| runs[0].format.clone());
        job.append("...", 0.0, f);
    }
    job
}

/// The rail's text as a galley no wider than `avail`: the whole text, or
/// the longest start of it that fits with "..." after it. `None` when not
/// even "..." fits.
fn fit(ui: &Ui, parts: &[String], t: &Tokens, avail: f32) -> Option<std::sync::Arc<egui::Galley>> {
    let runs = runs(parts, t);
    if runs.is_empty() {
        return None;
    }
    let total: usize = runs.iter().map(|r| r.text.chars().count()).sum();
    let galley = |n: usize| ui.painter().layout_job(job_of(&runs, n, total));
    let full = galley(total);
    if full.size().x <= avail {
        return Some(full);
    }
    // The widest start that fits: the width only grows with the length.
    let (mut lo, mut hi) = (0, total);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if galley(mid).size().x <= avail {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let g = galley(lo);
    (g.size().x <= avail).then_some(g)
}

/// The geometry of a rail, in points: the arrow, the dot (right rails) and
/// the room left for the text, all in one column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RailLayout {
    pub arrow: Rect,
    pub dot: Option<Pos2>,
    /// Where the text starts (its top) and how long it may be.
    pub text_top: f32,
    pub text_len: f32,
}

pub fn layout(rect: Rect, has_dot: bool) -> RailLayout {
    let cx = rect.center().x;
    let arrow = Rect::from_center_size(
        Pos2::new(cx, rect.top() + PAD + ARROW / 2.0),
        vec2(ARROW, ARROW),
    );
    let mut y = arrow.bottom() + GAP;
    let dot = has_dot.then(|| {
        let c = Pos2::new(cx, y + DOT / 2.0);
        y += DOT + GAP;
        c
    });
    RailLayout {
        arrow,
        dot,
        text_top: y,
        text_len: (rect.bottom() - PAD - y).max(0.0),
    }
}

/// Paints the rail into `rect` and returns its response. The text is laid
/// out, measured and cut to fit, then painted turned by 90 degrees.
fn paint_rail(
    ui: &mut Ui,
    rect: Rect,
    side: Side,
    t: &Tokens,
    summary: &Summary,
    name: &str,
) -> egui::Response {
    let id = ui.scope_id().with(("rail", name));
    let resp = ui.interact(rect, id, Sense::click());
    let hovered = resp.hovered() || resp.has_focus();
    let fill = if hovered {
        t.rail.lerp_to_gamma(t.text, 0.08)
    } else {
        t.rail
    };
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, fill);
    let edge = Stroke::new(1.0, t.text_dim.gamma_multiply(0.35));
    match side {
        Side::Left => p.vline(rect.right() - 0.5, rect.y_range(), edge),
        Side::Right => p.vline(rect.left() + 0.5, rect.y_range(), edge),
    };
    if resp.has_focus() {
        p.rect_stroke(
            rect.shrink(1.0),
            0.0,
            Stroke::new(2.0, t.accent_prepare),
            egui::StrokeKind::Inside,
        );
    }
    let lay = layout(rect, summary.dot.is_some());
    let arrow = match side {
        Side::Left => Icon::ChevronRight,
        Side::Right => Icon::ChevronLeft,
    };
    let tex = icons::texture_colored(ui.ctx(), arrow, ARROW, t.text_dim);
    p.image(
        tex.id(),
        lay.arrow,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
    if let (Some(c), Some(dot)) = (lay.dot, summary.dot) {
        p.circle_filled(c, DOT / 2.0, dot.colour(t));
    }
    if let Some(galley) = fit(ui, &summary.parts, t, lay.text_len) {
        let (w, h) = (galley.size().x, galley.size().y);
        let cx = rect.center().x;
        // The left rail reads bottom to top, the right ones top to bottom;
        // both sit at the top, right under the arrow.
        let (pos, angle) = match side {
            Side::Left => (
                Pos2::new(cx - h / 2.0, lay.text_top + w),
                -std::f32::consts::FRAC_PI_2,
            ),
            Side::Right => (
                Pos2::new(cx + h / 2.0, lay.text_top),
                std::f32::consts::FRAC_PI_2,
            ),
        };
        p.add(TextShape::new(pos, galley, t.text).with_angle(angle));
    }
    resp
}

/// Draws a collapsed panel as a rail in its own panel. Returns true when
/// the person clicked it (open the panel).
pub fn show_rail(ui: &mut Ui, kind: SidePanel, side: Side, t: &Tokens, summary: &Summary) -> bool {
    let id = match kind {
        SidePanel::Objects => "flatcraft-objects-rail",
        SidePanel::Settings => "flatcraft-settings-rail",
        SidePanel::Stage => "flatcraft-cut-stage-rail",
    };
    let panel = match side {
        Side::Left => egui::Panel::left(id),
        Side::Right => egui::Panel::right(id),
    };
    let name = format!("Open the {} panel. {}", kind.title(), summary.full());
    let mut clicked = false;
    panel
        .exact_size(RAIL_W)
        .resizable(false)
        .frame(egui::Frame::new())
        .show(ui, |ui| {
            let rect = ui.max_rect();
            let resp = paint_rail(ui, rect, side, t, summary, kind.title());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name.as_str()));
            set_name(ui, &resp, &name);
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            groups::card(
                &resp,
                &format!("Open the {} panel", kind.title()),
                &format!(
                    "{}. This is {}. Click anywhere on the strip to open it.",
                    summary.full(),
                    kind.what()
                ),
                None,
                false,
            );
            clicked = resp.clicked();
        });
    clicked
}

/// The chevron in a panel's header, which collapses the panel into its
/// rail. Returns true when clicked. Put it at the right end of the header
/// row with [`header`].
fn collapse_button(ui: &mut Ui, kind: SidePanel, side: Side) -> bool {
    let icon = match side {
        Side::Left => Icon::ChevronLeft,
        Side::Right => Icon::ChevronRight,
    };
    let name = format!("Collapse the {} panel", kind.title());
    let tex = icons::texture(ui.ctx(), icon, super::ICON_SIZE);
    let image = egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        vec2(super::ICON_SIZE, super::ICON_SIZE),
    ))
    .alt_text(&name);
    let resp = ui.add(egui::Button::image(image));
    groups::card(
        &resp,
        &name,
        &format!(
            "Folds this panel into a thin strip to give the canvas more room. The strip still shows a short summary. Click it to open the panel again. This is {}.",
            kind.what()
        ),
        None,
        false,
    );
    resp.clicked()
}

/// A panel's heading with the collapse chevron at the right end. Returns
/// true when the chevron was clicked.
pub fn header(ui: &mut Ui, kind: SidePanel, side: Side, heading: &str) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(heading).font(fonts::semibold(14.0)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clicked = collapse_button(ui, kind, side);
        });
    });
    clicked
}
