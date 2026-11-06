use std::{
    sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize, Ordering},
    },
    time::Duration,
};

use super::editor::Editor;
use crate::quiet::{QUIET, when_quiet};

/// Three keystrokes each schedule a publish. Only the last one publishes, and only after the buffer
/// has been unchanged for the full `QUIET`.
#[tokio::test(start_paused = true)]
async fn only_the_last_of_a_burst_is_published_and_only_once_it_is_quiet() {
    let editor = Editor::new(true).await;
    let held = Arc::new(AtomicI32::new(0));
    let published = Arc::new(AtomicUsize::new(0));

    for version in 1..=3 {
        held.store(version, Ordering::Relaxed);
        let (held, published) = (Arc::clone(&held), Arc::clone(&published));
        when_quiet(
            editor.client.clone(),
            || "publishing".to_owned(),
            version,
            move || async move { Some(held.load(Ordering::Relaxed)) },
            move || async move {
                assert_eq!(version, 3, "a stale version was published");
                published.fetch_add(1, Ordering::Relaxed);
            },
        );
        // A sleep's deadline is set when its task is first polled, which in an editor is at once.
        editor.settle().await;
        tokio::time::advance(Duration::from_millis(100)).await;
    }

    tokio::time::advance(QUIET - Duration::from_millis(101)).await;
    editor.settle().await;
    assert_eq!(
        published.load(Ordering::Relaxed),
        0,
        "published before the quiet"
    );

    tokio::time::advance(Duration::from_millis(2)).await;
    editor.settle().await;
    assert_eq!(published.load(Ordering::Relaxed), 1);
}

#[tokio::test(start_paused = true)]
async fn a_buffer_that_is_gone_is_not_published() {
    let editor = Editor::new(true).await;
    let published = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&published);
    when_quiet(
        editor.client.clone(),
        || "publishing".to_owned(),
        1,
        || async { None },
        move || async move {
            counted.fetch_add(1, Ordering::Relaxed);
        },
    );
    tokio::time::advance(QUIET * 2).await;
    editor.settle().await;
    assert_eq!(published.load(Ordering::Relaxed), 0);
}
