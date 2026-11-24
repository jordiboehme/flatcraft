//! GPGL (Silhouette Cameo 1/2) commands, quantizing, job emitter, safety check and decoder.
//!
//! Clean-room: written only from `docs/protocol/gpgl.md`.

pub mod command;
pub mod decode;
pub mod emit;
pub mod quantize;
pub mod safety;

pub use command::{DevPoint, EmitError, GpglCommand, Tb50Mode};
pub use decode::{DecodedCommand, decode};
pub use emit::{CutSettings, EmitPath, EmittedJob, ListingLine, emit_job};
pub use safety::{SafetyError, check_safe, check_safe_fragment};
