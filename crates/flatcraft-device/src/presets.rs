//! System presets embedded into the binary.

pub const PLOTTERS: &[(&str, &str)] = &[
    (
        "silhouette-cameo-2",
        include_str!("../presets/silhouette-cameo-2.toml"),
    ),
    (
        "file-output-gpgl-cameo2",
        include_str!("../presets/file-output-gpgl-cameo2.toml"),
    ),
];

pub const MATERIALS: &[(&str, &str)] = &[
    ("copy-paper", include_str!("../presets/copy-paper.toml")),
    ("cardstock", include_str!("../presets/cardstock.toml")),
    ("vinyl", include_str!("../presets/vinyl.toml")),
    (
        "heat-transfer-vinyl",
        include_str!("../presets/heat-transfer-vinyl.toml"),
    ),
];

/// True for the id of a built-in plotter or material preset. User presets
/// can never take such an id.
pub fn is_system(id: &str) -> bool {
    PLOTTERS.iter().chain(MATERIALS).any(|(p, _)| *p == id)
}
