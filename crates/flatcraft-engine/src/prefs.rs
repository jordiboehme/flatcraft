//! App preferences, stored as `preferences.toml` in the config dir.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name of the preferences inside the config dir.
pub const FILE_NAME: &str = "preferences.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

/// Old files may still hold `mcp_enabled` and `mcp_port` (MCP is stdio only now); unknown keys are ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub allow_agent_cuts: bool,
    pub default_plotter: String,
    pub default_material: String,
    pub theme: Theme,
    pub recent_projects: Vec<PathBuf>,
    /// Where File Output writes when the plotter profile names no directory.
    /// `None` means `<Documents>/FlatCraft Output`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<PathBuf>,
    /// Reload changed linked files without asking (the GUI checks on project
    /// open and when the window gets focus). Only the `changed` ids of
    /// `document.check_sources` are reloaded, never `missing` or
    /// `unreadable`. Off by default.
    pub auto_reload: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            allow_agent_cuts: false,
            default_plotter: "file-output-gpgl-cameo2".into(),
            default_material: "copy-paper".into(),
            theme: Theme::Dark,
            recent_projects: Vec::new(),
            output_dir: None,
            auto_reload: false,
        }
    }
}

impl Preferences {
    /// Loads the preferences. A missing file gives the defaults; a broken file
    /// gives the defaults and a warning on stderr.
    pub fn load(config_dir: &Path) -> Self {
        Self::read(config_dir).unwrap_or_else(|e| {
            eprintln!("warning: {e}. Using default preferences.");
            Self::default()
        })
    }

    /// Reads the preferences. A missing file gives the defaults; a file that
    /// cannot be read or parsed is an error that names it.
    pub fn read(config_dir: &Path) -> Result<Self, String> {
        let path = config_dir.join(FILE_NAME);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        toml::from_str(&text).map_err(|e| format!("{} is broken: {e}", path.display()))
    }

    /// Modification time of the preferences file, `None` when it is missing.
    pub fn modified(config_dir: &Path) -> Option<std::time::SystemTime> {
        std::fs::metadata(config_dir.join(FILE_NAME))
            .and_then(|m| m.modified())
            .ok()
    }

    /// Writes the preferences crash-safely (temp file, sync, rename).
    pub fn save(&self, config_dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(config_dir)?;
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
        flatcraft_device::fsio::write_atomic(&config_dir.join(FILE_NAME), text.as_bytes())
    }
}

/// The config dir: `FLATCRAFT_CONFIG_DIR` if set, else the platform config dir
/// for `io.github.jordiboehme.flatcraft`. Only `main` calls this; library code
/// gets the dir passed in.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FLATCRAFT_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    directories::ProjectDirs::from("io.github", "jordiboehme", "FlatCraft")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| {
            let fallback = PathBuf::from(".flatcraft");
            eprintln!(
                "warning: no home directory found, so the config dir is {} (relative to the working directory). Set FLATCRAFT_CONFIG_DIR to choose one.",
                fallback.display()
            );
            fallback
        })
}
