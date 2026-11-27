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
/// The direction closed shapes are cut in (a plotter setting).
pub use flatcraft_core::geometry::RingDirection;
pub use profile::{
    AxisMapping, Capabilities, CornerModeSpec, DEFAULT_MAT_COLOR_DARK, DEFAULT_MAT_COLOR_LIGHT,
    GpglDialect, Grid, KnifeCompensation, MatTack, MaterialGroup, MaterialOverride,
    MaterialProfile, Media, MediaError, MediaField, MediaKind, PlotterProfile, Protocol,
    TransportSpec, UNCALIBRATED_MARGIN_MM,
};
pub use store::{PRESET_NOTES_FILE, PresetDiagnostic, PresetInfo, ProfileError, ProfileStore};
pub use transport::{FileTransport, SendReport, Transport, TransportError};
