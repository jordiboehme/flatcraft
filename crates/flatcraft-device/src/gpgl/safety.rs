//! The last line of defence before any transport (gpgl.md sections 1, 3, 6 and 7).
//!
//! This is an allowlist, not a blocklist. It works on raw bytes only and does not trust the
//! emitter. A byte string is accepted only when it parses completely, from the first to the
//! last byte, as a sequence of these tokens:
//!
//! - `ESC 0x04` (init) or `ESC 0x05` (status query). Any other byte after ESC, or a lone ESC,
//!   is rejected.
//! - One of the commands FlatCraft emits, closed by ETX (`0x03`), with nothing else in it:
//!   `!<n>` (speed 1-10), `FX<n>` (force 1-33), `FC<n>` (blade offset 0-100), `FY0`, `FY1`,
//!   `FN0`, `TB50,0`, `TB50,1`, `\<n>,<n>`, `Z<n>,<n>`, `L0`, `FE0,0`, `FF0,0,0`, `M<n>,<n>`,
//!   `D<n>,<n>` with more `,<n>,<n>` pairs, `H` and `FG`.
//!
//! `<n>` is one or more ASCII digits with no sign, no spaces and no leading zeros ("0" itself
//! is fine), at most `i32::MAX`. Bytes after the last ETX are rejected.
//!
//! A rejected token that starts with `FB`, or with `TB` followed by a number that reads as 72
//! (signs, spaces and leading zeros ignored), is reported as [`SafetyError::Forbidden`].
//! Every other rejection is [`SafetyError::Unrecognised`].
//!
//! [`check_safe`] also requires the raw bytes to end with exactly `H` + ETX. A
//! [`CheckedJob`] is a job that passed it; transports take only that.
//! [`check_safe_fragment`] uses the same allowlist without that rule.

use super::command::{FC_RANGE, FORCE_RANGE, SPEED_RANGE};
use super::decode::{render_raw, tb_code};
use super::emit::EmittedJob;

const ESC: u8 = 0x1b;
const ETX: u8 = 0x03;
/// At most this many bytes of a rejected token are shown in the error.
const SHOW_MAX: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SafetyError {
    #[error("forbidden command '{command}': it writes the device calibration permanently")]
    Forbidden { command: String },
    #[error(
        "unrecognised bytes '{bytes}' at offset {offset}: only commands FlatCraft emits itself may be sent"
    )]
    Unrecognised { offset: usize, bytes: String },
    #[error("the job does not end with the home command H")]
    NotTerminated,
}

/// One allowlisted token, borrowed from the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token<'a> {
    /// `ESC` and its second byte (0x04 or 0x05).
    Esc(u8),
    /// A command without its closing ETX.
    Command(&'a [u8]),
}

/// `<n>`: digits only, no leading zero except "0", at most `i32::MAX`.
fn number(b: &[u8]) -> Option<u64> {
    if b.is_empty() || b.len() > 10 || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    if b.len() > 1 && b[0] == b'0' {
        return None;
    }
    let n = b.iter().fold(0u64, |n, d| n * 10 + u64::from(d - b'0'));
    (n <= i32::MAX as u64).then_some(n)
}

fn number_in(b: &[u8], min: u64, max: u64) -> bool {
    number(b).is_some_and(|n| (min..=max).contains(&n))
}

/// `<n>,<n>` repeated: true when there are `pairs` pairs (`None` = one or more).
fn coordinates(b: &[u8], pairs: Option<usize>) -> bool {
    let mut count = 0usize;
    for part in b.split(|c| *c == b',') {
        if number(part).is_none() {
            return false;
        }
        count += 1;
    }
    count.is_multiple_of(2) && count >= 2 && pairs.is_none_or(|p| count == p * 2)
}

/// Whether `cmd` (without ETX) is exactly one of the allowlisted commands.
fn allowed(cmd: &[u8]) -> bool {
    const FIXED: &[&[u8]] = &[
        b"FY0", b"FY1", b"FN0", b"TB50,0", b"TB50,1", b"L0", b"FE0,0", b"FF0,0,0", b"H", b"FG",
    ];
    if FIXED.contains(&cmd) {
        return true;
    }
    if let Some(r) = cmd.strip_prefix(b"!") {
        return number_in(r, SPEED_RANGE[0].into(), SPEED_RANGE[1].into());
    }
    if let Some(r) = cmd.strip_prefix(b"FX") {
        return number_in(r, FORCE_RANGE[0].into(), FORCE_RANGE[1].into());
    }
    if let Some(r) = cmd.strip_prefix(b"FC") {
        return number_in(r, FC_RANGE[0].into(), FC_RANGE[1].into());
    }
    if let Some(r) = cmd
        .strip_prefix(b"\\")
        .or_else(|| cmd.strip_prefix(b"Z"))
        .or_else(|| cmd.strip_prefix(b"M"))
    {
        return coordinates(r, Some(1));
    }
    if let Some(r) = cmd.strip_prefix(b"D") {
        return coordinates(r, None);
    }
    false
}

fn reject(offset: usize, token: &[u8]) -> SafetyError {
    let forbidden = token.starts_with(b"FB")
        || token
            .strip_prefix(b"TB")
            .is_some_and(|after| tb_code(after) == Some(72));
    let mut shown = render_raw(&token[..token.len().min(SHOW_MAX)]);
    if token.len() > SHOW_MAX {
        shown.push_str("...");
    }
    if forbidden {
        SafetyError::Forbidden { command: shown }
    } else {
        SafetyError::Unrecognised {
            offset,
            bytes: shown,
        }
    }
}

/// Parses all of `bytes` into allowlisted tokens, or names the first token that is not.
fn tokenize(bytes: &[u8]) -> Result<Vec<Token<'_>>, SafetyError> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == ESC {
            match bytes.get(i + 1) {
                Some(&code @ (0x04 | 0x05)) => {
                    out.push(Token::Esc(code));
                    i += 2;
                }
                _ => return Err(reject(i, &bytes[i..bytes.len().min(i + 2)])),
            }
            continue;
        }
        let Some(len) = bytes[i..].iter().position(|b| *b == ETX) else {
            // Bytes after the last ETX: never complete, never sent.
            return Err(reject(i, &bytes[i..]));
        };
        let cmd = &bytes[i..i + len];
        if !allowed(cmd) {
            return Err(reject(i, cmd));
        }
        out.push(Token::Command(cmd));
        i += len + 1;
    }
    Ok(out)
}

/// Checks a complete job: every byte belongs to an allowlisted token, and the raw bytes end
/// with exactly `H` + ETX.
pub fn check_safe(bytes: &[u8]) -> Result<(), SafetyError> {
    let tokens = tokenize(bytes)?;
    if !bytes.ends_with(b"H\x03") || tokens.last() != Some(&Token::Command(b"H")) {
        return Err(SafetyError::NotTerminated);
    }
    Ok(())
}

/// A job that passed [`check_safe`]. [`CheckedJob::check`] is the only way
/// to build one, and [`crate::transport::Transport::send_job`] takes only
/// this type, so no transport can skip the safety check.
///
/// ```compile_fail
/// use flatcraft_device::gpgl::{CheckedJob, EmittedJob};
/// let job = EmittedJob { bytes: b"FB1\x03H\x03".to_vec(), listing: vec![], collapsed: 0 };
/// let _ = CheckedJob(job);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedJob(EmittedJob);

impl CheckedJob {
    /// Runs [`check_safe`] on the job's bytes.
    pub fn check(job: EmittedJob) -> Result<Self, SafetyError> {
        check_safe(&job.bytes)?;
        Ok(Self(job))
    }
}

/// Read access only: the bytes cannot change after the check.
impl std::ops::Deref for CheckedJob {
    type Target = EmittedJob;

    fn deref(&self) -> &EmittedJob {
        &self.0
    }
}

/// Like [`check_safe`] but without the `H` rule, for short sequences such as a status query.
pub fn check_safe_fragment(bytes: &[u8]) -> Result<(), SafetyError> {
    tokenize(bytes).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn encode(tokens: &[Token<'_>]) -> Vec<u8> {
        let mut out = Vec::new();
        for t in tokens {
            match t {
                Token::Esc(c) => out.extend_from_slice(&[ESC, *c]),
                Token::Command(c) => {
                    out.extend_from_slice(c);
                    out.push(ETX);
                }
            }
        }
        out
    }

    #[test]
    fn forbidden_commands_are_named() {
        for bad in [
            &b"FB1\x03H\x03"[..],
            b"TB72,1\x03H\x03",
            b"TB072,1\x03H\x03",
            b"TB 72,1\x03H\x03",
            b"TB+72,1\x03H\x03",
            b"\x1b\x04FB1\x03H\x03",
            b"M1,1\x03FB1",
            b"TB72",
        ] {
            assert!(
                matches!(check_safe(bad), Err(SafetyError::Forbidden { .. })),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn anything_else_is_unrecognised() {
        for bad in [
            &b"fb1\x03H\x03"[..],
            b" FB1\x03H\x03",
            b"L0\nFB1\x03H\x03",
            b"F\x1b\x04B1\x03H\x03",
            b"\x1b\x15FB1\x03H\x03",
            b"M1,1\x1b\x04FB1\x03H\x03",
            b"M1\x1b\x03FB1\x03H\x03",
            b"\x01FB1\x03H\x03",
            b"TB71\x03H\x03",
            b"FA\x03H\x03",
            b"SO0\x03H\x03",
            b"!0\x03H\x03",
            b"!11\x03H\x03",
            b"!05\x03H\x03",
            b"FX34\x03H\x03",
            b"FC101\x03H\x03",
            b"M-1,0\x03H\x03",
            b"M+1,0\x03H\x03",
            b"M 1,0\x03H\x03",
            b"M01,0\x03H\x03",
            b"M1,2,3\x03H\x03",
            b"M2147483648,0\x03H\x03",
            b"D1,2,3\x03H\x03",
            b"D\x03H\x03",
            b"D1,,2\x03H\x03",
            b"\x1b",
            b"H\x1b\x05\x03",
        ] {
            assert!(
                matches!(check_safe(bad), Err(SafetyError::Unrecognised { .. })),
                "{bad:?}: {:?}",
                check_safe(bad)
            );
        }
    }

    #[test]
    fn allowlisted_tokens_pass() {
        assert_eq!(
            check_safe(
                b"\x1b\x04\x1b\x05FG\x03!10\x03FX33\x03FC0\x03FC100\x03FY0\x03FY1\x03FN0\x03TB50,0\x03TB50,1\x03\\0,0\x03Z12192,6096\x03L0\x03FE0,0\x03FF0,0,0\x03M0,2147483647\x03D1,2,3,4\x03H\x03"
            ),
            Ok(())
        );
        assert_eq!(check_safe_fragment(b""), Ok(()));
        assert_eq!(check_safe_fragment(b"\x1b\x05"), Ok(()));
    }

    #[test]
    fn job_must_end_with_home() {
        assert_eq!(check_safe(b""), Err(SafetyError::NotTerminated));
        assert_eq!(check_safe(b"M1,1\x03"), Err(SafetyError::NotTerminated));
        assert_eq!(
            check_safe(b"H\x03\x1b\x05"),
            Err(SafetyError::NotTerminated)
        );
        assert_eq!(
            check_safe(b"H\x03\x1b\x04"),
            Err(SafetyError::NotTerminated)
        );
    }

    fn fragments() -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(
            prop_oneof![
                prop::sample::select(vec![
                    &b"\x1b"[..],
                    b"\x04",
                    b"\x05",
                    b"\x15",
                    b"\x03",
                    b"FB1",
                    b"TB72",
                    b"TB50,0",
                    b"H",
                    b"M1,1",
                    b"D1,2,3,4",
                    b"!5",
                    b"FX10",
                    b"FC18",
                    b"0",
                    b",",
                    b" ",
                    b"\n",
                    b"FY1",
                    b"L0",
                ])
                .prop_map(<[u8]>::to_vec),
                any::<u8>().prop_map(|b| vec![b]),
            ],
            0..24,
        )
        .prop_map(|v| v.concat())
    }

    fn only_allowlisted(tokens: &[Token<'_>]) -> bool {
        tokens.iter().all(|t| match t {
            Token::Esc(c) => *c == 0x04 || *c == 0x05,
            Token::Command(c) => allowed(c) && !c.starts_with(b"FB") && !c.starts_with(b"TB72"),
        })
    }

    proptest! {
        #[test]
        fn accepted_bytes_are_only_allowlisted_tokens(
            bytes in prop::collection::vec(any::<u8>(), 0..64)
        ) {
            if let Ok(tokens) = tokenize(&bytes) {
                prop_assert_eq!(encode(&tokens), bytes.clone());
                prop_assert!(only_allowlisted(&tokens));
            }
        }

        #[test]
        fn accepted_fragments_are_only_allowlisted_tokens(bytes in fragments()) {
            if let Ok(tokens) = tokenize(&bytes) {
                prop_assert_eq!(encode(&tokens), bytes.clone());
                prop_assert!(only_allowlisted(&tokens));
            }
        }
    }
}
