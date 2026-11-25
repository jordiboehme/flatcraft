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

/// PROVISIONAL endpoints for a log made with `ByteLog::new`: 0x01 out, 0x82 in, as expected on
/// the Cameo 2. gpgl.md section 1 has no Cameo 2 descriptor dump for them (open question 1), so
/// the real ones come from the USB descriptor.
/// TODO(M3, USB transport): the transport passes its descriptor endpoints through
/// `ByteLog::with_endpoints` and these defaults go away.
pub const PROVISIONAL_OUT_EP: u8 = 0x01;
pub const PROVISIONAL_IN_EP: u8 = 0x82;

#[derive(Debug, Clone)]
pub struct ByteLog {
    out_ep: u8,
    in_ep: u8,
    start: Instant,
    entries: VecDeque<LogEntry>,
    /// Counts the entries ever added, so a viewer can spot a change even
    /// when a full log keeps its length.
    revision: u64,
}

impl Default for ByteLog {
    fn default() -> Self {
        Self::new()
    }
}

impl ByteLog {
    pub fn new() -> Self {
        Self::with_endpoints(PROVISIONAL_OUT_EP, PROVISIONAL_IN_EP)
    }

    /// A log that writes these endpoint addresses (from the transport's descriptor) into its JSONL.
    pub fn with_endpoints(out_ep: u8, in_ep: u8) -> Self {
        Self {
            out_ep,
            in_ep,
            start: Instant::now(),
            entries: VecDeque::new(),
            revision: 0,
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
        self.revision += 1;
    }

    /// Counts the entries ever added. It changes with every entry, also
    /// when a full log drops its oldest one.
    pub fn revision(&self) -> u64 {
        self.revision
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
    fn endpoint(&self, dir: Dir) -> u8 {
        match dir {
            Dir::Out => self.out_ep,
            Dir::In => self.in_ep,
        }
    }

    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        for e in &self.entries {
            out.push_str(&jsonl_line(
                e.t_ms as f64 / 1000.0,
                e.dir,
                self.endpoint(e.dir),
                None,
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
    /// USB device address, when the capture knows it. Left out of the JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev: Option<u8>,
}

/// One JSONL line (no newline) for a transfer.
pub(crate) fn jsonl_line(t: f64, dir: Dir, ep: u8, dev: Option<u8>, bytes: &[u8]) -> String {
    let line = Line {
        t,
        dir,
        ep: format!("0x{ep:02x}"),
        hex: hex(bytes),
        dev,
    };
    serde_json::to_string(&line).expect("a log line always serialises")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_come_from_the_log() {
        let mut log = ByteLog::with_endpoints(0x03, 0x84);
        log.push(Dir::Out, b"H");
        log.push(Dir::In, b"0");
        let text = log.to_jsonl();
        assert!(text.contains("\"ep\":\"0x03\""));
        assert!(text.contains("\"ep\":\"0x84\""));
        assert!(!text.contains("dev"));
    }
}
