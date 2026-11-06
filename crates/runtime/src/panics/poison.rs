//! Poisoning for tokio's `RwLock`, which does not poison on its own.
//!
//! `std::sync::RwLock` marks itself poisoned when a writer unwinds, so later readers learn the data
//! may be half-written. Tokio's lock releases the guard, and the next reader sees whatever the
//! writer had done. For an index, that means plausible answers from a state no file on disk
//! describes. A server that must give a correct answer or a deterministic nothing (phrost's
//! zero-fallbacks rule) cannot allow that.
//!
//! The protocol layer knows that a request panicked but not what the handler changed. The guard
//! knows, because it is the thing being dropped.
//!
//! A server should refuse a poisoned index instead of rebuilding it. A panic under a write is a
//! bug, and its input is still there: a file that fails in lowering fails again when re-indexed,
//! and the server spends the session crashing and re-indexing. phrost's `Session::failed` does not
//! retry for the same reason. Its refusal says which state the server is in and that a restart
//! fixes it.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tokio::sync::RwLockWriteGuard;

/// A write guard that sets a poison flag if the write unwinds.
///
/// It derefs to the guarded value, so a write site reads the same as with a plain guard.
pub struct PoisoningWrite<'a, T> {
    guard: RwLockWriteGuard<'a, T>,
    poisoned: Arc<AtomicBool>,
}

impl<'a, T> PoisoningWrite<'a, T> {
    pub fn over(guard: RwLockWriteGuard<'a, T>, poisoned: Arc<AtomicBool>) -> Self {
        Self { guard, poisoned }
    }
}

impl<T> std::ops::Deref for PoisoningWrite<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> std::ops::DerefMut for PoisoningWrite<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> std::fmt::Debug for PoisoningWrite<'_, T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PoisoningWrite")
            .finish_non_exhaustive()
    }
}

impl<T> Drop for PoisoningWrite<'_, T> {
    /// Poisons when `thread::panicking()` is true. An unwind runs every destructor between the
    /// panic and the catch on the panicking thread. An async task unwinds on the worker polling
    /// it, so a guard held across an `await` is dropped there while panicking.
    ///
    /// `Relaxed` is enough. A later request on another task reads the flag, and the lock release
    /// right after this drop publishes everything the writer did.
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.poisoned.store(true, Ordering::Relaxed);
        }
    }
}
