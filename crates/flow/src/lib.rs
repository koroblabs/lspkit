//! Drives a language server in process, the way a flow test does.
//!
//! Flow tests cover bugs that only show up in a running server: version guards, readiness, and
//! what an edit does to the next request. They depend on the order of messages and the state
//! carried between them, so lower-level tests cannot see them. A server that works only sometimes
//! usually has a sequencing bug rather than a wrong answer.
//!
//! The service is driven directly rather than over a pipe, so a test is deterministic. A task
//! stands in for the client: it reads everything the server sends, and [`Flow`] answers questions
//! about it. The consumer wraps a [`Flow`] in a fixture of its own, which knows what its project
//! is, how its server is built and when startup is over.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_util::{SinkExt as _, StreamExt as _, future::BoxFuture};
use lspkit_runtime::protocol::{
    ClientSocket, LanguageServer, LspService,
    jsonrpc::{Request, Response},
};
use serde_json::Value;

#[cfg(test)]
mod tests;

/// What the stand-in client does with a request the server sends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    /// Reads every request and answers it with `null`, as an editor answers requests a test does
    /// not care about. The channel holds one message, so a server that sends two with nobody
    /// reading deadlocks. The server awaits its requests, so an unanswered
    /// `client/registerCapability` hangs `initialized` forever.
    Answering,
    /// Records every request and answers none, as a client may for requests it did not ask for.
    /// This found a startup hang in phrost that [`Client::Answering`] had hidden. Requests are
    /// still read, because a client that stops reading is a different failure.
    Silent,
}

/// A server, a task reading what it sends the client, and the requests' next id.
pub struct Flow<S: LanguageServer> {
    service: LspService<S>,
    /// Server-to-client traffic, in order.
    seen: Arc<Mutex<Vec<Request>>>,
    next_id: i64,
    /// How many turns [`Flow::settle`] gives the other tasks.
    turns: usize,
}

impl<S: LanguageServer> std::fmt::Debug for Flow<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Flow")
            .field("next_id", &self.next_id)
            .finish_non_exhaustive()
    }
}

impl<S: LanguageServer> Flow<S> {
    /// A server built by `build`, with `client` standing in for the editor. Call it inside a tokio
    /// runtime, because the client is spawned as a task on it.
    pub fn new(
        build: impl FnOnce(lspkit_runtime::protocol::Client) -> S,
        client: Client,
    ) -> Flow<S> {
        let (service, socket) = LspService::new(build);
        let seen = Arc::new(Mutex::new(Vec::new()));
        tokio::spawn(read(socket, Arc::clone(&seen), client));
        Flow {
            service,
            seen,
            next_id: 0,
            turns: 1,
        }
    }

    /// Lets [`Flow::settle`] give the other tasks `turns` turns instead of one, for a server whose
    /// handlers leave work to tasks of their own.
    pub fn settling_for(mut self, turns: usize) -> Flow<S> {
        self.turns = turns.max(1);
        self
    }

    /// The server itself, for what a consumer's fixture asks it directly, such as whether startup
    /// is over.
    pub fn server(&self) -> &S {
        self.service.inner()
    }

    /// Sends a request and waits for its response.
    pub async fn request(&mut self, method: &'static str, params: Value) -> Response {
        let (_, pending) = self.start(method, params).await;
        let response = pending.await;
        self.settle().await;
        response
    }

    /// Sends a request and returns its id and its unfinished future, for a test that acts while the
    /// request is in flight. `call` registers the id before the future is first polled, so a cancel
    /// sent immediately still finds the request.
    pub async fn start(
        &mut self,
        method: &'static str,
        params: Value,
    ) -> (i64, BoxFuture<'static, Response>) {
        self.next_id += 1;
        let id = self.next_id;
        let request = Request::build(method).id(id).params(params).finish();
        let pending = self.service.call(request);
        (
            id,
            Box::pin(async move { pending.await.expect("a request gets a response") }),
        )
    }

    /// Sends a notification, which has no response.
    pub async fn notify(&mut self, method: &'static str, params: Value) {
        let request = Request::build(method).params(params).finish();
        let response = self.service.call(request).await;
        assert!(response.is_none(), "a notification has no response");
        self.settle().await;
    }

    /// Lets the client's task, and any task the server spawned, catch up before a test inspects
    /// what was sent. It yields instead of sleeping, since they all run on the same runtime.
    pub async fn settle(&self) {
        for _ in 0..self.turns {
            tokio::task::yield_now().await;
        }
    }

    /// Advances a paused clock by `by` and lets whatever was waiting on it run, such as a debounced
    /// publish. The test must run with `start_paused`. A real sleep would slow the suite by the
    /// delay being tested.
    pub async fn after(&self, by: Duration) {
        tokio::time::advance(by).await;
        for _ in 0..self.turns.max(8) {
            tokio::task::yield_now().await;
        }
    }

    /// Every method the server sent the client, in order.
    pub fn methods(&self) -> Vec<String> {
        self.with_seen(|seen| seen.iter().map(|r| r.method().to_owned()).collect())
    }

    /// The parameters of every message the server sent with `method`, in order.
    pub fn params(&self, method: &str) -> Vec<Value> {
        self.with_seen(|seen| {
            seen.iter()
                .filter(|request| request.method() == method)
                .filter_map(|request| serde_json::to_value(request.params()?).ok())
                .collect()
        })
    }

    /// Every set of diagnostics published for `uri`, oldest first.
    pub fn publications(&self, uri: &str) -> Vec<Vec<Value>> {
        self.params("textDocument/publishDiagnostics")
            .into_iter()
            .filter(|params| params["uri"] == uri)
            .map(|params| {
                params["diagnostics"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Every message the server logged at `severity`, in order: `MessageType::ERROR` is 1,
    /// `WARNING` 2, `INFO` 3, `LOG` 4.
    pub fn logged(&self, severity: i64) -> Vec<String> {
        self.params("window/logMessage")
            .into_iter()
            .filter(|params| params["type"] == severity)
            .filter_map(|params| Some(params["message"].as_str()?.to_owned()))
            .collect()
    }

    /// Drops the traffic recorded so far, keeping its capacity.
    ///
    /// The record grows with every message, so a leak test that repeats the same steps would
    /// otherwise measure the stand-in client instead of the server. Keeping the capacity means a
    /// cycle that sends as many messages as the previous one allocates nothing here.
    pub fn forget(&self) {
        self.seen
            .lock()
            .expect("the client task does not panic")
            .clear();
    }

    fn with_seen<T>(&self, f: impl FnOnce(&[Request]) -> T) -> T {
        f(&self.seen.lock().expect("the client task does not panic"))
    }
}

/// The stand-in client: reads everything the server sends, and answers it or not.
async fn read(socket: ClientSocket, seen: Arc<Mutex<Vec<Request>>>, client: Client) {
    let (mut requests, mut responses) = socket.split();
    while let Some(request) = requests.next().await {
        let id = request.id().cloned();
        seen.lock()
            .expect("no other panic holds the lock")
            .push(request);
        if client == Client::Answering
            && let Some(id) = id
            && responses
                .send(Response::from_ok(id, Value::Null))
                .await
                .is_err()
        {
            break;
        }
    }
}

// Compiles and runs the README's examples as doctests, so they stay in step with the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
