//! Decoding raster images with hard limits.

use std::io::Cursor;

use image::{GrayImage, ImageError, ImageReader, Limits};

use crate::ImportError;
use crate::svg::MAX_INPUT_BYTES;

/// Largest accepted width or height in pixels.
pub const MAX_DIMENSION: u32 = 16384;
/// Largest memory the decoder may allocate.
pub const MAX_ALLOC_BYTES: u64 = 512 * 1024 * 1024;

fn map_error(e: ImageError) -> ImportError {
    match e {
        ImageError::Limits(_) => ImportError::TooLarge,
        other => ImportError::Parse(other.to_string()),
    }
}

/// Decodes PNG, JPEG, WebP or GIF (first frame) to grey. Transparent pixels
/// count as white paper. The format is guessed from the content. Images over
/// 16384 x 16384 px, or needing more than 512 MB, give `ImportError::TooLarge`
/// before any pixel data is allocated.
pub fn decode(bytes: &[u8]) -> Result<GrayImage, ImportError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(ImportError::TooLarge);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_ALLOC_BYTES);

    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ImportError::Parse(e.to_string()))?;
    reader.limits(limits);
    let image = reader.decode().map_err(map_error)?;

    let rgba = image.into_rgba8();
    let (w, h) = rgba.dimensions();
    let mut grey = GrayImage::new(w, h);
    for (src, dst) in rgba.pixels().zip(grey.pixels_mut()) {
        let [r, g, b, a] = src.0.map(u32::from);
        let luma = (299 * r + 587 * g + 114 * b) / 1000;
        // Composite over white.
        dst.0[0] = ((luma * a + 255 * (255 - a)) / 255) as u8;
    }
    Ok(grey)
}
