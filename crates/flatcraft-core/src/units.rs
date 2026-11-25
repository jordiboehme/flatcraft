//! Unit constants and conversions. The internal unit is millimetres (`f64`).

/// Millimetres per inch.
pub const MM_PER_INCH: f64 = 25.4;

/// CSS pixels per inch (SVG user units).
const PX_PER_INCH: f64 = 96.0;

/// Converts SVG pixels (96 per inch) to millimetres.
pub fn px_to_mm(px: f64) -> f64 {
    px * MM_PER_INCH / PX_PER_INCH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn px_to_mm_uses_96_dpi() {
        assert!((px_to_mm(96.0) - 25.4).abs() < 1e-12);
    }
}
