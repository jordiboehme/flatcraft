//! Preset store: system presets plus user presets with inheritance.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

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
];
/// Keys of older versions that no longer exist; dropped when a user preset is loaded.
const LEGACY_PLOTTER_KEYS: &[&str] = &["margins"];
const MATERIAL_REPLACE_KEYS: &[&str] = &[];
const META_KEYS: &[&str] = &["id", "name", "inherits"];

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
    #[error("user preset '{id}' would shadow a system preset")]
    ShadowsSystem { id: String },
    #[error("preset file {path} has no id")]
    MissingId { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetInfo {
    pub id: String,
    pub name: String,
    pub system: bool,
    pub inherits: Option<String>,
}

/// Merge `over` onto `base`. Plain sub-tables merge key by key. Keys in
/// `replace_keys` and any array value are replaced whole.
pub fn merge_tables(base: &Table, over: &Table, replace_keys: &[&str]) -> Table {
    let mut out = base.clone();
    for (k, v) in over {
        let merged = match (out.get(k), v) {
            (Some(Value::Table(b)), Value::Table(o)) if !replace_keys.contains(&k.as_str()) => {
                Value::Table(merge_tables(b, o, replace_keys))
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
                let d = diff_tables(p, c, replace_keys);
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
                if let Some(r) = find_removed(&key, p, c, replace_keys) {
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
        m.validate().map_err(|reason| ProfileError::Invalid {
            id: id.to_string(),
            reason,
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
                flatten_keys(&key, sub, replace_keys, out)
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

#[derive(Debug, Clone)]
pub struct ProfileStore {
    user_dir: PathBuf,
    plotters: BTreeMap<String, Entry>,
    materials: BTreeMap<String, Entry>,
}

fn parse_system(src: &str, id: &str) -> Table {
    src.parse::<Table>()
        .unwrap_or_else(|e| panic!("embedded preset '{id}' is broken: {e}"))
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
        };
        for (id, src) in presets::PLOTTERS {
            store.plotters.insert(
                id.to_string(),
                Entry {
                    table: parse_system(src, id),
                    system: true,
                },
            );
        }
        for (id, src) in presets::MATERIALS {
            store.materials.insert(
                id.to_string(),
                Entry {
                    table: parse_system(src, id),
                    system: true,
                },
            );
        }
        for kind in [Kind::Plotter, Kind::Material] {
            store.load_user_dir(kind)?;
        }
        Ok(store)
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
            let text = fs::read_to_string(&path).map_err(|e| io(&path, e))?;
            let table = text.parse::<Table>().map_err(|source| ProfileError::Toml {
                path: path.clone(),
                source,
            })?;
            let mut table = table;
            if kind == Kind::Plotter {
                for k in LEGACY_PLOTTER_KEYS {
                    table.remove(*k);
                }
            }
            let id = id_of(&table)
                .ok_or_else(|| ProfileError::MissingId { path: path.clone() })?
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
            seen.insert(id.clone(), path.clone());
            self.map_mut(kind).insert(
                id,
                Entry {
                    table,
                    system: false,
                },
            );
        }
        Ok(())
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
        Ok(merged)
    }

    fn resolve<T: DeserializeOwned>(&self, kind: Kind, id: &str) -> Result<T, ProfileError> {
        let table = self.resolve_table(kind, id, &mut Vec::new())?;
        table
            .try_into()
            .map_err(|e: toml::de::Error| ProfileError::Invalid {
                id: id.to_string(),
                reason: e.to_string(),
            })
    }

    pub fn plotter(&self, id: &str) -> Result<PlotterProfile, ProfileError> {
        let p: PlotterProfile = self.resolve(Kind::Plotter, id)?;
        validate_media(id, &p)?;
        Ok(p)
    }

    pub fn material(&self, id: &str) -> Result<MaterialProfile, ProfileError> {
        self.resolve(Kind::Material, id)
    }

    /// Sorted dotted keys the user preset sets itself. Empty for system presets
    /// and unknown ids. Plotters are searched first, then materials, so an id
    /// used by both kinds reports the plotter.
    pub fn modified_fields(&self, id: &str) -> Vec<String> {
        for kind in [Kind::Plotter, Kind::Material] {
            if let Some(e) = self.map(kind).get(id) {
                let mut out = Vec::new();
                if !e.system {
                    flatten_keys("", &e.table, kind.replace_keys(), &mut out);
                    out.sort();
                }
                return out;
            }
        }
        Vec::new()
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
        let text = toml::to_string_pretty(&table)?;
        let dir = self.user_dir.join(kind.dir());
        let io = |path: &Path, source| ProfileError::Io {
            path: path.to_path_buf(),
            source,
        };
        fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
        let path = dir.join(format!("{id}.toml"));
        // Temp file, then rename: a crash mid-write never leaves a broken
        // preset, which would stop the next start.
        let tmp = dir.join(format!("{id}.toml.tmp"));
        let written = fs::write(&tmp, text).and_then(|()| fs::rename(&tmp, &path));
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(io(&path, e));
        }
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
        self.save(Kind::Plotter, profile, &profile.id, &profile.name, parent)
    }

    pub fn save_user_material(
        &mut self,
        profile: &MaterialProfile,
        parent: &str,
    ) -> Result<(), ProfileError> {
        self.save(Kind::Material, profile, &profile.id, &profile.name, parent)
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
