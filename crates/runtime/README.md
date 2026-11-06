# lspkit-runtime

The language-independent part of a language server: the LSP types, the `LanguageServer` trait a
server implements, and the message loop that drives it. `serve_stdio` runs a server over stdin and
stdout. It was extracted from phrost and is shared with orthra.

A server implements `LanguageServer` and sends messages to the editor through a `Client`. It takes
every protocol type from `lspkit_runtime::protocol` and does not depend on `ls-types` or an LSP
framework itself.

## Example

```rust,no_run
use lspkit_runtime::protocol::{
    Client, LanguageServer, jsonrpc,
    ls_types::{
        Diagnostic, DidOpenTextDocumentParams, InitializeParams, InitializeResult,
        ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind,
    },
};

struct Backend {
    client: Client,
}

impl Backend {
    fn new(client: Client) -> Backend {
        Backend { client }
    }
}

impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
                ..ServerCapabilities::default()
            },
            ..InitializeResult::default()
        })
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let hello = Diagnostic { message: "hello".to_owned(), ..Diagnostic::default() };
        self.client
            .publish_diagnostics(params.text_document.uri, vec![hello], None)
            .await;
    }
}

fn main() -> std::process::ExitCode {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime");
    runtime
        .block_on(lspkit_runtime::serve_stdio("hello", Backend::new))
        .exit_code()
}
```

## Behavior

The loop runs each request on its own task, with no limit on how many run at once. Notifications are
handled in the order they arrive. A panic in a handler answers that request with an error and the
server keeps running. Each response is serialized once.

- `protocol`: the `ls-types` types (pinned with `=`), JSON-RPC, `LanguageServer`, `Client` and the
  loop. Taking them from one place keeps one copy of the types in a dependency graph.
- `cancel`: `$/cancelRequest` stops a request's task. Work that does not await can poll or await the
  request's `Token`.
- `panics`: a write that unwinds poisons the data it was writing, and a panic in a spawned task is
  reported.
- `transport`: output goes through a queue, so a client that reads slowly grows the queue and does
  not block the server.
- `client`: server-to-client requests time out, so a client that never answers cannot stall the
  server.
- `progress`: progress bars, created the way the protocol and Zed require.
- `quiet`: publishes a buffer's findings after 300 ms without edits.
- `reported`: tracks which closed files a project-wide pass reported, so the next pass clears only
  those.

The module docs keep the measurements from phrost and orthra behind each choice.

Part of [lspkit](../../README.md), a toolkit for building language servers.

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT license](../../LICENSE-MIT) at your option.
