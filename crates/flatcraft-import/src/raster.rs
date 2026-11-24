//! Decoding raster images with hard limits.

use std::io::Cursor;

use image::{DynamicImage, GrayImage, ImageDecoder, ImageError, ImageReader, Limits};

use crate::ImportError;
use crate::svg::MAX_INPUT_BYTES;

/// Largest accepted width or height in pixels.
pub const MAX_DIMENSION: u32 = 16384;
/// Largest memory the decoder may allocate.
pub const MAX_ALLOC_BYTES: u64 = 512 * 1024 * 1024;

/// Decodes PNG, JPEG, WebP or GIF (first frame) to grey. Transparent pixels
/// count as white paper. The format is guessed from the content. Images over
/// 16384 px per side, or needing more than 512 MB, give
/// `ImportError::ImageTooLarge`; the size is checked from the header before
/// any pixel data is allocated.
pub fn decode(bytes: &[u8]) -> Result<GrayImage, ImportError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(ImportError::TooLarge);
    }
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_ALLOC_BYTES);

    // Read the header unlimited so a refusal can name the size; the limits are
    // applied before any pixel is decoded.
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ImportError::Parse(e.to_string()))?;
    reader.limits(Limits::no_limits());
    let mut decoder = reader.into_decoder().map_err(|e| map_error(e, 0, 0))?;
    let (w, h) = decoder.dimensions();
    if w > MAX_DIMENSION || h > MAX_DIMENSION {
        return Err(ImportError::ImageTooLarge {
            width: w,
            height: h,
        });
    }
    decoder.set_limits(limits).map_err(|e| map_error(e, w, h))?;
    let image = DynamicImage::from_decoder(decoder).map_err(|e| map_error(e, w, h))?;
    Ok(to_grey(image))
}

fn map_error(e: ImageError, width: u32, height: u32) -> ImportError {
    match e {
        ImageError::Limits(_) => ImportError::ImageTooLarge { width, height },
        other => ImportError::Parse(other.to_string()),
    }
}

/// Grey without an RGBA round trip. Only an image with an alpha channel is
/// composited over white.
fn to_grey(image: DynamicImage) -> GrayImage {
    if !image.has_alpha() {
        return image.into_luma8();
    }
    let la = image.to_luma_alpha8();
    drop(image);
    let (w, h) = la.dimensions();
    let mut grey = GrayImage::new(w, h);
    for (src, dst) in la.pixels().zip(grey.pixels_mut()) {
        let [l, a] = src.0.map(u32::from);
        dst.0[0] = ((l * a + 255 * (255 - a)) / 255) as u8;
    }
    grey
}
