//! PDF writer (pdf-writer): one page the size of the mat's grid at 1:1,
//! thin black strokes, no fill. The content stream draws in millimetres
//! with y down, like the mat: its transform scales mm to points and flips
//! y against the page height, so the mat's 0,0 is the top left of the
//! page. The stream is not compressed.

use pdf_writer::types::{LineCapStyle, LineJoinStyle};
use pdf_writer::{Content, Pdf, Rect, Ref, TextStr};

use super::ExportDoc;

/// Points per millimetre.
pub const PT_PER_MM: f64 = 72.0 / 25.4;

/// Stroke width, mm.
pub const STROKE_MM: f32 = 0.1;

/// The PDF bytes of `doc`.
pub fn write(doc: &ExportDoc) -> Vec<u8> {
    let catalog = Ref::new(1);
    let tree = Ref::new(2);
    let page = Ref::new(3);
    let content_id = Ref::new(4);
    let info = Ref::new(5);
    let (w, h) = (
        (doc.width_mm * PT_PER_MM) as f32,
        (doc.height_mm * PT_PER_MM) as f32,
    );

    let mut pdf = Pdf::new();
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).kids([page]).count(1);
    {
        let mut p = pdf.page(page);
        p.media_box(Rect::new(0.0, 0.0, w, h));
        p.parent(tree);
        p.contents(content_id);
        p.resources();
    }
    pdf.document_info(info)
        .title(TextStr("FlatCraft cut paths"))
        .producer(TextStr("FlatCraft"));

    let k = PT_PER_MM as f32;
    let mut c = Content::new();
    c.save_state();
    c.transform([k, 0.0, 0.0, -k, 0.0, h]);
    c.set_line_width(STROKE_MM);
    c.set_line_join(LineJoinStyle::RoundJoin);
    c.set_line_cap(LineCapStyle::RoundCap);
    c.set_stroke_gray(0.0);
    for shape in doc.objects.iter().flat_map(|o| &o.shapes) {
        let Some((first, rest)) = shape.points.split_first() else {
            continue;
        };
        c.move_to(first.x as f32, first.y as f32);
        for p in rest {
            c.line_to(p.x as f32, p.y as f32);
        }
        if shape.closed {
            c.close_path();
        }
        c.stroke();
    }
    c.restore_state();
    pdf.stream(content_id, &c.finish());
    pdf.finish()
}
