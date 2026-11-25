//! FlatCraft path crate: turns placed vector paths into ordered toolpaths.
//!
//! The pipeline is a chain of small pure functions, one per module, in
//! pipeline order: `flatten`, `clean`, `order`, `overcut`, `passes`. `plan`
//! wires them together. `feed` holds the feed bands the order can work in.
//! `containment` holds the ring geometry they share:
//! the containment tree, the kept side of each ring and the job's kept
//! material. `rings` holds the ring direction, entry candidates and exit
//! states the order uses, and `strategy` what the order optimises for.

pub mod clean;
pub mod containment;
pub mod feed;
pub mod flatten;
pub mod order;
pub mod overcut;
pub mod passes;
pub mod plan;
pub mod rings;
pub mod strategy;

pub use clean::Polyline;
pub use containment::{KeepMode, KeptMaterial};
pub use plan::{
    ASSUMED_START_HEADING, AlignmentStroke, AppliedCompensation, AxisRun, CORNER_DEG, CutPlan,
    KeepSide, PathRef, PipelineSettings, PlacedPath, PlanError, SettingsField, Toolpath,
    ToolpathKind, WASTE_SEG, plan,
};
pub use strategy::StrategySettings;
