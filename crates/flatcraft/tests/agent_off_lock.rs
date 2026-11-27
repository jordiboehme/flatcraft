//! The window keeps drawing while an agent's long command computes: the
//! command holds the session lock only for its snapshot and its store
//! (`flatcraft_engine::unlocked`), and a result built on a document the
//! window changed meanwhile is not stored.

mod common;

use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use flatcraft_engine::session::{Origin, UnlockedHook};
use flatcraft_engine::unlocked;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

use common::WORKER_TIMEOUT;

/// A black square on white, 300 x 300 px.
fn write_square(path: &Path) {
    let mut px = tiny_skia::Pixmap::new(300, 300).unwrap();
    px.fill(tiny_skia::Color::WHITE);
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::BLACK);
    px.fill_rect(
        tiny_skia::Rect::from_xywh(50.0, 50.0, 200.0, 200.0).unwrap(),
        &paint,
        tiny_skia::Transform::identity(),
        None,
    );
    px.save_png(path).unwrap();
}

/// Holds every unlocked command in its compute step until the test lets
/// it go: `entered` gets the command id, `release` lets it go on.
struct Gate {
    entered: Receiver<String>,
    release: Sender<()>,
}

fn gate(s: &SharedSession) -> Gate {
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let entered_tx = Mutex::new(entered_tx);
    let release_rx = Mutex::new(release_rx);
    s.lock().unwrap().unlocked_hook = Some(UnlockedHook(Arc::new(move |id: &str| {
        entered_tx.lock().unwrap().send(id.to_string()).unwrap();
        // Never hangs the test: after the timeout the command goes on.
        let _ = release_rx.lock().unwrap().recv_timeout(WORKER_TIMEOUT);
    })));
    Gate { entered, release }
}

/// Starts the agent's command on its own thread, as the MCP server does.
fn agent(
    s: &SharedSession,
    id: &'static str,
    params: Value,
) -> std::thread::JoinHandle<Result<Value, flatcraft_engine::CommandError>> {
    let s = s.clone();
    std::thread::spawn(move || unlocked::execute(&s, id, params, Origin::Agent))
}

#[test]
fn the_window_draws_while_an_agent_imports_traces_and_previews() {
    let dir = tempfile::tempdir().unwrap();
    let s = common::session(&dir);
    let mut h = common::harness(&dir, s.clone(), true);
    let png = dir.path().join("square.png");
    write_square(&png);
    let g = gate(&s);
    for (id, params) in [
        ("object.import", json!({ "path": png })),
        (
            "object.retrace",
            json!({ "id": 1, "settings": { "threshold": 90 } }),
        ),
        (
            "document.preview",
            json!({ "kind": "canvas", "max_px": 256 }),
        ),
    ] {
        let run = agent(&s, id, params);
        assert_eq!(
            g.entered.recv_timeout(WORKER_TIMEOUT).unwrap(),
            id,
            "the agent's command reached its compute step"
        );
        // The window draws frames, each takes the session lock, while
        // the agent still computes.
        for _ in 0..3 {
            h.step();
        }
        assert!(s.try_lock().is_ok(), "{id}: the session lock is free");
        assert!(!run.is_finished(), "{id}: the frames ran while it computed");
        g.release.send(()).unwrap();
        let r = run.join().unwrap();
        assert!(r.is_ok(), "{id}: {r:?}");
        common::settle(&mut h);
    }
    let st = s.lock().unwrap();
    assert_eq!(st.doc().document.objects.len(), 1);
    assert_eq!(st.doc().undo.undo_len(), 2, "one undo step per command");
}

#[test]
fn an_import_lands_once_while_the_person_moves_another_object_every_frame() {
    use flatcraft::ui::Action;
    let dir = tempfile::tempdir().unwrap();
    let s = common::session(&dir);
    let png = dir.path().join("square.png");
    write_square(&png);
    let picker = common::FakePicker::answering([png.clone(), png]);
    let mut h = common::harness_with(&dir, s.clone(), move |a| a.with_file_picker(picker));
    let mine = s
        .lock()
        .unwrap()
        .execute(
            "object.import_svg",
            json!({ "svg": r#"<svg xmlns="http://www.w3.org/2000/svg" width="50mm" height="50mm" viewBox="0 0 50 50"><rect x="10" y="10" width="20" height="20"/></svg>"#, "name": "mine" }),
            Origin::User,
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    common::app_mut(&mut h).ui.workspace = flatcraft::ui::Workspace::Prepare;
    common::settle(&mut h);
    let g = gate(&s);
    let ctx = h.ctx.clone();
    for (who, start) in ["window", "agent"].into_iter().zip([0usize, 1]) {
        let doc_id = s.lock().unwrap().doc().id();
        let before = s.lock().unwrap().doc().undo.undo_len();
        let run = (who == "agent").then(|| {
            agent(
                &s,
                "object.import",
                json!({ "path": dir.path().join("square.png") }),
            )
        });
        if who == "window" {
            common::app_mut(&mut h).apply(&ctx, vec![Action::Import { doc_id }]);
        }
        assert_eq!(
            g.entered.recv_timeout(WORKER_TIMEOUT).unwrap(),
            "object.import"
        );
        // The person nudges their object in every frame while it traces.
        for _ in 0..4 {
            s.lock()
                .unwrap()
                .execute(
                    "object.move",
                    json!({ "ids": [mine], "dx_mm": 1.0 }),
                    Origin::User,
                )
                .unwrap();
            h.step();
        }
        g.release.send(()).unwrap();
        if let Some(run) = run {
            assert!(run.join().unwrap().is_ok(), "{who}");
        }
        common::settle(&mut h);
        // Traced once and applied to the document as it is now.
        assert!(g.entered.try_recv().is_err(), "{who}: no second try");
        let st = s.lock().unwrap();
        let doc = &st.doc().document;
        assert_eq!(doc.objects.len(), 2 + start, "{who}: the import landed");
        let moved = doc.get(flatcraft_core::document::ObjectId(mine)).unwrap();
        let x = 20.0 + 4.0 * (start as f64 + 1.0);
        assert!(
            (moved.transform.center_mm.x - x).abs() < 1e-4,
            "{who}: the moves are kept"
        );
        assert_eq!(
            st.doc().undo.undo_len(),
            before + 5,
            "{who}: 4 moves and one import step"
        );
    }
}

/// Holds every Cut plan the window builds until the test lets it go, and
/// counts them.
struct PlanGate {
    entered: Receiver<()>,
    release: Sender<()>,
}

fn plan_gate(app: &mut flatcraft::ui::FlatCraftApp) -> PlanGate {
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let entered_tx = Mutex::new(entered_tx);
    let release_rx = Mutex::new(release_rx);
    app.ui.cut.set_builder(move |input| {
        entered_tx.lock().unwrap().send(()).unwrap();
        let _ = release_rx.lock().unwrap().recv_timeout(WORKER_TIMEOUT);
        flatcraft_engine::job::build_classified(input)
    });
    PlanGate { entered, release }
}

/// The texts painted last frame.
fn texts(h: &common::AppHarness) -> Vec<String> {
    common::painted(h)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_cut_plan_builds_on_a_worker_and_a_stale_plan_is_never_shown() {
    use flatcraft::ui::cut::{PLANNING, Stage};
    let dir = tempfile::tempdir().unwrap();
    let s = common::session(&dir);
    let id = common::import_square(&s);
    let mut h = common::harness(&dir, s.clone(), false);
    common::app_mut(&mut h).ui.cut.stage = Stage::Path;
    common::app_mut(&mut h).ui.workspace = flatcraft::ui::Workspace::Cut;
    common::settle(&mut h);
    let gate = plan_gate(common::app_mut(&mut h));
    let built = common::app(&h).ui.cut.plans_built();
    // A change gives a new input A.
    s.lock()
        .unwrap()
        .execute(
            "object.move",
            json!({ "ids": [id], "dx_mm": 3.0 }),
            Origin::User,
        )
        .unwrap();
    h.step();
    gate.entered.recv_timeout(WORKER_TIMEOUT).unwrap();
    // The plan of A builds; the window keeps drawing frames meanwhile.
    for _ in 0..3 {
        h.step();
    }
    let cut = &common::app(&h).ui.cut;
    assert!(cut.planning() && cut.plan.is_none(), "{:?}", cut.plan);
    assert!(
        cut.previous.is_some(),
        "the plan before is kept to draw faded"
    );
    assert!(texts(&h).iter().any(|t| t == PLANNING), "the note shows");
    // The person moves the square while A builds: A is stale now.
    s.lock()
        .unwrap()
        .execute(
            "object.move",
            json!({ "ids": [id], "dx_mm": 7.0 }),
            Origin::User,
        )
        .unwrap();
    h.step();
    gate.release.send(()).unwrap();
    // B starts in the frame that takes A; A is dropped, never shown.
    let start = std::time::Instant::now();
    while gate.entered.try_recv().is_err() {
        assert!(start.elapsed() < WORKER_TIMEOUT, "B did not start");
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    for _ in 0..3 {
        h.step();
    }
    let cut = &common::app(&h).ui.cut;
    assert_eq!(cut.plans_built(), built + 1, "A was built");
    assert!(cut.plan.is_none(), "A is not shown");
    assert!(cut.planning());
    gate.release.send(()).unwrap();
    common::settle(&mut h);
    let cut = &common::app(&h).ui.cut;
    assert_eq!(cut.plans_built(), built + 2);
    let shown = cut.job().expect("the plan of B");
    let now = flatcraft_engine::job::build(&s.lock().unwrap()).unwrap();
    assert_eq!(
        shown.job.bytes, now.job.bytes,
        "the plan of the moved square"
    );
    assert!(!texts(&h).iter().any(|t| t == PLANNING), "the note is gone");
}

#[test]
fn the_window_draws_while_its_own_import_runs() {
    use flatcraft::ui::{Action, long, tools};
    let dir = tempfile::tempdir().unwrap();
    let s = common::session(&dir);
    let png = dir.path().join("square.png");
    write_square(&png);
    let picker = common::FakePicker::answering([png]);
    let mut h = common::harness_with(&dir, s.clone(), move |a| a.with_file_picker(picker));
    common::app_mut(&mut h).ui.workspace = flatcraft::ui::Workspace::Prepare;
    common::settle(&mut h);
    let g = gate(&s);
    let doc_id = s.lock().unwrap().doc().id();
    let ctx = h.ctx.clone();
    common::app_mut(&mut h).apply(&ctx, vec![Action::Import { doc_id }]);
    assert_eq!(
        g.entered.recv_timeout(WORKER_TIMEOUT).unwrap(),
        "object.import"
    );
    // The import traces on its worker; the window draws and the lock is
    // free.
    for _ in 0..3 {
        h.step();
    }
    assert!(s.try_lock().is_ok(), "the session lock is free");
    assert!(common::app(&h).long.busy());
    assert!(
        texts(&h).iter().any(|t| t == "Importing..."),
        "the busy note"
    );
    // An Open asked for now (Cmd+O, the menu) waits its turn, and the note
    // says so.
    let project = dir.path().join("p.flatcraft");
    s.lock()
        .unwrap()
        .execute("project.save", json!({ "path": project }), Origin::User)
        .unwrap();
    common::app_mut(&mut h).apply(&ctx, vec![Action::Open(project)]);
    h.step();
    assert!(
        texts(&h).iter().any(|t| t == "Importing... (opening next)"),
        "{:?}",
        texts(&h)
    );
    let snap = flatcraft::ui::prepare::snapshot(common::app_mut(&mut h), &ctx);
    assert_eq!(
        tools::by_id("import").unwrap().blocked(&snap),
        Some(long::BUSY_CARD),
        "Import is dimmed and says why"
    );
    assert_eq!(s.lock().unwrap().doc().document.objects.len(), 0);
    // One release for the import, one for the open after it.
    g.release.send(()).unwrap();
    g.release.send(()).unwrap();
    common::settle(&mut h);
    assert!(!common::app(&h).long.busy());
    let st = s.lock().unwrap();
    // The import landed in the first document, and the project opened
    // next to it after it.
    assert_eq!(st.docs.len(), 2);
    assert_eq!(st.docs[0].document.objects.len(), 1);
    assert_eq!(st.docs[0].undo.undo_len(), 1, "one undo step");
    drop(st);
    assert!(!texts(&h).iter().any(|t| t.starts_with("Importing...")));
}
