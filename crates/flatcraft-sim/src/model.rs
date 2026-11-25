//! The drag knife model along one straight axis segment.
//!
//! The tip trails the axis at distance `d` and moves only along the
//! tip-to-axis direction (spec 8.6). Along a straight axis move the tip
//! follows a tractrix, which has a closed form: with `phi` the signed angle
//! from the move direction `u` to the tip-to-axis vector `v`,
//! `dphi/ds = -sin(phi) / d`, so `tan(phi / 2)` decays as `exp(-s / d)`.
//! The result is exact for the model, so the step size of a caller can not
//! hide an error.

use kurbo::{Point, Vec2};

/// Below this the move or the offset has no length.
const TINY: f64 = 1e-12;

/// Where the tip ends when the axis moves straight from `axis_from` to
/// `axis_to`, with the tip at `tip` (at distance `d` from `axis_from`).
///
/// An exact push (the axis moves straight at the tip) is the model's
/// singular point: the tip is carried along with the axis.
pub fn drag(axis_from: Point, axis_to: Point, tip: Point, d: f64) -> Point {
    let step = axis_to - axis_from;
    let s = step.length();
    if d <= TINY {
        return axis_to;
    }
    if s <= TINY {
        return tip;
    }
    let u = step / s;
    let phi0 = signed_angle(u, axis_from - tip);
    // Modelling choice at the exact push (phi0 = pi): the pursuit equation
    // has an unstable fixed point there, and an exact solution keeps the tip
    // in front of the axis forever, carried along. A move a hair away from
    // pi (more than 1e-9 rad) takes the closed form instead, where the tip
    // swings round to trail within a few d. So the result is discontinuous
    // at phi0 near pi. Neither branch is what a real blade does when pushed
    // (it flips or digs in), so the trace marks such moves `pushed` (push
    // angle above `PUSHED_DEG`) and the metrics report them instead of
    // trusting the tip position.
    if (std::f64::consts::PI - phi0.abs()).abs() < 1e-9 {
        return tip + step;
    }
    let phi = 2.0 * ((phi0 / 2.0).tan() * (-s / d).exp()).atan();
    axis_to - d * (phi.cos() * u + phi.sin() * u.turn_90())
}

/// The angle in degrees between the axis move and the tip-to-axis vector:
/// 0 when the tip trails straight behind, 180 when the axis pushes it.
/// 0 when the move or the offset has no length.
pub fn push_angle_deg(axis_from: Point, axis_to: Point, tip: Point) -> f64 {
    let (u, v) = (axis_to - axis_from, axis_from - tip);
    if u.length() <= TINY || v.length() <= TINY {
        return 0.0;
    }
    signed_angle(u, v).abs().to_degrees()
}

/// The signed angle from `a` to `b` in (-pi, pi]; positive turns `a` towards `a.turn_90()`.
fn signed_angle(a: Vec2, b: Vec2) -> f64 {
    let angle = a.cross(b).atan2(a.dot(b));
    if angle == -std::f64::consts::PI {
        std::f64::consts::PI
    } else {
        angle
    }
}
