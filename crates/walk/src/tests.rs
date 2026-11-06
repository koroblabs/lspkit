use std::{fs, path::Path};

use crate::{OverrideBuilder, Walk};

/// A project on disk: `files` relative to its root, each with a line of text.
fn project(files: &[&str]) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("a scratch directory");
    for file in files {
        let path = root.path().join(file);
        fs::create_dir_all(path.parent().expect("a parent")).expect("directories");
        fs::write(&path, "x\n").expect("a file");
    }
    root
}

fn relative(root: &Path, files: Vec<std::path::PathBuf>) -> Vec<String> {
    files
        .into_iter()
        .map(|file| {
            file.strip_prefix(root)
                .expect("under the root")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

#[test]
fn ignored_hidden_and_pruned_files_are_left_out_without_a_git_repository() {
    let root = project(&[
        ".gitignore",
        "src/a.rs",
        "src/b.rs",
        "build/out.rs",
        ".cache/c.rs",
        "node_modules/d/index.js",
        "docs/notes.md",
    ]);
    fs::write(root.path().join(".gitignore"), "build/\n").expect("write");
    let found = Walk::new([root.path()])
        .prune(|entry| entry.is_dir && entry.path.ends_with("node_modules"))
        .files();
    assert_eq!(
        relative(root.path(), found),
        ["docs/notes.md", "src/a.rs", "src/b.rs"],
        "no .git, and the .gitignore still counts; hidden and pruned are left out"
    );
}

#[test]
fn a_server_can_walk_hidden_directories_ignore_nothing_and_keep_some_files() {
    let root = project(&[".gitignore", ".hidden/a.php", "vendor/b.php", "c.txt"]);
    fs::write(root.path().join(".gitignore"), "vendor/\n").expect("write");
    let found = Walk::new([root.path()])
        .skip_hidden(false)
        .ignore_files(false)
        .keep(|path| path.extension().is_some_and(|ext| ext == "php"))
        .files();
    assert_eq!(
        relative(root.path(), found),
        [".hidden/a.php", "vendor/b.php"]
    );
}

#[test]
fn a_projects_own_settings_exclude_like_an_ignore_file() {
    let root = project(&["keep.md", "generated/skip.md"]);
    let mut overrides = OverrideBuilder::new(root.path());
    overrides.add("!generated/").expect("a glob");
    let found = Walk::new([root.path()])
        .overrides(overrides.build().expect("overrides"))
        .files();
    assert_eq!(relative(root.path(), found), ["keep.md"]);
}

#[test]
fn several_roots_and_a_root_that_is_a_file_come_back_in_one_sorted_list() {
    let root = project(&["b/two.rs", "a/one.rs", "c.rs"]);
    let found = Walk::new([
        root.path().join("b"),
        root.path().join("c.rs"),
        root.path().join("a"),
    ])
    .files();
    assert_eq!(
        relative(root.path(), found),
        ["a/one.rs", "b/two.rs", "c.rs"]
    );
    assert!(
        Walk::new(Vec::<std::path::PathBuf>::new())
            .files()
            .is_empty()
    );
}

#[test]
fn an_ancestors_ignore_file_does_not_empty_a_project_without_a_git_repository() {
    let outer = project(&["inner/src/a.rs", "inner/b.rs"]);
    fs::write(outer.path().join(".gitignore"), "*\n").expect("write");
    let inner = outer.path().join("inner");
    let found = Walk::new([&inner]).files();
    assert_eq!(relative(&inner, found), ["b.rs", "src/a.rs"]);
}

#[test]
fn a_repositorys_root_ignore_file_applies_to_a_walk_from_a_subdirectory() {
    let outer = project(&["sub/a.rs", "sub/b.log"]);
    fs::create_dir(outer.path().join(".git")).expect("a .git");
    fs::write(outer.path().join(".gitignore"), "*.log\n").expect("write");
    let sub = outer.path().join("sub");
    let found = Walk::new([&sub]).files();
    assert_eq!(relative(&sub, found), ["a.rs"]);
}

#[test]
fn overlapping_roots_list_a_file_once() {
    let root = project(&["a/b/c.rs", "a/d.rs"]);
    let a = root.path().join("a");
    let found = Walk::new([a.clone(), a.join("b"), a.clone()]).files();
    assert_eq!(relative(&a, found), ["b/c.rs", "d.rs"]);
}
