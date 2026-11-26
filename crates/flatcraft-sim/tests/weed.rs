//! Weed box and weed lines (Task 51) judged by simulation: the blade of a
//! weed path never cuts into kept material (host knife compensation:
//! max_into_kept_mm is 0; tip mode: no worse than without weeds), on a
//! mat and without one.

use flatcraft_core::document::{ObjectId, WeedLines, WeedSettings};
use flatcraft_path::{
    Compensation, HostSettings, PathRef, PipelineSettings, PlacedPath, ToolpathKind, plan,
};
use flatcraft_sim::{SimSettings, measure, simulate};
use kurbo::{BezPath, Circle, Point, Rect, Shape};

const D: f64 = 0.3;

fn placed(object: u64, path: BezPath) -> PlacedPath {
    PlacedPath {
        path,
        source: PathRef {
            object: ObjectId(object),
            path_index: 0,
        },
    }
}

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    let mut b = BezPath::new();
    b.move_to((x, y));
    b.line_to((x + w, y));
    b.line_to((x + w, y + h));
    b.line_to((x, y + h));
    b.close_path();
    b
}

/// Labels with letters in them: rings with counters, circles, an open
/// score line, spread over three feed bands.
fn layout() -> Vec<PlacedPath> {
    let mut o = rect_path(20.0, 20.0, 40.0, 30.0);
    o.extend(rect_path(28.0, 28.0, 24.0, 14.0));
    let mut score = BezPath::new();
    score.move_to((80.0, 120.0));
    score.line_to((140.0, 130.0));
    vec![
        placed(1, o),
        placed(2, Circle::new((110.0, 40.0), 18.0).to_path(0.01)),
        placed(3, rect_path(30.0, 80.0, 60.0, 25.0)),
        placed(4, score),
        placed(5, Circle::new((60.0, 150.0), 8.0).to_path(0.01)),
    ]
}

#[test]
fn weed_paths_never_cut_into_kept_material() {
    let weed = WeedSettings {
        weed_box: true,
        margin_mm: 3.0,
        per_object: false,
        lines: WeedLines::Both,
        spacing_mm: 10.0,
    };
    let base = PipelineSettings {
        weed,
        ..PipelineSettings::with_defaults(Rect::new(0.0, 0.0, 300.0, 300.0), Point::ZERO)
    };
    let host = Compensation::Host(HostSettings::new(D, 0.0));
    // Each switch is on in some case and off in another: a mat and no mat,
    // one box and a box per object, host and tip compensation.
    for (compensation, monotonic_feed, per_object) in [
        (host, false, false),
        (host, true, true),
        (host, true, false),
        (Compensation::Tip, false, true),
    ] {
        let s = PipelineSettings {
            compensation,
            monotonic_feed,
            weed: WeedSettings { per_object, ..weed },
            ..base.clone()
        };
        let out = plan(&layout(), &s).unwrap();
        let weeds = out
            .toolpaths
            .iter()
            .filter(|t| t.kind == ToolpathKind::Weed)
            .count();
        assert!(weeds > 3, "{weeds} weed paths");
        let sim = SimSettings::float(D);
        let trace = simulate(&out.toolpaths, out.home, &sim).unwrap();
        let (m, _) = measure(&out.toolpaths, &trace, &sim);
        let what = format!(
            "{compensation:?}, no mat {monotonic_feed}, per object {per_object}: {:?}",
            m.worst_spots
        );
        match compensation {
            Compensation::Host(_) => assert_eq!(m.max_into_kept_mm, 0.0, "{what}"),
            // Tip mode leaves the drag to the firmware, so the
            // simulator already sees the design's own drop
            // points swing in. Weeds must add nothing to that.
            Compensation::Tip => {
                let bare = PipelineSettings {
                    weed: WeedSettings::default(),
                    ..s.clone()
                };
                let out = plan(&layout(), &bare).unwrap();
                let trace = simulate(&out.toolpaths, out.home, &sim).unwrap();
                let (without, _) = measure(&out.toolpaths, &trace, &sim);
                assert!(
                    m.max_into_kept_mm <= without.max_into_kept_mm,
                    "{what}: {} with weeds, {} without",
                    m.max_into_kept_mm,
                    without.max_into_kept_mm
                );
            }
        }
    }
}
