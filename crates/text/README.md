# lspkit-text

Line starts and columns for one text, with columns in bytes, UTF-16 code units or characters. LSP
counts UTF-16 code units unless the client offers UTF-8 at `initialize`. The two agree on ASCII, so
a wrong conversion passes tests written in English and misplaces the cursor in a file with a
Ukrainian comment. This crate does the conversion in one place, so code that only names a position
never sees an encoding.

## Example

```rust
use lspkit_text::{Encoding, LineCol, LineIndex, TextSize};

let text = "x = 1\ny = \"Привет\" + z\n";
let index = LineIndex::new(text);
let z = TextSize::try_from(text.find('z').unwrap()).unwrap();

// Inside the server, columns are bytes: each Cyrillic letter is two.
assert_eq!(index.line_col(z), LineCol { line: 1, col: 21 });
// A UTF-16 client counts each as one unit.
let utf16 = index.line_col_in(text, z, Encoding::Utf16).unwrap();
assert_eq!(utf16, LineCol { line: 1, col: 15 });
// And back to the offset.
assert_eq!(index.offset_in(text, utf16, Encoding::Utf16), Some(z));
```

## Notes

- A line ends at `\n`, `\r\n` or a lone `\r`, the three terminators LSP counts.
- A `LineIndex` holds only offsets, so it is cheap to keep one beside every text. Queries that count
  characters take the text it was built from.
- A position that names nothing (past the end, inside a character, or between the `\r` and `\n` of a
  terminator) gives `None`. A column past the end of its line means the end of the line, as LSP
  specifies.
- With the `lsp` feature, `lspkit_text::lsp` converts protocol `Position`s and `Range`s, and
  `negotiate` picks the encoding at `initialize`: UTF-8 if the client offers it, else UTF-16.
  `apply` applies a client's incremental edits. `apply_each` also reports each change in bytes just
  before applying it, so a syntax tree can be edited in step with the text.

Part of [lspkit](../../README.md), a toolkit for language servers.

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT license](../../LICENSE-MIT) at your option.
