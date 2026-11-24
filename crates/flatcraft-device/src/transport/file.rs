//! File Output: writes the job bytes, a listing and a byte log to a directory.
//!
//! For one job named `name` with extension `ext` it writes:
//! - `<name>.<ext>`: the exact job bytes,
//! - `<name>.<ext>.txt`: one line per command, `command<TAB># note`,
//! - `<name>.bytelog.jsonl`: the bytes as the capture fixtures log them.
//!
//! When one of these names is taken, `-2`, `-3` and so on are appended to
//! `name`. Every file is written to a temp file first and then renamed, so a
//! half-written file never shows up under its real name.

use std::io::Write;
use std::path::{Path, PathBuf};

use super::{SendReport, Transport, TransportError, split_commands};
use crate::bytelog::{ByteLog, Dir};
use crate::gpgl::{EmittedJob, check_safe};

/// Highest suffix tried before giving up.
const MAX_SUFFIX: u32 = 10_000;

#[derive(Debug, Clone)]
pub struct FileTransport {
    dir: PathBuf,
    extension: String,
}

impl FileTransport {
    pub fn new(dir: PathBuf, extension: String) -> Self {
        Self { dir, extension }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
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

    /// The first stem (`name`, `name-2`, ...) whose three files are all free.
    fn free_targets(&self, name: &str) -> Result<[PathBuf; 3], TransportError> {
        for n in 1..=MAX_SUFFIX {
            let stem = if n == 1 {
                name.to_string()
            } else {
                format!("{name}-{n}")
            };
            let targets = self.targets(&stem);
            let mut taken = false;
            for t in &targets {
                // symlink_metadata also sees dangling links, which a rename would replace.
                taken |= t.symlink_metadata().is_ok();
            }
            if !taken {
                return Ok(targets);
            }
        }
        Err(TransportError::Io(format!(
            "there are already {MAX_SUFFIX} jobs named '{name}' in {}",
            self.dir.display()
        )))
    }
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

fn tmp_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    target.with_file_name(format!(".{name}.tmp"))
}

fn write_tmp(path: &Path, data: &[u8]) -> Result<(), TransportError> {
    let mut f = std::fs::File::create(path).map_err(|e| io_error("create", path, e))?;
    f.write_all(data).map_err(|e| io_error("write", path, e))?;
    f.sync_all().map_err(|e| io_error("write", path, e))
}

/// Writes every `(target, data)` pair through a temp file and renames them in
/// order. On any error, every temp file and every file already renamed is
/// removed again, so a failed job leaves nothing behind.
fn write_all_atomic(files: &[(&Path, &[u8])]) -> Result<(), TransportError> {
    let tmps: Vec<PathBuf> = files.iter().map(|(t, _)| tmp_path(t)).collect();
    let mut renamed: Vec<&Path> = Vec::new();
    let result = (|| {
        for ((_, data), tmp) in files.iter().zip(&tmps) {
            write_tmp(tmp, data)?;
        }
        for ((target, _), tmp) in files.iter().zip(&tmps) {
            std::fs::rename(tmp, target).map_err(|e| io_error("write", target, e))?;
            renamed.push(target);
        }
        Ok(())
    })();
    if result.is_err() {
        for tmp in &tmps {
            let _ = std::fs::remove_file(tmp);
        }
        for target in renamed {
            let _ = std::fs::remove_file(target);
        }
    }
    result
}

impl Transport for FileTransport {
    fn send_job(
        &mut self,
        job: &EmittedJob,
        name: &str,
        log: &mut ByteLog,
    ) -> Result<SendReport, TransportError> {
        // The safety check comes before anything touches the file system.
        check_safe(&job.bytes).map_err(TransportError::Unsafe)?;
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

        std::fs::create_dir_all(&self.dir).map_err(|e| io_error("create", &self.dir, e))?;
        let [bytes_path, txt_path, log_path] = self.free_targets(name)?;
        // The job file goes last: once it exists, its listing and log exist too.
        write_all_atomic(&[
            (&txt_path, listing.as_bytes()),
            (&log_path, jsonl.as_bytes()),
            (&bytes_path, &job.bytes),
        ])?;

        for c in &commands {
            log.push(Dir::Out, c);
        }
        Ok(SendReport {
            files: vec![bytes_path, txt_path, log_path],
            bytes_sent: job.bytes.len(),
        })
    }
}
