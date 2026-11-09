//! The loop's guarantees: `$/cancelRequest` stops a request, requests run without a cap,
//! notifications finish before the next message is read, and the lifecycle follows the
//! specification.

use std::{
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncWriteExt as _, BufReader, ReadBuf},
    sync::Barrier,
};

use crate::{
    cancel, codec,
    jsonrpc::{Message, Request, Response, Result},
    protocol::{ClientSocket, LanguageServer, LspService, ls_types::*},
    transport::{self, Ended},
};

/// A server whose hover waits for the test's signal, and whose edits apply slowly.
#[derive(Default)]
struct Probe {
    /// When set, every hover waits here, so a test can count how many requests run at once.
    gate: Option<Arc<Barrier>>,
    /// Set once the blocking work a hover started saw its request cancelled.
    saw_cancel: Arc<AtomicBool>,
    /// What the last edit said, for a hover to read back.
    text: Mutex<String>,
}

impl LanguageServer for Probe {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult::default())
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let line = params.text_document_position_params.position.line;
        if line == 99 {
            panic!("hover fell over");
        }
        if line == 7 {
            // Work that does not await, like a model's forward pass, can only poll the token.
            let token = cancel::current().expect("a request has a token");
            let saw = Arc::clone(&self.saw_cancel);
            tokio::task::spawn_blocking(move || {
                while !token.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                saw.store(true, Ordering::Release);
            });
            std::future::pending::<()>().await;
        }
        if let Some(gate) = &self.gate {
            gate.wait().await;
        }
        let text = self.text.lock().expect("uncontended").clone();
        Ok(Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(text)),
            range: None,
        }))
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // Slow on purpose: a request sent after this must still see what it wrote.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let text = params
            .content_changes
            .into_iter()
            .next_back()
            .map(|change| change.text);
        *self.text.lock().expect("uncontended") = text.unwrap_or_default();
    }
}

fn request(id: i64, method: &'static str, params: Value) -> Request {
    Request::build(method).id(id).params(params).finish()
}

fn notification(method: &'static str, params: Value) -> Request {
    Request::build(method).params(params).finish()
}

fn hover(line: u32) -> Value {
    json!({ "textDocument": { "uri": "file:///a.txt" }, "position": { "line": line, "character": 0 } })
}

/// The code a response answered with, or its result.
fn said(response: Option<Response>) -> String {
    let response = response.expect("a request is answered");
    match response.error() {
        Some(error) => format!("error {}", error.code.code()),
        None => response
            .result()
            .map(|result| result.to_string())
            .unwrap_or_default(),
    }
}

async fn started(probe: Probe) -> (LspService<Probe>, ClientSocket) {
    let (service, socket) = LspService::new(|_| probe);
    let answered = service
        .call(request(1, "initialize", json!({ "capabilities": {} })))
        .await;
    assert!(answered.is_some_and(|response| response.is_ok()));
    (service, socket)
}

#[tokio::test]
async fn a_cancelled_request_is_answered_at_once_and_its_work_is_told() {
    let probe = Probe::default();
    let saw_cancel = Arc::clone(&probe.saw_cancel);
    let (service, _socket) = started(probe).await;

    let answering = service.call(request(2, "textDocument/hover", hover(7)));
    tokio::time::sleep(Duration::from_millis(20)).await;
    service
        .call(notification("$/cancelRequest", json!({ "id": 2 })))
        .await;

    let answered = tokio::time::timeout(Duration::from_secs(5), answering)
        .await
        .expect("a cancelled request is answered, not left waiting");
    assert_eq!(said(answered), "error -32800");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !saw_cancel.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the blocking work saw its request cancelled");
}

/// Ten requests that can only finish together. `tower-lsp-server` polled four at a time, so this
/// would hang there: the first four hold the only slots, waiting for six that never start.
#[tokio::test]
async fn requests_are_not_capped_at_any_number_at_once() {
    let probe = Probe {
        gate: Some(Arc::new(Barrier::new(10))),
        ..Probe::default()
    };
    let (service, _socket) = started(probe).await;

    let answering: Vec<_> = (0..10)
        .map(|id| service.call(request(10 + id, "textDocument/hover", hover(0))))
        .collect();
    let answered = tokio::time::timeout(
        Duration::from_secs(5),
        futures_util::future::join_all(answering),
    )
    .await
    .expect("all ten run at once");
    assert!(
        answered
            .into_iter()
            .all(|response| response.is_some_and(|r| r.is_ok()))
    );
}

#[tokio::test(start_paused = true)]
async fn an_edit_has_landed_before_the_request_sent_after_it_is_answered() {
    let (service, socket) = LspService::new(|_| Probe::default());
    let (mut client, stdin) = tokio::io::duplex(1 << 16);
    let (stdout, server_said) = tokio::io::duplex(1 << 16);
    tokio::spawn(crate::transport::serve(
        "test", stdin, stdout, socket, service,
    ));

    let mut sent = Vec::new();
    for message in [
        request(1, "initialize", json!({ "capabilities": {} })),
        notification("initialized", json!({})),
        notification(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": "file:///a.txt", "version": 2 },
                "contentChanges": [{ "text": "after the edit" }]
            }),
        ),
        request(2, "textDocument/hover", hover(0)),
    ] {
        sent.extend_from_slice(codec::frame(&Message::Request(message)).as_bytes());
    }
    client.write_all(&sent).await.expect("stdin accepts");

    let mut server_said = BufReader::new(server_said);
    let mut answers = Vec::new();
    while answers.len() < 2 {
        if let Ok(Message::Response(response)) =
            codec::read(&mut server_said, &mut String::new()).await
        {
            answers.push(said(Some(response)));
        }
    }
    assert_eq!(answers[1], r#"{"contents":"after the edit"}"#);
}

#[tokio::test]
async fn the_lifecycle_is_answered_as_the_protocol_says() {
    let (service, _socket) = LspService::new(|_| Probe::default());
    let mut answers = vec![said(
        service
            .call(request(1, "textDocument/hover", hover(0)))
            .await,
    )];
    for (id, method, params) in [
        (2, "initialize", json!({ "capabilities": {} })),
        (3, "initialize", json!({ "capabilities": {} })),
        (4, "textDocument/hover", json!({ "no": "position" })),
        (5, "textDocument/definition", hover(0)),
        (6, "nobody/knows", json!(null)),
        (7, "textDocument/hover", hover(99)),
        (8, "shutdown", json!(null)),
        (9, "textDocument/hover", hover(0)),
    ] {
        answers.push(said(service.call(request(id, method, params)).await));
    }
    expect_test::expect![[r#"
        before initialize: error -32002
        initialize: {"capabilities":{}}
        initialize again: error -32600
        unreadable params: error -32602
        not implemented: error -32601
        no such method: error -32601
        a panic: error -32603
        shutdown: null
        after shutdown: error -32600"#]]
    .assert_eq(
        &[
            "before initialize",
            "initialize",
            "initialize again",
            "unreadable params",
            "not implemented",
            "no such method",
            "a panic",
            "shutdown",
            "after shutdown",
        ]
        .iter()
        .zip(answers)
        .map(|(what, answer)| format!("{what}: {answer}"))
        .collect::<Vec<_>>()
        .join("\n"),
    );
}

/// A server that keeps its client, as every real one does.
struct Keeps {
    _client: crate::protocol::Client,
}

impl LanguageServer for Keeps {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult::default())
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

/// The client's side of a session ends with the server even while its answer half is held, as a
/// harness holds both halves in one task. When the answer half held a whole client, it kept the
/// stream's channel open and the reading task waited forever. In phrost's flow tests that leaked
/// every server a test built, with its channel and recorded traffic.
#[tokio::test(start_paused = true)]
async fn the_stream_ends_once_the_server_is_gone() {
    use futures_util::StreamExt as _;

    let (service, socket) = LspService::new(|client| Keeps { _client: client });
    let (mut stream, _answers) = socket.split();
    drop(service);
    let next = tokio::time::timeout(Duration::from_secs(60), stream.next()).await;
    assert!(
        matches!(next, Ok(None)),
        "the stream outlived the server that sends on it"
    );
}

/// A stdin that delivers a script and then stays open, like a client that sends `exit` and keeps
/// the pipe. It records whether anything tried to read past the script.
struct HeldOpen {
    script: Vec<u8>,
    read_past: Arc<AtomicBool>,
}

impl AsyncRead for HeldOpen {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.script.is_empty() {
            // Real stdin waits here on a blocking thread, and a runtime does not finish while one
            // is waiting: the server would outlive the session until the client closed the pipe.
            self.read_past.store(true, Ordering::Release);
            return Poll::Pending;
        }
        let script = std::mem::take(&mut self.script);
        buf.put_slice(&script);
        Poll::Ready(Ok(()))
    }
}

async fn session(messages: Vec<Request>) -> (Ended, bool) {
    let (service, socket) = LspService::new(|_| Probe::default());
    let script = messages
        .into_iter()
        .flat_map(|message| codec::frame(&Message::Request(message)).as_bytes().to_vec())
        .collect();
    let read_past = Arc::new(AtomicBool::new(false));
    let stdin = HeldOpen {
        script,
        read_past: Arc::clone(&read_past),
    };
    let ended = tokio::time::timeout(
        Duration::from_secs(5),
        transport::serve("test", stdin, tokio::io::sink(), socket, service),
    )
    .await
    .expect("the session ends at `exit`, with the input still open");
    (ended, read_past.load(Ordering::Acquire))
}

#[tokio::test]
async fn exit_ends_the_session_with_the_input_still_open_and_nothing_read_after_it() {
    let (ended, read_past) = session(vec![
        request(1, "initialize", json!({ "capabilities": {} })),
        notification("initialized", json!({})),
        request(2, "shutdown", json!(null)),
        notification("exit", json!(null)),
    ])
    .await;
    assert_eq!(
        ended,
        Ended::Exit {
            after_shutdown: true
        }
    );
    assert_eq!(ended.exit_code(), std::process::ExitCode::SUCCESS);
    assert!(!read_past, "a read waited on the input after `exit`");
}

#[tokio::test]
async fn exit_without_shutdown_ends_with_the_status_that_says_so() {
    let (ended, read_past) = session(vec![
        request(1, "initialize", json!({ "capabilities": {} })),
        notification("exit", json!(null)),
    ])
    .await;
    assert_eq!(
        ended,
        Ended::Exit {
            after_shutdown: false
        }
    );
    assert_eq!(ended.exit_code(), std::process::ExitCode::from(1));
    assert!(!read_past);
}

/// A server whose first `initialize` never finishes, as one stuck reading a huge project would.
#[derive(Default)]
struct SlowStart {
    started: AtomicBool,
}

impl LanguageServer for SlowStart {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        if !self.started.swap(true, Ordering::AcqRel) {
            std::future::pending::<()>().await;
        }
        Ok(InitializeResult::default())
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_cancelled_initialize_can_be_sent_again() {
    let (service, _socket) = LspService::new(|_| SlowStart::default());

    let first = service.call(request(1, "initialize", json!({ "capabilities": {} })));
    tokio::time::sleep(Duration::from_millis(20)).await;
    service
        .call(notification("$/cancelRequest", json!({ "id": 1 })))
        .await;
    let first = said(first.await);
    let again = said(
        service
            .call(request(2, "initialize", json!({ "capabilities": {} })))
            .await,
    );

    expect_test::expect![[r#"
        cancelled: error -32800
        sent again: {"capabilities":{}}"#]]
    .assert_eq(&format!("cancelled: {first}\nsent again: {again}"));
}
