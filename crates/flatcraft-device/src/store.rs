//! Preset store: system presets plus user presets with inheritance.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use serde::de::DeserializeOwned;
use toml::{Table, Value};

use crate::presets;
use crate::profile::{MaterialProfile, PlotterProfile};

const PLOTTER_REPLACE_KEYS: &[&str] = &[
    "protocol",
    "transport",
    "knife_compensation",
    "speed_range",
    "force_range",
    "passes_range",
];
/// Keys of older versions that no longer exist; dropped when a user preset is loaded.
const LEGACY_PLOTTER_KEYS: &[&str] = &["margins"];
const MATERIAL_REPLACE_KEYS: &[&str] = &[];
/// Keys a preset sets for itself, never inherited or compared with its
/// parent. Notes belong to one preset: a child starts without them.
const META_KEYS: &[&str] = &["id", "name", "inherits", NOTES_KEY];

const NOTES_KEY: &str = "notes";

/// The file in the user preset folder that holds the person's notes on
/// system presets, so the system presets stay as they ship:
/// `[plotters]` and `[materials]` tables of id = "notes".
pub const PRESET_NOTES_FILE: &str = "preset-notes.toml";

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("cannot read or write {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot parse {path}: {source}")]
    Toml {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("cannot serialise preset: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("unknown preset '{id}'")]
    UnknownPreset { id: String },
    #[error("preset '{id}' has a bad parent: {reason}")]
    Inheritance { id: String, reason: String },
    #[error("preset '{id}' is invalid: {reason}")]
    Invalid { id: String, reason: String },
    #[error(
        "user preset '{id}' would shadow a system preset: a built-in preset uses this id now, so this file is not loaded. It is left as it is: give it a new id and file name to use it again"
    )]
    ShadowsSystem { id: String },
    #[error("preset file {path} has no id")]
    MissingId { path: PathBuf },
}

/// A user preset file that was skipped at load, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetDiagnostic {
    pub path: PathBuf,
    pub reason: String,
}

/// Longest reason a diagnostic keeps, in chars.
const MAX_REASON_CHARS: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetInfo {
    pub id: String,
    pub name: String,
    pub system: bool,
    pub inherits: Option<String>,
}

/// Merge `over` onto `base`. Plain sub-tables merge key by key. Top-level
/// keys in `replace_keys` and any array value are replaced whole; a nested
/// key with such a name merges like any other table.
pub fn merge_tables(base: &Table, over: &Table, replace_keys: &[&str]) -> Table {
    let mut out = base.clone();
    for (k, v) in over {
        let merged = match (out.get(k), v) {
            (Some(Value::Table(b)), Value::Table(o)) if !replace_keys.contains(&k.as_str()) => {
                Value::Table(merge_tables(b, o, &[]))
            }
            _ => v.clone(),
        };
        out.insert(k.clone(), merged);
    }
    out
}

/// Fields of `profile` that differ from `parent`, same nesting rules as merge.
fn diff_tables(parent: &Table, profile: &Table, replace_keys: &[&str]) -> Table {
    let mut out = Table::new();
    for (k, v) in profile {
        if META_KEYS.contains(&k.as_str()) {
            continue;
        }
        match (parent.get(k), v) {
            (Some(Value::Table(p)), Value::Table(c)) if !replace_keys.contains(&k.as_str()) => {
                let d = diff_tables(p, c, &[]);
                if !d.is_empty() {
                    out.insert(k.clone(), Value::Table(d));
                }
            }
            (Some(p), c) if p == c => {}
            _ => {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    out
}

/// First dotted key the parent has that the child lacks (a removal).
fn find_removed(
    prefix: &str,
    parent: &Table,
    child: &Table,
    replace_keys: &[&str],
) -> Option<String> {
    for (k, pv) in parent {
        if prefix.is_empty() && META_KEYS.contains(&k.as_str()) {
            continue;
        }
        let key = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match (child.get(k), pv) {
            (None, _) => return Some(key),
            (Some(Value::Table(c)), Value::Table(p)) if !replace_keys.contains(&k.as_str()) => {
                if let Some(r) = find_removed(&key, p, c, &[]) {
                    return Some(r);
                }
            }
            _ => {}
        }
    }
    None
}

fn media_id(v: &Value) -> Option<&str> {
    v.as_table()?.get("id")?.as_str()
}

/// Merges two `media` lists by mat id: an entry of `own` replaces the parent mat with the same
/// id (in place), a new id is appended, and `hidden = true` removes the parent mat from the list.
fn merge_media(preset: &str, parent: &[Value], own: &[Value]) -> Result<Vec<Value>, ProfileError> {
    let mut seen: Vec<&str> = Vec::new();
    for mat in own.iter().filter_map(media_id) {
        if seen.contains(&mat) {
            return Err(ProfileError::Invalid {
                id: preset.to_string(),
                reason: format!("mat id '{mat}' is used twice"),
            });
        }
        seen.push(mat);
    }
    let mut out: Vec<Value> = parent.to_vec();
    for e in own {
        let Some(mat) = media_id(e) else {
            out.push(e.clone());
            continue;
        };
        let hidden = e.get("hidden").and_then(Value::as_bool).unwrap_or(false);
        let at = out.iter().position(|m| media_id(m) == Some(mat));
        match (at, hidden) {
            (Some(i), true) => {
                out.remove(i);
            }
            (None, true) => {
                return Err(ProfileError::Invalid {
                    id: preset.to_string(),
                    reason: format!(
                        "Mat '{mat}' is hidden, but there is no built-in mat with that id to hide."
                    ),
                });
            }
            (Some(i), false) => out[i] = e.clone(),
            (None, false) => out.push(e.clone()),
        }
    }
    Ok(out)
}

/// The `media` entries a user preset must store: mats that differ from the parent or are new,
/// and `{ id, hidden = true }` for parent mats the profile no longer lists.
fn diff_media(parent: &[Value], own: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = own
        .iter()
        .filter(|m| !parent.contains(m))
        .cloned()
        .collect();
    for p in parent {
        let Some(id) = media_id(p) else { continue };
        if !own.iter().any(|m| media_id(m) == Some(id)) {
            let mut t = Table::new();
            t.insert("id".into(), Value::String(id.to_string()));
            t.insert("hidden".into(), Value::Boolean(true));
            out.push(Value::Table(t));
        }
    }
    out
}

/// Checks every mat of `p` ([`crate::Media::validate`]) and that no mat id is used twice.
pub fn validate_media(id: &str, p: &PlotterProfile) -> Result<(), ProfileError> {
    for (i, m) in p.media.iter().enumerate() {
        if p.media[..i].iter().any(|o| o.id == m.id) {
            return Err(ProfileError::Invalid {
                id: id.to_string(),
                reason: format!("mat id '{}' is used twice", m.id),
            });
        }
        m.validate().map_err(|e| ProfileError::Invalid {
            id: id.to_string(),
            reason: e.to_string(),
        })?;
    }
    Ok(())
}

/// Ids are kebab-case: `^[a-z0-9]+(-[a-z0-9]+)*$`.
fn validate_id(id: &str) -> Result<(), ProfileError> {
    let ok = !id.is_empty()
        && id.split('-').all(|seg| {
            !seg.is_empty()
                && seg
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        });
    if ok {
        Ok(())
    } else {
        Err(ProfileError::Invalid {
            id: id.to_string(),
            reason: "ids use lowercase letters, digits and single hyphens, like 'my-cutter-2'"
                .into(),
        })
    }
}

fn own_keys(t: &Table, kind: Kind, out: &mut Vec<String>) {
    flatten_keys("", t, kind.replace_keys(), out);
}

fn flatten_keys(prefix: &str, t: &Table, replace_keys: &[&str], out: &mut Vec<String>) {
    for (k, v) in t {
        if prefix.is_empty() && META_KEYS.contains(&k.as_str()) {
            continue;
        }
        let key = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            Value::Table(sub) if !replace_keys.contains(&k.as_str()) => {
                flatten_keys(&key, sub, &[], out)
            }
            Value::Array(mats) if prefix.is_empty() && k == "media" => {
                out.extend(
                    mats.iter()
                        .filter_map(media_id)
                        .map(|id| format!("media.{id}")),
                );
            }
            _ => out.push(key),
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    table: Table,
    system: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plotter,
    Material,
}

impl Kind {
    fn replace_keys(self) -> &'static [&'static str] {
        match self {
            Kind::Plotter => PLOTTER_REPLACE_KEYS,
            Kind::Material => MATERIAL_REPLACE_KEYS,
        }
    }
    fn dir(self) -> &'static str {
        match self {
            Kind::Plotter => "plotters",
            Kind::Material => "materials",
        }
    }
}

#[derive(Debug)]
pub struct ProfileStore {
    user_dir: PathBuf,
    plotters: BTreeMap<String, Entry>,
    materials: BTreeMap<String, Entry>,
    /// User preset files that could not be loaded.
    diagnostics: Vec<PresetDiagnostic>,
    /// The person's notes on system presets ([`PRESET_NOTES_FILE`]).
    system_notes: SystemNotes,
    /// Counts saves and deletes.
    revision: u64,
    /// Resolved profiles of this revision, so a GUI can ask every frame.
    cache: Mutex<Resolved>,
}

/// Profiles resolved since the last save or delete. Only good ones: an
/// error is resolved again (it is rare and its type is not `Clone`).
#[derive(Debug, Clone, Default)]
struct Resolved {
    plotters: HashMap<String, PlotterProfile>,
    materials: HashMap<String, MaterialProfile>,
}

impl Clone for ProfileStore {
    fn clone(&self) -> Self {
        Self {
            user_dir: self.user_dir.clone(),
            plotters: self.plotters.clone(),
            materials: self.materials.clone(),
            diagnostics: self.diagnostics.clone(),
            system_notes: self.system_notes.clone(),
            revision: self.revision,
            cache: Mutex::new(self.resolved().clone()),
        }
    }
}

/// The person's notes on system presets, by kind and id.
#[derive(Debug, Clone, Default)]
struct SystemNotes {
    plotters: BTreeMap<String, String>,
    materials: BTreeMap<String, String>,
    /// The file could not be read: it is never written over.
    broken: bool,
}

impl SystemNotes {
    fn map(&self, kind: Kind) -> &BTreeMap<String, String> {
        match kind {
            Kind::Plotter => &self.plotters,
            Kind::Material => &self.materials,
        }
    }

    fn map_mut(&mut self, kind: Kind) -> &mut BTreeMap<String, String> {
        match kind {
            Kind::Plotter => &mut self.plotters,
            Kind::Material => &mut self.materials,
        }
    }

    /// Reads [`PRESET_NOTES_FILE`]; a missing file is no notes.
    fn read(path: &Path) -> Result<Self, String> {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.to_string()),
        };
        let table = text.parse::<Table>().map_err(|e| e.to_string())?;
        let mut out = Self::default();
        for kind in [Kind::Plotter, Kind::Material] {
            let Some(t) = table.get(kind.dir()) else {
                continue;
            };
            let t = t
                .as_table()
                .ok_or_else(|| format!("[{}] must be a table", kind.dir()))?;
            for (id, v) in t {
                let v = v
                    .as_str()
                    .ok_or_else(|| format!("the notes of '{id}' must be text"))?;
                out.map_mut(kind).insert(id.clone(), v.to_string());
            }
        }
        Ok(out)
    }

    fn to_toml(&self) -> Result<String, toml::ser::Error> {
        let mut table = Table::new();
        for kind in [Kind::Plotter, Kind::Material] {
            let t: Table = self
                .map(kind)
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect();
            if !t.is_empty() {
                table.insert(kind.dir().into(), Value::Table(t));
            }
        }
        toml::to_string_pretty(&table)
    }
}

/// Notes as stored: trimmed at the end, `None` when nothing is left.
fn clean_notes(text: &str) -> Option<String> {
    let t = text.trim_end();
    (!t.trim().is_empty()).then(|| t.to_string())
}

/// A preset compiled into the app. A broken one is an error, never a panic.
fn parse_system(src: &str, id: &str) -> Result<Table, ProfileError> {
    src.parse::<Table>().map_err(|e| ProfileError::Invalid {
        id: id.to_string(),
        reason: format!("the built-in preset is broken: {e}"),
    })
}

fn invalid(id: &str, reason: String) -> ProfileError {
    ProfileError::Invalid {
        id: id.to_string(),
        reason,
    }
}

fn id_of(t: &Table) -> Option<&str> {
    t.get("id").and_then(Value::as_str)
}

impl ProfileStore {
    /// Load the system presets and the user presets under `user_dir`
    /// (`plotters/*.toml`, `materials/*.toml`). A missing dir is fine.
    pub fn load(user_dir: &Path) -> Result<Self, ProfileError> {
        let mut store = ProfileStore {
            user_dir: user_dir.to_path_buf(),
            plotters: BTreeMap::new(),
            materials: BTreeMap::new(),
            diagnostics: Vec::new(),
            system_notes: SystemNotes::default(),
            revision: 0,
            cache: Mutex::default(),
        };
        let notes_path = user_dir.join(PRESET_NOTES_FILE);
        match SystemNotes::read(&notes_path) {
            Ok(n) => store.system_notes = n,
            Err(reason) => {
                store.system_notes.broken = true;
                store.diagnostics.push(PresetDiagnostic {
                    path: notes_path,
                    reason,
                });
            }
        }
        for (id, src) in presets::PLOTTERS {
            store.plotters.insert(
                id.to_string(),
                Entry {
                    table: parse_system(src, id)?,
                    system: true,
                },
            );
        }
        for (id, src) in presets::MATERIALS {
            store.materials.insert(
                id.to_string(),
                Entry {
                    table: parse_system(src, id)?,
                    system: true,
                },
            );
        }
        for kind in [Kind::Plotter, Kind::Material] {
            store.load_user_dir(kind)?;
        }
        Ok(store)
    }

    fn resolved(&self) -> std::sync::MutexGuard<'_, Resolved> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads the user presets again (after the person fixed or removed a
    /// file), keeping the system presets. Cached profiles are dropped and
    /// the revision moves on.
    pub fn reload(&mut self) -> Result<(), ProfileError> {
        let revision = self.revision;
        *self = Self::load(&self.user_dir)?;
        self.revision = revision + 1;
        Ok(())
    }

    /// Counts the saves and deletes of this store. Resolved profiles are
    /// cached per revision.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Notes a change of the presets: every cached profile may be stale
    /// (a parent changes its children).
    fn changed(&mut self) {
        self.revision += 1;
        *self.resolved() = Resolved::default();
    }

    fn map(&self, kind: Kind) -> &BTreeMap<String, Entry> {
        match kind {
            Kind::Plotter => &self.plotters,
            Kind::Material => &self.materials,
        }
    }

    fn map_mut(&mut self, kind: Kind) -> &mut BTreeMap<String, Entry> {
        match kind {
            Kind::Plotter => &mut self.plotters,
            Kind::Material => &mut self.materials,
        }
    }

    fn load_user_dir(&mut self, kind: Kind) -> Result<(), ProfileError> {
        let dir = self.user_dir.join(kind.dir());
        if !dir.is_dir() {
            return Ok(());
        }
        let io = |path: &Path, source| ProfileError::Io {
            path: path.to_path_buf(),
            source,
        };
        let mut files: Vec<PathBuf> = Vec::new();
        for e in fs::read_dir(&dir).map_err(|e| io(&dir, e))? {
            let p = e.map_err(|e| io(&dir, e))?.path();
            if p.extension().is_some_and(|x| x == "toml") {
                files.push(p);
            }
        }
        files.sort();
        let mut seen: BTreeMap<String, PathBuf> = BTreeMap::new();
        for path in files {
            match self.load_user_file(kind, &path, &seen) {
                Ok((id, table)) => {
                    seen.insert(id.clone(), path.clone());
                    self.map_mut(kind).insert(
                        id,
                        Entry {
                            table,
                            system: false,
                        },
                    );
                }
                Err(e) => {
                    let mut reason = e.to_string();
                    if let Some((cut, _)) = reason.char_indices().nth(MAX_REASON_CHARS) {
                        reason.truncate(cut);
                        reason.push_str("...");
                    }
                    self.diagnostics.push(PresetDiagnostic { path, reason });
                }
            }
        }
        Ok(())
    }

    /// Reads one user preset file. The first file of an id wins.
    fn load_user_file(
        &self,
        kind: Kind,
        path: &Path,
        seen: &BTreeMap<String, PathBuf>,
    ) -> Result<(String, Table), ProfileError> {
        let text = fs::read_to_string(path).map_err(|source| ProfileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mut table = text.parse::<Table>().map_err(|source| ProfileError::Toml {
            path: path.to_path_buf(),
            source,
        })?;
        if kind == Kind::Plotter {
            for k in LEGACY_PLOTTER_KEYS {
                table.remove(*k);
            }
        }
        let id = id_of(&table)
            .ok_or_else(|| ProfileError::MissingId {
                path: path.to_path_buf(),
            })?
            .to_string();
        validate_id(&id)?;
        if path.file_stem().and_then(|s| s.to_str()) != Some(id.as_str()) {
            return Err(ProfileError::Invalid {
                id,
                reason: format!("the file name of {} must match the id", path.display()),
            });
        }
        if self.map(kind).get(&id).is_some_and(|e| e.system) {
            return Err(ProfileError::ShadowsSystem { id });
        }
        if let Some(first) = seen.get(&id) {
            return Err(ProfileError::Invalid {
                id,
                reason: format!(
                    "defined twice, in {} and {}",
                    first.display(),
                    path.display()
                ),
            });
        }
        Ok((id, table))
    }

    /// User preset files that were skipped at load because they cannot be
    /// read, and why. Such a file is never overwritten: a save with its id
    /// is refused until the file is fixed or removed (and the store loaded
    /// again).
    pub fn diagnostics(&self) -> &[PresetDiagnostic] {
        &self.diagnostics
    }

    /// Refuses a save that would replace a file which is not this loaded
    /// user preset: on a case-insensitive file system `Mine.toml` and
    /// `mine.toml` are one file. The folder is listed, so the check works
    /// the same on every file system.
    fn check_no_clash(&self, kind: Kind, id: &str, dir: &Path) -> Result<(), ProfileError> {
        let want = format!("{id}.toml");
        let ours = self.map(kind).get(id).is_some_and(|e| !e.system);
        let Ok(entries) = fs::read_dir(dir) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let file = entry.file_name();
            let Some(file) = file.to_str() else { continue };
            if file.eq_ignore_ascii_case(&want) && !(ours && file == want) {
                return Err(ProfileError::Invalid {
                    id: id.to_string(),
                    reason: format!(
                        "A file named {file} already exists in your presets folder. Rename or remove it first."
                    ),
                });
            }
        }
        Ok(())
    }

    fn broken_file(&self, kind: Kind, id: &str) -> Option<&PresetDiagnostic> {
        let path = self.user_dir.join(kind.dir()).join(format!("{id}.toml"));
        self.diagnostics.iter().find(|d| d.path == path)
    }

    /// The diagnostic of an unreadable plotter preset file named `<id>.toml`.
    /// Its id counts as taken.
    pub fn broken_plotter_file(&self, id: &str) -> Option<&PresetDiagnostic> {
        self.broken_file(Kind::Plotter, id)
    }

    /// The diagnostic of an unreadable material preset file named
    /// `<id>.toml`. Its id counts as taken.
    pub fn broken_material_file(&self, id: &str) -> Option<&PresetDiagnostic> {
        self.broken_file(Kind::Material, id)
    }

    fn infos(&self, kind: Kind) -> Vec<PresetInfo> {
        let mut v: Vec<PresetInfo> = self
            .map(kind)
            .iter()
            .map(|(id, e)| PresetInfo {
                id: id.clone(),
                name: e
                    .table
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(id)
                    .to_string(),
                system: e.system,
                inherits: e
                    .table
                    .get("inherits")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
            .collect();
        v.sort_by(|a, b| b.system.cmp(&a.system).then_with(|| a.id.cmp(&b.id)));
        v
    }

    pub fn plotters(&self) -> Vec<PresetInfo> {
        self.infos(Kind::Plotter)
    }

    pub fn materials(&self) -> Vec<PresetInfo> {
        self.infos(Kind::Material)
    }

    fn resolve_table(
        &self,
        kind: Kind,
        id: &str,
        stack: &mut Vec<String>,
    ) -> Result<Table, ProfileError> {
        let entry = self
            .map(kind)
            .get(id)
            .ok_or_else(|| ProfileError::UnknownPreset { id: id.to_string() })?;
        let own = &entry.table;
        let Some(parent_id) = own.get("inherits").and_then(Value::as_str) else {
            let mut t = own.clone();
            if let Some(Value::Array(o)) = own.get("media") {
                t.insert("media".into(), Value::Array(merge_media(id, &[], o)?));
            }
            return Ok(t);
        };
        if stack.iter().any(|s| s == id) {
            return Err(ProfileError::Inheritance {
                id: id.to_string(),
                reason: format!("cycle through '{id}'"),
            });
        }
        stack.push(id.to_string());
        if !self.map(kind).contains_key(parent_id) {
            return Err(ProfileError::Inheritance {
                id: id.to_string(),
                reason: format!("unknown parent '{parent_id}'"),
            });
        }
        let parent = self.resolve_table(kind, parent_id, stack)?;
        stack.pop();
        let mut merged = merge_tables(&parent, own, kind.replace_keys());
        if let Some(Value::Array(o)) = own.get("media") {
            let no_mats = Vec::new();
            let p = match parent.get("media") {
                Some(Value::Array(p)) => p,
                _ => &no_mats,
            };
            merged.insert("media".into(), Value::Array(merge_media(id, p, o)?));
        }
        merged.insert("id".into(), Value::String(id.to_string()));
        let name = own
            .get("name")
            .cloned()
            .unwrap_or_else(|| Value::String(id.to_string()));
        merged.insert("name".into(), name);
        merged.insert("inherits".into(), Value::String(parent_id.to_string()));
        match own.get(NOTES_KEY) {
            Some(n) => merged.insert(NOTES_KEY.into(), n.clone()),
            None => merged.remove(NOTES_KEY),
        };
        Ok(merged)
    }

    fn resolve<T: DeserializeOwned>(&self, kind: Kind, id: &str) -> Result<T, ProfileError> {
        #[cfg(test)]
        tests::RESOLVES.with(|n| n.set(n.get() + 1));
        let mut table = self.resolve_table(kind, id, &mut Vec::new())?;
        if self.map(kind).get(id).is_some_and(|e| e.system) {
            match self.system_notes.map(kind).get(id) {
                Some(n) => table.insert(NOTES_KEY.into(), Value::String(n.clone())),
                None => table.remove(NOTES_KEY),
            };
        }
        table
            .try_into()
            .map_err(|e: toml::de::Error| ProfileError::Invalid {
                id: id.to_string(),
                reason: e.to_string(),
            })
    }

    /// The plotter preset `id` with inheritance resolved. Cached until the
    /// next save or delete.
    pub fn plotter(&self, id: &str) -> Result<PlotterProfile, ProfileError> {
        if let Some(p) = self.resolved().plotters.get(id) {
            return Ok(p.clone());
        }
        let p: PlotterProfile = self.resolve(Kind::Plotter, id)?;
        validate_media(id, &p)?;
        p.validate_values().map_err(|reason| invalid(id, reason))?;
        self.resolved().plotters.insert(id.to_string(), p.clone());
        Ok(p)
    }

    /// The plotter preset `id` with inheritance resolved but without the
    /// value checks (mats, tolerances), so the Profiles window can load a
    /// preset with a bad value and let the person fix it. Jobs use
    /// [`ProfileStore::plotter`].
    pub fn plotter_unchecked(&self, id: &str) -> Result<PlotterProfile, ProfileError> {
        self.resolve(Kind::Plotter, id)
    }

    /// The material preset `id` like [`ProfileStore::plotter_unchecked`].
    pub fn material_unchecked(&self, id: &str) -> Result<MaterialProfile, ProfileError> {
        self.resolve(Kind::Material, id)
    }

    /// The material preset `id` with inheritance resolved. Cached until the
    /// next save or delete.
    pub fn material(&self, id: &str) -> Result<MaterialProfile, ProfileError> {
        if let Some(m) = self.resolved().materials.get(id) {
            return Ok(m.clone());
        }
        let m: MaterialProfile = self.resolve(Kind::Material, id)?;
        m.validate_values().map_err(|reason| invalid(id, reason))?;
        self.resolved().materials.insert(id.to_string(), m.clone());
        Ok(m)
    }

    fn modified(&self, kind: Kind, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(e) = self.map(kind).get(id).filter(|e| !e.system) {
            own_keys(&e.table, kind, &mut out);
            out.sort();
        }
        out
    }

    /// Sorted dotted keys the user plotter preset `id` sets itself.
    pub fn modified_plotter_fields(&self, id: &str) -> Vec<String> {
        self.modified(Kind::Plotter, id)
    }

    /// Sorted dotted keys the user material preset `id` sets itself.
    pub fn modified_material_fields(&self, id: &str) -> Vec<String> {
        self.modified(Kind::Material, id)
    }

    fn save<T: Serialize + DeserializeOwned>(
        &mut self,
        kind: Kind,
        profile: &T,
        id: &str,
        name: &str,
        parent: &str,
    ) -> Result<(), ProfileError> {
        validate_id(id)?;
        if self.map(kind).get(id).is_some_and(|e| e.system) {
            return Err(ProfileError::ShadowsSystem { id: id.to_string() });
        }
        if let Some(d) = self.broken_file(kind, id) {
            // Never overwrite a file the user may still want to repair.
            return Err(ProfileError::Invalid {
                id: id.to_string(),
                reason: format!(
                    "A preset file with this name exists but can't be read: {}. Fix or remove it first, then press Reload presets in the Profiles window.",
                    d.reason
                ),
            });
        }
        // Walk the parent chain: it must not lead back to this id.
        let mut cur = Some(parent.to_string());
        let mut hops = 0;
        while let Some(c) = cur {
            if c == id || hops > 1000 {
                return Err(ProfileError::Inheritance {
                    id: id.to_string(),
                    reason: format!("parent '{parent}' would create a cycle"),
                });
            }
            hops += 1;
            cur = self
                .map(kind)
                .get(&c)
                .and_then(|e| e.table.get("inherits"))
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if id == parent {
            return Err(ProfileError::Inheritance {
                id: id.to_string(),
                reason: "a preset cannot inherit from itself".into(),
            });
        }
        let parent_table =
            self.resolve_table(kind, parent, &mut Vec::new())
                .map_err(|e| match e {
                    ProfileError::UnknownPreset { .. } => ProfileError::Inheritance {
                        id: id.to_string(),
                        reason: format!("unknown parent '{parent}'"),
                    },
                    other => other,
                })?;
        // Normalise the parent through the profile type so both sides are
        // compared in serialised form.
        let parent_profile: T =
            parent_table
                .try_into()
                .map_err(|e: toml::de::Error| ProfileError::Invalid {
                    id: parent.to_string(),
                    reason: e.to_string(),
                })?;
        let parent_ser = Table::try_from(&parent_profile)?;
        let own_ser = Table::try_from(profile)?;
        if let Some(key) = find_removed("", &parent_ser, &own_ser, kind.replace_keys()) {
            return Err(ProfileError::Invalid {
                id: id.to_string(),
                reason: format!(
                    "Clearing the inherited value '{key}' is not supported yet. Set a value instead."
                ),
            });
        }
        let mut diff = diff_tables(&parent_ser, &own_ser, kind.replace_keys());
        if let (Some(Value::Array(p)), Some(Value::Array(o))) =
            (parent_ser.get("media"), own_ser.get("media"))
        {
            diff.remove("media");
            let changed = diff_media(p, o);
            if !changed.is_empty() {
                diff.insert("media".into(), Value::Array(changed));
            }
        }
        let mut table = Table::new();
        table.insert("id".into(), Value::String(id.to_string()));
        table.insert("name".into(), Value::String(name.to_string()));
        table.insert("inherits".into(), Value::String(parent.to_string()));
        for (k, v) in diff {
            table.insert(k, v);
        }
        if let Some(n) = own_ser
            .get(NOTES_KEY)
            .and_then(Value::as_str)
            .and_then(clean_notes)
        {
            table.insert(NOTES_KEY.into(), Value::String(n));
        }
        let text = toml::to_string_pretty(&table)?;
        let dir = self.user_dir.join(kind.dir());
        let io = |path: &Path, source| ProfileError::Io {
            path: path.to_path_buf(),
            source,
        };
        fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
        let path = dir.join(format!("{id}.toml"));
        self.check_no_clash(kind, id, &dir)?;
        // A crash mid-write must never leave a broken preset: that would
        // stop the next start.
        crate::fsio::write_atomic(&path, text.as_bytes()).map_err(|e| io(&path, e))?;
        self.changed();
        self.map_mut(kind).insert(
            id.to_string(),
            Entry {
                table,
                system: false,
            },
        );
        Ok(())
    }

    pub fn save_user_plotter(
        &mut self,
        profile: &PlotterProfile,
        parent: &str,
    ) -> Result<(), ProfileError> {
        validate_media(&profile.id, profile)?;
        profile
            .validate_values()
            .map_err(|reason| invalid(&profile.id, reason))?;
        self.save(Kind::Plotter, profile, &profile.id, &profile.name, parent)
    }

    pub fn save_user_material(
        &mut self,
        profile: &MaterialProfile,
        parent: &str,
    ) -> Result<(), ProfileError> {
        profile
            .validate_values()
            .map_err(|reason| invalid(&profile.id, reason))?;
        self.save(Kind::Material, profile, &profile.id, &profile.name, parent)
    }

    /// Sets the notes of preset `id` (empty text removes them). A system
    /// preset keeps them in [`PRESET_NOTES_FILE`], so its own values stay
    /// as they ship; a user preset keeps them in its own file. Refused for
    /// an unknown id, and while the file to write cannot be read.
    fn set_notes(&mut self, kind: Kind, id: &str, text: &str) -> Result<(), ProfileError> {
        let notes = clean_notes(text);
        let entry = self
            .map(kind)
            .get(id)
            .ok_or_else(|| ProfileError::UnknownPreset { id: id.to_string() })?;
        let io = |path: &Path, source| ProfileError::Io {
            path: path.to_path_buf(),
            source,
        };
        if entry.system {
            let path = self.user_dir.join(PRESET_NOTES_FILE);
            if self.system_notes.broken {
                return Err(invalid(
                    id,
                    format!(
                        "{} can't be read, so it is not written over. Fix or remove it first, then press Reload presets in the Profiles window.",
                        path.display()
                    ),
                ));
            }
            let mut next = self.system_notes.clone();
            match &notes {
                Some(n) => next.map_mut(kind).insert(id.to_string(), n.clone()),
                None => next.map_mut(kind).remove(id),
            };
            let text = next.to_toml()?;
            fs::create_dir_all(&self.user_dir).map_err(|e| io(&self.user_dir, e))?;
            crate::fsio::write_atomic(&path, text.as_bytes()).map_err(|e| io(&path, e))?;
            self.system_notes = next;
        } else {
            let mut table = entry.table.clone();
            match &notes {
                Some(n) => table.insert(NOTES_KEY.into(), Value::String(n.clone())),
                None => table.remove(NOTES_KEY),
            };
            let text = toml::to_string_pretty(&table)?;
            let dir = self.user_dir.join(kind.dir());
            fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
            let path = dir.join(format!("{id}.toml"));
            crate::fsio::write_atomic(&path, text.as_bytes()).map_err(|e| io(&path, e))?;
            self.map_mut(kind).insert(
                id.to_string(),
                Entry {
                    table,
                    system: false,
                },
            );
        }
        self.changed();
        Ok(())
    }

    /// Sets the notes of plotter preset `id` (see `set_notes`).
    pub fn set_plotter_notes(&mut self, id: &str, text: &str) -> Result<(), ProfileError> {
        self.set_notes(Kind::Plotter, id, text)
    }

    /// Sets the notes of material preset `id`, system or user: a system
    /// preset keeps them in [`PRESET_NOTES_FILE`], a user preset in its own
    /// file. Empty text removes them.
    pub fn set_material_notes(&mut self, id: &str, text: &str) -> Result<(), ProfileError> {
        self.set_notes(Kind::Material, id, text)
    }

    fn delete(&mut self, kind: Kind, id: &str) -> Result<(), ProfileError> {
        let entry = self
            .map(kind)
            .get(id)
            .ok_or_else(|| ProfileError::UnknownPreset { id: id.to_string() })?;
        if entry.system {
            return Err(ProfileError::Invalid {
                id: id.to_string(),
                reason: "system presets cannot be deleted".into(),
            });
        }
        let children: Vec<&str> = self
            .map(kind)
            .iter()
            .filter(|(_, e)| e.table.get("inherits").and_then(Value::as_str) == Some(id))
            .map(|(c, _)| c.as_str())
            .collect();
        if !children.is_empty() {
            return Err(ProfileError::Invalid {
                id: id.to_string(),
                reason: format!(
                    "other presets inherit from it ({}). Delete those first",
                    children.join(", ")
                ),
            });
        }
        let path = self.user_dir.join(kind.dir()).join(format!("{id}.toml"));
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(ProfileError::Io { path, source }),
        }
        self.map_mut(kind).remove(id);
        self.changed();
        Ok(())
    }

    /// Deletes a user plotter preset and its file. System presets and presets other
    /// presets inherit from are refused.
    pub fn delete_user_plotter(&mut self, id: &str) -> Result<(), ProfileError> {
        self.delete(Kind::Plotter, id)
    }

    /// Deletes a user material preset and its file, with the same rules as
    /// [`ProfileStore::delete_user_plotter`].
    pub fn delete_user_material(&mut self, id: &str) -> Result<(), ProfileError> {
        self.delete(Kind::Material, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    thread_local! {
        /// How often a profile was resolved from TOML on this thread.
        pub(super) static RESOLVES: Cell<usize> = const { Cell::new(0) };
    }

    fn resolves() -> usize {
        RESOLVES.with(Cell::get)
    }

    #[test]
    fn a_broken_embedded_preset_is_an_error() {
        assert!(matches!(
            parse_system("id = [", "broken"),
            Err(ProfileError::Invalid { .. })
        ));
        assert!(parse_system("id = \"fine\"", "fine").is_ok());
    }

    #[test]
    fn resolved_profiles_are_cached_until_a_save_or_delete() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ProfileStore::load(dir.path()).unwrap();
        let id = "file-output-gpgl-cameo2";
        let first = store.plotter(id).unwrap();
        store.material("vinyl").unwrap();
        let before = resolves();
        for _ in 0..3 {
            assert_eq!(store.plotter(id).unwrap(), first);
            store.material("vinyl").unwrap();
        }
        assert_eq!(resolves(), before, "a cached profile needs no TOML work");

        let mut mine = first.clone();
        mine.id = "mine".into();
        mine.name = "Mine".into();
        mine.scale_x = 1.01;
        store.save_user_plotter(&mine, id).unwrap();
        assert_eq!(store.plotter("mine").unwrap().scale_x, 1.01);
        mine.scale_x = 1.02;
        store.save_user_plotter(&mine, id).unwrap();
        assert_eq!(
            store.plotter("mine").unwrap().scale_x,
            1.02,
            "a save drops the cache"
        );
        let clone = store.clone();
        store.delete_user_plotter("mine").unwrap();
        assert!(store.plotter("mine").is_err(), "a delete drops the cache");
        assert_eq!(clone.plotter("mine").unwrap().scale_x, 1.02);
    }
}
