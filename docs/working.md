# Working on a server in this family

The process every language server built on lspkit is worked on by: phrost (PHP), orthra (spell
checking), and whatever comes next. Each server's `CLAUDE.md` imports this file and keeps only what
is about its own language: its boundaries, its budgets, its model-tier examples, its gotchas. A rule
that holds for every server lives here once, because a copy in each server is a copy that drifts.
Every line here was paid for in one of them, and says so where the number matters.

## Agents: parallel, and by the risk the work creates

Default to **delegating to a team of subagents** rather than doing everything in one context, under
three constraints:

- **Disjoint write sets.** Split by crate or by feature directory. Two agents editing the same crate
  is a merge conflict you will debug for an hour.
- **Worktree isolation is not total.** `git stash`, `/tmp` and `UPDATE_EXPECT=1` all leak through it
  (the hazards below). Tell an agent about them when you dispatch it, because it cannot discover
  them without damage.
- **A worktree when the write sets can't be disjoint.** `isolation: "worktree"` plus
  `run_in_background`, then extract `git -C <worktree> diff HEAD -- crates/`, apply it on the
  branch, verify, commit, and `git worktree remove`. Keep `.claude/` in `.gitignore`, or worktrees
  get committed as gitlinks.

**Pick the model by the risk the work creates, not by how big it looks.** The cheapest one reads:
analysis sweeps, code search, summaries, doc edits, fixture conversion, mechanical moves and
renames, `UPDATE_EXPECT` acceptance passes. The middle one builds bounded things: a feature over
projections that exist, handler plumbing, tests, single-module refactors. The most capable one takes
anything that creates a long-lived boundary or touches core semantics -- the crate graph, a foreign
engine's integration, the concurrency model, cache invalidation, **every perf and memory trade-off**
-- and cross-crate debugging. Never a cheap model for a decision that creates a boundary; never an
expensive one to read files. Each server names its own examples.

Recipes that work in every server:

- **Understand first.** Parallel cheap readers, one per subsystem, each returning a structured map.
  Synthesize, _then_ edit: that is how the right layer gets changed once instead of the wrong layer
  three times.
- **Mechanical bulk work goes wide.** Converting fixtures, porting renderers, doc sweeps.
- **One phase per agent.** Roadmap phases are disjoint by design; never run two at once.
- **Verify adversarially.** For a perf, quietness or boundary claim, spawn an independent verifier
  prompted to **refute** it -- "prove this allocation is on the hot path". One agent's "looks fine"
  is not evidence.

**Verify every agent's output against a real build.** A green summary is a claim, and
rust-analyzer's diagnostics lie mid-workflow from a stale cache: `cargo check` and `cargo test` are
the only truth.

**Another session may own a repository.** When a second Claude session works in a server, one
session writes each repository at a time: say so before touching it, never edit the other's tree,
and hand over a revision and an exact change list instead. lspkit itself is written by one session
at a time, for the same reason.

## Hazards that leak through every isolation

Each one cost a session, and none of them is discoverable before it bites.

- **Give each agent its own patch path, never a shared `/tmp/mine.patch`.** Four agents told to take
  a baseline with `git diff > /tmp/mine.patch` and `git apply -R` shared the file; one applied
  another's diff and spent the session believing its tree was shared. The tell is asymmetry: tracked
  edits identical in both trees while the new _files_ are not, since `git diff` carries no untracked
  file. Name the patch after the agent.
- **`git stash` is shared by every worktree.** `refs/stash` lives in the common git dir, so a stash
  pushed from an isolated worktree lands on the stack every other agent uses, and `git stash pop`
  takes whatever is on top. For a before/after baseline, use `git diff > <agent>.patch` with
  `git apply -R` / `git apply`, never stash.
- **`UPDATE_EXPECT=1` under `cargo nextest` corrupts a snapshot file when several of its tests need
  accepting.** nextest runs a process per test; each reads the file fresh and writes back only its
  own patch. A real expectation was replaced by `expect![""]`. To accept several, run
  `UPDATE_EXPECT=1 cargo test -p <crate> --lib <filter>`: threads in one process, which expect-test
  synchronizes. And accept with `--no-fail-fast`, or every next run surfaces one more.
- **`UPDATE_EXPECT=1` inside a worktree writes into the MAIN checkout.** expect-test resolves the
  file against the outermost ancestor holding a `Cargo.toml`. Pass
  `CARGO_WORKSPACE_DIR=<worktree path>/` (trailing slash) for any test run in a worktree, and
  `git status` the main checkout after an agent reports.
- **One cargo command at a time.** A background `cargo` beside a foreground one races on the build
  lock and leaves `target/` missing `.rmeta` files. Run `cargo clean` when `target/` has grown and
  builds take minutes: a slow build is what turns a small task into a long one.
- **Pin every engine that decides what a file contains with `=`, and commit `Cargo.lock`.** A server
  is a binary. phrost's predecessor floated its parser on an ignored lockfile and a fresh checkout
  broke typed class constants. `lspkit_arch::baseline` checks the lockfile.

## The gate, and committing

- **Never commit red.** `just pr-ready` -- `cargo fmt --check`, `cargo clippy -D warnings`, the
  tests, the arch checks -- passes first.
- **The gate's exit status has to reach the commit.** Five times in phrost a red commit went in
  because it did not: a `;` after the gate, and four pipelines (`| tail -1`, `| head`, a piped
  clippy) whose status was the last command's. Put the commit _inside_ the `&&` chain, run every
  gate command un-piped (`... > log 2>&1 && ...`) and read the log afterwards.
- One commit per coherent step, with a present-tense subject that describes the behavior change, not
  the files touched. Opening a PR is the human's call.

## Measuring

- **Every server has a `perf` command and a budgets file**, the source of truth for what a request
  may cost. `lspkit-perf` takes the percentile (nearest rank) and gives the verdict, and the command
  exits non-zero when a budget is exceeded, so a claim about latency is checked rather than argued.
- **A change that touches a hot path, the index, parsing or the runtime is incomplete without a
  measurement**, before and after, with the deltas reported. "It compiles" is not "it's fast".
- **Every server runs `incremental == from scratch` under random edits** (`lspkit-fuzz`) for each
  layer that keeps state across edits, with a palette of its own and a script of the edits it knows
  to fear. Same-length edits are the ones that find a stale cache: orthra's mutants survived every
  random edit that changed a length and fell to `go(` becoming `// `. A cache that engages only on
  big inputs needs its threshold lowered in tests, or no fixture reaches it.
- **Every server's binary allocates through `lspkit_heap::Heap`**, and a server with state that
  should come and go -- a closed buffer, an unloaded model -- has a test that the live bytes
  (`lspkit_heap::stats()`) are back where they were after it went. A leak is a logic bug the
  allocator cannot fix; counting is what makes it a failing test instead of a slow climb. Take the
  crate's `override` feature too, so what a parser written in C freed leaves the process, and read
  memory as the kernel charges it: `lspkit_perf::rss` reports the footprint on macOS, where `ps`'s
  RSS keeps counting pages the allocator has already given back and so never falls.
- **Every server has `check --ratchet`** (`lspkit-ratchet`): its findings counted by code against a
  baseline that only goes down. It is what makes a strict check liveable, and a code nothing has
  agreed to fails it, so a new check is a decision somebody reads in a diff.
- **Measure on an idle machine, one sweep at a time.** A concurrent `cargo build` inflates every p95
  and reads exactly like a regression, and two servers measured at once cross-kill each other's
  runs.
- **Take the number from a build started after the last edit, in the same command.** A ratchet was
  committed on a number a build from before the final fix had produced; the hour after it was spent
  diffing against fiction.
- **Undo a temporary sabotage by editing it back by hand.** `git checkout <file>` throws away the
  work beside it, and restoring a backup by `mv`/`cp` keeps its old mtime, so cargo reuses the
  sabotaged build and the "after" is really the "during". The tell is a measurement better than the
  unmodified baseline, which no A/B can produce.
- **`rg -r` is _replace_, not recursive.** rg recurses by default; any `-r` in a search is the bug.
- **An in-process harness cannot see what is about the process.** The flow tests drive the service
  directly, so a server that never exits, an exit status, or a read waiting on real stdin are
  invisible to them. lspkit's own loop shipped with a reader that read past `exit`: every in-process
  test passed, and the process outlived the session until the client closed the pipe -- a blocking
  stdin read is a thread a tokio runtime waits for. orthra found it by running its binary. The
  template's `crates/cli/tests/exit.rs` runs the real binary for exactly this; anything about
  startup, exit or the pipes gets a test of that kind, not a flow test.

## Code

- `#![forbid(unsafe_code)]` through the workspace's lints, which every crate inherits
  (`lspkit_arch::baseline` checks both). The exception is `lspkit-heap`, in lspkit: a global
  allocator and the few foreign calls that map pages and hand freed memory back to the operating
  system.
- `panic = "unwind"` in both profiles. A bug in one handler answers that one request with an error
  (`lspkit_runtime::panics`); with `abort`, it takes the editor's server down. Checked by the
  baseline too.
- `pub(crate)` by default; `pub` only for real public API.
- Files under ~600 lines; split by responsibility before ~1 000.
- One feature per directory. No catch-all `utils`: a shared grab-bag is edited by everyone and owned
  by no one.
- Tests in a `tests.rs` file or a `tests/` directory beside the code, never an inline
  `mod tests { ... }`: the reads check tells request-path code from test code by path.
- No compatibility shims or transitional layers unless a gradual migration was asked for.
- Comments explain non-obvious constraints and intent, and keep their measurements. One that says
  "currently" is probably describing an implementation detail that should not be documented.
- Methods in `impl` blocks over free functions; typed DTOs over ad-hoc strings.

## lspkit itself

What every server needs that is not about its language belongs in lspkit, not in one server twice: a
second copy is a second place to fix the same bug. Code arrives there once a server has proven it
and another needs it. A server takes the protocol from `lspkit_runtime::protocol` and never names
the framework under it. A change to lspkit reaches a server as a `rev` bump in its own commit, with
that server's gate run: `just try ../<server>` before pushing, `just bump "where ..."` after.

A new server starts from `just new <name> ../<name>` in lspkit: the template already has the crate
boundaries, the arch checks, the gate, the perf command and this process wired in, so the work
starts at the language.
