use std::fs;

use crate::workspace::Workspace;

fn root(manifest: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("a scratch directory");
    fs::write(root.path().join("Cargo.toml"), manifest).expect("a manifest");
    root
}

#[test]
#[should_panic(expected = "no such directory")]
fn a_directory_that_does_not_exist_is_not_an_empty_scan() {
    let root = root("[workspace]\nmembers = []\n");
    Workspace::load(root.path()).rust_files("crates/tpyo");
}

#[test]
#[should_panic(expected = "no [workspace] table with a `members` list")]
fn a_root_manifest_without_members_says_what_is_unsupported() {
    let root = root("[package]\nname = \"solo\"\n");
    Workspace::load(root.path());
}

#[test]
#[should_panic(expected = "is a glob, which is not supported")]
fn a_glob_member_says_what_is_unsupported() {
    let root = root("[workspace]\nmembers = [\"crates/*\"]\n");
    Workspace::load(root.path());
}

#[cfg(unix)]
#[test]
fn a_symlink_loop_is_not_followed() {
    let root = root("[workspace]\nmembers = []\n");
    let src = root.path().join("src");
    fs::create_dir(&src).expect("src");
    fs::write(src.join("a.rs"), "fn a() {}\n").expect("a file");
    std::os::unix::fs::symlink(&src, src.join("loop")).expect("a symlink");
    let files = Workspace::load(root.path()).rust_files("src");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "src/a.rs");
}
