//! The baseline cut pipeline: types and the `plan` entry point.

use std::ops::Range;

use flatcraft_core::document::ObjectId;
use flatcraft_core::document::WeedSettings;
use flatcraft_core::limits::{
    MAX_OVERCUT_MM, MAX_TOLERANCE_MM, MAX_WEED_MARGIN_MM, MAX_WEED_SPACING_MM, MIN_TOLERANCE_MM,
    MIN_WEED_MARGIN_MM, MIN_WEED_SPACING_MM,
};
use kurbo::{BezPath, PathEl, Point, Rect, Vec2};

use flatcraft_core::geometry::RingDirection;

use crate::containment::{self, KeepMode, KeptMaterial, Regions};
use crate::knife::{self, Compensation, CornerMode};
use crate::strategy::StrategySettings;
use crate::{clean, flatten, order, passes, weed};

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
    /// Cut the passes of a closed ring as chained laps without lifting,
    /// with the overcut once after the last lap ([`crate::passes::chain`]).
    /// Off, they are separate lifted laps, each with its own overcut
    /// ([`crate::passes::separate`]). Reverse alternate passes never chain.
    pub chain_closed_passes: bool,
    /// Where the blade starts: the device home (the media corner the
    /// plotter homes to), in app millimetres. It lies outside the grid when
    /// the grid does not start at the media corner, for example at minus
    /// the grid origin.
    pub home: Point,
    /// The direction every ring is cut in.
    pub direction: RingDirection,
    /// Order the cuts in feed bands ([`crate::feed`]): the head never goes
    /// back more than a band, for media without a mat.
    pub monotonic_feed: bool,
    /// The blade heading at the start of the order. The order sets it per
    /// feed band; leave it at [`ASSUMED_START_HEADING`].
    pub start_heading: Vec2,
    /// What the cut order optimises for.
    pub strategy: StrategySettings,
    /// Paths shorter than this are dropped and counted in
    /// [`CutPlan::dropped_short`]. The engine uses half a plotter step.
    pub min_cut_mm: f64,
    /// Where the knife is steered: the tip path as it is ([`Compensation::Tip`],
    /// the plotter's firmware compensates) or host-compensated axis runs.
    pub compensation: Compensation,
    /// What host compensation does at a corner ([`knife::apply`]).
    pub corner_mode: CornerMode,
    /// A stroke cut first to turn the blade to a known heading. M2 never
    /// sets it from the engine.
    pub alignment_stroke: Option<AlignmentStroke>,
    /// The weed box and weed lines cut after the design ([`crate::weed`]).
    pub weed: WeedSettings,
    /// What each object counts as for the weed box per object: the
    /// top-level group of a grouped object, so a group gets one box.
    /// Objects not listed count on their own.
    pub weed_units: std::collections::BTreeMap<ObjectId, ObjectId>,
    /// The keep mode of each object ([`KeepMode`]). Objects not listed
    /// are [`KeepMode::Pieces`].
    pub keep_modes: std::collections::BTreeMap<ObjectId, KeepMode>,
}

/// The default flatten and join tolerance, in millimetres.
pub const DEFAULT_TOLERANCE_MM: f64 = 0.01;

/// The default overcut of a closed ring, in millimetres.
pub const DEFAULT_OVERCUT_MM: f64 = 0.5;

/// The default shortest cut: half a step of a 20 units per mm plotter.
pub const DEFAULT_MIN_CUT_MM: f64 = 0.025;

impl PipelineSettings {
    /// The keep mode of object `o` ([`PipelineSettings::keep_modes`]).
    pub fn keep_mode(&self, o: ObjectId) -> KeepMode {
        self.keep_modes.get(&o).copied().unwrap_or_default()
    }

    /// The spec defaults for `cutting_area` with travel from `home`: 0.01 mm
    /// tolerances, 0.5 mm overcut, one pass, chained closed passes,
    /// counter-clockwise rings, no feed bands, the Quality strategy, a
    /// 0.025 mm shortest cut, tip compensation, arcs at corners, no
    /// alignment stroke and no weeding.
    pub fn with_defaults(cutting_area: Rect, home: Point) -> Self {
        Self {
            cutting_area,
            flatten_tolerance_mm: DEFAULT_TOLERANCE_MM,
            join_tolerance_mm: DEFAULT_TOLERANCE_MM,
            overcut_mm: DEFAULT_OVERCUT_MM,
            passes: 1,
            reverse_alternate_passes: false,
            chain_closed_passes: true,
            home,
            direction: RingDirection::Ccw,
            monotonic_feed: false,
            start_heading: ASSUMED_START_HEADING,
            strategy: StrategySettings::default(),
            min_cut_mm: DEFAULT_MIN_CUT_MM,
            compensation: Compensation::Tip,
            corner_mode: CornerMode::Arc,
            alignment_stroke: None,
            weed: WeedSettings::default(),
            weed_units: Default::default(),
            keep_modes: Default::default(),
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
    /// A weed line or weed box ([`crate::weed`]): a cut through the waste
    /// only, after the design, so the waste peels off easily. It adds no
    /// kept material and the simulator measures it as waste.
    Weed,
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
    /// Total blade-down length of the axis runs, including overcut,
    /// repeated passes and the waste moves of host compensation.
    pub cut_mm: f64,
    /// Where the first travel move starts (`PipelineSettings::home`).
    pub home: Point,
    /// Where the head (the axis) is after the last toolpath: the last point
    /// of its last axis run. A later plan that goes on from here uses it as
    /// its `home`.
    pub end: Point,
    /// The blade heading after the last toolpath: the direction of its last
    /// move. A later plan that goes on from here uses it as its
    /// `start_heading`.
    pub end_heading: Vec2,
    pub compensation: AppliedCompensation,
    /// Paths left out because they are shorter than
    /// `PipelineSettings::min_cut_mm`.
    pub dropped_short: usize,
    /// Weed boxes left out because they do not fit inside the cutting
    /// area with the weed clearance to the design.
    pub dropped_weed_boxes: usize,
    /// Weed boxes left out because their outline would cut kept material.
    pub blocked_weed_boxes: usize,
    /// The job's kept material, as the knife stage judged every waste move
    /// against it: one source of truth for the simulator and the canvas.
    pub kept: std::sync::Arc<KeptMaterial>,
    /// What rounding the host runs to the device units did
    /// ([`crate::quantize::round_runs`]); the default when they are not
    /// rounded.
    pub rounding: crate::quantize::RoundReport,
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
    /// A host compensation or corner mode value.
    Knife,
    /// The alignment stroke.
    AlignmentStroke,
    /// The weed box margin.
    WeedMargin,
    /// The weed line spacing.
    WeedSpacing,
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
    if let Some(v) = knife::invalid_value(s) {
        return bad(F::Knife, v);
    }
    if let Some(a) = s.alignment_stroke {
        for v in [a.start.x, a.start.y, a.direction.x, a.direction.y] {
            if !v.is_finite() {
                return bad(F::AlignmentStroke, v);
            }
        }
        if a.direction.hypot() == 0.0 {
            return bad(F::AlignmentStroke, 0.0);
        }
        if !(a.length_mm.is_finite() && a.length_mm > 0.0) {
            return bad(F::AlignmentStroke, a.length_mm);
        }
    }
    let w = s.weed;
    if !(MIN_WEED_MARGIN_MM..=MAX_WEED_MARGIN_MM).contains(&w.margin_mm) {
        return bad(F::WeedMargin, w.margin_mm);
    }
    if !(MIN_WEED_SPACING_MM..=MAX_WEED_SPACING_MM).contains(&w.spacing_mm) {
        return bad(F::WeedSpacing, w.spacing_mm);
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
        F::Knife => format!(
            "The knife settings must be finite numbers: offsets, radii and overshoot of at least 0 mm, angles from 0 to 180 degrees and a step above 0 (got {value})."
        ),
        F::AlignmentStroke => format!(
            "The alignment stroke must have a finite start, a direction and a length above 0 mm (got {value})."
        ),
        F::WeedMargin => format!(
            "The weed box margin must be a number from {MIN_WEED_MARGIN_MM} to {MAX_WEED_MARGIN_MM} mm (got {value})."
        ),
        F::WeedSpacing => format!(
            "The weed line spacing must be a number from {MIN_WEED_SPACING_MM} to {MAX_WEED_SPACING_MM} mm (got {value})."
        ),
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

/// The polylines of one placed path as the cut sees them, before the
/// order, the overcut, the passes and the knife: curves flattened with
/// `flatten_tolerance_mm`, ends joined within `join_tolerance_mm`, nearly
/// collinear points merged. A ring that collapsed to a line becomes an open
/// line; one without length is left out. [`plan`] cuts exactly these
/// (minus the ones shorter than its shortest cut), so an export of "what
/// will be cut" reads them here.
pub fn shape_lines(
    path: &BezPath,
    flatten_tolerance_mm: f64,
    join_tolerance_mm: f64,
) -> Vec<clean::Polyline> {
    let flat = flatten::flatten(path, flatten_tolerance_mm);
    lines_of_flat(&flat, flatten_tolerance_mm, join_tolerance_mm).0
}

/// [`shape_lines`] of an already flattened path, and how many rings
/// without length were left out.
fn lines_of_flat(
    flat: &BezPath,
    flatten_tolerance_mm: f64,
    join_tolerance_mm: f64,
) -> (Vec<clean::Polyline>, usize) {
    let mut out = Vec::new();
    let mut collapsed = 0;
    for line in clean::clean(flat, join_tolerance_mm) {
        let mut points = flatten::merge_collinear(&line.points, flatten_tolerance_mm);
        let mut closed = line.closed;
        if closed && points.len() < 4 {
            // The ring collapsed to fewer than 3 distinct vertices. With no
            // length it would plunge the blade without moving: drop it.
            // Otherwise [a, b, a] is a line, cut once as an open path
            // (same as clean does for a two-point ring).
            if clean::length(&points) < ZERO_LEN_MM {
                collapsed += 1;
                continue;
            }
            points.truncate(2);
            closed = false;
        }
        out.push(clean::Polyline { points, closed });
    }
    (out, collapsed)
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
/// 8. weeding: the weed lines, then the weed box, after the design
///    ([`weed::generate`], [`weed::sequence`])
/// 9. passes and overcut: chained laps on closed rings, the overcut after
///    the last lap ([`passes::chain`])
/// 10. knife: the alignment stroke and host compensation ([`knife::apply`]),
///     with the job's kept material built from the first lap of each
///     ordered `Cut` ring (Ruling F3)
pub fn plan(input: &[PlacedPath], s: &PipelineSettings) -> Result<CutPlan, PlanError> {
    plan_with_regions(input, s, &Regions::new())
}

/// [`plan`], with kept-material outlines that stand for the rings of the
/// objects they list (see [`KeptMaterial::from_toolpaths_and_regions`]):
/// the kept material, the weeds and host compensation read them; they are
/// never cut.
pub fn plan_with_regions(
    input: &[PlacedPath],
    s: &PipelineSettings,
    regions: &Regions,
) -> Result<CutPlan, PlanError> {
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
        let (lines, collapsed) = lines_of_flat(f, s.flatten_tolerance_mm, s.join_tolerance_mm);
        // A ring without length counts as too short only when there is a
        // shortest cut to be shorter than.
        if s.min_cut_mm > 0.0 {
            dropped_short += collapsed;
        }
        for line in lines {
            // Shorter than half a plotter step, the cut is no cut at all:
            // leave it out, but count it so the engine can say so.
            if clean::length(&line.points) < s.min_cut_mm {
                dropped_short += 1;
                continue;
            }
            toolpaths.push(Toolpath::new(line.points, line.closed, placed.source));
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
    // tree. One closure gives the keep mode of each object to the keep
    // sides, the weeds and the kept material.
    let mode_of = |o: ObjectId| s.keep_mode(o);
    let tree = containment::build(&toolpaths);
    for (i, t) in toolpaths.iter_mut().enumerate() {
        t.keep = containment::keep_side(&tree, i, t.closed, mode_of(t.source.object));
    }
    let toolpaths = order::order(toolpaths, &tree, s.home, s);
    // Weeds read the design as one lap each, before passes and overcut.
    let unit_of = |o: ObjectId| s.weed_units.get(&o).copied().unwrap_or(o);
    let weeds = weed::generate_with_regions(
        &toolpaths,
        regions,
        mode_of,
        unit_of,
        s.cutting_area,
        &s.weed,
    );
    let toolpaths = weed::sequence(toolpaths, &weeds, s);
    let mut toolpaths = if s.chain_closed_passes {
        passes::chain(
            toolpaths,
            s.passes,
            s.reverse_alternate_passes,
            s.overcut_mm,
        )
    } else {
        passes::separate(
            toolpaths,
            s.passes,
            s.reverse_alternate_passes,
            s.overcut_mm,
        )
    };

    // Ruling F3: `from_toolpaths` reads `lap()` of pass 1, never `points`,
    // so chained laps and passes never cancel a ring.
    let kept =
        KeptMaterial::from_toolpaths_and_regions(&toolpaths, regions, mode_of, s.cutting_area);
    let (compensation, rounding) = knife::apply_rounded(&mut toolpaths, s, &kept, s.start_heading);
    let (end, end_heading) = knife::end_state(&toolpaths, s.home, s.start_heading);

    // Blade-down axis length: the points themselves without host
    // compensation.
    let cut_mm = toolpaths
        .iter()
        .flat_map(Toolpath::runs)
        .map(|r| clean::length(&r.points))
        .sum();
    let travel_mm = travel_mm(&toolpaths, s.home);
    Ok(CutPlan {
        toolpaths,
        travel_mm,
        cut_mm,
        home: s.home,
        end,
        end_heading,
        compensation,
        dropped_short,
        dropped_weed_boxes: weeds.dropped_boxes,
        blocked_weed_boxes: weeds.blocked_boxes,
        kept: std::sync::Arc::new(kept),
        rounding,
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
