//! Registry commands and the helpers they share.

pub mod arrange;
pub mod edit;
pub mod object;
pub mod profiles;
pub mod project;

use flatcraft_core::document::{Document, Object, ObjectId};
use flatcraft_core::geometry::bounds;
use flatcraft_device::ProfileError;
use kurbo::{Point, Rect, Vec2};
use serde::de::DeserializeOwned;

use crate::session::{CommandError, Session};

/// Defines a unit struct that implements `Command` by parsing `$params` and
/// calling `$run(session, params, origin)`.
macro_rules! command {
    (
        $ty:ident,
        id: $id:literal,
        title: $title:literal,
        mutates: $mutates:expr,
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
/// Every id must exist.
pub(crate) fn resolve_ids(s: &Session, ids: &[u64]) -> Result<Vec<ObjectId>, CommandError> {
    let doc = s.doc();
    let ids: Vec<ObjectId> = if ids.is_empty() {
        doc.selection.iter().copied().collect()
    } else {
        let mut out = Vec::new();
        for &raw in ids {
            let id = ObjectId(raw);
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    };
    if ids.is_empty() {
        return Err(CommandError::InvalidParams(
            "no object ids given and nothing is selected".into(),
        ));
    }
    for id in &ids {
        check_exists(&doc.document, *id)?;
    }
    Ok(ids)
}

pub(crate) fn check_exists(doc: &Document, id: ObjectId) -> Result<(), CommandError> {
    doc.get(id)
        .map(|_| ())
        .ok_or_else(|| CommandError::NotFound(format!("no object with id {}", id.0)))
}

pub(crate) fn object_mut(doc: &mut Document, id: ObjectId) -> Result<&mut Object, CommandError> {
    doc.get_mut(id)
        .ok_or_else(|| CommandError::NotFound(format!("no object with id {}", id.0)))
}

/// Bounding box of the placed geometry in mat mm, `None` if the object has
/// no geometry.
pub(crate) fn placed_bounds(o: &Object) -> Option<Rect> {
    bounds(&o.placed_paths())
}

/// Bounding box of the placed geometry, or the transform centre as a point
/// when the object has no geometry.
pub(crate) fn placed_bounds_or_center(o: &Object) -> Rect {
    placed_bounds(o)
        .unwrap_or_else(|| Rect::from_points(o.transform.center_mm, o.transform.center_mm))
}

pub(crate) fn translate(o: &mut Object, d: Vec2) {
    o.transform.center_mm += d;
}

/// Degrees in [0, 360).
pub(crate) fn normalize_deg(deg: f64) -> f64 {
    let r = deg.rem_euclid(360.0);
    if r >= 360.0 { 0.0 } else { r }
}

/// The cutting area of the active document's plotter and media.
pub(crate) fn cutting_area(s: &Session) -> Result<Rect, CommandError> {
    let job = &s.doc().document.job;
    let plotter = s.profiles.plotter(&job.plotter_id).map_err(profile_error)?;
    plotter.cutting_area(&job.media_id).ok_or_else(|| {
        CommandError::NotFound(format!(
            "media '{}' is not defined for plotter '{}'",
            job.media_id, job.plotter_id
        ))
    })
}

pub(crate) fn profile_error(e: ProfileError) -> CommandError {
    match e {
        ProfileError::UnknownPreset { id } => {
            CommandError::NotFound(format!("no plotter or material preset with id '{id}'"))
        }
        other => CommandError::Failed(other.to_string()),
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
