//! FlatCraft import crate.

pub mod dpi;
pub mod fonts;
pub mod raster;
pub mod svg;
pub mod trace;

/// Errors from importing a file.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("could not parse the file: {0}")]
    Parse(String),
    #[error("the file is larger than 50 MB")]
    TooLarge,
    #[error("{}", image_too_large_text(*width, *height))]
    ImageTooLarge { width: u32, height: u32 },
}

/// Something the importer could not honour. The import still succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportWarning {
    ClipOrMaskIgnored { count: usize },
    ImagesSkipped { count: usize },
    NoPaths,
}

fn image_too_large_text(width: u32, height: u32) -> String {
    let max = raster::MAX_DIMENSION;
    if width == 0 && height == 0 {
        format!("The image is too large to import (at most {max} pixels per side).")
    } else {
        format!(
            "The image is too large to import ({width} x {height} pixels, at most {max} per side)."
        )
    }
}
