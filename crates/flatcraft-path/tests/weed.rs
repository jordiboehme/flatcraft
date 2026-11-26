//! Weed box and weed lines (Task 51), through the whole pipeline.

use flatcraft_core::document::{ObjectId, WeedLines, WeedSettings};
use flatcraft_path::feed::{BAND_MM, band_of};
use flatcraft_path::weed::{self, BOX_OVERLAP_MM, WEED_BOX_OBJECT, WEED_CLEARANCE_MM};
use flatcraft_path::{
    CutPlan, KeepMode, PathRef, PipelineSettings, PlacedPath, PlanError, SettingsField, Toolpath,
    ToolpathKind, plan,
};
use kurbo::{BezPath, Circle, Point, Rect, Shape};
use proptest::prelude::*;

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((x, y));
    p.line_to((x + w, y));
    p.line_to((x + w, y + h));
    p.line_to((x, y + h));
    p.close_path();
    p
}

fn placed(id: u64, path: BezPath) -> PlacedPath {
    PlacedPath {
        path,
        source: PathRef {
            object: ObjectId(id),
            path_index: 0,
        },
    }
}

fn settings(weed: WeedSettings) -> PipelineSettings {
    PipelineSettings {
        weed,
        ..PipelineSettings::with_defaults(Rect::new(0.0, 0.0, 300.0, 300.0), Point::ZERO)
    }
}

fn box_only(margin_mm: f64, per_object: bool) -> WeedSettings {
    WeedSettings {
        weed_box: true,
        margin_mm,
        per_object,
        ..WeedSettings::default()
    }
}

fn lines_only(lines: WeedLines, spacing_mm: f64) -> WeedSettings {
    WeedSettings {
        lines,
        spacing_mm,
        ..WeedSettings::default()
    }
}

fn weeds(out: &CutPlan) -> Vec<&Toolpath> {
    out.toolpaths
        .iter()
        .filter(|t| t.kind == ToolpathKind::Weed)
        .collect()
}

fn bounds(points: &[Point]) -> Rect {
    points
        .iter()
        .skip(1)
        .fold(Rect::from_points(points[0], points[0]), |r, p| {
            r.union_pt(*p)
        })
}

fn close(a: Rect, b: Rect) -> bool {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1]
        .iter()
        .all(|d| d.abs() < 1e-9)
}

#[test]
fn the_box_keeps_the_margin_around_the_design() {
    // structural: the box is the design box plus the margin by
    // construction; flatcraft-sim/tests/weed.rs judges the behaviour.
    let input = [
        placed(1, rect_path(50.0, 60.0, 20.0, 10.0)),
        placed(2, rect_path(100.0, 40.0, 5.0, 5.0)),
    ];
    let out = plan(&input, &settings(box_only(3.0, false))).unwrap();
    let w = weeds(&out);
    assert_eq!(w.len(), 1, "{w:?}");
    let b = w[0];
    assert!(b.closed);
    assert_eq!(b.source.object, WEED_BOX_OBJECT);
    // A closed ring: four corners and the start again, then the overcut.
    assert_eq!(b.lap().len(), 5);
    assert!(close(bounds(b.lap()), Rect::new(47.0, 37.0, 108.0, 73.0)));
    assert_eq!(out.dropped_weed_boxes, 0);
}

#[test]
fn a_box_per_object_merges_boxes_that_would_touch() {
    // structural: restates the merge rule; the behaviour is judged by
    // weeds_keep_clear_of_the_design and the simulation.
    // 1 and 2 are 5 mm apart: their 3 mm boxes overlap and merge. 3 is
    // far away and gets its own box.
    let input = [
        placed(1, rect_path(20.0, 20.0, 10.0, 10.0)),
        placed(2, rect_path(35.0, 20.0, 10.0, 10.0)),
        placed(3, rect_path(150.0, 150.0, 10.0, 10.0)),
    ];
    let out = plan(&input, &settings(box_only(3.0, true))).unwrap();
    let mut boxes: Vec<Rect> = weeds(&out).iter().map(|t| bounds(t.lap())).collect();
    boxes.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    assert_eq!(boxes.len(), 2, "{boxes:?}");
    assert!(close(boxes[0], Rect::new(17.0, 17.0, 48.0, 33.0)));
    assert!(close(boxes[1], Rect::new(147.0, 147.0, 163.0, 163.0)));
    // One box around everything without per object.
    let out = plan(&input, &settings(box_only(3.0, false))).unwrap();
    let boxes: Vec<Rect> = weeds(&out).iter().map(|t| bounds(t.lap())).collect();
    assert_eq!(boxes.len(), 1);
    assert!(close(boxes[0], Rect::new(17.0, 17.0, 163.0, 163.0)));
}

#[test]
fn a_box_that_does_not_fit_is_left_out_and_counted() {
    // 0.5 mm from the area's left edge: a box there would lie on the design.
    let input = [placed(1, rect_path(0.5, 20.0, 10.0, 10.0))];
    let out = plan(&input, &settings(box_only(3.0, false))).unwrap();
    assert!(weeds(&out).is_empty());
    assert_eq!(out.dropped_weed_boxes, 1);
    // 1.5 mm fits: the box is clamped to the area and still keeps 1 mm.
    let input = [placed(1, rect_path(1.5, 20.0, 10.0, 10.0))];
    let out = plan(&input, &settings(box_only(3.0, false))).unwrap();
    let w = weeds(&out);
    assert_eq!(w.len(), 1);
    assert!(close(bounds(w[0].lap()), Rect::new(0.0, 17.0, 14.5, 33.0)));
}

#[test]
fn no_weed_paths_when_off() {
    // structural
    let input = [
        placed(1, rect_path(20.0, 20.0, 100.0, 100.0)),
        placed(2, rect_path(150.0, 150.0, 10.0, 10.0)),
    ];
    for weed in [
        WeedSettings::default(),
        WeedSettings {
            margin_mm: 10.0,
            spacing_mm: 5.0,
            per_object: true,
            ..WeedSettings::default()
        },
    ] {
        let out = plan(&input, &settings(weed)).unwrap();
        assert!(weeds(&out).is_empty());
        assert_eq!(out.toolpaths.len(), 2);
    }
}

#[test]
fn design_first_then_lines_then_the_box() {
    // structural: the order `weed::sequence` builds.
    let input = [
        placed(1, rect_path(20.0, 20.0, 30.0, 30.0)),
        placed(2, rect_path(80.0, 30.0, 40.0, 20.0)),
        placed(3, Circle::new((60.0, 100.0), 15.0).to_path(0.1)),
    ];
    let weed = WeedSettings {
        weed_box: true,
        lines: WeedLines::Both,
        spacing_mm: 10.0,
        ..WeedSettings::default()
    };
    for passes in [1, 2] {
        let s = PipelineSettings {
            passes,
            ..settings(weed)
        };
        let out = plan(&input, &s).unwrap();
        // 0 design, 1 weed line, 2 weed box: never goes down.
        let rank: Vec<u8> = out
            .toolpaths
            .iter()
            .map(|t| match (t.kind, t.closed) {
                (ToolpathKind::Cut, _) => 0,
                (ToolpathKind::Weed, false) => 1,
                (ToolpathKind::Weed, true) => 2,
                (ToolpathKind::Alignment, _) => panic!("no alignment stroke"),
            })
            .collect();
        assert!(rank.windows(2).all(|w| w[0] <= w[1]), "{rank:?}");
        assert!(rank.contains(&1), "{rank:?}");
        assert_eq!(rank.last(), Some(&2));
        // Weed paths get the passes too: the box chains its laps.
        let b = out.toolpaths.last().unwrap();
        assert_eq!(b.laps, passes);
    }
}

#[test]
fn weed_lines_split_at_shapes_and_skip_counters() {
    // A ring with a hole (one object, like the letter O): a horizontal
    // line through its middle has a piece left and right of it, none in
    // the counter, and keeps the clearance.
    let mut o = rect_path(100.0, 100.0, 60.0, 40.0);
    o.extend(rect_path(110.0, 105.0, 40.0, 30.0));
    let input = [placed(1, o)];
    let s = settings(WeedSettings {
        margin_mm: 10.0,
        ..lines_only(WeedLines::Horizontal, 20.0)
    });
    let out = plan(&input, &s).unwrap();
    let at_110: Vec<(f64, f64)> = weeds(&out)
        .iter()
        .filter(|t| (t.points[0].y - 110.0).abs() < 1e-9)
        .map(|t| {
            let (a, b) = (t.points[0].x, t.points[1].x);
            (a.min(b), a.max(b))
        })
        .collect();
    let mut at_110 = at_110;
    at_110.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(at_110.len(), 2, "{at_110:?}");
    // The lines run across the design's box plus the margin (lines at
    // 90 + 20 k: 110 crosses the counter).
    assert!((at_110[0].0 - 90.0).abs() < 1e-9, "{at_110:?}");
    assert!(at_110[0].1 < 100.0 - WEED_CLEARANCE_MM, "{at_110:?}");
    assert!(at_110[1].0 > 160.0 + WEED_CLEARANCE_MM, "{at_110:?}");
    assert!((at_110[1].1 - 170.0).abs() < 1e-9, "{at_110:?}");
}

#[test]
fn weed_settings_out_of_range_are_refused() {
    let input = [placed(1, rect_path(20.0, 20.0, 10.0, 10.0))];
    for (weed, field) in [
        (box_only(0.5, false), SettingsField::WeedMargin),
        (box_only(f64::NAN, false), SettingsField::WeedMargin),
        (lines_only(WeedLines::Both, 1.0), SettingsField::WeedSpacing),
    ] {
        match plan(&input, &settings(weed)) {
            Err(PlanError::InvalidSettings { field: f, .. }) => assert_eq!(f, field),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn without_a_mat_weed_lines_follow_their_feed_band() {
    // Shapes in three bands and vertical lines through all of them: each
    // weed piece lies in one band and comes before any design path of a
    // later band.
    let input = [
        placed(1, rect_path(20.0, 10.0, 30.0, 20.0)),
        placed(2, rect_path(60.0, 70.0, 30.0, 20.0)),
        placed(3, rect_path(20.0, 120.0, 70.0, 20.0)),
    ];
    let s = PipelineSettings {
        monotonic_feed: true,
        ..settings(WeedSettings {
            weed_box: true,
            lines: WeedLines::Both,
            spacing_mm: 15.0,
            ..WeedSettings::default()
        })
    };
    let out = plan(&input, &s).unwrap();
    let band = |y: f64| (y.max(0.0) / BAND_MM).floor() as usize;
    let paths = &out.toolpaths;
    let mut seen_lines = 0;
    for (i, t) in paths.iter().enumerate() {
        if t.kind != ToolpathKind::Weed || t.closed {
            continue;
        }
        seen_lines += 1;
        let top = t.points[0].y.min(t.points[1].y);
        let bottom = t.points[0].y.max(t.points[1].y);
        // One band per piece (a piece may end on the band's lower edge).
        assert!(
            bottom <= (band(top) + 1) as f64 * BAND_MM + 1e-9,
            "piece {i} spans bands: {top}..{bottom}"
        );
        for later in &paths[i + 1..] {
            if later.kind == ToolpathKind::Cut {
                assert!(
                    band_of(later) >= band(top),
                    "a design path of an earlier band comes after weed piece {i}"
                );
            }
        }
    }
    assert!(seen_lines > 0);
}

#[test]
fn without_a_mat_the_box_is_cut_band_by_band() {
    let input = [
        placed(1, rect_path(20.0, 10.0, 30.0, 20.0)),
        placed(2, rect_path(60.0, 70.0, 30.0, 20.0)),
        placed(3, rect_path(20.0, 120.0, 70.0, 20.0)),
    ];
    let s = PipelineSettings {
        monotonic_feed: true,
        ..settings(WeedSettings {
            weed_box: true,
            margin_mm: 3.0,
            lines: WeedLines::Both,
            spacing_mm: 15.0,
            ..WeedSettings::default()
        })
    };
    let out = plan(&input, &s).unwrap();
    let paths = &out.toolpaths;
    let band = |y: f64| (y.max(0.0) / BAND_MM).floor() as usize;
    let top = |t: &Toolpath| t.points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    // No closed weed path: the box comes in open band pieces.
    assert!(
        !paths
            .iter()
            .any(|t| t.kind == ToolpathKind::Weed && t.closed)
    );
    let pieces: Vec<&Toolpath> = paths
        .iter()
        .filter(|t| t.source.object == WEED_BOX_OBJECT)
        .collect();
    // Cuts at 50 and 100 on both sides: four pieces.
    assert_eq!(pieces.len(), 4, "{pieces:?}");
    // The roll never goes back for a weed path: after a weed path of band
    // b, nothing of an earlier band is cut. A box piece may run on
    // BOX_OVERLAP_MM past its band edge (the join overlap), so its band is
    // read that far below its top.
    let weed_band = |t: &Toolpath| {
        if t.source.object == WEED_BOX_OBJECT {
            band(top(t) + BOX_OVERLAP_MM)
        } else {
            band(top(t))
        }
    };
    for (i, t) in paths.iter().enumerate() {
        if t.kind != ToolpathKind::Weed {
            continue;
        }
        let b = weed_band(t);
        for later in &paths[i + 1..] {
            let lb = if later.kind == ToolpathKind::Weed {
                weed_band(later)
            } else {
                band_of(later)
            };
            assert!(lb >= b, "toolpath {i} of band {b} is followed by band {lb}");
        }
    }
    // The pieces cover the whole rectangle and overlap at every join: each
    // piece runs BOX_OVERLAP_MM past its join into the next piece.
    let expected = Rect::new(17.0, 7.0, 93.0, 143.0);
    let on_side = |p: Point| {
        ((p.x - expected.x0).abs() < 1e-9 || (p.x - expected.x1).abs() < 1e-9)
            && (expected.y0 - 1e-9..=expected.y1 + 1e-9).contains(&p.y)
            || ((p.y - expected.y0).abs() < 1e-9 || (p.y - expected.y1).abs() < 1e-9)
                && (expected.x0 - 1e-9..=expected.x1 + 1e-9).contains(&p.x)
    };
    assert!(pieces.iter().all(|t| t.points.iter().all(|p| on_side(*p))));
    let length: f64 = pieces
        .iter()
        .map(|t| {
            t.points
                .windows(2)
                .map(|w| w[0].distance(w[1]))
                .sum::<f64>()
        })
        .sum();
    let perimeter = 2.0 * (expected.width() + expected.height());
    let overlap = pieces.len() as f64 * BOX_OVERLAP_MM;
    assert!((length - perimeter - overlap).abs() < 1e-9, "{length}");
    let near = |p: Point, t: &Toolpath| {
        t.points.windows(2).any(|w| {
            let v = w[1] - w[0];
            let u = ((p - w[0]).dot(v) / v.hypot2()).clamp(0.0, 1.0);
            p.distance(w[0] + v * u) < 1e-9
        })
    };
    // Every point of the outline lies on a piece.
    let corners = [
        Point::new(expected.x0, expected.y0),
        Point::new(expected.x1, expected.y0),
        Point::new(expected.x1, expected.y1),
        Point::new(expected.x0, expected.y1),
    ];
    for i in 0..4 {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        for k in 0..=400 {
            let p = a.lerp(b, f64::from(k) / 400.0);
            assert!(pieces.iter().any(|t| near(p, t)), "{p:?} is not cut");
        }
    }
    // Every piece end lies on another piece.
    for t in &pieces {
        for end in [t.points[0], *t.points.last().unwrap()] {
            let n = pieces.iter().filter(|u| near(end, u)).count();
            assert!(n >= 2, "end {end:?} is cut by {n} piece(s)");
        }
    }
    // The joins are where the outline crosses a band edge (50 and 100 on
    // both sides). One piece runs on past the join into the next, so the
    // join and 0.4 mm on one side of it are cut by two pieces.
    let cuts = |p: Point| pieces.iter().filter(|u| near(p, u)).count();
    for x in [expected.x0, expected.x1] {
        for y in [50.0, 100.0] {
            let j = Point::new(x, y);
            assert_eq!(cuts(j), 2, "join {j:?}");
            let (up, down) = (Point::new(x, y - 0.4), Point::new(x, y + 0.4));
            assert!(
                cuts(up) == 2 || cuts(down) == 2,
                "no overlap at the join {j:?}: {} above, {} below",
                cuts(up),
                cuts(down)
            );
        }
    }
    // The top edge is in the box's first band and the bottom edge after
    // all the design.
    let with = |y: f64| {
        paths
            .iter()
            .position(|t| {
                t.source.object == WEED_BOX_OBJECT
                    && t.points.windows(2).any(|w| w[0].y == y && w[1].y == y)
            })
            .unwrap()
    };
    let (top_edge, bottom_edge) = (with(expected.y0), with(expected.y1));
    assert_eq!(band(top(&paths[top_edge])), 0);
    let last_design = paths
        .iter()
        .rposition(|t| t.kind == ToolpathKind::Cut)
        .unwrap();
    assert!(bottom_edge > last_design, "{bottom_edge} {last_design}");
}

#[test]
fn without_a_mat_no_weed_piece_is_shorter_than_1_mm() {
    // The weed region runs from y 49.5 to 100.6 (design 52.5..97.6 plus
    // the 3 mm margin): cut at the band edges the vertical lines would
    // leave 0.5 mm and 0.6 mm end pieces. They join their neighbour, and
    // each line in the gap is still one unbroken cut from end to end.
    let input = [
        placed(1, rect_path(20.0, 52.5, 20.0, 45.1)),
        placed(2, rect_path(120.0, 52.5, 20.0, 45.1)),
    ];
    let s = PipelineSettings {
        monotonic_feed: true,
        ..settings(lines_only(WeedLines::Vertical, 15.0))
    };
    let out = plan(&input, &s).unwrap();
    let w = weeds(&out);
    assert!(!w.is_empty());
    for t in &w {
        let length: f64 = t.points.windows(2).map(|p| p[0].distance(p[1])).sum();
        assert!(length >= 1.0, "a weed piece of {length} mm: {:?}", t.points);
    }
    // The lines in the gap between the two shapes.
    let mut xs: Vec<f64> = w
        .iter()
        .map(|t| t.points[0].x)
        .filter(|x| (45.0..115.0).contains(x))
        .collect();
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    assert!(!xs.is_empty());
    for x in xs {
        let mut spans: Vec<(f64, f64)> = w
            .iter()
            .filter(|t| t.points[0].x == x)
            .map(|t| {
                let (a, b) = (t.points[0].y, t.points.last().unwrap().y);
                (a.min(b), a.max(b))
            })
            .collect();
        spans.sort_by(|p, q| p.0.total_cmp(&q.0));
        assert!((spans[0].0 - 49.5).abs() < 1e-9, "x {x}: {spans:?}");
        assert!(
            (spans.last().unwrap().1 - 100.6).abs() < 1e-9,
            "x {x}: {spans:?}"
        );
        for pair in spans.windows(2) {
            assert!(
                pair[1].0 <= pair[0].1 + 1e-9,
                "x {x}: a bridge in {spans:?}"
            );
        }
    }
}

#[test]
fn per_object_lines_stay_inside_the_boxes() {
    // Two stickers 200 mm apart: the vinyl between the boxes stays whole.
    let input = [
        placed(1, rect_path(10.0, 20.0, 40.0, 40.0)),
        placed(2, rect_path(250.0, 20.0, 40.0, 40.0)),
    ];
    let s = settings(WeedSettings {
        weed_box: true,
        margin_mm: 8.0,
        per_object: true,
        lines: WeedLines::Both,
        spacing_mm: 5.0,
    });
    let out = plan(&input, &s).unwrap();
    let boxes: Vec<Rect> = weeds(&out)
        .iter()
        .filter(|t| t.closed)
        .map(|t| bounds(t.lap()))
        .collect();
    assert_eq!(boxes.len(), 2);
    let lines: Vec<&Toolpath> = weeds(&out).into_iter().filter(|t| !t.closed).collect();
    assert!(!lines.is_empty());
    for t in lines {
        let inside = boxes.iter().any(|b| {
            t.points.iter().all(|p| {
                (b.x0 - 1e-9..=b.x1 + 1e-9).contains(&p.x)
                    && (b.y0 - 1e-9..=b.y1 + 1e-9).contains(&p.y)
            })
        });
        assert!(inside, "weed line {:?} runs outside every box", t.points);
    }
}

#[test]
fn boxes_closer_than_a_short_piece_merge() {
    // Margin 3: box one ends at x 43. Box two starts 0.5 mm after it (one
    // box, no sliver), then 6 mm after it (two boxes).
    let count = |gap: f64| {
        let input = [
            placed(1, rect_path(20.0, 20.0, 20.0, 20.0)),
            placed(2, rect_path(43.0 + gap + 3.0, 20.0, 20.0, 20.0)),
        ];
        let out = plan(&input, &settings(box_only(3.0, true))).unwrap();
        weeds(&out).len()
    };
    assert_eq!(count(0.5), 1);
    assert_eq!(count(6.0), 2);
}

#[test]
fn no_weed_line_between_shapes_1_5_mm_apart() {
    let input = [
        placed(1, rect_path(20.0, 20.0, 40.0, 60.0)),
        placed(2, rect_path(61.5, 20.0, 40.0, 60.0)),
    ];
    let s = settings(WeedSettings {
        margin_mm: 10.0,
        lines: WeedLines::Both,
        spacing_mm: 5.0,
        ..WeedSettings::default()
    });
    let out = plan(&input, &s).unwrap();
    let design: Vec<(Point, Point)> = out
        .toolpaths
        .iter()
        .filter(|t| t.kind == ToolpathKind::Cut)
        .flat_map(|t| {
            t.points
                .windows(2)
                .map(|w| (w[0], w[1]))
                .collect::<Vec<_>>()
        })
        .collect();
    let w = weeds(&out);
    assert!(!w.is_empty());
    for t in w {
        let (a, b) = (t.points[0], t.points[1]);
        // Nothing in the 1.5 mm gap between the shapes.
        let mid = a.midpoint(b);
        assert!(
            !((60.0..=61.5).contains(&mid.x) && (20.0..=80.0).contains(&mid.y)),
            "a weed line in the gap: {a:?} {b:?}"
        );
        for &(c, d) in &design {
            assert!(seg_distance(a, b, c, d) >= WEED_CLEARANCE_MM - 1e-9);
        }
    }
}

#[test]
fn a_kept_sheet_drops_the_box_and_the_lines() {
    // A stub keep rule for Task 29: the sheet is kept (Stencil). The box
    // asks the same question as the lines, so both are left out and the
    // box is counted.
    let ring = Toolpath::new(
        vec![
            Point::new(50.0, 50.0),
            Point::new(90.0, 50.0),
            Point::new(90.0, 90.0),
            Point::new(50.0, 90.0),
            Point::new(50.0, 50.0),
        ],
        true,
        PathRef {
            object: ObjectId(1),
            path_index: 0,
        },
    );
    let s = WeedSettings {
        weed_box: true,
        lines: WeedLines::Both,
        spacing_mm: 5.0,
        margin_mm: 10.0,
        ..WeedSettings::default()
    };
    let area = Rect::new(0.0, 0.0, 300.0, 300.0);
    let w = weed::generate(std::slice::from_ref(&ring), |_| KeepMode::Stencil, area, &s);
    assert!(w.lines.is_empty() && w.boxes.is_empty(), "{w:?}");
    assert_eq!(w.blocked_boxes, 1);
    // The same with the pieces kept: a box and lines.
    let w = weed::generate(&[ring], |_| KeepMode::Pieces, area, &s);
    assert_eq!(w.boxes.len(), 1);
    assert!(!w.lines.is_empty());
    assert_eq!(w.blocked_boxes, 0);
}

// ------------------------------------------------------------ property

/// The distance between segments `a`-`b` and `c`-`d`, 0 when they cross.
fn seg_distance(a: Point, b: Point, c: Point, d: Point) -> f64 {
    let cross = |o: Point, p: Point, q: Point| (p - o).cross(q - o);
    let (d1, d2) = (cross(c, d, a), cross(c, d, b));
    let (d3, d4) = (cross(a, b, c), cross(a, b, d));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return 0.0;
    }
    let pt = |p: Point, s0: Point, s1: Point| {
        let v = s1 - s0;
        let t = if v.hypot2() > 0.0 {
            ((p - s0).dot(v) / v.hypot2()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        p.distance(s0 + v * t)
    };
    pt(a, c, d)
        .min(pt(b, c, d))
        .min(pt(c, a, b))
        .min(pt(d, a, b))
}

/// Plain even-odd point in polygon, independent of the crate's own.
fn inside(p: Point, ring: &[Point]) -> bool {
    let mut odd = false;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if p.x < x {
                odd = !odd;
            }
        }
    }
    odd
}

#[derive(Debug, Clone)]
enum Shape_ {
    Rect(f64, f64, f64, f64),
    Ring(f64, f64, f64, f64),
    Circle(f64, f64, f64),
    Line(f64, f64, f64, f64),
}

fn shape() -> impl Strategy<Value = Shape_> {
    prop_oneof![
        (10.0f64..200.0, 10.0f64..200.0, 2.0f64..80.0, 2.0f64..80.0)
            .prop_map(|(x, y, w, h)| Shape_::Rect(x, y, w, h)),
        (10.0f64..200.0, 10.0f64..200.0, 12.0f64..80.0, 12.0f64..80.0)
            .prop_map(|(x, y, w, h)| Shape_::Ring(x, y, w, h)),
        (30.0f64..240.0, 30.0f64..240.0, 1.0f64..25.0)
            .prop_map(|(x, y, r)| Shape_::Circle(x, y, r)),
        (
            10.0f64..280.0,
            10.0f64..280.0,
            10.0f64..280.0,
            10.0f64..280.0
        )
            .prop_map(|(a, b, c, d)| Shape_::Line(a, b, c, d)),
    ]
}

fn path_of(s: &Shape_) -> BezPath {
    match *s {
        Shape_::Rect(x, y, w, h) => rect_path(x, y, w, h),
        Shape_::Ring(x, y, w, h) => {
            let mut p = rect_path(x, y, w, h);
            p.extend(rect_path(x + 4.0, y + 4.0, w - 8.0, h - 8.0));
            p
        }
        Shape_::Circle(x, y, r) => Circle::new((x, y), r).to_path(0.05),
        Shape_::Line(a, b, c, d) => {
            let mut p = BezPath::new();
            p.move_to((a, b));
            p.line_to((c, d));
            p
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Weed lines and boxes never come within 1 mm of a design cut and
    /// never cut inside a design ring.
    #[test]
    fn weeds_keep_clear_of_the_design(
        shapes in proptest::collection::vec(shape(), 1..8),
        spacing in 5.0f64..30.0,
        margin in 1.0f64..10.0,
        per_object in any::<bool>(),
        monotonic_feed in any::<bool>(),
    ) {
        let input: Vec<PlacedPath> = shapes
            .iter()
            .enumerate()
            .map(|(i, s)| placed(i as u64 + 1, path_of(s)))
            .collect();
        let s = PipelineSettings {
            monotonic_feed,
            ..settings(WeedSettings {
                weed_box: true,
                margin_mm: margin,
                per_object,
                lines: WeedLines::Both,
                spacing_mm: spacing,
            })
        };
        let out = plan(&input, &s).unwrap();
        let design: Vec<&Toolpath> = out
            .toolpaths
            .iter()
            .filter(|t| t.kind == ToolpathKind::Cut)
            .collect();
        let segs: Vec<(Point, Point)> = design
            .iter()
            .flat_map(|t| t.points.windows(2).map(|w| (w[0], w[1])))
            .collect();
        let rings: Vec<&[Point]> = design.iter().filter(|t| t.closed).map(|t| t.lap()).collect();
        for w in weeds(&out) {
            for e in w.lap().windows(2) {
                for &(a, b) in &segs {
                    let d = seg_distance(e[0], e[1], a, b);
                    prop_assert!(
                        d >= WEED_CLEARANCE_MM - 1e-9,
                        "weed {:?} is {d} mm from a design cut", w.source
                    );
                }
                if !w.closed {
                    // Clear of every cut, so the whole piece lies on the
                    // side of its midpoint.
                    let mid = e[0].midpoint(e[1]);
                    prop_assert!(
                        rings.iter().all(|r| !inside(mid, r)),
                        "weed line at {mid:?} cuts inside a design ring"
                    );
                }
            }
        }
    }
}
