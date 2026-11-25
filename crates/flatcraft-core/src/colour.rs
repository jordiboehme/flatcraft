//! Colours that read on a mat, after the WCAG 2 contrast rule. The canvas
//! and the engine's preview image use the same rule.

fn linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Relative luminance after WCAG 2.
pub fn luminance(c: [u8; 3]) -> f64 {
    0.2126 * linear(c[0]) + 0.7152 * linear(c[1]) + 0.0722 * linear(c[2])
}

/// WCAG contrast ratio, 1 to 21.
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Near-black on light mats, near-white on dark ones.
pub fn contrast_colour(mat: [u8; 3]) -> [u8; 3] {
    const DARK: [u8; 3] = [0x1B, 0x1E, 0x22];
    const LIGHT: [u8; 3] = [0xF4, 0xF5, 0xF7];
    if contrast_ratio(mat, DARK) >= contrast_ratio(mat, LIGHT) {
        DARK
    } else {
        LIGHT
    }
}

/// The mat margin (the part outside the cutting area): the mat colour a
/// little darker on a light mat and a little lighter on a dark one, so the
/// cutting area reads as the part that counts on either.
pub fn margin_colour(mat: [u8; 3]) -> [u8; 3] {
    if luminance(mat) >= 0.18 {
        mat.map(|c| (f64::from(c) * 0.88).round() as u8)
    } else {
        mat.map(|c| (f64::from(c) + (255.0 - f64::from(c)) * 0.14).round() as u8)
    }
}

/// The colour of cut moves (blade down) on `mat`: the orange of the cut
/// line in the icon style, a deeper one on light mats and a brighter one on
/// dark ones, whichever reads better. Travel moves use [`contrast_colour`].
pub fn cut_colour(mat: [u8; 3]) -> [u8; 3] {
    const DEEP: [u8; 3] = [0xB8, 0x4A, 0x00];
    const BRIGHT: [u8; 3] = [0xFF, 0x9A, 0x3C];
    if contrast_ratio(mat, DEEP) >= contrast_ratio(mat, BRIGHT) {
        DEEP
    } else {
        BRIGHT
    }
}

/// The colour of the selection box, its handles, the selection frame and
/// the Arrange ghosts on `mat`: a teal that reads on light mats (the
/// lighter teal on dark ones), and never the colour of the object outlines.
pub fn selection_colour(mat: [u8; 3]) -> [u8; 3] {
    const DEEP: [u8; 3] = [0x0A, 0x7A, 0x8F];
    const BRIGHT: [u8; 3] = [0x35, 0xD6, 0xE6];
    if contrast_ratio(mat, DEEP) >= contrast_ratio(mat, BRIGHT) {
        DEEP
    } else {
        BRIGHT
    }
}

/// An object's own colour if it reads on the mat (ratio at least 3), else
/// the contrast colour.
pub fn object_colour(own: Option<[u8; 3]>, mat: [u8; 3]) -> [u8; 3] {
    match own {
        Some(c) if contrast_ratio(c, mat) >= 3.0 => c,
        _ => contrast_colour(mat),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margin_is_darker_and_selection_reads_on_paper_and_dark() {
        let paper = [0xFB, 0xF4, 0xE4];
        let m = margin_colour(paper);
        assert!(luminance(m) < luminance(paper));
        assert!(contrast_ratio(m, paper) > 1.1);
        for mat in [paper, [0x20, 0x22, 0x26]] {
            let c = selection_colour(mat);
            assert!(contrast_ratio(c, mat) >= 3.0, "{mat:?}");
            assert_ne!(c, contrast_colour(mat));
        }
    }

    #[test]
    fn margin_stays_visible_on_dark_mats() {
        let dark = [0x20, 0x22, 0x26];
        let m = margin_colour(dark);
        assert!(luminance(m) > luminance(dark));
        assert!(contrast_ratio(m, dark) > 1.1);
    }

    #[test]
    fn cut_orange_reads_on_paper_and_dark_and_differs_from_the_selection() {
        for mat in [[0xFB, 0xF4, 0xE4], [0x20, 0x22, 0x26]] {
            let c = cut_colour(mat);
            assert!(contrast_ratio(c, mat) >= 3.0, "{mat:?}");
            assert_ne!(c, selection_colour(mat));
            assert_ne!(c, contrast_colour(mat));
        }
        assert_eq!(cut_colour([0xFB, 0xF4, 0xE4]), [0xB8, 0x4A, 0x00]);
        assert_eq!(cut_colour([0x20, 0x22, 0x26]), [0xFF, 0x9A, 0x3C]);
    }

    #[test]
    fn contrast_follows_the_mat() {
        assert_eq!(contrast_colour([0xF4, 0xF1, 0xEA]), [0x1B, 0x1E, 0x22]);
        assert_eq!(contrast_colour([0x20, 0x20, 0x20]), [0xF4, 0xF5, 0xF7]);
        // Dark grid lines on paper, light ones on a dark mat a person picks.
        assert_eq!(contrast_colour([0xFB, 0xF4, 0xE4]), [0x1B, 0x1E, 0x22]);
        assert_eq!(contrast_colour([0x2E, 0x3A, 0x4A]), [0xF4, 0xF5, 0xF7]);
        // Black on white is 21:1.
        assert!((contrast_ratio([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
        // A light grey shape on a light mat falls back to the contrast colour.
        let mat = [0xF4, 0xF1, 0xEA];
        assert_eq!(
            object_colour(Some([0xDD, 0xDD, 0xDD]), mat),
            contrast_colour(mat)
        );
        assert_eq!(
            object_colour(Some([0xC0, 0x10, 0x10]), mat),
            [0xC0, 0x10, 0x10]
        );
        assert_eq!(object_colour(None, mat), contrast_colour(mat));
    }
}
