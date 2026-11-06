# lspkit-ratchet

A project's findings counted by code, against a baseline whose counts may fall but not rise. A
strict check is only usable if its existing findings can be accepted and new ones cannot slip in;
the baseline records the accepted count per code. It reads the baseline files of both phrost's and
orthra's `check --ratchet`.

## Example

```rust
use lspkit_ratchet::{Baseline, Counts, Outcome};

// The counts the project accepted, as the baseline file records them.
let baseline = Baseline::parse("files = 120\n\n[counts]\nunknown-word = 10\nmixed-script = 2\n")
    .expect("a baseline");

// What this run found.
let mut found = Counts::new();
found.insert("unknown-word".to_owned(), 7);
found.insert("mixed-script".to_owned(), 2);
found.insert("brand-new-check".to_owned(), 1);

let comparison = baseline.compare(&found);
// A code the baseline does not name counts as a rise from zero.
assert_eq!(comparison.outcome(), Outcome::Rose);
print!("{}", comparison.report());

// Written back with `baseline.store(path, "mytool check --accept")` once it is accepted.
```

## Notes

- Counts are compared per code. Ten fixed findings of one code and ten new ones of another leave the
  total unchanged.
- A code the baseline does not name counts as a rise from zero, so enabling a new check shows up as
  a diff someone reviews.
- A fall is reported, and the server decides what happens to the file (`Comparison::outcome` returns
  `Held`, `Fell` or `Rose`). One server writes the lower count back at once, so a later rise to the
  old number cannot pass. The other waits for `--accept`, because its baselines live in other
  people's checkouts.
- `Baseline::parse` reads `files = N` with a `[counts]` table, or the older form with codes and
  counts at the top level. `Baseline::load` returns `Ok(None)` for a missing file.

Part of [lspkit](../../README.md), a toolkit for language servers.

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT license](../../LICENSE-MIT) at your option.
