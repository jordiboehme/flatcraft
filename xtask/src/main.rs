//! Developer tasks. `cargo xtask capture <file.pcapng>` turns a Wireshark recording into fixtures.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use flatcraft_device::capture::{self, CaptureRecord};
use flatcraft_device::gpgl::decode;

const ETX: u8 = 0x03;
const NO_COMMANDS: &str =
    "warning: no commands found: check SUBMIT/COMPLETE and the device (try --device).";
const TSHARK_MISSING: &str =
    "tshark not found. Install Wireshark (brew install --cask wireshark-app).";

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
        /// Folder for the fixtures. Default: tests/fixtures/captures in the workspace root.
        #[arg(long)]
        out: Option<PathBuf>,
        /// USB device address of the plotter. Default: the address with the most bulk OUT bytes.
        #[arg(long)]
        device: Option<u8>,
        /// Fixture name. Default: the file name without extension.
        #[arg(long)]
        name: Option<String>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().task {
        Task::Capture {
            file,
            out,
            name,
            device,
        } => run_capture(&file, out.as_deref(), name, device),
    }
}

/// The workspace root: the folder above this crate, so the default does not depend on the cwd.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn run_capture(
    file: &Path,
    out_dir: Option<&Path>,
    name: Option<String>,
    device: Option<u8>,
) -> Result<()> {
    let out_dir = out_dir.map_or_else(
        || workspace_root().join("tests/fixtures/captures"),
        Path::to_path_buf,
    );
    let out_dir = out_dir.as_path();
    let name = match name {
        Some(n) => n,
        None => file
            .file_stem()
            .and_then(|s| s.to_str())
            .context("cannot derive a name from the file, pass --name")?
            .to_string(),
    };
    let text = run_tshark(file)?;
    let parsed = capture::parse_tshark(&text);
    if parsed.bad > 0 {
        eprintln!(
            "warning: {} line(s) with data could not be read. First one:\n{}",
            parsed.bad,
            parsed.first_bad.as_deref().unwrap_or("")
        );
    }
    let records = select_device(parsed.records, device)?;
    if !has_commands(&records) {
        eprintln!("{NO_COMMANDS}");
    }
    if records.is_empty() {
        bail!(
            "no USB data found in {}. Check in Wireshark that you see URB_BULK out packets to the Cameo before you capture again, and keep the pcapng.",
            file.display()
        );
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

/// True when there are OUT bytes (commands) to decode.
fn has_commands(records: &[CaptureRecord]) -> bool {
    !capture::split_out_in(records).0.is_empty()
}

/// Keeps the records of one device. Prints the per-device bulk OUT bytes and the address used.
fn select_device(records: Vec<CaptureRecord>, device: Option<u8>) -> Result<Vec<CaptureRecord>> {
    let counts = capture::bulk_out_bytes_by_device(&records);
    if let Some(d) = device
        && !counts.is_empty()
        && !counts.iter().any(|(a, _)| *a == d)
    {
        let valid: Vec<String> = counts.iter().map(|(a, _)| a.to_string()).collect();
        bail!(
            "no bulk OUT data from device {d}. Valid addresses: {}",
            valid.join(", ")
        );
    }
    if counts.is_empty() {
        if device.is_some() {
            bail!("--device given, but the capture has no device addresses with bulk OUT data");
        }
        return Ok(records);
    }
    let list: Vec<String> = counts
        .iter()
        .map(|(d, n)| format!("device {d}: {n} bytes"))
        .collect();
    println!("bulk OUT bytes per device: {}", list.join(", "));
    let used = device.unwrap_or(counts[0].0);
    println!(
        "using device address {used}{}",
        if device.is_some() {
            ""
        } else {
            " (the busiest)"
        }
    );
    Ok(capture::retain_device(records, used))
}

/// Thin wrapper: runs tshark and returns its field output.
fn run_tshark(file: &Path) -> Result<String> {
    let output = Command::new("tshark")
        .arg("-r")
        .arg(file)
        .args(["-T", "fields", "-E", "separator=/t"])
        .args(capture::TSHARK_FIELDS.iter().flat_map(|f| ["-e", *f]))
        .args(["-Y", "usb.capdata"])
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

    // Columns: time, ep, darwin ep, dev, darwin dev, darwin request type, data.
    const SAMPLE: &str = "0.000\t\t0x01\t\t7\t0\t1b:04\n0.010\t\t0x01\t\t7\t0\t\n0.020\t\t0x01\t\t7\t0\t46:47:03\n\
                          0.021\t\t0x01\t\t7\t1\t46:47:03\n\
                          0.030\t\t0x82\t\t7\t1\t43:41:4d:45:4f:20:56:31:2e:31:30:03\n0.040\t\t0x01\t\t7\t0\t48:03\n";

    #[test]
    fn listing_has_commands_and_replies_and_no_doubled_commands() {
        let parsed = capture::parse_tshark(SAMPLE);
        assert_eq!(parsed.bad, 0);
        let listing = listing_text(&parsed.records);
        let (cmds, replies) = listing.split_once("\n# replies\n").unwrap();
        assert_eq!(cmds.lines().next().unwrap(), "<ESC>\\x04\tinit");
        assert_eq!(cmds.lines().filter(|l| l.starts_with("FG\t")).count(), 1);
        assert!(cmds.contains("H\t"));
        assert_eq!(replies, "\"CAMEO V1.10\"\n");
    }

    #[test]
    fn default_device_is_the_busiest_and_others_are_dropped() {
        let text = "0\t\t0x01\t\t3\t0\t48\n1\t\t0x01\t\t7\t0\t48:48:48\n";
        let kept = select_device(capture::parse_tshark(text).records, None).unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].device, Some(7));
        let kept = select_device(capture::parse_tshark(text).records, Some(3)).unwrap();
        assert_eq!(kept[0].device, Some(3));
    }

    #[test]
    fn unknown_device_is_refused_with_the_valid_addresses() {
        let text = "0\t\t0x01\t\t3\t0\t48\n1\t\t0x01\t\t7\t0\t48:48\n";
        let err = select_device(capture::parse_tshark(text).records, Some(9)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("device 9") && msg.contains("7, 3"), "{msg}");
    }

    #[test]
    fn a_device_with_only_in_data_leaves_no_commands() {
        let text = "0\t\t0x82\t\t7\t1\t30:03\n";
        let kept = select_device(capture::parse_tshark(text).records, None).unwrap();
        assert!(!has_commands(&kept));
    }

    #[test]
    fn split_replies_marks_a_tail_without_etx() {
        assert_eq!(
            split_replies(b"0\x031"),
            vec!["\"0\"".to_string(), "\"1\" (no ETX)".to_string()]
        );
    }
}
