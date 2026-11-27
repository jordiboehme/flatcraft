//! Export of what will be cut: the cut geometry of the objects as placed
//! on the mat, as closed and open polylines in millimetres, written as SVG,
//! DXF or PDF.
//!
//! The geometry is the cut's own (one source of truth): the objects'
//! cut pieces placed by [`crate::job::place_pieces`] (tracing, scale,
//! rotation, mirror and the cut options applied), then flattened, joined
//! and cleaned by [`flatcraft_path::shape_lines`] with the plotter's
//! flatten tolerance, exactly as the cut plan reads them. The weed box and
//! weed lines of the job settings are cut too, so they are exported as
//! well: [`flatcraft_path::weed::generate`] on the exported shapes, the
//! same function and the same geometry the plan cuts. Not included:
//! overcut, passes, lead-ins, swivels, knife compensation and travel.
//!
//! Coordinates: the mat's grid origin is 0,0 and y points down (the app's
//! own frame). The page is the size of the mat's grid. Each writer maps
//! this frame onto its format: SVG keeps it, DXF and PDF flip y so the
//! layout shows the right way round.

pub mod dxf;
pub mod pdf;
pub mod svg;

use flatcraft_core::document::ObjectId;
use kurbo::Point;
use serde::Deserialize;

use crate::job::{JobInput, JobObject};
use crate::session::CommandError;

/// A file format of the export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// Scalable Vector Graphics, for vector editors and the web.
    Svg,
    /// AutoCAD R12 ASCII drawing, for CAD, laser and cutter software.
    Dxf,
    /// One PDF page at 1:1, for printing and viewing.
    Pdf,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 3] = [ExportFormat::Svg, ExportFormat::Dxf, ExportFormat::Pdf];

    /// The file name extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Svg => "svg",
            ExportFormat::Dxf => "dxf",
            ExportFormat::Pdf => "pdf",
        }
    }

    /// The short name people know it by.
    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Svg => "SVG",
            ExportFormat::Dxf => "DXF",
            ExportFormat::Pdf => "PDF",
        }
    }
}

/// Which objects an export takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportScope {
    /// The selected objects.
    Selected,
    /// Every object of the document.
    All,
}

impl ExportScope {
    /// The scope when none is given: the selection when there is one.
    pub fn default_for(has_selection: bool) -> Self {
        if has_selection {
            ExportScope::Selected
        } else {
            ExportScope::All
        }
    }
}

/// One cut shape: a polyline in mat millimetres. A closed shape does not
/// repeat its first point at the end.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportShape {
    pub points: Vec<Point>,
    pub closed: bool,
}

/// The shapes of one object.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportObject {
    pub id: ObjectId,
    pub name: String,
    pub shapes: Vec<ExportShape>,
}

/// Everything an export writes: the page (the mat's grid), the objects
/// with their shapes, in document order, and the weed box and weed lines
/// around the exported objects.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportDoc {
    pub width_mm: f64,
    pub height_mm: f64,
    pub objects: Vec<ExportObject>,
    /// The weed lines, then the weed boxes (see [`flatcraft_path::weed`]).
    pub weeds: Vec<ExportShape>,
}

impl ExportDoc {
    /// How many shapes the export holds, weeds included.
    pub fn shape_count(&self) -> usize {
        self.objects.iter().map(|o| o.shapes.len()).sum::<usize>() + self.weeds.len()
    }
}

/// The name of the weed group (SVG) and layer (DXF).
pub const WEED_NAME: &str = "Weed box and weed lines";

/// The weed shapes for the exported `design` (one lap per toolpath, as
/// the plan reads them), with the job's weed settings.
fn weed_shapes(
    design: &[flatcraft_path::Toolpath],
    input: &JobInput,
    area: kurbo::Rect,
) -> Vec<ExportShape> {
    // The keep mode of each object, as in the plan.
    let units = input.weed_units();
    let keep = input.keep_modes();
    let weeds = flatcraft_path::weed::generate(
        design,
        |o| keep.get(&o).copied().unwrap_or_default(),
        |o| units.get(&o).copied().unwrap_or(o),
        area,
        &input.job.weed,
    );
    let lines = weeds.lines.iter().map(|l| ExportShape {
        points: l.to_vec(),
        closed: false,
    });
    let boxes = weeds.boxes.iter().map(|r| ExportShape {
        points: vec![
            Point::new(r.x0, r.y0),
            Point::new(r.x1, r.y0),
            Point::new(r.x1, r.y1),
            Point::new(r.x0, r.y1),
        ],
        closed: true,
    });
    lines.chain(boxes).collect()
}

/// Whether an object is exported: hidden objects never are (as in the
/// cut), skipped ones only with `include_skipped`.
pub fn is_exported(visible: bool, cut_enabled: bool, include_skipped: bool) -> bool {
    visible && (cut_enabled || include_skipped)
}

/// The export of `input`: the objects in `ids` (all when `None`) that are
/// exported (see [`is_exported`]), with their cut shapes. Refused with the
/// reason when nothing is left to export.
pub fn collect(
    input: &JobInput,
    ids: Option<&[ObjectId]>,
    include_skipped: bool,
) -> Result<ExportDoc, CommandError> {
    let (media, _) = crate::commands::effective_media(&input.plotter, &input.job)?;
    let tolerance = input.plotter.flatten_tolerance_mm;
    // The cut leaves out what is shorter than half a plotter step.
    let min_cut = 0.5 / input.plotter.units_per_mm;
    let cut = input.cut_geometry();
    // A welded group's pieces belong to its backmost object: it is
    // exported when any member is.
    let wanted = |o: &&JobObject| {
        ids.is_none_or(|ids| {
            ids.contains(&o.id)
                || cut
                    .merged_into
                    .iter()
                    .any(|(member, back)| *back == o.id && ids.contains(member))
        })
    };
    let mut objects = Vec::new();
    let mut design = Vec::new();
    let (mut hidden, mut skipped) = (0, 0);
    for o in input.objects.iter().filter(wanted) {
        if !o.visible {
            hidden += 1;
            continue;
        }
        if !is_exported(o.visible, o.cut_enabled, include_skipped) {
            skipped += 1;
            continue;
        }
        let mut placed = Vec::new();
        crate::job::place_pieces(o.id, cut.pieces(o.id), &mut placed);
        let shapes: Vec<ExportShape> = placed
            .iter()
            .flat_map(|p| {
                flatcraft_path::shape_lines(&p.path, tolerance, crate::job::JOIN_TOLERANCE_MM)
            })
            .filter(|l| flatcraft_path::clean::length(&l.points) >= min_cut)
            .map(|l| {
                design.push(flatcraft_path::Toolpath::new(
                    l.points.clone(),
                    l.closed,
                    flatcraft_path::PathRef {
                        object: o.id,
                        path_index: 0,
                    },
                ));
                let mut points = l.points;
                if l.closed {
                    points.pop();
                }
                ExportShape {
                    points,
                    closed: l.closed,
                }
            })
            .collect();
        if !shapes.is_empty() {
            objects.push(ExportObject {
                id: o.id,
                name: o.name.clone(),
                shapes,
            });
        }
    }
    if objects.is_empty() {
        let mut text = String::from("Nothing to export: no object has cut paths.");
        if hidden > 0 {
            text.push_str(&format!(
                " {} hidden; hidden objects are never exported, show them first.",
                count(hidden)
            ));
        }
        if skipped > 0 {
            text.push_str(&format!(
                " {} skipped for cutting; tick \"Include objects that are not cut\" (include_skipped) to export them.",
                count(skipped)
            ));
        }
        return Err(CommandError::Refused(text));
    }
    let area = input
        .plotter
        .cutting_area(&media.id)
        .ok_or_else(|| CommandError::Failed("the mat has no cutting area".into()))?;
    Ok(ExportDoc {
        width_mm: media.grid.width_mm,
        height_mm: media.grid.height_mm,
        objects,
        weeds: weed_shapes(&design, input, area),
    })
}

fn count(n: usize) -> String {
    if n == 1 {
        "1 object is".to_string()
    } else {
        format!("{n} objects are")
    }
}

/// The bytes of `doc` in `format`.
pub fn write(doc: &ExportDoc, format: ExportFormat) -> Vec<u8> {
    match format {
        ExportFormat::Svg => svg::write(doc).into_bytes(),
        ExportFormat::Dxf => dxf::write(doc).into_bytes(),
        ExportFormat::Pdf => pdf::write(doc),
    }
}

/// A number for a text format: at most four decimals (0.1 micrometre),
/// no trailing zeros, never `-0`.
pub fn num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short_and_never_minus_zero() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(12.5), "12.5");
        assert_eq!(num(0.123456), "0.1235");
        assert_eq!(num(-0.00001), "0");
        assert_eq!(num(-2.25), "-2.25");
        assert_eq!(num(300.0), "300");
    }

    #[test]
    fn hidden_never_and_skipped_only_when_asked() {
        assert!(is_exported(true, true, false));
        assert!(!is_exported(true, false, false));
        assert!(is_exported(true, false, true));
        assert!(!is_exported(false, true, true));
        assert!(!is_exported(false, false, true));
    }

    #[test]
    fn extensions_match_the_formats() {
        let exts: Vec<&str> = ExportFormat::ALL.iter().map(|f| f.extension()).collect();
        assert_eq!(exts, ["svg", "dxf", "pdf"]);
    }
}
