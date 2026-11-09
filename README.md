# lspkit

[![CI](https://github.com/koroblabs/lspkit/actions/workflows/ci.yml/badge.svg)](https://github.com/koroblabs/lspkit/actions/workflows/ci.yml)

Building blocks for language servers in Rust: the LSP message loop, text positions, an allocator
that returns memory to the OS, and the test and measurement tools that go with them. None of it
knows about any particular language.

It was extracted from [phrost](https://github.com/koroblabs/phrost), a PHP language server, and is
shared with [orthra](https://github.com/koroblabs/orthra), a spell checker. Each crate is used by at
least one of them.

## Crates

| Crate                              | Purpose                                                                     |
| ---------------------------------- | --------------------------------------------------------------------------- |
| [`lspkit-runtime`](crates/runtime) | The `LanguageServer` trait, JSON-RPC, and the message loop over stdio.      |
| [`lspkit-text`](crates/text)       | Line index, UTF-8/UTF-16 columns, applying a client's edits.                |
| [`lspkit-heap`](crates/heap)       | Global allocator that gives freed memory back to the OS and counts it.      |
| [`lspkit-walk`](crates/walk)       | Parallel, deterministic walk of a project's files that respects gitignore.  |
| [`lspkit-flow`](crates/flow)       | Drives a server in process for integration tests.                           |
| [`lspkit-fixture`](crates/fixture) | Multi-file test fixtures in one string, with cursor markers.                |
| [`lspkit-fuzz`](crates/fuzz)       | Seeded random edits to check incremental results against a fresh rebuild.   |
| [`lspkit-arch`](crates/arch)       | Dependency and filesystem-access rules for a workspace, checked as tests.   |
| [`lspkit-perf`](crates/perf)       | Percentiles, the process's memory and latency budgets for a `perf` command. |
| [`lspkit-ratchet`](crates/ratchet) | Per-code finding counts against a baseline that may only go down.           |
| [`lspkit-memsize`](crates/memsize) | Heap size of data structures, for memory reports.                           |
| [`lspkit-arena`](crates/arena)     | Typed arenas with `u32` ids.                                                |

The crates do not depend on each other, except `lspkit-flow` on `lspkit-runtime`.

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

impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                ..ServerCapabilities::default()
            },
            ..InitializeResult::default()
        })
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let hello = Diagnostic {
            message: "hello".to_owned(),
            ..Diagnostic::default()
        };
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
        .block_on(lspkit_runtime::serve_stdio("hello", |client| Backend { client }))
        .exit_code()
}
```

This example is compiled with the tests, like the examples in each crate's README.

To start a full server (a language crate, the LSP crate, a CLI with `serve`, `check` and `perf`,
architecture tests and CI), generate one from [`template/`](template):

```sh
just new <name> ../<name>
```

## Using it

lspkit is not on crates.io; the `lspkit` crate there is a different project. Depend on it by git and
pin a revision, the same one for every lspkit crate:

```toml
[dependencies]
lspkit-runtime = { git = "https://github.com/koroblabs/lspkit.git", rev = "<sha>" }
lspkit-text = { git = "https://github.com/koroblabs/lspkit.git", rev = "<sha>", features = ["lsp"] }
```

Take the protocol types from `lspkit_runtime::protocol` rather than depending on `ls-types` or an
LSP framework directly. `lspkit-arch` can enforce that.

Requirements and support:

- Rust 1.99 or newer, edition 2024. Async runs on tokio.
- CI runs on Linux. `lspkit-heap` maps memory itself on 64-bit Linux and Android, Apple platforms
  and Windows, and uses the system allocator elsewhere.
- The API is 0.x and changes when the servers need it to. See the [changelog](CHANGELOG.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). `just pr-ready` runs everything CI runs.

## License

Copyright (c) 2026 Korob Labs <opensource@koroblabs.com>.

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT)
at your option.

`lspkit-runtime` contains code derived from tower-lsp and tower-lsp-server, used under the MIT
license; their copyright notices are in [`crates/runtime/LICENSE-MIT`](crates/runtime/LICENSE-MIT).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the
work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
