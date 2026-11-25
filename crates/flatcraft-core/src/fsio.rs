//! Crash-safe file writes, shared by project files, the preset store, the
//! preferences and the engine's file-writing commands.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes `bytes` to `path` so a crash or power loss leaves either the old
/// file or the new one, never a half-written one: write a temp file next
/// to it, `sync_all` it, rename it over `path`, then (on unix) sync the
/// folder so the rename itself is on disk.
///
/// The temp file has a name of its own (`.<name>.<pid>.<n>.tmp`) and is
/// created new (`create_new`, which never follows a symlink), so a link
/// planted next to the file cannot redirect the write and two writers never
/// share a temp file. Only the temp file this call created is removed when
/// a step fails; other files are never touched.
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

/// Creates a new temp file next to `path` with a name no other writer uses:
/// the process id and a counter.
fn create_temp(path: &Path, name: &std::ffi::OsStr) -> io::Result<(std::path::PathBuf, File)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id();
    let mut last = None;
    for _ in 0..100 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut t = std::ffi::OsString::from(".");
        t.push(name);
        t.push(format!(".{pid}.{n}.tmp"));
        let tmp = path.with_file_name(t);
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
