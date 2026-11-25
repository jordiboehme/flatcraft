//! The shared dialog (Task 40): its parts, its keys, the per-document
//! questions on Quit, and Escape never running the destructive action.

mod common;

use common::*;
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::dialog::CLOSE_NAME;
use flatcraft::ui::menu_model::{Cmd, MenuModel};
use flatcraft::ui::{Action, Unsaved, UnsavedChoice};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::json;

fn act(h: &mut AppHarness, action: Action) {
    let ctx = h.ctx.clone();
    app_mut(h).apply(&ctx, vec![action]);
    h.run();
}

/// `n` documents with a square each, all with unsaved changes; the last
/// one is active.
fn dirty_docs(n: usize) -> (tempfile::TempDir, SharedSession) {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    for _ in 1..n {
        s.lock()
            .unwrap()
            .execute("document.new", json!({}), Origin::User)
            .unwrap();
        import_square(&s);
    }
    (dir, s)
}

fn docs(s: &SharedSession) -> usize {
    s.lock().unwrap().docs.len()
}

fn doc_id(s: &SharedSession, i: usize) -> u64 {
    s.lock().unwrap().docs[i].id()
}

fn closing(h: &AppHarness) -> bool {
    app(h).ui.closing
}

/// Opens the Close question for the first document.
fn ask_close(h: &mut AppHarness, s: &SharedSession) -> u64 {
    let id = doc_id(s, 0);
    act(h, Action::Close(id));
    h.get_by_label("Save changes to Untitled 1?");
    id
}

#[test]
fn the_save_changes_dialog_has_its_parts() {
    let (dir, s) = dirty_docs(1);
    let mut h = harness(&dir, s.clone(), false);
    ask_close(&mut h, &s);
    h.get_by_role_and_label(Role::Image, "Warning");
    h.get_by_label("Untitled 1 has unsaved changes. If you don't save, they are lost.");
    let close = h.get_by_role_and_label(Role::Button, CLOSE_NAME).rect();
    let dont = h.get_by_role_and_label(Role::Button, "Don't Save").rect();
    let cancel = h.get_by_role_and_label(Role::Button, "Cancel").rect();
    let save = h
        .get_all_by_role_and_label(Role::Button, "Save")
        .last()
        .unwrap()
        .rect();
    let title = h.get_by_label("Save changes to Untitled 1?").rect();
    // × at the top right, the footer under the text: Don't Save on the
    // left, Cancel and then Save on the right.
    assert!(close.left() > title.left() && close.top() <= title.bottom());
    assert!(dont.top() > title.bottom());
    assert!(dont.right() < cancel.left() && cancel.right() < save.left());
    assert!((dont.center().y - save.center().y).abs() < 1.0);
    assert!(save.right() <= close.right() + 1.0);
}

#[test]
fn escape_and_the_close_button_cancel() {
    let (dir, s) = dirty_docs(1);
    let mut h = harness(&dir, s.clone(), false);
    ask_close(&mut h, &s);
    h.key_press(Key::Escape);
    h.run();
    assert!(app(&h).ui.unsaved.is_none());
    assert_eq!(docs(&s), 1);
    assert!(s.lock().unwrap().doc().dirty, "nothing was discarded");
    assert_eq!(s.lock().unwrap().doc().document.objects.len(), 1);
    ask_close(&mut h, &s);
    h.get_by_role_and_label(Role::Button, CLOSE_NAME).click();
    h.run();
    assert!(app(&h).ui.unsaved.is_none());
    assert_eq!(s.lock().unwrap().doc().document.objects.len(), 1);
}

#[test]
fn enter_saves() {
    let (dir, s) = dirty_docs(1);
    let path = dir.path().join("kept.flatcraft");
    let picker = FakePicker::answering([path.clone()]);
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker.clone()));
    ask_close(&mut h, &s);
    h.key_press(Key::Enter);
    h.run();
    assert!(app(&h).ui.unsaved.is_none());
    assert_eq!(
        flatcraft_core::project::load(&path).unwrap().objects.len(),
        1
    );
    assert!(
        s.lock().unwrap().doc().document.objects.is_empty(),
        "closed"
    );
}

#[test]
fn command_d_is_dont_save_and_not_duplicate() {
    let (dir, s) = dirty_docs(1);
    let id = import_square(&s);
    s.lock()
        .unwrap()
        .execute("object.select", json!({ "ids": [id] }), Origin::User)
        .unwrap();
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let before = s.lock().unwrap().doc().document.objects.len();
    assert_eq!(before, 2);
    ask_close(&mut h, &s);
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.run();
    assert!(app(&h).ui.unsaved.is_none());
    let g = s.lock().unwrap();
    assert_eq!(g.docs.len(), 1);
    assert!(
        g.doc().document.objects.is_empty(),
        "discarded, nothing duplicated"
    );
}

#[test]
fn quit_asks_for_each_document_in_turn() {
    let (dir, s) = dirty_docs(3);
    let path = dir.path().join("first.flatcraft");
    let picker = FakePicker::answering([path.clone()]);
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker.clone()));
    act(&mut h, Action::Quit);
    // 1: Save.
    h.get_by_label("Save changes to Untitled 1?");
    h.get_by_label("2 more documents have unsaved changes.");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(
        flatcraft_core::project::load(&path).unwrap().objects.len(),
        1
    );
    assert!(!closing(&h));
    // 2: Don't Save.
    h.get_by_label("Save changes to Untitled 2?");
    h.get_by_label("1 more document has unsaved changes.");
    h.get_by_role_and_label(Role::Button, "Don't Save").click();
    h.run();
    assert!(!closing(&h));
    // 3: Cancel stops quitting; the third document keeps its changes.
    h.get_by_label("Save changes to Untitled 3?");
    h.key_press(Key::Escape);
    h.run();
    assert!(!closing(&h));
    assert!(app(&h).ui.unsaved.is_none());
    assert_eq!(docs(&s), 3);
    // Again: only the documents still unsaved are asked about.
    act(&mut h, Action::Quit);
    h.get_by_label("Save changes to Untitled 2?");
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.run();
    h.get_by_label("Save changes to Untitled 3?");
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.run();
    assert!(closing(&h), "the last answer quits");
}

#[test]
fn escape_never_runs_the_destructive_action() {
    // Close, Quit, and Quit with an agent's send waiting.
    let (dir, s) = dirty_docs(2);
    let mut h = harness(&dir, s.clone(), false);
    ask_close(&mut h, &s);
    h.key_press(Key::Escape);
    h.run();
    act(&mut h, Action::Quit);
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(docs(&s), 2);
    assert!(s.lock().unwrap().docs.iter().all(|d| d.dirty));
    assert!(!closing(&h));

    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    {
        let mut g = s.lock().unwrap();
        g.execute(
            "job.set_plotter",
            json!({ "id": "silhouette-cameo-2" }),
            Origin::User,
        )
        .unwrap();
        g.execute("job.send", json!({}), Origin::Agent).unwrap_err();
        g.doc_mut().dirty = false;
    }
    let mut h = harness(&dir, s.clone(), true);
    act(&mut h, Action::Quit);
    assert_eq!(app(&h).ui.unsaved, Some(Unsaved::PendingSend));
    h.get_by_role_and_label(Role::Button, "Quit anyway");
    for key in [Key::Escape, Key::Enter] {
        act(&mut h, Action::Quit);
        h.key_press(key);
        h.run();
        assert!(!closing(&h), "{key:?} quit");
        assert!(s.lock().unwrap().pending_send().is_some());
    }
    // Cmd+D is not Quit anyway either: that key is only Don't Save.
    act(&mut h, Action::Quit);
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.run();
    assert!(!closing(&h));
}

#[test]
fn an_answer_to_an_old_question_is_ignored() {
    let (dir, s) = dirty_docs(2);
    let mut h = harness(&dir, s.clone(), false);
    act(&mut h, Action::Quit);
    let first = app(&h).ui.unsaved.clone().unwrap();
    act(
        &mut h,
        Action::Unsaved(first.clone(), UnsavedChoice::Discard),
    );
    // Now the second document is asked about; a second Discard meant for
    // the first question does not answer it.
    assert_ne!(app(&h).ui.unsaved.as_ref(), Some(&first));
    act(&mut h, Action::Unsaved(first, UnsavedChoice::Discard));
    assert!(app(&h).ui.unsaved.is_some());
    assert!(!closing(&h));
}

#[test]
fn rename_uses_the_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    app_mut(&mut h).ui.prepare.rename = Some(flatcraft::ui::prepare::RenameDialog {
        doc_id: s.lock().unwrap().doc().id(),
        id,
        name: String::new(),
    });
    h.run();
    // The title and the primary button.
    assert_eq!(h.query_all_by_label("Rename").count(), 2);
    h.get_by_role_and_label(Role::Image, "Info");
    h.get_by_role_and_label(Role::Button, CLOSE_NAME);
    // An empty name cannot be used: Enter does nothing.
    h.key_press(Key::Enter);
    h.run();
    assert!(app(&h).ui.prepare.rename.is_some());
    h.get_by_label("Name").type_text("Lid");
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert!(app(&h).ui.prepare.rename.is_none());
    let name = s.lock().unwrap().doc().document.objects[0].name.clone();
    assert_eq!(name, "Lid");
}

#[test]
fn a_dialog_dims_the_menu_bar_but_quit() {
    let (dir, s) = dirty_docs(1);
    let id = import_square(&s);
    s.lock()
        .unwrap()
        .execute("object.select", json!({ "ids": [id] }), Origin::User)
        .unwrap();
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    let model = |h: &mut AppHarness| {
        let ctx = h.ctx.clone();
        let snap = flatcraft::ui::prepare::snapshot_with(app_mut(h), &ctx, false);
        app(h).menu_model(&ctx, Some(&snap))
    };
    let enabled = |m: &MenuModel, cmd: &Cmd| m.leaves().iter().any(|l| &l.cmd == cmd && l.enabled);
    let dup = Cmd::Tool("duplicate");
    assert!(enabled(&model(&mut h), &dup));
    ask_close(&mut h, &s);
    let m = model(&mut h);
    assert!(!enabled(&m, &dup), "Cmd+D must reach the dialog");
    assert!(enabled(&m, &Cmd::Quit));
    assert_eq!(
        m.leaves().iter().filter(|l| l.enabled).count(),
        1,
        "only Quit"
    );
}

/// An agent's send waiting for the person, on a document with no unsaved
/// changes.
fn waiting_send() -> (tempfile::TempDir, SharedSession) {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    {
        let mut g = s.lock().unwrap();
        g.execute(
            "job.set_plotter",
            json!({ "id": "silhouette-cameo-2" }),
            Origin::User,
        )
        .unwrap();
        g.execute("job.send", json!({}), Origin::Agent).unwrap_err();
        g.doc_mut().dirty = false;
    }
    (dir, s)
}

const SEND_QUESTION: &str = "Allow the agent to cut on Silhouette Cameo 2?";

fn key_event(key: Key, pressed: bool, repeat: bool, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers,
    }
}

#[test]
fn no_key_allows_an_agent_cut() {
    for (key, mods) in [
        (Key::Enter, Modifiers::NONE),
        (Key::Escape, Modifiers::NONE),
        (Key::D, Modifiers::COMMAND),
    ] {
        let (dir, s) = waiting_send();
        let mut h = harness(&dir, s.clone(), true);
        h.get_by_label(SEND_QUESTION);
        h.get_by_role_and_label(Role::Image, "Warning");
        h.key_press_modifiers(mods, key);
        h.run();
        // Cmd+D does nothing; Enter and Escape deny. None allows.
        let g = s.lock().unwrap();
        if key == Key::D {
            assert!(g.pending_send().is_some(), "Cmd+D answered");
        } else {
            assert!(g.pending_send().is_none(), "{key:?} did not deny");
        }
        drop(g);
        assert!(
            app(&h)
                .ui
                .toasts
                .iter()
                .all(|t| !t.text.contains("milestone 3")),
            "{key:?} allowed the cut"
        );
    }
    // Only a click on Allow allows.
    let (dir, s) = waiting_send();
    let mut h = harness(&dir, s.clone(), true);
    h.get_by_role_and_label(Role::Button, "Allow").click();
    h.run();
    assert!(s.lock().unwrap().pending_send().is_none());
    assert!(
        app(&h)
            .ui
            .toasts
            .iter()
            .any(|t| t.text == "USB cutting arrives in milestone 3.")
    );
}

#[test]
fn a_key_held_while_a_dialog_opens_does_not_answer_it() {
    let (dir, s) = dirty_docs(1);
    let mut h = harness(&dir, s.clone(), false);
    // Enter goes down, and the dialog opens while it is held.
    h.event(key_event(Key::Enter, true, false, Modifiers::NONE));
    act(&mut h, Action::Close(doc_id(&s, 0)));
    h.get_by_label("Save changes to Untitled 1?");
    // It repeats, then comes up: nothing is answered.
    h.event(key_event(Key::Enter, true, true, Modifiers::NONE));
    h.run();
    h.event(key_event(Key::Enter, false, false, Modifiers::NONE));
    h.run();
    assert!(app(&h).ui.unsaved.is_some());
    assert_eq!(docs(&s), 1);
    // A fresh Escape now cancels.
    h.key_press(Key::Escape);
    h.run();
    assert!(app(&h).ui.unsaved.is_none());
}

#[test]
fn a_held_command_d_answers_only_one_document() {
    let (dir, s) = dirty_docs(2);
    let mut h = harness(&dir, s.clone(), false);
    act(&mut h, Action::Quit);
    h.get_by_label("Save changes to Untitled 1?");
    h.event(key_event(Key::D, true, false, Modifiers::COMMAND));
    h.run();
    h.get_by_label("Save changes to Untitled 2?");
    // The key repeats while held: the second question stays.
    for _ in 0..3 {
        h.event(key_event(Key::D, true, true, Modifiers::COMMAND));
        h.run();
    }
    h.event(key_event(Key::D, false, false, Modifiers::COMMAND));
    h.run();
    h.get_by_label("Save changes to Untitled 2?");
    assert!(!closing(&h));
}

#[test]
fn quit_also_asks_about_a_document_an_agent_changed_meanwhile() {
    // A has unsaved changes, B is clean.
    let (dir, s) = dirty_docs(1);
    s.lock()
        .unwrap()
        .execute("document.new", json!({}), Origin::User)
        .unwrap();
    // B is the active document, where the agent works.
    let path = dir.path().join("a.flatcraft");
    let picker = FakePicker::answering([path.clone()]);
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker.clone()));
    act(&mut h, Action::Quit);
    h.get_by_label("Save changes to Untitled 1?");
    // An agent edits B while the question is open.
    s.lock()
        .unwrap()
        .execute(
            "object.import_svg",
            json!({ "svg": SQUARE_SVG, "name": "late" }),
            Origin::Agent,
        )
        .unwrap();
    assert!(s.lock().unwrap().docs[1].dirty);
    h.key_press(Key::Enter);
    h.run();
    assert!(path.exists(), "A was saved");
    assert!(!closing(&h), "B is not answered yet");
    h.get_by_label("Save changes to Untitled 2?");
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.run();
    assert!(closing(&h));
}

#[test]
fn quit_skips_a_document_saved_meanwhile() {
    let (dir, s) = dirty_docs(2);
    let mut h = harness(&dir, s.clone(), false);
    act(&mut h, Action::Quit);
    h.get_by_label("Save changes to Untitled 1?");
    // The asked document is saved meanwhile (as an agent or a script
    // would; agents may not write into this test's config dir).
    let a = doc_id(&s, 0);
    s.lock()
        .unwrap()
        .execute(
            "project.save",
            json!({ "doc_id": a, "path": dir.path().join("a.flatcraft") }),
            Origin::User,
        )
        .unwrap();
    h.run();
    h.get_by_label("Save changes to Untitled 2?");
    assert!(h.query_by_label("Save changes to Untitled 1?").is_none());
}

#[test]
fn keys_answer_only_the_dialog_on_top() {
    // The agent's send dialog, and the unsaved-changes dialog over it.
    let (dir, s) = waiting_send();
    s.lock().unwrap().doc_mut().dirty = true;
    let mut h = harness(&dir, s.clone(), true);
    act(&mut h, Action::Close(doc_id(&s, 0)));
    h.get_by_label(SEND_QUESTION);
    h.get_by_label("Save changes to Untitled 1?");
    h.key_press(Key::Escape);
    h.run();
    // Escape cancelled the top dialog only; the agent's request still waits.
    assert!(app(&h).ui.unsaved.is_none());
    assert!(s.lock().unwrap().pending_send().is_some());
}

#[test]
fn rename_takes_a_name_typed_in_the_same_frame_as_enter() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    let mut h = harness(&dir, s.clone(), false);
    to_prepare(&mut h);
    app_mut(&mut h).ui.prepare.rename = Some(flatcraft::ui::prepare::RenameDialog {
        doc_id: s.lock().unwrap().doc().id(),
        id,
        name: String::new(),
    });
    h.run();
    h.event(egui::Event::Text("Lid".into()));
    h.key_press(Key::Enter);
    h.run();
    assert!(app(&h).ui.prepare.rename.is_none());
    assert_eq!(s.lock().unwrap().doc().document.objects[0].name, "Lid");
}

#[test]
fn quit_asks_again_about_a_document_changed_after_its_answer() {
    let (dir, s) = dirty_docs(2);
    let a = doc_id(&s, 0);
    let path = dir.path().join("a.flatcraft");
    let picker = FakePicker::answering([path.clone()]);
    let mut h = harness_with(&dir, s.clone(), |app| app.with_file_picker(picker.clone()));
    act(&mut h, Action::Quit);
    h.get_by_label("Save changes to Untitled 1?");
    h.key_press(Key::Enter);
    h.run();
    assert!(
        path.exists() && !s.lock().unwrap().docs[0].dirty,
        "A was saved"
    );
    h.get_by_label("Save changes to Untitled 2?");
    // An agent changes A again after its answer.
    {
        let mut g = s.lock().unwrap();
        g.execute("document.switch", json!({ "doc_id": a }), Origin::User)
            .unwrap();
        g.execute(
            "object.import_svg",
            json!({ "svg": SQUARE_SVG, "name": "late" }),
            Origin::Agent,
        )
        .unwrap();
        assert!(g.docs[0].dirty);
    }
    // B: Don't Save. A is asked again (its name is now its file name).
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.run();
    assert!(!closing(&h), "A changed after its answer");
    h.get_by_label("Save changes to a?");
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.run();
    assert!(closing(&h));
}

#[test]
fn two_documents_with_one_name_do_not_share_their_key_state() {
    use flatcraft::ui::dialog::{Dialog, Kind};
    // A key held from the answer to the first question does not answer
    // the second, even with the same title.
    let ctx = egui::Context::default();
    let t = flatcraft::ui::theme::Tokens::dark();
    let frame = |ctx: &egui::Context, d: &Dialog, events: Vec<egui::Event>| {
        let mut answer = None;
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            answer = d.show(ui.ctx(), &t, |_| {});
        });
        out.textures_delta.clear();
        answer
    };
    // The first question arms with no key down, then Enter answers it.
    let d = Dialog::new("unsaved", Kind::Warning, "Save changes to x.flatcraft?")
        .about(1)
        .primary("Save");
    frame(&ctx, &d, vec![]);
    frame(&ctx, &d, vec![]);
    let enter = |repeat| key_event(Key::Enter, true, repeat, Modifiers::NONE);
    assert_eq!(
        frame(&ctx, &d, vec![enter(false)]),
        Some(flatcraft::ui::dialog::Answer::Primary)
    );
    // Enter is still held (it repeats) when the second document's question
    // appears with the same title: it does not answer it.
    let second = d.clone().about(2);
    assert_eq!(frame(&ctx, &second, vec![enter(true)]), None);
    assert_eq!(frame(&ctx, &second, vec![enter(true)]), None);
}
