//! Job preview and send.

use serde::Deserialize;
use serde_json::{Value, json};

use super::{NoParams, command};
use crate::session::{CommandError, Origin, Session};

/// Most listing lines `job.preview` returns.
const PREVIEW_LINES: usize = 200;

fn run_preview(s: &mut Session, _: NoParams, _: Origin) -> Result<Value, CommandError> {
    let input = crate::job::JobInput::of(s)?;
    let result = match crate::job::build_classified(&input) {
        Ok(r) => r,
        // A job that cannot be cut for reasons the report explains: the
        // reasons as data, so an agent can run their fixes.
        Err(f) if !f.issues.is_empty() => {
            let reason = match &f.error {
                CommandError::Refused(text) | CommandError::Failed(text) => text.clone(),
                other => other.to_string(),
            };
            return Ok(json!({
                "ready": false,
                "nothing_to_cut": f.nothing_to_cut,
                "reason": reason,
                "toolpaths": 0,
                "warnings": f.warnings,
                "issues": crate::report::issues_json(&f.issues),
            }));
        }
        Err(f) => return Err(f.error),
    };
    let passes = crate::job::summary(&result).passes;
    let listing: Vec<Value> = result
        .job
        .listing
        .iter()
        .take(PREVIEW_LINES)
        .map(|l| json!({ "command": l.command, "note": l.note }))
        .collect();
    let value = json!({
        "ready": true,
        "nothing_to_cut": false,
        "issues": crate::report::issues_json(&result.issues),
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
    description: "Plans the cut of the active document and builds the GPGL bytes without sending anything. Read the project notes (document.notes or document.state) before a cut. Only visible objects are cut; they must lie inside the cutting area. Speed, force and passes come from the material for this plotter, then the job overrides. When the job cannot be cut for a reason you can fix (nothing to cut, or objects outside the cutting area) it does not fail: it returns {ready: false, nothing_to_cut (true when no object gives anything to cut), reason (the refusal job.send would give), toolpaths: 0, warnings: [text], issues: [issue]} with every reason that applies, each with fixes to run (codes empty_document, hidden, skipped, fill_ignored, no_paths, too_small, outside_area, larger_than_area; see document.check for the issue format and the codes). Other problems (a broken profile, passes out of range) are still errors. A job that can be cut returns {ready: true, nothing_to_cut: false, issues (problems that do not stop the cut, like fill_ignored), toolpaths (count), cut_mm (blade-down length incl. overcut and passes), travel_mm, warnings: [text], settings: {speed, force, passes, track_enhancing}, bytes (job size), listing_total, listing: [{command, note}] (the first 200 commands; notes read \"object <id> '<name>', path <i> [<label>], pass <p>\", or \"..., passes 1-<n>\" for a closed path cut <n> times in a row without lifting the blade; weed paths read \"weed line <n>, pass <p>\", \"weed box <n>, pass <p>\" or, without a mat, \"weed box <n> piece, pass <p>\" for the band pieces of a box; with host knife compensation a path the blade lifts inside is cut as several runs, and each of their notes ends in \" run <k>\", counted from 1)}. Warnings also name paths too small for the plotter, curves tighter than the material's minimum radius, and a host compensation offset that differs from the blade offset.",
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
        "diagnostics": log.plain_diagnostics,
    }))
}

command!(
    JobHistory,
    id: "job.history",
    title: "Cut history",
    mutates: false,
    params: HistoryParams,
    run: run_history,
    description: "Reads the local cut log: one entry per completed send, newest first, with how it went when the person said so. Use it to find settings that worked for a material. Read-only: only the person records outcomes, in the window. Paths are file names only (the project file, the output files), never absolute paths. Returns {total (matching entries), entries: [{id, date (RFC 3339 UTC), document, path (project file name), plotter_id, plotter, material_id, material, mat_id, mat, speed, force, passes, strategy, keep_mode (pieces, stencil or both when every cut object has it, else mixed), size: {bytes, toolpaths, cut_mm, travel_mm}, target: {kind: file (with files: output file names) or usb}, outcome: {verdict: good|bad, note, date} or null}], diagnostics: [text] (log lines that could not be read and were skipped, like \"cut-log.jsonl line 3 was skipped (damaged)\")}.",
);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TroubleshootParams {
    /// What went wrong: not_through, tore_or_dragged, pieces_moved or wrong_place_or_size.
    pub symptom: crate::troubleshoot::Symptom,
}

fn run_troubleshoot(
    s: &mut Session,
    p: TroubleshootParams,
    _: Origin,
) -> Result<Value, CommandError> {
    let input = crate::job::JobInput::of(s)?;
    let log = s.cut_log();
    let last_good = log.last_good(&input.job.material_id);
    let list = crate::troubleshoot::checklist(&input, p.symptom, last_good);
    serde_json::to_value(list).map_err(|e| CommandError::Failed(e.to_string()))
}

command!(
    JobTroubleshoot,
    id: "job.troubleshoot",
    title: "It didn't cut right",
    mutates: false,
    params: TroubleshootParams,
    run: run_troubleshoot,
    description: "What to check when a cut went wrong, in order, for the active document: the same checklist the window's \"It didn't cut right...\" helper shows, so give the person this advice in this order. symptom: not_through (the pieces do not come out), tore_or_dragged (ragged edges, torn corners, material pulled along), pieces_moved (material or pieces shifted or lifted), wrong_place_or_size (wrong spot, wrong size or backwards). Returns {symptom, title, lead, settings: {material_id, material, plotter_id, plotter, speed, force, passes (what the job cuts with now), material_speed, material_force, material_passes, speed_range, force_range, passes_range, blade_depth_hint}, last_good: {entry, date, plotter_id, plotter, speed, force, passes, note} or null (the newest cut with this material the person rated good in the cut log), items: [{check, title, message, hint, likely, object_ids, fixes: [{label, description, command, args}], blocked: [{label, reason}]}]}. check is one of notes, last_good, blade_depth, force, passes, speed, blade, mat, material_face, tiny_details, cut_order, mirror, scale, position, test_shape. A fix is one registry command with absolute values: run it with execute {command, params: args}; each is one undo step. Speed, force and passes fixes set the job overrides (job.set_overrides), never the material preset; cut_order runs job.set_strategy, mirror runs object.mirror with mirrored true or false, test_shape runs tools.test_shape (add or move it) and object.set_cut (cut only the test shape). For wrong_place_or_size on heat transfer vinyl that is not mirrored, mirror comes first. blocked lists fixes that cannot run now, with the reason. The blade depth is a hint only: it is set by hand on the blade and never sent. Propose a change to the person before you run it; you cannot confirm a USB cut yourself.",
);
