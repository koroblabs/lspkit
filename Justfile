# `just --list` shows every recipe. Run `just pr-ready` before asking for review.

# Full test suite.
test:
    cargo nextest run --workspace

# Tests for one crate, optionally filtered by name.
test-one crate filter="":
    cargo nextest run -p {{crate}} {{filter}}

# Accept new snapshot output, for the workspace or one crate. Read the diff before committing it.
# Uses `cargo test`, not nextest: nextest runs one process per test, and tests accepting into the
# same file overwrite each other (`docs/working.md`, "Hazards").
accept crate="":
    UPDATE_EXPECT=1 cargo test {{ if crate == "" { "--workspace" } else { "-p " + crate } }}

[doc("The boundaries: allowed edges, foreign engines, filesystem reads on the request path.")]
arch:
    cargo test -p lspkit-arch

# Prettier formats the Markdown, at the same pinned version here and in CI. Hand-wrapped Markdown
# drifts with every edit.
prettier := "npx --yes prettier@3.8.0"

[doc("Formatting and clippy, the way CI runs them.")]
lint:
    cargo fmt --all --check
    {{prettier}} --check "**/*.md"
    cargo clippy --workspace --all-targets --all-features -- -D warnings

[doc("Format every crate, and the Markdown.")]
fmt:
    cargo fmt --all
    {{prettier}} --write "**/*.md"

[doc("Licenses, bans, sources and advisories.")]
deny:
    cargo deny check

[doc("The API docs as docs.rs builds them, warnings denied, and every doc example run.")]
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
    cargo test --workspace --doc --all-features

# Targets CI does not run on, for the heap's Windows calls and its system-allocator fallback where
# pages are not mapped (FreeBSD stands in for all such targets). They are linted, not run.
cross_targets := "x86_64-pc-windows-msvc x86_64-unknown-freebsd"

[doc("The heap's other platforms, linted without being run.")]
cross:
    rustup target add {{cross_targets}}
    for target in {{cross_targets}}; do cargo clippy -p lspkit-heap --all-targets --target "$target" -- -D warnings || exit 1; done

[doc("The gate before asking for review.")]
pr-ready: lint test doc deny cross template

[doc("A server's lints and tests against this checkout, with nothing pushed: `just try ../phrost`.")]
try server:
    tools/consumers.sh try {{server}}

[doc("Every server moved to the pushed HEAD, its gate run, and committed if green: `just bump \"where X\"`.")]
bump why *servers:
    tools/consumers.sh bump "{{why}}" {{servers}}

[doc("The template generated against this checkout and built: lints, tests, its arch checks.")]
template:
    tools/template.sh check

[doc("A new server from the template, pinned to the pushed HEAD: `just new orthra2 ../orthra2`.")]
new name dir:
    tools/template.sh new {{name}} {{dir}}
