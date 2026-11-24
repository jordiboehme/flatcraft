//! SVG import: paths in millimetres, y down (same as the mat).

use std::sync::Arc;

use flatcraft_core::geometry::CutPath;
use flatcraft_core::units::px_to_mm;
use kurbo::{BezPath, Point};
use usvg::tiny_skia_path::{PathSegment, Transform};

use crate::{ImportError, ImportWarning};

const MAX_INPUT_BYTES: usize = 50 * 1024 * 1024;

/// Result of an SVG import.
#[derive(Debug, Clone)]
pub struct SvgImport {
    pub paths: Vec<CutPath>,
    pub warnings: Vec<ImportWarning>,
}

#[derive(Default)]
struct Walk {
    paths: Vec<CutPath>,
    clips: usize,
    images: usize,
}

/// Imports an SVG. Every visible path (fill and stroke alike) becomes a
/// `CutPath` in millimetres, in document order. Text is used as outlines, so
/// `fonts` must hold the fonts the document needs.
pub fn import_svg(
    bytes: &[u8],
    fonts: Arc<usvg::fontdb::Database>,
) -> Result<SvgImport, ImportError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(ImportError::TooLarge);
    }
    let opt = usvg::Options {
        fontdb: fonts,
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(bytes, &opt).map_err(|e| ImportError::Parse(e.to_string()))?;

    let mut walk = Walk::default();
    walk_group(tree.root(), None, &mut walk);

    let mut warnings = Vec::new();
    if walk.clips > 0 {
        warnings.push(ImportWarning::ClipOrMaskIgnored { count: walk.clips });
    }
    if walk.images > 0 {
        warnings.push(ImportWarning::ImagesSkipped { count: walk.images });
    }
    if walk.paths.is_empty() {
        warnings.push(ImportWarning::NoPaths);
    }
    Ok(SvgImport {
        paths: walk.paths,
        warnings,
    })
}

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
                let path = to_bez(p.data(), p.abs_transform());
                if !path.elements().is_empty() {
                    walk.paths.push(CutPath {
                        path,
                        color,
                        label: label.map(str::to_owned),
                    });
                }
            }
            usvg::Node::Text(t) => walk_group(t.flattened(), label, walk),
            usvg::Node::Image(_) => walk.images += 1,
        }
    }
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
