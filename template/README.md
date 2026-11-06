# kitserver

A language server built on [lspkit](https://github.com/koroblabs/lspkit). lspkit provides everything
that is not specific to the language, and this repository holds the part that is.

- `crates/check`: the language checks. It uses no protocol types.
- `crates/lsp`: the server, on `lspkit_runtime`. It tracks open documents and publishes findings.
- `crates/cli`: `kitserver serve`, `kitserver check <path>...` and `kitserver perf <path>...`.
- `crates/tools/arch`: the crate boundaries, checked as tests.

Run `just pr-ready` before committing. A release archive holds the binary, `LICENSE-APACHE`,
`LICENSE-MIT` and the `THIRD-PARTY-LICENSES.md` that `just licenses` writes (it needs
`cargo install cargo-about`). `CLAUDE.md` describes how the work is done.
