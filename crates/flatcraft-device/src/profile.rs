//! Plotter and material profile types.

use std::collections::BTreeMap;
use std::path::PathBuf;

use flatcraft_core::geometry::RingDirection;
use kurbo::{Point, Rect};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpglDialect {
    Cameo12,
    Cameo3Plus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    Gpgl { dialect: GpglDialect },
}

/// How jobs reach the plotter. A `File` transport without a directory writes
/// to `<Documents>/FlatCraft Output`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportSpec {
    Usb {
        vid: u16,
        pid: u16,
    },
    File {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        directory: Option<PathBuf>,
        extension: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum KnifeCompensation {
    Firmware {
        fc: u16,
    },
    Host {
        offset_mm: f64,
        pivot_threshold_deg: f64,
    },
    None,
}

impl KnifeCompensation {
    /// The FC value for the emitter. Only firmware compensation sends one.
    pub fn fc_value(&self) -> u16 {
        match self {
            KnifeCompensation::Firmware { fc } => *fc,
            KnifeCompensation::Host { .. } | KnifeCompensation::None => 0,
        }
    }

    /// Checks the values of host compensation: the offset is a number
    /// above 0 and at most [`MAX_HOST_OFFSET_MM`], the pivot threshold 0
    /// to [`MAX_PIVOT_THRESHOLD_DEG`]. Firmware and none always pass. The
    /// text says what to fix.
    ///
    /// [`MAX_HOST_OFFSET_MM`]: flatcraft_core::limits::MAX_HOST_OFFSET_MM
    /// [`MAX_PIVOT_THRESHOLD_DEG`]: flatcraft_core::limits::MAX_PIVOT_THRESHOLD_DEG
    pub fn validate(&self) -> Result<(), String> {
        use flatcraft_core::limits::{MAX_HOST_OFFSET_MM, MAX_PIVOT_THRESHOLD_DEG};
        if let KnifeCompensation::Host {
            offset_mm,
            pivot_threshold_deg,
        } = *self
        {
            if !(offset_mm.is_finite() && offset_mm > 0.0 && offset_mm <= MAX_HOST_OFFSET_MM) {
                return Err(format!(
                    "the host blade offset must be above 0 and at most {MAX_HOST_OFFSET_MM} mm (got {offset_mm})"
                ));
            }
            if !(0.0..=MAX_PIVOT_THRESHOLD_DEG).contains(&pivot_threshold_deg) {
                return Err(format!(
                    "the pivot threshold must be 0 to {MAX_PIVOT_THRESHOLD_DEG} deg (got {pivot_threshold_deg})"
                ));
            }
        }
        Ok(())
    }
}

/// Largest physical or grid size of a mat, in mm.
pub const MAX_SIZE_MM: f64 = 10_000.0;

/// Distance kept free inside the grid of a mat that is not calibrated yet.
pub const UNCALIBRATED_MARGIN_MM: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MediaKind {
    /// A sticky cutting mat with a printed grid.
    #[default]
    Mat,
    /// Material fed without a mat.
    NoMat,
}

/// The grid printed on a mat. Its origin is measured in mm from the mat's top-left corner in
/// the unmirrored physical frame (before any axis mirror).
/// FlatCraft's 0,0 is the grid's 0,0, and the grid is the area you can cut in.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    pub origin_x_mm: f64,
    pub origin_y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
}

/// The mat field a [`MediaError`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaField {
    Width,
    Height,
    OriginX,
    OriginY,
    GridWidth,
    GridHeight,
}

/// Why a mat is invalid, and which field to fix. `reason` reads on its own
/// ("the grid size must be above 0"); the full text names the mat.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("mat '{id}': {reason}")]
pub struct MediaError {
    pub id: String,
    pub field: MediaField,
    pub reason: String,
}

impl Media {
    /// Checks the sizes: all finite, physical size above 0, grid origin from 0, grid size above
    /// 0, and the grid inside the physical mat. The error names the first field to fix.
    pub fn validate(&self) -> Result<(), MediaError> {
        use MediaField as F;
        let g = &self.grid;
        let fields = [
            (F::Width, self.width_mm),
            (F::Height, self.height_mm),
            (F::OriginX, g.origin_x_mm),
            (F::OriginY, g.origin_y_mm),
            (F::GridWidth, g.width_mm),
            (F::GridHeight, g.height_mm),
        ];
        let err = |field, reason: String| {
            Err(MediaError {
                id: self.id.clone(),
                field,
                reason,
            })
        };
        let sizes = [F::Width, F::Height, F::GridWidth, F::GridHeight];
        if let Some((f, _)) = fields
            .iter()
            .find(|(f, v)| sizes.contains(f) && *v > MAX_SIZE_MM)
        {
            return err(
                *f,
                format!("sizes above {MAX_SIZE_MM} mm are not supported"),
            );
        }
        if let Some((f, _)) = fields.iter().find(|(_, v)| !v.is_finite()) {
            return err(*f, "every size and origin must be a finite number".into());
        }
        if self.width_mm <= 0.0 {
            return err(F::Width, "the physical size must be above 0".into());
        }
        if self.height_mm <= 0.0 {
            return err(F::Height, "the physical size must be above 0".into());
        }
        if g.origin_x_mm < 0.0 {
            return err(F::OriginX, "the grid origin must not be negative".into());
        }
        if g.origin_y_mm < 0.0 {
            return err(F::OriginY, "the grid origin must not be negative".into());
        }
        if g.width_mm <= 0.0 {
            return err(F::GridWidth, "the grid size must be above 0".into());
        }
        if g.height_mm <= 0.0 {
            return err(F::GridHeight, "the grid size must be above 0".into());
        }
        let outside = "the grid (origin plus size) must lie inside the physical mat";
        if g.origin_x_mm + g.width_mm > self.width_mm {
            return err(F::GridWidth, outside.into());
        }
        if g.origin_y_mm + g.height_mm > self.height_mm {
            return err(F::GridHeight, outside.into());
        }
        Ok(())
    }
}

/// A mat (or "no mat") of a plotter. `width_mm` and `height_mm` are the physical size.
///
/// In a user preset an entry with the same id as an inherited mat replaces that mat as a
/// whole, so it must be a complete mat (all fields, not just the changed ones).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "MediaFile")]
pub struct Media {
    pub id: String,
    pub name: String,
    pub kind: MediaKind,
    pub width_mm: f64,
    pub height_mm: f64,
    pub grid: Grid,
    /// True once the grid origin was measured on the real mat.
    pub calibrated: bool,
    /// The colour of the mat on the canvas, `#RRGGBB` in files.
    #[serde(serialize_with = "hex_color::serialize")]
    pub color: [u8; 3],
}

/// The paper colour: a warm cream, the light value of the icon style's paper
/// token. A mat is a physical object, so it looks the same in both app themes.
pub const PAPER_COLOR: [u8; 3] = [0xFB, 0xF4, 0xE4];
/// Default mat colour on the canvas: paper.
pub const DEFAULT_MAT_COLOR: [u8; 3] = PAPER_COLOR;
/// Default colour of "No mat" on the canvas: paper.
pub const DEFAULT_NO_MAT_COLOR: [u8; 3] = PAPER_COLOR;
/// The factory colours before paper (a blue-grey mat, a cream no-mat). A mat
/// that still holds one of them takes the new default when it is read.
const OLD_MAT_COLOR: [u8; 3] = [0x8F, 0xA8, 0xB8];
const OLD_NO_MAT_COLOR: [u8; 3] = [0xF4, 0xF1, 0xEA];

/// Colours as `#RRGGBB` strings.
mod hex_color {
    use serde::Serializer;

    pub fn parse(s: &str) -> Result<[u8; 3], String> {
        let bad = || format!("'{s}' is not a colour, use #RRGGBB");
        let hex = s.strip_prefix('#').ok_or_else(bad)?;
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let v = u32::from_str_radix(hex, 16).map_err(|_| bad())?;
        Ok([(v >> 16) as u8, (v >> 8) as u8, v as u8])
    }

    pub fn serialize<S: Serializer>(c: &[u8; 3], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]))
    }
}

/// The file form of [`Media`]: `kind`, `grid` and `calibrated` are optional, so older
/// files load. A missing grid is the whole physical size from the mat's top-left corner
/// (unmirrored physical frame, before any axis mirror).
#[derive(Deserialize)]
struct MediaFile {
    id: String,
    name: String,
    #[serde(default)]
    kind: MediaKind,
    width_mm: f64,
    height_mm: f64,
    #[serde(default)]
    grid: Option<Grid>,
    #[serde(default)]
    calibrated: bool,
    #[serde(default)]
    color: Option<String>,
}

impl TryFrom<MediaFile> for Media {
    type Error = String;

    fn try_from(f: MediaFile) -> Result<Self, String> {
        let color = match &f.color {
            Some(c) => match (hex_color::parse(c)?, f.kind) {
                (OLD_MAT_COLOR, MediaKind::Mat) => DEFAULT_MAT_COLOR,
                (OLD_NO_MAT_COLOR, MediaKind::NoMat) => DEFAULT_NO_MAT_COLOR,
                (own, _) => own,
            },
            None if f.kind == MediaKind::NoMat => DEFAULT_NO_MAT_COLOR,
            None => DEFAULT_MAT_COLOR,
        };
        let grid = f.grid.unwrap_or(Grid {
            origin_x_mm: 0.0,
            origin_y_mm: 0.0,
            width_mm: f.width_mm,
            height_mm: f.height_mm,
        });
        Ok(Media {
            id: f.id,
            name: f.name,
            kind: f.kind,
            width_mm: f.width_mm,
            height_mm: f.height_mm,
            grid,
            calibrated: f.calibrated,
            color,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxisMapping {
    pub mirror_x: bool,
    pub mirror_y: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub track_enhancing: bool,
    pub registration_marks: bool,
    pub auto_blade: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlotterProfile {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits: Option<String>,
    pub protocol: Protocol,
    pub transport: TransportSpec,
    pub media: Vec<Media>,
    pub units_per_mm: f64,
    pub axis: AxisMapping,
    pub scale_x: f64,
    pub scale_y: f64,
    pub knife_compensation: KnifeCompensation,
    /// The direction closed shapes are cut in. Files without the key get
    /// counter-clockwise.
    #[serde(default)]
    pub ring_direction: RingDirection,
    pub blade_offset_mm: f64,
    pub speed_range: [u8; 2],
    pub force_range: [u8; 2],
    /// The pass counts a job may use. Older presets load with 1 to 20.
    #[serde(default = "default_passes_range")]
    pub passes_range: [u32; 2],
    pub capabilities: Capabilities,
    pub flatten_tolerance_mm: f64,
    /// The person's notes on this plotter. Never inherited. A system
    /// preset gets them from the user-side annotation file
    /// ([`crate::store::PRESET_NOTES_FILE`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Pass counts of a plotter preset that does not set `passes_range`.
pub const DEFAULT_PASSES_RANGE: [u32; 2] = [1, 20];

fn default_passes_range() -> [u32; 2] {
    DEFAULT_PASSES_RANGE
}

impl PlotterProfile {
    /// Checks the values against sane limits ([`flatcraft_core::limits`]):
    /// device units per mm (above 0), the x and y scale ([`SCALE_RANGE`]),
    /// the flatten tolerance, and the blade offset and pivot threshold of
    /// host knife compensation. The text says what to fix.
    ///
    /// [`SCALE_RANGE`]: flatcraft_core::limits::SCALE_RANGE
    pub fn validate_values(&self) -> Result<(), String> {
        use flatcraft_core::limits::*;
        if !(self.units_per_mm.is_finite() && self.units_per_mm > 0.0) {
            return Err(format!(
                "the device units per mm must be a number above 0 (got {})",
                self.units_per_mm
            ));
        }
        for (axis, scale) in [("x", self.scale_x), ("y", self.scale_y)] {
            if !SCALE_RANGE.contains(&scale) {
                return Err(format!(
                    "the {axis} scale must be {} to {} (got {scale})",
                    SCALE_RANGE.start(),
                    SCALE_RANGE.end()
                ));
            }
        }
        if !(MIN_TOLERANCE_MM..=MAX_TOLERANCE_MM).contains(&self.flatten_tolerance_mm) {
            return Err(format!(
                "the flatten tolerance must be {MIN_TOLERANCE_MM} to {MAX_TOLERANCE_MM} mm (got {})",
                self.flatten_tolerance_mm
            ));
        }
        self.knife_compensation.validate()
    }

    /// The mat `media_id`, or the plotter's first mat when it no longer
    /// lists that one (hidden or deleted in a user preset, or a project
    /// from elsewhere). The flag is true for that fallback; `None` only for
    /// a plotter without any mat.
    pub fn media_or_first(&self, media_id: &str) -> Option<(&Media, bool)> {
        match self.media.iter().find(|m| m.id == media_id) {
            Some(m) => Some((m, false)),
            None => self.media.first().map(|m| (m, true)),
        }
    }

    /// The device home (device 0,0, the media corner the plotter homes to)
    /// in app millimetres: minus the grid origin, or the far media edge on a
    /// mirrored axis. Travel to the first cut starts here.
    pub fn home_mm(&self, media: &Media) -> Point {
        let x = if self.axis.mirror_x {
            media.width_mm - media.grid.origin_x_mm
        } else {
            -media.grid.origin_x_mm
        };
        let y = if self.axis.mirror_y {
            media.height_mm - media.grid.origin_y_mm
        } else {
            -media.grid.origin_y_mm
        };
        Point::new(x, y)
    }

    /// The area objects may use, in app coordinates (origin = the grid's 0,0): the whole
    /// grid for a calibrated mat, the grid minus [`UNCALIBRATED_MARGIN_MM`] on every side
    /// otherwise.
    pub fn cutting_area(&self, media_id: &str) -> Option<Rect> {
        let m = self.media.iter().find(|m| m.id == media_id)?;
        let inset = if m.calibrated {
            0.0
        } else {
            UNCALIBRATED_MARGIN_MM
        };
        Some(Rect::new(
            inset,
            inset,
            m.grid.width_mm - inset,
            m.grid.height_mm - inset,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct MaterialOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overcut_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_mode: Option<CornerModeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_radius_mm: Option<f64>,
}

/// What host knife compensation does at a sharp corner of this material.
/// In files: `corner_mode = "Arc"` or
/// `corner_mode = { LiftOvershoot = { overshoot_mm = 0.1 } }`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum CornerModeSpec {
    /// The blade swivels around the corner point while it stays down.
    #[default]
    Arc,
    /// The blade cuts `overshoot_mm` past the corner, lifts, and starts the
    /// next line fresh. Only where the overshoot lands in waste; every other
    /// corner gets the arc.
    LiftOvershoot { overshoot_mm: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialProfile {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits: Option<String>,
    pub speed: u8,
    pub force: u8,
    pub passes: u32,
    pub reverse_alternate_passes: bool,
    pub overcut_mm: f64,
    pub track_enhancing: bool,
    /// What host compensation does at sharp corners. Files without the key
    /// get [`CornerModeSpec::Arc`]. It is always written, so a child of a
    /// lift-and-overshoot preset can set it back to the arc.
    #[serde(default)]
    pub corner_mode: CornerModeSpec,
    /// Curves tighter than this radius get a warning: the blade may round
    /// or tear them. `None` uses the plotter's `blade_offset_mm`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_radius_mm: Option<f64>,
    /// The material sits on a backing sheet, like vinyl: small loose
    /// parts (the middle of a stencil's O) stay on it until transfer tape
    /// lifts them. Paper and cardstock have none, so such parts fall out.
    /// Files without the key get true. It is always written, so a child
    /// of a preset without a backing sheet can set it back to true.
    #[serde(default = "backing_default")]
    pub backing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blade_depth_hint: Option<String>,
    /// Where the material picker lists it. Files without the key get
    /// [`MaterialGroup::Other`].
    #[serde(default)]
    pub group: MaterialGroup,
    /// How sticky a mat this material wants: a recommendation shown to
    /// the person, never sent to the plotter. Files without the key get
    /// [`MatTack::Standard`].
    #[serde(default)]
    pub mat_tack: MatTack,
    /// Tips that ship with a built-in preset (which side up, how to weed).
    /// Unlike [`MaterialProfile::notes`] they are inherited, and the
    /// person does not edit them in the window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tips: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<String, MaterialOverride>,
    /// The person's notes on this material. Never inherited. A system
    /// preset gets them from the user-side annotation file
    /// ([`crate::store::PRESET_NOTES_FILE`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

fn backing_default() -> bool {
    true
}

/// The groups of the material picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialGroup {
    /// Adhesive vinyl on a backing sheet.
    Vinyl,
    /// Heat transfer vinyl: cut from the back, so the design is mirrored.
    IronOn,
    /// Paper, cardstock, kraft, vellum and sticker paper.
    PaperAndCard,
    /// Films, clings, magnet sheets and every material without a group.
    #[default]
    Other,
}

impl MaterialGroup {
    /// The picker order.
    pub const ALL: [MaterialGroup; 4] = [
        MaterialGroup::Vinyl,
        MaterialGroup::IronOn,
        MaterialGroup::PaperAndCard,
        MaterialGroup::Other,
    ];

    /// The heading in the material picker.
    pub fn label(self) -> &'static str {
        match self {
            MaterialGroup::Vinyl => "Vinyl",
            MaterialGroup::IronOn => "Iron-on",
            MaterialGroup::PaperAndCard => "Paper and card",
            MaterialGroup::Other => "Film and other",
        }
    }
}

/// How sticky a cutting mat a material wants. A recommendation only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatTack {
    /// A light grip: thin paper and vellum come off without tearing.
    Light,
    /// The everyday grip for vinyl, iron-on and most card.
    #[default]
    Standard,
    /// A strong grip for heavy card and stiff sheets that would shift.
    Strong,
}

impl MatTack {
    pub const ALL: [MatTack; 3] = [MatTack::Light, MatTack::Standard, MatTack::Strong];

    pub fn label(self) -> &'static str {
        match self {
            MatTack::Light => "Light",
            MatTack::Standard => "Standard",
            MatTack::Strong => "Strong",
        }
    }
}

impl MaterialProfile {
    /// Checks the overcut, the corner overshoot and the minimum radius,
    /// also of the per-plotter overrides, against
    /// [`flatcraft_core::limits`].
    pub fn validate_values(&self) -> Result<(), String> {
        use flatcraft_core::limits::{MAX_CORNER_OVERSHOOT_MM, MAX_MIN_RADIUS_MM, MAX_OVERCUT_MM};
        let o = || self.overrides.values();
        let overcuts = std::iter::once(self.overcut_mm).chain(o().filter_map(|o| o.overcut_mm));
        for v in overcuts {
            if !(0.0..=MAX_OVERCUT_MM).contains(&v) {
                return Err(format!(
                    "the overcut must be 0 to {MAX_OVERCUT_MM} mm (got {v})"
                ));
            }
        }
        let corners = std::iter::once(self.corner_mode).chain(o().filter_map(|o| o.corner_mode));
        for c in corners {
            if let CornerModeSpec::LiftOvershoot { overshoot_mm: v } = c
                && !(v > 0.0 && v <= MAX_CORNER_OVERSHOOT_MM)
            {
                return Err(format!(
                    "the corner overshoot must be above 0 and at most {MAX_CORNER_OVERSHOOT_MM} mm (got {v})"
                ));
            }
        }
        let radii = self
            .min_radius_mm
            .into_iter()
            .chain(o().filter_map(|o| o.min_radius_mm));
        for v in radii {
            if !(v > 0.0 && v <= MAX_MIN_RADIUS_MM) {
                return Err(format!(
                    "the minimum radius must be above 0 and at most {MAX_MIN_RADIUS_MM} mm (got {v})"
                ));
            }
        }
        Ok(())
    }

    /// This material with the override for `plotter_id` applied, if any.
    pub fn for_plotter(&self, plotter_id: &str) -> MaterialProfile {
        let mut out = self.clone();
        if let Some(o) = self.overrides.get(plotter_id) {
            if let Some(v) = o.speed {
                out.speed = v;
            }
            if let Some(v) = o.force {
                out.force = v;
            }
            if let Some(v) = o.passes {
                out.passes = v;
            }
            if let Some(v) = o.overcut_mm {
                out.overcut_mm = v;
            }
            if let Some(v) = o.corner_mode {
                out.corner_mode = v;
            }
            if let Some(v) = o.min_radius_mm {
                out.min_radius_mm = Some(v);
            }
        }
        out
    }
}
