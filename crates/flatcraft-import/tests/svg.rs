use kurbo::Shape;
use std::path::PathBuf;
use std::sync::Arc;

use flatcraft_core::geometry::{CutPath, bounds};
use flatcraft_import::fonts::system_fonts;
use flatcraft_import::svg::{SvgImport, import_svg};
use flatcraft_import::{ImportError, ImportWarning};

fn fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/svg")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn load(name: &str) -> SvgImport {
    import_svg(&fixture(name), Arc::new(usvg::fontdb::Database::new())).unwrap()
}

fn assert_bounds(paths: &[CutPath], exp: (f64, f64, f64, f64)) {
    let r = bounds(paths).expect("bounds");
    let got = (r.x0, r.y0, r.x1, r.y1);
    for (g, e) in [
        (got.0, exp.0),
        (got.1, exp.1),
        (got.2, exp.2),
        (got.3, exp.3),
    ] {
        assert!((g - e).abs() < 1e-4, "got {got:?}, want {exp:?}");
    }
}

#[test]
fn px_units() {
    assert_bounds(&load("px.svg").paths, (2.6458, 2.6458, 28.0458, 15.3458));
}

#[test]
fn mm_units() {
    assert_bounds(&load("mm.svg").paths, (0.0, 0.0, 50.0, 20.0));
}

#[test]
fn inch_units() {
    assert_bounds(&load("in.svg").paths, (0.0, 0.0, 25.4, 12.7));
}

#[test]
fn viewbox_scales() {
    assert_bounds(&load("viewbox-scale.svg").paths, (0.0, 0.0, 20.0, 20.0));
}

#[test]
fn hidden_elements_are_skipped() {
    let r = load("hidden.svg");
    assert_eq!(r.paths.len(), 1);
    assert_bounds(&r.paths, (0.0, 0.0, 10.0 * 25.4 / 96.0, 10.0 * 25.4 / 96.0));
}

#[test]
fn clip_path_warns() {
    let r = load("clip.svg");
    assert_eq!(r.paths.len(), 1);
    assert!(
        r.warnings
            .contains(&ImportWarning::ClipOrMaskIgnored { count: 1 })
    );
}

#[test]
fn text_becomes_paths() {
    let fonts = system_fonts();
    if fonts.is_empty() {
        eprintln!("skipping: no system fonts");
        return;
    }
    let r = import_svg(&fixture("text.svg"), fonts).unwrap();
    assert!(!r.paths.is_empty());
}

#[test]
fn colors_are_kept() {
    let r = load("colors.svg");
    assert_eq!(r.paths.len(), 3);
    assert_eq!(r.paths[0].color, Some([255, 0, 0]));
    assert_eq!(r.paths[1].color, Some([0, 0, 255]));
    assert_eq!(r.paths[2].color, Some([0, 0, 0]));
}

#[test]
fn labels_come_from_groups() {
    let r = load("colors.svg");
    assert_eq!(r.paths[0].label, None);
    assert_eq!(r.paths[1].label.as_deref(), Some("blue"));
}

#[test]
fn broken_svg_is_an_error() {
    let r = import_svg(
        &fixture("broken.svg"),
        Arc::new(usvg::fontdb::Database::new()),
    );
    assert!(matches!(r, Err(ImportError::Parse(_))));
}

#[test]
fn source_order_is_kept() {
    let r = load("colors.svg");
    let xs: Vec<f64> = r.paths.iter().map(|p| p.path.bounding_box().x0).collect();
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
}

#[test]
fn empty_svg_warns_no_paths() {
    let r = import_svg(
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#,
        Arc::new(usvg::fontdb::Database::new()),
    )
    .unwrap();
    assert!(r.paths.is_empty());
    assert_eq!(r.warnings, vec![ImportWarning::NoPaths]);
}

#[test]
fn too_large_is_rejected() {
    let big = vec![b' '; 50 * 1024 * 1024 + 1];
    let r = import_svg(&big, Arc::new(usvg::fontdb::Database::new()));
    assert!(matches!(r, Err(ImportError::TooLarge)));
}
