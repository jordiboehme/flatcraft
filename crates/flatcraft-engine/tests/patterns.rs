//! Test pattern sheet and preview rendering.

use flatcraft_core::geometry::{CutPath, bounds};
use flatcraft_device::ProfileStore;
use flatcraft_device::gpgl::quantize::to_device;
use flatcraft_engine::job;
use flatcraft_engine::patterns::sheet_svg;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::preview::{PreviewKind, render_png};
use flatcraft_engine::registry;
use flatcraft_engine::session::{CommandError, Origin, Session};
use flatcraft_import::{fonts, svg::import_svg};
use kurbo::{PathEl, Point, Rect, Shape};
use serde_json::{Value, json};

fn session(dir: &tempfile::TempDir) -> Session {
    let profiles = ProfileStore::load(dir.path()).unwrap();
    let prefs = Preferences {
        output_dir: Some(dir.path().join("output")),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, false, dir.path().to_path_buf())
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

fn sheet_paths() -> Vec<CutPath> {
    import_svg(sheet_svg().as_bytes(), fonts::system_fonts())
        .unwrap()
        .paths
}

fn labelled<'a>(paths: &'a [CutPath], label: &str) -> Vec<&'a CutPath> {
    paths
        .iter()
        .filter(|p| p.label.as_deref() == Some(label))
        .collect()
}

fn bounds_of(paths: &[&CutPath]) -> Rect {
    let owned: Vec<CutPath> = paths.iter().map(|p| (*p).clone()).collect();
    bounds(&owned).expect("paths have bounds")
}

#[test]
fn sheet_imports_at_true_size() {
    let paths = sheet_paths();
    let sq = bounds_of(&labelled(&paths, "square-50"));
    assert!((sq.width() - 50.0).abs() < 1e-3, "{sq:?}");
    assert!((sq.height() - 50.0).abs() < 1e-3, "{sq:?}");
    let rx = bounds_of(&labelled(&paths, "ruler-x"));
    assert!((rx.width() - 100.0).abs() < 1e-3, "{rx:?}");
    let ry = bounds_of(&labelled(&paths, "ruler-y"));
    assert!((ry.height() - 100.0).abs() < 1e-3, "{ry:?}");
    let f = bounds_of(&labelled(&paths, "f-glyph"));
    assert!((f.height() - 20.0).abs() < 1e-3, "{f:?}");
    let line = bounds_of(&labelled(&paths, "open-line"));
    assert!((line.width() - 40.0).abs() < 1e-3, "{line:?}");
    let hole = labelled(&paths, "hole-in-square");
    assert_eq!(hole.len(), 2);
    let mut sizes: Vec<f64> = hole.iter().map(|p| p.path.bounding_box().width()).collect();
    sizes.sort_by(f64::total_cmp);
    assert!((sizes[0] - 10.0).abs() < 1e-3 && (sizes[1] - 30.0).abs() < 1e-3);
}

#[test]
fn circles_have_the_listed_radii() {
    let paths = sheet_paths();
    let mut diameters: Vec<f64> = labelled(&paths, "circles")
        .iter()
        .map(|p| p.path.bounding_box().width())
        .collect();
    diameters.sort_by(f64::total_cmp);
    let want = [1.0, 2.0, 4.0, 10.0, 20.0, 50.0];
    assert_eq!(diameters.len(), want.len());
    for (d, w) in diameters.iter().zip(want) {
        assert!((d - w).abs() < 1e-3, "{d} vs {w}");
    }
}

#[test]
fn every_pattern_has_a_group_and_the_layout_keeps_its_distance() {
    let paths = sheet_paths();
    let groups = [
        "square-50",
        "circles",
        "hole-in-square",
        "open-line",
        "s-curve",
        "f-glyph",
        "ruler-x",
        "ruler-y",
    ];
    let mut rects: Vec<(String, Rect)> = Vec::new();
    for g in groups {
        let found = labelled(&paths, g);
        assert!(!found.is_empty(), "no path labelled {g}");
        rects.push((g.to_string(), bounds_of(&found)));
    }
    let stars: Vec<&CutPath> = paths
        .iter()
        .filter(|p| p.label.as_deref().is_some_and(|l| l.starts_with("star-")))
        .collect();
    rects.push(("star".into(), bounds_of(&stars)));
    for (name, r) in &rects {
        assert!(
            r.x0 >= 10.0 - 1e-6
                && r.y0 >= 10.0 - 1e-6
                && r.x1 <= 270.0 + 1e-6
                && r.y1 <= 270.0 + 1e-6,
            "{name} too close to the border: {r:?}"
        );
    }
    for (i, (an, a)) in rects.iter().enumerate() {
        for (bn, b) in &rects[i + 1..] {
            let dx = (a.x0 - b.x1).max(b.x0 - a.x1);
            let dy = (a.y0 - b.y1).max(b.y0 - a.y1);
            assert!(
                dx.max(dy) >= 10.0 - 1e-6,
                "{an} and {bn} are closer than 10 mm"
            );
        }
    }
}

/// Angle at the apex (the point with the smallest y) of a closed triangle, deg.
fn apex_angle(p: &CutPath) -> f64 {
    let pts: Vec<Point> = p
        .path
        .elements()
        .iter()
        .filter_map(|e| match e {
            PathEl::MoveTo(q) | PathEl::LineTo(q) => Some(*q),
            _ => None,
        })
        .collect();
    assert!(
        p.path
            .elements()
            .iter()
            .any(|e| matches!(e, PathEl::ClosePath)),
        "star tip must be closed"
    );
    let mut distinct: Vec<Point> = Vec::new();
    for q in pts {
        if !distinct.iter().any(|d| d.distance(q) < 1e-6) {
            distinct.push(q);
        }
    }
    assert_eq!(distinct.len(), 3, "a tip is a triangle");
    let apex = *distinct.iter().min_by(|a, b| a.y.total_cmp(&b.y)).unwrap();
    let others: Vec<Point> = distinct.into_iter().filter(|q| *q != apex).collect();
    let (a, b) = (others[0] - apex, others[1] - apex);
    assert!((a.hypot() - 20.0).abs() < 1e-3 && (b.hypot() - 20.0).abs() < 1e-3);
    (a.dot(b) / (a.hypot() * b.hypot())).acos().to_degrees()
}

#[test]
fn star_has_eight_tips() {
    let paths = sheet_paths();
    let stars: Vec<&CutPath> = paths
        .iter()
        .filter(|p| p.label.as_deref().is_some_and(|l| l.starts_with("star-")))
        .collect();
    assert_eq!(stars.len(), 8);
    let mut apexes_x = Vec::new();
    for want in [15, 30, 45, 60, 90, 120, 150, 170] {
        let label = format!("star-{want}");
        let tip = labelled(&paths, &label);
        assert_eq!(tip.len(), 1, "{label}");
        let got = apex_angle(tip[0]);
        assert!((got - f64::from(want)).abs() < 0.01, "{label}: {got}");
        let apex = tip[0]
            .path
            .elements()
            .iter()
            .filter_map(|e| match e {
                PathEl::MoveTo(q) | PathEl::LineTo(q) => Some(*q),
                _ => None,
            })
            .min_by(|a, b| a.y.total_cmp(&b.y))
            .unwrap();
        apexes_x.push(apex.x);
    }
    for w in apexes_x.windows(2) {
        assert!(
            (w[1] - w[0] - 25.0).abs() < 1e-3,
            "apex pitch {}",
            w[1] - w[0]
        );
    }
}

#[test]
fn f_glyph_is_not_mirrored_in_device_space() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(&mut s, "tools.test_patterns", json!({}));
    let result = job::build(&s).expect("the sheet builds with the file output profile");
    let sheet = &s.doc().document.objects[0];
    let paths = sheet.placed_paths();
    let f_index = paths
        .iter()
        .position(|p| p.label.as_deref() == Some("f-glyph"))
        .expect("f-glyph path");

    // The job listing names the F through its label.
    assert!(
        result
            .job
            .listing
            .iter()
            .any(|l| l.note.contains("[f-glyph]")),
        "no listing note names f-glyph"
    );
    let plotter = s
        .profiles
        .plotter(&s.doc().document.job.plotter_id)
        .unwrap();
    let media = plotter
        .media
        .iter()
        .find(|m| m.id == s.doc().document.job.media_id)
        .unwrap();
    let pts: Vec<_> = result
        .plan
        .toolpaths
        .iter()
        .filter(|t| t.source.object == sheet.id && t.source.path_index == f_index)
        .flat_map(|t| t.points.iter().map(|p| to_device(*p, &plotter, media)))
        .collect();
    assert!(pts.len() >= 10);

    let min_x = pts.iter().map(|p| p.x).min().unwrap();
    let min_y = pts.iter().map(|p| p.y).min().unwrap();
    let max_y = pts.iter().map(|p| p.y).max().unwrap();
    let u = plotter.units_per_mm;
    let mm = |v: f64| (v * u).round() as i32;
    assert!((max_y - min_y - mm(20.0)).abs() <= 1, "F is 20 mm tall");

    // Arm tips: the largest x in the band of each arm.
    let band_max_x = |from_mm: f64, to_mm: f64| {
        pts.iter()
            .filter(|p| p.y >= min_y + mm(from_mm) - 1 && p.y <= min_y + mm(to_mm) + 1)
            .map(|p| p.x)
            .max()
            .unwrap()
    };
    let top_arm_x = band_max_x(0.0, 4.0);
    let mid_arm_x = band_max_x(8.0, 12.0);
    let top_arm_y = pts
        .iter()
        .filter(|p| p.x == top_arm_x)
        .map(|p| p.y)
        .min()
        .unwrap();
    let mid_arm_y = pts
        .iter()
        .filter(|p| p.x == mid_arm_x)
        .map(|p| p.y)
        .min()
        .unwrap();

    // Stem on the left, the arms point right.
    assert!(min_x < top_arm_x && min_x < mid_arm_x);
    assert!(top_arm_x - min_x >= mm(13.0), "top arm is 14 mm wide");
    assert!(
        mid_arm_x - min_x >= mm(10.0) && mid_arm_x < top_arm_x,
        "middle arm is shorter"
    );
    // The stem spans the full height at the left edge.
    let stem_ys: Vec<i32> = pts
        .iter()
        .filter(|p| p.x <= min_x + 1)
        .map(|p| p.y)
        .collect();
    assert!(stem_ys.contains(&min_y) && stem_ys.contains(&max_y));
    // Top arm above the middle arm.
    assert!(top_arm_y < mid_arm_y);
}

fn decode_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).unwrap();
    (img.width(), img.height())
}

#[test]
fn preview_png_decodes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(&mut s, "tools.test_patterns", json!({}));
    for (kind, max_px) in [
        (PreviewKind::Canvas, 512),
        (PreviewKind::Path, 512),
        (PreviewKind::Canvas, 64),
        (PreviewKind::Path, 777),
    ] {
        let png = render_png(&s, kind, max_px).unwrap();
        let (w, h) = decode_size(&png);
        assert_eq!(w.max(h), max_px, "{kind:?}");
        assert!(w.min(h) > 0);
    }
}

#[test]
fn preview_canvas_works_on_an_empty_document_and_path_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let png = render_png(&s, PreviewKind::Canvas, 200).unwrap();
    assert_eq!(decode_size(&png).0.max(decode_size(&png).1), 200);
    assert!(matches!(
        render_png(&s, PreviewKind::Path, 200),
        Err(CommandError::Refused(_))
    ));
}

#[test]
fn canvas_preview_draws_the_object_in_its_colour() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="10" y="10" width="80" height="80" fill="#ff0000"/></svg>"##;
    run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg, "name": "red" }),
    );
    let png = render_png(&s, PreviewKind::Canvas, 400).unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    assert!(
        img.pixels()
            .any(|p| p.0[0] > 200 && p.0[1] < 80 && p.0[2] < 80),
        "no red pixel"
    );
}

#[test]
fn preview_command_returns_base64_png() {
    use base64::Engine;
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(&mut s, "tools.test_patterns", json!({}));
    let r = run(
        &mut s,
        "document.preview",
        json!({ "kind": "canvas", "max_px": 300 }),
    );
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(r["png_base64"].as_str().unwrap())
        .unwrap();
    let (w, h) = decode_size(&bytes);
    assert_eq!(w.max(h), 300);
    assert_eq!(r["width"].as_u64().unwrap(), u64::from(w));
    assert_eq!(r["height"].as_u64().unwrap(), u64::from(h));
    // Default max_px is 1024.
    let r = run(&mut s, "document.preview", json!({ "kind": "path" }));
    assert_eq!(
        r["width"]
            .as_u64()
            .unwrap()
            .max(r["height"].as_u64().unwrap()),
        1024
    );
    assert!(!s.doc().undo.can_redo());
}

#[test]
fn preview_command_checks_max_px() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    for px in [63, 4097, 0] {
        let e = s
            .execute(
                "document.preview",
                json!({ "kind": "canvas", "max_px": px }),
                Origin::User,
            )
            .unwrap_err();
        assert!(matches!(e, CommandError::InvalidParams(_)), "{px}");
    }
    for px in [64, 4096] {
        run(
            &mut s,
            "document.preview",
            json!({ "kind": "canvas", "max_px": px }),
        );
    }
    let e = s
        .execute(
            "document.preview",
            json!({ "kind": "canvas", "bogus": 1 }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)));
    assert!(
        !registry::get("document.preview")
            .unwrap()
            .mutates_document()
    );
}

#[test]
fn test_patterns_adds_a_centred_object() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let r = run(&mut s, "tools.test_patterns", json!({}));
    let id = r["id"].as_u64().unwrap();
    assert!(r.get("svg_path").is_none());
    let doc = &s.doc().document;
    assert_eq!(doc.objects.len(), 1);
    let o = &doc.objects[0];
    assert_eq!(o.id.0, id);
    assert_eq!(o.name, "Test patterns");
    // The 280 mm sheet page is centred in the cutting area (5..299.8).
    let b = o.placed_paths();
    let b = bounds(&b).unwrap();
    // Geometry spans 10..270 on the page, so on the mat 10 mm inside the centred page.
    let page_x0 = (5.0 + 299.8) / 2.0 - 140.0;
    assert!((b.x0 - (page_x0 + 10.0)).abs() < 1e-3, "{b:?}");
    assert!((b.y0 - (page_x0 + 10.0)).abs() < 1e-3, "{b:?}");
    assert!(s.doc().undo.can_undo());
}

#[test]
fn test_patterns_writes_the_svg_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("sheet.svg");
    let r = run(
        &mut s,
        "tools.test_patterns",
        json!({ "add_to_document": false, "svg_path": path }),
    );
    assert!(r.get("id").is_none());
    assert_eq!(r["svg_path"].as_str().unwrap(), path.to_str().unwrap());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), sheet_svg());
    assert!(s.doc().document.objects.is_empty());
    let leftovers: Vec<_> = std::fs::read_dir(out.path()).unwrap().collect();
    assert_eq!(leftovers.len(), 1, "no temp file is left");
}

#[test]
fn test_patterns_refuses_agent_writes_into_the_config_dir() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let path = dir.path().join("sheet.svg");
    let e = s
        .execute(
            "tools.test_patterns",
            json!({ "add_to_document": false, "svg_path": path }),
            Origin::Agent,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "{e}");
    assert!(!path.exists());
    // The person may.
    run(
        &mut s,
        "tools.test_patterns",
        json!({ "add_to_document": false, "svg_path": path }),
    );
    assert!(path.exists());
}

#[test]
fn test_patterns_needs_something_to_do() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let e = s
        .execute(
            "tools.test_patterns",
            json!({ "add_to_document": false }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)));
}
