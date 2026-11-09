//! Times the message loop on the workloads the CHANGELOG measures: a client on duplex pipes drives
//! `transport::serve`, and each scenario is the best of five rounds.
//!
//! `cargo bench -p lspkit-runtime [-- <filter>]`. Run it before and after a change, on an idle
//! machine. It times the whole round trip, so a change to the handlers' own work (building 500
//! completion items) shows too; what the loop adds is clearest in the first scenario, whose
//! requests the server answers at once.
use std::time::{Duration, Instant};

use lspkit_runtime::{
    protocol::{
        Client, LanguageServer, LspService, jsonrpc,
        ls_types::{
            CompletionItem, CompletionItemKind, CompletionParams, CompletionResponse, Diagnostic,
            DiagnosticSeverity, DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
            DidOpenTextDocumentParams, Documentation, FileChangeType, FileEvent, InitializeParams,
            InitializeResult, NumberOrString, Position, Range, TextDocumentContentChangeEvent,
            TextDocumentItem, Uri, VersionedTextDocumentIdentifier,
        },
    },
    transport,
};
use tokio::io::{
    AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream,
};

struct Bench {
    client: Client,
}

impl LanguageServer for Bench {
    async fn initialize(&self, _: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        Ok(InitializeResult::default())
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.client
            .publish_diagnostics(params.text_document.uri, diagnostics(50), None)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        std::hint::black_box(params);
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        std::hint::black_box(params);
    }

    async fn completion(&self, _: CompletionParams) -> jsonrpc::Result<Option<CompletionResponse>> {
        Ok(Some(CompletionResponse::Array(completions(500))))
    }
}

fn uri(i: usize) -> Uri {
    format!("file:///project/src/file_{i}.php")
        .parse()
        .expect("a file URI")
}

fn diagnostics(count: usize) -> Vec<Diagnostic> {
    (0..count)
        .map(|i| Diagnostic {
            range: Range::new(Position::new(i as u32, 4), Position::new(i as u32, 17)),
            severity: Some(DiagnosticSeverity::WARNING),
            code: Some(NumberOrString::String("undefined-method".into())),
            source: Some("phrost".into()),
            message: format!("Call to an undefined method Foo::bar{i}() on an object of type Foo"),
            ..Diagnostic::default()
        })
        .collect()
}

fn completions(count: usize) -> Vec<CompletionItem> {
    (0..count)
        .map(|i| CompletionItem {
            label: format!("getUserPreferenceValue{i}"),
            kind: Some(CompletionItemKind::METHOD),
            detail: Some(format!(
                "public function getUserPreferenceValue{i}(string $key, mixed $default = null): mixed"
            )),
            documentation: Some(Documentation::String(format!(
                "Returns the preference stored under `$key` for the current user, or `$default` \
                 when unset. Item {i}."
            ))),
            insert_text: Some(format!("getUserPreferenceValue{i}($0)")),
            sort_text: Some(format!("{i:05}")),
            filter_text: Some(format!("getUserPreferenceValue{i}")),
            ..CompletionItem::default()
        })
        .collect()
}

fn framed(body: &[u8]) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body);
    out
}

fn notification(method: &str, params: impl serde::Serialize) -> Vec<u8> {
    let body = serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params});
    framed(&serde_json::to_vec(&body).expect("JSON"))
}

fn request(id: u64, method: &str, params: impl serde::Serialize) -> Vec<u8> {
    let body = serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    framed(&serde_json::to_vec(&body).expect("JSON"))
}

/// Reads framed messages into one reused buffer.
struct Reader {
    input: BufReader<DuplexStream>,
    header: Vec<u8>,
    body: Vec<u8>,
}

impl Reader {
    async fn next(&mut self) -> Option<&[u8]> {
        let mut length = None;
        loop {
            self.header.clear();
            let read = self.input.read_until(b'\n', &mut self.header).await.ok()?;
            if read == 0 {
                return None;
            }
            let line = std::str::from_utf8(&self.header).ok()?.trim_end();
            if line.is_empty() {
                if length.is_some() {
                    break;
                }
                continue;
            }
            if let Some(value) = line.strip_prefix("Content-Length:") {
                length = value.trim().parse::<usize>().ok();
            }
        }
        self.body.clear();
        self.body.resize(length?, 0);
        self.input.read_exact(&mut self.body).await.ok()?;
        Some(&self.body)
    }
}

fn is_publish(body: &[u8]) -> bool {
    std::str::from_utf8(body).is_ok_and(|body| body.contains("textDocument/publishDiagnostics"))
}

fn is_response(body: &[u8]) -> bool {
    std::str::from_utf8(body)
        .is_ok_and(|body| body.contains("\"result\"") || body.contains("\"error\""))
}

/// A request the server answers at once (method not found). After a batch of notifications it is
/// dispatched once they are handled, so its answer says the batch is done.
fn sentinel(next_id: &mut u64, bytes: &mut Vec<u8>) {
    *next_id += 1;
    bytes.extend(request(
        *next_id,
        "textDocument/hover",
        serde_json::json!({}),
    ));
}

struct Scenario {
    name: String,
    bytes: Vec<u8>,
    publishes: usize,
    responses: usize,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    // `cargo bench` passes `--bench` to the binary; a word after `--` is the filter.
    let filter = std::env::args()
        .skip(1)
        .find(|arg| !arg.starts_with("--"))
        .unwrap_or_default();
    let (mut to_server, server_stdin) = tokio::io::duplex(1 << 20);
    let (server_stdout, from_server) = tokio::io::duplex(1 << 20);
    let (service, socket) = LspService::new(|client| Bench { client });
    let server = tokio::spawn(transport::serve(
        "bench",
        server_stdin,
        server_stdout,
        socket,
        service,
    ));
    let mut reader = Reader {
        input: BufReader::with_capacity(1 << 16, from_server),
        header: Vec::new(),
        body: Vec::new(),
    };

    to_server
        .write_all(&request(1, "initialize", InitializeParams::default()))
        .await
        .expect("stdin accepts");
    assert!(is_response(reader.next().await.expect("an answer")));
    to_server
        .write_all(&notification("initialized", serde_json::json!({})))
        .await
        .expect("stdin accepts");

    let mut next_id = 100u64;
    let mut scenarios = Vec::new();
    {
        let mut bytes = Vec::new();
        for _ in 0..2000 {
            sentinel(&mut next_id, &mut bytes);
        }
        scenarios.push(Scenario {
            name: "2 000 small requests answered at once".into(),
            bytes,
            publishes: 0,
            responses: 2000,
        });
    }
    {
        let mut bytes = Vec::new();
        for i in 0..2000 {
            let params = DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri(i),
                    language_id: "php".into(),
                    version: 1,
                    text: "<?php\n".into(),
                },
            };
            bytes.extend(notification("textDocument/didOpen", params));
        }
        scenarios.push(Scenario {
            name: "2 000 didOpen, each answered with 50 diagnostics".into(),
            bytes,
            publishes: 2000,
            responses: 0,
        });
    }
    {
        let mut bytes = Vec::new();
        let changes: Vec<FileEvent> = (0..1000)
            .map(|i| FileEvent::new(uri(i), FileChangeType::CHANGED))
            .collect();
        for _ in 0..200 {
            let params = DidChangeWatchedFilesParams {
                changes: changes.clone(),
            };
            bytes.extend(notification("workspace/didChangeWatchedFiles", params));
        }
        let each = bytes.len() / 200 / 1024;
        sentinel(&mut next_id, &mut bytes);
        scenarios.push(Scenario {
            name: format!("200 didChangeWatchedFiles of 1 000 events ({each} KB each)"),
            bytes,
            publishes: 0,
            responses: 1,
        });
    }
    {
        let mut bytes = Vec::new();
        let size = serde_json::to_vec(&completions(500)).expect("JSON").len() / 1024;
        for _ in 0..300 {
            next_id += 1;
            let params = serde_json::json!({
                "textDocument": {"uri": "file:///a.php"},
                "position": {"line": 0, "character": 0},
            });
            bytes.extend(request(next_id, "textDocument/completion", params));
        }
        scenarios.push(Scenario {
            name: format!("300 completions of {size} KB"),
            bytes,
            publishes: 0,
            responses: 300,
        });
    }
    {
        let mut bytes = Vec::new();
        let text =
            "<?php\nfunction compute(int $alpha, int $beta): int { return $alpha * $beta + 1; }\n"
                .repeat(600);
        for version in 0..1000 {
            let params = DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: uri(0),
                    version,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: text.clone(),
                }],
            };
            bytes.extend(notification("textDocument/didChange", params));
        }
        sentinel(&mut next_id, &mut bytes);
        scenarios.push(Scenario {
            name: format!("1 000 full-sync didChange of {} KB", text.len() / 1024),
            bytes,
            publishes: 0,
            responses: 1,
        });
    }

    println!("{:<60} {:>10}", "scenario (best of five)", "ms");
    for scenario in &scenarios {
        if !scenario.name.contains(&filter) {
            continue;
        }
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let started = Instant::now();
            let writer = {
                let bytes = scenario.bytes.clone();
                let mut to_server = to_server;
                tokio::spawn(async move {
                    to_server.write_all(&bytes).await.expect("stdin accepts");
                    to_server
                })
            };
            let (mut publishes, mut responses) = (0, 0);
            while publishes < scenario.publishes || responses < scenario.responses {
                let body = reader.next().await.expect("the server is running");
                if is_publish(body) {
                    publishes += 1;
                } else if is_response(body) {
                    responses += 1;
                }
            }
            best = best.min(started.elapsed());
            to_server = writer.await.expect("the writer finished");
        }
        println!("{:<60} {:>10.2}", scenario.name, best.as_secs_f64() * 1e3);
    }

    next_id += 1;
    to_server
        .write_all(&request(next_id, "shutdown", serde_json::Value::Null))
        .await
        .expect("stdin accepts");
    assert!(is_response(reader.next().await.expect("an answer")));
    to_server
        .write_all(&notification("exit", serde_json::Value::Null))
        .await
        .expect("stdin accepts");
    let ended = server.await.expect("the loop finished");
    assert!(matches!(
        ended,
        transport::Ended::Exit {
            after_shutdown: true
        }
    ));
}
