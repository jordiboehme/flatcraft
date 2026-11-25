//! `document.export`: writes what will be cut as SVG, DXF or PDF.

use std::path::PathBuf;

use flatcraft_core::document::ObjectId;
use flatcraft_core::fsio;
use serde::Deserialize;
use serde_json::{Value, json};

use super::command;
use super::project::write_target;
use crate::export::{self, ExportFormat, ExportScope};
use crate::job::JobInput;
use crate::session::{CommandError, Origin, Session};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportParams {
    /// File to write. It is written as given; give it the format's extension (.svg, .dxf or .pdf).
    pub path: PathBuf,
    /// File format: "svg", "dxf" or "pdf".
    pub format: ExportFormat,
    /// "selected" (the selected objects) or "all". Default: "selected" when something is selected, else "all".
    pub scope: Option<ExportScope>,
    /// Also export objects that are switched off for cutting (skipped). Hidden objects are never exported. Default false.
    #[serde(default)]
    pub include_skipped: bool,
}

fn run_export(s: &mut Session, p: ExportParams, origin: Origin) -> Result<Value, CommandError> {
    let selection: Vec<ObjectId> = s.doc().selection.iter().copied().collect();
    let scope = p
        .scope
        .unwrap_or_else(|| ExportScope::default_for(!selection.is_empty()));
    let ids = match scope {
        ExportScope::All => None,
        ExportScope::Selected if selection.is_empty() => {
            return Err(CommandError::InvalidParams(
                "scope is \"selected\" but nothing is selected".into(),
            ));
        }
        ExportScope::Selected => Some(selection.as_slice()),
    };
    let input = JobInput::of(s)?;
    let doc = export::collect(&input, ids, p.include_skipped)?;
    let target = write_target(&p.path, &s.config_dir, origin)?;
    let bytes = export::write(&doc, p.format);
    fsio::write_atomic_at(&target, &bytes)
        .map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", p.path.display())))?;
    Ok(json!({
        "path": p.path,
        "format": p.format.extension(),
        "objects": doc.objects.len(),
        "shapes": doc.shape_count(),
        "width_mm": doc.width_mm,
        "height_mm": doc.height_mm,
    }))
}

command!(
    DocumentExport,
    id: "document.export",
    title: "Export cut paths",
    mutates: false,
    params: ExportParams,
    run: run_export,
    description: "Writes what will be cut as an SVG, DXF or PDF file: the cut shapes of the objects as placed on the mat (after tracing, scale, rotation and mirror), as closed and open polylines in mm, flattened with the plotter's flatten tolerance. Overcut, passes, knife compensation and travel are not included. Coordinates: the mat grid's 0,0 is 0,0 and the page is the size of the grid. SVG keeps y down (width and height in mm, one group per object); DXF is ASCII R12 in mm with one layer per object and y up (flipped, so the layout reads the right way round); PDF is one page at 1:1. Hidden objects are never exported; skipped objects only with include_skipped. This is not a project save: the document keeps its path and unsaved state. Agents may not write inside the FlatCraft config directory. Returns {path, format, objects, shapes, width_mm, height_mm}.",
);
