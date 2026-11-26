//! The MCP tools on top of the command registry.

use std::sync::{Arc, Mutex};

use flatcraft_engine::commands::sources::{self, CheckResult};
use flatcraft_engine::registry;
use flatcraft_engine::session::{CommandError, Origin, Session};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{OnChange, SharedSession};

/// What the server tells a connecting agent.
pub const INSTRUCTIONS: &str = "FlatCraft prepares and sends cutting plotter jobs. \
Every length is in millimetres. The origin 0,0 is the 0,0 of the grid printed on the selected mat, x runs right and y runs down. \
Objects must stay inside the cutting area, which the document tool (document.state) reports as cutting_area_mm. \
Use list_commands to find commands, describe_command for the parameters of one, execute to run it, \
document to read the state and preview to see the result as a PNG. \
A USB cut started by an agent goes ahead only if the person turned on 'Allow agents to start cuts'. \
Otherwise, with a window open the person confirms it; without a window it is refused. File Output is always allowed. \
Problems come as issues you can act on: object.import and object.import_svg return report.issues, document.check checks objects again, \
and job.preview returns issues (with ready false instead of an error) when nothing can be cut or objects are off the mat. \
Each issue has a stable code, a severity (info, warning or error), object_ids, paths, message (the text the person sees), hint, \
and fixes: a list of {label, description, command, args}; run a fix with execute {command, params: args}. No fix means: follow the hint. \
Codes: open_paths, hairline, below_min_feature, fill_ignored, thin_slivers, outside_area, larger_than_area, missing_font, clip_ignored, images_skipped, \
dashes_ignored, no_paths, nothing_traced, trace_downscaled, dpi_ignored, shapes_dropped, stencil_island (import report and document.check); \
empty_document, hidden, skipped, too_small (job.preview, nothing to cut). describe_command document.check explains each code. \
When a cut went wrong, run job.troubleshoot {symptom: not_through, tore_or_dragged, pieces_moved or wrong_place_or_size}: it returns the checklist \
the window's \"It didn't cut right...\" helper shows, in order, so give the same advice in the same order. Its items have fixes in the same shape; \
speed, force and passes fixes change only this job's overrides, never the presets. Propose them to the person: you may run them, but you cannot confirm a USB cut.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListArgs {
    /// Only commands whose id, title or description contains this text (case-insensitive).
    #[serde(default)]
    pub filter: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DescribeArgs {
    /// Command id, like `object.move`.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExecuteArgs {
    /// Command id, like `object.move`.
    pub command: String,
    /// The command's parameters as an object (see describe_command). Omit for none.
    #[serde(default)]
    pub params: Option<Value>,
}

/// What to draw. "canvas": media, cutting area and objects. "path": planned toolpaths in cut order.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(inline)]
pub enum PreviewKind {
    Canvas,
    Path,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PreviewArgs {
    pub kind: PreviewKind,
    /// Larger image side in px, 64 to 4096 (default 1024).
    #[serde(default)]
    pub max_px: Option<u32>,
}

/// Runs `document.check_sources` for an agent without holding the session
/// lock while files are hashed. Tests replace it.
pub type SourceChecker = Arc<dyn Fn(&Mutex<Session>) -> (CheckResult, bool) + Send + Sync>;

/// The MCP server. Cheap to clone.
#[derive(Clone)]
pub struct FlatcraftMcp {
    session: SharedSession,
    on_change: OnChange,
    source_check: SourceChecker,
    tool_router: ToolRouter<Self>,
}

fn text_ok(value: &Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(value.to_string())])
}

fn text_err(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

fn error_result(e: &CommandError) -> CallToolResult {
    text_err(e.to_string())
}

#[tool_router]
impl FlatcraftMcp {
    pub fn new(session: SharedSession, on_change: OnChange) -> Self {
        Self {
            session,
            on_change,
            source_check: Arc::new(sources::check_unlocked),
            tool_router: Self::tool_router(),
        }
    }

    /// Runs the agent's `document.check_sources` with `f` instead of
    /// [`sources::check_unlocked`] (tests).
    pub fn with_source_check(
        mut self,
        f: impl Fn(&Mutex<Session>) -> (CheckResult, bool) + Send + Sync + 'static,
    ) -> Self {
        self.source_check = Arc::new(f);
        self
    }

    /// `document.check_sources` without params: stat and compare under the
    /// lock, hash without it, on tokio's blocking pool.
    async fn check_sources(&self) -> CallToolResult {
        let session = Arc::clone(&self.session);
        let check = Arc::clone(&self.source_check);
        let done = tokio::task::spawn_blocking(move || {
            session
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .last_agent_activity = Some(std::time::Instant::now());
            check(&session).0.to_json()
        })
        .await;
        match done {
            Ok(v) => text_ok(&v),
            Err(_) => text_err("The call stopped with an internal error"),
        }
    }

    /// `project.save` with the thumbnail drawn and the file written
    /// without the session lock, so the window keeps drawing meanwhile.
    async fn save(&self, params: Value) -> CallToolResult {
        let session = Arc::clone(&self.session);
        let on_change = Arc::clone(&self.on_change);
        let done = tokio::task::spawn_blocking(move || {
            let r =
                flatcraft_engine::commands::project::save_unlocked(&session, params, Origin::Agent);
            on_change();
            r
        })
        .await;
        match done {
            Ok(Ok(v)) => text_ok(&v),
            Ok(Err(e)) => error_result(&e),
            Err(_) => text_err("The call stopped with an internal error"),
        }
    }

    /// Runs `f` on the session on tokio's blocking pool. The lock is never held
    /// across an await. Marks the agent activity; tells the GUI afterwards only if `notify`.
    async fn with_session<T: Send + 'static>(
        &self,
        notify: bool,
        f: impl FnOnce(&mut Session) -> T + Send + 'static,
    ) -> Result<T, CallToolResult> {
        let session = Arc::clone(&self.session);
        let on_change = Arc::clone(&self.on_change);
        tokio::task::spawn_blocking(move || {
            let mut s = session.lock().unwrap_or_else(|p| p.into_inner());
            s.last_agent_activity = Some(std::time::Instant::now());
            let out = f(&mut s);
            drop(s);
            if notify {
                on_change();
            }
            out
        })
        .await
        .map_err(|_| text_err("The call stopped with an internal error"))
    }

    #[tool(
        description = "App version, the active plotter and material, and whether a FlatCraft window is open. No input."
    )]
    async fn version(&self) -> CallToolResult {
        match self
            .with_session(false, |s| {
                let job = &s.doc().document.job;
                json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "plotter": job.plotter_id,
                    "material": job.material_id,
                    "window_open": s.has_ui,
                })
            })
            .await
        {
            Ok(v) => text_ok(&v),
            Err(e) => e,
        }
    }

    #[tool(
        description = "List the commands: id, title and a one-line description. Optional filter: text that the id, title or description contains."
    )]
    async fn list_commands(&self, Parameters(args): Parameters<ListArgs>) -> CallToolResult {
        let filter = args
            .filter
            .map(|f| f.to_lowercase())
            .filter(|f| !f.is_empty());
        let list: Vec<Value> = registry::all()
            .iter()
            .filter(|c| {
                filter.as_ref().is_none_or(|f| {
                    [c.id(), c.title(), c.description()]
                        .iter()
                        .any(|t| t.to_lowercase().contains(f.as_str()))
                })
            })
            .map(|c| {
                json!({
                    "id": c.id(),
                    "title": c.title(),
                    "description": first_sentence(c.description()),
                })
            })
            .collect();
        // Not a session call, but still agent activity.
        let _ = self.with_session(false, |_| ()).await;
        text_ok(&Value::Array(list))
    }

    #[tool(
        description = "The full description and the JSON schema of one command's parameters. Input: id. The schema has its definitions inlined (no $ref)."
    )]
    async fn describe_command(&self, Parameters(args): Parameters<DescribeArgs>) -> CallToolResult {
        let _ = self.with_session(false, |_| ()).await;
        let Some(c) = registry::get(&args.id) else {
            return error_result(&CommandError::UnknownCommand(args.id));
        };
        let schema = serde_json::to_value(c.params_schema()).unwrap_or(Value::Null);
        text_ok(&json!({
            "id": c.id(),
            "title": c.title(),
            "description": c.description(),
            "mutates_document": c.mutates_document(),
            "schema": inline_defs(schema),
        }))
    }

    #[tool(
        description = "Run a command (see list_commands and describe_command) as an agent. Input: command, params (an object). Returns the command's JSON; a failure is a tool error with the reason."
    )]
    async fn execute(&self, Parameters(args): Parameters<ExecuteArgs>) -> CallToolResult {
        let params = args.params.unwrap_or_else(|| json!({}));
        let params = if params.is_null() { json!({}) } else { params };
        let command = args.command;
        if command == "document.check_sources" && params.as_object().is_some_and(|o| o.is_empty()) {
            return self.check_sources().await;
        }
        if command == "project.save" {
            return self.save(params).await;
        }
        match self
            .with_session(true, move |s| s.execute(&command, params, Origin::Agent))
            .await
        {
            Ok(Ok(v)) => text_ok(&v),
            Ok(Err(e)) => error_result(&e),
            Err(e) => e,
        }
    }

    #[tool(description = "The state of the active document (document.state). No input.")]
    async fn document(&self) -> CallToolResult {
        match self
            .with_session(false, |s| {
                s.execute("document.state", json!({}), Origin::Agent)
            })
            .await
        {
            Ok(Ok(v)) => text_ok(&v),
            Ok(Err(e)) => error_result(&e),
            Err(e) => e,
        }
    }

    #[tool(
        description = "Draw the active document as a PNG image. Input: kind ('canvas' or 'path'), optional max_px (64 to 4096, default 1024)."
    )]
    async fn preview(&self, Parameters(args): Parameters<PreviewArgs>) -> CallToolResult {
        let mut params = json!({ "kind": match args.kind { PreviewKind::Canvas => "canvas", PreviewKind::Path => "path" } });
        if let Some(px) = args.max_px {
            params["max_px"] = json!(px);
        }
        let r = match self
            .with_session(false, move |s| {
                s.execute("document.preview", params, Origin::Agent)
            })
            .await
        {
            Ok(r) => r,
            Err(e) => return e,
        };
        match r {
            Ok(v) => {
                let Some(png) = v["png_base64"].as_str() else {
                    return text_err("The preview has no image");
                };
                CallToolResult::success(vec![
                    ContentBlock::image(png, "image/png"),
                    ContentBlock::text(
                        json!({ "width": v["width"], "height": v["height"] }).to_string(),
                    ),
                ])
            }
            Err(e) => error_result(&e),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FlatcraftMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("flatcraft", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

/// The first sentence of a description, for lists.
fn first_sentence(text: &str) -> &str {
    match text.find(". ") {
        Some(i) => &text[..=i],
        None => text,
    }
}

/// Replaces every `{"$ref": "#/$defs/X"}` by the definition and drops `$defs`.
/// Some MCP clients cannot follow `$ref`, so the schema stands alone. A cyclic
/// definition keeps its `$ref` (and the `$defs` stay) after a few levels.
pub fn inline_defs(mut schema: Value) -> Value {
    let defs = schema
        .as_object_mut()
        .and_then(|o| o.remove("$defs"))
        .unwrap_or(Value::Null);
    let mut out = resolve(schema, &defs, 0);
    if contains_ref(&out)
        && let (Some(o), Value::Object(_)) = (out.as_object_mut(), &defs)
    {
        o.insert("$defs".into(), defs);
    }
    out
}

fn contains_ref(v: &Value) -> bool {
    match v {
        Value::Object(o) => o.contains_key("$ref") || o.values().any(contains_ref),
        Value::Array(a) => a.iter().any(contains_ref),
        _ => false,
    }
}

fn resolve(v: Value, defs: &Value, depth: u32) -> Value {
    match v {
        Value::Object(mut o) => {
            if depth < 8
                && let Some(Value::String(r)) = o.get("$ref")
                && let Some(name) = r.strip_prefix("#/$defs/")
                && let Some(def) = defs.get(name)
            {
                let mut target = resolve(def.clone(), defs, depth + 1);
                // Keep sibling keywords such as a description next to the $ref.
                if let Value::Object(t) = &mut target {
                    o.remove("$ref");
                    for (k, val) in o {
                        t.insert(k, resolve(val, defs, depth + 1));
                    }
                }
                return target;
            }
            Value::Object(
                o.into_iter()
                    .map(|(k, val)| (k, resolve(val, defs, depth)))
                    .collect(),
            )
        }
        Value::Array(a) => Value::Array(a.into_iter().map(|x| resolve(x, defs, depth)).collect()),
        other => other,
    }
}
