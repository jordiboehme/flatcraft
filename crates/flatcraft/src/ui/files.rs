//! File dialogs behind a trait, so tests can answer them.

use std::path::PathBuf;

/// What a file dialog asks for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileRequest {
    pub title: String,
    /// Filter name and extensions without the dot.
    pub filters: Vec<(String, Vec<String>)>,
    pub file_name: Option<String>,
    pub directory: Option<PathBuf>,
}

/// Opens file dialogs. The app uses [`NativePicker`]; tests inject answers.
pub trait FilePicker {
    fn pick_file(&mut self, req: &FileRequest) -> Option<PathBuf>;
    fn save_file(&mut self, req: &FileRequest) -> Option<PathBuf>;
}

/// The platform dialogs (rfd).
#[derive(Debug, Default)]
pub struct NativePicker;

impl NativePicker {
    fn dialog(req: &FileRequest) -> rfd::FileDialog {
        let mut d = rfd::FileDialog::new().set_title(&req.title);
        for (name, exts) in &req.filters {
            d = d.add_filter(name, exts);
        }
        if let Some(name) = &req.file_name {
            d = d.set_file_name(name);
        }
        if let Some(dir) = &req.directory {
            d = d.set_directory(dir);
        }
        d
    }
}

impl FilePicker for NativePicker {
    fn pick_file(&mut self, req: &FileRequest) -> Option<PathBuf> {
        Self::dialog(req).pick_file()
    }

    fn save_file(&mut self, req: &FileRequest) -> Option<PathBuf> {
        Self::dialog(req).save_file()
    }
}

/// A dialog filter from a name and extensions.
pub fn filter(name: &str, extensions: &[&str]) -> (String, Vec<String>) {
    (
        name.to_string(),
        extensions.iter().map(|e| e.to_string()).collect(),
    )
}
