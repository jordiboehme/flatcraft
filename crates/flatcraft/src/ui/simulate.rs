//! The Simulate stage of the Cut workspace: the knife simulation of the
//! job on the mat, played back on the timeline, with its grade, metrics,
//! worst spots and the comparison of the cut strategies.
//!
//! A simulation runs in the engine's three steps (`simulate::snapshot`,
//! `compute`, `store`): the snapshot is taken under a short session lock,
//! the knife is simulated on a worker thread, and the run is stored as the
//! document's last simulation only while the document and the profiles
//! are as they were. A frame waits a moment for a run it just started
//! (the frame's one wait budget), so a small job shows at once; a big one
//! lands in a later frame. Meanwhile the last run shows faded under a
//! "Simulating..." note, and nothing counts from it. A run of an older
//! state is never shown as the current one.

use crate::ui::metrics;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use flatcraft_engine::CommandError;
use flatcraft_engine::simulate::{self, SimGrid, SimInput, SimKey, SimMode, SimRun};
use flatcraft_mcp::SharedSession;
use flatcraft_path::ToolpathKind;
use flatcraft_sim::{DisplaySample, END_TO_END_LIMIT_MM, Grade, is_cut_move};

use super::canvas::{self, CanvasView};
use super::prepare::Snapshot;
use super::theme::Tokens;
use super::timeline::SimClock;
use super::{Action, fonts, lock};

/// The note on the canvas and in the panel while a simulation runs.
pub const SIMULATING: &str = "Simulating...";

/// The hover card of that note.
pub const SIMULATING_CARD: &str = "FlatCraft is running the knife over your latest change: where the blade tip really goes at every corner and start. The faded lines are the simulation before it. Everything else in the window keeps working meanwhile.";

/// The note on the canvas in firmware mode.
pub const FIRMWARE_CANVAS: &str = "The sent path: the plotter adds its own knife compensation.";

/// The grade line in firmware mode: nothing is graded.
pub const NOT_GRADED: &str = "Not graded: the plotter compensates the knife";

/// The one-line note under it.
pub const FIRMWARE_LINE: &str = "The plotter adds its own knife compensation.";

/// The card of the grade line in firmware mode.
pub const NOT_GRADED_CARD: &str = "With firmware compensation the plotter turns the blade around every corner itself. FlatCraft cannot look inside the plotter, so it does not grade the cut and colours nothing green, amber or red. Play still shows what is sent: the cut order, the travel and each time the knife goes up and down.";

/// The radius and the width of a worst spot's ring, in points, at every
/// zoom: it marks the place without hiding it.
pub const RING_RADIUS: f32 = 9.0;
pub const RING_WIDTH: f32 = 2.5;

/// The button that compares the strategies, and its card.
pub const COMPARE: &str = "Compare strategies";
pub const COMPARE_CARD: &str = "Simulates this job with each cut order (Quality, Fast and Outlines last) and lists them side by side: the grade, the worst miss, the travel and the time. Use one with its button. The document is not changed until you do.";
/// The note while the strategies are compared.
pub const COMPARING: &str = "Comparing...";

/// Computes a simulation; tests replace it with a slow one.
pub type SimComputer = Arc<dyn Fn(SimInput) -> Result<SimRun, CommandError> + Send + Sync>;

/// The runs of a strategy comparison, or why there are none.
type Runs = Result<Vec<Arc<SimRun>>, CommandError>;

/// What the compare worker gives back.
type Compared = (CompareKey, Runs);

/// What the simulation worker gives back.
type Simulated = (SimKey, Result<SimRun, CommandError>);

/// The document state a strategy comparison is valid for: the document,
/// its revision and the profiles' revision.
type CompareKey = (u64, u64, u64);

/// A worst spot as the window shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct SpotView {
    pub at: kurbo::Point,
    /// Short, for the list: "Strays 0.081 mm".
    pub label: String,
    /// The engine's note, for the card: what and where.
    pub note: String,
}

/// The simulation of the current state, with what the window derives
/// from it once.
#[derive(Debug, Clone)]
pub struct Current {
    pub run: Arc<SimRun>,
    /// The worst spots that are a problem (see [`problem_spots`]).
    pub spots: Vec<SpotView>,
}

impl Current {
    fn new(run: Arc<SimRun>) -> Self {
        Self {
            spots: problem_spots(&run),
            run,
        }
    }
}

/// The worst spots of `run` that are a problem, as the preview image
/// rings them: not a plain deviation within the limit (that is only the
/// least good of a good cut).
pub fn problem_spots(run: &SimRun) -> Vec<SpotView> {
    if run.result.is_none() {
        return Vec::new();
    }
    let report = run.report_json();
    let Some(spots) = report["worst_spots"].as_array() else {
        return Vec::new();
    };
    spots
        .iter()
        .filter_map(|s| {
            let d = s["deviation_mm"].as_f64()?;
            let kind = s["kind"].as_str()?;
            if kind == "deviation" && d <= END_TO_END_LIMIT_MM {
                return None;
            }
            let label = match kind {
                "deviation" => format!("Strays {d:.3} mm"),
                "uncut" => format!("Not cut through ({d:.3} mm off)"),
                "pushed" => "Blade pushed".to_string(),
                _ => format!("Cuts {d:.3} mm into kept"),
            };
            Some(SpotView {
                at: kurbo::Point::new(s["x_mm"].as_f64()?, s["y_mm"].as_f64()?),
                label,
                note: s["note"].as_str().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

/// The strategy comparison.
#[derive(Default)]
struct Compare {
    /// Asked for with the button, taken in the next update.
    requested: bool,
    /// The rows and the state they are valid for.
    rows: Option<Compared>,
    running: Option<(CompareKey, Receiver<Compared>)>,
}

/// Everything Simulate remembers between frames.
pub struct SimState {
    computer: SimComputer,
    /// The key of the newest request.
    wanted: Option<SimKey>,
    /// The simulation of the current state, or why there is none: `None`
    /// while it runs.
    current: Option<(SimKey, Result<Current, CommandError>)>,
    /// The simulation before, while a newer one runs: drawn faded, never
    /// counted.
    pub previous: Option<Arc<SimRun>>,
    running: Option<(SimKey, Receiver<Simulated>)>,
    queued: Option<SimInput>,
    compare: Compare,
    /// The timeline of the simulation.
    pub clock: SimClock,
    /// How many simulations finished, for tests.
    pub computed: usize,
    /// Blade-down samples drawn in the last frame, and of the whole run,
    /// for tests.
    pub drawn_down: (usize, usize),
}

impl Default for SimState {
    fn default() -> Self {
        Self {
            computer: Arc::new(simulate::compute),
            wanted: None,
            current: None,
            previous: None,
            running: None,
            queued: None,
            compare: Compare::default(),
            clock: SimClock::default(),
            computed: 0,
            drawn_down: (0, 0),
        }
    }
}

impl Clone for SimState {
    /// A copy keeps the results and starts idle.
    fn clone(&self) -> Self {
        Self {
            computer: self.computer.clone(),
            current: self.current.clone(),
            previous: self.previous.clone(),
            clock: self.clock,
            ..Self::default()
        }
    }
}

impl std::fmt::Debug for SimState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimState")
            .field("running", &self.running.is_some())
            .field("queued", &self.queued.is_some())
            .field("comparing", &self.compare.running.is_some())
            .finish_non_exhaustive()
    }
}

/// The key a simulation of `snap` has.
pub fn key_of(snap: &Snapshot) -> SimKey {
    SimKey {
        doc_id: snap.doc_id,
        doc_revision: snap.doc_rev,
        profiles_revision: snap.profiles_rev,
        strategy: snap.job.strategy,
        grid: SimGrid::Device,
    }
}

fn compare_key(snap: &Snapshot) -> CompareKey {
    (snap.doc_id, snap.doc_rev, snap.profiles_rev)
}

fn internal() -> CommandError {
    CommandError::Failed("FlatCraft hit an internal error while simulating. Try again.".into())
}

impl SimState {
    /// Computes simulations with `f` instead of the engine (tests).
    pub fn set_computer(
        &mut self,
        f: impl Fn(SimInput) -> Result<SimRun, CommandError> + Send + Sync + 'static,
    ) {
        self.computer = Arc::new(f);
    }

    /// True while a simulation or a comparison runs or waits.
    pub fn busy(&self) -> bool {
        self.running.is_some()
            || self.queued.is_some()
            || self.compare.running.is_some()
            || self.compare.requested
    }

    /// True while the simulation of the current state runs.
    pub fn simulating(&self) -> bool {
        self.running.is_some() || self.queued.is_some()
    }

    /// True while the strategies are compared.
    pub fn comparing(&self) -> bool {
        self.compare.running.is_some() || self.compare.requested
    }

    /// The simulation of `snap`'s state, if it finished.
    pub fn current(&self, snap: &Snapshot) -> Option<&Result<Current, CommandError>> {
        let key = key_of(snap);
        self.current
            .as_ref()
            .filter(|(k, _)| *k == key)
            .map(|(_, c)| c)
    }

    /// The comparison of `snap`'s state, if it finished.
    pub fn compared(&self, snap: &Snapshot) -> Option<&Result<Vec<Arc<SimRun>>, CommandError>> {
        let key = compare_key(snap);
        self.compare
            .rows
            .as_ref()
            .filter(|(k, _)| *k == key)
            .map(|(_, r)| r)
    }

    /// Asks for the strategy comparison of the current state.
    pub fn request_compare(&mut self) {
        self.compare.requested = true;
    }

    /// Makes `run` the simulation of `key`.
    fn take(&mut self, key: SimKey, run: Result<Arc<SimRun>, CommandError>) {
        let total = run.as_ref().map_or(0.0, |r| total_mm(r));
        self.clock.reset(total);
        // A queued copy of the same state would only run again for nothing.
        if self.queued.as_ref().is_some_and(|q| q.key == key) {
            self.queued = None;
        }
        self.current = Some((key, run.map(Current::new)));
        self.previous = None;
        self.wanted = None;
    }

    /// Asks for the simulation of `snap` when `active` (the Simulate stage
    /// shows) and it is not there yet, and takes what the workers
    /// finished. Waits until `deadline` for a run it starts. Takes the
    /// session lock only for a snapshot and a store.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        session: &SharedSession,
        snap: &Snapshot,
        deadline: std::time::Instant,
        active: bool,
    ) {
        let key = key_of(snap);
        // A run of an older state is never the current one.
        if self.current.as_ref().is_some_and(|(k, _)| *k != key) {
            if let Some((_, Ok(c))) = self.current.take() {
                self.previous = Some(c.run);
            }
            self.clock.playing = None;
        }
        // Another document: its runs are never drawn, not even faded.
        if self
            .previous
            .as_ref()
            .is_some_and(|r| r.key.doc_id != key.doc_id)
        {
            self.previous = None;
        }
        let ckey = compare_key(snap);
        if self.compare.rows.as_ref().is_some_and(|(k, _)| *k != ckey) {
            self.compare.rows = None;
        }
        let needs = active && self.current.is_none() && self.wanted.as_ref() != Some(&key);
        if needs {
            let s = lock(session);
            match s.last_sim().filter(|r| r.key == key).cloned() {
                // An agent's job.simulate already ran it.
                Some(run) => {
                    drop(s);
                    self.take(key, Ok(run));
                }
                None => match simulate::snapshot(&s, None, SimGrid::Device) {
                    Ok(input) => {
                        drop(s);
                        self.wanted = Some(input.key.clone());
                        self.queued = Some(input);
                    }
                    Err(e) => {
                        drop(s);
                        self.take(key, Err(e));
                    }
                },
            }
        }
        // A comparison waits for the run of the current state, so the own
        // strategy is reused, never simulated twice. Closing Simulate
        // drops a request.
        if !active {
            self.compare.requested = false;
        } else if self.compare.requested && !self.simulating() {
            self.compare.requested = false;
            self.start_compare(ctx, session, snap);
        }
        self.pump(ctx, session, deadline);
        self.pump_compare(session, snap);
    }

    /// Takes a run the worker finished (and starts a queued one) without a
    /// snapshot and without waiting: for frames that show no document
    /// (Home), so a run never stays running there. A comparison asked for
    /// is dropped, as when Simulate does not show; a finished one is kept
    /// under its key, so Simulate shows it on return when the document did
    /// not change meanwhile.
    pub fn poll(&mut self, ctx: &egui::Context, session: &SharedSession) {
        self.pump(ctx, session, std::time::Instant::now());
        self.compare.requested = false;
        let Some((_, rx)) = &self.compare.running else {
            return;
        };
        match rx.try_recv() {
            Ok(done) => {
                self.compare.running = None;
                self.compare.rows = Some(done);
            }
            Err(TryRecvError::Empty) => {}
            // The worker died: nothing to show; Simulate can ask again.
            Err(TryRecvError::Disconnected) => self.compare.running = None,
        }
    }

    /// Starts the queued run when the worker is free, and takes a run it
    /// finished: waits until `deadline` for one it just started.
    fn pump(&mut self, ctx: &egui::Context, session: &SharedSession, deadline: std::time::Instant) {
        loop {
            let mut started = false;
            if self.running.is_none() {
                let Some(input) = self.queued.take() else {
                    return;
                };
                let (tx, rx) = mpsc::channel();
                let key = input.key.clone();
                let computer = self.computer.clone();
                let repaint = ctx.clone();
                let k = key.clone();
                let spawned = std::thread::Builder::new()
                    .name("flatcraft-simulate".into())
                    .spawn(move || {
                        let _ = tx.send((k, computer(input)));
                        repaint.request_repaint();
                    });
                if spawned.is_err() {
                    eprintln!("flatcraft: cannot start the simulation worker");
                    self.take(key, Err(internal()));
                    return;
                }
                self.running = Some((key, rx));
                started = true;
            }
            let Some((key, rx)) = self.running.as_ref() else {
                return;
            };
            let got = if started {
                rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    .ok()
            } else {
                match rx.try_recv() {
                    Ok(r) => Some(r),
                    Err(TryRecvError::Empty) => None,
                    // The worker died: say so once, never ask again for
                    // this state.
                    Err(TryRecvError::Disconnected) => Some((key.clone(), Err(internal()))),
                }
            };
            let Some((key, result)) = got else {
                return;
            };
            self.running = None;
            self.computed += 1;
            if self.wanted.as_ref() != Some(&key) {
                // A run of an older state is dropped; the newest is next.
                continue;
            }
            match result {
                Ok(run) => {
                    let run = Arc::new(run);
                    if simulate::store(&mut lock(session), run.clone()) {
                        self.take(key, Ok(run));
                    } else {
                        // The document changed meanwhile: the next frame
                        // asks again for its new state.
                        self.previous = Some(run);
                        self.wanted = None;
                    }
                }
                Err(e) => self.take(key, Err(e)),
            }
        }
    }

    /// Snapshots the three strategies under one lock and compares them on
    /// a worker. The run of the current state is reused.
    fn start_compare(&mut self, ctx: &egui::Context, session: &SharedSession, snap: &Snapshot) {
        if self.compare.running.is_some() {
            return;
        }
        let ckey = compare_key(snap);
        let inputs = {
            let s = lock(session);
            simulate::STRATEGIES
                .iter()
                .map(|&st| {
                    let input = simulate::snapshot(&s, Some(st), SimGrid::Device)?;
                    let reuse = match &self.current {
                        Some((k, Ok(c))) if *k == input.key => Some(c.run.clone()),
                        _ => s.last_sim().filter(|r| r.key == input.key).cloned(),
                    };
                    Ok(match reuse {
                        Some(run) => Err(run),
                        None => Ok(input),
                    })
                })
                .collect::<Result<Vec<_>, CommandError>>()
        };
        let inputs = match inputs {
            Ok(i) => i,
            Err(e) => {
                self.compare.rows = Some((ckey, Err(e)));
                return;
            }
        };
        let (tx, rx) = mpsc::channel();
        let computer = self.computer.clone();
        let repaint = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("flatcraft-compare".into())
            .spawn(move || {
                let runs: Result<Vec<Arc<SimRun>>, CommandError> = std::thread::scope(|scope| {
                    let handles: Vec<_> = inputs
                        .into_iter()
                        .map(|i| match i {
                            Err(run) => Err(run),
                            Ok(input) => {
                                let c = computer.clone();
                                Ok(scope.spawn(move || c(input).map(Arc::new)))
                            }
                        })
                        .collect();
                    handles
                        .into_iter()
                        .map(|h| match h {
                            Err(run) => Ok(run),
                            Ok(h) => h.join().unwrap_or_else(|_| Err(internal())),
                        })
                        .collect()
                });
                let _ = tx.send((ckey, runs));
                repaint.request_repaint();
            });
        if spawned.is_err() {
            self.compare.rows = Some((ckey, Err(internal())));
            return;
        }
        self.compare.running = Some((ckey, rx));
    }

    /// Takes a finished comparison: kept only for the current state. The
    /// run of the document's own strategy is stored too.
    fn pump_compare(&mut self, session: &SharedSession, snap: &Snapshot) {
        let Some((_, rx)) = &self.compare.running else {
            return;
        };
        let got = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => (compare_key(snap), Err(internal())),
        };
        self.compare.running = None;
        let (ckey, rows) = got;
        if ckey != compare_key(snap) {
            return;
        }
        if let Ok(runs) = &rows {
            let key = key_of(snap);
            if let Some(own) = runs.iter().find(|r| r.key == key) {
                let stored = simulate::store(&mut lock(session), own.clone());
                if stored && self.current.is_none() {
                    self.take(key, Ok(own.clone()));
                }
            }
        }
        self.compare.rows = Some((ckey, rows));
    }
}

/// The head travel of `run`, mm: the simulated samples, or in firmware
/// mode the sent path from the home (its travel and its cuts).
pub fn total_mm(run: &SimRun) -> f64 {
    if run.result.is_some() {
        run.display.last().map_or(0.0, |s| s.t_mm)
    } else {
        run.commanded.travel_mm + run.commanded.cut_mm
    }
}

// ---------------------------------------------------------------- colours

/// The panel colour of a deviation: within [`END_TO_END_LIMIT_MM`] good,
/// within twice that warn, beyond bad; `None` for no deviation (NaN, the
/// blade is up). For the grade on a panel; the mat picks its own colours
/// ([`tip_colour`]).
pub fn quality_color(dev_mm: f32, t: &Tokens) -> Option<Color32> {
    let d = f64::from(dev_mm);
    if d.is_nan() {
        None
    } else if d <= END_TO_END_LIMIT_MM {
        Some(t.quality_good)
    } else if d <= 2.0 * END_TO_END_LIMIT_MM {
        Some(t.quality_warn)
    } else {
        Some(t.quality_bad)
    }
}

/// The colour of the blade tip on `mat` at `dev_mm`, the same as the
/// engine's preview image (3:1 on the mat); `None` for NaN.
pub fn tip_colour(mat: [u8; 3], dev_mm: f32) -> Option<Color32> {
    (!dev_mm.is_nan())
        .then(|| canvas::rgb(flatcraft_engine::preview::deviation_colour(mat, dev_mm)))
}

/// The panel colour of a grade.
pub fn grade_colour(g: Grade, t: &Tokens) -> Color32 {
    match g {
        Grade::Good => t.quality_good,
        Grade::Fair => t.quality_warn,
        Grade::Poor => t.quality_bad,
    }
}

/// Why a job did not grade Good, as the engine grades it, the most
/// serious reason first: (short, for the stage bar and the comparison;
/// long, a phrase after the grade in the panel). `None` for Good.
///
/// The score only counts the line that is missed or strays, so a job can
/// score 98 and still be Poor (it cuts into what you keep). The grade
/// therefore always comes with its reason, and the score is shown as what
/// it is: the share of the line on target.
pub fn grade_reason(m: &flatcraft_sim::JobMetrics) -> Option<(&'static str, String)> {
    if m.grade == Grade::Good {
        return None;
    }
    Some(if m.max_into_kept_mm > 0.0 {
        (
            "cuts into kept",
            format!(
                "cuts up to {:.2} mm into material you keep",
                m.max_into_kept_mm
            ),
        )
    } else if m.score < 90.0 {
        (
            "misses the line",
            format!("misses {:.0} % of the line", 100.0 - m.score),
        )
    } else if m.pushed > 0 {
        let times = if m.pushed == 1 {
            "once".to_string()
        } else {
            format!("{} times", m.pushed)
        };
        (
            "blade pushed",
            format!("pushes the blade from behind {times}"),
        )
    } else if m.max_hausdorff_mm > END_TO_END_LIMIT_MM {
        (
            "strays from the line",
            format!("strays up to {:.2} mm from your line", m.max_hausdorff_mm),
        )
    } else {
        (
            "misses some line",
            format!("misses {:.0} % of the line", 100.0 - m.score),
        )
    })
}

/// The grade with its reason: "Good", "Poor: cuts up to 0.23 mm into
/// material you keep".
pub fn grade_line(m: &flatcraft_sim::JobMetrics) -> String {
    match grade_reason(m) {
        Some((_, why)) => format!("{}: {why}", grade_text(m.grade)),
        None => grade_text(m.grade).to_string(),
    }
}

/// The grade with its short reason: "Good", "Poor · cuts into kept".
pub fn grade_short(m: &flatcraft_sim::JobMetrics) -> String {
    match grade_reason(m) {
        Some((why, _)) => format!("{} · {why}", grade_text(m.grade)),
        None => grade_text(m.grade).to_string(),
    }
}

/// The score as what it measures: "98 % of the line on target".
pub fn on_target_text(score: f64) -> String {
    format!("{score:.0} % of the line on target")
}

/// The name of a grade as the window shows it.
pub fn grade_text(g: Grade) -> &'static str {
    match g {
        Grade::Good => "Good",
        Grade::Fair => "Fair",
        Grade::Poor => "Poor",
    }
}

/// The ring colour of a worst spot.
pub fn ring_colour() -> Color32 {
    canvas::rgb(flatcraft_engine::preview::RING)
}

/// The colour of blade-down waste moves.
pub fn waste_colour() -> Color32 {
    canvas::rgb(flatcraft_engine::preview::WASTE_INK)
}

/// The colour of the head's path.
pub fn head_colour() -> Color32 {
    Color32::from_rgba_unmultiplied(0x8a, 0x8f, 0x98, 200)
}

// ---------------------------------------------------------------- drawing

/// How a stretch of tip moves is drawn.
#[derive(Clone, Copy, PartialEq)]
enum Ink {
    Tip(Color32),
    Waste,
    Alignment,
}

/// Screen points without repeats: a tip that stands still while the head
/// turns would make a long miter spike.
fn push_point(line: &mut Vec<Pos2>, p: Pos2) {
    if line.last().is_none_or(|l| l.distance(p) > 0.05) {
        line.push(p);
    }
}

fn lerp(a: kurbo::Point, b: kurbo::Point, f: f64) -> kurbo::Point {
    a + (b - a) * f
}

/// What [`draw`] reads besides the run.
pub struct DrawInput<'a> {
    pub view: &'a CanvasView,
    pub mat: [u8; 3],
    pub contrast: Color32,
    pub t: &'a Tokens,
    /// Head travel to draw up to, mm.
    pub t_mm: f64,
    /// The previous run while a newer one runs: faded, no rings, no head.
    pub faded: bool,
}

/// Draws `run` up to `t_mm`: the head's path thin grey, travel dotted, the
/// blade tip in the quality colours of the mat, waste moves dashed, the
/// alignment stroke highlighted, and the head and the blade where the
/// time stands. Firmware mode draws the commanded path. Returns the
/// blade-down samples drawn and the run's total.
pub fn draw(painter: &egui::Painter, run: &SimRun, d: &DrawInput) -> (usize, usize) {
    let fade = if d.faded { 0.35 } else { 1.0 };
    let travel = d.contrast.gamma_multiply(0.45 * fade);
    if run.result.is_none() {
        return draw_commanded(painter, run, d, travel, fade);
    }
    let samples: &[DisplaySample] = &run.display;
    let t_mm = if d.faded { f64::INFINITY } else { d.t_mm };
    let n = samples.partition_point(|s| s.t_mm <= t_mm);
    let total_down = samples.iter().filter(|s| s.down).count();
    let drawn_down = samples[..n].iter().filter(|s| s.down).count();
    // The sample where the time stands, between two samples.
    let tail = (n > 0 && n < samples.len()).then(|| {
        let (a, b) = (samples[n - 1], samples[n]);
        let span = b.t_mm - a.t_mm;
        let f = if span > 0.0 {
            ((t_mm - a.t_mm) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        DisplaySample {
            axis: lerp(a.axis, b.axis, f),
            tip: lerp(a.tip, b.tip, f),
            ..b
        }
    });
    let shown: Vec<DisplaySample> = samples[..n].iter().copied().chain(tail).collect();
    let screen = |p: kurbo::Point| d.view.mm_to_screen(p);
    let kind = |s: &DisplaySample| {
        run.plan_index
            .get(s.toolpath)
            .and_then(|&i| run.job.plan.toolpaths.get(i))
            .map(|t| t.kind)
    };
    // Travel, dotted, and the head's path, thin grey. Each as whole
    // polylines: a travel move has many samples, and dots that start over
    // at every sample would run together into a solid line.
    let head = Stroke::new(1.0, head_colour().gamma_multiply(fade));
    let mut axis: Vec<Pos2> = Vec::new();
    let mut up: Vec<Pos2> = Vec::new();
    let flush_axis = |axis: &mut Vec<Pos2>| {
        if axis.len() > 1 {
            painter.add(egui::Shape::line(std::mem::take(axis), head));
        }
        axis.clear();
    };
    let flush_up = |up: &mut Vec<Pos2>| {
        if up.len() > 1 {
            painter.extend(egui::Shape::dotted_line(up, travel, 5.0, 0.9));
        }
        up.clear();
    };
    for w in shown.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let line = if is_cut_move(a, b) {
            flush_up(&mut up);
            &mut axis
        } else {
            flush_axis(&mut axis);
            &mut up
        };
        if line.is_empty() {
            line.push(screen(a.axis));
        }
        push_point(line, screen(b.axis));
    }
    flush_axis(&mut axis);
    flush_up(&mut up);
    // The tip: consecutive moves of one ink as one line.
    let accent = canvas::rgb(canvas::cut_colour(d.mat)).gamma_multiply(fade);
    let flush = |ink: Option<Ink>, line: Vec<Pos2>| {
        if line.len() < 2 {
            return;
        }
        match ink {
            Some(Ink::Tip(c)) => {
                painter.add(egui::Shape::line(
                    line,
                    Stroke::new(1.5, c.gamma_multiply(fade)),
                ));
            }
            Some(Ink::Waste) => {
                painter.extend(egui::Shape::dashed_line(
                    &line,
                    Stroke::new(1.5, waste_colour().gamma_multiply(fade)),
                    3.0,
                    3.0,
                ));
            }
            Some(Ink::Alignment) => {
                painter.add(egui::Shape::line(line, Stroke::new(3.0, accent)));
            }
            None => {}
        }
    };
    let mut ink: Option<Ink> = None;
    let mut line: Vec<Pos2> = Vec::new();
    for w in shown.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if !is_cut_move(a, b) {
            flush(ink.take(), std::mem::take(&mut line));
            continue;
        }
        let this = if kind(b) == Some(ToolpathKind::Alignment) {
            Ink::Alignment
        } else if b.waste {
            Ink::Waste
        } else {
            Ink::Tip(tip_colour(d.mat, b.deviation_mm).unwrap_or(accent))
        };
        if ink != Some(this) {
            flush(ink, std::mem::take(&mut line));
            ink = Some(this);
            line.push(screen(a.tip));
        }
        push_point(&mut line, screen(b.tip));
    }
    flush(ink, line);
    // The head and the blade where the time stands, while not at the end.
    if !d.faded
        && let Some(s) = tail
    {
        let (h, tip) = (screen(s.axis), screen(s.tip));
        painter.line_segment([h, tip], Stroke::new(1.5, d.contrast));
        painter.circle_stroke(h, 5.0, Stroke::new(1.5, d.contrast));
        painter.circle_filled(tip, 2.5, d.contrast);
    }
    (drawn_down, total_down)
}

/// Moves `left` mm along the line from `a` to `b`: the point reached,
/// and true when the whole line fits.
fn advance(a: kurbo::Point, b: kurbo::Point, left: &mut f64) -> (kurbo::Point, bool) {
    let len = a.distance(b);
    if *left >= len {
        *left -= len;
        (b, true)
    } else {
        let f = if len > 0.0 {
            (*left / len).max(0.0)
        } else {
            0.0
        };
        *left = 0.0;
        (lerp(a, b, f), false)
    }
}

/// Firmware mode: the sent path, as the plotter gets it, up to `d.t_mm`
/// of head travel from the home: travel dotted, cuts in a colour that is
/// never a grade colour, and the head where the time stands. Returns
/// the cut segments drawn and the run's total.
fn draw_commanded(
    painter: &egui::Painter,
    run: &SimRun,
    d: &DrawInput,
    travel: Color32,
    fade: f32,
) -> (usize, usize) {
    let ink = Stroke::new(
        2.0,
        canvas::rgb(flatcraft_core::colour::commanded_colour(d.mat)).gamma_multiply(fade),
    );
    let screen = |p: kurbo::Point| d.view.mm_to_screen(p);
    let mut left = if d.faded { f64::INFINITY } else { d.t_mm };
    let mut drawn = 0;
    let mut head = None;
    let mut at = run.job.plan.home;
    'runs: for &i in &run.plan_index {
        let Some(tp) = run.job.plan.toolpaths.get(i) else {
            continue;
        };
        for r in tp.runs() {
            let Some(&first) = r.points.first() else {
                continue;
            };
            let (end, whole) = advance(at, first, &mut left);
            if at.distance(end) > 0.0 {
                painter.extend(egui::Shape::dotted_line(
                    &[screen(at), screen(end)],
                    travel,
                    5.0,
                    0.9,
                ));
            }
            if !whole {
                head = Some(end);
                break 'runs;
            }
            let mut line = Vec::with_capacity(r.points.len());
            line.push(screen(first));
            let mut stop = false;
            for w in r.points.windows(2) {
                let (end, whole) = advance(w[0], w[1], &mut left);
                push_point(&mut line, screen(end));
                if !whole {
                    head = Some(end);
                    stop = true;
                    break;
                }
                drawn += 1;
            }
            if line.len() > 1 {
                painter.add(egui::Shape::line(line, ink));
            }
            if stop {
                break 'runs;
            }
            at = *r.points.last().unwrap_or(&first);
        }
    }
    // The head where the time stands, while not at the end.
    if !d.faded
        && let Some(h) = head
    {
        let h = screen(h);
        painter.circle_stroke(h, 5.0, Stroke::new(1.5, d.contrast));
        painter.circle_filled(h, 2.5, d.contrast);
    }
    (drawn, run.commanded.segments)
}

/// The rings of the worst spots, each a small button: its card says what
/// happens there, and a click asks to centre the view on it (returned).
pub fn rings(
    ui: &Ui,
    painter: &egui::Painter,
    rect: Rect,
    view: &CanvasView,
    spots: &[SpotView],
) -> Option<kurbo::Point> {
    let mut centre = None;
    let stroke = Stroke::new(RING_WIDTH, ring_colour());
    for (i, spot) in spots.iter().enumerate() {
        let c = view.mm_to_screen(spot.at);
        if !rect.contains(c) {
            continue;
        }
        painter.circle_stroke(c, RING_RADIUS, stroke);
        let hit = Rect::from_center_size(c, Vec2::splat(2.0 * RING_RADIUS + 4.0));
        let r = ui.interact(
            hit,
            ui.scope_id().with(("flatcraft-sim-spot", i)),
            Sense::click(),
        );
        let name = spot_name(i);
        super::settings::set_name(ui, &r, &name);
        super::groups::card(&r, &name, &spot_card(spot), None, false);
        if r.clicked() {
            centre = Some(spot.at);
        }
    }
    centre
}

/// "Worst spot 1".
pub fn spot_name(i: usize) -> String {
    format!("Worst spot {}", i + 1)
}

fn spot_card(spot: &SpotView) -> String {
    format!(
        "{} Click to centre the view here and zoom in.",
        spot.note.trim()
    )
}

/// The view moved so `at` sits in the middle of `rect`, zoomed in to at
/// least 8 points per mm, so the spot can be seen.
pub fn centred(view: &CanvasView, rect: Rect, at: kurbo::Point) -> CanvasView {
    let px_per_mm = view.px_per_mm.max(8.0);
    let mut v = CanvasView {
        offset: Vec2::ZERO,
        px_per_mm,
    };
    let now = v.mm_to_screen(at);
    v.offset = rect.center() - now;
    v
}

/// The note at the top left of the canvas.
pub fn canvas_note(painter: &egui::Painter, rect: Rect, text: &str, t: &Tokens) {
    let galley = painter.layout_no_wrap(
        text.to_string(),
        fonts::medium(metrics::TEXT_SECONDARY),
        t.text_dim,
    );
    let at = rect.left_top()
        + Vec2::new(
            canvas::RULER + metrics::SPACE_8,
            canvas::RULER + metrics::SPACE_8,
        );
    painter.galley(at, galley, t.text_dim);
}

// ---------------------------------------------------------------- panel

/// "45 s" or "3 min 05 s".
pub fn time_text(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    if s < 60 {
        format!("{s} s")
    } else {
        format!("{} min {:02} s", s / 60, s % 60)
    }
}

fn mm(v: f64) -> String {
    format!("{v:.3} mm")
}

/// The card of the grade.
pub const GRADE_CARD: &str = "How well the blade follows your lines, after a full simulation of the drag knife: its tip trails a little behind the head, so it can cut a corner short or start off the line. Good means at least 98 % of the line is on target, every spot is within 0.06 mm of the line, nothing you keep is cut and the blade is never pushed. Fair means at least 90 % of the line is on target and nothing you keep is cut, but some spots stray, the blade is pushed or a little of the line is missed. Poor means the blade cuts into material you keep or less than 90 % of the line is on target. The reason after the grade says what counts most.";

/// The card of the share of the line on target.
pub const ON_TARGET_CARD: &str = "How much of your line the blade cuts within the limit, in percent. It only counts the line itself: a cut into material you keep or a pushed blade makes the grade worse even when this is high.";

/// The ids of the Simulate stage's sections.
pub const NUMBERS: &str = "sim.numbers";
pub const HINTS: &str = "sim.hints";
pub const SPOTS: &str = "sim.spots";
pub const STRATEGIES: &str = "sim.strategies";
pub const LEGEND: &str = "sim.legend";

/// The cards of those sections.
pub const NUMBERS_CARD: &str = "The numbers of the simulated cut: how far the blade tip strays, what it misses, how far it cuts and travels, and a rough time.";
pub const SENT_NUMBERS_CARD: &str = "The numbers of the path the plotter is sent: how far it cuts and travels, how often the knife lifts, and a rough time.";
pub const HINTS_CARD: &str = "What the simulation noticed about this job and this simulation.";
pub const SPOTS_CARD: &str = "The places where the blade tip strays most from your line, beyond the limit. Click one to zoom in on it.";
pub const STRATEGIES_CARD: &str = COMPARE_CARD;
pub const LEGEND_CARD: &str =
    "What the lines and areas on the mat mean. Hover a row to learn more.";

/// Why Worst spots is dimmed in firmware mode.
pub const NO_SPOTS_FIRMWARE: &str =
    "The plotter compensates the knife itself, so FlatCraft has no worst spots to show.";

/// A metric: its name, its card and its value.
type Metric = (&'static str, &'static str, String);

fn metrics_of(run: &SimRun) -> Vec<Metric> {
    match &run.result {
        Some(r) => {
            let m = &r.metrics;
            vec![
                (
                    "Worst miss",
                    "The farthest the blade tip strays from your line anywhere in the job. Within 0.06 mm it is green on the mat.",
                    mm(m.max_hausdorff_mm),
                ),
                (
                    "Worst corner",
                    "The farthest the tip misses a corner. A drag knife cuts corners short unless FlatCraft or the plotter steers it around them.",
                    mm(m.max_corner_deviation_mm),
                ),
                (
                    "Closure gap",
                    "How far the end of a closed cut misses its start. A gap leaves the piece hanging by a thread.",
                    mm(m.max_closure_gap_mm),
                ),
                (
                    "Not cut through",
                    "The length of line the blade never reaches. The piece will not come out there.",
                    format!("{:.1} mm", m.uncut_mm),
                ),
                (
                    "Into kept material",
                    "The deepest the blade tip goes into a part you keep, for example while it turns. Zero is what you want.",
                    mm(m.max_into_kept_mm),
                ),
                (
                    "Blade pushed",
                    "Moves where the head pushes the blade from behind instead of pulling it. That can tear the material.",
                    m.pushed.to_string(),
                ),
                (
                    "Cut length",
                    "How far the blade cuts, waste moves included.",
                    super::cut::mm_text(m.cut_mm),
                ),
                (
                    "Travel",
                    "How far the head moves with the blade up, between cuts.",
                    super::cut::mm_text(m.travel_mm),
                ),
                (
                    "Lifts",
                    "How often the blade goes up. Each lift and drop costs a little time.",
                    m.lifts.to_string(),
                ),
                (
                    "Time (estimate)",
                    "A rough guess of the cut time at this speed, from the moves and the lifts. Real plotters vary.",
                    time_text(run.estimate_s),
                ),
            ]
        }
        None => {
            let c = &run.commanded;
            vec![
                (
                    "Cut length",
                    "How far the head moves with the blade down.",
                    super::cut::mm_text(c.cut_mm),
                ),
                (
                    "Travel",
                    "How far the head moves with the blade up, between cuts.",
                    super::cut::mm_text(c.travel_mm),
                ),
                (
                    "Segments",
                    "The straight moves the plotter gets. Curves are sent as many short ones.",
                    c.segments.to_string(),
                ),
                ("Lifts", "How often the blade goes up.", c.lifts.to_string()),
                (
                    "Largest turn per vertex",
                    "The largest change of direction between two segments of a curve. Small numbers mean smooth curves.",
                    format!("{:.1} deg", c.max_turn_per_vertex_deg),
                ),
                (
                    "Time (estimate)",
                    "A rough guess of the cut time at this speed, from the moves and the lifts. Real plotters vary.",
                    time_text(run.estimate_s),
                ),
            ]
        }
    }
}

/// The grade line: a dot in the grade's colour (none in firmware mode,
/// where nothing is graded) and the text, with its card.
fn grade_line_ui(
    ui: &mut Ui,
    text: &str,
    colour: Option<Color32>,
    card: (&str, &str),
    why: Option<&str>,
) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(
                metrics::SPACE_16,
                metrics::line_height(ui, &metrics::heading()),
            ),
            Sense::hover(),
        );
        if let Some(c) = colour {
            ui.painter().circle_filled(rect.center(), 6.0, c);
        }
        let g = ui.add_enabled(
            why.is_none(),
            egui::Label::new(RichText::new(text).font(metrics::heading())).wrap(),
        );
        super::groups::card(&g, card.0, card.1, why, false);
    });
}

/// The stage panel of Simulate for a finished run: the grade and its
/// note, then the sections Numbers, Hints, Worst spots, Strategies and
/// Legend. In firmware mode nothing is graded: no grade colour, no worst
/// spots, and the numbers are of the sent path.
pub fn panel(
    ui: &mut Ui,
    st: &mut SimState,
    cur: &Current,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) -> Option<kurbo::Point> {
    use super::inspector::blocks;
    let run = &cur.run;
    let mut centre = None;
    let firmware = run.result.is_none();
    match &run.result {
        Some(r) => {
            let m = &r.metrics;
            // The grade and why first; the score after it, as what it is.
            grade_line_ui(
                ui,
                &grade_line(m),
                Some(grade_colour(m.grade, t)),
                ("Grade", GRADE_CARD),
                None,
            );
            let s = blocks::status_line(ui, &on_target_text(m.score), t.text_dim);
            super::groups::card(&s, "On target", ON_TARGET_CARD, None, false);
        }
        None => {
            let card = format!("{NOT_GRADED_CARD}\n\n{}", simulate::FIRMWARE_NOTE);
            grade_line_ui(ui, NOT_GRADED, None, ("Grade", &card), None);
            let s = blocks::status_line(ui, FIRMWARE_LINE, t.text_dim);
            super::groups::card(&s, "Knife compensation", &card, None, false);
        }
    }
    let rows = metrics_of(run);
    let summary = match &run.result {
        Some(r) => format!(
            "worst miss {} · {}",
            mm(r.metrics.max_hausdorff_mm),
            time_text(run.estimate_s)
        ),
        None => format!(
            "travel {:.0} mm · {}",
            run.commanded.travel_mm,
            time_text(run.estimate_s)
        ),
    };
    let card = if firmware {
        SENT_NUMBERS_CARD
    } else {
        NUMBERS_CARD
    };
    blocks::section(
        ui,
        NUMBERS,
        "Numbers",
        card,
        &summary,
        true,
        |ui| {
            let names: Vec<&str> = rows.iter().map(|r| r.0).collect();
            blocks::labels(ui, &names);
            for (name, card, value) in &rows {
                blocks::data_row(ui, name, card, value, None);
            }
        },
        None,
    );
    let n = run.warnings.len();
    blocks::section(
        ui,
        HINTS,
        "Hints",
        HINTS_CARD,
        &super::cut::hints_summary(n),
        true,
        |ui| {
            if n == 0 {
                blocks::status_line(ui, "Nothing to tell.", t.text_dim);
                return;
            }
            for (i, w) in run.warnings.iter().enumerate() {
                if i > 0 {
                    ui.add_space(metrics::SPACE_4);
                }
                blocks::hint_card(ui, t, None, None, w, &[]);
            }
            ui.add_space(metrics::SPACE_4);
        },
        None,
    );
    let spots = match cur.spots.len() {
        _ if firmware => "not in firmware mode".to_string(),
        0 => "none".to_string(),
        1 => "1 spot".to_string(),
        n => format!("{n} spots"),
    };
    blocks::section(
        ui,
        SPOTS,
        "Worst spots",
        SPOTS_CARD,
        &spots,
        true,
        |ui| {
            if firmware {
                let r = ui.add_enabled(
                    false,
                    egui::Label::new(RichText::new(NO_SPOTS_FIRMWARE).color(t.text_dim)).wrap(),
                );
                super::groups::card(
                    &r,
                    "Worst spots",
                    SPOTS_CARD,
                    Some(NO_SPOTS_FIRMWARE),
                    false,
                );
                ui.add_space(metrics::ROW_GAP);
                return;
            }
            if cur.spots.is_empty() {
                blocks::status_line(
                    ui,
                    "None beyond the limit. Clean cut, Mr. Sulu.",
                    t.text_dim,
                );
            }
            for (i, spot) in cur.spots.iter().enumerate() {
                let text = format!("{}. {}", i + 1, spot.label);
                let r = ui.button(text);
                let name = spot_name(i);
                super::groups::card(&r, &name, &spot_card(spot), None, false);
                if r.clicked() {
                    centre = Some(spot.at);
                }
                ui.add_space(metrics::ROW_GAP);
            }
        },
        None,
    );
    let compared = if st.comparing() {
        COMPARING.to_string()
    } else if st.compared(snap).is_some() {
        "compared".to_string()
    } else {
        "not compared yet".to_string()
    };
    blocks::section(
        ui,
        STRATEGIES,
        "Strategies",
        STRATEGIES_CARD,
        &compared,
        true,
        |ui| {
            ui.spacing_mut().item_spacing.y = metrics::ROW_GAP;
            compare(ui, st, snap, t, actions);
            ui.add_space(metrics::ROW_GAP);
        },
        None,
    );
    blocks::section(
        ui,
        LEGEND,
        "Legend",
        LEGEND_CARD,
        if firmware {
            "sent path, travel, kept, waste"
        } else {
            "head, tip, travel, spots, kept, waste"
        },
        true,
        |ui| legend(ui, snap, run, t),
        None,
    );
    centre
}

/// The panel while a simulation runs: the grade with "...", and the
/// Numbers section with "Simulating..." in its header and its rows
/// dimmed with "..." (so nothing jumps).
pub fn running_panel(ui: &mut Ui, t: &Tokens) {
    use super::inspector::blocks;
    ui.horizontal(|ui| {
        ui.spinner();
        let r = ui.add_enabled(
            false,
            egui::Label::new(RichText::new("...").font(metrics::heading())),
        );
        super::groups::card(&r, "Grade", GRADE_CARD, Some(SIMULATING_CARD), false);
    });
    let note = blocks::status_line(ui, "", t.text_dim);
    super::groups::card(&note, SIMULATING, SIMULATING_CARD, None, false);
    let names = [
        "Worst miss",
        "Worst corner",
        "Cut length",
        "Travel",
        "Time (estimate)",
    ];
    blocks::section_live(
        ui,
        NUMBERS,
        "Numbers",
        NUMBERS_CARD,
        SIMULATING,
        SIMULATING,
        true,
        |ui| {
            blocks::labels(ui, &names);
            for name in names {
                // Dimmed, and their cards say why.
                blocks::data_row(ui, name, SIMULATING_CARD, "...", Some(SIMULATING_CARD));
            }
        },
    );
}

/// The legend rows of the Simulate stage: name and hover card.
pub const LEGEND_HEAD: (&str, &str) = (
    "Head",
    "The thin grey line is where the head moves. The blade tip trails a little behind it, so the tip's line is the one that counts.",
);
pub const LEGEND_GOOD: (&str, &str) = (
    "On the line",
    "Green: the blade tip is within 0.06 mm of your line. That is as close as the plotter can cut.",
);
pub const LEGEND_WARN: (&str, &str) = (
    "Close",
    "Amber: the tip is within 0.12 mm of your line, twice the limit. Most materials forgive this.",
);
pub const LEGEND_BAD: (&str, &str) = (
    "Off the line",
    "Red: the tip is more than 0.12 mm from your line. Look here: a corner may be cut short or torn.",
);
pub const LEGEND_WASTE_MOVE: (&str, &str) = (
    "Waste move",
    "Dashed slate lines are blade-down moves in the waste on purpose: the turns and lead-ins that point the blade before a cut. They never touch what you keep.",
);
pub const LEGEND_ALIGNMENT: (&str, &str) = (
    "Alignment stroke",
    "The thick orange line is the alignment stroke: a short first cut in the waste that turns the blade to a known direction.",
);
pub const LEGEND_RING: (&str, &str) = (
    "Worst spot",
    "A ring marks a place beyond the limit. Hover it to see what happens there, click it to zoom in.",
);
pub const LEGEND_COMMANDED: (&str, &str) = (
    "Commanded path",
    "The path the plotter is told to move along. The firmware turns the blade around the corners itself, so FlatCraft cannot show where the tip really goes.",
);

fn legend(ui: &mut Ui, snap: &Snapshot, run: &SimRun, t: &Tokens) {
    use super::cut::LegendSample as S;
    let mat = canvas::mat_rgb(snap);
    let contrast = canvas::rgb(canvas::contrast_colour(mat));
    let travel = (
        super::cut::LEGEND_TRAVEL,
        S::Dotted,
        contrast.gamma_multiply(0.45),
    );
    let kept = (super::cut::LEGEND_KEPT, S::Kept, t.kept_paper);
    let waste = (
        super::cut::LEGEND_WASTE,
        S::Waste,
        super::kept::hatch_colour(mat, t),
    );
    let rows: Vec<((&str, &str), S, Color32)> = if run.result.is_some() {
        let [g, a, r] = flatcraft_core::colour::deviation_colours(mat).map(canvas::rgb);
        let mut rows = vec![
            (LEGEND_HEAD, S::Thin, head_colour()),
            (LEGEND_GOOD, S::Solid, g),
            (LEGEND_WARN, S::Solid, a),
            (LEGEND_BAD, S::Solid, r),
            (LEGEND_WASTE_MOVE, S::Dashed, waste_colour()),
        ];
        if run
            .job
            .plan
            .toolpaths
            .iter()
            .any(|t| t.kind == ToolpathKind::Alignment)
        {
            rows.push((
                LEGEND_ALIGNMENT,
                S::Thick,
                canvas::rgb(canvas::cut_colour(mat)),
            ));
        }
        rows.extend([travel, (LEGEND_RING, S::Ring, ring_colour()), kept, waste]);
        rows
    } else {
        vec![
            (
                LEGEND_COMMANDED,
                S::Solid,
                canvas::rgb(flatcraft_core::colour::commanded_colour(mat)),
            ),
            travel,
            kept,
            waste,
        ]
    };
    let rows: Vec<super::cut::LegendRow> = rows
        .into_iter()
        .map(|(text, s, c)| (text, s, c, None))
        .collect();
    super::cut::legend_rows(ui, mat, contrast, &rows);
}

/// The card of a strategy's numbers in the comparison.
pub const COMPARE_ROW_CARD: &str = "On target: the share of your line the blade cuts within the limit. Worst miss: the farthest the blade tip strays from your line. Travel: how far the head moves with the blade up. Crossings: blade-up moves over a finished cut, each can snag a loose piece. Time: a rough guess of the cut time.";

fn compare(ui: &mut Ui, st: &mut SimState, snap: &Snapshot, t: &Tokens, actions: &mut Vec<Action>) {
    let comparing = st.comparing();
    let Some(rows) = st.compared(snap).cloned() else {
        let r = ui.add_enabled(!comparing, egui::Button::new(COMPARE));
        let why = comparing.then_some("Wait a moment: the strategies are being compared.");
        super::groups::card(&r, COMPARE, COMPARE_CARD, why, false);
        if r.clicked() {
            st.request_compare();
        }
        if comparing {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(COMPARING).color(t.text_dim));
            });
        }
        return;
    };
    let runs = match rows {
        Ok(r) => r,
        Err(e) => {
            ui.label(RichText::new(super::person_text(&e)).color(t.danger));
            return;
        }
    };
    // One small block per strategy, so the panel keeps its width.
    let own = snap.job.strategy;
    for run in &runs {
        let name = super::settings_window::strategy_name(run.strategy);
        let mine = run.strategy == own;
        egui::Frame::new()
            .fill(if mine {
                t.surface
            } else {
                Color32::TRANSPARENT
            })
            .corner_radius(t.radius)
            .inner_margin(egui::Margin::symmetric(metrics::SPACE_8 as i8, metrics::SPACE_4 as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).strong());
                    if let Some(r) = &run.result {
                        let m = &r.metrics;
                        // The colour on a dot (3:1 for graphics), the
                        // text in the text colour (4.5:1 on any fill).
                        let (dot, _) =
                            ui.allocate_exact_size(
                                Vec2::new(metrics::DOT_SLOT, metrics::line_height(ui, &metrics::body())),
                                Sense::hover(),
                            );
                        ui.painter()
                            .circle_filled(dot.center(), 4.0, grade_colour(m.grade, t));
                        let g = ui.label(grade_short(m));
                        super::groups::card(&g, "Grade", GRADE_CARD, None, false);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if mine { "In use" } else { "Use" };
                        let r = ui.add_enabled(!mine, egui::Button::new(label).small());
                        let title = format!("Use {name}");
                        super::settings::set_name(ui, &r, &title);
                        let why = mine.then_some("This job already cuts in this order.");
                        super::groups::card(
                            &r,
                            &title,
                            "Cuts this document in this order. One undo step.",
                            why,
                            false,
                        );
                        if r.clicked() {
                            actions.push(Action::Run {
                                doc_id: snap.doc_id,
                                command: "job.set_strategy",
                                params: serde_json::json!({
                                    "strategy": simulate::strategy_name(run.strategy)
                                }),
                            });
                        }
                    });
                });
                let numbers = match &run.result {
                    Some(r) => {
                        let m = &r.metrics;
                        let crossings = match m.travel_crossings {
                            1 => "1 crossing".to_string(),
                            n => format!("{n} crossings"),
                        };
                        format!(
                            "{:.0} % on target · worst miss {} · travel {:.0} mm · {crossings} · {}",
                            m.score,
                            mm(m.max_hausdorff_mm),
                            m.travel_mm,
                            time_text(run.estimate_s)
                        )
                    }
                    None => format!(
                        "Travel {:.0} mm · {}",
                        run.commanded.travel_mm,
                        time_text(run.estimate_s)
                    ),
                };
                let n = ui.add(egui::Label::new(RichText::new(numbers).color(t.text_dim)).wrap());
                super::groups::card(&n, name, COMPARE_ROW_CARD, None, false);
            });
    }
    if runs.iter().all(|r| r.result.is_none()) {
        ui.label(
            RichText::new("Firmware mode: only the travel and the time are compared.")
                .color(t.text_dim),
        );
    }
}

/// The mode the plotter of `snap` simulates in, if there is one.
pub fn mode_of(snap: &Snapshot) -> Option<SimMode> {
    snap.plotter.as_ref().map(SimMode::of)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The metrics of a 50 mm square simulated with the blade offset `d`
    /// and no compensation.
    fn metrics(d: f64) -> flatcraft_sim::JobMetrics {
        let p = |x: f64, y: f64| kurbo::Point::new(x, y);
        let square = flatcraft_path::Toolpath::new(
            vec![
                p(10.0, 10.0),
                p(60.0, 10.0),
                p(60.0, 60.0),
                p(10.0, 60.0),
                p(10.0, 10.0),
            ],
            true,
            flatcraft_path::PathRef {
                object: flatcraft_core::document::ObjectId(1),
                path_index: 0,
            },
        );
        flatcraft_sim::run(
            &[square],
            kurbo::Point::ZERO,
            &flatcraft_sim::SimSettings::float(d),
            5,
            &flatcraft_sim::TimeModel::PROVISIONAL,
        )
        .unwrap()
        .metrics
    }

    /// The grade always comes with the reason that decided it, so a high
    /// score next to Poor never reads as a contradiction; the reason
    /// follows the engine's grading order.
    #[test]
    fn grade_reason_explains_a_grade_the_score_does_not() {
        let good = metrics(0.0);
        assert_eq!(good.grade, Grade::Good);
        assert_eq!(grade_reason(&good), None);
        assert_eq!(grade_line(&good), "Good");
        // Poor with a high score: cuts into kept material.
        let into_kept = flatcraft_sim::JobMetrics {
            grade: Grade::Poor,
            score: 98.0,
            max_into_kept_mm: 0.234,
            ..good.clone()
        };
        assert_eq!(
            grade_line(&into_kept),
            "Poor: cuts up to 0.23 mm into material you keep"
        );
        assert_eq!(grade_short(&into_kept), "Poor · cuts into kept");
        let missed = flatcraft_sim::JobMetrics {
            grade: Grade::Poor,
            score: 80.0,
            ..good.clone()
        };
        assert_eq!(grade_line(&missed), "Poor: misses 20 % of the line");
        let pushed = flatcraft_sim::JobMetrics {
            grade: Grade::Fair,
            pushed: 2,
            ..good.clone()
        };
        assert_eq!(
            grade_line(&pushed),
            "Fair: pushes the blade from behind 2 times"
        );
        let strays = flatcraft_sim::JobMetrics {
            grade: Grade::Fair,
            max_hausdorff_mm: 0.1,
            ..good.clone()
        };
        assert_eq!(
            grade_line(&strays),
            "Fair: strays up to 0.10 mm from your line"
        );
        // A real uncompensated square: graded worse than Good, with a reason.
        let pen = metrics(0.3);
        assert_ne!(pen.grade, Grade::Good);
        assert!(grade_reason(&pen).is_some());
        assert_eq!(on_target_text(97.6), "98 % of the line on target");
    }

    #[test]
    fn time_text_reads_plainly() {
        assert_eq!(time_text(4.4), "4 s");
        assert_eq!(time_text(65.0), "1 min 05 s");
    }
}
