//! FlatCraft device crate: plotter and material profiles, GPGL emitter,
//! byte log and transports.

pub mod bytelog;
pub mod capture;
/// Crash-safe writes; they live in `flatcraft-core` so project files use them too.
pub use flatcraft_core::fsio;
pub mod gpgl;
pub mod presets;
pub mod profile;
pub mod store;
pub mod transport;

pub use bytelog::{ByteLog, Dir, LogEntry};
pub use profile::{
    AxisMapping, Capabilities, DEFAULT_MAT_COLOR, DEFAULT_NO_MAT_COLOR, GpglDialect, Grid,
    KnifeCompensation, MaterialOverride, MaterialProfile, Media, MediaError, MediaField, MediaKind,
    PAPER_COLOR, PlotterProfile, Protocol, TransportSpec, UNCALIBRATED_MARGIN_MM,
};
pub use store::{PresetDiagnostic, PresetInfo, ProfileError, ProfileStore};
pub use transport::{FileTransport, SendReport, Transport, TransportError};
