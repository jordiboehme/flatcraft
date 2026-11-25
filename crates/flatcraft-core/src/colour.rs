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
    fn contrast_follows_the_mat() {
        assert_eq!(contrast_colour([0xF4, 0xF1, 0xEA]), [0x1B, 0x1E, 0x22]);
        assert_eq!(contrast_colour([0x20, 0x20, 0x20]), [0xF4, 0xF5, 0xF7]);
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
