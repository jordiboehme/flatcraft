//! The engine side of the knife simulator.

use flatcraft_device::{Media, PlotterProfile};
use flatcraft_sim::DeviceGrid;
use kurbo::Point;

/// The simulator's device grid for `plotter` on `media`: the plotter's units
/// per mm and the mat's grid origin. Mirror and scale are left out on
/// purpose (they only correct the machine's own error).
pub fn device_grid(plotter: &PlotterProfile, media: &Media) -> DeviceGrid {
    DeviceGrid {
        units_per_mm: plotter.units_per_mm,
        origin: Point::new(media.grid.origin_x_mm, media.grid.origin_y_mm),
    }
}
