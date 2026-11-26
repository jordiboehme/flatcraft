//! The MCP tools through rmcp's in-memory client.

use std::sync::{Arc, Mutex};

use flatcraft_device::ProfileStore;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::Session;
use flatcraft_mcp::{FlatcraftMcp, SharedSession};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="20" y="20" width="50" height="30"/></svg>"#;

struct Fixture {
    _dir: tempfile::TempDir,
    session: SharedSession,
    client: RunningService<RoleClient, ()>,
}

async fn connect() -> Fixture {
    connect_with(|server| server).await
}

async fn connect_with(setup: impl FnOnce(FlatcraftMcp) -> FlatcraftMcp) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let profiles = ProfileStore::load(dir.path()).unwrap();
    let prefs = Preferences {
        output_dir: Some(dir.path().join("output")),
        ..Preferences::default()
    };
    let session: SharedSession = Arc::new(Mutex::new(Session::new(
        profiles,
        prefs,
        false,
        dir.path().to_path_buf(),
    )));
    let server = setup(FlatcraftMcp::new(session.clone(), Arc::new(|| {})));
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        let running = server.serve(server_io).await.expect("server starts");
        let _ = running.waiting().await;
    });
    let client = ().serve(client_io).await.expect("client connects");
    Fixture {
        _dir: dir,
        session,
        client,
    }
}

async fn call(f: &Fixture, tool: &str, args: Value) -> CallToolResult {
    let params = CallToolRequestParams::new(tool.to_string())
        .with_arguments(args.as_object().expect("an object").clone());
    f.client
        .call_tool(params)
        .await
        .expect("a tool result, not a protocol error")
}

fn text(r: &CallToolResult) -> String {
    r.content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn list_commands_includes_job_send() {
    let f = connect().await;
    let r = call(&f, "list_commands", json!({})).await;
    assert_ne!(r.is_error, Some(true));
    let list: Value = serde_json::from_str(&text(&r)).unwrap();
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"job.send"), "{ids:?}");
    let filtered = call(&f, "list_commands", json!({ "filter": "job." })).await;
    let list: Value = serde_json::from_str(&text(&filtered)).unwrap();
    assert!(list.as_array().unwrap().len() < ids.len());
    assert!(list[0]["title"].is_string() && list[0]["description"].is_string());
}

#[tokio::test]
async fn describe_command_inlines_defs() {
    let f = connect().await;
    let r = call(&f, "describe_command", json!({ "id": "document.preview" })).await;
    assert_ne!(r.is_error, Some(true), "{}", text(&r));
    let v: Value = serde_json::from_str(&text(&r)).unwrap();
    assert!(v["description"].is_string());
    let schema = v["schema"].to_string();
    assert!(
        !schema.contains("$ref") && !schema.contains("$defs"),
        "{schema}"
    );
    let bad = call(&f, "describe_command", json!({ "id": "nope" })).await;
    assert_eq!(bad.is_error, Some(true));
}

#[tokio::test]
async fn execute_import_svg_then_document_shows_object() {
    let f = connect().await;
    let r = call(
        &f,
        "execute",
        json!({ "command": "object.import_svg", "params": { "svg": SVG, "name": "box" } }),
    )
    .await;
    assert_ne!(r.is_error, Some(true), "{}", text(&r));
    let d = call(&f, "document", json!({})).await;
    let state: Value = serde_json::from_str(&text(&d)).unwrap();
    assert_eq!(state["objects"].as_array().unwrap().len(), 1);
    assert_eq!(state["objects"][0]["name"], "box");
    assert!(f.session.lock().unwrap().last_agent_activity.is_some());
}

#[tokio::test]
async fn execute_error_is_a_tool_error() {
    let f = connect().await;
    let r = call(&f, "execute", json!({ "command": "nope" })).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("Unknown command"), "{}", text(&r));
}

#[tokio::test]
async fn version_reports_plotter_and_window() {
    let f = connect().await;
    let r = call(&f, "version", json!({})).await;
    let v: Value = serde_json::from_str(&text(&r)).unwrap();
    assert!(v["version"].is_string());
    assert_eq!(v["plotter"], "file-output-gpgl-cameo2");
    assert_eq!(v["window_open"], false);
}

#[tokio::test]
async fn preview_returns_png_image() {
    use base64::Engine;
    let f = connect().await;
    call(
        &f,
        "execute",
        json!({ "command": "object.import_svg", "params": { "svg": SVG } }),
    )
    .await;
    let r = call(&f, "preview", json!({ "kind": "canvas", "max_px": 128 })).await;
    assert_ne!(r.is_error, Some(true), "{}", text(&r));
    let img = r
        .content
        .iter()
        .find_map(|c| c.as_image())
        .expect("an image block");
    assert_eq!(img.mime_type, "image/png");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&img.data)
        .unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
}

#[tokio::test]
async fn agent_usb_send_headless_is_refused() {
    let f = connect().await;
    call(
        &f,
        "execute",
        json!({ "command": "object.import_svg", "params": { "svg": SVG } }),
    )
    .await;
    let r = call(
        &f,
        "execute",
        json!({ "command": "job.set_plotter", "params": { "id": "silhouette-cameo-2" } }),
    )
    .await;
    assert_ne!(r.is_error, Some(true), "{}", text(&r));
    let r = call(&f, "execute", json!({ "command": "job.send" })).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("headless"), "{}", text(&r));
}

#[tokio::test]
async fn old_prefs_with_mcp_keys_still_load() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("preferences.toml"),
        "mcp_enabled = false\nmcp_port = 4000\nallow_agent_cuts = true\n",
    )
    .unwrap();
    let p = Preferences::load(dir.path());
    assert!(p.allow_agent_cuts, "the file must load, not fall back");
}

#[tokio::test]
async fn preview_kind_is_an_enum_in_the_schema() {
    let f = connect().await;
    let tools = f.client.list_all_tools().await.unwrap();
    let t = tools.iter().find(|t| t.name == "preview").unwrap();
    let schema = serde_json::to_string(&*t.input_schema).unwrap();
    assert!(
        schema.contains("canvas") && schema.contains("path"),
        "{schema}"
    );
    assert!(!schema.contains("$ref"), "{schema}");
}

#[tokio::test]
async fn huge_max_px_is_a_tool_error() {
    let f = connect().await;
    let r = call(&f, "preview", json!({ "kind": "canvas", "max_px": 100000 })).await;
    assert_eq!(r.is_error, Some(true), "{}", text(&r));
}

#[tokio::test]
async fn a_panic_under_the_lock_does_not_stop_the_server() {
    let f = connect().await;
    let s = f.session.clone();
    let _ = std::thread::spawn(move || {
        let _g = s.lock().unwrap();
        panic!("boom");
    })
    .join();
    assert!(f.session.is_poisoned());
    let r = call(&f, "version", json!({})).await;
    assert_ne!(r.is_error, Some(true), "{}", text(&r));
}

#[tokio::test]
async fn read_only_tools_do_not_fire_on_change() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let f = connect().await;
    let n = Arc::new(AtomicUsize::new(0));
    let n2 = n.clone();
    let server = FlatcraftMcp::new(
        f.session.clone(),
        Arc::new(move || {
            n2.fetch_add(1, Ordering::SeqCst);
        }),
    );
    let (sio, cio) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        let r = server.serve(sio).await.unwrap();
        let _ = r.waiting().await;
    });
    let client = ().serve(cio).await.unwrap();
    for (tool, args) in [
        ("version", json!({})),
        ("list_commands", json!({})),
        ("describe_command", json!({ "id": "job.send" })),
        ("document", json!({})),
        ("preview", json!({ "kind": "canvas" })),
    ] {
        let p = CallToolRequestParams::new(tool.to_string())
            .with_arguments(args.as_object().unwrap().clone());
        client.call_tool(p).await.unwrap();
    }
    assert_eq!(n.load(Ordering::SeqCst), 0);
    let p = CallToolRequestParams::new("execute".to_string()).with_arguments(
        json!({ "command": "document.state" })
            .as_object()
            .unwrap()
            .clone(),
    );
    client.call_tool(p).await.unwrap();
    assert_eq!(n.load(Ordering::SeqCst), 1);
}

#[test]
fn inline_defs_handles_nesting_repeats_siblings_and_cycles() {
    use flatcraft_mcp::server::inline_defs;
    let schema = json!({
        "type": "object",
        "properties": {
            "a": { "$ref": "#/$defs/Outer" },
            "b": { "$ref": "#/$defs/Outer", "description": "second use" }
        },
        "$defs": {
            "Outer": { "type": "object", "properties": { "i": { "$ref": "#/$defs/Inner" } } },
            "Inner": { "type": "integer" }
        }
    });
    let out = inline_defs(schema);
    assert!(!out.to_string().contains("$ref") && out.get("$defs").is_none());
    assert_eq!(out["properties"]["a"]["properties"]["i"]["type"], "integer");
    assert_eq!(out["properties"]["b"]["properties"]["i"]["type"], "integer");
    assert_eq!(out["properties"]["b"]["description"], "second use");

    let cyclic = json!({
        "$ref": "#/$defs/Node",
        "$defs": { "Node": { "type": "object", "properties": { "next": { "$ref": "#/$defs/Node" } } } }
    });
    let out = inline_defs(cyclic);
    assert!(out.get("$defs").is_some(), "cycle keeps its defs: {out}");
    assert_eq!(out["type"], "object");
}

#[tokio::test]
async fn check_sources_hashes_without_the_session_lock() {
    use flatcraft_engine::commands::sources::check_unlocked_with;
    use std::sync::atomic::{AtomicBool, Ordering};
    let hashed = Arc::new(AtomicBool::new(false));
    let seen = hashed.clone();
    let f = connect_with(move |server| {
        server.with_source_check(move |session| {
            check_unlocked_with(session, |plan| {
                assert!(session.try_lock().is_ok(), "the lock is free while hashing");
                seen.store(true, Ordering::SeqCst);
                plan.run()
            })
        })
    })
    .await;
    let file = f._dir.path().join("a.svg");
    std::fs::write(&file, SVG).unwrap();
    let id = f
        .session
        .lock()
        .unwrap()
        .execute(
            "object.import",
            json!({ "path": file }),
            flatcraft_engine::session::Origin::User,
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    std::fs::write(&file, SVG.replace("x=\"20\"", "x=\"21\"")).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
        .unwrap();
    let r = call(
        &f,
        "execute",
        json!({ "command": "document.check_sources", "params": {} }),
    )
    .await;
    assert_ne!(r.is_error, Some(true), "{}", text(&r));
    let v: Value = serde_json::from_str(&text(&r)).unwrap();
    assert_eq!(v["changed"], json!([id]), "{v}");
    assert!(
        hashed.load(Ordering::SeqCst),
        "the MCP check took the unlocked path"
    );
}

#[test]
fn instructions_put_the_origin_on_the_mat_grid() {
    let text = flatcraft_mcp::server::INSTRUCTIONS;
    assert!(text.contains("grid"), "{text}");
    assert!(text.contains("cutting_area_mm"), "{text}");
    assert!(!text.contains("top-left corner of the media"), "{text}");
    assert!(!text.contains('\u{2014}'), "no em-dash");
}

#[test]
fn instructions_name_the_report_codes() {
    let text = flatcraft_mcp::server::INSTRUCTIONS;
    for code in flatcraft_engine::report::Code::ALL {
        assert!(text.contains(code.as_str()), "{} missing", code.as_str());
    }
    assert!(text.contains("fixes"), "{text}");
    assert!(text.contains("document.check"), "{text}");
}

async fn exec(f: &Fixture, command: &str, params: Value) -> Value {
    let r = call(
        f,
        "execute",
        json!({ "command": command, "params": params }),
    )
    .await;
    assert_ne!(r.is_error, Some(true), "{command}: {}", text(&r));
    serde_json::from_str(&text(&r)).unwrap()
}

/// An agent reads the import report, runs the offered fix with one
/// execute call, and the issue is gone.
#[tokio::test]
async fn an_agent_runs_the_fix_of_an_import_report() {
    let f = connect().await;
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><rect x="20" y="20" width="1" height="1"/></svg>"#;
    let r = exec(
        &f,
        "object.import_svg",
        json!({ "svg": svg, "name": "dot" }),
    )
    .await;
    let issue = &r["report"]["issues"][0];
    assert_eq!(issue["code"], "below_min_feature");
    assert_eq!(issue["severity"], "warning");
    assert_eq!(issue["object_ids"], json!([r["id"]]));
    let fix = &issue["fixes"][0];
    exec(&f, fix["command"].as_str().unwrap(), fix["args"].clone()).await;
    let c = exec(&f, "document.check", json!({})).await;
    assert_eq!(c["issues"], json!([]), "{c}");
}

/// job.preview of a job with nothing to cut is data with the reasons,
/// not a tool error.
#[tokio::test]
async fn nothing_to_cut_comes_back_as_reasons_with_fixes() {
    let f = connect().await;
    let r = exec(
        &f,
        "object.import_svg",
        json!({ "svg": SVG, "name": "box" }),
    )
    .await;
    exec(
        &f,
        "object.set_cut",
        json!({ "ids": [r["id"]], "cut": false }),
    )
    .await;
    let p = exec(&f, "job.preview", json!({})).await;
    assert_eq!(p["ready"], false);
    assert_eq!(p["nothing_to_cut"], true);
    let issue = &p["issues"][0];
    assert_eq!(issue["code"], "skipped");
    assert_eq!(
        issue["message"],
        "'box' is set to skip, so the knife leaves it out."
    );
    let fix = &issue["fixes"][0];
    assert_eq!(fix["command"], "object.set_cut");
    exec(&f, "object.set_cut", fix["args"].clone()).await;
    let p = exec(&f, "job.preview", json!({})).await;
    assert_eq!(p["ready"], true, "{p}");
}

#[test]
fn instructions_point_to_the_troubleshooter() {
    let text = flatcraft_mcp::server::INSTRUCTIONS;
    assert!(text.contains("job.troubleshoot"), "{text}");
    for s in flatcraft_engine::troubleshoot::Symptom::ALL {
        assert!(text.contains(s.as_str()), "{} missing", s.as_str());
    }
    assert!(!text.contains('\u{2014}'));
}

/// An agent gets the same checklist the window shows, runs a fix with
/// one execute (the job override changes, the preset does not), and
/// still cannot send to a USB plotter on its own.
#[tokio::test]
async fn an_agent_troubleshoots_like_the_window() {
    use flatcraft_engine::troubleshoot::{Symptom, checklist};
    let f = connect().await;
    exec(
        &f,
        "object.import_svg",
        json!({ "svg": SVG, "name": "box" }),
    )
    .await;
    let c = exec(&f, "job.troubleshoot", json!({ "symptom": "not_through" })).await;
    let want = {
        let s = f.session.lock().unwrap();
        let input = flatcraft_engine::job::JobInput::of(&s).unwrap();
        let log = s.cut_log();
        checklist(
            &input,
            Symptom::NotThrough,
            log.last_good(&input.job.material_id),
        )
    };
    assert_eq!(c, serde_json::to_value(&want).unwrap());
    let force = c["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["check"] == "force")
        .unwrap();
    let fix = &force["fixes"][0];
    assert_eq!(fix["label"], "Force +2");
    assert_eq!(fix["command"], "job.set_overrides");
    let before = c["settings"]["force"].as_u64().unwrap();
    let out = exec(&f, "job.set_overrides", fix["args"].clone()).await;
    assert_eq!(out["force"], before + 2);
    let p = exec(&f, "profiles.get", json!({ "id": "copy-paper" })).await;
    assert_eq!(p["force"], before, "the preset stays");
    // A USB cut still needs the person.
    exec(&f, "job.set_plotter", json!({ "id": "silhouette-cameo-2" })).await;
    let r = call(
        &f,
        "execute",
        json!({ "command": "job.send", "params": {} }),
    )
    .await;
    assert_eq!(r.is_error, Some(true), "{}", text(&r));
}
