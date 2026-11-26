//! DXF writer: ASCII AutoCAD R12 (`AC1009`), millimetres (`$INSUNITS` 4).
//! One layer per object (its name made safe for R12 and unique), declared
//! in the LAYER table; one POLYLINE with VERTEX entities per shape, the
//! closed flag set on rings (the first point is not repeated).
//!
//! DXF's y axis points up, the mat's points down: y is flipped against
//! the page height (`y_dxf = height - y`), so the drawing shows the layout
//! the right way round, with the mat's 0,0 at the top left of the page.

use std::fmt::Write;

use super::{ExportDoc, num};

/// Longest layer name R12 takes.
const MAX_LAYER: usize = 31;

/// The DXF text of `doc`.
pub fn write(doc: &ExportDoc) -> String {
    let layers = layer_names(doc);
    let h = doc.height_mm;
    let mut out = Dxf::default();
    out.section("HEADER");
    out.pair(9, "$ACADVER");
    out.pair(1, "AC1009");
    // $INSUNITS came with later DXF versions; R12 readers ignore it and
    // take the numbers as they are. The coordinates are millimetres either
    // way, and newer readers read the unit from here.
    out.pair(9, "$INSUNITS");
    out.pair(70, "4");
    out.pair(9, "$EXTMIN");
    out.point(0.0, 0.0);
    out.pair(9, "$EXTMAX");
    out.point(doc.width_mm, doc.height_mm);
    out.end_section();

    out.section("TABLES");
    out.pair(0, "TABLE");
    out.pair(2, "LTYPE");
    out.pair(70, "1");
    out.pair(0, "LTYPE");
    out.pair(2, "CONTINUOUS");
    out.pair(70, "0");
    out.pair(3, "Solid line");
    out.pair(72, "65");
    out.pair(73, "0");
    out.pair(40, "0.0");
    out.pair(0, "ENDTAB");
    out.pair(0, "TABLE");
    out.pair(2, "LAYER");
    out.pair(70, &layers.len().to_string());
    for name in &layers {
        out.pair(0, "LAYER");
        out.pair(2, name);
        out.pair(70, "0");
        out.pair(62, "7");
        out.pair(6, "CONTINUOUS");
    }
    out.pair(0, "ENDTAB");
    out.end_section();

    out.section("ENTITIES");
    for (o, layer) in doc.objects.iter().zip(&layers) {
        for shape in &o.shapes {
            out.pair(0, "POLYLINE");
            out.pair(8, layer);
            out.pair(66, "1");
            out.point(0.0, 0.0);
            out.pair(70, if shape.closed { "1" } else { "0" });
            for p in &shape.points {
                out.pair(0, "VERTEX");
                out.pair(8, layer);
                out.point(p.x, h - p.y);
            }
            out.pair(0, "SEQEND");
            out.pair(8, layer);
        }
    }
    out.end_section();
    out.pair(0, "EOF");
    out.0
}

#[derive(Default)]
struct Dxf(String);

impl Dxf {
    /// One group: the code right-aligned in three columns, then the value.
    fn pair(&mut self, code: i32, value: &str) {
        let _ = write!(self.0, "{code:>3}\n{value}\n");
    }

    /// A point (codes 10, 20, 30), z = 0.
    fn point(&mut self, x: f64, y: f64) {
        self.pair(10, &num(x));
        self.pair(20, &num(y));
        self.pair(30, "0");
    }

    fn section(&mut self, name: &str) {
        self.pair(0, "SECTION");
        self.pair(2, name);
    }

    fn end_section(&mut self) {
        self.pair(0, "ENDSEC");
    }
}

/// One layer name per object, in order: upper case `A-Z`, `0-9`, `_` and
/// `-` (anything else becomes `_`), at most 31 characters, never empty and
/// never twice (a clash gets `_<object id>`).
fn layer_names(doc: &ExportDoc) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for o in &doc.objects {
        let mut name: String = o
            .name
            .chars()
            .map(|c| {
                let c = c.to_ascii_uppercase();
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .take(MAX_LAYER)
            .collect();
        if name.is_empty() || name == "0" {
            name = format!("OBJECT_{}", o.id.0);
        }
        if out.contains(&name) {
            let suffix = format!("_{}", o.id.0);
            let keep = MAX_LAYER.saturating_sub(suffix.len());
            name = format!("{}{suffix}", name.chars().take(keep).collect::<String>());
            let mut n = 2;
            while out.contains(&name) {
                name = format!("OBJECT_{}_{n}", o.id.0);
                n += 1;
            }
        }
        out.push(name);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{ExportObject, ExportShape};
    use flatcraft_core::document::ObjectId;
    use kurbo::Point;

    fn object(id: u64, name: &str) -> ExportObject {
        ExportObject {
            id: ObjectId(id),
            name: name.into(),
            shapes: vec![ExportShape {
                points: vec![Point::new(0.0, 0.0), Point::new(10.0, 5.0)],
                closed: false,
            }],
        }
    }

    #[test]
    fn layer_names_are_safe_short_and_unique() {
        let doc = ExportDoc {
            width_mm: 100.0,
            height_mm: 100.0,
            objects: vec![
                object(1, "Star badge"),
                object(2, "star-badge"),
                object(3, "star badge"),
                object(4, ""),
                object(5, &"x".repeat(50)),
                object(6, "Über"),
            ],
        };
        let names = layer_names(&doc);
        assert_eq!(names[0], "STAR_BADGE");
        assert_eq!(names[1], "STAR-BADGE");
        assert_eq!(names[2], "STAR_BADGE_3");
        assert_eq!(names[3], "OBJECT_4");
        assert_eq!(names[4].len(), MAX_LAYER);
        assert_eq!(names[5], "_BER");
    }

    #[test]
    fn an_open_line_has_no_closed_flag_and_y_is_flipped() {
        let doc = ExportDoc {
            width_mm: 100.0,
            height_mm: 80.0,
            objects: vec![object(1, "a")],
        };
        let text = write(&doc);
        assert!(text.contains(" 70\n0\n  0\nVERTEX\n"), "{text}");
        // (10, 5) on the mat is (10, 75) in the drawing.
        assert!(text.contains(" 10\n10\n 20\n75\n"), "{text}");
        assert!(text.starts_with("  0\nSECTION\n  2\nHEADER\n"));
        assert!(text.ends_with("  0\nEOF\n"));
    }
}
