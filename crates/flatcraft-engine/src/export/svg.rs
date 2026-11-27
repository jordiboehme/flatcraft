//! SVG writer: the page is the mat's grid, `width`/`height` in mm and a
//! `viewBox` in mm, so one user unit is one millimetre and y points down
//! as on the mat. One `<g>` per object (`id="object-<id>"`, the object's
//! name in `data-name`), one stroked `<path>` per shape, no fill. The weed
//! box and weed lines come last, in `<g id="weed">`.

use std::fmt::Write;

use super::{ExportDoc, ExportShape, num};

/// Stroke width of the paths, mm: thin, but visible in a viewer.
pub const STROKE_MM: f64 = 0.1;

/// The SVG text of `doc`.
pub fn write(doc: &ExportDoc) -> String {
    let (w, h) = (num(doc.width_mm), num(doc.height_mm));
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\">"
    );
    out.push_str("<title>FlatCraft cut paths</title>\n");
    for o in &doc.objects {
        let _ = writeln!(
            out,
            "<g id=\"object-{}\" data-name=\"{}\" fill=\"none\" stroke=\"#000000\" stroke-width=\"{}\" stroke-linejoin=\"round\" stroke-linecap=\"round\">",
            o.id.0,
            escape(&o.name),
            num(STROKE_MM)
        );
        for shape in &o.shapes {
            let _ = writeln!(out, "<path d=\"{}\"/>", path_data(shape));
        }
        out.push_str("</g>\n");
    }
    if !doc.weeds.is_empty() {
        let _ = writeln!(
            out,
            "<g id=\"weed\" data-name=\"{}\" fill=\"none\" stroke=\"#000000\" stroke-width=\"{}\" stroke-linejoin=\"round\" stroke-linecap=\"round\">",
            super::WEED_NAME,
            num(STROKE_MM)
        );
        for shape in &doc.weeds {
            let _ = writeln!(out, "<path d=\"{}\"/>", path_data(shape));
        }
        out.push_str("</g>\n");
    }
    out.push_str("</svg>\n");
    out
}

/// The `d` attribute of one shape.
fn path_data(shape: &ExportShape) -> String {
    let mut d = String::new();
    for (i, p) in shape.points.iter().enumerate() {
        let cmd = if i == 0 { "M" } else { " L" };
        let _ = write!(d, "{cmd}{} {}", num(p.x), num(p.y));
    }
    if shape.closed {
        d.push_str(" Z");
    }
    d
}

/// `s` as XML attribute text. Control characters, which XML 1.0 does not
/// allow, become spaces.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{ExportObject, ExportShape};
    use flatcraft_core::document::ObjectId;
    use kurbo::Point;

    fn doc() -> ExportDoc {
        ExportDoc {
            width_mm: 304.8,
            height_mm: 304.8,
            objects: vec![ExportObject {
                id: ObjectId(7),
                name: "tab\tname".into(),
                shapes: vec![
                    ExportShape {
                        points: vec![
                            Point::new(1.0, 2.0),
                            Point::new(3.5, 2.0),
                            Point::new(3.5, 4.0),
                        ],
                        closed: true,
                    },
                    ExportShape {
                        points: vec![Point::new(10.0, 10.0), Point::new(20.0, 12.25)],
                        closed: false,
                    },
                ],
            }],
            weeds: Vec::new(),
        }
    }

    #[test]
    fn one_group_per_object_and_one_path_per_shape() {
        let text = write(&doc());
        assert!(text.contains(r#"width="304.8mm" height="304.8mm" viewBox="0 0 304.8 304.8""#));
        assert!(
            text.contains(r#"<g id="object-7" data-name="tab name""#),
            "{text}"
        );
        assert!(
            text.contains(r#"<path d="M1 2 L3.5 2 L3.5 4 Z"/>"#),
            "{text}"
        );
        assert!(text.contains(r#"<path d="M10 10 L20 12.25"/>"#), "{text}");
        assert_eq!(text.matches("<g ").count(), 1);
        assert_eq!(text.matches("<path ").count(), 2);
    }
}
