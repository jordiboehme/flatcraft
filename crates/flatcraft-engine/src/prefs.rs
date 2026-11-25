//! App preferences, stored as `preferences.toml` in the config dir.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name of the preferences inside the config dir.
pub const FILE_NAME: &str = "preferences.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Theme {
    Dark,
    Light,
    /// Follows the operating system's appearance, live. The default of a
    /// new install.
    #[default]
    System,
}

/// What the person chose for a side panel: nothing yet (the panel follows
/// the window width), collapsed to its rail, or open. A choice wins over
/// the width until the person uses the opposite control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelUser {
    #[default]
    None,
    Collapsed,
    Open,
}

/// The theme of a preferences file that has no `theme` key. Only builds
/// that knew just Dark wrote such files, so they keep Dark; a new install
/// has no file and gets [`Theme::System`] from `Preferences::default`.
fn legacy_theme() -> Theme {
    Theme::Dark
}

/// Old files may still hold `mcp_enabled` and `mcp_port` (MCP is stdio only now); unknown keys are ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub allow_agent_cuts: bool,
    pub default_plotter: String,
    pub default_material: String,
    #[serde(default = "legacy_theme")]
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
    /// The window's "Each object" switch: rotate and mirror turn each
    /// selected object about its own centre instead of the selection as one
    /// group. Only the window writes it (a UI preference, ruling R16); no
    /// registry command reads or changes it. Off by default.
    pub rotate_each: bool,
    /// The ribbon actions pinned as buttons, by id (see
    /// [`RIBBON_PIN_IDS`]), for every group on every tab. Only the window
    /// writes it (a UI preference); no registry command reads or changes it.
    /// A file without the key gets [`default_ribbon_pins`], or the migrated
    /// `arrange_pins` of an earlier version (see [`Preferences::read`]).
    pub ribbon_pins: Vec<String>,
    /// The gap the one-click Auto arrange leaves between objects, in mm.
    /// The Auto arrange panel stores it on OK. Only the window writes it.
    pub auto_arrange_spacing_mm: f64,
    /// What the person chose for the Prepare Objects panel, the Prepare
    /// Cut and Job panel and the Cut workspace's stage panel. Only the
    /// window writes them (UI preferences); no registry command reads or
    /// changes them.
    pub panel_objects: PanelUser,
    pub panel_settings: PanelUser,
    pub panel_stage: PanelUser,
}

/// Every id `ribbon_pins` can hold, in the order of the ribbon's table
/// (`flatcraft::ui::tools::TOOLS`). An Align id carries what it lines up
/// with: `align.left.selection` and `align.left.area` are two pins.
pub const RIBBON_PIN_IDS: [&str; 38] = [
    "import",
    "trace",
    "test_patterns",
    "duplicate",
    "copies",
    "delete",
    "reload",
    "move_copy",
    "rotate",
    "rotate.left",
    "rotate.right",
    "mirror.horizontal",
    "mirror.vertical",
    "align.left.selection",
    "align.centre.selection",
    "align.right.selection",
    "align.top.selection",
    "align.middle.selection",
    "align.bottom.selection",
    "align.left.area",
    "align.centre.area",
    "align.right.area",
    "align.top.area",
    "align.middle.area",
    "align.bottom.area",
    "distribute.horizontal",
    "distribute.vertical",
    "pattern.rectangular",
    "pattern.circular",
    "auto_arrange.panel",
    "auto_arrange",
    "zoom.fit",
    "zoom.100",
    "view.grid",
    "view.rulers",
    "panel.objects",
    "panel.settings",
    "panel.debug",
];

/// The pins of the Home and View tabs of a new install: every button
/// those tabs showed before pins existed.
const HOME_VIEW_DEFAULTS: [&str; 14] = [
    "import",
    "trace",
    "test_patterns",
    "duplicate",
    "copies",
    "delete",
    "reload",
    "zoom.fit",
    "zoom.100",
    "view.grid",
    "view.rulers",
    "panel.objects",
    "panel.settings",
    "panel.debug",
];

/// The Arrange pins of a new install: Move / Copy, Rectangular pattern,
/// Rotate -90 and +90, and both Mirror directions.
const ARRANGE_DEFAULTS: [&str; 6] = [
    "move_copy",
    "pattern.rectangular",
    "rotate.left",
    "rotate.right",
    "mirror.horizontal",
    "mirror.vertical",
];

/// The pins of a new install: the buttons the ribbon showed before pins
/// were there for every group, so nothing disappears.
pub fn default_ribbon_pins() -> Vec<String> {
    HOME_VIEW_DEFAULTS
        .iter()
        .chain(&ARRANGE_DEFAULTS)
        .map(|s| s.to_string())
        .collect()
}

/// The ids an `arrange_pins` id of an earlier version (Task 33 and its
/// first whole-command ids) pins now. `align_to_area` is what the old
/// Align directions lined up with; an Align pin keeps that choice in its
/// new id.
fn old_arrange_id(old: &str, align_to_area: bool) -> Vec<String> {
    let to = if align_to_area { "area" } else { "selection" };
    let align = |m: &str| format!("align.{m}.{to}");
    let all_align = || {
        ["left", "centre", "right", "top", "middle", "bottom"]
            .iter()
            .map(|m| align(m))
            .collect::<Vec<_>>()
    };
    let one = |s: &str| vec![s.to_string()];
    match old {
        "move_copy" => one("move_copy"),
        "rotate" => one("rotate"),
        "rotate_left" => one("rotate.left"),
        "rotate_right" => one("rotate.right"),
        "mirror_horizontal" => one("mirror.horizontal"),
        "mirror_vertical" => one("mirror.vertical"),
        "mirror" => vec!["mirror.horizontal".into(), "mirror.vertical".into()],
        "align_left" => vec![align("left")],
        "align_hcenter" => vec![align("centre")],
        "align_right" => vec![align("right")],
        "align_top" => vec![align("top")],
        "align_vcenter" => vec![align("middle")],
        "align_bottom" => vec![align("bottom")],
        "align" => all_align(),
        "distribute_horizontal" => one("distribute.horizontal"),
        "distribute_vertical" => one("distribute.vertical"),
        "distribute" => vec!["distribute.horizontal".into(), "distribute.vertical".into()],
        "rectangular_pattern" => one("pattern.rectangular"),
        "circular_pattern" => one("pattern.circular"),
        // The old Auto arrange pin ran in one click, as `auto_arrange` does.
        "auto_arrange" => one("auto_arrange"),
        _ => Vec::new(),
    }
}

/// The `arrange_pins` of an earlier version as ribbon pins of the Arrange
/// tab. Unknown ids and repeats are dropped and the order is kept. A list
/// that only held unknown ids gets the Arrange defaults, so an old file
/// never empties the tab by accident; an empty list stays empty (the
/// person unpinned everything). `align_to_area` is the Align choice the
/// old pins ran with.
pub fn migrate_arrange_pins(old: &[String], align_to_area: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in old.iter().flat_map(|o| old_arrange_id(o, align_to_area)) {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    if out.is_empty() && !old.is_empty() {
        return ARRANGE_DEFAULTS.map(String::from).to_vec();
    }
    out
}

/// Ribbon pins as stored: unknown ids and repeats are dropped, the order
/// is kept. A list of only unknown ids gets [`default_ribbon_pins`]; an
/// empty list stays empty.
pub fn clean_ribbon_pins(pins: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in pins {
        if RIBBON_PIN_IDS.contains(&p.as_str()) && !out.contains(p) {
            out.push(p.clone());
        }
    }
    if out.is_empty() && !pins.is_empty() {
        return default_ribbon_pins();
    }
    out
}

/// The pins of a file written before `ribbon_pins` existed: the Home and
/// View defaults (those tabs had no pins) plus its migrated
/// `arrange_pins`. The old builds never stored what Align lined up with:
/// every session started with "To the selection", so that is the choice
/// the old Align pins keep.
pub fn ribbon_pins_from_arrange(old: &[String]) -> Vec<String> {
    let mut out: Vec<String> = HOME_VIEW_DEFAULTS.map(String::from).to_vec();
    out.extend(migrate_arrange_pins(old, false));
    out
}

/// The spacing of Auto arrange until the person picks another one, as the
/// engine's `arrange.auto` default.
pub const DEFAULT_AUTO_SPACING_MM: f64 = 5.0;

impl Default for Preferences {
    fn default() -> Self {
        Self {
            allow_agent_cuts: false,
            default_plotter: "file-output-gpgl-cameo2".into(),
            default_material: "copy-paper".into(),
            theme: Theme::System,
            recent_projects: Vec::new(),
            output_dir: None,
            auto_reload: false,
            rotate_each: false,
            ribbon_pins: default_ribbon_pins(),
            auto_arrange_spacing_mm: DEFAULT_AUTO_SPACING_MM,
            panel_objects: PanelUser::None,
            panel_settings: PanelUser::None,
            panel_stage: PanelUser::None,
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
        let broken = |e: toml::de::Error| format!("{} is broken: {e}", path.display());
        let table: toml::Table = toml::from_str(&text).map_err(broken)?;
        let mut prefs: Self = Self::deserialize(table.clone()).map_err(broken)?;
        let strings = |key: &str| -> Option<Vec<String>> {
            table.get(key)?.as_array().map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        };
        prefs.ribbon_pins = match (strings("ribbon_pins"), strings("arrange_pins")) {
            (Some(pins), _) => clean_ribbon_pins(&pins),
            // A file of an earlier version: its Arrange pins move over.
            (None, Some(old)) => ribbon_pins_from_arrange(&old),
            (None, None) => default_ribbon_pins(),
        };
        if !(prefs.auto_arrange_spacing_mm.is_finite() && prefs.auto_arrange_spacing_mm >= 0.0) {
            prefs.auto_arrange_spacing_mm = DEFAULT_AUTO_SPACING_MM;
        }
        Ok(prefs)
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
