# CLAUDE.md — lspkit (what a language server needs that is not about its language)

lspkit is the shared base of the language servers in this family:

- `../phrost` — PHP, the code most of this was extracted from;
- `../orthra` — spell checking.

What lives here is everything a server needs that is not about its language: the protocol and the
loop under it, the rules every server is held to, the instruments, the test formats. What a server
keeps is its language and its numbers -- its own allowed edges, its budgets. A new server should be
able to start from lspkit and write only the part that knows its language.

The rule everything here is judged by is the consumers' rule: **fast and resource-cheap**. A change
that adds latency to a consumer's hot path, or memory to its steady state, is wrong by default,
however tidy it makes this repository.

## 1. What belongs here

1. **Nothing about a language.** No PHP and no spelling, and nothing that knows what a document
   means. A crate whose doc comment needs a consumer's vocabulary to explain itself belongs in that
   consumer.
2. **Every crate has a consumer.** Code arrives here because a consumer used it first and a second
   one needs it. A crate written for a consumer that might exist is a guess at an API, and guesses
   at APIs are what extraction is supposed to avoid.
3. **Moved, then generalized only as far as a consumer needs.** A crate arrives as the code that was
   proven where it came from, with the measurements in its doc comments kept, and changes only where
   a second consumer genuinely differs. `runtime` took the server's name as a parameter and
   `PoisoningWrite` became generic over what it guards. Everything else came across as it was.
4. **No crate here depends on another.** Each is taken on its own, and `lspkit-arch` checks that
   about this workspace (`crates/arch/src/tests/this_workspace.rs`).

## 2. How a change reaches the consumers

Consumers depend on lspkit by git, **pinned to a `rev`**. That is phrost's "pin with `=` and commit
`Cargo.lock`" rule applied across repositories: a consumer's build never changes because this one
moved.

1. Change it here, with tests, and with `just pr-ready` green.
2. Before pushing, run `just try ../phrost` and `just try ../orthra`: each server's lints and tests
   against this checkout, through `--config patch...` on cargo's command line, with the server's
   `Cargo.lock` put back afterwards. Nothing is pushed and no file of the server's changes for good.
3. Push, then `just bump "where <what changed>"`. It moves every lspkit `rev` of each server to the
   pushed HEAD, runs that server's own `just pr-ready`, and commits "Take lspkit <rev>, where ..."
   when the gate is green and nothing but the manifest and lockfile changed. It refuses a server
   with uncommitted work, so another agent's tree is never committed. Run the server's perf sweep
   too if the crate is on its hot path. A change that is green here and red in a server is a red
   change.
4. A breaking API change is fine. This is not a published library. But the bump in each server lands
   with the fix to that server, in that server's own commit -- `bump` leaves a red bump in the
   server's tree for exactly that -- never as a "fix it later". When another agent owns the server's
   tree, hand it the revision and the list of changes rather than editing its tree.

**The protocol and its loop are lspkit's.** A server takes the protocol types, JSON-RPC, the server
trait and the loop from `lspkit_runtime::protocol`; `lspkit-arch`'s `edges::KIT` refuses a server
crate that names `ls-types` or any framework (`tower-lsp-server`, `lsp-server`, `async-lsp`, ...),
without the server listing the rule. That is what let `tower-lsp-server` be replaced by lspkit's own
loop (`crate::server`) with no server's handler rewritten: the trait and the names are the ones the
servers were written against. What the loop promises -- a task per request with no cap,
`$/cancelRequest` that stops one and tells its work (`cancel`), notifications landed before the next
message, the lifecycle as the specification says, a panic answered as an error -- is pinned by
`crates/runtime/src/tests/message_loop.rs`; change it there first. `ls-types` is pinned with `=`,
and `tokio` must be the version the servers use; move either here and in every server at once.

## 3. How every server here is worked on

The process -- agents and model tiers, the hazards that leak through worktrees, the gate, measuring,
code quality -- is one document for every server, and lives here because it is not about a language:

@docs/working.md

Each server's `CLAUDE.md` imports it the same way and keeps only what is its own. A rule that would
hold for every server goes into `docs/working.md`, not into one of them; a second copy is a copy
that drifts.

## 4. Working here

Four things are specific to this repository:

- **A change here is a change in every server.** Before calling one done, run `just try` on each
  (`../phrost`, `../orthra`), and after pushing, `just bump`. A server is not on this machine's path
  by accident: those are the checkouts to use.
- **The provenance stays.** Doc comments that say "measured in phrost" keep saying it. They are the
  reason the code is shaped the way it is, and a reader deciding whether to change it needs to know
  the shape was paid for.
- **Whole-result `expect-test` snapshots, never substrings**, as in the servers.
- **`just bench` is the measurement for a change here.** It times lspkit-text's index, edits and
  column conversions and lspkit-runtime's message loop. Run it on the revision before the change and
  after, on an idle machine, and put both figures in the commit; a server's `perf` sweep then
  confirms what reached it.
- **`template/` is a server, and changes with the API.** A change here that a server would have to
  follow is made in `template/` in the same commit; `just template` (in `pr-ready`, and a CI job)
  generates it against this checkout and builds it. It is never built where it lies -- `kitserver`
  and `LSPKIT_REV` are placeholders -- so work on it through `tools/template.sh scratch <dir>`.
