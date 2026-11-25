//! The stdio transport core over an in-memory pipe.

use std::sync::{Arc, Mutex};

use flatcraft_device::ProfileStore;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::Session;
use flatcraft_mcp::{SharedSession, stdio};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn session(dir: &tempfile::TempDir) -> SharedSession {
    let profiles = ProfileStore::load(dir.path()).unwrap();
    Arc::new(Mutex::new(Session::new(
        profiles,
        Preferences::default(),
        false,
        dir.path().to_path_buf(),
    )))
}

#[tokio::test]
async fn stdio_answers_initialize_and_tools_list() {
    let dir = tempfile::tempdir().unwrap();
    let (client_w, server_r) = tokio::io::duplex(1 << 16);
    let (server_w, client_r) = tokio::io::duplex(1 << 16);
    let s = session(&dir);
    let server = tokio::spawn(stdio::serve_with(s, Arc::new(|| {}), server_r, server_w));

    let mut w = client_w;
    let mut lines = BufReader::new(client_r).lines();
    let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},
        "clientInfo":{"name":"test","version":"0"}}});
    w.write_all(format!("{init}\n").as_bytes()).await.unwrap();
    let a: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(a["id"], 1);
    assert!(a["result"]["instructions"].as_str().unwrap().contains("mm"));
    let note = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
    w.write_all(format!("{note}\n").as_bytes()).await.unwrap();
    let list = json!({"jsonrpc":"2.0","id":2,"method":"tools/list"});
    w.write_all(format!("{list}\n").as_bytes()).await.unwrap();
    let a: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(a["id"], 2);
    let mut names: Vec<&str> = a["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "describe_command",
            "document",
            "execute",
            "list_commands",
            "preview",
            "version"
        ]
    );
    drop(w);
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn stdio_ends_when_input_closes() {
    let dir = tempfile::tempdir().unwrap();
    let (client_w, server_r) = tokio::io::duplex(1 << 16);
    let (server_w, _client_r) = tokio::io::duplex(1 << 16);
    let server = tokio::spawn(stdio::serve_with(
        session(&dir),
        Arc::new(|| {}),
        server_r,
        server_w,
    ));
    drop(client_w);
    server.await.unwrap().unwrap();
}
