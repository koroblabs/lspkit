//! Runs the binary as an editor does and checks that `exit` ends the process while stdin is still
//! open.
//!
//! Several clients send `exit` and keep the pipe open, or wait for the child to end. A server that
//! only stops when its input closes would outlive the session. In-process tests cannot catch this:
//! a read blocked on real stdin is a thread the runtime waits for at shutdown.

use std::{
    io::Write as _,
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

fn framed(json: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{json}", json.len())
}

/// Runs `kitserver serve`, sends `messages`, and waits for the process with stdin left open.
fn ends_after(messages: &[&str]) -> ExitStatus {
    let mut child = Command::new(env!("CARGO_BIN_EXE_kitserver"))
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("the server starts");
    let mut stdin = child.stdin.take().expect("stdin is piped");
    for message in messages {
        stdin
            .write_all(framed(message).as_bytes())
            .expect("the server reads its input");
    }
    stdin.flush().expect("the input flushes");

    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("the process can be waited on") {
            drop(stdin);
            return status;
        }
        if started.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            panic!("the server was still running 10 s after `exit`, with its input open");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

const INITIALIZE: &str =
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
const SHUTDOWN: &str = r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#;
const EXIT: &str = r#"{"jsonrpc":"2.0","method":"exit"}"#;

#[test]
fn exit_after_shutdown_ends_the_process_with_success() {
    let status = ends_after(&[INITIALIZE, INITIALIZED, SHUTDOWN, EXIT]);
    assert_eq!(status.code(), Some(0));
}

#[test]
fn exit_without_shutdown_ends_it_with_one() {
    let status = ends_after(&[INITIALIZE, INITIALIZED, EXIT]);
    assert_eq!(status.code(), Some(1));
}
