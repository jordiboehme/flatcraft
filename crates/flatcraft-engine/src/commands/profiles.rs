//! Profile queries and job settings of the active document.

use flatcraft_device::{PresetInfo, ProfileError};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{NoParams, command, profile_error};
use crate::session::{CommandError, Origin, Session};

/// Largest accepted pass count.
const MAX_PASSES: u32 = 20;

fn info_json(i: &PresetInfo) -> Value {
    json!({ "id": i.id, "name": i.name, "system": i.system, "inherits": i.inherits })
}

fn run_list(s: &mut Session, _: NoParams, _: Origin) -> Result<Value, CommandError> {
    Ok(json!({
        "plotters": s.profiles.plotters().iter().map(info_json).collect::<Vec<_>>(),
        "materials": s.profiles.materials().iter().map(info_json).collect::<Vec<_>>(),
    }))
}

command!(
    ProfilesList,
    id: "profiles.list",
    title: "List profiles",
    mutates: false,
    params: NoParams,
    run: run_list,
    description: "Lists the plotter and material presets, built-in (system) and user-made. Returns {plotters: [{id, name, system, inherits}], materials: [{id, name, system, inherits}]}. Use profiles.get for the details.",
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
    description: "Returns one plotter or material preset with inheritance resolved, as JSON with an extra field kind (plotter or material). Plotters list their media (mm), margins (mm), speed_range and force_range; materials list speed, force, passes, overcut_mm and per-plotter overrides.",
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
    description: "Sets the plotter preset of the active document. If the current media is not available on it, the plotter's first media is used. Speed or force overrides outside the new plotter's ranges are cleared. Returns {plotter_id, media_id, cleared_overrides: [name]}.",
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
    description: "Sets the media (mat or sheet size) of the active document. The id must be one of the plotter's media (see profiles.get on the plotter). Returns {media_id, width_mm, height_mm, cutting_area_mm: {x0,y0,x1,y1}}; the origin is the top-left of the media, y down.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OverridesParams {
    /// Speed override; must be inside the plotter's speed_range.
    pub speed: Option<u32>,
    /// Force (blade pressure) override; must be inside the plotter's force_range.
    pub force: Option<u32>,
    /// Number of passes, 1 to 20.
    pub passes: Option<u32>,
    /// First remove all overrides, so the material values apply again (default false).
    #[serde(default)]
    pub clear: bool,
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
        .map(|v| check_range("speed", v, plotter.speed_range))
        .transpose()?;
    let force = p
        .force
        .map(|v| check_range("force", v, plotter.force_range))
        .transpose()?;
    if let Some(n) = p.passes
        && !(1..=MAX_PASSES).contains(&n)
    {
        return Err(CommandError::InvalidParams(format!(
            "passes {n} is outside 1 to {MAX_PASSES}"
        )));
    }
    let job = &mut s.doc_mut().document.job;
    if p.clear {
        job.speed = None;
        job.force = None;
        job.passes = None;
    }
    if speed.is_some() {
        job.speed = speed;
    }
    if force.is_some() {
        job.force = force;
    }
    if p.passes.is_some() {
        job.passes = p.passes;
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
    description: "Overrides the material's speed, force or passes for the active document. Values are checked against the plotter's speed_range and force_range (Cameo 2: speed 1-10, force 1-33); passes 1-20. Fields you leave out stay as they are; clear: true first removes all overrides. Returns {speed, force, passes}, where null means the material value.",
);
