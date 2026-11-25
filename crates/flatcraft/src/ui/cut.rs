//! The Cut workspace: the stage bar (Device & Material, Path, Simulate,
//! Send), the mat canvas with the toolpaths on top, the timeline and the
//! stage panel on the right.
//!
//! The job is planned with the engine's `job::build_input`, the same build
//! `job.preview` runs. The input is taken in the frame's snapshot (one
//! short lock) and the build runs after the lock is released, only when
//! the input changed: the same geometry `Arc`s placed the same way with the
//! same settings and profiles give the same plan.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use flatcraft_device::TransportSpec;
use flatcraft_engine::CommandError;
use flatcraft_engine::job::{JobInput, JobResult, build_input, file_output_dir, summary};

use super::canvas::{self, CanvasView};
use super::collapse::{self, Group, Mode};
use super::prepare::Snapshot;
use super::rails::{self, Side, SidePanel};
use super::theme::Tokens;
use super::timeline::{self, Timeline};
use super::{Action, FlatCraftApp, fonts, person_text, settings};

/// Where a label sits from its start point, in points.
const LABEL_OFFSET: Vec2 = Vec2::new(5.0, -4.0);
/// Rough size of label text at the label font: width per character and
/// line height, in points.
const LABEL_CHAR_W: f32 = 6.0;
const LABEL_H: f32 = 11.0;

/// Order numbers and arrows are left out while more starts than this are
/// on screen; a hint asks to zoom in.
pub const MAX_LABELS: usize = 300;
/// The hint shown instead of the order numbers.
pub const ZOOM_HINT: &str = "Zoom in to see the order numbers";
/// Text of the disabled Simulate stage.
pub const SIMULATE_HINT: &str = "The knife simulator arrives in milestone 2.";

/// The four steps of the stage bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stage {
    #[default]
    Device,
    Path,
    Simulate,
    Send,
}

impl Stage {
    pub const ALL: [Stage; 4] = [Stage::Device, Stage::Path, Stage::Simulate, Stage::Send];

    pub fn name(self) -> &'static str {
        match self {
            Stage::Device => "Device & Material",
            Stage::Path => "Path",
            Stage::Simulate => "Simulate",
            Stage::Send => "Send",
        }
    }

    pub fn number(self) -> usize {
        match self {
            Stage::Device => 1,
            Stage::Path => 2,
            Stage::Simulate => 3,
            Stage::Send => 4,
        }
    }

    pub fn enabled(self) -> bool {
        self != Stage::Simulate
    }
}

/// The plan of the current input, or why there is none.
pub type Plan = Result<Arc<JobResult>, CommandError>;

/// Everything Cut remembers between frames.
#[derive(Debug, Clone)]
pub struct CutState {
    pub stage: Stage,
    pub view: CanvasView,
    pub fit_pending: bool,
    /// The document the view was fitted for.
    pub view_doc: Option<u64>,
    /// The canvas area of the last frame, in points.
    pub canvas_rect: Rect,
    pub timeline: Timeline,
    /// The document and input the plan was built from.
    plan_key: Option<(u64, JobInput)>,
    pub plan: Option<Plan>,
    /// The active plotter's transport.
    pub transport: Option<TransportSpec>,
    pub labels: LabelCache,
}

impl Default for CutState {
    fn default() -> Self {
        Self {
            stage: Stage::default(),
            view: CanvasView::default(),
            fit_pending: true,
            view_doc: None,
            canvas_rect: Rect::NOTHING,
            timeline: Timeline::default(),
            plan_key: None,
            plan: None,
            transport: None,
            labels: LabelCache::default(),
        }
    }
}

impl CutState {
    /// Rebuilds the plan when the snapshot's job input changed. Call it
    /// without the session lock.
    pub fn update(&mut self, snap: &Snapshot) {
        self.transport = snap.plotter.as_ref().map(|p| p.transport.clone());
        match &snap.job_input {
            None => {}
            Some(Ok(input)) => {
                let same = self
                    .plan_key
                    .as_ref()
                    .is_some_and(|(d, i)| *d == snap.doc_id && i == input);
                if !same {
                    let plan = build_input(input).map(Arc::new);
                    let total = plan.as_ref().map_or(0, |r| r.plan.toolpaths.len());
                    self.timeline.reset(total);
                    self.plan = Some(plan);
                    self.plan_key = Some((snap.doc_id, input.clone()));
                }
            }
            Some(Err(e)) => {
                if !matches!(&self.plan, Some(Err(old)) if old == e) {
                    self.timeline.reset(0);
                }
                self.plan = Some(Err(e.clone()));
                self.plan_key = None;
            }
        }
    }

    /// The built job, if the plan worked.
    pub fn job(&self) -> Option<Arc<JobResult>> {
        match &self.plan {
            Some(Ok(j)) => Some(j.clone()),
            _ => None,
        }
    }

    /// How many toolpaths the canvas draws.
    pub fn visible_toolpaths(&self) -> usize {
        self.job()
            .map_or(0, |j| self.timeline.visible().min(j.plan.toolpaths.len()))
    }
}

/// "File Output → <dir>" or "USB 0b4d:112b".
pub fn transport_text(transport: &TransportSpec, snap: &Snapshot) -> String {
    match transport {
        TransportSpec::File { directory, .. } => {
            let dir: PathBuf = file_output_dir(
                directory.as_deref(),
                snap.output_pref.as_deref(),
                &snap.config_dir,
            );
            format!("File Output → {}", dir.display())
        }
        TransportSpec::Usb { vid, pid } => format!("USB {vid:04x}:{pid:04x}"),
    }
}

/// "Send to File Output" or "Send to <model>".
pub fn send_label(snap: &Snapshot) -> String {
    match &snap.plotter {
        Some(p) if matches!(p.transport, TransportSpec::Usb { .. }) => {
            format!("Send to {}", p.model)
        }
        _ => "Send to File Output".to_string(),
    }
}

// ---------------------------------------------------------------- stage bar

/// The stage bar replaces the ribbon in Cut: steps 1 to 4 in the Cut
/// accent, and the Debug panel toggle at the right. The toggle is drawn
/// first, so the steps get the room that is left; when they do not fit,
/// they show only their numbers (the name is their accessible name and
/// their tooltip).
pub fn stage_bar(ui: &mut Ui, st: &mut CutState, show_debug: &mut bool, t: &Tokens) {
    let full = ui.available_rect_before_wrap();
    let debug = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(full)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                super::ribbon::toggle(
                    ui,
                    Mode::Full,
                    super::icons::Icon::Debug,
                    "Debug panel",
                    show_debug,
                )
            },
        )
        .response
        .rect;
    let gap = ui.spacing().item_spacing.x;
    let mut rest = full;
    rest.set_right((debug.left() - gap).max(full.left()));
    let height = ui.spacing().interact_size.y;
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rest)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            let v = ui.visuals_mut();
            v.selection.bg_fill = t.accent_cut.gamma_multiply(0.35);
            v.selection.stroke = Stroke::new(1.0, t.accent_cut);
            let groups = [Group {
                title: "Stages",
                keep: false,
                tile: None,
                draw: stages,
            }];
            collapse::show(ui, bar_id(), 0, height, &groups, st);
        },
    );
}

/// The id of the stage bar, for [`collapse::level`].
pub fn bar_id() -> egui::Id {
    egui::Id::unique("flatcraft-stage-bar")
}

fn stages(ui: &mut Ui, mode: Mode, st: &mut CutState) {
    for stage in Stage::ALL {
        let on = st.stage == stage;
        let text = match mode {
            Mode::Icons | Mode::Collapsed => format!("{}", stage.number()),
            Mode::Full | Mode::List => format!("{}  {}", stage.number(), stage.name()),
        };
        let text = RichText::new(text).font(fonts::medium(13.0));
        let mut r = ui
            .add_enabled(stage.enabled(), egui::Button::selectable(on, text))
            .on_disabled_hover_text(SIMULATE_HINT);
        if mode == Mode::Icons && stage.enabled() {
            r = r.on_hover_text(stage.name());
        }
        r.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::accesskit::Role::Tab,
                stage.enabled(),
                on,
                stage.name(),
            )
        });
        if r.clicked() && stage.enabled() {
            st.stage = stage;
            if mode == Mode::List {
                ui.close();
            }
        }
    }
}

// ---------------------------------------------------------------- layout

/// Draws the Cut workspace.
pub fn show(ui: &mut Ui, app: &mut FlatCraftApp, snap: &Snapshot, actions: &mut Vec<Action>) {
    let t = app.tokens;
    if app.ui.prepare.panels.collapsed(SidePanel::Stage) {
        let dot = match &app.ui.cut.plan {
            Some(plan) => rails::plan_dot(plan),
            None => rails::snapshot_dot(snap),
        };
        let summary = rails::stage_summary(app.ui.cut.stage, dot);
        if rails::show_rail(ui, SidePanel::Stage, Side::Right, &t, &summary) {
            app.ui.prepare.panels.set_collapsed(SidePanel::Stage, false);
        }
    } else {
        egui::Panel::right("flatcraft-cut-stage")
            .resizable(true)
            .default_size(280.0)
            .frame(egui::Frame::new().fill(t.panel).inner_margin(t.spacing))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    stage_panel(ui, app, snap, actions);
                });
            });
    }
    egui::Panel::bottom("flatcraft-timeline")
        .frame(egui::Frame::new().fill(t.panel).inner_margin(t.spacing))
        .show(ui, |ui| {
            timeline::show(ui, &mut app.ui.cut.timeline, &t);
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(t.bg))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            cut_canvas(
                ui,
                rect,
                snap,
                &mut app.ui.cut,
                &t,
                app.ui.prepare.show_grid,
            );
        });
}

/// The plan's problem in a red box, in the words of the refusal.
fn error_box(ui: &mut Ui, e: &CommandError, t: &Tokens) {
    egui::Frame::new()
        .fill(t.danger.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, t.danger))
        .corner_radius(t.radius)
        .inner_margin(t.spacing)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Houston, we have a problem").font(fonts::semibold(13.0)));
            ui.label(person_text(e));
        });
}

/// Paths, cut length and travel length of a plan.
fn stats(ui: &mut Ui, job: &JobResult, t: &Tokens) {
    egui::Grid::new("flatcraft-cut-stats")
        .num_columns(2)
        .spacing([t.spacing * 2.0, t.spacing * 0.5])
        .show(ui, |ui| {
            ui.label("Paths");
            ui.label(job.plan.toolpaths.len().to_string());
            ui.end_row();
            ui.label("Cut length");
            ui.label(mm_text(job.plan.cut_mm));
            ui.end_row();
            ui.label("Travel length");
            ui.label(mm_text(job.plan.travel_mm));
            ui.end_row();
        });
}

/// A length for the stats, like "200.5 mm".
pub fn mm_text(mm: f64) -> String {
    format!("{mm:.1} mm")
}

fn warnings(ui: &mut Ui, job: &JobResult, t: &Tokens) {
    for w in &job.warnings {
        ui.label(RichText::new(w).color(t.accent_cut));
    }
}

fn stage_panel(ui: &mut Ui, app: &mut FlatCraftApp, snap: &Snapshot, actions: &mut Vec<Action>) {
    let t = app.tokens;
    let st = &app.ui.cut;
    let stage = st.stage;
    let title = format!("{}. {}", stage.number(), stage.name());
    if rails::header(ui, SidePanel::Stage, Side::Right, &title) {
        app.ui.prepare.panels.set_collapsed(SidePanel::Stage, true);
    }
    ui.add_space(t.spacing / 2.0);
    match stage {
        Stage::Device => {
            settings::job(ui, &mut app.ui.prepare, snap, &t, actions);
            if let Some(tr) = &app.ui.cut.transport {
                ui.separator();
                ui.label(RichText::new("Transport").font(fonts::semibold(13.0)));
                ui.label(transport_text(tr, snap));
            }
        }
        Stage::Path => match &st.plan {
            Some(Ok(job)) => {
                stats(ui, job, &t);
                warnings(ui, job, &t);
                ui.add_space(t.spacing);
                ui.label(
                    RichText::new(
                        "Numbers show the cut order, dots the starts, arrows the direction and the dotted lines where the knife travels up.",
                    )
                    .color(t.text_dim),
                );
            }
            Some(Err(e)) => error_box(ui, e, &t),
            None => {}
        },
        Stage::Simulate => {
            ui.label(SIMULATE_HINT);
        }
        Stage::Send => {
            let ok = matches!(st.plan, Some(Ok(_)));
            if let Some(p) = &snap.plotter {
                ui.label(RichText::new(&p.name).font(fonts::medium(13.0)));
                ui.label(RichText::new(transport_text(&p.transport, snap)).color(t.text_dim));
                ui.add_space(t.spacing / 2.0);
            }
            match &st.plan {
                Some(Ok(job)) => {
                    stats(ui, job, &t);
                    ui.label(format!(
                        "Speed {}, force {}, {}",
                        job.settings.speed,
                        job.settings.force,
                        passes_text(job)
                    ));
                    warnings(ui, job, &t);
                }
                Some(Err(e)) => error_box(ui, e, &t),
                None => {}
            }
            ui.add_space(t.spacing);
            let label = send_label(snap);
            let hint = if ok {
                "Punch it, Chewie."
            } else {
                "Fix the problem above first."
            };
            let tex = super::icons::texture(ui.ctx(), super::icons::Icon::Send, super::ICON_SIZE);
            let image = egui::Image::from_texture(egui::load::SizedTexture::new(
                tex.id(),
                Vec2::splat(super::ICON_SIZE),
            ));
            let r = ui
                .add_enabled(ok, egui::Button::image_and_text(image, label.as_str()))
                .on_hover_text(hint)
                .on_disabled_hover_text(hint);
            settings::set_name(ui, &r, &label);
            if r.clicked() {
                actions.push(Action::Send {
                    doc_id: snap.doc_id,
                });
            }
        }
    }
}

fn passes_text(job: &JobResult) -> String {
    let passes = summary(job).passes;
    if passes == 1 {
        "1 pass".into()
    } else {
        format!("{passes} passes")
    }
}

// ---------------------------------------------------------------- canvas

/// The mat with the toolpaths on top. Only pan and zoom: Cut edits nothing.
fn cut_canvas(
    ui: &mut Ui,
    rect: Rect,
    snap: &Snapshot,
    st: &mut CutState,
    t: &Tokens,
    show_grid: bool,
) {
    st.canvas_rect = rect;
    if st.view_doc != Some(snap.doc_id) {
        st.view_doc = Some(snap.doc_id);
        st.fit_pending = true;
    }
    if st.fit_pending {
        if let Some(m) = &snap.mat {
            st.view = CanvasView::fit(snap.mat_rect(m), canvas::fit_rect(rect));
        }
        st.fit_pending = false;
    }
    let resp = ui.interact(
        rect,
        ui.scope_id().with("flatcraft-cut-canvas"),
        Sense::drag(),
    );
    canvas::navigate(ui, &resp, &mut st.view);

    let painter = ui.painter_at(rect);
    let view = st.view;
    let (mat_rgb, contrast) = canvas::draw_mat(&painter, rect, snap, &view, show_grid, t);
    let job = st.job().filter(|_| st.stage != Stage::Device);
    let faded = if job.is_some() { 0.3 } else { 1.0 };
    canvas::draw_objects(&painter, snap, &view, mat_rgb, faded, true, |_| None);
    if let Some(job) = &job {
        let visible = st.visible_toolpaths();
        let crowded = draw_toolpaths(
            &painter,
            &view,
            rect,
            job,
            visible,
            (contrast, canvas::rgb(canvas::cut_colour(mat_rgb))),
            &mut st.labels,
        );
        if crowded {
            painter.text(
                rect.left_bottom() + Vec2::new(canvas::RULER + t.spacing, -t.spacing),
                Align2::LEFT_BOTTOM,
                ZOOM_HINT,
                fonts::medium(12.0),
                t.text_dim,
            );
        }
    }
    canvas::draw_area(&painter, snap, &view, t, contrast);
    canvas::draw_rulers(&painter, rect, &view, t);
}

/// Draws the first `visible` toolpaths: the cut lines, dotted travel moves
/// from the device home, a start dot, a direction arrow and the order number.
/// Cut moves, start dots and arrows are in the cut orange (`cut`), travel
/// and the order numbers in the mat's contrast colour, so all read on any mat.
fn draw_toolpaths(
    painter: &egui::Painter,
    view: &CanvasView,
    rect: Rect,
    job: &Arc<JobResult>,
    visible: usize,
    (contrast, cut): (Color32, Color32),
    cache: &mut LabelCache,
) -> bool {
    let paths = &job.plan.toolpaths;
    let travel = contrast.gamma_multiply(0.45);
    let font = fonts::medium(10.0);
    let mut at = job.plan.home;
    // On-screen starts with their order number and direction.
    let mut starts: Vec<(usize, Pos2)> = Vec::new();
    let mut arrows: Vec<(Pos2, Vec2)> = Vec::new();
    for (i, tp) in paths.iter().take(visible).enumerate() {
        let Some(first) = tp.points.first().copied() else {
            continue;
        };
        let a = view.mm_to_screen(at);
        let start = view.mm_to_screen(first);
        painter.extend(egui::Shape::dotted_line(&[a, start], travel, 5.0, 0.9));
        let pts = screen_polyline(view, &tp.points);
        // The newest drawn path stands out while the timeline is scrubbed.
        let width = if i + 1 == visible && visible < paths.len() {
            2.5
        } else {
            1.5
        };
        // A closed toolpath already returns to its start and runs on by
        // the overcut, so it is drawn as the open line it is cut as.
        if let Some(next) = pts.iter().find(|p| p.distance(start) > 0.5) {
            arrows.push((start, (*next - start).normalized()));
        }
        painter.add(egui::Shape::line(pts, Stroke::new(width, cut)));
        painter.circle_filled(start, 3.0, cut);
        if rect.contains(start) {
            starts.push((i + 1, start));
        }
        at = *tp.points.last().unwrap_or(&first);
    }
    let key = LabelKey {
        plan: job.clone(),
        visible,
        view: *view,
        rect,
    };
    let Some(labels) = cache.get(key, &starts) else {
        return true;
    };
    for (start, dir) in arrows {
        painter.arrow(start + dir * 4.0, dir * 12.0, Stroke::new(1.5, cut));
    }
    for l in labels.iter() {
        painter.text(l.at, Align2::LEFT_BOTTOM, &l.text, font.clone(), contrast);
    }
    false
}

/// An order number label: its bottom-left corner and its text.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    /// The start point the label belongs to.
    pub start: Pos2,
    pub at: Pos2,
    pub text: String,
}

/// Where the order numbers go. Starts within 1 point of each other share
/// one label ("6, 17"); a label that would cover another moves below it.
pub fn place_labels(starts: &[(usize, Pos2)]) -> Vec<Label> {
    fn bounds(at: Pos2, text: &str) -> Rect {
        Rect::from_min_max(
            Pos2::new(at.x, at.y - LABEL_H),
            Pos2::new(at.x + LABEL_CHAR_W * text.len() as f32, at.y),
        )
    }
    let mut out: Vec<Label> = Vec::new();
    for (n, start) in starts {
        if let Some(l) = out.iter_mut().find(|l| l.start.distance(*start) <= 1.0) {
            l.text.push_str(&format!(", {n}"));
            continue;
        }
        let text = n.to_string();
        let mut at = *start + LABEL_OFFSET;
        // Each move puts the label below one it covered, so this ends
        // after at most one move per placed label.
        for _ in 0..=out.len() {
            let mine = bounds(at, &text);
            match out.iter().find(|l| bounds(l.at, &l.text).intersects(mine)) {
                Some(l) => at.y = l.at.y + LABEL_H + 1.0,
                None => break,
            }
        }
        out.push(Label {
            start: *start,
            at,
            text,
        });
    }
    out
}

/// What the placed labels were made for: the plan, how many toolpaths are
/// drawn, the view and the canvas.
#[derive(Debug, Clone)]
pub struct LabelKey {
    /// The plan. Holding it keeps its address from being reused, and keys
    /// compare it with `Arc::ptr_eq`, never by content.
    pub plan: Arc<JobResult>,
    pub visible: usize,
    pub view: CanvasView,
    pub rect: Rect,
}

impl PartialEq for LabelKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.plan, &other.plan)
            && self.visible == other.visible
            && self.view == other.view
            && self.rect == other.rect
    }
}

/// The order numbers of the last frame, reused while nothing moved.
#[derive(Debug, Clone, Default)]
pub struct LabelCache {
    key: Option<LabelKey>,
    labels: Option<Arc<Vec<Label>>>,
    /// How often labels were placed (tests).
    pub placed: usize,
}

impl LabelCache {
    /// The labels for `key`, placing them from `starts` (on-screen starts
    /// only) when the key changed. `None` when there are more than
    /// [`MAX_LABELS`] starts.
    pub fn get(&mut self, key: LabelKey, starts: &[(usize, Pos2)]) -> Option<Arc<Vec<Label>>> {
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.labels = (starts.len() <= MAX_LABELS).then(|| {
                self.placed += 1;
                Arc::new(place_labels(starts))
            });
        }
        self.labels.clone()
    }
}

/// Screen points of a toolpath without repeated points. egui's tessellator
/// turns a zero-length segment into a long miter spike, and toolpaths
/// repeat their start point to close.
fn screen_polyline(view: &CanvasView, points: &[kurbo::Point]) -> Vec<Pos2> {
    let mut out: Vec<Pos2> = Vec::with_capacity(points.len());
    for p in points {
        let q = view.mm_to_screen(*p);
        if out.last().is_none_or(|l| l.distance(q) > 0.05) {
            out.push(q);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label_box(l: &Label) -> Rect {
        Rect::from_min_max(
            Pos2::new(l.at.x, l.at.y - LABEL_H),
            Pos2::new(l.at.x + LABEL_CHAR_W * l.text.len() as f32, l.at.y),
        )
    }

    #[test]
    fn same_start_gives_one_merged_label() {
        let p = Pos2::new(100.0, 100.0);
        let labels = place_labels(&[(6, p), (17, p + Vec2::new(0.4, 0.3))]);
        assert_eq!(labels.len(), 1, "{labels:?}");
        assert_eq!(labels[0].text, "6, 17");
    }

    #[test]
    fn close_labels_do_not_cover_each_other() {
        let labels = place_labels(&[
            (1, Pos2::new(100.0, 100.0)),
            (2, Pos2::new(104.0, 101.0)),
            (3, Pos2::new(102.0, 99.0)),
        ]);
        assert_eq!(labels.len(), 3);
        for (i, a) in labels.iter().enumerate() {
            for b in &labels[i + 1..] {
                assert!(!label_box(a).intersects(label_box(b)), "{a:?} covers {b:?}");
            }
        }
        // Far apart labels stay next to their start.
        let far = place_labels(&[(1, Pos2::ZERO), (2, Pos2::new(200.0, 0.0))]);
        assert_eq!(far[1].at, Pos2::new(200.0, 0.0) + LABEL_OFFSET);
    }

    /// A real plan: a square on the default plotter.
    fn plan() -> Arc<JobResult> {
        let dir = tempfile::tempdir().unwrap();
        let profiles = flatcraft_device::ProfileStore::load(dir.path()).unwrap();
        let mut s = flatcraft_engine::Session::new(
            profiles,
            flatcraft_engine::Preferences::default(),
            false,
            dir.path().to_path_buf(),
        );
        s.execute(
            "object.import_svg",
            serde_json::json!({ "name": "sq", "svg": r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="20" y="20" width="10" height="10"/></svg>"# }),
            flatcraft_engine::Origin::User,
        )
        .unwrap();
        Arc::new(flatcraft_engine::job::build(&s).unwrap())
    }

    thread_local! {
        static PLAN: Arc<JobResult> = plan();
    }

    fn key(visible: usize, px: f32) -> LabelKey {
        LabelKey {
            plan: PLAN.with(Arc::clone),
            visible,
            view: CanvasView {
                offset: Vec2::ZERO,
                px_per_mm: px,
            },
            rect: Rect::from_min_size(Pos2::ZERO, Vec2::splat(500.0)),
        }
    }

    #[test]
    fn labels_are_placed_once_per_view() {
        let mut cache = LabelCache::default();
        let starts = [(1, Pos2::new(10.0, 10.0)), (2, Pos2::new(50.0, 10.0))];
        let a = cache.get(key(2, 2.0), &starts).unwrap();
        let b = cache.get(key(2, 2.0), &starts).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(cache.placed, 1);
        cache.get(key(2, 3.0), &starts);
        cache.get(key(1, 3.0), &starts[..1]);
        assert_eq!(cache.placed, 3);
    }

    #[test]
    fn an_equal_but_new_plan_places_labels_again() {
        let mut cache = LabelCache::default();
        let starts = [(1, Pos2::new(10.0, 10.0))];
        cache.get(key(1, 2.0), &starts);
        let mut other = key(1, 2.0);
        other.plan = Arc::new((*other.plan).clone());
        assert_eq!(*other.plan, *key(1, 2.0).plan);
        cache.get(other.clone(), &starts);
        assert_eq!(cache.placed, 2);
        cache.get(other, &starts);
        assert_eq!(cache.placed, 2);
    }

    #[test]
    fn too_many_starts_show_no_labels() {
        let mut cache = LabelCache::default();
        let many: Vec<(usize, Pos2)> = (0..=MAX_LABELS)
            .map(|i| (i + 1, Pos2::new(i as f32, 0.0)))
            .collect();
        assert!(cache.get(key(many.len(), 2.0), &many).is_none());
        assert!(
            cache
                .get(key(MAX_LABELS, 2.0), &many[..MAX_LABELS])
                .is_some()
        );
    }

    #[test]
    fn repeated_points_are_dropped() {
        let view = CanvasView::default();
        let pts = [
            kurbo::Point::new(0.0, 0.0),
            kurbo::Point::new(10.0, 0.0),
            kurbo::Point::new(10.0, 0.0),
            kurbo::Point::new(10.0, 10.0),
            kurbo::Point::new(0.0, 0.0),
            kurbo::Point::new(0.0, 0.0),
        ];
        let line = screen_polyline(&view, &pts);
        assert_eq!(line.len(), 4);
        assert!(line.windows(2).all(|w| w[0].distance(w[1]) > 0.05));
    }
}
