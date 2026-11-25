//! File dialogs behind a trait, so tests can answer them.

use std::path::{Path, PathBuf};

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

/// The name of the link that shows a file in the file manager.
pub fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Show in Finder"
    } else {
        "Show in folder"
    }
}

/// Shows a file in the platform file manager. The app uses
/// [`NativeRevealer`]; tests record the paths instead of starting
/// processes.
pub trait Revealer {
    fn reveal(&mut self, file: &Path) -> Result<(), String>;
}

/// `open -R` on macOS, `explorer /select,` on Windows, `xdg-open` on the
/// folder elsewhere.
#[derive(Debug, Default)]
pub struct NativeRevealer;

impl Revealer for NativeRevealer {
    fn reveal(&mut self, file: &Path) -> Result<(), String> {
        let mut cmd = if cfg!(target_os = "macos") {
            let mut c = std::process::Command::new("open");
            c.arg("-R").arg(file);
            c
        } else if cfg!(target_os = "windows") {
            // Two arguments: one "/select,<path>" argument would be quoted
            // as a whole when the path has spaces ("FlatCraft Output"),
            // which Explorer does not understand.
            let mut c = std::process::Command::new("explorer");
            c.arg("/select,").arg(file);
            c
        } else {
            let dir = file.parent().unwrap_or(file);
            let mut c = std::process::Command::new("xdg-open");
            c.arg(dir);
            c
        };
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        // Reaped off the UI thread, so no zombie process stays behind.
        std::thread::spawn(move || child.wait());
        Ok(())
    }
}
