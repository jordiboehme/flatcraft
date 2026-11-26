//! The cut log: what was sent, with which settings, and how it went. One
//! JSON object per line in `<config dir>/cut-log.jsonl`, only ever
//! appended to, never sent anywhere.
//!
//! Two kinds of line:
//! - `{"kind":"cut", ...}`: a completed send ([`CutEntry`]), written by
//!   every send that finished, from the window or from an agent;
//! - `{"kind":"outcome","entry":"<id>", ...}`: how that cut went
//!   ([`Outcome`]), which only the person records in the window. A later
//!   outcome of the same entry replaces an earlier one when the log is
//!   read.
//!
//! A line that cannot be read (a crash cut it short, a hand edit broke it)
//! is skipped and named in [`CutLog::diagnostics`]; the rest of the log
//! still counts.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use flatcraft_core::document::CutStrategy;
use serde::{Deserialize, Serialize};

/// The log's file name in the config dir.
pub const FILE: &str = "cut-log.jsonl";

/// The most of the log that is read: the newest part of a larger file.
const MAX_READ_BYTES: u64 = 16 * 1024 * 1024;

/// How a cut went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Good,
    Bad,
}

impl Verdict {
    /// The word the window shows.
    pub fn word(self) -> &'static str {
        match self {
            Verdict::Good => "good",
            Verdict::Bad => "bad",
        }
    }
}

/// What the person said about a cut.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub verdict: Verdict,
    /// A few words on it, maybe empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// When it was recorded, RFC 3339 UTC.
    pub date: String,
}

/// Where a job went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// File Output: the files written.
    File { files: Vec<PathBuf> },
    /// A plotter on USB (milestone 3).
    Usb,
}

/// The size of a job.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct JobSize {
    pub bytes: usize,
    pub toolpaths: usize,
    pub cut_mm: f64,
    pub travel_mm: f64,
}

/// One completed send.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CutEntry {
    /// Unique within the log; outcomes name it.
    pub id: String,
    /// When the send finished, RFC 3339 UTC.
    pub date: String,
    /// The document's name: its project file name, else the job name.
    pub document: String,
    /// The project file, when the document had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    pub plotter_id: String,
    pub plotter: String,
    pub material_id: String,
    pub material: String,
    pub mat_id: String,
    pub mat: String,
    pub speed: u8,
    pub force: u8,
    pub passes: u32,
    pub strategy: CutStrategy,
    /// The keep mode of the objects the job cut: "pieces", "stencil" or
    /// "both" when they all share it, "mixed" when they do not. Entries
    /// written before keep modes have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_mode: Option<String>,
    pub size: JobSize,
    pub target: Target,
    /// How it went: merged in from the outcome lines when the log is read,
    /// never written in this line.
    #[serde(skip)]
    pub outcome: Option<Outcome>,
}

/// One line of the log file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Line {
    Cut(Box<CutEntry>),
    Outcome {
        entry: String,
        #[serde(flatten)]
        outcome: Outcome,
    },
}

/// The log as read: the cuts, oldest first, with their outcomes, and the
/// lines that were skipped.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CutLog {
    pub entries: Vec<CutEntry>,
    /// One text per skipped line, like "cut-log.jsonl line 3: ...", with
    /// the reason in detail, for the window.
    pub diagnostics: Vec<String>,
    /// The same problems without the details, for agents: a reason can
    /// repeat a text of the line, such as a path.
    pub plain_diagnostics: Vec<String>,
}

impl CutLog {
    /// The entry with this id.
    pub fn get(&self, id: &str) -> Option<&CutEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// The cut "Rate the cut of ..." asks about for this material: the
    /// newest one nobody rated yet.
    pub fn to_rate(&self, material_id: &str) -> Option<&CutEntry> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.material_id == material_id && e.outcome.is_none())
    }

    /// The newest cut with this material that the person rated good: the
    /// troubleshooter's "Last good cut with this material".
    pub fn last_good(&self, material_id: &str) -> Option<&CutEntry> {
        self.entries.iter().rev().find(|e| {
            e.material_id == material_id
                && e.outcome
                    .as_ref()
                    .is_some_and(|o| o.verdict == Verdict::Good)
        })
    }

    /// True when the log has a cut with this material: the material
    /// picker suggests a test cut for one that was never cut.
    pub fn has_cut(&self, material_id: &str) -> bool {
        self.entries.iter().any(|e| e.material_id == material_id)
    }

    /// The cut the Job card recalls for this material: the newest with an
    /// outcome, else the newest.
    pub fn last_time(&self, material_id: &str) -> Option<&CutEntry> {
        let mut with_material = self
            .entries
            .iter()
            .rev()
            .filter(|e| e.material_id == material_id);
        let newest = with_material.clone().next();
        with_material.find(|e| e.outcome.is_some()).or(newest)
    }
}

/// The day of an RFC 3339 UTC time (as the log writes them) at
/// `offset_secs` east of UTC, like "2026-11-27" for
/// "2026-11-26T23:30:00Z" at UTC+1. A text that is not such a time shows
/// its first ten characters.
pub fn day_at(date: &str, offset_secs: i32) -> String {
    let fallback = || date.get(..10).unwrap_or(date).to_string();
    let num = |r: std::ops::Range<usize>| date.get(r).and_then(|t| t.parse::<i64>().ok());
    let (Some(y), Some(m), Some(d), Some(h), Some(min), Some(sec)) = (
        num(0..4),
        num(5..7),
        num(8..10),
        num(11..13),
        num(14..16),
        num(17..19),
    ) else {
        return fallback();
    };
    if date.get(10..11) != Some("T") || !date.ends_with('Z') || !(1..=12).contains(&m) {
        return fallback();
    }
    let secs = days_from_civil(y, m, d) * 86_400 + h * 3600 + min * 60 + sec;
    let local = secs + i64::from(offset_secs);
    let (y, m, d) = crate::commands::sources::civil_from_days(local.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 of a date (Howard Hinnant's days-from-civil
/// algorithm, proleptic Gregorian calendar).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The log file in `config_dir`.
pub fn path(config_dir: &Path) -> PathBuf {
    config_dir.join(FILE)
}

/// Now, RFC 3339 UTC.
pub fn now() -> String {
    crate::commands::sources::rfc3339(SystemTime::now()).unwrap_or_default()
}

/// A new entry id: the time in nanoseconds and a counter, so two sends in
/// the same instant still differ.
pub fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{nanos:x}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Reads the log in `config_dir`. A missing file is an empty log; a file
/// that cannot be read is an empty log with a diagnostic. Of a file larger
/// than 16 MB only the newest part is read.
pub fn read(config_dir: &Path) -> CutLog {
    let file = path(config_dir);
    let mut log = CutLog::default();
    let text = match read_tail(&file) {
        Ok(Some(t)) => t,
        Ok(None) => return log,
        Err(e) => {
            log.diagnostics.push(format!("{FILE} cannot be read: {e}"));
            log.plain_diagnostics.push(format!("{FILE} cannot be read"));
            return log;
        }
    };
    let mut outcomes: Vec<(usize, String, Outcome)> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Line>(line) {
            Ok(Line::Cut(e)) => log.entries.push(*e),
            Ok(Line::Outcome { entry, outcome }) => outcomes.push((n, entry, outcome)),
            Err(e) => {
                log.diagnostics
                    .push(format!("{FILE} line {n} was skipped: {e}"));
                log.plain_diagnostics.push(damaged(n));
            }
        }
    }
    for (n, entry, outcome) in outcomes {
        match log.entries.iter_mut().find(|e| e.id == entry) {
            Some(e) => e.outcome = Some(outcome),
            None => {
                log.diagnostics.push(format!(
                    "{FILE} line {n} was skipped: it is about cut {entry}, which is not in the log"
                ));
                log.plain_diagnostics.push(damaged(n));
            }
        }
    }
    log
}

/// A skipped line, said without its content.
fn damaged(n: usize) -> String {
    format!("{FILE} line {n} was skipped (damaged)")
}

/// The file's text, or its newest [`MAX_READ_BYTES`] from the first whole
/// line on. `None` when there is no file.
fn read_tail(file: &Path) -> std::io::Result<Option<String>> {
    let mut f = match std::fs::File::open(file) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let len = f.metadata()?.len();
    let start = len.saturating_sub(MAX_READ_BYTES);
    f.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    f.take(MAX_READ_BYTES).read_to_end(&mut bytes)?;
    if start > 0 {
        // The first line is likely cut in two: start after it.
        let cut = bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |p| p + 1);
        bytes.drain(..cut);
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// Appends one line. A last line that a crash left without its line end
/// gets one first, so the new line stays readable.
fn append_line(config_dir: &Path, line: &Line) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let file = path(config_dir);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(&file)?;
    let mut text = String::new();
    if f.metadata()?.len() > 0 {
        f.seek(SeekFrom::End(-1))?;
        let mut last = [0u8; 1];
        f.read_exact(&mut last)?;
        if last[0] != b'\n' {
            text.push('\n');
        }
    }
    text.push_str(&serde_json::to_string(line).map_err(std::io::Error::other)?);
    text.push('\n');
    f.write_all(text.as_bytes())?;
    f.sync_data()
}

/// Appends a completed send.
pub fn append(config_dir: &Path, entry: &CutEntry) -> std::io::Result<()> {
    let mut entry = entry.clone();
    entry.outcome = None;
    append_line(config_dir, &Line::Cut(Box::new(entry)))
}

/// Appends how the cut `entry` went. Not reachable from the command
/// registry: only the person records outcomes, in the window
/// ([`crate::session::Session::record_outcome`]).
pub(crate) fn append_outcome(
    config_dir: &Path,
    entry: &str,
    outcome: &Outcome,
) -> std::io::Result<()> {
    append_line(
        config_dir,
        &Line::Outcome {
            entry: entry.to_string(),
            outcome: outcome.clone(),
        },
    )
}

/// Whether the window asks "How did it go?" right after a send to
/// `target`. Only a cut on a plotter: File Output cut nothing yet.
pub fn asks_after(target: &Target) -> bool {
    matches!(target, Target::Usb)
}
