//! Project file: pretty JSON, written atomically.

use std::path::Path;

use thiserror::Error;

use crate::document::{Document, ObjectId};

pub const FORMAT_VERSION: u32 = 1;

/// The version a project with linked copies is written with: its members
/// keep their shared fields in `link_groups`, which older builds cannot
/// read, so they say "newer FlatCraft" instead of "broken file". Projects
/// without linked copies stay at [`FORMAT_VERSION`].
pub const LINKED_FORMAT_VERSION: u32 = 2;

/// The newest version this build reads.
pub const NEWEST_FORMAT_VERSION: u32 = LINKED_FORMAT_VERSION;

/// Keys of an object that linked copies share; the project file keeps them
/// once per group, in `link_groups`.
const SHARED_KEYS: [&str; 5] = ["source", "cut", "link", "local_bounds_mm", "frame_mm"];

/// Keys of an object's transform that linked copies share.
const SHARED_TRANSFORM_KEYS: [&str; 2] = ["scale_x", "scale_y"];

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
    #[error(
        "The project file is not valid: '{name}' is a linked copy of group {group}, but that group is missing."
    )]
    MissingGroup { name: String, group: u64 },
}

/// Writes the project crash-safely ([`crate::fsio::write_atomic`]: a new
/// temp file in the same folder, synced, then renamed over `path`).
pub fn save(doc: &Document, path: &Path) -> Result<(), ProjectError> {
    save_at(doc, &crate::fsio::resolve_links(path)?)
}

/// Like [`save`], for a `target` the caller has already resolved and
/// checked; links at it are not followed again
/// ([`crate::fsio::write_atomic_at`]).
pub fn save_at(doc: &Document, target: &Path) -> Result<(), ProjectError> {
    let json = serde_json::to_string_pretty(&to_value(doc)?)?;
    crate::fsio::write_atomic_at(target, json.as_bytes())?;
    Ok(())
}

/// The project as JSON. The shared fields of linked copies are written once
/// per group, in `link_groups`, and left out of the members.
pub fn to_value(doc: &Document) -> Result<serde_json::Value, ProjectError> {
    use serde_json::{Map, Value};
    let mut value = serde_json::to_value(doc)?;
    let mut groups: Vec<(u64, Map<String, Value>)> = Vec::new();
    if let Some(objects) = value.get_mut("objects").and_then(Value::as_array_mut) {
        for o in objects.iter_mut().filter_map(Value::as_object_mut) {
            let Some(g) = o.get("link_group").and_then(Value::as_u64) else {
                continue;
            };
            let shared = take_shared(o);
            if !groups.iter().any(|(id, _)| *id == g) {
                groups.push((g, shared));
            }
        }
    }
    if !groups.is_empty() {
        let records = groups
            .into_iter()
            .map(|(id, mut shared)| {
                shared.insert("id".into(), id.into());
                Value::Object(shared)
            })
            .collect();
        value["link_groups"] = Value::Array(records);
        value["format_version"] = LINKED_FORMAT_VERSION.into();
    }
    Ok(value)
}

/// Removes the shared keys from one object's JSON and returns them.
fn take_shared(
    o: &mut serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut shared = serde_json::Map::new();
    for key in SHARED_KEYS {
        if let Some(v) = o.remove(key) {
            shared.insert(key.into(), v);
        }
    }
    if let Some(t) = o
        .get_mut("transform")
        .and_then(serde_json::Value::as_object_mut)
    {
        for key in SHARED_TRANSFORM_KEYS {
            if let Some(v) = t.remove(key) {
                shared.insert(key.into(), v);
            }
        }
    }
    shared
}

/// Puts the shared fields of each linked copy's group back into its JSON.
/// A field the member carries itself is kept, never overwritten: when it
/// differs from the group's, the member leaves the group on load
/// ([`Document::share_link_groups`]).
fn unpack_link_groups(value: &mut serde_json::Value) -> Result<(), ProjectError> {
    use serde_json::Value;
    let Some(groups) = value.get_mut("link_groups").map(Value::take) else {
        return check_no_members(value);
    };
    let groups: Vec<Value> = match groups {
        Value::Array(a) => a,
        _ => Vec::new(),
    };
    let find = |g: u64| {
        groups
            .iter()
            .find(|r| r.get("id").and_then(Value::as_u64) == Some(g))
    };
    let Some(objects) = value.get_mut("objects").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for o in objects.iter_mut().filter_map(Value::as_object_mut) {
        let Some(g) = o.get("link_group").and_then(Value::as_u64) else {
            continue;
        };
        let record = find(g).ok_or_else(|| missing_group(o, g))?;
        for key in SHARED_KEYS {
            if let Some(v) = record.get(key) {
                o.entry(key).or_insert_with(|| v.clone());
            }
        }
        if let Some(t) = o.get_mut("transform").and_then(Value::as_object_mut) {
            for key in SHARED_TRANSFORM_KEYS {
                if let Some(v) = record.get(key) {
                    t.entry(key).or_insert_with(|| v.clone());
                }
            }
        }
    }
    Ok(())
}

/// A file without `link_groups` whose objects name a group: refused, so
/// the member is named instead of a "missing field source".
fn check_no_members(value: &serde_json::Value) -> Result<(), ProjectError> {
    let objects = value.get("objects").and_then(serde_json::Value::as_array);
    for o in objects
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_object)
    {
        if let Some(g) = o.get("link_group").and_then(serde_json::Value::as_u64)
            && !o.contains_key("source")
        {
            return Err(missing_group(o, g));
        }
    }
    Ok(())
}

fn missing_group(o: &serde_json::Map<String, serde_json::Value>, group: u64) -> ProjectError {
    let name = o
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("an object")
        .to_string();
    ProjectError::MissingGroup { name, group }
}

/// Loads a project. Objects come back with empty geometry; the engine
/// rebuilds it from the sources. Linked copies come back sharing one
/// source `Arc`. See [`load_report`] for the warnings.
pub fn load(path: &Path) -> Result<Document, ProjectError> {
    load_report(path).map(|(doc, _)| doc)
}

/// [`load`], plus the ids of the linked copies whose own fields no longer
/// matched their group (a hand-edited file): they were unlinked and kept
/// their own design (see [`unlinked_warning`]).
pub fn load_report(path: &Path) -> Result<(Document, Vec<ObjectId>), ProjectError> {
    load_report_str(&std::fs::read_to_string(path)?)
}

/// [`load_report`] for project text that is not a file on disk (a sample
/// built into the app).
pub fn load_report_str(text: &str) -> Result<(Document, Vec<ObjectId>), ProjectError> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    let found = value
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if found > u64::from(NEWEST_FORMAT_VERSION) {
        return Err(ProjectError::NewerFormat {
            found: u32::try_from(found).unwrap_or(u32::MAX),
            supported: NEWEST_FORMAT_VERSION,
        });
    }
    let mut value = value;
    unpack_link_groups(&mut value)?;
    let mut doc: Document = serde_json::from_value(value)?;
    let unlinked = doc.share_link_groups();
    let min_next = doc.objects.iter().map(|o| o.id.0 + 1).max().unwrap_or(1);
    doc.next_id = doc.next_id.max(min_next);
    doc.format_version = FORMAT_VERSION;
    Ok((doc, unlinked))
}

/// The warning for a linked copy that was unlinked on open.
pub fn unlinked_warning(name: &str) -> String {
    format!("{name}: it no longer matched its linked copies, so it was unlinked")
}
