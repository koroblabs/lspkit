# lspkit-heap

A global allocator for language servers. Small allocations go to the system allocator unchanged.
Blocks of 256 KiB or more are mapped from the operating system, and freed ones are kept in a bounded
cache for reuse, so a freed parse buffer leaves the process on macOS, Linux and Windows. It also
counts live memory, so a leak can fail a test. Used by phrost and orthra.

This is the only crate in lspkit that uses `unsafe`.

## Example

Install it in the server's binary:

```rust,ignore
#[global_allocator]
static HEAP: lspkit_heap::Heap = lspkit_heap::Heap;
```

To catch a leak, do the work and assert the live bytes are back where they were:

```rust
let before = lspkit_heap::stats().live();
{
    // Open and close a file a hundred times, unload a model, ...
    let buffer = vec![0u8; 1 << 20];
    drop(buffer);
}
assert_eq!(lspkit_heap::stats().live(), before);

// After bootstrap, or after unloading a model, return what was just freed.
let returned = lspkit_heap::release_freed();
println!("{returned} bytes returned to the operating system");
```

`stats()` counts only what went through `Heap`, so it reads zeros in a program that has not
installed it, such as this example.

## Notes

- Measured in phrost on a 30 965-file checkout, macOS malloc held 90 MB of empty but dirty large
  regions after bootstrap, a third of the process, for the life of the server. Using mimalloc as the
  global allocator and reusing an arena across files both measured worse. On macOS 26,
  `malloc_zone_pressure_relief` returns 0 bytes.
- Freed large blocks (`LARGE` and up) are kept for reuse up to 32 MiB, or the limit passed to
  `set_cache_limit`, and unmapped beyond it.
- `stats()` reports live bytes (small and large), the large peak, and cached bytes. Small
  allocations are counted in per-thread shards so indexing threads do not contend for a cache line.
- `release_freed()` costs milliseconds. Call it after freeing a lot at once, never on a request
  path.
- The `mimalloc` feature is for targets such as musl, whose allocator neither returns memory nor can
  be asked to. It assumes the binary installs mimalloc.

Part of [lspkit](../../README.md), a toolkit for language servers.

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT license](../../LICENSE-MIT) at your option.
