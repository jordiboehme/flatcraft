//! Developer tasks. `cargo xtask capture <file.pcapng>` turns a Wireshark recording into fixtures.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use flatcraft_device::capture::{self, CaptureRecord};
use flatcraft_device::gpgl::decode;

const ETX: u8 = 0x03;
const TSHARK_MISSING: &str = "tshark not found. Install Wireshark (brew install --cask wireshark).";

#[derive(Parser)]
#[command(name = "xtask", about = "FlatCraft developer tasks")]
struct Cli {
    #[command(subcommand)]
    task: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Turn a Wireshark .pcapng into <name>.jsonl and <name>.gpgl.txt fixtures.
    Capture {
        /// The recording.
        file: PathBuf,
        /// Folder for the fixtures.
        #[arg(long, default_value = "tests/fixtures/captures")]
        out: PathBuf,
        /// Fixture name. Default: the file name without extension.
        #[arg(long)]
        name: Option<String>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().task {
        Task::Capture { file, out, name } => run_capture(&file, &out, name),
    }
}

fn run_capture(file: &Path, out_dir: &Path, name: Option<String>) -> Result<()> {
    let name = match name {
        Some(n) => n,
        None => file
            .file_stem()
            .and_then(|s| s.to_str())
            .context("cannot derive a name from the file, pass --name")?
            .to_string(),
    };
    let text = run_tshark(file)?;
    let records = parse_tshark_output(&text);
    if records.is_empty() {
        bail!("no USB data found in {}", file.display());
    }
    std::fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    let jsonl = out_dir.join(format!("{name}.jsonl"));
    let listing = out_dir.join(format!("{name}.gpgl.txt"));
    std::fs::write(&jsonl, capture::to_jsonl(&records))
        .with_context(|| format!("write {}", jsonl.display()))?;
    std::fs::write(&listing, listing_text(&records))
        .with_context(|| format!("write {}", listing.display()))?;
    println!(
        "{} transfers -> {} and {}",
        records.len(),
        jsonl.display(),
        listing.display()
    );
    Ok(())
}

/// Thin wrapper: runs tshark and returns its field output.
fn run_tshark(file: &Path) -> Result<String> {
    let output = Command::new("tshark")
        .arg("-r")
        .arg(file)
        .args(["-T", "fields", "-E", "separator=/t"])
        .args(["-e", "frame.time_relative", "-e", "usb.endpoint_address"])
        .args(["-e", "usb.capdata", "-Y", "usb.capdata"])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(TSHARK_MISSING)
            } else {
                anyhow::Error::new(e).context("run tshark")
            }
        })?;
    if !output.status.success() {
        bail!(
            "tshark failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_tshark_output(text: &str) -> Vec<CaptureRecord> {
    text.lines()
        .filter_map(capture::parse_tshark_line)
        .collect()
}

/// The decoded OUT bytes, one command per line, then the ETX-split replies under `# replies`.
fn listing_text(records: &[CaptureRecord]) -> String {
    let (out_bytes, in_bytes) = capture::split_out_in(records);
    let mut s = String::new();
    for c in decode(&out_bytes) {
        let _ = writeln!(s, "{}\t{}", c.raw, c.meaning);
    }
    s.push_str("\n# replies\n");
    for reply in split_replies(&in_bytes) {
        let _ = writeln!(s, "{reply}");
    }
    s
}

/// Replies end with ETX. A tail without ETX is kept and marked.
fn split_replies(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        match rest.iter().position(|b| *b == ETX) {
            Some(p) => {
                out.push(format!("{:?}", String::from_utf8_lossy(&rest[..p])));
                rest = &rest[p + 1..];
            }
            None => {
                out.push(format!("{:?} (no ETX)", String::from_utf8_lossy(rest)));
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "0.000\t0x01\t1b:04\n0.010\t0x01\t\n0.020\t0x01\t46:47:03\n\
                          0.030\t0x82\t43:41:4d:45:4f:20:56:31:2e:31:30:03\n0.040\t0x01\t48:03\n";

    #[test]
    fn parses_tshark_output_and_skips_empty_lines() {
        let records = parse_tshark_output(SAMPLE);
        assert_eq!(records.len(), 4);
    }

    #[test]
    fn listing_has_commands_and_replies() {
        let listing = listing_text(&parse_tshark_output(SAMPLE));
        let (cmds, replies) = listing.split_once("\n# replies\n").unwrap();
        assert!(cmds.contains("<ESC>\\x04\t") || cmds.starts_with("<ESC>"));
        assert!(cmds.contains("FG\t"));
        assert!(cmds.contains("H\t"));
        assert_eq!(replies, "\"CAMEO V1.10\"\n");
    }

    #[test]
    fn split_replies_marks_a_tail_without_etx() {
        assert_eq!(
            split_replies(b"0\x031"),
            vec!["\"0\"".to_string(), "\"1\" (no ETX)".to_string()]
        );
    }
}
