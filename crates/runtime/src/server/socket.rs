//! The client's end of a session, for a transport or a test acting as the editor: a stream of what
//! the server sends, and a sink for answers to the server's requests.

use std::{
    fmt,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_util::{Sink, Stream};
use tokio::sync::mpsc;

use super::client::Waiting;
use crate::jsonrpc::{Request, Response};

/// The client's end of a session. Its stream ends once the server is gone, because the socket
/// holds only the receiving end of the channel the server's clients send on.
pub struct ClientSocket {
    outgoing: mpsc::Receiver<Request>,
    waiting: Arc<Waiting>,
}

impl fmt::Debug for ClientSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientSocket").finish_non_exhaustive()
    }
}

impl ClientSocket {
    pub(crate) fn new(outgoing: mpsc::Receiver<Request>, waiting: Arc<Waiting>) -> ClientSocket {
        ClientSocket { outgoing, waiting }
    }

    /// Splits the socket into what the server sends and where the client's answers go. The order
    /// matches `tower-lsp-server`'s split, so a harness written against it works unchanged.
    pub fn split(self) -> (RequestStream, ResponseSink) {
        (
            RequestStream {
                outgoing: self.outgoing,
            },
            ResponseSink {
                waiting: self.waiting,
            },
        )
    }
}

/// What the server sends the client, in order.
pub struct RequestStream {
    outgoing: mpsc::Receiver<Request>,
}

impl fmt::Debug for RequestStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestStream").finish_non_exhaustive()
    }
}

impl Stream for RequestStream {
    type Item = Request;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Request>> {
        self.outgoing.poll_recv(cx)
    }
}

/// The client's answers to the server's requests.
pub struct ResponseSink {
    waiting: Arc<Waiting>,
}

impl fmt::Debug for ResponseSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResponseSink").finish_non_exhaustive()
    }
}

impl ResponseSink {
    /// Marks that nothing more will be read from the client, so the server's waiting requests, and
    /// any sent later, fail instead of waiting forever.
    pub(crate) fn close(&self) {
        self.waiting.close();
    }
}

impl Sink<Response> for ResponseSink {
    type Error = std::convert::Infallible;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, response: Response) -> Result<(), Self::Error> {
        self.waiting.answered(response);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}
