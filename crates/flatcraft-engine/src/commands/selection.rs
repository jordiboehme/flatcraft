//! Selection commands that pick by place, like the canvas does.

use flatcraft_core::document::ObjectId;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{command, ids_json, knife_of};
use crate::lasso::{self, DocumentOutlines, Lasso, Mode};
use crate::session::{CommandError, Origin, Session};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetInPolygonParams {
    /// The corners of the shape in mat mm, as [x, y] pairs, 3 or more. The
    /// last one joins the first.
    pub points_mm: Vec<[f64; 2]>,
    /// False (the default): only objects fully inside are picked, like the
    /// selection box. True: every object the shape touches or encloses,
    /// like the lasso.
    #[serde(default)]
    pub touch: bool,
    /// What happens to the selection there is: replace (the default), add,
    /// remove, or toggle (selected objects leave, the others join, as
    /// Shift does on the canvas).
    #[serde(default)]
    pub mode: Mode,
    /// Pick inside this group (as after a double-click on the canvas): its
    /// members are picked one by one, nothing outside it. Without it, a
    /// group is picked as one piece.
    #[serde(default)]
    pub group: Option<u64>,
}

fn run_set_in_polygon(
    s: &mut Session,
    p: SetInPolygonParams,
    _: Origin,
) -> Result<Value, CommandError> {
    let shape = Lasso::new(p.points_mm.iter().map(|[x, y]| kurbo::Point::new(*x, *y))).ok_or_else(
        || {
            CommandError::InvalidParams(
                "points_mm needs 3 or more points (numbers) that enclose an area".into(),
            )
        },
    )?;
    let knife = knife_of(s);
    let doc = &s.doc().document;
    let tree = doc.tree();
    let within = match p.group {
        Some(g) if tree.is_group(ObjectId(g)) => Some(ObjectId(g)),
        Some(g) => {
            return Err(CommandError::InvalidParams(format!(
                "group {g} is not a group of this document"
            )));
        }
        None => None,
    };
    let picked = lasso::pick(
        &tree,
        &DocumentOutlines::new(doc, knife),
        &shape,
        p.touch,
        within,
    );
    let current: Vec<ObjectId> = s.doc().selection.iter().copied().collect();
    let next = lasso::combine(&current, &picked.units, p.mode);
    let st = s.doc_mut();
    st.selection = next.iter().copied().collect();
    let sel: Vec<ObjectId> = st.selection.iter().copied().collect();
    Ok(json!({ "selection": ids_json(&sel), "picked": ids_json(&picked.ids) }))
}

command!(
    SelectionSetInPolygon,
    id: "selection.set_in_polygon",
    title: "Select in a shape",
    mutates: false,
    session: true,
    params: SetInPolygonParams,
    run: run_set_in_polygon,
    description: "Selects the objects in a free shape on the mat, as the canvas selection box and lasso do. points_mm are the shape's corners in mat mm ([x, y], 3 or more, the last joins the first). Without touch only objects whose outline lies fully inside are picked (the box); with touch true every object the shape touches or encloses (the lasso). A group is picked as one piece: fully inside when all its shown members are, touched when one is. With group (a group id) its members are picked one by one, as inside an entered group on the canvas. Hidden objects are never picked. mode: replace (default), add, remove, or toggle (selected ones leave, the others join). It changes no document and adds no undo step. Returns {selection: [id], picked: [id]}.",
);
