//! The message loop, and an output queue so a slow client cannot stall the server.
//!
//! One task reads requests, collects answers and writes them (see [`LspService`]), so a write that
//! blocks stops all three. Writing straight to stdout would let the client's reading speed decide
//! whether the server responds at all.
//!
//! A completion response can be 107 KB and a pipe buffer is 64 KB. If the client stops reading,
//! writing that response blocks, and with it the whole loop: no request is answered and
//! `$/cancelRequest` is never read. Measured in phrost, the server sat at 0.0 % CPU, and after 510
//! more requests it had stopped reading stdin as well. A stack dump showed every worker in
//! `park_condvar` and no analysis running.
//!
//! A queue, `Buffered`, sits between the loop and the pipe. It accepts bytes at once, and `Drain`
//! empties it as fast as the client reads. A slow client then costs memory, which is bounded and
//! recovered once it reads again.
//!
//! Editors read continuously, so this rarely triggers. When it does, the server stops without any
//! error, which is hard to diagnose.

mod buffered;

#[cfg(test)]
mod tests;

use crate::{
    codec::{self, Frame, Unread},
    jsonrpc::{Error, Id, Message, Response},
    server::{ClientSocket, LanguageServer, LspService},
};
use futures_util::{SinkExt as _, StreamExt as _, future::BoxFuture, stream::FuturesUnordered};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader},
    sync::mpsc,
};

/// How many unread bytes the server holds for a client before writes block.
///
/// Without a bound, a client that never reads again would grow the queue past phrost's entire
/// 249 MB footprint on a 30 000-file project, until the OS killed the server. With a bound, the
/// loop blocks again once it is reached, but stays responsive until then.
///
/// 8 MiB is about 78 of the largest responses measured on a real project, and 128 pipe buffers. An
/// editor that stalls that long has closed the pipe or stopped, and a broken pipe is reported
/// instead of waited on.
///
/// At the bound, the server goes quiet, keeps every byte it owes in order, and delivers it all when
/// the client reads again. It never drops or truncates a message: a half-written frame would
/// corrupt the stream, and a client cannot recover from that.
const CAPACITY: usize = 8 * 1024 * 1024;

/// Serves the protocol, writing through a queue so the client's reading speed does not block the
/// message loop.
///
/// The drain is polled beside the loop instead of spawned. Two futures in one task are enough,
/// since a blocked write leaves the other runnable. A panic in the drain then stays as fatal as one
/// in the loop; a spawned drain would die quietly and leave a silent server.
pub async fn serve<I, O, S>(
    name: &str,
    stdin: I,
    stdout: O,
    socket: ClientSocket,
    service: LspService<S>,
) -> Ended
where
    I: AsyncRead + Unpin + Send + 'static,
    O: AsyncWrite + Unpin,
    S: LanguageServer,
{
    let (outbox, drain) = buffered::pair(stdout, CAPACITY);

    let (ended, drained) =
        tokio::join!(dispatch(name, stdin, outbox, socket, service), drain.run());

    // Stderr, because the channel to the client is the one that failed.
    if let Err(error) = drained {
        eprintln!("{name}: stopped writing to the client: {error}");
    }
    ended
}

/// How many messages the reader holds for the loop while it handles a notification.
///
/// It is far above an editor's burst after `initialized` (tens of messages), so a handler waiting
/// on the client still gets its answer. It is bounded so that a notification stuck forever on a
/// silent client holds at most this many messages, each a whole document under full sync. The
/// channel allocates slots in blocks as messages arrive, so unused capacity costs nothing.
const READ_AHEAD: usize = 1024;

/// How a session ended, for the process's exit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "how the session ended is what the process should exit with: `Ended::exit_code`"]
pub enum Ended {
    /// The client sent `exit`; `after_shutdown` when `shutdown` had been received first.
    Exit { after_shutdown: bool },
    /// The input closed without `exit`: the client went away.
    InputClosed,
}

impl Ended {
    /// The exit status the protocol requires: 0 after `shutdown` then `exit`, 1 for `exit` without
    /// `shutdown`. A client that went away gets 0, since nobody reads it.
    pub fn exit_code(self) -> std::process::ExitCode {
        match self {
            Ended::Exit {
                after_shutdown: false,
            } => std::process::ExitCode::from(1),
            _ => std::process::ExitCode::SUCCESS,
        }
    }
}

/// The loop: read a message, dispatch it, and write what comes back, until `exit` or end of input.
///
/// - A request goes to the service, which answers it on its own task. The loop goes back to
///   reading and writes the answer when it is ready.
/// - A notification is finished before the next message is read, so a `didChange` is applied
///   before a request sent after it is dispatched.
/// - A response to one of the server's own requests is delivered by the reader directly, never
///   queued behind a notification, so it may overtake notifications the client sent earlier (see
///   `Client::apply_edit`). Otherwise a notification handler that awaits the client (registering a
///   capability, reading configuration, creating a progress bar) would wait for an answer that can
///   only be read after it finishes. Under `client::ask` that cost five seconds and progress bars
///   for the session; a raw `send_request` hung the server for good. Flow tests cannot catch this,
///   because their harness delivers answers directly.
///
/// Reading runs on its own task and feeds this loop through a channel, because a `select!` that
/// dropped a read mid-message would lose its bytes and every later message. The channel holds
/// [`READ_AHEAD`] messages so the reader keeps reading while a notification is handled. Right after
/// initialization a client sends a burst of requests (tokens, lenses and hints for every open
/// editor); with a bound of sixteen, the answer a handler awaited sat in the pipe behind them. When
/// the reader stops, at `exit` or end of input, the server's pending requests fail.
async fn dispatch<I, W, S>(
    name: &str,
    stdin: I,
    mut outbox: W,
    socket: ClientSocket,
    service: LspService<S>,
) -> Ended
where
    I: AsyncRead + Unpin + Send + 'static,
    W: Outbox,
    S: LanguageServer,
{
    let (read, mut incoming) = mpsc::channel(READ_AHEAD);
    let (mut outgoing, mut answers) = socket.split();
    // Stop reading at `exit`. tokio reads stdin on a blocking thread, and the runtime does not
    // shut down while that read waits. Several clients send `exit` and keep the pipe open, so
    // reading on would leave a server that never exits (orthra hit this).
    let reader = tokio::spawn(async move {
        let mut stdin = BufReader::new(stdin);
        loop {
            let message = codec::read(&mut stdin).await;
            if let Ok(Message::Response(response)) = message {
                let _ = answers.send(response).await;
                continue;
            }
            let last = match &message {
                Err(Unread::Closed | Unread::Broken(_)) => true,
                Ok(Message::Request(request)) => request.method() == "exit",
                _ => false,
            };
            if read.send(message).await.is_err() || last {
                break;
            }
        }
        answers.close();
    });

    // Whether `shutdown` was received before `exit`, which decides the exit status. Received, not
    // answered: a client may send `exit` right after `shutdown`, while the answer is in flight.
    let mut shutdown_received = false;
    let mut ended = Ended::InputClosed;
    let mut answering: FuturesUnordered<Answer> = FuturesUnordered::new();
    let mut handling: Option<Answer> = None;
    loop {
        let written = tokio::select! {
            biased;
            Some(request) = outgoing.next() => write(&mut outbox, Message::Request(request)).await,
            Some(answered) = answering.next() => match answered {
                Some(response) => write(&mut outbox, Message::Response(response)).await,
                None => Ok(()),
            },
            _ = finish(&mut handling), if handling.is_some() => {
                handling = None;
                Ok(())
            }
            message = incoming.recv(), if handling.is_none() => match message {
                None | Some(Err(Unread::Closed)) => break,
                // Stderr, as for a broken pipe: the connection to the client is what broke.
                Some(Err(Unread::Broken(why))) => {
                    eprintln!("{name}: stopped reading from the client, its framing is lost: {why}");
                    break;
                }
                Some(Err(Unread::Malformed(why))) => {
                    let error = Error {
                        message: why.into(),
                        ..Error::parse_error()
                    };
                    write(&mut outbox, Message::Response(Response::from_error(Id::Null, error))).await
                }
                // The reader delivers responses itself; none reach the channel.
                Some(Ok(Message::Response(_))) => Ok(()),
                Some(Ok(Message::Request(request))) => {
                    let exit = request.method() == "exit";
                    shutdown_received |= request.method() == "shutdown";
                    let notification = request.id().is_none();
                    let answer = service.call(request);
                    if exit {
                        let _ = answer.await;
                        ended = Ended::Exit {
                            after_shutdown: shutdown_received,
                        };
                        break;
                    }
                    if notification {
                        handling = Some(answer);
                    } else {
                        answering.push(answer);
                    }
                    Ok(())
                }
            },
        };
        if written.is_err() {
            break;
        }
    }

    reader.abort();
    // Closing the outbox lets the drain finish once it has written the queue.
    let _ = outbox.shutdown().await;
    ended
}

type Answer = BoxFuture<'static, Option<Response>>;

/// Where the loop sends each framed message.
pub(crate) trait Outbox: AsyncWrite + Unpin + Send {
    /// By default, writes the frame's bytes. The queue takes the frame's buffer instead.
    fn send(&mut self, frame: Frame) -> impl Future<Output = std::io::Result<()>> + Send {
        async move { self.write_all(frame.as_bytes()).await }
    }
}

impl Outbox for buffered::Buffered {
    fn send(&mut self, frame: Frame) -> impl Future<Output = std::io::Result<()>> + Send {
        let mut frame = Some(frame);
        std::future::poll_fn(move |context| self.poll_send(context, &mut frame))
    }
}

/// Runs the current notification to completion.
async fn finish(handling: &mut Option<Answer>) {
    if let Some(handling) = handling.as_mut() {
        handling.await;
    }
}

async fn write<W: Outbox>(outbox: &mut W, message: Message) -> std::io::Result<()> {
    outbox.send(codec::frame(&message)).await
}
