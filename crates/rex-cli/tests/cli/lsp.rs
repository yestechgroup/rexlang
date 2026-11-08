//! `rexlang` CLI tests: lsp.

use crate::support::*;

// --- `rexlang lsp` -----------------------------------------------------------

use std::io::{BufRead, BufReader, Read as _};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

/// Writes one `Content-Length` framed message.
fn write_framed<W: std::io::Write>(sink: &mut W, body: &str) {
    write!(sink, "Content-Length: {}\r\n\r\n{}", body.len(), body).expect("write framed message");
    sink.flush().expect("flush framed message");
}

/// Reads one `Content-Length` framed message.
fn read_framed(source: &mut BufReader<std::process::ChildStdout>) -> String {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        source.read_line(&mut line).expect("read header line");
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            content_length = Some(value.parse().expect("Content-Length is a number"));
        }
    }
    let length = content_length.expect("a Content-Length header");
    let mut body = vec![0u8; length];
    source.read_exact(&mut body).expect("read framed body");
    String::from_utf8(body).expect("framed body is UTF-8")
}

#[test]
fn lsp_subcommand_speaks_framed_json_rpc_over_stdio() {
    let mut child = rexlang()
        .args(["lsp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rexlang lsp");

    let mut stdin = child.stdin.take().expect("stdin piped");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout piped"));

    // The whole interaction runs on a helper thread so the test can enforce
    // a hard timeout instead of hanging CI.
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let initialize = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"capabilities": {}},
        });
        write_framed(&mut stdin, &initialize.to_string());
        let initialize_response: serde_json::Value =
            serde_json::from_str(&read_framed(&mut stdout)).expect("initialize response");

        let shutdown = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown"});
        write_framed(&mut stdin, &shutdown.to_string());
        let shutdown_response: serde_json::Value =
            serde_json::from_str(&read_framed(&mut stdout)).expect("shutdown response");

        let exit = serde_json::json!({"jsonrpc": "2.0", "method": "exit"});
        write_framed(&mut stdin, &exit.to_string());

        sender
            .send((initialize_response, shutdown_response))
            .expect("send results");
    });

    let result = receiver.recv_timeout(Duration::from_secs(10));
    child.kill().expect("kill the server");
    let _ = child.wait();

    if let Err(mpsc::RecvTimeoutError::Timeout) = result {
        panic!("rexlang lsp did not answer within 10s");
    }
    worker.join().expect("worker thread");
    let (initialize_response, shutdown_response) = result.expect("results");

    assert_eq!(initialize_response["id"], 1);
    assert!(
        initialize_response["result"]["capabilities"].is_object(),
        "initialize must return capabilities: {initialize_response}"
    );
    assert_eq!(shutdown_response["id"], 2);
    assert!(
        shutdown_response.get("error").is_none(),
        "shutdown must succeed: {shutdown_response}"
    );
}
