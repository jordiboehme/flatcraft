//! SVG import: paths in millimetres, y down (same as the mat).

use std::sync::Arc;

use flatcraft_core::geometry::{Cap, CutPath, FillRule, Join, Paint, StrokeInfo};
use flatcraft_core::units::px_to_mm;
use kurbo::{BezPath, Point, Rect};
use usvg::tiny_skia_path::{PathSegment, Transform};

use crate::{ImportError, ImportWarning};

/// Largest accepted input, compressed or decompressed.
pub(crate) const MAX_INPUT_BYTES: usize = 50 * 1024 * 1024;

/// Result of an SVG import.
#[derive(Debug, Clone)]
pub struct SvgImport {
    pub paths: Vec<CutPath>,
    /// `ClipOrMaskIgnored` counts groups with a clip path or mask, not shapes.
    pub warnings: Vec<ImportWarning>,
    /// The SVG page (its width and height) in the same mm coordinates as
    /// `paths`. The viewBox is already applied, so it starts at (0, 0).
    pub page_frame_mm: Rect,
}

/// Returns the plain SVG bytes. Gzip input is decompressed here with a hard
/// cap, because usvg would read it unbounded. Every gzip member is read, as
/// `gunzip` does.
fn plain_bytes(bytes: &[u8], cap: usize) -> Result<std::borrow::Cow<'_, [u8]>, ImportError> {
    use std::io::Read;
    if bytes.len() > cap {
        return Err(ImportError::TooLarge);
    }
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(bytes.into());
    }
    let mut out = Vec::new();
    flate2::read::MultiGzDecoder::new(bytes)
        .take(cap as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| ImportError::Parse(format!("invalid gzip data: {e}")))?;
    if out.len() > cap {
        return Err(ImportError::UnpacksTooLarge);
    }
    Ok(out.into())
}

#[derive(Default)]
struct Walk {
    paths: Vec<CutPath>,
    clips: usize,
    images: usize,
    dashes: usize,
}

/// Imports an SVG. Every visible path (fill and stroke alike) becomes a
/// `CutPath` in millimetres, in document order, with its fill rule and
/// stroke in `paint`. Paths with neither fill nor stroke are skipped. Text is used as outlines, so
/// `fonts` must hold the fonts the document needs.
pub fn import_svg(
    bytes: &[u8],
    fonts: Arc<usvg::fontdb::Database>,
) -> Result<SvgImport, ImportError> {
    import_svg_capped(bytes, fonts, MAX_INPUT_BYTES)
}

pub(crate) fn import_svg_capped(
    bytes: &[u8],
    fonts: Arc<usvg::fontdb::Database>,
    cap: usize,
) -> Result<SvgImport, ImportError> {
    let bytes = plain_bytes(bytes, cap)?;
    let mut opt = usvg::Options {
        fontdb: fonts,
        ..usvg::Options::default()
    };
    // Never read files named by `<image href>`.
    opt.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let tree =
        usvg::Tree::from_data(&bytes, &opt).map_err(|e| ImportError::Parse(e.to_string()))?;

    let mut walk = Walk::default();
    walk_group(tree.root(), None, &mut walk);

    let mut warnings = Vec::new();
    if walk.clips > 0 {
        warnings.push(ImportWarning::ClipOrMaskIgnored { count: walk.clips });
    }
    if walk.images > 0 {
        warnings.push(ImportWarning::ImagesSkipped { count: walk.images });
    }
    if walk.dashes > 0 {
        warnings.push(ImportWarning::DashesIgnored { count: walk.dashes });
    }
    if walk.paths.is_empty() {
        warnings.push(ImportWarning::NoPaths);
    }
    let size = tree.size();
    let page_frame_mm = Rect::new(
        0.0,
        0.0,
        px_to_mm(f64::from(size.width())),
        px_to_mm(f64::from(size.height())),
    );
    Ok(SvgImport {
        paths: walk.paths,
        warnings,
        page_frame_mm,
    })
}

/// Recursion depth is bounded by usvg's `max_depth`.
fn walk_group(group: &usvg::Group, label: Option<&str>, walk: &mut Walk) {
    if group.clip_path().is_some() || group.mask().is_some() {
        walk.clips += 1;
    }
    let label = if group.id().is_empty() {
        label
    } else {
        Some(group.id())
    };
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => walk_group(g, label, walk),
            usvg::Node::Path(p) => {
                if !p.is_visible() {
                    continue;
                }
                let color = match (p.fill().map(|f| f.paint()), p.stroke().map(|s| s.paint())) {
                    (Some(usvg::Paint::Color(c)), _) => Some([c.red, c.green, c.blue]),
                    (None, Some(usvg::Paint::Color(c))) => Some([c.red, c.green, c.blue]),
                    _ => None,
                };
                let paint = paint(p);
                if paint.fill.is_none() && paint.stroke.is_none() {
                    continue;
                }
                if p.stroke().is_some_and(|s| s.dasharray().is_some()) {
                    walk.dashes += 1;
                }
                let path = to_bez(p.data(), p.abs_transform());
                if !path.elements().is_empty() {
                    walk.paths.push(CutPath {
                        path,
                        color,
                        label: label.map(str::to_owned),
                        paint,
                    });
                }
            }
            usvg::Node::Text(t) => walk_group(t.flattened(), label, walk),
            usvg::Node::Image(_) => walk.images += 1,
        }
    }
}

/// Fill rule and stroke of a usvg path. The stroke width is converted to
/// mm in the path's final coordinates (scaled by `sqrt(|det|)` of its
/// absolute transform).
fn paint(p: &usvg::Path) -> Paint {
    let fill = p.fill().map(|f| match f.rule() {
        usvg::FillRule::NonZero => FillRule::NonZero,
        usvg::FillRule::EvenOdd => FillRule::EvenOdd,
    });
    let stroke = p.stroke().map(|s| {
        let ts = p.abs_transform();
        let det = f64::from(ts.sx) * f64::from(ts.sy) - f64::from(ts.kx) * f64::from(ts.ky);
        let width_px = f64::from(s.width().get()) * det.abs().sqrt();
        let limit = f64::from(s.miterlimit().get());
        StrokeInfo {
            width_mm: px_to_mm(width_px),
            join: match s.linejoin() {
                // kurbo has no clipped miter; the plain miter is closest.
                usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => Join::Miter { limit },
                usvg::LineJoin::Round => Join::Round,
                usvg::LineJoin::Bevel => Join::Bevel,
            },
            cap: match s.linecap() {
                usvg::LineCap::Butt => Cap::Butt,
                usvg::LineCap::Round => Cap::Round,
                usvg::LineCap::Square => Cap::Square,
            },
        }
    });
    Paint { fill, stroke }
}

fn to_bez(path: &usvg::tiny_skia_path::Path, ts: Transform) -> BezPath {
    let pt = |p: usvg::tiny_skia_path::Point| {
        let mut p = p;
        ts.map_point(&mut p);
        Point::new(px_to_mm(f64::from(p.x)), px_to_mm(f64::from(p.y)))
    };
    let mut out = BezPath::new();
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => out.move_to(pt(p)),
            PathSegment::LineTo(p) => out.line_to(pt(p)),
            PathSegment::QuadTo(a, b) => out.quad_to(pt(a), pt(b)),
            PathSegment::CubicTo(a, b, c) => out.curve_to(pt(a), pt(b), pt(c)),
            PathSegment::Close => out.close_path(),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn svgz_bomb_is_refused() {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        e.write_all(&vec![b' '; 3 * 1024 * 1024]).unwrap();
        let gz = e.finish().unwrap();
        let start = std::time::Instant::now();
        let r = import_svg_capped(&gz, Arc::new(usvg::fontdb::Database::new()), 1024 * 1024);
        assert!(matches!(r, Err(ImportError::UnpacksTooLarge)), "{r:?}");
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn an_unpacked_svgz_over_the_limit_says_so() {
        assert_eq!(
            ImportError::UnpacksTooLarge.to_string(),
            "The file unpacks to more than 50 MB."
        );
    }
}
