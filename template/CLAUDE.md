# CLAUDE.md — kitserver

kitserver is a language server built on lspkit. The rule everything here is judged by is the
family's: it must be **fast and resource-cheap**. A change that adds latency to a request, or makes
memory grow with the size of a project, is wrong by default however correct the feature is.

This repository holds only what is about its language. Everything else -- the protocol and the loop
under it, panic containment, progress, publishing, the fixture format, the perf arithmetic, the arch
checks -- comes from lspkit at a pinned revision, and a fix there reaches this server as a `rev`
bump.

## How this repository is worked on

The process every server in the family shares, imported whole. It expects `../lspkit` beside this
checkout:

@../lspkit/docs/working.md

## 1. Boundaries are the dependency graph

`crates/tools/arch/src/dependency_rules.rs` holds the allowed edges, and a test fails on any other.
Adding an edge is a deliberate edit with its reason in the commit.

- `kitserver-check` knows the language and no protocol type: it takes text and gives findings in
  bytes. Everything the server answers is computed here.
- `kitserver-lsp` translates: documents in, findings out, positions in the encoding the client
  chose. No language semantics.
- `kitserver-cli` is the command line and the instruments. Never on a request path.
- A foreign engine -- a parser, a dictionary, a model runtime -- gets exactly one crate that may
  name it, as a `Foreign` row, so swapping or upgrading it is a one-crate job. lspkit already holds
  the protocol to that rule (`edges::KIT`).

## 2. Zero fallbacks

The index either has an answer or returns a deterministic nothing. A fallback in a language server
is a read: something asks, gets nothing, and goes to disk to make up for it. `no_fallbacks.rs` lists
every file allowed to read, with what it is for, and a new read fails the test until somebody writes
that down.

## 3. Budgets

`crates/cli/src/perf/budget.rs` is the source of truth for what each request may cost;
`just perf <project>` measures them on real files and exits non-zero when one is over. Every number
there was set before anything was measured -- lower it once a measurement shows the headroom,
because the headroom is what makes a regression visible.

## 4. Definition of done

1. `just pr-ready` green.
2. New behavior has a whole-result snapshot at the layer that owns it, not only end to end.
3. A change on a request path is measured, before and after, and the deltas reported.

## 5. Gotchas

Each bug paid for once, written down in the shape the family uses: what looked true, what was true,
the tell, and what generalizes. When one turns out to hold for every server, it moves to lspkit's
`docs/working.md`.
