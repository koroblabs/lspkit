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
- The `override` feature makes mimalloc (v2) the process's `malloc`, under `Heap`'s small
  allocations and under every C library in the binary, and `release_freed()` collects what they
  freed. Measured on macOS 27 with the kernel's footprint, which `lspkit_perf::rss` reads there:
  after indexing 29 094 files, phrost went from 175 MB to 160 MB and orthra on the helix checkout
  from 28 MB to 14 MB, and allocation-heavy work ran up to twice as fast. `ps`'s RSS does not show
  it: it counts pages the kernel has been told it may take. Take it for macOS only: on Linux glibc
  gives a C library's freed blocks back by itself, and orthra measured 4.5 to 5 MB more RSS per
  project with mimalloc there, 3.6 MB of it transparent huge pages. The `mimalloc` feature alone
  only asks mimalloc to collect, for a binary that installs it itself.
- With either feature, `Heap` sets mimalloc's `abandoned_page_purge` before `main`, so a thread that
  exits purges the pages it freed instead of leaving them committed in the segments its live data
  still holds. phrost's indexing threads exit after bootstrap: footprint after bootstrap 152-156 MB
  to 137-138, peak 193-196 to 173-174, the same indexing time.

Part of [lspkit](../../README.md), a toolkit for language servers.

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT license](../../LICENSE-MIT) at your option.
