use flatcraft_device::ProfileStore;
use flatcraft_device::gpgl::quantize::to_device;
use flatcraft_engine::simulate::device_grid;
use kurbo::Point;

#[test]
fn device_grid_snaps_like_the_emitter_with_a_grid_origin() {
    let dir = tempfile::tempdir().unwrap();
    let store = ProfileStore::load(dir.path()).unwrap();
    let plotter = store.plotter("file-output-gpgl-cameo2").unwrap();
    assert!(!plotter.axis.mirror_x && !plotter.axis.mirror_y);
    assert_eq!((plotter.scale_x, plotter.scale_y), (1.0, 1.0));
    let mut media = plotter.media[0].clone();
    media.grid.origin_x_mm = 3.33;
    media.grid.origin_y_mm = 7.12;
    let grid = device_grid(&plotter, &media);
    assert_eq!(grid.units_per_mm, plotter.units_per_mm);
    // a deterministic spread of points over the mat, including half-step cases
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut points: Vec<Point> = (0..1000)
        .map(|_| Point::new(next() * 290.0, next() * 290.0))
        .collect();
    points.push(Point::new(0.0, 0.0));
    points.push(Point::new(0.025 - 0.03, 0.025 - 0.02));
    for p in points {
        let snapped = grid.snap(p);
        let dev = to_device(p, &plotter, &media);
        let upm = plotter.units_per_mm;
        let x = (snapped.x + media.grid.origin_x_mm) * upm;
        let y = (snapped.y + media.grid.origin_y_mm) * upm;
        assert!(
            (x - f64::from(dev.x)).abs() < 1e-9,
            "{p:?}: {snapped:?} vs {dev:?}"
        );
        assert!(
            (y - f64::from(dev.y)).abs() < 1e-9,
            "{p:?}: {snapped:?} vs {dev:?}"
        );
    }
}

// ------------------------------------------------------------ job.simulate

use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::preview::{PreviewKind, render_png};
use flatcraft_engine::session::{CommandError, Origin, Session, UnlockedHook};
use flatcraft_engine::simulate::{self, FIRMWARE_NOTE, SimGrid, SimMode};
use flatcraft_sim::{END_TO_END_LIMIT_MM, Grade, SpotKind};
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

fn session(d: &Dirs) -> Session {
    let profiles = ProfileStore::load(d.config.path()).unwrap();
    let prefs = Preferences {
        output_dir: Some(d.out.path().to_path_buf()),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, false, d.config.path().to_path_buf())
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

/// A user plotter preset `id` that inherits File Output, with `body`.
fn user_plotter(config: &Path, id: &str, body: &str) {
    let plotters = config.join("plotters");
    std::fs::create_dir_all(&plotters).unwrap();
    std::fs::write(
        plotters.join(format!("{id}.toml")),
        format!("id = \"{id}\"\nname = \"{id}\"\ninherits = \"file-output-gpgl-cameo2\"\n{body}"),
    )
    .unwrap();
}

/// A session on a plotter `id` with the knife compensation `knife` and a
/// blade offset of `blade` mm.
fn plotter_session(d: &Dirs, id: &str, knife: &str, blade: f64) -> Session {
    user_plotter(
        d.config.path(),
        id,
        &format!("knife_compensation = {knife}\nblade_offset_mm = {blade:?}\n"),
    );
    let mut s = session(d);
    run(&mut s, "job.set_plotter", json!({ "id": id }));
    s
}

const HOST: &str = "{ Host = { offset_mm = 0.3, pivot_threshold_deg = 0.0 } }";
const PEN: &str = "\"None\"";
const FIRMWARE: &str = "{ Firmware = { fc = 18 } }";

fn svg(body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200">{body}</svg>"#
    )
}

/// The mat colour of the presets.
const PAPER: [u8; 3] = [0xFB, 0xF4, 0xE4];

const SQUARE_50: &str = r#"<rect x="20" y="20" width="50" height="50"/>"#;

fn import(s: &mut Session, body: &str) -> u64 {
    run(
        s,
        "object.import_svg",
        json!({ "svg": svg(body), "name": "shape" }),
    )["id"]
        .as_u64()
        .unwrap()
}

#[test]
fn host_square_grades_good() {
    let d = dirs();
    let mut s = plotter_session(&d, "host", HOST, 0.3);
    import(&mut s, SQUARE_50);
    let r = run(&mut s, "job.simulate", json!({}));
    assert_eq!(r["mode"], "host", "{r}");
    assert_eq!(r["simulated"], true);
    assert_eq!(r["grade"], "good", "{r}");
    // The Good criterion is the end-to-end guarantee against the source.
    let h = r["metrics"]["max_hausdorff_mm"].as_f64().unwrap();
    assert!(h <= END_TO_END_LIMIT_MM, "{h}");
    assert_eq!(r["metrics"]["pushed"], 0);
    assert_eq!(r["metrics"]["max_into_kept_mm"], 0.0);
    assert_eq!(r["strategy"], "quality");
    assert_eq!(r["limits"]["end_to_end_mm"], END_TO_END_LIMIT_MM);
    assert!(r["estimate_s"].as_f64().unwrap() > 0.0);
    assert!(r.get("note").is_none(), "{r}");
    assert_eq!(r["doc_id"], s.doc().id());
    // The same counts as the job: one ring, one lift.
    assert_eq!(r["metrics"]["lifts"], 1);
    let stored = s.last_sim().expect("stored");
    assert_eq!(stored.mode, SimMode::Host);
    assert_eq!(stored.job.toolpaths_cut(), 1);
    assert!(!stored.display.is_empty());
}

#[test]
fn pen_mode_shows_uncompensated_corners() {
    let d = dirs();
    let mut s = plotter_session(&d, "pen", PEN, 0.3);
    import(&mut s, SQUARE_50);
    let r = run(&mut s, "job.simulate", json!({}));
    assert_eq!(r["mode"], "pen", "{r}");
    assert_eq!(r["simulated"], true);
    assert_ne!(r["grade"], "good", "{r}");
    assert!(r["metrics"]["max_corner_deviation_mm"].as_f64().unwrap() > 0.1);
    let w = r["warnings"].to_string();
    assert!(w.contains("no knife compensation"), "{w}");
    // A pen: no trailing tip, so nothing to say and a clean square.
    let d0 = dirs();
    let mut s = plotter_session(&d0, "pen0", PEN, 0.0);
    import(&mut s, SQUARE_50);
    let r = run(&mut s, "job.simulate", json!({}));
    assert_eq!(r["grade"], "good", "{r}");
    assert!(!r["warnings"].to_string().contains("no knife compensation"));
}

/// The open item of Task 51: a plain design in tip mode showed
/// max_into_kept_mm of about d. It is the drop point: the blade comes
/// down with the tip still trailing d behind the head along the travel,
/// and on a hole the travel came across the kept ring. A real knife does
/// that when nothing compensates (pen mode with a knife), so it is
/// reported; at d 0 (a pen) it is gone.
#[test]
fn pen_mode_into_kept_is_the_drop_point() {
    let d = dirs();
    let mut s = plotter_session(&d, "pen", PEN, 0.3);
    // A square with a hole: the hole is waste inside a kept piece.
    import(
        &mut s,
        r#"<path fill-rule="evenodd" d="M20 20h50v50h-50z M35 35h20v20h-20z"/>"#,
    );
    let run_ = simulate::current(&mut s, None).unwrap();
    let res = run_.result.as_ref().unwrap();
    let m = &res.metrics;
    let starts: Vec<kurbo::Point> = run_
        .plan_index
        .iter()
        .map(|&i| run_.job.plan.toolpaths[i].points[0])
        .collect();
    // The deepest nick is d: the tip lands d behind the drop point of
    // the hole, inside the kept ring.
    assert!(
        (m.max_into_kept_mm - 0.3).abs() < 1e-3,
        "{}",
        m.max_into_kept_mm
    );
    let spot = m.worst_spots[0];
    assert_eq!(spot.kind, SpotKind::IntoKept);
    let near_start = starts
        .iter()
        .map(|p| p.distance(spot.at))
        .fold(f64::INFINITY, f64::min);
    assert!(
        near_start <= 0.3 + 1e-6,
        "the deepest spot lies within d of a drop point: {near_start}"
    );
    // The other spots are the outer corners, which an uncompensated tip
    // rounds into the piece, about 0.14 mm deep at d 0.3.
    for x in &m.worst_spots[1..] {
        assert_eq!(x.kind, SpotKind::IntoKept);
        assert!(x.deviation_mm < 0.15, "{x:?}");
    }
    let d0 = dirs();
    let mut s = plotter_session(&d0, "pen0", PEN, 0.0);
    import(
        &mut s,
        r#"<path fill-rule="evenodd" d="M20 20h50v50h-50z M35 35h20v20h-20z"/>"#,
    );
    let run0 = simulate::current(&mut s, None).unwrap();
    assert_eq!(
        run0.result.as_ref().unwrap().metrics.max_into_kept_mm,
        0.0,
        "a pen never cuts into kept material"
    );
}

#[test]
fn firmware_mode_is_not_simulated() {
    let d = dirs();
    let mut s = plotter_session(&d, "fw", FIRMWARE, 0.25);
    import(&mut s, r#"<circle cx="50" cy="50" r="20"/>"#);
    let r = run(&mut s, "job.simulate", json!({}));
    assert_eq!(r["mode"], "firmware", "{r}");
    assert_eq!(r["simulated"], false);
    assert_eq!(r["note"], FIRMWARE_NOTE);
    assert!(r["score"].is_null() && r["grade"].is_null(), "{r}");
    let turn = r["max_turn_per_vertex_deg"].as_f64().unwrap();
    assert!(turn > 0.0 && turn < 30.0, "{turn}");
    assert!(r["metrics"]["segments"].as_u64().unwrap() > 10);
    assert_eq!(r["metrics"]["lifts"], 1);
    let keys: Vec<&String> = r["metrics"].as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        ["cut_mm", "lifts", "segments", "travel_mm"],
        "the subset"
    );
    assert!(r["estimate_s"].as_f64().unwrap() > 0.0);
    assert_eq!(r["worst_spots"], json!([]));
    let stored = s.last_sim().expect("stored");
    assert!(stored.result.is_none());
}

#[test]
fn compare_strategies_reports_three() {
    let d = dirs();
    let mut s = plotter_session(&d, "host", HOST, 0.3);
    import(
        &mut s,
        r#"<rect x="20" y="20" width="30" height="30"/><rect x="100" y="20" width="10" height="10"/><circle cx="60" cy="100" r="15"/>"#,
    );
    let r = run(&mut s, "job.compare_strategies", json!({}));
    let rows = r["strategies"].as_array().unwrap();
    let names: Vec<&str> = rows
        .iter()
        .map(|x| x["strategy"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["quality", "fast", "outlines_last"]);
    for row in rows {
        for k in [
            "score",
            "grade",
            "max_hausdorff_mm",
            "travel_mm",
            "travel_crossings",
            "segments",
            "lifts",
            "estimate_s",
        ] {
            assert!(!row[k].is_null(), "{k} in {row}");
        }
    }
    assert!(names.contains(&r["best_quality"].as_str().unwrap()), "{r}");
    let fastest = r["fastest"].as_str().unwrap();
    let min = rows
        .iter()
        .map(|x| x["estimate_s"].as_f64().unwrap())
        .fold(f64::INFINITY, f64::min);
    let fast_row = rows.iter().find(|x| x["strategy"] == fastest).unwrap();
    assert_eq!(fast_row["estimate_s"].as_f64().unwrap(), min);
    // The document's strategy did not change, and its run is stored.
    assert_eq!(s.doc().document.job.strategy, Default::default());
    assert_eq!(
        s.last_sim().unwrap().strategy,
        flatcraft_core::document::CutStrategy::Quality
    );
    // job.simulate with a strategy reads the same as its row.
    let fast = run(&mut s, "job.simulate", json!({ "strategy": "fast" }));
    assert_eq!(fast["strategy"], "fast");
    assert_eq!(fast["score"], rows[1]["score"]);
    // The old name is gone.
    let e = s
        .execute(
            "job.simulate",
            json!({ "strategy": "keep_sheet" }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
    // Firmware: nothing to rank by quality.
    let d2 = dirs();
    let mut s = plotter_session(&d2, "fw", FIRMWARE, 0.25);
    import(&mut s, SQUARE_50);
    let r = run(&mut s, "job.compare_strategies", json!({}));
    assert!(r["best_quality"].is_null(), "{r}");
    assert!(r["fastest"].is_string(), "{r}");
    assert!(r["strategies"][0]["grade"].is_null());
}

#[test]
fn stale_run_is_not_stored() {
    let d = dirs();
    let mut s = plotter_session(&d, "host", HOST, 0.3);
    let id = import(&mut s, SQUARE_50);
    let input = simulate::snapshot(&s, None, SimGrid::Device).unwrap();
    run(&mut s, "object.move", json!({ "ids": [id], "dx_mm": 5.0 }));
    let stale = Arc::new(simulate::compute(input).unwrap());
    assert!(!simulate::store(&mut s, stale));
    assert!(s.last_sim().is_none());
    // A fresh one is stored, and an edit makes it stale.
    let input = simulate::snapshot(&s, None, SimGrid::Device).unwrap();
    let fresh = Arc::new(simulate::compute(input).unwrap());
    assert!(simulate::store(&mut s, fresh.clone()));
    assert!(Arc::ptr_eq(s.last_sim().unwrap(), &fresh));
    run(&mut s, "object.move", json!({ "ids": [id], "dx_mm": 1.0 }));
    assert!(s.last_sim().is_none());
}

/// 2000 small squares in one object.
fn many_squares() -> String {
    let mut body = String::new();
    for i in 0..2000 {
        let (col, row) = (i % 50, i / 50);
        body.push_str(&format!(
            r#"<rect x="{}" y="{}" width="2" height="2"/>"#,
            5 + col * 4,
            5 + row * 4
        ));
    }
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="250mm" height="250mm" viewBox="0 0 250 250">{body}</svg>"#
    )
}

#[test]
fn simulate_does_not_hold_the_session_lock() {
    let d = dirs();
    let s = plotter_session(&d, "host", HOST, 0.3);
    let shared = Arc::new(Mutex::new(s));
    shared
        .lock()
        .unwrap()
        .execute(
            "object.import_svg",
            json!({ "svg": many_squares(), "name": "grid" }),
            Origin::User,
        )
        .unwrap();
    // The hook runs inside compute: it tells the test compute has begun
    // and waits until the test has used the session.
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let go_rx = Mutex::new(go_rx);
    let started_tx = Mutex::new(started_tx);
    shared.lock().unwrap().unlocked_hook = Some(UnlockedHook(Arc::new(move |id: &str| {
        assert_eq!(id, "job.simulate");
        started_tx.lock().unwrap().send(()).unwrap();
        go_rx.lock().unwrap().recv().unwrap();
    })));
    let input = simulate::snapshot(&shared.lock().unwrap(), None, SimGrid::Device).unwrap();
    let worker = std::thread::spawn(move || simulate::compute(input));
    started_rx.recv().unwrap();
    // compute is running now: the lock must be free within 50 ms.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
    let state = loop {
        if let Ok(mut s) = shared.try_lock() {
            break s
                .execute("document.state", json!({}), Origin::Agent)
                .unwrap();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the session lock was held during compute"
        );
        std::thread::yield_now();
    };
    assert_eq!(state["objects"].as_array().unwrap().len(), 1);
    go_tx.send(()).unwrap();
    let r = Arc::new(worker.join().unwrap().unwrap());
    assert_eq!(r.job.toolpaths_cut(), 2000);
    assert_eq!(r.result.as_ref().unwrap().metrics.lifts, 2000);
    let mut s = shared.lock().unwrap();
    s.unlocked_hook = None;
    assert!(simulate::store(&mut s, r));
}

/// The MCP server and the window run job.simulate off the lock
/// (`unlocked::execute`); its run is stored in the document all the same.
#[test]
fn unlocked_simulate_stores_the_run() {
    let d = dirs();
    let shared = Arc::new(Mutex::new(plotter_session(&d, "host", HOST, 0.3)));
    import(&mut shared.lock().unwrap(), SQUARE_50);
    assert!(flatcraft_engine::unlocked::runs_unlocked("job.simulate"));
    assert!(flatcraft_engine::unlocked::runs_unlocked(
        "job.compare_strategies"
    ));
    let r = flatcraft_engine::unlocked::execute(&shared, "job.simulate", json!({}), Origin::Agent)
        .unwrap();
    assert_eq!(r["grade"], "good");
    let s = shared.lock().unwrap();
    assert!(s.last_sim().is_some(), "adopted from the snapshot run");
}

/// The red, green and blue of pixel (x, y) of a PNG.
fn pixel(png: &[u8], x: u32, y: u32) -> [u8; 3] {
    let img = image::load_from_memory(png).unwrap().to_rgb8();
    let p = img.get_pixel(x, y);
    [p[0], p[1], p[2]]
}

#[test]
fn canvas_preview_draws_cut_geometry() {
    let d = dirs();
    let mut s = session(&d);
    import(
        &mut s,
        r#"<line x1="23" y1="53.5" x2="123" y2="53.5" stroke="black" stroke-width="3"/>"#,
    );
    let st = run(&mut s, "document.state", json!({}));
    let b = &st["objects"][0]["bounds_mm"];
    let png = render_png(&s, PreviewKind::Canvas, 2048).unwrap();
    let plotter = s
        .profiles
        .plotter(&s.doc().document.job.plotter_id)
        .unwrap();
    let media = plotter
        .media_or_first(&s.doc().document.job.media_id)
        .unwrap()
        .0;
    let scale = 2048.0 / media.width_mm.max(media.height_mm);
    let px = |mm_x: f64, mm_y: f64| {
        (
            ((mm_x + media.grid.origin_x_mm) * scale) as u32,
            ((mm_y + media.grid.origin_y_mm) * scale) as u32,
        )
    };
    // The line's centre in mat mm (it was placed by the import).
    let (cx, cy) = (
        (b["x0"].as_f64().unwrap() + b["x1"].as_f64().unwrap()) / 2.0,
        (b["y0"].as_f64().unwrap() + b["y1"].as_f64().unwrap()) / 2.0,
    );
    let mat = media.color_or(flatcraft_device::DEFAULT_MAT_COLOR_LIGHT);
    // Off the 10 mm grid lines of the mat.
    let cx = (cx / 10.0).floor() * 10.0 + 4.5;
    let (x, y) = px(cx, cy);
    assert_eq!(pixel(&png, x, y), mat, "the centre line is not drawn");
    let (x, y) = px(cx, cy + 1.5);
    assert_ne!(pixel(&png, x, y), mat, "the outline 1.5 mm off is drawn");
}

#[test]
fn simulation_preview_decodes() {
    for (knife, blade) in [(HOST, 0.3), (PEN, 0.3), (FIRMWARE, 0.25)] {
        let d = dirs();
        let mut s = plotter_session(&d, "p", knife, blade);
        import(&mut s, SQUARE_50);
        let png = render_png(&s, PreviewKind::Simulation, 512).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgb8();
        assert_eq!(img.width().max(img.height()), 512);
        let green = flatcraft_engine::preview::deviation_colour(PAPER, 0.0);
        let has_green = img.pixels().any(|p| [p[0], p[1], p[2]] == green);
        if knife == FIRMWARE {
            assert!(!has_green, "firmware draws the commanded path only");
        } else {
            assert!(has_green, "{knife}: the tip within the limit is green");
        }
        if knife == PEN {
            // The red corners are short: look at them big.
            let big = render_png(&s, PreviewKind::Simulation, 4096).unwrap();
            let big = image::load_from_memory(&big).unwrap().to_rgb8();
            let red = flatcraft_engine::preview::deviation_colour(PAPER, 1.0);
            assert!(
                big.pixels().any(|p| [p[0], p[1], p[2]] == red),
                "uncompensated corners are red"
            );
        }
        // document.preview stores the run it drew.
        run(
            &mut s,
            "document.preview",
            json!({ "kind": "simulation", "max_px": 128 }),
        );
        assert!(s.last_sim().is_some());
    }
}

#[test]
fn grade_names_follow_the_sim() {
    assert_eq!(simulate::grade_name(Grade::Good), "good");
    assert_eq!(simulate::grade_name(Grade::Fair), "fair");
    assert_eq!(simulate::grade_name(Grade::Poor), "poor");
}

/// The extent in px of the ring colour within `win` px of pixel (x, y).
fn ring_extent(png: &[u8], x: u32, y: u32, win: u32) -> (u32, u32) {
    let img = image::load_from_memory(png).unwrap().to_rgb8();
    let ring = flatcraft_engine::preview::RING;
    let (mut x0, mut x1, mut y0, mut y1) = (u32::MAX, 0, u32::MAX, 0);
    for py in y.saturating_sub(win)..(y + win).min(img.height()) {
        for px in x.saturating_sub(win)..(x + win).min(img.width()) {
            let p = img.get_pixel(px, py);
            if [p[0], p[1], p[2]] == ring {
                (x0, x1, y0, y1) = (x0.min(px), x1.max(px), y0.min(py), y1.max(py));
            }
        }
    }
    assert!(x0 <= x1, "a ring near ({x}, {y})");
    (x1 - x0 + 1, y1 - y0 + 1)
}

/// The worst-spot rings have the same size in pixels at every image size.
#[test]
fn simulation_rings_keep_their_pixel_size() {
    let d = dirs();
    let mut s = plotter_session(&d, "pen", PEN, 0.3);
    import(
        &mut s,
        r#"<path fill-rule="evenodd" d="M20 20h50v50h-50z M35 35h20v20h-20z"/>"#,
    );
    let spot = simulate::current(&mut s, None)
        .unwrap()
        .result
        .as_ref()
        .unwrap()
        .metrics
        .worst_spots[0];
    let plotter = s
        .profiles
        .plotter(&s.doc().document.job.plotter_id)
        .unwrap();
    let media = plotter
        .media_or_first(&s.doc().document.job.media_id)
        .unwrap()
        .0
        .clone();
    let r = flatcraft_engine::preview::RING_RADIUS_PX;
    let w = flatcraft_engine::preview::RING_WIDTH_PX;
    let want = 2.0 * r + w;
    for max_px in [512, 4096] {
        let png = render_png(&s, PreviewKind::Simulation, max_px).unwrap();
        let scale = f64::from(max_px) / media.width_mm.max(media.height_mm);
        let x = ((spot.at.x + media.grid.origin_x_mm) * scale) as u32;
        let y = ((spot.at.y + media.grid.origin_y_mm) * scale) as u32;
        let (ew, eh) = ring_extent(&png, x, y, (r + w) as u32 + 2);
        for e in [ew, eh] {
            assert!(
                (f64::from(e) - f64::from(want)).abs() <= 3.0,
                "{max_px} px: ring {e} px across, want about {want}"
            );
        }
    }
}

/// A project thumbnail draws the cut geometry, like the canvas preview.
#[test]
fn thumbnail_draws_cut_geometry() {
    let d = dirs();
    let mut s = session(&d);
    import(
        &mut s,
        r#"<line x1="23" y1="53.5" x2="123" y2="53.5" stroke="black" stroke-width="3"/>"#,
    );
    let plotter = s
        .profiles
        .plotter(&s.doc().document.job.plotter_id)
        .unwrap();
    let doc = s.doc().document.clone();
    let max_px = 2048;
    let thumb = flatcraft_engine::preview::render_document_png(&plotter, &doc, max_px).unwrap();
    let canvas = render_png(&s, PreviewKind::Canvas, max_px).unwrap();
    assert_eq!(thumb, canvas, "one drawing path");
    let seeded = flatcraft_engine::preview::render_document_png_seeded(
        &plotter,
        &doc,
        s.doc().cached_cut_geometry().as_ref(),
        max_px,
    )
    .unwrap();
    assert_eq!(seeded, canvas);
}

/// The 10 mm grid lines stop at the edge of the printed grid; the mat
/// below it is plain margin.
#[test]
fn grid_lines_stop_at_the_grid_edge() {
    let d = dirs();
    let s = session(&d);
    let png = render_png(&s, PreviewKind::Canvas, 1024).unwrap();
    let plotter = s
        .profiles
        .plotter(&s.doc().document.job.plotter_id)
        .unwrap();
    let media = plotter
        .media_or_first(&s.doc().document.job.media_id)
        .unwrap()
        .0;
    assert!(media.height_mm > media.grid.origin_y_mm + media.grid.height_mm + 5.0);
    let scale = 1024.0 / media.width_mm.max(media.height_mm);
    let margin = flatcraft_core::colour::margin_colour(
        media.color_or(flatcraft_device::DEFAULT_MAT_COLOR_LIGHT),
    );
    let bottom = media.grid.origin_y_mm + media.grid.height_mm;
    for k in 0..=30 {
        let x_mm = media.grid.origin_x_mm + f64::from(k) * 10.0;
        if x_mm > media.width_mm - 0.5 {
            break;
        }
        let x = (x_mm * scale).round() as u32;
        for y_mm in [bottom + 1.0, bottom + 5.0, media.height_mm - 1.0] {
            let y = (y_mm * scale) as u32;
            assert_eq!(pixel(&png, x.min(1023), y), margin, "x {x_mm} y {y_mm}");
        }
    }
}

/// A stale run is dropped at once (its trace holds memory); its summary
/// stays.
#[test]
fn a_stale_run_is_dropped_and_its_summary_kept() {
    let d = dirs();
    let mut s = plotter_session(&d, "host", HOST, 0.3);
    let id = import(&mut s, SQUARE_50);
    run(&mut s, "job.simulate", json!({}));
    let run_ = s.last_sim().unwrap().clone();
    let summary = s.doc().sim_summary().unwrap().clone();
    assert_eq!(summary.key, run_.key);
    assert_eq!(
        summary.metrics.as_ref().unwrap().score,
        run_.result.as_ref().unwrap().metrics.score
    );
    assert!(summary.metrics.as_ref().unwrap().per_path.is_empty());
    run(&mut s, "object.move", json!({ "ids": [id], "dx_mm": 2.0 }));
    assert!(s.doc().stored_sim().is_none(), "the stale run is dropped");
    assert_eq!(s.doc().sim_summary().unwrap().key, run_.key);
    // Undo also moves the revision on: nothing stale comes back.
    run(&mut s, "job.simulate", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().stored_sim().is_none());
    // A profile change drops it at the next command.
    run(&mut s, "job.simulate", json!({}));
    assert!(s.doc().stored_sim().is_some());
    s.profiles.reload().unwrap();
    run(&mut s, "document.state", json!({}));
    assert!(s.doc().stored_sim().is_none(), "the profiles changed");
}

/// A big host job keeps its samples to draw near DISPLAY_POINTS: lead-ins
/// and other waste moves are no problems that must stay.
#[test]
fn big_host_job_display_stays_near_the_cap() {
    let d = dirs();
    let mut s = plotter_session(&d, "host", HOST, 0.3);
    // Cardstock lifts at sharp corners and overshoots into the waste.
    run(&mut s, "job.set_material", json!({ "id": "cardstock" }));
    s.execute(
        "object.import_svg",
        json!({ "svg": many_squares(), "name": "grid" }),
        Origin::User,
    )
    .unwrap();
    let run_ = simulate::current(&mut s, None).unwrap();
    let res = run_.result.as_ref().unwrap();
    let samples = res.trace.samples.len();
    assert!(samples > 4 * simulate::DISPLAY_POINTS, "{samples}");
    // Waste samples far from any line: the old rule kept every one.
    let far_waste = res
        .trace
        .samples
        .iter()
        .zip(&res.deviation_mm)
        .filter(|(x, d)| {
            x.down && x.seg == flatcraft_path::WASTE_SEG && f64::from(**d) > END_TO_END_LIMIT_MM
        })
        .count();
    eprintln!(
        "samples {samples}, far waste {far_waste}, shown {}",
        run_.display.len()
    );
    assert!(
        run_.display.len() <= simulate::DISPLAY_POINTS + 2,
        "{} shown",
        run_.display.len()
    );
    assert!(run_.display.len() >= simulate::DISPLAY_POINTS * 9 / 10);
}

/// Firmware mode draws the commanded path in the mat's neutral contrast
/// colour, never in a colour of the grade or the cut orange, on a light
/// and on a dark mat.
#[test]
fn firmware_preview_draws_the_commanded_path_neutral() {
    for mat in ["#F2EAD8", "#202226", "#808080"] {
        let d = dirs();
        let media = format!(
            "media = [{{ id = \"mat-12x12\", name = \"Mat 12x12\", kind = \"Mat\", color = \"{mat}\", width_mm = 304.8, height_mm = 330.2, calibrated = false, grid = {{ origin_x_mm = 0.0, origin_y_mm = 0.0, width_mm = 304.8, height_mm = 304.8 }} }}]\n"
        );
        user_plotter(
            d.config.path(),
            "fw",
            &format!("knife_compensation = {FIRMWARE}\nblade_offset_mm = 0.25\n{media}"),
        );
        let mut s = session(&d);
        run(&mut s, "job.set_plotter", json!({ "id": "fw" }));
        run(&mut s, "job.set_media", json!({ "id": "mat-12x12" }));
        import(&mut s, SQUARE_50);
        let png = render_png(&s, PreviewKind::Simulation, 4096).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgb8();
        let colour = {
            let plotter = s.profiles.plotter("fw").unwrap();
            plotter
                .media_or_first("mat-12x12")
                .unwrap()
                .0
                .color
                .unwrap()
        };
        let ink = flatcraft_core::colour::commanded_colour(colour);
        let has = |c: [u8; 3]| img.pixels().any(|p| [p[0], p[1], p[2]] == c);
        assert!(has(ink), "{mat}: the commanded path");
        assert_ne!(ink, flatcraft_core::colour::grid_line_colour(colour));
        // Thicker than a 1 px grid line: some row crosses the square's
        // left edge with at least 2 solid pixels.
        let widest = (0..img.height())
            .map(|y| {
                let mut run = 0u32;
                let mut best = 0u32;
                for x in 0..img.width() {
                    let p = img.get_pixel(x, y);
                    if [p[0], p[1], p[2]] == ink {
                        run += 1;
                        best = best.max(run);
                    } else {
                        run = 0;
                    }
                }
                best
            })
            .filter(|&b| b > 0)
            .min()
            .unwrap();
        assert!(widest >= 2, "{mat}: {widest} px");
        assert!(!has(flatcraft_core::colour::cut_colour(colour)), "{mat}");
        for c in flatcraft_core::colour::deviation_colours(colour) {
            assert!(!has(c), "{mat}: {c:?}");
        }
    }
}

/// A session on a host plotter whose mat has the colour `mat` (#rrggbb).
fn mat_session(d: &Dirs, mat: &str) -> Session {
    let media = format!(
        "media = [{{ id = \"mat-12x12\", name = \"Mat 12x12\", kind = \"Mat\", color = \"{mat}\", width_mm = 304.8, height_mm = 330.2, calibrated = false, grid = {{ origin_x_mm = 0.0, origin_y_mm = 0.0, width_mm = 304.8, height_mm = 304.8 }} }}]\n"
    );
    user_plotter(
        d.config.path(),
        "host",
        &format!("knife_compensation = {HOST}\nblade_offset_mm = 0.3\n{media}"),
    );
    let mut s = session(d);
    run(&mut s, "job.set_plotter", json!({ "id": "host" }));
    run(&mut s, "job.set_media", json!({ "id": "mat-12x12" }));
    s
}

/// On a mid-grey mat the tip colours are measured against that grey: a
/// set that reads 3:1 is drawn and no note is needed.
#[test]
fn tip_colours_follow_a_mid_grey_mat() {
    let grey = [0x80, 0x80, 0x80];
    let set = flatcraft_core::colour::deviation_colours(grey);
    for c in set {
        assert!(
            flatcraft_core::colour::contrast_ratio(c, grey) >= 3.0,
            "{c:?}"
        );
    }
    let d = dirs();
    let mut s = mat_session(&d, "#808080");
    import(&mut s, SQUARE_50);
    let r = run(&mut s, "job.simulate", json!({}));
    assert!(
        !r["warnings"].to_string().contains("hard to tell"),
        "{}",
        r["warnings"]
    );
    let png = render_png(&s, PreviewKind::Simulation, 2048).unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgb8();
    assert!(
        img.pixels().any(|p| [p[0], p[1], p[2]] == set[0]),
        "the green of the grey mat's set"
    );
}

/// The float simulation measures the algorithm alone: host compensation
/// plans in exact floats (axis points off the device grid) and the result
/// meets the algorithm limit, while the device run plans every point on a
/// device unit. Before Task 16 the float run simulated the plan rounded
/// for the device and read the rounding (0.018 mm on a 50 mm square).
#[test]
fn float_simulation_plans_in_floats() {
    let d = dirs();
    let mut s = plotter_session(&d, "host", HOST, 0.3);
    import(
        &mut s,
        r#"<rect x="20" y="20" width="50" height="50"/><circle cx="120" cy="60" r="20"/>"#,
    );
    let plotter = s.profiles.plotter("host").unwrap();
    let media = plotter
        .media_or_first(&s.doc().document.job.media_id)
        .unwrap()
        .0
        .clone();
    let grid = device_grid(&plotter, &media);
    let off_grid = |r: &simulate::SimRun| {
        r.job
            .plan
            .toolpaths
            .iter()
            .flat_map(|t| t.runs())
            .any(|run| {
                run.points.iter().any(|&q| {
                    let (u, v) = (grid.to_units(q), grid.rounded_units(q));
                    (u.x - v.x).abs() > 1e-6 || (u.y - v.y).abs() > 1e-6
                })
            })
    };
    let float = simulate::compute(simulate::snapshot(&s, None, SimGrid::Float).unwrap()).unwrap();
    let device = simulate::compute(simulate::snapshot(&s, None, SimGrid::Device).unwrap()).unwrap();
    assert!(
        off_grid(&float),
        "the float run was planned for the device grid"
    );
    assert!(!off_grid(&device), "the device run has points off the grid");
    let m = &float.result.as_ref().unwrap().metrics;
    assert!(
        m.max_hausdorff_mm.max(m.max_correspondence_mm) <= flatcraft_sim::ALGORITHM_LIMIT_MM,
        "{}",
        m.max_hausdorff_mm
    );
}

/// A float run's job is planned in exact numbers the plotter cannot take:
/// it has no bytes and `send_built` refuses it before the gate, while the
/// device run's job is a real one.
#[test]
fn a_float_run_job_is_never_sent() {
    let d = dirs();
    let mut s = plotter_session(&d, "host", HOST, 0.3);
    import(&mut s, r#"<circle cx="120" cy="60" r="20"/>"#);
    let plotter = s.profiles.plotter("host").unwrap();
    let float = simulate::compute(simulate::snapshot(&s, None, SimGrid::Float).unwrap()).unwrap();
    assert!(float.job.simulation_only);
    assert!(float.job.job.bytes.is_empty());
    let err = flatcraft_engine::job::send_built(
        &mut s,
        &plotter,
        float.job.clone(),
        "float".into(),
        Origin::User,
    )
    .unwrap_err();
    assert_eq!(
        err,
        CommandError::Failed(flatcraft_engine::job::SIMULATION_ONLY.into())
    );
    assert!(!flatcraft_engine::job::SIMULATION_ONLY.contains('\u{2014}'));
    let device = simulate::compute(simulate::snapshot(&s, None, SimGrid::Device).unwrap()).unwrap();
    assert!(!device.job.simulation_only);
    assert!(!device.job.job.bytes.is_empty());
}

// ------------------------------------------------------------ visible outline

/// A back plate with two counters, in visible outline, and a front plate
/// over its right part that covers no counter.
fn lettered_plates(s: &mut Session) -> (u64, u64) {
    let back = import(
        s,
        r#"<path fill-rule="evenodd" d="M20 20h80v40h-80z M30 30h10v20h-10z M50 30h10v20h-10z"/>"#,
    );
    let front = import(s, r#"<rect x="80" y="10" width="40" height="60"/>"#);
    run(
        s,
        "object.set_cut_options",
        json!({ "ids": [back], "mode": "visible_outline" }),
    );
    (back, front)
}

#[test]
fn a_visible_outline_frees_nothing_before_its_counters_are_cut() {
    let d = dirs();
    let mut s = session(&d);
    run(&mut s, "job.set_media", json!({ "id": "no-mat" }));
    let (back, front) = lettered_plates(&mut s);
    let input = flatcraft_engine::job::JobInput::of(&s).unwrap();
    let plan = flatcraft_engine::job::build_input(&input).unwrap().plan;
    let cuts: Vec<&flatcraft_path::Toolpath> = plan
        .toolpaths
        .iter()
        .filter(|t| t.kind == flatcraft_path::ToolpathKind::Cut && t.pass == 1)
        .collect();
    let at = |f: &dyn Fn(&flatcraft_path::Toolpath) -> bool| -> Vec<usize> {
        cuts.iter()
            .enumerate()
            .filter(|(_, t)| f(t))
            .map(|(k, _)| k)
            .collect()
    };
    let counters = at(&|t| t.source.object.0 == back && t.closed);
    let outline = at(&|t| t.source.object.0 == back && !t.closed);
    let front_ring = at(&|t| t.source.object.0 == front);
    assert_eq!(counters.len(), 2, "{cuts:?}");
    assert!(!outline.is_empty() && front_ring.len() == 1);
    // The back piece comes free with the last cut of its outline: its own
    // open path or the front ring that closes it.
    let freed = outline.iter().chain(&front_ring).max().unwrap();
    assert!(
        counters.iter().all(|c| c < freed),
        "counters {counters:?}, outline {outline:?}, front {front_ring:?}"
    );
    // The counters are holes of the piece: the outside is kept.
    for c in &counters {
        assert_eq!(cuts[*c].keep, flatcraft_path::KeepSide::Outside);
    }
}

#[test]
fn lead_ins_of_a_visible_outline_stay_out_of_kept_material() {
    let d = dirs();
    let mut s = plotter_session(&d, "host", HOST, 0.3);
    lettered_plates(&mut s);
    let r = run(&mut s, "job.simulate", json!({}));
    assert_eq!(r["simulated"], true, "{r}");
    let into = r["metrics"]["max_into_kept_mm"].as_f64().unwrap();
    assert_eq!(into, 0.0, "{r}");
}

/// A 150 mm tall plate in visible outline, its left and right edges
/// covered at y 90..100, with a counter near its bottom: its outline is
/// cut as a top and a bottom part. On a roll the feed must never go back,
/// and the counter still comes before the cut that frees the plate.
#[test]
fn a_tall_visible_outline_keeps_the_feed_monotonic() {
    let d = dirs();
    let mut s = session(&d);
    run(&mut s, "job.set_media", json!({ "id": "no-mat" }));
    let back = import(
        &mut s,
        r#"<path fill-rule="evenodd" d="M30 10h40v150h-40z M45 140h10v10h-10z"/>"#,
    );
    let left = import(&mut s, r#"<rect x="20" y="90" width="15" height="10"/>"#);
    let right = import(&mut s, r#"<rect x="65" y="90" width="15" height="10"/>"#);
    run(
        &mut s,
        "object.set_cut_options",
        json!({ "ids": [back], "mode": "visible_outline" }),
    );
    let input = flatcraft_engine::job::JobInput::of(&s).unwrap();
    let plan = flatcraft_engine::job::build_input(&input).unwrap().plan;
    let cuts: Vec<&flatcraft_path::Toolpath> = plan
        .toolpaths
        .iter()
        .filter(|t| t.kind == flatcraft_path::ToolpathKind::Cut && t.pass == 1)
        .collect();
    let bands: Vec<usize> = cuts
        .iter()
        .map(|t| flatcraft_path::feed::band_of(t))
        .collect();
    assert!(
        bands.windows(2).all(|w| w[0] <= w[1]),
        "the feed goes back: {bands:?}"
    );
    let at = |f: &dyn Fn(&flatcraft_path::Toolpath) -> bool| -> Vec<usize> {
        cuts.iter()
            .enumerate()
            .filter(|(_, t)| f(t))
            .map(|(k, _)| k)
            .collect()
    };
    let counter = at(&|t| t.source.object.0 == back && t.closed);
    let outline = at(&|t| t.source.object.0 == back && !t.closed);
    let fronts = at(&|t| [left, right].contains(&t.source.object.0));
    assert_eq!(counter.len(), 1, "{cuts:?}");
    assert_eq!(outline.len(), 2, "a top and a bottom part: {cuts:?}");
    let freed = outline.iter().chain(&fronts).max().unwrap();
    assert!(
        counter[0] < *freed,
        "counter {counter:?}, outline {outline:?}, fronts {fronts:?}"
    );
}

/// A front ring that only touches the plate at a corner does not close
/// it: on a mat the counter comes before every cut that does (the plate's
/// own outline and the ring that clips its left edge).
#[test]
fn a_counter_comes_before_every_cut_that_closes_its_outline() {
    let d = dirs();
    let mut s = session(&d);
    let back = import(
        &mut s,
        r#"<path fill-rule="evenodd" d="M30 10h40v140h-40z M45 130h10v10h-10z"/>"#,
    );
    let f1 = import(&mut s, r#"<rect x="25" y="60" width="10" height="10"/>"#);
    let f2 = import(&mut s, r#"<rect x="70" y="150" width="40" height="50"/>"#);
    run(
        &mut s,
        "object.set_cut_options",
        json!({ "ids": [back], "mode": "visible_outline" }),
    );
    let input = flatcraft_engine::job::JobInput::of(&s).unwrap();
    let plan = flatcraft_engine::job::build_input(&input).unwrap().plan;
    let cuts: Vec<&flatcraft_path::Toolpath> = plan
        .toolpaths
        .iter()
        .filter(|t| t.kind == flatcraft_path::ToolpathKind::Cut && t.pass == 1)
        .collect();
    let at = |f: &dyn Fn(&flatcraft_path::Toolpath) -> bool| -> Vec<usize> {
        cuts.iter()
            .enumerate()
            .filter(|(_, t)| f(t))
            .map(|(k, _)| k)
            .collect()
    };
    let counter = at(&|t| t.source.object.0 == back && t.closed);
    let closing = at(&|t| (t.source.object.0 == back && !t.closed) || t.source.object.0 == f1);
    assert_eq!(counter.len(), 1, "{cuts:?}");
    assert_eq!(at(&|t| t.source.object.0 == f2).len(), 1);
    assert!(
        closing.iter().all(|c| counter[0] < *c),
        "counter {counter:?}, closing cuts {closing:?}"
    );
    assert_eq!(plan.freed_early, 0);
}

/// A plate in visible outline lies in a background ring; the front ring
/// that clips its bottom crosses that background ring. The counter comes
/// before every cut that closes the plate, also the front ring with its
/// other container, on a mat and without one (where the feed stays
/// monotonic).
fn straddling_case(no_mat: bool) {
    let d = dirs();
    let mut s = session(&d);
    if no_mat {
        run(&mut s, "job.set_media", json!({ "id": "no-mat" }));
    }
    let background = import(&mut s, r#"<rect x="20" y="20" width="100" height="100"/>"#);
    let plate = import(
        &mut s,
        r#"<path fill-rule="evenodd" d="M40 40h60v70h-60z M60 70h10v10h-10z"/>"#,
    );
    let front = import(&mut s, r#"<rect x="30" y="100" width="80" height="40"/>"#);
    run(
        &mut s,
        "object.set_cut_options",
        json!({ "ids": [plate], "mode": "visible_outline" }),
    );
    let input = flatcraft_engine::job::JobInput::of(&s).unwrap();
    let plan = flatcraft_engine::job::build_input(&input).unwrap().plan;
    let cuts: Vec<&flatcraft_path::Toolpath> = plan
        .toolpaths
        .iter()
        .filter(|t| t.kind == flatcraft_path::ToolpathKind::Cut && t.pass == 1)
        .collect();
    let at = |f: &dyn Fn(&flatcraft_path::Toolpath) -> bool| -> Vec<usize> {
        cuts.iter()
            .enumerate()
            .filter(|(_, t)| f(t))
            .map(|(k, _)| k)
            .collect()
    };
    let counter = at(&|t| t.source.object.0 == plate && t.closed);
    let outline = at(&|t| t.source.object.0 == plate && !t.closed);
    let front_ring = at(&|t| t.source.object.0 == front);
    let back_ring = at(&|t| t.source.object.0 == background);
    assert_eq!(
        (counter.len(), front_ring.len(), back_ring.len()),
        (1, 1, 1),
        "{cuts:?}"
    );
    assert!(!outline.is_empty());
    for c in outline.iter().chain(&front_ring) {
        assert!(counter[0] < *c, "counter {counter:?} before {c} ({cuts:?})");
    }
    assert!(
        outline.iter().all(|o| o < &back_ring[0]),
        "the plate before its background"
    );
    assert_eq!(plan.freed_early, 0);
    if no_mat {
        let bands: Vec<usize> = cuts
            .iter()
            .map(|t| flatcraft_path::feed::band_of(t))
            .collect();
        assert!(bands.windows(2).all(|w| w[0] <= w[1]), "{bands:?}");
    }
}

#[test]
fn a_front_ring_across_the_background_also_waits_for_the_counter() {
    straddling_case(false);
    straddling_case(true);
}
