//! The project file is a zip archive: `mimetype`, `project.json`, the
//! sources once per content and an optional `thumbnail.png`.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use flatcraft_core::document::{Document, GroupId, JobSettings, Source, TraceSettings};
use flatcraft_core::project::{self, Archive, ArchiveLimits, ProjectError};

const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"><rect width="10" height="10"/></svg>"#;
/// The start of a PNG (the bytes are never decoded here).
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-not-a-real-picture-";
const JPEG: &[u8] = b"\xff\xd8\xff\xe0-jpeg-";

fn job() -> JobSettings {
    JobSettings {
        plotter_id: "cameo2".into(),
        material_id: "cardstock".into(),
        media_id: "a4".into(),
        speed: None,
        force: None,
        passes: None,
        strategy: Default::default(),
    }
}

fn svg(name: &str, bytes: &[u8]) -> Source {
    Source::Svg {
        name: name.into(),
        bytes: bytes.to_vec().into(),
    }
}

fn raster(name: &str, bytes: &[u8]) -> Source {
    Source::Raster {
        name: name.into(),
        bytes: bytes.to_vec().into(),
        trace: TraceSettings::default(),
    }
}

fn bytes_of(s: &Source) -> &Arc<[u8]> {
    match s {
        Source::Svg { bytes, .. } | Source::Raster { bytes, .. } => bytes,
    }
}

/// The entries of a zip file in order: name and compression method.
fn entries(path: &Path) -> Vec<(String, zip::CompressionMethod)> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    (0..z.len())
        .map(|i| {
            let f = z.by_index(i).unwrap();
            (f.name().unwrap().to_string(), f.compression())
        })
        .collect()
}

fn saved(doc: &Document, thumbnail: Option<&[u8]>) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.flatcraft");
    project::save_at(doc, &path, thumbnail).unwrap();
    (dir, path)
}

#[test]
fn round_trip_with_svg_and_raster_sources() {
    let mut doc = Document::new(job());
    doc.add("star".into(), svg("star.svg", SVG), vec![]);
    doc.add("photo".into(), raster("photo.png", PNG), vec![]);
    doc.add("snap".into(), raster("snap.jpg", JPEG), vec![]);
    let (_dir, path) = saved(&doc, None);
    let loaded = project::load(&path).unwrap();
    assert_eq!(loaded.objects.len(), 3);
    for (a, b) in loaded.objects.iter().zip(&doc.objects) {
        assert_eq!(a.source, b.source);
        assert_eq!(a.name, b.name);
    }
    let names: Vec<String> = entries(&path).into_iter().map(|(n, _)| n).collect();
    let sha = |b: &[u8]| {
        use sha2::Digest;
        sha2::Sha256::digest(b)
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>()
    };
    assert!(
        names.contains(&format!("sources/{}.svg", sha(SVG))),
        "{names:?}"
    );
    assert!(
        names.contains(&format!("sources/{}.png", sha(PNG))),
        "{names:?}"
    );
    assert!(
        names.contains(&format!("sources/{}.jpg", sha(JPEG))),
        "{names:?}"
    );
    // project.json names the files instead of embedding the bytes.
    let text = serde_json::to_string(&Archive::read(&path).unwrap().project).unwrap();
    assert!(!text.contains("\"bytes\""), "{text}");
    assert!(
        text.contains(&format!("sources/{}.svg", sha(SVG))),
        "{text}"
    );
}

#[test]
fn duplicates_and_linked_copies_share_one_source_entry() {
    let mut doc = Document::new(job());
    let a = doc.add("a".into(), svg("a.svg", SVG), vec![]);
    let b = doc.add("b".into(), svg("b.svg", SVG), vec![]);
    let c = doc.add("c".into(), svg("a.svg", SVG), vec![]);
    let d = doc.add("d".into(), svg("a.svg", SVG), vec![]);
    let shared = doc.get(a).unwrap().source.clone();
    for id in [c, d] {
        doc.get_mut(id).unwrap().source = shared.clone();
        doc.get_mut(id).unwrap().link_group = Some(GroupId(1));
    }
    doc.get_mut(a).unwrap().link_group = Some(GroupId(1));
    let (_dir, path) = saved(&doc, None);
    let sources: Vec<String> = entries(&path)
        .into_iter()
        .map(|(n, _)| n)
        .filter(|n| n.starts_with("sources/"))
        .collect();
    assert_eq!(sources.len(), 1, "{sources:?}");
    let loaded = project::load(&path).unwrap();
    let first = bytes_of(&loaded.get(a).unwrap().source);
    for id in [b, c, d] {
        assert!(
            Arc::ptr_eq(first, bytes_of(&loaded.get(id).unwrap().source)),
            "the same content is one Arc after loading"
        );
    }
}

#[test]
fn mimetype_comes_first_and_is_stored_uncompressed() {
    let mut doc = Document::new(job());
    doc.add("star".into(), svg("star.svg", SVG), vec![]);
    doc.add("photo".into(), raster("photo.png", PNG), vec![]);
    let (_dir, path) = saved(&doc, Some(PNG));
    let raw = std::fs::read(&path).unwrap();
    // The ODF way: the first local header is "mimetype", stored, no extra
    // field, so the type can be read at a fixed offset.
    assert_eq!(&raw[0..4], b"PK\x03\x04");
    assert_eq!(u16::from_le_bytes([raw[8], raw[9]]), 0, "stored");
    assert_eq!(u16::from_le_bytes([raw[26], raw[27]]), 8, "name length");
    assert_eq!(u16::from_le_bytes([raw[28], raw[29]]), 0, "no extra field");
    assert_eq!(&raw[30..38], b"mimetype");
    assert_eq!(
        &raw[38..38 + project::MIMETYPE.len()],
        project::MIMETYPE.as_bytes()
    );
    assert_eq!(project::MIMETYPE, "application/vnd.flatcraft+zip");

    let list = entries(&path);
    assert_eq!(list[0].0, "mimetype");
    assert_eq!(list[1].0, "project.json");
    use zip::CompressionMethod::{Deflated, Stored};
    for (name, method) in &list {
        let want = if name == "project.json" || name.ends_with(".svg") {
            Deflated
        } else {
            Stored
        };
        assert_eq!(*method, want, "{name}");
    }
    assert!(list.iter().any(|(n, _)| n == "thumbnail.png"));
    // project.json is pretty JSON, version 1.
    let mut z = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let mut text = String::new();
    z.by_name("project.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert!(text.contains("\"format_version\": 1"), "{text}");
}

#[test]
fn the_thumbnail_is_read_back() {
    let mut doc = Document::new(job());
    doc.add("star".into(), svg("star.svg", SVG), vec![]);
    let (_dir, path) = saved(&doc, Some(PNG));
    assert_eq!(
        project::read_thumbnail(&path).unwrap().as_deref(),
        Some(PNG)
    );
    let (_dir2, without) = saved(&doc, None);
    assert_eq!(project::read_thumbnail(&without).unwrap(), None);
    assert_eq!(project::THUMBNAIL_PX, 512);
}

fn refused(bytes: &[u8]) -> ProjectError {
    project::load_report_bytes(bytes).unwrap_err()
}

fn archive_of(doc: &Document) -> Archive {
    project::to_archive(doc, None).unwrap()
}

fn one_object() -> Document {
    let mut doc = Document::new(job());
    doc.add("Daisy".into(), svg("daisy.svg", SVG), vec![]);
    doc
}

#[test]
fn a_plain_json_project_of_a_development_build_is_refused() {
    let e = refused(br#"  {"format_version": 1, "objects": []}"#);
    assert!(matches!(e, ProjectError::PlainJson), "{e:?}");
    let text = e.to_string();
    assert!(text.contains("development build"), "{text}");
    assert!(!text.contains('\u{2014}'), "no em-dash: {text}");
}

#[test]
fn something_else_is_not_a_project() {
    let e = refused(b"hello");
    assert!(matches!(e, ProjectError::NotAProject), "{e:?}");
    assert!(e.to_string().contains("not a FlatCraft project"), "{e}");
}

#[test]
fn a_version_2_file_is_refused() {
    let mut archive = archive_of(&one_object());
    archive.project["format_version"] = 2.into();
    let e = refused(&archive.to_bytes().unwrap());
    assert!(
        matches!(
            e,
            ProjectError::NewerFormat {
                found: 2,
                supported: 1
            }
        ),
        "{e:?}"
    );
    assert!(e.to_string().contains("newer FlatCraft"), "{e}");
}

#[test]
fn a_missing_source_is_refused_naming_the_object() {
    let mut archive = archive_of(&one_object());
    archive.sources.clear();
    let e = refused(&archive.to_bytes().unwrap());
    assert!(matches!(e, ProjectError::MissingSource { .. }), "{e:?}");
    let text = e.to_string();
    assert!(text.contains("Daisy"), "{text}");
    assert!(text.contains("missing"), "{text}");
}

#[test]
fn a_source_with_the_wrong_sha256_is_refused_naming_the_object() {
    let mut archive = archive_of(&one_object());
    let name = archive.sources.keys().next().unwrap().clone();
    archive
        .sources
        .insert(name, Arc::from(&b"<svg>changed</svg>"[..]));
    let e = refused(&archive.to_bytes().unwrap());
    assert!(matches!(e, ProjectError::DamagedSource { .. }), "{e:?}");
    let text = e.to_string();
    assert!(text.contains("Daisy"), "{text}");
    assert!(text.contains("checksum"), "{text}");
}

#[test]
fn a_reference_outside_sources_is_refused() {
    let mut archive = archive_of(&one_object());
    archive.project["objects"][0]["source"]["Svg"]["file"] = "project.json".into();
    let e = refused(&archive.to_bytes().unwrap());
    assert!(matches!(e, ProjectError::MissingSource { .. }), "{e:?}");
}

/// A zip with the right mimetype and project.json plus one more entry.
fn zip_with(extra_name: &str, extra: &[u8]) -> Vec<u8> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    let archive = archive_of(&one_object());
    let mut out = std::io::Cursor::new(Vec::new());
    let mut z = zip::ZipWriter::new(&mut out);
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    z.start_file("mimetype", stored).unwrap();
    z.write_all(project::MIMETYPE.as_bytes()).unwrap();
    z.start_file("project.json", stored).unwrap();
    z.write_all(&serde_json::to_vec(&archive.project).unwrap())
        .unwrap();
    for (name, bytes) in &archive.sources {
        z.start_file(name.as_str(), stored).unwrap();
        z.write_all(bytes).unwrap();
    }
    z.start_file(extra_name, stored).unwrap();
    z.write_all(extra).unwrap();
    z.finish().unwrap();
    out.into_inner()
}

#[test]
fn an_entry_with_dot_dot_is_refused() {
    for name in [
        "../evil.txt",
        "sources/../../evil.txt",
        "/etc/evil",
        "C:\\evil",
    ] {
        let e = refused(&zip_with(name, b"x"));
        assert!(matches!(e, ProjectError::UnsafeEntry(_)), "{name}: {e:?}");
        assert!(e.to_string().contains("unsafe"), "{e}");
    }
}

#[test]
fn an_oversized_entry_is_refused() {
    let limits = ArchiveLimits {
        max_source_bytes: 1000,
        ..ArchiveLimits::default()
    };
    let big = vec![b'a'; 5000];
    let mut doc = Document::new(job());
    doc.add("Huge".into(), svg("huge.svg", &big), vec![]);
    let bytes = project::to_archive(&doc, None).unwrap().to_bytes().unwrap();
    let e = Archive::from_bytes_with(&bytes, &limits).unwrap_err();
    assert!(matches!(e, ProjectError::TooLarge { .. }), "{e:?}");
    assert!(e.to_string().contains("too large"), "{e}");
    // The total counts too.
    let limits = ArchiveLimits {
        max_total_bytes: 2000,
        ..ArchiveLimits::default()
    };
    let e = Archive::from_bytes_with(&bytes, &limits).unwrap_err();
    assert!(matches!(e, ProjectError::TooLarge { .. }), "{e:?}");
    // And the defaults let it through.
    assert!(Archive::from_bytes(&bytes).is_ok());
}

#[test]
fn a_zip_without_mimetype_or_project_json_is_refused() {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    let mut out = std::io::Cursor::new(Vec::new());
    let mut z = zip::ZipWriter::new(&mut out);
    z.start_file("readme.txt", SimpleFileOptions::default())
        .unwrap();
    z.write_all(b"hi").unwrap();
    z.finish().unwrap();
    let e = refused(&out.into_inner());
    assert!(matches!(e, ProjectError::NotAProject), "{e:?}");

    let mut archive = archive_of(&one_object());
    archive.project = serde_json::Value::Null;
    let mut bytes = Vec::new();
    {
        let mut out = std::io::Cursor::new(&mut bytes);
        let mut z = zip::ZipWriter::new(&mut out);
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("mimetype", stored).unwrap();
        z.write_all(project::MIMETYPE.as_bytes()).unwrap();
        z.finish().unwrap();
    }
    let e = refused(&bytes);
    assert!(matches!(e, ProjectError::Damaged(_)), "{e:?}");
    assert!(e.to_string().contains("project.json"), "{e}");
}

#[cfg(unix)]
#[test]
fn a_failed_save_leaves_the_old_file() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("work");
    std::fs::create_dir(&folder).unwrap();
    let path = folder.join("p.flatcraft");
    let doc = one_object();
    project::save(&doc, &path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut bigger = doc.clone();
    bigger.add("Two".into(), svg("two.svg", b"<svg/>"), vec![]);
    // The folder is read-only: the temp file cannot be made.
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
    // Root ignores the mode: then there is nothing to test here.
    if std::fs::write(folder.join("probe"), b"x").is_ok() {
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let result = project::save(&bigger, &path);
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(project::load(&path).unwrap().objects.len(), 1);
}

#[test]
fn archive_write_and_read_round_trip() {
    let mut sources = BTreeMap::new();
    let archive = project::to_archive(&one_object(), Some(PNG)).unwrap();
    for (k, v) in &archive.sources {
        sources.insert(k.clone(), v.clone());
    }
    let back = Archive::from_bytes(&archive.to_bytes().unwrap()).unwrap();
    assert_eq!(back.project, archive.project);
    assert_eq!(back.sources, sources);
    assert_eq!(back.thumbnail.as_deref(), Some(PNG));
}

#[test]
fn duplicate_entry_names_are_refused() {
    let archive = archive_of(&one_object());
    let (name, bytes) = archive.sources.iter().next().unwrap();
    // The writer refuses a second entry of the same name, so the zip gets
    // one of the same length that is renamed in its headers afterwards.
    let other = name.replace(".svg", ".svx");
    let mut zip = zip_with(&other, bytes);
    let at: Vec<usize> = (0..zip.len() - other.len())
        .filter(|&i| zip[i..].starts_with(other.as_bytes()))
        .collect();
    assert_eq!(at.len(), 2, "the local and the central header");
    for i in at {
        zip[i..i + name.len()].copy_from_slice(name.as_bytes());
    }
    let e = refused(&zip);
    assert!(matches!(e, ProjectError::Damaged(_)), "{e:?}");
    assert!(e.to_string().contains("same name"), "{e}");
}

#[test]
fn only_format_version_1_is_read() {
    for (version, found) in [
        (serde_json::json!(0), "0"),
        (serde_json::json!("1"), "\"1\""),
        (serde_json::json!(-1), "-1"),
        (serde_json::Value::Null, "null"),
    ] {
        let mut archive = archive_of(&one_object());
        archive.project["format_version"] = version;
        let e = refused(&archive.to_bytes().unwrap());
        assert!(matches!(e, ProjectError::UnknownFormat { .. }), "{e:?}");
        let text = e.to_string();
        assert!(
            text.contains(&format!("format version is {found}")) && text.contains("version 1"),
            "{text}"
        );
    }
    let mut archive = archive_of(&one_object());
    archive
        .project
        .as_object_mut()
        .unwrap()
        .remove("format_version");
    let e = refused(&archive.to_bytes().unwrap());
    assert!(e.to_string().contains("format version is missing"), "{e}");
}

/// Changes the sizes the central directory gives for `name` (both the
/// packed and the unpacked size).
fn patch_central_size(bytes: &mut [u8], name: &str, size: u32) {
    let at = (0..bytes.len() - 46)
        .find(|&i| {
            bytes[i..i + 4] == *b"PK\x01\x02" && {
                let n = u16::from_le_bytes([bytes[i + 28], bytes[i + 29]]) as usize;
                bytes.get(i + 46..i + 46 + n) == Some(name.as_bytes())
            }
        })
        .expect("the entry");
    bytes[at + 20..at + 24].copy_from_slice(&size.to_le_bytes());
    bytes[at + 24..at + 28].copy_from_slice(&size.to_le_bytes());
}

#[test]
fn a_header_that_understates_the_size_is_a_damaged_file() {
    let archive = archive_of(&one_object());
    let mut bytes = zip_with("extra.txt", b"x");
    let source = archive.sources.keys().next().unwrap().clone();
    patch_central_size(&mut bytes, &source, 10);
    let e = refused(&bytes);
    assert!(matches!(e, ProjectError::Damaged(_)), "{e:?}");
    let text = e.to_string();
    assert!(text.contains("damaged"), "{text}");
    assert!(!text.contains("read or write"), "no disk problem: {text}");
}

#[test]
fn a_broken_compressed_stream_is_a_damaged_file() {
    let mut doc = one_object();
    doc.add(
        "Long".into(),
        svg("long.svg", &b"<svg>".repeat(2000)),
        vec![],
    );
    let archive = project::to_archive(&doc, None).unwrap();
    let mut bytes = archive.to_bytes().unwrap();
    // Scramble the middle of project.json's deflate data (it follows the
    // stored mimetype entry).
    let start = 30 + 8 + project::MIMETYPE.len() + 30 + "project.json".len();
    for b in &mut bytes[start + 4..start + 40] {
        *b ^= 0x5a;
    }
    let e = refused(&bytes);
    assert!(matches!(e, ProjectError::Damaged(_)), "{e:?}");
}
