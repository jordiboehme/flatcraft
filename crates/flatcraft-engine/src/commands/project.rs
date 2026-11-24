//! Document and project file commands.

use std::path::{Component, Path, PathBuf};

use flatcraft_core::document::{Object, Source};
use flatcraft_core::project::{self, ProjectError};
use flatcraft_import::fonts;
use flatcraft_import::svg::import_svg;
use serde::Deserialize;
use serde_json::{Value, json};

use super::object::warning_text;
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
    })
}

fn run_state(s: &mut Session, _: NoParams, _: Origin) -> Result<Value, CommandError> {
    let area = cutting_area(s).ok().map(rect_json).unwrap_or(Value::Null);
    let st = s.doc();
    let job = &st.document.job;
    let selection: Vec<_> = st.selection.iter().copied().collect();
    Ok(json!({
        "index": s.active,
        "documents": s.docs.len(),
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
    description: "Describes the active document. All lengths are mm; the origin is the top-left of the media, x right, y down; rotation is degrees clockwise. Returns {index, documents, objects: [{id, name, kind: svg|raster, visible, bounds_mm: {x0,y0,x1,y1} of the placed geometry, transform: {center_mm: [x,y], scale_x, scale_y, rotation_deg, mirror}}], selection: [id], job: {plotter_id, material_id, media_id, speed, force, passes} (null overrides mean the material value), cutting_area_mm: {x0,y0,x1,y1} (objects must stay inside it), dirty, path, can_undo, can_redo}.",
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
    /// Index of the open document, starting at 0 (see document.state).
    pub index: usize,
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

fn run_switch(s: &mut Session, p: IndexParams, _: Origin) -> Result<Value, CommandError> {
    check_index(s, p.index)?;
    s.active = p.index;
    Ok(json!({ "index": s.active, "documents": s.docs.len() }))
}

command!(
    DocumentSwitch,
    id: "document.switch",
    title: "Switch document",
    mutates: false,
    params: IndexParams,
    run: run_switch,
    description: "Makes the open document with this index (starting at 0) the active one. Commands work on the active document. Returns {index, documents}.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseParams {
    /// Index of the open document, starting at 0.
    pub index: usize,
    /// Close even if the document has unsaved changes (default false).
    #[serde(default)]
    pub discard_changes: bool,
}

fn run_close(s: &mut Session, p: CloseParams, _: Origin) -> Result<Value, CommandError> {
    check_index(s, p.index)?;
    if s.docs[p.index].dirty && !p.discard_changes {
        return Err(CommandError::Refused(format!(
            "document {} has unsaved changes. Save it with project.save, or pass discard_changes: true",
            p.index
        )));
    }
    s.docs.remove(p.index);
    if s.docs.is_empty() {
        let st = s.new_doc_state();
        s.docs.push(st);
    }
    if p.index < s.active || s.active >= s.docs.len() {
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
    description: "Closes the open document with this index (starting at 0). A document with unsaved changes is only closed with discard_changes: true. Closing the last document opens a new empty one. Returns {index (the active document), documents}.",
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
}

fn run_save(s: &mut Session, p: SaveParams, origin: Origin) -> Result<Value, CommandError> {
    let path = p.path.or_else(|| s.doc().path.clone()).ok_or_else(|| {
        CommandError::InvalidParams("no path: this document was never saved, so give a path".into())
    })?;
    if origin == Origin::Agent && inside(&path, &s.config_dir) {
        return Err(CommandError::Refused(format!(
            "agents may not write into the FlatCraft config directory ({})",
            s.config_dir.display()
        )));
    }
    project::save(&s.doc().document, &path).map_err(|e| CommandError::Failed(e.to_string()))?;
    let st = s.doc_mut();
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
    description: "Saves the active document as a project file (JSON with the original SVG sources embedded). Without path it uses the document's current path; a document that was never saved needs a path. Agents may not write inside the FlatCraft config directory. Returns {path}.",
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
        match &o.source {
            Source::Svg { bytes, .. } => match import_svg(bytes, fonts::system_fonts()) {
                Ok(import) => {
                    for w in &import.warnings {
                        warnings.push(format!("{}: {}", o.name, warning_text(w)));
                    }
                    o.set_geometry(import.paths);
                }
                Err(e) => warnings.push(format!("{}: cannot rebuild the shape: {e}", o.name)),
            },
            Source::Raster { .. } => {
                warnings.push(format!(
                    "{}: raster tracing arrives in a later task",
                    o.name
                ));
            }
        }
    }
    let objects = document.objects.len();
    let mut st = DocState::new(document);
    st.path = Some(p.path.clone());
    let current = s.doc();
    let pristine = current.document.objects.is_empty()
        && !current.dirty
        && current.path.is_none()
        && !current.undo.can_undo()
        && !current.undo.can_redo();
    if pristine {
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
    description: "Opens a project file and makes it the active document. It replaces the active document only if that one is empty and untouched; otherwise it opens as an extra document. The cut shapes are rebuilt from the embedded sources. Returns {index, path, objects (count), warnings: [text]}.",
);
