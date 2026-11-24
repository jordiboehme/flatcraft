//! Document model: objects on the mat plus job settings.

use std::sync::Arc;

use kurbo::Rect;
use serde::{Deserialize, Serialize};

use crate::geometry::{CutPath, bounds};
use crate::project::FORMAT_VERSION;
use crate::transform::Transform;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ObjectId(pub u64);

/// Raster trace settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TraceSettings {
    /// `None` = automatic (Otsu).
    pub threshold: Option<u8>,
    pub invert: bool,
    pub despeckle_mm2: f64,
    pub corner_threshold_deg: f64,
    /// 0-10.
    pub smoothing: u8,
    /// `None` = 6 * blade_offset_mm.
    pub min_feature_mm: Option<f64>,
    pub dpi_override: Option<f64>,
}

impl Default for TraceSettings {
    fn default() -> Self {
        Self {
            threshold: None,
            invert: false,
            despeckle_mm2: 0.5,
            corner_threshold_deg: 60.0,
            smoothing: 5,
            min_feature_mm: None,
            dpi_override: None,
        }
    }
}

mod base64_bytes {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}

/// Original imported file, kept so the project is self-contained.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Source {
    Svg {
        name: String,
        #[serde(with = "base64_bytes")]
        bytes: Vec<u8>,
    },
    Raster {
        name: String,
        #[serde(with = "base64_bytes")]
        bytes: Vec<u8>,
        trace: TraceSettings,
    },
}

/// What of an object is cut. Milestone 2 adds `MergedOutline` and
/// `VisibleOutline` (they need polygon booleans).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutMode {
    /// Every path as drawn (centre lines), whatever its fill or stroke.
    AllLines,
    /// The outline of every visible shape: fills plus strokes as set by
    /// `StrokeMode`.
    #[default]
    ObjectOutlines,
}

/// How strokes are cut in `CutMode::ObjectOutlines`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokeMode {
    /// Outline when the placed stroke is at least `auto_threshold_mm` wide,
    /// else centre line.
    #[default]
    Auto,
    CentreLine,
    Outline,
}

/// Per-object cut settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CutOptions {
    pub mode: CutMode,
    pub strokes: StrokeMode,
    /// A broken value (null, as JSON writes NaN) loads as NaN, so
    /// `is_valid` catches it instead of the whole project failing.
    #[serde(deserialize_with = "lenient_f64")]
    pub auto_threshold_mm: f64,
}

fn lenient_f64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(d)?.unwrap_or(f64::NAN))
}

impl CutOptions {
    /// Allowed `auto_threshold_mm`, mm.
    pub const AUTO_THRESHOLD_MM: std::ops::RangeInclusive<f64> = 0.05..=10.0;

    /// True if the threshold is a number in `AUTO_THRESHOLD_MM`.
    pub fn is_valid(&self) -> bool {
        Self::AUTO_THRESHOLD_MM.contains(&self.auto_threshold_mm)
    }
}

impl Default for CutOptions {
    fn default() -> Self {
        Self {
            mode: CutMode::ObjectOutlines,
            strokes: StrokeMode::Auto,
            auto_threshold_mm: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Object {
    pub id: ObjectId,
    pub name: String,
    pub source: Source,
    pub transform: Transform,
    pub visible: bool,
    /// Projects from before cut options load with the defaults.
    #[serde(default)]
    pub cut: CutOptions,
    #[serde(skip)]
    pub geometry: Arc<Vec<CutPath>>,
    #[serde(skip)]
    pub local_bounds: Rect,
}

impl Object {
    /// Sets the local geometry and recomputes `local_bounds`
    /// (`Rect::ZERO` when empty).
    pub fn set_geometry(&mut self, geometry: Vec<CutPath>) {
        self.local_bounds = bounds(&geometry).unwrap_or(Rect::ZERO);
        self.geometry = Arc::new(geometry);
    }

    /// The paths in mat millimetres.
    pub fn placed_paths(&self) -> Vec<CutPath> {
        let affine = self.transform.affine(self.local_bounds);
        self.geometry
            .iter()
            .map(|p| CutPath {
                path: affine * &p.path,
                color: p.color,
                label: p.label.clone(),
                paint: p.paint,
            })
            .collect()
    }
}

/// Job settings; `None` options mean the material value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobSettings {
    pub plotter_id: String,
    pub material_id: String,
    pub media_id: String,
    pub speed: Option<u8>,
    pub force: Option<u8>,
    pub passes: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub format_version: u32,
    pub objects: Vec<Object>,
    pub job: JobSettings,
    pub next_id: u64,
}

impl Document {
    pub fn new(job: JobSettings) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            objects: Vec::new(),
            job,
            next_id: 1,
        }
    }

    pub fn add(&mut self, name: String, source: Source, geometry: Vec<CutPath>) -> ObjectId {
        let id = ObjectId(self.next_id);
        self.next_id += 1;
        let mut object = Object {
            id,
            name,
            source,
            transform: Transform::default(),
            visible: true,
            cut: CutOptions::default(),
            geometry: Arc::default(),
            local_bounds: Rect::ZERO,
        };
        object.set_geometry(geometry);
        self.objects.push(object);
        id
    }

    pub fn get(&self, id: ObjectId) -> Option<&Object> {
        self.objects.iter().find(|o| o.id == id)
    }

    pub fn get_mut(&mut self, id: ObjectId) -> Option<&mut Object> {
        self.objects.iter_mut().find(|o| o.id == id)
    }

    pub fn remove(&mut self, id: ObjectId) -> Option<Object> {
        let index = self.objects.iter().position(|o| o.id == id)?;
        Some(self.objects.remove(index))
    }
}
