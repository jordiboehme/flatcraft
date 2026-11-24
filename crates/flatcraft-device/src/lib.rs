//! FlatCraft device crate: plotter and material profiles (more later).

pub mod presets;
pub mod profile;
pub mod store;

pub use profile::{
    AxisMapping, Capabilities, GpglDialect, KnifeCompensation, Margins, MaterialOverride,
    MaterialProfile, Media, PlotterProfile, Protocol, TransportSpec,
};
pub use store::{PresetInfo, ProfileError, ProfileStore};
