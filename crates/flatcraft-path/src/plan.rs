//! The baseline cut pipeline: types and the `plan` entry point.

use std::ops::Range;

use flatcraft_core::document::ObjectId;
use flatcraft_core::limits::{MAX_OVERCUT_MM, MAX_TOLERANCE_MM, MIN_TOLERANCE_MM};
use kurbo::{BezPath, PathEl, Point, Rect, Vec2};

use crate::{clean, flatten, order, overcut, passes};

/// Which path of which object a toolpath came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathRef {
    pub object: ObjectId,
    pub path_index: usize,
}

/// A vector path placed on the mat, in app millimetres (origin = the 0,0 of the grid printed on the mat, y down).
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedPath {
    pub path: BezPath,
    pub source: PathRef,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PipelineSettings {
    pub cutting_area: Rect,
    pub flatten_tolerance_mm: f64,
    pub join_tolerance_mm: f64,
    pub overcut_mm: f64,
    pub passes: u32,
    pub reverse_alternate_passes: bool,
    /// Where the blade starts: the device home (the media corner the
    /// plotter homes to), in app millimetres. It lies outside the grid when
    /// the grid does not start at the media corner, for example at minus
    /// the grid origin.
    pub home: Point,
}

/// The side of a closed ring the kept material lies on. Open paths use `Both`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepSide {
    Inside,
    Outside,
    Both,
}

/// What a toolpath is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolpathKind {
    /// Part of the design.
    Cut,
    /// The optional alignment stroke: the first blade-down run of a job,
    /// which turns the blade to a known heading. It counts as waste.
    Alignment,
}

/// A short straight stroke that turns the blade to `direction` before the first cut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignmentStroke {
    pub start: Point,
    pub direction: Vec2,
    pub length_mm: f64,
}

/// The `seg` value of a deliberate waste move: a swivel, a lead-in, an
/// overshoot or the alignment stroke.
pub const WASTE_SEG: u32 = u32::MAX;

/// The heading the blade is assumed to have before the first cut of a job
/// when there is no alignment stroke.
pub const ASSUMED_START_HEADING: Vec2 = Vec2::new(1.0, 0.0);

/// A turn of at least this many degrees is a corner.
pub const CORNER_DEG: f64 = 30.0;

/// One blade-down run of the axis (the knife holder).
#[derive(Debug, Clone, PartialEq)]
pub struct AxisRun {
    pub points: Vec<Point>,
    /// `seg[j]` is the index into the toolpath's `points` of the intended
    /// segment (`points[i]` to `points[i + 1]`) executed while the axis moves
    /// to `points[j]` of this run. A pivot arc carries the segment that ends
    /// at its vertex. [`WASTE_SEG`] marks deliberate waste moves only;
    /// `seg[0]` is always `WASTE_SEG` (the drop point).
    pub seg: Vec<u32>,
}

/// The blade is down along `points`; a travel move lies between consecutive toolpaths.
#[derive(Debug, Clone, PartialEq)]
pub struct Toolpath {
    /// The intended tip path: the flattened, cleaned path with its laps
    /// and overcut.
    pub points: Vec<Point>,
    pub closed: bool,
    pub source: PathRef,
    /// 1-based pass number.
    pub pass: u32,
    /// How many laps of a closed ring are chained in `points` (1 by default).
    pub laps: u32,
    pub keep: KeepSide,
    pub kind: ToolpathKind,
    /// The axis runs. `None` means one run along `points` (no host compensation).
    pub axis: Option<Vec<AxisRun>>,
    /// Where the first lap of a closed ring lies in `points`, as a closed
    /// ring (first point equals last point). The whole of `points` for an
    /// open path. Code that rewrites `points` keeps it in step.
    pub lap_range: Range<usize>,
}

impl Toolpath {
    /// A single-pass, single-lap `Cut` toolpath with `Both` sides kept, no
    /// axis runs and the whole of `points` as its lap.
    pub fn new(points: Vec<Point>, closed: bool, source: PathRef) -> Self {
        let lap_range = 0..points.len();
        Self {
            points,
            closed,
            source,
            pass: 1,
            laps: 1,
            keep: KeepSide::Both,
            kind: ToolpathKind::Cut,
            axis: None,
            lap_range,
        }
    }

    /// The axis runs. Without host compensation this is one run along
    /// `points` with `seg = [WASTE_SEG, 0, 1, ..]`.
    pub fn runs(&self) -> Vec<AxisRun> {
        match &self.axis {
            Some(runs) => runs.clone(),
            None => {
                let seg = std::iter::once(WASTE_SEG)
                    .chain((0..self.points.len().saturating_sub(1)).map(|i| i as u32))
                    .collect();
                vec![AxisRun {
                    points: self.points.clone(),
                    seg,
                }]
            }
        }
    }

    /// The first lap of a closed ring, closed, without the other laps and
    /// without the overcut. For an open path, all of `points`.
    pub fn lap(&self) -> &[Point] {
        if self.closed {
            &self.points[self.lap_range.clone()]
        } else {
            &self.points
        }
    }
}

/// The compensation a [`CutPlan`] carries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppliedCompensation {
    /// The points are the intended tip path (no host compensation).
    Tip,
    /// The axis runs are host compensated with this offset.
    Host { offset_mm: f64 },
    /// A host calibration series: each tile has its own offset.
    HostCalibration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CutPlan {
    pub toolpaths: Vec<Toolpath>,
    /// Sum of travel distances, starting from the device home.
    pub travel_mm: f64,
    /// Total blade-down length, including overcut and repeated passes.
    pub cut_mm: f64,
    /// Where the first travel move starts (`PipelineSettings::home`).
    pub home: Point,
    pub compensation: AppliedCompensation,
}

/// The pipeline setting an [`PlanError::InvalidSettings`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    FlattenTolerance,
    JoinTolerance,
    Overcut,
    Passes,
    CuttingArea,
    Home,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PlanError {
    #[error("there is nothing to cut")]
    Empty,
    #[error("{}", invalid_settings_message(*.field, *.value))]
    InvalidSettings { field: SettingsField, value: f64 },
    #[error("{} object(s) reach outside the cutting area: {}", .objects.len(), format_ids(.objects))]
    OutOfArea { objects: Vec<ObjectId> },
}

/// Anything shorter than this has no length.
const ZERO_LEN_MM: f64 = 1e-9;

fn validate(s: &PipelineSettings) -> Result<(), PlanError> {
    use SettingsField as F;
    let bad = |field, value| Err(PlanError::InvalidSettings { field, value });
    for (field, v) in [
        (F::FlattenTolerance, s.flatten_tolerance_mm),
        (F::JoinTolerance, s.join_tolerance_mm),
    ] {
        if !(MIN_TOLERANCE_MM..=MAX_TOLERANCE_MM).contains(&v) {
            return bad(field, v);
        }
    }
    if !(0.0..=MAX_OVERCUT_MM).contains(&s.overcut_mm) {
        return bad(F::Overcut, s.overcut_mm);
    }
    if s.passes < 1 {
        return bad(F::Passes, f64::from(s.passes));
    }
    for v in [s.home.x, s.home.y] {
        if !v.is_finite() {
            return bad(F::Home, v);
        }
    }
    let a = s.cutting_area;
    for v in [a.x0, a.y0, a.x1, a.y1] {
        if !v.is_finite() {
            return bad(F::CuttingArea, v);
        }
    }
    for size in [a.x1 - a.x0, a.y1 - a.y0] {
        if size <= 0.0 {
            return bad(F::CuttingArea, size);
        }
    }
    Ok(())
}

/// Every point of the flattened path lies in `area`, with a 1e-6 mm epsilon so
/// an object aligned exactly to the margin passes. A MoveTo that no LineTo
/// follows draws nothing and moves no blade, so it is not checked.
fn inside_area(flat: &BezPath, area: Rect) -> bool {
    const EPS: f64 = 1e-6;
    let inside = |p: &Point| {
        (area.x0 - EPS..=area.x1 + EPS).contains(&p.x)
            && (area.y0 - EPS..=area.y1 + EPS).contains(&p.y)
    };
    let els = flat.elements();
    els.iter().enumerate().all(|(k, el)| match el {
        PathEl::LineTo(p) => inside(p),
        PathEl::MoveTo(p) => !matches!(els.get(k + 1), Some(PathEl::LineTo(_))) || inside(p),
        _ => true,
    })
}

fn invalid_settings_message(field: SettingsField, value: f64) -> String {
    use SettingsField as F;
    match field {
        F::FlattenTolerance | F::JoinTolerance => {
            let label = if field == F::FlattenTolerance {
                "flatten"
            } else {
                "join"
            };
            format!(
                "The {label} tolerance must be a number from {MIN_TOLERANCE_MM} to {MAX_TOLERANCE_MM} mm (got {value})."
            )
        }
        F::Overcut => {
            format!("The overcut must be a number from 0 to {MAX_OVERCUT_MM} mm (got {value}).")
        }
        F::Passes => format!("The number of passes must be at least 1 (got {value})."),
        F::CuttingArea => format!(
            "The cutting area must have finite coordinates and a positive width and height (got {value})."
        ),
        F::Home => format!("The home position must have finite coordinates (got {value})."),
    }
}

fn format_ids(ids: &[ObjectId]) -> String {
    ids.iter()
        .map(|id| id.0.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Run the full pipeline (after validating the settings): flatten, area check, clean, merge collinear,
/// order, overcut, passes.
pub fn plan(input: &[PlacedPath], s: &PipelineSettings) -> Result<CutPlan, PlanError> {
    validate(s)?;
    let flat: Vec<BezPath> = input
        .iter()
        .map(|p| flatten::flatten(&p.path, s.flatten_tolerance_mm))
        .collect();

    let mut outside: Vec<ObjectId> = input
        .iter()
        .zip(&flat)
        .filter(|(_, f)| !inside_area(f, s.cutting_area))
        .map(|(p, _)| p.source.object)
        .collect();
    if !outside.is_empty() {
        outside.sort();
        outside.dedup();
        return Err(PlanError::OutOfArea { objects: outside });
    }

    let mut toolpaths = Vec::new();
    for (placed, f) in input.iter().zip(&flat) {
        for line in clean::clean(f, s.join_tolerance_mm) {
            let mut points = flatten::merge_collinear(&line.points, s.flatten_tolerance_mm);
            let mut closed = line.closed;
            if closed && points.len() < 4 {
                // The ring collapsed to fewer than 3 distinct vertices. With no
                // length it would plunge the blade without moving: drop it.
                // Otherwise [a, b, a] is a line, cut once as an open path
                // (same as clean does for a two-point ring).
                if clean::length(&points) < ZERO_LEN_MM {
                    continue;
                }
                points.truncate(2);
                closed = false;
            }
            toolpaths.push(Toolpath::new(points, closed, placed.source));
        }
    }
    if toolpaths.is_empty() {
        return Err(PlanError::Empty);
    }

    let mut toolpaths = order::nearest_neighbour(toolpaths, s.home);
    for t in toolpaths.iter_mut().filter(|t| t.closed) {
        t.lap_range = 0..t.points.len();
        t.points = overcut::apply(&t.points, s.overcut_mm);
    }
    let toolpaths = passes::expand(toolpaths, s.passes, s.reverse_alternate_passes);

    let cut_mm = toolpaths.iter().map(|t| clean::length(&t.points)).sum();
    let mut travel_mm = 0.0;
    let mut here = s.home;
    for t in &toolpaths {
        travel_mm += here.distance(t.points[0]);
        here = t.points[t.points.len() - 1];
    }
    Ok(CutPlan {
        toolpaths,
        travel_mm,
        cut_mm,
        home: s.home,
        compensation: AppliedCompensation::Tip,
    })
}
