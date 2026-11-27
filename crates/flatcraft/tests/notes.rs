//! Project notes in the window: the Notes section under Objects, the dot on
//! the Objects rail, the description on the Start page and in the tab's
//! card, and the notes in the Cut workspace.

mod common;
use common::RunPlan;

use common::*;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::rails::{self, Dot, SidePanel};
use flatcraft::ui::{Workspace, prepare};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

fn run(s: &SharedSession, cmd: &str, params: Value) -> Value {
    s.lock()
        .unwrap()
        .execute(cmd, params, Origin::User)
        .unwrap()
}

fn snap(h: &mut AppHarness) -> prepare::Snapshot {
    let ctx = h.ctx.clone();
    prepare::snapshot(app_mut(h), &ctx)
}

/// Clicks into the field and types `text`, staying in it.
fn type_into(h: &mut AppHarness, label: &str, text: &str) {
    h.get_by_label(label).click();
    h.run_plan();
    h.get_by_label(label).type_text(text);
    h.run_plan();
    h.run_plan();
}

/// Opens the Notes at the bottom of the Objects panel (folded at first).
fn open_notes(h: &mut AppHarness) {
    if h.query_by_label("Project notes").is_none() {
        h.get_by_role_and_label(Role::Button, "Notes").click();
        h.run_plan();
    }
}

/// Prepare with the Notes open.
fn in_prepare(dir: &tempfile::TempDir, s: SharedSession) -> AppHarness {
    let mut h = harness_with(dir, s, |a| a);
    to_prepare(&mut h);
    h.run_plan();
    open_notes(&mut h);
    h
}

#[test]
fn typing_notes_is_one_undo_step_and_marks_the_document_unsaved() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    run(
        &s,
        "project.save",
        json!({ "path": dir.path().join("p.flatcraft") }),
    );
    let steps = undo_len(&s);
    let mut h = in_prepare(&dir, s.clone());
    type_into(&mut h, "Project notes", "Blue mat, face down.");
    {
        let st = s.lock().unwrap();
        assert_eq!(st.doc().document.notes, "Blue mat, face down.");
        assert!(st.doc().dirty, "a notes edit is an unsaved change");
    }
    assert_eq!(undo_len(&s), steps + 1, "one typing burst, one undo step");

    type_into(&mut h, "Project description", "Kitchen labels");
    assert_eq!(
        s.lock().unwrap().doc().document.description,
        "Kitchen labels"
    );

    run(&s, "edit.undo", json!({}));
    h.run_plan();
    assert_eq!(s.lock().unwrap().doc().document.description, "");
    assert_eq!(field_text(&h, "Project description"), "");
    run(&s, "edit.undo", json!({}));
    h.run_plan();
    assert_eq!(s.lock().unwrap().doc().document.notes, "");
    assert_eq!(field_text(&h, "Project notes"), "");
}

#[test]
fn an_agent_append_shows_in_the_field() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = in_prepare(&dir, s.clone());
    s.lock()
        .unwrap()
        .execute(
            "document.append_notes",
            json!({ "text": "Agent: try force 14." }),
            Origin::Agent,
        )
        .unwrap();
    h.run_plan();
    assert_eq!(field_text(&h, "Project notes"), "Agent: try force 14.");
}

#[test]
fn the_notes_section_collapses_and_has_hover_cards() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = in_prepare(&dir, s.clone());
    h.ctx
        .global_style_mut(|st| st.interaction.tooltip_delay = 0.0);
    for label in ["Project description", "Project notes"] {
        let at = h.get_by_label(label).rect().center();
        h.hover_at(at);
        h.run_plan();
        h.run_plan();
        assert!(
            h.query_by_label_contains("only on this computer").is_some()
                || h.query_by_label_contains("project file").is_some(),
            "{label} has a hover card"
        );
    }
    h.get_by_role_and_label(Role::Button, "Notes").click();
    h.run_plan();
    assert!(h.query_by_label("Project notes").is_none(), "collapsed");
    h.get_by_role_and_label(Role::Button, "Notes").click();
    h.run_plan();
    assert!(h.query_by_label("Project notes").is_some(), "open again");
}

#[test]
fn the_objects_rail_has_a_dot_when_the_project_has_notes() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = in_prepare(&dir, s.clone());
    assert_eq!(rails::objects_summary(&snap(&mut h)).dot, None);
    run(
        &s,
        "document.set_notes",
        json!({ "notes": "Use the blue mat." }),
    );
    h.run_plan();
    let summary = rails::objects_summary(&snap(&mut h));
    assert_eq!(summary.dot, Some(Dot::Notes));
    assert!(
        summary.full().ends_with(Dot::Notes.label()),
        "{}",
        summary.full()
    );
    // A description alone is no note.
    run(
        &s,
        "document.set_notes",
        json!({ "notes": "", "description": "Labels" }),
    );
    h.run_plan();
    assert_eq!(rails::objects_summary(&snap(&mut h)).dot, None);

    run(
        &s,
        "document.set_notes",
        json!({ "notes": "Use the blue mat." }),
    );
    app_mut(&mut h)
        .ui
        .prepare
        .panels
        .set_collapsed(SidePanel::Objects, true);
    h.run_plan();
    let rail = h.get_by_label_contains("Open the Objects panel");
    assert!(
        rail.accesskit_node()
            .label()
            .unwrap_or_default()
            .contains(Dot::Notes.label()),
        "the rail says it"
    );
}

#[test]
fn the_tab_card_shows_the_description() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    run(
        &s,
        "document.set_notes",
        json!({ "description": "Gift tags for the fair" }),
    );
    run(&s, "document.new", json!({}));
    let mut h = harness_with(&dir, s.clone(), |a| a);
    h.ctx
        .global_style_mut(|st| st.interaction.tooltip_delay = 0.0);
    h.run_plan();
    let tab = format!("Untitled 1 {}", flatcraft::ui::app_bar::DIRTY);
    let at = h.get_by_role_and_label(Role::Button, &tab).rect().center();
    h.hover_at(at);
    h.run_plan();
    h.run_plan();
    assert!(
        h.query_by_label_contains("Gift tags for the fair")
            .is_some()
    );
}

#[test]
fn the_start_page_shows_the_description_under_the_name() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let path = dir.path().join("tags.flatcraft");
    {
        let mut st = s.lock().unwrap();
        st.execute(
            "document.set_notes",
            json!({ "description": "Gift tags for the fair" }),
            Origin::User,
        )
        .unwrap();
        st.execute("project.save", json!({ "path": path }), Origin::User)
            .unwrap();
        let id = st.doc().id();
        st.execute("document.new", json!({}), Origin::User).unwrap();
        st.execute("document.close", json!({ "doc_id": id }), Origin::User)
            .unwrap();
        st.prefs.recent_projects = vec![path.clone()];
    }
    let h = harness_with(&dir, s.clone(), |a| a);
    let texts: Vec<String> = painted(&h)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect();
    assert!(
        texts.iter().any(|t| t == "Gift tags for the fair"),
        "{texts:?}"
    );
    assert_eq!(app(&h).ui.workspace, Workspace::Home);
}

#[test]
fn the_cut_workspace_shows_the_notes_above_the_send_button() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    run(
        &s,
        "document.set_notes",
        json!({ "notes": "Tape the corners down." }),
    );
    let mut h = harness_with(&dir, s.clone(), |a| a);
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    h.run_plan();
    h.get_by_role_and_label(Role::Tab, "Send").click();
    h.run_plan();
    let notes = h.get_by_label("Tape the corners down.").rect();
    let send = h
        .get_by_role_and_label(Role::Button, "Send to File Output")
        .rect();
    assert!(
        notes.bottom() <= send.top(),
        "the notes sit above the button"
    );
    // Collapsible, with a "Notes" heading.
    h.get_by_role_and_label(Role::Button, "Notes").click();
    h.run_plan();
    assert!(h.query_by_label("Tape the corners down.").is_none());

    // No notes: no section.
    run(&s, "document.set_notes", json!({ "notes": "" }));
    h.run_plan();
    assert!(h.query_by_role_and_label(Role::Button, "Notes").is_none());
}

#[test]
fn typing_over_many_frames_is_still_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let steps = undo_len(&s);
    let mut h = in_prepare(&dir, s.clone());
    h.get_by_label("Project notes").click();
    h.run_plan();
    for c in ["a", "b", "c", "d"] {
        h.get_by_label("Project notes").type_text(c);
        h.run_plan();
        h.run_plan();
    }
    assert_eq!(s.lock().unwrap().doc().document.notes, "abcd");
    assert_eq!(undo_len(&s), steps + 1, "one visit, one undo step");
    // Leaving the field and coming back is a new step.
    h.key_press(egui::Key::Escape);
    h.run_plan();
    h.get_by_label("Project notes").click();
    h.run_plan();
    h.get_by_label("Project notes").type_text("e");
    h.run_plan();
    h.run_plan();
    assert_eq!(undo_len(&s), steps + 2);
}

#[test]
fn the_notes_stay_reachable_in_a_short_window_with_many_objects() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let ids: Vec<u64> = (0..12).map(|_| import_square(&s)).collect();
    run(&s, "object.select", json!({ "ids": [ids[0]] }));
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 700.0], 8, |a| a);
    to_prepare(&mut h);
    h.run_plan();
    open_notes(&mut h);
    h.get_by_label("Project notes").scroll_to_me();
    h.run_plan();
    h.run_plan();
    let field = h.get_by_label("Project notes").rect();
    assert!(
        field.bottom() <= 700.0 && field.top() >= 0.0,
        "the field can be seen: {field:?}"
    );
    type_into(&mut h, "Project notes", "short window");
    assert_eq!(s.lock().unwrap().doc().document.notes, "short window");
}
