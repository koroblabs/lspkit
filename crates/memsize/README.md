# lspkit-memsize

Reports the heap memory a structure owns, by type. `ps` gives a language server's resident total;
this shows which structures hold it.

Implement `MemSize::heap_size` for your types to report what a value owns on the heap (not the value
itself), and compose the results with `flat`, `deep`, `vec`, `vec_flat` and `table`.

## Example

```rust
use lspkit_memsize::{MemSize, deep, vec_flat};

struct Document {
    text: String,
    line_starts: Vec<u32>,
}

impl MemSize for Document {
    fn heap_size(&self) -> usize {
        self.text.heap_size() + vec_flat(&self.line_starts)
    }
}

let documents = vec![Document {
    text: "fn main() {}\n".to_owned(),
    line_starts: vec![0, 13],
}];
// The slice's buffer, plus what each document owns.
assert!(deep(&documents) > std::mem::size_of::<Document>());
```

## Notes

- The owner of a shared allocation counts its bytes, and every other holder counts only the pointer.
  Otherwise a name interned once and shared by hundreds of thousands of holders would be counted
  several times over. Use `flat` for elements that own nothing beyond themselves and `deep` for
  elements that do.
- `vec` and `vec_flat` count a `Vec`'s capacity, because a vector grown by pushing can hold nearly
  twice its length, and the report explains resident bytes.
- `table` estimates a hash table from its length (power-of-two buckets, one control byte each, load
  factor under 7/8). It is accurate to within one doubling and always errs in the same direction, so
  a change in the report still reflects a change in the program.

Part of [lspkit](../../README.md), a toolkit for language servers.

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT license](../../LICENSE-MIT) at your option.
