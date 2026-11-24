//! Linked sources: imported objects remember their file, so the owner can
//! edit it in another app and reload it here. The embedded copy is the
//! source of truth: a missing, renamed or broken file never stops a project
//! from opening, saving, editing or cutting, and a failed reload of one
//! object never changes that object.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use flatcraft_core::document::{Document, ObjectId, Source, SourceLink};
use flatcraft_core::geometry::CutPath;
use flatcraft_import::svg::import_svg;
use flatcraft_import::{ImportError, fonts};
use kurbo::Rect;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::object::{blade_offset_mm, trace_report, warning_text};
use super::project::lexical;
use super::{NoParams, check_exists, command, ids_json, resolve_ids};
use crate::session::{CommandError, Origin, Session};

/// Largest file that is imported or reloaded.
pub(crate) const MAX_SOURCE_BYTES: u64 = 50 * 1024 * 1024;

// ---------------------------------------------------------------- files

/// Why a source file could not be read.
#[derive(Debug)]
pub(crate) enum ReadError {
    NotFound,
    TooLarge,
    Io(std::io::Error),
}

impl ReadError {
    /// Text for people and agents, without the path.
    pub(crate) fn text(&self) -> String {
        match self {
            ReadError::NotFound => "the file was not found".into(),
            ReadError::TooLarge => "the file is larger than 50 MB".into(),
            ReadError::Io(e) => format!("the file cannot be read: {e}"),
        }
    }
}

/// A file read with the import size cap, plus its modification time.
pub(crate) struct SourceFile {
    pub bytes: Vec<u8>,
    pub modified: Option<String>,
}

/// Reads at most 50 MB; a larger file is refused without reading it all.
pub(crate) fn read_capped(path: &Path) -> Result<SourceFile, ReadError> {
    let file = std::fs::File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ReadError::NotFound,
        _ => ReadError::Io(e),
    })?;
    let meta = file.metadata().map_err(ReadError::Io)?;
    if !meta.is_file() {
        return Err(ReadError::NotFound);
    }
    if meta.len() > MAX_SOURCE_BYTES {
        return Err(ReadError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(ReadError::Io)?;
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        return Err(ReadError::TooLarge);
    }
    Ok(SourceFile {
        bytes,
        modified: meta.modified().ok().and_then(rfc3339),
    })
}

/// The kind of object a file becomes, by its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileKind {
    Svg,
    Raster,
}

pub(crate) const SUPPORTED_TYPES: &str = ".svg, .svgz, .png, .jpg, .jpeg, .webp, .gif";

/// The file kind by extension (case does not matter).
pub(crate) fn file_kind(path: &Path) -> Result<FileKind, CommandError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "svg" | "svgz" => Ok(FileKind::Svg),
        "png" | "jpg" | "jpeg" | "webp" | "gif" => Ok(FileKind::Raster),
        _ => {
            let shown = if ext.is_empty() {
                "no extension".to_string()
            } else {
                format!("'.{ext}'")
            };
            Err(CommandError::InvalidParams(format!(
                "unsupported file type ({shown}). Supported types: {SUPPORTED_TYPES}"
            )))
        }
    }
}

/// An absolute path without `.` and `..`, without touching the file system
/// (no symlink resolution, so it compares with what the user gave).
pub(crate) fn absolute(path: &Path) -> PathBuf {
    lexical(&std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// `file` relative to the folder `dir`, both absolute. `None` when they
/// have no common root (another drive on Windows) or a part is not UTF-8.
/// The parts are joined with `/` on every platform, so a project folder
/// moved between macOS, Linux and Windows still finds its files.
pub(crate) fn relative_to(file: &Path, dir: &Path) -> Option<PathBuf> {
    let file: Vec<Component> = file.components().collect();
    let dir: Vec<Component> = dir.components().collect();
    let common = file.iter().zip(&dir).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return None;
    }
    let mut parts: Vec<&str> = vec![".."; dir.len() - common];
    for c in &file[common..] {
        parts.push(c.as_os_str().to_str()?);
    }
    Some(PathBuf::from(parts.join("/")))
}

/// The folder of a project file, absolute.
fn project_dir(project: Option<&Path>) -> Option<PathBuf> {
    project.and_then(|p| absolute(p).parent().map(Path::to_path_buf))
}

/// Where a linked file is now: relative to the project file first (so a
/// project folder can move as a whole), then the absolute path.
pub(crate) fn locate(link: &SourceLink, project: Option<&Path>) -> Option<PathBuf> {
    let relative = link
        .relative
        .as_ref()
        .zip(project_dir(project))
        .map(|(rel, dir)| lexical(&dir.join(rel)));
    relative
        .into_iter()
        .chain(std::iter::once(link.path.clone()))
        .find(|p| p.is_file())
}

/// A link to `file` (absolute). `None` for a path that is not UTF-8, which
/// the project file (JSON) cannot hold.
pub(crate) fn new_link(file: &Path, project: Option<&Path>, f: &SourceFile) -> Option<SourceLink> {
    file.to_str()?;
    Some(SourceLink {
        path: file.to_path_buf(),
        relative: project_dir(project).and_then(|dir| relative_to(file, &dir)),
        sha256: sha256_hex(&f.bytes),
        modified: f.modified.clone(),
    })
}

// ---------------------------------------------------------------- fingerprint

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// sha256 of a file, read in chunks (no size cap, little memory).
fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// RFC 3339 in UTC with nanoseconds, like `2026-11-24T10:00:00.000000000Z`.
/// `None` before 1970. The text is only compared, never parsed back.
pub(crate) fn rfc3339(t: SystemTime) -> Option<String> {
    let d = t.duration_since(UNIX_EPOCH).ok()?;
    let secs = d.as_secs();
    let (y, m, day) = civil_from_days(i64::try_from(secs / 86_400).ok()?);
    let rem = secs % 86_400;
    Some(format!(
        "{y:04}-{m:02}-{day:02}T{:02}:{:02}:{:02}.{:09}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        d.subsec_nanos()
    ))
}

/// Year, month and day of a day count since 1970-01-01 (Howard Hinnant's
/// days-to-civil algorithm, proleptic Gregorian calendar).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

// ---------------------------------------------------------------- rebuild

/// Geometry built from source bytes.
pub(crate) struct Rebuilt {
    pub paths: Vec<CutPath>,
    /// The page frame in local mm (SVG page, image pixel frame).
    pub frame: Rect,
    pub warnings: Vec<String>,
}

/// Builds the geometry of `source`'s kind from `bytes`: an SVG import, or a
/// trace with the source's stored trace settings.
pub(crate) fn rebuild(
    source: &Source,
    bytes: &[u8],
    blade_offset_mm: f64,
) -> Result<Rebuilt, ImportError> {
    match source {
        Source::Svg { .. } => {
            let import = import_svg(bytes, fonts::system_fonts())?;
            Ok(Rebuilt {
                frame: import.page_frame_mm,
                warnings: import.warnings.iter().map(warning_text).collect(),
                paths: import.paths,
            })
        }
        Source::Raster { trace, .. } => {
            let t = flatcraft_import::trace::trace(bytes, trace, blade_offset_mm)?;
            let (warnings, _) = trace_report(&t, trace.invert);
            Ok(Rebuilt {
                frame: Rect::new(0.0, 0.0, t.size_mm.0, t.size_mm.1),
                paths: t.paths,
                warnings,
            })
        }
    }
}

fn embedded_bytes(source: &Source) -> &[u8] {
    match source {
        Source::Svg { bytes, .. } | Source::Raster { bytes, .. } => bytes,
    }
}

// ---------------------------------------------------------------- reload

/// What reloading one object did.
enum Outcome {
    Reloaded,
    Unchanged,
    Missing,
    Failed,
    NoLink,
}

/// Reloads one object from `path`, or from where its link points. All or
/// nothing: the new values are built first and swapped in only when every
/// step worked. Warnings are prefixed with the object's name and id.
fn reload_one(
    s: &mut Session,
    id: ObjectId,
    path: Option<PathBuf>,
    warnings: &mut Vec<String>,
) -> Outcome {
    let blade = blade_offset_mm(s);
    let project = s.doc().path.clone();
    let Some(o) = s.doc().document.get(id) else {
        return Outcome::Failed;
    };
    let who = format!("{} (id {})", o.name, id.0);
    let Some(link) = &o.link else {
        warnings.push(format!("{who} has no linked file, so it was not reloaded"));
        return Outcome::NoLink;
    };
    let Some(path) = path.or_else(|| locate(link, project.as_deref())) else {
        return Outcome::Missing;
    };
    let file = match read_capped(&path) {
        Ok(f) => f,
        Err(ReadError::NotFound) => return Outcome::Missing,
        Err(e) => {
            warnings.push(format!(
                "{who}: {} ({}), so the object was kept as it was",
                e.text(),
                path.display()
            ));
            return Outcome::Failed;
        }
    };
    let Some(link) = new_link(&path, project.as_deref(), &file) else {
        warnings.push(format!(
            "{who}: the path {} cannot be stored in a project file, so the object was kept as it was",
            path.display()
        ));
        return Outcome::Failed;
    };
    if link.sha256 == sha256_hex(embedded_bytes(&o.source)) {
        if let Some(o) = s.doc_mut().document.get_mut(id) {
            o.link = Some(link);
        }
        return Outcome::Unchanged;
    }
    let rebuilt = match rebuild(&o.source, &file.bytes, blade) {
        Ok(r) => r,
        Err(e) => {
            warnings.push(format!(
                "{who}: cannot read the new version of {} ({e}), so the object was kept as it was",
                path.display()
            ));
            return Outcome::Failed;
        }
    };
    let Some(o) = s.doc_mut().document.get_mut(id) else {
        return Outcome::Failed;
    };
    for w in rebuilt.warnings {
        warnings.push(format!("{who}: {w}"));
    }
    match &mut o.source {
        Source::Svg { bytes, .. } | Source::Raster { bytes, .. } => *bytes = file.bytes,
    }
    o.set_geometry_anchored(rebuilt.paths, rebuilt.frame);
    o.link = Some(link);
    Outcome::Reloaded
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReloadParams {
    /// Object ids to reload. Empty (the default) reloads every linked object.
    #[serde(default)]
    pub ids: Vec<u64>,
}

fn run_reload(s: &mut Session, p: ReloadParams, _: Origin) -> Result<Value, CommandError> {
    let ids: Vec<ObjectId> = if p.ids.is_empty() {
        let doc = &s.doc().document;
        doc.objects
            .iter()
            .filter(|o| o.link.is_some())
            .map(|o| o.id)
            .collect()
    } else {
        let mut out = Vec::new();
        for &raw in &p.ids {
            let id = ObjectId(raw);
            check_exists(&s.doc().document, id)?;
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    };
    let (mut reloaded, mut unchanged, mut missing, mut failed) = (vec![], vec![], vec![], vec![]);
    let mut warnings = Vec::new();
    for id in ids {
        match reload_one(s, id, None, &mut warnings) {
            Outcome::Reloaded => reloaded.push(id),
            Outcome::Unchanged => unchanged.push(id),
            Outcome::Missing => missing.push(id),
            Outcome::Failed => failed.push(id),
            Outcome::NoLink => {}
        }
    }
    Ok(json!({
        "reloaded": ids_json(&reloaded),
        "unchanged": ids_json(&unchanged),
        "missing": ids_json(&missing),
        "failed": ids_json(&failed),
        "warnings": warnings,
    }))
}

command!(
    ObjectReload,
    id: "object.reload",
    title: "Reload from file",
    mutates: true,
    params: ReloadParams,
    run: run_reload,
    description: "Reads linked files again after they were changed in another app and rebuilds the objects (SVG import, or a trace with the object's stored trace settings). Position, scale, rotation, mirror, cut options, name and visibility stay; the object stays anchored to the file's page (SVG page, image frame), so unchanged shapes keep their place on the mat. A file is looked for relative to the project file first, then at its absolute path. ids: objects to reload; empty (default) means every linked object. One undo step for the whole call. The call never fails because of a file: a file that is gone is listed in missing, a file that cannot be read or is broken is listed in failed with a warning, and those objects keep their embedded copy unchanged. Returns {reloaded: [id], unchanged: [id] (same content), missing: [id], failed: [id], warnings: [text]}.",
);

// ---------------------------------------------------------------- relink

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelinkParams {
    /// Id of the object to point to another file.
    pub id: u64,
    /// The file's new location. It must exist and be of the object's kind: .svg or .svgz for SVG objects, .png, .jpg, .jpeg, .webp or .gif for traced images.
    pub path: PathBuf,
}

fn run_relink(s: &mut Session, p: RelinkParams, _: Origin) -> Result<Value, CommandError> {
    let id = ObjectId(p.id);
    check_exists(&s.doc().document, id)?;
    let kind = file_kind(&p.path)?;
    let path = absolute(&p.path);
    let o = s.doc().document.get(id).expect("checked above");
    match (&o.source, kind) {
        (Source::Svg { .. }, FileKind::Raster) => {
            return Err(CommandError::InvalidParams(format!(
                "object {} is an SVG; pick an .svg or .svgz file",
                id.0
            )));
        }
        (Source::Raster { .. }, FileKind::Svg) => {
            return Err(CommandError::InvalidParams(format!(
                "object {} is a traced image; pick a .png, .jpg, .jpeg, .webp or .gif file",
                id.0
            )));
        }
        _ => {}
    }
    if !path.is_file() {
        return Err(CommandError::NotFound(format!("file {}", path.display())));
    }
    if o.link.is_none() {
        // Linking an unlinked object; `reload_one` fills in the rest.
        let o = s.doc_mut().document.get_mut(id).expect("checked above");
        o.link = Some(SourceLink {
            path: path.clone(),
            relative: None,
            sha256: String::new(),
            modified: None,
        });
    }
    let mut warnings = Vec::new();
    let (mut reloaded, mut unchanged) = (vec![], vec![]);
    match reload_one(s, id, Some(path.clone()), &mut warnings) {
        Outcome::Reloaded => reloaded.push(id),
        Outcome::Unchanged => unchanged.push(id),
        Outcome::Missing => {
            return Err(CommandError::NotFound(format!("file {}", path.display())));
        }
        Outcome::Failed | Outcome::NoLink => {
            return Err(CommandError::Failed(warnings.join("; ")));
        }
    }
    let also_found = siblings(s, id, &path);
    Ok(json!({
        "reloaded": ids_json(&reloaded),
        "unchanged": ids_json(&unchanged),
        "also_found": also_found,
        "warnings": warnings,
    }))
}

/// Other objects whose file is missing but whose file name exists in the
/// folder of `path`.
fn siblings(s: &Session, except: ObjectId, path: &Path) -> Vec<Value> {
    let Some(folder) = path.parent() else {
        return Vec::new();
    };
    let project = s.doc().path.as_deref();
    s.doc()
        .document
        .objects
        .iter()
        .filter(|o| o.id != except)
        .filter_map(|o| {
            let link = o.link.as_ref()?;
            if locate(link, project).is_some() {
                return None;
            }
            let candidate = folder.join(link.path.file_name()?);
            candidate
                .is_file()
                .then(|| json!({ "id": o.id.0, "path": candidate }))
        })
        .collect()
}

command!(
    ObjectRelink,
    id: "object.relink",
    title: "Reconnect file",
    mutates: true,
    params: RelinkParams,
    run: run_relink,
    description: "Points an object's link to a file at a new location (after it was moved or renamed) and reloads it from there, keeping position, scale, rotation, mirror, cut options, name and visibility. An unlinked object gets a link. The file must exist and match the object's kind (.svg/.svgz for SVG objects, .png/.jpg/.jpeg/.webp/.gif for traced images); a file that cannot be read or parsed fails the call and changes nothing. One undo step. Returns {reloaded: [id] (the content changed), unchanged: [id] (same content, only the link moved), also_found: [{id, path}] (other objects whose file is missing but whose file name exists in the new folder; relink them too), warnings: [text]}.",
);

// ---------------------------------------------------------------- unlink

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnlinkParams {
    /// Object ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
}

fn run_unlink(s: &mut Session, p: UnlinkParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let doc = &mut s.doc_mut().document;
    for id in &ids {
        if let Some(o) = doc.get_mut(*id) {
            o.link = None;
        }
    }
    Ok(json!({ "ids": ids_json(&ids) }))
}

command!(
    ObjectUnlink,
    id: "object.unlink",
    title: "Unlink file",
    mutates: true,
    params: UnlinkParams,
    run: run_unlink,
    description: "Forgets the file an object was imported from. The object keeps its embedded copy and stays as it is; it just no longer reloads or reports changes. One undo step. An empty ids list means the current selection. Returns {ids}.",
);

// ---------------------------------------------------------------- check

fn run_check(s: &mut Session, _: NoParams, _: Origin) -> Result<Value, CommandError> {
    let project = s.doc().path.as_deref();
    let (mut changed, mut missing) = (Vec::new(), Vec::new());
    for o in &s.doc().document.objects {
        let Some(link) = &o.link else { continue };
        let Some(path) = locate(link, project) else {
            missing.push(o.id);
            continue;
        };
        let modified = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(rfc3339);
        if modified.is_some() && modified == link.modified {
            continue;
        }
        // The time changed: compare the content with the embedded copy. A
        // file that exists but cannot be read counts as changed; reloading
        // it then says why.
        let same = sha256_file(&path).is_ok_and(|sha| sha == sha256_hex(embedded_bytes(&o.source)));
        if !same {
            changed.push(o.id);
        }
    }
    Ok(json!({ "changed": ids_json(&changed), "missing": ids_json(&missing) }))
}

command!(
    DocumentCheckSources,
    id: "document.check_sources",
    title: "Check linked files",
    mutates: false,
    params: NoParams,
    run: run_check,
    description: "Checks the files the objects of the active document were imported from, without changing anything. A file is looked for relative to the project file first, then at its absolute path. Compares the modification time first and the content (sha256) only when the time changed. Returns {changed: [id] (reload them with object.reload), missing: [id] (point them to the new place with object.relink)}.",
);

// ---------------------------------------------------------------- save

/// New links for saving `doc` as `new_project`: each file is looked for from
/// where the project was (`old_project`), and the relative path is made
/// against the new folder. A missing file keeps its last known path; its
/// relative path is carried over to the new folder. Returns the old links of
/// the changed objects, so a failed save can put them back.
pub(crate) fn relink_for_save(
    doc: &mut Document,
    old_project: Option<&Path>,
    new_project: &Path,
) -> Vec<(ObjectId, Option<SourceLink>)> {
    let Some(new_dir) = project_dir(Some(new_project)) else {
        return Vec::new();
    };
    let old_dir = project_dir(old_project);
    let mut previous = Vec::new();
    for o in &mut doc.objects {
        let Some(link) = &o.link else { continue };
        let mut next = link.clone();
        if let Some(found) = locate(link, old_project) {
            next.relative = relative_to(&found, &new_dir);
            next.path = found;
        } else {
            let candidate = link
                .relative
                .as_ref()
                .zip(old_dir.as_ref())
                .map(|(rel, dir)| lexical(&dir.join(rel)))
                .unwrap_or_else(|| link.path.clone());
            next.relative = relative_to(&candidate, &new_dir);
        }
        if &next != link {
            previous.push((o.id, o.link.replace(next)));
        }
    }
    previous
}

/// Puts back the links `relink_for_save` replaced.
pub(crate) fn restore_links(doc: &mut Document, previous: Vec<(ObjectId, Option<SourceLink>)>) {
    for (id, link) in previous {
        if let Some(o) = doc.get_mut(id) {
            o.link = link;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn rfc3339_of_known_times() {
        assert_eq!(
            rfc3339(UNIX_EPOCH).as_deref(),
            Some("1970-01-01T00:00:00.000000000Z")
        );
        let t = UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789);
        assert_eq!(
            rfc3339(t).as_deref(),
            Some("2023-11-14T22:13:20.123456789Z")
        );
        // A leap day.
        let t = UNIX_EPOCH + Duration::from_secs(951_782_400);
        assert_eq!(
            rfc3339(t).as_deref(),
            Some("2000-02-29T00:00:00.000000000Z")
        );
        assert_eq!(rfc3339(UNIX_EPOCH - Duration::from_secs(1)), None);
    }

    #[test]
    fn relative_paths() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            relative_to(&p("/a/b/art/x.svg"), &p("/a/b")),
            Some(p("art/x.svg"))
        );
        assert_eq!(
            relative_to(&p("/a/two/x.svg"), &p("/a/three")),
            Some(p("../two/x.svg"))
        );
        assert_eq!(
            relative_to(&p("/x.svg"), &p("/a/b")),
            Some(p("../../x.svg"))
        );
        assert_eq!(relative_to(&p("x.svg"), &p("/a")), None);
        // Always `/`, whatever the platform.
        let rel = relative_to(&p("/a/two/x.svg"), &p("/a/three")).unwrap();
        assert_eq!(rel.to_str(), Some("../two/x.svg"));
    }

    #[test]
    fn sha256_of_abc() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
