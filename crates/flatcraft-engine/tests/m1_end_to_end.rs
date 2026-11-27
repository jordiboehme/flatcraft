//! M1 end to end: a fresh config dir, the test sheet, a traced PNG,
//! auto-arrange and File Output, the way an agent drives it over MCP.

use std::path::{Path, PathBuf};

use flatcraft_core::document::ObjectId;
use flatcraft_device::ProfileStore;
use flatcraft_device::gpgl::check_safe;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{Origin, Session};
use flatcraft_path::Toolpath;
use kurbo::{Rect, Shape};
use serde_json::{Value, json};

const PLOTTER: &str = "file-output-gpgl-cameo2";

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::Agent)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

/// A 240 x 240 px PNG (20.32 mm at the default 300 dpi): a black 200 px
/// square at 20,20 with a white 80 px hole at 80,80.
fn ring_png(path: &Path) {
    let mut img = image::GrayImage::from_pixel(240, 240, image::Luma([255]));
    for y in 20..220 {
        for x in 20..220 {
            let hole = (80..160).contains(&x) && (80..160).contains(&y);
            if !hole {
                img.put_pixel(x, y, image::Luma([0]));
            }
        }
    }
    img.save(path).unwrap();
}

fn bbox(t: &Toolpath) -> Rect {
    t.points
        .iter()
        .fold(Rect::from_points(t.points[0], t.points[0]), |r, p| {
            r.union_pt(*p)
        })
}

/// Position in the plan of the first toolpath that `pick` accepts.
fn first(plan: &[Toolpath], pick: impl Fn(&Toolpath) -> bool) -> usize {
    plan.iter()
        .position(pick)
        .expect("a toolpath matches the pick")
}

/// All (y, x) device pairs of the move and draw commands in the listing.
fn device_points(listing: &str) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    for line in listing.lines() {
        let cmd = line.split_whitespace().next().unwrap_or("");
        let Some(args) = cmd.strip_prefix('M').or_else(|| cmd.strip_prefix('D')) else {
            continue;
        };
        let nums: Vec<i64> = args
            .split(',')
            .filter(|n| !n.is_empty())
            .map(|n| n.parse().unwrap_or_else(|_| panic!("bad number in {line}")))
            .collect();
        assert!(
            nums.len().is_multiple_of(2),
            "odd coordinate count in {line}"
        );
        out.extend(nums.chunks(2).map(|c| (c[0], c[1])));
    }
    out
}

#[test]
fn m1_end_to_end() {
    // 1. A fresh config dir, a separate output dir and a separate source dir.
    let config = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let prefs = Preferences {
        output_dir: Some(out.path().to_path_buf()),
        ..Preferences::default()
    };
    assert_eq!(prefs.default_plotter, PLOTTER);
    let profiles = ProfileStore::load(config.path()).unwrap();
    let mut s = Session::new(profiles, prefs, false, config.path().to_path_buf());

    // The default plotter and the default mat with its grid origin.
    let state = run(&mut s, "document.state", json!({}));
    assert_eq!(state["job"]["plotter_id"], PLOTTER, "{state}");
    let mat = &state["mat"];
    assert_eq!(mat["id"], "mat-12x12", "{mat}");
    assert_eq!(mat["calibrated"], false, "{mat}");
    assert_eq!(mat["grid"]["origin_x_mm"].as_f64(), Some(0.0), "{mat}");
    assert_eq!(mat["grid"]["origin_y_mm"].as_f64(), Some(0.0), "{mat}");
    // Uncalibrated: the cutting area is the grid inset by 5 mm.
    let area = &state["cutting_area_mm"];
    let a = |k: &str| area[k].as_f64().unwrap();
    let area = Rect::new(a("x0"), a("y0"), a("x1"), a("y1"));
    for (got, want) in [
        (area.x0, 5.0),
        (area.y0, 5.0),
        (area.x1, 299.8),
        (area.y1, 299.8),
    ] {
        assert!((got - want).abs() < 1e-9, "{area:?}");
    }

    // 2. Test sheet, a traced PNG, auto-arrange.
    let sheet = run(&mut s, "tools.test_patterns", json!({}))["id"]
        .as_u64()
        .unwrap();
    let png = src.path().join("ring.png");
    ring_png(&png);
    let r = run(&mut s, "object.import", json!({ "path": png }));
    let ring = r["id"].as_u64().unwrap();
    assert!(r["threshold_used"].is_u64(), "{r}");
    let size = r["size_mm"].as_array().unwrap();
    assert!((size[0].as_f64().unwrap() - 20.32).abs() < 1e-6, "{r}");

    // The import selected the ring; with nothing selected all are packed.
    run(&mut s, "object.select", json!({ "ids": [] }));
    let r = run(&mut s, "arrange.auto", json!({}));
    let mut placed: Vec<u64> = r["placed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    placed.sort_unstable();
    assert_eq!(placed, vec![sheet, ring], "{r}");
    assert!(r["did_not_fit"].as_array().unwrap().is_empty(), "{r}");

    // The tallest object, the sheet, sits at the top-left of the cutting area.
    let state = run(&mut s, "document.state", json!({}));
    let objects = state["objects"].as_array().unwrap().clone();
    assert_eq!(objects.len(), 2);
    let sheet_bounds = &objects
        .iter()
        .find(|o| o["id"].as_u64() == Some(sheet))
        .unwrap()["bounds_mm"];
    assert!(
        (sheet_bounds["x0"].as_f64().unwrap() - 5.0).abs() < 1e-6
            && (sheet_bounds["y0"].as_f64().unwrap() - 5.0).abs() < 1e-6,
        "{sheet_bounds}"
    );

    // Preview first, and keep its bytes: job.send replaces the last job.
    let preview = run(&mut s, "job.preview", json!({}));
    let preview_bytes = s.doc().last_job().unwrap().job.bytes.clone();
    assert_eq!(
        preview["bytes"].as_u64().unwrap() as usize,
        preview_bytes.len()
    );
    assert_eq!(preview["settings"]["passes"], 1, "{preview}");

    let sent = run(&mut s, "job.send", json!({ "name": "m1" }));
    let files: Vec<PathBuf> = sent["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| PathBuf::from(f.as_str().unwrap()))
        .collect();
    assert_eq!(
        files,
        vec![
            out.path().join("m1.gpgl"),
            out.path().join("m1.gpgl.txt"),
            out.path().join("m1.bytelog.jsonl"),
        ]
    );
    for f in &files {
        assert!(f.is_file(), "{} is missing", f.display());
    }

    // 3. The bytes: safe, complete, the same as the preview.
    let bytes = std::fs::read(&files[0]).unwrap();
    check_safe(&bytes).unwrap();
    assert!(bytes.ends_with(b"H\x03"));
    assert_eq!(bytes, preview_bytes, "the file differs from job.preview");
    assert_eq!(sent["bytes_sent"].as_u64().unwrap() as usize, bytes.len());
    // The physical 12x12 mat (330.2 x 304.8 mm, y first) is the work area.
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("Z6604,6096\x03"), "{text}");

    // The listing names every object.
    let listing = std::fs::read_to_string(&files[1]).unwrap();
    for o in &objects {
        let tag = format!("object {} '{}'", o["id"], o["name"].as_str().unwrap());
        assert!(listing.contains(&tag), "the listing does not name {tag}");
    }
    assert!(listing.contains("'Test patterns'") && listing.contains("'ring'"));

    // The grid origin of the default mat (0, 0) is where app coordinates
    // start: the sheet's top-left at 5,5 mm is device 100,100.
    let pts = device_points(&listing);
    let min_y = pts.iter().map(|p| p.0).min().unwrap();
    let min_x = pts.iter().map(|p| p.1).min().unwrap();
    assert!(
        (min_y - 100).abs() <= 1 && (min_x - 100).abs() <= 1,
        "{min_y},{min_x}"
    );

    // 4. Holes are cut before their outline (inside first), on the sheet and
    //    on the traced image.
    let job = s.doc().last_job().unwrap();
    let plan = &job.plan.toolpaths;
    let geometry = &s.doc().document.get(ObjectId(sheet)).unwrap().geometry;
    let hole_paths: Vec<(usize, f64)> = geometry
        .iter()
        .enumerate()
        .filter(|(_, p)| p.label.as_deref() == Some("hole-in-square"))
        .map(|(i, p)| (i, p.path.bounding_box().width()))
        .collect();
    assert_eq!(hole_paths.len(), 2, "{hole_paths:?}");
    let index_of = |w: f64| {
        hole_paths
            .iter()
            .find(|(_, pw)| (pw - w).abs() < 1e-3)
            .unwrap()
            .0
    };
    let (hole, outline) = (index_of(10.0), index_of(30.0));
    let at = |i: usize| {
        first(plan, |t| {
            t.source.object == ObjectId(sheet) && t.source.path_index == i
        })
    };
    assert!(at(hole) < at(outline), "sheet: outline cut before its hole");

    let ring_paths: Vec<(usize, f64)> = plan
        .iter()
        .enumerate()
        .filter(|(_, t)| t.source.object == ObjectId(ring))
        .map(|(i, t)| (i, bbox(t).width()))
        .collect();
    assert_eq!(ring_paths.len(), 2, "{ring_paths:?}");
    let small = ring_paths
        .iter()
        .find(|(_, w)| (w - 6.77).abs() < 0.5)
        .unwrap_or_else(|| panic!("no 6.8 mm hole: {ring_paths:?}"));
    let big = ring_paths
        .iter()
        .find(|(_, w)| (w - 16.93).abs() < 0.5)
        .unwrap_or_else(|| panic!("no 16.9 mm outline: {ring_paths:?}"));
    assert!(small.0 < big.0, "traced image: outline cut before its hole");
}
