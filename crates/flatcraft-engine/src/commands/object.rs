//! Object commands: import, select, show, delete, duplicate and transform.

use std::io::Read;
use std::path::PathBuf;

use flatcraft_core::document::{ObjectId, Source, TraceSettings};
use flatcraft_core::geometry::CutPath;
use flatcraft_import::svg::import_svg;
use flatcraft_import::trace::{TraceResult, trace};
use flatcraft_import::{ImportError, ImportWarning, fonts};
use kurbo::{Point, Rect, Vec2};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    command, cutting_area, ids_json, object_mut, placed_bounds, placed_bounds_or_center, rect_json,
    resolve_ids, translate,
};
use crate::commands::normalize_deg;
use crate::session::{CommandError, Origin, Session};

/// Largest file `object.import` reads.
const MAX_IMPORT_BYTES: u64 = 50 * 1024 * 1024;

/// Text for an import warning, for people and agents.
pub(crate) fn warning_text(w: &ImportWarning) -> String {
    match w {
        ImportWarning::ClipOrMaskIgnored { count } => {
            format!("{count} clip paths or masks were ignored; the full shapes will be cut")
        }
        ImportWarning::ImagesSkipped { count } => {
            format!("{count} embedded images were skipped; only vector paths are cut")
        }
        ImportWarning::NoPaths => "the file has no visible paths, so nothing will be cut".into(),
    }
}

fn import_error(e: ImportError) -> CommandError {
    match e {
        ImportError::ImageTooLarge { .. } => CommandError::Failed(e.to_string()),
        other => CommandError::Failed(format!("cannot import the file: {other}")),
    }
}

/// Blade offset of the active document's plotter, mm. 0.25 if the plotter
/// cannot be resolved.
pub(crate) fn blade_offset_mm(s: &Session) -> f64 {
    let id = &s.doc().document.job.plotter_id;
    blade_offset_for(s, id)
}

pub(crate) fn blade_offset_for(s: &Session, plotter_id: &str) -> f64 {
    s.profiles
        .plotter(plotter_id)
        .map(|p| p.blade_offset_mm)
        .unwrap_or(0.25)
}

/// Adds an object to the active document and selects it. It keeps its
/// position from the source page if it lies inside the cutting area;
/// otherwise its top-left corner goes to the top-left of the cutting area.
fn add_object(
    s: &mut Session,
    name: String,
    source: Source,
    paths: Vec<CutPath>,
) -> Result<ObjectId, CommandError> {
    let area = cutting_area(s).ok();
    let doc = &mut s.doc_mut().document;
    let id = doc.add(name, source, paths);
    let o = object_mut(doc, id)?;
    o.transform.center_mm = o.local_bounds.center();
    if let (Some(area), Some(b)) = (area, placed_bounds(o)) {
        let inside = b.x0 >= area.x0 && b.y0 >= area.y0 && b.x1 <= area.x1 && b.y1 <= area.y1;
        if !inside {
            translate(o, Point::new(area.x0, area.y0) - Point::new(b.x0, b.y0));
        }
    }
    let st = s.doc_mut();
    st.selection.clear();
    st.selection.insert(id);
    Ok(id)
}

fn bounds_json(s: &Session, id: ObjectId) -> Value {
    s.doc()
        .document
        .get(id)
        .and_then(placed_bounds)
        .map(rect_json)
        .unwrap_or(Value::Null)
}

/// Adds an SVG object to the active document and selects it.
pub(crate) fn add_svg(
    s: &mut Session,
    name: String,
    bytes: Vec<u8>,
) -> Result<Value, CommandError> {
    let import = import_svg(&bytes, fonts::system_fonts()).map_err(import_error)?;
    let warnings: Vec<String> = import.warnings.iter().map(warning_text).collect();
    let id = add_object(s, name.clone(), Source::Svg { name, bytes }, import.paths)?;
    Ok(json!({ "id": id.0, "bounds_mm": bounds_json(s, id), "warnings": warnings }))
}

/// Warnings and numbers that describe a trace, for people and agents.
fn trace_report(t: &TraceResult, invert: bool) -> (Vec<String>, Value) {
    let mut warnings = Vec::new();
    if t.paths.is_empty() {
        let shape = if invert { "light" } else { "dark" };
        warnings.push(format!(
            "nothing was traced: no {shape} shape is big enough. Try another threshold, invert, or a smaller despeckle area"
        ));
    }
    if let Some((w, h)) = t.downscaled_from {
        warnings.push(format!(
            "The image was traced at a lower resolution ({w} x {h} px) to keep memory in check."
        ));
    }
    if let Some(dpi) = t.ignored_dpi {
        warnings.push(format!(
            "The file declares {dpi:.1} dpi, which is not plausible. It was ignored and 300 dpi used; set dpi_override to change it."
        ));
    }
    if t.dropped_small > 0 {
        warnings.push(format!(
            "{} shapes were dropped because they are smaller than the minimum feature size",
            t.dropped_small
        ));
    }
    let numbers = json!({
        "dropped_small": t.dropped_small,
        "threshold_used": t.threshold_used,
        "size_mm": [t.size_mm.0, t.size_mm.1],
        "downscaled_from": t.downscaled_from.map(|(w, h)| [w, h]),
    });
    (warnings, numbers)
}

/// Adds a traced raster object to the active document and selects it.
fn add_raster(s: &mut Session, name: String, bytes: Vec<u8>) -> Result<Value, CommandError> {
    let settings = TraceSettings::default();
    let t = trace(&bytes, &settings, blade_offset_mm(s)).map_err(import_error)?;
    let (warnings, numbers) = trace_report(&t, settings.invert);
    let id = add_object(
        s,
        name.clone(),
        Source::Raster {
            name,
            bytes,
            trace: settings,
        },
        t.paths,
    )?;
    let mut out = json!({ "id": id.0, "bounds_mm": bounds_json(s, id), "warnings": warnings });
    merge(&mut out, numbers);
    Ok(out)
}

fn merge(into: &mut Value, from: Value) {
    if let (Some(a), Value::Object(b)) = (into.as_object_mut(), from) {
        a.extend(b);
    }
}

// ---------------------------------------------------------------- import

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportParams {
    /// Path of the file to import. Supported: .svg, .svgz, .png, .jpg, .jpeg, .webp and .gif.
    pub path: PathBuf,
}

fn run_import(s: &mut Session, p: ImportParams, _: Origin) -> Result<Value, CommandError> {
    let ext = p
        .path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let raster = matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif");
    if !raster && ext != "svg" && ext != "svgz" {
        let shown = if ext.is_empty() {
            "no extension".to_string()
        } else {
            format!("'.{ext}'")
        };
        return Err(CommandError::InvalidParams(format!(
            "unsupported file type ({shown}). Supported types: .svg, .svgz, .png, .jpg, .jpeg, .webp, .gif"
        )));
    }
    let name = p
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported")
        .to_string();
    let file = std::fs::File::open(&p.path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CommandError::NotFound(format!("file {}", p.path.display()))
        } else {
            CommandError::Failed(format!("cannot read {}: {e}", p.path.display()))
        }
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_IMPORT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| CommandError::Failed(format!("cannot read {}: {e}", p.path.display())))?;
    if bytes.len() as u64 > MAX_IMPORT_BYTES {
        return Err(CommandError::InvalidParams(
            "the file is larger than 50 MB".into(),
        ));
    }
    if raster {
        add_raster(s, name, bytes)
    } else {
        add_svg(s, name, bytes)
    }
}

command!(
    ObjectImport,
    id: "object.import",
    title: "Import file",
    mutates: true,
    params: ImportParams,
    run: run_import,
    description: "Imports a vector file from disk as a new object and selects it. Supported: .svg and .svgz, and the raster types .png, .jpg, .jpeg, .webp and .gif (by extension). The object name is the file name without extension. SVG sizes come from the SVG in mm. A raster image is traced into cut outlines with the default trace settings (automatic threshold, dark pixels are cut) at the dpi stored in the file, or 300 dpi; use object.retrace to change the settings. Images over 16384 x 16384 px are refused. The object keeps its position from the SVG page if that lies inside the cutting area, otherwise its top-left corner moves to the top-left of the cutting area. Coordinates are mm, origin top-left of the media, y down. Returns {id, bounds_mm: {x0,y0,x1,y1}, warnings: [text]}; raster imports also return dropped_small (shapes below the minimum feature size that were dropped), threshold_used (0-255) and size_mm [width, height] of the whole image.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportSvgParams {
    /// The SVG document as text.
    pub svg: String,
    /// Name of the new object.
    pub name: String,
}

fn run_import_svg(s: &mut Session, p: ImportSvgParams, _: Origin) -> Result<Value, CommandError> {
    add_svg(s, p.name, p.svg.into_bytes())
}

command!(
    ObjectImportSvg,
    id: "object.import_svg",
    title: "Import SVG text",
    mutates: true,
    params: ImportSvgParams,
    run: run_import_svg,
    description: "Imports SVG markup given as text as a new object and selects it. Give the SVG a width and height in mm (for example width=\"50mm\") so the size is exact; plain numbers are CSS pixels (96 per inch). Every visible path, fill or stroke, becomes a cut line; text is cut as outlines. Placement as in object.import. Coordinates are mm, origin top-left of the media, y down. Returns {id, bounds_mm: {x0,y0,x1,y1}, warnings: [text]}.",
);

// ---------------------------------------------------------------- retrace

/// Trace settings as the command takes them. Missing fields use the defaults.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct RetraceSettings {
    /// Grey level 0-255; pixels darker than this are cut. Omit or null for automatic (Otsu).
    pub threshold: Option<u8>,
    /// Cut the light pixels instead of the dark ones.
    pub invert: bool,
    /// Shapes with a smaller area in mm2 are specks and are ignored.
    pub despeckle_mm2: f64,
    /// Angles sharper than this many degrees stay corners (0-180).
    pub corner_threshold_deg: f64,
    /// 0 follows the pixels closely, 10 gives the smoothest curves.
    pub smoothing: u8,
    /// Shapes with a bounding box side below this many mm are dropped. Omit for 6 times the blade offset.
    pub min_feature_mm: Option<f64>,
    /// Pixels per inch to use instead of the one stored in the file (default 300).
    pub dpi_override: Option<f64>,
}

impl Default for RetraceSettings {
    fn default() -> Self {
        TraceSettings::default().into()
    }
}

impl From<TraceSettings> for RetraceSettings {
    fn from(t: TraceSettings) -> Self {
        Self {
            threshold: t.threshold,
            invert: t.invert,
            despeckle_mm2: t.despeckle_mm2,
            corner_threshold_deg: t.corner_threshold_deg,
            smoothing: t.smoothing,
            min_feature_mm: t.min_feature_mm,
            dpi_override: t.dpi_override,
        }
    }
}

impl RetraceSettings {
    fn into_trace(self) -> Result<TraceSettings, CommandError> {
        let bad = |m: &str| Err(CommandError::InvalidParams(m.to_string()));
        if self.smoothing > 10 {
            return bad("smoothing must be between 0 and 10");
        }
        if !(self.despeckle_mm2.is_finite() && self.despeckle_mm2 >= 0.0) {
            return bad("despeckle_mm2 must be 0 or more");
        }
        if !(0.0..=180.0).contains(&self.corner_threshold_deg) {
            return bad("corner_threshold_deg must be between 0 and 180");
        }
        if let Some(m) = self.min_feature_mm
            && !(m.is_finite() && m >= 0.0)
        {
            return bad("min_feature_mm must be 0 or more");
        }
        if let Some(d) = self.dpi_override
            && !(d.is_finite() && d > 0.0)
        {
            return bad("dpi_override must be a positive number (dpi)");
        }
        Ok(TraceSettings {
            threshold: self.threshold,
            invert: self.invert,
            despeckle_mm2: self.despeckle_mm2,
            corner_threshold_deg: self.corner_threshold_deg,
            smoothing: self.smoothing,
            min_feature_mm: self.min_feature_mm,
            dpi_override: self.dpi_override,
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RetraceParams {
    /// Id of a raster object.
    pub id: u64,
    /// The new trace settings. Fields you leave out go back to their defaults.
    pub settings: RetraceSettings,
}

fn run_retrace(s: &mut Session, p: RetraceParams, _: Origin) -> Result<Value, CommandError> {
    let id = ObjectId(p.id);
    let settings = p.settings.into_trace()?;
    let blade = blade_offset_mm(s);
    let o = s
        .doc()
        .document
        .get(id)
        .ok_or_else(|| CommandError::NotFound(format!("no object with id {}", id.0)))?;
    let Source::Raster { bytes, .. } = &o.source else {
        return Err(CommandError::InvalidParams(format!(
            "object {} is not a raster image, only traced images can be retraced",
            id.0
        )));
    };
    let t = trace(bytes, &settings, blade).map_err(import_error)?;
    let invert = settings.invert;
    let (warnings, numbers) = trace_report(&t, invert);
    let o = object_mut(&mut s.doc_mut().document, id)?;
    if let Source::Raster { trace, .. } = &mut o.source {
        *trace = settings;
    }
    // Keep the image frame in place, not the geometry centre: the placement
    // is relative to the centre of the local bounds, which the new trace
    // changes. Map the new bounds centre through the old placement.
    let old_placement = o.transform.affine(o.local_bounds);
    o.set_geometry(t.paths);
    o.transform.center_mm = old_placement * o.local_bounds.center();
    let mut out = json!({ "id": id.0, "bounds_mm": bounds_json(s, id), "warnings": warnings });
    merge(&mut out, numbers);
    Ok(out)
}

command!(
    ObjectRetrace,
    id: "object.retrace",
    title: "Retrace image",
    mutates: true,
    params: RetraceParams,
    run: run_retrace,
    description: "Traces a raster object again with new settings and keeps its position, scale, rotation and mirror. Settings (all optional, omitted ones use the defaults): threshold (0-255, null = automatic), invert (cut the light pixels), despeckle_mm2 (default 0.5), corner_threshold_deg (default 60), smoothing (0-10, default 5), min_feature_mm (null = 6 times the blade offset of the plotter), dpi_override (null = dpi of the file, else 300). Only raster objects can be retraced. Returns {id, bounds_mm, warnings, dropped_small, threshold_used, size_mm}.",
);

// ---------------------------------------------------------------- select / visible / delete

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectParams {
    /// Object ids to select. An empty list clears the selection.
    #[serde(default)]
    pub ids: Vec<u64>,
}

fn run_select(s: &mut Session, p: SelectParams, _: Origin) -> Result<Value, CommandError> {
    let ids = if p.ids.is_empty() {
        Vec::new()
    } else {
        resolve_ids(s, &p.ids)?
    };
    let st = s.doc_mut();
    st.selection = ids.iter().copied().collect();
    let sel: Vec<ObjectId> = st.selection.iter().copied().collect();
    Ok(json!({ "selection": ids_json(&sel) }))
}

command!(
    ObjectSelect,
    id: "object.select",
    title: "Select",
    mutates: false,
    params: SelectParams,
    run: run_select,
    description: "Replaces the selection with the given object ids. An empty list clears the selection. Other object commands use the selection when their ids list is empty. Returns {selection: [id]}.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetVisibleParams {
    /// Object ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// False hides the objects: hidden objects are not cut.
    pub visible: bool,
}

fn run_set_visible(s: &mut Session, p: SetVisibleParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let doc = &mut s.doc_mut().document;
    for id in &ids {
        object_mut(doc, *id)?.visible = p.visible;
    }
    Ok(json!({ "ids": ids_json(&ids), "visible": p.visible }))
}

command!(
    ObjectSetVisible,
    id: "object.set_visible",
    title: "Show or hide",
    mutates: true,
    params: SetVisibleParams,
    run: run_set_visible,
    description: "Shows or hides objects. Hidden objects stay in the document but are not cut and are skipped by arrange.auto. An empty ids list means the current selection. Returns {ids, visible}.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdsParams {
    /// Object ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
}

fn run_delete(s: &mut Session, p: IdsParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let st = s.doc_mut();
    for id in &ids {
        st.document.remove(*id);
        st.selection.remove(id);
    }
    Ok(json!({ "deleted": ids_json(&ids) }))
}

command!(
    ObjectDelete,
    id: "object.delete",
    title: "Delete",
    mutates: true,
    params: IdsParams,
    run: run_delete,
    description: "Deletes objects (one undo step). An empty ids list means the current selection. Returns {deleted: [id]}.",
);

// ---------------------------------------------------------------- duplicate

fn default_offset() -> [f64; 2] {
    [10.0, 10.0]
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuplicateParams {
    /// Object ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// Offset of the copies in mm, [dx, dy]; y points down.
    #[serde(default = "default_offset")]
    pub offset_mm: [f64; 2],
}

fn run_duplicate(s: &mut Session, p: DuplicateParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let offset = Vec2::new(p.offset_mm[0], p.offset_mm[1]);
    let st = s.doc_mut();
    let mut new_ids = Vec::new();
    for id in &ids {
        let mut copy = object_mut(&mut st.document, *id)?.clone();
        let new_id = ObjectId(st.document.next_id);
        st.document.next_id += 1;
        copy.id = new_id;
        translate(&mut copy, offset);
        st.document.objects.push(copy);
        new_ids.push(new_id);
    }
    st.selection = new_ids.iter().copied().collect();
    Ok(json!({ "ids": ids_json(&new_ids) }))
}

command!(
    ObjectDuplicate,
    id: "object.duplicate",
    title: "Duplicate",
    mutates: true,
    params: DuplicateParams,
    run: run_duplicate,
    description: "Copies objects and moves the copies by offset_mm [dx, dy] (default [10, 10]; mm, y down). The copies become the selection. An empty ids list means the current selection. Returns {ids: [new id]} in the order of the originals.",
);

// ---------------------------------------------------------------- move

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveParams {
    /// Object ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// Relative move along x in mm (right is positive).
    pub dx_mm: Option<f64>,
    /// Relative move along y in mm (down is positive).
    pub dy_mm: Option<f64>,
    /// Absolute target [x, y] in mm for the centre of the objects' common bounding box. Do not combine with dx_mm/dy_mm.
    pub center_mm: Option<[f64; 2]>,
}

fn run_move(s: &mut Session, p: MoveParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let relative = p.dx_mm.is_some() || p.dy_mm.is_some();
    let delta = match (relative, p.center_mm) {
        (true, Some(_)) => {
            return Err(CommandError::InvalidParams(
                "give either dx_mm/dy_mm or center_mm, not both".into(),
            ));
        }
        (false, None) => {
            return Err(CommandError::InvalidParams(
                "give dx_mm/dy_mm for a relative move or center_mm for an absolute one".into(),
            ));
        }
        (true, None) => Vec2::new(p.dx_mm.unwrap_or(0.0), p.dy_mm.unwrap_or(0.0)),
        (false, Some([x, y])) => {
            let doc = &s.doc().document;
            let group = ids
                .iter()
                .filter_map(|id| doc.get(*id))
                .map(placed_bounds_or_center)
                .reduce(|a, b| a.union(b))
                .unwrap_or(Rect::ZERO);
            Point::new(x, y) - group.center()
        }
    };
    let doc = &mut s.doc_mut().document;
    for id in &ids {
        translate(object_mut(doc, *id)?, delta);
    }
    Ok(json!({ "ids": ids_json(&ids), "moved_by_mm": [delta.x, delta.y] }))
}

command!(
    ObjectMove,
    id: "object.move",
    title: "Move",
    mutates: true,
    params: MoveParams,
    run: run_move,
    description: "Moves objects. Either relative with dx_mm and dy_mm (a missing one is 0), or absolute with center_mm [x, y], which puts the centre of the objects' common bounding box there. Units mm, origin top-left of the media, x right, y down. An empty ids list means the current selection. Returns {ids, moved_by_mm: [dx, dy]}.",
);

// ---------------------------------------------------------------- scale

#[derive(Debug, Clone, Copy, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    #[default]
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Anchor {
    fn point(self, r: Rect) -> Point {
        match self {
            Anchor::Center => r.center(),
            Anchor::TopLeft => Point::new(r.x0, r.y0),
            Anchor::TopRight => Point::new(r.x1, r.y0),
            Anchor::BottomLeft => Point::new(r.x0, r.y1),
            Anchor::BottomRight => Point::new(r.x1, r.y1),
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScaleParams {
    /// Object ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// Uniform scale factor, for example 2 doubles the size. Do not combine with width_mm/height_mm.
    pub factor: Option<f64>,
    /// Target width of the placed bounding box in mm.
    pub width_mm: Option<f64>,
    /// Target height of the placed bounding box in mm.
    pub height_mm: Option<f64>,
    /// Keep the proportions (default true). With only one of width_mm/height_mm the object scales uniformly; with both it fits inside them.
    #[serde(default = "default_true")]
    pub keep_aspect: bool,
    /// Point of the bounding box that stays fixed (default center).
    #[serde(default)]
    pub anchor: Anchor,
}

fn positive(name: &str, v: Option<f64>) -> Result<Option<f64>, CommandError> {
    match v {
        Some(x) if !(x.is_finite() && x > 0.0) => Err(CommandError::InvalidParams(format!(
            "{name} must be greater than 0"
        ))),
        other => Ok(other),
    }
}

fn run_scale(s: &mut Session, p: ScaleParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let factor = positive("factor", p.factor)?;
    let width = positive("width_mm", p.width_mm)?;
    let height = positive("height_mm", p.height_mm)?;
    let sized = width.is_some() || height.is_some();
    if factor.is_some() == sized {
        return Err(CommandError::InvalidParams(
            "give either factor, or width_mm and/or height_mm".into(),
        ));
    }
    let doc = &mut s.doc_mut().document;
    for id in &ids {
        let o = object_mut(doc, *id)?;
        let old = placed_bounds(o).ok_or_else(|| {
            CommandError::InvalidParams(format!("object {} has no geometry to scale", id.0))
        })?;
        let ratio =
            |target: Option<f64>, have: f64, what: &str| -> Result<Option<f64>, CommandError> {
                match target {
                    None => Ok(None),
                    Some(_) if have <= 1e-12 => Err(CommandError::InvalidParams(format!(
                        "object {} has no {what}, so its {what} cannot be set",
                        id.0
                    ))),
                    Some(t) => Ok(Some(t / have)),
                }
            };
        let (fx, fy) = if let Some(f) = factor {
            (f, f)
        } else {
            let rw = ratio(width, old.width(), "width")?;
            let rh = ratio(height, old.height(), "height")?;
            if p.keep_aspect {
                let f = match (rw, rh) {
                    (Some(a), Some(b)) => a.min(b),
                    (Some(a), None) | (None, Some(a)) => a,
                    (None, None) => 1.0,
                };
                (f, f)
            } else {
                (rw.unwrap_or(1.0), rh.unwrap_or(1.0))
            }
        };
        if fx == fy {
            o.transform.scale_x *= fx;
            o.transform.scale_y *= fy;
        } else {
            // Screen axes map to local axes only at multiples of 90 degrees.
            let quarter = o.transform.rotation_deg / 90.0;
            let k = quarter.round();
            if (quarter - k).abs() > 1e-9 {
                return Err(CommandError::InvalidParams(format!(
                    "object {} is rotated by {} degrees; width and height can only be set separately at multiples of 90 degrees. Use keep_aspect or a factor",
                    id.0, o.transform.rotation_deg
                )));
            }
            if (k as i64).rem_euclid(2) == 1 {
                o.transform.scale_x *= fy;
                o.transform.scale_y *= fx;
            } else {
                o.transform.scale_x *= fx;
                o.transform.scale_y *= fy;
            }
        }
        if let Some(new) = placed_bounds(o) {
            translate(o, p.anchor.point(old) - p.anchor.point(new));
        }
    }
    Ok(json!({ "ids": ids_json(&ids) }))
}

command!(
    ObjectScale,
    id: "object.scale",
    title: "Scale",
    mutates: true,
    params: ScaleParams,
    run: run_scale,
    description: "Scales each object on its own (one undo step for all). Either a uniform factor, or width_mm and/or height_mm for the placed bounding box in mm. keep_aspect (default true) scales uniformly: with one size given it follows that size, with both it fits inside them. With keep_aspect false only the given sides change; this needs a rotation that is a multiple of 90 degrees. anchor (center, top_left, top_right, bottom_left, bottom_right; default center) is the point of each bounding box that stays fixed. Origin top-left of the media, y down. An empty ids list means the current selection. Returns {ids}.",
);

// ---------------------------------------------------------------- rotate

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RotateParams {
    /// Object ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// Angle in degrees, clockwise on screen, added to the current rotation.
    pub degrees: f64,
}

fn run_rotate(s: &mut Session, p: RotateParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let doc = &mut s.doc_mut().document;
    let mut out = Vec::new();
    for id in &ids {
        let t = &mut object_mut(doc, *id)?.transform;
        t.rotation_deg = normalize_deg(t.rotation_deg + p.degrees);
        out.push(json!({ "id": id.0, "rotation_deg": t.rotation_deg }));
    }
    Ok(json!({ "objects": out }))
}

command!(
    ObjectRotate,
    id: "object.rotate",
    title: "Rotate",
    mutates: true,
    params: RotateParams,
    run: run_rotate,
    description: "Rotates each object around its own centre by degrees, relative to its current rotation. Positive is clockwise on screen (y points down). The stored rotation is kept in [0, 360). An empty ids list means the current selection. Returns {objects: [{id, rotation_deg}]}.",
);

// ---------------------------------------------------------------- mirror

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MirrorAxis {
    /// Flip left and right (across the vertical line through the centre).
    Horizontal,
    /// Flip top and bottom (across the horizontal line through the centre).
    Vertical,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MirrorParams {
    /// Object ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// horizontal flips left and right, vertical flips top and bottom.
    pub axis: MirrorAxis,
}

fn run_mirror(s: &mut Session, p: MirrorParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let doc = &mut s.doc_mut().document;
    for id in &ids {
        let t = &mut object_mut(doc, *id)?.transform;
        t.mirror = !t.mirror;
        t.rotation_deg = normalize_deg(match p.axis {
            MirrorAxis::Horizontal => -t.rotation_deg,
            MirrorAxis::Vertical => 180.0 - t.rotation_deg,
        });
    }
    Ok(json!({ "ids": ids_json(&ids) }))
}

command!(
    ObjectMirror,
    id: "object.mirror",
    title: "Mirror",
    mutates: true,
    params: MirrorParams,
    run: run_mirror,
    description: "Mirrors each object along a screen axis through its own centre. horizontal flips left and right, vertical flips top and bottom. Use horizontal for iron-on vinyl, which is cut from the back. An empty ids list means the current selection. Returns {ids}.",
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_retrace_settings_are_the_trace_defaults() {
        let s = RetraceSettings::default().into_trace().unwrap();
        assert_eq!(s, TraceSettings::default());
        let from_empty: RetraceSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(from_empty.into_trace().unwrap(), TraceSettings::default());
    }
}
