# lspkit-fixture

A test workspace written as one string: several files, per-file flags, and markers for the positions
a test asks about. A marker one byte off would make hundreds of snapshots agree on the wrong
position, so the format's edge cases are pinned and every marker must be asked about.

## Example

```rust
use lspkit_fixture::Fixture;

let fixture = Fixture::parse(
    r#"
    //- /composer.json
    {"name": "acme/app"}

    //- /src/Use.php  open
    <?php
    function go(Repo $r) {
        $r->fi@member@nd($0);
    }
    "#,
);

let file = fixture.subject(); // the last file
assert_eq!(file.path(), "/src/Use.php");
assert_eq!(file.flags(), ["open"]); // unread, for the server to interpret
assert!(!file.text().contains('@') && !file.text().contains("$0"));

let markers = fixture.markers();
let cursor = markers.cursor();
let member = markers.position("member");
assert_eq!(cursor.path, "/src/Use.php");
assert!(member.offset < cursor.offset);
markers.assert_all_used(); // panics naming any marker nobody asked about
```

## Format

- `//- /path flags` starts a file. Paths start at the root with a leading slash. Flags belong to the
  server: `File::flags` returns them unread so the server can refuse one it does not know. The only
  flag read here is `raw`, which leaves a file's text as written (`Fixture::parse_with` adds more).
- `$0` is the cursor and `@name@` a named marker. A name is an ASCII identifier, so an e-mail
  address or a lone `@` stays text. Both are cut from the text, and a backslash makes either sigil
  text.
- `Markers::assert_all_used` panics with the name of each marker nothing asked about. It catches a
  misspelled name, and text that only looked like a marker.
- Common indentation is trimmed, trailing blank lines are dropped, and each file ends in exactly one
  newline.
- A fixture is test code, so malformed input panics with a specific message instead of returning an
  error.

Part of [lspkit](../../README.md), a toolkit for language servers.

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT license](../../LICENSE-MIT) at your option.
