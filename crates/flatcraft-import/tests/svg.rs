use kurbo::Shape;
use std::path::PathBuf;
use std::sync::Arc;

use flatcraft_core::geometry::{CutPath, bounds};
use flatcraft_import::svg::{SvgImport, import_svg};
use flatcraft_import::{ImportError, ImportWarning};

fn fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/svg")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// IBM Plex Sans from the repository (OFL), so text renders the same on
/// every machine, whatever fonts it has installed.
fn plex() -> Arc<usvg::fontdb::Database> {
    let mut db = usvg::fontdb::Database::new();
    let font = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/fonts/IBMPlexSans-Regular.ttf");
    db.load_font_file(&font)
        .unwrap_or_else(|e| panic!("{}: {e}", font.display()));
    assert_eq!(db.len(), 1);
    Arc::new(db)
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
    let r = import_svg(&fixture("text.svg"), plex()).unwrap();
    assert!(!r.paths.is_empty());
    assert!(
        !r.warnings.contains(&ImportWarning::TextSkipped),
        "{:?}",
        r.warnings
    );
}

#[test]
fn text_without_its_font_is_skipped_with_a_warning() {
    let r = import_svg(
        &fixture("text.svg"),
        Arc::new(usvg::fontdb::Database::new()),
    )
    .unwrap();
    assert!(r.paths.is_empty());
    assert!(
        r.warnings.contains(&ImportWarning::TextSkipped),
        "{:?}",
        r.warnings
    );
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

fn gzip(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

#[test]
fn svgz_is_decompressed() {
    let r = import_svg(
        &gzip(&fixture("mm.svg")),
        Arc::new(usvg::fontdb::Database::new()),
    )
    .unwrap();
    assert_bounds(&r.paths, (0.0, 0.0, 50.0, 20.0));
}

/// `gzip -c a b > c` and `cat a.gz b.gz` give one file of several gzip
/// members; all of them are part of the SVG.
#[test]
fn svgz_with_several_gzip_members_is_read_whole() {
    let svg = fixture("mm.svg");
    let (a, b) = svg.split_at(svg.len() / 2);
    let mut bytes = gzip(a);
    bytes.extend(gzip(b));
    let r = import_svg(&bytes, Arc::new(usvg::fontdb::Database::new())).unwrap();
    assert_bounds(&r.paths, (0.0, 0.0, 50.0, 20.0));
}

#[test]
fn entity_expansion_is_refused_or_bounded() {
    let start = std::time::Instant::now();
    let r = import_svg(
        &fixture("entities.svg"),
        Arc::new(usvg::fontdb::Database::new()),
    );
    // Generous: a real expansion takes far longer than this.
    assert!(start.elapsed() < std::time::Duration::from_secs(10));
    assert!(matches!(r, Err(ImportError::Parse(_))), "{r:?}");
}

#[test]
fn nested_group_transform_in_scaled_viewbox() {
    let r = load("nested.svg");
    assert_bounds(&r.paths[..1], (30.0, 0.0, 50.0, 20.0));
}

#[test]
fn inner_group_id_wins() {
    let r = load("nested.svg");
    assert_eq!(r.paths[0].label.as_deref(), Some("inner"));
    assert_eq!(r.paths[1].label.as_deref(), Some("outer"));
}

#[test]
fn stroke_only_path_bounds_are_the_centerline() {
    let r = load("stroke.svg");
    assert_bounds(&r.paths, (10.0, 10.0, 40.0, 30.0));
}

#[test]
fn text_bounds_follow_group_transform() {
    let fonts = plex();
    let a = bounds(
        &import_svg(&fixture("text.svg"), fonts.clone())
            .unwrap()
            .paths,
    )
    .unwrap();
    let b = bounds(
        &import_svg(&fixture("text-shifted.svg"), fonts)
            .unwrap()
            .paths,
    )
    .unwrap();
    let dx = 48.0 * 25.4 / 96.0;
    assert!((b.x0 - a.x0 - dx).abs() < 1e-4 && (b.x1 - a.x1 - dx).abs() < 1e-4);
    assert!((b.y0 - a.y0).abs() < 1e-4 && (b.y1 - a.y1).abs() < 1e-4);
}

fn tiny_png() -> Vec<u8> {
    let img = image::GrayImage::from_pixel(4, 4, image::Luma([0]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// usvg makes an image node only for an href it could load, and every
/// image node gives an `ImagesSkipped` warning. So a real PNG behind the
/// href shows whether the file was read.
#[test]
fn external_image_file_is_not_read() {
    use base64::Engine as _;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pixel.png");
    std::fs::write(&file, tiny_png()).unwrap();
    let svg = |href: &str| {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10"><image width="5" height="5" xlink:href="{href}"/><rect width="1" height="1"/></svg>"#
        )
    };
    let skipped = |svg: String| {
        import_text(&svg)
            .warnings
            .iter()
            .any(|w| matches!(w, ImportWarning::ImagesSkipped { .. }))
    };
    // The check can see an image: an embedded one is decoded and skipped.
    let data = base64::engine::general_purpose::STANDARD.encode(tiny_png());
    assert!(skipped(svg(&format!("data:image/png;base64,{data}"))));
    // A file on disk is never opened, by absolute path or file URL.
    let path = file.display().to_string();
    assert!(!skipped(svg(&path)), "the file {path} was read");
    assert!(
        !skipped(svg(&format!("file://{path}"))),
        "the file URL was read"
    );
}

// ---------------------------------------------------------------- paint

fn import_text(svg: &str) -> SvgImport {
    import_svg(svg.as_bytes(), Arc::new(usvg::fontdb::Database::new())).unwrap()
}

#[test]
fn stroke_info_is_read() {
    use flatcraft_core::geometry::{Cap, Join};
    let r = import_text(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><path d="M10 10 H90" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"/></svg>"#,
    );
    assert_eq!(r.paths.len(), 1);
    let paint = r.paths[0].paint;
    assert_eq!(paint.fill, None);
    let s = paint.stroke.expect("stroke");
    assert!((s.width_mm - 0.5292).abs() < 1e-4, "{}", s.width_mm);
    assert_eq!(s.join, Join::Miter { limit: 4.0 });
    assert_eq!(s.cap, Cap::Round);
}

#[test]
fn fill_rule_is_read() {
    use flatcraft_core::geometry::FillRule;
    let r = import_text(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><path d="M0 0 H30 V30 H0 Z M10 10 H20 V20 H10 Z" fill-rule="evenodd"/></svg>"#,
    );
    assert_eq!(r.paths.len(), 1);
    assert_eq!(r.paths[0].paint.fill, Some(FillRule::EvenOdd));
    assert_eq!(r.paths[0].paint.stroke, None);
    let plain = import_text(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="10" height="10"/></svg>"#,
    );
    assert_eq!(plain.paths[0].paint.fill, Some(FillRule::NonZero));
}

#[test]
fn scaled_group_scales_stroke_width() {
    let r = import_text(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><g transform="scale(2)"><path d="M10 10 H50" fill="none" stroke="black" stroke-width="1.5" stroke-linejoin="bevel"/></g></svg>"#,
    );
    let s = r.paths[0].paint.stroke.expect("stroke");
    assert!((s.width_mm - 3.0).abs() < 1e-4, "{}", s.width_mm);
    assert_eq!(s.join, flatcraft_core::geometry::Join::Bevel);
}

#[test]
fn path_without_fill_and_stroke_is_skipped() {
    let r = import_text(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><path d="M0 0 H30" fill="none" stroke="none"/><rect width="10" height="10"/></svg>"#,
    );
    assert_eq!(r.paths.len(), 1);
}

#[test]
fn dashes_warn() {
    let r = import_text(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><path d="M10 10 H90" fill="none" stroke="black" stroke-width="2" stroke-dasharray="4 2"/><path d="M10 20 H90" fill="none" stroke="black"/></svg>"#,
    );
    assert_eq!(r.paths.len(), 2);
    assert!(
        r.warnings
            .contains(&ImportWarning::DashesIgnored { count: 1 }),
        "{:?}",
        r.warnings
    );
}

// ---------------------------------------------------------------- page frame

#[test]
fn page_frame_is_the_svg_page_in_mm() {
    let r = import_text(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="100mm" viewBox="10 10 100 50"><rect x="10" y="10" width="20" height="10"/></svg>"#,
    );
    let f = r.page_frame_mm;
    assert!(f.x0.abs() < 1e-4 && f.y0.abs() < 1e-4, "{f:?}");
    assert!(
        (f.width() - 200.0).abs() < 1e-3 && (f.height() - 100.0).abs() < 1e-3,
        "{f:?}"
    );
    // The viewBox is already applied: the rect sits at the page's top-left.
    assert_bounds(&r.paths, (0.0, 0.0, 40.0, 20.0));
}

#[test]
fn page_frame_of_a_pixel_svg() {
    let r = import_text(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="48"><rect x="0" y="0" width="10" height="10"/></svg>"#,
    );
    let f = r.page_frame_mm;
    assert!(
        (f.width() - 25.4).abs() < 1e-4 && (f.height() - 12.7).abs() < 1e-4,
        "{f:?}"
    );
}
