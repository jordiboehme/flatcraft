//! Long commands off the session lock (`flatcraft_engine::unlocked`): the
//! lock is free while they compute, a result built on an older document
//! is never stored, and the results are the same as under the lock.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use flatcraft_device::ProfileStore;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{CommandError, Origin, Session, UnlockedHook};
use flatcraft_engine::unlocked::{self, changed_error};
use serde_json::{Value, json};

type Shared = Arc<Mutex<Session>>;

/// A session whose config dir is `dir/config`, so agents may write to
/// `dir`.
fn session(dir: &std::path::Path) -> Session {
    let config = dir.join("config");
    let profiles = ProfileStore::load(&config).unwrap();
    let prefs = Preferences {
        output_dir: Some(dir.join("output")),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, false, config)
}

/// Gives `path` a fixed modification time, so two runs link the same.
fn fixed_time(path: &std::path::Path) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000))
        .unwrap();
}

fn shared(dir: &std::path::Path) -> Shared {
    Arc::new(Mutex::new(session(dir)))
}

/// Runs `f` with the session and the command id in the compute step of
/// every unlocked command. The hook holds the session weakly.
fn set_hook(s: &Shared, f: impl Fn(&Mutex<Session>, &str) + Send + Sync + 'static) {
    let weak: Weak<Mutex<Session>> = Arc::downgrade(s);
    s.lock().unwrap().unlocked_hook = Some(UnlockedHook(Arc::new(move |id: &str| {
        if let Some(s) = weak.upgrade() {
            f(&s, id);
        }
    })));
}

/// A 300 x 300 px image, white with a black square.
fn square_png(dir: &std::path::Path) -> std::path::PathBuf {
    let mut img = image::GrayImage::from_pixel(300, 300, image::Luma([255]));
    for y in 50..250 {
        for x in 50..250 {
            img.put_pixel(x, y, image::Luma([0]));
        }
    }
    let path = dir.join("square.png");
    img.save(&path).unwrap();
    fixed_time(&path);
    path
}

const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="20" y="20" width="50" height="30"/></svg>"#;

fn unlocked_run(s: &Shared, id: &str, params: Value) -> Result<Value, CommandError> {
    unlocked::execute(s, id, params, Origin::Agent)
}

fn locked_run(s: &Shared, id: &str, params: Value) -> Result<Value, CommandError> {
    s.lock().unwrap().execute(id, params, Origin::Agent)
}

fn add_svg(s: &Shared) -> u64 {
    locked_run(
        s,
        "object.import_svg",
        json!({ "svg": SVG, "name": "rect" }),
    )
    .unwrap()["id"]
        .as_u64()
        .unwrap()
}

fn nudge(s: &Mutex<Session>, id: u64) {
    s.lock()
        .unwrap()
        .execute(
            "object.move",
            json!({ "ids": [id], "dx_mm": 1.0 }),
            Origin::User,
        )
        .unwrap();
}

fn undo_len(s: &Shared) -> usize {
    s.lock().unwrap().doc().undo.undo_len()
}

fn objects(s: &Shared) -> usize {
    s.lock().unwrap().doc().document.objects.len()
}

/// The steps an agent takes, as `(command, params)`. `dir` holds the input
/// files; output files go to `out`.
fn steps(dir: &std::path::Path, out: &std::path::Path) -> Vec<(&'static str, Value)> {
    let png = square_png(dir);
    let svg = dir.join("shape.svg");
    std::fs::write(&svg, SVG).unwrap();
    fixed_time(&svg);
    vec![
        ("object.import", json!({ "path": png })),
        ("object.import_svg", json!({ "svg": SVG, "name": "text" })),
        (
            "object.import",
            json!({ "paths": [svg, dir.join("missing.svg")] }),
        ),
        (
            "object.retrace",
            json!({ "id": 1, "settings": { "threshold": 100 } }),
        ),
        ("object.retrace", json!({ "id": 2, "settings": {} })),
        ("object.reload", json!({ "ids": [1] })),
        // Inside the cutting area, so the plan has something to cut.
        (
            "object.move",
            json!({ "ids": [1], "dx_mm": 30.0, "dy_mm": 30.0 }),
        ),
        ("document.check", json!({})),
        ("job.preview", json!({})),
        (
            "document.preview",
            json!({ "kind": "canvas", "max_px": 128 }),
        ),
        ("document.preview", json!({ "kind": "path", "max_px": 128 })),
        ("document.preview", json!({ "kind": "path", "max_px": 1 })),
        (
            "document.export",
            json!({ "path": out.join("cut.svg"), "format": "svg", "scope": "all" }),
        ),
        ("object.import", json!({ "path": dir.join("nothing.png") })),
    ]
}

/// What a step returned, without the paths that differ between the two
/// runs.
fn comparable(r: Result<Value, CommandError>, dir: &std::path::Path) -> String {
    let text = match r {
        Ok(v) => v.to_string(),
        Err(e) => format!("error: {e}"),
    };
    let dir = serde_json::to_string(&dir.display().to_string()).unwrap();
    text.replace(dir.trim_matches('"'), "<dir>")
}

/// A document as text, its shapes included, with `dir` taken out.
fn doc_text(d: &flatcraft_core::document::Document, dir: &std::path::Path) -> String {
    let shapes: Vec<String> = d
        .objects
        .iter()
        .map(|o| format!("{:?}", o.geometry))
        .collect();
    let text = format!("{} {shapes:?}", serde_json::to_string(d).unwrap());
    let dir = serde_json::to_string(&dir.display().to_string()).unwrap();
    text.replace(dir.trim_matches('"'), "<dir>")
}

#[test]
fn results_and_errors_are_the_same_as_under_the_lock() {
    let a_dir = tempfile::tempdir().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let a = shared(a_dir.path());
    let b = shared(b_dir.path());
    let a_steps = steps(a_dir.path(), a_dir.path());
    let b_steps = steps(b_dir.path(), b_dir.path());
    for ((id, pa), (_, pb)) in a_steps.into_iter().zip(b_steps) {
        let locked = comparable(locked_run(&a, id, pa), a_dir.path());
        let unlocked = comparable(unlocked_run(&b, id, pb), b_dir.path());
        assert_eq!(locked, unlocked, "{id}");
        if id == "job.preview" {
            assert!(locked.contains(r#""ready":true"#), "{locked}");
        }
    }
    let (a, b) = (a.lock().unwrap(), b.lock().unwrap());
    let (da, db) = (a.doc(), b.doc());
    assert_eq!(
        doc_text(&da.document, a_dir.path()),
        doc_text(&db.document, b_dir.path())
    );
    assert_eq!(da.revision(), db.revision());
    assert_eq!(
        da.undo.undo_len(),
        db.undo.undo_len(),
        "one undo step per command"
    );
    assert_eq!(da.dirty, db.dirty);
    assert_eq!(da.selection, db.selection);
    assert_eq!(
        da.last_job().map(|j| j.job.bytes.clone()),
        db.last_job().map(|j| j.job.bytes.clone())
    );
    assert!(b.last_agent_activity.is_some());
}

#[test]
fn project_open_gives_the_same_result_as_under_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let s = shared(dir.path());
    unlocked_run(
        &s,
        "object.import",
        json!({ "path": square_png(dir.path()) }),
    )
    .unwrap();
    let file = dir.path().join("p.flatcraft");
    locked_run(&s, "project.save", json!({ "path": file })).unwrap();
    for params in [
        json!({ "path": file }),
        json!({ "path": dir.path().join("gone.flatcraft") }),
        json!({ "path": file, "extra": 1 }),
    ] {
        let a = shared(dir.path());
        let b = shared(dir.path());
        let locked = locked_run(&a, "project.open", params.clone());
        let unlocked = unlocked_run(&b, "project.open", params);
        assert_eq!(locked, unlocked);
        let (a, b) = (a.lock().unwrap(), b.lock().unwrap());
        assert_eq!(a.docs.len(), b.docs.len());
        assert_eq!(
            doc_text(&a.doc().document, dir.path()),
            doc_text(&b.doc().document, dir.path())
        );
        assert_eq!(a.doc().path, b.doc().path);
    }
}

#[test]
fn the_lock_is_free_while_long_commands_compute() {
    let dir = tempfile::tempdir().unwrap();
    let s = shared(dir.path());
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    set_hook(&s, move |s, id| {
        // The window takes the lock now: it is free.
        let guard = s
            .try_lock()
            .expect("the session lock is free while computing");
        drop(guard);
        log.lock().unwrap().push(id.to_string());
    });
    let png = square_png(dir.path());
    unlocked_run(&s, "object.import", json!({ "path": png })).unwrap();
    unlocked_run(
        &s,
        "object.retrace",
        json!({ "id": 1, "settings": { "threshold": 90 } }),
    )
    .unwrap();
    unlocked_run(&s, "document.preview", json!({ "kind": "path" })).unwrap();
    unlocked_run(&s, "job.preview", json!({})).unwrap();
    let file = dir.path().join("p.flatcraft");
    locked_run(&s, "project.save", json!({ "path": file })).unwrap();
    unlocked_run(&s, "project.open", json!({ "path": file })).unwrap();
    unlocked_run(
        &s,
        "document.export",
        json!({ "path": dir.path().join("a.svg"), "format": "svg" }),
    )
    .unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        [
            "object.import",
            "object.retrace",
            "document.preview",
            "job.preview",
            "project.open",
            "document.export"
        ]
    );
}

#[test]
fn a_change_meanwhile_is_not_stored_and_the_command_runs_again() {
    let dir = tempfile::tempdir().unwrap();
    let s = shared(dir.path());
    let id = add_svg(&s);
    let tries = Arc::new(AtomicUsize::new(0));
    let t = tries.clone();
    set_hook(&s, move |s, _| {
        // The person moves the object while the first try computes.
        if t.fetch_add(1, Ordering::SeqCst) == 0 {
            nudge(s, id);
        }
    });
    let png = square_png(dir.path());
    let r = unlocked_run(&s, "object.import", json!({ "path": png })).unwrap();
    assert_eq!(tries.load(Ordering::SeqCst), 2, "one more try");
    assert_eq!(objects(&s), 2);
    // The import, the move and the import: the move was kept.
    assert_eq!(undo_len(&s), 3);
    let g = s.lock().unwrap();
    let moved = g
        .doc()
        .document
        .get(flatcraft_core::document::ObjectId(id))
        .unwrap();
    assert!(
        (moved.transform.center_mm.x - 46.0).abs() < 1e-6,
        "{:?}",
        moved.transform
    );
    assert_eq!(
        g.doc().selection.iter().map(|i| i.0).collect::<Vec<_>>(),
        vec![r["id"].as_u64().unwrap()]
    );
}

#[test]
fn a_document_that_keeps_changing_gives_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let s = shared(dir.path());
    let id = add_svg(&s);
    let tries = Arc::new(AtomicUsize::new(0));
    let t = tries.clone();
    set_hook(&s, move |s, _| {
        t.fetch_add(1, Ordering::SeqCst);
        nudge(s, id);
    });
    let png = square_png(dir.path());
    for (command, params, what) in [
        (
            "object.import",
            json!({ "path": png }),
            "the file was imported",
        ),
        (
            "object.retrace",
            json!({ "id": id, "settings": {} }),
            "the image was traced",
        ),
        ("job.preview", json!({}), "the job was planned"),
        (
            "document.preview",
            json!({ "kind": "canvas" }),
            "the preview was drawn",
        ),
    ] {
        tries.store(0, Ordering::SeqCst);
        let before = undo_len(&s);
        let r = unlocked_run(&s, command, params);
        assert_eq!(r, Err(changed_error(what)), "{command}");
        assert_eq!(tries.load(Ordering::SeqCst), 2, "{command}");
        // Only the two moves are new steps.
        assert_eq!(undo_len(&s), before + 2, "{command}");
    }
    assert_eq!(objects(&s), 1, "nothing was imported");
    assert!(
        s.lock().unwrap().doc().last_job().is_none(),
        "no plan was stored"
    );
    let text = changed_error("the file was imported").to_string();
    assert!(!text.contains('\u{2014}'), "{text}");
}

#[test]
fn an_export_is_written_only_from_the_document_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let s = shared(dir.path());
    let id = add_svg(&s);
    let tries = Arc::new(AtomicUsize::new(0));
    let t = tries.clone();
    set_hook(&s, move |s, _| {
        // The person moves the object while the first try makes the file.
        if t.fetch_add(1, Ordering::SeqCst) == 0 {
            nudge(s, id);
        }
    });
    let path = dir.path().join("cut.svg");
    let r = unlocked_run(
        &s,
        "document.export",
        json!({ "path": path, "format": "svg" }),
    )
    .unwrap();
    assert_eq!(r["objects"], 1);
    assert_eq!(tries.load(Ordering::SeqCst), 2, "made once more");
    // The file shows the moved object: the same file as an export now.
    let now = dir.path().join("now.svg");
    locked_run(
        &s,
        "document.export",
        json!({ "path": now, "format": "svg" }),
    )
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&now).unwrap());

    // A document that keeps changing: no file at all.
    let t = tries.clone();
    set_hook(&s, move |s, _| {
        t.fetch_add(1, Ordering::SeqCst);
        nudge(s, id);
    });
    tries.store(0, Ordering::SeqCst);
    let other = dir.path().join("other.svg");
    let r = unlocked_run(
        &s,
        "document.export",
        json!({ "path": other, "format": "svg" }),
    );
    assert_eq!(r, Err(changed_error("the file was exported")));
    assert_eq!(tries.load(Ordering::SeqCst), 2);
    assert!(
        !other.exists(),
        "nothing built on an older state is written"
    );
}

#[test]
fn open_decides_where_the_project_goes_when_it_is_stored() {
    let dir = tempfile::tempdir().unwrap();
    let s = shared(dir.path());
    add_svg(&s);
    let file = dir.path().join("p.flatcraft");
    locked_run(&s, "project.save", json!({ "path": file })).unwrap();
    let s = shared(dir.path());
    assert!(s.lock().unwrap().doc().is_pristine());
    // The person imports into the empty document while the project loads:
    // the project must open next to it, never over it.
    set_hook(&s, |s, _| {
        s.lock()
            .unwrap()
            .execute(
                "object.import_svg",
                json!({ "svg": SVG, "name": "mine" }),
                Origin::User,
            )
            .unwrap();
    });
    let r = unlocked_run(&s, "project.open", json!({ "path": file })).unwrap();
    assert_eq!(r["index"], 1);
    let g = s.lock().unwrap();
    assert_eq!(g.docs.len(), 2);
    assert_eq!(g.docs[0].document.objects[0].name, "mine");
}

#[test]
fn open_runs_again_when_the_profiles_change() {
    let dir = tempfile::tempdir().unwrap();
    let s = shared(dir.path());
    add_svg(&s);
    let file = dir.path().join("p.flatcraft");
    locked_run(&s, "project.save", json!({ "path": file })).unwrap();
    let tries = Arc::new(AtomicUsize::new(0));
    let t = tries.clone();
    set_hook(&s, move |s, _| {
        t.fetch_add(1, Ordering::SeqCst);
        s.lock().unwrap().profiles.reload().unwrap();
    });
    let docs = s.lock().unwrap().docs.len();
    let r = unlocked_run(&s, "project.open", json!({ "path": file }));
    assert_eq!(r, Err(changed_error("the project was opened")));
    assert_eq!(tries.load(Ordering::SeqCst), 2);
    assert_eq!(s.lock().unwrap().docs.len(), docs, "nothing was opened");
}

#[test]
fn a_panic_while_computing_is_a_plain_error() {
    let dir = tempfile::tempdir().unwrap();
    let s = shared(dir.path());
    add_svg(&s);
    set_hook(&s, |_, _| panic!("test: the compute step panics"));
    let r = unlocked_run(&s, "document.preview", json!({ "kind": "canvas" }));
    assert!(
        matches!(&r, Err(CommandError::Failed(t)) if t.contains("internal error")),
        "{r:?}"
    );
    assert_eq!(objects(&s), 1);
    assert!(s.try_lock().is_ok(), "the lock is not poisoned");
}

#[test]
fn other_commands_run_under_the_lock() {
    assert!(unlocked::runs_unlocked("object.import"));
    assert!(unlocked::runs_unlocked("document.preview"));
    assert!(!unlocked::runs_unlocked("object.move"));
    assert!(!unlocked::runs_unlocked("job.send"));
    let dir = tempfile::tempdir().unwrap();
    let s = shared(dir.path());
    set_hook(&s, |_, id| panic!("{id} ran off the lock"));
    let id = add_svg(&s);
    unlocked_run(&s, "object.move", json!({ "ids": [id], "dx_mm": 1.0 })).unwrap();
    assert_eq!(
        unlocked_run(&s, "no.such", json!({})),
        Err(CommandError::UnknownCommand("no.such".into()))
    );
}

#[test]
fn a_save_that_finishes_meanwhile_leaves_the_import_unsaved() {
    let dir = tempfile::tempdir().unwrap();
    let s = shared(dir.path());
    let file = dir.path().join("p.flatcraft");
    locked_run(&s, "project.save", json!({ "path": file })).unwrap();
    add_svg(&s);
    assert!(s.lock().unwrap().doc().dirty);
    let saved = Arc::new(AtomicUsize::new(0));
    let n = saved.clone();
    let f = file.clone();
    set_hook(&s, move |s, _| {
        // The person saves while the agent's file is traced.
        if n.fetch_add(1, Ordering::SeqCst) == 0 {
            s.lock()
                .unwrap()
                .execute("project.save", json!({ "path": f }), Origin::User)
                .unwrap();
            assert!(!s.lock().unwrap().doc().dirty);
        }
    });
    unlocked_run(
        &s,
        "object.import",
        json!({ "path": square_png(dir.path()) }),
    )
    .unwrap();
    assert_eq!(
        saved.load(Ordering::SeqCst),
        1,
        "a save changes no revision"
    );
    let g = s.lock().unwrap();
    assert_eq!(g.doc().document.objects.len(), 2);
    assert!(g.doc().dirty, "the import is not in the saved file");
}
