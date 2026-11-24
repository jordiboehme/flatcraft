//! The last line of defence before any transport (gpgl.md sections 6 and 7).
//!
//! It works on raw bytes only and does not trust the emitter. It rejects the calibration
//! writes `FB` and `TB72`, and a job whose last command is not `H`.

use super::decode::render_raw;

const ESC: u8 = 0x1b;
const ETX: u8 = 0x03;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SafetyError {
    #[error("forbidden command '{command}': it writes the device calibration permanently")]
    Forbidden { command: String },
    #[error("the job does not end with the home command H")]
    NotTerminated,
}

/// Removes every `ESC 0x04` and `ESC 0x05` pair. Removing them everywhere (not only at a
/// command start) is the cautious side: it can only bring a hidden `FB` or `TB72` to light.
fn strip_esc_pairs(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == ESC && matches!(bytes.get(i + 1), Some(0x04 | 0x05)) {
            i += 2;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

fn is_forbidden(segment: &[u8]) -> bool {
    // Look at every line of the segment too and ignore leading spaces and control bytes, so
    // `FB` cannot hide behind a line break or a space. Compare without case, to be safe.
    segment.split(|b| *b == b'\n' || *b == b'\r').any(|part| {
        let start = part.iter().position(|b| *b > b' ').unwrap_or(part.len());
        let cmd = &part[start..];
        cmd.get(..2).is_some_and(|p| p.eq_ignore_ascii_case(b"FB"))
            || cmd
                .get(..4)
                .is_some_and(|p| p.eq_ignore_ascii_case(b"TB72"))
    })
}

/// Splits into ETX-terminated commands plus the unterminated tail (maybe empty).
fn scan(bytes: &[u8]) -> Result<(Vec<Vec<u8>>, Vec<u8>), SafetyError> {
    let clean = strip_esc_pairs(bytes);
    let mut parts: Vec<Vec<u8>> = clean.split(|b| *b == ETX).map(<[u8]>::to_vec).collect();
    // `split` always yields at least one part: the bytes after the last ETX.
    let tail = parts.pop().unwrap_or_default();
    // Second view: ESC commands are two bytes (gpgl.md section 1), so any ESC pair also ends
    // the command before it. A job is rejected when either view shows a forbidden command.
    let esc_view = split_at_esc_pairs(bytes);
    for seg in parts
        .iter()
        .chain(std::iter::once(&tail))
        .chain(esc_view.iter())
    {
        if is_forbidden(seg) {
            return Err(SafetyError::Forbidden {
                command: render_raw(seg),
            });
        }
    }
    Ok((parts, tail))
}

/// Splits at ETX and at every ESC pair (any second byte). The ESC pairs themselves are
/// dropped. A lone ESC at the end is dropped as well.
fn split_at_esc_pairs(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            ESC => {
                out.push(std::mem::take(&mut cur));
                i += 2;
            }
            ETX => {
                out.push(std::mem::take(&mut cur));
                i += 1;
            }
            b => {
                cur.push(b);
                i += 1;
            }
        }
    }
    out.push(cur);
    out
}

/// Checks a complete job: no forbidden command, and the last command is exactly `H` followed
/// by ETX, with nothing after it.
pub fn check_safe(bytes: &[u8]) -> Result<(), SafetyError> {
    let (commands, tail) = scan(bytes)?;
    if !tail.is_empty() {
        return Err(SafetyError::NotTerminated);
    }
    match commands.last() {
        Some(last) if last.as_slice() == b"H" => Ok(()),
        _ => Err(SafetyError::NotTerminated),
    }
}

/// Like [`check_safe`] but without the `H` rule, for short sequences such as a status query.
pub fn check_safe_fragment(bytes: &[u8]) -> Result<(), SafetyError> {
    scan(bytes).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_commands_cannot_hide() {
        for bad in [
            &b"fb1\x03H\x03"[..],
            b" FB1\x03H\x03",
            b"L0\nFB1\x03H\x03",
            b"F\x1b\x04B1\x03H\x03",
            b"TB72",
            b"\x1b\x15FB1\x03H\x03",
            b"M1,1\x1b\x04FB1\x03H\x03",
            b"M1\x1b\x03FB1\x03H\x03",
            b"\x01FB1\x03H\x03",
            b"\tTB72,1\x03H\x03",
        ] {
            assert!(
                matches!(check_safe_fragment(bad), Err(SafetyError::Forbidden { .. })),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn harmless_commands_pass() {
        assert_eq!(check_safe(b"TB71\x03FA\x03TB50,1\x03FX5\x03H\x03"), Ok(()));
    }
}
