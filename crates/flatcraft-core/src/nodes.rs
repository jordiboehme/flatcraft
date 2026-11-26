//! Groups of objects (Group and Ungroup, Cmd+G): a tree of named group
//! nodes over the document's objects.
//!
//! Not the same as linked copies ([`crate::document::Object::link_group`],
//! one shared source) or a merged outline (one geometry): a group only
//! says "these objects belong together", so selection, moves, arrange and
//! the weed box treat them as one. Every member keeps its own geometry,
//! settings and placement.
//!
//! Members keep their placement on the mat (their transform is not relative
//! to the group), so grouping and ungrouping never move a single point. A
//! change of the group (move, scale, rotate, mirror) changes each member.
//! The group itself keeps only what is its own: its id, its name, its parent
//! and the angle it was turned by as a whole ([`GroupNode::rotation_deg`]).
//!
//! Ids: a group's id comes from the same counter as object ids
//! (`Document::next_id`), so one id never names both an object and a group,
//! and every command that takes object ids can take a group id too.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::document::ObjectId;

/// A group: a named node whose children are objects and other groups.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupNode {
    pub id: ObjectId,
    pub name: String,
    /// The group this group is in; `None` at the top level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ObjectId>,
    /// The angle the group was turned by as a whole, in [0, 360), degrees
    /// clockwise: the Angle field shows it for a selected group. The
    /// members carry the turn themselves; this is only the group's own
    /// reading of it. 0 for a new group.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation_deg: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// The group tree of a document, as a read-only view: the groups and, for
/// every object in document order, the group it is in. The engine and the
/// window both ask it which objects move as one ([`Tree::units`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tree {
    groups: Vec<GroupNode>,
    /// Every object in document order with its parent group.
    leaves: Vec<(ObjectId, Option<ObjectId>)>,
}

impl Tree {
    /// A tree of `groups` over `leaves` (objects in document order with
    /// their parent).
    pub fn new(groups: Vec<GroupNode>, leaves: Vec<(ObjectId, Option<ObjectId>)>) -> Self {
        Self { groups, leaves }
    }

    pub fn groups(&self) -> &[GroupNode] {
        &self.groups
    }

    /// True when there are no groups.
    pub fn is_flat(&self) -> bool {
        self.groups.is_empty()
    }

    pub fn group(&self, id: ObjectId) -> Option<&GroupNode> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn is_group(&self, id: ObjectId) -> bool {
        self.group(id).is_some()
    }

    pub fn is_leaf(&self, id: ObjectId) -> bool {
        self.leaves.iter().any(|(l, _)| *l == id)
    }

    /// The group `id` (an object or a group) is in.
    pub fn parent(&self, id: ObjectId) -> Option<ObjectId> {
        if let Some((_, p)) = self.leaves.iter().find(|(l, _)| *l == id) {
            return *p;
        }
        self.group(id).and_then(|g| g.parent)
    }

    /// The groups `id` is in, the nearest first.
    pub fn ancestors(&self, id: ObjectId) -> Vec<ObjectId> {
        let mut out = Vec::new();
        let mut at = self.parent(id);
        while let Some(g) = at {
            // A loop (a broken file) ends the walk.
            if out.contains(&g) {
                break;
            }
            out.push(g);
            at = self.parent(g);
        }
        out
    }

    /// The outermost group `id` is in, or `id` itself at the top level.
    pub fn top(&self, id: ObjectId) -> ObjectId {
        self.ancestors(id).last().copied().unwrap_or(id)
    }

    /// True when `id` is `group` or lies in it, at any depth.
    pub fn within(&self, id: ObjectId, group: ObjectId) -> bool {
        id == group || self.ancestors(id).contains(&group)
    }

    /// How many groups `id` is in.
    pub fn depth(&self, id: ObjectId) -> usize {
        self.ancestors(id).len()
    }

    /// The objects of `id` in document order: the object itself, or every
    /// object in the group at any depth. Empty for an unknown id.
    pub fn leaves_of(&self, id: ObjectId) -> Vec<ObjectId> {
        if self.is_leaf(id) {
            return vec![id];
        }
        if !self.is_group(id) {
            return Vec::new();
        }
        self.leaves
            .iter()
            .map(|(l, _)| *l)
            .filter(|l| self.ancestors(*l).contains(&id))
            .collect()
    }

    /// The children of `group` (or of the top level for `None`): objects
    /// and groups, in the order of their first object in the document.
    /// Groups without objects come last.
    pub fn children(&self, group: Option<ObjectId>) -> Vec<ObjectId> {
        let mut out = Vec::new();
        for (leaf, _) in &self.leaves {
            // The node on the path from `leaf` up that sits right in `group`.
            let up = self.ancestors(*leaf);
            let mut path = vec![*leaf];
            path.extend(&up);
            let node = match group {
                None => path.last().copied(),
                Some(g) => up.iter().position(|a| *a == g).map(|k| path[k]),
            };
            if let Some(node) = node
                && !out.contains(&node)
            {
                out.push(node);
            }
        }
        for g in &self.groups {
            if g.parent == group && !out.contains(&g.id) {
                out.push(g.id);
            }
        }
        out
    }

    /// The objects of `ids` (objects or groups), each once, in document
    /// order. Unknown ids are skipped.
    pub fn expand(&self, ids: &[ObjectId]) -> Vec<ObjectId> {
        let want: BTreeSet<ObjectId> = ids.iter().flat_map(|id| self.leaves_of(*id)).collect();
        self.leaves
            .iter()
            .map(|(l, _)| *l)
            .filter(|l| want.contains(l))
            .collect()
    }

    /// What moves as one for the objects `leaves`: each object goes up to
    /// the outermost group whose objects are all in `leaves`, or stays
    /// itself when its own group is only partly there (members picked
    /// inside a group). In document order of their first object, each
    /// once.
    pub fn units(&self, leaves: &[ObjectId]) -> Vec<ObjectId> {
        let set: BTreeSet<ObjectId> = leaves.iter().copied().collect();
        let mut full: BTreeMap<ObjectId, bool> = BTreeMap::new();
        let mut out = Vec::new();
        for (leaf, _) in &self.leaves {
            if !set.contains(leaf) {
                continue;
            }
            let mut unit = *leaf;
            for g in self.ancestors(*leaf) {
                let all = *full
                    .entry(g)
                    .or_insert_with(|| self.leaves_of(g).iter().all(|l| set.contains(l)));
                if !all {
                    break;
                }
                unit = g;
            }
            if !out.contains(&unit) {
                out.push(unit);
            }
        }
        out
    }

    /// The top-level node of every object (its outermost group, or itself).
    pub fn top_of_each(&self) -> BTreeMap<ObjectId, ObjectId> {
        self.leaves
            .iter()
            .map(|(l, _)| (*l, self.top(*l)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(v: u64) -> ObjectId {
        ObjectId(v)
    }

    /// 1, 2 in group 10; 3 and group 10 in group 11; 4 alone.
    fn nested() -> Tree {
        Tree::new(
            vec![
                GroupNode {
                    id: id(10),
                    name: "Inner".into(),
                    parent: Some(id(11)),
                    rotation_deg: 0.0,
                },
                GroupNode {
                    id: id(11),
                    name: "Outer".into(),
                    parent: None,
                    rotation_deg: 0.0,
                },
            ],
            vec![
                (id(1), Some(id(10))),
                (id(2), Some(id(10))),
                (id(3), Some(id(11))),
                (id(4), None),
            ],
        )
    }

    #[test]
    fn leaves_children_and_tops() {
        let t = nested();
        assert_eq!(t.leaves_of(id(11)), vec![id(1), id(2), id(3)]);
        assert_eq!(t.leaves_of(id(10)), vec![id(1), id(2)]);
        assert_eq!(t.children(None), vec![id(11), id(4)]);
        assert_eq!(t.children(Some(id(11))), vec![id(10), id(3)]);
        assert_eq!(t.children(Some(id(10))), vec![id(1), id(2)]);
        assert_eq!(t.top(id(1)), id(11));
        assert_eq!(t.top(id(4)), id(4));
        assert_eq!(t.depth(id(2)), 2);
        assert!(t.within(id(1), id(11)));
        assert!(!t.within(id(4), id(11)));
    }

    #[test]
    fn units_climb_only_through_whole_groups() {
        let t = nested();
        // The whole outer group is one unit.
        assert_eq!(t.units(&[id(1), id(2), id(3)]), vec![id(11)]);
        // The inner group whole, the outer only partly: the inner group.
        assert_eq!(t.units(&[id(1), id(2)]), vec![id(10)]);
        // One member inside a group stays itself.
        assert_eq!(t.units(&[id(1)]), vec![id(1)]);
        assert_eq!(t.units(&[id(1), id(4)]), vec![id(1), id(4)]);
        assert_eq!(t.expand(&[id(10), id(4)]), vec![id(1), id(2), id(4)]);
    }

    #[test]
    fn a_loop_in_a_broken_file_ends_the_walk() {
        let t = Tree::new(
            vec![
                GroupNode {
                    id: id(10),
                    name: "A".into(),
                    parent: Some(id(11)),
                    rotation_deg: 0.0,
                },
                GroupNode {
                    id: id(11),
                    name: "B".into(),
                    parent: Some(id(10)),
                    rotation_deg: 0.0,
                },
            ],
            vec![(id(1), Some(id(10)))],
        );
        assert_eq!(t.ancestors(id(1)), vec![id(10), id(11)]);
    }
}
