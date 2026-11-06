//! Cancellation of the request a handler is answering.
//!
//! `$/cancelRequest` stops a request's task at its next `.await`, and the client gets
//! `RequestCancelled` at once. Work that does not await (a parse, an index walk, a model's forward
//! pass on a blocking thread) keeps running. A handler clones [`current`] into such work, which
//! checks it between steps. Work that cannot be interrupted, like a single forward pass, checks it
//! afterwards and discards the result.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::sync::Notify;

/// One request's cancellation, shared by everything working on it.
#[derive(Debug, Clone, Default)]
pub struct Token {
    inner: Arc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    cancelled: AtomicBool,
    notify: Notify,
}

tokio::task_local! {
    static CURRENT: Token;
}

/// The token of the request this task is answering, or `None` outside one: a notification, a
/// background task, a test calling a handler directly.
pub fn current() -> Option<Token> {
    CURRENT.try_with(Token::clone).ok()
}

impl Token {
    pub fn new() -> Token {
        Token::default()
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
        self.inner.notify.notify_waiters();
    }

    /// Resolves once the request is cancelled, and never if it is not.
    pub async fn cancelled(&self) {
        loop {
            let notified = self.inner.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }

    /// Runs `work` with this token as [`current`].
    pub(crate) fn scope<F: Future>(self, work: F) -> impl Future<Output = F::Output> {
        CURRENT.scope(self, work)
    }
}
