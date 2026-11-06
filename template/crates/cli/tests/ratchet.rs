//! `check --ratchet` as CI runs it: a baseline is written with `--accept`, holds, and then breaks
//! when one more finding appears.

use std::process::Command;

fn check(project: &std::path::Path, flags: &[&str]) -> (Option<i32>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_kitserver"))
        .arg("check")
        .args(flags)
        .arg(project)
        .output()
        .expect("the checker runs");
    let said = String::from_utf8_lossy(&output.stdout).into_owned();
    (output.status.code(), said)
}

#[test]
fn a_baseline_holds_until_a_finding_is_added() {
    let project = tempfile::tempdir().expect("a scratch project");
    std::fs::write(project.path().join("a.txt"), "one \ntwo\n").expect("a file");

    let (accepted, _) = check(project.path(), &["--accept"]);
    assert_eq!(accepted, Some(0));
    assert!(project.path().join("kitserver-baseline.toml").is_file());

    let (held, said) = check(project.path(), &["--ratchet"]);
    assert_eq!(held, Some(0), "{said}");

    std::fs::write(project.path().join("b.txt"), "three \n").expect("a second file");
    let (broken, said) = check(project.path(), &["--ratchet"]);
    assert_eq!(broken, Some(1), "{said}");
    assert!(
        said.contains("trailing-whitespace") && said.contains("ROSE"),
        "{said}"
    );
}
