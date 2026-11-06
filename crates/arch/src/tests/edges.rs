use expect_test::expect;

use crate::{
    edges::{Edges, Foreign, Pattern, Rule},
    workspace::Manifest,
};

const RULES: &[Rule] = &[
    Rule::new("app-cli", &["app-*"]),
    Rule::new("app-engine", &["app-text", "parser-core"]).dev(&["app-harness"]),
    Rule::new("app-text", &[]),
    Rule::new("app-harness", &["app-engine", "app-text"]),
    Rule::new("app-future", &["app-text"]),
];

const FOREIGN: &[Foreign] = &[Foreign {
    pattern: Pattern::Prefix("tree-sitter"),
    owners: &["app-engine"],
    why: "one crate to re-pin",
}];

const EDGES: Edges<'static> = Edges {
    policed: &["app-", "parser-"],
    rules: RULES,
    dev_everywhere: &["app-testkit"],
    foreign: FOREIGN,
};

#[test]
fn a_manifest_that_breaks_the_rules_in_every_way_it_can() {
    let manifests = [
        // Upward, a policed foreign prefix, a build dependency, and a dev-only crate as a normal
        // one.
        r#"
            [package]
            name = "app-text"
            [dependencies]
            app-engine.workspace = true
            parser-core = "1"
            [build-dependencies]
            app-cli.workspace = true
        "#,
        // A foreign engine renamed and behind a target table: neither is a way around the rule.
        r#"
            [package]
            name = "app-harness"
            [target.'cfg(unix)'.dependencies]
            ts = { package = "tree-sitter-rust", version = "0.24" }
            [dependencies]
            app-harness-extra = { path = "x" }
        "#,
        // Allowed: a prefix rule, a rule's own dev list, and the everywhere list.
        r#"
            [package]
            name = "app-cli"
            [dependencies]
            app-engine.workspace = true
            app-text.workspace = true
        "#,
        r#"
            [package]
            name = "app-engine"
            [dependencies]
            app-text.workspace = true
            tree-sitter = "0.25"
            [dev-dependencies]
            app-harness.workspace = true
            app-testkit.workspace = true
        "#,
        // A crate nobody wrote a rule for.
        r#"
            [package]
            name = "app-new"
        "#,
    ]
    .map(Manifest::parse);
    expect![[r#"
        app-text -> app-engine (Normal) is not an allowed edge. If it is genuinely right, add it to the rules with the reason in the commit; otherwise route the fact through a layer that is already allowed
        app-text -> parser-core (Normal) is not an allowed edge. If it is genuinely right, add it to the rules with the reason in the commit; otherwise route the fact through a layer that is already allowed
        app-text -> app-cli (Build) is not an allowed edge. If it is genuinely right, add it to the rules with the reason in the commit; otherwise route the fact through a layer that is already allowed
        app-harness -> app-harness-extra (Normal) is not an allowed edge. If it is genuinely right, add it to the rules with the reason in the commit; otherwise route the fact through a layer that is already allowed
        app-harness names tree-sitter-rust, which only app-engine may: one crate to re-pin
        app-new has no rule: adding a crate means declaring what it may reach
    "#]]
    .assert_eq(&(EDGES.violations(&manifests).join("\n") + "\n"));

    expect![[r#"
        the rules name app-text, which is not a workspace member
        the rules name app-future, which is not a workspace member
    "#]]
    .assert_eq(&(EDGES.stale_rules(&manifests[1..]).join("\n") + "\n"));
}

#[test]
fn a_cycle_is_found_through_a_prefix_rule_too() {
    let rules = [
        Rule::new("app-a", &["app-b"]),
        Rule::new("app-b", &["app-c*"]),
        Rule::new("app-c", &["app-a"]),
        Rule::new("app-d", &["app-a"]),
    ];
    let edges = Edges {
        rules: &rules,
        ..EDGES
    };
    expect![[r#"
        the allowed edges form a cycle: app-a -> app-b -> app-c -> app-a
    "#]]
    .assert_eq(&(edges.cycles().join("\n") + "\n"));
    assert!(EDGES.cycles().is_empty(), "the real rules are a DAG");
}

#[test]
fn the_kit_rules_hold_in_a_workspace_that_never_listed_them() {
    let manifests = [
        // A server's own crate naming the framework, directly and under another name.
        r#"
            [package]
            name = "app-engine"
            [dependencies]
            tower-lsp-server = "0.23"
            [dev-dependencies]
            types = { package = "lsp-types", version = "0.97" }
        "#,
        // lspkit's own crates are the owners.
        r#"
            [package]
            name = "lspkit-runtime"
            [dependencies]
            tower-lsp-server = "0.23"
        "#,
    ]
    .map(Manifest::parse);
    let edges = Edges {
        rules: &[
            Rule::new("app-engine", &[]),
            Rule::new("lspkit-runtime", &[]),
        ],
        foreign: &[],
        ..EDGES
    };
    expect![[r#"
        app-engine names tower-lsp-server, which only lspkit-* may: a server takes the protocol from `lspkit_runtime::protocol`, so the framework under it is replaced in lspkit once, not in every server
        app-engine names lsp-types, which only lspkit-* may: a server takes the protocol from `lspkit_runtime::protocol`, so the framework under it is replaced in lspkit once, not in every server
    "#]]
    .assert_eq(&(edges.violations(&manifests).join("\n") + "\n"));
}

#[test]
fn a_dense_dag_has_no_cycles_and_is_not_walked_path_by_path() {
    // Every crate may take every later one: 2^39 paths from the first, 780 edges.
    let names: Vec<&'static str> = (0..40)
        .map(|i| &*Box::leak(format!("c{i:02}").into_boxed_str()))
        .collect();
    let rules: Vec<Rule> = (0..40)
        .map(|i| {
            let deps: &'static [&'static str] =
                Box::leak(names[i + 1..].to_vec().into_boxed_slice());
            Rule::new(names[i], deps)
        })
        .collect();
    let edges = Edges {
        rules: &rules,
        ..EDGES
    };
    assert!(edges.cycles().is_empty());
}

#[test]
fn a_workspace_inherited_dependency_is_checked_under_its_package_name() {
    let workspace_deps: toml::Table = r#"
        foo = { package = "ls-types", version = "=0.1" }
        plain = "1"
    "#
    .parse()
    .expect("a table");
    let manifest = Manifest::parse_with(
        r#"
            [package]
            name = "app-engine"
            [dependencies]
            foo.workspace = true
            plain = { workspace = true }
        "#,
        &workspace_deps,
    );
    let edges = Edges {
        rules: &[Rule::new("app-engine", &[])],
        foreign: &[],
        ..EDGES
    };
    expect![[r#"
        app-engine names ls-types, which only lspkit-* may: a server takes the protocol from `lspkit_runtime::protocol`, so the framework under it is replaced in lspkit once, not in every server
    "#]]
    .assert_eq(&(edges.violations([&manifest]).join("\n") + "\n"));
}
