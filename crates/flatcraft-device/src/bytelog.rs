//! Ring buffer of the bytes sent to and read from a plotter.

use std::collections::VecDeque;
use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Most entries a log keeps; older ones are dropped.
pub const CAPACITY: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Out,
    In,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Milliseconds since the log was created.
    pub t_ms: u64,
    pub dir: Dir,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ByteLog {
    start: Instant,
    entries: VecDeque<LogEntry>,
}

impl Default for ByteLog {
    fn default() -> Self {
        Self::new()
    }
}

impl ByteLog {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            entries: VecDeque::new(),
        }
    }

    /// Milliseconds since the log was created.
    pub fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Adds an entry stamped with the time since the log was created. When
    /// the log is full the oldest entry is dropped.
    pub fn push(&mut self, dir: Dir, bytes: &[u8]) {
        let t_ms = self.elapsed_ms();
        self.push_entry(LogEntry {
            t_ms,
            dir,
            bytes: bytes.to_vec(),
        });
    }

    /// Adds a finished entry, for example one copied from another log.
    pub fn push_entry(&mut self, entry: LogEntry) {
        if self.entries.len() == CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn entries(&self) -> impl Iterator<Item = &LogEntry> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// One JSON object per line, in the capture fixture format:
    /// `{"t": <seconds>, "dir": "out"|"in", "ep": "0x01"|"0x82", "hex": ".."}`.
    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        for e in &self.entries {
            out.push_str(&jsonl_line(
                e.t_ms as f64 / 1000.0,
                e.dir,
                default_ep(e.dir),
                &e.bytes,
            ));
            out.push('\n');
        }
        out
    }
}

/// One JSONL line of the capture fixture format. The struct keeps the key order t, dir, ep,
/// hex. `ByteLog` and the capture module both read and write exactly this.
#[derive(Serialize, Deserialize)]
pub(crate) struct Line {
    pub t: f64,
    pub dir: Dir,
    pub ep: String,
    pub hex: String,
}

/// One JSONL line (no newline) for a transfer.
pub(crate) fn jsonl_line(t: f64, dir: Dir, ep: u8, bytes: &[u8]) -> String {
    let line = Line {
        t,
        dir,
        ep: format!("0x{ep:02x}"),
        hex: hex(bytes),
    };
    serde_json::to_string(&line).expect("a log line always serialises")
}

/// The default endpoint of a direction on the Cameo 2: 0x01 out, 0x82 in.
fn default_ep(dir: Dir) -> u8 {
    match dir {
        Dir::Out => 0x01,
        Dir::In => 0x82,
    }
}

/// Lowercase hex, two digits per byte.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    s
}

/// Bytes from lowercase or uppercase hex. `None` for an odd length or a non-hex character.
pub(crate) fn unhex(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    b.as_chunks::<2>()
        .0
        .iter()
        .map(|p| {
            let hi = char::from(p[0]).to_digit(16)?;
            let lo = char::from(p[1]).to_digit(16)?;
            u8::try_from(hi * 16 + lo).ok()
        })
        .collect()
}
