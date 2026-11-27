//! Group commands: put objects together so they act as one object
//! (`object.group`), take them apart again (`object.ungroup`), and move
//! objects into or out of a group (`object.move_to_group`, the Objects
//! panel's drag). See [`flatcraft_core::nodes`].

use flatcraft_core::document::ObjectId;
use serde::Deserialize;
use serde_json::{Value, json};

use super::object::MAX_NAME_CHARS;
use super::{check_node, command, ids_json, resolve_units};
use crate::session::{CommandError, Origin, Session};

/// Why Group cannot run on `units` (what moves as one), or `None` when it
/// can. The window dims its Group items with this text.
pub fn group_refusal(units: usize) -> Option<&'static str> {
    (units < 2).then_some("Select 2 or more objects first.")
}

/// What Ungroup does with `units`: the groups it dissolves and the objects
/// (or nested groups) it takes out of their group. The window names its
/// Ungroup item after this ("Ungroup", "Remove 2 objects from group").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UngroupPlan {
    /// Whole groups: they dissolve, their members go one level up.
    pub dissolve: Vec<ObjectId>,
    /// Things picked inside a group: they leave it for its parent.
    pub release: Vec<ObjectId>,
}

impl UngroupPlan {
    /// The plan for `units` in the group tree `tree`.
    pub fn of(tree: &flatcraft_core::nodes::Tree, units: &[ObjectId]) -> Self {
        let mut plan = Self::default();
        for u in units {
            if tree.is_group(*u) {
                plan.dissolve.push(*u);
            } else if tree.parent(*u).is_some() {
                plan.release.push(*u);
            }
        }
        plan
    }

    pub fn is_empty(&self) -> bool {
        self.dissolve.is_empty() && self.release.is_empty()
    }

    /// The name of the Ungroup item: "Ungroup" for whole groups, "Remove
    /// 2 objects from group" for objects picked inside one.
    pub fn title(&self) -> String {
        match (self.dissolve.is_empty(), self.release.len()) {
            (true, 1) => "Remove 1 object from group".into(),
            (true, n) if n > 1 => format!("Remove {n} objects from group"),
            _ => "Ungroup".into(),
        }
    }
}

/// Why Ungroup cannot run: nothing selected is a group or in one.
pub const UNGROUP_REFUSAL: &str = "Select a group first.";

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupParams {
    /// Object or group ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// The group's name; default "Group N".
    pub name: Option<String>,
}

fn clean_name(name: &str) -> Result<String, CommandError> {
    let name = name.trim().to_string();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(CommandError::InvalidParams(format!(
            "name must be one line of plain text, 1 to {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(name)
}

fn run_group(s: &mut Session, p: GroupParams, _: Origin) -> Result<Value, CommandError> {
    let (leaves, units) = resolve_units(s, &p.ids)?;
    if let Some(why) = group_refusal(units.len()) {
        return Err(CommandError::InvalidParams(format!(
            "{why} A group needs 2 or more objects or groups; a whole group counts as one."
        )));
    }
    let name = match &p.name {
        Some(n) => clean_name(n)?,
        None => s.doc().document.new_group_name(),
    };
    let st = s.doc_mut();
    let id = st.document.group(&units, name);
    st.selection = leaves.into_iter().collect();
    Ok(json!({ "group_id": id.0, "ids": ids_json(&units) }))
}

command!(
    ObjectGroup,
    id: "object.group",
    title: "Group",
    mutates: true,
    params: GroupParams,
    run: run_group,
    description: "Puts objects together in a new group, in one undo step (the window's Group, Cmd+G or Ctrl+G). A group acts as one object: it is selected, moved, scaled, turned, mirrored, aligned, distributed, packed by arrange.auto, duplicated, deleted and hidden as a whole, and the weed box per object puts one box around it. Every member keeps its own geometry, cut settings, visibility and place on the mat: grouping moves nothing, and is not the same as linked copies (object.copies). ids are objects or groups (a group id stands for the whole group); 2 or more of them are needed, a whole group counting as one. Groups can hold groups. The new group goes into the nearest group that holds all of them, or the top level. name (optional, one line) defaults to \"Group N\". Every command that takes object ids takes a group id too. An empty ids list means the current selection; the members become the selection. Returns {group_id, ids (what went into the group)}.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UngroupParams {
    /// Object or group ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
}

fn run_ungroup(s: &mut Session, p: UngroupParams, _: Origin) -> Result<Value, CommandError> {
    let (leaves, units) = resolve_units(s, &p.ids)?;
    let doc = &s.doc().document;
    let plan = UngroupPlan::of(&doc.tree(), &units);
    if plan.is_empty() {
        return Err(CommandError::InvalidParams(format!(
            "{UNGROUP_REFUSAL} None of these objects is a group or in one."
        )));
    }
    let st = s.doc_mut();
    let mut todo = plan.dissolve.clone();
    todo.extend(&plan.release);
    let moved = st.document.ungroup(&todo);
    st.selection = leaves.into_iter().collect();
    Ok(json!({
        "ids": ids_json(&moved),
        "dissolved": ids_json(&plan.dissolve),
        "released": ids_json(&plan.release),
    }))
}

command!(
    ObjectUngroup,
    id: "object.ungroup",
    title: "Ungroup",
    mutates: true,
    params: UngroupParams,
    run: run_ungroup,
    description: "Takes groups apart, in one undo step (the window's Ungroup, Shift+Cmd+G or Shift+Ctrl+G). It works on what ids name: a whole group (its id, or all of its objects) dissolves and its members go one level up, so nested groups stay groups; objects picked inside a group (some, not all of it) leave that group for its parent. A group left with fewer than 2 members dissolves too. Nothing moves on the mat. Refuses when none of the ids is a group or in one. An empty ids list means the current selection, which stays selected. Returns {ids (what moved up a level), dissolved (group ids), released (object ids)}.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveToGroupParams {
    /// Object or group ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// The group to put them in; leave out (or null) for the top level.
    pub group_id: Option<u64>,
}

fn run_move_to_group(
    s: &mut Session,
    p: MoveToGroupParams,
    _: Origin,
) -> Result<Value, CommandError> {
    let (_, units) = resolve_units(s, &p.ids)?;
    let doc = &s.doc().document;
    let target = p.group_id.map(ObjectId);
    if let Some(g) = target {
        check_node(doc, g)?;
        if doc.group_node(g).is_none() {
            return Err(CommandError::InvalidParams(format!(
                "{} is an object, not a group",
                g.0
            )));
        }
        let tree = doc.tree();
        if units.iter().any(|u| tree.within(g, *u)) {
            return Err(CommandError::InvalidParams(
                "a group cannot go into itself or into a group inside it".into(),
            ));
        }
    }
    let doc = &mut s.doc_mut().document;
    for u in &units {
        doc.set_parent(*u, target);
    }
    doc.normalize_groups();
    Ok(json!({ "ids": ids_json(&units), "group_id": target.map(|g| g.0) }))
}

command!(
    ObjectMoveToGroup,
    id: "object.move_to_group",
    title: "Move to group",
    mutates: true,
    params: MoveToGroupParams,
    run: run_move_to_group,
    description: "Puts objects or groups into the group group_id, or at the top level when group_id is left out, in one undo step (the Objects panel does this when a row is dragged in or out of a group). Nothing moves on the mat. A group cannot go into itself or a group inside it. A group left with fewer than 2 members dissolves. An empty ids list means the current selection. Returns {ids (what moved), group_id}.",
);

// ---------------------------------------------------------------- order

/// Where `object.reorder` moves objects in the stacking order (document
/// order: later objects lie in front, as the canvas draws them and as the
/// visible outline cut mode reads them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReorderTo {
    /// In front of everything.
    Front,
    /// One step forward: in front of the next object (or whole group).
    Forward,
    /// One step back: behind the object (or whole group) right behind.
    Backward,
    /// Behind everything.
    Back,
}

impl ReorderTo {
    /// Its name in `object.reorder`.
    pub fn key(self) -> &'static str {
        match self {
            ReorderTo::Front => "front",
            ReorderTo::Forward => "forward",
            ReorderTo::Backward => "backward",
            ReorderTo::Back => "back",
        }
    }

    /// Why moving this way changes nothing for a selection: it is already
    /// in front (or at the back).
    pub fn refusal(self) -> &'static str {
        match self {
            ReorderTo::Front | ReorderTo::Forward => {
                "Already in front: nothing lies in front of the selection (in its group, when it is in one)."
            }
            ReorderTo::Backward | ReorderTo::Back => {
                "Already at the back: nothing lies behind the selection (in its group, when it is in one)."
            }
        }
    }
}

/// The document order `order` after moving the objects `leaves` to `to`.
/// What moves as one is a unit of `tree` (a whole group, or an object
/// picked on its own): its objects end up next to each other, in their
/// order. A step forward or back passes one sibling at the unit's own
/// level: inside a group, the next member (or member group), and nothing
/// at the end of the group; at the top level, the next object or whole
/// group. Equal to `order`
/// when nothing can move that way.
pub fn reordered(
    order: &[ObjectId],
    tree: &flatcraft_core::nodes::Tree,
    leaves: &[ObjectId],
    to: ReorderTo,
) -> Vec<ObjectId> {
    let chosen: std::collections::BTreeSet<ObjectId> = leaves.iter().copied().collect();
    let (picked, rest): (Vec<ObjectId>, Vec<ObjectId>) =
        order.iter().partition(|id| chosen.contains(id));
    match to {
        ReorderTo::Front => return rest.into_iter().chain(picked).collect(),
        ReorderTo::Back => return picked.into_iter().chain(rest).collect(),
        ReorderTo::Forward | ReorderTo::Backward => {}
    }
    let forward = to == ReorderTo::Forward;
    let mut order = order.to_vec();
    let at = |order: &[ObjectId], id: ObjectId| order.iter().position(|x| *x == id);
    // (the unit, its objects that move)
    let mut blocks: Vec<(ObjectId, Vec<ObjectId>)> = tree
        .units(leaves)
        .into_iter()
        .map(|u| {
            let objects = tree
                .leaves_of(u)
                .into_iter()
                .filter(|l| chosen.contains(l))
                .collect::<Vec<_>>();
            (u, objects)
        })
        .filter(|(_, b)| !b.is_empty())
        .collect();
    // The block nearest the way it moves goes first, so blocks keep
    // their order among themselves.
    let edge = |order: &[ObjectId], b: &[ObjectId]| {
        let p = b.iter().filter_map(|id| at(order, *id));
        if forward { p.max() } else { p.min() }
    };
    blocks.sort_by_key(|(_, b)| edge(&order, b));
    if forward {
        blocks.reverse();
    }
    for (unit, block) in blocks {
        let Some(end) = edge(&order, &block) else {
            continue;
        };
        // Inside a group the unit steps among its siblings only: at the
        // end of its group it stays.
        let scope = tree.parent(unit);
        let sibling =
            |o: &&ObjectId| !chosen.contains(*o) && scope.is_none_or(|g| tree.within(**o, g));
        let next = if forward {
            order[end + 1..].iter().find(sibling)
        } else {
            order[..end].iter().rev().find(sibling)
        };
        let Some(&next) = next else {
            continue;
        };
        // What is passed is the sibling at the moving unit's own level
        // that holds `next`: the child of their nearest common group (a
        // sibling object, or a sibling group as a whole), without what
        // moves.
        let around: std::collections::BTreeSet<ObjectId> =
            tree.ancestors(unit).into_iter().collect();
        let mut passed = next;
        for g in tree.ancestors(next) {
            if around.contains(&g) {
                break;
            }
            passed = g;
        }
        let other: Vec<ObjectId> = tree
            .leaves_of(passed)
            .into_iter()
            .filter(|l| !chosen.contains(l))
            .collect();
        let mut block_sorted = block.clone();
        block_sorted.sort_by_key(|id| at(&order, *id));
        order.retain(|id| !block.contains(id));
        let positions = other.iter().filter_map(|id| at(&order, *id));
        let insert = if forward {
            positions.max().map_or(order.len(), |p| p + 1)
        } else {
            positions.min().unwrap_or(0)
        };
        for (k, id) in block_sorted.into_iter().enumerate() {
            order.insert(insert + k, id);
        }
    }
    order
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReorderParams {
    /// Object or group ids; empty means the current selection.
    #[serde(default)]
    pub ids: Vec<u64>,
    /// front, forward (one step), backward (one step) or back.
    pub to: ReorderTo,
}

fn run_reorder(s: &mut Session, p: ReorderParams, _: Origin) -> Result<Value, CommandError> {
    let leaves = super::resolve_ids(s, &p.ids)?;
    let doc = &s.doc().document;
    let order: Vec<ObjectId> = doc.objects.iter().map(|o| o.id).collect();
    let new = reordered(&order, &doc.tree(), &leaves, p.to);
    let changed = new != order;
    if changed {
        let doc = &mut s.doc_mut().document;
        let place: std::collections::BTreeMap<ObjectId, usize> =
            new.iter().enumerate().map(|(k, id)| (*id, k)).collect();
        doc.objects
            .sort_by_key(|o| place.get(&o.id).copied().unwrap_or(usize::MAX));
    }
    Ok(json!({ "ids": ids_json(&leaves), "order": ids_json(&new), "changed": changed }))
}

command!(
    ObjectReorder,
    id: "object.reorder",
    title: "Arrange order",
    mutates: true,
    params: ReorderParams,
    run: run_reorder,
    description: "Changes which objects lie in front, in one undo step (the window's Bring to front Shift+Cmd+], Bring forward Cmd+], Send backward Cmd+[ and Send to back Shift+Cmd+[, Ctrl on Windows and Linux). The stacking order is the document order (document.state lists objects in it): later objects lie in front. It decides what the visible_outline cut mode covers, and how the canvas draws overlaps. to: front puts them in front of everything, back behind everything, forward one step in front of the next sibling, backward one step behind the sibling right behind; a sibling is an object or a whole group at the same level, so a group is passed as one block. A whole group moves as one block; its objects end up next to each other. Objects picked inside a group move on their own and step only among the members of their group (at the end of the group they stay). Nothing moves on the mat. Already in front (or at the back) changes nothing and records no undo step. An empty ids list means the current selection. Returns {ids (the objects moved), order (every object id in the new order, back to front), changed}.",
);
