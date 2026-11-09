# Changelog

lspkit is used from git at a pinned revision. Versions are tags. Any 0.x version may change the API;
each entry lists what a dependent has to change.

## Unreleased

### Changed

- heap: with the `mimalloc` or `override` feature, `Heap` sets two of mimalloc's options before
  `main`. `abandoned_page_purge`, so a thread that exits purges the pages it freed instead of
  leaving them committed in the segments its live data still holds: phrost's indexing threads exit
  after bootstrap, and on the 29 094-file checkout its footprint after bootstrap went from 152-156
  MB to 137-138, the peak from 193-196 to 173-174, with the same indexing time and request
  latencies; orthra, whose threads keep nothing, measured the same with and without. And `allow_thp`
  off: on a Linux host with transparent huge pages `always`, mimalloc's 32 MiB aligned segments took
  phrost on a 10 678-file Drupal checkout from 87 MB after bootstrap to 177, and orthra on helix
  from 25 MB to 30; with the option off, 87 and 26. Nothing to change in a server.
- heap, template: `override` is for macOS. On Linux glibc gives a C library's freed blocks back by
  itself, and mimalloc as the process's `malloc` measured the same footprint as glibc on phrost and
  1.2 MB more per orthra project, so there is nothing to gain. `Heap`'s cache of freed large blocks
  costs 11-13 MB on Linux with 20 files open (phrost 142 MB against 131 without `Heap`), the price
  of reusing a parse buffer; a server that would rather not pay it lowers `set_cache_limit`. The
  template takes the feature in its binary crate's
  `[target.'cfg(target_os = "macos")'.dependencies]` table, and a server does the same.

## 0.1.1 (2026-11-09)

No API changed. A server takes the heap crate's `override` feature in its binary crate's features
(`lspkit-heap/override`) to get the memory figures below; a server that installed mimalloc as its
global allocator itself drops that in favor of it. A server's memory budgets on macOS now judge the
footprint, so a budget written against `ps`'s RSS may have room to come down.

### Changed

- heap: a new `override` feature makes mimalloc the process's `malloc`, under `Heap`'s small
  allocations and under every C library in the binary, so `release_freed` collects what they freed
  too. Measured on macOS 27 by the kernel's footprint after bootstrap and `release_freed`: phrost on
  29 094 files went from 175 MB to 160 MB (peak 204 to 199, indexing 1.26 s to 1.17 s); orthra on
  the helix checkout from 28 MB to 14 MB and on the phrost checkout from 19 MB to 15 MB. Both
  servers' test suites pass with it. The `mimalloc` feature now selects mimalloc v2: v3 left 207 MB
  on phrost where v2 left 160 MB. The template turns `override` on. In the same runs `ps`'s RSS rose
  from 186 MB to 223 MB, which is what made mimalloc look worse when it was first tried; see perf.
- perf: `rss::current` reports the physical footprint on macOS, from the system's `footprint` tool,
  instead of `ps`'s resident set, which keeps counting pages an allocator has given back with
  `MADV_FREE_REUSABLE`. In one test the process stood at 150 MB of RSS and 47 MB of footprint, and
  the kernel reported the other 101 MB as reusable. Linux still reads `VmRSS`. A server's memory
  budgets on macOS now judge what the machine actually gives up.
- text: a column is counted from the bytes instead of decoding characters, and an edit's two
  positions are found in one scan that stores nothing instead of an index built through the edit's
  line. On a 2 KB Cyrillic line, offset to UTF-16 position went from 0.71 µs to 0.24 and position to
  offset from 1.15 µs to 0.26 (UTF-32: 0.05); an edit in the middle of a 1 MB file from 226 µs to
  216 and at its end from 408 to 390, with no allocation. Building an index costs the same, and the
  index no longer keeps spare capacity.
- runtime: four allocations fewer per message. The `Content-Length` header is formatted on the
  stack, header lines are read into one reused buffer, a request's method is borrowed as the
  protocol's own `'static` name instead of a `String`, and the panic watcher pins its future instead
  of boxing it. Under a counting allocator, 2 000 small requests answered at once took 3.8 ms
  instead of 4.7, with 30 000 allocations instead of 38 000; a message dominated by its payload
  costs the same.
- `just test`, `just accept` and CI run with `--all-features`, so lspkit-text's `lsp` tests and
  lspkit-heap's `override` test run. The `lsp` tests had run only when asked for by hand.
- `just bench` times the hot paths: lspkit-text's index, edits and column conversions, and
  lspkit-runtime's message loop on the workloads the 0.1.0 entry measured, the scenarios the figures
  above were taken from.

## 0.1.0 (2026-11-06)

First public release.

### Fixed

- runtime: a notification handler that waited on the client (for example `initialized` registering a
  capability) never received the answer, because the loop read nothing while the handler ran.
  Responses are now delivered by the reader as they arrive. They can arrive before notifications the
  client sent earlier; see `Client::apply_edit`.
- runtime: a huge `Content-Length` aborted the process when its buffer was allocated. Messages over
  256 MiB are now skipped, and header lines are limited to 4 KiB. A malformed or missing length ends
  the session with a message on stderr instead of silently losing every later message.
- runtime: fixed a leaked entry for requests answered before they were registered, a session that
  refused a second `initialize` after the first was cancelled, and server requests that waited
  forever when the client never answered or disconnected.
- text: `LineIndex::offset` could wrap a column near `u32::MAX` onto the same line in release
  builds.
- walk: a project without `.git` picked up ignore files from every parent directory, so a
  `~/.gitignore` containing `*` hid the whole project. Overlapping roots no longer list a file
  twice.
- heap: `mmap` was declared with Linux constants on every Unix, which made large allocations fail on
  the BSDs and was undefined behavior on 32-bit glibc. Memory is now mapped only on 64-bit Linux and
  Android, Apple platforms and Windows. Trimming the cache no longer holds its lock while unmapping,
  and the cache limit holds under concurrent frees.
- arch: the cycle check took exponential time on dense rule sets; a dependency renamed in
  `[workspace.dependencies]` escaped the rules; a missing directory passed with no files checked.
- ratchet: codes with non-ASCII characters were written in a form TOML cannot read back. The
  baseline file is now replaced atomically.
- memsize: `table` ignored padding between keys and values.
- fuzz: the shrinker could misplace an edit that followed a dropped deletion.

### Changed

- runtime: request parameters are kept as JSON text instead of a `serde_json::Value`, and outgoing
  messages are serialized straight into the buffer that is written. Measured through
  `transport::serve` (best of five):

  | Workload                               | Before | After | Allocated before | Allocated after |
  | -------------------------------------- | -----: | ----: | ---------------: | --------------: |
  | 2,000 `publishDiagnostics` of 50 items |  92 ms | 31 ms |           419 MB |          148 MB |
  | 200 `didChangeWatchedFiles` of 1,000   |  53 ms | 21 ms |           321 MB |           59 MB |
  | 300 completions of 143 KB              |  24 ms | 22 ms |           230 MB |          165 MB |
  | 1,000 full-sync `didChange` of 50 KB   |  31 ms | 24 ms |           349 MB |          344 MB |

- text: `lsp::apply` indexes only up to the line being edited, and line breaks are found with
  `memchr`. A single edit in a 1 MB file went from 552 µs to between 0.6 µs (near the top) and 188
  µs (at the end). `LineIndex::new` on 1 MB went from 552 µs to 188 µs.
- text: converting a position no longer decodes the line for UTF-8, or for ASCII text in any
  encoding. On a minified 1 MB single-line file, 2,000 conversions went from about 1 s to 0.014 ms
  (UTF-8) and 11.6 ms (UTF-16).
- heap: small allocations on 12 threads take about half the time (76.8 ms to 41.1 ms in the
  benchmark), because each counter shard now has its own 128-byte cache line. With the `mimalloc`
  feature, `release_freed` calls `mi_collect` in addition to the platform's trim.

- template: `just licenses` writes `THIRD-PARTY-LICENSES.md` (with `cargo-about`) listing the
  license of every crate the server's binary links, for release archives. Generated servers get
  their own `LICENSE-*` files, and their crates are `publish = false`.

### API changes

- runtime: `Request::params()` returns `Option<&Params>`. `Params` serializes and displays as the
  JSON text, so `serde_json::to_value(request.params()?)` works as before; `get()` returns the text.
  `Request::into_parts` and `FromParams::from_params` carry `Params`.
- runtime: `ErrorCode` is `#[non_exhaustive]`, adds `ServerNotInitialized`, `UnknownErrorCode`,
  `ServerCancelled` and `RequestFailed`, and compares by numeric code.
- runtime: `RequestStream` and `ResponseSink` are exported from `protocol`. `Bar` and `Ended` are
  `#[must_use]`. Outgoing JSON objects keep their fields in declaration order instead of sorted
  order.
- ratchet: `Baseline::{parse, load, store}` return `lspkit_ratchet::Error`.
- perf: `Verdict::into_result` returns `lspkit_perf::OverBudget`. Both new error types display the
  same text as before and convert into `String`. A function that returned `verdict.into_result()`
  from a `Result<(), String>` needs `Ok(verdict.into_result()?)`.
- walk: the builder methods and `files` are `#[must_use]`.
