//! Surviving a bug in a handler.
//!
//! Without containment, a `panic!` in a handler unwinds out of the request future, the server
//! loop, `block_on` and `main`. The process exits with 101, the request is never answered, and
//! every other open request is lost. `tower-lsp-server` 0.23, which the servers used before,
//! polled every request future in one task through `buffer_unordered`, so no `tokio::spawn` caught
//! the panic.
//!
//! lspkit's loop runs each request on its own task and answers its panic with an error
//! (`crate::server`); a notification's panic is reported. The handling has to stay visible, so that
//! a crash does not become a silent wrong answer:
//!
//! - The default hook still runs, so stderr keeps the full report. The client is also told which
//!   request failed and where the panic was raised.
//! - The request is answered. An editor waits forever on a JSON-RPC id with no response, so a
//!   caught panic becomes an error response.
//! - Only servers install this. A command-line checker (phrost's `check` and `perf`) still dies on
//!   a panic, because skipping the file it crashed on would report a clean result it has not
//!   earned.
//!
//! This does not keep the index correct. A read (goto, hover, completion: the whole hot path)
//! takes a read guard and mutates nothing, so an unwind changes nothing. A write holds a write
//! guard while it changes the index, and tokio's `RwLock` does not poison, so an unwind there
//! leaves the index readable and possibly half-updated. [`PoisoningWrite`] makes that visible.
//! After a caught panic the server keeps running, but its index may already be wrong.

mod hook;
mod poison;

pub use self::{hook::install, poison::PoisoningWrite};

use std::{
    any::Any,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll},
};

use ls_types::MessageType;

use crate::{
    jsonrpc::{Error, ErrorCode, Id},
    server::Client,
};

/// Whether this session has already shown a panic to the user as a message, beyond the log.
///
/// Only once per session: a panic reachable from one cursor position recurs on every keystroke,
/// and a dialog per keystroke would make the editor unusable.
static ANNOUNCED: AtomicBool = AtomicBool::new(false);

/// A caught panic, with its message and location.
#[derive(Debug)]
pub struct Panic {
    /// The `panic!` argument, or the message an `unwrap` produced.
    message: String,
    /// `file:line:column`, when [`hook`] was installed in time to see it.
    location: Option<String>,
}

impl Panic {
    /// Runs a background future and returns its panic, if any, instead of losing it to the runtime.
    ///
    /// A panicking `tokio::spawn`ed task does not stop the server. The runtime keeps the payload in
    /// a `JoinHandle` nobody holds, and the work silently does not happen. When the debounced
    /// diagnostics task panicked, the server answered everything else normally and the buffer
    /// showed no findings, which looks the same as a clean file.
    pub async fn watching<T>(work: impl Future<Output = T> + Send) -> Result<T, Self> {
        // Pinned to this frame rather than boxed. The futures that reach it from the loop are boxed
        // already, and a second box was an allocation on every request and every notification.
        let work = std::pin::pin!(work);
        Caught::over(work).await
    }

    /// Turns a `catch_unwind` payload into a printable panic.
    ///
    /// Both payload types are common: `panic!("literal")` and `expect("...")` carry a
    /// `&'static str`, while a formatted `panic!("{name}")` and arithmetic overflow carry a
    /// `String`.
    fn from_payload(payload: &(dyn Any + Send)) -> Self {
        let message = payload
            .downcast_ref::<&'static str>()
            .map(|text| (*text).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic carrying no message".to_owned());

        Self {
            message,
            location: hook::take_location(),
        }
    }

    /// One line saying what failed, where, and what was being done.
    fn describe(&self, doing: &str) -> String {
        match &self.location {
            Some(at) => format!(
                "{} failed while {doing}: {} (at {at})",
                hook::name(),
                self.message
            ),
            None => format!("{} failed while {doing}: {}", hook::name(), self.message),
        }
    }

    /// The error response the request gets, so the editor stops waiting on its id.
    pub(crate) fn into_error(self, doing: &str) -> Error {
        Error {
            code: ErrorCode::InternalError,
            message: self.describe(doing).into(),
            data: None,
        }
    }

    /// Logs the panic to the client, and the first time in a session also shows it as a message.
    ///
    /// A language server's log is read only by someone who already suspects the server. After a
    /// caught panic, answers may be missing or stale, so the user is told once.
    pub async fn report(&self, client: &Client, doing: &str) {
        client
            .log_message(MessageType::ERROR, self.describe(doing))
            .await;

        if !ANNOUNCED.swap(true, Ordering::Relaxed) {
            client
                .show_message(
                    MessageType::ERROR,
                    format!(
                        "{} hit an internal error and refused a request. It is still running, but \
                         answers after this may be missing or out of date -- the language server log \
                         has the detail, and restarting the server clears it.",
                        hook::name()
                    ),
                )
                .await;
        }
    }
}

/// Describes what the server was doing when a request or notification panicked.
///
/// The request id is included because a client sends several completions per second, and the
/// method alone does not say which one failed. Call it only after a panic: building it eagerly
/// cost a `format!` per message, including every keystroke's `didChange`.
pub(crate) fn describe(method: &str, id: Option<&Id>) -> String {
    match id {
        Some(id) => format!("answering {method} (request {id})"),
        None => format!("handling {method}"),
    }
}

/// Spawns `work` on its own task and reports a panic in it as `doing`.
///
/// Unreported, a panic in a spawned task goes unnoticed. tokio keeps the payload in a `JoinHandle`
/// nobody holds and the server keeps answering, but the work never finishes. A pass never
/// publishes, or a buffer never gets findings again, which looks like a clean file. `doing` is
/// built only after a panic, since this runs on every keystroke.
pub fn spawn<F>(
    client: Client,
    doing: impl FnOnce() -> String + Send + 'static,
    work: F,
) -> tokio::task::JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        if let Err(panicked) = Panic::watching(work).await {
            panicked.report(&client, &doing()).await;
        }
    })
}

/// A future that returns its inner future's panic instead of propagating it.
///
/// It requires `Unpin` instead of a pin projection, because `#![forbid(unsafe_code)]` rules out
/// writing one by hand. [`Panic::watching`] pins the future to its own frame, and a `Pin<&mut F>`
/// is `Unpin`.
struct Caught<F> {
    inner: F,
    /// Set once the inner future unwound out of `poll`.
    ///
    /// A future that panicked mid-poll may be in a state its author never intended, and the
    /// `Future` contract says nothing about polling it again, so it is not polled again.
    unwound: bool,
}

impl<F> Caught<F> {
    fn over(inner: F) -> Self {
        Self {
            inner,
            unwound: false,
        }
    }
}

impl<F: Future + Unpin> Future for Caught<F> {
    type Output = Result<F::Output, Panic>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.unwound {
            // Unreachable: the poll that unwound returned `Ready`, and a finished future is not
            // polled again. `Pending` instead of a panic, because a panic here could not be caught.
            return Poll::Pending;
        }

        // `AssertUnwindSafe` over a `&mut` to another future is a claim this type cannot check.
        // The module doc says what it covers and where it stops being true.
        let inner = &mut this.inner;
        match catch_unwind(AssertUnwindSafe(|| Pin::new(inner).poll(context))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(answer)) => Poll::Ready(Ok(answer)),
            Err(payload) => {
                this.unwound = true;
                Poll::Ready(Err(Panic::from_payload(payload.as_ref())))
            }
        }
    }
}
