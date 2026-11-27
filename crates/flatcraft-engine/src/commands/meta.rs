//! What a window needs beyond a command's schema to show one of its
//! parameters as a field: a short plain label and a unit, per command and
//! parameter. The schema stays the one source of the type, the default
//! and the range (`#[schemars(range(..))]` on the params struct), and the
//! doc comments stay the technical description agents read. The window's
//! tool cards read both through [`param_info`] and [`meta`].
//!
//! A parameter is named by its path in the params object: `spacing_mm`,
//! `layout.grid.columns` (a field of an enum variant), `center_mm[0]` (an
//! item of a fixed-size array).

use serde_json::Value;

use crate::registry;

/// The unit of a number parameter, as a field shows it after the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Millimetres.
    Mm,
    /// Degrees, clockwise on screen.
    Deg,
    /// A whole number of things (copies, columns); shown without a unit.
    Count,
    /// Not a number (a switch or a choice).
    None,
}

impl Unit {
    /// What a field shows after the value; empty for a count.
    pub fn symbol(self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::Deg => "deg",
            Unit::Count | Unit::None => "",
        }
    }
}

/// The label and unit of one parameter of one command.
#[derive(Debug, Clone, Copy)]
pub struct ParamMeta {
    pub command: &'static str,
    pub path: &'static str,
    /// A short plain label ("Total angle").
    pub label: &'static str,
    pub unit: Unit,
    /// For an enum: each value with its plain label, in schema order.
    pub choices: &'static [(&'static str, &'static str)],
    /// The number a window field starts with where the schema has no
    /// default. The parameter's doc comment says it too, so agents read
    /// the same number.
    pub start: Option<f64>,
    /// Another command's parameter this one is the same field as, in a
    /// card that runs either command (Move / Copy: `object.move` dx_mm is
    /// `object.duplicate` offset_mm[0]). The card shows that field once
    /// and sends its value as either parameter; its start value is that
    /// parameter's.
    pub same_as: Option<(&'static str, &'static str)>,
}

const fn p(
    command: &'static str,
    path: &'static str,
    label: &'static str,
    unit: Unit,
) -> ParamMeta {
    ParamMeta {
        command,
        path,
        label,
        unit,
        choices: &[],
        start: None,
        same_as: None,
    }
}

const fn from(mut m: ParamMeta, start: f64) -> ParamMeta {
    m.start = Some(start);
    m
}

const fn same(mut m: ParamMeta, command: &'static str, path: &'static str) -> ParamMeta {
    m.same_as = Some((command, path));
    m
}

/// The parameters a window shows for each command, in the order its tool
/// card shows them. A command listed here gets a tool card built from
/// these rows alone.
pub const PARAMS: &[ParamMeta] = &[
    same(
        p("object.move", "dx_mm", "X offset", Unit::Mm),
        "object.duplicate",
        "offset_mm[0]",
    ),
    same(
        p("object.move", "dy_mm", "Y offset", Unit::Mm),
        "object.duplicate",
        "offset_mm[1]",
    ),
    p("object.duplicate", "offset_mm[0]", "X offset", Unit::Mm),
    p("object.duplicate", "offset_mm[1]", "Y offset", Unit::Mm),
    p("object.duplicate", "count", "Number of copies", Unit::Count),
    from(p("object.rotate", "degrees", "Turn by", Unit::Deg), 90.0),
    ParamMeta {
        choices: &[
            ("group", "As one group"),
            ("each", "Each about its own centre"),
        ],
        ..p("object.rotate", "about", "Turn about", Unit::None)
    },
    from(
        p(
            "object.copies",
            "layout.grid.columns",
            "Columns",
            Unit::Count,
        ),
        3.0,
    ),
    from(p("object.copies", "count", "Copies", Unit::Count), 5.0),
    from(p("object.copies", "gap_x_mm", "Spacing X", Unit::Mm), 5.0),
    from(p("object.copies", "gap_y_mm", "Spacing Y", Unit::Mm), 5.0),
    p("object.copies", "linked", "Linked copies", Unit::None),
    from(
        p("object.circular_copies", "count", "Instances", Unit::Count),
        6.0,
    ),
    p(
        "object.circular_copies",
        "total_deg",
        "Total angle",
        Unit::Deg,
    ),
    p(
        "object.circular_copies",
        "center_mm[0]",
        "Centre X",
        Unit::Mm,
    ),
    p(
        "object.circular_copies",
        "center_mm[1]",
        "Centre Y",
        Unit::Mm,
    ),
    p(
        "object.circular_copies",
        "rotate_copies",
        "Rotate copies",
        Unit::None,
    ),
    p(
        "object.circular_copies",
        "linked",
        "Linked copies",
        Unit::None,
    ),
    p("arrange.auto", "spacing_mm", "Spacing", Unit::Mm),
];

/// The label and unit of `path` of `command`.
pub fn meta(command: &str, path: &str) -> Option<&'static ParamMeta> {
    PARAMS
        .iter()
        .find(|m| m.command == command && m.path == path)
}

/// The parameters of `command` a window shows, in order.
pub fn fields_of(command: &str) -> impl Iterator<Item = &'static ParamMeta> + '_ {
    PARAMS.iter().filter(move |m| m.command == command)
}

/// The field `path` of `command` is: itself, or the parameter it is the
/// same field as ([`ParamMeta::same_as`]).
pub fn canonical<'a>(command: &'a str, path: &'a str) -> (&'a str, &'a str) {
    meta(command, path)
        .and_then(|m| m.same_as)
        .unwrap_or((command, path))
}

/// The value a field of `path` of `command` starts with: the schema's
/// default, else the start value of the table (of the parameter it is
/// the same field as, if any). `None` when there is neither: the centre
/// of a circle (the selection's) or Turn about (the window's Each object
/// switch).
pub fn start_value(command: &str, path: &str) -> Option<Value> {
    let (command, path) = canonical(command, path);
    if let Some(d) = param_info(command, path)?.default.filter(|d| !d.is_null()) {
        return Some(d);
    }
    let n = meta(command, path)?.start?;
    Some(if n.fract() == 0.0 && n >= 0.0 {
        Value::from(n as u64)
    } else {
        Value::from(n)
    })
}

/// The params schema of `command` as JSON, made once per command.
pub fn schema_of(command: &str) -> Option<std::sync::Arc<Value>> {
    use std::collections::HashMap;
    use std::sync::{Arc, LazyLock, Mutex};
    static CACHE: LazyLock<Mutex<HashMap<&'static str, Arc<Value>>>> =
        LazyLock::new(Default::default);
    let c = registry::get(command)?;
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = cache.get(c.id()) {
        return Some(v.clone());
    }
    let v = Arc::new(serde_json::to_value(c.params_schema()).ok()?);
    cache.insert(c.id(), v.clone());
    Some(v)
}

/// The first value of `params` outside the range its schema gives, as
/// a message that names the parameter and its range; `None` when every
/// value is in range. The registry checks every command's params with
/// it before the command runs, so the schema is the one source of the
/// ranges for agents and the window alike.
pub fn out_of_range(schema: &Value, params: &Value) -> Option<String> {
    walk(schema, schema, params, "")
}

fn walk(root: &Value, schema: &Value, value: &Value, at: &str) -> Option<String> {
    match value {
        Value::Number(n) => {
            let x = n.as_f64()?;
            let target = concrete(root, schema);
            let min = bound(schema, &target, "minimum");
            let max = bound(schema, &target, "maximum");
            let low = min.is_some_and(|m| x < m);
            let high = max.is_some_and(|m| x > m);
            if !(low || high) {
                return None;
            }
            Some(match (min, max) {
                (Some(lo), Some(hi)) => {
                    format!("{at} must be from {lo} to {hi}; it was {x}")
                }
                (Some(lo), None) => format!("{at} must be {lo} or more; it was {x}"),
                (None, Some(hi)) => format!("{at} must be at most {hi}; it was {x}"),
                (None, None) => unreachable!("a value out of range has a bound"),
            })
        }
        Value::Object(map) => map.iter().find_map(|(k, v)| {
            let sub = find_property(root, schema, k)?;
            let path = if at.is_empty() {
                k.clone()
            } else {
                format!("{at}.{k}")
            };
            walk(root, sub, v, &path)
        }),
        Value::Array(items_v) => {
            let item = items(root, schema)?;
            items_v
                .iter()
                .enumerate()
                .find_map(|(i, v)| walk(root, item, v, &format!("{at}[{i}]")))
        }
        _ => None,
    }
}

/// The type of a parameter, as its field shows it.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamKind {
    Number,
    Integer,
    Bool,
    /// One of these string values.
    Enum(Vec<String>),
}

/// What the schema says about one parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamInfo {
    pub kind: ParamKind,
    /// The bounds (inclusive), when the schema has them.
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// The value it has when it is left out.
    pub default: Option<Value>,
    /// The doc comment (for agents).
    pub description: String,
}

/// What the schema of `command` says about the parameter at `path`.
/// `None` when there is no such command or parameter.
pub fn param_info(command: &str, path: &str) -> Option<ParamInfo> {
    let schema = schema_of(command)?;
    let root: &Value = &schema;
    let mut node = root;
    let mut description = String::new();
    let mut default = None;
    for step in path.split('.') {
        let (name, index) = match step.split_once('[') {
            Some((n, rest)) => (n, Some(rest.trim_end_matches(']').parse::<usize>().ok()?)),
            None => (step, None),
        };
        node = find_property(root, node, name)?;
        // A field's own description and default sit next to its `$ref`.
        description = text(node, "description").unwrap_or_default();
        default = node.get("default").cloned();
        if let Some(i) = index {
            default = default.and_then(|d| d.get(i).cloned());
            node = items(root, node)?;
        }
    }
    let target = concrete(root, node);
    let kind = kind_of(root, &target)?;
    if description.is_empty() {
        description = text(&target, "description").unwrap_or_default();
    }
    Some(ParamInfo {
        kind,
        min: bound(node, &target, "minimum"),
        max: bound(node, &target, "maximum"),
        default,
        description,
    })
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn bound(field: &Value, target: &Value, key: &str) -> Option<f64> {
    field
        .get(key)
        .or_else(|| target.get(key))
        .and_then(Value::as_f64)
}

/// Follows a `$ref` to its definition.
fn deref<'a>(root: &'a Value, v: &'a Value) -> &'a Value {
    match v.get("$ref").and_then(Value::as_str) {
        Some(r) => r
            .strip_prefix("#/")
            .and_then(|p| root.pointer(&format!("/{p}")))
            .unwrap_or(v),
        None => v,
    }
}

/// The alternatives of a schema (`anyOf`, `oneOf`), the schema itself
/// when it has none. A `null` alternative (an `Option`) is left out.
fn alternatives<'a>(root: &'a Value, v: &'a Value) -> Vec<&'a Value> {
    let v = deref(root, v);
    let list = v
        .get("anyOf")
        .or_else(|| v.get("oneOf"))
        .and_then(Value::as_array);
    match list {
        Some(list) => list
            .iter()
            .map(|a| deref(root, a))
            .filter(|a| a.get("type").and_then(Value::as_str) != Some("null"))
            .collect(),
        None => vec![v],
    }
}

/// The property `name` of an object schema, through refs and the variants
/// of an enum.
fn find_property<'a>(root: &'a Value, v: &'a Value, name: &str) -> Option<&'a Value> {
    alternatives(root, v)
        .into_iter()
        .find_map(|a| a.get("properties").and_then(|p| p.get(name)))
}

/// The item schema of an array schema.
fn items<'a>(root: &'a Value, v: &'a Value) -> Option<&'a Value> {
    alternatives(root, v)
        .into_iter()
        .find_map(|a| a.get("items"))
}

/// The schema with a ref followed and a single non-null alternative taken.
fn concrete(root: &Value, v: &Value) -> Value {
    let alts = alternatives(root, v);
    if alts.len() == 1 {
        let mut out = alts[0].clone();
        // An `Option<f64>`: `"type": ["number", "null"]`.
        if let Some(types) = out.get("type").and_then(Value::as_array) {
            let t: Vec<&Value> = types
                .iter()
                .filter(|t| t.as_str() != Some("null"))
                .collect();
            if t.len() == 1 {
                out["type"] = t[0].clone();
            }
        }
        out
    } else {
        deref(root, v).clone()
    }
}

fn kind_of(root: &Value, v: &Value) -> Option<ParamKind> {
    match v.get("type").and_then(Value::as_str) {
        Some("number") => return Some(ParamKind::Number),
        Some("integer") => return Some(ParamKind::Integer),
        Some("boolean") => return Some(ParamKind::Bool),
        _ => {}
    }
    // An enum of strings: `oneOf` of `const`s (or a plain `enum`).
    if let Some(values) = v.get("enum").and_then(Value::as_array) {
        return values
            .iter()
            .map(|x| x.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .map(ParamKind::Enum);
    }
    let alts = v
        .get("oneOf")
        .or_else(|| v.get("anyOf"))
        .and_then(Value::as_array)?;
    alts.iter()
        .map(|a| deref(root, a))
        .filter(|a| a.get("type").and_then(Value::as_str) != Some("null"))
        .map(|a| a.get("const").and_then(Value::as_str).map(str::to_string))
        .collect::<Option<Vec<_>>>()
        .map(ParamKind::Enum)
}
