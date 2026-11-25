//! `flatcraft mcp` speaks only MCP on stdout.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Everything the child gets is bounded by this, so a hang fails the test
/// instead of stalling CI.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Kills the child and fails the test.
fn fail(child: &mut Child, why: &str) -> ! {
    let _ = child.kill();
    let _ = child.wait();
    panic!("{why}");
}

/// Joins the reader; a reader that panicked is reported as such.
fn join_reader(child: &mut Child, reader: JoinHandle<()>, done: &mpsc::Receiver<String>) -> String {
    let rest = match done.recv_timeout(TIMEOUT) {
        Ok(rest) => rest,
        Err(mpsc::RecvTimeoutError::Timeout) => fail(child, "stdout did not close within 30 s"),
        Err(mpsc::RecvTimeoutError::Disconnected) => String::new(),
    };
    if reader.join().is_err() {
        fail(
            child,
            "the stdout reader thread panicked (see its message above)",
        );
    }
    rest
}

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

    let (first_tx, first_rx) = mpsc::channel();
    let (rest_tx, rest_rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut out = BufReader::new(stdout);
        let mut first = String::new();
        out.read_line(&mut first).expect("read the first line");
        first_tx.send(first).unwrap();
        let mut rest = String::new();
        out.read_to_string(&mut rest)
            .expect("read the rest of stdout");
        rest_tx.send(rest).unwrap();
    });

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;
    writeln!(stdin, "{init}").unwrap();
    stdin.flush().unwrap();
    let first = match first_rx.recv_timeout(TIMEOUT) {
        Ok(line) => line,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            fail(&mut child, "no answer to initialize within 30 s")
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = reader.join();
            fail(
                &mut child,
                "the stdout reader thread panicked before the first line",
            )
        }
    };
    let answer: serde_json::Value = match serde_json::from_str(&first) {
        Ok(v) => v,
        Err(e) => fail(
            &mut child,
            &format!("stdout is not JSON-RPC ({e}): {first:?}"),
        ),
    };
    assert_eq!(answer["jsonrpc"], "2.0");
    assert_eq!(answer["id"], 1);
    assert!(answer["result"]["serverInfo"].is_object(), "{answer}");

    // Closing stdin ends the server; nothing else may be on stdout.
    drop(stdin);
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait().unwrap() {
            Some(status) => break status,
            None if Instant::now() > deadline => fail(
                &mut child,
                "flatcraft mcp did not exit within 30 s of stdin closing",
            ),
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let rest = join_reader(&mut child, reader, &rest_rx);
    assert!(rest.is_empty(), "extra bytes on stdout: {rest:?}");
    assert!(status.success(), "{status}");
}
