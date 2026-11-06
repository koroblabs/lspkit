//! A queue between the code that writes a response and the pipe that carries it.

use std::{
    collections::VecDeque,
    io,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    task::{Context, Poll, Waker},
};

use tokio::io::{AsyncWrite, AsyncWriteExt as _};

use crate::codec::Frame;

/// Builds the two halves of the queue: the writer the transport uses, and the drain that empties
/// it.
///
/// `capacity` is how many bytes the queue holds for a client that is not reading, counted as the
/// memory its frames take. Beyond it the writer blocks (see [`Buffered::poll_send`] and the parent
/// module's `CAPACITY`).
pub(crate) fn pair<O>(out: O, capacity: usize) -> (Buffered, Drain<O>) {
    let shared = Arc::new(Shared {
        queue: Mutex::new(Queue::default()),
        capacity,
    });

    (
        Buffered {
            shared: Arc::clone(&shared),
        },
        Drain { shared, out },
    )
}

/// The writer the protocol layer sees. It only queues bytes; `Drain` writes them out.
#[derive(Debug)]
pub(crate) struct Buffered {
    shared: Arc<Shared>,
}

impl Buffered {
    /// Takes a framed message whole, keeping its buffer without a copy, once there is any room.
    ///
    /// Unlike `poll_write`, it does not cut the message to the room left. A message is queued as
    /// soon as the queue is below its bound, so the bound is exceeded by at most one message and
    /// nothing is copied to fit.
    pub(crate) fn poll_send(
        &self,
        context: &mut Context<'_>,
        frame: &mut Option<Frame>,
    ) -> Poll<io::Result<()>> {
        let mut queue = self.shared.locked();
        if queue.broken {
            return Poll::Ready(Err(gone()));
        }
        if queue.held >= self.shared.capacity {
            queue.waiting_transport = Some(context.waker().clone());
            return Poll::Pending;
        }
        if let Some(frame) = frame.take() {
            queue.held += frame.cost();
            queue.chunks.push_back(frame);
            wake(queue.waiting_drain.take());
        }
        Poll::Ready(Ok(()))
    }
}

/// The other end: writes queued bytes to the client as fast as the client reads.
pub(crate) struct Drain<O> {
    shared: Arc<Shared>,
    out: O,
}

impl<O: AsyncWrite + Unpin> Drain<O> {
    /// Empties the queue until the writer closes it.
    ///
    /// This is the only future that may block on the client. Everything the server does to answer
    /// a request happens on the other side of the queue, so a slow client slows only this.
    pub(crate) async fn run(self) -> io::Result<()> {
        let Self { shared, mut out } = self;

        loop {
            let waiting = std::future::poll_fn(|context| shared.take(context)).await;

            let Some(chunk) = waiting else {
                // Closed and empty: flush the underlying writer, then let the waiting shutdown
                // finish.
                let flushed = out.flush().await;
                shared.finish(flushed.is_err());
                return flushed;
            };

            if let Err(error) = Self::deliver(&mut out, chunk.as_bytes(), &shared).await {
                // Nothing can be delivered any more. Tell the writer, or it stays blocked on a
                // queue that never empties, which is the hang this type exists to prevent.
                shared.fail();
                return Err(error);
            }
        }
    }

    /// Writes one chunk, flushing only when nothing is queued behind it.
    ///
    /// The flush makes a response visible to a client waiting on it, so it must happen before this
    /// blocks. A flush between two chunks of a backlog would only cost a syscall.
    async fn deliver(out: &mut O, chunk: &[u8], shared: &Shared) -> io::Result<()> {
        out.write_all(chunk).await?;
        if shared.idle() {
            out.flush().await?;
        }

        Ok(())
    }
}

/// State shared by both halves.
#[derive(Debug)]
struct Shared {
    queue: Mutex<Queue>,
    capacity: usize,
}

#[derive(Debug, Default)]
struct Queue {
    /// Frames waiting to be written, in arrival order.
    chunks: VecDeque<Frame>,
    /// The memory those frames take, compared against `capacity`.
    held: usize,
    /// The writer has shut down: no more bytes will arrive.
    closed: bool,
    /// Everything that arrived has reached the client and been flushed.
    emptied: bool,
    /// The client's end failed. Nothing more can be delivered, and nothing should wait for it.
    broken: bool,
    /// The transport, waiting either for room or for the last byte to leave.
    ///
    /// One slot serves both, because the transport is a single sequential writer and cannot wait
    /// for both at once.
    waiting_transport: Option<Waker>,
    /// The drain, parked on an empty queue.
    waiting_drain: Option<Waker>,
}

impl Shared {
    fn locked(&self) -> MutexGuard<'_, Queue> {
        // Recover from poisoning instead of propagating it. These critical sections only move
        // bytes and wakers and cannot panic, and a panic here would take the transport down.
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// One chunk for the drain, or `None` once the writer has closed and the queue is empty.
    fn take(&self, context: &mut Context<'_>) -> Poll<Option<Frame>> {
        let mut queue = self.locked();

        if let Some(chunk) = queue.chunks.pop_front() {
            queue.held -= chunk.cost();
            wake(queue.waiting_transport.take());
            return Poll::Ready(Some(chunk));
        }

        if queue.closed {
            return Poll::Ready(None);
        }

        queue.waiting_drain = Some(context.waker().clone());
        Poll::Pending
    }

    /// Whether the drain is about to have nothing left to write.
    fn idle(&self) -> bool {
        self.locked().chunks.is_empty()
    }

    /// Records that everything queued has reached the client, so a pending shutdown can finish.
    fn finish(&self, failed: bool) {
        let mut queue = self.locked();
        queue.emptied = true;
        queue.broken |= failed;
        wake(queue.waiting_transport.take());
    }

    /// Records that the client's end is gone, so nothing waits on it.
    fn fail(&self) {
        let mut queue = self.locked();
        queue.broken = true;
        queue.chunks.clear();
        queue.held = 0;
        wake(queue.waiting_transport.take());
    }
}

impl AsyncWrite for Buffered {
    /// Copies as much of `buf` as there is room for and returns at once, so the queue works as a
    /// plain writer. The message loop uses [`Buffered::poll_send`] instead, which takes frames
    /// whole.
    ///
    /// `AsyncWrite` allows a short write and the caller loops, so this blocks only when there is no
    /// room at all.
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut queue = self.shared.locked();
        if queue.broken {
            return Poll::Ready(Err(gone()));
        }

        let room = self.shared.capacity.saturating_sub(queue.held);
        if room == 0 {
            queue.waiting_transport = Some(context.waker().clone());
            return Poll::Pending;
        }

        let taken = buf.len().min(room);
        let frame = Frame::copied(&buf[..taken]);
        queue.held += frame.cost();
        queue.chunks.push_back(frame);
        wake(queue.waiting_drain.take());

        Poll::Ready(Ok(taken))
    }

    /// Returns at once without writing anything.
    ///
    /// This deliberately reads the trait loosely. A writer above may flush after every message
    /// (`tower-lsp-server`'s `FramedWrite` did), and a flush that waited for the client would bring
    /// back the blocking the queue removes. Callers need the bytes delivered in order, which holds
    /// once `poll_write` accepts them. [`Buffered::poll_shutdown`] is the call that waits.
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.shared.locked().broken {
            return Poll::Ready(Err(gone()));
        }

        Poll::Ready(Ok(()))
    }

    /// Waits for the last byte to reach the client.
    ///
    /// `shutdown` is answered like any other request, so its response may still be queued when the
    /// loop closes the stream. Returning early would drop it, and the editor would see a crash
    /// instead of a clean stop.
    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut queue = self.shared.locked();
        queue.closed = true;

        // Broken counts as done: the remaining bytes have nowhere to go, and waiting for a client
        // that has gone away is the hang this type prevents.
        if queue.broken || queue.emptied {
            return Poll::Ready(Ok(()));
        }

        queue.waiting_transport = Some(context.waker().clone());
        wake(queue.waiting_drain.take());
        Poll::Pending
    }
}

impl Drop for Buffered {
    /// Closes the queue, so the drain can always finish.
    ///
    /// Normally the transport shuts down first and the queue is already closed. Without this, a
    /// transport torn down without a shutdown would leave the drain waiting on a queue nothing adds
    /// to.
    fn drop(&mut self) {
        let mut queue = self.shared.locked();
        queue.closed = true;
        wake(queue.waiting_drain.take());
    }
}

fn wake(waker: Option<Waker>) {
    if let Some(waker) = waker {
        waker.wake();
    }
}

fn gone() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "the client stopped reading")
}

impl<O> std::fmt::Debug for Drain<O> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Drain").finish_non_exhaustive()
    }
}
