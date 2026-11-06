# lspkit-flow

An in-process harness for flow tests of a language server built on `lspkit-runtime`. Flow tests
cover bugs that only show up in a running server: version guards, readiness, and what an edit does
to the next request. The service is driven directly rather than over a pipe, so tests are
deterministic. A task stands in for the editor and records everything the server sends it.

Use it only as a dev-dependency.

## Example

```rust
use lspkit_flow::{Client, Flow};
use lspkit_runtime::protocol::{
    Client as Editor, LanguageServer, jsonrpc,
    ls_types::{Diagnostic, DidOpenTextDocumentParams, InitializeParams, InitializeResult},
};
use serde_json::json;

struct Backend {
    client: Editor,
}

impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        Ok(InitializeResult::default())
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let finding = Diagnostic { message: "found".to_owned(), ..Diagnostic::default() };
        self.client
            .publish_diagnostics(params.text_document.uri, vec![finding], None)
            .await;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut flow = Flow::new(|client| Backend { client }, Client::Answering);
    let initialized = flow.request("initialize", json!({ "capabilities": {} })).await;
    assert!(initialized.is_ok());

    let uri = "file:///a.txt";
    flow.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "", "version": 1, "text": "a" } }),
    )
    .await;

    assert_eq!(flow.methods(), ["textDocument/publishDiagnostics"]);
    assert_eq!(flow.publications(uri).len(), 1);
}
```

## Usage

- `Client::Answering` answers every server-to-client request with `null`, as an editor does for
  requests a test does not care about. Without an answer, a server awaiting
  `client/registerCapability` hangs `initialized` forever. `Client::Silent` records requests and
  answers none; it found a startup hang in phrost.
- `Flow::request`, `notify` and `start` drive the server. `start` returns a request still in flight,
  for a test that cancels or edits while it runs. `methods`, `params`, `publications` and `logged`
  report what the server sent.
- `settle` yields instead of sleeping, and `after` advances a paused tokio clock, so a debounced
  publish is tested without waiting out the delay.
- `forget` drops the recorded traffic, so a leak test measures the server and not the stand-in
  client.

Part of [lspkit](../../README.md), a toolkit for language servers.

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT license](../../LICENSE-MIT) at your option.
