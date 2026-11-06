//! Freed large blocks kept for the next allocation of their size, up to a bound.
//!
//! Unlike the system allocator's cache, this one is bounded. A language server frees a parse arena
//! and soon asks for another of the same size, so keeping a few avoids system calls at steady
//! state. Keeping all of them is the 90 MB phrost measured macOS holding for the life of the
//! server.

use core::{
    cell::UnsafeCell,
    hint::spin_loop,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::{counts, pages};

/// Classes are powers of two from 256 KiB to 64 MiB; a larger block is never kept.
pub(crate) const SMALLEST: usize = 256 * 1024;
const LARGEST_KEPT: usize = 64 * 1024 * 1024;
const CLASSES: usize = (LARGEST_KEPT.trailing_zeros() - SMALLEST.trailing_zeros()) as usize + 1;
/// Blocks kept per class, enough for each worker thread to reuse one.
const SLOTS: usize = 8;

/// The most bytes kept at once: 32 MiB unless a server sets it.
static LIMIT: AtomicUsize = AtomicUsize::new(32 * 1024 * 1024);

/// The mapped size of a large block of `size` bytes: its power-of-two class up to the largest kept
/// class, whole 64 KiB units above it (Windows maps in 64 KiB units, and no target here has a
/// larger page).
pub(crate) fn mapped(size: usize) -> usize {
    if size <= LARGEST_KEPT {
        size.max(SMALLEST).next_power_of_two()
    } else {
        size.div_ceil(64 * 1024) * (64 * 1024)
    }
}

fn class(mapped: usize) -> Option<usize> {
    (mapped <= LARGEST_KEPT).then(|| (mapped.trailing_zeros() - SMALLEST.trailing_zeros()) as usize)
}

/// Sets the most bytes kept and unmaps the excess immediately.
pub(crate) fn set_limit(bytes: usize) {
    LIMIT.store(bytes, Ordering::Relaxed);
    trim_to(bytes);
}

/// Returns a block of `mapped` bytes, reusing a kept one if there is one, else mapping a fresh
/// one. The flag is true for a fresh block, which is zeroed.
pub(crate) fn take(mapped: usize) -> (*mut u8, bool) {
    if let Some(class) = class(mapped) {
        let reused = KEPT.with(|kept| {
            let slot = kept[class].iter_mut().find(|slot| **slot != 0)?;
            // Count under the lock with the slot change: `give` and `trim_to` check the bound
            // against this count, so it must never lag the slots.
            counts::cached(-(mapped as isize));
            Some(core::mem::replace(slot, 0))
        });
        if let Some(block) = reused {
            return (block as *mut u8, false);
        }
    }
    (pages::map(mapped), true)
}

/// Keeps a freed block for reuse if it fits under the bound, else unmaps it.
///
/// # Safety
///
/// `ptr` is a block of exactly `mapped` bytes that [`take`] handed out, and nothing uses it
/// afterwards.
pub(crate) unsafe fn give(ptr: *mut u8, mapped: usize) {
    if let Some(class) = class(mapped) {
        let kept = KEPT.with(|kept| {
            // Check under the lock. Otherwise threads freeing at once could each see room for one
            // more block and together exceed the bound.
            if counts::cached_bytes() + mapped > LIMIT.load(Ordering::Relaxed) {
                return false;
            }
            let Some(slot) = kept[class].iter_mut().find(|slot| **slot == 0) else {
                return false;
            };
            *slot = ptr as usize;
            counts::cached(mapped as isize);
            true
        });
        if kept {
            return;
        }
    }
    // SAFETY: per the caller's contract, this is the whole block and it is not used again.
    unsafe { pages::unmap(ptr, mapped) };
}

/// Unmaps every kept block and returns how many bytes that was.
pub(crate) fn empty() -> usize {
    trim_to(0)
}

fn trim_to(limit: usize) -> usize {
    // Take blocks out under the lock and unmap them after releasing it. Unmapping a full cache is
    // dozens of `munmap`s of dirty pages and takes milliseconds. On Linux each one takes the
    // process's `mmap_lock` for writing, so under the spin lock every other thread's large
    // allocation or free would spin that long. The list is on the stack because nothing here may
    // allocate.
    let mut taken = [(0usize, 0usize); SLOTS * CLASSES];
    let mut count = 0;
    KEPT.with(|kept| {
        for (class, slots) in kept.iter_mut().enumerate().rev() {
            let size = SMALLEST << class;
            for slot in slots.iter_mut() {
                if *slot != 0
                    && counts::cached_bytes() > limit
                    && let Some(entry) = taken.get_mut(count)
                {
                    *entry = (core::mem::replace(slot, 0), size);
                    count += 1;
                    counts::cached(-(size as isize));
                }
            }
        }
    });
    let mut released = 0;
    for &(block, size) in &taken[..count] {
        // SAFETY: a kept block is a whole mapping of its class's size, and removing it from its
        // slot means nothing else can hand it out or unmap it.
        unsafe { pages::unmap(block as *mut u8, size) };
        released += size;
    }
    released
}

/// The kept blocks by class, behind a spin lock. The allocator cannot use a lock that allocates,
/// and the work under this one is a few loads and stores, never a system call.
static KEPT: Locked<[[usize; SLOTS]; CLASSES]> = Locked::new([[0; SLOTS]; CLASSES]);

struct Locked<T> {
    busy: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the value is only reached through `with`, which holds the lock.
unsafe impl<T: Send> Sync for Locked<T> {}

impl<T> Locked<T> {
    const fn new(value: T) -> Locked<T> {
        Locked {
            busy: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    fn with<R>(&self, work: impl FnOnce(&mut T) -> R) -> R {
        // Spin on a plain load and retry the exchange only once the lock looks free. Retrying the
        // exchange in the loop would pull the cache line away from the holder on every turn.
        while self
            .busy
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            while self.busy.load(Ordering::Relaxed) {
                spin_loop();
            }
        }
        // SAFETY: the lock is held, so this is the only reference until it is released.
        let result = work(unsafe { &mut *self.value.get() });
        self.busy.store(false, Ordering::Release);
        result
    }
}
