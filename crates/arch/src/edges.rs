//! Which crates may depend on which, and which crates may name each foreign engine.
//!
//! Only dependencies whose names start with one of [`Edges::policed`] are checked against the
//! rules, and every [`Foreign`] engine is checked against its owners. Other third-party crates are
//! each crate's own choice. The rules cover a project's own layers and the engines it expects to
//! swap.
//!
//! [`KIT`] adds lspkit's own rules to every workspace's foreign engines. They cannot be turned
//! off, so a server does not have to remember to list them.

use std::collections::HashMap;

use crate::workspace::{DepKind, Manifest};

/// One crate's permitted dependencies. A name ending in `*` is a prefix: `phrost-*`.
#[derive(Debug, Clone, Copy)]
pub struct Rule {
    pub krate: &'static str,
    /// Permitted in `[dependencies]` and `[build-dependencies]`.
    pub deps: &'static [&'static str],
    /// Permitted on top of `deps`, in `[dev-dependencies]` only.
    pub dev: &'static [&'static str],
}

impl Rule {
    pub const fn new(krate: &'static str, deps: &'static [&'static str]) -> Rule {
        Rule {
            krate,
            deps,
            dev: &[],
        }
    }

    pub const fn dev(self, dev: &'static [&'static str]) -> Rule {
        Rule { dev, ..self }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Pattern {
    Exact(&'static str),
    Prefix(&'static str),
}

impl Pattern {
    fn matches(self, name: &str) -> bool {
        match self {
            Pattern::Exact(exact) => name == exact,
            Pattern::Prefix(prefix) => name.starts_with(prefix),
        }
    }
}

/// A foreign engine, and the only crates that may name it, in any kind of dependency.
#[derive(Debug, Clone, Copy)]
pub struct Foreign {
    pub pattern: Pattern,
    /// A name ending in `*` is a prefix: `lspkit-*`.
    pub owners: &'static [&'static str],
    pub why: &'static str,
}

/// Crates a server reaches only through lspkit, checked in every workspace that uses this crate.
///
/// These are the protocol's types and every language-server framework. A server takes the types
/// and the loop from `lspkit_runtime::protocol`, so replacing either is one change in lspkit and a
/// revision bump in each server. A server that named the framework directly would break when it
/// was replaced, as `tower-lsp-server` was by lspkit's own loop. Two copies of the protocol types
/// in one graph do not compile, so the other frameworks are listed too.
pub const KIT: &[Foreign] = &[
    Foreign {
        pattern: Pattern::Exact("tower-lsp-server"),
        owners: &["lspkit-*"],
        why: KIT_PROTOCOL,
    },
    Foreign {
        pattern: Pattern::Exact("ls-types"),
        owners: &["lspkit-*"],
        why: KIT_PROTOCOL,
    },
    Foreign {
        pattern: Pattern::Exact("tower-lsp"),
        owners: &["lspkit-*"],
        why: KIT_PROTOCOL,
    },
    Foreign {
        pattern: Pattern::Exact("lsp-types"),
        owners: &["lspkit-*"],
        why: KIT_PROTOCOL,
    },
    Foreign {
        pattern: Pattern::Exact("lsp-server"),
        owners: &["lspkit-*"],
        why: KIT_PROTOCOL,
    },
    Foreign {
        pattern: Pattern::Exact("async-lsp"),
        owners: &["lspkit-*"],
        why: KIT_PROTOCOL,
    },
];

const KIT_PROTOCOL: &str = "a server takes the protocol from `lspkit_runtime::protocol`, so the \
                            framework under it is replaced in lspkit once, not in every server";

/// A workspace's rules about its own graph.
#[derive(Debug, Clone, Copy)]
pub struct Edges<'a> {
    /// Name prefixes the rules apply to: `["orthra-", "lspkit-"]`.
    pub policed: &'a [&'a str],
    pub rules: &'a [Rule],
    /// Test-only dependencies every crate may take, such as a fixture format. These describe test
    /// inputs and are not layers of the project.
    pub dev_everywhere: &'a [&'a str],
    pub foreign: &'a [Foreign],
}

impl Edges<'_> {
    /// Every edge the rules do not allow, and every crate that has no rule, so a new crate needs a
    /// rule before it passes.
    pub fn violations<'m>(&self, manifests: impl IntoIterator<Item = &'m Manifest>) -> Vec<String> {
        let mut out = Vec::new();
        for manifest in manifests {
            let Some(rule) = self.rules.iter().find(|rule| rule.krate == manifest.name) else {
                out.push(format!(
                    "{} has no rule: adding a crate means declaring what it may reach",
                    manifest.name
                ));
                continue;
            };
            for dep in &manifest.deps {
                let name = dep.name.as_str();
                if self.policed.iter().any(|prefix| name.starts_with(prefix)) {
                    let allowed = allows(rule.deps, name)
                        || (dep.kind == DepKind::Dev
                            && (allows(rule.dev, name) || allows(self.dev_everywhere, name)));
                    if !allowed {
                        out.push(format!(
                            "{} -> {name} ({:?}) is not an allowed edge. If it is genuinely right, \
                             add it to the rules with the reason in the commit; otherwise route the \
                             fact through a layer that is already allowed",
                            manifest.name, dep.kind
                        ));
                    }
                }
                for engine in self
                    .foreign
                    .iter()
                    .chain(KIT)
                    .filter(|engine| engine.pattern.matches(name))
                {
                    if !allows(engine.owners, &manifest.name) {
                        out.push(format!(
                            "{} names {name}, which only {} may: {}",
                            manifest.name,
                            engine.owners.join(" and "),
                            engine.why
                        ));
                    }
                }
            }
        }
        out
    }

    /// Rules that name crates which are not workspace members.
    ///
    /// Separate from [`Edges::violations`] because a workspace may write a rule before its crate.
    /// Otherwise call it too: a rule left behind by a deleted crate would grant its edges to the
    /// next crate that takes the name.
    pub fn stale_rules<'m>(
        &self,
        manifests: impl IntoIterator<Item = &'m Manifest>,
    ) -> Vec<String> {
        let names: Vec<&str> = manifests
            .into_iter()
            .map(|manifest| manifest.name.as_str())
            .collect();
        self.rules
            .iter()
            .filter(|rule| !names.contains(&rule.krate))
            .map(|rule| {
                format!(
                    "the rules name {}, which is not a workspace member",
                    rule.krate
                )
            })
            .collect()
    }

    /// Cycles in the allowed edges. Cargo rejects a cyclic graph only when someone adds the edge;
    /// this rejects the rule that would permit it.
    ///
    /// A depth-first search that marks finished crates, so a dense graph costs each edge once
    /// instead of once per path. Each cycle is reported once, rotated to start at its
    /// lexicographically smallest crate.
    pub fn cycles(&self) -> Vec<String> {
        let mut state: HashMap<&str, Visit> = HashMap::new();
        let mut out = Vec::new();
        for rule in self.rules {
            self.visit(rule.krate, &mut state, &mut Vec::new(), &mut out);
        }
        out.sort();
        out.dedup();
        out
    }

    fn visit<'r>(
        &'r self,
        krate: &'r str,
        state: &mut HashMap<&'r str, Visit>,
        path: &mut Vec<&'r str>,
        out: &mut Vec<String>,
    ) {
        match state.get(krate) {
            Some(Visit::Done) => return,
            Some(Visit::OnPath) => {
                let at = path
                    .iter()
                    .position(|&seen| seen == krate)
                    .expect("a crate on the path is in it");
                let mut cycle = path[at..].to_vec();
                let first = (0..cycle.len())
                    .min_by_key(|&i| cycle[i])
                    .expect("a cycle has a crate");
                cycle.rotate_left(first);
                cycle.push(cycle[0]);
                out.push(format!(
                    "the allowed edges form a cycle: {}",
                    cycle.join(" -> ")
                ));
                return;
            }
            None => {}
        }
        let Some(rule) = self.rules.iter().find(|rule| rule.krate == krate) else {
            return;
        };
        state.insert(krate, Visit::OnPath);
        path.push(krate);
        for target in self
            .rules
            .iter()
            .filter(|target| target.krate != krate && allows(rule.deps, target.krate))
        {
            self.visit(target.krate, state, path, out);
        }
        path.pop();
        state.insert(krate, Visit::Done);
    }
}

/// Where the cycle search stands on a crate: on the current path, or finished.
#[derive(Clone, Copy)]
enum Visit {
    OnPath,
    Done,
}

/// Whether `name` is in `list`, where an entry ending in `*` is a prefix.
fn allows(list: &[&str], name: &str) -> bool {
    list.iter().any(|entry| match entry.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *entry,
    })
}
