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

/// Largest source file (an imported SVG or picture), bytes. The importer
/// refuses bigger files, so a project never needs more.
pub const MAX_SOURCE_BYTES: u64 = 50 * 1024 * 1024;

/// Largest `project.json` in a project file, bytes, unpacked.
pub const MAX_PROJECT_JSON_BYTES: u64 = 64 * 1024 * 1024;

/// Largest `thumbnail.png` in a project file, bytes.
pub const MAX_THUMBNAIL_BYTES: u64 = 16 * 1024 * 1024;

/// Most bytes a project file may unpack to, all entries together. Guards
/// against a zip bomb: a small file that unpacks to gigabytes.
pub const MAX_PROJECT_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;

/// Most entries in a project file.
pub const MAX_PROJECT_ENTRIES: usize = 10_000;
