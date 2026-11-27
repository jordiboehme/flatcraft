//! Arrange commands: align, distribute and Auto arrange (packing).

use flatcraft_core::document::{Object, ObjectId};
use kurbo::{Rect, Vec2};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    arrange_bounds, command, common_box, cutting_area, ids_json, inside, knife_of, object_mut,
    resolve_ids, translate, unit_leaves, units_of,
};
use crate::placement;
use crate::session::{CommandError, Origin, Session};

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
    let mw = knife_of(s);
    let target = match p.relative_to {
        AlignTarget::CuttingArea => cutting_area(s)?,
        AlignTarget::Selection => common_box(&s.doc().document, &ids, mw).unwrap_or(Rect::ZERO),
    };
    // A whole group lines up as one box; its members keep their places.
    let units = units_of(&s.doc().document, &ids);
    let doc = &mut s.doc_mut().document;
    for unit in &units {
        let leaves = unit_leaves(doc, *unit);
        let b = common_box(doc, &leaves, mw).unwrap_or(Rect::ZERO);
        let d = match p.mode {
            AlignMode::Left => Vec2::new(target.x0 - b.x0, 0.0),
            AlignMode::Hcenter => Vec2::new(target.center().x - b.center().x, 0.0),
            AlignMode::Right => Vec2::new(target.x1 - b.x1, 0.0),
            AlignMode::Top => Vec2::new(0.0, target.y0 - b.y0),
            AlignMode::Vcenter => Vec2::new(0.0, target.center().y - b.center().y),
            AlignMode::Bottom => Vec2::new(0.0, target.y1 - b.y1),
        };
        for id in &leaves {
            translate(object_mut(doc, *id)?, d);
        }
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
    description: "Aligns the bounding boxes of objects; a whole group lines up as one box and its members keep their places in it. mode: left, hcenter, right, top, vcenter or bottom. relative_to: selection (the common bounding box of the objects, default) or cutting_area (the mat's grid, minus a 5 mm safety margin while the mat is not calibrated; left puts objects at the left edge of that area). Origin the 0,0 of the grid printed on the mat, y down, units mm. An empty ids list means the current selection. Returns {ids}.",
);

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DistributeAxis {
    /// Left to right, along x.
    Horizontal,
    /// Top to bottom, along y (down).
    Vertical,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DistributeParams {
    /// Object ids, at least 3; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// The direction to spread the objects in.
    pub axis: DistributeAxis,
}

/// The start and end of `r` on `axis`.
fn span(r: Rect, axis: DistributeAxis) -> (f64, f64) {
    match axis {
        DistributeAxis::Horizontal => (r.x0, r.x1),
        DistributeAxis::Vertical => (r.y0, r.y1),
    }
}

fn run_distribute(s: &mut Session, p: DistributeParams, _: Origin) -> Result<Value, CommandError> {
    let from_selection = p.ids.is_empty();
    let leaves = resolve_ids(s, &p.ids)?;
    // A whole group is one item with its common box.
    let ids = units_of(&s.doc().document, &leaves);
    if ids.len() < 3 {
        let have = if from_selection {
            format!("{} are selected", ids.len())
        } else {
            format!("got {}", ids.len())
        };
        return Err(CommandError::InvalidParams(format!(
            "Distribute needs at least 3 objects, {have}."
        )));
    }
    let mw = knife_of(s);
    let doc = &s.doc().document;
    let mut items: Vec<(ObjectId, f64, f64)> = Vec::with_capacity(ids.len());
    for id in &ids {
        let b = common_box(doc, &unit_leaves(doc, *id), mw)
            .ok_or_else(|| CommandError::NotFound(format!("no object with id {}", id.0)))?;
        let (lo, hi) = span(b, p.axis);
        items.push((*id, lo, hi));
    }
    // Stable: equal centres keep the given order.
    items.sort_by(|a, b| (a.1 + a.2).total_cmp(&(b.1 + b.2)));
    let n = items.len();
    let (start, end) = (items[0].1, items[n - 1].2);
    let total: f64 = items.iter().map(|(_, lo, hi)| hi - lo).sum();
    let gap = (end - start - total) / (n - 1) as f64;
    let doc = &mut s.doc_mut().document;
    // Each start from the fixed first object, never from the previous
    // move, so rounding does not add up. The ends do not move at all.
    let mut before = 0.0;
    for (k, (id, lo, hi)) in items.iter().enumerate() {
        if k > 0 && k < n - 1 {
            let target = start + before + k as f64 * gap;
            let d = target - lo;
            let d = match p.axis {
                DistributeAxis::Horizontal => Vec2::new(d, 0.0),
                DistributeAxis::Vertical => Vec2::new(0.0, d),
            };
            for leaf in unit_leaves(doc, *id) {
                translate(object_mut(doc, leaf)?, d);
            }
        }
        before += hi - lo;
    }
    let order: Vec<ObjectId> = items.iter().map(|(id, _, _)| *id).collect();
    Ok(json!({ "ids": ids_json(&order), "gap_mm": gap }))
}

command!(
    ArrangeDistribute,
    id: "arrange.distribute",
    title: "Distribute",
    mutates: true,
    params: DistributeParams,
    run: run_distribute,
    description: "Spreads 3 or more objects (a whole group counts as one, with its common box) so the gaps between neighbouring bounding boxes are equal. axis: horizontal (left to right, along x) or vertical (top to bottom, along y down). The objects are ordered by the centre of their bounding box on that axis; equal centres keep the order of ids (for the selection, that is id order). The first and the last object stay where they are, the others move only along the axis. If the objects together are longer than the span, the gaps are negative and they overlap evenly. Units mm, origin the 0,0 of the grid printed on the mat. An empty ids list means the current selection. Refuses fewer than 3 objects. Returns {ids (in the new order), gap_mm}.",
);

/// Moves `o` by `d` if it then lies inside `area`. Otherwise it stays
/// exactly where it was and the result is false.
fn place(o: &mut Object, d: Vec2, area: Rect, mw: crate::cutgeom::Knife) -> bool {
    let centre = o.transform.center_mm;
    translate(o, d);
    if arrange_bounds(o, mw).is_some_and(|nb| inside(nb, area)) {
        return true;
    }
    // Put the saved centre back: moving back by -d can leave float noise.
    o.transform.center_mm = centre;
    false
}

/// [`place`] for a unit: a whole group moves by `d` if its box then lies
/// inside `area` (every member's box), else every member stays exactly
/// where it was.
fn place_unit(
    doc: &mut flatcraft_core::document::Document,
    unit: ObjectId,
    d: Vec2,
    area: Rect,
    mw: crate::cutgeom::Knife,
) -> Result<bool, CommandError> {
    if doc.group_node(unit).is_none() {
        return Ok(place(object_mut(doc, unit)?, d, area, mw));
    }
    let leaves = unit_leaves(doc, unit);
    let saved: Vec<kurbo::Point> = leaves
        .iter()
        .filter_map(|id| doc.get(*id))
        .map(|o| o.transform.center_mm)
        .collect();
    let mut fits = true;
    for id in &leaves {
        let o = object_mut(doc, *id)?;
        translate(o, d);
        if o.visible && arrange_bounds(o, mw).is_some_and(|nb| !inside(nb, area)) {
            fits = false;
        }
    }
    if !fits {
        for (id, c) in leaves.iter().zip(saved) {
            object_mut(doc, *id)?.transform.center_mm = c;
        }
    }
    Ok(fits)
}

fn default_spacing() -> f64 {
    5.0
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutoParams {
    /// The objects to pack; empty means the current selection, and with
    /// nothing selected every visible object.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// Gap between objects in mm (default 5), 0 to 100.
    #[serde(default = "default_spacing")]
    #[schemars(range(min = 0, max = 100))]
    pub spacing_mm: f64,
}

fn run_auto(s: &mut Session, p: AutoParams, _: Origin) -> Result<Value, CommandError> {
    // The range of spacing_mm is the schema's (checked by the registry).
    let area = cutting_area(s)?;
    // Empty: the selection, and with nothing selected every visible object.
    let chosen = if p.ids.is_empty() {
        s.doc().selection.iter().copied().collect()
    } else {
        resolve_ids(s, &p.ids)?
    };
    let mw = knife_of(s);
    let doc = &mut s.doc_mut().document;
    // A whole group packs as one item, a member picked inside a group
    // alone (see `placement::auto_keys` and `placement::auto_units`).
    let order: Vec<ObjectId> = doc.objects.iter().map(|o| o.id).collect();
    let (keys, chosen_units) = placement::auto_keys(&doc.tree(), &order, &chosen);
    let leaves: Vec<(ObjectId, placement::AutoObject<ObjectId>)> = doc
        .objects
        .iter()
        .zip(keys)
        .map(|(o, key)| {
            let item = placement::AutoObject {
                key: o.id,
                visible: o.visible,
                // An untraced picture by its image; nothing at all (an
                // empty object without a frame) cannot be placed.
                bounds: arrange_bounds(o, mw),
            };
            (key, item)
        })
        .collect();
    let objects = placement::auto_units(&leaves);
    let plan = placement::auto_arrange(&objects, &chosen_units, area, p.spacing_mm);
    let mut did_not_fit = plan.did_not_fit;
    let mut placed = Vec::new();
    for (id, d) in plan.moves {
        // Never report an object as placed unless it really is inside.
        if place_unit(doc, id, d, area, mw)? {
            placed.push(id);
        } else {
            did_not_fit.push(id);
        }
    }
    Ok(json!({
        "placed": ids_json(&placed),
        "did_not_fit": ids_json(&did_not_fit),
        "fixed": ids_json(&plan.fixed),
        "hidden": ids_json(&plan.hidden),
    }))
}

command!(
    ArrangeAuto,
    id: "arrange.auto",
    title: "Auto arrange",
    mutates: true,
    params: AutoParams,
    run: run_auto,
    description: "Packs objects into the cutting area, keeping rotation and size, in one undo step. ids are the objects to pack; an empty list means the current selection, and with nothing selected every visible object. Hidden objects are never packed and never obstacles; hidden objects among the ones to pack stay where they are and are listed in hidden. A picture that is not traced yet counts by its image. A whole group packs as one item with the common box of its visible members, and its members keep their places in it; the group's id is listed in the results. Members picked inside a group (some, not all of it) pack on their own, and the rest of their group stays where it is as one obstacle. Groups that are not chosen are obstacles as a whole. Every other visible object stays exactly where it is and is an obstacle: the packed objects fit around it, spacing_mm (default 5) away from it, and never overlap it. Packing works on bounding boxes: tallest first (equal heights in id order), each one at the free spot nearest the top of the cutting area, then nearest its left, with spacing_mm between packed objects. The same input always gives the same result. Objects that do not fit, or have nothing to place at all, keep their position and are listed in did_not_fit; they may overlap other objects. Origin the 0,0 of the grid printed on the mat, y down, units mm. Returns {placed: [id], did_not_fit: [id], fixed: [id] (the obstacles), hidden: [id]}.",
);

#[cfg(test)]
mod tests {
    use super::*;
    use flatcraft_core::document::{Document, JobSettings, Source};
    use flatcraft_core::geometry::CutPath;
    use kurbo::Point;

    #[test]
    fn an_object_that_does_not_fit_keeps_its_exact_centre() {
        let mut doc = Document::new(JobSettings {
            plotter_id: String::new(),
            material_id: String::new(),
            media_id: String::new(),
            speed: None,
            force: None,
            passes: None,
            strategy: Default::default(),
            weed: Default::default(),
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
            Rect::new(50.0, 50.0, 60.0, 60.0),
            Default::default()
        ));
        assert_eq!(o.transform.center_mm, Point::new(0.1, 0.1));
    }
}
