//! Project file: a zip archive, written atomically.
//!
//! A `.flatcraft` file holds, in this order:
//! - `mimetype`: [`MIMETYPE`], first and stored uncompressed, so a tool
//!   can tell the file type from a fixed offset (as ODF does);
//! - `project.json`: the project as pretty JSON (deflated). Each source
//!   names its file in the archive (`"file": "sources/<sha256>.<ext>"`)
//!   instead of holding the bytes;
//! - `sources/<sha256>.<ext>`: each source file as raw bytes, once per
//!   content, so linked copies and duplicates share one entry. SVG is
//!   deflated; pictures (PNG, JPEG, WebP, GIF) are already compressed and
//!   stored as they are;
//! - `thumbnail.png` (optional): a picture of the mat with its objects,
//!   [`THUMBNAIL_PX`] on the larger side, for the Start page.
//!
//! [`FORMAT_VERSION`] 1 is the only version until the first release: a
//! newer version is refused, and plain-JSON projects of development
//! builds are refused with a message of their own.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::document::{Document, ObjectId, Source};
use crate::limits;

pub const FORMAT_VERSION: u32 = 1;

/// The content of the `mimetype` entry.
pub const MIMETYPE: &str = "application/vnd.flatcraft+zip";

/// The larger side of `thumbnail.png`, in pixels.
pub const THUMBNAIL_PX: u32 = 512;

const MIMETYPE_ENTRY: &str = "mimetype";
const PROJECT_ENTRY: &str = "project.json";
const THUMBNAIL_ENTRY: &str = "thumbnail.png";
const SOURCES_DIR: &str = "sources/";

/// Keys of an object that linked copies share; the project file keeps them
/// once per group, in `link_groups`.
const SHARED_KEYS: [&str; 5] = ["source", "cut", "link", "local_bounds_mm", "frame_mm"];

/// Keys of an object's transform that linked copies share.
const SHARED_TRANSFORM_KEYS: [&str; 2] = ["scale_x", "scale_y"];

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("Could not read or write the project file: {0}")]
    Io(#[from] std::io::Error),
    #[error("The project file is not valid: {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "This project was saved by a newer FlatCraft (format {found}, this version reads up to {supported}). Please update FlatCraft."
    )]
    NewerFormat { found: u32, supported: u32 },
    #[error(
        "This project was saved by a development build of FlatCraft as plain JSON. This version cannot open it. Please import its designs again."
    )]
    PlainJson,
    #[error("This file is not a FlatCraft project.")]
    NotAProject,
    #[error("The project file is damaged: {0}")]
    Damaged(String),
    #[error("The project file is not valid: it holds an entry with an unsafe name ({0}).")]
    UnsafeEntry(String),
    #[error("The project file is too large to open safely: {what} is over {limit_mb} MB unpacked.")]
    TooLarge { what: String, limit_mb: u64 },
    #[error("The project file is not valid: the source file of '{name}' ({file}) is missing.")]
    MissingSource { name: String, file: String },
    #[error(
        "The project file is damaged: the source file of '{name}' ({file}) does not match its checksum."
    )]
    DamagedSource { name: String, file: String },
    #[error(
        "The project file is not valid: '{name}' is a linked copy of group {group}, but that group is missing."
    )]
    MissingGroup { name: String, group: u64 },
}

impl From<zip::result::ZipError> for ProjectError {
    fn from(e: zip::result::ZipError) -> Self {
        match e {
            zip::result::ZipError::Io(io) => ProjectError::Io(io),
            other => ProjectError::Damaged(other.to_string()),
        }
    }
}

// ---------------------------------------------------------------- archive

/// Size limits for reading a project file (see [`crate::limits`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveLimits {
    /// Largest entry under `sources/`, bytes.
    pub max_source_bytes: u64,
    /// Largest `project.json`, bytes.
    pub max_project_json_bytes: u64,
    /// Largest `thumbnail.png`, bytes.
    pub max_thumbnail_bytes: u64,
    /// Most bytes all entries together may unpack to.
    pub max_total_bytes: u64,
    /// Most entries.
    pub max_entries: usize,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: limits::MAX_SOURCE_BYTES,
            max_project_json_bytes: limits::MAX_PROJECT_JSON_BYTES,
            max_thumbnail_bytes: limits::MAX_THUMBNAIL_BYTES,
            max_total_bytes: limits::MAX_PROJECT_TOTAL_BYTES,
            max_entries: limits::MAX_PROJECT_ENTRIES,
        }
    }
}

/// The parts of a project file. [`to_archive`] makes one from a document
/// and [`load_archive`] makes a document from one; tests and tools use it
/// to look into or change a saved project.
#[derive(Debug, Clone, PartialEq)]
pub struct Archive {
    /// `project.json`.
    pub project: serde_json::Value,
    /// The entries under `sources/`, by full entry name.
    pub sources: BTreeMap<String, Arc<[u8]>>,
    /// `thumbnail.png`, if there is one.
    pub thumbnail: Option<Vec<u8>>,
}

impl Archive {
    /// The zip file: `mimetype` first and stored, then `project.json`,
    /// the sources by name and the thumbnail. The same archive always
    /// gives the same bytes (no time stamps).
    pub fn to_bytes(&self) -> Result<Vec<u8>, ProjectError> {
        use zip::CompressionMethod::{Deflated, Stored};
        use zip::write::SimpleFileOptions;
        let opts = |m| SimpleFileOptions::default().compression_method(m);
        let mut out = std::io::Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(&mut out);
        z.start_file(MIMETYPE_ENTRY, opts(Stored))?;
        z.write_all(MIMETYPE.as_bytes())?;
        z.start_file(PROJECT_ENTRY, opts(Deflated))?;
        let mut json = serde_json::to_vec_pretty(&self.project)?;
        json.push(b'\n');
        z.write_all(&json)?;
        for (name, bytes) in &self.sources {
            let method = if name.ends_with(".svg") {
                Deflated
            } else {
                Stored
            };
            z.start_file(name.as_str(), opts(method))?;
            z.write_all(bytes)?;
        }
        if let Some(png) = &self.thumbnail {
            z.start_file(THUMBNAIL_ENTRY, opts(Stored))?;
            z.write_all(png)?;
        }
        z.finish()?;
        Ok(out.into_inner())
    }

    /// Writes the archive crash-safely ([`crate::fsio::write_atomic`]).
    pub fn write(&self, path: &Path) -> Result<(), ProjectError> {
        crate::fsio::write_atomic(path, &self.to_bytes()?)?;
        Ok(())
    }

    /// Reads the project file at `path`, lets `change` change it and
    /// writes it back (for tools and tests that change a saved project).
    pub fn edit(path: &Path, change: impl FnOnce(&mut Archive)) -> Result<(), ProjectError> {
        let mut archive = Self::read(path)?;
        change(&mut archive);
        archive.write(path)
    }

    /// Puts the bytes of `source` into the archive (once per content) and
    /// returns its entry name, the `file` that project.json refers to.
    pub fn add_source(&mut self, source: &Source) -> String {
        let file = source_file(source);
        self.sources
            .entry(file.clone())
            .or_insert_with(|| source_bytes(source).clone());
        file
    }

    /// Reads a project file with the default limits.
    pub fn read(path: &Path) -> Result<Self, ProjectError> {
        Self::from_bytes(&read_file(path)?)
    }

    /// Reads a project file's bytes with the default limits.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProjectError> {
        Self::from_bytes_with(bytes, &ArchiveLimits::default())
    }

    /// Reads a project file's bytes. Refused: plain JSON (a development
    /// build's project), anything that is not a zip with the FlatCraft
    /// `mimetype` and a `project.json`, entry names that could point out
    /// of a folder, and entries or a total over `limits`. Entries it does
    /// not know are skipped.
    pub fn from_bytes_with(bytes: &[u8], limits: &ArchiveLimits) -> Result<Self, ProjectError> {
        let mut z = open_zip(bytes, limits)?;
        let mut total = 0u64;
        let mut project = None;
        let mut sources = BTreeMap::new();
        let mut thumbnail = None;
        for i in 0..z.len() {
            let mut f = z.by_index(i)?;
            let name = f.name()?.into_owned();
            check_name(&name)?;
            let limit = if name == PROJECT_ENTRY {
                limits.max_project_json_bytes
            } else if name == THUMBNAIL_ENTRY {
                limits.max_thumbnail_bytes
            } else if name.starts_with(SOURCES_DIR) && !f.is_dir() {
                limits.max_source_bytes
            } else {
                continue;
            };
            let data = read_bounded(&mut f, &name, limit, limits, &mut total)?;
            if name == PROJECT_ENTRY {
                project = Some(data);
            } else if name == THUMBNAIL_ENTRY {
                thumbnail = Some(data);
            } else {
                sources.insert(name, Arc::from(data));
            }
        }
        let project =
            project.ok_or_else(|| ProjectError::Damaged("it has no project.json".into()))?;
        Ok(Self {
            project: serde_json::from_slice(&project)?,
            sources,
            thumbnail,
        })
    }
}

/// Reads a whole project file, refusing one larger than any project can be.
fn read_file(path: &Path) -> Result<Vec<u8>, ProjectError> {
    let f = std::fs::File::open(path)?;
    let cap = limits::MAX_PROJECT_TOTAL_BYTES;
    let mut bytes = Vec::new();
    f.take(cap + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Err(too_large("the file", cap));
    }
    Ok(bytes)
}

fn too_large(what: &str, limit: u64) -> ProjectError {
    ProjectError::TooLarge {
        what: what.to_string(),
        limit_mb: limit.div_ceil(1024 * 1024),
    }
}

/// Opens the zip and checks the type: plain JSON, a file that is no zip
/// and a zip without the FlatCraft `mimetype` are refused.
fn open_zip<'a>(
    bytes: &'a [u8],
    limits: &ArchiveLimits,
) -> Result<zip::ZipArchive<std::io::Cursor<&'a [u8]>>, ProjectError> {
    if bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') {
        return Err(ProjectError::PlainJson);
    }
    let mut z =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| ProjectError::NotAProject)?;
    if z.len() > limits.max_entries {
        return Err(ProjectError::Damaged(format!(
            "it has more than {} entries",
            limits.max_entries
        )));
    }
    let mime = match z.by_name(MIMETYPE_ENTRY) {
        Ok(mut f) => {
            let mut text = Vec::new();
            (&mut f).take(256).read_to_end(&mut text)?;
            text
        }
        Err(_) => return Err(ProjectError::NotAProject),
    };
    if mime != MIMETYPE.as_bytes() {
        return Err(ProjectError::NotAProject);
    }
    Ok(z)
}

/// Refuses an entry name that could point out of a folder when unpacked:
/// absolute, with a drive, with `..`, or with a backslash.
fn check_name(name: &str) -> Result<(), ProjectError> {
    let unsafe_name = name.is_empty()
        || name.starts_with('/')
        || name.contains('\\')
        || name.contains(':')
        || name.contains('\0')
        || name.split('/').any(|part| part == "..");
    if unsafe_name {
        return Err(ProjectError::UnsafeEntry(name.to_string()));
    }
    Ok(())
}

/// Reads one entry, at most `limit` bytes and no more than the total
/// allows. The sizes in the zip headers are not trusted: the read itself
/// is bounded.
fn read_bounded(
    f: &mut impl Read,
    name: &str,
    limit: u64,
    limits: &ArchiveLimits,
    total: &mut u64,
) -> Result<Vec<u8>, ProjectError> {
    let room = limits.max_total_bytes.saturating_sub(*total);
    let cap = limit.min(room);
    let mut data = Vec::new();
    f.take(cap + 1).read_to_end(&mut data)?;
    let n = data.len() as u64;
    if n > cap {
        return Err(if n > limit {
            too_large(name, limit)
        } else {
            too_large("the whole project", limits.max_total_bytes)
        });
    }
    *total += n;
    Ok(data)
}

/// The thumbnail of a project file, without reading its sources. `None`
/// when the file has none.
pub fn read_thumbnail(path: &Path) -> Result<Option<Vec<u8>>, ProjectError> {
    let file = std::fs::File::open(path)?;
    thumbnail_from(file)
}

/// [`read_thumbnail`] for a project file's bytes.
pub fn thumbnail_from_bytes(bytes: &[u8]) -> Result<Option<Vec<u8>>, ProjectError> {
    thumbnail_from(std::io::Cursor::new(bytes))
}

fn thumbnail_from(reader: impl Read + Seek) -> Result<Option<Vec<u8>>, ProjectError> {
    let mut z = zip::ZipArchive::new(reader).map_err(|_| ProjectError::NotAProject)?;
    let limits = ArchiveLimits::default();
    match z.by_name(MIMETYPE_ENTRY) {
        Ok(mut f) => {
            let mut text = Vec::new();
            (&mut f).take(256).read_to_end(&mut text)?;
            if text != MIMETYPE.as_bytes() {
                return Err(ProjectError::NotAProject);
            }
        }
        Err(_) => return Err(ProjectError::NotAProject),
    }
    let mut f = match z.by_name(THUMBNAIL_ENTRY) {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut total = 0;
    read_bounded(
        &mut f,
        THUMBNAIL_ENTRY,
        limits.max_thumbnail_bytes,
        &limits,
        &mut total,
    )
    .map(Some)
}

// ---------------------------------------------------------------- sources

/// sha256 of `bytes`, lower-case hex.
fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn source_bytes(source: &Source) -> &Arc<[u8]> {
    match source {
        Source::Svg { bytes, .. } | Source::Raster { bytes, .. } => bytes,
    }
}

/// The file name extension of a source, from its kind and, for a picture,
/// the first bytes.
fn source_ext(source: &Source) -> &'static str {
    match source {
        Source::Svg { .. } => "svg",
        Source::Raster { bytes, .. } => {
            let b: &[u8] = bytes;
            if b.starts_with(b"\x89PNG") {
                "png"
            } else if b.starts_with(b"\xff\xd8\xff") {
                "jpg"
            } else if b.starts_with(b"GIF8") {
                "gif"
            } else if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
                "webp"
            } else {
                "bin"
            }
        }
    }
}

/// The entry of a source in the archive: `sources/<sha256>.<ext>`.
pub fn source_file(source: &Source) -> String {
    format!(
        "{SOURCES_DIR}{}.{}",
        sha256_hex(source_bytes(source)),
        source_ext(source)
    )
}

/// The sha256 a source entry name claims, if the name has the right form.
fn claimed_sha(file: &str) -> Option<&str> {
    let rest = file.strip_prefix(SOURCES_DIR)?;
    let (sha, _ext) = rest.split_once('.')?;
    (sha.len() == 64 && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).then_some(sha)
}

// ---------------------------------------------------------------- save

/// Writes the project crash-safely ([`crate::fsio::write_atomic`]: a new
/// temp file in the same folder, synced, then renamed over `path`),
/// without a thumbnail.
pub fn save(doc: &Document, path: &Path) -> Result<(), ProjectError> {
    save_at(doc, &crate::fsio::resolve_links(path)?, None)
}

/// Like [`save`], for a `target` the caller has already resolved and
/// checked; links at it are not followed again
/// ([`crate::fsio::write_atomic_at`]). `thumbnail` is a PNG of the mat
/// (the engine draws it; core has no renderer).
pub fn save_at(
    doc: &Document,
    target: &Path,
    thumbnail: Option<&[u8]>,
) -> Result<(), ProjectError> {
    let bytes = to_archive(doc, thumbnail)?.to_bytes()?;
    crate::fsio::write_atomic_at(target, &bytes)?;
    Ok(())
}

/// The project file of `doc` as its parts.
pub fn to_archive(doc: &Document, thumbnail: Option<&[u8]>) -> Result<Archive, ProjectError> {
    let mut archive = Archive {
        project: to_value(doc)?,
        sources: BTreeMap::new(),
        thumbnail: thumbnail.map(<[u8]>::to_vec),
    };
    for o in &doc.objects {
        archive.add_source(&o.source);
    }
    Ok(archive)
}

/// The project as JSON (`project.json`). Each source names its file in
/// the archive ([`source_file`]). The shared fields of linked copies are
/// written once per group, in `link_groups`, and left out of the members.
pub fn to_value(doc: &Document) -> Result<serde_json::Value, ProjectError> {
    use serde_json::{Map, Value};
    let mut value = serde_json::to_value(doc)?;
    value["format_version"] = FORMAT_VERSION.into();
    let mut groups: Vec<(u64, Map<String, Value>)> = Vec::new();
    if let Some(objects) = value.get_mut("objects").and_then(Value::as_array_mut) {
        for (o, object) in objects.iter_mut().zip(&doc.objects) {
            let Some(o) = o.as_object_mut() else {
                continue;
            };
            if let Some(inner) = o
                .get_mut("source")
                .and_then(Value::as_object_mut)
                .and_then(|s| s.values_mut().next())
                .and_then(Value::as_object_mut)
            {
                inner.insert("file".into(), source_file(&object.source).into());
            }
            let Some(g) = o.get("link_group").and_then(Value::as_u64) else {
                continue;
            };
            let shared = take_shared(o);
            if !groups.iter().any(|(id, _)| *id == g) {
                groups.push((g, shared));
            }
        }
    }
    if !groups.is_empty() {
        let records = groups
            .into_iter()
            .map(|(id, mut shared)| {
                shared.insert("id".into(), id.into());
                Value::Object(shared)
            })
            .collect();
        value["link_groups"] = Value::Array(records);
    }
    Ok(value)
}

/// Removes the shared keys from one object's JSON and returns them.
fn take_shared(
    o: &mut serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut shared = serde_json::Map::new();
    for key in SHARED_KEYS {
        if let Some(v) = o.remove(key) {
            shared.insert(key.into(), v);
        }
    }
    if let Some(t) = o
        .get_mut("transform")
        .and_then(serde_json::Value::as_object_mut)
    {
        for key in SHARED_TRANSFORM_KEYS {
            if let Some(v) = t.remove(key) {
                shared.insert(key.into(), v);
            }
        }
    }
    shared
}

/// Puts the shared fields of each linked copy's group back into its JSON.
/// A field the member carries itself is kept, never overwritten: when it
/// differs from the group's, the member leaves the group on load
/// ([`Document::share_link_groups`]).
fn unpack_link_groups(value: &mut serde_json::Value) -> Result<(), ProjectError> {
    use serde_json::Value;
    let Some(groups) = value.get_mut("link_groups").map(Value::take) else {
        return check_no_members(value);
    };
    let groups: Vec<Value> = match groups {
        Value::Array(a) => a,
        _ => Vec::new(),
    };
    let find = |g: u64| {
        groups
            .iter()
            .find(|r| r.get("id").and_then(Value::as_u64) == Some(g))
    };
    let Some(objects) = value.get_mut("objects").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for o in objects.iter_mut().filter_map(Value::as_object_mut) {
        let Some(g) = o.get("link_group").and_then(Value::as_u64) else {
            continue;
        };
        let record = find(g).ok_or_else(|| missing_group(o, g))?;
        for key in SHARED_KEYS {
            if let Some(v) = record.get(key) {
                o.entry(key).or_insert_with(|| v.clone());
            }
        }
        if let Some(t) = o.get_mut("transform").and_then(Value::as_object_mut) {
            for key in SHARED_TRANSFORM_KEYS {
                if let Some(v) = record.get(key) {
                    t.entry(key).or_insert_with(|| v.clone());
                }
            }
        }
    }
    Ok(())
}

/// A file without `link_groups` whose objects name a group: refused, so
/// the member is named instead of a "missing field source".
fn check_no_members(value: &serde_json::Value) -> Result<(), ProjectError> {
    let objects = value.get("objects").and_then(serde_json::Value::as_array);
    for o in objects
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_object)
    {
        if let Some(g) = o.get("link_group").and_then(serde_json::Value::as_u64)
            && !o.contains_key("source")
        {
            return Err(missing_group(o, g));
        }
    }
    Ok(())
}

fn missing_group(o: &serde_json::Map<String, serde_json::Value>, group: u64) -> ProjectError {
    let name = o
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("an object")
        .to_string();
    ProjectError::MissingGroup { name, group }
}

/// Loads a project. Objects come back with empty geometry; the engine
/// rebuilds it from the sources. Linked copies come back sharing one
/// source `Arc`. See [`load_report`] for the warnings.
pub fn load(path: &Path) -> Result<Document, ProjectError> {
    load_report(path).map(|(doc, _)| doc)
}

/// [`load`], plus the ids of the linked copies whose own fields no longer
/// matched their group (a hand-edited file): they were unlinked and kept
/// their own design (see [`unlinked_warning`]).
pub fn load_report(path: &Path) -> Result<(Document, Vec<ObjectId>), ProjectError> {
    load_report_bytes(&read_file(path)?)
}

/// [`load_report`] for a project file's bytes that are not a file on disk
/// (a sample built into the app).
pub fn load_report_bytes(bytes: &[u8]) -> Result<(Document, Vec<ObjectId>), ProjectError> {
    load_archive(Archive::from_bytes(bytes)?)
}

/// Makes the document of an archive. Every source project.json names must
/// be in the archive with the sha256 its name claims; else the error
/// names the object. Content that is the same comes back as one `Arc`.
pub fn load_archive(archive: Archive) -> Result<(Document, Vec<ObjectId>), ProjectError> {
    use serde_json::Value;
    let Archive {
        project: mut value,
        sources,
        ..
    } = archive;
    let found = value
        .get("format_version")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if found > u64::from(FORMAT_VERSION) {
        return Err(ProjectError::NewerFormat {
            found: u32::try_from(found).unwrap_or(u32::MAX),
            supported: FORMAT_VERSION,
        });
    }
    unpack_link_groups(&mut value)?;
    // Each member carries its own source reference now, so a missing or
    // damaged file names the object. The bytes go in before linked copies
    // are compared (`share_link_groups`).
    let mut checked: HashMap<String, Arc<[u8]>> = HashMap::new();
    let mut bytes_of: Vec<Arc<[u8]>> = Vec::new();
    for o in value
        .get("objects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = o
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("an object")
            .to_string();
        let file = o
            .get("source")
            .and_then(Value::as_object)
            .and_then(|s| s.values().next())
            .and_then(|inner| inner.get("file"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if let Some(bytes) = checked.get(&file) {
            bytes_of.push(bytes.clone());
            continue;
        }
        let missing = || ProjectError::MissingSource {
            name: name.clone(),
            file: if file.is_empty() {
                "no file named".into()
            } else {
                file.clone()
            },
        };
        let sha = claimed_sha(&file).ok_or_else(missing)?;
        let bytes = sources.get(&file).ok_or_else(missing)?;
        if sha256_hex(bytes) != sha {
            return Err(ProjectError::DamagedSource {
                name,
                file: file.clone(),
            });
        }
        checked.insert(file, bytes.clone());
        bytes_of.push(bytes.clone());
    }
    let mut doc: Document = serde_json::from_value(value)?;
    for (o, bytes) in doc.objects.iter_mut().zip(bytes_of) {
        match &mut o.source {
            Source::Svg { bytes: b, .. } | Source::Raster { bytes: b, .. } => *b = bytes,
        }
    }
    let unlinked = doc.share_link_groups();
    let min_next = doc.objects.iter().map(|o| o.id.0 + 1).max().unwrap_or(1);
    doc.next_id = doc.next_id.max(min_next);
    doc.format_version = FORMAT_VERSION;
    Ok((doc, unlinked))
}

/// The warning for a linked copy that was unlinked on open.
pub fn unlinked_warning(name: &str) -> String {
    format!("{name}: it no longer matched its linked copies, so it was unlinked")
}
