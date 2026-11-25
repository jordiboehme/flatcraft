//! Crash-safe file writes, shared by project files, the preset store, the
//! preferences and the engine's file-writing commands.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Writes `bytes` to `path` so a crash or power loss leaves either the old
/// file or the new one, never a half-written one: write a temp file next
/// to it, `sync_all` it, rename it over `path`, then (on unix) sync the
/// folder so the rename itself is on disk.
///
/// The temp file has a name of its own (`.<name>.<pid>.<n>.tmp`) and is
/// created new (`create_new`, which never follows a symlink), so a link
/// planted next to the file cannot redirect the write and two writers never
/// share a temp file. Only the temp file this call created is removed when
/// a step fails. Leftovers of crashed writes of other processes, older than
/// [`STALE_TEMP`], are cleaned up first; nothing else is touched.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path has no file name"))?;
    clean_stale_temps(path, name);
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

/// A temp file of another process older than this is a leftover of a
/// crash and is removed.
pub const STALE_TEMP: Duration = Duration::from_secs(3600);

/// Removes leftovers of crashed writes next to `path`: files named exactly
/// `.<name>.<pid>.<n>.tmp` (digits only) of another process, older than
/// [`STALE_TEMP`]. Best effort; anything else is never touched.
fn clean_stale_temps(path: &Path, name: &std::ffi::OsStr) {
    let (Some(dir), Some(name)) = (path.parent(), name.to_str()) else {
        return;
    };
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let prefix = format!(".{name}.");
    let own = std::process::id();
    for entry in entries.flatten() {
        let file = entry.file_name();
        let Some(rest) = file
            .to_str()
            .and_then(|f| f.strip_prefix(&prefix))
            .and_then(|r| r.strip_suffix(".tmp"))
        else {
            continue;
        };
        let Some((pid, n)) = rest.split_once('.') else {
            continue;
        };
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        if !digits(pid) || !digits(n) || pid.parse::<u32>().ok() == Some(own) {
            continue;
        }
        // Only plain files, never a link or a folder.
        let Ok(meta) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        let old = meta
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE_TEMP);
        if meta.is_file() && old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Counts the temp files of this process.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The temp file name `.<name>.<pid>.<n>.tmp`.
fn temp_name(name: &std::ffi::OsStr, pid: u32, n: u64) -> std::ffi::OsString {
    let mut t = std::ffi::OsString::from(".");
    t.push(name);
    t.push(format!(".{pid}.{n}.tmp"));
    t
}

/// Creates a new temp file next to `path` with a name no other writer uses:
/// the process id and a counter.
fn create_temp(path: &Path, name: &std::ffi::OsStr) -> io::Result<(std::path::PathBuf, File)> {
    let pid = std::process::id();
    let mut last = None;
    for _ in 0..100 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = path.with_file_name(temp_name(name, pid, n));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::SystemTime;

    /// The tests look at the shared counter, so they run one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[cfg(unix)]
    #[test]
    fn never_follows_a_link_planted_at_the_next_temp_names() {
        let _serial = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let d = tempfile::tempdir().unwrap();
        let victim = d.path().join("victim.txt");
        fs::write(&victim, "keep me").unwrap();
        let path = d.path().join("a.toml");
        let next = COUNTER.load(Ordering::SeqCst);
        let pid = std::process::id();
        for n in next..next + 5 {
            let link = d
                .path()
                .join(temp_name(std::ffi::OsStr::new("a.toml"), pid, n));
            std::os::unix::fs::symlink(&victim, link).unwrap();
        }
        write_atomic(&path, b"new").unwrap();
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep me");
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    }

    fn age(path: &Path, secs: u64) {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(secs))
            .unwrap();
    }

    #[test]
    fn old_temp_files_of_other_writers_are_cleaned_up() {
        let _serial = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("a.toml");
        let other = std::process::id().wrapping_add(1);
        let file = |n: &str| d.path().join(n);
        let old_crash = file(&format!(".a.toml.{other}.0.tmp"));
        let fresh = file(&format!(".a.toml.{other}.1.tmp"));
        let ours = file(&format!(".a.toml.{}.999999.tmp", std::process::id()));
        let unrelated = [
            file(".a.toml.notes.tmp"),
            file(&format!(".b.toml.{other}.0.tmp")),
            file("a.toml.tmp"),
        ];
        for f in [&old_crash, &fresh, &ours].into_iter().chain(&unrelated) {
            fs::write(f, "x").unwrap();
        }
        for f in [&old_crash, &ours].into_iter().chain(&unrelated) {
            age(f, 2 * 3600);
        }
        write_atomic(&path, b"new").unwrap();
        assert!(!old_crash.exists(), "an old leftover of a crash is removed");
        assert!(fresh.exists(), "another writer may still be busy");
        assert!(ours.exists(), "our own process is never cleaned up");
        for f in &unrelated {
            assert!(f.exists(), "{} is not ours to remove", f.display());
        }
    }
}
