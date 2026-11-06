//! The queue's guarantees: the loop never waits for the client, and the bytes arrive unchanged.

use std::{
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    time::Duration,
};

use crate::protocol::{
    Client, ClientSocket, LanguageServer, LspService, jsonrpc::Result, ls_types::*,
};
use futures_util::poll;
use tokio::io::{AsyncWrite, AsyncWriteExt as _};

use super::{Outbox as _, buffered::pair};
use crate::{
    codec::{self, Frame},
    jsonrpc::Message,
};

/// A stdout that accepts nothing until it is released, and keeps everything it is given.
///
/// It stands in for a client that has stopped reading. A real pipe fills after 64 KB; this one
/// accepts nothing at all, which tests the same property with no setup.
#[derive(Clone, Default)]
struct Held(Arc<Mutex<HeldState>>);

#[derive(Default)]
struct HeldState {
    accepting: bool,
    failing: bool,
    written: Vec<u8>,
    waiting: Option<Waker>,
}

impl Held {
    /// A client reading normally.
    fn reading() -> Self {
        let held = Self::default();
        held.0.lock().expect("uncontended").accepting = true;
        held
    }

    /// A client whose pipe is gone.
    fn broken() -> Self {
        let held = Self::default();
        let mut state = held.0.lock().expect("uncontended");
        state.accepting = true;
        state.failing = true;
        drop(state);
        held
    }

    /// Starts reading, as a client that had stalled would.
    fn release(&self) {
        let mut state = self.0.lock().expect("uncontended");
        state.accepting = true;
        if let Some(waker) = state.waiting.take() {
            waker.wake();
        }
    }

    fn written(&self) -> Vec<u8> {
        self.0.lock().expect("uncontended").written.clone()
    }
}

impl AsyncWrite for Held {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut state = self.0.lock().expect("uncontended");
        if state.failing {
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, "gone")));
        }
        if !state.accepting {
            state.waiting = Some(context.waker().clone());
            return Poll::Pending;
        }

        state.written.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// A writer the loop uses directly, as it did before the queue, for the test without one.
impl super::Outbox for Held {}

impl std::fmt::Debug for Held {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Held").finish_non_exhaustive()
    }
}

#[tokio::test]
async fn a_response_too_big_for_the_pipe_does_not_park_the_writer() {
    let client = Held::default();
    let (mut writer, _drain) = pair(client.clone(), super::CAPACITY);

    // The measured size of a member completion on a real project; a pipe buffer is 64 KB.
    let response = vec![b'x'; 107 * 1024];

    let mut direct = client.clone();
    assert!(
        poll!(Box::pin(direct.write(&response))).is_pending(),
        "the client under test has to be one that really does not read"
    );

    writer
        .write_all(&response)
        .await
        .expect("the queue takes it");
    assert_eq!(
        client.written(),
        Vec::<u8>::new(),
        "and it went nowhere yet"
    );
}

#[tokio::test]
async fn every_byte_arrives_in_the_order_it_was_written() {
    let client = Held::reading();
    // Far smaller than any message, so the writer fills the queue, blocks and is woken repeatedly.
    // That is the path where a reordered or dropped byte would show.
    let (mut writer, drain) = pair(client.clone(), 64);
    let draining = tokio::spawn(drain.run());

    let sent: Vec<u8> = (0..40_000u32).map(|byte| (byte % 251) as u8).collect();
    for chunk in sent.chunks(7) {
        writer.write_all(chunk).await.expect("the queue takes it");
    }
    writer.shutdown().await.expect("the queue empties");
    draining
        .await
        .expect("the drain does not panic")
        .expect("the client took everything");

    assert_eq!(client.written(), sent);
}

#[tokio::test]
async fn a_client_that_never_reads_is_held_to_the_bound_and_then_backs_up() {
    let (mut writer, _drain) = pair(Held::default(), 1024);

    assert_eq!(
        writer.write(&[b'a'; 700]).await.expect("room for it"),
        700,
        "under the bound, all of it"
    );
    assert_eq!(
        writer.write(&[b'b'; 700]).await.expect("room for some"),
        324,
        "across the bound, as much as fits"
    );
    assert!(
        poll!(Box::pin(writer.write(b"c"))).is_pending(),
        "at the bound the writer waits, which is the deliberate failure at the extreme"
    );
}

#[tokio::test]
async fn shutdown_waits_for_the_last_byte_to_reach_the_client() {
    let client = Held::default();
    let (mut writer, drain) = pair(client.clone(), super::CAPACITY);
    let draining = tokio::spawn(drain.run());

    let farewell = b"Content-Length: 38\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":9,\"result\":null}";
    writer
        .write_all(farewell)
        .await
        .expect("the queue takes it");

    let mut closing = Box::pin(writer.shutdown());
    assert!(poll!(closing.as_mut()).is_pending(), "nothing has left yet");
    assert_eq!(client.written(), Vec::<u8>::new());

    client.release();
    closing.await.expect("the queue empties");
    draining
        .await
        .expect("the drain does not panic")
        .expect("the client took everything");

    assert_eq!(client.written(), farewell);
}

#[tokio::test]
async fn a_broken_pipe_ends_the_writer_rather_than_parking_it_forever() {
    let (mut writer, drain) = pair(Held::broken(), 1024);
    let draining = tokio::spawn(drain.run());

    // The first write reaches the queue: nothing has failed yet, because nothing has been sent.
    writer.write_all(b"one").await.expect("the queue takes it");
    draining
        .await
        .expect("the drain does not panic")
        .expect_err("the client's end is gone");

    let error = writer
        .write_all(b"two")
        .await
        .expect_err("and the writer is told, rather than left waiting on a queue nobody empties");
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);

    writer.shutdown().await.expect("shutting down still ends");
}

// --- the same promises for frames, which the loop hands over whole ---

fn notification(n: usize) -> Frame {
    let request = crate::jsonrpc::Request::build("$/progress")
        .params(serde_json::json!({ "token": "t", "value": { "kind": "report", "percentage": n } }))
        .finish();
    codec::frame(&Message::Request(request))
}

#[tokio::test]
async fn frames_arrive_whole_and_in_order_through_every_park() {
    let client = Held::reading();
    // Smaller than one frame, so every send waits for the drain and is woken again.
    let (mut writer, drain) = pair(client.clone(), 64);
    let draining = tokio::spawn(drain.run());

    let mut sent = Vec::new();
    // Bounded, so that a writer nobody wakes fails here instead of hanging the suite.
    tokio::time::timeout(Duration::from_secs(10), async {
        for n in 0..2_000 {
            let frame = notification(n);
            sent.extend_from_slice(frame.as_bytes());
            writer.send(frame).await.expect("the queue takes it");
        }
        writer.shutdown().await.expect("the queue empties");
    })
    .await
    .expect("every parked send is woken");
    draining
        .await
        .expect("the drain does not panic")
        .expect("the client took everything");

    assert_eq!(client.written(), sent);
}

#[tokio::test]
async fn a_frame_is_taken_whole_below_the_bound_and_waits_above_it() {
    let (mut writer, _drain) = pair(Held::default(), 1024);

    writer
        .send(Frame::copied(&[b'a'; 700]))
        .await
        .expect("under the bound");
    // A frame crossing the bound is still taken whole, so the bound is exceeded by at most that
    // one frame.
    writer
        .send(Frame::copied(&[b'b'; 700]))
        .await
        .expect("room left when it came");
    assert!(
        poll!(Box::pin(writer.send(Frame::copied(b"c")))).is_pending(),
        "past the bound the writer waits"
    );
}

#[tokio::test]
async fn a_broken_pipe_ends_a_frame_writer_rather_than_parking_it_forever() {
    let (mut writer, drain) = pair(Held::broken(), 1024);
    let draining = tokio::spawn(drain.run());

    writer
        .send(notification(1))
        .await
        .expect("the queue takes it");
    draining
        .await
        .expect("the drain does not panic")
        .expect_err("the client's end is gone");

    let error = writer
        .send(notification(2))
        .await
        .expect_err("and the writer is told");
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
}

/// The bound counts a frame's memory, including spare capacity, and not only the bytes it sends.
/// Frames are presized with room to spare, so a queue of small frames counted by length held more
/// than its bound.
#[tokio::test]
async fn the_bound_counts_the_memory_a_frame_takes() {
    let frame = notification(1);
    assert!(frame.cost() >= frame.as_bytes().len());
    let (mut writer, _drain) = pair(Held::default(), frame.cost());

    writer
        .send(frame)
        .await
        .expect("exactly at the bound once taken");
    assert!(
        poll!(Box::pin(writer.send(notification(2)))).is_pending(),
        "and full by what it takes, not by what it sends"
    );
}

// --- the whole transport, against a client that reads nothing ---

/// Everything the server was asked to do, in order.
type Asked = Arc<Mutex<Vec<String>>>;

/// A language server that only records what it was asked.
///
/// The tests here are about the loop, so this is enough: a handler that never ran is a request that
/// was never answered.
struct Recording(Asked);

impl Recording {
    fn note(&self, what: String) {
        self.0.lock().expect("uncontended").push(what);
    }
}

impl LanguageServer for Recording {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        self.note("initialize".to_owned());
        Ok(InitializeResult::default())
    }

    async fn shutdown(&self) -> Result<()> {
        self.note("shutdown".to_owned());
        Ok(())
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        // By line, so the test can name the last request instead of counting how many got through.
        let line = params.text_document_position_params.position.line;
        self.note(format!("hover {line}"));

        // The first answer is larger than a pipe's buffer, so writing it blocks unless something
        // takes it whole. Small responses fit in the pipe, which is why the bug only showed with
        // completion lists.
        Ok((line == 0).then(|| Hover {
            contents: HoverContents::Scalar(MarkedString::String("x".repeat(107 * 1024))),
            range: None,
        }))
    }
}

/// How many hovers follow `initialize`. More than the transport's concurrency limit, so a server
/// that answers a few and then stops is distinguishable from one that keeps going.
const HOVERS: u32 = 20;

fn framed(message: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{message}", message.len()).into_bytes()
}

fn initialize() -> Vec<u8> {
    framed(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#)
}

fn hovers() -> Vec<u8> {
    (0..HOVERS)
        .flat_map(|line| {
            framed(&format!(
                r#"{{"jsonrpc":"2.0","id":{},"method":"textDocument/hover","params":{{"textDocument":{{"uri":"file:///a.php"}},"position":{{"line":{line},"character":0}}}}}}"#,
                line + 2
            ))
        })
        .collect()
}

fn recording() -> (Asked, LspService<Recording>, ClientSocket) {
    let asked = Asked::default();
    let built = Arc::clone(&asked);
    let (service, socket) = LspService::new(move |_| Recording(built));
    (asked, service, socket)
}

/// Lets every runnable task run. Under a paused clock nothing sleeps: the runtime advances time
/// only when no task can make progress, which is the state the tests assert on.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn requests_are_still_answered_while_the_client_reads_nothing() {
    let (asked, service, socket) = recording();
    let (mut client, server) = tokio::io::duplex(1 << 16);

    tokio::spawn(super::serve(
        "test",
        server,
        Held::default(),
        socket,
        service,
    ));

    client
        .write_all(&initialize())
        .await
        .expect("stdin accepts");
    // Separately, as a client sends them: nothing else until `initialize` is answered.
    settle().await;
    client.write_all(&hovers()).await.expect("stdin accepts");
    settle().await;

    let expected: Vec<String> = std::iter::once("initialize".to_owned())
        .chain((0..HOVERS).map(|line| format!("hover {line}")))
        .collect();
    assert_eq!(*asked.lock().expect("uncontended"), expected);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn without_the_queue_the_same_client_stops_the_server() {
    let (asked, service, socket) = recording();
    let (mut client, server) = tokio::io::duplex(1 << 16);

    // Deliberately not `super::serve`: the loop gets stdout directly, as it did before this module.
    // The test shows why the queue exists: when the loop writes inline, a blocked write blocks
    // reading too.
    tokio::spawn(super::dispatch(
        "test",
        server,
        Held::default(),
        socket,
        service,
    ));

    client
        .write_all(&initialize())
        .await
        .expect("stdin accepts");
    settle().await;
    client.write_all(&hovers()).await.expect("stdin accepts");
    settle().await;

    let asked = asked.lock().expect("uncontended");
    assert!(
        !asked.contains(&format!("hover {}", HOVERS - 1)),
        "the last request should never have been reached; got {asked:?}"
    );
}

// --- a notification handler that waits for the client ---

/// A server whose `initialized` registers a capability and waits for the client's answer, as a
/// server that watches files or asks for configuration does.
struct Asking {
    client: Client,
    asked: Asked,
}

impl LanguageServer for Asking {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult::default())
    }

    async fn initialized(&self, _: InitializedParams) {
        let answer = self.client.register_capability(Vec::new()).await;
        self.asked
            .lock()
            .expect("uncontended")
            .push(format!("registered: {answer:?}"));
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let line = params.text_document_position_params.position.line;
        self.asked
            .lock()
            .expect("uncontended")
            .push(format!("hover {line}"));
        Ok(None)
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_notification_waiting_for_the_client_gets_its_answer_behind_a_burst_of_requests() {
    let asked = Asked::default();
    let built = Arc::clone(&asked);
    let (service, socket) = LspService::new(move |client| Asking {
        client,
        asked: built,
    });
    let (mut client, server) = tokio::io::duplex(1 << 20);
    let stdout = Held::reading();
    tokio::spawn(super::serve(
        "test",
        server,
        stdout.clone(),
        socket,
        service,
    ));

    client
        .write_all(&initialize())
        .await
        .expect("stdin accepts");
    settle().await;
    client
        .write_all(&framed(
            r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
        ))
        .await
        .expect("stdin accepts");
    settle().await;
    let written = String::from_utf8(stdout.written()).expect("the server writes UTF-8");
    assert!(
        written.contains(r#""method":"client/registerCapability""#),
        "the server should have asked by now; it wrote {written}"
    );

    // What an editor sends right after initialization, ahead of its answer in the pipe: more
    // requests than any small read-ahead bound.
    client.write_all(&hovers()).await.expect("stdin accepts");
    client
        .write_all(&framed(r#"{"jsonrpc":"2.0","id":0,"result":null}"#))
        .await
        .expect("stdin accepts");
    settle().await;

    let expected: Vec<String> = std::iter::once("registered: Ok(())".to_owned())
        .chain((0..HOVERS).map(|line| format!("hover {line}")))
        .collect();
    assert_eq!(*asked.lock().expect("uncontended"), expected);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_client_gone_while_a_notification_waits_on_it_still_ends_the_session() {
    let asked = Asked::default();
    let built = Arc::clone(&asked);
    let (service, socket) = LspService::new(move |client| Asking {
        client,
        asked: built,
    });
    let (mut client, server) = tokio::io::duplex(1 << 20);
    let session = tokio::spawn(super::serve(
        "test",
        server,
        Held::reading(),
        socket,
        service,
    ));

    client
        .write_all(&initialize())
        .await
        .expect("stdin accepts");
    settle().await;
    client
        .write_all(&framed(
            r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
        ))
        .await
        .expect("stdin accepts");
    settle().await;
    // The client goes away without answering what `initialized` is waiting for.
    drop(client);

    let ended = tokio::time::timeout(Duration::from_secs(60), session)
        .await
        .expect("the session ends rather than waiting for an answer nobody will read")
        .expect("no panic");
    assert_eq!(ended, super::Ended::InputClosed);
    expect_test::expect![[r#"
        [
            "registered: Err(Error { code: InternalError, message: \"Internal error\", data: None })",
        ]
    "#]]
    .assert_debug_eq(&*asked.lock().expect("uncontended"));
}
