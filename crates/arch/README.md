# lspkit-arch

Architecture checks for a Cargo workspace, run as ordinary tests. You write your rules as data in a
test of your own, and this crate checks the workspace's manifests and sources against them.

Rules kept only in documentation get broken. phrost's predecessor documented that its analysis crate
must not contain language semantics, and that crate grew to 28 520 lines that reached into parser
internals, because nothing failed when someone added a convenient `use`.

## Example

The edge rules can be checked on manifests alone:

```rust
use lspkit_arch::{edges::{Edges, Rule}, workspace::Manifest};

let edges = Edges {
    policed: &["app-"],
    rules: &[Rule::new("app-engine", &["app-text"]), Rule::new("app-text", &[])],
    dev_everywhere: &[],
    foreign: &[],
};

let good = Manifest::parse("[package]\nname = \"app-engine\"\n[dependencies]\napp-text = \"1\"\n");
assert!(edges.violations([&good]).is_empty());

// `app-text` depending upward on `app-engine` is not permitted.
let bad = Manifest::parse("[package]\nname = \"app-text\"\n[dependencies]\napp-engine = \"1\"\n");
assert!(!edges.violations([&bad]).is_empty());
```

In a real project this is a test that reads the workspace from disk (it needs the repository, so it
is not run here):

```rust,no_run
use lspkit_arch::{
    baseline::Baseline,
    edges::{Edges, KIT, Rule},
    reads::{DEFAULT_PATTERNS, Reads},
    workspace::Workspace,
};

#[test]
fn the_architecture_holds() {
    let workspace = Workspace::above(env!("CARGO_MANIFEST_DIR"), 2);

    let edges = Edges {
        policed: &["app-"],
        rules: &[Rule::new("app-engine", &["app-text"]), Rule::new("app-text", &[])],
        dev_everywhere: &[],
        foreign: KIT,
    };
    assert!(edges.violations(workspace.manifests()).is_empty());

    let reads = Reads {
        patterns: DEFAULT_PATTERNS,
        unpoliced: &[],
        allowed: &[],
        min_policed: 1,
    };
    assert!(reads.violations(&workspace.rust_files("crates")).is_empty());

    assert!(Baseline { own_lints: &[] }.violations(&workspace).is_empty());
}
```

## Checks

- `edges`: which crates may depend on which, and which crates may name each foreign engine. `KIT`
  adds rules every workspace gets without listing them: the protocol types and any language-server
  framework are reached only through `lspkit-runtime`.
- `reads`: every filesystem read on a request's path, each with a written reason. In a language
  server, a fallback is usually a read. The scan is plain text matching that skips test files and
  anything after `//`.
- `baseline`: settings one line could change without any test failing. These are `panic = "unwind"`
  (with `abort`, one handler's panic kills the server), `unsafe_code` forbidden with lints
  inherited, and a committed `Cargo.lock`.

The rules belong to your project. Reading, matching and reporting live here, so a fix to how a
manifest is read reaches every consumer at once.

Part of [lspkit](../../README.md), a toolkit for language servers.

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT license](../../LICENSE-MIT) at your option.
