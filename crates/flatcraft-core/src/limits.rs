//! Sane limits of cut settings, shared by the profile checks and the cut
//! pipeline.

/// Smallest flatten or join tolerance, mm; below it flattening takes ages.
pub const MIN_TOLERANCE_MM: f64 = 0.001;
/// Largest flatten or join tolerance, mm; above it curves turn into
/// polygons you can see.
pub const MAX_TOLERANCE_MM: f64 = 1.0;
/// Largest overcut, mm.
pub const MAX_OVERCUT_MM: f64 = 5.0;
/// Largest pivot threshold of host knife compensation, degrees.
pub const MAX_PIVOT_THRESHOLD_DEG: f64 = 180.0;

/// Allowed plotter scale factors (`scale_x`, `scale_y`). A calibration
/// corrects a few percent; anything outside this range is a typo.
pub const SCALE_RANGE: std::ops::RangeInclusive<f64> = 0.5..=2.0;
