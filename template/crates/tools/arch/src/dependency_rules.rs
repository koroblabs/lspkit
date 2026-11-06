//! The allowed-edges matrix. An unlisted crate fails the test, so a new crate must declare what it
//! may depend on.

use lspkit_arch::{
    edges::{Edges, Foreign, Rule},
    workspace::Workspace,
};

const RULES: &[Rule] = &[
    // The language crate gets no protocol: `lspkit-text` without its `lsp` feature.
    Rule::new("kitserver-check", &["lspkit-text"]).dev(&["lspkit-fixture"]),
    Rule::new(
        "kitserver-lsp",
        &["kitserver-check", "lspkit-runtime", "lspkit-text"],
    )
    .dev(&["lspkit-flow", "lspkit-fuzz"]),
    Rule::new(
        "kitserver-cli",
        &[
            "kitserver-*",
            "lspkit-heap",
            "lspkit-perf",
            "lspkit-ratchet",
            "lspkit-walk",
        ],
    ),
    Rule::new("kitserver-arch", &["lspkit-arch"]),
];

/// Foreign engines, each with the one crate allowed to name it. Empty until the language needs one;
/// a parser would go here, owned by `kitserver-check` or a crate of its own. lspkit already applies
/// this rule to the protocol (`edges::KIT`).
const FOREIGN: &[Foreign] = &[];

#[test]
fn the_edges_hold() {
    let workspace = Workspace::above(env!("CARGO_MANIFEST_DIR"), 3);
    let edges = Edges {
        policed: &["kitserver-", "lspkit-"],
        rules: RULES,
        dev_everywhere: &[],
        foreign: FOREIGN,
    };
    let mut problems = edges.violations(workspace.manifests());
    problems.extend(edges.stale_rules(workspace.manifests()));
    problems.extend(edges.cycles());
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
