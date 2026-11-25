//! System font database, loaded once.

use std::sync::{Arc, OnceLock};

static SYSTEM_FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();

/// IBM Plex Sans (OFL-1.1), the font FlatCraft ships for its window.
const PLEX_SANS: &[u8] = include_bytes!("../../../assets/fonts/IBMPlexSans-Regular.ttf");

/// Returns the shared font database: the system fonts and IBM Plex Sans,
/// which FlatCraft ships, so SVG text in Plex draws on every computer.
/// Fonts are loaded on first use.
pub fn system_fonts() -> Arc<usvg::fontdb::Database> {
    SYSTEM_FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            db.load_font_data(PLEX_SANS.to_vec());
            Arc::new(db)
        })
        .clone()
}

/// The font database an SVG needs: the system fonts when it may hold text
/// (the word "text" appears anywhere, so a `<text>` element in any
/// namespace, or a style that names one, always gets them; a gzipped
/// SVG always gets them too), else an
/// empty one, so
/// rebuilding a drawing without text never waits for the system fonts to
/// load (a few hundred ms on the first call).
pub fn fonts_for(svg: &[u8]) -> Arc<usvg::fontdb::Database> {
    static EMPTY: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    // A gzipped SVG (.svgz, magic 1f 8b) hides its words: it may hold text.
    if svg.starts_with(&[0x1f, 0x8b]) || svg.windows(4).any(|w| w == b"text") {
        system_fonts()
    } else {
        EMPTY
            .get_or_init(|| Arc::new(usvg::fontdb::Database::new()))
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_svgs_that_may_hold_text_load_the_system_fonts() {
        assert_eq!(fonts_for(b"<svg><rect/></svg>").len(), 0);
        let with_text = fonts_for(b"<svg><svg:text>Hi</svg:text></svg>");
        assert!(Arc::ptr_eq(&with_text, &system_fonts()));
        let gzipped = fonts_for(&[0x1f, 0x8b, 0x08, 0x00]);
        assert!(Arc::ptr_eq(&gzipped, &system_fonts()));
    }
}
