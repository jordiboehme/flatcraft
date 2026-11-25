//! Job building: document -> cut plan -> GPGL bytes, and sending.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use flatcraft_core::document::{CutOptions, Document, JobSettings, ObjectId};
use flatcraft_core::geometry::CutPath;
use flatcraft_core::transform::Transform;
use flatcraft_device::gpgl::{CheckedJob, CutSettings, EmitError, EmitPath, EmittedJob, emit_job};
use flatcraft_device::{
    FileTransport, MaterialProfile, PlotterProfile, SendReport, Transport, TransportError,
    TransportSpec,
};
use flatcraft_path::{CutPlan, PathRef, PipelineSettings, PlacedPath, PlanError, plan};
use serde_json::{Value, json};

use crate::commands::profile_error;
use crate::commands::project::{in_config, lexical, resolve_strict, resolved};
use crate::cutgeom;
use crate::gate::{self, GateDecision};
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
    pub sent: Option<SendReport>,
}

/// Replaces line breaks and other control characters with spaces, so a note
/// stays on one listing line.
fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Listing note of one toolpath. The File Output listing, `job.preview`
/// and the Debug panel show it to people and agents, so keep the format
/// stable.
fn note(id: ObjectId, name: &str, path: usize, label: Option<&str>, pass: u32) -> String {
    let label = label
        .map(|l| format!(" [{}]", one_line(l)))
        .unwrap_or_default();
    format!(
        "object {} '{}', path {path}{label}, pass {pass}",
        id.0,
        one_line(name)
    )
}

/// Problems the person can fix (settings, placement, media) are refusals;
/// broken profiles and internal errors are failures.
fn emit_error(e: EmitError) -> CommandError {
    let text = format!("cannot build the GPGL job: {e}");
    match e {
        EmitError::OutOfRange { .. }
        | EmitError::CoordinateOutOfRange { .. }
        | EmitError::UnknownMedia { .. }
        | EmitError::EmptyJob => CommandError::Refused(text),
        EmitError::HostCompensationNotYet
        | EmitError::DialectNotSupported
        | EmitError::EmptyDraw
        | EmitError::InvalidProfile { .. } => CommandError::Failed(text),
    }
}

/// One object as the job build reads it. The geometry `Arc` is shared with
/// the document, so taking a [`JobInput`] is cheap.
#[derive(Debug, Clone)]
pub struct JobObject {
    pub id: ObjectId,
    pub name: String,
    pub visible: bool,
    pub transform: Transform,
    pub local_bounds: kurbo::Rect,
    pub cut: CutOptions,
    pub geometry: Arc<Vec<CutPath>>,
}

/// Two inputs are equal when the same geometry `Arc` is placed the same
/// way. The comparison never walks the geometry, so a GUI can compare
/// inputs every frame; it holds the `Arc`s, so an address cannot be reused.
impl PartialEq for JobObject {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.visible == other.visible
            && self.transform == other.transform
            && self.local_bounds == other.local_bounds
            && self.cut == other.cut
            && Arc::ptr_eq(&self.geometry, &other.geometry)
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
}

impl JobInput {
    /// The objects of `doc` as the build reads them.
    fn objects_of(doc: &Document) -> Vec<JobObject> {
        doc.objects
            .iter()
            .map(|o| JobObject {
                id: o.id,
                name: o.name.clone(),
                visible: o.visible,
                transform: o.transform,
                local_bounds: o.local_bounds,
                cut: o.cut,
                geometry: o.geometry.clone(),
            })
            .collect()
    }

    /// The active document's objects, job settings and profiles.
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
        Ok(Self::from_parts(doc, plotter, &material))
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
        }
    }
}

/// Builds the job of the active document: the visible objects' cut paths
/// (per their cut options, see `cutgeom`), the material values for this
/// plotter, then the job overrides.
pub fn build(session: &Session) -> Result<JobResult, CommandError> {
    build_input(&JobInput::of(session)?)
}

/// Builds a job from a [`JobInput`]. It does not need the session, so the
/// GUI runs it without holding the session lock.
pub fn build_input(input: &JobInput) -> Result<JobResult, CommandError> {
    let JobInput {
        objects,
        job,
        plotter,
        material,
    } = input;
    let (media, mat_warning) = crate::commands::effective_media(plotter, job)?;
    let media_id = media.id.clone();
    let area = plotter
        .cutting_area(&media_id)
        .ok_or_else(|| CommandError::Failed("the mat has no cutting area".into()))?;
    let names: std::collections::BTreeMap<ObjectId, &str> =
        objects.iter().map(|o| (o.id, o.name.as_str())).collect();

    let mut warnings: Vec<String> = mat_warning.into_iter().collect();
    let mut placed = Vec::new();
    let mut labels = std::collections::BTreeMap::new();
    for o in objects {
        if !o.visible {
            warnings.push(format!(
                "'{}' is hidden and will not be cut.",
                one_line(&o.name)
            ));
            continue;
        }
        // One source path may give several cut paths (stroke outline
        // rings); they all keep the source path's index.
        let affine = o.transform.affine(o.local_bounds);
        let before = placed.len();
        let mut no_area = 0;
        for (i, p) in o.geometry.iter().enumerate() {
            if let Some(label) = &p.label {
                labels.insert((o.id, i), label.clone());
            }
            let cut = cutgeom::cut_path(p, &o.cut, affine, plotter.flatten_tolerance_mm);
            if cut.is_empty() && p.paint.stroke.is_none() && !p.path.elements().is_empty() {
                // A stroke-less fill without area, or thinner than the
                // tolerance, is not visible and gives nothing to cut.
                no_area += 1;
            }
            for path in cut {
                placed.push(PlacedPath {
                    path,
                    source: PathRef {
                        object: o.id,
                        path_index: i,
                    },
                });
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
        )));
    }
    let pipeline = PipelineSettings {
        cutting_area: area,
        flatten_tolerance_mm: plotter.flatten_tolerance_mm,
        join_tolerance_mm: JOIN_TOLERANCE_MM,
        overcut_mm: material.overcut_mm,
        passes,
        reverse_alternate_passes: material.reverse_alternate_passes,
        home: plotter.home_mm(media),
    };
    let cut_plan = plan(&placed, &pipeline).map_err(|e| match e {
        PlanError::Empty => {
            let mut text = String::from("Nothing to cut: no visible object has cut paths.");
            for w in &warnings {
                text.push(' ');
                text.push_str(w);
            }
            CommandError::Refused(text)
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
            CommandError::Refused(format!(
                "These objects are outside the cutting area: {}. Move or scale them, or pick a bigger mat.",
                names.join(", ")
            ))
        }
        other @ PlanError::InvalidSettings { .. } => CommandError::Refused(other.to_string()),
    })?;

    let paths: Vec<EmitPath> = cut_plan
        .toolpaths
        .iter()
        .map(|t| {
            let id = t.source.object;
            let name = names.get(&id).copied().unwrap_or("");
            let label = labels.get(&(id, t.source.path_index)).map(String::as_str);
            EmitPath {
                points: t.points.clone(),
                note: note(id, name, t.source.path_index, label, t.pass),
            }
        })
        .collect();
    let settings = CutSettings {
        speed: job.speed.unwrap_or(material.speed),
        force: job.force.unwrap_or(material.force),
        track_enhancing: material.track_enhancing && plotter.capabilities.track_enhancing,
    };
    let emitted = emit_job(&paths, plotter, &media_id, &settings).map_err(emit_error)?;
    Ok(JobResult {
        plan: cut_plan,
        job: emitted,
        settings,
        warnings,
        sent: None,
    })
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
        toolpaths: result.plan.toolpaths.len(),
        cut_mm: result.plan.cut_mm,
        travel_mm: result.plan.travel_mm,
        warnings: result.warnings.clone(),
        speed: result.settings.speed,
        force: result.settings.force,
        passes: result
            .plan
            .toolpaths
            .iter()
            .map(|t| t.pass)
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

/// Runs the gate, builds the active document's job and sends it. When the
/// person has to confirm, the built job is stored as the pending send and
/// nothing is sent; build errors come back right away.
pub(crate) fn send(
    s: &mut Session,
    name: Option<String>,
    origin: Origin,
) -> Result<Value, CommandError> {
    let plotter = plotter(s)?;
    let name = job_name(s, name);
    match gate::check_send(origin, &plotter.transport, &s.prefs, s.has_ui) {
        GateDecision::Refuse(reason) => Err(CommandError::Refused(reason)),
        GateDecision::NeedsConfirmation => {
            let result = build(s)?;
            let summary = summary(&result);
            let job = checked(result.job)?;
            s.request_confirmation(
                name,
                plotter.id.clone(),
                plotter.transport.clone(),
                job,
                summary,
            );
            Err(CommandError::Refused(WAITING.into()))
        }
        GateDecision::Allow => {
            let mut result = build(s)?;
            let job = checked(result.job.clone())?;
            let report = deliver_built(s, &plotter.transport, &job, &name, origin)?;
            let value = sent_json(&report, &summary(&result));
            result.sent = Some(report);
            s.doc_mut().set_last_sent(result);
            Ok(value)
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
        ];
        for e in refused {
            assert!(matches!(emit_error(e), CommandError::Refused(_)));
        }
        let failed = [
            EmitError::HostCompensationNotYet,
            EmitError::DialectNotSupported,
            EmitError::EmptyDraw,
            EmitError::InvalidProfile { reason: "r".into() },
        ];
        for e in failed {
            assert!(matches!(emit_error(e), CommandError::Failed(_)));
        }
    }

    #[test]
    fn unsafe_bytes_never_wait_for_confirmation() {
        let bad = EmittedJob {
            bytes: b"FB1\x03H\x03".to_vec(),
            listing: vec![],
        };
        let e = checked(bad).unwrap_err();
        assert!(matches!(e, CommandError::Failed(_)), "{e}");
        assert!(e.to_string().contains("safety check"), "{e}");
        let unterminated = EmittedJob {
            bytes: b"\x1b\x04".to_vec(),
            listing: vec![],
        };
        assert!(checked(unterminated).is_err());
        let good = EmittedJob {
            bytes: b"\x1b\x04H\x03".to_vec(),
            listing: vec![],
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
