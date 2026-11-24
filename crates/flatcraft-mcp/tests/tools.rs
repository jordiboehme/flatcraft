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
    let server = FlatcraftMcp::new(session.clone(), Arc::new(|| {}));
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
