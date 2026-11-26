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

/// The built-in materials, in the order the material picker lists them
/// inside each group. Their values and sources are in `docs/materials.md`.
pub const MATERIALS: &[(&str, &str)] = &[
    ("vinyl", include_str!("../presets/vinyl.toml")),
    (
        "vinyl-removable",
        include_str!("../presets/vinyl-removable.toml"),
    ),
    (
        "heat-transfer-vinyl",
        include_str!("../presets/heat-transfer-vinyl.toml"),
    ),
    (
        "heat-transfer-vinyl-glitter",
        include_str!("../presets/heat-transfer-vinyl-glitter.toml"),
    ),
    (
        "heat-transfer-vinyl-flock",
        include_str!("../presets/heat-transfer-vinyl-flock.toml"),
    ),
    (
        "sticker-paper",
        include_str!("../presets/sticker-paper.toml"),
    ),
    ("copy-paper", include_str!("../presets/copy-paper.toml")),
    ("cardstock", include_str!("../presets/cardstock.toml")),
    (
        "cardstock-heavy",
        include_str!("../presets/cardstock-heavy.toml"),
    ),
    ("kraft-paper", include_str!("../presets/kraft-paper.toml")),
    ("vellum", include_str!("../presets/vellum.toml")),
    ("stencil-film", include_str!("../presets/stencil-film.toml")),
    ("window-cling", include_str!("../presets/window-cling.toml")),
    ("magnet-sheet", include_str!("../presets/magnet-sheet.toml")),
];

/// True for the id of a built-in plotter or material preset. User presets
/// can never take such an id.
pub fn is_system(id: &str) -> bool {
    PLOTTERS.iter().chain(MATERIALS).any(|(p, _)| *p == id)
}
