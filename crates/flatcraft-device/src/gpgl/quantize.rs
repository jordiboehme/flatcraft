//! Millimetre points to device units (gpgl.md section 2).
//!
//! Every absolute point is converted on its own, so rounding errors never add up.

use kurbo::Point;

use super::DevPoint;
use crate::profile::{Media, PlotterProfile};

/// The rounded device coordinates as `(y, x)` before any range check.
///
/// Add the grid origin of the mat (app coordinates become device coordinates), mirror inside
/// the physical media rect, multiply by the profile scale, multiply by units per mm,
/// round half away from zero (`f64::round`).
pub(crate) fn to_device_f64(p: Point, plotter: &PlotterProfile, media: &Media) -> (f64, f64) {
    let mut x = p.x + media.grid.origin_x_mm;
    let mut y = p.y + media.grid.origin_y_mm;
    if plotter.axis.mirror_x {
        x = media.width_mm - x;
    }
    if plotter.axis.mirror_y {
        y = media.height_mm - y;
    }
    x *= plotter.scale_x;
    y *= plotter.scale_y;
    (
        (y * plotter.units_per_mm).round(),
        (x * plotter.units_per_mm).round(),
    )
}

/// Converts an app point (mm, origin = the mat's grid 0,0) to device units.
///
/// This does no range check: values outside `i32` saturate and NaN becomes 0. The emitter
/// checks every coordinate against the media before it builds any command, so use
/// [`super::emit_job`] for bytes that go to a device.
pub fn to_device(p: Point, plotter: &PlotterProfile, media: &Media) -> DevPoint {
    let (y, x) = to_device_f64(p, plotter, media);
    DevPoint {
        y: y as i32,
        x: x as i32,
    }
}
