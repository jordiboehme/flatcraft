//! FlatCraft import crate.

pub mod dpi;
pub mod fonts;
pub mod raster;
pub mod svg;
pub mod trace;

/// Errors from importing a file.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ImportError {
    #[error("{}", parse_text(.0))]
    Parse(String),
    #[error("The file is larger than 50 MB.")]
    TooLarge,
    /// A compressed file (.svgz) that is small itself but unpacks to more.
    #[error("The file unpacks to more than 50 MB.")]
    UnpacksTooLarge,
    #[error("{}", image_too_large_text(*width, *height))]
    ImageTooLarge { width: u32, height: u32 },
}

/// Something the importer could not honour. The import still succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportWarning {
    ClipOrMaskIgnored {
        count: usize,
    },
    ImagesSkipped {
        count: usize,
    },
    NoPaths,
    /// Dashed strokes; the dashes are cut as a solid line.
    DashesIgnored {
        count: usize,
    },
    /// Text whose font is not installed: it is not drawn, so not cut.
    TextSkipped,
}

/// Every error text is a sentence, so it can be shown as it is.
fn parse_text(detail: &str) -> String {
    format!(
        "The file could not be parsed: {}.",
        detail.trim_end().trim_end_matches('.')
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every import error is a sentence that starts with a capital, so
    /// the engine and the GUI can show any of them as they are.
    #[test]
    fn error_texts_are_sentences() {
        let texts = [
            ImportError::Parse("bad tag".into()).to_string(),
            ImportError::TooLarge.to_string(),
            ImportError::UnpacksTooLarge.to_string(),
            ImportError::ImageTooLarge {
                width: 0,
                height: 0,
            }
            .to_string(),
            ImportError::ImageTooLarge {
                width: 20000,
                height: 10,
            }
            .to_string(),
        ];
        assert_eq!(texts[0], "The file could not be parsed: bad tag.");
        assert_eq!(texts[1], "The file is larger than 50 MB.");
        assert_eq!(texts[2], "The file unpacks to more than 50 MB.");
        for t in &texts {
            assert!(t.starts_with(char::is_uppercase) && t.ends_with('.'), "{t}");
        }
    }
}
