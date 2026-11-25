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
    /// USB device address when the capture has one. Left out of the JSONL when `None`.
    pub device: Option<u8>,
    pub bytes: Vec<u8>,
}

/// The tshark `-e` fields, in the column order `parse_tshark_line` reads. macOS captures
/// (LINKTYPE_USB_DARWIN) fill the `darwin` columns; Linux usbmon and Windows USBPcap fill the
/// plain ones. The parser takes whichever is not empty.
pub const TSHARK_FIELDS: [&str; 7] = [
    "frame.time_relative",
    "usb.endpoint_address",
    "usb.darwin.endpoint_address",
    "usb.device_address",
    "usb.darwin.device_address",
    "usb.darwin.request_type",
    "usb.capdata",
];

/// What one tshark line turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedLine {
    Record(CaptureRecord),
    /// A readable line that carries nothing we keep: no data, OUT data on the COMPLETE packet
    /// or IN data on the SUBMIT packet (the same bytes already appear on the other packet).
    Skipped,
    /// Data that could not be read.
    Bad,
}

/// Parses one tab separated line with the columns of `TSHARK_FIELDS`.
pub fn parse_tshark_row(line: &str) -> ParsedLine {
    let line = line.trim_end_matches(['\r', '\n']);
    let col: Vec<&str> = line.split('\t').map(str::trim).collect();
    let get = |i: usize| col.get(i).copied().unwrap_or("");
    // Commas join several data fields in one cell and colons separate bytes: drop both.
    let data: String = get(6).chars().filter(|c| *c != ':' && *c != ',').collect();
    if data.is_empty() {
        return ParsedLine::Skipped;
    }
    let first = |a: &str, b: &str| {
        if a.is_empty() {
            b.to_string()
        } else {
            a.to_string()
        }
    };
    let (Ok(t), Some(ep), Some(bytes)) = (
        get(0).parse::<f64>(),
        parse_ep(&first(get(1), get(2))),
        unhex(&data),
    ) else {
        return ParsedLine::Bad;
    };
    let dir = if ep & 0x80 != 0 { Dir::In } else { Dir::Out };
    // Darwin request type: 0 is SUBMIT, 1 is COMPLETE. OUT data is in the SUBMIT, IN data in
    // the COMPLETE. Taking the other packet would double every command.
    match (dir, parse_num(get(5))) {
        (Dir::Out, Some(1)) | (Dir::In, Some(0)) => return ParsedLine::Skipped,
        _ => {}
    }
    let device = match first(get(3), get(4)).as_str() {
        "" => None,
        d => match parse_num(d) {
            Some(d) => Some(d),
            None => return ParsedLine::Bad,
        },
    };
    ParsedLine::Record(CaptureRecord {
        t,
        dir,
        ep,
        device,
        bytes,
    })
}

/// The record of one line, `None` for lines that are skipped or bad.
pub fn parse_tshark_line(line: &str) -> Option<CaptureRecord> {
    match parse_tshark_row(line) {
        ParsedLine::Record(r) => Some(r),
        _ => None,
    }
}

/// The result of reading a whole tshark output.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TsharkParse {
    pub records: Vec<CaptureRecord>,
    /// Non-empty lines with data that could not be read.
    pub bad: usize,
    pub first_bad: Option<String>,
}

/// Reads the whole output. Bad lines are counted, never silently dropped.
pub fn parse_tshark(text: &str) -> TsharkParse {
    let mut out = TsharkParse::default();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match parse_tshark_row(line) {
            ParsedLine::Record(r) => out.records.push(r),
            ParsedLine::Skipped => {}
            ParsedLine::Bad => {
                out.bad += 1;
                if out.first_bad.is_none() {
                    out.first_bad = Some(line.to_string());
                }
            }
        }
    }
    out
}

/// Bulk OUT bytes per device address, the busiest first. Records without an address are
/// not counted.
pub fn bulk_out_bytes_by_device(records: &[CaptureRecord]) -> Vec<(u8, usize)> {
    let mut counts: Vec<(u8, usize)> = Vec::new();
    for r in records {
        let (Some(dev), Dir::Out) = (r.device, r.dir) else {
            continue;
        };
        if r.ep & 0x7f == 0 {
            continue;
        }
        match counts.iter_mut().find(|(d, _)| *d == dev) {
            Some((_, n)) => *n += r.bytes.len(),
            None => counts.push((dev, r.bytes.len())),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    counts
}

/// The records of one device. Records without an address are kept.
pub fn retain_device(records: Vec<CaptureRecord>, device: u8) -> Vec<CaptureRecord> {
    records
        .into_iter()
        .filter(|r| r.device.is_none_or(|d| d == device))
        .collect()
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
            device: line.dev,
            bytes,
        });
    }
    Ok(records)
}

/// Writes the JSONL fixture format, one line per record.
pub fn to_jsonl(records: &[CaptureRecord]) -> String {
    let mut out = String::new();
    for r in records {
        out.push_str(&jsonl_line(r.t, r.dir, r.ep, r.device, &r.bytes));
        out.push('\n');
    }
    out
}

/// An endpoint address. Always hex, with or without the `0x` prefix: `0x82` and `82` are the
/// same endpoint.
fn parse_ep(s: &str) -> Option<u8> {
    let s = s.trim();
    let digits = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u8::from_str_radix(digits, 16).ok()
}

/// A device address or a request type: decimal, or hex with a `0x` prefix.
fn parse_num(s: &str) -> Option<u8> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u8::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(t: f64, dir: Dir, ep: u8, bytes: &[u8]) -> CaptureRecord {
        CaptureRecord {
            t,
            dir,
            ep,
            device: None,
            bytes: bytes.to_vec(),
        }
    }

    // Columns: time, ep, darwin ep, dev, darwin dev, darwin request type, data.
    #[test]
    fn parses_out_line() {
        let r = parse_tshark_line("0.123\t0x01\t\t\t\t\t1b:04").unwrap();
        assert_eq!(r, rec(0.123, Dir::Out, 0x01, &[0x1b, 0x04]));
    }

    #[test]
    fn parses_in_line() {
        let r = parse_tshark_line("1.5\t0x82\t\t\t\t\t30:03").unwrap();
        assert_eq!(r.dir, Dir::In);
        assert_eq!(r.ep, 0x82);
        assert_eq!(r.bytes, b"0\x03");
    }

    #[test]
    fn parses_a_darwin_line_with_the_plain_endpoint_empty() {
        let r = parse_tshark_line("2.0\t\t0x01\t\t7\t0\t46:47:03").unwrap();
        assert_eq!(r.ep, 0x01);
        assert_eq!(r.device, Some(7));
        assert_eq!(r.dir, Dir::Out);
        assert_eq!(r.bytes, b"FG\x03");
    }

    #[test]
    fn endpoints_are_always_hex_with_or_without_the_prefix() {
        let r = parse_tshark_line("0\t82\t\t\t\t\t30").unwrap();
        assert_eq!(r.ep, 0x82);
        assert_eq!(r.dir, Dir::In);
        let r = parse_tshark_line("0\t\t0x01\t\t\t\t30").unwrap();
        assert_eq!(r.ep, 0x01);
        let jsonl = "{\"t\":0.0,\"dir\":\"in\",\"ep\":\"82\",\"hex\":\"30\"}";
        assert_eq!(read_jsonl(jsonl).unwrap()[0].ep, 0x82);
    }

    #[test]
    fn device_addresses_are_decimal() {
        let r = parse_tshark_line("0\t0x01\t\t10\t\t\t30").unwrap();
        assert_eq!(r.device, Some(10));
    }

    #[test]
    fn parses_a_usbpcap_line() {
        let r = parse_tshark_line("0.5\t0x82\t\t3\t\t\t30:03").unwrap();
        assert_eq!(r.ep, 0x82);
        assert_eq!(r.device, Some(3));
        assert_eq!(r.dir, Dir::In);
    }

    #[test]
    fn darwin_out_data_comes_only_from_submit() {
        assert!(parse_tshark_line("1\t\t0x01\t\t7\t0\t48:03").is_some());
        assert!(parse_tshark_line("1\t\t0x01\t\t7\t1\t48:03").is_none());
    }

    #[test]
    fn darwin_in_data_comes_only_from_complete() {
        assert!(parse_tshark_line("1\t\t0x82\t\t7\t1\t30:03").is_some());
        assert!(parse_tshark_line("1\t\t0x82\t\t7\t0\t30:03").is_none());
    }

    #[test]
    fn skips_lines_without_capdata() {
        assert_eq!(parse_tshark_row("0.1\t0x01\t\t\t\t\t"), ParsedLine::Skipped);
        assert_eq!(parse_tshark_row("0.1\t0x01"), ParsedLine::Skipped);
        assert_eq!(parse_tshark_row(""), ParsedLine::Skipped);
    }

    #[test]
    fn unreadable_data_is_bad() {
        for bad in [
            "x\t0x01\t\t\t\t\t1b",
            "0.1\t0x01\t\t\t\t\tzz",
            "0.1\t0x01\t\t\t\t\t1b0",
            "0.1\t\t\t\t\t\t1b",
            "0.1\t0x01\t\tq\t\t\t1b",
        ] {
            assert_eq!(parse_tshark_row(bad), ParsedLine::Bad, "{bad}");
        }
    }

    #[test]
    fn comma_joined_data_is_concatenated() {
        let r = parse_tshark_line("0.5\t0x01\t\t\t\t\t46:47,03").unwrap();
        assert_eq!(r.bytes, b"FG\x03");
    }

    #[test]
    fn tolerates_trailing_newline_and_crlf() {
        let r = parse_tshark_line("0.5\t0x01\t\t\t\t\t48:03\r\n").unwrap();
        assert_eq!(r.bytes, b"H\x03");
    }

    #[test]
    fn parse_tshark_counts_bad_lines_and_keeps_the_first() {
        let text = "0.1\t0x01\t\t\t\t\t48\n\nbroken\t0x01\t\t\t\t\t48\n0.2\t0x01\t\t\t\t\tzz\n0.3\t0x01\t\t\t\t\t\n";
        let p = parse_tshark(text);
        assert_eq!(p.records.len(), 1);
        assert_eq!(p.bad, 2);
        assert!(p.first_bad.unwrap().starts_with("broken"));
    }

    #[test]
    fn device_counts_use_bulk_out_bytes_and_the_busiest_wins() {
        let d = |dev, dir, ep, n: usize| CaptureRecord {
            device: Some(dev),
            ..rec(0.0, dir, ep, &vec![0; n])
        };
        let records = vec![
            d(3, Dir::Out, 0x02, 10),
            d(7, Dir::Out, 0x01, 100),
            d(7, Dir::Out, 0x00, 5000),
            d(3, Dir::In, 0x81, 9000),
            d(7, Dir::Out, 0x01, 20),
            rec(0.0, Dir::Out, 0x01, b"no address"),
        ];
        assert_eq!(bulk_out_bytes_by_device(&records), vec![(7, 120), (3, 10)]);
        let kept = retain_device(records, 7);
        assert_eq!(kept.len(), 4);
        assert!(kept.iter().all(|r| r.device != Some(3)));
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
    fn jsonl_keeps_the_device_address_when_there_is_one() {
        let records = vec![CaptureRecord {
            device: Some(7),
            ..rec(0.5, Dir::Out, 0x01, b"H")
        }];
        let text = to_jsonl(&records);
        assert!(text.contains("\"dev\":7"));
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
