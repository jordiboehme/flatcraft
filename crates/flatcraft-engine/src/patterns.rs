//! The test pattern sheet: what to cut first on a real machine to check
//! scale, mirroring and corner quality.
//!
//! The sheet is a mm-based SVG, 280 x 280 mm, so it fits inside the cutting
//! area of the 12x12 inch mat (294.8 mm). Every pattern is one `<g id>` group,
//! which the importer keeps as `CutPath.label`. The patterns lie at least
//! 10 mm apart, and none comes closer than 10 mm to the sheet border.

use std::fmt::Write;

/// Page size of the sheet, mm.
pub const SHEET_MM: f64 = 280.0;

/// Name of the object `tools.test_patterns` adds.
pub const SHEET_NAME: &str = "Test patterns";

/// Apex angles of the star tips, degrees.
pub const STAR_ANGLES: [u32; 8] = [15, 30, 45, 60, 90, 120, 150, 170];

/// Leg length of a star tip, mm.
const STAR_LEG_MM: f64 = 20.0;
/// Distance between the apexes of neighbouring tips, mm.
const STAR_PITCH_MM: f64 = 25.0;
/// Circle radii, mm.
const CIRCLE_RADII: [f64; 6] = [0.5, 1.0, 2.0, 5.0, 10.0, 25.0];

fn group(out: &mut String, id: &str, body: &str) {
    let _ = writeln!(out, r#"<g id="{id}">{body}</g>"#);
}

/// The sheet as SVG text.
///
/// Layout (page mm, y down):
/// - `square-50` 10..60 x 10..60; `hole-in-square` 80..110 x 10..40 with the
///   10 mm hole centred; `open-line` 130..170 at y 25; `s-curve` 190..260 x 10..40.
/// - `circles` along y 105 from x 10 to 147 with 10 mm between them;
///   `f-glyph` 170..184 x 85..105; `ruler-y` at x 260 from y 70 to 170.
/// - `star` tips with apexes at y 150, x 25, 50, ... 200.
/// - `ruler-x` at y 215 from x 10 to 110.
///
/// The F is asymmetric on purpose: stem on the left (x 170..174), a 14 mm top
/// arm and a 11 mm middle arm pointing right (+x). A mirrored or rotated
/// output shows at once.
pub fn sheet_svg() -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{SHEET_MM}mm" height="{SHEET_MM}mm" viewBox="0 0 {SHEET_MM} {SHEET_MM}" fill="none" stroke="#000000" stroke-width="0.1">"##
    );

    group(
        &mut s,
        "square-50",
        r#"<rect x="10" y="10" width="50" height="50"/>"#,
    );

    // Eight separate closed V tips, apex up (-y).
    for (i, angle) in STAR_ANGLES.iter().enumerate() {
        let half = f64::from(*angle).to_radians() / 2.0;
        let ax = 25.0 + STAR_PITCH_MM * i as f64;
        let ay = 150.0;
        let dx = STAR_LEG_MM * half.sin();
        let dy = STAR_LEG_MM * half.cos();
        group(
            &mut s,
            &format!("star-{angle}"),
            &format!(
                r#"<path d="M{ax:.6} {ay:.6} L{:.6} {:.6} L{:.6} {:.6} Z"/>"#,
                ax + dx,
                ay + dy,
                ax - dx,
                ay + dy
            ),
        );
    }

    // Circles in a row, 10 mm between them, centres on y 105.
    let mut circles = String::new();
    let mut x = 10.0;
    for r in CIRCLE_RADII {
        let _ = write!(circles, r#"<circle cx="{}" cy="105" r="{r}"/>"#, x + r);
        x += 2.0 * r + 10.0;
    }
    group(&mut s, "circles", &circles);

    group(
        &mut s,
        "hole-in-square",
        r#"<rect x="80" y="10" width="30" height="30"/><rect x="90" y="20" width="10" height="10"/>"#,
    );
    group(&mut s, "open-line", r#"<path d="M130 25 H170"/>"#);
    group(
        &mut s,
        "s-curve",
        r#"<path d="M190 40 C200 5 225 5 225 25 C225 45 250 45 260 10"/>"#,
    );
    group(
        &mut s,
        "f-glyph",
        r#"<path d="M170 85 H184 V89 H174 V93 H181 V97 H174 V105 H170 Z"/>"#,
    );

    // Rulers: a 100 mm line with a 5 mm tick every 10 mm.
    let mut rx = String::from("M10 215 H110");
    for i in 0..=10 {
        let _ = write!(rx, " M{} 215 V210", 10 + 10 * i);
    }
    group(&mut s, "ruler-x", &format!(r#"<path d="{rx}"/>"#));
    let mut ry = String::from("M260 70 V170");
    for i in 0..=10 {
        let _ = write!(ry, " M260 {} H255", 70 + 10 * i);
    }
    group(&mut s, "ruler-y", &format!(r#"<path d="{ry}"/>"#));

    s.push_str("</svg>\n");
    s
}
