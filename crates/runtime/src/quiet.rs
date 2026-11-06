//! Publishes a buffer's findings once the user stops typing.

use std::{future::Future, time::Duration};

use crate::server::Client;

use crate::panics;

/// How long a buffer must go unchanged before its findings are published.
///
/// The delay saves no work: checking a file takes milliseconds in phrost and orthra. It keeps
/// squiggles from flickering on half-typed words. Only changes wait; opening a file publishes at
/// once.
pub const QUIET: Duration = Duration::from_millis(300);

/// Spawns a task that waits [`QUIET`], then runs `work` if `current` still returns `version`.
///
/// Nothing is tracked or cancelled. Each change schedules its own wait, and a stale task wakes,
/// sees that the version moved, and exits. A map of timers per document would do the same and
/// could leak. `current` returns `None` for a buffer that is gone or one the server cannot report
/// on yet.
///
/// A panic in `work` is reported as `doing` ([`panics::spawn`]). Unreported, the buffer would
/// silently stop getting findings, which looks like a file with no problems.
pub fn when_quiet<V, C, CF, W, WF>(
    client: Client,
    doing: impl FnOnce() -> String + Send + 'static,
    version: V,
    current: C,
    work: W,
) where
    V: PartialEq + Send + 'static,
    C: FnOnce() -> CF + Send + 'static,
    CF: Future<Output = Option<V>> + Send,
    W: FnOnce() -> WF + Send + 'static,
    WF: Future<Output = ()> + Send,
{
    panics::spawn(client, doing, async move {
        tokio::time::sleep(QUIET).await;
        if current().await.as_ref() == Some(&version) {
            work().await;
        }
    });
}
