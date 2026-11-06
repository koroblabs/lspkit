# lspkit-fuzz

Seeded random edits, and a sweep that checks a server's incremental state against a fresh build
after every edit. Random edits find invalidation bugs that hand-written tests miss. You give the
sweep four functions of your server (build the live state, apply an edit, ask it, build fresh), a
palette of what edits paste in, and some seeds. On a difference it panics with the seed, the edit
and the text, shrunk to the edits that still fail.

## Example

```rust
use std::collections::BTreeMap;
use lspkit_fuzz::{Edit, Files, Script, Subject, Sweep};

// A server's incremental state: line counts per file, updated edit by edit.
let mut subject = Subject {
    open: |files: &Files| -> BTreeMap<String, usize> {
        files.iter().map(|(path, text)| (path.clone(), text.lines().count())).collect()
    },
    edited: |live: &mut BTreeMap<String, usize>, path: &str, _: &Edit, now: &str| {
        live.insert(path.to_owned(), now.lines().count());
    },
    answer: |live: &mut BTreeMap<String, usize>| live.values().sum::<usize>(),
    scratch: |files: &Files| files.values().map(|text| text.lines().count()).sum::<usize>(),
};

let mut files = Files::new();
files.insert("a.txt".to_owned(), "one\ntwo\n".to_owned());

// Random edits, same-length ones included.
Sweep { seeds: &[1, 2, 3], edits: 50, palette: &["\n", "x", "// "], span: 8 }
    .run(&files, &mut subject);

// A risky edit, named by what it changes.
let script = Script::new(&files).replace("a.txt", "two", "// ");
subject.script(&files, &script);
```

## Notes

- Same-length edits are made on purpose. In orthra, three mutants broke incremental state while
  every random fixture passed: an edit that changes the length shifts every range after it, so a
  stale cache entry no longer matches. Each was caught by a same-length edit (`go(` becoming `// `).
  `Edits` makes those, and a `Script` adds the edits a test knows are risky.
- The sweep compares after every edit, so the failing edit is the one reported.
- Runs are deterministic. The generator is a small built-in xorshift, every failure names the seed,
  the edit and the text, and the failing edits are shrunk to the fewest that still fail.
- Edits fall on character boundaries, since no editor sends one that splits a UTF-8 sequence.
- Small fixtures never reach a cache that engages only on big inputs. Lower its threshold in test
  builds (orthra cuts its query at 4 KB in release and 16 bytes in tests).

Part of [lspkit](../../README.md), a toolkit for language servers.

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT license](../../LICENSE-MIT) at your option.
