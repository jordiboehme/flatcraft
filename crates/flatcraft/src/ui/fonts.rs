//! IBM Plex, built into the binary. egui's default fonts are off.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily, FontId};

const SANS_REGULAR: &[u8] = include_bytes!("../../../../assets/fonts/IBMPlexSans-Regular.ttf");
const SANS_MEDIUM: &[u8] = include_bytes!("../../../../assets/fonts/IBMPlexSans-Medium.ttf");
const SANS_SEMIBOLD: &[u8] = include_bytes!("../../../../assets/fonts/IBMPlexSans-SemiBold.ttf");
const MONO_REGULAR: &[u8] = include_bytes!("../../../../assets/fonts/IBMPlexMono-Regular.ttf");

/// Family name of IBM Plex Sans Medium.
pub const MEDIUM: &str = "Plex Sans Medium";
/// Family name of IBM Plex Sans SemiBold.
pub const SEMIBOLD: &str = "Plex Sans SemiBold";

/// The font definitions: Plex Sans for proportional text, Plex Mono for
/// monospace, and the Medium and SemiBold weights as named families.
pub fn definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::empty();
    for (name, bytes) in [
        ("plex-sans-regular", SANS_REGULAR),
        ("plex-sans-medium", SANS_MEDIUM),
        ("plex-sans-semibold", SANS_SEMIBOLD),
        ("plex-mono-regular", MONO_REGULAR),
    ] {
        defs.font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    defs.families
        .insert(FontFamily::Proportional, vec!["plex-sans-regular".into()]);
    defs.families.insert(
        FontFamily::Monospace,
        vec!["plex-mono-regular".into(), "plex-sans-regular".into()],
    );
    defs.families.insert(
        FontFamily::Name(MEDIUM.into()),
        vec!["plex-sans-medium".into(), "plex-sans-regular".into()],
    );
    defs.families.insert(
        FontFamily::Name(SEMIBOLD.into()),
        vec!["plex-sans-semibold".into(), "plex-sans-regular".into()],
    );
    defs
}

/// Installs the Plex fonts. Call it before the first frame that shows text.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM.into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}
