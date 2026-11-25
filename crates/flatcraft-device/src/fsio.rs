//! Crash-safe file writes, shared by the preset store, the preferences and
//! the engine's file-writing commands.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

/// Writes `bytes` to `path` so a crash or power loss leaves either the old
/// file or the new one, never a half-written one: write `<name>.tmp` next
/// to it, `sync_all` it, rename it over `path`, then (on unix) sync the
/// folder so the rename itself is on disk. The temp file is removed when
/// any step fails.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path has no file name"))?;
    let mut tmp_name = name.to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let written = (|| {
        let mut f = File::create(&tmp)?;
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
