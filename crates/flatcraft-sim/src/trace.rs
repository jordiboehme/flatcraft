//! Running the model over the axis runs of a job.
//!
//! The head starts at the home position with the blade up. Each run is one
//! blade-down stretch: the head travels to its first point with the blade up
//! (the tip keeps its offset vector), drops the blade there and drags it
//! along the run. Travel arrives at the drop point blade up and the drop
//! point is then repeated blade down, so the moves into `down` samples sum
//! to the cut length and the others to the travel.

use flatcraft_path::{Toolpath, WASTE_SEG};
use kurbo::{Point, Vec2};

use crate::model::{drag, push_angle_deg};
use crate::{PUSHED_DEG, SimSettings};

/// One point of the simulated job.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub axis: Point,
    pub tip: Point,
    /// The blade is down during the move into this sample.
    pub down: bool,
    /// The toolpath being cut, or travelled to while the blade is up.
    pub toolpath: usize,
    /// The run of that toolpath.
    pub run: usize,
    /// The intended segment of the move into this sample: the run's `seg`
    /// entry, and `WASTE_SEG` for travel and the drop point.
    pub seg: u32,
    /// Axis travel from the home position to this sample, in mm, blade up
    /// and down.
    pub t_mm: f64,
    /// The move into this sample pushes the blade from behind (blade down,
    /// push angle above `PUSHED_DEG`).
    pub pushed: bool,
}

/// The simulated job. The first sample is the home position, blade up.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimTrace {
    pub samples: Vec<Sample>,
}

struct Head {
    axis: Point,
    tip: Point,
    t_mm: f64,
}

/// Simulate `toolpaths` in order, starting at `home` (the plan's home) with
/// the blade up and the tip at `home - d * start_heading`. Samples are at
/// most `sample_step_mm` of axis travel apart and on every run point. With a
/// grid, every run point is snapped first.
pub fn simulate(toolpaths: &[Toolpath], home: Point, s: &SimSettings) -> SimTrace {
    let d = s.blade_offset_mm;
    let mut head = Head {
        axis: home,
        tip: home - d * unit_or_x(s.start_heading),
        t_mm: 0.0,
    };
    let mut samples = vec![Sample {
        axis: head.axis,
        tip: head.tip,
        down: false,
        toolpath: 0,
        run: 0,
        seg: WASTE_SEG,
        t_mm: 0.0,
        pushed: false,
    }];
    let snap = |p: Point| s.grid.map_or(p, |g| g.snap(p));
    for (ti, t) in toolpaths.iter().enumerate() {
        for (ri, run) in t.runs().iter().enumerate() {
            let Some(&first) = run.points.first() else {
                continue;
            };
            let at = |seg, down| Sample {
                axis: Point::ZERO,
                tip: Point::ZERO,
                down,
                toolpath: ti,
                run: ri,
                seg,
                t_mm: 0.0,
                pushed: false,
            };
            // blade up: the tip keeps its offset vector
            let offset = head.axis - head.tip;
            move_to(
                &mut head,
                &mut samples,
                snap(first),
                s,
                at(WASTE_SEG, false),
                |_, a, _| (a - offset, false),
            );
            let drop = Sample {
                axis: head.axis,
                tip: head.tip,
                t_mm: head.t_mm,
                ..at(WASTE_SEG, true)
            };
            samples.push(drop);
            for (j, &p) in run.points.iter().enumerate().skip(1) {
                let seg = run.seg.get(j).copied().unwrap_or(WASTE_SEG);
                move_to(
                    &mut head,
                    &mut samples,
                    snap(p),
                    s,
                    at(seg, true),
                    |from, a, tip| {
                        let pushed = push_angle_deg(from, a, tip) > PUSHED_DEG;
                        (drag(from, a, tip, d), pushed)
                    },
                );
            }
        }
    }
    SimTrace { samples }
}

/// Move the axis straight to `to` in steps of at most `sample_step_mm`,
/// pushing one sample per step. `step(from, to, tip)` gives the tip after
/// the axis step `from -> to` with the tip at `tip`, and whether that step
/// pushes the blade. A zero-length blade-down move still gets its sample.
fn move_to(
    head: &mut Head,
    samples: &mut Vec<Sample>,
    to: Point,
    s: &SimSettings,
    template: Sample,
    mut step: impl FnMut(Point, Point, Point) -> (Point, bool),
) {
    let from = head.axis;
    let len = from.distance(to);
    if len == 0.0 {
        if template.down {
            samples.push(Sample {
                axis: head.axis,
                tip: head.tip,
                t_mm: head.t_mm,
                ..template
            });
        }
        return;
    }
    let n = (len / s.sample_step_mm).ceil().max(1.0) as usize;
    let mut prev = from;
    for k in 1..=n {
        let axis = if k == n {
            to
        } else {
            from.lerp(to, k as f64 / n as f64)
        };
        let (tip, pushed) = step(prev, axis, head.tip);
        head.t_mm += prev.distance(axis);
        head.axis = axis;
        head.tip = tip;
        samples.push(Sample {
            axis,
            tip,
            t_mm: head.t_mm,
            pushed: template.down && pushed,
            ..template
        });
        prev = axis;
    }
}

/// `v` as a unit vector, or +x when it has no length.
fn unit_or_x(v: Vec2) -> Vec2 {
    let len = v.length();
    if len > 0.0 {
        v / len
    } else {
        Vec2::new(1.0, 0.0)
    }
}
