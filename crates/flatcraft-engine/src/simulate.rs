//! The engine side of the knife simulator: the quality report of a job
//! (`job.simulate`), the comparison of the cut strategies
//! (`job.compare_strategies`) and the samples the Simulation preview draws.
//!
//! A simulation takes three steps, so the session lock is never held while
//! the knife is simulated:
//!
//! 1. [`snapshot`], under the lock: the document's objects, the resolved
//!    profiles, the strategy, the mode and the cached cut geometry as a
//!    seed. Cheap: nothing is cut or planned here.
//! 2. [`compute`], without the lock: the job is built (as `job.preview`
//!    builds it) and its cut toolpaths are simulated.
//! 3. [`store`], under the lock again: the run is kept as the document's
//!    last simulation only while the document and the profiles are still
//!    at the revisions of its key.
//!
//! The registry commands run the three steps in a row on the session they
//! get. `crate::unlocked` runs them on a snapshot of the document without
//! the lock (the MCP server and the window), and keeps the stored run only
//! when the document did not change meanwhile.

use std::sync::Arc;

use flatcraft_core::document::CutStrategy;
use flatcraft_device::{KnifeCompensation, Media, PlotterProfile};
use flatcraft_path::{CORNER_DEG, Toolpath, ToolpathKind};
use flatcraft_sim::{
    ALGORITHM_LIMIT_MM, DEVICE_LIMIT_MM, DeviceGrid, DisplaySample, END_TO_END_LIMIT_MM, Grade,
    SimError, SimResult, SimSettings, SpotKind, TimeModel, decimate, estimate_seconds,
    measure_kept, simulate,
};
use kurbo::Point;
use serde_json::{Value, json};

use crate::job::{self, JobInput, JobResult};
use crate::session::{CommandError, Session, UnlockedHook};

/// At most about this many samples are kept for drawing (see
/// [`flatcraft_sim::decimate`]).
pub const DISPLAY_POINTS: usize = 20_000;

/// At most this many toolpaths are listed in `worst_paths`.
pub const WORST_PATHS: usize = 20;

/// What the plotter does about the blade offset, so what can be simulated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimMode {
    /// FlatCraft steers the blade (host compensation): simulated.
    Host,
    /// No compensation at all (a pen): simulated with the profile's blade
    /// offset, so a knife wrongly set to "None" shows its real corners.
    Pen,
    /// The plotter's firmware compensates: not simulated, since what the
    /// firmware does is not known exactly yet.
    Firmware,
}

impl SimMode {
    /// The mode of `plotter`'s knife compensation.
    pub fn of(plotter: &PlotterProfile) -> Self {
        match plotter.knife_compensation {
            KnifeCompensation::Host { .. } => Self::Host,
            KnifeCompensation::None => Self::Pen,
            KnifeCompensation::Firmware { .. } => Self::Firmware,
        }
    }

    /// The name agents read.
    pub fn name(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Pen => "pen",
            Self::Firmware => "firmware",
        }
    }
}

/// Whether the simulator rounds every point to the plotter's device grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimGrid {
    /// Every run point is snapped to the device grid first: what the
    /// plotter really gets. The commands use this.
    Device,
    /// No rounding: the algorithm alone. Host compensation plans in exact
    /// floats (no device grid) and the simulator snaps nothing. Never sent.
    Float,
}

/// What a simulation was run for. A stored run is valid only while the
/// document and the profiles are still at these revisions.
#[derive(Debug, Clone, PartialEq)]
pub struct SimKey {
    /// The open document (`DocState::id`).
    pub doc_id: u64,
    pub doc_revision: u64,
    pub profiles_revision: u64,
    /// The strategy the job was planned with.
    pub strategy: CutStrategy,
    pub grid: SimGrid,
}

/// Everything a simulation reads, taken under the session lock.
#[derive(Debug, Clone)]
pub struct SimInput {
    /// The objects, the job settings (with the strategy to simulate), the
    /// resolved profiles and the cached cut geometry as a seed.
    pub input: JobInput,
    pub mode: SimMode,
    pub strategy: CutStrategy,
    pub key: SimKey,
    /// Tests only: the session's `unlocked_hook`, run at the start of
    /// [`compute`] with the id `job.simulate`.
    hook: Option<UnlockedHook>,
}

/// One simulated job.
#[derive(Debug, Clone)]
pub struct SimRun {
    pub key: SimKey,
    pub mode: SimMode,
    pub strategy: CutStrategy,
    /// The job as `job.preview` builds it.
    pub job: JobResult,
    /// The simulation of the toolpaths that are really cut
    /// ([`JobResult::cut_toolpaths`]); `None` in firmware mode.
    pub result: Option<Arc<SimResult>>,
    /// The samples to draw (see [`DISPLAY_POINTS`]); empty in firmware mode.
    pub display: Arc<Vec<DisplaySample>>,
    /// The job's warnings, and what the simulation adds.
    pub warnings: Vec<String>,
    /// For each simulated toolpath (the index the trace, the metrics and
    /// `display` use), its index in `job.plan.toolpaths`.
    pub plan_index: Vec<usize>,
    /// The listing note of every toolpath of the plan.
    pub notes: Vec<String>,
    /// The estimated job time in seconds (see [`TimeModel`]), every mode.
    pub estimate_s: f64,
    /// What the plotter is commanded to do, every mode.
    pub commanded: Commanded,
    /// The input, to offer its cut geometry to the document at store time.
    input: JobInput,
}

/// What stays of a [`SimRun`] once it is stale: the numbers, without the
/// trace, the samples and the job.
#[derive(Debug, Clone, PartialEq)]
pub struct SimSummary {
    pub key: SimKey,
    pub mode: SimMode,
    pub strategy: CutStrategy,
    /// The job metrics without the per-path list; `None` in firmware mode.
    pub metrics: Option<flatcraft_sim::JobMetrics>,
    pub estimate_s: f64,
    pub commanded: Commanded,
}

/// The commanded path of the toolpaths that are really cut, without any
/// knife model: what firmware mode can report.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Commanded {
    /// Blade-up travel from the home, mm.
    pub travel_mm: f64,
    /// Blade-down length of the runs, mm.
    pub cut_mm: f64,
    /// Axis segments of the runs.
    pub segments: usize,
    /// Blade-down runs (each ends with a lift).
    pub lifts: usize,
    /// The largest heading change at one vertex of a `Cut` path that is
    /// not a corner (below `CORNER_DEG`): how coarse the curves are.
    pub max_turn_per_vertex_deg: f64,
}

/// The note when the simulation picture's green, amber and red do not
/// reach 3:1 on the mat colour.
pub const MAT_COLOUR_NOTE: &str = "On this mat colour the green, amber and red of the simulation picture are hard to tell from the mat. Green is within the limit, amber within twice the limit and red beyond; the numbers of this report hold either way.";

/// The note `job.simulate` gives in firmware mode.
pub const FIRMWARE_NOTE: &str = "Firmware compensation cannot be simulated exactly yet. This shows the commanded path, travel, segment count and the largest heading change per vertex on curves.";

/// The simulator's device grid for `plotter` on `media`: the emitter's own
/// app-to-device transform (units per mm, grid origin, mirror and scale),
/// so the simulator rounds every point exactly as the plotter gets it.
pub fn device_grid(plotter: &PlotterProfile, media: &Media) -> DeviceGrid {
    flatcraft_device::gpgl::quantize::device_grid(plotter, media)
}

/// Step 1, under the lock: what the simulation of the active document
/// reads. `strategy` replaces the document's own for this run only.
pub fn snapshot(
    s: &Session,
    strategy: Option<CutStrategy>,
    grid: SimGrid,
) -> Result<SimInput, CommandError> {
    let st = s.doc();
    let doc = &st.document;
    let plotter = s
        .profiles
        .plotter(&doc.job.plotter_id)
        .map_err(crate::commands::profile_error)?;
    let material = s
        .profiles
        .material(&doc.job.material_id)
        .map_err(crate::commands::profile_error)?;
    let mut input =
        JobInput::from_parts(doc, plotter, &material).with_seed(st.cached_cut_geometry());
    let strategy = strategy.unwrap_or(doc.job.strategy);
    input.job.strategy = strategy;
    Ok(SimInput {
        mode: SimMode::of(&input.plotter),
        strategy,
        key: SimKey {
            doc_id: st.id(),
            doc_revision: st.revision(),
            profiles_revision: s.profiles.revision(),
            strategy,
            grid,
        },
        input,
        hook: s.unlocked_hook.clone(),
    })
}

pub(crate) fn sim_error(e: SimError) -> CommandError {
    match e {
        SimError::TooManySamples { .. } => CommandError::Refused(e.to_string()),
        SimError::NonFinite { .. } | SimError::BladeOffset { .. } => {
            CommandError::Failed(format!("cannot simulate the job: {e}"))
        }
    }
}

/// The turn from `a` to `b` in degrees, `None` when one has no length.
fn turn_deg(a: kurbo::Vec2, b: kurbo::Vec2) -> Option<f64> {
    (a.hypot2() > 0.0 && b.hypot2() > 0.0).then(|| a.cross(b).atan2(a.dot(b)).abs().to_degrees())
}

/// The commanded path of `toolpaths` from `home` (see [`Commanded`]).
pub fn commanded(toolpaths: &[Toolpath], home: Point) -> Commanded {
    let mut c = Commanded {
        travel_mm: 0.0,
        cut_mm: 0.0,
        segments: 0,
        lifts: 0,
        max_turn_per_vertex_deg: 0.0,
    };
    let mut here = home;
    for run in toolpaths.iter().flat_map(Toolpath::runs) {
        let Some(&first) = run.points.first() else {
            continue;
        };
        c.travel_mm += here.distance(first);
        c.lifts += 1;
        c.segments += run.points.len() - 1;
        c.cut_mm += run
            .points
            .windows(2)
            .map(|w| w[0].distance(w[1]))
            .sum::<f64>();
        here = *run.points.last().unwrap_or(&first);
    }
    for t in toolpaths.iter().filter(|t| t.kind == ToolpathKind::Cut) {
        for w in t.points.windows(3) {
            if let Some(a) = turn_deg(w[1] - w[0], w[2] - w[1])
                && a < CORNER_DEG
            {
                c.max_turn_per_vertex_deg = c.max_turn_per_vertex_deg.max(a);
            }
        }
    }
    c
}

/// What the simulation adds to the job's warnings.
fn sim_warnings(mode: SimMode, plotter: &PlotterProfile) -> Vec<String> {
    let d = plotter.blade_offset_mm;
    match mode {
        SimMode::Pen if d > 0.0 => vec![format!(
            "The plotter preset has no knife compensation, so this simulates a blade that trails {d} mm behind the head around every corner and at every start. For a pen, set the blade offset to 0 in the Profiles window. For a knife, pick Host or Firmware compensation."
        )],
        _ => Vec::new(),
    }
}

/// Step 2, without the lock: builds the job and simulates its cut
/// toolpaths. Host and pen mode simulate with the profile's blade offset,
/// on the device grid or in floats as the key says; firmware mode only
/// reports the commanded path (`result: None`).
pub fn compute(input: SimInput) -> Result<SimRun, CommandError> {
    if let Some(h) = &input.hook {
        (h.0)("job.simulate");
    }
    let SimInput {
        input,
        mode,
        strategy,
        key,
        hook: _,
    } = input;
    // The float run plans in floats too, so it measures the algorithm
    // alone; a plan rounded for the device would read the rounding.
    let units = match key.grid {
        SimGrid::Device => job::PlanUnits::Device,
        SimGrid::Float => job::PlanUnits::Float,
    };
    let job = job::build_planned(&input, units).map_err(|f| f.error)?;
    let plotter = &input.plotter;
    let (media, _) = crate::commands::effective_media(plotter, &input.job)?;
    let plan_index: Vec<usize> = job::cut_mask(&job)
        .into_iter()
        .enumerate()
        .filter_map(|(i, cut)| cut.then_some(i))
        .collect();
    let cut: Vec<Toolpath> = plan_index
        .iter()
        .map(|&i| job.plan.toolpaths[i].clone())
        .collect();
    let home = job.plan.home;
    let estimate_s = estimate_seconds(&cut, home, job.settings.speed, &TimeModel::PROVISIONAL);
    let notes = job::toolpath_notes(&job.plan, &input.objects, &input.cut_geometry());
    let mut warnings = job.warnings.clone();
    warnings.extend(sim_warnings(mode, plotter));
    if mode != SimMode::Firmware && !flatcraft_core::colour::deviation_colours_read(media.color) {
        warnings.push(MAT_COLOUR_NOTE.to_string());
    }
    let (result, display) = match mode {
        SimMode::Firmware => (None, Vec::new()),
        SimMode::Host | SimMode::Pen => {
            let d = plotter.blade_offset_mm;
            let settings = match key.grid {
                SimGrid::Device => SimSettings::device(d, device_grid(plotter, media)),
                SimGrid::Float => SimSettings::float(d),
            };
            let trace = simulate(&cut, home, &settings).map_err(sim_error)?;
            let (metrics, deviation_mm) = measure_kept(&cut, &trace, &settings, &job.plan.kept);
            let result = SimResult {
                trace,
                deviation_mm,
                metrics,
                seconds: estimate_s,
            };
            let display = decimate(&result, DISPLAY_POINTS);
            (Some(Arc::new(result)), display)
        }
    };
    Ok(SimRun {
        key,
        mode,
        strategy,
        commanded: commanded(&cut, home),
        job,
        result,
        display: Arc::new(display),
        warnings,
        plan_index,
        notes,
        estimate_s,
        input,
    })
}

/// True while `key` still describes the document `key.doc_id` of `s`.
fn fresh(s: &Session, key: &SimKey) -> bool {
    s.doc_index(key.doc_id).is_ok_and(|i| {
        s.docs[i].revision() == key.doc_revision && s.profiles.revision() == key.profiles_revision
    })
}

/// Step 3, under the lock: keeps `run` as its document's last simulation,
/// only while the document and the profiles are still at the revisions of
/// its key. Returns whether it was stored. The cut geometry the run built
/// is offered to the document's cache too.
pub fn store(s: &mut Session, run: Arc<SimRun>) -> bool {
    if !fresh(s, &run.key) {
        return false;
    }
    s.offer_cut_geometry(run.key.doc_id, run.key.doc_revision, &run.input);
    let Ok(i) = s.doc_index(run.key.doc_id) else {
        return false;
    };
    s.docs[i].set_last_sim(run);
    true
}

/// The stored simulation of the active document for `key`, if it is still
/// valid (see [`Session::last_sim`]).
fn reusable(s: &Session, key: &SimKey) -> Option<Arc<SimRun>> {
    s.last_sim().filter(|r| r.key == *key).cloned()
}

/// The simulation of the active document with `strategy` (the document's
/// own when `None`) on the device grid: the stored one while it is valid,
/// else the three steps in a row.
pub fn current(
    s: &mut Session,
    strategy: Option<CutStrategy>,
) -> Result<Arc<SimRun>, CommandError> {
    let input = snapshot(s, strategy, SimGrid::Device)?;
    if let Some(run) = reusable(s, &input.key) {
        return Ok(run);
    }
    let run = Arc::new(compute(input)?);
    store(s, run.clone());
    Ok(run)
}

/// [`current`] without storing anything: for a caller that only reads
/// the session (a preview image).
pub fn current_unstored(s: &Session) -> Result<Arc<SimRun>, CommandError> {
    let input = snapshot(s, None, SimGrid::Device)?;
    if let Some(run) = reusable(s, &input.key) {
        return Ok(run);
    }
    Ok(Arc::new(compute(input)?))
}

/// The strategies `job.compare_strategies` compares, in its order.
pub const STRATEGIES: [CutStrategy; 3] = [
    CutStrategy::Quality,
    CutStrategy::Fast,
    CutStrategy::OutlinesLast,
];

/// One simulation per strategy of [`STRATEGIES`], computed side by side
/// (the stored one reused while valid). The run of the document's own
/// strategy is stored.
pub fn compare(s: &mut Session) -> Result<Vec<Arc<SimRun>>, CommandError> {
    let inputs = STRATEGIES
        .iter()
        .map(|&st| snapshot(s, Some(st), SimGrid::Device))
        .collect::<Result<Vec<_>, _>>()?;
    let own = s.doc().document.job.strategy;
    let runs: Vec<Result<Arc<SimRun>, CommandError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = inputs
            .into_iter()
            .map(|input| match reusable(s, &input.key) {
                Some(run) => Ok(run),
                None => Err(scope.spawn(move || compute(input).map(Arc::new))),
            })
            .collect();
        handles
            .into_iter()
            .map(|h| match h {
                Ok(run) => Ok(run),
                Err(handle) => handle.join().unwrap_or_else(|_| {
                    Err(crate::session::internal_error("job.compare_strategies"))
                }),
            })
            .collect()
    });
    let runs = runs.into_iter().collect::<Result<Vec<_>, _>>()?;
    if let Some(run) = runs.iter().find(|r| r.strategy == own) {
        store(s, run.clone());
    }
    Ok(runs)
}

/// The name of a strategy, as `job.set_strategy` takes it.
pub fn strategy_name(s: CutStrategy) -> &'static str {
    match s {
        CutStrategy::Quality => "quality",
        CutStrategy::Fast => "fast",
        CutStrategy::OutlinesLast => "outlines_last",
    }
}

/// The name of a grade.
pub fn grade_name(g: Grade) -> &'static str {
    match g {
        Grade::Good => "good",
        Grade::Fair => "fair",
        Grade::Poor => "poor",
    }
}

fn kind_name(k: SpotKind) -> &'static str {
    match k {
        SpotKind::Deviation => "deviation",
        SpotKind::Uncut => "uncut",
        SpotKind::Pushed => "pushed",
        SpotKind::IntoKept => "into_kept",
    }
}

fn spot_text(k: SpotKind, d: f64) -> String {
    match k {
        SpotKind::Deviation => format!("The cut strays {d:.3} mm from the line"),
        SpotKind::Uncut => format!("The line is not cut through here ({d:.3} mm off)"),
        SpotKind::Pushed => "The blade is pushed from behind here".to_string(),
        SpotKind::IntoKept => format!("The blade tip cuts {d:.3} mm into material you keep"),
    }
}

impl SimRun {
    /// The plan index and the 1-based number (as the path preview numbers
    /// the toolpaths) of simulated toolpath `i`.
    fn number(&self, i: usize) -> (usize, usize) {
        let p = self.plan_index.get(i).copied().unwrap_or(i);
        (p, p + 1)
    }

    fn note_of(&self, i: usize) -> String {
        let (p, _) = self.number(i);
        self.notes.get(p).cloned().unwrap_or_default()
    }

    /// The numbers of this run without its trace, samples and job.
    pub fn summary(&self) -> SimSummary {
        SimSummary {
            key: self.key.clone(),
            mode: self.mode,
            strategy: self.strategy,
            metrics: self.result.as_ref().map(|r| flatcraft_sim::JobMetrics {
                per_path: Vec::new(),
                ..r.metrics.clone()
            }),
            estimate_s: self.estimate_s,
            commanded: self.commanded,
        }
    }

    /// The score, `None` in firmware mode.
    pub fn score(&self) -> Option<f64> {
        self.result.as_ref().map(|r| r.metrics.score)
    }

    /// The grade, `None` in firmware mode.
    pub fn grade(&self) -> Option<Grade> {
        self.result.as_ref().map(|r| r.metrics.grade)
    }

    /// The report of `job.simulate`.
    pub fn report_json(&self) -> Value {
        let limits = json!({
            "algorithm_mm": ALGORITHM_LIMIT_MM,
            "device_mm": DEVICE_LIMIT_MM,
            "end_to_end_mm": END_TO_END_LIMIT_MM,
        });
        let c = &self.commanded;
        let Some(r) = &self.result else {
            return json!({
                "mode": self.mode.name(),
                "simulated": false,
                "note": FIRMWARE_NOTE,
                "strategy": strategy_name(self.strategy),
                "score": null,
                "grade": null,
                "metrics": {
                    "travel_mm": c.travel_mm,
                    "cut_mm": c.cut_mm,
                    "segments": c.segments,
                    "lifts": c.lifts,
                },
                "max_turn_per_vertex_deg": c.max_turn_per_vertex_deg,
                "estimate_s": self.estimate_s,
                "limits": limits,
                "worst_spots": [],
                "worst_paths": [],
                "warnings": self.warnings,
            });
        };
        let m = &r.metrics;
        let worst_spots: Vec<Value> = m
            .worst_spots
            .iter()
            .map(|s| {
                let (_, number) = self.number(s.toolpath);
                json!({
                    "x_mm": s.at.x,
                    "y_mm": s.at.y,
                    "deviation_mm": s.deviation_mm,
                    "kind": kind_name(s.kind),
                    "toolpath": number,
                    "note": format!("{} ({}).", spot_text(s.kind, s.deviation_mm), self.note_of(s.toolpath)),
                })
            })
            .collect();
        let mut paths: Vec<&flatcraft_sim::PathMetrics> = m
            .per_path
            .iter()
            .filter(|p| {
                self.job
                    .plan
                    .toolpaths
                    .get(self.number(p.toolpath).0)
                    .is_some_and(|t| t.kind == ToolpathKind::Cut)
            })
            .collect();
        paths.sort_by(|a, b| b.hausdorff_mm.total_cmp(&a.hausdorff_mm));
        let worst_paths: Vec<Value> = paths
            .iter()
            .take(WORST_PATHS)
            .map(|p| {
                let (_, number) = self.number(p.toolpath);
                json!({
                    "toolpath": number,
                    "note": self.note_of(p.toolpath),
                    "hausdorff_mm": p.hausdorff_mm,
                    "corner_deviation_mm": p.max_corner_deviation_mm,
                    "start_deviation_mm": p.max_start_deviation_mm,
                    "closure_gap_mm": p.closure_gap_mm,
                    "uncut_mm": p.uncut_mm,
                    "stray_mm": p.stray_mm,
                    "pushed": p.pushed,
                })
            })
            .collect();
        json!({
            "mode": self.mode.name(),
            "simulated": true,
            "strategy": strategy_name(self.strategy),
            "score": m.score,
            "grade": grade_name(m.grade),
            "metrics": {
                "max_hausdorff_mm": m.max_hausdorff_mm,
                "max_correspondence_mm": m.max_correspondence_mm,
                "max_span_frechet_mm": m.max_span_frechet_mm,
                "max_corner_deviation_mm": m.max_corner_deviation_mm,
                "max_into_kept_mm": m.max_into_kept_mm,
                "max_twist_deg": m.max_twist_deg,
                "max_start_deviation_mm": m.max_start_deviation_mm,
                "max_closure_gap_mm": m.max_closure_gap_mm,
                "uncut_mm": m.uncut_mm,
                "stray_mm": m.stray_mm,
                "waste_mm": m.waste_mm,
                "cut_mm": m.cut_mm,
                "travel_mm": m.travel_mm,
                "travel_crossings": m.travel_crossings,
                "segments": m.segments,
                "lifts": m.lifts,
                "pushed": m.pushed,
            },
            "estimate_s": self.estimate_s,
            "limits": limits,
            "worst_spots": worst_spots,
            "worst_paths": worst_paths,
            "warnings": self.warnings,
        })
    }

    /// The row of `job.compare_strategies`.
    pub fn compare_json(&self) -> Value {
        let c = &self.commanded;
        match &self.result {
            Some(r) => {
                let m = &r.metrics;
                json!({
                    "strategy": strategy_name(self.strategy),
                    "score": m.score,
                    "grade": grade_name(m.grade),
                    "max_hausdorff_mm": m.max_hausdorff_mm,
                    "travel_mm": m.travel_mm,
                    "travel_crossings": m.travel_crossings,
                    "segments": m.segments,
                    "lifts": m.lifts,
                    "estimate_s": self.estimate_s,
                })
            }
            None => json!({
                "strategy": strategy_name(self.strategy),
                "score": null,
                "grade": null,
                "max_hausdorff_mm": null,
                "travel_mm": c.travel_mm,
                "travel_crossings": null,
                "segments": c.segments,
                "lifts": c.lifts,
                "estimate_s": self.estimate_s,
            }),
        }
    }
}

fn grade_rank(g: Grade) -> u8 {
    match g {
        Grade::Good => 0,
        Grade::Fair => 1,
        Grade::Poor => 2,
    }
}

/// The run with the best quality: the best grade, then the smallest
/// `max_hausdorff_mm`, then the highest score; the first of equals.
/// `None` when nothing was simulated (firmware mode).
pub fn best_quality(runs: &[Arc<SimRun>]) -> Option<&SimRun> {
    runs.iter()
        .filter_map(|r| r.result.as_ref().map(|res| (r, &res.metrics)))
        .min_by(|(_, a), (_, b)| {
            grade_rank(a.grade)
                .cmp(&grade_rank(b.grade))
                .then(a.max_hausdorff_mm.total_cmp(&b.max_hausdorff_mm))
                .then(b.score.total_cmp(&a.score))
        })
        .map(|(r, _)| r.as_ref())
}

/// The run with the smallest time estimate; the first of equals.
pub fn fastest(runs: &[Arc<SimRun>]) -> Option<&SimRun> {
    runs.iter()
        .min_by(|a, b| a.estimate_s.total_cmp(&b.estimate_s))
        .map(AsRef::as_ref)
}
