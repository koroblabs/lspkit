//! lspkit checked against its own rules, with this crate as its checks crate.

use crate::{
    baseline::Baseline,
    edges::{Edges, Foreign, Pattern, Rule},
    reads::{Allowed, DEFAULT_PATTERNS, Reads},
    workspace::Workspace,
};

/// No lspkit crate depends on another, so a consumer takes only the crates it uses. The exception
/// is `lspkit-flow`, which drives the runtime's service and is only a dev-dependency of servers
/// that already depend on the runtime.
const RULES: &[Rule] = &[
    Rule::new("lspkit-arch", &[]),
    Rule::new("lspkit-arena", &[]),
    Rule::new("lspkit-fixture", &[]),
    Rule::new("lspkit-flow", &["lspkit-runtime"]),
    Rule::new("lspkit-fuzz", &[]),
    Rule::new("lspkit-heap", &[]),
    Rule::new("lspkit-memsize", &[]),
    Rule::new("lspkit-perf", &[]),
    Rule::new("lspkit-ratchet", &[]),
    Rule::new("lspkit-runtime", &[]),
    Rule::new("lspkit-text", &[]),
    Rule::new("lspkit-walk", &[]),
];

const FOREIGN: &[Foreign] = &[
    Foreign {
        pattern: Pattern::Exact("ls-types"),
        owners: &["lspkit-runtime", "lspkit-text"],
        why: "the protocol's types: the runtime, which every server takes them through, and \
              lspkit-text behind its `lsp` feature, which converts their positions",
    },
    Foreign {
        pattern: Pattern::Exact("libmimalloc-sys"),
        owners: &["lspkit-heap"],
        why: "the one crate allowed to write `unsafe`, for the allocator and the foreign calls under it",
    },
];

fn workspace() -> Workspace {
    Workspace::above(env!("CARGO_MANIFEST_DIR"), 2)
}

#[test]
fn the_edges_hold() {
    let workspace = workspace();
    let edges = Edges {
        policed: &["lspkit-"],
        rules: RULES,
        dev_everywhere: &[],
        foreign: FOREIGN,
    };
    let mut problems = edges.violations(workspace.manifests());
    problems.extend(edges.stale_rules(workspace.manifests()));
    problems.extend(edges.cycles());
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

#[test]
fn nothing_reads_the_disk_but_the_checks() {
    let workspace = workspace();
    let reads = Reads {
        patterns: DEFAULT_PATTERNS,
        unpoliced: &[("crates/arch/", "reads a workspace in order to check it")],
        allowed: &[
            Allowed {
                file: "crates/perf/src/rss.rs",
                why: "the process's own resident memory, for a perf command; never on a request path",
            },
            Allowed {
                file: "crates/walk/src/lib.rs",
                why: "finding a project's files: where the set of files comes from, at startup and \
                      when the disk changes, not a stand-in for a missing answer",
            },
            Allowed {
                file: "crates/ratchet/src/lib.rs",
                why: "a project's baseline file, read and written by a check command; never on a \
                      request path",
            },
        ],
        min_policed: 5,
    };
    let problems = reads.violations(&workspace.rust_files("crates"));
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

#[test]
fn the_baseline_holds() {
    let baseline = Baseline {
        own_lints: &["lspkit-heap"],
    };
    let problems = baseline.violations(&workspace());
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

/// Every crate carries the workspace's license files, because a tool that lists a binary's licenses
/// (cargo-about) reads them from the crate's own directory. `lspkit-runtime`'s MIT file adds the
/// notices of the code it derives from.
#[test]
fn every_crate_carries_the_license_files() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |path: &std::path::Path| {
        std::fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
    };
    let (mit, apache) = (
        read(&root.join("LICENSE-MIT")),
        read(&root.join("LICENSE-APACHE")),
    );
    let holder = mit.lines().nth(2).expect("a copyright line");
    let mut problems = Vec::new();
    for rule in RULES {
        let dir = root
            .join("crates")
            .join(rule.krate.trim_start_matches("lspkit-"));
        if read(&dir.join("LICENSE-APACHE")) != apache {
            problems.push(format!("{}: LICENSE-APACHE differs", rule.krate));
        }
        let own = read(&dir.join("LICENSE-MIT"));
        let derived = rule.krate == "lspkit-runtime" && own.lines().nth(2) == Some(holder);
        if own != mit && !derived {
            problems.push(format!("{}: LICENSE-MIT differs", rule.krate));
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
