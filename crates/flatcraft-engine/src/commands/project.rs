//! Document and project file commands.

use std::path::{Component, Path, PathBuf};

use std::sync::Arc;

use flatcraft_core::document::{CutOptions, GroupId, Object, ObjectId, Source};
use flatcraft_core::geometry::{CutPath, bounds};
use flatcraft_core::project::{self, ProjectError};
use flatcraft_device::MediaKind;
use flatcraft_import::fonts;
use flatcraft_import::svg::import_svg;
use kurbo::Rect;
use serde::Deserialize;
use serde_json::{Value, json};

use super::object::{blade_offset_for, warning_text};
use super::sources::{project_dir, relink_for_save, restore_links};
use super::{NoParams, command, cutting_area, ids_json, point_arr, rect_json};
use crate::session::{CommandError, DocState, Origin, Session};

// ---------------------------------------------------------------- document.state

fn object_json(o: &Object, bounds: Option<Rect>) -> Value {
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
        "bounds_mm": bounds.map(rect_json).unwrap_or(Value::Null),
        "transform": {
            "center_mm": point_arr(t.center_mm),
            "scale_x": t.scale_x,
            "scale_y": t.scale_y,
            "rotation_deg": t.rotation_deg,
            "mirror": t.mirror,
        },
        "cut": super::object::cut_json(&o.cut),
        "link": o.link.as_ref().map(|l| json!({ "path": l.path, "relative": l.relative })),
        "link_group": o.link_group.map(|g| g.0),
    })
}

/// The mat the document uses (see `effective_media`) and the warning when
/// it is not the document's own mat. Null when the plotter is unknown.
fn mat_json(s: &Session) -> (Value, Option<String>) {
    let job = &s.doc().document.job;
    let Ok(plotter) = s.profiles.plotter(&job.plotter_id) else {
        return (Value::Null, None);
    };
    let Ok((m, warning)) = super::effective_media(&plotter, job) else {
        return (Value::Null, None);
    };
    let mat = json!({
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
    });
    (mat, warning)
}

fn run_state(s: &mut Session, _: NoParams, _: Origin) -> Result<Value, CommandError> {
    let (mat, mat_warning) = mat_json(s);
    let area = cutting_area(s).ok().map(rect_json).unwrap_or(Value::Null);
    let bounds = s.doc_mut().placed_bounds_all();
    let st = s.doc();
    let job = &st.document.job;
    let selection: Vec<_> = st.selection.iter().copied().collect();
    Ok(json!({
        "index": s.active,
        "documents": s.docs.len(),
        "doc_id": st.id(),
        "doc_ids": s.docs.iter().map(|d| d.id()).collect::<Vec<_>>(),
        "objects": st
            .document
            .objects
            .iter()
            .zip(bounds)
            .map(|(o, b)| object_json(o, b))
            .collect::<Vec<_>>(),
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
        "warnings": mat_warning.into_iter().collect::<Vec<_>>(),
    }))
}

command!(
    DocumentState,
    id: "document.state",
    title: "Document state",
    mutates: false,
    params: NoParams,
    run: run_state,
    description: "Describes the active document. All lengths are mm; the origin is the 0,0 of the grid printed on the mat, x right, y down; rotation is degrees clockwise. Returns {index, documents, doc_id (stable id of the active document), doc_ids (stable ids of all open documents, in index order), objects: [{id, name, kind: svg|raster, visible, bounds_mm: {x0,y0,x1,y1} of what is cut (the cut geometry per the cut options, stroke outlines included), transform: {center_mm: [x,y], scale_x, scale_y, rotation_deg, mirror}, cut: {mode: all_lines|object_outlines, strokes: auto|centre_line|outline, auto_threshold_mm} (see object.set_cut_options), link: {path (last known absolute path of the imported file), relative (to the project file, or null)} or null when the object has no linked file}], selection: [id], job: {plotter_id, material_id, media_id, speed, force, passes} (null overrides mean the material value), mat: {id, name, kind: mat|no_mat, width_mm, height_mm (physical), grid: {origin_x_mm, origin_y_mm, width_mm, height_mm}, calibrated, color: \"#RRGGBB\"} (null when unknown; when the plotter no longer lists job.media_id, hidden or deleted, this is the plotter's first mat, which jobs then use too), cutting_area_mm: {x0,y0,x1,y1} (the grid, minus a 5 mm safety margin while the mat is not calibrated; objects must stay inside it), dirty, path, can_undo, can_redo, warnings: [text] (for example a mat that is no longer available)}.",
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
    session: true,
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
    session: true,
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
    session: true,
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

/// The path with symlinks resolved as far as the file system allows: a link
/// at the path itself is followed even when it dangles, because a save
/// writes through it (`fsio::write_atomic`); then the longest part that
/// exists is resolved and the rest is appended. Lenient: what cannot be
/// resolved stays lexical. Only for the config dir itself; paths an agent
/// writes to go through [`resolve_strict`].
pub(crate) fn resolved(path: &Path) -> PathBuf {
    let linked = flatcraft_device::fsio::resolve_links(path).unwrap_or_else(|_| path.to_path_buf());
    let mut base = linked.as_path();
    let mut tail = Vec::new();
    loop {
        if let Ok(mut p) = base.canonicalize() {
            p.extend(tail.iter().rev());
            return p;
        }
        match (base.parent(), base.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name);
                base = parent;
            }
            _ => return lexical(&linked),
        }
    }
}

/// The path with the links at it followed and the folders above resolved
/// by the file system. The part that does not exist yet must be plain
/// names, at most `missing` of them: a `..` after a folder that does not
/// exist, or a link loop, is an error, never a lexical guess (which could
/// walk through a linked folder somewhere else).
pub(crate) fn resolve_strict(path: &Path, missing: usize) -> std::io::Result<PathBuf> {
    resolve_strict_linked(&flatcraft_device::fsio::resolve_links(path)?, missing)
}

fn resolve_strict_linked(linked: &Path, missing: usize) -> std::io::Result<PathBuf> {
    let mut base = linked;
    let mut tail = Vec::new();
    loop {
        match base.canonicalize() {
            Ok(mut p) => {
                p.extend(tail.iter().rev());
                return Ok(p);
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            Err(_) => {}
        }
        match (base.parent(), base.file_name()) {
            (Some(parent), Some(name)) if tail.len() < missing => {
                tail.push(name);
                base = parent;
            }
            (Some(_), Some(_)) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "the folder does not exist",
                ));
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "the path cannot be resolved (a '..' after a folder that does not exist)",
                ));
            }
        }
    }
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

/// True if the resolved `target` or the `given` path (both absolute) lies
/// in the config dir.
pub(crate) fn in_config(target: &Path, given: &Path, config_dir: &Path) -> bool {
    let cfg = absolute(config_dir);
    target.starts_with(resolved(&cfg)) || lexical(given).starts_with(lexical(&cfg))
}

/// The file a command's write to `path` replaces, resolved once (links at
/// the path followed, folders resolved by the file system; its folder must
/// exist), and checked against the agent limit. The command writes to
/// exactly this target without following links again
/// (`fsio::write_atomic_at`), so a link planted after the check cannot
/// redirect the write.
pub(crate) fn write_target(
    path: &Path,
    config_dir: &Path,
    origin: Origin,
) -> Result<PathBuf, CommandError> {
    let given = absolute(path);
    let refused = || {
        CommandError::Refused(format!(
            "agents may not write into the FlatCraft config directory ({})",
            config_dir.display()
        ))
    };
    let target = match resolve_strict(&given, 1) {
        Ok(target) => target,
        // Never written, but an agent aiming at the config dir hears why.
        Err(_) if origin == Origin::Agent && in_config(&resolved(&given), &given, config_dir) => {
            return Err(refused());
        }
        Err(e) => {
            return Err(CommandError::Failed(format!(
                "cannot write {}: {e}",
                path.display()
            )));
        }
    };
    if origin == Origin::Agent && in_config(&target, &given, config_dir) {
        return Err(refused());
    }
    #[cfg(test)]
    tests::between_check_and_write();
    Ok(target)
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
    let target = write_target(&path, &s.config_dir, origin)?;
    // Linked files: refresh their paths for the new place of the project.
    // Only metadata, so it is not an undo step; a failed save puts it back.
    let st = &mut s.docs[index];
    let previous = relink_for_save(&mut st.document, st.path.as_deref(), &path);
    if let Err(e) = project::save_at(&st.document, &target) {
        restore_links(&mut st.document, previous);
        return Err(CommandError::Failed(e.to_string()));
    }
    if !previous.is_empty() {
        st.touch();
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
    session: true,
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

/// Saved and rebuilt local bounds further apart than this, mm, on any
/// side, mean the source draws differently here.
const BOUNDS_CHANGED_MM: f64 = 0.01;

fn bounds_differ(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)]
        .iter()
        .any(|(p, q)| (p - q).abs() > BOUNDS_CHANGED_MM)
}

fn run_open(s: &mut Session, p: OpenParams, _: Origin) -> Result<Value, CommandError> {
    let mut document = project::load(&p.path).map_err(|e| match e {
        ProjectError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
            CommandError::NotFound(format!("file {}", p.path.display()))
        }
        other => CommandError::Failed(other.to_string()),
    })?;
    for o in &document.objects {
        if !o.transform.is_valid() {
            return Err(CommandError::Failed(format!(
                "cannot open {}: the transform of '{}' is broken (its position, scale or rotation is not a usable number, or beyond 1 km). Fix or remove that object in the project file.",
                p.path.display(),
                o.name
            )));
        }
    }
    let mut warnings = Vec::new();
    let blade = blade_offset_for(s, &document.job.plotter_id);
    // Linked copies share one source: it is rebuilt once per group, and the
    // members share the geometry `Arc`.
    let mut groups: Vec<(GroupId, Option<Shared>)> = Vec::new();
    for o in &mut document.objects {
        let known = o
            .link_group
            .and_then(|g| groups.iter().find(|(id, _)| *id == g).cloned());
        // Members after the first say nothing the first did not say.
        let first = known.is_none();
        if !o.cut.is_valid() {
            o.cut = CutOptions::default();
            if first {
                warnings.push(format!(
                    "{}: the cut options were broken and are back to the defaults",
                    o.name
                ));
            }
        }
        let rebuilt = match known {
            Some((_, shared)) => shared,
            None => {
                let shared = rebuild_shared(o, blade, &mut warnings);
                if let Some(g) = o.link_group {
                    groups.push((g, shared.clone()));
                }
                shared
            }
        };
        if let Some((geometry, local_bounds, frame)) = rebuilt {
            let saved = o.local_bounds != Rect::ZERO || o.frame != Rect::ZERO;
            let mut anchored = None;
            if saved && rect_is_sane(o.local_bounds) && rect_is_sane(o.frame) {
                // The saved bounds and frame are what the transform was
                // made against: keep the frame in place, as reload does.
                let mut a = o.clone();
                a.share_geometry_anchored(geometry.clone(), local_bounds, frame);
                let c = a.transform.center_mm;
                if c.x.is_finite() && c.y.is_finite() {
                    anchored = Some(a);
                }
            }
            match anchored {
                Some(a) => {
                    // Text in an SVG is set with the fonts of this
                    // computer; other fonts give other shapes.
                    let svg = matches!(o.source, Source::Svg { .. });
                    if first
                        && svg
                        && o.local_bounds != Rect::ZERO
                        && bounds_differ(o.local_bounds, a.local_bounds)
                    {
                        warnings.push(format!(
                            "{} looks different on this computer (missing fonts?)",
                            o.name
                        ));
                    }
                    *o = a;
                }
                None => {
                    if saved && first {
                        warnings.push(format!(
                            "{}: the saved position data is broken; the object is placed by its shapes, as in older projects",
                            o.name
                        ));
                    }
                    // A project from before the saved bounds: the
                    // transform was made against the rebuilt bounds.
                    o.geometry = geometry;
                    o.local_bounds = local_bounds;
                    o.frame = frame;
                }
            }
        }
    }
    // The file keeps the shared fields once per group, and a member's own
    // copy of them is replaced by the group's when it loads, so members
    // agree. A group of one (the others were removed by hand) is no group.
    document.dissolve_lonely_groups();
    if !document.link_groups_agree() {
        // Never expected. Repaired, so the project opens and later
        // commands are not refused for a mismatch they did not make.
        let leads: Vec<(ObjectId, String)> = document
            .group_leads()
            .into_iter()
            .filter_map(|id| document.get(id).map(|o| (id, o.name.clone())))
            .collect();
        for (id, name) in leads {
            document.spread_shared(id);
            warnings.push(format!(
                "{name}: its linked copies did not match, so they now follow it"
            ));
        }
    }
    if let Some(o) = document.objects.iter().find(|o| !o.placement_is_valid()) {
        return Err(CommandError::Failed(format!(
            "cannot open {}: '{}' is placed beyond 1 km. Fix or remove that object in the project file.",
            p.path.display(),
            o.name
        )));
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

/// Geometry rebuilt from a source: the paths, their local bounds and the
/// page frame.
type Shared = (Arc<Vec<CutPath>>, Rect, Rect);

/// Rebuilds the geometry of `o` from its embedded source. A failure is a
/// warning and `None`: the object opens without shapes.
fn rebuild_shared(o: &Object, blade: f64, warnings: &mut Vec<String>) -> Option<Shared> {
    let (paths, frame) = match &o.source {
        Source::Svg { bytes, .. } => match import_svg(bytes, fonts::system_fonts()) {
            Ok(import) => {
                for w in &import.warnings {
                    warnings.push(format!("{}: {}", o.name, warning_text(w)));
                }
                (import.paths, import.page_frame_mm)
            }
            Err(e) => {
                warnings.push(format!(
                    "{}: cannot rebuild the shape: {}",
                    o.name,
                    joined(&e.to_string())
                ));
                return None;
            }
        },
        Source::Raster { bytes, trace, .. } => {
            match flatcraft_import::trace::trace(bytes, trace, blade) {
                Ok(t) => (t.paths, Rect::new(0.0, 0.0, t.size_mm.0, t.size_mm.1)),
                Err(e) => {
                    warnings.push(format!(
                        "{}: cannot trace the image: {}",
                        o.name,
                        joined(&e.to_string())
                    ));
                    return None;
                }
            }
        }
    };
    let local_bounds = bounds(&paths).unwrap_or(Rect::ZERO);
    Some((Arc::new(paths), local_bounds, frame))
}

/// A sentence joined after a colon: its first letter in lower case.
fn joined(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A saved `local_bounds` or `frame`: finite, ordered and within 1 km.
fn rect_is_sane(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1]
        .iter()
        .all(|v| v.is_finite() && v.abs() <= flatcraft_core::transform::MAX_PLACEMENT_MM)
        && r.x0 <= r.x1
        && r.y0 <= r.y1
}

command!(
    ProjectOpen,
    id: "project.open",
    title: "Open project",
    mutates: false,
    session: true,
    params: OpenParams,
    run: run_open,
    description: "Opens a project file and makes it the active document. It replaces the active document only if that one is empty and untouched; otherwise it opens as an extra document. The cut shapes are rebuilt from the embedded sources, so a project opens even when the imported files are gone; use document.check_sources to see which linked files changed or are missing. Returns {index, path, objects (count), warnings: [text]}.",
);

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::cutgeom::tests::CUT_BOUNDS_CALLS;
    use std::cell::RefCell;

    thread_local! {
        /// Runs once between the agent path check and the write.
        static BETWEEN: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    }

    /// Runs `f` once between the next agent path check and its write.
    pub(crate) fn set_between(f: impl FnOnce() + 'static) {
        BETWEEN.with(|b| *b.borrow_mut() = Some(Box::new(f)));
    }

    pub(crate) fn between_check_and_write() {
        if let Some(f) = BETWEEN.with(|b| b.borrow_mut().take()) {
            f();
        }
    }

    fn session(config: &Path) -> Session {
        let profiles = flatcraft_device::ProfileStore::load(config).unwrap();
        let prefs = crate::prefs::Preferences {
            output_dir: Some(config.join("output")),
            ..Default::default()
        };
        Session::new(profiles, prefs, false, config.to_path_buf())
    }

    /// A link loop at the file name still leaves the folder links above it
    /// resolved, so a linked folder into the config dir is seen.
    #[cfg(unix)]
    #[test]
    fn a_link_loop_below_a_linked_folder_is_inside() {
        let config = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let folder = elsewhere.path().join("folder");
        std::os::unix::fs::symlink(config.path(), &folder).unwrap();
        let looped = config.path().join("x");
        std::os::unix::fs::symlink(&looped, &looped).unwrap();
        let mut s = session(config.path());
        let e = s
            .execute(
                "project.save",
                json!({ "path": folder.join("x") }),
                Origin::Agent,
            )
            .unwrap_err();
        assert!(matches!(e, CommandError::Refused(_)), "{e}");
        assert!(std::fs::symlink_metadata(&looped).unwrap().is_symlink());
    }

    /// A link planted at the target after the check but before the write
    /// is not followed: the write goes to the file that was checked.
    #[cfg(unix)]
    #[test]
    fn a_link_planted_after_the_check_is_not_followed() {
        let config = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let prefs = config.path().join("preferences.toml");
        std::fs::write(&prefs, "theme = \"Light\"\n").unwrap();
        let mut s = session(config.path());
        for (id, file, params) in [
            ("project.save", "x.flatcraft", "path"),
            ("tools.test_patterns", "x.svg", "svg_path"),
        ] {
            let target = elsewhere.path().join(file);
            let (link_to, at) = (prefs.clone(), target.clone());
            BETWEEN.with(|b| {
                *b.borrow_mut() = Some(Box::new(move || {
                    std::os::unix::fs::symlink(link_to, at).unwrap();
                }));
            });
            let r = s.execute(id, json!({ params: target }), Origin::Agent);
            assert!(r.is_ok(), "{id}: {r:?}");
            assert_eq!(
                std::fs::read_to_string(&prefs).unwrap(),
                "theme = \"Light\"\n",
                "{id} wrote through the planted link"
            );
            assert!(
                !std::fs::symlink_metadata(&target)
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                "{id}: the write replaced the link"
            );
        }
    }

    const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="20mm" viewBox="0 0 20 20"><rect x="2" y="2" width="5" height="5"/></svg>"#;

    fn calls() -> usize {
        CUT_BOUNDS_CALLS.with(std::cell::Cell::get)
    }

    #[test]
    fn state_computes_cut_bounds_once_per_object_change() {
        let dir = tempfile::tempdir().unwrap();
        let profiles = flatcraft_device::ProfileStore::load(dir.path()).unwrap();
        let prefs = crate::prefs::Preferences {
            output_dir: Some(dir.path().join("output")),
            ..Default::default()
        };
        let mut s = Session::new(profiles, prefs, false, dir.path().to_path_buf());
        let mut ids = Vec::new();
        for _ in 0..2 {
            let r = s
                .execute(
                    "object.import_svg",
                    json!({ "svg": SVG, "name": "sq" }),
                    Origin::User,
                )
                .unwrap();
            ids.push(r["id"].as_u64().unwrap());
        }
        let state = |s: &mut Session| {
            s.execute("document.state", json!({}), Origin::User)
                .unwrap()
        };
        let first = state(&mut s);
        let before = calls();
        assert_eq!(state(&mut s), first);
        assert_eq!(calls(), before, "nothing changed, nothing is recomputed");
        s.execute(
            "object.move",
            json!({ "ids": [ids[0]], "dx_mm": 1.0 }),
            Origin::User,
        )
        .unwrap();
        let before = calls();
        let moved = state(&mut s);
        assert_eq!(calls(), before + 1, "only the moved object is recomputed");
        assert_eq!(
            moved["objects"][0]["bounds_mm"]["x0"].as_f64().unwrap(),
            first["objects"][0]["bounds_mm"]["x0"].as_f64().unwrap() + 1.0
        );
    }
}
