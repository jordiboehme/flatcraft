//! The Home page's small pictures of recent projects and samples, cached
//! as PNG files in `<config dir>/thumbnails`. Never inside a project's
//! folder.
//!
//! A picture is stored under its id (a project's absolute path, or a
//! sample's name) and a version (a project file's modification time and
//! size): `<hash of id>-<hash of version>.png`. Storing a new version
//! removes the older ones of the same id, so a project that is saved
//! again gets a fresh picture and the folder does not grow.

use std::io;
use std::path::{Path, PathBuf};

use crate::commands::sources::sha256_hex;

/// The folder inside the config dir.
pub const DIR: &str = "thumbnails";

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

fn prefix(id: &str) -> String {
    sha256_hex(id.as_bytes())[..16].to_string()
}

/// Where the picture of `id` at `version` is stored.
pub fn file(config_dir: &Path, id: &str, version: &str) -> PathBuf {
    let v = &sha256_hex(version.as_bytes())[..12];
    config_dir.join(DIR).join(format!("{}-{v}.png", prefix(id)))
}

/// The stored picture of `id` at `version`, if there is one.
pub fn cached(config_dir: &Path, id: &str, version: &str) -> Option<Vec<u8>> {
    std::fs::read(file(config_dir, id, version)).ok()
}

/// Stores the picture of `id` at `version` and removes its older
/// versions. Returns the file.
pub fn store(config_dir: &Path, id: &str, version: &str, png: &[u8]) -> io::Result<PathBuf> {
    let path = file(config_dir, id, version);
    std::fs::create_dir_all(config_dir.join(DIR))?;
    flatcraft_device::fsio::write_atomic(&path, png)?;
    remove_where(config_dir, id, |p| p != path)?;
    Ok(path)
}

/// Removes every stored picture of `id` (a project taken off the list).
pub fn forget(config_dir: &Path, id: &str) -> io::Result<()> {
    remove_where(config_dir, id, |_| true)
}

fn remove_where(config_dir: &Path, id: &str, remove: impl Fn(&Path) -> bool) -> io::Result<()> {
    let dir = config_dir.join(DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let start = format!("{}-", prefix(id));
    for entry in entries.flatten() {
        let path = entry.path();
        let ours = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(&start) && n.ends_with(".png"));
        if ours && remove(&path) {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_version_replaces_the_old_one_and_forget_removes_all() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path();
        let a = store(config, "/p/a.flatcraft", "1-10", b"one").unwrap();
        let b = store(config, "/p/b.flatcraft", "1-10", b"other").unwrap();
        assert_eq!(cached(config, "/p/a.flatcraft", "1-10").unwrap(), b"one");
        let a2 = store(config, "/p/a.flatcraft", "2-12", b"two").unwrap();
        assert_ne!(a, a2);
        assert!(!a.exists(), "the old version is gone");
        assert!(cached(config, "/p/a.flatcraft", "1-10").is_none());
        assert_eq!(cached(config, "/p/a.flatcraft", "2-12").unwrap(), b"two");
        assert!(b.exists(), "another project keeps its picture");
        forget(config, "/p/a.flatcraft").unwrap();
        assert!(!a2.exists());
        assert!(b.exists());
        assert!(a2.starts_with(config.join(DIR)));
    }

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
}
