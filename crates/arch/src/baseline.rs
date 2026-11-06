//! Settings every server built on lspkit keeps, checked by a test.
//!
//! Each is a one-line change that no other test would catch:
//!
//! - `panic = "unwind"` in the release and dev profiles, and `"abort"` in none. lspkit's loop
//!   catches a handler's panic and answers that request with an error. With `abort` everything
//!   still compiles and passes, and the first panic in a handler kills the editor's server.
//! - `unsafe_code = "forbid"` for the workspace, with every crate inheriting the workspace's lints.
//!   A crate with its own lints table silently opts out of the forbid. A crate that needs `unsafe`
//!   is named as an exception, as lspkit names `lspkit-heap`.
//! - `Cargo.lock` committed, because a server is a binary. phrost's predecessor ignored its
//!   lockfile, and a fresh checkout pulled a parser version that broke typed class constants.

use std::{fs, path::Path};

use crate::workspace::Workspace;

/// The baseline for one workspace.
#[derive(Debug, Clone, Copy, Default)]
pub struct Baseline<'a> {
    /// Crates that keep lints of their own, each because it writes `unsafe`: `["lspkit-heap"]`.
    pub own_lints: &'a [&'a str],
}

impl Baseline<'_> {
    /// Every way `workspace` departs from the baseline.
    ///
    /// # Panics
    ///
    /// When the root or a member manifest cannot be read or parsed.
    pub fn violations(&self, workspace: &Workspace) -> Vec<String> {
        let root = read(&workspace.root.join("Cargo.toml"));
        let members: Vec<(String, String)> = workspace
            .members
            .iter()
            .map(|member| {
                let text = read(&workspace.root.join(&member.dir).join("Cargo.toml"));
                (member.manifest.name.clone(), text)
            })
            .collect();
        let lock = Lock::of(&workspace.root);
        self.judge(&root, &members, lock)
    }

    /// [`Baseline::violations`] over manifests already read: the root's, each member's by name, and
    /// the lockfile's state.
    pub fn judge(&self, root: &str, members: &[(String, String)], lock: Lock) -> Vec<String> {
        let mut out = Vec::new();
        let root: toml::Table = root.parse().expect("the root manifest parses");

        let profiles = root.get("profile").and_then(toml::Value::as_table);
        for required in ["release", "dev"] {
            let panic = profiles
                .and_then(|profiles| profiles.get(required))
                .and_then(|profile| profile.get("panic"))
                .and_then(toml::Value::as_str);
            if panic != Some("unwind") {
                out.push(format!(
                    "[profile.{required}] must state panic = \"unwind\": a handler's panic is \
                     contained by unwinding, and with \"abort\" the first one takes the server down"
                ));
            }
        }
        for (name, profile) in profiles.into_iter().flatten() {
            if profile.get("panic").and_then(toml::Value::as_str) == Some("abort") {
                out.push(format!(
                    "[profile.{name}] sets panic = \"abort\", which no panic can be contained under"
                ));
            }
        }

        let forbid = root
            .get("workspace")
            .and_then(|workspace| workspace.get("lints"))
            .and_then(|lints| lints.get("rust"))
            .and_then(|rust| rust.get("unsafe_code"))
            .and_then(toml::Value::as_str);
        if forbid != Some("forbid") {
            out.push(
                "[workspace.lints.rust] must set unsafe_code = \"forbid\"; a crate that needs \
                 `unsafe` is named as an exception, with its own lints"
                    .to_owned(),
            );
        }

        for (name, text) in members {
            if self.own_lints.contains(&name.as_str()) {
                continue;
            }
            let manifest: toml::Table = text.parse().expect("a member manifest parses");
            let inherits = manifest
                .get("lints")
                .and_then(|lints| lints.get("workspace"))
                .and_then(toml::Value::as_bool)
                == Some(true);
            if !inherits {
                out.push(format!(
                    "{name} does not take the workspace's lints ([lints] workspace = true), so the \
                     unsafe forbid does not reach it"
                ));
            }
        }

        match lock {
            Lock::Committed => {}
            Lock::Missing => out.push(
                "Cargo.lock is missing: a server is a binary, and its dependency versions are \
                 part of what it is"
                    .to_owned(),
            ),
            Lock::Ignored => out.push(
                "Cargo.lock is in .gitignore: a fresh checkout would float every dependency to \
                 whatever is newest"
                    .to_owned(),
            ),
        }
        out
    }
}

/// Where a workspace's lockfile stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    Committed,
    Missing,
    /// Present, and named by the root `.gitignore`.
    Ignored,
}

impl Lock {
    /// Checks the lockfile's presence and the root `.gitignore`. Only a line that is exactly
    /// `Cargo.lock` or `/Cargo.lock` counts as ignoring it; a glob (`*.lock`), a negation, a nested
    /// `.gitignore` or `.git/info/exclude` is not seen.
    fn of(root: &Path) -> Lock {
        if !root.join("Cargo.lock").is_file() {
            return Lock::Missing;
        }
        let ignored = fs::read_to_string(root.join(".gitignore")).is_ok_and(|ignore| {
            ignore
                .lines()
                .map(str::trim)
                .any(|line| matches!(line, "Cargo.lock" | "/Cargo.lock"))
        });
        if ignored {
            Lock::Ignored
        } else {
            Lock::Committed
        }
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}
