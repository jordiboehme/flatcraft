//! Raster decoding, DPI detection and tracing. All images are built in memory.

use std::io::Cursor;
use std::time::{Duration, Instant};

use flatcraft_core::document::TraceSettings;
use flatcraft_core::geometry::bounds;
use flatcraft_import::dpi::read_dpi;
use flatcraft_import::trace::trace;
use flatcraft_import::{ImportError, raster};
use image::codecs::jpeg::{JpegEncoder, PixelDensity, PixelDensityUnit};
use image::{GrayImage, ImageFormat, Luma};
use kurbo::PathEl;

const BLADE: f64 = 0.25;

fn white(w: u32, h: u32) -> GrayImage {
    GrayImage::from_pixel(w, h, Luma([255]))
}

fn fill(img: &mut GrayImage, x0: u32, y0: u32, w: u32, h: u32, v: u8) {
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            img.put_pixel(x, y, Luma([v]));
        }
    }
}

fn png(img: &GrayImage) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

fn crc(kind: &[u8], data: &[u8]) -> u32 {
    let mut c = flate2::Crc::new();
    c.update(kind);
    c.update(data);
    c.sum()
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = (data.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(kind);
    v.extend_from_slice(data);
    v.extend_from_slice(&crc(kind, data).to_be_bytes());
    v
}

/// Inserts a `pHYs` chunk right after IHDR.
fn with_phys(png: &[u8], px_per_m: u32, unit: u8) -> Vec<u8> {
    let mut data = px_per_m.to_be_bytes().to_vec();
    data.extend_from_slice(&px_per_m.to_be_bytes());
    data.push(unit);
    let split = 8 + 25; // signature + IHDR chunk
    let mut out = png[..split].to_vec();
    out.extend(chunk(b"pHYs", &data));
    out.extend_from_slice(&png[split..]);
    out
}

fn square_img() -> GrayImage {
    let mut img = white(300, 300);
    fill(&mut img, 50, 50, 200, 200, 0);
    img
}

fn width_of(t: &flatcraft_import::trace::TraceResult) -> f64 {
    bounds(&t.paths).unwrap().width()
}

fn closed(p: &flatcraft_core::geometry::CutPath) -> bool {
    p.path.elements().last() == Some(&PathEl::ClosePath)
}

#[test]
fn png_phys_dpi_is_read() {
    let bytes = with_phys(&png(&white(10, 10)), 11811, 1);
    let dpi = read_dpi(&bytes).unwrap();
    assert!((dpi - 300.0).abs() < 0.1, "{dpi}");
    // Unit 0 means unknown.
    assert_eq!(read_dpi(&with_phys(&png(&white(10, 10)), 11811, 0)), None);
    assert_eq!(read_dpi(&png(&white(10, 10))), None);
}

#[test]
fn jfif_density_is_read() {
    let img = white(16, 16);
    for (density, want) in [
        (PixelDensity::dpi(300), 300.0),
        (
            PixelDensity {
                density: (118, 118),
                unit: PixelDensityUnit::Centimeters,
            },
            118.0 * 2.54,
        ),
    ] {
        let mut out = Vec::new();
        let mut enc = JpegEncoder::new(&mut out);
        enc.set_pixel_density(density);
        enc.encode_image(&img).unwrap();
        let dpi = read_dpi(&out).unwrap();
        assert!((dpi - want).abs() < 0.1, "{dpi} vs {want}");
    }
    let mut plain = Cursor::new(Vec::new());
    img.write_to(&mut plain, ImageFormat::Jpeg).unwrap();
    // The default encoder writes aspect-ratio-only density (unit 0).
    assert_eq!(read_dpi(plain.get_ref()), None);
    assert_eq!(read_dpi(b"GIF89a"), None);
    assert_eq!(read_dpi(&[]), None);
}

#[test]
fn no_dpi_defaults_to_300() {
    let mut img = white(300, 300);
    fill(&mut img, 0, 100, 300, 100, 0);
    let t = trace(&png(&img), &TraceSettings::default(), BLADE).unwrap();
    assert!((t.size_mm.0 - 25.4).abs() < 1e-9, "{:?}", t.size_mm);
    assert!((width_of(&t) - 25.4).abs() < 0.1);
}

#[test]
fn dpi_comes_from_file_and_override_wins() {
    let bytes = with_phys(&png(&square_img()), 5906, 1); // about 150 dpi
    let t = trace(&bytes, &TraceSettings::default(), BLADE).unwrap();
    assert!((width_of(&t) - 33.87).abs() < 0.2, "{}", width_of(&t));
    let s = TraceSettings {
        dpi_override: Some(600.0),
        ..TraceSettings::default()
    };
    let t = trace(&bytes, &s, BLADE).unwrap();
    assert!((width_of(&t) - 8.47).abs() < 0.1, "{}", width_of(&t));
}

#[test]
fn square_traces_to_one_closed_path() {
    let t = trace(&png(&square_img()), &TraceSettings::default(), BLADE).unwrap();
    assert_eq!(t.paths.len(), 1);
    assert!(closed(&t.paths[0]));
    let b = bounds(&t.paths).unwrap();
    assert!((b.width() - 16.93).abs() < 0.1, "{b:?}");
    assert!((b.height() - 16.93).abs() < 0.1, "{b:?}");
    // Image pixel (50, 50) is at 50 * 25.4 / 300 mm, y down.
    assert!(
        (b.x0 - 4.23).abs() < 0.1 && (b.y0 - 4.23).abs() < 0.1,
        "{b:?}"
    );
    assert_eq!(t.dropped_small, 0);
    assert!(t.threshold_used > 0 && t.threshold_used < 255);
}

#[test]
fn explicit_threshold_is_reported() {
    let s = TraceSettings {
        threshold: Some(100),
        ..TraceSettings::default()
    };
    let t = trace(&png(&square_img()), &s, BLADE).unwrap();
    assert_eq!(t.threshold_used, 100);
    assert_eq!(t.paths.len(), 1);
    // A grey figure at 150 is background below threshold 100.
    let mut img = white(300, 300);
    fill(&mut img, 50, 50, 200, 200, 150);
    assert!(trace(&png(&img), &s, BLADE).unwrap().paths.is_empty());
}

#[test]
fn ring_traces_outer_and_hole() {
    let mut img = square_img();
    fill(&mut img, 100, 100, 100, 100, 255);
    let t = trace(&png(&img), &TraceSettings::default(), BLADE).unwrap();
    assert_eq!(t.paths.len(), 2);
    assert!(t.paths.iter().all(closed));
    let mut sizes: Vec<f64> = t
        .paths
        .iter()
        .map(|p| bounds(std::slice::from_ref(p)).unwrap().width())
        .collect();
    sizes.sort_by(f64::total_cmp);
    assert!(
        (sizes[0] - 8.47).abs() < 0.1 && (sizes[1] - 16.93).abs() < 0.1,
        "{sizes:?}"
    );
}

#[test]
fn invert_swaps_figure_and_ground() {
    let s = TraceSettings {
        invert: true,
        ..TraceSettings::default()
    };
    let t = trace(&png(&square_img()), &s, BLADE).unwrap();
    // The white frame around the square: outer border and the square-shaped hole.
    assert_eq!(t.paths.len(), 2);
    let b = bounds(&t.paths).unwrap();
    assert!((b.width() - 25.4).abs() < 0.1, "{b:?}");
}

fn dots(size: u32) -> Vec<u8> {
    let mut img = white(300, 300);
    for i in 0..3 {
        fill(&mut img, 40 + i * 80, 100, size, size, 0);
    }
    png(&img)
}

#[test]
fn small_dots_are_dropped() {
    let s = TraceSettings {
        despeckle_mm2: 0.0,
        ..TraceSettings::default()
    };
    let t = trace(&dots(12), &s, BLADE).unwrap();
    assert!(t.dropped_small >= 1, "{}", t.dropped_small);
    assert!(t.paths.is_empty());
    // An explicit smaller feature size keeps them.
    let s = TraceSettings {
        min_feature_mm: Some(0.5),
        ..s
    };
    let t = trace(&dots(12), &s, BLADE).unwrap();
    assert_eq!(t.paths.len(), 3);
    assert_eq!(t.dropped_small, 0);
}

#[test]
fn despeckle_removes_specks() {
    let s = TraceSettings {
        min_feature_mm: Some(0.0),
        ..TraceSettings::default()
    };
    let t = trace(&dots(3), &s, BLADE).unwrap();
    assert!(t.paths.is_empty());
    // Area, not side length: a 3 x 300 px line is long but only 0.5 mm wide.
    let mut img = white(400, 400);
    fill(&mut img, 50, 50, 300, 3, 0);
    let t = trace(&png(&img), &s, BLADE).unwrap();
    assert_eq!(t.paths.len(), 1);
}

#[test]
fn decode_supports_png_jpeg_gif_and_rejects_garbage() {
    let img = square_img();
    let g = raster::decode(&png(&img)).unwrap();
    assert_eq!(g.dimensions(), (300, 300));
    let mut jpg = Cursor::new(Vec::new());
    img.write_to(&mut jpg, ImageFormat::Jpeg).unwrap();
    assert_eq!(
        raster::decode(jpg.get_ref()).unwrap().dimensions(),
        (300, 300)
    );
    let mut gif = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(img)
        .to_rgba8()
        .write_to(&mut gif, ImageFormat::Gif)
        .unwrap();
    assert_eq!(
        raster::decode(gif.get_ref()).unwrap().dimensions(),
        (300, 300)
    );
    assert!(matches!(
        raster::decode(b"not an image at all"),
        Err(ImportError::Parse(_))
    ));
}

#[test]
fn transparent_pixels_count_as_white() {
    let mut rgba = image::RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 0, 0]));
    for y in 20..80 {
        for x in 20..80 {
            rgba.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
        }
    }
    let mut out = Cursor::new(Vec::new());
    rgba.write_to(&mut out, ImageFormat::Png).unwrap();
    let t = trace(out.get_ref(), &TraceSettings::default(), BLADE).unwrap();
    assert_eq!(t.paths.len(), 1);
}

/// A PNG header that claims 30000 x 30000 pixels; no pixel data follows.
fn bomb() -> Vec<u8> {
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&30000u32.to_be_bytes());
    ihdr.extend_from_slice(&30000u32.to_be_bytes());
    ihdr.extend_from_slice(&[1, 0, 0, 0, 0]); // 1 bit, grey
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    out.extend(chunk(b"IHDR", &ihdr));
    out.extend(chunk(b"IEND", &[]));
    out
}

#[test]
fn decompression_bomb_is_refused() {
    let start = Instant::now();
    let bytes = bomb();
    assert!(matches!(raster::decode(&bytes), Err(ImportError::TooLarge)));
    assert!(matches!(
        trace(&bytes, &TraceSettings::default(), BLADE),
        Err(ImportError::TooLarge)
    ));
    assert!(start.elapsed() < Duration::from_secs(2));
}
