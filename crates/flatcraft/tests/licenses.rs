//! The bundled third-party licence list (`assets/licenses/THIRD-PARTY.txt`,
//! shown by About > Licenses) matches `cargo metadata`: every crate the app
//! links on any platform, with its version and licence.
//!
//! After a dependency change, rewrite the file with
//! `FLATCRAFT_BLESS_LICENSES=1 cargo test -p flatcraft --test licenses`.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

const HEADER: &str = "\
FlatCraft is free software under the MPL-2.0. It stands on the shoulders of
these works. Thank you, makers.

Fonts
  IBM Plex Sans and IBM Plex Mono - OFL-1.1 (assets/fonts/OFL.txt)

Icons
  Lucide 1.79.0 - ISC (assets/icons/LICENSE)

Rust crates (every platform, from cargo metadata)
";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn metadata() -> Value {
    let cargo = option_env!("CARGO").unwrap_or("cargo");
    let out = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--manifest-path",
        ])
        .arg(root().join("Cargo.toml"))
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("cargo metadata prints JSON")
}

/// The list as it should be: the crates reachable from `flatcraft` through
/// normal dependencies (build and dev dependencies are not shipped),
/// without the workspace's own crates.
fn expected(meta: &Value) -> String {
    let members: BTreeSet<&str> = meta["workspace_members"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let packages: HashMap<&str, &Value> = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["id"].as_str().unwrap(), p))
        .collect();
    let nodes: HashMap<&str, &Value> = meta["resolve"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n["id"].as_str().unwrap(), n))
        .collect();
    let app = packages
        .values()
        .find(|p| p["name"] == "flatcraft")
        .map(|p| p["id"].as_str().unwrap())
        .unwrap();
    let mut seen = BTreeSet::new();
    let mut todo = vec![app];
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        for dep in nodes[id]["deps"].as_array().unwrap() {
            let normal = dep["dep_kinds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|k| k["kind"].is_null());
            if normal {
                todo.push(dep["pkg"].as_str().unwrap());
            }
        }
    }
    let mut rows: Vec<(String, String, String)> = seen
        .into_iter()
        .filter(|id| !members.contains(id))
        .map(|id| {
            let p = packages[id];
            let license = p["license"]
                .as_str()
                .unwrap_or("custom licence file")
                .to_string();
            (
                p["name"].as_str().unwrap().to_string(),
                p["version"].as_str().unwrap().to_string(),
                license,
            )
        })
        .collect();
    rows.sort();
    let mut out = HEADER.to_string();
    for (name, version, license) in rows {
        out.push_str(&format!("  {name} {version} - {license}\n"));
    }
    out
}

#[test]
fn bundled_licence_list_matches_cargo_metadata() {
    let want = expected(&metadata());
    let path = root().join("assets/licenses/THIRD-PARTY.txt");
    if std::env::var_os("FLATCRAFT_BLESS_LICENSES").is_some() {
        std::fs::write(&path, &want).unwrap();
    }
    let have = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        have == want,
        "assets/licenses/THIRD-PARTY.txt is out of date. Run \
         FLATCRAFT_BLESS_LICENSES=1 cargo test -p flatcraft --test licenses"
    );
    assert_eq!(
        flatcraft::ui::about::THIRD_PARTY.replace("\r\n", "\n"),
        want
    );
    assert!(!want.contains('\u{2014}'));
}
