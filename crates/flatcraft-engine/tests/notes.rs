//! Project notes: the description and the notes of a document, through the
//! command registry.

use flatcraft_device::ProfileStore;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{CommandError, Origin, Session};
use serde_json::{Value, json};

fn session(dir: &tempfile::TempDir) -> Session {
    let profiles = ProfileStore::load(dir.path()).unwrap();
    let prefs = Preferences {
        output_dir: Some(dir.path().join("output")),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, false, dir.path().to_path_buf())
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

#[test]
fn setting_notes_is_one_undo_step_and_marks_the_document_unsaved() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    assert!(!s.doc().dirty);
    run(
        &mut s,
        "document.set_notes",
        json!({ "description": "Gift tags", "notes": "Kraft paper, force 14." }),
    );
    assert_eq!(s.doc().document.description, "Gift tags");
    assert_eq!(s.doc().document.notes, "Kraft paper, force 14.");
    assert!(s.doc().dirty);
    assert_eq!(s.doc().undo.undo_len(), 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().document.description, "");
    assert_eq!(s.doc().document.notes, "");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc().document.notes, "Kraft paper, force 14.");
}

#[test]
fn a_typing_burst_is_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    for text in ["K", "Kr", "Kra", "Kraft"] {
        run(
            &mut s,
            "document.set_notes",
            json!({ "notes": text, "coalesce_key": "notes-1" }),
        );
    }
    assert_eq!(s.doc().undo.undo_len(), 1);
    run(
        &mut s,
        "document.set_notes",
        json!({ "notes": "Kraft!", "coalesce_key": "notes-2" }),
    );
    assert_eq!(s.doc().undo.undo_len(), 2);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().document.notes, "Kraft");
}

#[test]
fn the_same_text_again_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(&mut s, "document.set_notes", json!({ "notes": "same" }));
    run(
        &mut s,
        "project.save",
        json!({ "path": dir.path().join("p.flatcraft") }),
    );
    assert!(!s.doc().dirty);
    run(&mut s, "document.set_notes", json!({ "notes": "same" }));
    assert!(!s.doc().dirty);
    assert_eq!(s.doc().undo.undo_len(), 1);
}

#[test]
fn the_description_is_one_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(
        &mut s,
        "document.set_notes",
        json!({ "description": "two\nlines\r\nhere" }),
    );
    assert_eq!(s.doc().document.description, "two lines here");
}

#[test]
fn notes_read_back_for_agents() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(
        &mut s,
        "document.set_notes",
        json!({ "description": "Stencil", "notes": "Tape the corners." }),
    );
    let r = s
        .execute("document.notes", json!({}), Origin::Agent)
        .unwrap();
    assert_eq!(
        r,
        json!({ "description": "Stencil", "notes": "Tape the corners." })
    );
}

#[test]
fn agents_append_but_never_replace() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(
        &mut s,
        "document.set_notes",
        json!({ "notes": "Tape the corners." }),
    );
    let refused = s.execute(
        "document.set_notes",
        json!({ "notes": "gone" }),
        Origin::Agent,
    );
    assert!(
        matches!(refused, Err(CommandError::Refused(_))),
        "{refused:?}"
    );
    assert_eq!(s.doc().document.notes, "Tape the corners.");

    let r = s
        .execute(
            "document.append_notes",
            json!({ "text": "Agent: the star needs force 18." }),
            Origin::Agent,
        )
        .unwrap();
    assert_eq!(
        s.doc().document.notes,
        "Tape the corners.\nAgent: the star needs force 18."
    );
    assert_eq!(r["notes"], s.doc().document.notes.as_str());
    assert!(s.doc().dirty);
    // One undo step, as for any change.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().document.notes, "Tape the corners.");

    // On empty notes there is no blank first line.
    run(&mut s, "document.set_notes", json!({ "notes": "" }));
    s.execute(
        "document.append_notes",
        json!({ "text": "first" }),
        Origin::Agent,
    )
    .unwrap();
    assert_eq!(s.doc().document.notes, "first");

    let empty = s.execute(
        "document.append_notes",
        json!({ "text": "  " }),
        Origin::Agent,
    );
    assert!(
        matches!(empty, Err(CommandError::InvalidParams(_))),
        "{empty:?}"
    );
}

#[test]
fn notes_have_a_size_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let long = "x".repeat(flatcraft_engine::commands::notes::MAX_NOTES_CHARS + 1);
    let r = s.execute("document.set_notes", json!({ "notes": long }), Origin::User);
    assert!(matches!(r, Err(CommandError::InvalidParams(_))), "{r:?}");
    let r = s.execute(
        "document.append_notes",
        json!({ "text": long }),
        Origin::Agent,
    );
    assert!(matches!(r, Err(CommandError::InvalidParams(_))), "{r:?}");
    assert_eq!(s.doc().document.notes, "");
}

#[test]
fn notes_survive_save_and_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(
        &mut s,
        "document.set_notes",
        json!({ "description": "Labels", "notes": "Weed slowly." }),
    );
    let path = dir.path().join("p.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    run(&mut s, "document.new", json!({}));
    run(&mut s, "project.open", json!({ "path": path }));
    assert_eq!(s.doc().document.description, "Labels");
    assert_eq!(s.doc().document.notes, "Weed slowly.");
    assert!(!s.doc().dirty);
}
