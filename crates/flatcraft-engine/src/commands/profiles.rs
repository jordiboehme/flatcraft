//! Profile queries and job settings of the active document.

use flatcraft_core::document::{WeedLines, WeedSettings};
use flatcraft_device::{PresetInfo, ProfileError};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{NoParams, command, profile_error};
use crate::session::{CommandError, Origin, Session};

fn info_json(i: &PresetInfo) -> Value {
    json!({ "id": i.id, "name": i.name, "system": i.system, "inherits": i.inherits })
}

fn run_list(s: &mut Session, _: NoParams, _: Origin) -> Result<Value, CommandError> {
    Ok(json!({
        "plotters": s.profiles.plotters().iter().map(info_json).collect::<Vec<_>>(),
        "materials": s
            .profiles
            .materials()
            .iter()
            .map(|i| {
                let mut v = info_json(i);
                // A preset that cannot be read has no group; profiles.get
                // says why.
                let group = s.profiles.material_unchecked(&i.id).ok().map(|m| m.group);
                v["group"] = json!(group);
                v
            })
            .collect::<Vec<_>>(),
    }))
}

command!(
    ProfilesList,
    id: "profiles.list",
    title: "List profiles",
    mutates: false,
    params: NoParams,
    run: run_list,
    description: "Lists the plotter and material presets, built-in (system) and user-made. Returns {plotters: [{id, name, system, inherits}], materials: [{id, name, system, inherits, group}]}. group is vinyl, iron_on, paper_and_card or other (null when the preset cannot be read): the groups of the window's material picker. Use profiles.get for the values and the tips.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdParams {
    /// Preset or media id, as listed by profiles.list or profiles.get.
    pub id: String,
}

fn to_json<T: serde::Serialize>(kind: &str, v: &T) -> Result<Value, CommandError> {
    let mut value = serde_json::to_value(v).map_err(|e| CommandError::Failed(e.to_string()))?;
    if let Value::Object(map) = &mut value {
        map.insert("kind".into(), kind.into());
    }
    Ok(value)
}

fn run_get(s: &mut Session, p: IdParams, _: Origin) -> Result<Value, CommandError> {
    match s.profiles.plotter(&p.id) {
        Ok(plotter) => return to_json("plotter", &plotter),
        Err(ProfileError::UnknownPreset { .. }) => {}
        Err(e) => return Err(profile_error(e)),
    }
    let material = s.profiles.material(&p.id).map_err(profile_error)?;
    to_json("material", &material)
}

command!(
    ProfilesGet,
    id: "profiles.get",
    title: "Show profile",
    mutates: false,
    params: IdParams,
    run: run_get,
    description: "Returns one plotter or material preset with inheritance resolved, as JSON with an extra field kind (plotter or material). Plotters list their media (mm; each with kind, physical size, printed grid origin and size, calibrated), speed_range, force_range and passes_range; materials list speed, force, passes, overcut_mm, backing (true when it sits on a backing sheet), blade_depth_hint, group (vinyl, iron_on, paper_and_card or other), mat_tack (light, standard or strong: the mat grip to recommend, never sent), tips (built-in advice on which side goes up and how to weed; iron_on materials are cut mirrored) and per-plotter overrides. Both have notes when the person wrote some on that preset (left out when none): read them before a cut, they are the person's own experience. Agents cannot change notes or presets.",
);

fn run_set_plotter(s: &mut Session, p: IdParams, _: Origin) -> Result<Value, CommandError> {
    let plotter = s.profiles.plotter(&p.id).map_err(profile_error)?;
    let job = &mut s.doc_mut().document.job;
    if !plotter.media.iter().any(|m| m.id == job.media_id) {
        let first = plotter.media.first().ok_or_else(|| {
            CommandError::Failed(format!("plotter '{}' defines no media", plotter.id))
        })?;
        job.media_id = first.id.clone();
    }
    let mut cleared = Vec::new();
    let in_range = |v: u8, r: [u8; 2]| v >= r[0] && v <= r[1];
    if job.speed.is_some_and(|v| !in_range(v, plotter.speed_range)) {
        job.speed = None;
        cleared.push("speed");
    }
    if job.force.is_some_and(|v| !in_range(v, plotter.force_range)) {
        job.force = None;
        cleared.push("force");
    }
    let [lo, hi] = plotter.passes_range;
    if job.passes.is_some_and(|v| v < lo.max(1) || v > hi) {
        job.passes = None;
        cleared.push("passes");
    }
    job.plotter_id = plotter.id.clone();
    Ok(json!({
        "plotter_id": job.plotter_id,
        "media_id": job.media_id,
        "cleared_overrides": cleared,
    }))
}

command!(
    JobSetPlotter,
    id: "job.set_plotter",
    title: "Set plotter",
    mutates: true,
    params: IdParams,
    run: run_set_plotter,
    description: "Sets the plotter preset of the active document. If the current media is not available on it, the plotter's first media is used. Speed, force or passes overrides outside the new plotter's ranges are cleared. Returns {plotter_id, media_id, cleared_overrides: [name]}.",
);

fn run_set_material(s: &mut Session, p: IdParams, _: Origin) -> Result<Value, CommandError> {
    let material = s.profiles.material(&p.id).map_err(profile_error)?;
    let job = &mut s.doc_mut().document.job;
    job.material_id = material.id.clone();
    Ok(json!({ "material_id": job.material_id }))
}

command!(
    JobSetMaterial,
    id: "job.set_material",
    title: "Set material",
    mutates: true,
    params: IdParams,
    run: run_set_material,
    description: "Sets the material preset of the active document. The material gives speed, force and passes unless job.set_overrides overrides them. Returns {material_id}.",
);

fn run_set_media(s: &mut Session, p: IdParams, _: Origin) -> Result<Value, CommandError> {
    let plotter_id = s.doc().document.job.plotter_id.clone();
    let plotter = s.profiles.plotter(&plotter_id).map_err(profile_error)?;
    let Some(media) = plotter.media.iter().find(|m| m.id == p.id) else {
        let known: Vec<&str> = plotter.media.iter().map(|m| m.id.as_str()).collect();
        return Err(CommandError::NotFound(format!(
            "plotter '{plotter_id}' has no media '{}'; available: {}",
            p.id,
            known.join(", ")
        )));
    };
    let area = plotter.cutting_area(&media.id);
    s.doc_mut().document.job.media_id = media.id.clone();
    Ok(json!({
        "media_id": media.id,
        "width_mm": media.width_mm,
        "height_mm": media.height_mm,
        "cutting_area_mm": area.map(super::rect_json),
    }))
}

command!(
    JobSetMedia,
    id: "job.set_media",
    title: "Set media",
    mutates: true,
    params: IdParams,
    run: run_set_media,
    description: "Sets the media (mat or sheet size) of the active document. The id must be one of the plotter's media (see profiles.get on the plotter). Returns {media_id, width_mm, height_mm, cutting_area_mm: {x0,y0,x1,y1}}; the origin is the 0,0 of the grid printed on the mat, y down. cutting_area_mm is the grid, minus a 5 mm safety margin while the mat is not calibrated.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OverridesParams {
    /// Speed override; must be inside the plotter's speed_range. null removes the override, so the material value applies again.
    #[serde(default, deserialize_with = "present")]
    pub speed: Option<Option<u32>>,
    /// Force (blade pressure) override; must be inside the plotter's force_range. null removes the override.
    #[serde(default, deserialize_with = "present")]
    pub force: Option<Option<u32>>,
    /// Number of passes; must be inside the plotter's passes_range (1 to 20 by default). null removes the override.
    #[serde(default, deserialize_with = "present")]
    pub passes: Option<Option<u32>>,
    /// First remove all overrides, so the material values apply again (default false).
    #[serde(default)]
    pub clear: bool,
}

/// A field that is present: `Some(None)` for `null`, `Some(Some(v))` for a
/// value. A missing field stays `None` (`#[serde(default)]`).
fn present<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

fn check_range(name: &str, v: u32, r: [u8; 2]) -> Result<u8, CommandError> {
    if v >= u32::from(r[0]) && v <= u32::from(r[1]) {
        Ok(v as u8)
    } else {
        Err(CommandError::InvalidParams(format!(
            "{name} {v} is outside the plotter's range {} to {}",
            r[0], r[1]
        )))
    }
}

fn run_set_overrides(
    s: &mut Session,
    p: OverridesParams,
    _: Origin,
) -> Result<Value, CommandError> {
    let plotter_id = s.doc().document.job.plotter_id.clone();
    let plotter = s.profiles.plotter(&plotter_id).map_err(profile_error)?;
    let speed = p
        .speed
        .map(|v| {
            v.map(|v| check_range("speed", v, plotter.speed_range))
                .transpose()
        })
        .transpose()?;
    let force = p
        .force
        .map(|v| {
            v.map(|v| check_range("force", v, plotter.force_range))
                .transpose()
        })
        .transpose()?;
    let [lo, hi] = plotter.passes_range;
    if let Some(Some(n)) = p.passes
        && !(lo.max(1)..=hi).contains(&n)
    {
        return Err(CommandError::InvalidParams(format!(
            "passes {n} is outside the plotter's range {} to {hi}",
            lo.max(1)
        )));
    }
    let job = &mut s.doc_mut().document.job;
    if p.clear {
        job.speed = None;
        job.force = None;
        job.passes = None;
    }
    if let Some(v) = speed {
        job.speed = v;
    }
    if let Some(v) = force {
        job.force = v;
    }
    if let Some(v) = p.passes {
        job.passes = v;
    }
    Ok(json!({ "speed": job.speed, "force": job.force, "passes": job.passes }))
}

command!(
    JobSetOverrides,
    id: "job.set_overrides",
    title: "Set cut overrides",
    mutates: true,
    params: OverridesParams,
    run: run_set_overrides,
    description: "Overrides the material's speed, force or passes for the active document. Values are checked against the plotter's speed_range, force_range and passes_range (Cameo 2: speed 1-10, force 1-33, passes 1-20). Fields you leave out stay as they are; a field set to null removes that override, so the material value applies again; clear: true first removes all overrides. Returns {speed, force, passes}, where null means the material value.",
);

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StrategyParam {
    /// Clean corners first: entries on waste-side corners, few blade turns, short travel.
    Quality,
    /// Short travel first: blade turns cost nothing.
    Fast,
    /// As quality, but every outline comes after every hole and inner piece, so the sheet holds together until the end.
    OutlinesLast,
}

impl From<StrategyParam> for flatcraft_core::document::CutStrategy {
    fn from(p: StrategyParam) -> Self {
        use flatcraft_core::document::CutStrategy;
        match p {
            StrategyParam::Quality => CutStrategy::Quality,
            StrategyParam::Fast => CutStrategy::Fast,
            StrategyParam::OutlinesLast => CutStrategy::OutlinesLast,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrategyParams {
    /// quality, fast or outlines_last.
    pub strategy: StrategyParam,
}

fn run_set_strategy(s: &mut Session, p: StrategyParams, _: Origin) -> Result<Value, CommandError> {
    s.doc_mut().document.job.strategy = p.strategy.into();
    Ok(json!({ "strategy": s.doc().document.job.strategy }))
}

command!(
    JobSetStrategy,
    id: "job.set_strategy",
    title: "Set cut strategy",
    mutates: true,
    params: StrategyParams,
    run: run_set_strategy,
    description: "Sets the cut strategy of the active document, in one undo step: quality (clean corners first), fast (short travel first) or outlines_last (as quality, but every outline after every hole and inner piece, so the sheet holds together until the end: on a mat for the whole job; without a mat, per 50 mm feed band). Every strategy cuts the holes and inner pieces of a shape before its outline. New documents start with the strategy of the preferences. Returns {strategy}.",
);

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WeedLinesParam {
    /// No weed lines.
    Off,
    /// Lines across the carriage (along x).
    Horizontal,
    /// Lines in the feed direction (along y).
    Vertical,
    /// Both directions: a grid.
    Both,
}

impl From<WeedLinesParam> for WeedLines {
    fn from(p: WeedLinesParam) -> Self {
        match p {
            WeedLinesParam::Off => WeedLines::Off,
            WeedLinesParam::Horizontal => WeedLines::Horizontal,
            WeedLinesParam::Vertical => WeedLines::Vertical,
            WeedLinesParam::Both => WeedLines::Both,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeedingParams {
    /// Cut a weed box (a rectangle around the design).
    #[serde(rename = "box")]
    pub weed_box: Option<bool>,
    /// How far the weed box stays from the design, mm (1 to 50).
    pub margin_mm: Option<f64>,
    /// One weed box per object instead of one around everything.
    pub per_object: Option<bool>,
    /// Weed lines: off, horizontal, vertical or both.
    pub lines: Option<WeedLinesParam>,
    /// The distance between weed lines, mm (5 to 300).
    pub spacing_mm: Option<f64>,
}

/// The weed settings as `document.state` and `job.set_weeding` show them.
pub(crate) fn weed_json(w: &WeedSettings) -> Value {
    json!({
        "box": w.weed_box,
        "margin_mm": w.margin_mm,
        "per_object": w.per_object,
        "lines": w.lines,
        "spacing_mm": w.spacing_mm,
    })
}

fn check_mm(name: &str, v: f64, lo: f64, hi: f64) -> Result<f64, CommandError> {
    if v.is_finite() && (lo..=hi).contains(&v) {
        Ok(v)
    } else {
        Err(CommandError::InvalidParams(format!(
            "{name} {v} is outside the range {lo} to {hi} mm"
        )))
    }
}

fn run_set_weeding(s: &mut Session, p: WeedingParams, _: Origin) -> Result<Value, CommandError> {
    use flatcraft_core::limits::{
        MAX_WEED_MARGIN_MM, MAX_WEED_SPACING_MM, MIN_WEED_MARGIN_MM, MIN_WEED_SPACING_MM,
    };
    let margin = p
        .margin_mm
        .map(|v| check_mm("margin_mm", v, MIN_WEED_MARGIN_MM, MAX_WEED_MARGIN_MM))
        .transpose()?;
    let spacing = p
        .spacing_mm
        .map(|v| check_mm("spacing_mm", v, MIN_WEED_SPACING_MM, MAX_WEED_SPACING_MM))
        .transpose()?;
    let w = &mut s.doc_mut().document.job.weed;
    if let Some(v) = p.weed_box {
        w.weed_box = v;
    }
    if let Some(v) = margin {
        w.margin_mm = v;
    }
    if let Some(v) = p.per_object {
        w.per_object = v;
    }
    if let Some(v) = p.lines {
        w.lines = v.into();
    }
    if let Some(v) = spacing {
        w.spacing_mm = v;
    }
    Ok(weed_json(&s.doc().document.job.weed))
}

command!(
    JobSetWeeding,
    id: "job.set_weeding",
    title: "Set weed box and weed lines",
    mutates: true,
    params: WeedingParams,
    run: run_set_weeding,
    description: "Sets the weed box and weed lines of the active document, in one undo step. Weeding means peeling the waste off the backing after the cut; these extra cuts go through the waste only and make that easier. box: a rectangle around the design, margin_mm (1 to 50, default 3) away from it; per_object: one box per object instead of one around everything (boxes that would overlap or come closer than 5 mm become one). A box that does not fit inside the cutting area with 1 mm to spare, or whose outline would cut kept material, is left out with a warning. lines: off, horizontal, vertical or both, every spacing_mm (5 to 300, default 20), across the design plus margin_mm (also with the box off), and with per_object only inside the boxes; they stop more than 1 mm before every design cut, never cut inside a shape or a letter's counter, and pieces shorter than 5 mm are left out. Weed lines are cut after the design, the weed box last; without a mat the weed lines and the box outline are cut band by band after each feed band's design, so the roll never goes back for them, and the box pieces overlap by 0.5 mm at their joins. They are cut with the job's passes, shown in the path preview and included in document.export. Fields you leave out stay as they are. Returns {box, margin_mm, per_object, lines, spacing_mm}.",
);
