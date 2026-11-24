//! FlatCraft device crate: plotter and material profiles, GPGL emitter,
//! byte log and transports.

pub mod bytelog;
pub mod gpgl;
pub mod presets;
pub mod profile;
pub mod store;
pub mod transport;

pub use bytelog::{ByteLog, Dir, LogEntry};
pub use profile::{
    AxisMapping, Capabilities, GpglDialect, KnifeCompensation, Margins, MaterialOverride,
    MaterialProfile, Media, PlotterProfile, Protocol, TransportSpec,
};
pub use store::{PresetInfo, ProfileError, ProfileStore};
pub use transport::{FileTransport, SendReport, Transport, TransportError};
