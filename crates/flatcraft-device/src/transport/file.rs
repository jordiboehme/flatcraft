//! File Output: writes the job bytes, a listing and a byte log to a directory.
//!
//! For one job named `name` with extension `ext` it writes:
//! - `<name>.<ext>`: the exact job bytes,
//! - `<name>.<ext>.txt`: one line per command, `command<TAB># note`,
//! - `<name>.bytelog.jsonl`: the bytes as the capture fixtures log them.
//!
//! When one of these names is taken, `-2`, `-3` and so on are appended to
//! `name`. The three names are taken with `create_new` (empty files) before
//! anything is written, so a file that appears meanwhile is never replaced.
//! Every file is then written to a temp file of its own (see
//! `flatcraft_core::fsio::write_temp_for`) and renamed over its empty
//! placeholder, so a half-written file never shows up under its real name.
//! A crash before the renames can leave empty placeholders; an empty job
//! file is never a job. Leftover temp files never block a name.

use std::path::{Path, PathBuf};

use super::{SendReport, Transport, TransportError, split_commands};
use crate::bytelog::{ByteLog, Dir};
use crate::gpgl::CheckedJob;

/// Highest suffix tried before giving up.
const MAX_SUFFIX: u32 = 10_000;

#[derive(Debug, Clone)]
pub struct FileTransport {
    dir: PathBuf,
    /// The folder as the person set it, for reports and errors; `dir` may
    /// be its resolved form.
    shown: Option<PathBuf>,
    extension: String,
}

impl FileTransport {
    pub fn new(dir: PathBuf, extension: String) -> Self {
        Self {
            dir,
            shown: None,
            extension,
        }
    }

    /// Writes into `dir` (see [`FileTransport::new`]) but names `shown`
    /// in the report and in errors: the folder as the person set it, when
    /// `dir` is its resolved form.
    pub fn with_shown_dir(mut self, shown: PathBuf) -> Self {
        self.shown = Some(shown);
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `path` in or at [`FileTransport::dir`], as the shown folder names it.
    fn shown(&self, path: &Path) -> PathBuf {
        match (&self.shown, path.strip_prefix(&self.dir)) {
            (Some(shown), Ok(rel)) if rel.as_os_str().is_empty() => shown.clone(),
            (Some(shown), Ok(rel)) => shown.join(rel),
            _ => path.to_path_buf(),
        }
    }

    fn io_error(&self, what: &str, path: &Path, e: std::io::Error) -> TransportError {
        io_error(what, &self.shown(path), e)
    }

    /// The three target paths for `stem`.
    fn targets(&self, stem: &str) -> [PathBuf; 3] {
        let ext = &self.extension;
        [
            self.dir.join(format!("{stem}.{ext}")),
            self.dir.join(format!("{stem}.{ext}.txt")),
            self.dir.join(format!("{stem}.bytelog.jsonl")),
        ]
    }

    /// Takes the first stem (`name`, `name-2`, ...) whose three files are
    /// all free, with empty placeholders (see [`reserve`]).
    fn take_targets(&self, name: &str) -> Result<[PathBuf; 3], TransportError> {
        for n in 1..=MAX_SUFFIX {
            let stem = if n == 1 {
                name.to_string()
            } else {
                format!("{name}-{n}")
            };
            let targets = self.targets(&stem);
            if reserve(&targets).map_err(|e| self.io_error("create", &self.dir, e))? {
                return Ok(targets);
            }
        }
        Err(TransportError::Io(format!(
            "there are already {MAX_SUFFIX} jobs named '{name}' in {}",
            self.shown(&self.dir).display()
        )))
    }
}

/// Creates every target as a new empty file (`create_new`, which also
/// refuses a dangling link). Returns false and gives back the names it took
/// when one of them exists already.
fn reserve(targets: &[PathBuf]) -> std::io::Result<bool> {
    let mut taken: Vec<&Path> = Vec::new();
    for t in targets {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(t)
        {
            Ok(_) => taken.push(t),
            Err(e) => {
                for t in &taken {
                    let _ = std::fs::remove_file(t);
                }
                return if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Ok(false)
                } else {
                    Err(e)
                };
            }
        }
    }
    Ok(true)
}

/// A file name part made only of `[A-Za-z0-9._-]`, not empty and not
/// starting with a dot.
fn valid_part(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn io_error(what: &str, path: &Path, e: std::io::Error) -> TransportError {
    TransportError::Io(format!("cannot {what} {}: {e}", path.display()))
}

/// Writes every `(target, data)` pair through a temp file of its own and
/// renames them in order over their placeholders. On any error it removes
/// the temp files it created and every target (placeholder or renamed), so
/// a failed job leaves nothing behind and never deletes a file it did not
/// write.
fn write_all_atomic(
    files: &[(&Path, &[u8])],
    io_error: impl Fn(&str, &Path, std::io::Error) -> TransportError,
) -> Result<(), TransportError> {
    let mut created: Vec<PathBuf> = Vec::new();
    let result = (|| {
        for (target, data) in files {
            let tmp = flatcraft_core::fsio::write_temp_for(target, data, None)
                .map_err(|e| io_error("write", target, e))?;
            created.push(tmp);
        }
        for ((target, _), tmp) in files.iter().zip(&created) {
            std::fs::rename(tmp, target).map_err(|e| io_error("write", target, e))?;
        }
        Ok(())
    })();
    if result.is_err() {
        for tmp in &created {
            let _ = std::fs::remove_file(tmp);
        }
        for (target, _) in files {
            let _ = std::fs::remove_file(target);
        }
    }
    result
}

impl Transport for FileTransport {
    fn send_job(
        &mut self,
        job: &CheckedJob,
        name: &str,
        log: &mut ByteLog,
    ) -> Result<SendReport, TransportError> {
        if !valid_part(name) {
            return Err(TransportError::Io(format!(
                "'{name}' is not a usable job name: use only letters, digits, '.', '_' and '-', and do not start with '.'"
            )));
        }
        if !valid_part(&self.extension) {
            return Err(TransportError::Io(format!(
                "'{}' is not a usable file extension",
                self.extension
            )));
        }

        let commands = split_commands(&job.bytes);
        let mut job_log = ByteLog::new();
        for c in &commands {
            job_log.push(Dir::Out, c);
        }
        let mut listing = String::new();
        for l in &job.listing {
            // Notes are single-line already; make sure of it, one entry per line.
            let note = l.note.replace(['\n', '\r'], " ");
            listing.push_str(&format!("{}\t# {note}\n", l.command));
        }
        let jsonl = job_log.to_jsonl();

        std::fs::create_dir_all(&self.dir).map_err(|e| self.io_error("create", &self.dir, e))?;
        let [bytes_path, txt_path, log_path] = self.take_targets(name)?;
        // The job file is filled last: once it has its bytes, its listing
        // and log are complete too. Until then it is an empty placeholder.
        write_all_atomic(
            &[
                (&txt_path, listing.as_bytes()),
                (&log_path, jsonl.as_bytes()),
                (&bytes_path, &job.bytes),
            ],
            |what, path, e| self.io_error(what, path, e),
        )?;

        for c in &commands {
            log.push(Dir::Out, c);
        }
        Ok(SendReport {
            files: [bytes_path, txt_path, log_path]
                .iter()
                .map(|p| self.shown(p))
                .collect(),
            bytes_sent: job.bytes.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserving_never_takes_a_name_that_appeared_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        // b appears after the job picked its names.
        std::fs::write(&b, "someone else's").unwrap();
        assert!(!reserve(&[a.clone(), b.clone()]).unwrap());
        assert!(!a.exists(), "the reservation of a is given back");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "someone else's");
    }

    #[test]
    fn a_failed_later_file_removes_the_earlier_files() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("missing").join("b.txt");
        assert!(reserve(std::slice::from_ref(&a)).unwrap());
        // b's folder does not exist, so its temp file cannot be created.
        let e = write_all_atomic(&[(&a, b"1"), (&b, b"2")], io_error).unwrap_err();
        assert!(matches!(e, TransportError::Io(_)), "{e:?}");
        assert!(!a.exists());
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(left.is_empty(), "{left:?}");
    }
}
