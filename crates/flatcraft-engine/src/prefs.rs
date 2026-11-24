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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub mcp_enabled: bool,
    pub mcp_port: u16,
    pub allow_agent_cuts: bool,
    pub default_plotter: String,
    pub default_material: String,
    pub theme: Theme,
    pub recent_projects: Vec<PathBuf>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            mcp_enabled: true,
            mcp_port: 27891,
            allow_agent_cuts: false,
            default_plotter: "file-output-gpgl-cameo2".into(),
            default_material: "copy-paper".into(),
            theme: Theme::Dark,
            recent_projects: Vec::new(),
        }
    }
}

impl Preferences {
    /// Loads the preferences. A missing file gives the defaults; a broken file
    /// gives the defaults and a warning on stderr.
    pub fn load(config_dir: &Path) -> Self {
        let path = config_dir.join(FILE_NAME);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                eprintln!(
                    "warning: cannot read {}: {e}. Using default preferences.",
                    path.display()
                );
                return Self::default();
            }
        };
        toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!(
                "warning: {} is broken: {e}. Using default preferences.",
                path.display()
            );
            Self::default()
        })
    }

    /// Writes the preferences atomically (temp file, then rename).
    pub fn save(&self, config_dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(config_dir)?;
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
        let path = config_dir.join(FILE_NAME);
        let tmp = config_dir.join(format!("{FILE_NAME}.tmp"));
        let result = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path));
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
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
        .unwrap_or_else(|| PathBuf::from(".flatcraft"))
}
