//! Project notes: a one-line description and plain-text notes that the
//! person keeps with a project. Agents read them and may add to the notes,
//! but never replace them.

use serde::Deserialize;
use serde_json::{Value, json};

use super::{NoParams, command};
use crate::session::{CommandError, Origin, Session};

/// The longest notes, in characters. Far below the size a project file may
/// have, so a project with long notes always opens again.
pub const MAX_NOTES_CHARS: usize = 20_000;

/// The longest description, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 200;

/// Why an agent cannot use `document.set_notes`.
pub const AGENTS_APPEND_ONLY: &str = "agents cannot replace the notes or the description; use document.append_notes to add a line to the notes";

fn notes_json(s: &Session) -> Value {
    let d = &s.doc().document;
    json!({ "description": d.description, "notes": d.notes })
}

/// The description as one line: line breaks become spaces.
fn one_line(text: &str) -> String {
    text.split(['\r', '\n'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn check_len(what: &str, text: &str, max: usize) -> Result<(), CommandError> {
    if text.chars().count() > max {
        return Err(CommandError::InvalidParams(format!(
            "the {what} may be at most {max} characters long"
        )));
    }
    Ok(())
}

fn run_notes(s: &mut Session, _: NoParams, _: Origin) -> Result<Value, CommandError> {
    Ok(notes_json(s))
}

command!(
    DocumentNotes,
    id: "document.notes",
    title: "Project notes",
    mutates: false,
    params: NoParams,
    run: run_notes,
    description: "Reads the notes the person keeps with the active document: what the project is and what to remember when cutting it (which mat, which side up, a force that worked). Returns {description (one line, empty when none), notes (plain text, empty when none)}. Use document.append_notes to add a line; agents cannot replace or delete notes.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppendParams {
    /// The text to add at the end of the notes, on a new line. Not empty.
    pub text: String,
}

fn run_append(s: &mut Session, p: AppendParams, _: Origin) -> Result<Value, CommandError> {
    let text = p.text.trim_end();
    if text.trim().is_empty() {
        return Err(CommandError::InvalidParams(
            "text is empty; give the line to add".into(),
        ));
    }
    let notes = &s.doc().document.notes;
    let joined = if notes.is_empty() {
        text.to_string()
    } else if notes.ends_with('\n') {
        format!("{notes}{text}")
    } else {
        format!("{notes}\n{text}")
    };
    check_len("notes", &joined, MAX_NOTES_CHARS)?;
    s.doc_mut().document.notes = joined;
    Ok(notes_json(s))
}

command!(
    DocumentAppendNotes,
    id: "document.append_notes",
    title: "Add to project notes",
    mutates: true,
    params: AppendParams,
    run: run_append,
    description: "Adds text at the end of the active document's notes, on a new line, and keeps everything that was there. One undo step; the document then has unsaved changes. The notes may be at most 20000 characters. Returns {description, notes} as they are now.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetParams {
    /// The new one-line description; line breaks become spaces. Missing: unchanged.
    pub description: Option<String>,
    /// The new notes. Missing: unchanged.
    pub notes: Option<String>,
    /// Edits with the same key in a row, with no other change in between, are one undo step (typing).
    pub coalesce_key: Option<String>,
}

fn run_set(s: &mut Session, p: SetParams, origin: Origin) -> Result<Value, CommandError> {
    if origin == Origin::Agent {
        return Err(CommandError::Refused(AGENTS_APPEND_ONLY.into()));
    }
    let description = p.description.as_deref().map(one_line);
    if let Some(d) = &description {
        check_len("description", d, MAX_DESCRIPTION_CHARS)?;
    }
    if let Some(n) = &p.notes {
        check_len("notes", n, MAX_NOTES_CHARS)?;
    }
    if let Some(key) = p.coalesce_key {
        s.coalesce_with(format!("notes:{key}"), Vec::new());
    }
    let doc = &mut s.doc_mut().document;
    if let Some(d) = description {
        doc.description = d;
    }
    if let Some(n) = p.notes {
        doc.notes = n;
    }
    Ok(notes_json(s))
}

command!(
    DocumentSetNotes,
    id: "document.set_notes",
    title: "Edit project notes",
    mutates: true,
    params: SetParams,
    run: run_set,
    description: "Replaces the description and/or the notes of the active document, as the person types them in the window. Agents are refused: they use document.append_notes, which never removes text. Returns {description, notes}.",
);
