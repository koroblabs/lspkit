use expect_test::expect;

use crate::baseline::{Baseline, Lock};

const KEPT: &str = r#"
    [workspace]
    members = ["crates/a"]
    [profile.release]
    panic = "unwind"
    [profile.dev]
    panic = "unwind"
    [workspace.lints.rust]
    unsafe_code = "forbid"
"#;

fn member(name: &str, lints: &str) -> (String, String) {
    (
        name.to_owned(),
        format!("[package]\nname = \"{name}\"\n{lints}"),
    )
}

#[test]
fn a_workspace_that_keeps_the_baseline_says_nothing() {
    let baseline = Baseline {
        own_lints: &["app-heap"],
    };
    let members = [
        member("app-a", "[lints]\nworkspace = true\n"),
        member("app-heap", "[lints.rust]\nunsafe_code = \"allow\"\n"),
    ];
    assert!(baseline.judge(KEPT, &members, Lock::Committed).is_empty());
}

#[test]
fn every_way_out_of_the_baseline_is_named() {
    let root = r#"
        [workspace]
        members = ["crates/a"]
        [profile.release]
        panic = "abort"
        [profile.profiling]
        inherits = "release"
        panic = "abort"
        [workspace.lints.rust]
        unsafe_code = "warn"
    "#;
    let members = [
        member("app-a", ""),
        member("app-b", "[lints.rust]\nunsafe_code = \"allow\"\n"),
        member("app-c", "[lints]\nworkspace = true\n"),
    ];
    let found = Baseline::default().judge(root, &members, Lock::Ignored);
    expect![[r#"
        [profile.release] must state panic = "unwind": a handler's panic is contained by unwinding, and with "abort" the first one takes the server down
        [profile.dev] must state panic = "unwind": a handler's panic is contained by unwinding, and with "abort" the first one takes the server down
        [profile.profiling] sets panic = "abort", which no panic can be contained under
        [profile.release] sets panic = "abort", which no panic can be contained under
        [workspace.lints.rust] must set unsafe_code = "forbid"; a crate that needs `unsafe` is named as an exception, with its own lints
        app-a does not take the workspace's lints ([lints] workspace = true), so the unsafe forbid does not reach it
        app-b does not take the workspace's lints ([lints] workspace = true), so the unsafe forbid does not reach it
        Cargo.lock is in .gitignore: a fresh checkout would float every dependency to whatever is newest
    "#]]
    .assert_eq(&(found.join("\n") + "\n"));
    assert_eq!(
        Baseline::default().judge(KEPT, &[], Lock::Missing),
        [
            "Cargo.lock is missing: a server is a binary, and its dependency versions are part of what it is"
        ]
    );
}
