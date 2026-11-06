//! Tests `Heap` installed as this binary's global allocator, so the test harness allocates
//! through it too.

use std::{
    alloc::{GlobalAlloc, Layout},
    sync::{Mutex, MutexGuard},
    thread,
};

use lspkit_heap::{Heap, LARGE, release_freed, set_cache_limit, stats};

#[global_allocator]
static HEAP: Heap = Heap;

/// The counts cover the whole process, so tests run one at a time. Otherwise a test counting large
/// blocks would see another test's allocations.
static TURN: Mutex<()> = Mutex::new(());

fn turn() -> MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Allocates `layout` through `Heap` directly, fills every byte, reads every byte back, frees it.
fn round_trip(layout: Layout, zeroed: bool) {
    // SAFETY: a non-zero layout; the block is written and read only within its size, then freed
    // with the layout it was allocated with.
    unsafe {
        let ptr = if zeroed {
            HEAP.alloc_zeroed(layout)
        } else {
            HEAP.alloc(layout)
        };
        assert!(!ptr.is_null(), "{layout:?} allocated");
        assert_eq!(ptr as usize % layout.align(), 0, "{layout:?} aligned");
        if zeroed {
            let bytes = std::slice::from_raw_parts(ptr, layout.size());
            assert!(bytes.iter().all(|&b| b == 0), "{layout:?} zeroed");
        }
        let pattern = (layout.size() % 251) as u8;
        ptr.write_bytes(pattern, layout.size());
        let bytes = std::slice::from_raw_parts(ptr, layout.size());
        assert!(
            bytes.iter().all(|&b| b == pattern),
            "{layout:?} kept what was written"
        );
        HEAP.dealloc(ptr, layout);
    }
}

#[test]
fn every_size_and_alignment_is_aligned_writable_and_zeroed_when_asked() {
    let _turn = turn();
    let sizes = [
        1,
        7,
        64,
        4096,
        LARGE - 1,
        LARGE,
        LARGE + 1,
        1 << 20,
        (3 << 20) + 17,
        (65 << 20) + 3,
    ];
    for size in sizes {
        for align in [1, 8, 16, 4096, 8192] {
            let layout = Layout::from_size_align(size, align).expect("a layout");
            round_trip(layout, false);
            round_trip(layout, true);
        }
    }
}

#[test]
fn a_kept_block_handed_out_zeroed_is_zero() {
    let _turn = turn();
    let layout = Layout::from_size_align(1 << 20, 8).expect("a layout");
    // SAFETY: allocated, written within its size, freed with its layout; then the same for the
    // zeroed one that may reuse it.
    unsafe {
        // Empty the cache so the block freed below is the one reused next. A fresh mapping is
        // zeroed anyway, so without reuse the test proves nothing.
        release_freed();
        let dirty = HEAP.alloc(layout);
        dirty.write_bytes(0xAB, layout.size());
        HEAP.dealloc(dirty, layout);
        let clean = HEAP.alloc_zeroed(layout);
        assert_eq!(clean, dirty, "the kept block is the one handed out again");
        let bytes = std::slice::from_raw_parts(clean, layout.size());
        assert!(bytes.iter().all(|&b| b == 0), "a reused block is zeroed");
        HEAP.dealloc(clean, layout);
    }
}

#[test]
fn growing_across_the_line_and_shrinking_back_keeps_every_byte() {
    let _turn = turn();
    let mut grown: Vec<u8> = Vec::new();
    for i in 0..(4 << 20) {
        grown.push((i % 253) as u8);
    }
    assert!(grown.iter().enumerate().all(|(i, &b)| b == (i % 253) as u8));
    grown.truncate(100);
    grown.shrink_to_fit();
    assert!(grown.iter().enumerate().all(|(i, &b)| b == (i % 253) as u8));
}

#[test]
fn a_block_resized_within_its_class_does_not_move() {
    let _turn = turn();
    let layout = Layout::from_size_align(300 * 1024, 8).expect("a layout");
    // SAFETY: allocated, resized within the same 512 KiB class, freed with the resized layout.
    unsafe {
        let ptr = HEAP.alloc(layout);
        ptr.write_bytes(7, layout.size());
        let resized = HEAP.realloc(ptr, layout, 400 * 1024);
        assert_eq!(resized, ptr, "the class already had the room");
        let bytes = std::slice::from_raw_parts(resized, layout.size());
        assert!(bytes.iter().all(|&b| b == 7));
        HEAP.dealloc(
            resized,
            Layout::from_size_align(400 * 1024, 8).expect("a layout"),
        );
    }
}

#[test]
fn many_threads_allocating_large_blocks_never_share_one() {
    let _turn = turn();
    let workers: Vec<_> = (0..8)
        .map(|worker: usize| {
            thread::spawn(move || {
                for round in 0..300 {
                    let size = LARGE + ((worker * 7919 + round * 104_729) % (3 << 20));
                    let fill = (worker * 31 + round) as u8;
                    let block = vec![fill; size];
                    assert!(
                        block.iter().all(|&b| b == fill),
                        "a block shared between threads"
                    );
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().expect("a worker finishes");
    }
}

#[test]
fn large_blocks_are_counted_kept_under_the_bound_and_given_back() {
    let _turn = turn();
    release_freed();
    let before = stats();

    let blocks: Vec<Vec<u8>> = (0..10).map(|i| vec![i as u8; 1 << 20]).collect();
    let holding = stats();
    assert!(
        holding.large >= before.large + (10 << 20),
        "ten 1 MiB blocks are counted as large: {holding:?}"
    );
    assert!(holding.large_peak >= holding.large);

    drop(blocks);
    let freed = stats();
    assert_eq!(
        freed.large, before.large,
        "every large block given back is counted out"
    );
    assert!(freed.cached > 0, "freed blocks are kept for reuse");
    assert!(freed.cached <= 32 << 20, "and kept under the bound");

    assert!(
        release_freed() >= freed.cached,
        "release gives back what was kept"
    );
    assert_eq!(stats().cached, 0);

    // With a bound of zero, a freed block is unmapped immediately.
    let _default = DefaultLimit;
    set_cache_limit(0);
    drop(vec![1u8; 1 << 20]);
    assert_eq!(stats().cached, 0);
}

/// Restores the default cache bound when dropped, so a failing test does not leave later tests
/// running with no cache.
struct DefaultLimit;

impl Drop for DefaultLimit {
    fn drop(&mut self) {
        set_cache_limit(32 << 20);
    }
}

#[test]
fn lowering_the_bound_gives_back_what_is_above_it_at_once() {
    let _turn = turn();
    let _default = DefaultLimit;
    release_freed();
    let blocks: Vec<Vec<u8>> = (0..8).map(|_| vec![1u8; 1 << 20]).collect();
    drop(blocks);
    assert_eq!(stats().cached, 8 << 20, "eight 1 MiB blocks kept");

    set_cache_limit(3 << 20);
    assert_eq!(
        stats().cached,
        3 << 20,
        "and only what fits under the new bound stays"
    );
}

/// The leak test a server writes with these counts: repeating the same work leaves the same
/// memory held afterwards.
#[test]
fn work_done_and_undone_leaves_the_live_bytes_where_they_were() {
    let _turn = turn();
    let work = || {
        let table: Vec<String> = (0..2000).map(|i| format!("name {i}")).collect();
        let arena = vec![0u8; 2 << 20];
        (table.len(), arena.len())
    };
    work();
    let before = stats();
    for _ in 0..100 {
        work();
    }
    let after = stats();
    assert_eq!(after.large, before.large);
    assert!(
        after.small.abs_diff(before.small) < 16 * 1024,
        "small: {} before, {} after",
        before.small,
        after.small
    );
}
