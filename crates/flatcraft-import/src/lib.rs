//! FlatCraft import crate.

pub mod fonts;
pub mod svg;

/// Errors from importing a file.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("could not parse the file: {0}")]
    Parse(String),
    #[error("the file is larger than 50 MB")]
    TooLarge,
}

/// Something the importer could not honour. The import still succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportWarning {
    ClipOrMaskIgnored { count: usize },
    ImagesSkipped { count: usize },
    NoPaths,
}
