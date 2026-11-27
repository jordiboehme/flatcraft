//! Registry commands and the helpers they share.

pub mod arrange;
pub mod check;
pub mod edit;
pub mod export;
pub mod group;
pub mod job;
pub mod notes;
pub mod object;
pub mod profiles;
pub mod project;
pub mod sources;
pub mod tools;

use flatcraft_core::document::{Document, JobSettings, Object, ObjectId};
use flatcraft_core::geometry::bounds;
use flatcraft_device::{Media, PlotterProfile, ProfileError};
use kurbo::{Point, Rect, Vec2};
use serde::de::DeserializeOwned;

use crate::cutgeom::Knife;
use crate::session::{CommandError, Session};

/// Defines a unit struct that implements `Command` by parsing `$params` and
/// calling `$run(session, params, origin)`.
macro_rules! command {
    (
        $ty:ident,
        id: $id:literal,
        title: $title:literal,
        mutates: $mutates:expr,
        $(session: $session:expr,)?
        params: $params:ty,
        run: $run:path,
        description: $desc:expr $(,)?
    ) => {
        pub struct $ty;

        impl $crate::registry::Command for $ty {
            fn id(&self) -> &'static str {
                $id
            }
            fn title(&self) -> &'static str {
                $title
            }
            fn description(&self) -> &'static str {
                $desc
            }
            fn params_schema(&self) -> schemars::Schema {
                schemars::schema_for!($params)
            }
            fn mutates_document(&self) -> bool {
                $mutates
            }
            fn changes_session(&self) -> bool {
                false $(|| $session)?
            }
            fn run(
                &self,
                s: &mut $crate::session::Session,
                params: serde_json::Value,
                origin: $crate::session::Origin,
            ) -> Result<serde_json::Value, $crate::session::CommandError> {
                let p: $params = $crate::commands::parse(params)?;
                $run(s, p, origin)
            }
        }
    };
}
pub(crate) use command;

/// Params of commands that take none.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}

/// Parses params; `null` counts as `{}`. Unknown fields are refused.
pub(crate) fn parse<P: DeserializeOwned>(params: serde_json::Value) -> Result<P, CommandError> {
    let params = if params.is_null() {
        serde_json::Value::Object(Default::default())
    } else {
        params
    };
    serde_json::from_value(params).map_err(|e| CommandError::InvalidParams(e.to_string()))
}

/// The objects a command works on: `ids` if given, else the selection.
/// A group id stands for every object in the group (see
/// [`flatcraft_core::nodes`]). Every id must exist. Objects come in the
/// order given (a group's in document order), each once.
pub(crate) fn resolve_ids(s: &Session, ids: &[u64]) -> Result<Vec<ObjectId>, CommandError> {
    let doc = s.doc();
    let given: Vec<u64> = if ids.is_empty() {
        doc.selection.iter().map(|i| i.0).collect()
    } else {
        ids.to_vec()
    };
    if given.is_empty() {
        return Err(CommandError::InvalidParams(
            "no object ids given and nothing is selected".into(),
        ));
    }
    let out = expand_ids(&doc.document, &given)?;
    if out.is_empty() {
        return Err(CommandError::InvalidParams(
            "the given groups hold no objects".into(),
        ));
    }
    Ok(out)
}

/// The objects of `ids` as given (no selection fallback): an object id
/// stands for itself, a group id for every object in the group, at any
/// depth. Every id must exist. Objects come in the order given (a group's
/// in document order), each once. Every command that takes object ids
/// goes through here, so each one takes a group id too.
pub(crate) fn expand_ids(doc: &Document, ids: &[u64]) -> Result<Vec<ObjectId>, CommandError> {
    let tree = (!doc.groups.is_empty()).then(|| doc.tree());
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for raw in ids {
        let id = ObjectId(*raw);
        check_node(doc, id)?;
        let leaves = match &tree {
            Some(t) if t.is_group(id) => t.leaves_of(id),
            _ => vec![id],
        };
        for leaf in leaves {
            if seen.insert(leaf) {
                out.push(leaf);
            }
        }
    }
    Ok(out)
}

/// What moves as one for the objects `leaves` (see
/// [`flatcraft_core::nodes::Tree::units`]): whole groups, and objects
/// picked inside a group or outside any. In document order.
pub(crate) fn units_of(doc: &Document, leaves: &[ObjectId]) -> Vec<ObjectId> {
    if doc.groups.is_empty() {
        return leaves.to_vec();
    }
    doc.tree().units(leaves)
}

/// [`resolve_ids`], then the units of those objects ([`units_of`]):
/// `(objects, units)`.
pub(crate) fn resolve_units(
    s: &Session,
    ids: &[u64],
) -> Result<(Vec<ObjectId>, Vec<ObjectId>), CommandError> {
    let leaves = resolve_ids(s, ids)?;
    let units = units_of(&s.doc().document, &leaves);
    Ok((leaves, units))
}

/// The objects of a unit: the object itself, or every object of the group.
pub(crate) fn unit_leaves(doc: &Document, unit: ObjectId) -> Vec<ObjectId> {
    if doc.group_node(unit).is_some() {
        doc.tree().leaves_of(unit)
    } else {
        vec![unit]
    }
}

/// The objects of `ids` that count for their common box: the shown ones,
/// or all of them when every one is hidden. A hidden member of a group or
/// a selection is not part of its box, as the window draws it.
pub(crate) fn box_members<'a>(doc: &'a Document, ids: &[ObjectId]) -> Vec<&'a Object> {
    let objs: Vec<&Object> = ids.iter().filter_map(|id| doc.get(*id)).collect();
    let any_shown = objs.iter().any(|o| o.visible);
    objs.into_iter()
        .filter(|o| o.visible || !any_shown)
        .collect()
}

/// The box of what is cut of the objects `ids` together (see
/// [`placed_bounds_or_center`] and [`box_members`]), `None` for no
/// objects.
pub(crate) fn common_box(doc: &Document, ids: &[ObjectId], knife: Knife) -> Option<Rect> {
    box_members(doc, ids)
        .into_iter()
        .map(|o| placed_bounds_or_center(o, knife))
        .reduce(|a, b| a.union(b))
}

/// What every box of what is cut is measured with: the active document's
/// plotter's flatten tolerance and narrowest filled shape
/// (`report::min_width_mm` of its blade offset), the same the cut geometry
/// uses; the default plotter's when the plotter is unknown.
pub fn knife_of(s: &Session) -> Knife {
    match s.profiles.plotter(&s.doc().document.job.plotter_id) {
        Ok(p) => Knife {
            tolerance_mm: p.flatten_tolerance_mm,
            min_width_mm: crate::report::min_width_mm(p.blade_offset_mm),
        },
        Err(_) => Knife::default(),
    }
}

/// Fails unless `id` names an object or a group.
pub(crate) fn check_node(doc: &Document, id: ObjectId) -> Result<(), CommandError> {
    if doc.has_node(id) {
        Ok(())
    } else {
        Err(CommandError::NotFound(format!(
            "no object or group with id {}",
            id.0
        )))
    }
}

/// Fails unless `id` names an object. For commands that work on one
/// object by nature (retrace, relink); a group id gets a hint.
pub(crate) fn check_exists(doc: &Document, id: ObjectId) -> Result<(), CommandError> {
    if doc.get(id).is_some() {
        return Ok(());
    }
    if doc.group_node(id).is_some() {
        return Err(CommandError::InvalidParams(format!(
            "{} is a group; give one of its objects (document.state lists them)",
            id.0
        )));
    }
    Err(CommandError::NotFound(format!(
        "no object with id {}",
        id.0
    )))
}

pub(crate) fn object_mut(doc: &mut Document, id: ObjectId) -> Result<&mut Object, CommandError> {
    doc.get_mut(id)
        .ok_or_else(|| CommandError::NotFound(format!("no object with id {}", id.0)))
}

/// Bounding box of what is cut of the object (its cut geometry per its cut
/// options, so stroke outlines count, and slivers thinner than
/// the knife's narrowest feature do not; see [`knife_of`]), in mat mm. Falls back to the
/// centre lines when nothing is cut; `None` if the object has no geometry.
pub fn placed_bounds(o: &Object, knife: Knife) -> Option<Rect> {
    crate::cutgeom::cut_bounds(o, knife).or_else(|| bounds(&o.placed_paths()))
}

/// The source's page frame placed on the mat (its bounding box after the
/// object's transform), or `None` when the frame is unknown. For a picture
/// that traced to nothing this is the image the person placed.
pub fn placed_frame(o: &Object) -> Option<Rect> {
    (o.frame != Rect::ZERO).then(|| {
        o.transform
            .affine(o.local_bounds)
            .transform_rect_bbox(o.frame)
    })
}

/// The box Auto arrange packs or packs around: the cut geometry's box
/// ([`placed_bounds`]), or for an object without geometry (an untraced
/// picture) its placed frame ([`placed_frame`]).
///
/// It is [`centred_arrange_bounds`] moved by the centre, exactly as the
/// window's preview builds it from its cached shape, so both get the
/// same numbers to the last bit.
pub fn arrange_bounds(o: &Object, knife: Knife) -> Option<Rect> {
    let mut at_origin = Object::shape_only(o.geometry.clone(), o.local_bounds, o.transform, o.cut);
    at_origin.frame = o.frame;
    at_origin.transform.center_mm = Point::ORIGIN;
    centred_arrange_bounds(&at_origin, knife).map(|b| b + o.transform.center_mm.to_vec2())
}

/// The Auto arrange box of `o` as placed with its centre at 0,0 (the
/// caller sets it). [`arrange_bounds`] and the window's shape cache both
/// use it.
pub fn centred_arrange_bounds(o: &Object, knife: Knife) -> Option<Rect> {
    placed_bounds(o, knife).or_else(|| placed_frame(o))
}

/// Bounding box of the placed geometry, or the transform centre as a point
/// when the object has no geometry.
pub(crate) fn placed_bounds_or_center(o: &Object, knife: Knife) -> Rect {
    placed_bounds(o, knife)
        .unwrap_or_else(|| Rect::from_points(o.transform.center_mm, o.transform.center_mm))
}

/// How far a box may reach over each edge of the cutting area and still
/// count as inside, mm (float noise). The planner uses the same.
pub const AREA_SLACK_MM: f64 = 1e-6;

/// True if `r` lies inside `area`, with a little slack for float noise.
/// The window's out-of-area count uses it too, so both always agree.
pub fn inside(r: Rect, area: Rect) -> bool {
    const SLACK: f64 = AREA_SLACK_MM;
    r.x0 >= area.x0 - SLACK
        && r.y0 >= area.y0 - SLACK
        && r.x1 <= area.x1 + SLACK
        && r.y1 <= area.y1 + SLACK
}

pub(crate) fn translate(o: &mut Object, d: Vec2) {
    o.transform.center_mm += d;
}

/// Degrees in [0, 360), never -0 (a mirror of 0 degrees).
pub(crate) fn normalize_deg(deg: f64) -> f64 {
    let r = deg.rem_euclid(360.0);
    if r >= 360.0 || r == 0.0 { 0.0 } else { r }
}

/// The cutting area of the active document's plotter and media (the
/// plotter's first mat when the document's mat is gone, see
/// [`effective_media`]).
pub(crate) fn cutting_area(s: &Session) -> Result<Rect, CommandError> {
    let job = &s.doc().document.job;
    let plotter = s.profiles.plotter(&job.plotter_id).map_err(profile_error)?;
    let (media, _) = effective_media(&plotter, job)?;
    plotter
        .cutting_area(&media.id)
        .ok_or_else(|| CommandError::Failed("the mat has no cutting area".into()))
}

/// The mat a job uses: the document's mat, or the plotter's first mat when
/// the plotter no longer lists it (hidden or deleted), with the warning
/// that says so.
pub fn effective_media<'a>(
    plotter: &'a PlotterProfile,
    job: &JobSettings,
) -> Result<(&'a Media, Option<String>), CommandError> {
    let (media, fell_back) = plotter.media_or_first(&job.media_id).ok_or_else(|| {
        CommandError::NotFound(format!(
            "media '{}' is not defined for plotter '{}', and the plotter has no mat",
            job.media_id, job.plotter_id
        ))
    })?;
    let warning = fell_back.then(|| {
        format!(
            "The mat '{}' is not available on this plotter (hidden or deleted), so FlatCraft uses '{}'. Pick a mat in the job settings.",
            job.media_id, media.name
        )
    });
    Ok((media, warning))
}

pub(crate) fn profile_error(e: ProfileError) -> CommandError {
    match e {
        ProfileError::UnknownPreset { id } => {
            CommandError::NotFound(format!("no plotter or material preset with id '{id}'"))
        }
        other => CommandError::Failed(profile_problem_text(&other)),
    }
}

/// The text of a profile error for people. A user preset with a bad value
/// can be repaired in the Profiles window, so its text says so; a built-in
/// preset cannot be edited there.
pub fn profile_problem_text(e: &ProfileError) -> String {
    match e {
        ProfileError::Invalid { id, .. } if !flatcraft_device::presets::is_system(id) => {
            format!("{e}. Open Settings to fix it.")
        }
        other => other.to_string(),
    }
}

pub(crate) fn rect_json(r: Rect) -> serde_json::Value {
    serde_json::json!({ "x0": r.x0, "y0": r.y0, "x1": r.x1, "y1": r.y1 })
}

pub(crate) fn point_arr(p: Point) -> serde_json::Value {
    serde_json::json!([p.x, p.y])
}

pub(crate) fn ids_json(ids: &[ObjectId]) -> serde_json::Value {
    serde_json::Value::Array(ids.iter().map(|i| i.0.into()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invalid(id: &str) -> ProfileError {
        ProfileError::Invalid {
            id: id.into(),
            reason: "bad".into(),
        }
    }

    #[test]
    fn only_user_presets_point_to_the_settings() {
        let user = profile_error(invalid("my-vinyl")).to_string();
        assert!(user.ends_with("Open Settings to fix it."), "{user}");
        for id in ["vinyl", "silhouette-cameo-2"] {
            let built_in = profile_error(invalid(id)).to_string();
            assert!(!built_in.contains("Open Settings"), "{built_in}");
        }
    }
}
