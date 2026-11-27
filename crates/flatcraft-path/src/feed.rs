//! Banded monotonic feed (Task 7, R8).
//!
//! Without a mat the sheet is held by the rollers and the feed motor moves
//! it back and forth. Long moves against the feed direction (y) wear the
//! grip and drift the sheet. With feed bands the cut order finishes a strip
//! of the sheet before it starts the next one, so the head never goes back
//! more than a band and one path.
//!
//! A path belongs to the band of its **maximum** y. A child lies inside its
//! parent, so its maximum y is never larger and it lands in the same or an
//! earlier band: every child still comes before its parent. The minimum y
//! would not give that.
//!
//! The bound is for the ordered paths: the head goes back at most a band
//! plus the tallest path. Overcut can add up to the overcut length (0.5 mm
//! by default) on top, and passes repeat a path in place, so they add no
//! backtrack.
//!
//! With the OutlinesLast strategy bands win over the outlines-last rule. A hole
//! whose outer ring lies in a later band is cut as an outline of its own
//! band: last in that band, not early.

use crate::containment::ContainmentTree;
use crate::plan::Toolpath;

/// The height of one feed band, in millimetres.
pub const BAND_MM: f64 = 50.0;

/// The feed band of `t`: floor of its largest y over [`BAND_MM`]. Paths
/// above y = 0 (and any not finite) are in band 0.
pub fn band_of(t: &Toolpath) -> usize {
    let max_y = t
        .points
        .iter()
        .map(|p| p.y)
        .filter(|y| y.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    if max_y.is_finite() && max_y > 0.0 {
        (max_y / BAND_MM).floor() as usize
    } else {
        0
    }
}

/// The band of every path. A path never lies in a later band than a ring
/// around it: a child that pokes a hair out of its parent's band (a
/// tolerance, an overcut) moves to the parent's band instead of after it.
pub fn bands(paths: &[Toolpath], tree: &ContainmentTree) -> Vec<usize> {
    let mut band: Vec<usize> = paths.iter().map(band_of).collect();
    let own = band.clone();
    for u in 0..band.len() {
        let mut at = tree.parent.get(u).copied().flatten();
        // The tree is a forest; the bound only guards a malformed input.
        let mut guard = band.len();
        while let Some(p) = at.filter(|_| guard > 0) {
            band[u] = band[u].min(own[p]);
            at = tree.parent.get(p).copied().flatten();
            guard -= 1;
        }
    }
    band
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::containment::{self, KeepMode};
    use crate::order::order;
    use crate::plan::{PathRef, PipelineSettings};
    use crate::strategy::StrategySettings;
    use flatcraft_core::document::{CutStrategy, ObjectId};
    use kurbo::{Point, Rect};

    fn ring(id: u64, x0: f64, y0: f64, x1: f64, y1: f64) -> Toolpath {
        Toolpath::new(
            [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
                .iter()
                .map(|&p| p.into())
                .collect(),
            true,
            PathRef {
                object: ObjectId(id),
                path_index: 0,
            },
        )
    }

    fn settings(strategy: CutStrategy, monotonic_feed: bool) -> PipelineSettings {
        PipelineSettings {
            monotonic_feed,
            strategy: StrategySettings::for_strategy(strategy),
            ..PipelineSettings::with_defaults(
                Rect::new(-1000.0, -1000.0, 1000.0, 1000.0),
                Point::ZERO,
            )
        }
    }

    fn run(paths: Vec<Toolpath>, s: &PipelineSettings) -> Vec<Toolpath> {
        let tree = containment::build(&paths);
        let mut paths = paths;
        for (i, t) in paths.iter_mut().enumerate() {
            t.keep = containment::keep_side(&tree, i, t.closed, KeepMode::Pieces);
        }
        order(paths, &tree, s.home, s)
    }

    fn ids(out: &[Toolpath]) -> Vec<u64> {
        out.iter().map(|t| t.source.object.0).collect()
    }

    /// A deterministic generator (an LCG), so every runner sees one layout.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self, lo: f64, hi: f64) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            lo + ((self.0 >> 11) as f64 / (1u64 << 53) as f64) * (hi - lo)
        }
    }

    /// Random pieces over 0..300 x 0..400 mm, a third of them with a hole.
    fn layout(seed: u64) -> Vec<Toolpath> {
        let mut rng = Lcg(seed);
        let mut paths = Vec::new();
        for i in 0..60u64 {
            let (x, y) = (rng.next(0.0, 260.0), rng.next(0.0, 360.0));
            let (w, h) = (rng.next(6.0, 40.0), rng.next(6.0, 40.0));
            paths.push(ring(2 * i + 1, x, y, x + w, y + h));
            if i % 3 == 0 {
                paths.push(ring(2 * i + 2, x + 2.0, y + 2.0, x + w - 2.0, y + h - 2.0));
            }
        }
        paths
    }

    /// The largest drop of the head against the feed direction: how far
    /// below the farthest y it has been so far it goes, over every travel
    /// and cut vertex (both are straight between vertices).
    fn backtrack(out: &[Toolpath], start: Point) -> f64 {
        let (mut top, mut worst) = (start.y, 0.0f64);
        for p in out.iter().flat_map(|t| t.points.iter()) {
            top = top.max(p.y);
            worst = worst.max(top - p.y);
        }
        worst
    }

    fn tallest(paths: &[Toolpath]) -> f64 {
        paths
            .iter()
            .map(|t| {
                let ys = t.points.iter().map(|p| p.y);
                ys.clone().fold(f64::MIN, f64::max) - ys.fold(f64::MAX, f64::min)
            })
            .fold(0.0, f64::max)
    }

    #[test]
    fn band_budgets_add_up_and_no_real_band_gets_zero() {
        use crate::order::band_budgets;
        for budget in [4_000_000u64, 10, 3] {
            let sizes = [1usize, 2, 1, 50, 3, 2];
            let b = band_budgets(budget, &sizes);
            assert_eq!(b.len(), sizes.len());
            if budget >= 1000 {
                assert_eq!(b.iter().sum::<u64>(), budget);
            }
            for (n, share) in sizes.iter().zip(&b) {
                assert!(*n < 2 || *share >= 1, "budget {budget}: {sizes:?} {b:?}");
            }
        }
    }

    #[test]
    fn band_of_uses_the_maximum_y() {
        assert_eq!(band_of(&ring(1, 0.0, 10.0, 5.0, 49.0)), 0);
        assert_eq!(band_of(&ring(1, 0.0, 10.0, 5.0, 50.0)), 1);
        assert_eq!(band_of(&ring(1, 0.0, 10.0, 5.0, 120.0)), 2);
        assert_eq!(band_of(&ring(1, 0.0, -30.0, 5.0, -10.0)), 0);
    }

    /// Measures the head, not the band rule: over the whole head path
    /// (travel and cut) it never goes back more than a band plus the
    /// tallest path.
    #[test]
    fn head_feed_backtrack_is_bounded() {
        for seed in [1, 7, 42] {
            let paths = layout(seed);
            let bound = BAND_MM + tallest(&paths);
            let s = settings(CutStrategy::Quality, true);
            let out = run(paths.clone(), &s);
            assert_eq!(out.len(), paths.len());
            let banded = backtrack(&out, s.home);
            assert!(banded <= bound, "seed {seed}: {banded} > {bound}");
            // The measure can tell: without bands the head goes back more.
            let plain = run(paths, &settings(CutStrategy::Quality, false));
            assert!(
                backtrack(&plain, s.home) > bound,
                "seed {seed}: the layout does not need bands"
            );
        }
    }

    #[test]
    fn children_still_come_first_with_bands() {
        for seed in [3, 9] {
            let paths = layout(seed);
            let tree = containment::build(&paths);
            for strategy in [CutStrategy::Quality, CutStrategy::OutlinesLast] {
                let out = run(paths.clone(), &settings(strategy, true));
                let at = |id: u64| ids(&out).iter().position(|&x| x == id).unwrap();
                for (c, parent) in tree.parent.iter().enumerate() {
                    if let Some(p) = parent {
                        let (cid, pid) = (paths[c].source.object.0, paths[*p].source.object.0);
                        assert!(
                            at(cid) < at(pid),
                            "seed {seed}: {cid} after its parent {pid}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_child_above_its_parents_band_moves_to_it() {
        // The hole pokes 1 mm over the band edge of its ring (a tree built by
        // hand: real geometry cannot do that, a tolerance can).
        let paths = vec![
            ring(1, 0.0, 10.0, 40.0, 49.5),
            ring(2, 5.0, 12.0, 30.0, 50.5),
        ];
        let tree = ContainmentTree {
            parent: vec![None, Some(0)],
            children: vec![vec![1], vec![]],
            roots: vec![0],
            depth: vec![0, 1],
            own_depth: vec![0, 1],
        };
        assert_eq!(band_of(&paths[1]), 1);
        assert_eq!(bands(&paths, &tree), vec![0, 0]);
    }

    #[test]
    fn outlines_last_with_bands_orders_per_band() {
        // Band 0: two pieces with a hole each; band 1: one piece with a hole.
        // Ids: pieces 1, 2, 5; holes 3, 4, 6.
        let paths = vec![
            ring(1, 0.0, 0.0, 30.0, 30.0),
            ring(2, 100.0, 0.0, 130.0, 30.0),
            ring(3, 5.0, 5.0, 10.0, 10.0),
            ring(4, 105.0, 5.0, 110.0, 10.0),
            ring(5, 0.0, 60.0, 30.0, 90.0),
            ring(6, 5.0, 65.0, 10.0, 70.0),
        ];
        let out = run(paths, &settings(CutStrategy::OutlinesLast, true));
        let order = ids(&out);
        let at = |id: u64| order.iter().position(|&x| x == id).unwrap();
        for (band0, band1) in [(1, 5), (2, 6), (3, 5), (4, 6), (1, 6), (2, 5)] {
            assert!(at(band0) < at(band1), "{order:?}: bands win");
        }
        for (hole, piece) in [(3, 1), (3, 2), (4, 1), (4, 2), (6, 5)] {
            assert!(at(hole) < at(piece), "{order:?}: outlines last in a band");
        }
    }
}
