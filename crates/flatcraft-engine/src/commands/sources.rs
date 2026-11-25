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
    /// A folder, FIFO, device or socket: never opened, a FIFO would block.
    NotAFile,
    TooLarge,
    Io(std::io::Error),
}

impl ReadError {
    /// Text for people and agents, without the path.
    pub(crate) fn text(&self) -> String {
        match self {
            ReadError::NotFound => "the file was not found".into(),
            ReadError::NotAFile => "it is not a regular file".into(),
            ReadError::TooLarge => "the file is larger than 50 MB".into(),
            ReadError::Io(e) => clip(format!("the file cannot be read: {e}")),
        }
    }
}

/// A file read with the import size cap, plus its modification time.
pub(crate) struct SourceFile {
    pub bytes: Vec<u8>,
    pub modified: Option<String>,
}

/// Reads at most 50 MB; a larger file is refused without reading it all.
/// Only regular files are opened (checked before opening, so a FIFO never
/// blocks the session).
pub(crate) fn read_capped(path: &Path) -> Result<SourceFile, ReadError> {
    let meta = std::fs::metadata(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ReadError::NotFound,
        _ => ReadError::Io(e),
    })?;
    if !meta.is_file() {
        return Err(ReadError::NotAFile);
    }
    if meta.len() > MAX_SOURCE_BYTES {
        return Err(ReadError::TooLarge);
    }
    let file = std::fs::File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ReadError::NotFound,
        _ => ReadError::Io(e),
    })?;
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

/// Longest error text from a parser that goes into a warning, in chars.
const MAX_DETAIL_CHARS: usize = 200;

/// Cuts a text to `MAX_DETAIL_CHARS`, so a parser cannot flood the result.
pub(crate) fn clip(text: String) -> String {
    match text.char_indices().nth(MAX_DETAIL_CHARS) {
        Some((cut, _)) => format!("{}...", &text[..cut]),
        None => text,
    }
}

/// The kind of object a file becomes, by its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Svg,
    Raster,
}

/// One file type `object.import` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportType {
    /// Name for people, as in file dialog filters.
    pub name: &'static str,
    /// Extensions without the dot, lowercase.
    pub extensions: &'static [&'static str],
    pub kind: FileKind,
}

/// Every file type `object.import` takes. The app builds its file dialog
/// filters and drop messages from this list.
pub const IMPORT_TYPES: &[ImportType] = &[
    ImportType {
        name: "SVG",
        extensions: &["svg", "svgz"],
        kind: FileKind::Svg,
    },
    ImportType {
        name: "PNG",
        extensions: &["png"],
        kind: FileKind::Raster,
    },
    ImportType {
        name: "JPEG",
        extensions: &["jpg", "jpeg"],
        kind: FileKind::Raster,
    },
    ImportType {
        name: "WebP",
        extensions: &["webp"],
        kind: FileKind::Raster,
    },
    ImportType {
        name: "GIF",
        extensions: &["gif"],
        kind: FileKind::Raster,
    },
];

/// The kind of file an object is rebuilt from.
pub(crate) fn source_kind(source: &Source) -> FileKind {
    match source {
        Source::Svg { .. } => FileKind::Svg,
        Source::Raster { .. } => FileKind::Raster,
    }
}

/// The lowercase extension of `path`, or "".
fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// The import type of `path` by its extension (case does not matter).
fn import_type(path: &Path) -> Option<&'static ImportType> {
    let ext = extension(path);
    IMPORT_TYPES
        .iter()
        .find(|t| t.extensions.contains(&ext.as_str()))
}

/// True if `object.import` takes this file, by its extension.
pub fn is_importable(path: &Path) -> bool {
    import_type(path).is_some()
}

/// The file kind by extension (case does not matter).
pub(crate) fn file_kind(path: &Path) -> Result<FileKind, CommandError> {
    if let Some(t) = import_type(path) {
        return Ok(t.kind);
    }
    let ext = extension(path);
    let shown = if ext.is_empty() {
        "no extension".to_string()
    } else {
        format!("'.{ext}'")
    };
    let supported: Vec<String> = IMPORT_TYPES
        .iter()
        .flat_map(|t| t.extensions)
        .map(|e| format!(".{e}"))
        .collect();
    Err(CommandError::InvalidParams(format!(
        "unsupported file type ({shown}). Supported types: {}",
        supported.join(", ")
    )))
}

/// An absolute path without `.` and `..`, without touching the file system
/// (no symlink resolution, so it compares with what the user gave).
pub(crate) fn absolute(path: &Path) -> PathBuf {
    lexical(&std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// True if two path parts are the same. Windows drive letters and
/// prefixes compare without case (`C:` is `c:`).
fn same_part(a: &Component, b: &Component) -> bool {
    match (a, b) {
        (Component::Prefix(x), Component::Prefix(y)) => {
            x.as_os_str().eq_ignore_ascii_case(y.as_os_str())
        }
        _ => a == b,
    }
}

/// True if two absolute folders are the same (lexically).
fn same_dir(a: &Path, b: &Path) -> bool {
    let (a, b) = (lexical(a), lexical(b));
    a.components().count() == b.components().count()
        && a.components()
            .zip(b.components())
            .all(|(x, y)| same_part(&x, &y))
}

/// `file` relative to the folder `dir`, both absolute. `None` when they
/// have no common root (another drive on Windows) or a part is not UTF-8.
/// The parts are joined with `/` on every platform, so a project folder
/// moved between macOS, Linux and Windows still finds its files.
pub(crate) fn relative_to(file: &Path, dir: &Path) -> Option<PathBuf> {
    let file: Vec<Component> = file.components().collect();
    let dir: Vec<Component> = dir.components().collect();
    let common = file
        .iter()
        .zip(&dir)
        .take_while(|(a, b)| same_part(a, b))
        .count();
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
pub(crate) fn project_dir(project: Option<&Path>) -> Option<PathBuf> {
    project.and_then(|p| absolute(p).parent().map(Path::to_path_buf))
}

/// The relative path of a link resolved against `dir`, but only when it was
/// made for that folder (`anchor_dir`).
fn anchored_relative(link: &SourceLink, dir: Option<&Path>) -> Option<PathBuf> {
    match (&link.relative, &link.anchor_dir, dir) {
        (Some(rel), Some(anchor), Some(dir)) if same_dir(anchor, dir) => {
            Some(lexical(&dir.join(rel)))
        }
        _ => None,
    }
}

/// The link's absolute path. A relative one (a hand-edited project) is
/// taken against the project folder, never the working directory.
fn absolute_path(link: &SourceLink, dir: Option<&Path>) -> Option<PathBuf> {
    if link.path.is_absolute() {
        Some(lexical(&link.path))
    } else {
        dir.map(|d| lexical(&d.join(&link.path)))
    }
}

/// The places a linked file may be, in order: the path relative to the
/// project file, but only while the project file is in the folder that
/// path was made for (`anchor_dir`); then the absolute path. Only names
/// with an extension of `kind` count, so a crafted project cannot make
/// FlatCraft read any other file.
fn candidates(link: &SourceLink, project: Option<&Path>, kind: FileKind) -> Vec<PathBuf> {
    let dir = project_dir(project);
    anchored_relative(link, dir.as_deref())
        .into_iter()
        .chain(absolute_path(link, dir.as_deref()))
        .filter(|p| file_kind(p).ok() == Some(kind))
        .collect()
}

/// Where a linked file is now: the first candidate that is a regular file,
/// so a folder or FIFO with the file's name is never taken for it.
pub(crate) fn locate(link: &SourceLink, project: Option<&Path>, kind: FileKind) -> Option<PathBuf> {
    candidates(link, project, kind)
        .into_iter()
        .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file()))
}

/// For a link `locate` did not find: the first candidate that exists but is
/// no regular file (a folder or FIFO), to report it as unreadable instead
/// of missing. Never opened.
fn locate_other(link: &SourceLink, project: Option<&Path>, kind: FileKind) -> Option<PathBuf> {
    candidates(link, project, kind)
        .into_iter()
        .find(|p| std::fs::metadata(p).is_ok())
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
        anchor_dir: project_dir(project),
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
    #[cfg(test)]
    tests::HASHED.with(|n| n.set(n.get() + 1));
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
    /// Same content. `true` when the link moved (a real change); a fresh
    /// modification time alone is `false`.
    Unchanged(bool),
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
    let kind = source_kind(&o.source);
    let Some(path) = path
        .or_else(|| locate(link, project.as_deref(), kind))
        .or_else(|| locate_other(link, project.as_deref(), kind))
    else {
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
        let moved = o
            .link
            .as_ref()
            .map(|l| (&l.path, &l.relative, &l.anchor_dir))
            != Some((&link.path, &link.relative, &link.anchor_dir));
        if let Some(o) = s.doc_mut().document.get_mut(id) {
            o.link = Some(link);
        }
        return Outcome::Unchanged(moved);
    }
    let rebuilt = match rebuild(&o.source, &file.bytes, blade) {
        Ok(r) => r,
        Err(e) => {
            warnings.push(format!(
                "{who}: cannot read the new version of {} ({}), so the object was kept as it was",
                path.display(),
                clip(e.to_string())
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
    let mut moved_any = false;
    for id in ids {
        match reload_one(s, id, None, &mut warnings) {
            Outcome::Reloaded => reloaded.push(id),
            Outcome::Unchanged(moved) => {
                moved_any |= moved;
                unchanged.push(id);
            }
            Outcome::Missing => missing.push(id),
            Outcome::Failed => failed.push(id),
            Outcome::NoLink => {}
        }
    }
    if reloaded.is_empty() && !moved_any {
        // Nothing the user would undo. A refreshed modification time stays
        // and is saved with the next real change.
        s.mark_no_user_change();
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
    description: "Reads linked files again after they were changed in another app and rebuilds the objects (SVG import, or a trace with the object's stored trace settings). Position, scale, rotation, mirror, cut options, name and visibility stay; the object stays anchored to the file's page (SVG page, image frame), so unchanged shapes keep their place on the mat. A file is looked for relative to the project file first (while the project is in the folder that path was made for), then at its absolute path; only files of the object's type (.svg/.svgz or an image type) count. ids: objects to reload; empty (default) means every linked object. One undo step for the whole call; a call that changed nothing (all unchanged, missing or failed) records no undo step and does not mark the document as changed. The call never fails because of a file: a file that is gone is listed in missing, a file that cannot be read or is broken is listed in failed with a warning, and those objects keep their embedded copy unchanged. Returns {reloaded: [id], unchanged: [id] (same content), missing: [id], failed: [id], warnings: [text]}.",
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
            anchor_dir: None,
        });
    }
    let mut warnings = Vec::new();
    let (mut reloaded, mut unchanged) = (vec![], vec![]);
    match reload_one(s, id, Some(path.clone()), &mut warnings) {
        Outcome::Reloaded => reloaded.push(id),
        Outcome::Unchanged(_) => unchanged.push(id),
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
            let kind = source_kind(&o.source);
            if locate(link, project, kind).is_some() {
                return None;
            }
            let candidate = folder.join(link.path.file_name()?);
            (file_kind(&candidate).ok() == Some(kind) && candidate.is_file())
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

/// What the change check found for one linked object.
#[derive(Debug, PartialEq, Eq)]
enum Check {
    Same,
    Changed,
    Missing,
    Unreadable,
}

/// The content the file must have to count as unchanged.
enum Expected {
    /// sha256 of the embedded bytes, from the link.
    Sha(String),
    /// The embedded bytes, when the link holds no usable sha256.
    Bytes(Vec<u8>),
}

/// What the stat step found for one link.
enum Probe {
    Known(Check),
    /// Same size, other time: only the content can tell.
    Hash {
        path: PathBuf,
        expected: Expected,
    },
}

/// True for 64 lower-case hex digits.
fn is_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Stats one link, without reading any file: the modification time first,
/// then the size. Files that are too large or not regular files are never
/// read.
fn probe(link: &SourceLink, source: &Source, project: Option<&Path>) -> Probe {
    let kind = source_kind(source);
    let Some(path) = locate(link, project, kind) else {
        return Probe::Known(match locate_other(link, project, kind) {
            Some(_) => Check::Unreadable,
            None => Check::Missing,
        });
    };
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Probe::Known(Check::Missing),
        Err(_) => return Probe::Known(Check::Unreadable),
    };
    if !meta.is_file() || meta.len() > MAX_SOURCE_BYTES {
        return Probe::Known(Check::Unreadable);
    }
    let modified = meta.modified().ok().and_then(rfc3339);
    if modified.is_some() && modified == link.modified {
        return Probe::Known(Check::Same);
    }
    let embedded = embedded_bytes(source);
    if meta.len() != embedded.len() as u64 {
        return Probe::Known(Check::Changed);
    }
    let expected = if is_sha256(&link.sha256) {
        Expected::Sha(link.sha256.clone())
    } else {
        Expected::Bytes(embedded.to_vec())
    };
    Probe::Hash { path, expected }
}

/// The first step of a change check, taken under the session lock: it only
/// stats files. [`SourceCheck::run`] hashes what is left without the lock,
/// and [`accept_check`] takes the result only while the document is
/// unchanged.
pub struct SourceCheck {
    doc_id: u64,
    revision: u64,
    probes: Vec<(ObjectId, Probe)>,
}

/// What a change check found, for the document `doc_id` at `revision`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub doc_id: u64,
    pub revision: u64,
    pub changed: Vec<ObjectId>,
    pub missing: Vec<ObjectId>,
    pub unreadable: Vec<ObjectId>,
}

impl CheckResult {
    /// The result as `document.check_sources` returns it.
    pub fn to_json(&self) -> Value {
        json!({
            "changed": ids_json(&self.changed),
            "missing": ids_json(&self.missing),
            "unreadable": ids_json(&self.unreadable),
        })
    }
}

/// Stats the linked files of the active document (see [`SourceCheck`]).
pub fn plan_check(s: &Session) -> SourceCheck {
    let st = s.doc();
    let project = st.path.as_deref();
    let probes = st
        .document
        .objects
        .iter()
        .filter_map(|o| Some((o.id, probe(o.link.as_ref()?, &o.source, project))))
        .collect();
    SourceCheck {
        doc_id: st.id(),
        revision: st.revision(),
        probes,
    }
}

impl SourceCheck {
    /// Hashes the files whose time changed but whose size did not. Call it
    /// without the session lock.
    pub fn run(self) -> CheckResult {
        let mut out = CheckResult {
            doc_id: self.doc_id,
            revision: self.revision,
            changed: Vec::new(),
            missing: Vec::new(),
            unreadable: Vec::new(),
        };
        for (id, probe) in self.probes {
            let check = match probe {
                Probe::Known(c) => c,
                Probe::Hash { path, expected } => {
                    let expected = match expected {
                        Expected::Sha(sha) => sha,
                        Expected::Bytes(bytes) => sha256_hex(&bytes),
                    };
                    match sha256_file(&path) {
                        Ok(sha) if sha == expected => Check::Same,
                        Ok(_) => Check::Changed,
                        Err(_) => Check::Unreadable,
                    }
                }
            };
            match check {
                Check::Same => {}
                Check::Changed => out.changed.push(id),
                Check::Missing => out.missing.push(id),
                Check::Unreadable => out.unreadable.push(id),
            }
        }
        out
    }
}

/// The result as JSON while its document is still open and unchanged since
/// [`plan_check`]; `None` when the check is stale and must run again.
pub fn accept_check(s: &Session, result: &CheckResult) -> Option<Value> {
    let index = s.doc_index(result.doc_id).ok()?;
    (s.docs[index].revision() == result.revision).then(|| result.to_json())
}

/// The change check. Only `changed` is meant for an automatic reload
/// (`Preferences::auto_reload`): `missing` and `unreadable` would fail again
/// on every check and must wait for the user. An agent's call holds the
/// session lock for the whole check; the GUI uses the three steps.
fn run_check(s: &mut Session, _: NoParams, _: Origin) -> Result<Value, CommandError> {
    Ok(plan_check(s).run().to_json())
}

command!(
    DocumentCheckSources,
    id: "document.check_sources",
    title: "Check linked files",
    mutates: false,
    params: NoParams,
    run: run_check,
    description: "Checks the files the objects of the active document were imported from, without changing anything. A file is looked for relative to the project file first, then at its absolute path; only files of the object's type count. Compares the modification time first, then the size, and the content (sha256) only when the size is the same. Returns {changed: [id] (the content differs; only these are meant for an automatic object.reload), missing: [id] (point them to the new place with object.relink), unreadable: [id] (the file exists but is larger than 50 MB, not a regular file or cannot be read; reloading it would fail)}.",
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
        if let Some(found) = locate(link, old_project, source_kind(&o.source)) {
            next.relative = relative_to(&found, &new_dir);
            next.path = found;
        } else {
            // Missing: carry the relative path over when it was made for
            // the old folder, else use the last known absolute path.
            let candidate = anchored_relative(link, old_dir.as_deref())
                .or_else(|| absolute_path(link, old_dir.as_deref()))
                .unwrap_or_else(|| link.path.clone());
            next.relative = relative_to(&candidate, &new_dir);
        }
        next.anchor_dir = Some(new_dir.clone());
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
    use std::cell::Cell;
    use std::time::Duration;

    thread_local! {
        /// How many files `sha256_file` hashed on this thread.
        pub(super) static HASHED: Cell<usize> = const { Cell::new(0) };
    }

    fn session(dir: &Path) -> Session {
        let profiles = flatcraft_device::ProfileStore::load(dir).unwrap();
        let prefs = crate::prefs::Preferences {
            output_dir: Some(dir.join("output")),
            ..Default::default()
        };
        Session::new(profiles, prefs, false, dir.to_path_buf())
    }

    fn check(s: &mut Session) -> Value {
        s.execute("document.check_sources", json!({}), Origin::User)
            .unwrap()
    }

    const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="20mm" viewBox="0 0 20 20"><rect x="2" y="2" width="5" height="5"/></svg>"#;

    fn later(path: &Path) {
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(SystemTime::now() + Duration::from_secs(60))
            .unwrap();
    }

    #[test]
    fn check_hashes_only_files_of_the_same_size() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(dir.path());
        let files = ["huge", "longer", "same"].map(|n| dir.path().join(format!("{n}.svg")));
        let mut ids = Vec::new();
        for f in &files {
            std::fs::write(f, SVG).unwrap();
            let r = s
                .execute("object.import", json!({ "path": f }), Origin::User)
                .unwrap();
            ids.push(r["id"].as_u64().unwrap());
        }
        // Sparse, far over the cap: never read.
        std::fs::File::options()
            .write(true)
            .open(&files[0])
            .unwrap()
            .set_len(4 * MAX_SOURCE_BYTES)
            .unwrap();
        std::fs::write(&files[1], format!("{SVG}\n")).unwrap();
        for f in &files {
            later(f);
        }
        HASHED.with(|n| n.set(0));
        let r = check(&mut s);
        assert_eq!(r["unreadable"], json!([ids[0]]), "{r}");
        assert_eq!(r["changed"], json!([ids[1]]), "{r}");
        assert_eq!(
            HASHED.with(Cell::get),
            1,
            "only the same-size file is hashed"
        );

        // Same size, other content: hashed and changed.
        std::fs::write(&files[2], SVG.replace("x=\"2\"", "x=\"3\"")).unwrap();
        later(&files[2]);
        HASHED.with(|n| n.set(0));
        let r = check(&mut s);
        assert_eq!(r["changed"], json!([ids[1], ids[2]]), "{r}");
        assert_eq!(HASHED.with(Cell::get), 1);
    }

    /// A linked object whose file has the same size as the embedded copy
    /// but other content and a newer time, so only a hash can tell.
    fn same_size_change(dir: &Path, s: &mut Session) -> u64 {
        let f = dir.join("same.svg");
        std::fs::write(&f, SVG).unwrap();
        let r = s
            .execute("object.import", json!({ "path": f }), Origin::User)
            .unwrap();
        std::fs::write(&f, SVG.replace("x=\"2\"", "x=\"3\"")).unwrap();
        later(&f);
        r["id"].as_u64().unwrap()
    }

    #[test]
    fn planning_a_check_never_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(dir.path());
        let id = same_size_change(dir.path(), &mut s);
        HASHED.with(|n| n.set(0));
        let plan = plan_check(&s);
        assert_eq!(HASHED.with(Cell::get), 0, "the lock step only stats");
        let result = plan.run();
        assert_eq!(HASHED.with(Cell::get), 1);
        assert_eq!(result.changed, vec![ObjectId(id)]);
        assert_eq!(accept_check(&s, &result), Some(result.to_json()));
    }

    #[test]
    fn a_check_of_a_changed_document_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(dir.path());
        let id = same_size_change(dir.path(), &mut s);
        let plan = plan_check(&s);
        s.execute(
            "object.move",
            json!({ "ids": [id], "dx_mm": 1.0 }),
            Origin::User,
        )
        .unwrap();
        let result = plan.run();
        assert_eq!(accept_check(&s, &result), None);
    }

    #[test]
    fn clip_caps_parser_text() {
        assert_eq!(clip("short".into()), "short");
        let c = clip("\u{e9}".repeat(300));
        assert_eq!(c.chars().count(), MAX_DETAIL_CHARS + 3);
        assert!(c.ends_with("..."));
    }

    #[cfg(windows)]
    #[test]
    fn drive_letters_compare_without_case() {
        let rel = relative_to(Path::new(r"C:\a\x.svg"), Path::new(r"c:\a\b")).unwrap();
        assert_eq!(rel.to_str(), Some("../x.svg"));
        assert!(same_dir(Path::new(r"C:\a"), Path::new(r"c:\a")));
    }

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
