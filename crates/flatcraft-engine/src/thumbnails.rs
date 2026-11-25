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
    fn bytes_version_follows_the_content() {
        assert_eq!(bytes_version(b"a"), bytes_version(b"a"));
        assert_ne!(bytes_version(b"a"), bytes_version(b"b"));
    }
}
