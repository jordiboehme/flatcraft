//! The built-in material presets: they parse, stay inside the plotter's
//! ranges, say the right things about backing sheets and mirroring, carry
//! no brand names, and are all written up in docs/materials.md.

use std::fs;
use std::path::PathBuf;

use flatcraft_device::*;
use tempfile::tempdir;

/// The exact HTV sentence the brief asks for.
const MIRROR_TIP: &str = "Mirror the design and cut with the shiny carrier side down";

/// The presets without a backing sheet: small loose parts fall out.
const NO_BACKING: &[&str] = &[
    "copy-paper",
    "cardstock",
    "cardstock-heavy",
    "kraft-paper",
    "vellum",
    "magnet-sheet",
];

const IRON_ON: &[&str] = &[
    "heat-transfer-vinyl",
    "heat-transfer-vinyl-glitter",
    "heat-transfer-vinyl-flock",
];

/// Common brand and product words of cutting machines and their
/// materials. None of them may appear in a material preset or in
/// docs/materials.md (outside the source URLs).
const BRANDS: &[&str] = &[
    "silhouette",
    "cameo",
    "portrait",
    "curio",
    "cricut",
    "brother",
    "scanncut",
    "siser",
    "easyweed",
    "stahls",
    "oracal",
    "orafol",
    "avery",
    "onlinelabels",
    "mylar",
    "thermoflex",
    "glitterflex",
    "gm crafts",
    "expressions",
    "printable press",
];

fn store() -> (tempfile::TempDir, ProfileStore) {
    let d = tempdir().unwrap();
    let s = ProfileStore::load(d.path()).unwrap();
    (d, s)
}

fn ids() -> Vec<&'static str> {
    presets::MATERIALS.iter().map(|(id, _)| *id).collect()
}

fn docs() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/materials.md");
    fs::read_to_string(path).unwrap()
}

/// `text` in lower case with every URL removed.
fn without_urls(text: &str) -> String {
    text.split_whitespace()
        .filter(|w| !w.contains("http://") && !w.contains("https://"))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn has_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

#[test]
fn the_list_is_the_one_jordi_asked_for() {
    assert_eq!(
        ids(),
        [
            "vinyl",
            "vinyl-removable",
            "heat-transfer-vinyl",
            "heat-transfer-vinyl-glitter",
            "heat-transfer-vinyl-flock",
            "sticker-paper",
            "copy-paper",
            "cardstock",
            "cardstock-heavy",
            "kraft-paper",
            "vellum",
            "stencil-film",
            "window-cling",
            "magnet-sheet",
        ]
    );
}

#[test]
fn every_preset_parses_and_keeps_its_id() {
    let (_d, s) = store();
    for (id, src) in presets::MATERIALS {
        let m = s.material(id).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(m.id, *id);
        assert!(m.inherits.is_none(), "{id}");
        // Every field is set in the file, nothing comes from a default.
        let table: toml::Table = src.parse().unwrap();
        for key in [
            "name",
            "group",
            "speed",
            "force",
            "passes",
            "backing",
            "mat_tack",
            "blade_depth_hint",
            "tips",
        ] {
            assert!(table.contains_key(key), "{id} sets no {key}");
        }
        assert!(!m.name.contains('\u{2014}') && !m.tips.as_deref().unwrap().contains('\u{2014}'));
    }
}

#[test]
fn values_stay_inside_the_plotter_ranges() {
    let (_d, s) = store();
    let p = s.plotter("silhouette-cameo-2").unwrap();
    for id in ids() {
        let m = s.material(id).unwrap();
        assert!(
            (p.speed_range[0]..=p.speed_range[1]).contains(&m.speed),
            "{id} speed {}",
            m.speed
        );
        assert!(
            (p.force_range[0]..=p.force_range[1]).contains(&m.force),
            "{id} force {}",
            m.force
        );
        assert!(
            (p.passes_range[0]..=p.passes_range[1]).contains(&m.passes),
            "{id} passes {}",
            m.passes
        );
        // The ratchet blade goes from 1 to 10.
        let hint = m.blade_depth_hint.as_deref().unwrap();
        let depth: u32 = hint.strip_prefix("blade ").unwrap().parse().unwrap();
        assert!((1..=10).contains(&depth), "{id}: {hint}");
        assert!(m.validate_values().is_ok(), "{id}");
        assert!(!m.track_enhancing, "{id}");
    }
}

#[test]
fn only_paper_and_card_have_no_backing_sheet() {
    let (_d, s) = store();
    for id in ids() {
        let m = s.material(id).unwrap();
        assert_eq!(m.backing, !NO_BACKING.contains(&id), "{id}");
    }
}

#[test]
fn iron_on_presets_say_mirror_and_carrier_side_down() {
    let (_d, s) = store();
    for id in ids() {
        let m = s.material(id).unwrap();
        let tips = m.tips.as_deref().unwrap();
        if IRON_ON.contains(&id) {
            assert_eq!(m.group, MaterialGroup::IronOn, "{id}");
            assert!(tips.starts_with(MIRROR_TIP), "{id}: {tips}");
        } else {
            assert_ne!(m.group, MaterialGroup::IronOn, "{id}");
            assert!(!tips.to_lowercase().contains("mirror"), "{id}: {tips}");
        }
    }
}

#[test]
fn every_group_has_presets_and_the_mat_tack_follows_the_material() {
    let (_d, s) = store();
    let all: Vec<MaterialProfile> = ids().iter().map(|id| s.material(id).unwrap()).collect();
    for g in MaterialGroup::ALL {
        assert!(all.iter().any(|m| m.group == g), "{g:?}");
    }
    let tack = |id: &str| all.iter().find(|m| m.id == id).unwrap().mat_tack;
    assert_eq!(tack("copy-paper"), MatTack::Light);
    assert_eq!(tack("vellum"), MatTack::Light);
    assert_eq!(tack("vinyl"), MatTack::Standard);
    assert_eq!(tack("cardstock-heavy"), MatTack::Strong);
    assert_eq!(tack("magnet-sheet"), MatTack::Strong);
}

#[test]
fn no_brand_names_in_presets_or_the_materials_doc() {
    let doc = without_urls(&docs());
    for (id, src) in presets::MATERIALS {
        let text = src.to_lowercase();
        for b in BRANDS {
            assert!(!has_word(&text, b), "{id} names '{b}'");
        }
    }
    for b in BRANDS {
        assert!(!has_word(&doc, b), "docs/materials.md names '{b}'");
    }
}

#[test]
fn the_deny_list_catches_a_brand() {
    // Structural: the checker itself finds a brand word and skips URLs.
    assert!(has_word("cut on a cameo mat", "cameo"));
    assert!(!has_word("a cameos", "cameo"));
    assert!(!without_urls("see https://example.com/silhouette-guide").contains("silhouette"));
}

#[test]
fn the_materials_doc_lists_every_preset() {
    let doc = docs();
    assert!(!doc.contains('\u{2014}'));
    for id in ids() {
        assert!(
            doc.contains(&format!("`{id}`")),
            "docs/materials.md misses {id}"
        );
    }
}

#[test]
fn an_older_material_file_gets_the_defaults() {
    let d = tempdir().unwrap();
    let dir = d.path().join("materials");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("felt.toml"),
        "id = \"felt\"\nname = \"Felt\"\nspeed = 3\nforce = 20\npasses = 1\n\
         reverse_alternate_passes = false\novercut_mm = 0.5\ntrack_enhancing = false\n",
    )
    .unwrap();
    let s = ProfileStore::load(d.path()).unwrap();
    let m = s.material("felt").unwrap();
    assert_eq!(m.group, MaterialGroup::Other);
    assert_eq!(m.mat_tack, MatTack::Standard);
    assert!(m.backing);
    assert_eq!(m.tips, None);
}

#[test]
fn a_child_keeps_its_own_mat_tack_and_backing_after_save_and_load() {
    let (d, mut s) = store();
    // Copy paper: light tack, no backing sheet. The child sets both back.
    let mut m = s.material("copy-paper").unwrap();
    m.id = "my-paper".into();
    m.name = "My paper".into();
    m.mat_tack = MatTack::Standard;
    m.backing = true;
    s.save_user_material(&m, "copy-paper").unwrap();
    assert_eq!(
        s.modified_material_fields("my-paper"),
        vec!["backing", "mat_tack"]
    );
    let s2 = ProfileStore::load(d.path()).unwrap();
    let back = s2.material("my-paper").unwrap();
    assert_eq!(back.mat_tack, MatTack::Standard);
    assert!(back.backing);
    // The tips and the group come along from the parent.
    let parent = s2.material("copy-paper").unwrap();
    assert_eq!(back.tips, parent.tips);
    assert_eq!(back.group, MaterialGroup::PaperAndCard);
}

#[test]
fn a_user_preset_with_a_new_built_in_id_is_left_alone_and_explained() {
    let d = tempdir().unwrap();
    let dir = d.path().join("materials");
    fs::create_dir_all(&dir).unwrap();
    let body = "id = \"kraft-paper\"\nname = \"My kraft\"\ninherits = \"cardstock\"\nforce = 22\n";
    let path = dir.join("kraft-paper.toml");
    fs::write(&path, body).unwrap();
    let s = ProfileStore::load(d.path()).unwrap();
    // The built-in one is listed, the file is skipped with a reason that
    // says what to do, and the file stays as it was.
    let listed: Vec<_> = s
        .materials()
        .into_iter()
        .filter(|i| i.id == "kraft-paper")
        .collect();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].system);
    assert_eq!(s.diagnostics().len(), 1);
    let reason = &s.diagnostics()[0].reason;
    assert!(reason.contains("new id"), "{reason}");
    assert_eq!(fs::read_to_string(&path).unwrap(), body);
}

#[test]
fn user_presets_made_from_the_old_presets_still_load() {
    // The four presets of M1 keep their ids, so files made from them stay.
    let d = tempdir().unwrap();
    let dir = d.path().join("materials");
    fs::create_dir_all(&dir).unwrap();
    for parent in ["vinyl", "heat-transfer-vinyl", "copy-paper", "cardstock"] {
        fs::write(
            dir.join(format!("my-{parent}.toml")),
            format!("id = \"my-{parent}\"\nname = \"Mine\"\ninherits = \"{parent}\"\nforce = 9\n"),
        )
        .unwrap();
    }
    let s = ProfileStore::load(d.path()).unwrap();
    assert!(s.diagnostics().is_empty(), "{:?}", s.diagnostics());
    for parent in ["vinyl", "heat-transfer-vinyl", "copy-paper", "cardstock"] {
        assert_eq!(s.material(&format!("my-{parent}")).unwrap().force, 9);
    }
    assert_eq!(
        s.material("my-heat-transfer-vinyl").unwrap().group,
        MaterialGroup::IronOn
    );
}
