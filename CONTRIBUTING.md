# Contributing

## Setup

The toolchain is pinned in `rust-toolchain.toml`; rustup installs it on first use. You also need:

```sh
cargo install just cargo-nextest cargo-deny --locked
```

and Node.js, which `npx` uses to run the pinned prettier for Markdown.

## Checks

```sh
just pr-ready
```

runs what CI runs: rustfmt and prettier, clippy with warnings denied, the tests, rustdoc with
warnings denied, every doc example (including the READMEs), cargo-deny, clippy for the heap on
Windows and FreeBSD, and a server generated from `template/` and built against this checkout.

Other recipes:

| Command                          | What it does                                                   |
| -------------------------------- | -------------------------------------------------------------- |
| `just test-one <crate> [filter]` | one crate's tests                                              |
| `just accept [crate]`            | accept new `expect-test` snapshots (review the diff)           |
| `just arch`                      | the workspace's dependency and filesystem-access rules         |
| `just fmt`                       | format Rust and Markdown                                       |
| `just try ../<server>`           | run a dependent server's lints and tests against this checkout |

## Guidelines

- Code here must not know about any particular language. If explaining it needs one server's terms,
  it belongs in that server.
- Add a crate only when a server already uses the code and a second one needs it.
- Crates do not depend on each other (`lspkit-flow` on `lspkit-runtime` is the exception).
  `lspkit-arch` checks this.
- A change to a hot path, the runtime or the allocator needs a before and after measurement in the
  commit message. `just bench` times lspkit-text's columns and edits and lspkit-runtime's message
  loop; run it on the revision before the change and after, on an idle machine.
- `unsafe` is allowed only in `lspkit-heap`.
- Prefer `pub(crate)`; make an item `pub` only when it is part of the API.
- Snapshot tests compare whole results with `expect-test`. Accept them with `just accept`, which
  runs plain `cargo test`: nextest runs each test in its own process, and concurrent accepts into
  one file overwrite each other.
- Tests go in a `tests.rs` file or a `tests/` directory next to the code.
- Anything about process startup, exit or pipes needs a test that runs a real binary.
- Commit messages describe the change in behavior, in the present tense.

## Changes that affect dependent servers

Servers depend on lspkit by git revision. Breaking changes are fine, but run `just try` on each
server before pushing, and after pushing run `just bump "<reason>"`, which moves every server to the
new revision, runs its checks and commits the bump. Update `template/` in the same commit as an API
change.

## License

Contributions are dual-licensed under the [Apache License 2.0](LICENSE-APACHE) and the
[MIT license](LICENSE-MIT), like the rest of the project.
