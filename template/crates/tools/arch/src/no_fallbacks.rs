//! Lists every filesystem read on the request path, with its reason. In a language server, a
//! fallback is usually a read: a lookup finds nothing and goes to disk instead. A new read fails
//! this test until its reason is written down.

use lspkit_arch::{
    reads::{DEFAULT_PATTERNS, Reads},
    workspace::Workspace,
};

#[test]
fn nothing_on_the_request_path_reads_the_disk_without_a_reason() {
    let workspace = Workspace::above(env!("CARGO_MANIFEST_DIR"), 3);
    let reads = Reads {
        patterns: DEFAULT_PATTERNS,
        unpoliced: &[
            (
                "crates/cli/",
                "the command line and the instruments, never on a request path",
            ),
            ("crates/tools/", "the checks themselves"),
        ],
        allowed: &[],
        min_policed: 4,
    };
    let problems = reads.violations(&workspace.rust_files("crates"));
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
