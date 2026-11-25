//! FlatCraft knife simulator: where the blade tip of a drag knife really goes.
//!
//! The model (spec 8.6) is a pursuit model: the tip trails the axis at the
//! blade offset `d` and moves only along the tip-to-axis direction. `model`
//! holds the closed-form step along one straight axis segment, `trace` runs
//! it over a list of toolpaths, `metrics` measures the trace against the
//! intended paths and `report` adds the time estimate.

mod grid;
pub mod metrics;
pub mod model;
pub mod report;
pub mod trace;

use flatcraft_path::ASSUMED_START_HEADING;
use kurbo::{Point, Vec2};

pub use metrics::{Grade, JobMetrics, PathMetrics, Spot, SpotKind, measure};
pub use report::{SimResult, TimeModel, estimate_seconds, run};
pub use trace::{Sample, SimTrace, simulate};

/// The quality limit of the float simulation against the intended path (R7).
pub const ALGORITHM_LIMIT_MM: f64 = 0.005;
/// The quality limit of the quantized device output (R7).
pub const DEVICE_LIMIT_MM: f64 = 0.03;
/// The spec target: device deviation plus the upstream error (R7).
pub const END_TO_END_LIMIT_MM: f64 = 0.05;
/// Intended length with no trace within this is not severed: uncut
/// (Ruling M2-P14, twice `END_TO_END_LIMIT_MM`). Device quantization alone
/// stays well below it.
pub const UNCUT_LIMIT_MM: f64 = 2.0 * END_TO_END_LIMIT_MM;
/// A blade-down move whose push angle is above this is pushed from behind.
pub const PUSHED_DEG: f64 = 150.0;
/// The sample step is the blade offset divided by this (pinned in Task 10).
pub const SAMPLE_STEP_FACTOR: f64 = 20.0;

/// The smallest sample step. A step below it (or one that is not a
/// positive number) is raised to it, so a bad setting can not hang the loop.
pub const MIN_SAMPLE_STEP_MM: f64 = 0.005;

/// The most samples one simulation makes (about 400 MB of samples).
pub const MAX_SAMPLES: usize = 5_000_000;

/// Why a simulation could not run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SimError {
    /// A run point (`toolpath: Some(i)`) or the home (`toolpath: None`) is not finite.
    NonFinite {
        toolpath: Option<usize>,
        run: usize,
        index: usize,
    },
    /// The blade offset is negative or not finite.
    BladeOffset { value: f64 },
    /// The trace would have more than `limit` samples.
    TooManySamples { limit: usize },
}

impl std::fmt::Display for SimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonFinite { toolpath: None, .. } => {
                write!(
                    f,
                    "The home position has a coordinate that is not a number."
                )
            }
            Self::NonFinite {
                toolpath: Some(t),
                run,
                index,
            } => write!(
                f,
                "Toolpath {t}, run {run}, point {index} has a coordinate that is not a number."
            ),
            Self::BladeOffset { value } => write!(
                f,
                "The blade offset must be a finite number of at least 0 mm (got {value})."
            ),
            Self::TooManySamples { limit } => write!(
                f,
                "The simulation would need more than {limit} samples. Simulate fewer paths at a time."
            ),
        }
    }
}

impl std::error::Error for SimError {}

/// `d / SAMPLE_STEP_FACTOR`, at least `MIN_SAMPLE_STEP_MM` so a pen profile does not
/// make 1000 samples per mm, and 0.01 mm with no offset.
fn sample_step_mm(d: f64) -> f64 {
    if d <= 0.0 {
        0.01
    } else {
        (d / SAMPLE_STEP_FACTOR).max(MIN_SAMPLE_STEP_MM)
    }
}

/// The device rounding: absolute coordinates on a grid of `1 / units_per_mm`
/// that starts at minus `origin` in app millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceGrid {
    pub units_per_mm: f64,
    /// The grid origin of the mat (app 0,0 lies at `origin` in device millimetres).
    pub origin: Point,
}

impl DeviceGrid {
    /// The emitter's rounding without mirror and scale (they only correct
    /// the machine's own error): `round((p + origin) * upm) / upm - origin`
    /// per axis.
    pub fn snap(&self, p: Point) -> Point {
        let upm = self.units_per_mm;
        Point::new(
            ((p.x + self.origin.x) * upm).round() / upm - self.origin.x,
            ((p.y + self.origin.y) * upm).round() / upm - self.origin.y,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimSettings {
    /// The real blade offset `d` in mm.
    pub blade_offset_mm: f64,
    /// The blade heading before the first move.
    pub start_heading: Vec2,
    /// The largest distance between two samples, in mm of axis travel.
    pub sample_step_mm: f64,
    /// Snap every run point to this grid first (device mode).
    pub grid: Option<DeviceGrid>,
    /// The strict tolerance: a waste tip (lead-in, overshoot, alignment)
    /// closer than this to a cut is on the cut, and farther inside kept
    /// material is into-kept. Uncut uses [`UNCUT_LIMIT_MM`] instead
    /// (Ruling M2-P14).
    pub uncut_eps_mm: f64,
}

impl SimSettings {
    /// The float simulation: no grid, and a strict tolerance of 0.02 mm (R7).
    pub fn float(blade_offset_mm: f64) -> Self {
        Self {
            blade_offset_mm,
            start_heading: ASSUMED_START_HEADING,
            sample_step_mm: sample_step_mm(blade_offset_mm),
            grid: None,
            uncut_eps_mm: 0.02,
        }
    }

    /// The device simulation: every run point snapped to `grid`, and a
    /// strict tolerance of `DEVICE_LIMIT_MM`.
    pub fn device(blade_offset_mm: f64, grid: DeviceGrid) -> Self {
        Self {
            grid: Some(grid),
            uncut_eps_mm: DEVICE_LIMIT_MM,
            ..Self::float(blade_offset_mm)
        }
    }
}
