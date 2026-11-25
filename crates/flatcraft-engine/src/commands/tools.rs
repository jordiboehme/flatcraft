//! Tools: the test pattern sheet and document previews.

use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use flatcraft_core::document::ObjectId;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::object::add_svg;
use super::project::inside;
use super::{command, cutting_area, object_mut};
use crate::patterns::{SHEET_MM, SHEET_NAME, sheet_svg};
use crate::preview::{MAX_PX, MIN_PX, PreviewKind, render_png};
use crate::session::{CommandError, Origin, Session};

// ---------------------------------------------------------------- tools.test_patterns

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestPatternsParams {
    /// Add the sheet to the active document as the object "Test patterns" (default true).
    #[serde(default = "default_true")]
    pub add_to_document: bool,
    /// Also write the sheet as an SVG file at this path.
    pub svg_path: Option<PathBuf>,
}

/// Writes `text` to `path` crash-safely (temp file, sync, rename).
fn write_atomic(path: &Path, text: &str) -> Result<(), CommandError> {
    if path.file_name().is_none() {
        return Err(CommandError::InvalidParams(
            "svg_path has no file name".into(),
        ));
    }
    flatcraft_device::fsio::write_atomic(path, text.as_bytes())
        .map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", path.display())))
}

fn run_test_patterns(
    s: &mut Session,
    p: TestPatternsParams,
    origin: Origin,
) -> Result<Value, CommandError> {
    if !p.add_to_document && p.svg_path.is_none() {
        return Err(CommandError::InvalidParams(
            "nothing to do: add_to_document is false and no svg_path is given".into(),
        ));
    }
    if let Some(path) = &p.svg_path
        && origin == Origin::Agent
        && inside(path, &s.config_dir)
    {
        return Err(CommandError::Refused(format!(
            "agents may not write into the FlatCraft config directory ({})",
            s.config_dir.display()
        )));
    }
    let svg = sheet_svg();
    let mut out = Map::new();
    if let Some(path) = &p.svg_path {
        write_atomic(path, &svg)?;
        out.insert("svg_path".into(), json!(path));
    }
    if p.add_to_document {
        let (id, _) = add_svg(s, SHEET_NAME.to_string(), svg.clone().into_bytes())?;
        centre_sheet(s, id)?;
        out.insert("id".into(), id.0.into());
    }
    Ok(Value::Object(out))
}

/// Puts the middle of the sheet page (not of its geometry) on the middle of
/// the cutting area.
fn centre_sheet(s: &mut Session, id: ObjectId) -> Result<(), CommandError> {
    let area = cutting_area(s)?;
    let o = object_mut(&mut s.doc_mut().document, id)?;
    let page_centre = kurbo::Point::new(SHEET_MM / 2.0, SHEET_MM / 2.0);
    o.transform.center_mm = area.center() + (o.local_bounds.center() - page_centre);
    Ok(())
}

command!(
    ToolsTestPatterns,
    id: "tools.test_patterns",
    title: "Test patterns",
    mutates: true,
    params: TestPatternsParams,
    run: run_test_patterns,
    description: "Makes the FlatCraft test sheet, 280 x 280 mm, to cut first on a real machine. It holds a 50 mm square, eight V tips (15 to 170 degrees, legs 20 mm), circles of radius 0.5 to 25 mm, a 30 mm square with a 10 mm hole, a 40 mm line, an S curve, an asymmetric F (stem on the left, arms pointing right, 20 mm tall) and 100 mm rulers along x and y with 10 mm ticks. With add_to_document (default true) it is added as the object 'Test patterns', centred in the cutting area. With svg_path it is also written as an SVG file (agents may not write inside the FlatCraft config directory). Returns {id (when added), svg_path (when written)}.",
);

// ---------------------------------------------------------------- document.preview

fn default_max_px() -> u32 {
    1024
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewParams {
    /// "canvas": media, cutting area and objects in their colours. "path": toolpaths in cut order, dashed travel moves and start dots.
    pub kind: PreviewKind,
    /// Size in px of the larger image side, 64 to 4096 (default 1024).
    #[serde(default = "default_max_px")]
    pub max_px: u32,
}

fn run_preview(s: &mut Session, p: PreviewParams, _: Origin) -> Result<Value, CommandError> {
    if !(MIN_PX..=MAX_PX).contains(&p.max_px) {
        return Err(CommandError::InvalidParams(format!(
            "max_px must be between {MIN_PX} and {MAX_PX}, not {}",
            p.max_px
        )));
    }
    let png = render_png(s, p.kind, p.max_px)?;
    let (width, height) = png_size(&png);
    Ok(json!({
        "png_base64": STANDARD.encode(&png),
        "width": width,
        "height": height,
    }))
}

/// Width and height from the IHDR chunk of a PNG we just wrote.
fn png_size(png: &[u8]) -> (u32, u32) {
    let be = |at: usize| u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]);
    (be(16), be(20))
}

command!(
    DocumentPreview,
    id: "document.preview",
    title: "Preview image",
    mutates: false,
    params: PreviewParams,
    run: run_preview,
    description: "Draws the active document as a PNG. kind 'canvas' shows the media, the cutting area and the visible objects in their colours; kind 'path' shows the planned toolpaths numbered in cut order, travel moves dashed and a red dot at every start point (it needs something to cut and plans like job.preview). max_px is the larger image side, 64 to 4096, default 1024. Returns {png_base64, width, height}.",
);
