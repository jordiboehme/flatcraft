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

/// At most about `max_points` samples of `r`, for drawing. Kept always,
/// whatever `max_points` says: the first and the last sample, every run
/// vertex (where the axis turns, and where the blade, the toolpath, the
/// run or the intended segment changes), every bad sample (deviation above
/// [`END_TO_END_LIMIT_MM`]), every pushed sample and both samples at each
/// edge between waste and path moves. So a job with more of these than
/// `max_points` gets them all, and more points than asked. The other
/// samples are thinned evenly to fill the rest.
pub fn decimate(r: &SimResult, max_points: usize) -> Vec<DisplaySample> {
    let samples = &r.trace.samples;
    let n = samples.len();
    if n == 0 {
        return Vec::new();
    }
    let waste = |i: usize| samples[i].down && samples[i].seg == WASTE_SEG;
    let deviation = |i: usize| r.deviation_mm.get(i).copied().unwrap_or(f32::NAN);
    let keep: Vec<bool> = (0..n)
        .map(|i| {
            if i == 0 || i + 1 == n {
                return true;
            }
            let (a, x, b) = (&samples[i - 1], &samples[i], &samples[i + 1]);
            let bad = f64::from(deviation(i)) > END_TO_END_LIMIT_MM;
            let changes =
                x.down != b.down || x.toolpath != b.toolpath || x.run != b.run || x.seg != b.seg;
            let edge = waste(i) != waste(i - 1) || waste(i) != waste(i + 1);
            let (u, v) = (x.axis - a.axis, b.axis - x.axis);
            let turns = {
                let (lu, lv) = (u.hypot(), v.hypot());
                lu > 0.0
                    && lv > 0.0
                    && ((u.cross(v) / (lu * lv)).abs() > STRAIGHT || u.dot(v) < 0.0)
            };
            bad || x.pushed || changes || edge || turns
        })
        .collect();
    let required = keep.iter().filter(|k| **k).count();
    let others = n - required;
    let budget = max_points.saturating_sub(required);
    // Every `stride`-th other sample: `others / stride <= budget`.
    let stride = if budget == 0 {
        usize::MAX
    } else {
        others.div_ceil(budget).max(1)
    };
    let mut seen_others = 0usize;
    (0..n)
        .filter(|&i| {
            if keep[i] {
                return true;
            }
            seen_others += 1;
            stride != usize::MAX && seen_others.is_multiple_of(stride)
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
        let bad = |s: &DisplaySample| f64::from(s.deviation_mm) > END_TO_END_LIMIT_MM;
        let all_bad = r
            .deviation_mm
            .iter()
            .filter(|d| f64::from(**d) > END_TO_END_LIMIT_MM)
            .count();
        assert!(all_bad > 0, "an uncompensated square has bad samples");
        let all_pushed = r.trace.samples.iter().filter(|s| s.pushed).count();
        assert!(all_pushed > 0, "the line that doubles back pushes");
        let shown = decimate(&r, 200);
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
        let all_bad = r
            .deviation_mm
            .iter()
            .filter(|d| f64::from(**d) > END_TO_END_LIMIT_MM)
            .count();
        let bad = shown
            .iter()
            .filter(|s| f64::from(s.deviation_mm) > END_TO_END_LIMIT_MM)
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
