//! Export of what will be cut: SVG, DXF and PDF, the scope rules, skipped
//! and hidden objects, the agent limit and the placement.

use std::path::Path;
use std::sync::Arc;

use flatcraft_device::ProfileStore;
use flatcraft_engine::export::{self, ExportFormat, ExportScope};
use flatcraft_engine::job::JobInput;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{CommandError, Origin, Session};
use kurbo::Point;
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

/// A square and a circle, so the export has a polygon and a flattened curve.
const SHAPES: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="20" y="20" width="30" height="30"/><circle cx="70" cy="70" r="15"/></svg>"#;

/// A right triangle with its right angle at the top left: not symmetric
/// in x or y, so a flip shows.
const TRIANGLE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><path d="M 10 10 L 60 10 L 10 30 Z"/></svg>"#;

fn import(s: &mut Session, svg: &str, name: &str) -> u64 {
    run(s, "object.import_svg", json!({ "svg": svg, "name": name }))["id"]
        .as_u64()
        .unwrap()
}

fn select(s: &mut Session, ids: &[u64]) {
    run(s, "object.select", json!({ "ids": ids }));
}

fn export_doc(s: &Session, ids: Option<&[u64]>, include_skipped: bool) -> export::ExportDoc {
    let input = JobInput::of(s).unwrap();
    let ids: Option<Vec<_>> = ids.map(|v| {
        v.iter()
            .map(|i| flatcraft_core::document::ObjectId(*i))
            .collect()
    });
    export::collect(&input, ids.as_deref(), include_skipped).unwrap()
}

fn export_to(s: &mut Session, path: &Path, format: &str, extra: Value) -> Value {
    let mut params = json!({ "path": path, "format": format });
    if let Value::Object(m) = extra {
        for (k, v) in m {
            params[k] = v;
        }
    }
    run(s, "document.export", params)
}

fn all_points(doc: &export::ExportDoc) -> Vec<Point> {
    doc.objects
        .iter()
        .flat_map(|o| o.shapes.iter())
        .flat_map(|sh| sh.points.iter().copied())
        .collect()
}

fn near(a: Point, b: Point, tol: f64) -> bool {
    a.distance(b) <= tol
}

// ---------------------------------------------------------------- SVG

#[test]
fn svg_reads_back_to_the_same_geometry() {
    let d = dirs();
    let mut s = session(&d);
    import(&mut s, SHAPES, "shapes");
    let doc = export_doc(&s, None, false);
    assert_eq!(doc.shape_count(), 2);
    let path = d.out.path().join("shapes.svg");
    let r = export_to(&mut s, &path, "svg", json!({}));
    assert_eq!(r["shapes"], 2);
    assert_eq!(r["path"].as_str().unwrap(), path.to_str().unwrap());

    let bytes = std::fs::read(&path).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(
        text.contains(&format!(r#"width="{}mm""#, export::num(doc.width_mm))),
        "{text}"
    );
    assert!(text.contains(&format!(
        r#"viewBox="0 0 {} {}""#,
        export::num(doc.width_mm),
        export::num(doc.height_mm)
    )));
    assert!(text.contains(r#"data-name="shapes""#), "{text}");
    assert!(text.contains(r#"fill="none""#));
    assert!(!text.contains("inkscape"));

    let fonts = Arc::new(usvg::fontdb::Database::new());
    let back = flatcraft_import::svg::import_svg(&bytes, fonts).unwrap();
    assert!((back.page_frame_mm.width() - doc.width_mm).abs() < 0.01);
    assert!((back.page_frame_mm.height() - doc.height_mm).abs() < 0.01);
    assert_eq!(back.paths.len(), 2);
    let ours = &doc.objects[0].shapes;
    for (sh, p) in ours.iter().zip(&back.paths) {
        // Read back as the cut reads paths: flattened, joined and with
        // collinear points merged at the default tolerance.
        let lines = flatcraft_path::shape_lines(&p.path, 0.01, 0.01);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].closed, sh.closed);
        let mut theirs = lines[0].points.clone();
        if lines[0].closed {
            theirs.pop();
        }
        // Both directions: the same number of vertices, each exported
        // vertex in the file and each vertex of the file exported, so a
        // stray extra or missing vertex fails.
        assert_eq!(theirs.len(), sh.points.len());
        for v in &sh.points {
            assert!(
                theirs.iter().any(|q| near(*v, *q, 0.01)),
                "{v:?} is missing"
            );
        }
        for q in &theirs {
            assert!(
                sh.points.iter().any(|v| near(*v, *q, 0.01)),
                "{q:?} was not exported"
            );
        }
    }
}

#[test]
fn names_are_escaped_in_svg() {
    let d = dirs();
    let mut s = session(&d);
    import(&mut s, TRIANGLE, "a <b> & \"c\"");
    let doc = export_doc(&s, None, false);
    let text = String::from_utf8(export::write(&doc, ExportFormat::Svg)).unwrap();
    assert!(
        text.contains(r#"data-name="a &lt;b&gt; &amp; &quot;c&quot;""#),
        "{text}"
    );
    let fonts = Arc::new(usvg::fontdb::Database::new());
    assert!(flatcraft_import::svg::import_svg(text.as_bytes(), fonts).is_ok());
}

// ---------------------------------------------------------------- DXF

/// The (code, value) pairs of an ASCII DXF.
fn dxf_pairs(text: &str) -> Vec<(i32, String)> {
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len() % 2, 0, "pairs");
    lines
        .chunks(2)
        .map(|c| (c[0].trim().parse().unwrap(), c[1].trim().to_string()))
        .collect()
}

#[test]
fn dxf_is_r12_in_mm_with_y_up() {
    let d = dirs();
    let mut s = session(&d);
    import(&mut s, TRIANGLE, "Tri angle/1");
    let doc = export_doc(&s, None, false);
    let path = d.out.path().join("tri.dxf");
    let r = export_to(&mut s, &path, "dxf", json!({}));
    assert_eq!(r["shapes"], 1);
    let text = std::fs::read_to_string(&path).unwrap();
    let pairs = dxf_pairs(&text);
    let at = |code: i32, value: &str| pairs.iter().position(|p| p.0 == code && p.1 == value);
    // Header: R12, millimetres.
    let ver = at(9, "$ACADVER").unwrap();
    assert_eq!(pairs[ver + 1], (1, "AC1009".to_string()));
    let units = at(9, "$INSUNITS").unwrap();
    assert_eq!(pairs[units + 1], (70, "4".to_string()));
    // The layer is declared in the LAYER table, with a sanitised name.
    let layer = "TRI_ANGLE_1";
    assert!(
        pairs
            .windows(2)
            .any(|w| w[0] == (0, "LAYER".into()) && w[1] == (2, layer.into())),
        "{text}"
    );
    // One closed POLYLINE on that layer, three vertices (no repeated end).
    let poly = at(0, "POLYLINE").unwrap();
    let seqend = at(0, "SEQEND").unwrap();
    let ent = &pairs[poly..seqend];
    assert!(ent.contains(&(8, layer.to_string())));
    assert!(ent.contains(&(66, "1".to_string())));
    assert!(ent.contains(&(70, "1".to_string())));
    let mut vertices = Vec::new();
    let mut i = 0;
    while i < ent.len() {
        if ent[i] == (0, "VERTEX".to_string()) {
            let x: f64 = ent[i..]
                .iter()
                .find(|p| p.0 == 10)
                .unwrap()
                .1
                .parse()
                .unwrap();
            let y: f64 = ent[i..]
                .iter()
                .find(|p| p.0 == 20)
                .unwrap()
                .1
                .parse()
                .unwrap();
            vertices.push(Point::new(x, y));
        }
        i += 1;
    }
    assert_eq!(vertices.len(), 3);
    assert!(text.trim_end().ends_with("EOF"));
    // y is flipped against the mat: y_dxf = height - y_mat. The triangle's
    // long edge is at the top of the mat, so in the DXF it is at the top
    // too (the higher y).
    let ours = &doc.objects[0].shapes[0].points;
    for (v, p) in vertices.iter().zip(ours) {
        assert!((v.x - p.x).abs() < 1e-3, "{v:?} {p:?}");
        assert!((v.y - (doc.height_mm - p.y)).abs() < 1e-3, "{v:?} {p:?}");
    }
    let top = vertices.iter().map(|v| v.y).fold(f64::MIN, f64::max);
    let count_top = vertices.iter().filter(|v| (v.y - top).abs() < 1e-6).count();
    assert_eq!(count_top, 2, "the long edge is the top edge: {vertices:?}");
}

#[test]
fn dxf_layer_names_are_unique() {
    let d = dirs();
    let mut s = session(&d);
    import(&mut s, TRIANGLE, "a b");
    import(&mut s, TRIANGLE, "a/b");
    let doc = export_doc(&s, None, false);
    let text = String::from_utf8(export::write(&doc, ExportFormat::Dxf)).unwrap();
    let pairs = dxf_pairs(&text);
    let mut layers: Vec<String> = pairs
        .windows(2)
        .filter(|w| w[0] == (0, "LAYER".into()) && w[1].0 == 2)
        .map(|w| w[1].1.clone())
        .collect();
    assert_eq!(layers.len(), 2, "{layers:?}");
    layers.dedup();
    assert_eq!(layers.len(), 2, "{layers:?}");
}

// ---------------------------------------------------------------- PDF

#[test]
fn pdf_is_one_page_the_size_of_the_grid() {
    let d = dirs();
    let mut s = session(&d);
    import(&mut s, SHAPES, "shapes");
    let doc = export_doc(&s, None, false);
    let path = d.out.path().join("shapes.pdf");
    let r = export_to(&mut s, &path, "pdf", json!({}));
    assert_eq!(r["shapes"], 2);
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    assert!(bytes.ends_with(b"%%EOF") || bytes.ends_with(b"%%EOF\n"));
    let text = String::from_utf8_lossy(&bytes);
    // 1:1 scale: the page is the grid in points.
    let pt = |mm: f64| mm * 72.0 / 25.4;
    let mb = text.find("/MediaBox [").unwrap();
    let rest = &text[mb + "/MediaBox [".len()..];
    let nums: Vec<f64> = rest[..rest.find(']').unwrap()]
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect();
    assert_eq!(nums.len(), 4);
    assert!((nums[2] - pt(doc.width_mm)).abs() < 0.01, "{nums:?}");
    assert!((nums[3] - pt(doc.height_mm)).abs() < 0.01, "{nums:?}");
    assert_eq!(text.matches("/Type /Page\n").count(), 1, "{text}");
    // One subpath per shape, stroked, never filled.
    let start = text.find("stream\n").unwrap() + "stream\n".len();
    let content = &text[start..start + text[start..].find("endstream").unwrap()];
    let ops: Vec<&str> = content
        .lines()
        .filter_map(|l| l.split_whitespace().last())
        .collect();
    assert_eq!(ops.iter().filter(|o| **o == "m").count(), 2);
    assert!(ops.contains(&"S"));
    assert!(
        !ops.iter()
            .any(|o| matches!(*o, "f" | "F" | "f*" | "B" | "b"))
    );
    // Drawn in mm with y down: the transform maps the mat onto the page.
    let k = 72.0 / 25.4;
    let cm = content
        .lines()
        .find(|l| l.ends_with(" cm"))
        .expect("a transform");
    let m: Vec<f64> = cm
        .split_whitespace()
        .take(6)
        .map(|n| n.parse().unwrap())
        .collect();
    assert!((m[0] - k).abs() < 1e-4 && (m[3] + k).abs() < 1e-4, "{cm}");
    assert!((m[5] - pt(doc.height_mm)).abs() < 0.01, "{cm}");
}

// ---------------------------------------------------------------- scope

#[test]
fn scope_is_the_selection_when_there_is_one() {
    let d = dirs();
    let mut s = session(&d);
    let a = import(&mut s, TRIANGLE, "a");
    let b = import(&mut s, SHAPES, "b");
    select(&mut s, &[a]);
    let path = d.out.path().join("x.svg");
    // Default with a selection: the selection.
    assert_eq!(export_to(&mut s, &path, "svg", json!({}))["shapes"], 1);
    assert_eq!(
        export_to(&mut s, &path, "svg", json!({ "scope": "all" }))["shapes"],
        3
    );
    select(&mut s, &[b]);
    assert_eq!(
        export_to(&mut s, &path, "svg", json!({ "scope": "selected" }))["shapes"],
        2
    );
    // Nothing selected: all, and "selected" says why it cannot.
    select(&mut s, &[]);
    assert_eq!(export_to(&mut s, &path, "svg", json!({}))["shapes"], 3);
    let e = s
        .execute(
            "document.export",
            json!({ "path": path, "format": "svg", "scope": "selected" }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::InvalidParams(_)), "{e}");
    assert_eq!(ExportScope::default_for(true), ExportScope::Selected);
    assert_eq!(ExportScope::default_for(false), ExportScope::All);
}

#[test]
fn skipped_objects_only_with_include_and_hidden_never() {
    let d = dirs();
    let mut s = session(&d);
    let a = import(&mut s, TRIANGLE, "a");
    let b = import(&mut s, SHAPES, "b");
    let c = import(&mut s, TRIANGLE, "c");
    run(
        &mut s,
        "object.set_cut",
        json!({ "ids": [b], "cut": false }),
    );
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [c], "visible": false }),
    );
    select(&mut s, &[]);
    let names = |doc: &export::ExportDoc| -> Vec<String> {
        doc.objects.iter().map(|o| o.name.clone()).collect()
    };
    assert_eq!(names(&export_doc(&s, None, false)), ["a"]);
    assert_eq!(names(&export_doc(&s, None, true)), ["a", "b"]);
    let path = d.out.path().join("x.dxf");
    assert_eq!(export_to(&mut s, &path, "dxf", json!({}))["shapes"], 1);
    assert_eq!(
        export_to(&mut s, &path, "dxf", json!({ "include_skipped": true }))["shapes"],
        3
    );
    // Only skipped or hidden objects: nothing to export, and it says why.
    run(
        &mut s,
        "object.set_cut",
        json!({ "ids": [a], "cut": false }),
    );
    let e = s
        .execute(
            "document.export",
            json!({ "path": path, "format": "svg" }),
            Origin::User,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "{e}");
    assert!(e.to_string().contains("Nothing to export"), "{e}");
    assert!(e.to_string().contains("skipped"), "{e}");
}

#[test]
fn an_empty_document_has_nothing_to_export() {
    let d = dirs();
    let mut s = session(&d);
    let path = d.out.path().join("x.pdf");
    let e = s
        .execute(
            "document.export",
            json!({ "path": path, "format": "pdf" }),
            Origin::User,
        )
        .unwrap_err();
    assert!(e.to_string().contains("Nothing to export"), "{e}");
    assert!(!path.exists());
}

// ---------------------------------------------------------------- agent limit

#[test]
fn agents_may_not_export_into_the_config_dir() {
    let d = dirs();
    let mut s = session(&d);
    import(&mut s, TRIANGLE, "a");
    let inside = d.config.path().join("cut.svg");
    let e = s
        .execute(
            "document.export",
            json!({ "path": inside, "format": "svg" }),
            Origin::Agent,
        )
        .unwrap_err();
    assert!(matches!(e, CommandError::Refused(_)), "{e}");
    assert!(e.to_string().contains("config directory"), "{e}");
    assert!(!inside.exists());
    // Elsewhere an agent may write, and the person may write anywhere.
    let outside = d.out.path().join("cut.svg");
    let r = s
        .execute(
            "document.export",
            json!({ "path": outside, "format": "svg" }),
            Origin::Agent,
        )
        .unwrap();
    assert_eq!(r["shapes"], 1);
    assert!(outside.exists());
    run(
        &mut s,
        "document.export",
        json!({ "path": inside, "format": "svg" }),
    );
    assert!(inside.exists());
}

#[test]
fn export_changes_nothing_in_the_document() {
    let d = dirs();
    let mut s = session(&d);
    import(&mut s, TRIANGLE, "a");
    let before = s.doc().revision();
    let dirty = s.doc().dirty;
    export_to(&mut s, &d.out.path().join("a.svg"), "svg", json!({}));
    assert_eq!(s.doc().revision(), before);
    assert_eq!(s.doc().dirty, dirty);
    assert!(s.doc().path.is_none(), "an export is not a save");
}

// ---------------------------------------------------------------- placement

#[test]
fn mirror_and_rotation_reach_the_export() {
    let d = dirs();
    let mut s = session(&d);
    let id = import(&mut s, TRIANGLE, "tri");
    let before = all_points(&export_doc(&s, None, false));
    let (x0, x1) = before
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let (y0, y1) = before
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    let centre = Point::new((x0 + x1) / 2.0, (y0 + y1) / 2.0);

    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [id], "axis": "horizontal" }),
    );
    let mirrored = all_points(&export_doc(&s, None, false));
    assert_eq!(mirrored.len(), before.len());
    for p in &before {
        let want = Point::new(2.0 * centre.x - p.x, p.y);
        assert!(
            mirrored.iter().any(|q| near(*q, want, 0.01)),
            "{want:?} not in {mirrored:?}"
        );
    }

    run(
        &mut s,
        "object.rotate",
        json!({ "ids": [id], "degrees": 90.0 }),
    );
    let rotated = all_points(&export_doc(&s, None, false));
    let (rx0, rx1) = rotated
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let (ry0, ry1) = rotated
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    // A quarter turn swaps width and height.
    assert!(((rx1 - rx0) - (y1 - y0)).abs() < 0.01);
    assert!(((ry1 - ry0) - (x1 - x0)).abs() < 0.01);
    assert!(rotated != mirrored);

    // The file holds the same placed points.
    let path = d.out.path().join("tri.svg");
    export_to(&mut s, &path, "svg", json!({}));
    let fonts = Arc::new(usvg::fontdb::Database::new());
    let back = flatcraft_import::svg::import_svg(&std::fs::read(&path).unwrap(), fonts).unwrap();
    let lines = flatcraft_path::shape_lines(&back.paths[0].path, 0.001, 0.01);
    for p in &rotated {
        assert!(lines[0].points.iter().any(|q| near(*p, *q, 0.01)), "{p:?}");
    }
}

#[test]
fn the_export_is_the_geometry_the_cut_uses() {
    // The cut's toolpaths (before order and overcut) and the export hold
    // the same vertices.
    let d = dirs();
    let mut s = session(&d);
    import(&mut s, SHAPES, "shapes");
    let doc = export_doc(&s, None, false);
    let result = flatcraft_engine::job::build(&s).unwrap();
    assert_eq!(result.plan.toolpaths.len(), doc.shape_count());
    for sh in doc.objects.iter().flat_map(|o| &o.shapes) {
        let found = result.plan.toolpaths.iter().any(|t| {
            let lap = t.lap();
            sh.points
                .iter()
                .all(|p| lap.iter().any(|q| near(*p, *q, 1e-9)))
        });
        assert!(found, "{sh:?}");
    }
}
