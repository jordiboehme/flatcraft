//! Job building: document -> cut plan -> GPGL bytes, and sending.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use flatcraft_core::document::{CutOptions, Document, GroupId, JobSettings, KeepMode, ObjectId};
use flatcraft_core::geometry::CutPath;
use flatcraft_core::transform::Transform;
use flatcraft_device::gpgl::{
    CheckedJob, CutSettings, EmitError, EmitPath, EmittedJob, GeometryKind, emit_job,
};
use flatcraft_device::{
    CornerModeSpec, FileTransport, KnifeCompensation, MaterialProfile, MediaKind, PlotterProfile,
    SendReport, Transport, TransportError, TransportSpec,
};
use flatcraft_path::{
    AppliedCompensation, Compensation, CornerMode, CutPlan, HostSettings, PathRef,
    PipelineSettings, PlacedPath, PlanError, StrategySettings, Toolpath, ToolpathKind, knife, plan,
};
use serde_json::{Value, json};

use crate::commands::profile_error;
use crate::commands::project::{in_config, lexical, resolve_strict, resolved};
use crate::cutgeom::{self, CutGeometry, CutPiece, PieceFacts};
use crate::cutlog;
use crate::gate::{self, GateDecision};
use crate::report::{self, BuildFacts, Issue};
use crate::session::{CommandError, JobSummary, Origin, Session};

/// Join tolerance of the pipeline: path ends closer than this are joined.
pub const JOIN_TOLERANCE_MM: f64 = 0.01;

/// Name of a job when there is no project file to name it after.
pub const DEFAULT_NAME: &str = "flatcraft-job";

/// Longest job name, in characters.
pub const MAX_NAME_LEN: usize = 100;

/// Sub folder of the Documents folder for File Output.
pub const OUTPUT_FOLDER: &str = "FlatCraft Output";

/// Refusal while an agent's USB send waits for the person.
pub const WAITING: &str = "Waiting for confirmation in the FlatCraft window.";

/// Refusal of every USB send in milestone 1.
pub const USB_NOT_AVAILABLE: &str = "USB cutting arrives in milestone 3.";

/// Result of the last job build.
#[derive(Debug, Clone, PartialEq)]
pub struct JobResult {
    pub plan: CutPlan,
    pub job: EmittedJob,
    pub settings: CutSettings,
    pub warnings: Vec<String>,
    /// How many of `warnings` only repeat the person's own choice (a hidden
    /// or a skipped object). The others are real problems of the job (a
    /// path outside the area, a mat that is not there, paths that are not
    /// cut). The window's state dot counts only those.
    pub choice_warnings: usize,
    /// Problems of a job that does cut (filled shapes the cut mode cuts
    /// nothing of), with their fixes. See `report`.
    pub issues: Vec<Issue>,
    /// The indexes of `warnings` that `issues` say again, with a fix: a
    /// window that shows the issues leaves these warnings out.
    pub covered_warnings: Vec<usize>,
    pub sent: Option<SendReport>,
    /// The plotter profile the job was built for, with every setting as
    /// it was then. [`send_built`] refuses to send it with another one.
    pub built_for: PlotterProfile,
}

/// The failure when a built job is sent with another plotter profile than
/// it was built for, or with the same one after its settings changed.
pub const BUILT_FOR_OTHER_PLOTTER: &str = "internal: the job was built for another plotter preset or for other settings of it, so it is not sent. Build the job again.";

/// Replaces line breaks and other control characters with spaces, so a note
/// stays on one listing line.
fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Listing note of one toolpath. The File Output listing, `job.preview`
/// and the Debug panel show it to people and agents, so keep the format
/// stable. A single lap reads `pass {p}`; a ring of chained laps reads
/// `passes {p}-{q}` (for example `passes 1-3`).
fn note(
    id: ObjectId,
    name: &str,
    path: usize,
    label: Option<&str>,
    pass: u32,
    laps: u32,
) -> String {
    let label = label
        .map(|l| format!(" [{}]", one_line(l)))
        .unwrap_or_default();
    let passes = if laps > 1 {
        format!("passes {pass}-{}", last_pass(pass, laps))
    } else {
        format!("pass {pass}")
    };
    format!(
        "object {} '{}', path {path}{label}, {passes}",
        id.0,
        one_line(name)
    )
}

/// Listing note of a weed toolpath, numbered from 1: `weed line {n},
/// pass {p}`, `weed box {n}, passes {p}-{q}`, or `weed box {n} piece, pass
/// {p}` for a band piece of a box (no mat).
fn weed_note(t: &flatcraft_path::Toolpath, pass: u32, laps: u32) -> String {
    let n = t.source.path_index + 1;
    let what = match (
        t.source.object == flatcraft_path::weed::WEED_BOX_OBJECT,
        t.closed,
    ) {
        (true, true) => format!("weed box {n}"),
        (true, false) => format!("weed box {n} piece"),
        _ => format!("weed line {n}"),
    };
    format!("{what}, {}", passes_text(pass, laps))
}

fn passes_text(pass: u32, laps: u32) -> String {
    if laps > 1 {
        format!("passes {pass}-{}", last_pass(pass, laps))
    } else {
        format!("pass {pass}")
    }
}

/// The warning when weed boxes do not fit inside the cutting area.
pub(crate) fn weed_box_warning(dropped: usize, per_object: bool) -> String {
    let what = if per_object && dropped > 1 {
        format!("{dropped} weed boxes do not fit")
    } else if per_object {
        "1 weed box does not fit".to_string()
    } else {
        "The weed box does not fit".to_string()
    };
    let it = if dropped > 1 { "they are" } else { "it is" };
    format!(
        "{what} inside the cutting area with 1 mm to spare around the design, so {it} not cut. Move the design away from the edge or make the weed box margin smaller."
    )
}

/// The warning when weed boxes would cut material that is kept.
pub(crate) fn weed_box_blocked_warning(blocked: usize) -> String {
    if blocked == 1 {
        "A weed box would cut through material you keep, so it is not cut.".to_string()
    } else {
        format!("{blocked} weed boxes would cut through material you keep, so they are not cut.")
    }
}

/// The number of the last pass in a toolpath of `laps` chained laps that
/// starts with pass `pass`.
fn last_pass(pass: u32, laps: u32) -> u32 {
    pass.saturating_add(laps.max(1) - 1)
}

/// What the points of a plan are, for the emitter's compensation check.
fn geometry_kind(c: AppliedCompensation) -> GeometryKind {
    match c {
        AppliedCompensation::Tip => GeometryKind::Tip,
        AppliedCompensation::Host { offset_mm } => GeometryKind::HostAxis { offset_mm },
        AppliedCompensation::HostCalibration => GeometryKind::HostCalibration,
    }
}

/// The warning when the plan left out paths shorter than half a device
/// step. With [`collapsed_warning`] it counts every path that is not cut
/// for being too small, each path once: the plan drops these before the
/// emitter sees them.
pub(crate) fn dropped_short_warning(n: usize) -> String {
    if n == 1 {
        "1 path is shorter than half a plotter step and is not cut.".to_string()
    } else {
        format!("{n} paths are shorter than half a plotter step and are not cut.")
    }
}

/// The warning when the emitter left out paths whose points all round to
/// one plotter step.
pub(crate) fn collapsed_warning(n: usize) -> String {
    if n == 1 {
        "1 path fits inside one plotter step and is not cut.".to_string()
    } else {
        format!("{n} paths fit inside one plotter step and are not cut.")
    }
}

/// The warning for curves tighter than the material's minimum radius
/// `r_mm`.
pub(crate) fn tight_curves_warning(n: usize, r_mm: f64) -> String {
    if n == 1 {
        format!("1 curve is tighter than {r_mm} mm. It may come out rounded or torn.")
    } else {
        format!("{n} curves are tighter than {r_mm} mm. They may come out rounded or torn.")
    }
}

/// The warning when host compensation steers with another offset than the
/// plotter's blade offset.
pub(crate) fn offset_mismatch_warning(host_mm: f64, blade_mm: f64) -> String {
    format!(
        "Host compensation uses {host_mm} mm, the blade offset is {blade_mm} mm. The simulator uses the blade offset."
    )
}

/// How far apart the host offset and the blade offset may be before the
/// job warns: closer is the same number typed twice.
const OFFSET_SAME_MM: f64 = 1e-9;

/// The places where the first pass of a `Cut` toolpath bends tighter than
/// `min_radius_mm`, over the first lap only, so passes, chained laps and
/// the overcut never count a curve twice. The same in every compensation
/// mode: it reads the intended tip path.
fn tight_curves(plan: &CutPlan, min_radius_mm: f64) -> usize {
    plan.toolpaths
        .iter()
        .filter(|t| t.kind == ToolpathKind::Cut && t.pass == 1)
        .map(|t| knife::min_radius_spots(t.lap(), min_radius_mm).len())
        .sum()
}

/// Where the knife is steered for `plotter` on `media` with `strategy`:
/// the tip path for firmware and no compensation; host compensation with
/// the profile's offset, rounded to the plotter's device grid (so plan,
/// preview, simulation and send share the points the plotter gets). The
/// pivot threshold is the larger of the profile's and the strategy's, so
/// a profile can only make it coarser.
fn compensation(
    plotter: &PlotterProfile,
    media: &flatcraft_device::Media,
    strategy: &StrategySettings,
) -> Compensation {
    match plotter.knife_compensation {
        KnifeCompensation::Host {
            offset_mm,
            pivot_threshold_deg,
        } => Compensation::Host(HostSettings {
            device_grid: Some(flatcraft_device::gpgl::quantize::device_grid(
                plotter, media,
            )),
            ..HostSettings::new(
                offset_mm,
                pivot_threshold_deg.max(strategy.host_pivot_threshold_deg),
            )
        }),
        KnifeCompensation::Firmware { .. } | KnifeCompensation::None => Compensation::Tip,
    }
}

/// The corner mode of a material, for the knife stage.
fn corner_mode(spec: CornerModeSpec) -> CornerMode {
    match spec {
        CornerModeSpec::Arc => CornerMode::Arc,
        CornerModeSpec::LiftOvershoot { overshoot_mm } => {
            CornerMode::LiftOvershoot { overshoot_mm }
        }
    }
}

/// Problems the person can fix (settings, placement, media, a plan made
/// for another knife compensation) are refusals; broken profiles and
/// internal errors are failures.
fn emit_error(e: EmitError) -> CommandError {
    let text = format!("cannot build the GPGL job: {e}");
    match e {
        EmitError::OutOfRange { .. }
        | EmitError::CoordinateOutOfRange { .. }
        | EmitError::UnknownMedia { .. }
        | EmitError::EmptyJob => CommandError::Refused(text),
        EmitError::CompensationMismatch { profile, geometry } => {
            let fix = if geometry == GeometryKind::HostCalibration.describe() {
                "An offset test series needs Host knife compensation: set it in the plotter preset in the Profiles window."
            } else if profile == "host" {
                "Build the job again, so it is planned for this plotter preset."
            } else {
                "Build the job again, so it is planned for this plotter preset, or set Host knife compensation in the Profiles window."
            };
            CommandError::Refused(format!("{text}. {fix}"))
        }
        EmitError::DialectNotSupported
        | EmitError::EmptyDraw
        | EmitError::InvalidProfile { .. } => CommandError::Failed(text),
    }
}

/// The refusal of a plan whose axis runs do not fit its own compensation.
pub const PLAN_MISMATCH: &str = "internal: compensation mismatch in the plan";

/// The names of a job's objects and the source index and label of every
/// cut piece, for the listing notes. A toolpath's `path_index` is the index
/// of its piece in the object's [`CutPiece`] list; the note names the
/// piece's source path.
struct NoteNames<'a> {
    names: std::collections::BTreeMap<ObjectId, &'a str>,
    pieces: std::collections::BTreeMap<ObjectId, Arc<Vec<CutPiece>>>,
}

impl<'a> NoteNames<'a> {
    fn of(objects: &'a [JobObject], cut: &CutGeometry) -> Self {
        let names = objects.iter().map(|o| (o.id, o.name.as_str())).collect();
        Self {
            names,
            pieces: cut.per_object.clone(),
        }
    }

    /// The listing note of toolpath `t`, without a run number.
    fn note(&self, t: &Toolpath) -> String {
        match t.kind {
            ToolpathKind::Weed => weed_note(t, t.pass, t.laps),
            ToolpathKind::Alignment => "alignment stroke".to_string(),
            ToolpathKind::Cut => {
                let id = t.source.object;
                let name = self.names.get(&id).copied().unwrap_or("");
                let piece = self
                    .pieces
                    .get(&id)
                    .and_then(|p| p.get(t.source.path_index));
                // A piece of several source paths has no one index: it is
                // named by its own.
                let path = piece.and_then(|p| p.source).unwrap_or(t.source.path_index);
                let label = piece.and_then(|p| p.label.as_deref());
                note(id, name, path, label, t.pass, t.laps)
            }
        }
    }
}

/// How many blade-down runs toolpath `t` is emitted as.
fn run_count(t: &Toolpath) -> usize {
    t.axis.as_ref().map_or(1, Vec::len)
}

/// The paths a plan sends: one per blade-down run, the alignment stroke
/// included, each with its listing note. A toolpath of several runs gets
/// ` run {k}` (from 1) on each note.
fn emit_paths(plan: &CutPlan, objects: &[JobObject], cut: &CutGeometry) -> Vec<EmitPath> {
    let names = NoteNames::of(objects, cut);
    plan.toolpaths
        .iter()
        .flat_map(|t| {
            let text = names.note(t);
            let several = run_count(t) > 1;
            t.runs()
                .into_iter()
                .enumerate()
                .map(move |(k, r)| EmitPath {
                    points: r.points,
                    note: if several {
                        format!("{text} run {}", k + 1)
                    } else {
                        text.clone()
                    },
                })
        })
        .collect()
}

/// Emits a plan for `plotter`: the one way from a [`CutPlan`] to GPGL
/// bytes. The geometry kind is what the plan says it applied, never a
/// claim of the caller, and every toolpath must carry axis runs exactly
/// when that compensation is host compensation: a plan that does not is
/// refused before anything is emitted. The emitter then refuses geometry
/// that does not fit the plotter's knife compensation. `objects` and the
/// pieces of `cut` name the toolpaths in the listing.
pub fn emit_plan(
    plan: &CutPlan,
    objects: &[JobObject],
    cut: &CutGeometry,
    plotter: &PlotterProfile,
    media_id: &str,
    settings: &CutSettings,
) -> Result<EmittedJob, CommandError> {
    let host = matches!(
        plan.compensation,
        AppliedCompensation::Host { .. } | AppliedCompensation::HostCalibration
    );
    // Every kind, not only cuts: under host compensation the knife stage
    // gives weed lines and the alignment stroke axis runs too, and an axis
    // run sent as a tip path would cut off the intended line.
    if plan.toolpaths.iter().any(|t| t.axis.is_some() != host) {
        return Err(CommandError::Failed(PLAN_MISMATCH.into()));
    }
    emit_job(
        &emit_paths(plan, objects, cut),
        geometry_kind(plan.compensation),
        plotter,
        media_id,
        settings,
    )
    .map_err(emit_error)
}

/// For each toolpath of `plan`, whether the emitter left out every one of
/// its runs because each rounds to one device point (`collapsed`, the
/// indexes of the emitted runs, in order).
fn left_out(plan: &CutPlan, collapsed: &[usize]) -> Vec<bool> {
    let mut first = 0;
    plan.toolpaths
        .iter()
        .map(|t| {
            let runs = first..first + run_count(t);
            first = runs.end;
            runs.clone().all(|i| collapsed.binary_search(&i).is_ok())
        })
        .collect()
}

/// How many paths of `plan` the emitter left out. A path counts once:
/// only its first pass is looked at, the alignment stroke never.
fn paths_not_cut(plan: &CutPlan, collapsed: &[usize]) -> usize {
    plan.toolpaths
        .iter()
        .zip(left_out(plan, collapsed))
        .filter(|(t, out)| *out && t.pass == 1 && t.kind != ToolpathKind::Alignment)
        .count()
}

/// One object as the job build reads it. The geometry `Arc` is shared with
/// the document, so taking a [`JobInput`] is cheap.
#[derive(Debug, Clone)]
pub struct JobObject {
    pub id: ObjectId,
    pub name: String,
    pub visible: bool,
    /// False leaves the object out of the cut; it stays visible.
    pub cut_enabled: bool,
    pub transform: Transform,
    pub local_bounds: kurbo::Rect,
    pub cut: CutOptions,
    /// What is kept after cutting: the planner keeps lead-ins and
    /// overshoots in the waste.
    pub keep: KeepMode,
    pub geometry: Arc<Vec<CutPath>>,
    /// The group of linked copies it belongs to: a fix of shared settings
    /// changes every member.
    pub link_group: Option<GroupId>,
    /// What it counts as for the weed box per object: its top-level group
    /// (see [`flatcraft_core::nodes`]), or itself.
    pub unit: ObjectId,
}

/// Two inputs are equal when the same geometry `Arc` is placed the same
/// way. The comparison never walks the geometry, so a GUI can compare
/// inputs every frame; it holds the `Arc`s, so an address cannot be reused.
impl PartialEq for JobObject {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.visible == other.visible
            && self.cut_enabled == other.cut_enabled
            && self.transform == other.transform
            && self.local_bounds == other.local_bounds
            && self.cut == other.cut
            && self.keep == other.keep
            && Arc::ptr_eq(&self.geometry, &other.geometry)
            && self.link_group == other.link_group
            && self.unit == other.unit
    }
}

/// Everything a job build reads, taken from a session under its lock, so
/// the build itself can run without the lock ([`build_input`]).
#[derive(Debug, Clone, PartialEq)]
pub struct JobInput {
    pub objects: Vec<JobObject>,
    pub job: JobSettings,
    pub plotter: PlotterProfile,
    /// The material with the plotter's overrides applied.
    pub material: MaterialProfile,
    /// The object pairs whose hints the person turned off
    /// (`Document::ignored_pairs`).
    pub ignored_pairs: Vec<flatcraft_core::document::IgnoredPair>,
    /// The top-level groups whose close-members hint is turned off
    /// (`Document::ignored_groups`).
    pub ignored_groups: Vec<ObjectId>,
    /// The name of every group, for the hints that name one.
    pub group_names: std::collections::BTreeMap<ObjectId, String>,
    /// The cut geometry of `objects` (see [`JobInput::cut_geometry`]).
    pub cut: CutSeed,
}

/// The cut geometry of a [`JobInput`]: a cached build to start from (the
/// document's, see `DocState::cut_geometry`), and the geometry once it was
/// asked for. Two seeds are always equal: the geometry follows from the
/// input's objects and profiles, and a seed only saves time (a stale seed
/// is checked object by object and never gives a wrong piece). So a new
/// seed never makes a window build its plan again.
#[derive(Debug, Clone, Default)]
pub struct CutSeed {
    seed: Option<Arc<CutGeometry>>,
    resolved: std::sync::OnceLock<Arc<CutGeometry>>,
}

impl CutSeed {
    /// A seed that starts from `seed`.
    pub fn new(seed: Option<Arc<CutGeometry>>) -> Self {
        Self {
            seed,
            resolved: std::sync::OnceLock::new(),
        }
    }
}

impl PartialEq for CutSeed {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl JobInput {
    /// The flatten tolerance of the cut geometry, mm.
    pub fn tolerance_mm(&self) -> f64 {
        self.plotter.flatten_tolerance_mm
    }

    /// The minimum feature width of the cut geometry, mm (see
    /// [`report::min_width_mm`]).
    pub fn min_width_mm(&self) -> f64 {
        report::min_width_mm(self.plotter.blade_offset_mm)
    }

    /// The cut geometry of the objects: built once per input, from the
    /// seed where it still fits. Every reader of the input (the build, the
    /// export, the report) shares it.
    pub fn cut_geometry(&self) -> Arc<CutGeometry> {
        self.cut
            .resolved
            .get_or_init(|| {
                cutgeom::objects_cut_geometry(
                    &self.objects,
                    self.tolerance_mm(),
                    self.min_width_mm(),
                    self.cut.seed.as_ref(),
                )
            })
            .clone()
    }

    /// The cut geometry if it was built (or taken from the seed) already.
    pub fn built_cut_geometry(&self) -> Option<Arc<CutGeometry>> {
        self.cut.resolved.get().cloned()
    }

    /// The objects of `doc` as the build reads them.
    pub(crate) fn objects_of(doc: &Document) -> Vec<JobObject> {
        let tops = doc.tree().top_of_each();
        doc.objects
            .iter()
            .map(|o| JobObject {
                id: o.id,
                name: o.name.clone(),
                visible: o.visible,
                cut_enabled: o.cut_enabled,
                transform: o.transform,
                local_bounds: o.local_bounds,
                cut: o.cut,
                keep: o.keep,
                geometry: o.geometry.clone(),
                link_group: o.link_group,
                unit: tops.get(&o.id).copied().unwrap_or(o.id),
            })
            .collect()
    }

    /// The top-level group of every grouped object, for the weed box per
    /// object: one box per group. Objects not listed are their own.
    pub fn weed_units(&self) -> std::collections::BTreeMap<ObjectId, ObjectId> {
        self.objects
            .iter()
            .filter(|o| o.unit != o.id)
            .map(|o| (o.id, o.unit))
            .collect()
    }

    /// The keep mode of every object that is not [`KeepMode::Pieces`]
    /// (the default of objects not listed).
    pub fn keep_modes(&self) -> std::collections::BTreeMap<ObjectId, KeepMode> {
        self.objects
            .iter()
            .filter(|o| o.keep != KeepMode::Pieces)
            .map(|o| (o.id, o.keep))
            .collect()
    }

    /// The active document's objects, job settings and profiles, with
    /// the document's cut geometry (`Session::cut_geometry`).
    pub fn of(session: &Session) -> Result<Self, CommandError> {
        let doc = &session.doc().document;
        let job = &doc.job;
        let plotter = session
            .profiles
            .plotter(&job.plotter_id)
            .map_err(profile_error)?;
        let material = session
            .profiles
            .material(&job.material_id)
            .map_err(profile_error)?;
        let input = Self::from_parts(doc, plotter, &material);
        let cut = session.cut_geometry_of(&input);
        let _ = input.cut.resolved.set(cut);
        Ok(input)
    }

    /// The input of `doc` with profiles the caller already resolved: the
    /// plotter of `doc.job` and its material. The plotter's override of
    /// the material is applied here (`MaterialProfile::for_plotter`, which
    /// gives the same result when it was applied already). The GUI uses it
    /// with the profiles of its snapshot, so it never assembles an input
    /// itself.
    pub fn from_parts(doc: &Document, plotter: PlotterProfile, material: &MaterialProfile) -> Self {
        let material = material.for_plotter(&plotter.id);
        Self {
            objects: Self::objects_of(doc),
            job: doc.job.clone(),
            plotter,
            material,
            ignored_pairs: doc.ignored_pairs.clone(),
            ignored_groups: doc.ignored_groups.clone(),
            group_names: doc.groups.iter().map(|g| (g.id, g.name.clone())).collect(),
            cut: CutSeed::default(),
        }
    }

    /// The input with the cut geometry `seed` to start from (a window
    /// passes the document's cached one, see `DocState::cached_cut_geometry`).
    pub fn with_seed(mut self, seed: Option<Arc<CutGeometry>>) -> Self {
        self.cut = CutSeed::new(seed);
        self
    }
}

/// Builds the job of the active document: the visible objects' cut paths
/// (per their cut options, see `cutgeom`), the material values for this
/// plotter, then the job overrides.
pub fn build(session: &Session) -> Result<JobResult, CommandError> {
    build_input(&JobInput::of(session)?)
}

impl JobResult {
    /// How many toolpaths are really cut: the plan's toolpaths without the
    /// ones the emitter left out (too small for one plotter step). The
    /// "Paths" count of `job.preview`, the send reply and the Cut stage.
    pub fn toolpaths_cut(&self) -> usize {
        self.cut_toolpaths().count()
    }

    /// The toolpaths that are really cut, in order: the plan's toolpaths
    /// without the ones the emitter left out. Every count of cut paths
    /// (all of them, the weed paths) reads them here.
    pub fn cut_toolpaths(&self) -> impl Iterator<Item = &Toolpath> {
        self.plan
            .toolpaths
            .iter()
            .zip(left_out(&self.plan, &self.job.collapsed))
            .filter(|(_, out)| !out)
            .map(|(t, _)| t)
    }

    /// The warnings that are real problems, not the person's own choices.
    pub fn problem_warnings(&self) -> usize {
        self.warnings.len().saturating_sub(self.choice_warnings)
    }
}

/// Why a job could not be built, for a caller that needs to tell "nothing
/// to cut" from a real refusal without reading the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildFailure {
    pub error: CommandError,
    /// No visible object has cut paths.
    pub nothing_to_cut: bool,
    /// Every reason the job cannot be cut that a person or an agent can
    /// fix, with the fixes: the reasons there is nothing to cut, or the
    /// objects outside the cutting area. Empty for other refusals.
    pub issues: Vec<Issue>,
    /// The warnings of the build so far (hidden and skipped objects, a
    /// mat that is not there); empty for other refusals.
    pub warnings: Vec<String>,
}

impl From<CommandError> for BuildFailure {
    fn from(error: CommandError) -> Self {
        Self {
            error,
            nothing_to_cut: false,
            issues: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

/// Builds a job from a [`JobInput`]. It does not need the session, so the
/// GUI runs it without holding the session lock.
pub fn build_input(input: &JobInput) -> Result<JobResult, CommandError> {
    build_classified(input).map_err(|f| f.error)
}

/// [`build_input`], and a failure says whether it was "nothing to cut".
pub fn build_classified(input: &JobInput) -> Result<JobResult, BuildFailure> {
    let JobInput {
        objects,
        job,
        plotter,
        material,
        ..
    } = input;
    let cut = input.cut_geometry();
    let (media, mat_warning) = crate::commands::effective_media(plotter, job)?;
    let media_id = media.id.clone();
    let area = plotter
        .cutting_area(&media_id)
        .ok_or_else(|| CommandError::Failed("the mat has no cutting area".into()))?;
    let names: std::collections::BTreeMap<ObjectId, &str> =
        objects.iter().map(|o| (o.id, o.name.as_str())).collect();

    let mut warnings: Vec<String> = mat_warning.into_iter().collect();
    let mut choice_warnings = 0;
    let mut facts = BuildFacts {
        objects: objects.len(),
        ..BuildFacts::default()
    };
    let mut issues = Vec::new();
    let mut covered_warnings = Vec::new();
    let mut placed = Vec::new();
    for o in objects {
        if !o.visible {
            facts.hidden.push(o.id);
            choice_warnings += 1;
            warnings.push(format!(
                "'{}' is hidden and will not be cut.",
                one_line(&o.name)
            ));
            continue;
        }
        if !o.cut_enabled {
            facts.skipped.push(o.id);
            choice_warnings += 1;
            warnings.push(format!(
                "'{}' is skipped and will not be cut.",
                one_line(&o.name)
            ));
            continue;
        }
        let before = placed.len();
        place_pieces(o.id, cut.pieces(o.id), &mut placed);
        let PieceFacts { no_area, slivers } = cut.facts(o.id);
        if slivers > 0 {
            warnings.push(cutgeom::sliver_warning(&o.name, slivers, cut.min_width_mm));
            if placed.len() > before {
                // The issue says the same as the warning just pushed.
                covered_warnings.push(warnings.len() - 1);
                issues.extend(report::thin_slivers_issue(input, o, slivers));
            }
        }
        if no_area > 0 {
            let what = if no_area == 1 {
                "1 filled path has no area or is thinner than the cut tolerance".to_string()
            } else {
                format!("{no_area} filled paths have no area or are thinner than the cut tolerance")
            };
            let end = if no_area == 1 { "is" } else { "are" };
            warnings.push(format!(
                "'{}': {what} ({} mm) and {end} not cut.",
                one_line(&o.name),
                plotter.flatten_tolerance_mm
            ));
        }
        if placed.len() == before {
            facts.nothing_placed.push((o.id, no_area));
        } else {
            facts.placed.push(o.id);
            if no_area > 0 {
                // The no-area warning just pushed says the same.
                covered_warnings.push(warnings.len() - 1);
                issues.extend(report::fill_ignored_issue(
                    input,
                    o,
                    no_area,
                    plotter.flatten_tolerance_mm,
                ));
            }
        }
        if placed.len() == before {
            warnings.push(format!(
                "'{}' has no cut paths, so nothing of it is cut.",
                one_line(&o.name)
            ));
        }
    }

    let passes = job.passes.unwrap_or(material.passes);
    let [lo, hi] = plotter.passes_range;
    if passes < lo.max(1) || passes > hi {
        let from = if job.passes.is_some() {
            "The pass override"
        } else {
            "The material"
        };
        return Err(CommandError::Refused(format!(
            "{from} asks for {passes} passes, but this plotter takes {} to {hi}. Set passes in the job settings or pick another material.",
            lo.max(1)
        ))
        .into());
    }
    let strategy = StrategySettings::for_strategy(job.strategy);
    let pipeline = PipelineSettings {
        flatten_tolerance_mm: plotter.flatten_tolerance_mm,
        join_tolerance_mm: JOIN_TOLERANCE_MM,
        overcut_mm: material.overcut_mm,
        passes,
        reverse_alternate_passes: material.reverse_alternate_passes,
        strategy,
        direction: plotter.ring_direction,
        compensation: compensation(plotter, media, &strategy),
        corner_mode: corner_mode(material.corner_mode),
        // M3 sets the alignment stroke.
        alignment_stroke: None,
        // Without a mat the feed motor moves the sheet: cut it in bands.
        monotonic_feed: media.kind == MediaKind::NoMat,
        // Half a plotter step: anything shorter moves no motor at all.
        min_cut_mm: 0.5 / plotter.units_per_mm,
        weed: job.weed,
        weed_units: input.weed_units(),
        keep_modes: input.keep_modes(),
        ..PipelineSettings::with_defaults(area, plotter.home_mm(media))
    };
    let cut_plan = plan(&placed, &pipeline).map_err(|e| match e {
        PlanError::Empty => {
            let nothing_to_cut = true;
            let mut text = String::from("Nothing to cut: no visible object has cut paths.");
            for w in &warnings {
                text.push(' ');
                text.push_str(w);
            }
            BuildFailure {
                error: CommandError::Refused(text),
                nothing_to_cut,
                issues: report::nothing_to_cut(input, &facts, false),
                warnings: warnings.clone(),
            }
        }
        PlanError::TooShort {
            dropped,
            min_cut_mm,
        } => {
            let mut text = format!(
                "Nothing to cut: {} shorter than half a plotter step ({min_cut_mm} mm).",
                short_paths(dropped)
            );
            for w in &warnings {
                text.push(' ');
                text.push_str(w);
            }
            BuildFailure {
                error: CommandError::Refused(text),
                nothing_to_cut: true,
                issues: report::nothing_to_cut(input, &facts, true),
                warnings: warnings.clone(),
            }
        }
        PlanError::OutOfArea { objects } => {
            let names: Vec<String> = objects
                .iter()
                .map(|id| {
                    names
                        .get(id)
                        .map_or_else(|| format!("object {}", id.0), |n| one_line(n))
                })
                .collect();
            BuildFailure {
                error: CommandError::Refused(format!(
                    "These objects are outside the cutting area: {}. Move or scale them, or pick a bigger mat.",
                    names.join(", ")
                )),
                nothing_to_cut: false,
                issues: report::out_of_area(input, &objects),
                warnings: warnings.clone(),
            }
        }
        other @ PlanError::InvalidSettings { .. } => {
            CommandError::Refused(other.to_string()).into()
        }
    })?;
    if cut_plan.dropped_short > 0 {
        warnings.push(dropped_short_warning(cut_plan.dropped_short));
    }

    if cut_plan.dropped_weed_boxes > 0 {
        warnings.push(weed_box_warning(
            cut_plan.dropped_weed_boxes,
            job.weed.per_object,
        ));
    }
    if cut_plan.blocked_weed_boxes > 0 {
        warnings.push(weed_box_blocked_warning(cut_plan.blocked_weed_boxes));
    }

    let settings = CutSettings {
        speed: job.speed.unwrap_or(material.speed),
        force: job.force.unwrap_or(material.force),
        track_enhancing: material.track_enhancing && plotter.capabilities.track_enhancing,
    };
    let emitted = emit_plan(&cut_plan, objects, &cut, plotter, &media_id, &settings)?;
    let not_cut = paths_not_cut(&cut_plan, &emitted.collapsed);
    if not_cut > 0 {
        warnings.push(collapsed_warning(not_cut));
    }
    // Every mode: the blade rounds tight curves with or without host
    // compensation.
    let min_radius_mm = material.min_radius_mm.unwrap_or(plotter.blade_offset_mm);
    let tight = tight_curves(&cut_plan, min_radius_mm);
    if tight > 0 {
        warnings.push(tight_curves_warning(tight, min_radius_mm));
    }
    // Plan decision (spec 8.5 vs 9): host compensation steers with the
    // profile's offset, the value the offset series tunes; the simulator
    // and the radius warning use the physical blade offset.
    if let KnifeCompensation::Host { offset_mm, .. } = plotter.knife_compensation
        && (offset_mm - plotter.blade_offset_mm).abs() > OFFSET_SAME_MM
    {
        warnings.push(offset_mismatch_warning(offset_mm, plotter.blade_offset_mm));
    }
    Ok(JobResult {
        plan: cut_plan,
        job: emitted,
        settings,
        warnings,
        choice_warnings,
        issues,
        covered_warnings,
        sent: None,
        built_for: plotter.clone(),
    })
}

/// Places the cut pieces of object `id` (in mat millimetres, the grid's
/// 0,0, y down; see [`CutGeometry`]): what the cut and the export both
/// start from. A placed path's `path_index` is the index of its piece.
pub(crate) fn place_pieces(id: ObjectId, pieces: &[CutPiece], placed: &mut Vec<PlacedPath>) {
    for (i, p) in pieces.iter().enumerate() {
        placed.push(PlacedPath {
            path: p.path.clone(),
            source: PathRef {
                object: id,
                path_index: i,
            },
        });
    }
}

/// "the only cut path is" or "all {n} cut paths are".
fn short_paths(n: usize) -> String {
    if n == 1 {
        "the only cut path is".to_string()
    } else {
        format!("all {n} cut paths are")
    }
}

/// `name` reduced to `[A-Za-z0-9._-]` (other characters become `_`),
/// without leading dots, at most 100 characters; empty gives `flatcraft-job`.
pub fn sanitize_name(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed: String = mapped
        .trim_start_matches('.')
        .chars()
        .take(MAX_NAME_LEN)
        .collect();
    if trimmed.is_empty() {
        DEFAULT_NAME.to_string()
    } else {
        trimmed
    }
}

/// Where File Output writes: the profile directory, else the preference,
/// else `<Documents>/FlatCraft Output`, else `<config dir>/output` (for
/// systems without a Documents folder).
pub fn output_dir(
    profile_dir: Option<&Path>,
    prefs_dir: Option<&Path>,
    documents: Option<&Path>,
    config_dir: &Path,
) -> PathBuf {
    profile_dir
        .or(prefs_dir)
        .map(Path::to_path_buf)
        .or_else(|| documents.map(|d| d.join(OUTPUT_FOLDER)))
        .unwrap_or_else(|| config_dir.join("output"))
}

/// Where File Output writes for this profile folder and preference, with
/// the platform Documents folder. The Cut workspace shows this folder.
pub fn file_output_dir(
    profile_dir: Option<&Path>,
    prefs_dir: Option<&Path>,
    config_dir: &Path,
) -> PathBuf {
    output_dir(
        profile_dir,
        prefs_dir,
        documents_dir().as_deref(),
        config_dir,
    )
}

/// The platform Documents folder, if there is one.
fn documents_dir() -> Option<PathBuf> {
    directories::UserDirs::new().and_then(|u| u.document_dir().map(Path::to_path_buf))
}

/// True if an agent may write File Output into the folder `given`, which
/// resolves to `target` (see `resolve_strict`): anywhere outside the config
/// dir, and inside it only below `<config dir>/output`.
fn agent_may_write(target: &Path, given: &Path, config_dir: &Path) -> bool {
    if !in_config(target, given, config_dir) {
        return true;
    }
    let cfg = std::path::absolute(config_dir).unwrap_or_else(|_| config_dir.to_path_buf());
    target.starts_with(resolved(&cfg).join("output"))
        && lexical(given).starts_with(lexical(&cfg).join("output"))
}

/// The job name: the given one, else the project file stem, else
/// `flatcraft-job`; always sanitised.
fn job_name(s: &Session, name: Option<String>) -> String {
    let raw = name.or_else(|| {
        s.doc()
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|n| n.to_string_lossy().into_owned())
    });
    sanitize_name(raw.as_deref().unwrap_or(DEFAULT_NAME))
}

fn plotter(s: &Session) -> Result<PlotterProfile, CommandError> {
    s.profiles
        .plotter(&s.doc().document.job.plotter_id)
        .map_err(profile_error)
}

/// The summary of a built job, for the confirmation dialog and the Cut
/// workspace.
pub fn summary(result: &JobResult) -> JobSummary {
    JobSummary {
        toolpaths: result.toolpaths_cut(),
        cut_mm: result.plan.cut_mm,
        travel_mm: result.plan.travel_mm,
        warnings: result.warnings.clone(),
        speed: result.settings.speed,
        force: result.settings.force,
        // A ring of chained laps carries several passes in one toolpath.
        passes: result
            .plan
            .toolpaths
            .iter()
            .map(|t| last_pass(t.pass, t.laps))
            .max()
            .unwrap_or(0),
    }
}

/// The reply of a successful send.
pub(crate) fn sent_json(report: &SendReport, summary: &JobSummary) -> Value {
    json!({
        "files": report.files,
        "bytes_sent": report.bytes_sent,
        "toolpaths": summary.toolpaths,
        "cut_mm": summary.cut_mm,
        "travel_mm": summary.travel_mm,
        "warnings": summary.warnings,
    })
}

/// Builds the active document's job and sends it ([`send_built`]).
pub(crate) fn send(
    s: &mut Session,
    name: Option<String>,
    origin: Origin,
) -> Result<Value, CommandError> {
    let plotter = plotter(s)?;
    let name = job_name(s, name);
    // A send the gate refuses says so before any build error.
    if let GateDecision::Refuse(reason) =
        gate::check_send(origin, &plotter.transport, &s.prefs, s.has_ui)
    {
        return Err(CommandError::Refused(reason));
    }
    let result = build(s)?;
    send_built(s, &plotter, result, name, origin)
}

/// Sends a built job of the active document: the one way a job leaves
/// FlatCraft. It runs the send gate, then the safety check before anything
/// waits or goes out. When the person has to confirm, the checked job is
/// stored as the pending send (a snapshot: confirming sends exactly these
/// bytes) and nothing is sent. Otherwise it is delivered through the
/// plotter's transport, logged and kept as the document's last sent job.
/// `result` must be built for `plotter` with its current settings (id and
/// every value): anything else is an internal failure and nothing is sent.
pub fn send_built(
    s: &mut Session,
    plotter: &PlotterProfile,
    mut result: JobResult,
    name: String,
    origin: Origin,
) -> Result<Value, CommandError> {
    if result.built_for != *plotter {
        return Err(CommandError::Failed(BUILT_FOR_OTHER_PLOTTER.into()));
    }
    let decision = gate::check_send(origin, &plotter.transport, &s.prefs, s.has_ui);
    if let GateDecision::Refuse(reason) = decision {
        return Err(CommandError::Refused(reason));
    }
    // Only a checked job can be sent or wait for confirmation.
    let job = checked(result.job.clone())?;
    let sum = summary(&result);
    let log = log_entry(s, &sum, result.job.bytes.len(), plotter, &name);
    if decision == GateDecision::NeedsConfirmation {
        s.request_confirmation(
            name,
            plotter.id.clone(),
            plotter.transport.clone(),
            job,
            sum,
            log,
        );
        return Err(CommandError::Refused(WAITING.into()));
    }
    let report = deliver_built(s, &plotter.transport, &job, &name, origin)?;
    let mut value = sent_json(&report, &sum);
    log_sent(s, log, &plotter.transport, &report, &mut value);
    result.sent = Some(report);
    s.doc_mut().set_last_sent(result);
    Ok(value)
}

/// The cut log entry of the active document's job, without its id, time
/// and target, which [`log_sent`] fills in once the job went out.
fn log_entry(
    s: &Session,
    summary: &JobSummary,
    bytes: usize,
    plotter: &PlotterProfile,
    name: &str,
) -> cutlog::CutEntry {
    let st = s.doc();
    let job = &st.document.job;
    let material = s.profiles.material(&job.material_id).ok();
    let mat = crate::commands::effective_media(plotter, job)
        .ok()
        .map(|(m, _)| (m.id.clone(), m.name.clone()));
    let (mat_id, mat) = mat.unwrap_or_else(|| (job.media_id.clone(), job.media_id.clone()));
    cutlog::CutEntry {
        id: String::new(),
        date: String::new(),
        document: st
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or_else(|| name.to_string(), |n| n.to_string_lossy().into_owned()),
        path: st.path.clone(),
        plotter_id: plotter.id.clone(),
        plotter: plotter.name.clone(),
        material_id: job.material_id.clone(),
        material: material.map_or_else(|| job.material_id.clone(), |m| m.name),
        mat_id,
        mat,
        speed: summary.speed,
        force: summary.force,
        passes: summary.passes,
        strategy: job.strategy,
        keep_mode: keep_mode_of(&st.document),
        size: cutlog::JobSize {
            bytes,
            toolpaths: summary.toolpaths,
            cut_mm: summary.cut_mm,
            travel_mm: summary.travel_mm,
        },
        target: cutlog::Target::Usb,
        outcome: None,
    }
}

/// The keep mode of the objects a job cuts (visible, not skipped), for
/// the cut log: its name when they all share it, "mixed" when they do
/// not, `None` when nothing is cut.
fn keep_mode_of(doc: &Document) -> Option<String> {
    let mut modes = doc
        .objects
        .iter()
        .filter(|o| o.visible && o.cut_enabled)
        .map(|o| o.keep);
    let first = modes.next()?;
    Some(if modes.all(|m| m == first) {
        first.key().to_string()
    } else {
        "mixed".to_string()
    })
}

/// Appends a completed send to the cut log and says so in the reply
/// (`cut_log_entry`). A log that cannot be written never fails the send:
/// the reply says why instead (`cut_log_error`).
pub(crate) fn log_sent(
    s: &mut Session,
    mut entry: cutlog::CutEntry,
    transport: &TransportSpec,
    report: &SendReport,
    value: &mut Value,
) {
    entry.id = cutlog::new_id();
    entry.date = cutlog::now();
    entry.target = match transport {
        TransportSpec::File { .. } => cutlog::Target::File {
            files: report.files.clone(),
        },
        TransportSpec::Usb { .. } => cutlog::Target::Usb,
    };
    match s.log_cut(&entry) {
        Ok(()) => value["cut_log_entry"] = entry.id.into(),
        Err(e) => {
            value["cut_log_error"] =
                format!("The job was sent, but the cut log could not be written: {e}").into();
        }
    }
}

/// Runs the safety check. Only a checked job can be sent or wait for
/// confirmation, so an unsafe job is refused at once and nothing is sent.
fn checked(job: EmittedJob) -> Result<CheckedJob, CommandError> {
    CheckedJob::check(job).map_err(|e| CommandError::Failed(TransportError::Unsafe(e).to_string()))
}

/// Sends an already built job through `transport`. The gate must have
/// allowed it, or the person confirmed it (`confirm_pending`).
pub(crate) fn deliver_built(
    s: &mut Session,
    transport: &TransportSpec,
    job: &CheckedJob,
    name: &str,
    origin: Origin,
) -> Result<SendReport, CommandError> {
    match transport {
        TransportSpec::File {
            directory,
            extension,
        } => {
            // Only the folder that is actually used must be absolute.
            let picked = match (directory.as_deref(), s.prefs.output_dir.as_deref()) {
                (Some(d), _) => Some(("The plotter profile's output folder", d)),
                (None, Some(d)) => Some(("The output folder in the preferences", d)),
                (None, None) => None,
            };
            if let Some((what, dir)) = picked
                && !dir.is_absolute()
            {
                return Err(CommandError::Refused(format!(
                    "{what} '{}' is not an absolute path. Set a full path, like /Users/me/Cuts or C:\\Cuts.",
                    dir.display()
                )));
            }
            let dir = file_output_dir(
                directory.as_deref(),
                s.prefs.output_dir.as_deref(),
                &s.config_dir,
            );
            // Resolved once: the check and the write see the same folder, so
            // a link swapped in after the check cannot redirect the job. A
            // folder that cannot be resolved (a `..` after a folder that does
            // not exist) is never guessed: agents are refused, people get an
            // error.
            let target = match resolve_strict(&dir, usize::MAX) {
                Ok(target) => target,
                Err(_) if origin == Origin::Agent => {
                    return Err(CommandError::Refused(format!(
                        "agents may not write into the FlatCraft config directory ({}), and the output folder {} cannot be resolved",
                        s.config_dir.display(),
                        dir.display()
                    )));
                }
                Err(e) => {
                    return Err(CommandError::Failed(format!(
                        "cannot use the output folder {}: {e}",
                        dir.display()
                    )));
                }
            };
            if origin == Origin::Agent && !agent_may_write(&target, &dir, &s.config_dir) {
                return Err(CommandError::Refused(format!(
                    "agents may not write into the FlatCraft config directory ({})",
                    s.config_dir.display()
                )));
            }
            #[cfg(test)]
            crate::commands::project::tests::between_check_and_write();
            // Reports and errors name the folder as the person gave it.
            FileTransport::new(target, extension.clone())
                .with_shown_dir(dir)
                .send_job(job, name, &mut s.byte_log)
        }
        TransportSpec::Usb { .. } => Err(TransportError::NotAvailable(USB_NOT_AVAILABLE.into())),
    }
    .map_err(transport_error)
}

fn transport_error(e: TransportError) -> CommandError {
    match e {
        TransportError::NotAvailable(msg) => CommandError::Refused(msg),
        e @ TransportError::Unsafe(_) => CommandError::Failed(e.to_string()),
        TransportError::Io(msg) => CommandError::Failed(msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weed_box_warnings_read_right() {
        assert_eq!(
            weed_box_blocked_warning(1),
            "A weed box would cut through material you keep, so it is not cut."
        );
        assert!(weed_box_blocked_warning(2).starts_with("2 weed boxes would cut"));
        assert!(weed_box_warning(1, false).starts_with("The weed box does not fit"));
        assert!(weed_box_warning(3, true).starts_with("3 weed boxes do not fit"));
    }

    const SQUARE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="20" y="20" width="30" height="30"/></svg>"#;

    /// A link in the output folder's path re-pointed into the config dir
    /// between the agent check and the write: the job still goes to the
    /// folder that was checked, and the report names the folder as given.
    #[cfg(unix)]
    #[test]
    fn a_link_re_pointed_after_the_check_does_not_redirect_file_output() {
        let config = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let real = elsewhere.path().join("real");
        std::fs::create_dir_all(real.join("jobs")).unwrap();
        let via = elsewhere.path().join("via");
        std::os::unix::fs::symlink(&real, &via).unwrap();
        let out = via.join("jobs");
        let profiles = flatcraft_device::ProfileStore::load(config.path()).unwrap();
        let prefs = crate::prefs::Preferences {
            output_dir: Some(out.clone()),
            ..Default::default()
        };
        let mut s = Session::new(profiles, prefs, false, config.path().to_path_buf());
        s.execute(
            "object.import_svg",
            json!({ "svg": SQUARE, "name": "sq" }),
            Origin::User,
        )
        .unwrap();
        let cfg = config.path().to_path_buf();
        std::fs::create_dir(cfg.join("jobs")).unwrap();
        let link = via.clone();
        crate::commands::project::tests::set_between(move || {
            std::fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink(&cfg, &link).unwrap();
        });
        let r = s
            .execute("job.send", json!({ "name": "swap" }), Origin::Agent)
            .unwrap();
        assert!(
            !config.path().join("jobs/swap.gpgl").exists(),
            "the job went through the re-pointed link"
        );
        assert!(real.join("jobs/swap.gpgl").exists());
        let files: Vec<PathBuf> = r["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| PathBuf::from(f.as_str().unwrap()))
            .collect();
        assert_eq!(files[0], out.join("swap.gpgl"), "the folder as given");
    }

    /// Errors name the output folder as the person set it, not the folder
    /// it resolves to.
    #[cfg(unix)]
    #[test]
    fn file_output_errors_name_the_folder_as_given() {
        let config = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let real = elsewhere.path().join("real");
        std::fs::create_dir(&real).unwrap();
        // A file where the job folder should be: creating it fails.
        std::fs::write(real.join("jobs"), "not a folder").unwrap();
        let via = elsewhere.path().join("via");
        std::os::unix::fs::symlink(&real, &via).unwrap();
        let profiles = flatcraft_device::ProfileStore::load(config.path()).unwrap();
        let prefs = crate::prefs::Preferences {
            output_dir: Some(via.join("jobs")),
            ..Default::default()
        };
        let mut s = Session::new(profiles, prefs, false, config.path().to_path_buf());
        s.execute(
            "object.import_svg",
            json!({ "svg": SQUARE, "name": "sq" }),
            Origin::User,
        )
        .unwrap();
        let e = s
            .execute("job.send", json!({ "name": "named" }), Origin::User)
            .unwrap_err()
            .to_string();
        assert!(e.contains(&via.join("jobs").display().to_string()), "{e}");
        assert!(!e.contains("real"), "{e}");
    }

    /// An output folder an agent names with a `..` after a folder that does
    /// not exist is refused, not guessed.
    #[cfg(unix)]
    #[test]
    fn an_unresolvable_output_folder_is_not_guessed() {
        let config = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(config.path(), elsewhere.path().join("cfglink")).unwrap();
        let dir = elsewhere.path().join("cfglink/nonexist/../jobs");
        let profiles = flatcraft_device::ProfileStore::load(config.path()).unwrap();
        let prefs = crate::prefs::Preferences {
            output_dir: Some(dir),
            ..Default::default()
        };
        let mut s = Session::new(profiles, prefs, false, config.path().to_path_buf());
        s.execute(
            "object.import_svg",
            json!({ "svg": SQUARE, "name": "sq" }),
            Origin::User,
        )
        .unwrap();
        let e = s
            .execute("job.send", json!({ "name": "dots" }), Origin::Agent)
            .unwrap_err();
        assert!(matches!(e, CommandError::Refused(_)), "{e}");
        assert!(!config.path().join("jobs").exists());
    }

    #[test]
    fn user_fixable_emit_errors_are_refusals() {
        let refused = [
            EmitError::OutOfRange {
                field: "speed",
                value: 0,
                min: 1,
                max: 10,
            },
            EmitError::CoordinateOutOfRange {
                note: "n".into(),
                axis: "x",
                mm: -1.0,
                origin_mm: 0.0,
                max_mm: 300.0,
                value: -20.0,
                max: 6000,
            },
            EmitError::UnknownMedia { id: "m".into() },
            EmitError::EmptyJob,
            EmitError::CompensationMismatch {
                profile: "host",
                geometry: "plain tip paths",
            },
        ];
        for e in refused {
            assert!(matches!(emit_error(e), CommandError::Refused(_)));
        }
        let failed = [
            EmitError::DialectNotSupported,
            EmitError::EmptyDraw,
            EmitError::InvalidProfile { reason: "r".into() },
        ];
        for e in failed {
            assert!(matches!(emit_error(e), CommandError::Failed(_)));
        }
    }

    #[test]
    fn the_emitted_geometry_is_what_the_plan_applied() {
        assert_eq!(geometry_kind(AppliedCompensation::Tip), GeometryKind::Tip);
        assert_eq!(
            geometry_kind(AppliedCompensation::Host { offset_mm: 0.3 }),
            GeometryKind::HostAxis { offset_mm: 0.3 }
        );
        assert_eq!(
            geometry_kind(AppliedCompensation::HostCalibration),
            GeometryKind::HostCalibration
        );
        assert!(!collapsed_warning(2).contains('\u{2014}'));
    }

    /// A mismatch the person can fix says how, in plain words.
    #[test]
    fn compensation_mismatches_say_the_fix() {
        let series = emit_error(EmitError::CompensationMismatch {
            profile: "firmware",
            geometry: GeometryKind::HostCalibration.describe(),
        });
        let CommandError::Refused(text) = series else {
            panic!("{series}")
        };
        assert!(text.contains("needs Host knife compensation"), "{text}");
        let stale = emit_error(EmitError::CompensationMismatch {
            profile: "host",
            geometry: GeometryKind::Tip.describe(),
        });
        let CommandError::Refused(text) = stale else {
            panic!("{stale}")
        };
        assert!(text.ends_with("Build the job again, so it is planned for this plotter preset."));
        for t in [
            dropped_short_warning(1),
            dropped_short_warning(3),
            collapsed_warning(1),
            collapsed_warning(3),
            tight_curves_warning(2, 0.25),
            offset_mismatch_warning(0.3, 0.25),
        ] {
            assert!(!t.contains('\u{2014}'), "{t}");
        }
        assert_eq!(
            dropped_short_warning(2),
            "2 paths are shorter than half a plotter step and are not cut."
        );
        assert_eq!(
            tight_curves_warning(3, 0.25),
            "3 curves are tighter than 0.25 mm. They may come out rounded or torn."
        );
        assert_eq!(
            offset_mismatch_warning(0.3, 0.25),
            "Host compensation uses 0.3 mm, the blade offset is 0.25 mm. The simulator uses the blade offset."
        );
    }

    #[test]
    fn unsafe_bytes_never_wait_for_confirmation() {
        let bad = EmittedJob {
            bytes: b"FB1\x03H\x03".to_vec(),
            listing: vec![],
            collapsed: vec![],
        };
        let e = checked(bad).unwrap_err();
        assert!(matches!(e, CommandError::Failed(_)), "{e}");
        assert!(e.to_string().contains("safety check"), "{e}");
        let unterminated = EmittedJob {
            bytes: b"\x1b\x04".to_vec(),
            listing: vec![],
            collapsed: vec![],
        };
        assert!(checked(unterminated).is_err());
        let good = EmittedJob {
            bytes: b"\x1b\x04H\x03".to_vec(),
            listing: vec![],
            collapsed: vec![],
        };
        checked(good).unwrap();
    }

    #[test]
    fn unsafe_bytes_keep_the_transport_text() {
        let e = TransportError::Unsafe(flatcraft_device::gpgl::SafetyError::NotTerminated);
        assert_eq!(
            transport_error(e.clone()),
            CommandError::Failed(e.to_string())
        );
    }
}
