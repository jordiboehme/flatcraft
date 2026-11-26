//! Builds a complete job (gpgl.md section 9).
//!
//! Every check runs before any byte is built, so an error never leaves partial output.

use kurbo::Point;

use super::command::{FC_RANGE, FORCE_RANGE, MAX_DRAW_POINTS, SPEED_RANGE};
use super::decode::render_raw;
use super::quantize::to_device_f64;
use super::{DevPoint, EmitError, GpglCommand, Tb50Mode};
use crate::profile::{GpglDialect, KnifeCompensation, Media, PlotterProfile, Protocol};

/// One path to cut, in app millimetres (origin = the 0,0 of the grid printed on the mat).
#[derive(Debug, Clone, PartialEq)]
pub struct EmitPath {
    pub points: Vec<Point>,
    /// Shown in the listing, for example "object 3 'star', path 2, pass 1".
    pub note: String,
}

/// What the points of the paths are, so the emitter can refuse to compensate
/// the blade offset twice (gpgl.md section 9).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GeometryKind {
    /// The blade tip's own path. Needs a firmware or "none" profile: the
    /// firmware (or nobody) handles the offset.
    Tip,
    /// The knife holder's axis path, already compensated by FlatCraft for a
    /// blade that trails by `offset_mm`. Needs a host profile with that offset.
    HostAxis { offset_mm: f64 },
    /// A host calibration pattern (an offset test series): compensated with
    /// several offsets on purpose. Needs a host profile.
    HostCalibration,
}

impl GeometryKind {
    /// The words for an error message.
    pub fn describe(self) -> &'static str {
        match self {
            GeometryKind::Tip => "plain tip paths",
            GeometryKind::HostAxis { .. } => "host-compensated paths",
            GeometryKind::HostCalibration => "a host calibration pattern",
        }
    }
}

/// Checks that `geometry` fits the knife compensation of the profile, and the
/// host values. Returns the FC value to send.
fn check_compensation(k: KnifeCompensation, geometry: GeometryKind) -> Result<u16, EmitError> {
    k.validate()
        .map_err(|reason| EmitError::InvalidProfile { reason })?;
    let profile = match k {
        KnifeCompensation::Firmware { .. } => "firmware",
        KnifeCompensation::Host { .. } => "host",
        KnifeCompensation::None => "no",
    };
    let fits = match (k, geometry) {
        (KnifeCompensation::Firmware { .. } | KnifeCompensation::None, GeometryKind::Tip) => true,
        (KnifeCompensation::Host { offset_mm, .. }, GeometryKind::HostAxis { offset_mm: g }) => {
            // NaN never matches.
            (offset_mm - g).abs() <= OFFSET_MATCH_MM
        }
        (KnifeCompensation::Host { .. }, GeometryKind::HostCalibration) => true,
        _ => false,
    };
    if !fits {
        return Err(EmitError::CompensationMismatch {
            profile,
            geometry: geometry.describe(),
        });
    }
    Ok(k.fc_value())
}

/// How close the offset of host axis paths must be to the profile's offset.
const OFFSET_MATCH_MM: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CutSettings {
    pub speed: u8,
    pub force: u8,
    pub track_enhancing: bool,
}

/// One command of the job in readable form: ESC shown as `<ESC>`, no ETX.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingLine {
    pub command: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmittedJob {
    pub bytes: Vec<u8>,
    pub listing: Vec<ListingLine>,
    /// The indexes into the given paths of the paths that were left out,
    /// in order: paths with fewer than 2 points, and paths whose points all
    /// round to one device point (gpgl.md section 9).
    pub collapsed: Vec<usize>,
}

/// Checks `value` against both the profile range and the protocol range. The profile can only
/// narrow the protocol limits, never widen them.
fn check_setting(
    field: &'static str,
    value: u8,
    profile: [u8; 2],
    protocol: [u8; 2],
) -> Result<(), EmitError> {
    let min = profile[0].max(protocol[0]);
    let max = profile[1].min(protocol[1]);
    if value < min || value > max {
        return Err(EmitError::OutOfRange {
            field,
            value: value.into(),
            min: min.into(),
            max: max.into(),
        });
    }
    Ok(())
}

fn media_units(mm: f64, units_per_mm: f64, what: &str) -> Result<i32, EmitError> {
    let u = (mm * units_per_mm).round();
    if !u.is_finite() || u < 1.0 || u > f64::from(i32::MAX) {
        return Err(EmitError::InvalidProfile {
            reason: format!("media {what} {mm} mm gives {u} device units"),
        });
    }
    Ok(u as i32)
}

/// One axis of one point: the device value, its limit, and the mm values for the message.
struct Axis {
    name: &'static str,
    value: f64,
    max: i32,
    mm: f64,
    origin_mm: f64,
    max_mm: f64,
}

fn check_coordinate(a: Axis, note: &str) -> Result<i32, EmitError> {
    if !a.value.is_finite() || a.value < 0.0 || a.value > f64::from(a.max) {
        return Err(EmitError::CoordinateOutOfRange {
            note: note.to_string(),
            axis: a.name,
            mm: a.mm,
            origin_mm: a.origin_mm,
            max_mm: a.max_mm,
            value: a.value,
            max: a.max.into(),
        });
    }
    Ok(a.value as i32)
}

fn quantize_path(
    path: &EmitPath,
    plotter: &PlotterProfile,
    media: &Media,
    height: i32,
    width: i32,
) -> Result<Vec<DevPoint>, EmitError> {
    path.points
        .iter()
        .map(|p| {
            let (y, x) = to_device_f64(*p, plotter, media);
            let y = Axis {
                name: "y",
                value: y,
                max: height,
                mm: p.y,
                origin_mm: media.grid.origin_y_mm,
                max_mm: media.height_mm,
            };
            let x = Axis {
                name: "x",
                value: x,
                max: width,
                mm: p.x,
                origin_mm: media.grid.origin_x_mm,
                max_mm: media.width_mm,
            };
            Ok(DevPoint {
                y: check_coordinate(y, &path.note)?,
                x: check_coordinate(x, &path.note)?,
            })
        })
        .collect()
}

/// Emits the whole job for `paths` on the media `media_id` of `plotter`.
///
/// `geometry` says what the points are; it must fit the plotter's knife
/// compensation ([`GeometryKind`]). Within a path, a point that rounds to the
/// same device point as the one before it is left out.
pub fn emit_job(
    paths: &[EmitPath],
    geometry: GeometryKind,
    plotter: &PlotterProfile,
    media_id: &str,
    settings: &CutSettings,
) -> Result<EmittedJob, EmitError> {
    let Protocol::Gpgl { dialect } = plotter.protocol;
    if dialect != GpglDialect::Cameo12 {
        return Err(EmitError::DialectNotSupported);
    }
    let fc = check_compensation(plotter.knife_compensation, geometry)?;
    if !(FC_RANGE[0]..=FC_RANGE[1]).contains(&fc) {
        return Err(EmitError::OutOfRange {
            field: "fc",
            value: fc.into(),
            min: FC_RANGE[0].into(),
            max: FC_RANGE[1].into(),
        });
    }
    let media = plotter
        .media
        .iter()
        .find(|m| m.id == media_id)
        .ok_or_else(|| EmitError::UnknownMedia {
            id: media_id.to_string(),
        })?;
    check_setting("speed", settings.speed, plotter.speed_range, SPEED_RANGE)?;
    check_setting("force", settings.force, plotter.force_range, FORCE_RANGE)?;
    let height = media_units(media.height_mm, plotter.units_per_mm, "height")?;
    let width = media_units(media.width_mm, plotter.units_per_mm, "width")?;

    // Paths with fewer than 2 points, also after merging points that round
    // to the same device point, are skipped and reported in `collapsed`,
    // never silently (gpgl.md section 9).
    let mut cut: Vec<(Vec<DevPoint>, &str)> = Vec::new();
    let mut collapsed = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        let mut points = quantize_path(path, plotter, media, height, width)?;
        points.dedup();
        if points.len() >= 2 {
            cut.push((points, &path.note));
        } else {
            collapsed.push(i);
        }
    }
    if cut.is_empty() {
        return Err(EmitError::EmptyJob);
    }

    let mut commands: Vec<(GpglCommand, Option<&str>)> = vec![
        (GpglCommand::Init, None),
        (GpglCommand::Speed(settings.speed), None),
        (GpglCommand::Force(settings.force), None),
        (GpglCommand::BladeOffset(fc), None),
        (GpglCommand::TrackEnhancing(settings.track_enhancing), None),
        (GpglCommand::OrientationPortrait, None),
        (GpglCommand::Tb50(Tb50Mode::Portrait), None),
        (
            GpglCommand::WorkAreaUpperLeft(DevPoint { y: 0, x: 0 }),
            None,
        ),
        (
            GpglCommand::WorkAreaLowerRight(DevPoint {
                y: height,
                x: width,
            }),
            None,
        ),
        (GpglCommand::LineTypeSolid, None),
        (GpglCommand::LiftBetweenPaths, None),
        (GpglCommand::CornerDefault, None),
    ];
    for (points, note) in &cut {
        commands.push((GpglCommand::Move(points[0]), Some(note)));
        for chunk in points[1..].chunks(MAX_DRAW_POINTS) {
            commands.push((GpglCommand::Draw(chunk.to_vec()), Some(note)));
        }
    }
    commands.push((GpglCommand::Home, None));

    let mut bytes = Vec::new();
    let mut listing = Vec::with_capacity(commands.len());
    for (cmd, note) in &commands {
        let b = cmd.to_bytes(dialect)?;
        let text = b.strip_suffix(&[0x03]).unwrap_or(&b);
        listing.push(ListingLine {
            command: render_raw(text),
            note: note.map_or_else(|| cmd.describe(), str::to_string),
        });
        bytes.extend_from_slice(&b);
    }
    Ok(EmittedJob {
        bytes,
        listing,
        collapsed,
    })
}
