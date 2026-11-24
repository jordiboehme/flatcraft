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

/// Writes the project to `<path>.tmp` in the same directory, then renames it.
pub fn save(doc: &Document, path: &Path) -> Result<(), ProjectError> {
    let json = serde_json::to_string_pretty(doc)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let result = std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(result?)
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
    Ok(serde_json::from_value(value)?)
}
