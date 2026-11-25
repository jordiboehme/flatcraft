//! The baseline cut pipeline: types and the `plan` entry point.

use flatcraft_core::document::ObjectId;
use kurbo::{BezPath, PathEl, Point, Rect};

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

/// The blade is down along `points`; a travel move lies between consecutive toolpaths.
#[derive(Debug, Clone, PartialEq)]
pub struct Toolpath {
    pub points: Vec<Point>,
    pub closed: bool,
    pub source: PathRef,
    /// 1-based pass number.
    pub pass: u32,
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

use flatcraft_core::limits::{MAX_OVERCUT_MM, MAX_TOLERANCE_MM, MIN_TOLERANCE_MM};

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
            toolpaths.push(Toolpath {
                points,
                closed,
                source: placed.source,
                pass: 1,
            });
        }
    }
    if toolpaths.is_empty() {
        return Err(PlanError::Empty);
    }

    let mut toolpaths = order::nearest_neighbour(toolpaths, s.home);
    for t in toolpaths.iter_mut().filter(|t| t.closed) {
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
    })
}
