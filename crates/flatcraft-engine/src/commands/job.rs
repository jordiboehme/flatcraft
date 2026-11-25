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
    description: "Plans the cut of the active document and builds the GPGL bytes without sending anything. Only visible objects are cut; they must lie inside the cutting area. Speed, force and passes come from the material for this plotter, then the job overrides. Returns {toolpaths (count), cut_mm (blade-down length incl. overcut and passes), travel_mm, warnings: [text], settings: {speed, force, passes, track_enhancing}, bytes (job size), listing_total, listing: [{command, note}] (the first 200 commands; notes read \"object <id> '<name>', path <i> [<label>], pass <p>\", or \"..., passes 1-<n>\" for a closed path cut <n> times in a row without lifting the blade)}.",
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
    description: "Builds the job of the active document (as job.preview) and sends it through the plotter's transport. File Output writes <name>.<ext> (the exact bytes), <name>.<ext>.txt (a readable listing) and <name>.bytelog.jsonl into the output folder; a taken name gets -2, -3 and so on. An output folder with a '..' after a folder that does not exist is refused. USB cutting arrives in milestone 3. For a USB plotter an agent needs the preference 'Allow agents to start cuts'; without it a session with a window asks the person to confirm (the call returns a refusal that says it is waiting) and a headless session refuses. Agents cannot confirm a cut themselves; a waiting request expires after 5 minutes. Returns {files: [path], bytes_sent, toolpaths, cut_mm, travel_mm, warnings: [text]}.",
);
