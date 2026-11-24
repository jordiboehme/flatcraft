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

#[cfg(test)]
mod tests {
    use super::*;

    fn missing(text: &str) -> Vec<char> {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        ctx.fonts_mut(|f| {
            text.chars()
                .filter(|c| !f.has_glyph(&FontId::proportional(14.0), *c))
                .collect()
        })
    }

    /// Characters egui itself draws as text in the widgets FlatCraft uses:
    /// the ellipsis of elided text, the minus and thin space of drag
    /// values, degrees and the multiplication sign. Arrows, check marks and
    /// menu chevrons are painted shapes, not glyphs.
    #[test]
    fn plex_covers_the_glyphs_egui_draws() {
        assert_eq!(
            missing("\u{2026}\u{2212}\u{2009}\u{b0}\u{d7}"),
            Vec::<char>::new()
        );
    }

    /// Plex has no macOS modifier symbols and no warning sign. egui only
    /// draws them for `Context::format_shortcut` texts and for images that
    /// fail to load. FlatCraft uses neither: shortcut hints are written out
    /// ("Cmd+S") and icons are textures. egui's own fonts would cover them,
    /// but they bring the OFL and Ubuntu font licences, which deny.toml does
    /// not allow. If this test fails, Plex gained them: drop the rule.
    #[test]
    fn plex_lacks_the_modifier_symbols() {
        assert_eq!(
            missing("\u{2318}\u{21e7}\u{2303}\u{2325}\u{26a0}"),
            vec!['\u{2318}', '\u{21e7}', '\u{2303}', '\u{2325}', '\u{26a0}']
        );
    }
}
