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
fn an_import_built_before_the_window_changed_the_document_is_not_stored() {
    let dir = tempfile::tempdir().unwrap();
    let s = common::session(&dir);
    let mut h = common::harness(&dir, s.clone(), true);
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
    common::settle(&mut h);
    let png = dir.path().join("square.png");
    write_square(&png);
    let g = gate(&s);
    let run = agent(&s, "object.import", json!({ "path": png }));
    g.entered.recv_timeout(WORKER_TIMEOUT).unwrap();
    // The person moves their object while the agent's first try traces.
    s.lock()
        .unwrap()
        .execute(
            "object.move",
            json!({ "ids": [mine], "dx_mm": 5.0 }),
            Origin::User,
        )
        .unwrap();
    h.step();
    g.release.send(()).unwrap();
    // The first result is dropped and the import runs once more.
    assert_eq!(
        g.entered.recv_timeout(WORKER_TIMEOUT).unwrap(),
        "object.import"
    );
    h.step();
    g.release.send(()).unwrap();
    let r = run.join().unwrap().unwrap();
    common::settle(&mut h);
    let st = s.lock().unwrap();
    let doc = &st.doc().document;
    assert_eq!(doc.objects.len(), 2);
    // The move is kept: the store did not put back the older document.
    let moved = doc.get(flatcraft_core::document::ObjectId(mine)).unwrap();
    assert!(
        (moved.transform.center_mm.x - 25.0).abs() < 1e-4,
        "{:?}",
        moved.transform
    );
    assert!(
        doc.get(flatcraft_core::document::ObjectId(
            r["id"].as_u64().unwrap()
        ))
        .is_some()
    );
    // The import, the move and the import.
    assert_eq!(st.doc().undo.undo_len(), 3);
}
