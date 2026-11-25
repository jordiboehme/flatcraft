//! Crash-safe file writes, shared by project files, the preset store, the
//! preferences and the engine's file-writing commands.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

/// Writes `bytes` to `path` so a crash or power loss leaves either the old
/// file or the new one, never a half-written one: write a temp file next
/// to it, `sync_all` it, rename it over `path`, then (on unix) sync the
/// folder so the rename itself is on disk. The temp file is removed when
/// any step fails.
///
/// The temp file is always created new (`create_new`, which never follows
/// a symlink), so a link planted at its name cannot redirect the write. A
/// name that is taken (a leftover of a crash, or a planted link) is tried
/// with a number added, and a leftover `<name>.tmp` that is a plain file
/// or a link is removed first; removing a link never touches its target.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path has no file name"))?;
    let (tmp, mut f) = create_temp(path, name)?;
    let written = (|| {
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    sync_parent(path);
    Ok(())
}

/// Creates a fresh temp file next to `path`: `<name>.tmp`, else
/// `<name>.<n>.tmp`.
fn create_temp(path: &Path, name: &std::ffi::OsStr) -> io::Result<(std::path::PathBuf, File)> {
    let tmp_at = |n: u32| {
        let mut t = name.to_os_string();
        if n > 0 {
            t.push(format!(".{n}"));
        }
        t.push(".tmp");
        path.with_file_name(t)
    };
    let first = tmp_at(0);
    if fs::symlink_metadata(&first).is_ok_and(|m| !m.is_dir()) {
        let _ = fs::remove_file(&first);
    }
    let mut last = None;
    for n in 0..100 {
        let tmp = tmp_at(n);
        match File::options().write(true).create_new(true).open(&tmp) {
            Ok(f) => return Ok((tmp, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no free temp file name")))
}

/// Best effort: some file systems refuse to sync a folder. The file itself
/// is already synced, so a failure here is not reported.
#[cfg(unix)]
fn sync_parent(path: &Path) {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty())
        && let Ok(d) = File::open(dir)
    {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) {}
