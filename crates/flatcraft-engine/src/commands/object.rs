//! Object commands: import, select, show, delete, duplicate, copies,
//! transform and cut options.

use std::path::PathBuf;

use flatcraft_core::document::{
    CutMode, CutOptions, Document, KeepMode, Object, ObjectId, Source, StrokeMode, TraceSettings,
};
use flatcraft_core::geometry::CutPath;
use flatcraft_core::nodes::GroupNode;
use flatcraft_import::svg::import_svg;
use flatcraft_import::trace::{TraceResult, trace};
use flatcraft_import::{ImportError, ImportWarning, fonts};
use kurbo::{Point, Rect, Vec2};
use serde::Deserialize;
use serde_json::{Value, json};

use super::sources::{FileKind, ReadError, absolute, clip, file_kind, new_link, read_capped};
use super::{
    check_exists, check_node, command, common_box, cutting_area, ids_json, inside, object_mut,
    placed_bounds, rect_json, resolve_ids, resolve_units, translate, unit_leaves, units_of,
};
use crate::commands::normalize_deg;
use crate::placement;
use crate::report::{self, Code, Issue, Report};
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
    let placed = super::arrange_bounds(o);
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
    let found = report::svg_issues(id, &import.warnings);
    let out = json!({
        "id": id.0,
        "bounds_mm": bounds_json(s, id),
        "warnings": warnings,
        "report": import_report(s, id, found).to_json(),
    });
    Ok((id, out))
}

/// The report of a new object: what the import found in the file, then
/// the geometry checks of `document.check`.
fn import_report(s: &Session, id: ObjectId, found: Vec<Issue>) -> Report {
    report_of(s, &[id], found)
}

/// The report of objects whose source was read again (import, reload,
/// relink, retrace): `found` (what the files had), then the geometry
/// checks of `ids`.
pub(crate) fn report_of(s: &Session, ids: &[ObjectId], found: Vec<Issue>) -> Report {
    let mut report = crate::job::JobInput::of(s)
        .map(|input| report::check(&input, Some(ids)))
        .unwrap_or_default();
    let mut issues = found;
    issues.append(&mut report.issues);
    report.issues = issues;
    report
}

/// What reading a source found, typed, for the report.
#[derive(Debug, Clone)]
pub(crate) enum Found {
    Svg(Vec<ImportWarning>),
    Trace(Vec<(Code, usize, String)>),
}

impl Found {
    /// The issues of object `id`, naming `members` (it and its linked
    /// copies, which share the source).
    pub(crate) fn issues(&self, id: ObjectId, members: &[ObjectId]) -> Vec<Issue> {
        let mut out = match self {
            Found::Svg(w) => report::svg_issues(id, w),
            Found::Trace(f) => f
                .iter()
                .map(|(code, n, text)| report::trace_issue(id, *code, *n, text))
                .collect(),
        };
        if members.len() > 1 {
            for i in &mut out {
                i.object_ids = members.iter().map(|m| m.0).collect();
            }
        }
        out
    }
}

/// What a trace found that is worth a warning: the code, how many shapes
/// it is about and the warning text.
pub(crate) fn trace_findings(t: &TraceResult, invert: bool) -> Vec<(Code, usize, String)> {
    let mut out = Vec::new();
    if t.paths.is_empty() {
        let shape = if invert { "light" } else { "dark" };
        out.push((
            Code::NothingTraced,
            0,
            format!(
                "nothing was traced: no {shape} shape is big enough. Try another threshold, invert, or a smaller despeckle area"
            ),
        ));
    }
    if let Some((w, h)) = t.downscaled_from {
        out.push((
            Code::TraceDownscaled,
            0,
            format!(
                "The image was traced at a lower resolution ({w} x {h} px) to keep memory in check."
            ),
        ));
    }
    if let Some(dpi) = t.ignored_dpi {
        out.push((
            Code::DpiIgnored,
            0,
            format!(
                "The file declares {dpi:.1} dpi, which is not plausible. It was ignored and 300 dpi used; set dpi_override to change it."
            ),
        ));
    }
    match t.dropped_small {
        0 => {}
        1 => out.push((
            Code::ShapesDropped,
            1,
            "1 shape was dropped because it is smaller than the minimum feature size".into(),
        )),
        n => out.push((
            Code::ShapesDropped,
            n,
            format!(
                "{n} shapes were dropped because they are smaller than the minimum feature size"
            ),
        )),
    }
    out
}

/// Warnings and numbers that describe a trace, for people and agents.
pub(crate) fn trace_report(t: &TraceResult, invert: bool) -> (Vec<String>, Value) {
    let warnings = trace_findings(t, invert)
        .into_iter()
        .map(|(_, _, text)| text)
        .collect();
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
    let findings = trace_findings(&t, settings.invert);
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
    let found = findings
        .iter()
        .map(|(code, n, text)| report::trace_issue(id, *code, *n, text))
        .collect();
    let mut out = json!({
        "id": id.0,
        "bounds_mm": bounds_json(s, id),
        "warnings": warnings,
        "report": import_report(s, id, found).to_json(),
    });
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
    let mut all = Report::default();
    for path in paths {
        match import_one(s, path) {
            Ok(out) => {
                ids.extend(out["id"].as_u64().map(ObjectId));
                if let Ok(r) = serde_json::from_value::<Report>(out["report"].clone()) {
                    all.extend(r);
                }
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
    Ok(json!({
        "imported": ids_json(&ids),
        "failed": failed,
        "warnings": warnings,
        "report": all.to_json(),
    }))
}

command!(
    ObjectImport,
    id: "object.import",
    title: "Import file",
    mutates: true,
    params: ImportParams,
    run: run_import,
    description: "Imports a vector file from disk as a new object and selects it. Supported: .svg and .svgz, and the raster types .png, .jpg, .jpeg, .webp and .gif (by extension). The object name is the file name without extension. The object keeps an embedded copy and remembers the file, so after the file is changed in another app object.reload picks up the changes. SVG sizes come from the SVG in mm. A raster image is traced into cut outlines with the default trace settings (automatic threshold, dark pixels are cut) at the dpi stored in the file, or 300 dpi; use object.retrace to change the settings. Images over 16384 x 16384 px are refused. The object keeps its position from the SVG page if that lies inside the cutting area, otherwise its top-left corner moves to the top-left of the cutting area. Coordinates are mm, origin the 0,0 of the grid printed on the mat, y down. With paths instead of path, several files are imported as one undo step and all new objects are selected; the call never fails because of a file: a file that cannot be imported is listed in failed and the others are still imported, and when none could be imported nothing changes (no undo step); it returns {imported: [id], failed: [{path, reason}], warnings: [text, each starting with the file path]}. With path it returns {id, bounds_mm: {x0,y0,x1,y1}, warnings: [text], report: {shapes, issues: [issue]}}; with paths the report covers every imported object (each issue names its object_ids). The report lists what to know about the new objects (open paths, hairlines, shapes below the minimum feature size, fills the cut mode ignores, objects larger than the cutting area, missing fonts and more), each with a code, a severity, a plain message, a hint and fixes that are ready-to-run commands; see document.check for the format and the codes. An empty issues list means nothing to report. Raster imports also return dropped_small (shapes below the minimum feature size that were dropped), despeckled (specks below the despeckle area that were removed), threshold_used (0-255) and size_mm [width, height] of the whole image.",
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
    description: "Imports SVG markup given as text as a new object and selects it. Give the SVG a width and height in mm (for example width=\"50mm\") so the size is exact; plain numbers are CSS pixels (96 per inch). Every visible path, fill or stroke, is cut as set by object.set_cut_options (default: the outline of fills and of strokes 0.5 mm or wider); text is cut as outlines. Placement as in object.import. Coordinates are mm, origin the 0,0 of the grid printed on the mat, y down. Returns {id, bounds_mm: {x0,y0,x1,y1}, warnings: [text], report: {shapes, issues: [issue]}}: the import report, with codes, plain messages and fixes as ready-to-run commands (see document.check for the format and the codes).",
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
    check_exists(&s.doc().document, id)?;
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
    let found = Found::Trace(trace_findings(&t, invert));
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
    let members = s.doc().document.members(id);
    let report = report_of(s, &members, found.issues(id, &members));
    let mut out = json!({
        "id": id.0,
        "bounds_mm": bounds_json(s, id),
        "warnings": warnings,
        "report": report.to_json(),
    });
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
    description: "Traces a raster object again with new settings and keeps its position, scale, rotation and mirror. Settings (all optional, omitted ones use the defaults): threshold (0-255, null = automatic), invert (cut the light pixels), despeckle_mm2 (default 0.5), corner_threshold_deg (default 60), smoothing (0-10, default 5), min_feature_mm (null = 6 times the blade offset of the plotter), dpi_override (null = dpi of the file, else 300). Only raster objects can be retraced. Linked copies (see object.copies) share the trace settings, so all members of the group are retraced in the same undo step, each keeping its own position, rotation and mirror. Returns {id, bounds_mm, warnings, report: {shapes, issues} (the import report of the object and its linked copies, see document.check), dropped_small (shapes below the minimum feature size), despeckled (specks below the despeckle area), threshold_used, size_mm}.",
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
pub struct SetCutParams {
    /// Object ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// False skips the objects: they stay visible but are not cut.
    pub cut: bool,
}

fn run_set_cut(s: &mut Session, p: SetCutParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let doc = &mut s.doc_mut().document;
    for id in &ids {
        object_mut(doc, *id)?.cut_enabled = p.cut;
    }
    Ok(json!({ "ids": ids_json(&ids), "cut": p.cut }))
}

command!(
    ObjectSetCut,
    id: "object.set_cut",
    title: "Skip or cut",
    mutates: true,
    params: SetCutParams,
    run: run_set_cut,
    description: "Skips or restores cutting of objects. A skipped object (cut false) stays visible but is left out of the job, with a warning. Linked copies (see object.copies) each keep their own setting, and copies and duplicates start with the flag of the object they were made from. An empty ids list means the current selection. Returns {ids, cut}.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameParams {
    /// The object or group to rename.
    pub id: u64,
    /// The new name. Leading and trailing spaces are dropped.
    pub name: String,
}

/// The longest object name `object.rename` takes, in characters.
pub const MAX_NAME_CHARS: usize = 200;

fn run_rename(s: &mut Session, p: RenameParams, _: Origin) -> Result<Value, CommandError> {
    let name = p.name.trim().to_string();
    if name.is_empty() {
        return Err(CommandError::InvalidParams("name must not be empty".into()));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(CommandError::InvalidParams(format!(
            "name must be at most {MAX_NAME_CHARS} characters"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(CommandError::InvalidParams(
            "name must be one line of plain text".into(),
        ));
    }
    let id = ObjectId(p.id);
    check_node(&s.doc().document, id)?;
    let doc = &mut s.doc_mut().document;
    match doc.group_node_mut(id) {
        Some(g) => g.name = name.clone(),
        None => object_mut(doc, id)?.name = name.clone(),
    }
    Ok(json!({ "id": p.id, "name": name }))
}

command!(
    ObjectRename,
    id: "object.rename",
    title: "Rename",
    mutates: true,
    params: RenameParams,
    run: run_rename,
    description: "Renames one object or group. The name is one line of text without leading or trailing spaces and must not be empty and at most 200 characters. Renaming to the name the object has changes nothing and records no undo step. Linked copies (see object.copies) each keep their own name. Returns {id, name}.",
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
    /// Copies of each object, 1 (default) to 500 in all; copy k is moved by k times offset_mm.
    #[serde(default = "one")]
    pub count: u32,
}

fn one() -> u32 {
    1
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

/// Copies `unit` (an object, or a whole group with its nested groups)
/// with new ids at the end of the document; `place` then moves each copied
/// object. The copy sits where the original does in the group tree (in the
/// same group). Objects keep their groups of linked copies, as
/// [`add_copy`]. Returns the new unit's id.
pub(crate) fn copy_unit(
    doc: &mut Document,
    unit: ObjectId,
    place: &mut dyn FnMut(&mut Object),
) -> Result<ObjectId, CommandError> {
    if doc.group_node(unit).is_none() {
        let new_id = add_copy(doc, unit, Vec2::ZERO)?;
        place(object_mut(doc, new_id)?);
        return Ok(new_id);
    }
    let tree = doc.tree();
    let groups: Vec<GroupNode> = doc
        .groups
        .iter()
        .filter(|g| tree.within(g.id, unit))
        .cloned()
        .collect();
    let mut map = std::collections::BTreeMap::new();
    for g in &groups {
        map.insert(g.id, doc.take_id());
    }
    for g in &groups {
        doc.groups.push(GroupNode {
            id: map[&g.id],
            name: g.name.clone(),
            parent: if g.id == unit {
                g.parent
            } else {
                g.parent.map(|p| map.get(&p).copied().unwrap_or(p))
            },
            rotation_deg: g.rotation_deg,
        });
    }
    for leaf in tree.leaves_of(unit) {
        let new_id = add_copy(doc, leaf, Vec2::ZERO)?;
        let o = object_mut(doc, new_id)?;
        o.parent = o.parent.map(|p| map.get(&p).copied().unwrap_or(p));
        place(o);
    }
    Ok(map[&unit])
}

fn run_duplicate(s: &mut Session, p: DuplicateParams, _: Origin) -> Result<Value, CommandError> {
    let (_, units) = resolve_units(s, &p.ids)?;
    if p.count == 0 || u64::from(p.count) * units.len() as u64 > u64::from(MAX_COPIES) {
        return Err(CommandError::InvalidParams(format!(
            "count must be 1 or more, and {MAX_COPIES} copies in all at most"
        )));
    }
    if !p.offset_mm.iter().all(|v| v.is_finite()) {
        return Err(CommandError::InvalidParams(
            "offset_mm must be two numbers".into(),
        ));
    }
    let offset = Vec2::new(p.offset_mm[0], p.offset_mm[1]);
    // The window's ghost preview uses the same function.
    let offsets = placement::step_offsets(offset, p.count);
    let st = s.doc_mut();
    let mut new_ids = Vec::new();
    let mut selection = Vec::new();
    for (k, step) in offsets.iter().enumerate() {
        for unit in &units {
            let mut valid = true;
            let new_id = copy_unit(&mut st.document, *unit, &mut |o| {
                translate(o, *step);
                valid &= o.placement_is_valid();
            })?;
            if !valid {
                return Err(CommandError::Refused(format!(
                    "Copy {} would land more than 1 km from the mat. Make fewer copies or a smaller offset.",
                    k + 1
                )));
            }
            new_ids.push(new_id);
            selection.extend(unit_leaves(&st.document, new_id));
        }
    }
    st.selection = selection.into_iter().collect();
    Ok(json!({ "ids": ids_json(&new_ids) }))
}

command!(
    ObjectDuplicate,
    id: "object.duplicate",
    title: "Duplicate",
    mutates: true,
    params: DuplicateParams,
    run: run_duplicate,
    description: "Copies objects and moves the copies by offset_mm [dx, dy] (default [10, 10]; mm, origin the 0,0 of the grid printed on the mat, y down), in one undo step. count (default 1) makes that many copies of each object, copy k moved by k times offset_mm, 500 copies in all at most; if a copy would land more than 1 km away, it refuses the whole call and adds nothing. The copies become the selection. A group is copied whole, with its nested groups, as a new group (its id is returned); a copy of an object inside a group joins that group. A copy of a linked copy is a linked copy too: it joins the original's group and shares its source, geometry, trace settings, cut options, scale and file link (object.unlink_copies makes it independent). An empty ids list means the current selection. Returns {ids: [new id]}: the first copies of the originals in their order, then the second copies, and so on.",
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
    /// The object or group to copy.
    pub id: u64,
    /// How many copies to add, 1 to 500.
    pub count: u32,
    /// "row" (default) or {"grid": {"columns": n}}.
    #[serde(default)]
    pub layout: CopiesLayout,
    /// Gap between neighbouring bounding boxes in mm (default 5), 0 or more.
    #[serde(default = "default_gap")]
    pub gap_mm: f64,
    /// Gap between columns in mm, 0 or more; default gap_mm.
    pub gap_x_mm: Option<f64>,
    /// Gap between rows in mm, 0 or more; default gap_mm.
    pub gap_y_mm: Option<f64>,
    /// true makes linked copies: they share the source, the geometry, the trace settings, the cut options, the keep mode, the scale and the file link with the original; each keeps its own position, rotation and mirror.
    #[serde(default)]
    pub linked: bool,
}

impl CopiesLayout {
    /// The columns of a grid, `None` for a row.
    pub fn columns(self) -> Option<u32> {
        match self {
            CopiesLayout::Row => None,
            CopiesLayout::Grid { columns } => Some(columns),
        }
    }
}

fn run_copies(s: &mut Session, p: CopiesParams, _: Origin) -> Result<Value, CommandError> {
    if !(1..=MAX_COPIES).contains(&p.count) {
        return Err(CommandError::InvalidParams(format!(
            "count must be 1 to {MAX_COPIES}"
        )));
    }
    let gap_x = p.gap_x_mm.unwrap_or(p.gap_mm);
    let gap_y = p.gap_y_mm.unwrap_or(p.gap_mm);
    for (name, v) in [
        ("gap_mm", p.gap_mm),
        ("gap_x_mm", gap_x),
        ("gap_y_mm", gap_y),
    ] {
        if !(v.is_finite() && v >= 0.0) {
            return Err(CommandError::InvalidParams(format!(
                "{name} must be 0 or more"
            )));
        }
    }
    if p.layout == (CopiesLayout::Grid { columns: 0 }) {
        return Err(CommandError::InvalidParams(
            "a grid needs 1 or more columns".into(),
        ));
    }
    let unit = ObjectId(p.id);
    check_node(&s.doc().document, unit)?;
    let area = cutting_area(s).ok();
    let st = s.doc_mut();
    let leaves = unit_leaves(&st.document, unit);
    // Linked copies: each object links to its own original.
    let mut link_of = std::collections::BTreeMap::new();
    for leaf in &leaves {
        let group = p.linked.then(|| {
            st.document
                .get(*leaf)
                .and_then(|o| o.link_group)
                .unwrap_or_else(|| st.document.new_group_id())
        });
        if group.is_some() {
            object_mut(&mut st.document, *leaf)?.link_group = group;
        }
        link_of.insert(*leaf, group);
    }
    // Measured once: a copy's bounds are the original's, moved. Only what
    // is shown and cut counts for the warning.
    let doc = &st.document;
    let b = common_box(doc, &leaves).unwrap_or(Rect::ZERO);
    let measured: Vec<Rect> = leaves
        .iter()
        .filter_map(|id| doc.get(*id))
        .filter(|o| o.visible && o.cut_enabled)
        .filter_map(placed_bounds)
        .collect();
    // The window's ghost preview uses the same function.
    let offsets = placement::grid_offsets(b, p.layout.columns(), p.count, gap_x, gap_y);
    let mut new_ids = Vec::with_capacity(p.count as usize);
    let mut selection = Vec::new();
    let mut outside = Vec::new();
    for (k, offset) in (1..).zip(offsets) {
        let mut valid = true;
        let new_id = copy_unit(&mut st.document, unit, &mut |copy| {
            // Without linked, a copy is independent, even of a linked copy.
            copy.link_group = None;
            translate(copy, offset);
            valid &= copy.placement_is_valid();
        })?;
        if !valid {
            return Err(CommandError::Refused(format!(
                "Copy {k} would land more than 1 km from the mat. Make fewer copies or a smaller gap."
            )));
        }
        // The copies of the objects come in the order of `leaves`.
        let copied = unit_leaves(&st.document, new_id);
        for (leaf, copy) in leaves.iter().zip(&copied) {
            object_mut(&mut st.document, *copy)?.link_group = link_of[leaf];
        }
        if let Some(area) = area
            && measured.iter().any(|m| !inside(*m + offset, area))
        {
            outside.push(new_id);
        }
        new_ids.push(new_id);
        selection.extend(copied);
    }
    st.selection = selection.into_iter().collect();
    Ok(json!({
        "ids": ids_json(&new_ids),
        "outside_area": ids_json(&outside),
        "warnings": outside_warnings(outside.len()),
    }))
}

/// The warning about `n` copies outside the cutting area.
fn outside_warnings(n: usize) -> Vec<String> {
    match n {
        0 => Vec::new(),
        1 => vec![
            "1 copy is outside the cutting area. Move it, or pick a bigger mat, before you cut."
                .into(),
        ],
        n => vec![format!(
            "{n} copies are outside the cutting area. Move them, or pick a bigger mat, before you cut."
        )],
    }
}

command!(
    ObjectCopies,
    id: "object.copies",
    title: "Copies",
    mutates: true,
    params: CopiesParams,
    run: run_copies,
    description: "Adds count copies (1 to 500) of one object or group in one undo step (a group is copied whole, with its nested groups, and laid out by its common box), laid out next to it by its bounding box plus gap_mm (default 5); gap_x_mm and gap_y_mm set the gap between columns and between rows on their own (each defaults to gap_mm). layout \"row\" (default) puts them in a row to the right of the original. layout {\"grid\": {\"columns\": n}} fills rows of n cells left to right, then goes down; the original is the first cell, so copy k sits in column k mod n and row k div n. Units mm, origin the 0,0 of the grid printed on the mat, x right, y down. Copies that land outside the cutting area are still added; they are listed in outside_area and get a warning. If a copy would land more than 1 km away, it refuses the whole call and adds nothing. Copies keep the original's file link, so a reload of that file updates all of them. linked: true (default false) makes linked copies: they and the original share the source, the geometry, the trace settings, the cut options, the keep mode, the scale and the file link, stored once in the project, so a change of one of these on any of them (object.scale, object.set_cut_options, object.set_keep, object.retrace, object.reload, object.relink, object.unlink) changes all of them in the same undo step; each keeps its own position, rotation, mirror, name and visibility. Copies of a linked copy join its group. object.unlink_copies makes copies independent again. The copies become the selection. Returns {ids: [new id], outside_area: [id], warnings}.",
);

// ---------------------------------------------------------------- circular copies

fn default_total_deg() -> f64 {
    360.0
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CircularCopiesParams {
    /// Object ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// Instances around the circle, the original included: 2 or more (so count - 1 copies of each object, 500 copies in all at most).
    pub count: u32,
    /// The angle the instances spread over in degrees, clockwise on screen (default 360). Not 0, at most 360 either way.
    #[serde(default = "default_total_deg")]
    pub total_deg: f64,
    /// The centre of the circle [x, y] in mm. Default: the centre of the objects' common bounding box.
    pub center_mm: Option<[f64; 2]>,
    /// true (default): each copy also turns by its angle, like spokes. false: copies keep the original's rotation.
    #[serde(default = "default_true")]
    pub rotate_copies: bool,
    /// true makes linked copies, as in object.copies: they share the source, the geometry, the trace settings, the cut options, the keep mode, the scale and the file link with their original.
    #[serde(default)]
    pub linked: bool,
}

fn run_circular_copies(
    s: &mut Session,
    p: CircularCopiesParams,
    _: Origin,
) -> Result<Value, CommandError> {
    let (ids, units) = resolve_units(s, &p.ids)?;
    if p.count < 2 || u64::from(p.count - 1) * units.len() as u64 > u64::from(MAX_COPIES) {
        return Err(CommandError::InvalidParams(format!(
            "count must be 2 or more, and {MAX_COPIES} copies in all at most"
        )));
    }
    if !(p.total_deg.is_finite() && p.total_deg != 0.0 && p.total_deg.abs() <= 360.0) {
        return Err(CommandError::InvalidParams(
            "total_deg must not be 0 and at most 360 either way".into(),
        ));
    }
    let pivot = match p.center_mm {
        Some([x, y]) if x.is_finite() && y.is_finite() => Point::new(x, y),
        Some(_) => {
            return Err(CommandError::InvalidParams(
                "center_mm must be two numbers".into(),
            ));
        }
        None => {
            let doc = &s.doc().document;
            let boxes: Vec<Rect> = common_box(doc, &ids).into_iter().collect();
            placement::pattern_centre(&boxes).unwrap_or_default()
        }
    };
    let area = cutting_area(s).ok();
    let st = s.doc_mut();
    let mut new_ids = Vec::new();
    let mut selection = Vec::new();
    let mut outside = Vec::new();
    for unit in &units {
        let leaves = unit_leaves(&st.document, *unit);
        let mut link_of = std::collections::BTreeMap::new();
        for leaf in &leaves {
            let group = p.linked.then(|| {
                st.document
                    .get(*leaf)
                    .and_then(|o| o.link_group)
                    .unwrap_or_else(|| st.document.new_group_id())
            });
            if group.is_some() {
                object_mut(&mut st.document, *leaf)?.link_group = group;
            }
            link_of.insert(*leaf, group);
        }
        let doc = &st.document;
        let centre = unit_centre(doc, *unit);
        // Measured once; a copy that does not turn is the original, moved.
        let measured: Option<Vec<Rect>> = (!p.rotate_copies).then(|| {
            leaves
                .iter()
                .filter_map(|id| doc.get(*id))
                .filter(|o| o.visible && o.cut_enabled)
                .filter_map(placed_bounds)
                .collect()
        });
        // The window's ghost preview uses the same functions.
        let turned =
            placement::circular_copies(centre, pivot, p.count, p.total_deg, p.rotate_copies);
        for (k, t) in (1..).zip(turned) {
            let mut valid = true;
            let new_id = copy_unit(&mut st.document, *unit, &mut |copy| {
                copy.link_group = None;
                let c = copy.transform.center_mm;
                copy.transform.center_mm = placement::carried(c, centre, t);
                copy.transform.rotation_deg =
                    normalize_deg(copy.transform.rotation_deg + t.rotate_deg);
                valid &= copy.placement_is_valid();
            })?;
            if !valid {
                return Err(CommandError::Refused(format!(
                    "Copy {k} would land more than 1 km from the mat. Make fewer copies or pick a centre closer to the objects."
                )));
            }
            turn_groups(&mut st.document, new_id, t.rotate_deg);
            let copied = unit_leaves(&st.document, new_id);
            for (leaf, copy) in leaves.iter().zip(&copied) {
                object_mut(&mut st.document, *copy)?.link_group = link_of[leaf];
            }
            if let Some(area) = area {
                let offset = t.center - centre;
                let doc = &st.document;
                let off = match &measured {
                    Some(m) => m.iter().any(|b| !inside(*b + offset, area)),
                    None => copied
                        .iter()
                        .filter_map(|id| doc.get(*id))
                        .filter(|o| o.visible && o.cut_enabled)
                        .filter_map(placed_bounds)
                        .any(|b| !inside(b, area)),
                };
                if off {
                    outside.push(new_id);
                }
            }
            new_ids.push(new_id);
            selection.extend(copied);
        }
    }
    st.selection = selection.into_iter().collect();
    Ok(json!({
        "ids": ids_json(&new_ids),
        "center_mm": [pivot.x, pivot.y],
        "outside_area": ids_json(&outside),
        "warnings": outside_warnings(outside.len()),
    }))
}

/// The point a unit turns about on its own: an object's centre, a
/// group's box centre (of what is cut).
pub(crate) fn unit_centre(doc: &Document, unit: ObjectId) -> Point {
    match doc.get(unit) {
        Some(o) => o.transform.center_mm,
        None => common_box(doc, &unit_leaves(doc, unit))
            .map(|b| b.center())
            .unwrap_or_default(),
    }
}

/// Adds `degrees` to the own angle of group `unit` and of every group in
/// it (see [`GroupNode::rotation_deg`]). Nothing for an object.
pub(crate) fn turn_groups(doc: &mut Document, unit: ObjectId, degrees: f64) {
    if doc.group_node(unit).is_none() {
        return;
    }
    let tree = doc.tree();
    for g in &mut doc.groups {
        if tree.within(g.id, unit) {
            g.rotation_deg = normalize_deg(g.rotation_deg + degrees);
        }
    }
}

command!(
    ObjectCircularCopies,
    id: "object.circular_copies",
    title: "Circular pattern",
    mutates: true,
    params: CircularCopiesParams,
    run: run_circular_copies,
    description: "Copies objects around a circle in one undo step, like the numbers on a clock. count is the number of instances with the original (2 or more; 500 copies in all at most). total_deg (default 360, not 0, at most 360 either way) is the angle they spread over, clockwise on screen with y down: a full turn divides by count, so 4 over 360 puts copies at 90, 180 and 270; a smaller arc divides by count - 1, so the last copy sits at its end (3 over 180 puts copies at 90 and 180). center_mm [x, y] is the centre of the circle; the default is the centre of the objects' common bounding box. Units mm, origin the 0,0 of the grid printed on the mat, x right, y down. Each object's centre turns about the centre of the circle; a whole group is copied as a new group and turns as one piece about the centre of its box. rotate_copies (default true) also turns each copy by its angle; false keeps the original's rotation. linked (default false) makes linked copies with the same rules as object.copies: a copy shares the source, the geometry, the trace settings, the cut options, the keep mode, the scale and the file link with its original, and keeps its own position, rotation, mirror, name and visibility; copies of a linked copy join its group. Without linked the copies are independent. Copies outside the cutting area are still added; they are listed in outside_area and get a warning. If a copy would land more than 1 km away, it refuses the whole call and adds nothing. An empty ids list means the current selection. The copies become the selection. Returns {ids: [new id], center_mm: [x, y], outside_area: [id], warnings}.",
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
    description: "Makes objects linked copies of the first one in ids, in one undo step. They must have the same source (the same SVG or image content, for example copies or two imports of one file); otherwise it refuses and changes nothing. Every object takes the first one's shared fields: the source with its trace settings, the geometry, the cut options, the keep mode, the scale and the file link. Each keeps its own position, rotation, mirror, name and visibility; new geometry stays anchored to the object's page frame and a new scale keeps the object's own centre (mm, origin the 0,0 of the grid printed on the mat, y down). Objects that are linked copies already bring their whole group along. From then on a change of a shared field on one member (object.scale, object.set_cut_options, object.set_keep, object.retrace, object.reload, object.relink, object.unlink) changes all of them. Needs 2 or more objects. An empty ids list means the current selection. Returns {group, ids: [every member]}.",
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
    /// Absolute target [x, y] in mm for the centre of the objects' common bounding box. Do not combine with dx_mm/dy_mm or x_mm/y_mm.
    pub center_mm: Option<[f64; 2]>,
    /// Absolute target in mm for the left edge of the objects' common bounding box (of the placed, rotated shapes). A missing y_mm keeps the box's top where it is. Do not combine with dx_mm/dy_mm or center_mm.
    pub x_mm: Option<f64>,
    /// Absolute target in mm for the top edge of the objects' common bounding box. A missing x_mm keeps the box's left edge where it is.
    pub y_mm: Option<f64>,
    /// Merges this move into the undo step of the move just before it when that one had the same key and the same ids and nothing else changed the document in between. For a burst of small moves (arrow-key nudges) that the person undoes as one.
    pub coalesce_key: Option<String>,
}

fn run_move(s: &mut Session, p: MoveParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let relative = p.dx_mm.is_some() || p.dy_mm.is_some();
    let corner = p.x_mm.is_some() || p.y_mm.is_some();
    // The box the window shows: hidden objects do not count.
    let common = |s: &Session| common_box(&s.doc().document, &ids).unwrap_or(Rect::ZERO);
    let delta = match (relative, p.center_mm, corner) {
        (false, None, false) => {
            return Err(CommandError::InvalidParams(
                "give dx_mm/dy_mm for a relative move, or center_mm or x_mm/y_mm for an absolute one"
                    .into(),
            ));
        }
        (true, None, false) => Vec2::new(p.dx_mm.unwrap_or(0.0), p.dy_mm.unwrap_or(0.0)),
        (false, Some([x, y]), false) => Point::new(x, y) - common(s).center(),
        (false, None, true) => {
            let b = common(s);
            Vec2::new(
                p.x_mm.map_or(0.0, |x| x - b.x0),
                p.y_mm.map_or(0.0, |y| y - b.y0),
            )
        }
        _ => {
            return Err(CommandError::InvalidParams(
                "give only one of dx_mm/dy_mm, center_mm and x_mm/y_mm".into(),
            ));
        }
    };
    if !(delta.x.is_finite() && delta.y.is_finite()) {
        return Err(CommandError::InvalidParams(
            "the target must be a finite number".into(),
        ));
    }
    let doc = &mut s.doc_mut().document;
    for id in &ids {
        translate(object_mut(doc, *id)?, delta);
    }
    if let Some(key) = p.coalesce_key {
        s.coalesce_with(key, ids.clone());
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
    description: "Moves objects. Either relative with dx_mm and dy_mm (a missing one is 0), or absolute: center_mm [x, y] puts the centre of the objects' common bounding box there, and x_mm and/or y_mm put its top-left corner there (the box of the placed, rotated shapes, as the window's X and Y fields show it; a missing one keeps that edge). Several objects move as one, so their layout stays. Units mm, origin the 0,0 of the grid printed on the mat, x right, y down. Each move is one undo step, except with coalesce_key: a move with the same key and the same ids as the move just before it, with no other change of the document in between, joins that move's undo step (the window's arrow-key nudges do this). A burst that ends where it started leaves no undo step. An empty ids list means the current selection. Returns {ids, moved_by_mm: [dx, dy]}.",
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
    /// The middle of the top edge.
    Top,
    /// The middle of the bottom edge.
    Bottom,
    /// The middle of the left edge.
    Left,
    /// The middle of the right edge.
    Right,
}

impl Anchor {
    fn point(self, r: Rect) -> Point {
        match self {
            Anchor::Center => r.center(),
            Anchor::TopLeft => Point::new(r.x0, r.y0),
            Anchor::TopRight => Point::new(r.x1, r.y0),
            Anchor::BottomLeft => Point::new(r.x0, r.y1),
            Anchor::BottomRight => Point::new(r.x1, r.y1),
            Anchor::Top => Point::new(r.center().x, r.y0),
            Anchor::Bottom => Point::new(r.center().x, r.y1),
            Anchor::Left => Point::new(r.x0, r.center().y),
            Anchor::Right => Point::new(r.x1, r.center().y),
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
    /// Scale the objects as one group (default false): the common bounding box of all of them scales about its anchor point, so their layout stays. width_mm and height_mm then set the size of that common box; with keep_aspect false they change apart, which needs every object turned by a multiple of 90 degrees.
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
    // Objects whose shared scale is set already in this command, through
    // a linked copy scaled earlier: one list for the whole command, so a
    // set of linked copies scales once, whether its members sit in groups
    // or not.
    let mut scaled: Vec<ObjectId> = Vec::new();
    if p.group {
        scale_group(s, &ids, factor, width, height, &p, &mut scaled, None)?;
        return Ok(json!({ "ids": ids_json(&ids) }));
    }
    // Each unit on its own: a whole group scales as one piece about its
    // own box, an object about its own.
    let units = units_of(&s.doc().document, &ids);
    let (groups, objects): (Vec<ObjectId>, Vec<ObjectId>) = units
        .iter()
        .partition(|u| s.doc().document.group_node(**u).is_some());
    // Measured first: a linked copy may scale before its turn comes (in a
    // group, or as an object earlier in `ids`).
    let mut olds = Vec::with_capacity(objects.len());
    for id in &objects {
        let doc = &mut s.doc_mut().document;
        olds.push(placed_bounds(object_mut(doc, *id)?).ok_or_else(|| {
            CommandError::InvalidParams(format!("object {} has no geometry to scale", id.0))
        })?);
    }
    // Every group's box too, before anything scales: a linked member
    // scaled with an earlier group must not move a later group's anchor.
    let mut group_olds = Vec::with_capacity(groups.len());
    for g in &groups {
        let doc = &s.doc().document;
        let leaves = unit_leaves(doc, *g);
        group_olds.push((leaves.clone(), scale_box(doc, &leaves)?));
    }
    for (leaves, old) in group_olds {
        scale_group(
            s,
            &leaves,
            factor,
            width,
            height,
            &p,
            &mut scaled,
            Some(old),
        )?;
    }
    let doc = &mut s.doc_mut().document;
    for (id, &old) in objects.iter().zip(&olds) {
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
            let Some(odd) = odd_quarter(o.transform.rotation_deg) else {
                return Err(CommandError::InvalidParams(format!(
                    "object {} is rotated by {} degrees; width and height can only be set separately at multiples of 90 degrees. Use keep_aspect or a factor",
                    id.0, o.transform.rotation_deg
                )));
            };
            if odd {
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

/// Whether a rotation is an odd multiple of 90 degrees (the local axes
/// swap on screen), or `None` when it is no multiple of 90 degrees.
fn odd_quarter(deg: f64) -> Option<bool> {
    let quarter = deg / 90.0;
    let k = quarter.round();
    ((quarter - k).abs() <= 1e-9).then_some((k as i64).rem_euclid(2) == 1)
}

/// The box `object.scale` scales a set of objects by: what is cut of the
/// shown ones (see `box_members`). Every id must exist and have geometry.
fn scale_box(doc: &Document, ids: &[ObjectId]) -> Result<Rect, CommandError> {
    for id in ids {
        check_exists(doc, *id)?;
    }
    let mut out: Option<Rect> = None;
    // Hidden objects are not part of the box (see `box_members`).
    for o in super::box_members(doc, ids) {
        let id = o.id;
        let b = placed_bounds(o).ok_or_else(|| {
            CommandError::InvalidParams(format!("object {} has no geometry to scale", id.0))
        })?;
        out = Some(out.map_or(b, |r| r.union(b)));
    }
    out.ok_or_else(|| CommandError::InvalidParams("no objects to scale".into()))
}

/// `object.scale` with `group`: the common bounding box of all objects
/// scales about its anchor, by one factor, or by a factor along x and one
/// along y when every object is straight. `measured` is the box measured
/// before the command changed anything (several groups scale in one
/// command), else it is measured now.
#[allow(clippy::too_many_arguments)]
fn scale_group(
    s: &mut Session,
    ids: &[ObjectId],
    factor: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    p: &ScaleParams,
    scaled: &mut Vec<ObjectId>,
    measured: Option<Rect>,
) -> Result<(), CommandError> {
    let doc = &mut s.doc_mut().document;
    let group_box = |doc: &Document| scale_box(doc, ids);
    let old = match measured {
        Some(b) => b,
        None => group_box(doc)?,
    };
    let ratio = |target: Option<f64>, have: f64, what: &str| match target {
        None => Ok(None),
        Some(_) if have <= 1e-12 => Err(CommandError::InvalidParams(format!(
            "the group has no {what}, so its {what} cannot be set"
        ))),
        Some(t) => Ok(Some(t / have)),
    };
    let (fx, fy) = match factor {
        Some(f) => (f, f),
        None => {
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
        }
    };
    // Width and height apart: every object must be straight, and linked
    // copies in `ids` (which share one scale) turned the same way.
    let mut odd = Vec::with_capacity(ids.len());
    for id in ids {
        let o = object_mut(doc, *id)?;
        let q = odd_quarter(o.transform.rotation_deg);
        if fx != fy {
            let Some(q) = q else {
                return Err(CommandError::InvalidParams(format!(
                    "object {} is turned by {} degrees; a group can change width and height apart only when every object is turned by a multiple of 90 degrees. Use keep_aspect or a factor",
                    id.0, o.transform.rotation_deg
                )));
            };
            let group = o.link_group;
            if group.is_some()
                && ids.iter().zip(&odd).any(|(other, oq)| {
                    *oq != Some(q) && doc.get(*other).is_some_and(|m| m.link_group == group)
                })
            {
                return Err(CommandError::InvalidParams(format!(
                    "object {} is a linked copy turned another way than a copy of it in ids; they share one scale, so width and height cannot change apart. Use keep_aspect or a factor",
                    id.0
                )));
            }
        }
        odd.push(q);
    }
    let anchor = p.anchor.point(old);
    for (id, q) in ids.iter().zip(&odd) {
        let already = scaled.contains(id);
        let o = object_mut(doc, *id)?;
        // A linked copy scaled earlier in this command keeps that scale
        // and only moves with the group.
        if !already {
            let (sx, sy) = if *q == Some(true) { (fy, fx) } else { (fx, fy) };
            o.transform.scale_x *= sx;
            o.transform.scale_y *= sy;
        }
        let c = o.transform.center_mm - anchor;
        o.transform.center_mm = anchor + Vec2::new(c.x * fx, c.y * fy);
    }
    // Linked copies outside `ids` take the scale and keep their centre.
    // Members inside scaled once each from the same scale, so they agree.
    for id in ids {
        if !scaled.contains(id) {
            doc.spread_shared(*id);
        }
    }
    for id in ids {
        for m in doc.members(*id) {
            if !scaled.contains(&m) {
                scaled.push(m);
            }
        }
    }
    // Cut outlines may not scale exactly (a stroke crossing the auto
    // threshold): put the anchor back where it was.
    let d = anchor - p.anchor.point(group_box(doc)?);
    for id in ids {
        translate(object_mut(doc, *id)?, d);
    }
    Ok(())
}

command!(
    ObjectScale,
    id: "object.scale",
    title: "Scale",
    mutates: true,
    params: ScaleParams,
    run: run_scale,
    description: "Scales each object on its own (one undo step for all; a whole group counts as one object and scales as one piece about its own box), or with group true all of them as one group about one anchor of their common bounding box, so their layout stays (the canvas does this for a multi-selection). Either a uniform factor, or width_mm and/or height_mm for the placed bounding box in mm. keep_aspect (default true) scales uniformly: with one size given it follows that size, with both it fits inside them. With keep_aspect false only the given sides change; this needs a rotation that is a multiple of 90 degrees (with group, for every object, and linked copies in ids turned the same way). anchor (center, top_left, top_right, bottom_left, bottom_right, or the middle of one edge: top, bottom, left, right; default center) is the point of each bounding box that stays fixed. Origin the 0,0 of the grid printed on the mat, y down. With strokes on auto (see object.set_cut_options), scaling can move a stroke across the 0.5 mm threshold and change how it is cut. Linked copies (see object.copies) share the scale: scaling one scales all members of its group in the same undo step, and each other member keeps its own centre, rotation and mirror; a group with several members in ids scales once, by its first member there. An empty ids list means the current selection. Returns {ids}.",
);

// ---------------------------------------------------------------- rotate

/// What a rotate or mirror of several objects turns about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum About {
    /// The objects as one group, about the mean of their centres (default).
    #[default]
    Group,
    /// Each object about its own centre.
    Each,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RotateParams {
    /// Object ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// Angle in degrees, clockwise on screen, added to the current rotation. At most 1e9 either way. Give this or angle_deg.
    #[serde(default)]
    pub degrees: Option<f64>,
    /// Absolute angle in degrees, clockwise on screen: every object gets this rotation, each about its own centre. Full turns are removed, so 735 is 15 and -90 is 270. At most 1e9 either way. Give this or degrees.
    #[serde(default)]
    pub angle_deg: Option<f64>,
    /// group (default): turn the objects as one group about their common centre, the mean of their centres. each: turn each about its own centre. One object turns about its own centre either way. With angle_deg only each is allowed.
    #[serde(default)]
    pub about: Option<About>,
}

/// The centres of the objects `ids` that exist, in order.
fn centres(doc: &Document, ids: &[ObjectId]) -> Vec<Point> {
    ids.iter()
        .filter_map(|id| doc.get(*id))
        .map(|o| o.transform.center_mm)
        .collect()
}

/// The largest angle `object.rotate` takes, either way. Past it an f64
/// no longer holds the angle exactly enough to reduce it by full turns.
pub const MAX_ANGLE_DEG: f64 = 1e9;

/// Refuses an angle that is not a number or too large to be exact. Any
/// other angle is fine: it is reduced by full turns into [0, 360).
pub fn check_angle(deg: f64) -> Result<(), CommandError> {
    if !deg.is_finite() {
        return Err(CommandError::InvalidParams(
            "the angle must be a number of degrees".into(),
        ));
    }
    if deg.abs() > MAX_ANGLE_DEG {
        return Err(CommandError::InvalidParams(format!(
            "the angle {deg} is too large: give at most 1000000000 degrees either way (larger numbers are not exact); full turns of 360 count as 0"
        )));
    }
    Ok(())
}

fn run_rotate(s: &mut Session, p: RotateParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let about = p.about.unwrap_or_default();
    // A relative turn, or an absolute angle that each object gets about
    // its own centre.
    let (degrees, absolute) = match (p.degrees, p.angle_deg) {
        (Some(d), None) => (d, false),
        (None, Some(a)) if about == About::Each || p.about.is_none() => (a, true),
        (None, Some(_)) => {
            return Err(CommandError::InvalidParams(
                "angle_deg turns each object about its own centre; leave out about or use about: each"
                    .into(),
            ));
        }
        _ => {
            return Err(CommandError::InvalidParams(
                "give degrees for a relative turn or angle_deg for an absolute angle, not both"
                    .into(),
            ));
        }
    };
    check_angle(degrees)?;
    let units = units_of(&s.doc().document, &ids);
    let doc = &mut s.doc_mut().document;
    // The turn of each unit: relative as given; for an absolute angle an
    // object gets it about its own centre, and a group turns as one piece
    // until its own angle is the given one.
    let turns: Vec<f64> = units
        .iter()
        .map(|u| match (absolute, doc.group_node(*u)) {
            (false, _) => degrees,
            (true, Some(g)) => normalize_deg(degrees) - g.rotation_deg,
            (true, None) => 0.0,
        })
        .collect();
    // The window's ghost preview uses the same function.
    let turn_units: Vec<placement::TurnUnit> = units
        .iter()
        .map(|u| placement::TurnUnit {
            centre: unit_centre(doc, *u),
            members: centres(doc, &unit_leaves(doc, *u)),
        })
        .collect();
    let mut out = Vec::new();
    for (k, unit) in units.iter().enumerate() {
        let deg = turns[k];
        let moved = if absolute {
            // Each unit on its own, by its own turn.
            placement::rotated_units(&turn_units[k..=k], deg, true).remove(0)
        } else {
            placement::rotated_units(&turn_units, deg, about == About::Each).remove(k)
        };
        for (id, centre) in unit_leaves(doc, *unit).into_iter().zip(moved) {
            let t = &mut object_mut(doc, id)?.transform;
            t.center_mm = centre;
            t.rotation_deg = if absolute && *unit == id {
                normalize_deg(degrees)
            } else {
                normalize_deg(t.rotation_deg + deg)
            };
            out.push(json!({
                "id": id.0,
                "rotation_deg": t.rotation_deg,
                "center_mm": [t.center_mm.x, t.center_mm.y],
            }));
        }
        turn_groups(doc, *unit, deg);
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
    description: "Rotates objects in one undo step: by degrees, relative to their current rotation, or to angle_deg, an absolute angle every object gets about its own centre (the window's Angle field does this; give one of the two). Positive is clockwise on screen (y points down). Any angle up to 1e9 either way is fine: full turns are removed, so the stored rotation is in [0, 360) (735 is 15, -90 is 270). about (only with degrees): group | each, default group. With group, several objects turn as one group about their common centre, the mean of the selected objects' centres: each centre turns about that point and each object's own rotation grows by the angle, so the layout turns with them (the window does this for a multi-selection). With each, every object turns about its own centre and the centres stay. One object turns about its own centre either way. A whole group counts as one object: it turns as one piece about the centre of its box, and with angle_deg it turns until its own angle (0 when it was made, see document.state groups) is angle_deg. Objects picked inside a group turn on their own. Positions in mm, origin the 0,0 of the grid printed on the mat. Linked copies (see object.copies) keep their own rotation: only the objects in ids turn. The stored rotation is kept in [0, 360). An empty ids list means the current selection. Returns {objects: [{id, rotation_deg, center_mm: [x, y]}]}.",
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
    /// group (default): mirror the objects as one group across the middle of their combined box, which stays in place. each: mirror each across its own centre.
    #[serde(default)]
    pub about: About,
    /// Leave out to flip. true or false sets the result instead: only the objects whose mirror state differs flip, so running it again changes nothing.
    pub mirrored: Option<bool>,
    /// Only with mirrored: objects that flip with the group whenever any object of ids flips, whatever their own mirror state (a symmetric shape such as the test shape, which must keep its place in the layout). Nothing flips when no object of ids needs to.
    #[serde(default)]
    pub follow: Vec<u64>,
}

fn run_mirror(s: &mut Session, p: MirrorParams, _: Origin) -> Result<Value, CommandError> {
    let mut ids = resolve_ids(s, &p.ids)?;
    if !p.follow.is_empty() && p.mirrored.is_none() {
        return Err(CommandError::InvalidParams(
            "follow needs mirrored (true or false)".into(),
        ));
    }
    // follow is checked like ids, whatever happens next.
    super::expand_ids(&s.doc().document, &p.follow)?;
    if let Some(want) = p.mirrored {
        let doc = &s.doc().document;
        // A whole group flips as one when any member needs to, so its
        // layout stays; loose objects and members picked inside a group
        // flip on their own.
        let flips = |id: &ObjectId| doc.get(*id).is_some_and(|o| o.transform.mirror != want);
        let mut keep = Vec::new();
        for unit in units_of(doc, &ids) {
            let leaves = unit_leaves(doc, unit);
            if leaves.iter().any(flips) {
                keep.extend(if doc.group_node(unit).is_some() {
                    leaves
                } else {
                    leaves.into_iter().filter(flips).collect()
                });
            }
        }
        ids = keep;
        // An empty follow list means none (not the selection).
        if !ids.is_empty() && !p.follow.is_empty() {
            for id in resolve_ids(s, &p.follow)? {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
    }
    let units = units_of(&s.doc().document, &ids);
    let doc = &mut s.doc_mut().document;
    // About group: everything as one piece. About each: each unit on its
    // own; a whole group is one unit and flips across its own box.
    let sets: Vec<(Vec<ObjectId>, bool)> = if p.about == About::Group {
        vec![(ids.clone(), true)]
    } else {
        units
            .iter()
            .map(|u| (unit_leaves(doc, *u), doc.group_node(*u).is_some()))
            .collect()
    };
    for (set, keep_box) in &sets {
        mirror_set(doc, set, p.axis, *keep_box)?;
    }
    // A flipped group's own angle flips as an object's does.
    let tree = doc.tree();
    for g in &mut doc.groups {
        if units.iter().any(|u| tree.within(g.id, *u)) {
            g.rotation_deg = flipped_deg(g.rotation_deg, p.axis);
        }
    }
    Ok(json!({ "ids": ids_json(&ids) }))
}

/// The angle of a shape turned by `deg` after a mirror along `axis`.
fn flipped_deg(deg: f64, axis: MirrorAxis) -> f64 {
    normalize_deg(match axis {
        MirrorAxis::Horizontal => -deg,
        MirrorAxis::Vertical => 180.0 - deg,
    })
}

/// Mirrors the objects `ids` along `axis`. Several mirror as one piece
/// across the middle of their combined box (of what is cut); one mirrors
/// across its own centre. With `keep_box` the combined box then stays
/// where it was: the design keeps its place on the mat.
fn mirror_set(
    doc: &mut Document,
    ids: &[ObjectId],
    axis: MirrorAxis,
    keep_box: bool,
) -> Result<(), CommandError> {
    let group_box = |doc: &Document| common_box(doc, ids);
    let before = keep_box.then(|| group_box(doc)).flatten();
    let centre = before.filter(|_| ids.len() >= 2).map(|b| b.center());
    for id in ids {
        let t = &mut object_mut(doc, *id)?.transform;
        if let Some(c) = centre {
            match axis {
                MirrorAxis::Horizontal => t.center_mm.x = 2.0 * c.x - t.center_mm.x,
                MirrorAxis::Vertical => t.center_mm.y = 2.0 * c.y - t.center_mm.y,
            }
        }
        t.mirror = !t.mirror;
        t.rotation_deg = flipped_deg(t.rotation_deg, axis);
    }
    // An object's cut box need not be centred on its centre: move the
    // flipped group back so its box is where it was.
    if let (Some(b), Some(a)) = (before, group_box(doc)) {
        let d = match axis {
            MirrorAxis::Horizontal => kurbo::Vec2::new(b.center().x - a.center().x, 0.0),
            MirrorAxis::Vertical => kurbo::Vec2::new(0.0, b.center().y - a.center().y),
        };
        for id in ids {
            translate(object_mut(doc, *id)?, d);
        }
    }
    Ok(())
}

command!(
    ObjectMirror,
    id: "object.mirror",
    title: "Mirror",
    mutates: true,
    params: MirrorParams,
    run: run_mirror,
    description: "Mirrors objects along a screen axis, in one undo step. horizontal flips left and right, vertical flips top and bottom. about: group | each, default group. With group, the objects mirror as one group across the middle of their combined box (of what is cut), and that box stays where it was: the layout flips with them and the design keeps its place on the mat (the window does this for a selection). With each, every object mirrors across its own centre and the centres stay; a whole group counts as one object and mirrors across the middle of its own box, which stays where it was. Positions in mm, origin the 0,0 of the grid printed on the mat, y down. Linked copies (see object.copies) keep their own mirror: only the objects in ids flip. Use horizontal for iron-on vinyl, which is cut from the back. mirrored: true or false makes it absolute: only objects whose mirror state differs flip (a whole group flips as one piece when any of its members differs) (as one group with about group), so a second run changes nothing; leave it out to flip every object. follow (only with mirrored) lists objects that flip with the group whenever any of ids flips, whatever their own state. An empty ids list means the current selection. Returns {ids} (the objects that flipped).",
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

// ---------------------------------------------------------------- keep

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KeepParam {
    /// Keep the cut shapes (a sticker): the outside and the holes are waste.
    Pieces,
    /// Keep the sheet (a stencil): the cut shapes fall out and are waste.
    Stencil,
    /// Keep both sides: the shapes and the sheet around them.
    Both,
}

impl From<KeepParam> for KeepMode {
    fn from(k: KeepParam) -> Self {
        match k {
            KeepParam::Pieces => KeepMode::Pieces,
            KeepParam::Stencil => KeepMode::Stencil,
            KeepParam::Both => KeepMode::Both,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetKeepParams {
    /// Object or group ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// What is kept after cutting: pieces, stencil or both.
    pub keep: KeepParam,
}

fn run_set_keep(s: &mut Session, p: SetKeepParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    set_keep(s, &ids, p.keep.into())
}

/// Sets `keep` on the objects `ids` and their linked copies, for
/// `object.set_keep` and `object.keep_at`.
fn set_keep(s: &mut Session, ids: &[ObjectId], keep: KeepMode) -> Result<Value, CommandError> {
    let doc = &mut s.doc_mut().document;
    let mut out = Vec::new();
    for id in ids {
        if out.contains(&id.0) {
            continue;
        }
        object_mut(doc, *id)?.keep = keep;
        // Linked copies share the keep mode.
        doc.spread_shared(*id);
        out.extend(doc.members(*id).into_iter().map(|m| m.0));
    }
    // What the new mode means for these objects: the stencil hints.
    let changed: Vec<ObjectId> = out.iter().map(|i| ObjectId(*i)).collect();
    let mut report = report_of(s, &changed, Vec::new());
    report
        .issues
        .retain(|i| i.code == report::Code::StencilIsland);
    Ok(json!({ "ids": out, "keep": keep.key(), "report": report.to_json() }))
}

command!(
    ObjectSetKeep,
    id: "object.set_keep",
    title: "Keep",
    mutates: true,
    params: SetKeepParams,
    run: run_set_keep,
    description: "Sets what is kept after cutting each object, in one undo step. The plan keeps the blade's lead-ins and corner overshoots in the waste and never in what is kept, and weed lines and the weed box cut only waste. keep: pieces (default) keeps the cut shapes, like a sticker: the outside and the holes are waste. stencil keeps the sheet, like a stencil: the cut shapes fall out and are waste, the rest of the cutting area is kept, so weeding cuts nothing outside them. both keeps both sides of its cuts: its shapes, the middle of a letter like O and the sheet around it all stay, so its own cuts only turn the blade in place and no lead-in, overshoot or weed line lands on that sheet. A stencil with a cut outer frame is pieces (the framed sheet is the piece). Across objects: a piece of a pieces or both object is kept; all stencil and both objects share one kept sheet, which each stencil's shapes cut out of, and a hole of a pieces object is cut free of that sheet, so it stays waste. A pieces object on that sheet keeps both sides of its outline, as a sticker on a stencil does. In stencil mode the middle of an O stays on the sheet: on a backing sheet the transfer tape lifts it with the stencil; on a material without backing (paper, cardstock) it falls out, and the report says so (stencil_island, info). A group id sets every object in the group. Linked copies (see object.copies) share their keep mode, so setting it on one sets it on all members of its group. An empty ids list means the current selection. Returns {ids: [every changed object, linked copies included], keep, report: {shapes, issues}} where issues holds only the stencil_island hints of these objects (see document.check).",
);

// ---------------------------------------------------------------- keep at

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MatPoint {
    pub x_mm: f64,
    pub y_mm: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeepAtParams {
    /// Object or group ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// Points on the mat that must stay after cutting, in mm (origin the 0,0 of the grid printed on the mat, y down): inside the objects' shape, or on the sheet around them.
    pub points: Vec<MatPoint>,
}

/// The name of a region in `object.keep_at`'s answer.
fn region_key(r: flatcraft_path::Region) -> &'static str {
    match r {
        flatcraft_path::Region::Shape => "shape",
        flatcraft_path::Region::Sheet => "sheet",
    }
}

fn run_keep_at(s: &mut Session, p: KeepAtParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    if p.points.is_empty() {
        return Err(CommandError::InvalidParams(
            "give at least one point that must stay".into(),
        ));
    }
    let doc = &s.doc().document;
    let cut = |id: &ObjectId| doc.get(*id).is_some_and(|o| o.visible && o.cut_enabled);
    if !ids.iter().any(cut) {
        return Err(CommandError::Failed(
            "these objects are hidden or skipped, so nothing of them is cut: show them or cut them again first".into(),
        ));
    }
    // The window's picker reads the same plan: its rings and its sheet.
    let job = crate::job::build(s).map_err(|e| {
        CommandError::Failed(format!(
            "the job cannot be planned, so the shape and the sheet cannot be told apart: {e}"
        ))
    })?;
    let kept = &job.plan.kept;
    if ids.iter().all(|id| kept.mode_of(*id).is_none()) {
        return Err(CommandError::Failed(
            "these objects have no closed cut to keep a side of, only open lines".into(),
        ));
    }
    let (mut shape, mut sheet) = (false, false);
    let mut regions = Vec::new();
    for (k, q) in p.points.iter().enumerate() {
        let at = Point::new(q.x_mm, q.y_mm);
        let Some(hit) = kept.region_at(at, |o| ids.contains(&o)) else {
            return Err(CommandError::InvalidParams(format!(
                "point {k} ({}, {}) lies on a cut line or outside the cutting area; give a point clearly inside the shape or on the sheet around it",
                q.x_mm, q.y_mm
            )));
        };
        match hit.region {
            flatcraft_path::Region::Shape => shape = true,
            flatcraft_path::Region::Sheet => sheet = true,
        }
        regions.push(region_key(hit.region));
    }
    let keep = flatcraft_path::keep_for(shape, sheet).expect("every point marks a region");
    let mut out = set_keep(s, &ids, keep)?;
    out["regions"] = json!(regions);
    Ok(out)
}

command!(
    ObjectKeepAt,
    id: "object.keep_at",
    title: "Keep at points",
    mutates: true,
    params: KeepAtParams,
    run: run_keep_at,
    description: "Sets the keep mode of objects by the points on the mat that must stay, as the window's \"Pick on the mat\" helper does (\"keep the letters\" is a point inside a letter). Each point lies in the objects' shape (inside their cut outline, by the even-odd rule of the plan: the middle of an O is not in the shape) or on the sheet around them. Shape only gives pieces, sheet only stencil, shape and sheet both; see object.set_keep for what each keeps. Points are mm, origin the 0,0 of the grid printed on the mat, x right, y down. A point on a cut line or outside the cutting area is refused, and so are objects that are all hidden, skipped or open lines only. It needs a job that can be planned (see job.preview). One undo step; nothing changes when the mode is already set. A group id sets every object in the group, and linked copies (see object.copies) share the keep mode. An empty ids list means the current selection. Returns {ids: [every changed object, linked copies included], keep, regions: [shape|sheet, one per point], report: {shapes, issues}} where issues holds only the stencil_island hints of these objects.",
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
