//! Plotter and material profile types.

use std::collections::BTreeMap;
use std::path::PathBuf;

use kurbo::Rect;
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

impl Media {
    /// Checks the sizes: all finite, physical size above 0, grid origin from 0, grid size above
    /// 0, and the grid inside the physical mat.
    pub fn validate(&self) -> Result<(), String> {
        let g = &self.grid;
        let all = [
            self.width_mm,
            self.height_mm,
            g.origin_x_mm,
            g.origin_y_mm,
            g.width_mm,
            g.height_mm,
        ];
        let id = &self.id;
        if [self.width_mm, self.height_mm, g.width_mm, g.height_mm]
            .iter()
            .any(|v| *v > MAX_SIZE_MM)
        {
            return Err(format!(
                "mat '{id}': sizes above {MAX_SIZE_MM} mm are not supported"
            ));
        }
        if all.iter().any(|v| !v.is_finite()) {
            return Err(format!(
                "mat '{id}': every size and origin must be a finite number"
            ));
        }
        if self.width_mm <= 0.0 || self.height_mm <= 0.0 {
            return Err(format!("mat '{id}': the physical size must be above 0"));
        }
        if g.origin_x_mm < 0.0 || g.origin_y_mm < 0.0 {
            return Err(format!("mat '{id}': the grid origin must not be negative"));
        }
        if g.width_mm <= 0.0 || g.height_mm <= 0.0 {
            return Err(format!("mat '{id}': the grid size must be above 0"));
        }
        if g.origin_x_mm + g.width_mm > self.width_mm
            || g.origin_y_mm + g.height_mm > self.height_mm
        {
            return Err(format!(
                "mat '{id}': the grid (origin plus size) must lie inside the physical mat"
            ));
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

/// Default mat colour on the canvas.
pub const DEFAULT_MAT_COLOR: [u8; 3] = [0x8F, 0xA8, 0xB8];
/// Default colour of "No mat" on the canvas.
pub const DEFAULT_NO_MAT_COLOR: [u8; 3] = [0xF4, 0xF1, 0xEA];

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
            Some(c) => hex_color::parse(c)?,
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
    pub blade_offset_mm: f64,
    pub speed_range: [u8; 2],
    pub force_range: [u8; 2],
    pub capabilities: Capabilities,
    pub flatten_tolerance_mm: f64,
}

impl PlotterProfile {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blade_depth_hint: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<String, MaterialOverride>,
}

impl MaterialProfile {
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
        }
        out
    }
}
