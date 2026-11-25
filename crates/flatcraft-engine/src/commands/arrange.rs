//! Arrange commands: align and automatic shelf packing.

use flatcraft_core::document::{Object, ObjectId};
use kurbo::{Point, Rect, Vec2};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    command, cutting_area, ids_json, object_mut, placed_bounds, placed_bounds_or_center,
    resolve_ids, translate,
};
use crate::session::{CommandError, Origin, Session};

/// Slack for float noise when testing whether something fits.
const FIT_EPS: f64 = 1e-9;

/// True if `r` lies inside `area`, with a little slack for float noise.
fn inside(r: Rect, area: Rect) -> bool {
    const SLACK: f64 = 1e-6;
    r.x0 >= area.x0 - SLACK
        && r.y0 >= area.y0 - SLACK
        && r.x1 <= area.x1 + SLACK
        && r.y1 <= area.y1 + SLACK
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlignMode {
    Left,
    Hcenter,
    Right,
    Top,
    Vcenter,
    Bottom,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlignTarget {
    /// The common bounding box of the given objects.
    #[default]
    Selection,
    /// The cutting area (the mat's grid, minus a safety margin while the mat is not calibrated).
    CuttingArea,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlignParams {
    /// Object ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// Which edge or centre line to line up.
    pub mode: AlignMode,
    /// What to align to (default selection).
    #[serde(default)]
    pub relative_to: AlignTarget,
}

fn run_align(s: &mut Session, p: AlignParams, _: Origin) -> Result<Value, CommandError> {
    let ids = resolve_ids(s, &p.ids)?;
    let target = match p.relative_to {
        AlignTarget::CuttingArea => cutting_area(s)?,
        AlignTarget::Selection => {
            let doc = &s.doc().document;
            ids.iter()
                .filter_map(|id| doc.get(*id))
                .map(placed_bounds_or_center)
                .reduce(|a, b| a.union(b))
                .unwrap_or(Rect::ZERO)
        }
    };
    let doc = &mut s.doc_mut().document;
    for id in &ids {
        let o = object_mut(doc, *id)?;
        let b = placed_bounds_or_center(o);
        let d = match p.mode {
            AlignMode::Left => Vec2::new(target.x0 - b.x0, 0.0),
            AlignMode::Hcenter => Vec2::new(target.center().x - b.center().x, 0.0),
            AlignMode::Right => Vec2::new(target.x1 - b.x1, 0.0),
            AlignMode::Top => Vec2::new(0.0, target.y0 - b.y0),
            AlignMode::Vcenter => Vec2::new(0.0, target.center().y - b.center().y),
            AlignMode::Bottom => Vec2::new(0.0, target.y1 - b.y1),
        };
        translate(o, d);
    }
    Ok(json!({ "ids": ids_json(&ids) }))
}

command!(
    ArrangeAlign,
    id: "arrange.align",
    title: "Align",
    mutates: true,
    params: AlignParams,
    run: run_align,
    description: "Aligns the bounding boxes of objects. mode: left, hcenter, right, top, vcenter or bottom. relative_to: selection (the common bounding box of the objects, default) or cutting_area (the mat's grid, minus a 5 mm safety margin while the mat is not calibrated; left puts objects at the left edge of that area). Origin the 0,0 of the grid printed on the mat, y down, units mm. An empty ids list means the current selection. Returns {ids}.",
);

/// Moves `o` by `d` if it then lies inside `area`. Otherwise it stays
/// exactly where it was and the result is false.
fn place(o: &mut Object, d: Vec2, area: Rect) -> bool {
    let centre = o.transform.center_mm;
    translate(o, d);
    if placed_bounds(o).is_some_and(|nb| inside(nb, area)) {
        return true;
    }
    // Put the saved centre back: moving back by -d can leave float noise.
    o.transform.center_mm = centre;
    false
}

fn default_spacing() -> f64 {
    5.0
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutoParams {
    /// Gap between objects in mm (default 5).
    #[serde(default = "default_spacing")]
    pub spacing_mm: f64,
}

fn run_auto(s: &mut Session, p: AutoParams, _: Origin) -> Result<Value, CommandError> {
    if !(p.spacing_mm.is_finite() && p.spacing_mm >= 0.0) {
        return Err(CommandError::InvalidParams(
            "spacing_mm must be 0 or more".into(),
        ));
    }
    let area = cutting_area(s)?;
    let doc = &mut s.doc_mut().document;
    let mut items: Vec<(ObjectId, Rect)> = Vec::new();
    // Objects without geometry (for example untraced rasters) cannot be placed.
    let mut did_not_fit = Vec::new();
    for o in doc.objects.iter().filter(|o| o.visible) {
        match placed_bounds(o) {
            Some(b) => items.push((o.id, b)),
            None => did_not_fit.push(o.id),
        }
    }
    // Stable: equal heights keep document order.
    items.sort_by(|a, b| b.1.height().total_cmp(&a.1.height()));

    let mut placed = Vec::new();
    let (mut x, mut y, mut row_h) = (area.x0, area.y0, 0.0_f64);
    for (id, b) in items {
        let (w, h) = (b.width(), b.height());
        if w > area.width() + FIT_EPS || h > area.height() + FIT_EPS {
            did_not_fit.push(id);
            continue;
        }
        // Wrap when the row already holds something, even zero-height lines.
        if x + w > area.x1 + FIT_EPS && x > area.x0 {
            y += row_h + p.spacing_mm;
            x = area.x0;
            row_h = 0.0;
        }
        if y + h > area.y1 + FIT_EPS {
            did_not_fit.push(id);
            continue;
        }
        let o = object_mut(doc, id)?;
        let d = Point::new(x, y) - Point::new(b.x0, b.y0);
        // Never report an object as placed unless it really is inside.
        if !place(o, d, area) {
            did_not_fit.push(id);
            continue;
        }
        placed.push(id);
        x += w + p.spacing_mm;
        row_h = row_h.max(h);
    }
    Ok(json!({ "placed": ids_json(&placed), "did_not_fit": ids_json(&did_not_fit) }))
}

command!(
    ArrangeAuto,
    id: "arrange.auto",
    title: "Auto arrange",
    mutates: true,
    params: AutoParams,
    run: run_auto,
    description: "Packs all visible objects into the cutting area in rows (shelf packing), keeping rotation and size. Objects are sorted by bounding box height, tallest first, and placed left to right from the top-left of the cutting area, with spacing_mm (default 5) between them. Objects that do not fit, or have no cut geometry yet, keep their position and are listed in did_not_fit; they may overlap the packed objects. Origin the 0,0 of the grid printed on the mat, y down, units mm. Returns {placed: [id], did_not_fit: [id]}.",
);

#[cfg(test)]
mod tests {
    use super::*;
    use flatcraft_core::document::{Document, JobSettings, Source};
    use flatcraft_core::geometry::CutPath;

    #[test]
    fn an_object_that_does_not_fit_keeps_its_exact_centre() {
        let mut doc = Document::new(JobSettings {
            plotter_id: String::new(),
            material_id: String::new(),
            media_id: String::new(),
            speed: None,
            force: None,
            passes: None,
        });
        let path = kurbo::BezPath::from_vec(vec![
            kurbo::PathEl::MoveTo(Point::new(0.0, 0.0)),
            kurbo::PathEl::LineTo(Point::new(1.0, 1.0)),
        ]);
        let id = doc.add(
            "line".into(),
            Source::Svg {
                name: "line".into(),
                bytes: Vec::new().into(),
            },
            vec![CutPath {
                path,
                color: None,
                label: None,
                paint: Default::default(),
            }],
        );
        let o = doc.get_mut(id).unwrap();
        o.transform.center_mm = Point::new(0.1, 0.1);
        // 0.1 + 0.7 - 0.7 is not 0.1 in floating point.
        assert!(!place(
            o,
            Vec2::new(0.7, 0.7),
            Rect::new(50.0, 50.0, 60.0, 60.0)
        ));
        assert_eq!(o.transform.center_mm, Point::new(0.1, 0.1));
    }
}
