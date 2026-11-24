//! Document and project file commands.

use std::path::{Component, Path, PathBuf};

use flatcraft_core::document::{CutOptions, Object, Source};
use flatcraft_core::project::{self, ProjectError};
use flatcraft_device::MediaKind;
use flatcraft_import::fonts;
use flatcraft_import::svg::import_svg;
use kurbo::Rect;
use serde::Deserialize;
use serde_json::{Value, json};

use super::object::{blade_offset_for, warning_text};
use super::sources::{project_dir, relink_for_save, restore_links};
use super::{NoParams, command, cutting_area, ids_json, placed_bounds, point_arr, rect_json};
use crate::session::{CommandError, DocState, Origin, Session};

// ---------------------------------------------------------------- document.state

fn object_json(o: &Object) -> Value {
    let kind = match o.source {
        Source::Svg { .. } => "svg",
        Source::Raster { .. } => "raster",
    };
    let t = &o.transform;
    json!({
        "id": o.id.0,
        "name": o.name,
        "kind": kind,
        "visible": o.visible,
        "bounds_mm": placed_bounds(o).map(rect_json).unwrap_or(Value::Null),
        "transform": {
            "center_mm": point_arr(t.center_mm),
            "scale_x": t.scale_x,
            "scale_y": t.scale_y,
            "rotation_deg": t.rotation_deg,
            "mirror": t.mirror,
        },
        "cut": super::object::cut_json(&o.cut),
        "link": o.link.as_ref().map(|l| json!({ "path": l.path, "relative": l.relative })),
    })
}

/// The active mat of the document, or null when the plotter no longer lists it.
fn mat_json(s: &Session) -> Value {
    let job = &s.doc().document.job;
    let Ok(plotter) = s.profiles.plotter(&job.plotter_id) else {
        return Value::Null;
    };
    let Some(m) = plotter.media.iter().find(|m| m.id == job.media_id) else {
        return Value::Null;
    };
    json!({
        "id": m.id,
        "name": m.name,
        "kind": match m.kind {
            MediaKind::Mat => "mat",
            MediaKind::NoMat => "no_mat",
        },
        "width_mm": m.width_mm,
        "height_mm": m.height_mm,
        "grid": {
            "origin_x_mm": m.grid.origin_x_mm,
            "origin_y_mm": m.grid.origin_y_mm,
            "width_mm": m.grid.width_mm,
            "height_mm": m.grid.height_mm,
        },
        "calibrated": m.calibrated,
        "color": format!("#{:02X}{:02X}{:02X}", m.color[0], m.color[1], m.color[2]),
    })
}

fn run_state(s: &mut Session, _: NoParams, _: Origin) -> Result<Value, CommandError> {
    let mat = mat_json(s);
    let area = cutting_area(s).ok().map(rect_json).unwrap_or(Value::Null);
    let st = s.doc();
    let job = &st.document.job;
    let selection: Vec<_> = st.selection.iter().copied().collect();
    Ok(json!({
        "index": s.active,
        "documents": s.docs.len(),
        "doc_id": st.id(),
        "doc_ids": s.docs.iter().map(|d| d.id()).collect::<Vec<_>>(),
        "objects": st.document.objects.iter().map(object_json).collect::<Vec<_>>(),
        "selection": ids_json(&selection),
        "job": {
            "plotter_id": job.plotter_id,
            "material_id": job.material_id,
            "media_id": job.media_id,
            "speed": job.speed,
            "force": job.force,
            "passes": job.passes,
        },
        "mat": mat,
        "cutting_area_mm": area,
        "dirty": st.dirty,
        "path": st.path,
        "can_undo": st.undo.can_undo(),
        "can_redo": st.undo.can_redo(),
    }))
}

command!(
    DocumentState,
    id: "document.state",
    title: "Document state",
    mutates: false,
    params: NoParams,
    run: run_state,
    description: "Describes the active document. All lengths are mm; the origin is the 0,0 of the grid printed on the mat, x right, y down; rotation is degrees clockwise. Returns {index, documents, doc_id (stable id of the active document), doc_ids (stable ids of all open documents, in index order), objects: [{id, name, kind: svg|raster, visible, bounds_mm: {x0,y0,x1,y1} of what is cut (the cut geometry per the cut options, stroke outlines included), transform: {center_mm: [x,y], scale_x, scale_y, rotation_deg, mirror}, cut: {mode: all_lines|object_outlines, strokes: auto|centre_line|outline, auto_threshold_mm} (see object.set_cut_options), link: {path (last known absolute path of the imported file), relative (to the project file, or null)} or null when the object has no linked file}], selection: [id], job: {plotter_id, material_id, media_id, speed, force, passes} (null overrides mean the material value), mat: {id, name, kind: mat|no_mat, width_mm, height_mm (physical), grid: {origin_x_mm, origin_y_mm, width_mm, height_mm}, calibrated, color: \"#RRGGBB\"} (null when unknown), cutting_area_mm: {x0,y0,x1,y1} (the grid, minus a 5 mm safety margin while the mat is not calibrated; objects must stay inside it), dirty, path, can_undo, can_redo}.",
);

// ---------------------------------------------------------------- document.new / switch / close

fn run_new(s: &mut Session, _: NoParams, _: Origin) -> Result<Value, CommandError> {
    let st = s.new_doc_state();
    s.docs.push(st);
    s.active = s.docs.len() - 1;
    Ok(json!({ "index": s.active, "documents": s.docs.len() }))
}

command!(
    DocumentNew,
    id: "document.new",
    title: "New document",
    mutates: false,
    params: NoParams,
    run: run_new,
    description: "Opens a new empty document with the default plotter, material and the 12x12 inch mat, and makes it the active document. Returns {index, documents}.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexParams {
    /// Index of the open document, starting at 0 (see document.state). Give index or doc_id.
    pub index: Option<usize>,
    /// Stable id of the open document (see document.state). Give index or doc_id.
    pub doc_id: Option<u64>,
}

fn check_index(s: &Session, index: usize) -> Result<(), CommandError> {
    if index < s.docs.len() {
        Ok(())
    } else {
        Err(CommandError::NotFound(format!(
            "no open document with index {index}; there are {}",
            s.docs.len()
        )))
    }
}

/// The index of the document an index or a stable id names. Exactly one
/// of the two must be given.
fn target(s: &Session, index: Option<usize>, doc_id: Option<u64>) -> Result<usize, CommandError> {
    match (index, doc_id) {
        (Some(index), None) => check_index(s, index).map(|()| index),
        (None, Some(id)) => s.doc_index(id),
        _ => Err(CommandError::InvalidParams(
            "give exactly one of index and doc_id".into(),
        )),
    }
}

fn run_switch(s: &mut Session, p: IndexParams, _: Origin) -> Result<Value, CommandError> {
    s.active = target(s, p.index, p.doc_id)?;
    Ok(json!({ "index": s.active, "documents": s.docs.len() }))
}

command!(
    DocumentSwitch,
    id: "document.switch",
    title: "Switch document",
    mutates: false,
    params: IndexParams,
    run: run_switch,
    description: "Makes an open document the active one, named by index (starting at 0) or by its stable doc_id; give exactly one. Commands work on the active document. Returns {index, documents}.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseParams {
    /// Index of the open document, starting at 0. Give index or doc_id.
    pub index: Option<usize>,
    /// Stable id of the open document (see document.state). Give index or doc_id.
    pub doc_id: Option<u64>,
    /// Close even if the document has unsaved changes (default false).
    #[serde(default)]
    pub discard_changes: bool,
}

fn run_close(s: &mut Session, p: CloseParams, _: Origin) -> Result<Value, CommandError> {
    let index = target(s, p.index, p.doc_id)?;
    if s.docs[index].dirty && !p.discard_changes {
        return Err(CommandError::Refused(format!(
            "document {index} has unsaved changes. Save it with project.save, or pass discard_changes: true"
        )));
    }
    s.docs.remove(index);
    if s.docs.is_empty() {
        let st = s.new_doc_state();
        s.docs.push(st);
    }
    if index < s.active || s.active >= s.docs.len() {
        s.active = s.active.saturating_sub(1);
    }
    Ok(json!({ "index": s.active, "documents": s.docs.len() }))
}

command!(
    DocumentClose,
    id: "document.close",
    title: "Close document",
    mutates: false,
    params: CloseParams,
    run: run_close,
    description: "Closes an open document, named by index (starting at 0) or by its stable doc_id; give exactly one. A document with unsaved changes is only closed with discard_changes: true. Closing the last document opens a new empty one. Returns {index (the active document), documents}.",
);

// ---------------------------------------------------------------- project.save / open

/// Removes `.` and resolves `..` without touching the file system.
pub(crate) fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The path with symlinks resolved as far as the file system allows.
pub(crate) fn resolved(path: &Path) -> PathBuf {
    if let Ok(p) = path.canonicalize() {
        return p;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        && let Ok(p) = parent.canonicalize()
    {
        return p.join(name);
    }
    lexical(path)
}

/// True if `path` is the config dir or lies inside it.
pub(crate) fn inside(path: &Path, config_dir: &Path) -> bool {
    let abs = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let path = abs(path);
    let cfg = abs(config_dir);
    resolved(&path).starts_with(resolved(&cfg)) || lexical(&path).starts_with(lexical(&cfg))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveParams {
    /// File to write, usually ending in .flatcraft. Default: the path the document was opened from or last saved to.
    pub path: Option<PathBuf>,
    /// Stable id of the open document to save (see document.state). Default: the active document.
    pub doc_id: Option<u64>,
}

fn run_save(s: &mut Session, p: SaveParams, origin: Origin) -> Result<Value, CommandError> {
    let index = match p.doc_id {
        Some(id) => s.doc_index(id)?,
        None => s.active,
    };
    let path = p
        .path
        .or_else(|| s.docs[index].path.clone())
        .ok_or_else(|| {
            CommandError::InvalidParams(
                "no path: this document was never saved, so give a path".into(),
            )
        })?;
    if origin == Origin::Agent && inside(&path, &s.config_dir) {
        return Err(CommandError::Refused(format!(
            "agents may not write into the FlatCraft config directory ({})",
            s.config_dir.display()
        )));
    }
    // Linked files: refresh their paths for the new place of the project.
    // Only metadata, so it is not an undo step; a failed save puts it back.
    let st = &mut s.docs[index];
    let previous = relink_for_save(&mut st.document, st.path.as_deref(), &path);
    if let Err(e) = project::save(&st.document, &path) {
        restore_links(&mut st.document, previous);
        return Err(CommandError::Failed(e.to_string()));
    }
    st.path = Some(path.clone());
    st.dirty = false;
    Ok(json!({ "path": path }))
}

command!(
    ProjectSave,
    id: "project.save",
    title: "Save project",
    mutates: false,
    params: SaveParams,
    run: run_save,
    description: "Saves an open document (the active one, or the one with doc_id) as a project file (JSON with the original SVG sources embedded). Without path it uses the document's current path; a document that was never saved needs a path. Linked files (see object.reload) keep pointing at the same files; their paths relative to the project file are updated. Agents may not write inside the FlatCraft config directory. Returns {path}.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenParams {
    /// Project file to open.
    pub path: PathBuf,
}

fn run_open(s: &mut Session, p: OpenParams, _: Origin) -> Result<Value, CommandError> {
    let mut document = project::load(&p.path).map_err(|e| match e {
        ProjectError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
            CommandError::NotFound(format!("file {}", p.path.display()))
        }
        other => CommandError::Failed(other.to_string()),
    })?;
    let mut warnings = Vec::new();
    for o in &mut document.objects {
        if !o.cut.is_valid() {
            o.cut = CutOptions::default();
            warnings.push(format!(
                "{}: the cut options were broken and are back to the defaults",
                o.name
            ));
        }
        match &o.source {
            Source::Svg { bytes, .. } => match import_svg(bytes, fonts::system_fonts()) {
                Ok(import) => {
                    for w in &import.warnings {
                        warnings.push(format!("{}: {}", o.name, warning_text(w)));
                    }
                    o.set_geometry(import.paths);
                    o.frame = import.page_frame_mm;
                }
                Err(e) => warnings.push(format!("{}: cannot rebuild the shape: {e}", o.name)),
            },
            Source::Raster { bytes, trace, .. } => {
                let blade = blade_offset_for(s, &document.job.plotter_id);
                match flatcraft_import::trace::trace(bytes, trace, blade) {
                    Ok(t) => {
                        o.set_geometry(t.paths);
                        o.frame = Rect::new(0.0, 0.0, t.size_mm.0, t.size_mm.1);
                    }
                    Err(e) => warnings.push(format!("{}: cannot trace the image: {e}", o.name)),
                }
            }
        }
    }
    // A relative path in a project file is relative to that file.
    let anchor = project_dir(Some(&p.path));
    for link in document.objects.iter_mut().filter_map(|o| o.link.as_mut()) {
        link.anchor_dir = anchor.clone();
    }
    let objects = document.objects.len();
    let mut st = DocState::new(document);
    st.path = Some(p.path.clone());
    if s.doc().is_pristine() {
        *s.doc_mut() = st;
    } else {
        s.docs.push(st);
        s.active = s.docs.len() - 1;
    }
    Ok(json!({
        "index": s.active,
        "path": p.path,
        "objects": objects,
        "warnings": warnings,
    }))
}

command!(
    ProjectOpen,
    id: "project.open",
    title: "Open project",
    mutates: false,
    params: OpenParams,
    run: run_open,
    description: "Opens a project file and makes it the active document. It replaces the active document only if that one is empty and untouched; otherwise it opens as an extra document. The cut shapes are rebuilt from the embedded sources, so a project opens even when the imported files are gone; use document.check_sources to see which linked files changed or are missing. Returns {index, path, objects (count), warnings: [text]}.",
);
