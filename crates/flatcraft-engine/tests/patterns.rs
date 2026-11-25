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
    for row in apexes_x.chunks(4) {
        for w in row.windows(2) {
            assert!(
                (w[1] - w[0] - 50.0).abs() < 1e-3,
                "apex pitch {}",
                w[1] - w[0]
            );
        }
    }
}

/// Builds the sheet job and checks the F in device units: 20 mm tall, stem on
/// the left spanning the full height, a long top arm above a shorter middle
/// arm, both pointing to larger device x.
fn f_check(s: &Session) -> Result<(), String> {
    let result = job::build(s).map_err(|e| e.to_string())?;
    let sheet = &s.doc().document.objects[0];
    let paths = sheet.placed_paths();
    let f_index = paths
        .iter()
        .position(|p| p.label.as_deref() == Some("f-glyph"))
        .ok_or("no f-glyph path")?;
    if !result
        .job
        .listing
        .iter()
        .any(|l| l.note.contains("[f-glyph]"))
    {
        return Err("no listing note names f-glyph".into());
    }
    let plotter = s
        .profiles
        .plotter(&s.doc().document.job.plotter_id)
        .map_err(|e| e.to_string())?;
    let media = plotter
        .media
        .iter()
        .find(|m| m.id == s.doc().document.job.media_id)
        .ok_or("no media")?;
    let pts: Vec<_> = result
        .plan
        .toolpaths
        .iter()
        .filter(|t| t.source.object == sheet.id && t.source.path_index == f_index)
        .flat_map(|t| t.points.iter().map(|p| to_device(*p, &plotter, media)))
        .collect();
    if pts.len() < 10 {
        return Err("too few F points".into());
    }
    let min_x = pts.iter().map(|p| p.x).min().unwrap();
    let min_y = pts.iter().map(|p| p.y).min().unwrap();
    let max_y = pts.iter().map(|p| p.y).max().unwrap();
    let mm = |v: f64| (v * plotter.units_per_mm).round() as i32;
    if (max_y - min_y - mm(20.0)).abs() > 1 {
        return Err("F is not 20 mm tall".into());
    }
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
    let first_y = |x: i32| pts.iter().filter(|p| p.x == x).map(|p| p.y).min().unwrap();
    // Stem on the left, the arms point right.
    if !(min_x < top_arm_x && min_x < mid_arm_x) {
        return Err("stem is not left of the arms".into());
    }
    if top_arm_x - min_x < mm(13.0) {
        return Err("top arm is not 14 mm wide".into());
    }
    if mid_arm_x - min_x < mm(10.0) || mid_arm_x >= top_arm_x {
        return Err("middle arm is not shorter than the top arm".into());
    }
    // The stem spans the full height at the left edge.
    let stem_ys: Vec<i32> = pts
        .iter()
        .filter(|p| p.x <= min_x + 1)
        .map(|p| p.y)
        .collect();
    if !(stem_ys.contains(&min_y) && stem_ys.contains(&max_y)) {
        return Err("stem does not span the full height".into());
    }
    // Top arm above the middle arm.
    if first_y(top_arm_x) >= first_y(mid_arm_x) {
        return Err("top arm is not above the middle arm".into());
    }
    Ok(())
}

#[test]
fn f_glyph_is_not_mirrored_in_device_space() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(&mut s, "tools.test_patterns", json!({}));
    f_check(&s).unwrap();
}

/// A session whose plotter is the file output profile with one axis mirrored.
fn mirrored_session(dir: &tempfile::TempDir, axis: &str) -> Session {
    let plotters = dir.path().join("plotters");
    std::fs::create_dir_all(&plotters).unwrap();
    std::fs::write(
        plotters.join("mirrored.toml"),
        format!(
            "id = \"mirrored\"\nname = \"Mirrored\"\ninherits = \"file-output-gpgl-cameo2\"\n[axis]\n{axis} = true\n"
        ),
    )
    .unwrap();
    let mut s = session(dir);
    run(&mut s, "job.set_plotter", json!({ "id": "mirrored" }));
    s
}

#[test]
fn f_glyph_check_fails_when_mirrored() {
    for axis in ["mirror_x", "mirror_y"] {
        let dir = tempfile::tempdir().unwrap();
        let mut s = mirrored_session(&dir, axis);
        let profile = s.profiles.plotter("mirrored").unwrap();
        assert_eq!(
            (profile.axis.mirror_x, profile.axis.mirror_y),
            (axis == "mirror_x", axis == "mirror_y")
        );
        run(&mut s, "tools.test_patterns", json!({}));
        assert!(f_check(&s).is_err(), "{axis}: the check must fail");
    }
}

#[test]
fn rulers_are_twelve_separate_paths_with_10_mm_ticks() {
    let paths = sheet_paths();
    for (label, along_x) in [("ruler-x", true), ("ruler-y", false)] {
        let rs = labelled(&paths, label);
        assert_eq!(rs.len(), 12, "{label}: 1 line + 11 ticks");
        for r in &rs {
            let subpaths = r
                .path
                .elements()
                .iter()
                .filter(|e| matches!(e, PathEl::MoveTo(_)))
                .count();
            assert_eq!(subpaths, 1, "{label}: one stroke per path");
        }
        let rects: Vec<Rect> = rs.iter().map(|r| r.path.bounding_box()).collect();
        let long = |r: &Rect| if along_x { r.width() } else { r.height() };
        let (lines, ticks): (Vec<_>, Vec<_>) = rects.iter().partition(|r| long(r) > 50.0);
        assert_eq!(lines.len(), 1);
        assert!((long(lines[0]) - 100.0).abs() < 1e-3);
        assert_eq!(ticks.len(), 11);
        let mut pos: Vec<f64> = ticks
            .iter()
            .map(|r| if along_x { r.x0 } else { r.y0 })
            .collect();
        pos.sort_by(f64::total_cmp);
        for w in pos.windows(2) {
            assert!((w[1] - w[0] - 10.0).abs() < 1e-3, "{label}: tick spacing");
        }
        for t in &ticks {
            assert!((if along_x { t.height() } else { t.width() } - 5.0).abs() < 1e-3);
        }
    }
}

#[test]
fn hole_and_circles_are_as_documented() {
    let paths = sheet_paths();
    let hole = labelled(&paths, "hole-in-square");
    let mut rects: Vec<Rect> = hole.iter().map(|p| p.path.bounding_box()).collect();
    rects.sort_by(|a, b| b.width().total_cmp(&a.width()));
    let (outer, inner) = (rects[0], rects[1]);
    assert!((outer.center().x - inner.center().x).abs() < 1e-3);
    assert!((outer.center().y - inner.center().y).abs() < 1e-3);

    let mut circles: Vec<Rect> = labelled(&paths, "circles")
        .iter()
        .map(|p| p.path.bounding_box())
        .collect();
    circles.sort_by(|a, b| a.width().total_cmp(&b.width()));
    let want = [
        (0.5, 10.5),
        (1.0, 22.0),
        (2.0, 35.0),
        (5.0, 52.0),
        (10.0, 77.0),
        (25.0, 122.0),
    ];
    assert_eq!(circles.len(), want.len());
    for (c, (r, cx)) in circles.iter().zip(want) {
        assert!((c.width() / 2.0 - r).abs() < 1e-3, "radius {r}");
        assert!((c.center().x - cx).abs() < 1e-3, "centre x of r {r}");
        assert!((c.center().y - 105.0).abs() < 1e-3, "centre y of r {r}");
    }
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

/// The first travel move starts at the 0,0 of the grid. The path preview
/// marks that point and labels it "H" (home), so the dashed line that
/// leaves it is not a mystery.
#[test]
fn path_preview_labels_the_travel_from_the_origin() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="150" y="150" width="20" height="20"/></svg>"##;
    run(
        &mut s,
        "object.import_svg",
        json!({ "svg": svg, "name": "far" }),
    );
    let mat = run(&mut s, "document.state", json!({}))["mat"].clone();
    let (w, h) = (
        mat["width_mm"].as_f64().unwrap(),
        mat["height_mm"].as_f64().unwrap(),
    );
    let max_px = 1024;
    let scale = f64::from(max_px) / w.max(h);
    let ox = mat["grid"]["origin_x_mm"].as_f64().unwrap() * scale;
    let oy = mat["grid"]["origin_y_mm"].as_f64().unwrap() * scale;
    let png = render_png(&s, PreviewKind::Path, max_px).unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    // Label pixels (near black) within 30 px right of and below the origin.
    let label = (0..30u32)
        .flat_map(|dy| (0..30u32).map(move |dx| (dx, dy)))
        .filter(|&(dx, dy)| {
            let p = img.get_pixel(ox as u32 + dx, oy as u32 + dy).0;
            p[0] < 0x20 && p[1] < 0x20 && p[2] < 0x20
        })
        .count();
    assert!(label > 0, "no label at the origin ({ox}, {oy})");
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
    // Only a file was written: the document is unchanged.
    assert!(!s.doc().undo.can_undo());
    assert!(!s.doc().dirty);
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

#[test]
fn test_sheet_has_no_overlapping_patterns() {
    let paths = sheet_paths();
    let mut labels: Vec<String> = paths.iter().filter_map(|p| p.label.clone()).collect();
    labels.sort();
    labels.dedup();
    // Every labelled group, with each star tip on its own.
    assert_eq!(labels.len(), 8 + 8);
    let rects: Vec<(String, Rect)> = labels
        .iter()
        .map(|l| (l.clone(), bounds_of(&labelled(&paths, l))))
        .collect();
    for (i, (an, a)) in rects.iter().enumerate() {
        for (bn, b) in &rects[i + 1..] {
            let dx = (a.x0 - b.x1).max(b.x0 - a.x1);
            let dy = (a.y0 - b.y1).max(b.y0 - a.y1);
            assert!(dx.max(dy) >= 5.0, "{an} and {bn} are closer than 5 mm");
        }
    }
}

#[test]
fn test_sheet_cuts_the_hole_before_its_square() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(&mut s, "tools.test_patterns", json!({}));
    let o = &s.doc().document.objects[0];
    let hole_paths: Vec<usize> = o
        .geometry
        .iter()
        .enumerate()
        .filter(|(_, p)| p.label.as_deref() == Some("hole-in-square"))
        .map(|(i, _)| i)
        .collect();
    assert!(!hole_paths.is_empty());
    let result = job::build(&s).unwrap();
    // (position in the cut order, width) of the hole-in-square toolpaths.
    let rings: Vec<(usize, f64)> = result
        .plan
        .toolpaths
        .iter()
        .enumerate()
        .filter(|(_, t)| hole_paths.contains(&t.source.path_index))
        .map(|(k, t)| {
            let xs = t.points.iter().map(|p| p.x);
            let w = xs.clone().fold(f64::MIN, f64::max) - xs.fold(f64::MAX, f64::min);
            (k, w)
        })
        .collect();
    assert_eq!(rings.len(), 2, "{rings:?}");
    let inner = rings.iter().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    let outer = rings.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    assert!(inner.1 < 12.0 && outer.1 > 29.0, "{rings:?}");
    assert!(inner.0 < outer.0, "the hole must be cut first: {rings:?}");
}
