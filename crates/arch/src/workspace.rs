//! Reading a workspace: its members, their manifests, and their sources.

use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct Workspace {
    pub root: PathBuf,
    pub members: Vec<Member>,
}

#[derive(Debug)]
pub struct Member {
    /// From the workspace root, as the root manifest lists it: `crates/text`.
    pub dir: String,
    pub manifest: Manifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

#[derive(Debug)]
pub struct Dep {
    /// The package name, which is not the key when the dependency is renamed.
    pub name: String,
    pub kind: DepKind,
}

#[derive(Debug)]
pub struct Manifest {
    pub name: String,
    pub deps: Vec<Dep>,
}

/// One source file: its path from the workspace root, with `/` separators, and its text.
#[derive(Debug)]
pub struct SourceFile {
    pub path: String,
    pub text: String,
}

impl Workspace {
    /// The workspace `depth` directories above `manifest_dir`. A checks crate at
    /// `crates/tools/checks` calls `Workspace::above(env!("CARGO_MANIFEST_DIR"), 3)`.
    ///
    /// # Panics
    ///
    /// When there is no such ancestor, or [`Workspace::load`] panics.
    pub fn above(manifest_dir: &str, depth: usize) -> Workspace {
        let root = Path::new(manifest_dir)
            .ancestors()
            .nth(depth)
            .unwrap_or_else(|| panic!("{manifest_dir} has no ancestor {depth} levels up"));
        Workspace::load(root)
    }

    /// Every member the root manifest lists, in its order.
    ///
    /// # Panics
    ///
    /// When a manifest cannot be read or parsed, the root has no `[workspace]` table with a
    /// `members` list, or a member is a glob (`crates/*`), which is not supported: list each.
    pub fn load(root: impl Into<PathBuf>) -> Workspace {
        let root = root.into();
        let table: toml::Table = read(&root.join("Cargo.toml"))
            .parse()
            .unwrap_or_else(|err| panic!("{}: {err}", root.join("Cargo.toml").display()));
        let workspace = table.get("workspace").and_then(toml::Value::as_table);
        let members = workspace
            .and_then(|workspace| workspace.get("members"))
            .and_then(toml::Value::as_array)
            .unwrap_or_else(|| {
                panic!(
                    "{}: no [workspace] table with a `members` list; a single-package manifest is \
                     not supported",
                    root.join("Cargo.toml").display()
                )
            });
        // What `foo.workspace = true` in a member resolves through.
        let inherited = workspace
            .and_then(|workspace| workspace.get("dependencies"))
            .and_then(toml::Value::as_table)
            .cloned()
            .unwrap_or_default();
        let members = members
            .iter()
            .map(|dir| {
                let dir = dir.as_str().expect("a member is a path").to_owned();
                assert!(
                    !dir.contains(['*', '?', '[']),
                    "workspace member `{dir}` is a glob, which is not supported: list each member"
                );
                let text = read(&root.join(&dir).join("Cargo.toml"));
                Member {
                    manifest: Manifest::parse_with(&text, &inherited),
                    dir,
                }
            })
            .collect();
        Workspace { root, members }
    }

    pub fn manifests(&self) -> impl Iterator<Item = &Manifest> {
        self.members.iter().map(|member| &member.manifest)
    }

    /// Every `.rs` file under `dir` (a path from the root), recursively and sorted, skipping
    /// `target` and not following symlinked directories.
    ///
    /// # Panics
    ///
    /// When `dir` does not exist, so a check over a mistyped directory fails instead of passing
    /// over nothing, or a file cannot be read.
    pub fn rust_files(&self, dir: &str) -> Vec<SourceFile> {
        let start = self.root.join(dir);
        assert!(
            start.is_dir(),
            "{}: no such directory (rust_files({dir:?}) under {})",
            start.display(),
            self.root.display()
        );
        let mut out = Vec::new();
        self.collect(&start, &mut out);
        out
    }

    fn collect(&self, dir: &Path, out: &mut Vec<SourceFile>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut found: Vec<(PathBuf, bool)> = entries
            .flatten()
            .map(|entry| {
                // The entry's own type, not a followed link's: a symlink loop must not recurse.
                let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
                (entry.path(), is_dir)
            })
            .collect();
        found.sort();
        for (path, is_dir) in found {
            if is_dir {
                if path.file_name().is_some_and(|name| name != "target") {
                    self.collect(&path, out);
                }
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let relative = path
                    .strip_prefix(&self.root)
                    .expect("a file under the root")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(SourceFile {
                    text: read(&path),
                    path: relative,
                });
            }
        }
    }
}

impl Manifest {
    /// Parses a manifest's package name and dependencies.
    ///
    /// # Panics
    ///
    /// When the text is not a manifest with a `[package].name`.
    pub fn parse(text: &str) -> Manifest {
        Manifest::parse_with(text, &toml::Table::new())
    }

    /// [`Manifest::parse`], resolving `foo.workspace = true` through `workspace_deps` (the root's
    /// `[workspace.dependencies]`). A `package = "..."` there gives the real name, so a rename in
    /// the root cannot get around a rule either.
    ///
    /// # Panics
    ///
    /// As [`Manifest::parse`].
    pub fn parse_with(text: &str, workspace_deps: &toml::Table) -> Manifest {
        let table: toml::Table = text.parse().expect("a manifest that parses");
        let name = table["package"]["name"]
            .as_str()
            .expect("[package].name")
            .to_owned();
        let mut deps = Vec::new();
        Self::collect(&table, workspace_deps, &mut deps);
        // `[target.'cfg(...)'.dependencies]` counts too.
        if let Some(targets) = table.get("target").and_then(toml::Value::as_table) {
            for target in targets.values().filter_map(toml::Value::as_table) {
                Self::collect(target, workspace_deps, &mut deps);
            }
        }
        Manifest { name, deps }
    }

    fn collect(table: &toml::Table, workspace_deps: &toml::Table, out: &mut Vec<Dep>) {
        for (section, kind) in [
            ("dependencies", DepKind::Normal),
            ("dev-dependencies", DepKind::Dev),
            ("build-dependencies", DepKind::Build),
        ] {
            let Some(deps) = table.get(section).and_then(toml::Value::as_table) else {
                continue;
            };
            for (key, spec) in deps {
                // A renamed dependency's real name is in `package`; the key is only its local
                // name, and checking the key would let a rename bypass a rule.
                let inherits = spec.get("workspace").and_then(toml::Value::as_bool) == Some(true);
                let spec = match workspace_deps.get(key) {
                    Some(root) if inherits => root,
                    _ => spec,
                };
                let name = spec
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(key);
                out.push(Dep {
                    name: name.to_owned(),
                    kind,
                });
            }
        }
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}
