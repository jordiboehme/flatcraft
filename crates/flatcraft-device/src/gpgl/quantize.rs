//! Millimetre points to device units (gpgl.md section 2).
//!
//! Every absolute point is converted on its own, so rounding errors never add up.

use flatcraft_core::device_grid::DeviceGrid;
use kurbo::{Point, Size, Vec2};

use super::DevPoint;
use crate::profile::{Media, PlotterProfile};

/// The app-to-device transform of `plotter` on `media`: units per mm,
/// the mat's grid origin, mirror inside the media rect and the calibration
/// scale. The knife stage and the simulator round with the same grid.
pub fn device_grid(plotter: &PlotterProfile, media: &Media) -> DeviceGrid {
    DeviceGrid {
        units_per_mm: plotter.units_per_mm,
        origin: Point::new(media.grid.origin_x_mm, media.grid.origin_y_mm),
        media_size: Size::new(media.width_mm, media.height_mm),
        mirror_x: plotter.axis.mirror_x,
        mirror_y: plotter.axis.mirror_y,
        scale: Vec2::new(plotter.scale_x, plotter.scale_y),
    }
}

/// The rounded device coordinates as `(y, x)` before any range check.
///
/// Add the grid origin of the mat (app coordinates become device coordinates), mirror inside
/// the physical media rect, multiply by the profile scale, multiply by units per mm,
/// round half away from zero (`f64::round`): [`DeviceGrid::rounded_units`].
pub(crate) fn to_device_f64(p: Point, plotter: &PlotterProfile, media: &Media) -> (f64, f64) {
    let u = device_grid(plotter, media).rounded_units(p);
    (u.y, u.x)
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
