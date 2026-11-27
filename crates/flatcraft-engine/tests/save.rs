//! Saves run their slow write without the session lock, one at a time.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use flatcraft_core::document::ObjectId;
use flatcraft_device::ProfileStore;
use flatcraft_engine::commands::project::{SaveParams, begin_save, finish_save, save_unlocked};
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{Origin, SaveWriteHook, Session};
use serde_json::{Value, json};

fn session(dir: &Path) -> Session {
    let profiles = ProfileStore::load(dir).unwrap();
    let prefs = Preferences {
        output_dir: Some(dir.join("output")),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, false, dir.to_path_buf())
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

const SQUARE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="10" y="10" width="20" height="20"/></svg>"#;

fn import(s: &mut Session) -> u64 {
    run(
        s,
        "object.import_svg",
        json!({ "svg": SQUARE, "name": "square" }),
    )["id"]
        .as_u64()
        .unwrap()
}

fn center_x(path: &Path, id: u64) -> f64 {
    let doc = flatcraft_core::project::load(path).unwrap();
    doc.get(ObjectId(id)).unwrap().transform.center_mm.x
}

/// The write step of every save stops in the hook: it says which file it
/// writes on `entered` and waits for `go` (true: it panics).
struct Hook {
    entered: Receiver<PathBuf>,
    go: Sender<bool>,
}

fn hook(s: &mut Session) -> Hook {
    let (entered_tx, entered) = channel();
    let (go, go_rx) = channel::<bool>();
    let entered_tx = Mutex::new(entered_tx);
    let go_rx = Mutex::new(go_rx);
    s.save_write_hook = Some(SaveWriteHook(Arc::new(move |p: &Path| {
        entered_tx.lock().unwrap().send(p.to_path_buf()).unwrap();
        if go_rx.lock().unwrap().recv().unwrap() {
            panic!("test: the write step panics");
        }
    })));
    Hook { entered, go }
}

fn save_in_thread(
    shared: &Arc<Mutex<Session>>,
    params: Value,
) -> std::thread::JoinHandle<Result<Value, String>> {
    let shared = Arc::clone(shared);
    std::thread::spawn(move || {
        save_unlocked(&shared, params, Origin::User).map_err(|e| e.to_string())
    })
}

/// `p` with its folder resolved, as the save writes it.
fn real(p: &Path) -> PathBuf {
    p.parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join(p.file_name().unwrap())
}

/// The second save waits for the first: nothing of it reaches the write
/// step while the first one writes.
fn assert_waits(h: &Hook) {
    std::thread::sleep(Duration::from_millis(200));
    assert!(h.entered.try_recv().is_err(), "the second save waits");
}

#[test]
fn a_nudge_back_during_the_write_keeps_the_document_dirty() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("p.flatcraft");
    let mut s = session(dir.path());
    let id = import(&mut s);
    run(&mut s, "project.save", json!({ "path": path }));
    assert!(!s.doc().dirty);
    let nudge = |s: &mut Session, dx: f64| {
        run(
            s,
            "object.move",
            json!({ "ids": [id], "dx_mm": dx, "dy_mm": 0.0, "coalesce_key": "keys" }),
        );
    };
    nudge(&mut s, 5.0);
    let params: SaveParams = serde_json::from_value(json!({})).unwrap();
    let job = begin_save(&mut s, params, Origin::User).unwrap();
    // Without the lock: the nudge goes back where the burst started.
    nudge(&mut s, -5.0);
    let written = job.write();
    finish_save(&mut s, job, written).unwrap();
    assert!(s.doc().dirty, "the file holds the nudged state");
    let now = s.doc().document.get(ObjectId(id)).unwrap().transform;
    assert!((center_x(&path, id) - now.center_mm.x - 5.0).abs() < 1e-6);
}

#[test]
fn two_saves_of_one_document_end_with_the_newer_one_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("p.flatcraft");
    let mut s = session(dir.path());
    let id = import(&mut s);
    let h = hook(&mut s);
    let shared = Arc::new(Mutex::new(s));
    let a = save_in_thread(&shared, json!({ "path": path }));
    assert_eq!(h.entered.recv().unwrap(), real(&path));
    // The first save writes without the lock: commands go on.
    run(
        &mut shared.lock().unwrap(),
        "object.move",
        json!({ "ids": [id], "dx_mm": 20.0, "dy_mm": 0.0 }),
    );
    let b = save_in_thread(&shared, json!({ "path": path }));
    assert_waits(&h);
    assert!(shared.try_lock().is_ok(), "the waiting save holds no lock");
    h.go.send(false).unwrap();
    assert_eq!(h.entered.recv().unwrap(), real(&path));
    h.go.send(false).unwrap();
    a.join().unwrap().unwrap();
    b.join().unwrap().unwrap();
    let s = shared.lock().unwrap();
    let now = s.doc().document.get(ObjectId(id)).unwrap().transform;
    assert!((center_x(&path, id) - now.center_mm.x).abs() < 1e-6);
    assert!(!s.doc().dirty);
}

#[test]
fn save_as_to_two_paths_ends_with_the_last_one() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let one = out.path().join("one.flatcraft");
    let two = out.path().join("two.flatcraft");
    let mut s = session(dir.path());
    import(&mut s);
    let h = hook(&mut s);
    let shared = Arc::new(Mutex::new(s));
    let a = save_in_thread(&shared, json!({ "path": one }));
    assert_eq!(h.entered.recv().unwrap(), real(&one));
    let b = save_in_thread(&shared, json!({ "path": two }));
    assert_waits(&h);
    h.go.send(false).unwrap();
    assert_eq!(h.entered.recv().unwrap(), real(&two));
    h.go.send(false).unwrap();
    a.join().unwrap().unwrap();
    b.join().unwrap().unwrap();
    let s = shared.lock().unwrap();
    assert_eq!(s.doc().path.as_deref(), Some(two.as_path()));
    assert!(two.exists() && one.exists());
    assert!(!s.doc().dirty);
}

#[test]
fn a_failed_save_does_not_undo_the_relink_of_another() {
    let dir = tempfile::tempdir().unwrap();
    let base = tempfile::tempdir().unwrap();
    let art = base.path().join("art");
    std::fs::create_dir_all(&art).unwrap();
    let file = art.join("a.svg");
    std::fs::write(&file, SQUARE).unwrap();
    for d in ["one", "two/deeper"] {
        std::fs::create_dir_all(base.path().join(d)).unwrap();
    }
    let p1 = base.path().join("one/p.flatcraft");
    let p2 = base.path().join("two/deeper/p.flatcraft");
    let mut s = session(dir.path());
    let id = run(&mut s, "object.import", json!({ "path": file }))["id"]
        .as_u64()
        .unwrap();
    let h = hook(&mut s);
    let shared = Arc::new(Mutex::new(s));
    let a = save_in_thread(&shared, json!({ "path": p1 }));
    assert_eq!(h.entered.recv().unwrap(), real(&p1));
    let b = save_in_thread(&shared, json!({ "path": p2 }));
    assert_waits(&h);
    // The first save panics in its write step: a plain error.
    h.go.send(true).unwrap();
    assert_eq!(h.entered.recv().unwrap(), real(&p2));
    h.go.send(false).unwrap();
    let e = a.join().unwrap().unwrap_err();
    assert!(e.contains("internal error"), "{e}");
    b.join().unwrap().unwrap();
    let s = shared.lock().unwrap();
    let link = s
        .doc()
        .document
        .get(ObjectId(id))
        .unwrap()
        .link
        .clone()
        .unwrap();
    let want = Some(PathBuf::from("../../art/a.svg"));
    assert_eq!(link.relative, want, "the second save's relink stays");
    let saved = flatcraft_core::project::load(&p2).unwrap();
    assert_eq!(
        saved
            .get(ObjectId(id))
            .unwrap()
            .link
            .as_ref()
            .unwrap()
            .relative,
        want
    );
    assert_eq!(s.doc().path.as_deref(), Some(p2.as_path()));
    assert!(!s.doc().dirty);
}

#[test]
fn a_panic_in_the_write_step_is_a_plain_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("p.flatcraft");
    let mut s = session(dir.path());
    import(&mut s);
    let h = hook(&mut s);
    h.go.send(true).unwrap();
    let shared = Mutex::new(s);
    let e = save_unlocked(&shared, json!({ "path": path }), Origin::User).unwrap_err();
    assert!(e.to_string().contains("internal error"), "{e}");
    let s = shared.lock().unwrap();
    assert!(s.doc().dirty, "not saved");
    assert!(s.doc().path.is_none());
    assert!(!path.exists());
}

#[test]
fn the_registry_save_refuses_while_another_save_writes() {
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    import(&mut s);
    let gate = s.save_gate();
    let _busy = gate.lock().unwrap();
    let e = s
        .execute(
            "project.save",
            json!({ "path": out.path().join("p.flatcraft") }),
            Origin::User,
        )
        .unwrap_err();
    assert!(e.to_string().contains("another save"), "{e}");
}
