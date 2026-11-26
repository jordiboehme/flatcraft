//! The engine side of the knife simulator.

use flatcraft_device::{Media, PlotterProfile};
use flatcraft_sim::DeviceGrid;

/// The simulator's device grid for `plotter` on `media`: the emitter's own
/// app-to-device transform (units per mm, grid origin, mirror and scale),
/// so the simulator rounds every point exactly as the plotter gets it.
pub fn device_grid(plotter: &PlotterProfile, media: &Media) -> DeviceGrid {
    flatcraft_device::gpgl::quantize::device_grid(plotter, media)
}
