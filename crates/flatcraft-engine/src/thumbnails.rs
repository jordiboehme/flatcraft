//! Ids and versions of the Home page's small pictures of recent projects
//! and samples. The pictures themselves live in the project files
//! (`thumbnail.png`, see `flatcraft_core::project`), so nothing is cached
//! in the config dir: Home reads a picture again when its file's version
//! (modification time and size) changes.

use std::path::Path;

use crate::commands::sources::sha256_hex;

/// The id of a project file: its absolute path.
pub fn project_id(path: &Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// The version of a project file: its modification time (ns since 1970)
/// and size. `None` when the file is missing or cannot be read.
pub fn project_version(path: &Path) -> Option<String> {
    let m = std::fs::metadata(path).ok()?;
    if !m.is_file() {
        return None;
    }
    let ns = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    Some(format!("{ns}-{}", m.len()))
}

/// The picture a project file carries (`thumbnail.png`), only if it is a
/// PNG that decodes. `None` when the file has none, it is broken, or the
/// file cannot be read: the caller then draws the project itself.
pub fn embedded(path: &Path) -> Option<Vec<u8>> {
    usable(flatcraft_core::project::read_thumbnail(path).ok()??)
}

/// [`embedded`] for a project file's bytes (a sample).
pub fn embedded_in(bytes: &[u8]) -> Option<Vec<u8>> {
    usable(flatcraft_core::project::thumbnail_from_bytes(bytes).ok()??)
}

fn usable(png: Vec<u8>) -> Option<Vec<u8>> {
    tiny_skia::Pixmap::decode_png(&png).ok().map(|_| png)
}

/// The version of a picture of bytes that are no file (a sample): their
/// hash.
pub fn bytes_version(bytes: &[u8]) -> String {
    sha256_hex(bytes)[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_has_no_version() {
        let dir = tempfile::tempdir().unwrap();
        assert!(project_version(&dir.path().join("gone.flatcraft")).is_none());
        assert!(
            project_version(dir.path()).is_none(),
            "a folder is no project"
        );
        let f = dir.path().join("x.flatcraft");
        std::fs::write(&f, b"{}").unwrap();
        assert!(project_version(&f).unwrap().ends_with("-2"));
    }

    #[test]
    fn a_broken_embedded_picture_is_not_used() {
        use flatcraft_core::document::{Document, JobSettings, Source};
        let mut doc = Document::new(JobSettings {
            plotter_id: "cameo2".into(),
            material_id: "cardstock".into(),
            media_id: "a4".into(),
            speed: None,
            force: None,
            passes: None,
            strategy: Default::default(),
        });
        doc.add(
            "x".into(),
            Source::Svg {
                name: "x.svg".into(),
                bytes: b"<svg/>".to_vec().into(),
            },
            vec![],
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.flatcraft");
        let good = tiny_skia::Pixmap::new(3, 2).unwrap().encode_png().unwrap();
        flatcraft_core::project::save_at(&doc, &path, Some(&good)).unwrap();
        assert_eq!(embedded(&path), Some(good));
        flatcraft_core::project::save_at(&doc, &path, Some(b"\x89PNG garbage")).unwrap();
        assert_eq!(embedded(&path), None);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(embedded_in(&bytes), None);
        assert_eq!(embedded(&dir.path().join("gone.flatcraft")), None);
    }

    #[test]
    fn bytes_version_follows_the_content() {
        assert_eq!(bytes_version(b"a"), bytes_version(b"a"));
        assert_ne!(bytes_version(b"a"), bytes_version(b"b"));
    }
}
