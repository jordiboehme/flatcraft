//! Explains GPGL bytes command by command (meanings from gpgl.md sections 3, 5 and 6).
//!
//! It works on any bytes and never panics.

use super::meaning;

const ESC: u8 = 0x1b;
const ETX: u8 = 0x03;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedCommand {
    /// The command without ETX, ESC shown as `<ESC>`, other control bytes as `\xNN`.
    pub raw: String,
    pub meaning: String,
}

/// Readable form of raw command bytes: printable ASCII as is, ESC as `<ESC>`, every other
/// byte as `\xNN`.
pub(crate) fn render_raw(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            ESC => s.push_str("<ESC>"),
            0x20..=0x7e => s.push(char::from(b)),
            _ => s.push_str(&format!("\\x{b:02x}")),
        }
    }
    s
}

/// Comma-separated integer arguments. `None` when any of them is not an integer.
fn args(rest: &str) -> Option<Vec<i64>> {
    if rest.is_empty() {
        return Some(Vec::new());
    }
    rest.split(',').map(|t| t.parse::<i64>().ok()).collect()
}

fn esc_meaning(code: u8) -> String {
    match code {
        0x04 => meaning::INIT.into(),
        0x05 => meaning::STATUS_QUERY.into(),
        0x15 => "tool setup (Cameo 3 and later only)".into(),
        _ => meaning::UNKNOWN.into(),
    }
}

/// The number after `TB`, read loosely: spaces, signs and leading zeros are skipped, so
/// `TB072`, `TB 72` and `TB+72` all give 72. Used by the decoder and by the safety check to
/// name the forbidden calibration write. `None` when no digits follow.
pub(crate) fn tb_code(after_tb: &[u8]) -> Option<u64> {
    let mut it = after_tb
        .iter()
        .copied()
        .skip_while(|b| matches!(b, b' ' | b'+' | b'-'))
        .peekable();
    if !it.peek().is_some_and(|b| b.is_ascii_digit()) {
        return None;
    }
    // Fold the digits, saturating: a huge number is never 72.
    Some(it.take_while(u8::is_ascii_digit).fold(0u64, |n, d| {
        n.saturating_mul(10).saturating_add(u64::from(d - b'0'))
    }))
}

fn text_meaning(cmd: &[u8]) -> Option<String> {
    if cmd.starts_with(b"FB") {
        return Some(meaning::forbidden("FB"));
    }
    if cmd
        .strip_prefix(b"TB")
        .is_some_and(|after| tb_code(after) == Some(72))
    {
        return Some(meaning::forbidden("TB72"));
    }
    let s = std::str::from_utf8(cmd).ok()?;
    if let Some(rest) = s.strip_prefix("TB") {
        let (code, rest) = rest.split_once(',').unwrap_or((rest, ""));
        return match code {
            "71" => Some("calibration read".into()),
            "50" => match args(rest)?.as_slice() {
                [0] => Some(meaning::tb50(false).into()),
                [1] => Some(meaning::tb50(true).into()),
                _ => None,
            },
            "99" | "52" | "51" | "53" | "55" => {
                Some("registration mark setup (not used by FlatCraft)".into())
            }
            "123" => Some("registration mark search, automatic (not used by FlatCraft)".into()),
            "23" => Some("registration mark search, manual (not used by FlatCraft)".into()),
            _ => None,
        };
    }
    let one = |rest: &str| -> Option<i64> {
        match args(rest)?.as_slice() {
            [v] => Some(*v),
            _ => None,
        }
    };
    match s {
        "FG" => return Some(meaning::FIRMWARE_QUERY.into()),
        "FA" => return Some("calibration read".into()),
        "SO0" => return Some("set origin here".into()),
        "H" => return Some(meaning::HOME.into()),
        "[" => return Some("query upper-left limit".into()),
        "U" => return Some("query lower-right limit".into()),
        "FN0" => return Some(meaning::ORIENTATION_PORTRAIT.into()),
        "FY0" => return Some(meaning::track_enhancing(true).into()),
        "FY1" => return Some(meaning::track_enhancing(false).into()),
        "L0" => return Some(meaning::LINE_SOLID.into()),
        _ => {}
    }
    if let Some(rest) = s.strip_prefix("FW") {
        return Some(format!("media preset {}", one(rest)?));
    }
    if let Some(rest) = s.strip_prefix("FX") {
        return match args(rest)?.as_slice() {
            [f] => Some(meaning::force(*f)),
            [f, t] => Some(format!("force {f}, tool {t} (Cameo 3 and later form)")),
            _ => None,
        };
    }
    if let Some(rest) = s.strip_prefix("FC") {
        return match args(rest)?.as_slice() {
            [p] => Some(meaning::blade_offset(*p)),
            [p, q, t] => Some(format!(
                "blade offset {p},{q}, tool {t} (Cameo 3 and later form)"
            )),
            _ => None,
        };
    }
    if let Some(rest) = s.strip_prefix("FE") {
        let a = args(rest)?;
        if a.is_empty() {
            return None;
        }
        return Some(meaning::lift(&a));
    }
    if let Some(rest) = s.strip_prefix("FF") {
        return match args(rest)?.as_slice() {
            [st, e, t] => Some(meaning::corner(*st, *e, *t)),
            _ => None,
        };
    }
    if let Some(rest) = s.strip_prefix('!') {
        return match args(rest)?.as_slice() {
            [v] => Some(meaning::speed(*v)),
            [v, t] => Some(format!("speed {v}, tool {t} (Cameo 3 and later form)")),
            _ => None,
        };
    }
    let point = |rest: &str| -> Option<(i64, i64)> {
        match args(rest)?.as_slice() {
            [y, x] => Some((*y, *x)),
            _ => None,
        }
    };
    if let Some(rest) = s.strip_prefix('\\') {
        let (y, x) = point(rest)?;
        return Some(meaning::work_area_upper_left(y, x));
    }
    if let Some(rest) = s.strip_prefix('Z') {
        let (y, x) = point(rest)?;
        return Some(meaning::work_area_lower_right(y, x));
    }
    if let Some(rest) = s.strip_prefix('M') {
        let (y, x) = point(rest)?;
        return Some(meaning::move_to(y, x));
    }
    if let Some(rest) = s.strip_prefix('D') {
        let a = args(rest)?;
        if a.is_empty() || a.len() % 2 != 0 {
            return None;
        }
        return Some(meaning::draw(a.len() / 2));
    }
    if let Some(rest) = s.strip_prefix('L') {
        return Some(format!("line type {}", one(rest)?));
    }
    if let Some(rest) = s.strip_prefix('J') {
        return Some(format!(
            "tool select {} (Cameo 3 and later only)",
            one(rest)?
        ));
    }
    if s.starts_with("TF") {
        return Some("auto blade depth (Cameo 3 and later only)".into());
    }
    if s.starts_with("TG") {
        return Some("mat type (Cameo 3 and later only)".into());
    }
    if s.starts_with("BE") {
        return Some("binary draw (Cameo 4 and Portrait 3 only)".into());
    }
    if s.starts_with("BZ") || s.starts_with('Y') || s.starts_with('W') || s.starts_with(')') {
        return Some("curve command (FlatCraft never sends it)".into());
    }
    None
}

/// Splits `bytes` into commands and explains each one. Unknown commands read "unknown".
pub fn decode(bytes: &[u8]) -> Vec<DecodedCommand> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == ESC {
            // ESC commands are two bytes with no ETX.
            match bytes.get(i + 1) {
                Some(&code) => {
                    out.push(DecodedCommand {
                        raw: render_raw(&bytes[i..i + 2]),
                        meaning: esc_meaning(code),
                    });
                    i += 2;
                }
                None => {
                    out.push(DecodedCommand {
                        raw: render_raw(&bytes[i..]),
                        meaning: "unknown".into(),
                    });
                    i += 1;
                }
            }
            continue;
        }
        let end = bytes[i..].iter().position(|b| *b == ETX).map(|p| i + p);
        let cmd = &bytes[i..end.unwrap_or(bytes.len())];
        let mut meaning = if cmd.is_empty() {
            "empty command".to_string()
        } else {
            text_meaning(cmd).unwrap_or_else(|| meaning::UNKNOWN.into())
        };
        if end.is_none() {
            meaning.push_str(" (no ETX, not complete)");
        }
        out.push(DecodedCommand {
            raw: render_raw(cmd),
            meaning,
        });
        i = end.map_or(bytes.len(), |e| e + 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_cases_do_not_panic() {
        assert!(decode(b"").is_empty());
        assert_eq!(decode(b"\x1b")[0].meaning, "unknown");
        assert_eq!(
            decode(b"!99999999999999999999999\x03")[0].meaning,
            "unknown"
        );
        assert_eq!(decode(b"\xff\xfe\x03")[0].raw, "\\xff\\xfe");
        assert_eq!(decode(b"H")[0].meaning, "home (no ETX, not complete)");
        assert_eq!(decode(b"\x1b\x05")[0].meaning, "status query");
    }

    #[test]
    fn known_commands_are_explained() {
        let d = decode(b"FC0\x03TB50,1\x03Z6096,6096\x03D1,2,3,4\x03TB71\x03FG\x03");
        let m: Vec<&str> = d.iter().map(|c| c.meaning.as_str()).collect();
        assert_eq!(
            m,
            [
                "blade offset 0 (no firmware compensation)",
                "orientation landscape (TB50)",
                "work area lower-right y=6096, x=6096",
                "draw through 2 points",
                "calibration read",
                "firmware version query",
            ]
        );
    }
}
