//! FlatCraft path crate: turns placed vector paths into ordered toolpaths.
//!
//! The pipeline is a chain of small pure functions, one per module, in
//! pipeline order: `flatten`, `clean`, `order`, `overcut`, `passes`. `plan`
//! wires them together. `containment` holds the ring geometry they share:
//! the containment tree, the kept side of each ring and the job's kept
//! material.

pub mod clean;
pub mod containment;
pub mod flatten;
pub mod order;
pub mod overcut;
pub mod passes;
pub mod plan;

pub use clean::Polyline;
pub use plan::{
    ASSUMED_START_HEADING, AlignmentStroke, AppliedCompensation, AxisRun, CORNER_DEG, CutPlan,
    KeepSide, PathRef, PipelineSettings, PlacedPath, PlanError, SettingsField, Toolpath,
    ToolpathKind, WASTE_SEG, plan,
};
