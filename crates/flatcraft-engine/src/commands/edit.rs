//! Undo and redo.

use serde::Deserialize;
use serde_json::{Value, json};

use super::command;
use crate::session::{CommandError, DocState, Origin, Session};

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryParams {
    /// Stable id of the open document (see document.state). Default: the active document.
    pub doc_id: Option<u64>,
}

fn history_target(s: &mut Session, doc_id: Option<u64>) -> Result<&mut DocState, CommandError> {
    let index = match doc_id {
        Some(id) => s.doc_index(id)?,
        None => s.active,
    };
    Ok(&mut s.docs[index])
}

fn run_undo(s: &mut Session, p: HistoryParams, _: Origin) -> Result<Value, CommandError> {
    let st = history_target(s, p.doc_id)?;
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
    params: HistoryParams,
    run: run_undo,
    description: "Undoes the last change to the active document, or to the one with doc_id (every object, arrange and job command is one step). Refused when there is nothing to undo. Returns {can_undo, can_redo}.",
);

fn run_redo(s: &mut Session, p: HistoryParams, _: Origin) -> Result<Value, CommandError> {
    let st = history_target(s, p.doc_id)?;
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
    params: HistoryParams,
    run: run_redo,
    description: "Redoes the last undone change of the active document, or of the one with doc_id. Refused when there is nothing to redo. Returns {can_undo, can_redo}.",
);
