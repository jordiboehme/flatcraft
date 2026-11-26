//! Job preview and send.

use serde::Deserialize;
use serde_json::{Value, json};

use super::{NoParams, command};
use crate::session::{CommandError, Origin, Session};

/// Most listing lines `job.preview` returns.
const PREVIEW_LINES: usize = 200;

fn run_preview(s: &mut Session, _: NoParams, _: Origin) -> Result<Value, CommandError> {
    let result = crate::job::build(s)?;
    let passes = crate::job::summary(&result).passes;
    let listing: Vec<Value> = result
        .job
        .listing
        .iter()
        .take(PREVIEW_LINES)
        .map(|l| json!({ "command": l.command, "note": l.note }))
        .collect();
    let value = json!({
        "toolpaths": result.plan.toolpaths.len(),
        "cut_mm": result.plan.cut_mm,
        "travel_mm": result.plan.travel_mm,
        "warnings": result.warnings,
        "settings": {
            "speed": result.settings.speed,
            "force": result.settings.force,
            "passes": passes,
            "track_enhancing": result.settings.track_enhancing,
        },
        "bytes": result.job.bytes.len(),
        "listing_total": result.job.listing.len(),
        "listing": listing,
    });
    s.doc_mut().set_last_job(result);
    Ok(value)
}

command!(
    JobPreview,
    id: "job.preview",
    title: "Preview job",
    mutates: false,
    params: NoParams,
    run: run_preview,
    description: "Plans the cut of the active document and builds the GPGL bytes without sending anything. Read the project notes (document.notes or document.state) before a cut. Only visible objects are cut; they must lie inside the cutting area. Speed, force and passes come from the material for this plotter, then the job overrides. Returns {toolpaths (count), cut_mm (blade-down length incl. overcut and passes), travel_mm, warnings: [text], settings: {speed, force, passes, track_enhancing}, bytes (job size), listing_total, listing: [{command, note}] (the first 200 commands; notes read \"object <id> '<name>', path <i> [<label>], pass <p>\", or \"..., passes 1-<n>\" for a closed path cut <n> times in a row without lifting the blade)}.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendParams {
    /// Job name for the output files. Default: the project file name, else "flatcraft-job". Only letters, digits, '.', '_' and '-' are kept; other characters become '_'.
    pub name: Option<String>,
}

fn run_send(s: &mut Session, p: SendParams, origin: Origin) -> Result<Value, CommandError> {
    crate::job::send(s, p.name, origin)
}

command!(
    JobSend,
    id: "job.send",
    title: "Send job",
    mutates: false,
    session: true,
    params: SendParams,
    run: run_send,
    description: "Builds the job of the active document (as job.preview) and sends it through the plotter's transport. Read the project notes (document.notes or document.state) before a cut. File Output writes <name>.<ext> (the exact bytes), <name>.<ext>.txt (a readable listing) and <name>.bytelog.jsonl into the output folder; a taken name gets -2, -3 and so on. An output folder with a '..' after a folder that does not exist is refused. Every completed send adds an entry to the local cut log (see job.history); the reply then has cut_log_entry (its id), or cut_log_error when the log could not be written (the job was still sent). USB cutting arrives in milestone 3. For a USB plotter an agent needs the preference 'Allow agents to start cuts'; without it a session with a window asks the person to confirm (the call returns a refusal that says it is waiting) and a headless session refuses. Agents cannot confirm a cut themselves; a waiting request expires after 5 minutes. Returns {files: [path], bytes_sent, toolpaths, cut_mm, travel_mm, warnings: [text], cut_log_entry or cut_log_error}.",
);

/// Most entries `job.history` returns.
const HISTORY_MAX: usize = 500;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryParams {
    /// Only cuts with this material preset id.
    pub material_id: Option<String>,
    /// Most entries to return, newest first: 1 to 500 (default 20).
    pub limit: Option<usize>,
}

/// The entry as agents see it: the project file and the output files by
/// name only, never as absolute paths.
fn names_only(e: &crate::cutlog::CutEntry) -> crate::cutlog::CutEntry {
    let name = |p: &std::path::Path| -> std::path::PathBuf {
        p.file_name()
            .map(std::path::PathBuf::from)
            .unwrap_or_default()
    };
    let mut e = e.clone();
    e.path = e.path.as_deref().map(name);
    if let crate::cutlog::Target::File { files } = &mut e.target {
        *files = files.iter().map(|f| name(f)).collect();
    }
    e
}

fn run_history(s: &mut Session, p: HistoryParams, _: Origin) -> Result<Value, CommandError> {
    let limit = p.limit.unwrap_or(20);
    if !(1..=HISTORY_MAX).contains(&limit) {
        return Err(CommandError::InvalidParams(format!(
            "limit must be 1 to {HISTORY_MAX}"
        )));
    }
    let log = crate::cutlog::read(&s.config_dir);
    let matching: Vec<&crate::cutlog::CutEntry> = log
        .entries
        .iter()
        .rev()
        .filter(|e| p.material_id.as_ref().is_none_or(|m| e.material_id == *m))
        .collect();
    let entries: Vec<Value> = matching
        .iter()
        .take(limit)
        .map(|e| {
            let mut v = serde_json::to_value(names_only(e)).unwrap_or(Value::Null);
            v["outcome"] = e.outcome.as_ref().map_or(Value::Null, |o| {
                serde_json::to_value(o).unwrap_or(Value::Null)
            });
            v
        })
        .collect();
    Ok(json!({
        "total": matching.len(),
        "entries": entries,
        "diagnostics": log.diagnostics,
    }))
}

command!(
    JobHistory,
    id: "job.history",
    title: "Cut history",
    mutates: false,
    params: HistoryParams,
    run: run_history,
    description: "Reads the local cut log: one entry per completed send, newest first, with how it went when the person said so. Use it to find settings that worked for a material. Read-only: only the person records outcomes, in the window. Paths are file names only (the project file, the output files), never absolute paths. Returns {total (matching entries), entries: [{id, date (RFC 3339 UTC), document, path (project file name), plotter_id, plotter, material_id, material, mat_id, mat, speed, force, passes, strategy, keep_mode, size: {bytes, toolpaths, cut_mm, travel_mm}, target: {kind: file (with files: output file names) or usb}, outcome: {verdict: good|bad, note, date} or null}], diagnostics: [text] (log lines that could not be read and were skipped)}.",
);
