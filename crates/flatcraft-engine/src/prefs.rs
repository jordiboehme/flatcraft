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
    /// A folder with a `..` after a folder that does not exist is refused.
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
        Self::load_reporting(config_dir).0
    }

    /// Like [`Preferences::load`], and also returns a one-line warning
    /// about a broken or unreadable file for a window to show
    /// ([`crate::session::Session::note_prefs_warning`]). The full text,
    /// with the parser's details, goes to stderr.
    pub fn load_reporting(config_dir: &Path) -> (Self, Option<String>) {
        match Self::read(config_dir) {
            Ok(prefs) => (prefs, None),
            Err(e) => (
                Self::default(),
                Some(warning(&e, "Using default preferences.")),
            ),
        }
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

    /// Modification time and size of the preferences file, `None` when it
    /// is missing. The size catches a change within one tick of a coarse
    /// file system clock.
    pub fn stamp(config_dir: &Path) -> Option<(Option<std::time::SystemTime>, u64)> {
        let m = std::fs::metadata(config_dir.join(FILE_NAME)).ok()?;
        Some((m.modified().ok(), m.len()))
    }

    /// Writes the preferences crash-safely (temp file, sync, rename). A file
    /// there that does not parse is copied to `preferences.toml.bak-<time>`
    /// first, so nothing the person wrote by hand is lost. Returns the
    /// warning about that copy (also printed on stderr), for a window to
    /// show.
    pub fn save(&self, config_dir: &Path) -> io::Result<Option<String>> {
        std::fs::create_dir_all(config_dir)?;
        let path = config_dir.join(FILE_NAME);
        let mut warning = None;
        if let Ok(text) = std::fs::read_to_string(&path)
            && toml::from_str::<Preferences>(&text).is_err()
        {
            let backup = back_up(config_dir, text.as_bytes())?;
            let w = format!(
                "{} was broken. FlatCraft keeps a copy as {} and writes a new one.",
                path.display(),
                backup.display()
            );
            eprintln!("warning: {w}");
            warning = Some(w);
        }
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
        flatcraft_device::fsio::write_atomic(&path, text.as_bytes())?;
        Ok(warning)
    }

    /// Changes only what `change` touches: re-reads the file, applies
    /// `change`, writes it and returns the fresh copy, so a change another
    /// FlatCraft process stored meanwhile is kept. `current` (the caller's
    /// copy) is the base when the file is missing or cannot be read or
    /// parsed; a broken file is backed up by [`Preferences::save`]. The
    /// returned copy holds the change even when writing failed. The result
    /// carries the warning of [`Preferences::save`], if any.
    pub fn update(
        config_dir: &Path,
        current: &Preferences,
        change: impl FnOnce(&mut Preferences),
    ) -> (Preferences, io::Result<Option<String>>) {
        let stored = config_dir
            .join(FILE_NAME)
            .try_exists()
            .unwrap_or(false)
            .then(|| Self::read(config_dir).ok())
            .flatten();
        let mut fresh = stored.unwrap_or_else(|| current.clone());
        change(&mut fresh);
        let written = fresh.save(config_dir);
        (fresh, written)
    }
}

/// Prints `error` with `then` on stderr in full and returns its first line
/// with `then`, for a toast: a parse error spans several lines.
pub(crate) fn warning(error: &str, then: &str) -> String {
    eprintln!("warning: {error}. {then}");
    let first = error.lines().next().unwrap_or_default().trim_end();
    format!("{}. {then}", first.trim_end_matches(['.', ':']))
}

/// Copies a broken preferences file to `preferences.toml.bak-<UTC time>`
/// (with `-2`, `-3` and so on when that name is taken) and returns the
/// copy's path. The name has no `:`, so every file system takes it.
fn back_up(config_dir: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    use std::io::Write;
    let stamp = crate::commands::sources::compact_utc(std::time::SystemTime::now())
        .unwrap_or_else(|| "unknown".into());
    for n in 1..=100 {
        let name = if n == 1 {
            format!("{FILE_NAME}.bak-{stamp}")
        } else {
            format!("{FILE_NAME}.bak-{stamp}-{n}")
        };
        let path = config_dir.join(name);
        match std::fs::File::options()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(bytes)?;
                f.sync_all()?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other(
        "no free name for a copy of the broken preferences",
    ))
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
