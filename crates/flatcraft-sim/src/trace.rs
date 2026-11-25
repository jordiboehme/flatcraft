//! Running the model over the axis runs of a job.
//!
//! The head starts at the home position with the blade up. Each run is one
//! blade-down stretch: the head travels to its first point with the blade up
//! (the tip keeps its offset vector), drops the blade there and drags it
//! along the run. Travel arrives at the drop point blade up and the drop
//! point is then repeated blade down, so the moves into `down` samples sum
//! to the cut length and the others to the travel.

use flatcraft_path::{AxisRun, Toolpath, WASTE_SEG};
use kurbo::{Point, Vec2};

use crate::model::{drag, push_angle_deg};
use crate::{MAX_SAMPLES, MIN_SAMPLE_STEP_MM, PUSHED_DEG, SimError, SimSettings};

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
/// most `sample_step_mm` of axis travel apart (at least
/// [`MIN_SAMPLE_STEP_MM`]) and on every run point. With a grid, every run
/// point is snapped first.
///
/// Refused, before any sample is made: a home or run point that is not
/// finite ([`SimError::NonFinite`]), a negative or non-finite blade offset
/// ([`SimError::BladeOffset`]) and a job that would need more than
/// [`MAX_SAMPLES`] samples ([`SimError::TooManySamples`]).
pub fn simulate(
    toolpaths: &[Toolpath],
    home: Point,
    s: &SimSettings,
) -> Result<SimTrace, SimError> {
    let d = s.blade_offset_mm;
    if !(d.is_finite() && d >= 0.0) {
        return Err(SimError::BladeOffset { value: d });
    }
    if !home.is_finite() {
        return Err(SimError::NonFinite {
            toolpath: None,
            run: 0,
            index: 0,
        });
    }
    let step = sample_step(s.sample_step_mm);
    let snap = |p: Point| s.grid.map_or(p, |g| g.snap(p));
    let runs: Vec<Vec<AxisRun>> = toolpaths.iter().map(Toolpath::runs).collect();

    // First pass: check every point and count the samples, so a bad job
    // fails before it allocates.
    let mut count = 1usize;
    let mut here = home;
    for (ti, t_runs) in runs.iter().enumerate() {
        for (ri, run) in t_runs.iter().enumerate() {
            debug_assert_eq!(
                run.seg.len(),
                run.points.len(),
                "one seg per run point (toolpath {ti}, run {ri})"
            );
            for (j, &p) in run.points.iter().enumerate() {
                let p = snap(p);
                if !p.is_finite() {
                    return Err(SimError::NonFinite {
                        toolpath: Some(ti),
                        run: ri,
                        index: j,
                    });
                }
                count = count
                    .saturating_add(steps(here, p, step))
                    .saturating_add(usize::from(j == 0));
                here = p;
            }
        }
    }
    if count > MAX_SAMPLES {
        return Err(SimError::TooManySamples { limit: MAX_SAMPLES });
    }

    let mut head = Head {
        axis: home,
        tip: home - d * unit_or_x(s.start_heading),
        t_mm: 0.0,
    };
    let mut samples = Vec::with_capacity(count);
    samples.push(Sample {
        axis: head.axis,
        tip: head.tip,
        down: false,
        toolpath: 0,
        run: 0,
        seg: WASTE_SEG,
        t_mm: 0.0,
        pushed: false,
    });
    for (ti, t_runs) in runs.iter().enumerate() {
        for (ri, run) in t_runs.iter().enumerate() {
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
                step,
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
                    step,
                    at(seg, true),
                    |from, a, tip| {
                        let pushed = push_angle_deg(from, a, tip) > PUSHED_DEG;
                        (drag(from, a, tip, d), pushed)
                    },
                );
            }
        }
    }
    Ok(SimTrace { samples })
}

/// The sample step to use: at least [`MIN_SAMPLE_STEP_MM`], and that when
/// the setting is not a number.
pub(crate) fn sample_step(step: f64) -> f64 {
    if step.is_nan() {
        MIN_SAMPLE_STEP_MM
    } else {
        step.max(MIN_SAMPLE_STEP_MM)
    }
}

/// How many samples `move_to` makes from `from` to `to` (both finite):
/// one per step, and one for a zero-length move (only blade-down moves
/// push it, so this can count one too many, never too few).
fn steps(from: Point, to: Point, step: f64) -> usize {
    let n = (from.distance(to) / step).ceil();
    if n >= usize::MAX as f64 {
        usize::MAX
    } else {
        (n as usize).max(1)
    }
}

/// Move the axis straight to `to` in steps of at most `step`, pushing one
/// sample per step. `advance(from, to, tip)` gives the tip after the axis
/// step `from -> to` with the tip at `tip`, and whether that step pushes
/// the blade. A zero-length blade-down move still gets its sample.
fn move_to(
    head: &mut Head,
    samples: &mut Vec<Sample>,
    to: Point,
    step: f64,
    template: Sample,
    mut advance: impl FnMut(Point, Point, Point) -> (Point, bool),
) {
    let from = head.axis;
    if from == to {
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
    let n = steps(from, to, step);
    let mut prev = from;
    for k in 1..=n {
        let axis = if k == n {
            to
        } else {
            from.lerp(to, k as f64 / n as f64)
        };
        let (tip, pushed) = advance(prev, axis, head.tip);
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
