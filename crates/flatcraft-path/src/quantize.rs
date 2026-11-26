//! Rounding host-compensated axis runs to the device units.
//!
//! The device takes whole units (20 per mm on the Cameo 2), so every axis
//! point moves by up to half a unit on each axis. A pivot arc of radius d
//! is only a few units across, and rounding its points can make a move
//! that runs back at the trailing tip: a pushed blade, which tears the
//! material. Where the knife stage knows the device units
//! ([`crate::HostSettings::device_grid`]) it rounds the runs itself and
//! drops each rounded point whose move would push the blade, using the
//! drag knife model ([`crate::model`]) to follow the tip.

use kurbo::{Point, Vec2};

use crate::model::{PUSHED_DEG, drag, push_angle_deg};
use crate::plan::{ASSUMED_START_HEADING, AxisRun, Toolpath};

/// The device rounding: absolute coordinates on a grid of `1 / units_per_mm`
/// that starts at minus `origin` in app millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceGrid {
    pub units_per_mm: f64,
    /// The grid origin of the mat (app 0,0 lies at `origin` in device millimetres).
    pub origin: Point,
}

impl DeviceGrid {
    /// The emitter's rounding without mirror and scale (they only correct
    /// the machine's own error): `round((p + origin) * upm) / upm - origin`
    /// per axis.
    pub fn snap(&self, p: Point) -> Point {
        let upm = self.units_per_mm;
        Point::new(
            ((p.x + self.origin.x) * upm).round() / upm - self.origin.x,
            ((p.y + self.origin.y) * upm).round() / upm - self.origin.y,
        )
    }

    /// A positive, finite unit size and a finite origin.
    pub fn is_valid(&self) -> bool {
        self.units_per_mm.is_finite() && self.units_per_mm > 0.0 && self.origin.is_finite()
    }
}

/// Round every axis run of `paths` (in cut order, the blade starting at
/// `start_heading`) to `grid`, for a blade of offset `d`:
///
/// - Every point is rounded. A point that rounds onto the one before it is
///   dropped.
/// - A point whose rounded move would push the blade (push angle above
///   [`PUSHED_DEG`] against the tip the model predicts) is dropped, and the
///   next point is tried from the same place.
/// - The first point (the drop) and the last point (the end of the run)
///   always stay. When the step onto the last point would push, the points
///   before it are dropped, newest first, until it does not. Only a run
///   whose single step from the drop point pushes is left as it is.
///
/// The tip is followed across runs as the simulator does: blade-up travel
/// keeps the tip's offset from the axis. Toolpaths without axis runs are
/// left alone.
pub fn round_runs(paths: &mut [Toolpath], grid: &DeviceGrid, d: f64, start_heading: Vec2) {
    let h = if start_heading.hypot() > 0.0 && start_heading.hypot().is_finite() {
        start_heading.normalize()
    } else {
        ASSUMED_START_HEADING
    };
    // The axis minus the tip.
    let mut offset = d * h;
    for t in paths.iter_mut() {
        let Some(runs) = t.axis.as_mut() else {
            continue;
        };
        for run in runs.iter_mut() {
            let Some(&first) = run.points.first() else {
                continue;
            };
            let first = grid.snap(first);
            let mut out = AxisRun {
                points: vec![first],
                seg: vec![run.seg[0]],
            };
            // The tip after each point of `out`.
            let mut tips = vec![first - offset];
            let last = run.points.len() - 1;
            for (k, (&p, &seg)) in run.points.iter().zip(&run.seg).enumerate().skip(1) {
                let b = grid.snap(p);
                let pushes = |out: &AxisRun, tips: &[Point]| {
                    let (a, tip) = (out.points[out.points.len() - 1], tips[tips.len() - 1]);
                    push_angle_deg(a, b, tip) > PUSHED_DEG
                };
                if k < last {
                    if b == out.points[out.points.len() - 1] || pushes(&out, &tips) {
                        continue;
                    }
                } else {
                    // The end of the run stays: drop the points before it
                    // until the step onto it no longer pushes.
                    while out.points.len() > 1 && pushes(&out, &tips) {
                        out.points.pop();
                        out.seg.pop();
                        tips.pop();
                    }
                    if b == out.points[out.points.len() - 1] {
                        continue;
                    }
                }
                let (a, tip) = (out.points[out.points.len() - 1], tips[tips.len() - 1]);
                tips.push(drag(a, b, tip, d));
                out.points.push(b);
                out.seg.push(seg);
            }
            let (axis, tip) = (out.points[out.points.len() - 1], tips[tips.len() - 1]);
            offset = axis - tip;
            *run = out;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{PathRef, WASTE_SEG};
    use flatcraft_core::document::ObjectId;

    fn grid() -> DeviceGrid {
        DeviceGrid {
            units_per_mm: 20.0,
            origin: Point::new(0.3, 0.0),
        }
    }

    fn toolpath(points: Vec<Point>) -> Toolpath {
        let mut t = Toolpath::new(
            points.clone(),
            false,
            PathRef {
                object: ObjectId(1),
                path_index: 0,
            },
        );
        let mut seg = vec![WASTE_SEG];
        seg.extend(0..points.len() as u32 - 1);
        t.axis = Some(vec![AxisRun { points, seg }]);
        t
    }

    /// Structural: every point lands on the grid, repeats are dropped and
    /// the run keeps its first and last point.
    #[test]
    fn points_land_on_the_grid() {
        let pts = vec![
            Point::new(10.01, 10.0),
            Point::new(10.02, 10.0),
            Point::new(20.013, 10.004),
            Point::new(20.0, 20.0),
        ];
        let mut paths = vec![toolpath(pts)];
        round_runs(&mut paths, &grid(), 0.25, Vec2::new(1.0, 0.0));
        let run = &paths[0].axis.as_ref().unwrap()[0];
        for q in &run.points {
            assert_eq!(grid().snap(*q), *q);
        }
        assert_eq!(run.points.len(), 3, "{:?}", run.points);
        assert_eq!(run.points[0], grid().snap(Point::new(10.01, 10.0)));
        assert_eq!(*run.points.last().unwrap(), Point::new(20.0, 20.0));
        assert_eq!(run.seg.len(), run.points.len());
    }

    /// A point that would move the axis straight back at the tip is dropped.
    #[test]
    fn a_move_back_at_the_tip_is_dropped() {
        // The blade trails along +x; the second point steps back by a unit.
        let pts = vec![
            Point::new(10.0, 10.0),
            Point::new(12.0, 10.0),
            Point::new(11.95, 10.0),
            Point::new(12.0, 12.0),
        ];
        let mut paths = vec![toolpath(pts)];
        round_runs(&mut paths, &grid(), 0.25, Vec2::new(1.0, 0.0));
        let run = &paths[0].axis.as_ref().unwrap()[0];
        assert!(
            !run.points.contains(&Point::new(11.95, 10.0)),
            "{:?}",
            run.points
        );
        assert_eq!(run.points.len(), 3);
    }

    /// The end of a run that steps back at the tip stays, and the point
    /// before it goes, so the last step does not push.
    #[test]
    fn a_last_move_back_at_the_tip_drops_the_point_before() {
        let pts = vec![
            Point::new(10.0, 10.0),
            Point::new(12.0, 10.0),
            Point::new(11.95, 10.0),
        ];
        let mut paths = vec![toolpath(pts)];
        round_runs(&mut paths, &grid(), 0.25, Vec2::new(1.0, 0.0));
        let run = &paths[0].axis.as_ref().unwrap()[0];
        assert_eq!(
            run.points,
            vec![Point::new(10.0, 10.0), Point::new(11.95, 10.0)]
        );
        assert_eq!(run.seg, vec![WASTE_SEG, 1]);
        // Every step trails the tip the model follows.
        let mut tip = run.points[0] - Vec2::new(0.25, 0.0);
        for w in run.points.windows(2) {
            assert!(push_angle_deg(w[0], w[1], tip) <= PUSHED_DEG);
            tip = drag(w[0], w[1], tip, 0.25);
        }
    }
}
