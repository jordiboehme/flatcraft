//! Undo and redo.

use serde_json::{Value, json};

use super::{NoParams, command};
use crate::session::{CommandError, Origin, Session};

fn run_undo(s: &mut Session, _: NoParams, _: Origin) -> Result<Value, CommandError> {
    let st = s.doc_mut();
    let current = st.document.clone();
    let previous = st
        .undo
        .undo(current)
        .ok_or_else(|| CommandError::Refused("there is nothing to undo".into()))?;
    st.document = previous;
    st.dirty = true;
    st.prune_selection();
    Ok(json!({ "can_undo": st.undo.can_undo(), "can_redo": st.undo.can_redo() }))
}

command!(
    EditUndo,
    id: "edit.undo",
    title: "Undo",
    mutates: false,
    params: NoParams,
    run: run_undo,
    description: "Undoes the last change to the active document (every object, arrange and job command is one step). Refused when there is nothing to undo. Returns {can_undo, can_redo}.",
);

fn run_redo(s: &mut Session, _: NoParams, _: Origin) -> Result<Value, CommandError> {
    let st = s.doc_mut();
    let current = st.document.clone();
    let next = st
        .undo
        .redo(current)
        .ok_or_else(|| CommandError::Refused("there is nothing to redo".into()))?;
    st.document = next;
    st.dirty = true;
    st.prune_selection();
    Ok(json!({ "can_undo": st.undo.can_undo(), "can_redo": st.undo.can_redo() }))
}

command!(
    EditRedo,
    id: "edit.redo",
    title: "Redo",
    mutates: false,
    params: NoParams,
    run: run_redo,
    description: "Redoes the last undone change of the active document. Refused when there is nothing to redo. Returns {can_undo, can_redo}.",
);
