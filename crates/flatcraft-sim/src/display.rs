//! A thinned copy of a simulated trace for drawing: a window or a preview
//! image has no use for millions of samples, but it must never lose the
//! places worth a look.

use flatcraft_path::WASTE_SEG;
use kurbo::Point;

use crate::{END_TO_END_LIMIT_MM, SimResult};

/// One sample as a view draws it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplaySample {
    pub axis: Point,
    pub tip: Point,
    /// The blade is down during the move into this sample.
    pub down: bool,
    /// Axis travel from the home to this sample, mm.
    pub t_mm: f64,
    /// The sample's deviation (see [`crate::measure`]); NaN while the
    /// blade is up.
    pub deviation_mm: f32,
    /// The index of the simulated toolpath.
    pub toolpath: usize,
    /// A blade-down waste move: a swivel, a lead-in, an overshoot or the
    /// drop point.
    pub waste: bool,
    /// The move into this sample pushes the blade from behind.
    pub pushed: bool,
}

/// Directions closer than this (as the sine of the angle between them) are
/// one straight move, so a sample between them is not a vertex.
const STRAIGHT: f64 = 1e-9;

/// At most about `max_points` samples of `r`, for drawing.
///
/// Kept always, whatever `max_points` says, are only the real problems: a
/// path sample farther than [`END_TO_END_LIMIT_MM`] from its line, a
/// pushed sample and the sample of every worst spot (a waste move into
/// kept material is one of them), plus the first and the last sample.
/// Only a job with more problems than `max_points` gets more points than
/// asked. The budget left is then spent in this order, each group thinned
/// evenly once it no longer fits:
///
/// 1. where the blade, the toolpath or the run changes (so travel is never
///    drawn as a cut),
/// 2. run vertices (where the axis turns or the intended segment changes)
///    and the edges between waste and path moves,
/// 3. every other sample.
///
/// A waste move's deviation (its distance to the nearest line) is not a
/// problem by itself: it lies in waste on purpose.
pub fn decimate(r: &SimResult, max_points: usize) -> Vec<DisplaySample> {
    let samples = &r.trace.samples;
    let n = samples.len();
    if n == 0 {
        return Vec::new();
    }
    let waste = |i: usize| samples[i].down && samples[i].seg == WASTE_SEG;
    let deviation = |i: usize| r.deviation_mm.get(i).copied().unwrap_or(f32::NAN);
    let spots: Vec<Point> = r.metrics.worst_spots.iter().map(|s| s.at).collect();
    // 0: must keep, 1 to 3: the groups of the doc comment.
    let tier: Vec<u8> = (0..n)
        .map(|i| {
            if i == 0 || i + 1 == n {
                return 0;
            }
            let (a, x, b) = (&samples[i - 1], &samples[i], &samples[i + 1]);
            let path = x.down && !waste(i);
            let bad = path && f64::from(deviation(i)) > END_TO_END_LIMIT_MM;
            let spot = spots.iter().any(|p| p.distance(x.tip) < 1e-9);
            if bad || x.pushed || spot {
                return 0;
            }
            if x.down != b.down || x.toolpath != b.toolpath || x.run != b.run {
                return 1;
            }
            let edge = waste(i) != waste(i - 1) || waste(i) != waste(i + 1);
            let (u, v) = (x.axis - a.axis, b.axis - x.axis);
            let turns = {
                let (lu, lv) = (u.hypot(), v.hypot());
                lu > 0.0
                    && lv > 0.0
                    && ((u.cross(v) / (lu * lv)).abs() > STRAIGHT || u.dot(v) < 0.0)
            };
            if x.seg != b.seg || edge || turns {
                2
            } else {
                3
            }
        })
        .collect();
    let mut count = [0usize; 4];
    for &t in &tier {
        count[usize::from(t)] += 1;
    }
    // Per group: how many of its samples are kept, spread evenly.
    let mut take = [0usize; 4];
    take[0] = count[0];
    let mut budget = max_points.saturating_sub(count[0]);
    for t in 1..4 {
        take[t] = count[t].min(budget);
        budget -= take[t];
    }
    let mut seen = [0usize; 4];
    (0..n)
        .filter(|&i| {
            let t = usize::from(tier[i]);
            let k = seen[t];
            seen[t] += 1;
            // The k-th of `count` is kept when the even spread of `take`
            // steps over it: exactly `take` of them, the last one always.
            (k + 1) * take[t] / count[t].max(1) > k * take[t] / count[t].max(1)
        })
        .map(|i| {
            let x = &samples[i];
            DisplaySample {
                axis: x.axis,
                tip: x.tip,
                down: x.down,
                t_mm: x.t_mm,
                deviation_mm: deviation(i),
                toolpath: x.toolpath,
                waste: waste(i),
                pushed: x.pushed,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SimSettings, TimeModel, run};
    use flatcraft_path::{PathRef, Toolpath};

    const SOURCE: PathRef = PathRef {
        object: flatcraft_core::document::ObjectId(1),
        path_index: 0,
    };

    fn square(x: f64, y: f64, side: f64) -> Toolpath {
        let p = |a: f64, b: f64| Point::new(x + a, y + b);
        Toolpath::new(
            vec![
                p(0.0, 0.0),
                p(side, 0.0),
                p(side, side),
                p(0.0, side),
                p(0.0, 0.0),
            ],
            true,
            SOURCE,
        )
    }

    /// Path samples (blade down, not waste) farther than the limit.
    fn path_bad(r: &SimResult) -> usize {
        r.trace
            .samples
            .iter()
            .zip(&r.deviation_mm)
            .filter(|(x, d)| x.down && x.seg != WASTE_SEG && f64::from(**d) > END_TO_END_LIMIT_MM)
            .count()
    }

    #[test]
    fn decimate_keeps_bad_and_pushed_samples() {
        // An uncompensated square: the tip cuts the corners (bad samples)
        // and the start is off the line.
        // A line that doubles back pushes the blade.
        let back = Toolpath::new(
            vec![
                Point::new(120.0, 10.0),
                Point::new(140.0, 10.0),
                Point::new(125.0, 10.0),
            ],
            false,
            SOURCE,
        );
        let paths = vec![square(10.0, 10.0, 50.0), square(80.0, 10.0, 20.0), back];
        let r = run(
            &paths,
            Point::ZERO,
            &SimSettings::float(0.3),
            5,
            &TimeModel::PROVISIONAL,
        )
        .unwrap();
        let n = r.trace.samples.len();
        let bad = |s: &DisplaySample| {
            s.down && !s.waste && f64::from(s.deviation_mm) > END_TO_END_LIMIT_MM
        };
        let all_bad = path_bad(&r);
        assert!(all_bad > 0, "an uncompensated square has bad samples");
        let all_pushed = r.trace.samples.iter().filter(|s| s.pushed).count();
        assert!(all_pushed > 0, "the line that doubles back pushes");
        // The uncompensated corners give about 1250 bad samples: a
        // budget above that keeps the vertices too.
        let shown = decimate(&r, 2000);
        assert!(shown.len() < n, "{} of {n}", shown.len());
        assert_eq!(shown.iter().filter(|s| bad(s)).count(), all_bad);
        assert_eq!(shown.iter().filter(|s| s.pushed).count(), all_pushed);
        // the corners and both ends are there
        assert_eq!(shown.first().unwrap().axis, r.trace.samples[0].axis);
        assert_eq!(shown.last().unwrap().axis, r.trace.samples[n - 1].axis);
        for t in &paths {
            for q in &t.points {
                assert!(
                    shown.iter().any(|s| s.axis == *q && s.down),
                    "vertex {q:?} kept"
                );
            }
        }
        // the t_mm order is the trace order
        assert!(shown.windows(2).all(|w| w[0].t_mm <= w[1].t_mm));
        // a large budget keeps everything
        assert_eq!(decimate(&r, n + 10).len(), n);
    }

    #[test]
    fn decimate_keeps_pushed_samples_even_over_budget() {
        // A tiny budget still keeps every required sample.
        let paths = vec![square(10.0, 10.0, 5.0)];
        let r = run(
            &paths,
            Point::ZERO,
            &SimSettings::float(0.3),
            5,
            &TimeModel::PROVISIONAL,
        )
        .unwrap();
        let shown = decimate(&r, 1);
        let all_bad = path_bad(&r);
        let bad = shown
            .iter()
            .filter(|s| s.down && !s.waste && f64::from(s.deviation_mm) > END_TO_END_LIMIT_MM)
            .count();
        assert_eq!(bad, all_bad);
        assert!(shown.len() > 1);
        assert!(
            decimate(
                &SimResult {
                    trace: Default::default(),
                    deviation_mm: Vec::new(),
                    metrics: r.metrics.clone(),
                    seconds: 0.0,
                },
                10
            )
            .is_empty()
        );
    }
}
