//! The baseline cut pipeline: types and the `plan` entry point.

use std::ops::Range;

use flatcraft_core::document::ObjectId;
use flatcraft_core::limits::{MAX_OVERCUT_MM, MAX_TOLERANCE_MM, MIN_TOLERANCE_MM};
use kurbo::{BezPath, PathEl, Point, Rect, Vec2};

use flatcraft_core::geometry::RingDirection;

use crate::containment::{self, KeepMode};
use crate::strategy::StrategySettings;
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
    /// The direction every ring is cut in.
    pub direction: RingDirection,
    /// Order the cuts in feed bands (Task 7). Off until then.
    pub monotonic_feed: bool,
    /// What the cut order optimises for.
    pub strategy: StrategySettings,
    /// Paths shorter than this are dropped and counted in
    /// [`CutPlan::dropped_short`]. The engine uses half a device step.
    pub min_cut_mm: f64,
}

/// The default flatten and join tolerance, in millimetres.
pub const DEFAULT_TOLERANCE_MM: f64 = 0.01;

/// The default overcut of a closed ring, in millimetres.
pub const DEFAULT_OVERCUT_MM: f64 = 0.5;

/// The default shortest cut: half a step of a 20 units per mm plotter.
pub const DEFAULT_MIN_CUT_MM: f64 = 0.025;

impl PipelineSettings {
    /// The spec defaults for `cutting_area` with travel from `home`: 0.01 mm
    /// tolerances, 0.5 mm overcut, one pass, counter-clockwise rings, no
    /// feed bands, the Quality strategy and a 0.025 mm shortest cut.
    pub fn with_defaults(cutting_area: Rect, home: Point) -> Self {
        Self {
            cutting_area,
            flatten_tolerance_mm: DEFAULT_TOLERANCE_MM,
            join_tolerance_mm: DEFAULT_TOLERANCE_MM,
            overcut_mm: DEFAULT_OVERCUT_MM,
            passes: 1,
            reverse_alternate_passes: false,
            home,
            direction: RingDirection::Ccw,
            monotonic_feed: false,
            strategy: StrategySettings::default(),
            min_cut_mm: DEFAULT_MIN_CUT_MM,
        }
    }
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
///
/// Plan decision: 30 degrees. The flattening vertices of an r = 0.5 mm circle
/// at 0.01 mm tolerance turn about 23 degrees, and those cannot hide a start
/// mark, so they must not count as corners.
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
                let seg = (0..self.points.len())
                    .map(|j| if j == 0 { WASTE_SEG } else { j as u32 - 1 })
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
    /// Paths left out because they are shorter than
    /// `PipelineSettings::min_cut_mm`.
    pub dropped_short: usize,
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
    MinCut,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PlanError {
    #[error("there is nothing to cut")]
    Empty,
    /// Every path was shorter than `min_cut_mm`, so nothing is left to cut.
    #[error("{}", too_short_message(*.dropped, *.min_cut_mm))]
    TooShort { dropped: usize, min_cut_mm: f64 },
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
    if !(s.min_cut_mm.is_finite() && s.min_cut_mm >= 0.0) {
        return bad(F::MinCut, s.min_cut_mm);
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
        F::MinCut => format!("The shortest cut must be a number of at least 0 mm (got {value})."),
    }
}

fn too_short_message(dropped: usize, min_cut_mm: f64) -> String {
    let what = if dropped == 1 {
        "the only cut path is".to_string()
    } else {
        format!("all {dropped} cut paths are")
    };
    format!("there is nothing to cut: {what} shorter than {min_cut_mm} mm")
}

fn format_ids(ids: &[ObjectId]) -> String {
    ids.iter()
        .map(|id| id.0.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Run the full pipeline after validating the settings:
///
/// 1. flatten
/// 2. area check (a `MoveTo` no drawing follows is ignored)
/// 3. clean
/// 4. merge collinear
/// 5. drop paths shorter than `min_cut_mm`, counted in `dropped_short`
/// 6. containment and keep sides
/// 7. order
/// 8. overcut and passes
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
    let mut dropped_short = 0;
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
                    // Counted as too short only when there is a shortest
                    // cut to be shorter than.
                    if s.min_cut_mm > 0.0 {
                        dropped_short += 1;
                    }
                    continue;
                }
                points.truncate(2);
                closed = false;
            }
            // Shorter than half a device step, the cut is no cut at all:
            // leave it out, but count it so the engine can say so.
            if clean::length(&points) < s.min_cut_mm {
                dropped_short += 1;
                continue;
            }
            toolpaths.push(Toolpath::new(points, closed, placed.source));
        }
    }
    if toolpaths.is_empty() {
        return Err(if dropped_short > 0 {
            PlanError::TooShort {
                dropped: dropped_short,
                min_cut_mm: s.min_cut_mm,
            }
        } else {
            PlanError::Empty
        });
    }

    // Ruling M2-P13: every toolpath's keep side comes from the containment
    // tree. Every object is Pieces until Task 29 supplies the real modes.
    let mode_of = |_: ObjectId| KeepMode::Pieces;
    let tree = containment::build(&toolpaths);
    for (i, t) in toolpaths.iter_mut().enumerate() {
        t.keep = containment::keep_side(&tree, i, t.closed, mode_of(t.source.object));
    }
    let mut toolpaths = order::order(toolpaths, &tree, s.home, s);
    for t in toolpaths.iter_mut().filter(|t| t.closed) {
        t.lap_range = 0..t.points.len();
        t.points = overcut::apply(&t.points, s.overcut_mm);
    }
    let toolpaths = passes::expand(toolpaths, s.passes, s.reverse_alternate_passes);

    let cut_mm = toolpaths.iter().map(|t| clean::length(&t.points)).sum();
    let travel_mm = travel_mm(&toolpaths, s.home);
    Ok(CutPlan {
        toolpaths,
        travel_mm,
        cut_mm,
        home: s.home,
        compensation: AppliedCompensation::Tip,
        dropped_short,
    })
}

/// The blade-up distance from `home` through the axis runs of `toolpaths`:
/// from the end of each run to the start of the next.
fn travel_mm(toolpaths: &[Toolpath], home: Point) -> f64 {
    let mut travel = 0.0;
    let mut here = home;
    let mut visit = |points: &[Point]| {
        if let (Some(&first), Some(&last)) = (points.first(), points.last()) {
            travel += here.distance(first);
            here = last;
        }
    };
    for t in toolpaths {
        match &t.axis {
            Some(runs) => runs.iter().for_each(|r| visit(&r.points)),
            None => visit(&t.points),
        }
    }
    travel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn travel_runs_between_axis_runs() {
        let source = PathRef {
            object: ObjectId(1),
            path_index: 0,
        };
        let p = |x: f64, y: f64| Point::new(x, y);
        let plain = Toolpath::new(vec![p(0.0, 0.0), p(10.0, 0.0)], false, source);
        let mut split = Toolpath::new(vec![p(20.0, 0.0), p(30.0, 0.0)], false, source);
        // Host compensation lifts once inside the toolpath: 2 mm of travel
        // between its runs, and the runs start and end off `points`.
        split.axis = Some(vec![
            AxisRun {
                points: vec![p(19.0, 0.0), p(24.0, 0.0)],
                seg: vec![WASTE_SEG, 0],
            },
            AxisRun {
                points: vec![p(26.0, 0.0), p(31.0, 0.0)],
                seg: vec![WASTE_SEG, 0],
            },
        ]);
        let travel = travel_mm(&[plain, split], p(0.0, -5.0));
        assert!((travel - (5.0 + 9.0 + 2.0)).abs() < 1e-12, "{travel}");
    }
}
