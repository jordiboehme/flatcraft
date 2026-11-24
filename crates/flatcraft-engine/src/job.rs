//! Job building: document -> cut plan -> GPGL bytes, and sending.

use std::path::{Path, PathBuf};

use flatcraft_core::document::ObjectId;
use flatcraft_device::gpgl::{CutSettings, EmitError, EmitPath, EmittedJob, check_safe, emit_job};
use flatcraft_device::{
    FileTransport, PlotterProfile, SendReport, Transport, TransportError, TransportSpec,
};
use flatcraft_path::{CutPlan, PathRef, PipelineSettings, PlacedPath, PlanError, plan};
use serde_json::{Value, json};

use crate::commands::profile_error;
use crate::commands::project::{inside, lexical, resolved};
use crate::gate::{self, GateDecision};
use crate::session::{CommandError, JobSummary, Origin, Session};

/// Join tolerance of the pipeline (spec default).
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

/// Listing note of one toolpath. Task 12 reads this format.
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

/// Builds the job of the active document: the visible objects' paths, the
/// material values for this plotter, then the job overrides.
pub fn build(session: &Session) -> Result<JobResult, CommandError> {
    let doc = &session.doc().document;
    let job = &doc.job;
    let plotter = session
        .profiles
        .plotter(&job.plotter_id)
        .map_err(profile_error)?;
    let material = session
        .profiles
        .material(&job.material_id)
        .map_err(profile_error)?
        .for_plotter(&plotter.id);
    let area = plotter.cutting_area(&job.media_id).ok_or_else(|| {
        CommandError::NotFound(format!(
            "media '{}' is not defined for plotter '{}'",
            job.media_id, job.plotter_id
        ))
    })?;

    let mut warnings = Vec::new();
    let mut placed = Vec::new();
    let mut labels = std::collections::BTreeMap::new();
    for o in &doc.objects {
        if !o.visible {
            warnings.push(format!(
                "'{}' is hidden and will not be cut.",
                one_line(&o.name)
            ));
            continue;
        }
        let paths = o.placed_paths();
        if paths.is_empty() {
            warnings.push(format!(
                "'{}' has no cut paths, so nothing of it is cut.",
                one_line(&o.name)
            ));
        }
        for (i, p) in paths.into_iter().enumerate() {
            if let Some(label) = p.label {
                labels.insert((o.id, i), label);
            }
            placed.push(PlacedPath {
                path: p.path,
                source: PathRef {
                    object: o.id,
                    path_index: i,
                },
            });
        }
    }

    let passes = job.passes.unwrap_or(material.passes);
    let pipeline = PipelineSettings {
        cutting_area: area,
        flatten_tolerance_mm: plotter.flatten_tolerance_mm,
        join_tolerance_mm: JOIN_TOLERANCE_MM,
        overcut_mm: material.overcut_mm,
        passes,
        reverse_alternate_passes: material.reverse_alternate_passes,
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
                    doc.get(*id)
                        .map_or_else(|| format!("object {}", id.0), |o| one_line(&o.name))
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
            let name = doc.get(id).map_or("", |o| o.name.as_str());
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
    let emitted = emit_job(&paths, &plotter, &job.media_id, &settings).map_err(emit_error)?;
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

/// The platform Documents folder, if there is one.
fn documents_dir() -> Option<PathBuf> {
    directories::UserDirs::new().and_then(|u| u.document_dir().map(Path::to_path_buf))
}

/// True if an agent may write File Output into `dir`: anywhere outside the
/// config dir, and inside it only below `<config dir>/output`.
fn agent_may_write(dir: &Path, config_dir: &Path) -> bool {
    if !inside(dir, config_dir) {
        return true;
    }
    let abs = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let dir = abs(dir);
    let cfg = abs(config_dir);
    resolved(&dir).starts_with(resolved(&cfg).join("output"))
        && lexical(&dir).starts_with(lexical(&cfg).join("output"))
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

/// The dialog summary of a built job.
pub(crate) fn summary(result: &JobResult) -> JobSummary {
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
            check_before_waiting(&result.job)?;
            let summary = summary(&result);
            s.request_confirmation(
                name,
                plotter.id.clone(),
                plotter.transport.clone(),
                result.job,
                summary,
            );
            Err(CommandError::Refused(WAITING.into()))
        }
        GateDecision::Allow => {
            let mut result = build(s)?;
            let report = deliver_built(s, &plotter.transport, &result.job, &name, origin)?;
            let value = sent_json(&report, &summary(&result));
            result.sent = Some(report);
            s.doc_mut().last_job = Some(result);
            Ok(value)
        }
    }
}

/// Only bytes that pass the safety check may wait for confirmation, so an
/// unsafe job is refused to the agent at once.
fn check_before_waiting(job: &EmittedJob) -> Result<(), CommandError> {
    check_safe(&job.bytes).map_err(|e| CommandError::Failed(TransportError::Unsafe(e).to_string()))
}

/// Sends an already built job through `transport`. The gate must have
/// allowed it, or the person confirmed it (`confirm_pending`).
pub(crate) fn deliver_built(
    s: &mut Session,
    transport: &TransportSpec,
    job: &EmittedJob,
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
            let dir = output_dir(
                directory.as_deref(),
                s.prefs.output_dir.as_deref(),
                documents_dir().as_deref(),
                &s.config_dir,
            );
            if origin == Origin::Agent && !agent_may_write(&dir, &s.config_dir) {
                return Err(CommandError::Refused(format!(
                    "agents may not write into the FlatCraft config directory ({})",
                    s.config_dir.display()
                )));
            }
            FileTransport::new(dir, extension.clone()).send_job(job, name, &mut s.byte_log)
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
        let e = check_before_waiting(&bad).unwrap_err();
        assert!(matches!(e, CommandError::Failed(_)), "{e}");
        assert!(e.to_string().contains("safety check"), "{e}");
        let unterminated = EmittedJob {
            bytes: b"\x1b\x04".to_vec(),
            listing: vec![],
        };
        assert!(check_before_waiting(&unterminated).is_err());
        let good = EmittedJob {
            bytes: b"\x1b\x04H\x03".to_vec(),
            listing: vec![],
        };
        check_before_waiting(&good).unwrap();
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
