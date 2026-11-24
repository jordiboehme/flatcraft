//! The closed set of GPGL commands FlatCraft can build (gpgl.md sections 3 and 9).
//!
//! There is deliberately no raw or string variant and no variant for the calibration writes
//! `FB` and `TB72` (gpgl.md section 6), so no code path can build them.

use crate::profile::GpglDialect;

/// Cutting speed limits for the Cameo 2 (gpgl.md section 3).
pub const SPEED_RANGE: [u8; 2] = [1, 10];
/// Blade force limits for the Cameo 2 (gpgl.md section 3).
pub const FORCE_RANGE: [u8; 2] = [1, 33];
/// FlatCraft's FC limit until captures say more (gpgl.md section 3, notes on FC).
pub const FC_RANGE: [u16; 2] = [0, 100];
/// At most this many points in one `D` command (gpgl.md section 9).
pub const MAX_DRAW_POINTS: usize = 16;

const ESC: u8 = 0x1b;
const ETX: u8 = 0x03;

/// A point in device units, sent as `y,x` (gpgl.md section 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DevPoint {
    pub y: i32,
    pub x: i32,
}

/// The two `TB50` orientation forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tb50Mode {
    /// `TB50,0`
    Portrait,
    /// `TB50,1`
    Landscape,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpglCommand {
    /// `ESC 0x04`
    Init,
    /// `ESC 0x05`
    StatusQuery,
    /// `FG`
    FirmwareQuery,
    /// `!<s>`
    Speed(u8),
    /// `FX<f>`
    Force(u8),
    /// `FC<p>`
    BladeOffset(u16),
    /// `FY0` (on) / `FY1` (off)
    TrackEnhancing(bool),
    /// `FN0`
    OrientationPortrait,
    /// `TB50,0` / `TB50,1`
    Tb50(Tb50Mode),
    /// `\<y>,<x>`
    WorkAreaUpperLeft(DevPoint),
    /// `Z<y>,<x>`
    WorkAreaLowerRight(DevPoint),
    /// `L0`
    LineTypeSolid,
    /// `FE0,0`
    LiftBetweenPaths,
    /// `FF0,0,0`
    CornerDefault,
    /// `M<y>,<x>`
    Move(DevPoint),
    /// `D<y>,<x>[,<y>,<x>...]`
    Draw(Vec<DevPoint>),
    /// `H`
    Home,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EmitError {
    #[error("{field} {value} is out of range, allowed is {min} to {max}")]
    OutOfRange {
        field: &'static str,
        value: i64,
        min: i64,
        max: i64,
    },
    #[error(
        "{axis} coordinate {value} (device units) in {note} is outside the media, allowed is 0 to {max}"
    )]
    CoordinateOutOfRange {
        note: String,
        axis: &'static str,
        value: f64,
        max: i64,
    },
    #[error("host knife compensation is not supported yet, use firmware compensation (FC) or none")]
    HostCompensationNotYet,
    #[error(
        "this GPGL dialect is not supported yet, FlatCraft only speaks the Cameo 1 and 2 dialect"
    )]
    DialectNotSupported,
    #[error("media '{id}' is not part of this plotter profile")]
    UnknownMedia { id: String },
    #[error("the job has no path with at least 2 points, there is nothing to cut")]
    EmptyJob,
    #[error("a draw command needs at least one point")]
    EmptyDraw,
    #[error("the plotter profile is not usable: {reason}")]
    InvalidProfile { reason: String },
}

fn check_range(field: &'static str, value: i64, min: i64, max: i64) -> Result<(), EmitError> {
    if value < min || value > max {
        return Err(EmitError::OutOfRange {
            field,
            value,
            min,
            max,
        });
    }
    Ok(())
}

fn check_point(p: &DevPoint) -> Result<(), EmitError> {
    // Negative numbers are never emitted (gpgl.md section 2).
    check_range("y", p.y.into(), 0, i32::MAX.into())?;
    check_range("x", p.x.into(), 0, i32::MAX.into())
}

fn push_point(out: &mut Vec<u8>, p: &DevPoint) {
    out.extend_from_slice(format!("{},{}", p.y, p.x).as_bytes());
}

impl GpglCommand {
    /// The exact bytes of this command, including the closing ETX for text commands.
    ///
    /// Every value is checked against the gpgl.md limits here too, so a hand-built command
    /// can never produce out-of-range bytes.
    pub fn to_bytes(&self, dialect: GpglDialect) -> Result<Vec<u8>, EmitError> {
        if dialect != GpglDialect::Cameo12 {
            return Err(EmitError::DialectNotSupported);
        }
        let text: Vec<u8> = match self {
            GpglCommand::Init => return Ok(vec![ESC, 0x04]),
            GpglCommand::StatusQuery => return Ok(vec![ESC, 0x05]),
            GpglCommand::FirmwareQuery => b"FG".to_vec(),
            GpglCommand::Speed(s) => {
                check_range(
                    "speed",
                    (*s).into(),
                    SPEED_RANGE[0].into(),
                    SPEED_RANGE[1].into(),
                )?;
                format!("!{s}").into_bytes()
            }
            GpglCommand::Force(f) => {
                check_range(
                    "force",
                    (*f).into(),
                    FORCE_RANGE[0].into(),
                    FORCE_RANGE[1].into(),
                )?;
                format!("FX{f}").into_bytes()
            }
            GpglCommand::BladeOffset(fc) => {
                check_range("fc", (*fc).into(), FC_RANGE[0].into(), FC_RANGE[1].into())?;
                format!("FC{fc}").into_bytes()
            }
            GpglCommand::TrackEnhancing(on) => if *on { b"FY0" } else { b"FY1" }.to_vec(),
            GpglCommand::OrientationPortrait => b"FN0".to_vec(),
            GpglCommand::Tb50(mode) => match mode {
                Tb50Mode::Portrait => b"TB50,0".to_vec(),
                Tb50Mode::Landscape => b"TB50,1".to_vec(),
            },
            GpglCommand::WorkAreaUpperLeft(p) => {
                check_point(p)?;
                let mut v = b"\\".to_vec();
                push_point(&mut v, p);
                v
            }
            GpglCommand::WorkAreaLowerRight(p) => {
                check_point(p)?;
                let mut v = b"Z".to_vec();
                push_point(&mut v, p);
                v
            }
            GpglCommand::LineTypeSolid => b"L0".to_vec(),
            GpglCommand::LiftBetweenPaths => b"FE0,0".to_vec(),
            GpglCommand::CornerDefault => b"FF0,0,0".to_vec(),
            GpglCommand::Move(p) => {
                check_point(p)?;
                let mut v = b"M".to_vec();
                push_point(&mut v, p);
                v
            }
            GpglCommand::Draw(points) => {
                if points.is_empty() {
                    return Err(EmitError::EmptyDraw);
                }
                let mut v = b"D".to_vec();
                for (i, p) in points.iter().enumerate() {
                    check_point(p)?;
                    if i > 0 {
                        v.push(b',');
                    }
                    push_point(&mut v, p);
                }
                v
            }
            GpglCommand::Home => b"H".to_vec(),
        };
        let mut out = text;
        out.push(ETX);
        Ok(out)
    }

    /// A short meaning for the job listing.
    pub fn describe(&self) -> String {
        match self {
            GpglCommand::Init => "init".into(),
            GpglCommand::StatusQuery => "status query".into(),
            GpglCommand::FirmwareQuery => "firmware version query".into(),
            GpglCommand::Speed(s) => format!("speed {s}"),
            GpglCommand::Force(f) => format!("force {f}"),
            GpglCommand::BladeOffset(0) => "blade offset 0 (no firmware compensation)".into(),
            GpglCommand::BladeOffset(fc) => format!("blade offset {fc} (firmware compensation)"),
            GpglCommand::TrackEnhancing(true) => "track enhancing on".into(),
            GpglCommand::TrackEnhancing(false) => "track enhancing off".into(),
            GpglCommand::OrientationPortrait => "orientation portrait".into(),
            GpglCommand::Tb50(Tb50Mode::Portrait) => "orientation portrait (TB50)".into(),
            GpglCommand::Tb50(Tb50Mode::Landscape) => "orientation landscape (TB50)".into(),
            GpglCommand::WorkAreaUpperLeft(p) => {
                format!("work area upper-left y={}, x={}", p.y, p.x)
            }
            GpglCommand::WorkAreaLowerRight(p) => {
                format!("work area lower-right y={}, x={}", p.y, p.x)
            }
            GpglCommand::LineTypeSolid => "line type solid".into(),
            GpglCommand::LiftBetweenPaths => "lift setting 0,0".into(),
            GpglCommand::CornerDefault => "corner sharpening 0,0,0".into(),
            GpglCommand::Move(p) => format!("move to y={}, x={}", p.y, p.x),
            GpglCommand::Draw(points) => format!("draw through {} points", points.len()),
            GpglCommand::Home => "home".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(c: GpglCommand) -> Result<Vec<u8>, EmitError> {
        c.to_bytes(GpglDialect::Cameo12)
    }

    #[test]
    fn hand_built_commands_are_range_checked() {
        assert!(bytes(GpglCommand::Speed(0)).is_err());
        assert!(bytes(GpglCommand::Speed(11)).is_err());
        assert!(bytes(GpglCommand::Force(0)).is_err());
        assert!(bytes(GpglCommand::Force(34)).is_err());
        assert!(bytes(GpglCommand::BladeOffset(101)).is_err());
        assert_eq!(bytes(GpglCommand::BladeOffset(100)).unwrap(), b"FC100\x03");
        assert!(bytes(GpglCommand::Move(DevPoint { y: -1, x: 0 })).is_err());
        assert!(bytes(GpglCommand::Draw(vec![DevPoint { y: 0, x: -5 }])).is_err());
        assert_eq!(bytes(GpglCommand::Draw(vec![])), Err(EmitError::EmptyDraw));
    }

    #[test]
    fn fixed_commands_have_exact_bytes() {
        assert_eq!(bytes(GpglCommand::Init).unwrap(), b"\x1b\x04");
        assert_eq!(bytes(GpglCommand::StatusQuery).unwrap(), b"\x1b\x05");
        assert_eq!(bytes(GpglCommand::FirmwareQuery).unwrap(), b"FG\x03");
        assert_eq!(
            bytes(GpglCommand::Tb50(Tb50Mode::Landscape)).unwrap(),
            b"TB50,1\x03"
        );
        assert_eq!(
            bytes(GpglCommand::WorkAreaUpperLeft(DevPoint { y: 0, x: 0 })).unwrap(),
            b"\\0,0\x03"
        );
    }
}
