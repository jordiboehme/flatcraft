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
    /// Sum of travel distances, starting from the origin.
    pub travel_mm: f64,
    /// Total blade-down length, including overcut and repeated passes.
    pub cut_mm: f64,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PlanError {
    #[error("there is nothing to cut")]
    Empty,
    #[error("{}", invalid_settings_message(.field, *.value))]
    InvalidSettings { field: &'static str, value: f64 },
    #[error("{} object(s) reach outside the cutting area: {}", .objects.len(), format_ids(.objects))]
    OutOfArea { objects: Vec<ObjectId> },
}

/// Anything shorter than this has no length.
const ZERO_LEN_MM: f64 = 1e-9;

/// Smallest tolerance accepted; below it kurbo's flattening takes ages.
const MIN_TOLERANCE_MM: f64 = 0.001;

fn validate(s: &PipelineSettings) -> Result<(), PlanError> {
    let bad = |field, value| Err(PlanError::InvalidSettings { field, value });
    for (field, v) in [
        ("flatten_tolerance_mm", s.flatten_tolerance_mm),
        ("join_tolerance_mm", s.join_tolerance_mm),
    ] {
        if !(v.is_finite() && v >= MIN_TOLERANCE_MM) {
            return bad(field, v);
        }
    }
    if !(s.overcut_mm.is_finite() && s.overcut_mm >= 0.0) {
        return bad("overcut_mm", s.overcut_mm);
    }
    if s.passes < 1 {
        return bad("passes", f64::from(s.passes));
    }
    let a = s.cutting_area;
    for v in [a.x0, a.y0, a.x1, a.y1] {
        if !v.is_finite() {
            return bad("cutting_area", v);
        }
    }
    for size in [a.x1 - a.x0, a.y1 - a.y0] {
        if size <= 0.0 {
            return bad("cutting_area", size);
        }
    }
    Ok(())
}

/// Every point of the flattened path lies in `area`, with a 1e-6 mm epsilon so
/// an object aligned exactly to the margin passes.
fn inside_area(flat: &BezPath, area: Rect) -> bool {
    const EPS: f64 = 1e-6;
    flat.elements().iter().all(|el| match el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => {
            (area.x0 - EPS..=area.x1 + EPS).contains(&p.x)
                && (area.y0 - EPS..=area.y1 + EPS).contains(&p.y)
        }
        _ => true,
    })
}

fn invalid_settings_message(field: &str, value: f64) -> String {
    match field {
        "flatten_tolerance_mm" | "join_tolerance_mm" => {
            let label = field.trim_end_matches("_tolerance_mm");
            format!(
                "The {label} tolerance must be a number of at least {MIN_TOLERANCE_MM} mm (got {value})."
            )
        }
        "overcut_mm" => {
            format!("The overcut must be a number of at least 0 mm (got {value}).")
        }
        "passes" => format!("The number of passes must be at least 1 (got {value})."),
        "cutting_area" => format!(
            "The cutting area must have finite coordinates and a positive width and height (got {value})."
        ),
        other => format!("The setting {other} is not valid (got {value})."),
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

    let mut toolpaths = order::nearest_neighbour(toolpaths, Point::ZERO);
    for t in toolpaths.iter_mut().filter(|t| t.closed) {
        t.points = overcut::apply(&t.points, s.overcut_mm);
    }
    let toolpaths = passes::expand(toolpaths, s.passes, s.reverse_alternate_passes);

    let cut_mm = toolpaths.iter().map(|t| clean::length(&t.points)).sum();
    let mut travel_mm = 0.0;
    let mut here = Point::ZERO;
    for t in &toolpaths {
        travel_mm += here.distance(t.points[0]);
        here = t.points[t.points.len() - 1];
    }
    Ok(CutPlan {
        toolpaths,
        travel_mm,
        cut_mm,
    })
}
