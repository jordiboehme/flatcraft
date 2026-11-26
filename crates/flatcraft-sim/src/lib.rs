//! FlatCraft knife simulator: where the blade tip of a drag knife really goes.
//!
//! The model (spec 8.6) is a pursuit model: the tip trails the axis at the
//! blade offset `d` and moves only along the tip-to-axis direction. `model`
//! holds the closed-form step along one straight axis segment, `trace` runs
//! it over a list of toolpaths, `metrics` measures the trace against the
//! intended paths and `report` adds the time estimate.

mod grid;
pub mod metrics;
pub mod report;
pub mod trace;

use flatcraft_path::ASSUMED_START_HEADING;
use kurbo::Vec2;

pub use metrics::{Grade, JobMetrics, PathMetrics, Spot, SpotKind, measure, measure_kept};
pub use report::{SimResult, TimeModel, estimate_seconds, run, run_plan};
pub use trace::{Sample, SimTrace, simulate};

/// The drag knife model. It lives in `flatcraft-path`, whose knife stage
/// uses it too ([`flatcraft_path::quantize`]); one model for both.
pub mod model {
    pub use flatcraft_path::model::{PUSHED_DEG, drag, push_angle_deg};
}
pub use flatcraft_path::DeviceGrid;
pub use model::PUSHED_DEG;

/// The quality limit of the float simulation against the intended path (R7).
pub const ALGORITHM_LIMIT_MM: f64 = 0.005;
/// The quality limit of the rounded device output against the planned
/// path (R7): 1.5 device units of the Cameo 2 (20 units per mm). Whole
/// units alone move a point up to `0.025 * (|sin a| + |cos a|)` off a line
/// at angle `a` (0.0354 mm at 45 deg), and on the pivot arcs of radius d,
/// only a few units across, the trailing tip adds to that. The worst over
/// 32 grid positions, plain and calibrated, measured 0.0569 mm (Task 10,
/// `docs/quality/knife-compensation.md`). The guarantee that counts is
/// [`END_TO_END_LIMIT_MM`], against the source shape. Whether `D` takes
/// decimals or half units is an open capture question
/// (`docs/protocol/gpgl.md`).
pub const DEVICE_LIMIT_MM: f64 = 0.075;
/// The device mode's strict tolerance for waste moves (lead-ins,
/// overshoots, the alignment stroke): a waste tip closer than this to a cut
/// is on the cut, and farther inside kept material is into-kept. One device
/// unit (0.05 mm), what the rounding alone can do. It does not follow
/// [`DEVICE_LIMIT_MM`], which also covers the tip drag on the pivot arcs.
pub const DEVICE_WASTE_TOLERANCE_MM: f64 = 0.05;
/// The end-to-end limit (R7), the guarantee that counts: the cut against
/// the source shape, after flattening, the knife stage and the device
/// rounding, at every grid position. 0.06 mm: one device unit plus the
/// flatten tolerance. The spec's first target of 0.05 mm cannot be held
/// with whole 0.05 mm units (Task 10, `docs/quality/knife-compensation.md`).
pub const END_TO_END_LIMIT_MM: f64 = 0.05 + flatcraft_path::plan::DEFAULT_TOLERANCE_MM;
/// Intended length with no trace within this is not severed: uncut
/// (Ruling M2-P14, twice `END_TO_END_LIMIT_MM`, so 0.12 mm). Device
/// quantization alone stays well below it.
pub const UNCUT_LIMIT_MM: f64 = 2.0 * END_TO_END_LIMIT_MM;
/// The sample step is the blade offset divided by this. Pinned by Task
/// 10's `sample_step_converges`: the smallest of 10, 20, 40 and 80 whose
/// float metrics stay within 0.0005 mm of those at 80 on the runs as
/// planned, and within 0.005 mm with the first start entered at 90 deg to
/// the assumed heading. 10 is enough as planned (0.0002 mm), but reads a
/// misaligned first start up to 0.012 mm low; 20 reads it at most
/// 0.005 mm low. At d 0.25 and 0.3 the step is 0.0125 and 0.015 mm, above
/// [`MIN_SAMPLE_STEP_MM`], which only clamps factors above 50 to 60.
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
    /// strict tolerance of [`DEVICE_WASTE_TOLERANCE_MM`].
    pub fn device(blade_offset_mm: f64, grid: DeviceGrid) -> Self {
        Self {
            grid: Some(grid),
            uncut_eps_mm: DEVICE_WASTE_TOLERANCE_MM,
            ..Self::float(blade_offset_mm)
        }
    }
}
