//! `document.check`: the geometry checks of the import report, again.

use flatcraft_core::document::ObjectId;
use serde::Deserialize;
use serde_json::Value;

use super::{command, expand_ids};
use crate::job::JobInput;
use crate::report;
use crate::session::{CommandError, Origin, Session};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckParams {
    /// The objects (or groups) to check. Omitted or empty: every object of the document (not the selection).
    #[serde(default)]
    pub ids: Vec<u64>,
}

fn run_check(s: &mut Session, p: CheckParams, _: Origin) -> Result<Value, CommandError> {
    // A group id checks every object of the group, each on its own.
    let ids: Vec<ObjectId> = expand_ids(&s.doc().document, &p.ids)?;
    let input = JobInput::of(s)?;
    let only = (!ids.is_empty()).then_some(ids.as_slice());
    Ok(report::check(&input, only).to_json())
}

command!(
    DocumentCheck,
    id: "document.check",
    title: "Check objects",
    mutates: false,
    params: CheckParams,
    run: run_check,
    description: "Checks objects for what keeps them from cutting well: the geometry part of the import report, which object.import and object.import_svg return as report. Run it again after a fix to see what is left. Returns {shapes (cut shapes, closed or open), issues: [issue]}. The same issue format is used by the import commands (report.issues) and by job.preview (issues). An issue is {code, severity (info | warning | error), object_ids: [id], paths (how many paths or shapes, 0 when it does not count), message (plain text, the same the window shows), hint (what to do, in plain words), fixes: [{label, description, command, args}]}. A fix is a registry command with ready arguments: run it with execute {command, params: args}; the window's buttons run exactly these. Fixes use absolute sizes and positions, so running one twice does no harm. An empty fixes list means there is no automatic fix; follow the hint. A fix is never offered when running it would raise another issue whose fix undoes it (object outlines versus all lines, scale up versus scale to fit), so applying offered fixes always comes to an end. Cut options and scale are shared by linked copies: an issue whose fix changes them lists every member in object_ids (once for the group), and the fix description says so. object.reload, object.relink and object.retrace return the same report for the objects they read again. Codes found by document.check: open_paths (warning: filled paths cut as lines, or outlines with a small gap, so the piece stays on the sheet; fix object.set_cut_options mode object_outlines when that closes them), hairline (warning: strokes cut around both edges that are thinner than the minimum feature size, 6 times the blade offset, 1.5 mm by default; fix object.set_cut_options strokes centre_line), below_min_feature (warning: closed shapes smaller than the minimum feature size; fix object.scale when every shape of the object is that small), fill_ignored (warning, error when nothing else is cut: filled paths without area that object_outlines cuts nothing of; fix object.set_cut_options mode all_lines), outside_area (error: the object fits but is not inside the cutting area; fix object.move x_mm, y_mm), larger_than_area (error: the object is larger than the cutting area; fix object.scale to fit, about an anchor that keeps it inside when possible), stencil_island (info, never blocks the cut: a Stencil object on a material without a backing sheet, like paper or cardstock, has closed inner parts, the middle of an O, that fall out; no fix, a stencil font with bridges keeps them). Codes only the import commands report, from the file itself: missing_font (warning: text whose font is not installed was not drawn), clip_ignored, images_skipped, dashes_ignored (info), no_paths (warning), and for traced images nothing_traced (warning), trace_downscaled, dpi_ignored, shapes_dropped (info). Codes only job.preview reports, as reasons there is nothing to cut (all error): empty_document (no fix), hidden (fix object.set_visible visible true), skipped (fix object.set_cut cut true), fill_ignored, no_paths, too_small (every cut is shorter than half a plotter step; fix object.scale). Unknown ids are refused.",
);
