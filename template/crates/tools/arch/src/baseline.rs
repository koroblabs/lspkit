//! The settings every server built on lspkit keeps: unwinding panics, `unsafe` forbidden in every
//! crate, and a committed lockfile. `lspkit_arch::baseline` checks them and explains each one.

use lspkit_arch::{baseline::Baseline, workspace::Workspace};

#[test]
fn the_baseline_holds() {
    let workspace = Workspace::above(env!("CARGO_MANIFEST_DIR"), 3);
    let problems = Baseline::default().violations(&workspace);
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
