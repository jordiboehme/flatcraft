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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Media {
    pub id: String,
    pub name: String,
    pub width_mm: f64,
    pub height_mm: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Margins {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
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
    pub margins: Margins,
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
    /// The media rectangle minus the margins, origin top-left of the media.
    pub fn cutting_area(&self, media_id: &str) -> Option<Rect> {
        let m = self.media.iter().find(|m| m.id == media_id)?;
        Some(Rect::new(
            self.margins.left,
            self.margins.top,
            m.width_mm - self.margins.right,
            m.height_mm - self.margins.bottom,
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
