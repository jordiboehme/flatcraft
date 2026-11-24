//! Tracing a raster image into cut outlines in millimetres.

use flatcraft_core::document::TraceSettings;
use flatcraft_core::geometry::CutPath;
use flatcraft_path::flatten::{flatten, merge_collinear};
use image::GrayImage;
use kurbo::{BezPath, PathEl, Point};
use visioncortex::clusters::Cluster;
use visioncortex::{BinaryImage, CompoundPathElement, PathSimplifyMode, Spline};

use crate::ImportError;
use crate::dpi::read_dpi;
use crate::raster::decode;

/// Resolution assumed when the file declares none.
pub const DEFAULT_DPI: f64 = 300.0;
/// Tolerance for flattening and for dropping collinear points, in mm.
const TOLERANCE_MM: f64 = 0.01;
/// The splice angle of the curve fitter, as in VTracer's default.
const SPLICE_THRESHOLD_DEG: f64 = 45.0;
const MAX_ITERATIONS: usize = 10;

/// What a trace produced.
#[derive(Debug, Clone)]
pub struct TraceResult {
    /// Closed outlines in mm, image top-left at (0, 0), y down. A figure with
    /// holes gives one path for its outline and one per hole.
    pub paths: Vec<CutPath>,
    /// Closed paths dropped because they are thinner than the minimum feature.
    pub dropped_small: usize,
    /// Width and height of the whole image in mm.
    pub size_mm: (f64, f64),
    /// Pixels darker than this were figure (before invert).
    pub threshold_used: u8,
    /// Original size in px when the image was too big for the tracing budget
    /// and was traced at a lower resolution. `size_mm` is still exact.
    pub downscaled_from: Option<(u32, u32)>,
    /// A declared resolution below 10 dpi that was ignored.
    pub ignored_dpi: Option<f64>,
}

/// Images above this many pixels are scaled down before tracing, to keep
/// memory and time in check.
pub const MAX_TRACE_PIXELS: u64 = 16_000_000;
/// Declared resolutions below this are taken as bogus.
const MIN_PLAUSIBLE_DPI: f64 = 10.0;

/// Otsu's threshold: dark pixels are those with a luma below the result. Ties
/// resolve to the middle of the tied range, so two-tone images split halfway.
pub fn otsu(image: &GrayImage) -> u8 {
    let mut hist = [0u64; 256];
    for p in image.pixels() {
        hist[usize::from(p.0[0])] += 1;
    }
    let total: u64 = hist.iter().sum();
    if total == 0 {
        return 128;
    }
    let sum_all: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();
    let (mut w0, mut sum0) = (0.0f64, 0.0f64);
    let mut best = -1.0f64;
    let (mut first, mut last) = (0usize, 0usize);
    for (t, &count) in hist.iter().enumerate().take(255) {
        w0 += count as f64;
        sum0 += t as f64 * count as f64;
        let w1 = total as f64 - w0;
        if w0 == 0.0 || w1 == 0.0 {
            continue;
        }
        let diff = sum0 / w0 - (sum_all - sum0) / w1;
        let var = w0 * w1 * diff * diff;
        if var > best * (1.0 + 1e-12) {
            best = var;
            first = t;
            last = t;
        } else if var >= best * (1.0 - 1e-12) {
            last = t;
        }
    }
    if best < 0.0 {
        return 128; // a single grey level
    }
    // Class 0 is 0..=t, so the first light value is t + 1.
    (((first + last) / 2) + 1).min(255) as u8
}

/// Traces `bytes` (PNG, JPEG, WebP or GIF).
///
/// 1. Grey, then figure = luma below the threshold (Otsu when `None`),
///    swapped by `invert`.
/// 2. visioncortex clusters the figure and fits splines to each outline and
///    hole. Clusters smaller than `despeckle_mm2` are speckles and vanish.
/// 3. px become mm at `dpi_override`, else the file's dpi, else 300.
/// 4. Paths are flattened at 0.01 mm and simplified. Closed paths with a
///    bounding-box side below the minimum feature (`min_feature_mm`, else
///    6 x blade offset) are dropped and counted.
pub fn trace(
    bytes: &[u8],
    s: &TraceSettings,
    blade_offset_mm: f64,
) -> Result<TraceResult, ImportError> {
    let mut ignored_dpi = None;
    let declared = if s.dpi_override.is_none() {
        read_dpi(bytes).filter(|&d| {
            let plausible = d >= MIN_PLAUSIBLE_DPI;
            if !plausible {
                ignored_dpi = Some(d);
            }
            plausible
        })
    } else {
        None
    };
    let dpi = s.dpi_override.or(declared).unwrap_or(DEFAULT_DPI);
    if !(dpi.is_finite() && dpi > 0.0) {
        return Err(ImportError::Parse("dpi must be a positive number".into()));
    }
    let grey = decode(bytes)?;
    let (orig_w, orig_h) = grey.dimensions();
    let (grey, downscaled_from) = fit_budget(grey);
    let (w, h) = grey.dimensions();
    let threshold = s.threshold.unwrap_or_else(|| otsu(&grey));

    let mut figure = BinaryImage::new_w_h(w as usize, h as usize);
    for (x, y, p) in grey.enumerate_pixels() {
        if (p.0[0] < threshold) != s.invert {
            figure.set_pixel(x as usize, y as usize, true);
        }
    }
    drop(grey);

    // After a downscale a traced pixel covers more than one source pixel.
    let mm_x = f64::from(orig_w) * 25.4 / dpi / f64::from(w);
    let mm_y = f64::from(orig_h) * 25.4 / dpi / f64::from(h);
    let min_area_px = if s.despeckle_mm2.is_finite() && s.despeckle_mm2 > 0.0 {
        s.despeckle_mm2 / (mm_x * mm_y)
    } else {
        0.0
    };
    let min_feature = s.min_feature_mm.unwrap_or(6.0 * blade_offset_mm).max(0.0);
    let corner = s.corner_threshold_deg.clamp(0.0, 180.0).to_radians();
    // Smoothing 0..=10 maps to a curve-fitting length threshold of 3.5..=10 px.
    let segment_length = 3.5 + 0.65 * f64::from(s.smoothing.min(10));

    let mut paths = Vec::new();
    let mut dropped_small = 0;
    for cluster in figure.to_clusters(false).iter() {
        if (cluster.size() as f64) < min_area_px {
            continue;
        }
        let found = splines(cluster, corner, segment_length);
        if found.is_empty() {
            // visioncortex drops outlines it cannot fit (a lone pixel).
            dropped_small += 1;
        }
        for spline in found {
            // A shape that collapses to fewer than three points counts as dropped.
            let Some(path) = to_cut_path(&spline, mm_x, mm_y) else {
                dropped_small += 1;
                continue;
            };
            let b = kurbo::Shape::bounding_box(&path.path);
            if b.width().min(b.height()) < min_feature {
                dropped_small += 1;
            } else {
                paths.push(path);
            }
        }
    }
    Ok(TraceResult {
        paths,
        dropped_small,
        size_mm: (
            f64::from(orig_w) * 25.4 / dpi,
            f64::from(orig_h) * 25.4 / dpi,
        ),
        threshold_used: threshold,
        downscaled_from,
        ignored_dpi,
    })
}

/// One spline per outline and hole of the cluster, in image pixels.
fn splines(cluster: &Cluster, corner: f64, segment_length: f64) -> Vec<Spline> {
    let compound = cluster.to_compound_path(
        PathSimplifyMode::Spline,
        corner,
        segment_length,
        MAX_ITERATIONS,
        SPLICE_THRESHOLD_DEG.to_radians(),
    );
    compound
        .paths
        .into_iter()
        .filter_map(|el| match el {
            CompoundPathElement::Spline(s) => Some(s),
            _ => None,
        })
        .collect()
}

/// Scales the image down to `MAX_TRACE_PIXELS`, keeping the aspect ratio.
fn fit_budget(grey: GrayImage) -> (GrayImage, Option<(u32, u32)>) {
    let (w, h) = grey.dimensions();
    let pixels = u64::from(w) * u64::from(h);
    if pixels <= MAX_TRACE_PIXELS {
        return (grey, None);
    }
    let k = (MAX_TRACE_PIXELS as f64 / pixels as f64).sqrt();
    let nw = ((f64::from(w) * k) as u32).max(1);
    let nh = ((f64::from(h) * k) as u32).max(1);
    let small = image::imageops::resize(&grey, nw, nh, image::imageops::FilterType::Triangle);
    (small, Some((w, h)))
}

fn to_cut_path(spline: &Spline, mm_x: f64, mm_y: f64) -> Option<CutPath> {
    if spline.is_empty() {
        return None;
    }
    let pt = |p: &visioncortex::PointF64| Point::new(p.x * mm_x, p.y * mm_y);
    let mut bez = BezPath::new();
    bez.move_to(pt(&spline.points[0]));
    for c in spline.points[1..].as_chunks::<3>().0 {
        bez.curve_to(pt(&c[0]), pt(&c[1]), pt(&c[2]));
    }
    bez.close_path();

    let mut points: Vec<Point> = flatten(&bez, TOLERANCE_MM)
        .elements()
        .iter()
        .filter_map(|el| match el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => Some(*p),
            _ => None,
        })
        .collect();
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    // Close the loop for the simplifier so the start point is not special.
    if let Some(&first) = points.first() {
        points.push(first);
    }
    let mut points = merge_collinear(&points, TOLERANCE_MM);
    points.pop();
    if points.len() < 3 {
        return None;
    }
    let mut path = BezPath::new();
    path.move_to(points[0]);
    for p in &points[1..] {
        path.line_to(*p);
    }
    path.close_path();
    Some(CutPath {
        path,
        color: None,
        label: None,
    })
}
