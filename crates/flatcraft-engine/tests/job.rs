//! Job building, File Output and the send gate.

use std::path::{Path, PathBuf};

use flatcraft_device::gpgl::check_safe;
use flatcraft_device::{ProfileStore, TransportSpec};
use flatcraft_engine::gate::{GateDecision, check_send};
use flatcraft_engine::job::{output_dir, sanitize_name};
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::registry;
use flatcraft_engine::session::{CommandError, Origin, Session};
use serde_json::{Value, json};

/// A config dir and a separate output dir, both temporary.
struct Dirs {
    config: tempfile::TempDir,
    out: tempfile::TempDir,
}

fn dirs() -> Dirs {
    Dirs {
        config: tempfile::tempdir().unwrap(),
        out: tempfile::tempdir().unwrap(),
    }
}

fn prefs(d: &Dirs) -> Preferences {
    Preferences {
        output_dir: Some(d.out.path().to_path_buf()),
        ..Preferences::default()
    }
}

fn session_with(d: &Dirs, prefs: Preferences, has_ui: bool) -> Session {
    let profiles = ProfileStore::load(d.config.path()).unwrap();
    Session::new(profiles, prefs, has_ui, d.config.path().to_path_buf())
}

fn session(d: &Dirs) -> Session {
    session_with(d, prefs(d), false)
}

const SQUARE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="20" y="20" width="30" height="30"/></svg>"#;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

fn import_square(s: &mut Session, name: &str) -> u64 {
    let r = run(
        s,
        "object.import_svg",
        json!({ "svg": SQUARE, "name": name }),
    );
    r["id"].as_u64().unwrap()
}

fn files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|it| {
            it.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn sent_paths(r: &Value) -> Vec<PathBuf> {
    r["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| PathBuf::from(f.as_str().unwrap()))
        .collect()
}

// ---------------------------------------------------------------- end to end

#[test]
fn send_square_to_file_output_end_to_end() {
    let d = dirs();
    let mut s = session(&d);
    let id = import_square(&mut s, "square");
    let r = s
        .execute("job.send", json!({ "name": "square" }), Origin::Agent)
        .unwrap();
    let paths = sent_paths(&r);
    assert_eq!(paths.len(), 3);
    assert_eq!(paths[0], d.out.path().join("square.gpgl"));
    let bytes = std::fs::read(&paths[0]).unwrap();
    assert!(bytes.ends_with(b"H\x03"));
    check_safe(&bytes).unwrap();
    assert_eq!(r["bytes_sent"].as_u64().unwrap() as usize, bytes.len());

    let txt = std::fs::read_to_string(d.out.path().join("square.gpgl.txt")).unwrap();
    let note = format!("object {id} 'square', path 0, pass 1");
    assert!(txt.lines().any(|l| l.ends_with(&note)), "{txt}");

    // The session byte log has the job, and last_job remembers the send.
    let logged: Vec<u8> = s.byte_log.entries().flat_map(|e| e.bytes.clone()).collect();
    assert_eq!(logged, bytes);
    let last = s.doc().last_job.as_ref().unwrap();
    assert_eq!(last.job.bytes, bytes);
    assert_eq!(last.sent.as_ref().unwrap().files, paths);
    assert!(last.plan.cut_mm > 119.0, "{}", last.plan.cut_mm);
}

#[test]
fn default_name_comes_from_the_project_file() {
    let d = dirs();
    let mut s = session(&d);
    import_square(&mut s, "square");
    let r = run(&mut s, "job.send", json!({}));
    assert_eq!(sent_paths(&r)[0], d.out.path().join("flatcraft-job.gpgl"));

    let project = d.out.path().join("My Badge.flatcraft");
    run(&mut s, "project.save", json!({ "path": project }));
    let r = run(&mut s, "job.send", json!({}));
    assert_eq!(sent_paths(&r)[0], d.out.path().join("My_Badge.gpgl"));
    let r = run(&mut s, "job.send", json!({}));
    assert_eq!(sent_paths(&r)[0], d.out.path().join("My_Badge-2.gpgl"));
}

#[test]
fn preview_summarises_and_stores_last_job() {
    let d = dirs();
    let mut s = session(&d);
    import_square(&mut s, "square");
    let r = run(&mut s, "job.preview", json!({}));
    assert_eq!(r["toolpaths"], 1);
    assert!(r["cut_mm"].as_f64().unwrap() > 119.0, "{r}");
    assert!(r["travel_mm"].as_f64().unwrap() > 0.0, "{r}");
    assert!(r["warnings"].is_array());
    let listing = r["listing"].as_array().unwrap();
    assert!(!listing.is_empty() && listing.len() <= 200);
    assert_eq!(listing.last().unwrap()["command"], "H");
    assert_eq!(r["settings"]["speed"], 5);
    assert_eq!(r["settings"]["force"], 10);
    assert_eq!(r["settings"]["passes"], 1);
    let last = s.doc().last_job.as_ref().unwrap();
    assert!(last.sent.is_none());
    // Preview writes nothing.
    assert!(files(d.out.path()).is_empty());
}

#[test]
fn hidden_objects_are_not_cut_and_named_in_warnings() {
    let d = dirs();
    let mut s = session(&d);
    import_square(&mut s, "shown");
    let hidden = import_square(&mut s, "ghost");
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [hidden], "visible": false }),
    );
    let r = run(&mut s, "job.preview", json!({}));
    assert_eq!(r["toolpaths"], 1);
    let w = r["warnings"].to_string();
    assert!(w.contains("ghost"), "{w}");
}

#[test]
fn send_empty_document_is_refused_and_writes_nothing() {
    let d = dirs();
    let mut s = session(&d);
    let e = s.execute("job.send", json!({}), Origin::User).unwrap_err();
    assert_eq!(
        e,
        CommandError::Refused("Nothing to cut: the document has no visible objects.".into())
    );
    assert!(files(d.out.path()).is_empty());
    assert!(s.byte_log.is_empty());

    // Only hidden objects: the same.
    let id = import_square(&mut s, "square");
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [id], "visible": false }),
    );
    let e = s.execute("job.send", json!({}), Origin::User).unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "{e}");
    assert!(files(d.out.path()).is_empty());
}

#[test]
fn send_out_of_area_names_object() {
    let d = dirs();
    let mut s = session(&d);
    import_square(&mut s, "inside");
    let id = import_square(&mut s, "runaway");
    run(
        &mut s,
        "object.move",
        json!({ "ids": [id], "dx_mm": 1000.0 }),
    );
    let e = s.execute("job.send", json!({}), Origin::User).unwrap_err();
    let CommandError::Refused(msg) = &e else {
        panic!("{e}")
    };
    assert_eq!(
        msg,
        "These objects are outside the cutting area: runaway. Move or scale them, or pick a bigger mat."
    );
    assert!(files(d.out.path()).is_empty());
}

#[test]
fn overrides_reach_the_emitter() {
    let d = dirs();
    let mut s = session(&d);
    import_square(&mut s, "square");
    run(
        &mut s,
        "job.set_overrides",
        json!({ "speed": 3, "force": 20, "passes": 2 }),
    );
    let r = run(&mut s, "job.send", json!({ "name": "fast" }));
    let bytes = std::fs::read(&sent_paths(&r)[0]).unwrap();
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    assert!(has(b"!3\x03"), "{}", String::from_utf8_lossy(&bytes));
    assert!(has(b"FX20\x03"), "{}", String::from_utf8_lossy(&bytes));
    let last = s.doc().last_job.as_ref().unwrap();
    assert_eq!(last.settings.speed, 3);
    assert_eq!(last.settings.force, 20);
    assert!(last.plan.toolpaths.iter().any(|t| t.pass == 2));
    let txt = std::fs::read_to_string(&sent_paths(&r)[1]).unwrap();
    assert!(txt.contains("pass 2"), "{txt}");
}

#[test]
fn notes_carry_cut_path_labels_and_no_line_breaks() {
    let d = dirs();
    let mut s = session(&d);
    let id = import_square(&mut s, "two\nlines");
    {
        let o = s
            .doc_mut()
            .document
            .get_mut(flatcraft_core::document::ObjectId(id))
            .unwrap();
        let mut paths = (*o.geometry).clone();
        paths[0].label = Some("outer".into());
        o.set_geometry(paths);
    }
    let r = run(&mut s, "job.send", json!({ "name": "labels" }));
    let txt = std::fs::read_to_string(&sent_paths(&r)[1]).unwrap();
    let note = format!("object {id} 'two lines', path 0 [outer], pass 1");
    assert!(txt.lines().any(|l| l.ends_with(&note)), "{txt}");
}

// ---------------------------------------------------------------- names and dirs

#[test]
fn names_are_sanitised() {
    assert_eq!(sanitize_name("square"), "square");
    assert_eq!(sanitize_name("My Badge v2.1"), "My_Badge_v2.1");
    assert_eq!(sanitize_name("../../mcp-token"), "_.._mcp-token");
    assert_eq!(sanitize_name("..\\x"), "_x");
    assert_eq!(sanitize_name("..."), "flatcraft-job");
    assert_eq!(sanitize_name(""), "flatcraft-job");
    assert_eq!(sanitize_name("/etc/passwd"), "_etc_passwd");
    assert_eq!(sanitize_name(&"a".repeat(300)).len(), 100);
    for n in ["a/b", "a\\b", "a:b", "a\0b", "\u{e9}t\u{e9}"] {
        let s = sanitize_name(n);
        assert!(
            s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
            "{n} -> {s}"
        );
        assert!(!s.starts_with('.'));
    }
}

#[test]
fn output_dir_fallback_chain() {
    let cfg = Path::new("/cfg");
    let p = Path::new("/profile");
    let pref = Path::new("/pref");
    let docs = Path::new("/docs");
    assert_eq!(output_dir(Some(p), Some(pref), Some(docs), cfg), p);
    assert_eq!(output_dir(None, Some(pref), Some(docs), cfg), pref);
    assert_eq!(
        output_dir(None, None, Some(docs), cfg),
        docs.join("FlatCraft Output")
    );
    assert_eq!(output_dir(None, None, None, cfg), cfg.join("output"));
}

#[test]
fn agent_job_names_never_reach_the_config_dir() {
    let d = dirs();
    let mut s = session(&d);
    import_square(&mut s, "square");
    for name in ["../mcp-token", "../../preferences", "/etc/passwd", ".."] {
        let r = s
            .execute("job.send", json!({ "name": name }), Origin::Agent)
            .unwrap();
        for f in sent_paths(&r) {
            assert_eq!(f.parent().unwrap(), d.out.path(), "{name}: {}", f.display());
        }
    }
    assert!(!d.config.path().join("mcp-token").exists());
    assert!(
        files(d.config.path()).is_empty(),
        "{:?}",
        files(d.config.path())
    );
}

/// A user plotter preset that writes File Output into `dir`.
fn file_plotter_into(config: &Path, dir: &Path, ext: &str) {
    let plotters = config.join("plotters");
    std::fs::create_dir_all(&plotters).unwrap();
    std::fs::write(
        plotters.join("into-config.toml"),
        format!(
            "id = \"into-config\"\nname = \"Into config\"\ninherits = \"silhouette-cameo-2\"\ntransport = {{ File = {{ directory = {:?}, extension = \"{ext}\" }} }}\n",
            dir.to_str().unwrap()
        ),
    )
    .unwrap();
}

#[test]
fn agent_file_output_into_config_dir_is_refused() {
    let d = dirs();
    file_plotter_into(d.config.path(), d.config.path(), "toml");
    let mut s = session(&d);
    import_square(&mut s, "square");
    run(&mut s, "job.set_plotter", json!({ "id": "into-config" }));
    let before = files(d.config.path());
    let e = s
        .execute("job.send", json!({ "name": "preferences" }), Origin::Agent)
        .unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "{e}");
    assert_eq!(files(d.config.path()), before);
    // The person may do it.
    s.execute("job.send", json!({ "name": "mine" }), Origin::User)
        .unwrap();
}

#[test]
fn agent_may_use_the_config_output_fallback() {
    let d = dirs();
    let out = d.config.path().join("output");
    file_plotter_into(d.config.path(), &out, "gpgl");
    let mut s = session(&d);
    import_square(&mut s, "square");
    run(&mut s, "job.set_plotter", json!({ "id": "into-config" }));
    let r = s
        .execute("job.send", json!({ "name": "job" }), Origin::Agent)
        .unwrap();
    assert_eq!(sent_paths(&r)[0], out.join("job.gpgl"));
}

#[test]
fn agent_cannot_write_into_config_dir() {
    let d = dirs();
    let mut s = session(&d);
    import_square(&mut s, "square");
    let token = d.config.path().join("mcp-token");
    let e = s
        .execute("project.save", json!({ "path": token }), Origin::Agent)
        .unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "{e}");
    assert!(!token.exists());
}

// ---------------------------------------------------------------- gate

fn usb() -> TransportSpec {
    TransportSpec::Usb {
        vid: 0x0b4d,
        pid: 0x112b,
    }
}

fn file() -> TransportSpec {
    TransportSpec::File {
        directory: None,
        extension: "gpgl".into(),
    }
}

fn with_agent_cuts() -> Preferences {
    Preferences {
        allow_agent_cuts: true,
        ..Preferences::default()
    }
}

#[test]
fn agent_headless_usb_refused() {
    assert_eq!(
        check_send(Origin::Agent, &usb(), &Preferences::default(), false),
        GateDecision::Refuse(
            "Agents may not start USB cuts in a headless session. Turn on 'Allow agents to start cuts' in the preferences.".into()
        )
    );
}

#[test]
fn agent_ui_usb_needs_confirmation() {
    assert_eq!(
        check_send(Origin::Agent, &usb(), &Preferences::default(), true),
        GateDecision::NeedsConfirmation
    );
}

#[test]
fn agent_with_pref_allowed() {
    for ui in [false, true] {
        assert_eq!(
            check_send(Origin::Agent, &usb(), &with_agent_cuts(), ui),
            GateDecision::Allow
        );
    }
}

#[test]
fn agent_file_always_allowed() {
    for ui in [false, true] {
        for p in [Preferences::default(), with_agent_cuts()] {
            assert_eq!(
                check_send(Origin::Agent, &file(), &p, ui),
                GateDecision::Allow
            );
        }
    }
}

#[test]
fn user_usb_allowed() {
    for ui in [false, true] {
        assert_eq!(
            check_send(Origin::User, &usb(), &Preferences::default(), ui),
            GateDecision::Allow
        );
    }
}

// ---------------------------------------------------------------- USB and confirmation

#[test]
fn usb_send_reports_not_available_in_m1() {
    let d = dirs();
    let mut s = session(&d);
    import_square(&mut s, "square");
    run(
        &mut s,
        "job.set_plotter",
        json!({ "id": "silhouette-cameo-2" }),
    );
    let e = s.execute("job.send", json!({}), Origin::User).unwrap_err();
    assert_eq!(
        e,
        CommandError::Refused("USB cutting arrives in milestone 3.".into())
    );
    assert!(files(d.out.path()).is_empty());
    assert!(s.byte_log.is_empty());
}

#[test]
fn agent_headless_usb_send_is_refused_by_the_gate() {
    let d = dirs();
    let mut s = session(&d);
    import_square(&mut s, "square");
    run(
        &mut s,
        "job.set_plotter",
        json!({ "id": "silhouette-cameo-2" }),
    );
    let e = s.execute("job.send", json!({}), Origin::Agent).unwrap_err();
    let CommandError::Refused(msg) = &e else {
        panic!("{e}")
    };
    assert!(msg.contains("headless"), "{msg}");
    assert!(s.pending_send.is_none());
}

#[test]
fn agent_with_pref_reaches_the_usb_transport() {
    let d = dirs();
    let mut p = prefs(&d);
    p.allow_agent_cuts = true;
    let mut s = session_with(&d, p, false);
    import_square(&mut s, "square");
    run(
        &mut s,
        "job.set_plotter",
        json!({ "id": "silhouette-cameo-2" }),
    );
    let e = s.execute("job.send", json!({}), Origin::Agent).unwrap_err();
    assert_eq!(
        e,
        CommandError::Refused("USB cutting arrives in milestone 3.".into())
    );
    assert!(s.pending_send.is_none());
}

#[test]
fn agent_cannot_self_confirm() {
    let d = dirs();
    let mut s = session_with(&d, prefs(&d), true);
    import_square(&mut s, "square");
    run(
        &mut s,
        "job.set_plotter",
        json!({ "id": "silhouette-cameo-2" }),
    );
    let e = s
        .execute("job.send", json!({ "confirmed": true }), Origin::Agent)
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
    assert!(e.to_string().contains("confirmed"), "{e}");
    assert!(s.pending_send.is_none());
    assert!(s.byte_log.is_empty());
    assert!(files(d.out.path()).is_empty());

    // Without it, the send waits for the person.
    let e = s
        .execute("job.send", json!({ "name": "wait" }), Origin::Agent)
        .unwrap_err();
    assert_eq!(
        e,
        CommandError::Refused("Waiting for confirmation in the FlatCraft window.".into())
    );
    let pending = s.pending_send.clone().unwrap();
    assert_eq!(pending.name, "wait");
    assert!(s.byte_log.is_empty());
    assert!(files(d.out.path()).is_empty());

    // Sending again replaces the pending request; it never sends.
    s.execute("job.send", json!({ "name": "again" }), Origin::Agent)
        .unwrap_err();
    let newer = s.pending_send.clone().unwrap();
    assert_ne!(newer.id, pending.id);
    assert_eq!(newer.name, "again");

    // No registry command confirms.
    assert!(registry::all().iter().all(|c| !c.id().contains("confirm")));

    // A wrong id changes nothing.
    let e = s.confirm_pending(pending.id).unwrap_err();
    assert!(matches!(e, CommandError::NotFound(_)), "{e}");
    assert_eq!(s.pending_send.as_ref(), Some(&newer));

    // The GUI confirms: it runs as the user and reaches the USB transport.
    let e = s.confirm_pending(newer.id).unwrap_err();
    assert_eq!(
        e,
        CommandError::Refused("USB cutting arrives in milestone 3.".into())
    );
    assert!(s.pending_send.is_none());
    let e = s.confirm_pending(newer.id).unwrap_err();
    assert!(matches!(e, CommandError::NotFound(_)), "{e}");
}

// ---------------------------------------------------------------- preferences

#[test]
fn no_command_changes_preferences() {
    let d = dirs();
    for origin in [Origin::Agent, Origin::User] {
        for ui in [false, true] {
            let before = prefs(&d);
            let mut s = session_with(&d, before.clone(), ui);
            import_square(&mut s, "square");
            for c in registry::all() {
                let _ = s.execute(c.id(), json!({}), origin);
            }
            assert_eq!(s.prefs, before);
            assert!(!d.config.path().join("preferences.toml").exists());
        }
    }
}

#[test]
fn preferences_output_dir_roundtrips() {
    let d = dirs();
    let p = prefs(&d);
    p.save(d.config.path()).unwrap();
    assert_eq!(Preferences::load(d.config.path()), p);
    let none = Preferences::default();
    none.save(d.config.path()).unwrap();
    assert_eq!(Preferences::load(d.config.path()).output_dir, None);
}
