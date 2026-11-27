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
    fn pick_folder(&mut self, req: &FileRequest) -> Option<PathBuf>;
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

    fn pick_folder(&mut self, req: &FileRequest) -> Option<PathBuf> {
        Self::dialog(req).pick_folder()
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

/// The menu item that shows a file in the file manager: "Reveal in
/// Finder" on macOS, "Show in Explorer" on Windows, "Show in folder"
/// elsewhere (Linux opens the folder that holds it).
pub fn reveal_menu_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else if cfg!(target_os = "windows") {
        "Show in Explorer"
    } else {
        "Show in folder"
    }
}

/// The menu item that shows an object's linked file in the file manager.
pub fn reveal_source_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Reveal source in Finder"
    } else if cfg!(target_os = "windows") {
        "Show source in Explorer"
    } else {
        "Show source in folder"
    }
}

/// Shows a file in the platform file manager. The app uses
/// [`NativeRevealer`]; tests record the paths instead of starting
/// processes.
pub trait Revealer {
    /// Starts showing `file`. An error here (the file is gone, the
    /// command cannot start) is shown at once.
    fn reveal(&mut self, file: &Path) -> Result<(), String>;

    /// Reasons of reveals that failed after they started, since the last
    /// call. The window shows each as an error toast.
    fn failures(&mut self) -> Vec<String> {
        Vec::new()
    }
}

/// `open -R` on macOS, `explorer /select,` on Windows, `xdg-open` on the
/// folder elsewhere. The command runs on its own; when it ends with an
/// error, its message waits in `failed` for [`Revealer::failures`].
#[derive(Debug, Default)]
pub struct NativeRevealer {
    failed: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl Revealer for NativeRevealer {
    fn reveal(&mut self, file: &Path) -> Result<(), String> {
        if !file.exists() {
            return Err("the file was moved or deleted".into());
        }
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
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        let child = cmd.spawn().map_err(|e| e.to_string())?;
        // Waited for off the UI thread, so no zombie process stays behind
        // and a failure is reported.
        let failed = self.failed.clone();
        std::thread::spawn(move || {
            // Explorer ends with 1 even when it worked, so only open
            // and xdg-open report.
            if let Ok(out) = child.wait_with_output()
                && !out.status.success()
                && !cfg!(target_os = "windows")
            {
                let text = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let why = if text.is_empty() {
                    format!("the file manager ended with {}", out.status)
                } else {
                    text
                };
                failed.lock().unwrap_or_else(|e| e.into_inner()).push(why);
            }
        });
        Ok(())
    }

    fn failures(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.failed.lock().unwrap_or_else(|e| e.into_inner()))
    }
}
