//! Capture records: USB transfers recorded with Wireshark, turned into fixtures.
//!
//! The JSONL format is the one `ByteLog::to_jsonl` writes: `{"t":..,"dir":..,"ep":..,"hex":..}`.

use crate::bytelog::{Dir, Line, jsonl_line, unhex};

/// One USB transfer with data.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureRecord {
    /// Seconds since the start of the capture.
    pub t: f64,
    pub dir: Dir,
    /// Endpoint address. Bit 0x80 set means device to host.
    pub ep: u8,
    pub bytes: Vec<u8>,
}

/// Parses one line of `tshark -T fields -e frame.time_relative -e usb.endpoint_address
/// -e usb.capdata` (tab separated). `None` when the line has no data or cannot be read.
pub fn parse_tshark_line(line: &str) -> Option<CaptureRecord> {
    let mut fields = line.trim_end_matches(['\r', '\n']).split('\t');
    let t: f64 = fields.next()?.trim().parse().ok()?;
    let ep = parse_ep(fields.next()?)?;
    // tshark joins several data fields with a comma; the colons inside one are dropped.
    let data: String = fields.next()?.chars().filter(|c| *c != ':').collect();
    let bytes = unhex(&data)?;
    if bytes.is_empty() {
        return None;
    }
    let dir = if ep & 0x80 != 0 { Dir::In } else { Dir::Out };
    Some(CaptureRecord { t, dir, ep, bytes })
}

/// The OUT bytes and the IN bytes, each in capture order. Control transfers (endpoint 0x00
/// and 0x80) are left out.
pub fn split_out_in(records: &[CaptureRecord]) -> (Vec<u8>, Vec<u8>) {
    let mut out = Vec::new();
    let mut inb = Vec::new();
    for r in records {
        match (r.dir, r.ep) {
            (_, 0x00 | 0x80) => {}
            (Dir::Out, _) => out.extend_from_slice(&r.bytes),
            (Dir::In, _) => inb.extend_from_slice(&r.bytes),
        }
    }
    (out, inb)
}

/// Reads the JSONL fixture format. Errors name the line number. Blank lines are skipped.
pub fn read_jsonl(text: &str) -> Result<Vec<CaptureRecord>, String> {
    let mut records = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let n = i + 1;
        let line: Line = serde_json::from_str(raw).map_err(|e| format!("line {n}: {e}"))?;
        let ep = parse_ep(&line.ep).ok_or_else(|| {
            format!(
                "line {n}: bad endpoint {:?}, expected like \"0x01\"",
                line.ep
            )
        })?;
        let bytes = unhex(&line.hex).ok_or_else(|| format!("line {n}: bad hex"))?;
        records.push(CaptureRecord {
            t: line.t,
            dir: line.dir,
            ep,
            bytes,
        });
    }
    Ok(records)
}

/// Writes the JSONL fixture format, one line per record.
pub fn to_jsonl(records: &[CaptureRecord]) -> String {
    let mut out = String::new();
    for r in records {
        out.push_str(&jsonl_line(r.t, r.dir, r.ep, &r.bytes));
        out.push('\n');
    }
    out
}

/// `0x82` (or `82`) to the endpoint address.
fn parse_ep(s: &str) -> Option<u8> {
    let s = s.trim();
    let digits = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u8::from_str_radix(digits, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(t: f64, dir: Dir, ep: u8, bytes: &[u8]) -> CaptureRecord {
        CaptureRecord {
            t,
            dir,
            ep,
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn parses_out_line() {
        let r = parse_tshark_line("0.123\t0x01\t1b:04").unwrap();
        assert_eq!(r, rec(0.123, Dir::Out, 0x01, &[0x1b, 0x04]));
    }

    #[test]
    fn parses_in_line() {
        let r = parse_tshark_line("1.5\t0x82\t30:03").unwrap();
        assert_eq!(r.dir, Dir::In);
        assert_eq!(r.ep, 0x82);
        assert_eq!(r.bytes, b"0\x03");
    }

    #[test]
    fn skips_lines_without_capdata() {
        assert!(parse_tshark_line("0.1\t0x01\t").is_none());
        assert!(parse_tshark_line("0.1\t0x01").is_none());
        assert!(parse_tshark_line("").is_none());
        assert!(parse_tshark_line("x\t0x01\t1b").is_none());
        assert!(parse_tshark_line("0.1\t0x01\tzz").is_none());
        assert!(parse_tshark_line("0.1\t0x01\t1b0").is_none());
    }

    #[test]
    fn tolerates_trailing_newline_and_crlf() {
        let r = parse_tshark_line("0.5\t0x01\t48:03\r\n").unwrap();
        assert_eq!(r.bytes, b"H\x03");
    }

    #[test]
    fn split_concatenates_in_order() {
        let records = [
            rec(0.0, Dir::Out, 0x01, b"FG"),
            rec(0.1, Dir::In, 0x82, b"A"),
            rec(0.2, Dir::Out, 0x01, b"\x03H"),
            rec(0.3, Dir::In, 0x82, b"B\x03"),
            rec(0.4, Dir::Out, 0x00, b"setup"),
        ];
        let (out, inb) = split_out_in(&records);
        assert_eq!(out, b"FG\x03H");
        assert_eq!(inb, b"AB\x03");
    }

    #[test]
    fn jsonl_round_trips_and_matches_bytelog_shape() {
        let records = vec![
            rec(0.123, Dir::Out, 0x01, &[0x1b, 0x04]),
            rec(1.5, Dir::In, 0x82, b"0\x03"),
        ];
        let text = to_jsonl(&records);
        assert_eq!(
            text,
            "{\"t\":0.123,\"dir\":\"out\",\"ep\":\"0x01\",\"hex\":\"1b04\"}\n\
             {\"t\":1.5,\"dir\":\"in\",\"ep\":\"0x82\",\"hex\":\"3003\"}\n"
        );
        assert_eq!(read_jsonl(&text).unwrap(), records);
    }

    #[test]
    fn read_jsonl_refuses_bad_lines_with_the_line_number() {
        let good = "{\"t\":0.0,\"dir\":\"out\",\"ep\":\"0x01\",\"hex\":\"48\"}";
        let bad_hex = "{\"t\":0.0,\"dir\":\"out\",\"ep\":\"0x01\",\"hex\":\"4z\"}";
        let bad_dir = "{\"t\":0.0,\"dir\":\"up\",\"ep\":\"0x01\",\"hex\":\"48\"}";
        let bad_ep = "{\"t\":0.0,\"dir\":\"out\",\"ep\":\"one\",\"hex\":\"48\"}";
        for bad in [bad_hex, bad_dir, bad_ep, "not json"] {
            let err = read_jsonl(&format!("{good}\n\n{bad}\n")).unwrap_err();
            assert!(err.starts_with("line 3:"), "{err}");
        }
        assert!(read_jsonl(&format!("{good}\n\n")).is_ok());
    }

    #[test]
    fn handles_a_large_capture() {
        let records: Vec<_> = (0..50_000)
            .map(|i| rec(f64::from(i) * 0.001, Dir::Out, 0x01, &[0xab; 64]))
            .collect();
        let text = to_jsonl(&records);
        assert_eq!(read_jsonl(&text).unwrap().len(), 50_000);
    }
}
