//! A global allocator for language servers that returns large freed blocks to the operating
//! system and counts live memory.
//!
//! This is the only crate in lspkit that uses `unsafe`, for the allocator and its foreign calls.
//!
//! A cold bootstrap parses every file into an arena that is freed once the file is lowered, and
//! nothing that size is allocated again. System allocators keep those blocks anyway. Measured in
//! phrost on a 30 965-file checkout, `vmmap` showed 90 MB of empty but dirty large regions held by
//! macOS malloc after bootstrap, a third of the process, for the life of the server. With
//! `MallocLargeCache=0` in the environment the process took 166 MB instead of 270, but a process
//! cannot set that for itself. Using mimalloc as the global allocator and reusing one arena across
//! files both measured worse. On macOS 26, `malloc_zone_pressure_relief` returns 0 bytes against
//! xzone and the 92 MB stays.
//!
//! The small blocks are the other two thirds, and the system allocator keeps those too once they
//! are freed: a parser written in C frees its trees into it, and on macOS nothing asks it to give
//! them back. The `override` feature makes mimalloc the process's `malloc`, so [`release_freed`]
//! collects them. What that changed, measured with the kernel's own accounting (`footprint`,
//! which `lspkit_perf::rss` reads on macOS; `ps`'s RSS still counts pages the kernel has been
//! told it may take, so it never falls):
//!
//! | After bootstrap, then `release_freed`      | `Heap` over macOS malloc | `Heap` with `override` |
//! | ------------------------------------------ | -----------------------: | ---------------------: |
//! | phrost, 29 094 files: footprint            |                   175 MB |                 160 MB |
//! | phrost: peak footprint                     |                   204 MB |                 199 MB |
//! | phrost: indexing time                      |                   1.26 s |                 1.17 s |
//! | orthra, helix checkout: footprint          |                    28 MB |                  14 MB |
//! | orthra, phrost checkout: footprint         |                    19 MB |                  15 MB |
//! | synthetic bootstrap, everything then freed |                    28 MB |                   2 MB |
//! | synthetic, allocation-heavy work           |                      1 x |       1.7 to 2.5 x faster |
//!
//! In the same runs `ps`'s RSS rose from 186 MB to 223 MB on phrost, which is what made mimalloc
//! look worse when it was first tried. Both servers' test suites pass with it. It is a macOS
//! remedy: on Linux, glibc gives a C library's freed blocks back by itself, and with `override`
//! orthra measured 4.5 MB more RSS per project there, mimalloc's own reserve. A server takes the
//! feature in a `[target.'cfg(target_os = "macos")'.dependencies]` table, as the template does.
//!
//! [`Heap`] handles large blocks itself:
//!
//! - Allocations under [`LARGE`] go to the system allocator unchanged. It is tuned for them.
//! - A block of [`LARGE`] bytes or more is mapped from the operating system. When freed, it is
//!   kept for the next block of its size class, up to [`set_cache_limit`] (32 MiB by default),
//!   and unmapped beyond that. Unmapping returns it to the operating system on macOS, Linux and
//!   Windows alike, whatever the system allocator's policy.
//! - [`stats`] counts live small and large bytes and what is cached. A leak test opens and closes
//!   a file a hundred times, or unloads a model, and asserts the live bytes are back where they
//!   were.
//!
//! Install it in the server's binary:
//!
//! ```ignore
//! #[global_allocator]
//! static HEAP: lspkit_heap::Heap = lspkit_heap::Heap;
//! ```
//!
//! Then call [`release_freed`] after freeing a lot at once, such as after bootstrap or after
//! unloading a model.

mod cache;
mod counts;
mod pages;

use std::alloc::{GlobalAlloc, Layout, System};

pub use self::counts::{Stats, stats};

/// The size, 256 KiB, from which [`Heap`] maps a block from the operating system instead of using
/// the system allocator. Parse arenas are above it; what allocators keep below it is small.
pub const LARGE: usize = cache::SMALLEST;

/// Sets the most bytes of freed large blocks kept for reuse (32 MiB by default). A block freed
/// beyond the limit goes straight back to the operating system. Lowering the limit releases the
/// excess immediately.
pub fn set_cache_limit(bytes: usize) {
    cache::set_limit(bytes);
}

/// The global allocator. Small allocations go to the system allocator, large ones to whole pages
/// with a bounded cache. See the crate documentation.
#[derive(Debug, Clone, Copy, Default)]
pub struct Heap;

impl Heap {
    /// Whether a layout is a large block. This depends only on the layout, because `dealloc` and
    /// `realloc` receive the layout `alloc` did and must route it the same way. Pages are aligned
    /// to at least 4 KiB, so a larger alignment stays with the system allocator, which can honor
    /// it. On a target where pages are not mapped here ([`pages::SUPPORTED`]), nothing is large and
    /// `Heap` is the system allocator with counts.
    #[inline]
    fn large(layout: Layout) -> bool {
        pages::SUPPORTED && layout.size() >= LARGE && layout.align() <= 4096
    }
}

// SAFETY: every block handed out is either the system allocator's, returned to it with the same
// layout, or a whole mapping of `cache::mapped(size)` bytes. A mapping is page-aligned, so it is
// aligned for any layout `large` accepts, and it goes back to the cache with the same size, which
// the layout determines.
unsafe impl GlobalAlloc for Heap {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if Heap::large(layout) {
            let mapped = cache::mapped(layout.size());
            let (ptr, _) = cache::take(mapped);
            if !ptr.is_null() {
                counts::large(mapped as isize);
            }
            return ptr;
        }
        // SAFETY: the caller's contract, passed on.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            counts::small(layout.size() as isize);
        }
        ptr
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if Heap::large(layout) {
            let mapped = cache::mapped(layout.size());
            let (ptr, fresh) = cache::take(mapped);
            if ptr.is_null() {
                return ptr;
            }
            if !fresh {
                // SAFETY: a kept block of `mapped >= size` bytes that now belongs to this
                // allocation.
                unsafe { ptr.write_bytes(0, layout.size()) };
            }
            counts::large(mapped as isize);
            return ptr;
        }
        // SAFETY: the caller's contract, passed on.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            counts::small(layout.size() as isize);
        }
        ptr
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if Heap::large(layout) {
            let mapped = cache::mapped(layout.size());
            counts::large(-(mapped as isize));
            // SAFETY: `alloc` mapped this block for the same layout, so it is a whole block of
            // `mapped` bytes.
            unsafe { cache::give(ptr, mapped) };
            return;
        }
        counts::small(-(layout.size() as isize));
        // SAFETY: the caller's contract, passed on.
        unsafe { System.dealloc(ptr, layout) };
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller guarantees `new_size`, rounded to the alignment, does not overflow.
        let grown = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        match (Heap::large(layout), Heap::large(grown)) {
            (false, false) => {
                // SAFETY: the caller's contract, passed on.
                let moved = unsafe { System.realloc(ptr, layout, new_size) };
                if !moved.is_null() {
                    counts::small(new_size as isize - layout.size() as isize);
                }
                moved
            }
            // The size class already has room for `new_size`, so the block stays put.
            (true, true) if cache::mapped(layout.size()) == cache::mapped(new_size) => ptr,
            _ => {
                // SAFETY: allocate the new layout, copy as many old bytes as fit, then free the
                // old block with its own layout.
                unsafe {
                    let moved = self.alloc(grown);
                    if !moved.is_null() {
                        ptr.copy_to_nonoverlapping(moved, layout.size().min(new_size));
                        self.dealloc(ptr, layout);
                    }
                    moved
                }
            }
        }
    }
}

/// Returns freed but still held memory to the operating system and reports how many bytes it can
/// account for.
///
/// The count is every large block [`Heap`] kept for reuse, plus what the system allocator reports.
/// The system allocator is asked too: glibc's `malloc_trim`, macOS's pressure relief, and
/// mimalloc's collect with the `mimalloc` feature. Only macOS reports a number, and on macOS 26
/// xzone reports 0 and releases nothing; mimalloc releases but reports nothing usable (see
/// `release_system`), so with it the count understates what left. The process's footprint says.
///
/// This walks the allocators' caches and costs milliseconds, so keep it off request paths. Call it
/// after freeing a lot at once, such as after bootstrap or after unloading a model. Other threads
/// can still allocate large blocks meanwhile, because blocks are unmapped after the cache's lock
/// is released.
pub fn release_freed() -> usize {
    cache::empty() + release_system()
}

/// The system allocator's half of [`release_freed`].
///
/// The `mimalloc` feature adds a call instead of replacing the platform's. Features are unified
/// across a build, so another crate can turn it on in a test binary that installs [`Heap`] over
/// glibc, and glibc must still be trimmed then. What mimalloc gives back is not in the count: the
/// figure it reports is what it has committed, and a page it purges with `MADV_FREE_REUSABLE` or
/// `MADV_DONTNEED` stays committed in that figure. In a test freeing 64 MB it fell by 512 KB while
/// the process's footprint fell by the 64 MB (`tests/allocator.rs` measures the footprint for
/// that reason).
fn release_system() -> usize {
    #[cfg(feature = "mimalloc")]
    // SAFETY: `mi_collect` takes a flag and touches only mimalloc's own heaps.
    unsafe {
        libmimalloc_sys::mi_collect(true);
    }
    release_platform()
}

fn release_platform() -> usize {
    #[cfg(target_os = "macos")]
    // SAFETY: a null zone means every zone, and a goal of zero asks for as much as possible. The
    // call takes no pointer into this program's memory and returns a count.
    unsafe {
        malloc_zone_pressure_relief(core::ptr::null_mut(), 0)
    }
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: `malloc_trim` takes a padding size and touches only glibc's own arenas.
    unsafe {
        let _ = malloc_trim(0);
        0
    }
    #[cfg(not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))))]
    0
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    /// From `malloc/malloc.h`: asks a zone (every zone, when null) to return freed pages, and
    /// returns how many bytes it released.
    fn malloc_zone_pressure_relief(zone: *mut core::ffi::c_void, goal: usize) -> usize;
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" {
    /// From `malloc.h`: releases free memory at the top of the heap and, since glibc 2.8, whole
    /// free pages inside it.
    fn malloc_trim(pad: usize) -> i32;
}

// Runs the README's examples as doctests so they stay in sync with the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
