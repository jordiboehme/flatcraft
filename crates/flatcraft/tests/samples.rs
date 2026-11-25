//! The samples on Home: original designs made for FlatCraft from simple
//! geometry, bundled as project files under MPL-2.0.
//!
//! `regenerate_samples` (ignored) writes them to `assets/samples`:
//! `cargo nextest run -p flatcraft --test samples --run-ignored only`.

mod common;

use std::f64::consts::{PI, TAU};
use std::fmt::Write as _;

use common::{FakePicker, app, app_mut, harness_with, session, wait_for_source_check, wait_until};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::Workspace;
use flatcraft::ui::home::SAMPLES;
use flatcraft_engine::session::Origin;
use serde_json::json;

// ---------------------------------------------------------------- geometry

fn poly(points: &[(f64, f64)]) -> String {
    let mut d = String::new();
    for (i, (x, y)) in points.iter().enumerate() {
        let _ = write!(d, "{}{x:.3},{y:.3} ", if i == 0 { "M" } else { "L" });
    }
    d.push('Z');
    format!(r#"<path d="{d}"/>"#)
}

fn circle(cx: f64, cy: f64, r: f64) -> String {
    format!(r#"<circle cx="{cx:.3}" cy="{cy:.3}" r="{r:.3}"/>"#)
}

/// A 300 x 300 mm page, so the shapes keep their place on the mat.
fn svg(body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="300mm" height="300mm" viewBox="0 0 300 300">{body}</svg>"#
    )
}

fn polar(cx: f64, cy: f64, r: f64, a: f64) -> (f64, f64) {
    (cx + r * a.sin(), cy - r * a.cos())
}

/// A daisy: a circle with `petals` soft waves.
fn daisy(cx: f64, cy: f64) -> String {
    let pts: Vec<(f64, f64)> = (0..240)
        .map(|i| {
            let a = TAU * i as f64 / 240.0;
            let r = 17.0 + 3.0 * (8.0 * a).cos();
            polar(cx, cy, r, a)
        })
        .collect();
    poly(&pts)
}

/// A diamond along the direction `a`, from radius `r0` to `r1`, `w` wide.
fn diamond(cx: f64, cy: f64, a: f64, r0: f64, r1: f64, w: f64) -> String {
    let mid = (r0 + r1) / 2.0;
    let (mx, my) = polar(cx, cy, mid, a);
    let (sx, sy) = (a.cos() * w / 2.0, a.sin() * w / 2.0);
    poly(&[
        polar(cx, cy, r0, a),
        (mx + sx, my + sy),
        polar(cx, cy, r1, a),
        (mx - sx, my - sy),
    ])
}

/// Block letters without closed counters, so each falls out as one piece.
fn letter(c: char, x: f64, y: f64) -> String {
    let p = |pts: &[(f64, f64)]| poly(&pts.iter().map(|(a, b)| (x + a, y + b)).collect::<Vec<_>>());
    match c {
        'C' => p(&[
            (0.0, 0.0),
            (50.0, 0.0),
            (50.0, 14.0),
            (14.0, 14.0),
            (14.0, 50.0),
            (50.0, 50.0),
            (50.0, 64.0),
            (0.0, 64.0),
        ]),
        'U' => p(&[
            (0.0, 0.0),
            (14.0, 0.0),
            (14.0, 50.0),
            (36.0, 50.0),
            (36.0, 0.0),
            (50.0, 0.0),
            (50.0, 64.0),
            (0.0, 64.0),
        ]),
        'T' => p(&[
            (0.0, 0.0),
            (50.0, 0.0),
            (50.0, 14.0),
            (32.0, 14.0),
            (32.0, 64.0),
            (18.0, 64.0),
            (18.0, 14.0),
            (0.0, 14.0),
        ]),
        _ => unreachable!("only C, U and T"),
    }
}

// ---------------------------------------------------------------- designs

type Session = flatcraft_mcp::SharedSession;

fn import(s: &Session, name: &str, body: &str) -> u64 {
    let r = s
        .lock()
        .unwrap()
        .execute(
            "object.import_svg",
            json!({ "svg": svg(body), "name": name }),
            Origin::User,
        )
        .unwrap();
    r["id"].as_u64().unwrap()
}

fn run(s: &Session, command: &str, params: serde_json::Value) -> serde_json::Value {
    s.lock()
        .unwrap()
        .execute(command, params, Origin::User)
        .unwrap_or_else(|e| panic!("{command}: {e}"))
}

/// 12 daisies: one, then 11 linked copies in 4 columns.
fn sticker_sheet(s: &Session) {
    let id = import(s, "Daisy", &daisy(55.0, 75.0));
    run(
        s,
        "object.copies",
        json!({ "id": id, "count": 11, "layout": { "grid": { "columns": 4 } }, "gap_mm": 22.0, "linked": true }),
    );
}

/// "CUT" in a sheet with rounded corners: the letters fall out.
fn stencil(s: &Session) {
    let mut body = r#"<rect x="30" y="100" width="240" height="104" rx="8"/>"#.to_string();
    for (i, c) in ['C', 'U', 'T'].into_iter().enumerate() {
        body.push_str(&letter(c, 55.0 + i as f64 * 70.0, 120.0));
    }
    import(s, "Stencil", &body);
}

/// A tag with clipped top corners and a hole, then 5 linked copies
/// around the middle of the mat.
fn gift_tags(s: &Session) {
    let (x0, x1, top, bottom) = (132.0, 168.0, 30.0, 92.0);
    let tag = poly(&[
        (x0 + 11.0, top),
        (x1 - 11.0, top),
        (x1, top + 11.0),
        (x1, bottom),
        (x0, bottom),
        (x0, top + 11.0),
    ]);
    let hole = circle(150.0, top + 10.0, 3.5);
    let id = import(s, "Gift tag", &format!("{tag}{hole}"));
    run(
        s,
        "object.circular_copies",
        json!({ "ids": [id], "count": 6, "center_mm": [150.0, 150.0], "linked": true }),
    );
}

/// An eight-point star with three rings of inner shapes.
fn window_star(s: &Session) {
    let (cx, cy) = (150.0, 150.0);
    let outer: Vec<(f64, f64)> = (0..16)
        .map(|i| {
            let a = PI / 8.0 * i as f64;
            polar(cx, cy, if i % 2 == 0 { 125.0 } else { 85.0 }, a)
        })
        .collect();
    let mut body = poly(&outer);
    for k in 0..8 {
        let a = PI / 4.0 * k as f64;
        body.push_str(&diamond(cx, cy, a, 52.0, 106.0, 16.0));
        body.push_str(&diamond(cx, cy, a + PI / 8.0, 48.0, 74.0, 11.0));
        let (x, y) = polar(cx, cy, 33.0, a + PI / 8.0);
        body.push_str(&circle(x, y, 5.0));
    }
    body.push_str(&circle(cx, cy, 14.0));
    import(s, "Window star", &body);
}

fn build(name: &str) -> (tempfile::TempDir, Session) {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    match name {
        "sticker-sheet" => sticker_sheet(&s),
        "stencil" => stencil(&s),
        "gift-tags" => gift_tags(&s),
        "window-star" => window_star(&s),
        other => panic!("no sample {other}"),
    }
    (dir, s)
}

#[test]
#[ignore = "writes assets/samples; run it after changing a design"]
fn regenerate_samples() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples");
    std::fs::create_dir_all(&out).unwrap();
    for sample in &SAMPLES {
        let (_dir, s) = build(sample.id);
        let doc = s.lock().unwrap().doc().document.clone();
        let value = flatcraft_core::project::to_value(&doc).unwrap();
        let text = serde_json::to_string_pretty(&value).unwrap() + "\n";
        std::fs::write(out.join(format!("{}.flatcraft", sample.id)), text).unwrap();
    }
}

// ---------------------------------------------------------------- checks

/// `a` and `b` agree, numbers within 1e-6 (a JSON round trip may change
/// the last digit). Returns the path of the first difference.
fn differs(a: &serde_json::Value, b: &serde_json::Value, at: &str) -> Option<String> {
    use serde_json::Value;
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            ((x - y).abs() > 1e-6).then(|| format!("{at}: {x} != {y}"))
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
            .iter()
            .zip(y)
            .enumerate()
            .find_map(|(i, (p, q))| differs(p, q, &format!("{at}[{i}]"))),
        (Value::Object(x), Value::Object(y)) if x.len() == y.len() => {
            x.iter().find_map(|(k, p)| match y.get(k) {
                Some(q) => differs(p, q, &format!("{at}.{k}")),
                None => Some(format!("{at}.{k} is missing")),
            })
        }
        _ => (a != b).then(|| format!("{at} differs")),
    }
}

#[test]
fn the_bundled_samples_are_the_designs_of_this_file() {
    for sample in &SAMPLES {
        let (_dir, s) = build(sample.id);
        let doc = s.lock().unwrap().doc().document.clone();
        let value = flatcraft_core::project::to_value(&doc).unwrap();
        let bundled: serde_json::Value = serde_json::from_str(sample.text).unwrap();
        if let Some(d) = differs(&value, &bundled, sample.id) {
            panic!("{d}: run regenerate_samples");
        }
    }
}

#[test]
fn every_sample_loads_plans_without_warnings_and_fits_the_default_mat() {
    for sample in &SAMPLES {
        assert!(!sample.text.contains("/Users"), "{}", sample.id);
        assert!(
            !sample.text.contains("\"link\""),
            "{} has a file link",
            sample.id
        );
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        let mut st = s.lock().unwrap();
        let r = flatcraft_engine::commands::project::open_text(&mut st, sample.text, sample.title)
            .unwrap();
        assert_eq!(r["warnings"], json!([]), "{}", sample.id);
        assert!(st.doc().path.is_none());
        let state = st
            .execute("document.state", json!({}), Origin::User)
            .unwrap();
        let area = &state["cutting_area_mm"];
        let (ax0, ay0, ax1, ay1) = (
            area["x0"].as_f64().unwrap(),
            area["y0"].as_f64().unwrap(),
            area["x1"].as_f64().unwrap(),
            area["y1"].as_f64().unwrap(),
        );
        for o in state["objects"].as_array().unwrap() {
            let b = &o["bounds_mm"];
            assert!(
                b["x0"].as_f64().unwrap() >= ax0
                    && b["y0"].as_f64().unwrap() >= ay0
                    && b["x1"].as_f64().unwrap() <= ax1
                    && b["y1"].as_f64().unwrap() <= ay1,
                "{}: {} at {b} is outside {area}",
                sample.id,
                o["name"]
            );
        }
        let job = flatcraft_engine::job::build(&st).unwrap();
        assert!(job.warnings.is_empty(), "{}: {:?}", sample.id, job.warnings);
        assert!(!job.plan.toolpaths.is_empty());
    }
}

#[test]
fn the_window_star_cuts_its_inner_shapes_first() {
    let sample = SAMPLES.iter().find(|x| x.id == "window-star").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut st = s.lock().unwrap();
    flatcraft_engine::commands::project::open_text(&mut st, sample.text, "star").unwrap();
    let job = flatcraft_engine::job::build(&st).unwrap();
    let paths = &job.plan.toolpaths;
    assert_eq!(paths.len(), 8 * 3 + 2, "25 inner shapes and the star");
    // The star is the one path that reaches 125 mm from the middle.
    let reach = |i: usize| {
        paths[i]
            .points
            .iter()
            .map(|p| ((p.x - 150.0).powi(2) + (p.y - 150.0).powi(2)).sqrt())
            .fold(0.0, f64::max)
    };
    let star = (0..paths.len())
        .max_by(|&a, &b| reach(a).total_cmp(&reach(b)))
        .unwrap();
    assert_eq!(star, paths.len() - 1, "the outline is cut last");
}

#[test]
fn the_patterns_are_linked_copies() {
    for (id, n) in [("sticker-sheet", 12), ("gift-tags", 6)] {
        let sample = SAMPLES.iter().find(|x| x.id == id).unwrap();
        let (doc, _) = flatcraft_core::project::load_report_str(sample.text).unwrap();
        assert_eq!(doc.objects.len(), n, "{id}");
        let group = doc.objects[0].link_group;
        assert!(group.is_some(), "{id}");
        assert!(doc.objects.iter().all(|o| o.link_group == group), "{id}");
    }
}

#[test]
fn a_sample_opens_as_an_untitled_copy() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let before: Vec<Vec<u8>> = SAMPLES.iter().map(|x| x.text.as_bytes().to_vec()).collect();
    let picker = FakePicker::default();
    let requests = picker.requests.clone();
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker));
    h.get_by_role_and_label(Role::Button, "Gift tags").click();
    wait_for_source_check(&mut h);
    {
        let st = s.lock().unwrap();
        assert_eq!(st.docs.len(), 1, "the empty start document is used");
        assert!(st.doc().path.is_none(), "untitled");
        assert_eq!(st.doc().document.objects.len(), 6);
    }
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
    h.get_by_role_and_label(Role::Button, "Untitled 1");
    // Save asks where: the bundled file is never the target.
    let doc = s.lock().unwrap().doc().id();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![flatcraft::ui::Action::Save { doc_id: doc }]);
    assert_eq!(requests.lock().unwrap()[0].title, "Save As");
    // A second sample opens next to it.
    app_mut(&mut h).ui.workspace = Workspace::Home;
    h.run();
    h.get_by_role_and_label(Role::Button, "Stencil").click();
    wait_for_source_check(&mut h);
    assert_eq!(s.lock().unwrap().docs.len(), 2);
    let after: Vec<Vec<u8>> = SAMPLES.iter().map(|x| x.text.as_bytes().to_vec()).collect();
    assert_eq!(before, after);
    // Nothing was written next to the samples or in the project folder.
    let samples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples");
    for sample in &SAMPLES {
        let on_disk =
            std::fs::read_to_string(samples.join(format!("{}.flatcraft", sample.id))).unwrap();
        assert_eq!(on_disk, sample.text);
    }
}

#[test]
fn sample_pictures_come_from_the_preview_renderer() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness_with(&dir, s.clone(), |a| a.with_inline_pictures(false));
    wait_until(&mut h, |a| a.ui.home.busy());
    for sample in &SAMPLES {
        let id = flatcraft::ui::home::sample_id(sample);
        let (version, thumb) = app(&h).ui.home.thumbs.get(&id).unwrap();
        assert_eq!(
            *version,
            flatcraft_engine::thumbnails::text_version(sample.text)
        );
        let flatcraft::ui::home::Thumb::Ready(tex) = thumb else {
            panic!("{} has no picture: {thumb:?}", sample.id);
        };
        // The renderer's size: the mat's larger side is SAMPLE_PX.
        let [w, h] = tex.size();
        assert_eq!(w.max(h), flatcraft::ui::home::SAMPLE_PX as usize);
    }
    // Kept in memory, never written next to the samples or in the cache.
    assert!(!dir.path().join(flatcraft_engine::thumbnails::DIR).exists());
}

#[test]
fn the_gift_tags_cut_each_hole_before_its_tag() {
    let sample = SAMPLES.iter().find(|x| x.id == "gift-tags").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut st = s.lock().unwrap();
    flatcraft_engine::commands::project::open_text(&mut st, sample.text, "tags").unwrap();
    let job = flatcraft_engine::job::build(&st).unwrap();
    let boxes: Vec<kurbo::Rect> = job
        .plan
        .toolpaths
        .iter()
        .map(|t| {
            t.points.iter().fold(
                kurbo::Rect::from_points(t.points[0], t.points[0]),
                |r, p| r.union_pt(*p),
            )
        })
        .collect();
    let holes: Vec<usize> = (0..boxes.len())
        .filter(|&i| boxes[i].width() < 10.0)
        .collect();
    assert_eq!(holes.len(), 6, "one hole per tag");
    assert_eq!(boxes.len(), 12);
    for h in holes {
        let tag = (0..boxes.len())
            .find(|&i| i != h && boxes[i].width() >= 10.0 && boxes[i].contains(boxes[h].center()))
            .unwrap_or_else(|| panic!("hole {h} is in no tag"));
        assert!(h < tag, "hole {h} is cut after its tag {tag}");
    }
}
