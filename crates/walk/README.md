# lspkit-walk

Finds a project's files for a language server. Files that git ignores are left out, with or without
a `.git`. The user's global git excludes are never read. The walk runs on every thread, and the
result comes back in a fixed order.

The builder holds each server's choices: whether hidden directories count as code, which directories
are never entered, and which files are wanted.

## Example

```rust
use lspkit_walk::Walk;

let project = tempfile::tempdir().expect("a scratch directory");
for file in ["src/a.php", "src/b.php", "build/out.php", "node_modules/x/index.js", "notes.md"] {
    let path = project.path().join(file);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("directories");
    std::fs::write(path, "x\n").expect("a file");
}
std::fs::write(project.path().join(".gitignore"), "build/\n").expect("a .gitignore");

let found = Walk::new([project.path()])
    .prune(|entry| entry.is_dir && entry.path.ends_with("node_modules"))
    .keep(|path| path.extension().is_some_and(|ext| ext == "php"))
    .files();

let names: Vec<_> = found
    .iter()
    .map(|file| file.strip_prefix(project.path()).expect("under the root"))
    .collect();
// `build/` is ignored without a `.git`, and the rest is sorted by the paths' bytes.
assert_eq!(names, ["src/a.php", "src/b.php"]);
```

## Notes

- `.gitignore`, `.ignore` and `.git/info/exclude` are honored. A `.gitignore` counts without a
  `.git`, so a project unpacked from a tarball reads the same as a checkout. The user's global
  excludes differ between machines and are never read.
- The walk runs on every thread. In phrost, the directory reads and `stat`s of a tree of thirty
  thousand files took 0.3 s of every start on one thread.
- `files` sorts by the paths' bytes, so the order does not depend on thread timing. On 29 000 files
  that takes 10 ms, against 50 ms for `Path`'s component-wise order. `files_unordered` skips the
  sort for a caller that puts the files in an ordered map anyway.
- Hidden entries are skipped unless `skip_hidden(false)` is set. `prune` keeps the walk out of a
  directory, and `overrides` takes globs from a project's own settings.

Part of [lspkit](../../README.md), a toolkit for language servers.

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT license](../../LICENSE-MIT) at your option.
