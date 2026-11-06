//! Live memory, counted on every allocation. Cheap enough to leave on in a server, and what a leak
//! test asserts against.

use core::{
    cell::Cell,
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering},
};

/// Small allocations are counted in shards, one assigned per thread, so eight indexing threads do
/// not contend for one cache line on every allocation.
const SHARDS: usize = 16;

/// One cache line each, at the widest any target has: 128 bytes on Apple silicon, and on x86 the
/// adjacent-line prefetcher pairs 64-byte lines. With 64-byte alignment, shards `2k` and `2k + 1`
/// shared a line pair on x86.
#[repr(align(128))]
struct Shard(AtomicIsize);

static SMALL: [Shard; SHARDS] = [const { Shard(AtomicIsize::new(0)) }; SHARDS];
static NEXT_SHARD: AtomicUsize = AtomicUsize::new(0);
static LARGE: AtomicUsize = AtomicUsize::new(0);
static LARGE_PEAK: AtomicUsize = AtomicUsize::new(0);
static CACHED: AtomicIsize = AtomicIsize::new(0);

std::thread_local! {
    static MINE: Cell<usize> = const { Cell::new(usize::MAX) };
}

fn shard() -> usize {
    // A thread being torn down cannot reach its locals; it counts in the first shard.
    MINE.try_with(|mine| {
        if mine.get() == usize::MAX {
            mine.set(NEXT_SHARD.fetch_add(1, Ordering::Relaxed) % SHARDS);
        }
        mine.get()
    })
    .unwrap_or(0)
}

pub(crate) fn small(delta: isize) {
    SMALL[shard()].0.fetch_add(delta, Ordering::Relaxed);
}

pub(crate) fn large(delta: isize) {
    if delta >= 0 {
        let now = LARGE.fetch_add(delta as usize, Ordering::Relaxed) + delta as usize;
        LARGE_PEAK.fetch_max(now, Ordering::Relaxed);
    } else {
        LARGE.fetch_sub(delta.unsigned_abs(), Ordering::Relaxed);
    }
}

pub(crate) fn cached(delta: isize) {
    CACHED.fetch_add(delta, Ordering::Relaxed);
}

pub(crate) fn cached_bytes() -> usize {
    CACHED.load(Ordering::Relaxed).max(0) as usize
}

/// Memory held through [`crate::Heap`].
///
/// Each field is read separately, so under concurrent allocation they do not describe one instant.
/// A leak test should read them after the work it measures has finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// Requested bytes in allocations held by the system allocator: every block under
    /// [`crate::LARGE`], plus the rare larger one aligned beyond a page.
    pub small: usize,
    /// Bytes in large blocks in use, counted at their mapped size class rather than the requested
    /// size.
    pub large: usize,
    /// The highest value `large` has reached.
    pub large_peak: usize,
    /// Bytes in freed large blocks kept for reuse. They are resident but not in use.
    pub cached: usize,
}

impl Stats {
    /// Bytes in use, small plus large. Large blocks count at their size class.
    #[must_use]
    pub fn live(&self) -> usize {
        self.small + self.large
    }
}

/// Returns the current counts. Only allocations through [`crate::Heap`] are counted, so a program
/// that has not installed it reads zeros.
pub fn stats() -> Stats {
    let small: isize = SMALL
        .iter()
        .map(|shard| shard.0.load(Ordering::Relaxed))
        .sum();
    Stats {
        small: small.max(0) as usize,
        large: LARGE.load(Ordering::Relaxed),
        large_peak: LARGE_PEAK.load(Ordering::Relaxed),
        cached: cached_bytes(),
    }
}
