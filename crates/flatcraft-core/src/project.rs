//! Project file: pretty JSON, written atomically.

use std::path::Path;

use thiserror::Error;

use crate::document::Document;

pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("Could not read or write the project file: {0}")]
    Io(#[from] std::io::Error),
    #[error("The project file is not valid: {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "This project was saved by a newer FlatCraft (format {found}, this version reads up to {supported}). Please update FlatCraft."
    )]
    NewerFormat { found: u32, supported: u32 },
}

/// Writes the project crash-safely ([`crate::fsio::write_atomic`]: a new
/// temp file in the same folder, synced, then renamed over `path`).
pub fn save(doc: &Document, path: &Path) -> Result<(), ProjectError> {
    save_at(doc, &crate::fsio::resolve_links(path)?)
}

/// Like [`save`], for a `target` the caller has already resolved and
/// checked; links at it are not followed again
/// ([`crate::fsio::write_atomic_at`]).
pub fn save_at(doc: &Document, target: &Path) -> Result<(), ProjectError> {
    let json = serde_json::to_string_pretty(doc)?;
    crate::fsio::write_atomic_at(target, json.as_bytes())?;
    Ok(())
}

/// Loads a project. Objects come back with empty geometry; the engine
/// rebuilds it from the sources.
pub fn load(path: &Path) -> Result<Document, ProjectError> {
    let text = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&text)?;
    let found = value
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if found > u64::from(FORMAT_VERSION) {
        return Err(ProjectError::NewerFormat {
            found: u32::try_from(found).unwrap_or(u32::MAX),
            supported: FORMAT_VERSION,
        });
    }
    let mut doc: Document = serde_json::from_value(value)?;
    let min_next = doc.objects.iter().map(|o| o.id.0 + 1).max().unwrap_or(1);
    doc.next_id = doc.next_id.max(min_next);
    doc.format_version = FORMAT_VERSION;
    Ok(doc)
}
