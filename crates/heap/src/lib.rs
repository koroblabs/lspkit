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
//! remedy: on Linux glibc gives a C library's freed blocks back by itself, and mimalloc there
//! measured the same footprint as glibc on phrost (87 MB after bootstrap on a 10 678-file Drupal
//! checkout) and 1.2 MB more per orthra project, so there is nothing to gain. A server takes the
//! feature in a `[target.'cfg(target_os = "macos")'.dependencies]` table, as the template does.
//!
//! With the feature, `Heap` also sets two of mimalloc's options before `main`:
//!
//! - `abandoned_page_purge`, so a thread that exits purges the pages it freed instead of leaving
//!   them committed in the segments its live data still holds. phrost's indexing threads exit
//!   after bootstrap, and the footprint after bootstrap went from 152-156 MB to 137-138, the peak
//!   from 193-196 to 173-174, with the same indexing time; see `MI_OPTION_ABANDONED_PAGE_PURGE`.
//! - `allow_thp` off. On a Linux host with transparent huge pages `always`, mimalloc's 32 MiB
//!   aligned segments are backed with 2 MiB pages, and phrost stood at 177 MB after bootstrap
//!   instead of 87; orthra at 30 MB instead of 26. See `MI_OPTION_ALLOW_THP`.
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

/// mimalloc's `abandoned_page_purge`, by its index in mimalloc v2's `mi_option_e`; the bindings
/// leave it out, and name `eager_commit_delay` (14) and `use_numa_nodes` (16) either side of
/// `purge_delay` (15), which the assertion below holds them to. A thread that exits leaves the
/// segments still holding its live data to mimalloc, and with them the pages it freed: those are
/// purged once their delay (10 ms, extended by every further free) has passed, and for pages freed
/// in the moments before the exit that is never, since nothing in an abandoned segment runs the
/// clock. With the option, the exiting thread purges them itself. In phrost the indexing threads
/// exit after bootstrap and left 15-18 MB this way: footprint after bootstrap 152-156 MB against
/// 137-138 with it, peak 193-196 against 173-174, the same indexing time (macOS 27, 2026-11-09).
/// orthra, whose threads keep nothing, measured the same with and without. `purge_delay` at 1 ms
/// instead of 10 measured noise on phrost and 0.5 MB on orthra, and stays.
#[cfg(feature = "mimalloc")]
const MI_OPTION_ABANDONED_PAGE_PURGE: libmimalloc_sys::mi_option_t = 12;

#[cfg(feature = "mimalloc")]
const _: () = assert!(
    libmimalloc_sys::mi_option_eager_commit_delay == 14
        && libmimalloc_sys::mi_option_use_numa_nodes == 16,
    "mimalloc's option table was renumbered; check MI_OPTION_ABANDONED_PAGE_PURGE"
);

/// mimalloc's `allow_thp`, by its index as above. On Linux it is whether mimalloc lets the kernel
/// back its memory with transparent huge pages; with THP `always` (`madvise` is the common
/// default, `always` is not rare) mimalloc's 32 MiB aligned segments get 2 MiB pages, and a
/// mostly free segment costs its whole page. Measured with mimalloc as the process's `malloc`
/// (Gentoo, kernel 6.18, THP `always`, 2026-11-09): phrost on a 10 678-file Drupal checkout stood
/// at 177 MB after bootstrap against 87 under glibc, and at 87 again with this off; orthra on the
/// helix checkout 30 MB against 25 and 26. On macOS the option changes nothing.
#[cfg(feature = "mimalloc")]
const MI_OPTION_ALLOW_THP: libmimalloc_sys::mi_option_t = 37;

/// Runs before `main`, from the binary's initializer section, so the options are set before any
/// thread exits or any segment is mapped. The process cannot put `MIMALLOC_*` in its own
/// environment in time, and a check on every allocation would be a load on the hot path.
#[cfg(feature = "mimalloc")]
#[used]
#[cfg_attr(
    target_vendor = "apple",
    unsafe(link_section = "__DATA,__mod_init_func")
)]
#[cfg_attr(
    any(target_os = "linux", target_os = "android", target_os = "freebsd"),
    unsafe(link_section = ".init_array")
)]
#[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
static CONFIGURE_MIMALLOC: unsafe extern "C" fn() = configure_mimalloc;

#[cfg(feature = "mimalloc")]
unsafe extern "C" fn configure_mimalloc() {
    // SAFETY: writes two entries of mimalloc's static option table, which is valid before mimalloc
    // has initialized anything else.
    unsafe {
        libmimalloc_sys::mi_option_set_enabled(MI_OPTION_ABANDONED_PAGE_PURGE, true);
        libmimalloc_sys::mi_option_set_enabled(MI_OPTION_ALLOW_THP, false);
    }
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
