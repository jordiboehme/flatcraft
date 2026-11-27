//! Task 16: structural properties of the whole pipeline on random layouts
//! (`support/shapes.rs`): star-shaped polygons, nestings of them and open
//! polylines, with random passes, overcut, strategies and compensation.
//! Every property here is structural: it checks the planned points
//! against the construction rules. `flatcraft-sim/tests/properties.rs`
//! simulates the same layouts.

#[path = "support/shapes.rs"]
mod shapes;

use flatcraft_core::document::CutStrategy;
use flatcraft_path::clean::length;
use flatcraft_path::containment::signed_area;
use flatcraft_path::{
    Compensation, CutPlan, HostSettings, PathRef, PipelineSettings, StrategySettings, Toolpath,
    ToolpathKind, WASTE_SEG, plan,
};
use kurbo::{Point, Rect, Vec2};
use proptest::prelude::*;

use shapes::Built;

/// The cutting area of the uncalibrated 12x12 mat: its grid, inset by 5 mm.
fn area() -> Rect {
    Rect::new(5.0, 5.0, 299.8, 299.8)
}

/// How much the plan's collinear merge (at the 0.01 mm flatten
/// tolerance) may shorten one path: it drops vertices within the
/// tolerance of the line through their neighbours.
const MERGE_MM: f64 = 1e-3;

const STRATEGIES: [CutStrategy; 3] = [
    CutStrategy::Quality,
    CutStrategy::Fast,
    CutStrategy::OutlinesLast,
];

#[derive(Debug, Clone, Copy)]
struct Knobs {
    overcut_mm: f64,
    passes: u32,
    chain: bool,
    reverse: bool,
    strategy: CutStrategy,
    host: Option<f64>,
}

fn knobs() -> impl Strategy<Value = Knobs> {
    (
        0.0f64..2.0,
        1u32..=3,
        any::<bool>(),
        any::<bool>(),
        0usize..3,
        proptest::option::of(0.1f64..0.5),
    )
        .prop_map(|(overcut_mm, passes, chain, reverse, s, host)| Knobs {
            overcut_mm,
            passes,
            chain,
            reverse,
            strategy: STRATEGIES[s],
            host,
        })
}

fn settings(k: Knobs) -> PipelineSettings {
    PipelineSettings {
        overcut_mm: k.overcut_mm,
        passes: k.passes,
        chain_closed_passes: k.chain,
        reverse_alternate_passes: k.reverse,
        strategy: StrategySettings::for_strategy(k.strategy),
        compensation: match k.host {
            Some(d) => Compensation::Host(HostSettings::new(d, 0.0)),
            None => Compensation::Tip,
        },
        ..PipelineSettings::with_defaults(area(), Point::ZERO)
    }
}

fn planned(built: &[Built], k: Knobs) -> CutPlan {
    plan(&shapes::placed(built), &settings(k)).expect("the layout plans")
}

fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    if len2 == 0.0 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + t * ab)
}

fn polyline_distance(p: Point, line: &[Point]) -> f64 {
    line.windows(2)
        .map(|w| segment_distance(p, w[0], w[1]))
        .fold(f64::INFINITY, f64::min)
}

/// The length two segments share when they lie on one line, else 0.
fn overlap(a: (Point, Point), b: (Point, Point)) -> f64 {
    let u = a.1 - a.0;
    let len = u.hypot();
    if len == 0.0 || (b.1 - b.0).hypot() == 0.0 {
        return 0.0;
    }
    let u = u / len;
    let off = |q: Point| u.cross(q - a.0).abs();
    if off(b.0) > 1e-7 || off(b.1) > 1e-7 {
        return 0.0;
    }
    let (s, t) = (u.dot(b.0 - a.0), u.dot(b.1 - a.0));
    (s.max(t).min(len) - s.min(t).max(0.0)).max(0.0)
}

fn cuts(out: &CutPlan) -> impl Iterator<Item = &Toolpath> {
    out.toolpaths.iter().filter(|t| t.kind == ToolpathKind::Cut)
}

fn of(out: &CutPlan, source: PathRef) -> impl Iterator<Item = &Toolpath> {
    cuts(out).filter(move |t| t.source == source)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Structural: every closed ring's first lap is a closed ring turning
    /// counter-clockwise (clockwise on a reversed pass), every point of the toolpath lies on that ring,
    /// and the toolpath is its laps plus exactly the overcut (at most one
    /// lap of it). Every ring is cut `passes` times. Open paths get no
    /// overcut.
    #[test]
    fn every_closed_ring_is_closed_and_overcut(
        layout in shapes::layout(6),
        k in knobs(),
    ) {
        let built = shapes::build(&layout);
        let out = planned(&built, k);
        for b in &built {
            let source = b.placed.source;
            let laps: u32 = of(&out, source).map(|t| t.laps).sum();
            prop_assert_eq!(laps, k.passes, "{:?}", source);
            for t in of(&out, source) {
                prop_assert_eq!(t.closed, b.closed);
                let lap = t.lap();
                let len = length(lap);
                prop_assert!(
                    (len - length(&b.points)).abs() <= MERGE_MM,
                    "{:?}: lap {} vs path {}", source, len, length(&b.points)
                );
                if !t.closed {
                    prop_assert_eq!(t.points.as_slice(), lap);
                    continue;
                }
                prop_assert!(lap.len() >= 4);
                prop_assert!(lap[0].distance(*lap.last().unwrap()) < 1e-9);
                // Reversed passes run the other way on purpose.
                let reversed = k.reverse && t.pass % 2 == 0;
                prop_assert_eq!(signed_area(lap) < 0.0, !reversed, "{:?} pass {}", source, t.pass);
                let want = f64::from(t.laps) * len + k.overcut_mm.min(len);
                prop_assert!(
                    (length(&t.points) - want).abs() < 1e-6,
                    "{:?}: {} vs {}", source, length(&t.points), want
                );
                for &q in &t.points {
                    prop_assert!(polyline_distance(q, lap) < 1e-9, "{:?}: {:?} off the ring", source, q);
                }
            }
        }
    }

    /// Structural: the first passes of all `Cut` toolpaths cut each input
    /// path once: no two of their segments overlap, and each input path's
    /// first-pass laps are as long as the path. What is cut beyond that is
    /// only the overcut and the further passes
    /// (`every_closed_ring_is_closed_and_overcut`).
    #[test]
    fn nothing_is_cut_twice_except_overcut_and_passes(
        layout in shapes::layout(6),
        k in knobs(),
    ) {
        let built = shapes::build(&layout);
        let out = planned(&built, k);
        for b in &built {
            let first: f64 = of(&out, b.placed.source)
                .filter(|t| t.pass == 1)
                .map(|t| length(t.lap()))
                .sum();
            prop_assert!((first - length(&b.points)).abs() <= MERGE_MM, "{:?}", b.placed.source);
        }
        let segs: Vec<(Point, Point, usize)> = cuts(&out)
            .enumerate()
            .filter(|(_, t)| t.pass == 1)
            .flat_map(|(i, t)| t.lap().windows(2).map(move |w| (w[0], w[1], i)).collect::<Vec<_>>())
            .collect();
        for (x, a) in segs.iter().enumerate() {
            for b in &segs[x + 1..] {
                let shared = overlap((a.0, a.1), (b.0, b.1));
                prop_assert!(shared < 1e-6, "{:?} and {:?} share {}", a, b, shared);
            }
        }
    }

    /// Structural: every path held by a closed ring (a hole, an island, a
    /// line inside) is cut, all its passes, before any pass of that ring,
    /// with every strategy (the generator knows the true nesting).
    #[test]
    fn children_come_before_parents(
        layout in shapes::layout(6),
        k in knobs(),
    ) {
        let built = shapes::build(&layout);
        let out = planned(&built, k);
        let at = |s: PathRef| -> Vec<usize> {
            out.toolpaths
                .iter()
                .enumerate()
                .filter(|(_, t)| t.kind == ToolpathKind::Cut && t.source == s)
                .map(|(i, _)| i)
                .collect()
        };
        for b in &built {
            let Some(parent) = b.parent else { continue };
            let (child, parent_at) = (at(b.placed.source), at(parent));
            prop_assert!(!child.is_empty() && !parent_at.is_empty());
            prop_assert!(
                child.iter().max() < parent_at.iter().min(),
                "{:?} at {:?} after its parent at {:?}", b.placed.source, child, parent_at
            );
        }
    }

    /// Structural: with host compensation (pivot at every corner, exact
    /// floats) every axis point that cuts segment `i` of the intended path
    /// lies on that segment shifted forward by the offset d along its
    /// direction, or on the pivot arc of radius d round its end vertex.
    #[test]
    fn host_axis_tracks_offset_on_straights(
        layout in shapes::layout(6),
        d in 0.1f64..0.5,
        k in knobs(),
    ) {
        let k = Knobs { host: Some(d), ..k };
        let built = shapes::build(&layout);
        let out = planned(&built, k);
        let mut on_straights = 0usize;
        for t in cuts(&out) {
            let runs = t.axis.as_ref().expect("host compensation gives axis runs");
            for r in runs {
                prop_assert_eq!(r.points.len(), r.seg.len());
                for (q, &i) in r.points.iter().zip(&r.seg) {
                    if i == WASTE_SEG {
                        continue;
                    }
                    let (a, b) = (t.points[i as usize], t.points[i as usize + 1]);
                    let dir: Vec2 = (b - a).normalize();
                    let straight = segment_distance(*q, a + d * dir, b + d * dir);
                    let arc = (q.distance(b) - d).abs();
                    if straight < 1e-9 {
                        on_straights += 1;
                    }
                    prop_assert!(
                        straight < 1e-9 || arc < 1e-9,
                        "seg {} of {:?}: {:?} is {} off the shifted segment, {} off the arc",
                        i, t.source, q, straight, arc
                    );
                }
            }
        }
        prop_assert!(on_straights > 0);
    }
}
