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
    prepare_export(s, p, origin)?.write()
}

/// An export made and checked, ready to write (see [`prepare_export`]).
pub(crate) struct PreparedExport {
    path: PathBuf,
    target: PathBuf,
    bytes: Vec<u8>,
    out: Value,
}

/// Everything of `document.export` but writing the file: checks the
/// scope and the target and makes the bytes. An agent's export runs it
/// off the session lock and writes only while the document is unchanged
/// (`crate::unlocked`).
pub(crate) fn prepare_export(
    s: &mut Session,
    p: ExportParams,
    origin: Origin,
) -> Result<PreparedExport, CommandError> {
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
    let out = json!({
        "path": p.path,
        "format": p.format.extension(),
        "objects": doc.objects.len(),
        "shapes": doc.shape_count(),
        "width_mm": doc.width_mm,
        "height_mm": doc.height_mm,
    });
    Ok(PreparedExport {
        path: p.path,
        target,
        bytes,
        out,
    })
}

impl PreparedExport {
    /// Writes the file crash-safely to the checked target and returns
    /// what `document.export` returns.
    pub(crate) fn write(self) -> Result<Value, CommandError> {
        fsio::write_atomic_at(&self.target, &self.bytes).map_err(|e| {
            CommandError::Failed(format!("cannot write {}: {e}", self.path.display()))
        })?;
        Ok(self.out)
    }
}

command!(
    DocumentExport,
    id: "document.export",
    title: "Export cut paths",
    mutates: false,
    params: ExportParams,
    run: run_export,
    description: "Writes what will be cut as an SVG, DXF or PDF file: the cut shapes of the objects as placed on the mat (after tracing, scale, rotation and mirror), as closed and open polylines in mm, flattened with the plotter's flatten tolerance. The weed box and weed lines of the job settings (job.set_weeding) are cut, so they are exported too, made for the exported objects (SVG group id \"weed\", DXF layer WEED). Overcut, passes, knife compensation and travel are not included. Coordinates: the mat grid's 0,0 is 0,0 and the page is the size of the grid. SVG keeps y down (width and height in mm, one group per object); DXF is ASCII R12 in mm with one layer per object and y up (flipped, so the layout reads the right way round); PDF is one page at 1:1. Hidden objects are never exported; skipped objects only with include_skipped. This is not a project save: the document keeps its path and unsaved state. Agents may not write inside the FlatCraft config directory. Returns {path, format, objects, shapes (weed shapes included), width_mm, height_mm}.",
);
