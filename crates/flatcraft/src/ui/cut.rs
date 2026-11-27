//! The Cut workspace: the stage bar (Device & Material, Path, Simulate,
//! Send), the mat canvas with the toolpaths on top, the timeline and the
//! stage panel on the right.
//!
//! The job is planned with the engine's `job::build_classified`, the same
//! build `job.preview` runs. The input is taken in the frame's snapshot
//! (one short lock) and the build runs on a worker thread, only when the
//! input changed: the same geometry `Arc`s placed the same way with the
//! same settings and profiles give the same plan. A frame waits a moment
//! ([`PLAN_WAIT`]) for a plan it just asked for, so a small job shows at
//! once; a big one lands in a later frame. Meanwhile the canvas shows the
//! last plan faded with a "Planning..." note, and everything that counts
//! (stats, legend, timeline, kept overlay) reads only the plan of the
//! current input. A plan built for an older input is never shown.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use flatcraft_device::TransportSpec;
use flatcraft_engine::CommandError;
use flatcraft_engine::job::{
    BuildFailure, JobInput, JobResult, build_classified, file_output_dir, summary,
};

use super::canvas::{self, CanvasView};
use super::collapse::{self, Group, Mode};
use super::icons::Icon;
use super::prepare::Snapshot;
use super::rails::{self, Side, SidePanel};
use super::theme::Tokens;
use super::timeline::{self, Timeline};
use super::{Action, FlatCraftApp, fonts, person_text, settings, settings_window};

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
/// Dash and gap of a weed path on the canvas, in points.
pub const WEED_DASH: f32 = 6.0;
pub const WEED_GAP: f32 = 4.0;
/// Text of the disabled Simulate stage.
pub const SIMULATE_HINT: &str = "Not ready yet in this build: the knife simulator is still on the workbench. Path shows the cut order in the meantime.";

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

/// How long a frame waits for a plan it just asked for. Small jobs land in
/// the same frame; a big one is taken in a later frame.
pub const PLAN_WAIT: std::time::Duration = std::time::Duration::from_millis(50);

/// The note on the canvas and in the stage panel while a plan is built.
pub const PLANNING: &str = "Planning...";

/// The hover card of the note and of a step that waits for the plan.
pub const PLANNING_CARD: &str = "FlatCraft is working out the cut for your latest change: the order, the knife moves and the weeding. The faded lines are the plan before it. Everything else in the window keeps working meanwhile.";

/// Why Send is dimmed while the plan is built.
pub const PLANNING_SEND: &str =
    "Wait a moment: FlatCraft is still planning the cut for your latest change.";

/// Builds a plan; tests replace it with a slow one.
pub type PlanBuilder = Arc<dyn Fn(&JobInput) -> Result<JobResult, BuildFailure> + Send + Sync>;

/// What a plan worker gives back: its document, revision and input (with
/// the cut geometry it built), and the plan.
type Built = (u64, u64, JobInput, Result<JobResult, BuildFailure>);

/// The worker that builds plans, one at a time. A copy starts idle.
struct PlanWorker {
    builder: PlanBuilder,
    /// The plan being built: its document and input, and where its result
    /// arrives.
    running: Option<((u64, JobInput), std::sync::mpsc::Receiver<Built>)>,
    /// The newest input asked for while another one was built.
    queued: Option<(u64, u64, JobInput)>,
    /// How many plans were built, for tests.
    built: usize,
}

impl Default for PlanWorker {
    fn default() -> Self {
        Self {
            builder: Arc::new(build_classified),
            running: None,
            queued: None,
            built: 0,
        }
    }
}

impl Clone for PlanWorker {
    fn clone(&self) -> Self {
        Self {
            builder: self.builder.clone(),
            ..Self::default()
        }
    }
}

impl std::fmt::Debug for PlanWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanWorker")
            .field("running", &self.running.is_some())
            .field("queued", &self.queued.is_some())
            .finish_non_exhaustive()
    }
}

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
    /// The plan of the current input: `None` while it is built.
    pub plan: Option<Plan>,
    /// The plan before, while the plan of a newer input is built: its
    /// cuts are drawn faded and the stage bar keeps its status lines, so
    /// nothing jumps. Nothing counts from it.
    pub previous: Option<Plan>,
    /// The document and input of the newest request.
    wanted: Option<(u64, JobInput)>,
    worker: PlanWorker,
    /// The plan failed because no visible object has cut paths.
    pub nothing_to_cut: bool,
    /// Why the plan failed, with fixes (`BuildFailure::issues`); empty
    /// when it worked or failed for another reason.
    pub issues: Vec<flatcraft_engine::report::Issue>,
    /// The active plotter's transport.
    pub transport: Option<TransportSpec>,
    pub labels: LabelCache,
    /// The kept and waste overlay of the plan, for Cut and Prepare.
    pub kept: super::kept::KeptCache,
    /// The project's notes above the send button are open.
    pub notes_open: bool,
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
            previous: None,
            wanted: None,
            worker: PlanWorker::default(),
            nothing_to_cut: false,
            issues: Vec::new(),
            transport: None,
            labels: LabelCache::default(),
            kept: super::kept::KeptCache::default(),
            notes_open: true,
        }
    }
}

impl CutState {
    /// Builds the plan of `f` instead of the engine's build (tests).
    pub fn set_builder(
        &mut self,
        f: impl Fn(&JobInput) -> Result<JobResult, BuildFailure> + Send + Sync + 'static,
    ) {
        self.worker.builder = Arc::new(f);
    }

    /// True while a plan is built or waits to be built.
    pub fn planning(&self) -> bool {
        self.worker.running.is_some() || self.worker.queued.is_some()
    }

    /// How many plans were built, for tests.
    pub fn plans_built(&self) -> usize {
        self.worker.built
    }

    /// Asks for the plan of the snapshot's job input when it changed, and
    /// takes a plan the worker finished. Call it without the session lock.
    /// Returns the input of a plan that landed in this frame, with its
    /// document and revision, so the caller can offer its cut geometry
    /// to the document's cache.
    pub fn update(&mut self, ctx: &egui::Context, snap: &Snapshot) -> Option<(u64, u64, JobInput)> {
        self.transport = snap.plotter.as_ref().map(|p| p.transport.clone());
        match &snap.job_input {
            None => {}
            Some(Ok(input)) => {
                let current = |k: &Option<(u64, JobInput)>| {
                    k.as_ref()
                        .is_some_and(|(d, i)| *d == snap.doc_id && i == input)
                };
                if !current(&self.plan_key) && !current(&self.wanted) {
                    self.wanted = Some((snap.doc_id, input.clone()));
                    // Nothing counts from the plan of an older input.
                    if let Some(plan) = self.plan.take() {
                        self.previous = Some(plan);
                    }
                    // The timeline counts the plan of this input only.
                    self.timeline.reset(0);
                    self.plan_key = None;
                    self.worker.queued = Some((snap.doc_id, snap.doc_rev, input.clone()));
                } else if current(&self.plan_key) {
                    // Back to the input of the plan shown: nothing to wait for.
                    self.wanted = None;
                    self.worker.queued = None;
                }
            }
            Some(Err(e)) => {
                if !matches!(&self.plan, Some(Err(old)) if old == e) {
                    self.timeline.reset(0);
                }
                self.nothing_to_cut = false;
                self.issues.clear();
                self.plan = Some(Err(e.clone()));
                self.plan_key = None;
                self.wanted = None;
                self.previous = None;
                self.worker.queued = None;
            }
        }
        self.pump(ctx)
    }

    /// Starts the queued plan when the worker is free, and takes a plan it
    /// finished: waits [`PLAN_WAIT`] for one it just started.
    fn pump(&mut self, ctx: &egui::Context) -> Option<(u64, u64, JobInput)> {
        let mut landed = None;
        loop {
            let mut started = false;
            if self.worker.running.is_none() {
                let Some((doc, rev, input)) = self.worker.queued.take() else {
                    return landed;
                };
                let (tx, rx) = std::sync::mpsc::channel();
                let builder = self.worker.builder.clone();
                let repaint = ctx.clone();
                let key = (doc, input.clone());
                let spawned = std::thread::Builder::new()
                    .name("flatcraft-plan".into())
                    .spawn(move || {
                        let built = builder(&input);
                        let _ = tx.send((doc, rev, input, built));
                        repaint.request_repaint();
                    });
                if spawned.is_err() {
                    eprintln!("flatcraft: cannot start the plan worker");
                    return landed;
                }
                self.worker.running = Some((key, rx));
                started = true;
            }
            let Some((_, rx)) = self.worker.running.as_ref() else {
                return landed;
            };
            let got = if started {
                rx.recv_timeout(PLAN_WAIT).ok()
            } else {
                match rx.try_recv() {
                    Ok(b) => Some(b),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    // The worker died: forget it, ask again next time.
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.worker.running = None;
                        self.wanted = None;
                        return landed;
                    }
                }
            };
            let Some((doc, rev, input, built)) = got else {
                return landed;
            };
            self.worker.running = None;
            self.worker.built += 1;
            let wanted = self
                .wanted
                .as_ref()
                .is_some_and(|(d, i)| *d == doc && *i == input);
            if wanted {
                if self
                    .worker
                    .queued
                    .as_ref()
                    .is_some_and(|(d, _, i)| *d == doc && *i == input)
                {
                    self.worker.queued = None;
                }
                self.take(doc, input.clone(), built);
                landed = Some((doc, rev, input));
            }
            // A plan of an older input is dropped; the newest one is next.
        }
    }

    /// Makes `built` the plan of the current input.
    fn take(&mut self, doc: u64, input: JobInput, built: Result<JobResult, BuildFailure>) {
        self.nothing_to_cut = built.as_ref().err().is_some_and(|f| f.nothing_to_cut);
        self.issues = built
            .as_ref()
            .err()
            .map(|f| f.issues.clone())
            .unwrap_or_default();
        let plan = built.map_err(|f| f.error).map(Arc::new);
        let total = plan.as_ref().map_or(0, |r| r.plan.toolpaths.len());
        self.timeline.reset(total);
        self.plan = Some(plan);
        self.plan_key = Some((doc, input));
        self.wanted = None;
        self.previous = None;
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

/// Height of the stage bar: the ribbon's tools row, so Cut's bar is as
/// tall as Prepare's groups.
pub const BAR_HEIGHT: f32 = super::groups::ROW_HEIGHT;
/// Size of the chevron between two steps, in points.
const CHEVRON: f32 = 14.0;

impl Stage {
    /// The stage's icon on the bar.
    pub fn icon(self) -> Icon {
        match self {
            Stage::Device => Icon::Plotter,
            Stage::Path => Icon::RouteTravel,
            Stage::Simulate => Icon::Play,
            Stage::Send => Icon::Send,
        }
    }

    /// The title of its hover card and of the stage panel: "1. Path".
    pub fn title(self) -> String {
        format!("{}. {}", self.number(), self.name())
    }

    /// What happens in the step and when to go there, for its hover card.
    pub fn hint(self) -> &'static str {
        match self {
            Stage::Device => {
                "Pick the plotter, the material and the mat, and check speed, force and passes. Start here with every new job: good blade settings make or break the cut."
            }
            Stage::Path => {
                "Shows the plan on the mat: the cut order, where each cut starts and which way it runs. Weed lines are extra cuts through the leftover vinyl, so it peels off easily. Go here before you cut, to see that the knife does what you expect."
            }
            Stage::Simulate => {
                "Plays the cut with the real knife on screen, so you spot a torn corner before your vinyl does."
            }
            Stage::Send => {
                "Sends the job to the plotter over USB. With File Output it saves the job as a file instead, and nothing is cut. Go here last, when the path looks right and the mat is loaded."
            }
        }
    }

    /// Why the stage is dimmed, when it is.
    pub fn blocked(self) -> Option<&'static str> {
        (!self.enabled()).then_some(SIMULATE_HINT)
    }
}

/// What is set for a stage now, or what is missing.
#[derive(Debug, Clone, PartialEq)]
enum Status {
    /// The plotter (its model, or "File Output") and the material.
    Device(String, String),
    /// The plotter or the material preset is missing: "plotter".
    Missing(&'static str),
    /// The planned job: strategy, passes ("2 passes"), weed paths.
    Plan(&'static str, String, bool),
    NothingToCut,
    NeedsFix,
    NotPlanned,
    /// The plan of the latest change is being built.
    Planning,
    NotReady,
    /// Ready to send; over USB when true, else to a file.
    Ready(bool),
}

impl Status {
    fn of(stage: Stage, st: &CutState, snap: &Snapshot) -> Status {
        let missing = if snap.plotter.is_none() {
            Some(Status::Missing("plotter"))
        } else if snap.material.is_none() {
            Some(Status::Missing("material"))
        } else {
            None
        };
        // While the plan of the latest change is built, the bar keeps the
        // status lines of the plan before, so it does not jump.
        let plan = match &st.plan {
            None if st.planning() => st.previous.as_ref(),
            p => p.as_ref(),
        };
        let unplanned = match plan {
            Some(Err(_)) if st.nothing_to_cut => Status::NothingToCut,
            Some(Err(_)) => Status::NeedsFix,
            None if st.planning() => Status::Planning,
            _ => Status::NotPlanned,
        };
        match stage {
            Stage::Device => match (&snap.plotter, &snap.material, missing) {
                (Some(p), Some(m), None) => {
                    let device = match p.transport {
                        TransportSpec::File { .. } => "File Output",
                        TransportSpec::Usb { .. } => p.model.as_str(),
                    };
                    Status::Device(device.to_string(), m.name.clone())
                }
                (_, _, why) => why.unwrap_or(Status::Missing("plotter")),
            },
            Stage::Path => match (plan, missing) {
                (Some(Ok(job)), _) => Status::Plan(
                    settings_window::strategy_name(snap.job.strategy),
                    passes_text(job),
                    weed_count(job) > 0,
                ),
                (_, Some(why)) => why,
                _ => unplanned,
            },
            Stage::Simulate => Status::NotReady,
            Stage::Send => match (plan, missing, &snap.plotter) {
                (_, Some(why), _) => why,
                (Some(Ok(_)), None, Some(p)) => {
                    Status::Ready(matches!(p.transport, TransportSpec::Usb { .. }))
                }
                _ => unplanned,
            },
        }
    }

    /// The short line under the step's title: "Quality · 2 passes".
    fn short(&self) -> String {
        match self {
            Status::Device(device, material) => format!("{device} · {material}"),
            Status::Missing(what) => format!("Pick a {what}"),
            Status::Plan(strategy, passes, weed) => {
                let weed = if *weed { " · weed" } else { "" };
                format!("{strategy} · {passes}{weed}")
            }
            Status::NothingToCut => "Nothing to cut".into(),
            Status::NeedsFix => "Needs a fix".into(),
            Status::NotPlanned => "Not planned yet".into(),
            Status::Planning => PLANNING.into(),
            Status::NotReady => "Not ready yet".into(),
            Status::Ready(true) => "USB · ready".into(),
            Status::Ready(false) => "File · ready".into(),
        }
    }

    /// The same in a plain sentence, for the card of a folded step.
    fn sentence(&self, stage: Stage) -> String {
        match self {
            Status::Device(device, material) => {
                format!("Set to {device} with {material}.")
            }
            Status::Missing(what) => {
                format!("No {what} is picked yet. Pick one in Device & Material.")
            }
            Status::Plan(strategy, passes, weed) => {
                let weed = if *weed { ", with weed lines" } else { "" };
                format!("Cuts in the {strategy} order, {passes}{weed}.")
            }
            Status::NothingToCut => "There is nothing to cut yet.".into(),
            Status::NeedsFix => "The job needs a fix first. Path says what.".into(),
            Status::NotPlanned => "The job is not planned yet.".into(),
            Status::Planning => "FlatCraft is planning the cut for your latest change.".into(),
            Status::NotReady => format!("{} is not ready yet in this build.", stage.name()),
            Status::Ready(true) => "Ready to send to the plotter over USB.".into(),
            Status::Ready(false) => "Ready to write the job to a file.".into(),
        }
    }
}

/// What a stage bar step shows under its title: what is set now, or what
/// is missing. Short, so the steps stay narrow.
pub fn stage_status(stage: Stage, st: &CutState, snap: &Snapshot) -> String {
    Status::of(stage, st, snap).short()
}

/// The status of a stage in a plain sentence, for the card of a folded
/// step, where the status line is not shown: "Simulate is not ready yet
/// in this build."
pub fn stage_status_sentence(stage: Stage, st: &CutState, snap: &Snapshot) -> String {
    Status::of(stage, st, snap).sentence(stage)
}

/// What the stage bar's steps draw with.
struct Bar<'a> {
    st: &'a mut CutState,
    /// The status line of each stage, in [`Stage::ALL`] order.
    status: [String; 4],
    /// The same as sentences, for the cards of folded steps.
    sentences: [String; 4],
    t: Tokens,
}

/// The stage bar replaces the ribbon in Cut: steps 1 to 4 drawn like the
/// ribbon's big buttons (icon, title, status line), with a chevron between
/// them, the selected one in the Cut accent, and the Debug panel toggle at
/// the right. The toggle is drawn first, so the steps get the room that is
/// left. They fold like the ribbon's groups ([`collapse`]): icon and title
/// with the status line, then icon and number, then a "»" list.
pub fn stage_bar(
    ui: &mut Ui,
    st: &mut CutState,
    snap: &Snapshot,
    show_debug: &mut bool,
    t: &Tokens,
) {
    let mut full = ui.available_rect_before_wrap();
    full.set_height(BAR_HEIGHT);
    let debug = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(full)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                super::ribbon::toggle(ui, Mode::Full, Icon::Debug, "Debug panel", show_debug);
            },
        )
        .response
        .rect;
    let gap = ui.spacing().item_spacing.x;
    let mut rest = full;
    rest.set_right((debug.left() - gap).max(full.left()));
    let status = Stage::ALL.map(|s| stage_status(s, st, snap));
    // The status lines set the widths: a new text measures again.
    let sig = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        status.hash(&mut h);
        h.finish()
    };
    let sentences = Stage::ALL.map(|s| stage_status_sentence(s, st, snap));
    let mut bar = Bar {
        st,
        status,
        sentences,
        t: *t,
    };
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rest)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            let groups = [Group {
                title: "Stages",
                keep: false,
                tile: None,
                draw: stages,
                fold: None,
            }];
            collapse::show(ui, bar_id(), sig, BAR_HEIGHT, &groups, &mut bar);
        },
    );
}

/// The id of the stage bar, for [`collapse::level`].
pub fn bar_id() -> egui::Id {
    egui::Id::unique("flatcraft-stage-bar")
}

/// A small chevron between two steps: they are done in this order. Only
/// painted, it is no widget of its own.
fn chevron(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(CHEVRON, BAR_HEIGHT), Sense::hover());
    let colour = ui.visuals().weak_text_color();
    let tex = super::icons::texture_colored(ui.ctx(), Icon::ChevronRight, CHEVRON, colour);
    egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        Vec2::splat(CHEVRON),
    ))
    .paint_at(
        ui,
        Rect::from_center_size(rect.center(), Vec2::splat(CHEVRON)),
    );
}

fn stages(ui: &mut Ui, mode: Mode, bar: &mut Bar) {
    // The selected stage in the Cut accent, on the bar and in the "»"
    // list (a popup does not inherit the bar's visuals).
    let v = ui.visuals_mut();
    v.selection.bg_fill = bar.t.accent_cut.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, bar.t.accent_cut);
    for (i, stage) in Stage::ALL.into_iter().enumerate() {
        let on = bar.st.stage == stage;
        let status = bar.status[i].as_str();
        // The icons' accent in the Cut colour (ink on the selected fill).
        let pal = super::icons::Palette::cut(&bar.t, on);
        let r = match mode {
            Mode::Full | Mode::Icons | Mode::Collapsed => {
                if i > 0 {
                    chevron(ui);
                }
                let (title, detail) = if mode == Mode::Full {
                    (
                        format!("{}  {}", stage.number(), stage.name()),
                        Some(status),
                    )
                } else {
                    (stage.number().to_string(), None)
                };
                super::groups::step_button(
                    ui,
                    stage.icon(),
                    &pal,
                    &title,
                    detail,
                    stage.name(),
                    stage.enabled(),
                    on,
                )
            }
            Mode::List => {
                let img = super::groups::image_with(ui, stage.icon(), super::ICON_SIZE, &pal);
                let text = format!("{}  {}", stage.number(), stage.name());
                let b = egui::Button::image_and_text(img, text)
                    .selected(on)
                    .right_text(RichText::new(status).color(ui.visuals().weak_text_color()));
                let r = ui.add_enabled(stage.enabled(), b);
                r.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::accesskit::Role::Tab,
                        stage.enabled(),
                        on,
                        stage.name(),
                    )
                });
                r
            }
        };
        // The status line is in the card too, where the bar has no room
        // for it; a dimmed stage's reason already says it.
        let what = if mode == Mode::Full || stage.blocked().is_some() {
            stage.hint().to_string()
        } else {
            format!("{} {}", stage.hint(), bar.sentences[i])
        };
        super::groups::card(&r, &stage.title(), &what, stage.blocked(), false);
        if r.clicked() && stage.enabled() {
            bar.st.stage = stage;
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
        let dot = rails::job_dot(snap, Some(&app.ui.cut));
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

/// Why the plan failed: every reason with its fixes when the engine
/// classified it, else the refusal in a red box.
fn problem(
    ui: &mut Ui,
    st: &CutState,
    e: &CommandError,
    doc_id: u64,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    if st.issues.is_empty() {
        error_box(ui, e, t);
    } else {
        super::report::reasons_box(ui, &st.issues, st.nothing_to_cut, doc_id, t, actions);
    }
}

/// Paths, cut length and travel length of a plan.
///
/// "Weed paths" always shows; dimmed, with the reason, while weeding is
/// off.
fn stats(ui: &mut Ui, job: &JobResult, snap: &Snapshot, t: &Tokens) {
    let why = weed_why(snap, job.plan.kept.sheet_kept_besides(|_| false));
    let weeding = why.is_none();
    egui::Grid::new("flatcraft-cut-stats")
        .num_columns(2)
        .spacing([t.spacing * 2.0, t.spacing * 0.5])
        .show(ui, |ui| {
            ui.label("Paths");
            ui.label(job.toolpaths_cut().to_string());
            ui.end_row();
            let r = ui.add_enabled(weeding, egui::Label::new("Weed paths"));
            super::groups::card(
                &r,
                "Weed paths",
                "The weed box and weed lines in the cut, counted in Paths too. They are dashed blue on the mat and cut after the design. Change them in Device & Material, under Weeding.",
                why,
                false,
            );
            ui.add_enabled(weeding, egui::Label::new(weed_count(job).to_string()));
            ui.end_row();
            ui.label("Cut length");
            ui.label(mm_text(job.plan.cut_mm));
            ui.end_row();
            ui.label("Travel length");
            ui.label(mm_text(job.plan.travel_mm));
            ui.end_row();
        });
}

/// The stats rows while the plan of the latest change is built: the same
/// rows (so nothing jumps), with "..." for every number, and the note.
fn planning_stats(ui: &mut Ui, t: &Tokens) {
    let note = ui.label(RichText::new(PLANNING).color(t.text_dim));
    super::groups::card(&note, PLANNING, PLANNING_CARD, None, false);
    egui::Grid::new("flatcraft-cut-stats")
        .num_columns(2)
        .spacing([t.spacing * 2.0, t.spacing * 0.5])
        .show(ui, |ui| {
            for row in ["Paths", "Weed paths", "Cut length", "Travel length"] {
                ui.add_enabled(false, egui::Label::new(row));
                ui.add_enabled(false, egui::Label::new("..."));
                ui.end_row();
            }
        });
}

/// How many weed paths are cut: the weed part of the job's cut
/// toolpaths, so "Weed paths" and "Paths" count the same way.
pub fn weed_count(job: &JobResult) -> usize {
    job.cut_toolpaths()
        .filter(|t| t.kind == flatcraft_path::ToolpathKind::Weed)
        .count()
}

/// A length for the stats, like "200.5 mm".
pub fn mm_text(mm: f64) -> String {
    format!("{mm:.1} mm")
}

/// The job's warnings, then its issues with their fix buttons. A warning
/// an issue says again (with a fix) is left out.
fn warnings(ui: &mut Ui, job: &JobResult, doc_id: u64, t: &Tokens, actions: &mut Vec<Action>) {
    for (i, w) in job.warnings.iter().enumerate() {
        if !job.covered_warnings.contains(&i) {
            ui.label(RichText::new(w).color(t.accent_cut));
        }
    }
    if !job.issues.is_empty() {
        ui.add_space(t.spacing / 2.0);
        let list = super::report::by_severity(job.issues.iter());
        super::report::issue_list(ui, &list, doc_id, t, true, actions);
    }
}

fn stage_panel(ui: &mut Ui, app: &mut FlatCraftApp, snap: &Snapshot, actions: &mut Vec<Action>) {
    let t = app.tokens;
    let st = &app.ui.cut;
    let stage = st.stage;
    let title = stage.title();
    if rails::header(ui, SidePanel::Stage, Side::Right, &title) {
        app.ui.prepare.panels.set_collapsed(SidePanel::Stage, true);
    }
    ui.add_space(t.spacing / 2.0);
    match stage {
        Stage::Device => {
            // Why nothing cuts, before the settings: the first thing seen
            // on entering Cut.
            if matches!(st.plan, Some(Err(_))) && !st.issues.is_empty() {
                super::report::reasons_box(
                    ui,
                    &st.issues,
                    st.nothing_to_cut,
                    snap.doc_id,
                    &t,
                    actions,
                );
                ui.add_space(t.spacing);
            }
            settings::job(ui, &mut app.ui.prepare, snap, &t, actions);
            super::materials::iron_on_reminder(ui, snap, &t, actions);
            if let Some(tr) = &app.ui.cut.transport {
                ui.separator();
                ui.label(RichText::new("Transport").font(fonts::semibold(13.0)));
                ui.label(transport_text(tr, snap));
            }
        }
        Stage::Path => match &st.plan {
            Some(Ok(job)) => {
                stats(ui, job, snap, &t);
                warnings(ui, job, snap.doc_id, &t, actions);
                ui.add_space(t.spacing);
                legend(ui, snap, job, &t);
                ui.label(
                    RichText::new(
                        "Numbers show the cut order, dots the starts and arrows the direction.",
                    )
                    .color(t.text_dim),
                );
            }
            Some(Err(e)) => problem(ui, st, e, snap.doc_id, &t, actions),
            None if st.planning() => planning_stats(ui, &t),
            None => {}
        },
        Stage::Simulate => {
            ui.label(SIMULATE_HINT);
        }
        Stage::Send => {
            let ok = matches!(st.plan, Some(Ok(_)));
            let planning = st.planning() && st.plan.is_none();
            if let Some(p) = &snap.plotter {
                ui.label(RichText::new(&p.name).font(fonts::medium(13.0)));
                ui.label(RichText::new(transport_text(&p.transport, snap)).color(t.text_dim));
                ui.add_space(t.spacing / 2.0);
            }
            match &st.plan {
                Some(Ok(job)) => {
                    stats(ui, job, snap, &t);
                    ui.label(format!(
                        "Speed {}, force {}, {}",
                        job.settings.speed,
                        job.settings.force,
                        passes_text(job)
                    ));
                    warnings(ui, job, snap.doc_id, &t, actions);
                }
                Some(Err(e)) => problem(ui, st, e, snap.doc_id, &t, actions),
                None if st.planning() => planning_stats(ui, &t),
                None => {}
            }
            super::materials::iron_on_reminder(ui, snap, &t, actions);
            ui.add_space(t.spacing);
            send_notes(ui, &mut app.ui.cut, snap, &t);
            let label = send_label(snap);
            let hint = if ok {
                "Punch it, Chewie."
            } else if planning {
                PLANNING_SEND
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
            // After a cut: what to check when it went wrong. It stays
            // usable when this job cannot be cut, the cut may be older.
            ui.add_space(t.spacing);
            let r = ui.button(super::troubleshoot::LABEL);
            super::groups::card(
                &r,
                super::troubleshoot::TITLE,
                super::troubleshoot::OPEN_HINT,
                None,
                false,
            );
            if r.clicked() {
                actions.push(Action::Troubleshoot);
            }
        }
    }
}

/// The legend rows of the Path stage: name and hover card.
pub const LEGEND_CUT: (&str, &str) = (
    "Cut",
    "Solid lines are the cuts of your design. The blade goes down and cuts along them.",
);
pub const LEGEND_WEED: (&str, &str) = (
    "Weed box and lines",
    "Dashed blue lines are the weed box and weed lines: extra cuts through the waste only, so it peels off easily after the cut. Change them in Device & Material, under Weeding.",
);
pub const LEGEND_TRAVEL: (&str, &str) = (
    "Travel",
    "Dotted lines are where the knife moves up, from the end of one cut to the start of the next. Nothing is cut there.",
);

pub const LEGEND_KEPT: (&str, &str) = (
    "Kept",
    "Plain paper is what you keep after cutting: the shapes of a sticker, the sheet of a stencil. The blade never starts or overshoots in it. Each object's Keep setting in Prepare decides what is kept.",
);
pub const LEGEND_WASTE: (&str, &str) = (
    "Waste",
    "Hatched areas are waste: the part you throw away after cutting. Lead-ins, the short moves that turn the blade before a cut, and weed lines only go here.",
);

/// Why the weed row of the legend and the weed path count are dimmed.
pub const WEEDING_OFF: &str = "Weeding is off. Turn it on in Device & Material, under Weeding.";

/// True when the shared sheet is kept (an object set to Stencil or Both
/// with a closed cut): the plan's own rule when the plan is current for
/// `snap`, else the document's guess ([`settings::sheet_kept`]).
pub fn sheet_kept_now(cut: &CutState, snap: &Snapshot) -> bool {
    match (&snap.job_input, cut.job()) {
        (Some(Ok(_)), Some(job)) => job.plan.kept.sheet_kept_besides(|_| false),
        _ => settings::sheet_kept(snap),
    }
}

/// Why the weed row and the weed path count are dimmed, if they are:
/// weeding is off, or an object keeps the sheet so nothing is waste to
/// weed.
pub fn weed_why(snap: &Snapshot, sheet_kept: bool) -> Option<&'static str> {
    if !snap.job.weed.any() {
        Some(WEEDING_OFF)
    } else if sheet_kept {
        Some(settings::SHEET_KEPT_NO_WEEDING)
    } else {
        None
    }
}

/// How a legend sample is drawn.
#[derive(Clone, Copy)]
enum Sample {
    Solid,
    Dashed,
    Dotted,
    /// A swatch of kept paper.
    Kept,
    /// A swatch of hatched waste.
    Waste,
}

/// A small legend of the Path view: a short line of each kind, drawn on
/// the mat colour with the canvas's own strokes and colours. The weed row
/// is dimmed, with the reason, while weeding is off.
fn legend(ui: &mut Ui, snap: &Snapshot, job: &JobResult, t: &Tokens) {
    let mat = canvas::mat_rgb(snap);
    let contrast = canvas::rgb(canvas::contrast_colour(mat));
    let weed_why = weed_why(snap, job.plan.kept.sheet_kept_besides(|_| false));
    let weeding = weed_why.is_none();
    let rows = [
        (
            LEGEND_CUT,
            Sample::Solid,
            canvas::rgb(canvas::cut_colour(mat)),
            true,
        ),
        (
            LEGEND_WEED,
            Sample::Dashed,
            canvas::weed_rgb(mat, t),
            weeding,
        ),
        (
            LEGEND_TRAVEL,
            Sample::Dotted,
            contrast.gamma_multiply(0.45),
            true,
        ),
        (LEGEND_KEPT, Sample::Kept, t.kept_paper, true),
        (
            LEGEND_WASTE,
            Sample::Waste,
            super::kept::hatch_colour(mat, t),
            true,
        ),
    ];
    for ((name, hint), sample, colour, on) in rows {
        let why = if on { None } else { weed_why };
        // A dimmed row fades its sample like egui fades a disabled label.
        let fade = if on { 1.0 } else { 0.4 };
        let colour = colour.gamma_multiply(fade);
        ui.horizontal(|ui| {
            let (rect, r) = ui.allocate_exact_size(Vec2::new(44.0, 14.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(rect, 3.0, canvas::rgb(mat).gamma_multiply(fade));
            let line = [
                rect.left_center() + Vec2::new(5.0, 0.0),
                rect.right_center() - Vec2::new(5.0, 0.0),
            ];
            match sample {
                Sample::Solid => {
                    p.line_segment(line, Stroke::new(1.5, colour));
                }
                Sample::Dashed => {
                    p.extend(egui::Shape::dashed_line(
                        &line,
                        Stroke::new(1.5, colour),
                        WEED_DASH,
                        WEED_GAP,
                    ));
                }
                Sample::Dotted => {
                    p.extend(egui::Shape::dotted_line(&line, colour, 5.0, 0.9));
                }
                Sample::Kept => {
                    // Outlined, so paper reads on a paper mat too.
                    p.rect(
                        rect.shrink(3.0),
                        2.0,
                        colour,
                        Stroke::new(1.0, contrast.gamma_multiply(0.35)),
                        egui::StrokeKind::Inside,
                    );
                }
                Sample::Waste => {
                    super::kept::hatch(p, rect.shrink(3.0), colour);
                }
            }
            super::groups::card(&r, name, hint, why, false);
            let l = ui.add_enabled(on, egui::Label::new(name));
            super::groups::card(&l, name, hint, why, false);
        });
    }
}

/// The card of the Notes heading above the send button.
pub const SEND_NOTES_HINT: &str = "The notes you keep with this project, right before you cut, so nothing is forgotten. Edit them in Prepare, under Objects.";

/// The project's notes above the send button, under a "Notes" heading
/// that folds them away. Nothing shows when the project has no notes.
fn send_notes(ui: &mut Ui, st: &mut CutState, snap: &Snapshot, t: &Tokens) {
    if snap.notes.trim().is_empty() {
        return;
    }
    if super::objects::section_toggle(ui, "Notes", st.notes_open, SEND_NOTES_HINT) {
        st.notes_open = !st.notes_open;
    }
    if st.notes_open {
        egui::Frame::new()
            .fill(t.accent_prepare.gamma_multiply(0.10))
            .corner_radius(t.radius)
            .inner_margin(t.spacing)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(snap.notes.trim_end()).wrap());
            });
    }
    ui.add_space(t.spacing);
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
/// The cuts of the last plan, faded, while the plan of the latest change
/// is built. No order numbers, starts or travel: they may be gone.
fn draw_previous(painter: &egui::Painter, view: &CanvasView, job: &JobResult, colour: Color32) {
    let stroke = Stroke::new(1.0, colour.gamma_multiply(0.35));
    for tp in &job.plan.toolpaths {
        let points: Vec<Pos2> = tp.points.iter().map(|p| view.mm_to_screen(*p)).collect();
        if points.len() > 1 {
            painter.add(egui::Shape::line(points, stroke));
        }
    }
}

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
    // Kept material and waste, always, under everything the plan draws.
    if let (Some(job), Some(area)) = (st.job(), snap.area) {
        st.kept
            .draw(ui.ctx(), &painter, &view, &job, area, mat_rgb, t);
    }
    let job = st.job().filter(|_| st.stage != Stage::Device);
    // While the plan of the latest change is built, the last plan shows
    // faded under a note; it never counts.
    let previous = match &st.previous {
        Some(Ok(prev)) if job.is_none() && st.planning() && st.stage != Stage::Device => {
            Some(prev.clone())
        }
        _ => None,
    };
    if let Some(prev) = &previous {
        draw_previous(&painter, &view, prev, contrast);
        let galley = painter.layout_no_wrap(PLANNING.to_string(), fonts::medium(12.0), t.text_dim);
        let at = rect.left_top() + Vec2::new(canvas::RULER + t.spacing, canvas::RULER + t.spacing);
        painter.galley(at, galley, t.text_dim);
    }
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
            (
                contrast,
                canvas::rgb(canvas::cut_colour(mat_rgb)),
                canvas::weed_rgb(mat_rgb, t),
            ),
            &mut st.labels,
        );
        if crowded {
            // Above the notices while they show there, so both stay
            // readable, and always inside the canvas.
            let galley =
                painter.layout_no_wrap(ZOOM_HINT.to_string(), fonts::medium(12.0), t.text_dim);
            let size = galley.size();
            let at = rect.left_bottom() + Vec2::new(canvas::RULER + t.spacing, -t.spacing);
            let hint =
                egui::Rect::from_min_max(at - Vec2::new(0.0, size.y), at + Vec2::new(size.x, 0.0));
            let at = match super::toasts::stack_rect(ui.ctx()) {
                Some(n) if n.intersects(hint) => egui::pos2(at.x, n.top() - t.spacing),
                _ => at,
            };
            let lowest_top = rect.top() + canvas::RULER + t.spacing + size.y;
            let at = egui::pos2(at.x, at.y.max(lowest_top).min(rect.bottom() - t.spacing));
            painter.galley(at - Vec2::new(0.0, size.y), galley, t.text_dim);
        }
    }
    canvas::draw_area(&painter, snap, &view, t, contrast);
    canvas::draw_rulers(&painter, rect, &view, t);
}

/// Draws the first `visible` toolpaths: the cut lines, dotted travel moves
/// from the device home, a start dot, a direction arrow and the order number.
/// Cut moves, start dots and arrows are in the cut orange (`cut`), weed
/// paths dashed in the weed blue (`weed`), travel and the order numbers in
/// the mat's contrast colour, so all read on any mat.
fn draw_toolpaths(
    painter: &egui::Painter,
    view: &CanvasView,
    rect: Rect,
    job: &Arc<JobResult>,
    visible: usize,
    (contrast, cut, weed): (Color32, Color32, Color32),
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
        if tp.kind == flatcraft_path::ToolpathKind::Weed {
            // Weed paths are dashed, so they read apart from the design
            // (solid) and the travel (dotted).
            painter.extend(egui::Shape::dashed_line(
                &pts,
                Stroke::new(width, weed),
                WEED_DASH,
                WEED_GAP,
            ));
        } else {
            painter.add(egui::Shape::line(pts, Stroke::new(width, cut)));
        }
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
