//! Object commands: import, select, show, delete, duplicate, copies,
//! transform and cut options.

use std::path::PathBuf;

use flatcraft_core::document::{
    CutMode, CutOptions, Document, ObjectId, Source, StrokeMode, TraceSettings,
};
use flatcraft_core::geometry::CutPath;
use flatcraft_import::svg::import_svg;
use flatcraft_import::trace::{TraceResult, trace};
use flatcraft_import::{ImportError, ImportWarning, fonts};
use kurbo::{Point, Rect, Vec2};
use serde::Deserialize;
use serde_json::{Value, json};

use super::sources::{FileKind, ReadError, absolute, clip, file_kind, new_link, read_capped};
use super::{
    check_exists, command, cutting_area, ids_json, inside, object_mut, placed_bounds,
    placed_bounds_or_center, rect_json, resolve_ids, translate,
};
use crate::commands::normalize_deg;
use crate::session::{CommandError, Origin, Session};

/// Text for an import warning, for people and agents.
pub(crate) fn warning_text(w: &ImportWarning) -> String {
    match w {
        ImportWarning::ClipOrMaskIgnored { count: 1 } => {
            "1 clip path or mask was ignored; the full shape will be cut".into()
        }
        ImportWarning::ClipOrMaskIgnored { count } => {
            format!("{count} clip paths or masks were ignored; the full shapes will be cut")
        }
        ImportWarning::ImagesSkipped { count: 1 } => {
            "1 embedded image was skipped; only vector paths are cut".into()
        }
        ImportWarning::ImagesSkipped { count } => {
            format!("{count} embedded images were skipped; only vector paths are cut")
        }
        ImportWarning::NoPaths => "the file has no visible paths, so nothing will be cut".into(),
        ImportWarning::DashesIgnored { count: 1 } => {
            "1 stroke is dashed; its dashes are cut as a solid line".into()
        }
        ImportWarning::DashesIgnored { count } => {
            format!("{count} strokes are dashed; dashes are cut as a solid line")
        }
        ImportWarning::TextSkipped => {
            "text uses a font that isn't installed, so it was skipped".into()
        }
    }
}

/// Import errors are sentences; a parse error carries foreign detail, so
/// it is clipped.
fn import_error(e: ImportError) -> CommandError {
    CommandError::Failed(clip(e.to_string()))
}

/// Blade offset of the active document's plotter, mm. 0.25 if the plotter
/// cannot be resolved.
pub fn blade_offset_mm(s: &Session) -> f64 {
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
/// An object without geometry (an image that traced to nothing) is placed
/// by its frame instead, so a later retrace lands in the cutting area.
/// `frame` is the source's page frame in local mm.
fn add_object(
    s: &mut Session,
    name: String,
    source: Source,
    paths: Vec<CutPath>,
    frame: Rect,
) -> Result<ObjectId, CommandError> {
    let area = cutting_area(s).ok();
    let doc = &mut s.doc_mut().document;
    let id = doc.add(name, source, paths);
    let o = object_mut(doc, id)?;
    o.frame = frame;
    o.transform.center_mm = o.local_bounds.center();
    let placed = placed_bounds(o).or_else(|| {
        (frame != Rect::ZERO).then(|| {
            o.transform
                .affine(o.local_bounds)
                .transform_rect_bbox(frame)
        })
    });
    if let (Some(area), Some(b)) = (area, placed) {
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
) -> Result<(ObjectId, Value), CommandError> {
    let import = import_svg(&bytes, fonts::system_fonts()).map_err(import_error)?;
    let mut warnings: Vec<String> = import.warnings.iter().map(warning_text).collect();
    let id = add_object(
        s,
        name.clone(),
        Source::Svg {
            name,
            bytes: bytes.into(),
        },
        import.paths,
        import.page_frame_mm,
    )?;
    warnings.extend(larger_than_area(s, id));
    let out = json!({ "id": id.0, "bounds_mm": bounds_json(s, id), "warnings": warnings });
    Ok((id, out))
}

/// Warnings and numbers that describe a trace, for people and agents.
pub(crate) fn trace_report(t: &TraceResult, invert: bool) -> (Vec<String>, Value) {
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
    match t.dropped_small {
        0 => {}
        1 => warnings
            .push("1 shape was dropped because it is smaller than the minimum feature size".into()),
        n => warnings.push(format!(
            "{n} shapes were dropped because they are smaller than the minimum feature size"
        )),
    }
    let numbers = json!({
        "dropped_small": t.dropped_small,
        "despeckled": t.despeckled,
        "threshold_used": t.threshold_used,
        "size_mm": [t.size_mm.0, t.size_mm.1],
        "downscaled_from": t.downscaled_from.map(|(w, h)| [w, h]),
    });
    (warnings, numbers)
}

/// Adds a traced raster object to the active document and selects it.
fn add_raster(
    s: &mut Session,
    name: String,
    bytes: Vec<u8>,
) -> Result<(ObjectId, Value), CommandError> {
    let settings = TraceSettings::default();
    let t = trace(&bytes, &settings, blade_offset_mm(s)).map_err(import_error)?;
    let (warnings, numbers) = trace_report(&t, settings.invert);
    let id = add_object(
        s,
        name.clone(),
        Source::Raster {
            name,
            bytes: bytes.into(),
            trace: settings,
        },
        t.paths,
        Rect::new(0.0, 0.0, t.size_mm.0, t.size_mm.1),
    )?;
    let mut warnings = warnings;
    warnings.extend(larger_than_area(s, id));
    let mut out = json!({ "id": id.0, "bounds_mm": bounds_json(s, id), "warnings": warnings });
    merge(&mut out, numbers);
    Ok((id, out))
}

/// A warning if the shapes of object `id` do not fit the cutting area in
/// width or height, so no move can bring them inside.
fn larger_than_area(s: &Session, id: ObjectId) -> Option<String> {
    let area = cutting_area(s).ok()?;
    let b = s.doc().document.get(id).and_then(placed_bounds)?;
    const EPS: f64 = 1e-6;
    (b.width() > area.width() + EPS || b.height() > area.height() + EPS).then(|| {
        format!(
            "The shapes are {:.0} x {:.0} mm, larger than the cutting area ({:.0} x {:.0} mm). Scale them down or pick a bigger mat before you cut.",
            b.width(),
            b.height(),
            area.width(),
            area.height()
        )
    })
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
    /// Path of the file to import. Supported: .svg, .svgz, .png, .jpg, .jpeg, .webp and .gif. Give path or paths.
    pub path: Option<PathBuf>,
    /// Several files to import in one undo step. A file that cannot be imported is listed in failed; the others are still imported, and the call never fails because of a file. Give path or paths.
    #[serde(default)]
    pub paths: Vec<PathBuf>,
}

/// Imports one file into the active document and selects it. A failure
/// changes nothing.
fn import_one(s: &mut Session, path: &std::path::Path) -> Result<Value, CommandError> {
    let kind = file_kind(path)?;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported")
        .to_string();
    let file = read_capped(path).map_err(|e| match e {
        ReadError::NotFound => CommandError::NotFound(format!("file {}", path.display())),
        ReadError::TooLarge | ReadError::NotAFile => {
            CommandError::InvalidParams(format!("cannot import {}: {}", path.display(), e.text()))
        }
        ReadError::Io(io) => CommandError::Failed(format!("cannot read {}: {io}", path.display())),
    })?;
    let abs = absolute(path);
    let link = new_link(&abs, s.doc().path.as_deref(), &file);
    let (id, out) = match kind {
        FileKind::Raster => add_raster(s, name, file.bytes),
        FileKind::Svg => add_svg(s, name, file.bytes),
    }
    .map_err(|e| with_path(e, path))?;
    object_mut(&mut s.doc_mut().document, id)?.link = link;
    Ok(out)
}

/// An import error that names the file, unless it does already.
fn with_path(e: CommandError, path: &std::path::Path) -> CommandError {
    let shown = path.display().to_string();
    let add = |m: String| {
        if m.contains(&shown) {
            m
        } else {
            format!("{shown}: {m}")
        }
    };
    match e {
        CommandError::InvalidParams(m) => CommandError::InvalidParams(add(m)),
        CommandError::NotFound(m) => CommandError::NotFound(add(m)),
        CommandError::Refused(m) => CommandError::Refused(add(m)),
        CommandError::Failed(m) => CommandError::Failed(add(m)),
        other => other,
    }
}

fn run_import(s: &mut Session, p: ImportParams, _: Origin) -> Result<Value, CommandError> {
    match (p.path, p.paths.is_empty()) {
        (Some(path), true) => import_one(s, &path),
        (None, false) => import_many(s, &p.paths),
        _ => Err(CommandError::InvalidParams(
            "give exactly one of path and paths".into(),
        )),
    }
}

/// Imports several files as one command. It never fails because of a
/// file: a file that cannot be imported is listed in `failed`, and a call
/// that imported nothing changes nothing (no undo step, not dirty).
fn import_many(s: &mut Session, paths: &[PathBuf]) -> Result<Value, CommandError> {
    let (mut ids, mut failed, mut warnings) = (Vec::new(), Vec::new(), Vec::new());
    for path in paths {
        match import_one(s, path) {
            Ok(out) => {
                ids.extend(out["id"].as_u64().map(ObjectId));
                let name = path.display();
                for w in out["warnings"].as_array().into_iter().flatten() {
                    if let Some(w) = w.as_str() {
                        warnings.push(format!("{name}: {w}"));
                    }
                }
            }
            Err(e) => failed.push(json!({ "path": path, "reason": e.to_string() })),
        }
    }
    if !ids.is_empty() {
        s.doc_mut().selection = ids.iter().copied().collect();
    }
    Ok(json!({ "imported": ids_json(&ids), "failed": failed, "warnings": warnings }))
}

command!(
    ObjectImport,
    id: "object.import",
    title: "Import file",
    mutates: true,
    params: ImportParams,
    run: run_import,
    description: "Imports a vector file from disk as a new object and selects it. Supported: .svg and .svgz, and the raster types .png, .jpg, .jpeg, .webp and .gif (by extension). The object name is the file name without extension. The object keeps an embedded copy and remembers the file, so after the file is changed in another app object.reload picks up the changes. SVG sizes come from the SVG in mm. A raster image is traced into cut outlines with the default trace settings (automatic threshold, dark pixels are cut) at the dpi stored in the file, or 300 dpi; use object.retrace to change the settings. Images over 16384 x 16384 px are refused. The object keeps its position from the SVG page if that lies inside the cutting area, otherwise its top-left corner moves to the top-left of the cutting area. Coordinates are mm, origin the 0,0 of the grid printed on the mat, y down. With paths instead of path, several files are imported as one undo step and all new objects are selected; the call never fails because of a file: a file that cannot be imported is listed in failed and the others are still imported, and when none could be imported nothing changes (no undo step); it returns {imported: [id], failed: [{path, reason}], warnings: [text, each starting with the file path]}. With path it returns {id, bounds_mm: {x0,y0,x1,y1}, warnings: [text]}; raster imports also return dropped_small (shapes below the minimum feature size that were dropped), despeckled (specks below the despeckle area that were removed), threshold_used (0-255) and size_mm [width, height] of the whole image.",
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
    add_svg(s, p.name, p.svg.into_bytes()).map(|(_, out)| out)
}

command!(
    ObjectImportSvg,
    id: "object.import_svg",
    title: "Import SVG text",
    mutates: true,
    params: ImportSvgParams,
    run: run_import_svg,
    description: "Imports SVG markup given as text as a new object and selects it. Give the SVG a width and height in mm (for example width=\"50mm\") so the size is exact; plain numbers are CSS pixels (96 per inch). Every visible path, fill or stroke, is cut as set by object.set_cut_options (default: the outline of fills and of strokes 0.5 mm or wider); text is cut as outlines. Placement as in object.import. Coordinates are mm, origin the 0,0 of the grid printed on the mat, y down. Returns {id, bounds_mm: {x0,y0,x1,y1}, warnings: [text]}.",
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
    let size_before = placed_bounds(o).map(|b| b.size());
    let invert = settings.invert;
    let (mut warnings, numbers) = trace_report(&t, invert);
    let o = object_mut(&mut s.doc_mut().document, id)?;
    if let Source::Raster { trace, .. } = &mut o.source {
        *trace = settings;
    }
    // Keep the image frame in place, not the geometry centre: the placement
    // is relative to the centre of the local bounds, which the new trace
    // changes.
    o.set_geometry_anchored(t.paths, Rect::new(0.0, 0.0, t.size_mm.0, t.size_mm.1));
    // Linked copies share the trace settings and the geometry.
    s.doc_mut().document.spread_shared(id);
    let o = object_mut(&mut s.doc_mut().document, id)?;
    // Warn only when the retrace made the shapes grow: a size the user
    // already had was warned about before.
    let grew = match (size_before, placed_bounds(o).map(|b| b.size())) {
        (Some(a), Some(b)) => b.width > a.width + 1e-6 || b.height > a.height + 1e-6,
        (None, Some(_)) => true,
        _ => false,
    };
    if grew {
        warnings.extend(larger_than_area(s, id));
    }
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
    description: "Traces a raster object again with new settings and keeps its position, scale, rotation and mirror. Settings (all optional, omitted ones use the defaults): threshold (0-255, null = automatic), invert (cut the light pixels), despeckle_mm2 (default 0.5), corner_threshold_deg (default 60), smoothing (0-10, default 5), min_feature_mm (null = 6 times the blade offset of the plotter), dpi_override (null = dpi of the file, else 300). Only raster objects can be retraced. Linked copies (see object.copies) share the trace settings, so all members of the group are retraced in the same undo step, each keeping its own position, rotation and mirror. Returns {id, bounds_mm, warnings, dropped_small (shapes below the minimum feature size), despeckled (specks below the despeckle area), threshold_used, size_mm}.",
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
    session: true,
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

/// Adds a copy of object `id`, moved by `offset`, with a new id, at the end
/// of the document. The copy shares the original's data (`Arc`s) and
/// keeps its group of linked copies; callers that want an independent copy
/// clear `link_group`. Returns the new id.
fn add_copy(doc: &mut Document, id: ObjectId, offset: Vec2) -> Result<ObjectId, CommandError> {
    let mut copy = object_mut(doc, id)?.clone();
    let new_id = ObjectId(doc.next_id);
    doc.next_id += 1;
    copy.id = new_id;
    translate(&mut copy, offset);
    doc.objects.push(copy);
    Ok(new_id)
}

fn run_duplicate(s: &mut Session, p: DuplicateParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let offset = Vec2::new(p.offset_mm[0], p.offset_mm[1]);
    let st = s.doc_mut();
    let mut new_ids = Vec::new();
    for id in &ids {
        new_ids.push(add_copy(&mut st.document, *id, offset)?);
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
    description: "Copies objects and moves the copies by offset_mm [dx, dy] (default [10, 10]; mm, y down). The copies become the selection. A copy of a linked copy is a linked copy too: it joins the original's group and shares its source, geometry, trace settings, cut options, scale and file link (object.unlink_copies makes it independent). An empty ids list means the current selection. Returns {ids: [new id]} in the order of the originals.",
);

// ---------------------------------------------------------------- copies

/// Most copies one `object.copies` call makes.
pub const MAX_COPIES: u32 = 500;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CopiesLayout {
    /// One row to the right of the original.
    #[default]
    Row,
    /// Rows of `columns` cells, filled left to right, then down. The
    /// original is the first cell.
    Grid {
        /// Cells per row, 1 or more.
        columns: u32,
    },
}

fn default_gap() -> f64 {
    5.0
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CopiesParams {
    /// The object to copy.
    pub id: u64,
    /// How many copies to add, 1 to 500.
    pub count: u32,
    /// "row" (default) or {"grid": {"columns": n}}.
    #[serde(default)]
    pub layout: CopiesLayout,
    /// Gap between neighbouring bounding boxes in mm (default 5), 0 or more.
    #[serde(default = "default_gap")]
    pub gap_mm: f64,
    /// true makes linked copies: they share the source, the geometry, the trace settings, the cut options, the scale and the file link with the original; each keeps its own position, rotation and mirror.
    #[serde(default)]
    pub linked: bool,
}

/// Grid cell (column, row) of copy `k` (1-based); the original is cell 0.
fn cell(layout: CopiesLayout, k: u32) -> (u32, u32) {
    match layout {
        CopiesLayout::Row => (k, 0),
        CopiesLayout::Grid { columns } => (k % columns, k / columns),
    }
}

fn run_copies(s: &mut Session, p: CopiesParams, _: Origin) -> Result<Value, CommandError> {
    if !(1..=MAX_COPIES).contains(&p.count) {
        return Err(CommandError::InvalidParams(format!(
            "count must be 1 to {MAX_COPIES}"
        )));
    }
    if !(p.gap_mm.is_finite() && p.gap_mm >= 0.0) {
        return Err(CommandError::InvalidParams(
            "gap_mm must be 0 or more".into(),
        ));
    }
    if p.layout == (CopiesLayout::Grid { columns: 0 }) {
        return Err(CommandError::InvalidParams(
            "a grid needs 1 or more columns".into(),
        ));
    }
    let id = ObjectId(p.id);
    check_exists(&s.doc().document, id)?;
    let area = cutting_area(s).ok();
    let st = s.doc_mut();
    let group = p.linked.then(|| {
        st.document
            .get(id)
            .and_then(|o| o.link_group)
            .unwrap_or_else(|| st.document.new_group_id())
    });
    let original = object_mut(&mut st.document, id)?;
    if group.is_some() {
        original.link_group = group;
    }
    // Measured once: a copy's bounds are the original's, moved.
    let measured = placed_bounds(original);
    let visible = original.visible;
    let b = measured.unwrap_or_else(|| placed_bounds_or_center(original));
    let (step_x, step_y) = (b.width() + p.gap_mm, b.height() + p.gap_mm);
    let mut new_ids = Vec::with_capacity(p.count as usize);
    let mut outside = Vec::new();
    for k in 1..=p.count {
        let (col, row) = cell(p.layout, k);
        // From the original each time, so rounding does not add up.
        let offset = Vec2::new(col as f64 * step_x, row as f64 * step_y);
        let new_id = add_copy(&mut st.document, id, offset)?;
        let copy = object_mut(&mut st.document, new_id)?;
        // Without linked, a copy is independent, even of a linked copy.
        copy.link_group = group;
        if !copy.placement_is_valid() {
            return Err(CommandError::Refused(format!(
                "Copy {k} would land more than 1 km from the mat. Make fewer copies or a smaller gap."
            )));
        }
        if let (Some(area), Some(m)) = (area, measured)
            && visible
            && !inside(m + offset, area)
        {
            outside.push(new_id);
        }
        new_ids.push(new_id);
    }
    st.selection = new_ids.iter().copied().collect();
    let warnings: Vec<String> = match outside.len() {
        0 => Vec::new(),
        1 => vec![
            "1 copy is outside the cutting area. Move it, or pick a bigger mat, before you cut."
                .into(),
        ],
        n => vec![format!(
            "{n} copies are outside the cutting area. Move them, or pick a bigger mat, before you cut."
        )],
    };
    Ok(json!({
        "ids": ids_json(&new_ids),
        "outside_area": ids_json(&outside),
        "warnings": warnings,
    }))
}

command!(
    ObjectCopies,
    id: "object.copies",
    title: "Copies",
    mutates: true,
    params: CopiesParams,
    run: run_copies,
    description: "Adds count copies (1 to 500) of one object in one undo step, laid out next to it by its bounding box plus gap_mm (default 5). layout \"row\" (default) puts them in a row to the right of the original. layout {\"grid\": {\"columns\": n}} fills rows of n cells left to right, then goes down; the original is the first cell, so copy k sits in column k mod n and row k div n. Units mm, origin the 0,0 of the grid printed on the mat, x right, y down. Copies that land outside the cutting area are still added; they are listed in outside_area and get a warning. If a copy would land more than 1 km away, it refuses the whole call and adds nothing. Copies keep the original's file link, so a reload of that file updates all of them. linked: true (default false) makes linked copies: they and the original share the source, the geometry, the trace settings, the cut options, the scale and the file link, stored once in the project, so a change of one of these on any of them (object.scale, object.set_cut_options, object.retrace, object.reload, object.relink, object.unlink) changes all of them in the same undo step; each keeps its own position, rotation, mirror, name and visibility. Copies of a linked copy join its group. object.unlink_copies makes copies independent again. The copies become the selection. Returns {ids: [new id], outside_area: [id], warnings}.",
);

// ---------------------------------------------------------------- link copies

/// The embedded file of a source, whatever its kind and settings.
fn source_bytes(source: &Source) -> (bool, &[u8]) {
    match source {
        Source::Svg { bytes, .. } => (false, bytes),
        Source::Raster { bytes, .. } => (true, bytes),
    }
}

fn run_link(s: &mut Session, p: IdsParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let doc = &mut s.doc_mut().document;
    let lead_id = ids[0];
    // Members of a group that one of the ids is in come along.
    let mut all: Vec<ObjectId> = Vec::new();
    for id in &ids {
        for m in doc.members(*id) {
            if !all.contains(&m) {
                all.push(m);
            }
        }
    }
    if all.len() < 2 {
        return Err(CommandError::InvalidParams(
            "linking needs at least 2 objects".into(),
        ));
    }
    let lead = doc.get(lead_id).cloned().expect("resolved above");
    for id in &all {
        let o = doc.get(*id).expect("resolved above");
        // By value: two imports of one file hold equal bytes in two Arcs.
        if source_bytes(&o.source) != source_bytes(&lead.source) {
            return Err(CommandError::InvalidParams(format!(
                "object {} does not have the same source as object {}; only copies of the same file or image can be linked",
                id.0, lead_id.0
            )));
        }
    }
    let group = lead.link_group.unwrap_or_else(|| doc.new_group_id());
    for id in &all {
        let o = object_mut(doc, *id)?;
        o.link_group = Some(group);
        if *id != lead_id {
            o.take_shared(&lead);
        }
    }
    let members = doc.group_members(group);
    Ok(json!({ "group": group.0, "ids": ids_json(&members) }))
}

command!(
    ObjectLink,
    id: "object.link",
    title: "Link copies",
    mutates: true,
    params: IdsParams,
    run: run_link,
    description: "Makes objects linked copies of the first one in ids, in one undo step. They must have the same source (the same SVG or image content, for example copies or two imports of one file); otherwise it refuses and changes nothing. Every object takes the first one's shared fields: the source with its trace settings, the geometry, the cut options, the scale and the file link. Each keeps its own position, rotation, mirror, name and visibility; new geometry stays anchored to the object's page frame and a new scale keeps the object's own centre (mm, origin the 0,0 of the grid printed on the mat, y down). Objects that are linked copies already bring their whole group along. From then on a change of a shared field on one member (object.scale, object.set_cut_options, object.retrace, object.reload, object.relink, object.unlink) changes all of them. Needs 2 or more objects. An empty ids list means the current selection. Returns {group, ids: [every member]}.",
);

fn run_unlink_copies(s: &mut Session, p: IdsParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let doc = &mut s.doc_mut().document;
    let mut out = Vec::new();
    for id in &ids {
        let o = object_mut(doc, *id)?;
        if o.link_group.take().is_some() {
            out.push(*id);
        }
    }
    Ok(json!({ "ids": ids_json(&out) }))
}

command!(
    ObjectUnlinkCopies,
    id: "object.unlink_copies",
    title: "Unlink copy",
    mutates: true,
    params: IdsParams,
    run: run_unlink_copies,
    description: "Makes linked copies independent again, in one undo step. Each object keeps everything it had (source, trace settings, geometry, cut options, scale, file link, position, rotation and mirror; mm, origin the 0,0 of the grid printed on the mat), but from then on a change to it no longer changes the others, and theirs no longer change it. The other members stay linked to each other; a group left with one member is no group. This does not forget the file: object.unlink does that. Objects that are not linked copies are skipped. An empty ids list means the current selection. Returns {ids: [ids that were unlinked]}.",
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
    description: "Moves objects. Either relative with dx_mm and dy_mm (a missing one is 0), or absolute with center_mm [x, y], which puts the centre of the objects' common bounding box there. Units mm, origin the 0,0 of the grid printed on the mat, x right, y down. An empty ids list means the current selection. Returns {ids, moved_by_mm: [dx, dy]}.",
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
    /// Scale the objects as one group (default false): the common bounding box of all of them scales about its anchor point, so their layout stays. width_mm and height_mm then set the size of that common box; keep_aspect must stay true.
    #[serde(default)]
    pub group: bool,
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
    if p.group {
        return scale_group(s, &ids, factor, width, height, &p);
    }
    let doc = &mut s.doc_mut().document;
    // Measured first: a linked copy may scale before its turn comes.
    let mut olds = Vec::with_capacity(ids.len());
    for id in &ids {
        olds.push(placed_bounds(object_mut(doc, *id)?).ok_or_else(|| {
            CommandError::InvalidParams(format!("object {} has no geometry to scale", id.0))
        })?);
    }
    // Objects whose scale is set already, through a linked copy earlier in
    // `ids`: a group scales once, by its first member in `ids`.
    let mut scaled: Vec<ObjectId> = Vec::new();
    for (id, &old) in ids.iter().zip(&olds) {
        if !scaled.contains(id) {
            scale_one(doc, *id, old, factor, width, height, p.keep_aspect)?;
            // Linked copies share the scale; the others keep their centre.
            doc.spread_shared(*id);
            scaled.extend(doc.members(*id));
        }
        let o = object_mut(doc, *id)?;
        if let Some(new) = placed_bounds(o) {
            translate(o, p.anchor.point(old) - p.anchor.point(new));
        }
    }
    Ok(json!({ "ids": ids_json(&ids) }))
}

/// Scales one object so its placed bounds `old` get the factor or the size.
fn scale_one(
    doc: &mut Document,
    id: ObjectId,
    old: Rect,
    factor: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    keep_aspect: bool,
) -> Result<(), CommandError> {
    let o = object_mut(doc, id)?;
    {
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
            if keep_aspect {
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
    }
    Ok(())
}

/// `object.scale` with `group`: one uniform factor for all objects about
/// the anchor of their common bounding box.
fn scale_group(
    s: &mut Session,
    ids: &[ObjectId],
    factor: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    p: &ScaleParams,
) -> Result<Value, CommandError> {
    if !p.keep_aspect {
        return Err(CommandError::InvalidParams(
            "a group scales with its proportions kept: leave keep_aspect true".into(),
        ));
    }
    let doc = &mut s.doc_mut().document;
    let group_box = |doc: &Document| -> Result<Rect, CommandError> {
        let mut out: Option<Rect> = None;
        for id in ids {
            let o = doc
                .get(*id)
                .ok_or_else(|| CommandError::NotFound(format!("no object with id {}", id.0)))?;
            let b = placed_bounds(o).ok_or_else(|| {
                CommandError::InvalidParams(format!("object {} has no geometry to scale", id.0))
            })?;
            out = Some(out.map_or(b, |r| r.union(b)));
        }
        out.ok_or_else(|| CommandError::InvalidParams("no objects to scale".into()))
    };
    let old = group_box(doc)?;
    let ratio = |target: Option<f64>, have: f64, what: &str| match target {
        None => Ok(None),
        Some(_) if have <= 1e-12 => Err(CommandError::InvalidParams(format!(
            "the group has no {what}, so its {what} cannot be set"
        ))),
        Some(t) => Ok(Some(t / have)),
    };
    let f = match factor {
        Some(f) => f,
        None => match (
            ratio(width, old.width(), "width")?,
            ratio(height, old.height(), "height")?,
        ) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => 1.0,
        },
    };
    let anchor = p.anchor.point(old);
    for id in ids {
        let o = object_mut(doc, *id)?;
        o.transform.scale_x *= f;
        o.transform.scale_y *= f;
        o.transform.center_mm = anchor + (o.transform.center_mm - anchor) * f;
    }
    // Linked copies outside `ids` take the scale and keep their centre.
    // Members inside scaled once each from the same scale, so they agree.
    for id in ids {
        doc.spread_shared(*id);
    }
    // Cut outlines may not scale exactly (a stroke crossing the auto
    // threshold): put the anchor back where it was.
    let d = anchor - p.anchor.point(group_box(doc)?);
    for id in ids {
        translate(object_mut(doc, *id)?, d);
    }
    Ok(json!({ "ids": ids_json(ids) }))
}

command!(
    ObjectScale,
    id: "object.scale",
    title: "Scale",
    mutates: true,
    params: ScaleParams,
    run: run_scale,
    description: "Scales each object on its own (one undo step for all), or with group true all of them as one group about one anchor of their common bounding box, so their layout stays (the canvas does this for a multi-selection). Either a uniform factor, or width_mm and/or height_mm for the placed bounding box in mm. keep_aspect (default true) scales uniformly: with one size given it follows that size, with both it fits inside them. With keep_aspect false only the given sides change; this needs a rotation that is a multiple of 90 degrees. anchor (center, top_left, top_right, bottom_left, bottom_right; default center) is the point of each bounding box that stays fixed. Origin the 0,0 of the grid printed on the mat, y down. With strokes on auto (see object.set_cut_options), scaling can move a stroke across the 0.5 mm threshold and change how it is cut. Linked copies (see object.copies) share the scale: scaling one scales all members of its group in the same undo step, and each other member keeps its own centre, rotation and mirror; a group with several members in ids scales once, by its first member there. An empty ids list means the current selection. Returns {ids}.",
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

// ---------------------------------------------------------------- cut options

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CutModeParam {
    /// Every path as drawn (centre lines), whatever its fill or stroke.
    AllLines,
    /// The outline of every visible shape: fills, plus strokes as set by strokes.
    ObjectOutlines,
}

impl From<CutModeParam> for CutMode {
    fn from(m: CutModeParam) -> Self {
        match m {
            CutModeParam::AllLines => CutMode::AllLines,
            CutModeParam::ObjectOutlines => CutMode::ObjectOutlines,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StrokeModeParam {
    /// Outline for strokes at least auto_threshold_mm wide (after scaling), centre line for thinner ones.
    Auto,
    /// Cut the path itself.
    CentreLine,
    /// Cut around the visible stroke shape, with its corner and end style.
    Outline,
}

impl From<StrokeModeParam> for StrokeMode {
    fn from(m: StrokeModeParam) -> Self {
        match m {
            StrokeModeParam::Auto => StrokeMode::Auto,
            StrokeModeParam::CentreLine => StrokeMode::CentreLine,
            StrokeModeParam::Outline => StrokeMode::Outline,
        }
    }
}

/// Cut options as `document.state` and `object.set_cut_options` report them.
pub(crate) fn cut_json(c: &CutOptions) -> Value {
    let mode = match c.mode {
        CutMode::AllLines => "all_lines",
        CutMode::ObjectOutlines => "object_outlines",
    };
    let strokes = match c.strokes {
        StrokeMode::Auto => "auto",
        StrokeMode::CentreLine => "centre_line",
        StrokeMode::Outline => "outline",
    };
    json!({ "mode": mode, "strokes": strokes, "auto_threshold_mm": c.auto_threshold_mm })
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetCutOptionsParams {
    /// Object ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// What is cut: all_lines or object_outlines. Omit to keep the current value.
    pub mode: Option<CutModeParam>,
    /// How strokes are cut in object_outlines: auto, centre_line or outline. Omit to keep the current value.
    pub strokes: Option<StrokeModeParam>,
    /// Strokes at least this wide in mm (after scaling) are cut as outline in auto, 0.05 to 10. Omit to keep the current value.
    #[schemars(range(min = 0.05, max = 10.0))]
    pub auto_threshold_mm: Option<f64>,
}

fn run_set_cut_options(
    s: &mut Session,
    p: SetCutOptionsParams,
    _: Origin,
) -> Result<Value, CommandError> {
    if p.mode.is_none() && p.strokes.is_none() && p.auto_threshold_mm.is_none() {
        return Err(CommandError::InvalidParams(
            "give at least one of mode, strokes and auto_threshold_mm".into(),
        ));
    }
    if let Some(t) = p.auto_threshold_mm
        && !CutOptions::AUTO_THRESHOLD_MM.contains(&t)
    {
        return Err(CommandError::InvalidParams(
            "auto_threshold_mm must be between 0.05 and 10".into(),
        ));
    }
    let ids = resolve_ids(s, &p.ids)?;
    let doc = &mut s.doc_mut().document;
    let mut out = Vec::new();
    let mut done = Vec::new();
    for id in &ids {
        if done.contains(id) {
            continue;
        }
        let c = &mut object_mut(doc, *id)?.cut;
        if let Some(m) = p.mode {
            c.mode = m.into();
        }
        if let Some(m) = p.strokes {
            c.strokes = m.into();
        }
        if let Some(t) = p.auto_threshold_mm {
            c.auto_threshold_mm = t;
        }
        let c = *c;
        // Linked copies share the cut options.
        doc.spread_shared(*id);
        for m in doc.members(*id) {
            out.push(json!({ "id": m.0, "cut": cut_json(&c) }));
            done.push(m);
        }
    }
    Ok(json!({ "objects": out }))
}

command!(
    ObjectSetCutOptions,
    id: "object.set_cut_options",
    title: "Cut options",
    mutates: true,
    params: SetCutOptionsParams,
    run: run_set_cut_options,
    description: "Sets what is cut of each object (one undo step). mode: all_lines cuts every path as drawn (its centre line), whatever its fill or stroke; object_outlines (default) cuts the outline of every visible shape: fills, plus strokes as set by strokes. strokes: auto (default) cuts strokes at least auto_threshold_mm wide (default 0.5, range 0.05 to 10; measured after scaling) around their outline and thinner ones as centre line; centre_line cuts the path itself; outline cuts around the visible stroke shape with its corner and end style (sharp miter corners stay sharp). A path with a fill and an outline stroke is cut along the edge of fill and stroke together; holes stay holes. Different paths that overlap are each cut completely. Fields you omit keep their value; give at least one. Traced images are fills; the cut mode only normalises their outlines (direction and start point), what is cut stays the same. A filled path with a centre-line stroke is cut along the edge of its fill. Linked copies (see object.copies) share their cut options, so setting them on one sets them on all members of its group, in the same undo step. An empty ids list means the current selection. Returns {objects: [{id, cut: {mode, strokes, auto_threshold_mm}}]}, with every changed object, linked copies included.",
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_measure_the_original_once() {
        use crate::cutgeom::tests::CUT_BOUNDS_CALLS;
        let dir = tempfile::tempdir().unwrap();
        let profiles = flatcraft_device::ProfileStore::load(dir.path()).unwrap();
        let prefs = crate::prefs::Preferences {
            output_dir: Some(dir.path().join("output")),
            ..Default::default()
        };
        let mut s = Session::new(profiles, prefs, false, dir.path().to_path_buf());
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="20mm" viewBox="0 0 20 20"><rect x="2" y="2" width="5" height="5"/></svg>"#;
        let id = s
            .execute(
                "object.import_svg",
                json!({ "svg": svg, "name": "sq" }),
                Origin::User,
            )
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        let mut calls_for = |count: u32| {
            let before = CUT_BOUNDS_CALLS.with(std::cell::Cell::get);
            s.execute(
                "object.copies",
                json!({ "id": id, "count": count, "layout": { "grid": { "columns": 7 } } }),
                Origin::User,
            )
            .unwrap();
            CUT_BOUNDS_CALLS.with(std::cell::Cell::get) - before
        };
        let one = calls_for(1);
        let many = calls_for(40);
        assert_eq!(one, many, "cut_bounds runs per copy");
    }

    #[test]
    fn import_errors_are_clipped() {
        let e = import_error(ImportError::Parse("x".repeat(500)));
        let text = e.to_string();
        assert!(text.len() < 260, "{text}");
        assert!(text.ends_with("..."), "{text}");
    }

    #[test]
    fn default_retrace_settings_are_the_trace_defaults() {
        let s = RetraceSettings::default().into_trace().unwrap();
        assert_eq!(s, TraceSettings::default());
        let from_empty: RetraceSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(from_empty.into_trace().unwrap(), TraceSettings::default());
    }

    #[test]
    fn counts_of_one_are_singular() {
        let one = |w| warning_text(&w);
        assert_eq!(
            one(ImportWarning::ClipOrMaskIgnored { count: 1 }),
            "1 clip path or mask was ignored; the full shape will be cut"
        );
        assert_eq!(
            one(ImportWarning::ImagesSkipped { count: 1 }),
            "1 embedded image was skipped; only vector paths are cut"
        );
        assert_eq!(
            one(ImportWarning::DashesIgnored { count: 1 }),
            "1 stroke is dashed; its dashes are cut as a solid line"
        );
        assert_eq!(
            one(ImportWarning::TextSkipped),
            "text uses a font that isn't installed, so it was skipped"
        );
        assert!(one(ImportWarning::DashesIgnored { count: 2 }).starts_with("2 strokes are dashed"));
        let t = TraceResult {
            paths: Vec::new(),
            dropped_small: 1,
            despeckled: 0,
            size_mm: (10.0, 10.0),
            threshold_used: 128,
            downscaled_from: None,
            ignored_dpi: None,
        };
        let (warnings, _) = trace_report(&t, false);
        assert!(
            warnings.contains(
                &"1 shape was dropped because it is smaller than the minimum feature size".into()
            ),
            "{warnings:?}"
        );
    }
}
