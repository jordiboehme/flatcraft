//! `flatcraft mcp` speaks only MCP on stdout.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn mcp_headless_answers_initialize() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_flatcraft"))
        .arg("mcp")
        .env("FLATCRAFT_CONFIG_DIR", dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    // A hang fails the test instead of stalling CI.
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut out = BufReader::new(stdout);
        let mut first = String::new();
        out.read_line(&mut first).unwrap();
        tx.send(first).unwrap();
        let mut rest = String::new();
        out.read_to_string(&mut rest).unwrap();
        rest
    });

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;
    writeln!(stdin, "{init}").unwrap();
    stdin.flush().unwrap();
    let first = match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(line) => line,
        Err(_) => {
            child.kill().unwrap();
            panic!("no answer to initialize within 30 s");
        }
    };
    let answer: serde_json::Value = serde_json::from_str(&first)
        .unwrap_or_else(|e| panic!("stdout is not JSON-RPC ({e}): {first:?}"));
    assert_eq!(answer["jsonrpc"], "2.0");
    assert_eq!(answer["id"], 1);
    assert!(answer["result"]["serverInfo"].is_object(), "{answer}");

    // Closing stdin ends the server; nothing else may be on stdout.
    drop(stdin);
    let status = child.wait().unwrap();
    let rest = reader.join().unwrap();
    assert!(rest.is_empty(), "extra bytes on stdout: {rest:?}");
    assert!(status.success(), "{status}");
}
