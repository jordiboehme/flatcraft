//! Document model: objects on the mat plus job settings.

use std::path::PathBuf;
use std::sync::Arc;

use kurbo::Rect;
use serde::{Deserialize, Serialize};

use crate::geometry::{CutPath, bounds};
use crate::project::FORMAT_VERSION;
use crate::transform::Transform;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ObjectId(pub u64);

/// A group of linked copies (see [`Object::link_group`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GroupId(pub u64);

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
    use std::sync::Arc;

    pub fn serialize<S: Serializer>(bytes: &Arc<[u8]>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<[u8]>, D::Error> {
        let text = String::deserialize(d)?;
        STANDARD
            .decode(text)
            .map(Arc::from)
            .map_err(serde::de::Error::custom)
    }
}

/// Original imported file, kept so the project is self-contained. The bytes
/// are shared (`Arc`), so undo snapshots and duplicates do not copy them;
/// replace them as a whole, never change them in place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Source {
    Svg {
        name: String,
        #[serde(with = "base64_bytes")]
        bytes: Arc<[u8]>,
    },
    Raster {
        name: String,
        #[serde(with = "base64_bytes")]
        bytes: Arc<[u8]>,
        trace: TraceSettings,
    },
}

/// Where an imported object's file lives on disk, so it can be reloaded
/// after the owner edits it in another app. Optional extra information: the
/// embedded `Source` stays the source of truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLink {
    /// Absolute path of the file when it was last read.
    pub path: PathBuf,
    /// Path relative to the folder of the project file, when the project had
    /// a path. Tried first, so a project folder can move as a whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative: Option<PathBuf>,
    /// sha256 of the embedded bytes, lower-case hex.
    #[serde(default)]
    pub sha256: String,
    /// Modification time of the file when it was last read, RFC 3339 UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    /// The folder `relative` was made against. `relative` is only used
    /// while the project file is in this folder; elsewhere (for example
    /// after "save as" and an undo) the absolute path is used, so a file
    /// with the same name next to the new project is never taken by
    /// mistake. Not stored: a relative path in a project file is relative
    /// to that file, so opening a project sets it to the project's folder.
    #[serde(skip)]
    pub anchor_dir: Option<PathBuf>,
}

/// A broken link loads as no link, so it never stops a project from opening.
fn is_zero_rect(r: &Rect) -> bool {
    *r == Rect::ZERO
}

fn lenient_link<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<SourceLink>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
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
    /// False keeps the object visible but leaves it out of the cut. Per
    /// copy, like `visible`. Projects from before this load as true, and
    /// true is not written, so they save as they were.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub cut_enabled: bool,
    /// Projects from before cut options load with the defaults.
    #[serde(default)]
    pub cut: CutOptions,
    /// The file this object was imported from. Projects from before links,
    /// imports from text and unlinked objects have none.
    #[serde(
        default,
        deserialize_with = "lenient_link",
        skip_serializing_if = "Option::is_none"
    )]
    pub link: Option<SourceLink>,
    #[serde(skip)]
    pub geometry: Arc<Vec<CutPath>>,
    /// Saved in the project, empty geometry or not: the transform is
    /// relative to its centre, so `project.open` needs it to keep the
    /// object in place when the rebuilt geometry differs. Older projects
    /// load `Rect::ZERO`.
    #[serde(
        default,
        rename = "local_bounds_mm",
        skip_serializing_if = "is_zero_rect"
    )]
    pub local_bounds: Rect,
    /// The source's page frame in local mm (SVG page box, image pixel
    /// frame). `Rect::ZERO` when unknown; see `anchor_frame`. Saved in the
    /// project for the same reason as `local_bounds`.
    #[serde(default, rename = "frame_mm", skip_serializing_if = "is_zero_rect")]
    pub frame: Rect,
    /// The group of linked copies this object belongs to. Members share
    /// the source (with the trace settings), the geometry, the cut options,
    /// the scale and the file link; each keeps its own position, rotation,
    /// mirror, name, visibility and whether it is cut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_group: Option<GroupId>,
}

impl Object {
    /// An object with only what drawing and bounds read: its geometry,
    /// placement and cut options. No source, name or file link; id 0. For
    /// views that draw a shape outside a document.
    pub fn shape_only(
        geometry: Arc<Vec<CutPath>>,
        local_bounds: Rect,
        transform: Transform,
        cut: CutOptions,
    ) -> Self {
        Self {
            id: ObjectId(0),
            name: String::new(),
            source: Source::Svg {
                name: String::new(),
                bytes: Arc::from(Vec::new()),
            },
            transform,
            visible: true,
            cut_enabled: true,
            cut,
            link: None,
            geometry,
            local_bounds,
            frame: Rect::ZERO,
            link_group: None,
        }
    }

    /// Sets the local geometry and recomputes `local_bounds`
    /// (`Rect::ZERO` when empty).
    pub fn set_geometry(&mut self, geometry: Vec<CutPath>) {
        self.local_bounds = bounds(&geometry).unwrap_or(Rect::ZERO);
        self.geometry = Arc::new(geometry);
    }

    /// The page frame, or `local_bounds` when the frame is unknown.
    pub fn anchor_frame(&self) -> Rect {
        if self.frame == Rect::ZERO {
            self.local_bounds
        } else {
            self.frame
        }
    }

    /// Replaces the geometry and the page frame and keeps the frame where
    /// it was on the mat: the new frame's top-left lands where the old
    /// frame's top-left was, with the same scale, rotation and mirror. Only
    /// `center_mm` changes, because the placement is relative to the centre
    /// of `local_bounds`.
    pub fn set_geometry_anchored(&mut self, geometry: Vec<CutPath>, frame: Rect) {
        let local_bounds = bounds(&geometry).unwrap_or(Rect::ZERO);
        self.share_geometry_anchored(Arc::new(geometry), local_bounds, frame);
    }

    /// [`Object::set_geometry_anchored`] with geometry another object
    /// already holds: the `Arc` is shared, not copied. `local_bounds` must
    /// be the bounds of `geometry`.
    pub fn share_geometry_anchored(
        &mut self,
        geometry: Arc<Vec<CutPath>>,
        local_bounds: Rect,
        frame: Rect,
    ) {
        let old_origin = self.anchor_frame().origin();
        let old_placement = self.transform.affine(self.local_bounds);
        self.geometry = geometry;
        self.local_bounds = local_bounds;
        self.frame = frame;
        let offset = old_origin - self.anchor_frame().origin();
        self.transform.center_mm = old_placement * (self.local_bounds.center() + offset);
    }

    /// True if `self` and `other` agree on every field linked copies share:
    /// the same geometry and source bytes (one `Arc` each), the same source
    /// with its trace settings, cut options, file link, local bounds, frame
    /// and scale.
    pub fn shares_with(&self, other: &Object) -> bool {
        let bytes = |o: &Object| match &o.source {
            Source::Svg { bytes, .. } | Source::Raster { bytes, .. } => bytes.clone(),
        };
        Arc::ptr_eq(&self.geometry, &other.geometry)
            && Arc::ptr_eq(&bytes(self), &bytes(other))
            && self.source == other.source
            && self.cut == other.cut
            && self.link == other.link
            && self.local_bounds == other.local_bounds
            && self.frame == other.frame
            && self.transform.scale_x == other.transform.scale_x
            && self.transform.scale_y == other.transform.scale_y
    }

    /// True if `self` and `other` hold equal values in every shared field
    /// (see [`Object::shares_with`]), whether or not they share `Arc`s.
    /// The geometry is not compared: a loaded project has none yet.
    pub fn shared_values_eq(&self, other: &Object) -> bool {
        self.source == other.source
            && self.cut == other.cut
            && self.link == other.link
            && self.local_bounds == other.local_bounds
            && self.frame == other.frame
            && self.transform.scale_x == other.transform.scale_x
            && self.transform.scale_y == other.transform.scale_y
    }

    /// Takes the shared fields of `from` (see [`Object::shares_with`]) and
    /// keeps its own place: new geometry stays anchored to its frame, as a
    /// reload does, and a new scale keeps its own centre. Position,
    /// rotation, mirror, name, visibility, cut flag and group stay its own.
    pub fn take_shared(&mut self, from: &Object) {
        if !Arc::ptr_eq(&self.geometry, &from.geometry)
            || self.local_bounds != from.local_bounds
            || self.frame != from.frame
        {
            self.share_geometry_anchored(from.geometry.clone(), from.local_bounds, from.frame);
        }
        self.source = from.source.clone();
        self.cut = from.cut;
        self.link = from.link.clone();
        self.transform.scale_x = from.transform.scale_x;
        self.transform.scale_y = from.transform.scale_y;
    }

    /// True if the object's placement is one `project.open` accepts: a
    /// valid transform, and its local bounds and frame placed on the mat
    /// stay finite and within `MAX_PLACEMENT_MM`. Commands that change a
    /// transform must keep this true, so every saved project reopens.
    pub fn placement_is_valid(&self) -> bool {
        use crate::transform::MAX_PLACEMENT_MM;
        if !self.transform.is_valid() {
            return false;
        }
        let affine = self.transform.affine(self.local_bounds);
        [self.local_bounds, self.frame].iter().all(|r| {
            let p = affine.transform_rect_bbox(*r);
            [p.x0, p.y0, p.x1, p.y1]
                .iter()
                .all(|v| v.is_finite() && v.abs() <= MAX_PLACEMENT_MM)
        })
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
            cut_enabled: true,
            cut: CutOptions::default(),
            link: None,
            geometry: Arc::default(),
            local_bounds: Rect::ZERO,
            frame: Rect::ZERO,
            link_group: None,
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

    /// The members of `id`'s group of linked copies in document order, or
    /// just `id` when it is not linked. Empty for an unknown id.
    pub fn members(&self, id: ObjectId) -> Vec<ObjectId> {
        match self.get(id).map(|o| o.link_group) {
            None => Vec::new(),
            Some(None) => vec![id],
            Some(Some(g)) => self.group_members(g),
        }
    }

    /// The members of group `g` in document order.
    pub fn group_members(&self, g: GroupId) -> Vec<ObjectId> {
        self.objects
            .iter()
            .filter(|o| o.link_group == Some(g))
            .map(|o| o.id)
            .collect()
    }

    /// The first member of every group of linked copies, in document order.
    pub fn group_leads(&self) -> Vec<ObjectId> {
        let mut seen = Vec::new();
        let mut out = Vec::new();
        for o in &self.objects {
            if let Some(g) = o.link_group
                && !seen.contains(&g)
            {
                seen.push(g);
                out.push(o.id);
            }
        }
        out
    }

    /// A group id no object uses.
    pub fn new_group_id(&self) -> GroupId {
        let max = self
            .objects
            .iter()
            .filter_map(|o| o.link_group)
            .map(|g| g.0)
            .max()
            .unwrap_or(0);
        GroupId(max + 1)
    }

    /// Gives the shared fields of `from` to every other member of its group
    /// ([`Object::take_shared`]): each keeps its own position, rotation and
    /// mirror. Commands that change a shared field of one member call this,
    /// so the whole group changes in the same undo step.
    pub fn spread_shared(&mut self, from: ObjectId) {
        let Some(lead) = self.get(from).filter(|o| o.link_group.is_some()).cloned() else {
            return;
        };
        for o in &mut self.objects {
            if o.id != from && o.link_group == lead.link_group {
                o.take_shared(&lead);
            }
        }
    }

    /// Makes the members of every group share their shared fields as one
    /// `Arc` each, without moving anything. For a freshly loaded project,
    /// whose members hold equal values in separate `Arc`s. Members whose
    /// shared fields differ (a hand-edited file) are never overwritten: the
    /// group keeps its majority ([`Document::unlink_minority`]) and the
    /// others leave it with their own design. Returns the ids of those.
    pub fn share_link_groups(&mut self) -> Vec<ObjectId> {
        let unlinked = self.unlink_minority(Object::shared_values_eq);
        let mut leads: Vec<Object> = Vec::new();
        for o in &mut self.objects {
            let Some(g) = o.link_group else { continue };
            match leads.iter().find(|l| l.link_group == Some(g)) {
                Some(lead) => {
                    o.source = lead.source.clone();
                    o.geometry = lead.geometry.clone();
                }
                None => leads.push(o.clone()),
            }
        }
        unlinked
    }

    /// Unlinks every member that does not share its fields with the
    /// majority of its group ([`Object::shares_with`]); each keeps its own
    /// design. Returns the ids of the unlinked members.
    pub fn unlink_mismatched(&mut self) -> Vec<ObjectId> {
        self.unlink_minority(Object::shares_with)
    }

    /// Splits each group into classes of members that are `same` as each
    /// other and keeps the largest class linked (on a tie, the one with the
    /// earliest member). Members outside it leave the group and keep their
    /// own data. Groups of one are dissolved. Returns the unlinked ids in
    /// document order.
    fn unlink_minority(&mut self, same: impl Fn(&Object, &Object) -> bool) -> Vec<ObjectId> {
        let mut unlinked = Vec::new();
        let groups: Vec<GroupId> = self
            .group_leads()
            .into_iter()
            .filter_map(|id| self.get(id)?.link_group)
            .collect();
        for g in groups {
            // Classes in order of their first member: (first index, members).
            let mut classes: Vec<(usize, Vec<usize>)> = Vec::new();
            for (i, o) in self.objects.iter().enumerate() {
                if o.link_group != Some(g) {
                    continue;
                }
                match classes
                    .iter_mut()
                    .find(|(first, _)| same(o, &self.objects[*first]))
                {
                    Some((_, members)) => members.push(i),
                    None => classes.push((i, vec![i])),
                }
            }
            // The first class of the largest size has the earliest member.
            let largest = classes.iter().map(|(_, m)| m.len()).max().unwrap_or(0);
            let keep = classes.iter().position(|(_, m)| m.len() == largest);
            for (k, (_, members)) in classes.iter().enumerate() {
                if Some(k) != keep {
                    for &i in members {
                        self.objects[i].link_group = None;
                        unlinked.push(i);
                    }
                }
            }
        }
        unlinked.sort_unstable();
        let ids = unlinked.into_iter().map(|i| self.objects[i].id).collect();
        self.dissolve_lonely_groups();
        ids
    }

    /// Drops groups with a single member: a group of one is no group.
    pub fn dissolve_lonely_groups(&mut self) {
        let mut counts = std::collections::HashMap::<GroupId, usize>::new();
        for g in self.objects.iter().filter_map(|o| o.link_group) {
            *counts.entry(g).or_default() += 1;
        }
        for o in &mut self.objects {
            if o.link_group.is_some_and(|g| counts[&g] < 2) {
                o.link_group = None;
            }
        }
    }

    /// True if the members of every group agree on their shared fields
    /// ([`Object::shares_with`]). The engine checks this after every
    /// command, so linked copies never drift apart.
    pub fn link_groups_agree(&self) -> bool {
        let mut first = std::collections::HashMap::<GroupId, &Object>::new();
        self.objects.iter().all(|o| match o.link_group {
            None => true,
            Some(g) => {
                let lead = *first.entry(g).or_insert(o);
                std::ptr::eq(lead, o) || o.shares_with(lead)
            }
        })
    }
}

fn default_true() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}
