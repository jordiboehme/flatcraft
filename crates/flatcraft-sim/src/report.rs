//! The time estimate and the full simulation result.

use flatcraft_path::{CutPlan, Toolpath};
use kurbo::Point;

use crate::metrics::{JobMetrics, measure, measure_kept};
use crate::{SimError, SimSettings, SimTrace, simulate};

/// How fast the plotter moves. A provisional model until the M3 timing
/// measurements: every text that shows its result calls it an estimate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeModel {
    /// Axis speed in mm/s per step of the speed setting.
    pub mm_per_s_per_speed: f64,
    /// The time of one lift (one blade-down run), in seconds.
    pub lift_s: f64,
}

impl TimeModel {
    /// A placeholder until M3 measures the real plotter.
    pub const PROVISIONAL: Self = Self {
        mm_per_s_per_speed: 10.0,
        lift_s: 0.15,
    };
}

/// The estimated job time in seconds (R8): both axes move together, so
/// each axis segment and each travel move (from `home` on) takes
/// `max(|dx|, |dy|) / (speed * mm_per_s_per_speed)`, plus `lift_s` per
/// blade-down run. A speed of 0 counts as 1.
pub fn estimate_seconds(toolpaths: &[Toolpath], home: Point, speed: u8, m: &TimeModel) -> f64 {
    let mm_per_s = f64::from(speed.max(1)) * m.mm_per_s_per_speed;
    let chebyshev = |a: Point, b: Point| (a.x - b.x).abs().max((a.y - b.y).abs());
    let mut here = home;
    let (mut mm, mut lifts) = (0.0, 0usize);
    for run in toolpaths.iter().flat_map(Toolpath::runs) {
        if run.points.is_empty() {
            continue;
        }
        lifts += 1;
        for &p in &run.points {
            mm += chebyshev(here, p);
            here = p;
        }
    }
    mm / mm_per_s + lifts as f64 * m.lift_s
}

/// A simulated job: the trace, the deviation of each sample (see
/// [`measure`]), the metrics and the estimated time.
#[derive(Debug, Clone, PartialEq)]
pub struct SimResult {
    pub trace: SimTrace,
    pub deviation_mm: Vec<f32>,
    pub metrics: JobMetrics,
    /// An estimate (see [`TimeModel`]).
    pub seconds: f64,
}

/// Simulate `toolpaths` from `home`, measure the trace and estimate the time.
pub fn run(
    toolpaths: &[Toolpath],
    home: Point,
    s: &SimSettings,
    speed: u8,
    time: &TimeModel,
) -> Result<SimResult, SimError> {
    let trace = simulate(toolpaths, home, s)?;
    let (metrics, deviation_mm) = measure(toolpaths, &trace, s);
    Ok(SimResult {
        seconds: estimate_seconds(toolpaths, home, speed, time),
        trace,
        deviation_mm,
        metrics,
    })
}

/// Simulate a planned job from its home, measure it with the plan's own
/// kept material ([`measure_kept`]) and estimate the time.
pub fn run_plan(
    plan: &CutPlan,
    s: &SimSettings,
    speed: u8,
    time: &TimeModel,
) -> Result<SimResult, SimError> {
    let trace = simulate(&plan.toolpaths, plan.home, s)?;
    let (metrics, deviation_mm) = measure_kept(&plan.toolpaths, &trace, s, &plan.kept);
    Ok(SimResult {
        seconds: estimate_seconds(&plan.toolpaths, plan.home, speed, time),
        trace,
        deviation_mm,
        metrics,
    })
}
