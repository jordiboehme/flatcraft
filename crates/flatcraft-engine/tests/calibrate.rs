//! The host blade offset calibration series (`tools.offset_series`): one
//! tile per offset in a row, planned through the one emit path and sent
//! through the one send path, and the simulation that ranks the tiles.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use flatcraft_device::ProfileStore;
use flatcraft_device::gpgl::check_safe;
use flatcraft_engine::calibrate::{self, NEEDS_HOST, TILE_MM, TILE_PITCH_MM};
use flatcraft_engine::job::{JobInput, WAITING};
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{CommandError, Origin, Session, UnlockedHook};
use flatcraft_engine::unlocked;
use flatcraft_path::AppliedCompensation;
use serde_json::{Value, json};

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

fn session_with(d: &Dirs, has_ui: bool) -> Session {
    let profiles = ProfileStore::load(d.config.path()).unwrap();
    let prefs = Preferences {
        output_dir: Some(d.out.path().to_path_buf()),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, has_ui, d.config.path().to_path_buf())
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

/// A user plotter preset `id` that inherits `parent`, with host
/// compensation at `offset` and a blade of `blade` mm.
fn host_plotter(config: &Path, id: &str, parent: &str, offset: f64, blade: f64) {
    let plotters = config.join("plotters");
    std::fs::create_dir_all(&plotters).unwrap();
    std::fs::write(
        plotters.join(format!("{id}.toml")),
        format!(
            "id = \"{id}\"\nname = \"{id}\"\ninherits = \"{parent}\"\nknife_compensation = {{ Host = {{ offset_mm = {offset:?}, pivot_threshold_deg = 0.0 }} }}\nblade_offset_mm = {blade:?}\n"
        ),
    )
    .unwrap();
}

/// A session on a File Output plotter with host compensation.
fn host_session(d: &Dirs, blade: f64) -> Session {
    host_plotter(
        d.config.path(),
        "host",
        "file-output-gpgl-cameo2",
        blade,
        blade,
    );
    let mut s = session_with(d, false);
    run(&mut s, "job.set_plotter", json!({ "id": "host" }));
    s
}

/// A session on the USB Cameo 2 with host compensation.
fn usb_host_session(d: &Dirs, has_ui: bool) -> Session {
    host_plotter(d.config.path(), "usb-host", "silhouette-cameo-2", 0.3, 0.3);
    let mut s = session_with(d, has_ui);
    run(&mut s, "job.set_plotter", json!({ "id": "usb-host" }));
    s
}

fn cutting_area(s: &mut Session) -> kurbo::Rect {
    let a = &run(s, "document.state", json!({}))["cutting_area_mm"];
    let f = |k: &str| a[k].as_f64().unwrap();
    kurbo::Rect::new(f("x0"), f("y0"), f("x1"), f("y1"))
}

fn files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn bytes_have(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle)
}

const OFFSETS: [f64; 3] = [0.2, 0.25, 0.3];

#[test]
fn offset_series_builds_one_tile_per_offset() {
    let d = dirs();
    let mut s = host_session(&d, 0.25);
    let area = cutting_area(&mut s);
    let r = calibrate::offset_series(&s, &OFFSETS).unwrap();
    assert_eq!(r.plan.compensation, AppliedCompensation::HostCalibration);
    let mut objects: Vec<u64> = r.plan.toolpaths.iter().map(|t| t.source.object.0).collect();
    objects.dedup();
    assert_eq!(
        objects,
        vec![1, 2, 3],
        "the tiles are cut one after another"
    );
    // Every tile's intended path lies in its own 25 mm tile, the row
    // starting at the cutting area's top-left, 30 mm apart.
    for t in &r.plan.toolpaths {
        let i = (t.source.object.0 - 1) as f64;
        let tile = kurbo::Rect::from_origin_size(
            (area.x0 + TILE_PITCH_MM * i, area.y0),
            (TILE_MM, TILE_MM),
        );
        for p in &t.points {
            assert!(tile.contains(*p), "{p:?} outside tile {i}: {tile:?}");
        }
    }
    let notes: Vec<&str> = r.job.listing.iter().map(|l| l.note.as_str()).collect();
    assert!(
        notes.iter().any(|n| n.contains("Offset 0.25 mm")),
        "the listing names each tile: {notes:?}"
    );

    // Structural: each tile is planned at its offset, from where the tile
    // before ended and with its last heading.
    let input = calibrate::prepare(&s, &OFFSETS, false, Origin::User).unwrap();
    let series = calibrate::build(&input).unwrap();
    assert_eq!(series.tiles.len(), 3);
    for (t, o) in series.tiles.iter().zip(OFFSETS) {
        assert_eq!(t.offset_mm, o);
    }
    for w in series.tiles.windows(2) {
        assert_eq!(w[1].plan.home, w[0].plan.end);
        assert_eq!(w[1].start_heading, w[0].plan.end_heading);
    }
    assert_eq!(series.job.job.bytes, r.job.bytes, "one build, one result");

    // The command says the same, per tile.
    let v = run(
        &mut s,
        "tools.offset_series",
        json!({ "offsets_mm": OFFSETS }),
    );
    let tiles = v["tiles"].as_array().unwrap();
    assert_eq!(tiles.len(), 3);
    for (t, o) in tiles.iter().zip(OFFSETS) {
        assert_eq!(t["offset_mm"], o);
        assert!(t["max_hausdorff_mm"].as_f64().is_some(), "{t}");
        assert!(t["grade"].is_string(), "{t}");
    }
    assert_eq!(v["sent"], Value::Null);
    assert_eq!(v["toolpaths"], r.toolpaths_cut());
    assert!(
        files(d.out.path()).is_empty(),
        "nothing is sent without send"
    );
    assert!(s.doc().document.objects.is_empty(), "nothing is added");
}

#[test]
fn offset_series_needs_host_profile() {
    let d = dirs();
    let mut s = session_with(&d, false);
    let e = calibrate::offset_series(&s, &OFFSETS).unwrap_err();
    assert_eq!(e, CommandError::Refused(NEEDS_HOST.into()));
    let e = s
        .execute(
            "tools.offset_series",
            json!({ "offsets_mm": OFFSETS }),
            Origin::Agent,
        )
        .unwrap_err();
    assert_eq!(e, CommandError::Refused(NEEDS_HOST.into()));
    assert!(!NEEDS_HOST.contains('\u{2014}'));
}

#[test]
fn offset_series_checks_its_offsets() {
    let d = dirs();
    let mut s = host_session(&d, 0.3);
    for bad in [
        json!([0.3]),
        json!([0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8]),
        json!([0.3, 0.04]),
        json!([0.3, 1.1]),
    ] {
        let e = s
            .execute(
                "tools.offset_series",
                json!({ "offsets_mm": bad }),
                Origin::Agent,
            )
            .unwrap_err();
        assert!(matches!(e, CommandError::InvalidParams(_)), "{bad}: {e}");
    }
    // The bounds themselves are fine.
    run(
        &mut s,
        "tools.offset_series",
        json!({ "offsets_mm": [0.05, 1.0] }),
    );
}

/// The device simulation with a blade of 0.3 mm: the tile planned for
/// 0.3 mm strays least.
#[test]
fn series_simulation_prefers_the_real_offset() {
    let d = dirs();
    let mut s = host_session(&d, 0.3);
    let offsets = [0.2, 0.25, 0.3, 0.35, 0.4];
    let v = run(
        &mut s,
        "tools.offset_series",
        json!({ "offsets_mm": offsets }),
    );
    println!("offset_mm | max_hausdorff_mm | grade | score | rank");
    for t in v["tiles"].as_array().unwrap() {
        println!(
            "{} | {} | {} | {} | {}",
            t["offset_mm"], t["max_hausdorff_mm"], t["grade"], t["score"], t["rank"]
        );
    }
    assert_eq!(v["blade_offset_mm"], 0.3);
    assert_eq!(v["best_offset_mm"], 0.3);
    let tiles = v["tiles"].as_array().unwrap();
    let h = |i: usize| tiles[i]["max_hausdorff_mm"].as_f64().unwrap();
    let best = (0..tiles.len())
        .min_by(|&a, &b| h(a).total_cmp(&h(b)))
        .unwrap();
    assert_eq!(tiles[best]["offset_mm"], 0.3);
    assert_eq!(tiles[best]["rank"], 1);
    // The farther the offset from the blade, the more it strays.
    assert!(h(0) > h(1) && h(1) > h(2), "{v}");
    assert!(h(4) > h(3) && h(3) > h(2), "{v}");
}

#[test]
fn offset_series_sends_to_file_output() {
    let d = dirs();
    let mut s = host_session(&d, 0.3);
    let v = s
        .execute(
            "tools.offset_series",
            json!({ "offsets_mm": OFFSETS, "send": true }),
            Origin::Agent,
        )
        .unwrap();
    let sent = &v["sent"];
    assert!(sent["bytes_sent"].as_u64().unwrap() > 0, "{v}");
    assert!(sent["cut_log_entry"].is_string(), "{v}");
    assert!(
        files(d.out.path())
            .iter()
            .any(|f| f == "offset-series.gpgl"),
        "{:?}",
        files(d.out.path())
    );
    let bytes = std::fs::read(d.out.path().join("offset-series.gpgl")).unwrap();
    check_safe(&bytes).unwrap();
    assert!(
        bytes_have(&bytes, b"FC0\x03"),
        "host compensation sends FC0"
    );
    assert!(bytes.ends_with(b"H\x03"));
    let last = s.doc().last_sent().unwrap();
    assert_eq!(last.plan.compensation, AppliedCompensation::HostCalibration);
}

#[test]
fn agent_usb_offset_series_headless_is_refused() {
    let d = dirs();
    let s = Arc::new(Mutex::new(usb_host_session(&d, false)));
    let params = json!({ "offsets_mm": OFFSETS, "send": true });
    let e =
        unlocked::execute(&s, "tools.offset_series", params.clone(), Origin::Agent).unwrap_err();
    let refusal = CommandError::Refused(
        "Agents may not start USB cuts in a headless session. Turn on 'Allow agents to start cuts' in the preferences.".into(),
    );
    assert_eq!(e, refusal);
    let mut s = s.lock().unwrap();
    assert_eq!(
        s.execute("tools.offset_series", params, Origin::Agent),
        Err(refusal)
    );
    assert!(s.pending_send().is_none());
    // Without send there is nothing to refuse.
    s.execute(
        "tools.offset_series",
        json!({ "offsets_mm": OFFSETS }),
        Origin::Agent,
    )
    .unwrap();
}

/// The MCP server runs the series off the lock; the send that waits for
/// the person is the real session's pending send, not a snapshot's.
#[test]
fn agent_usb_offset_series_with_window_waits() {
    let d = dirs();
    let s = Arc::new(Mutex::new(usb_host_session(&d, true)));
    let e = unlocked::execute(
        &s,
        "tools.offset_series",
        json!({ "offsets_mm": OFFSETS, "send": true }),
        Origin::Agent,
    )
    .unwrap_err();
    assert_eq!(e, CommandError::Refused(WAITING.into()));
    let s = s.lock().unwrap();
    let pending = s.pending_send().expect("the send waits for the person");
    assert_eq!(pending.doc, s.doc().id());
    assert_eq!(pending.plotter_id, "usb-host");
    assert_eq!(pending.name, calibrate::SERIES_NAME);
    check_safe(&pending.job.bytes).unwrap();
    assert!(bytes_have(&pending.job.bytes, b"FC0\x03"));
    assert!(files(d.out.path()).is_empty());
}

/// The series plans and simulates without the session lock: the hook
/// takes the lock in the compute step, which would deadlock otherwise.
#[test]
fn offset_series_runs_off_the_lock() {
    assert!(unlocked::runs_unlocked("tools.offset_series"));
    let d = dirs();
    let s = Arc::new(Mutex::new(host_session(&d, 0.3)));
    let calls = Arc::new(AtomicUsize::new(0));
    let weak = Arc::downgrade(&s);
    let seen = calls.clone();
    s.lock().unwrap().unlocked_hook = Some(UnlockedHook(Arc::new(move |id: &str| {
        assert_eq!(id, "tools.offset_series");
        if let Some(s) = weak.upgrade() {
            s.lock()
                .unwrap()
                .execute("document.state", json!({}), Origin::Agent)
                .unwrap();
            seen.fetch_add(1, Ordering::SeqCst);
        }
    })));
    let v = unlocked::execute(
        &s,
        "tools.offset_series",
        json!({ "offsets_mm": OFFSETS }),
        Origin::Agent,
    )
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(v["doc_id"], s.lock().unwrap().doc().id());
    assert_eq!(v["tiles"].as_array().unwrap().len(), 3);
}

/// A document that changes while the series is planned: the series runs
/// once more on the new state, and a second change drops it.
#[test]
fn offset_series_is_planned_again_when_the_document_changes() {
    let d = dirs();
    let s = Arc::new(Mutex::new(host_session(&d, 0.3)));
    let calls = Arc::new(AtomicUsize::new(0));
    let weak = Arc::downgrade(&s);
    let seen = calls.clone();
    s.lock().unwrap().unlocked_hook = Some(UnlockedHook(Arc::new(move |_: &str| {
        if let Some(s) = weak.upgrade() {
            seen.fetch_add(1, Ordering::SeqCst);
            s.lock()
                .unwrap()
                .execute(
                    "object.import_svg",
                    json!({ "svg": SQUARE, "name": "sq" }),
                    Origin::User,
                )
                .unwrap();
        }
    })));
    let e = unlocked::execute(
        &s,
        "tools.offset_series",
        json!({ "offsets_mm": OFFSETS, "send": true }),
        Origin::Agent,
    )
    .unwrap_err();
    assert_eq!(
        e,
        unlocked::changed_error("the offset series was planned"),
        "{e}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(
        files(d.out.path()).is_empty(),
        "a stale series is never sent"
    );
}

const SQUARE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="40mm" viewBox="0 0 40 40"><rect x="0" y="0" width="40" height="40"/></svg>"#;

/// The row keeps 5 mm away from the document's objects, hidden ones too,
/// and the refusal says why when there is no room.
#[test]
fn offset_series_avoids_the_objects_on_the_mat() {
    let d = dirs();
    let mut s = host_session(&d, 0.3);
    let id = run(
        &mut s,
        "object.import_svg",
        json!({ "svg": SQUARE, "name": "sq" }),
    )["id"]
        .as_u64()
        .unwrap();
    let area = cutting_area(&mut s);
    run(
        &mut s,
        "object.move",
        json!({ "ids": [id], "x_mm": area.x0 + 20.0, "y_mm": area.y0 }),
    );
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [id], "visible": false }),
    );
    let input = JobInput::of(&s).unwrap();
    let taken = flatcraft_engine::report::object_bounds(&input, &input.objects[0]).unwrap();
    let v = run(
        &mut s,
        "tools.offset_series",
        json!({ "offsets_mm": OFFSETS }),
    );
    for t in v["tiles"].as_array().unwrap() {
        let b = &t["bounds_mm"];
        let f = |k: &str| b[k].as_f64().unwrap();
        let tile = kurbo::Rect::new(f("x0"), f("y0"), f("x1"), f("y1"));
        let near = taken.inflate(4.999, 4.999);
        assert!(
            near.intersect(tile).is_zero_area(),
            "{tile:?} too close to {taken:?}"
        );
        assert!(tile.y0 >= area.y0 && tile.x1 <= area.x1);
    }

    // A row of seven tiles does not fit on a mat full of objects.
    let big = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}mm" height="{h}mm" viewBox="0 0 {w} {h}"><rect x="0" y="0" width="{w}" height="{h}"/></svg>"#,
        w = area.width() - 1.0,
        h = area.height() - 1.0
    );
    let mut s = host_session(&d, 0.3);
    run(
        &mut s,
        "object.import_svg",
        json!({ "svg": big, "name": "big" }),
    );
    let e = s
        .execute(
            "tools.offset_series",
            json!({ "offsets_mm": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7] }),
            Origin::Agent,
        )
        .unwrap_err();
    let CommandError::Refused(text) = e else {
        panic!("{e}")
    };
    assert!(
        text.starts_with("There is no free spot for the offset series"),
        "{text}"
    );
    assert!(!text.contains('\u{2014}'));
}

#[test]
fn the_tile_holds_the_brief_shapes() {
    let svg = calibrate::offset_tile_svg();
    assert!(svg.contains(r#"width="25mm" height="25mm""#), "{svg}");
    for id in ["square-10", "star-90", "star-30", "circles"] {
        assert!(svg.contains(&format!(r#"id="{id}""#)), "{id}: {svg}");
    }
    assert!(svg.contains(r#"width="10" height="10""#));
    assert!(svg.contains(r#"r="0.5""#) && svg.contains(r#"r="1""#));
}
