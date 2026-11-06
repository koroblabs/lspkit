//! Dispatches each message from the client: the lifecycle, a task per request, cancellation, and
//! panic handling.
//!
//! It differs from `tower-lsp-server` in three ways:
//!
//! - Every request runs on its own task, with no cap. `tower-lsp-server` polled them all in one
//!   task through `buffer_unordered(4)`, so a handler computing without awaiting held up the loop,
//!   and four slow requests held up everything.
//! - `$/cancelRequest` aborts the request's task at its next `.await`, answers the client with
//!   `RequestCancelled` at once, and sets the request's [`crate::cancel::Token`] for work that does
//!   not await.
//! - A panic is caught in its task and answers the request with an error. It is always logged, and
//!   shown to the user once, in the wording phrost's tests pin.
//!
//! Notifications are not spawned. The transport passes them one at a time and waits for each, so a
//! `didChange` is applied before the next message is read, and a request sent after an edit sees
//! the edited text.

use std::{
    collections::HashMap,
    fmt,
    future::Future,
    sync::{Arc, Mutex},
};

use futures_util::future::BoxFuture;
use ls_types::CancelParams;
use serde::Serialize;
use serde_json::value::RawValue;
use tokio::task::AbortHandle;

use super::{
    client::Client,
    methods::{self, LanguageServer, answer},
    socket::ClientSocket,
    state::{Stage, State},
};
use crate::{
    cancel::Token,
    jsonrpc::{Error, Id, Params, Request, Response, Result},
    panics::{Panic, describe},
};

/// The server, as the transport drives it.
pub struct LspService<S> {
    shared: Arc<Shared<S>>,
}

struct Shared<S> {
    server: Arc<S>,
    client: Client,
    state: Arc<State>,
    custom: HashMap<&'static str, Custom<S>>,
    /// Requests being answered, so a cancel can reach them.
    running: Mutex<HashMap<Id, Running>>,
}

struct Running {
    token: Token,
    abort: AbortHandle,
}

type Custom<S> =
    Arc<dyn Fn(Arc<S>, Option<Params>) -> BoxFuture<'static, Result<Box<RawValue>>> + Send + Sync>;

impl<S> fmt::Debug for LspService<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LspService").finish_non_exhaustive()
    }
}

impl<S: LanguageServer> LspService<S> {
    /// Makes the server with `build` and returns it with the socket the client side reads the
    /// server's messages from.
    pub fn new(build: impl FnOnce(Client) -> S) -> (LspService<S>, ClientSocket) {
        LspService::build(build).finish()
    }

    /// Like `new`, but custom methods can be added before it is finished.
    pub fn build(build: impl FnOnce(Client) -> S) -> LspServiceBuilder<S> {
        let state = Arc::new(State::new());
        let (client, outgoing) = Client::new(Arc::clone(&state));
        LspServiceBuilder {
            server: Arc::new(build(client.clone())),
            client,
            state,
            outgoing,
            custom: HashMap::new(),
        }
    }

    /// The server itself.
    pub fn inner(&self) -> &S {
        &self.shared.server
    }

    /// Handles one message from the client. For a request the future returns the response; for a
    /// notification it returns `None`.
    pub fn call(&self, request: Request) -> BoxFuture<'static, Option<Response>> {
        let shared = Arc::clone(&self.shared);
        match request.id().cloned() {
            Some(id) => shared.request(id, request),
            None => shared.notification(request),
        }
    }
}

impl<S: LanguageServer> Shared<S> {
    fn request(self: Arc<Self>, id: Id, request: Request) -> BoxFuture<'static, Option<Response>> {
        let (method, _, params) = request.into_parts();
        let refused = match (self.state.get(), method.as_ref()) {
            (Stage::Uninitialized, "initialize") => None,
            (Stage::Uninitialized, _) => Some(Error::not_initialized()),
            (_, "initialize") => Some(Error::invalid_request()),
            (Stage::ShutDown | Stage::Exited, _) => Some(Error::invalid_request()),
            _ => None,
        };
        if let Some(refused) = refused {
            return Box::pin(async move { Some(Response::from_error(id, refused)) });
        }

        let answering = match methods::request(Arc::clone(&self.server), &method, params) {
            Ok(answering) => answering,
            Err(params) => match self.custom.get(method.as_ref()) {
                Some(custom) => custom(Arc::clone(&self.server), params),
                None => Box::pin(async { Err(Error::method_not_found()) }),
            },
        };
        let lifecycle = match method.as_ref() {
            "initialize" => {
                self.state.set(Stage::Initializing);
                Some(Stage::Initialized)
            }
            "shutdown" => Some(Stage::ShutDown),
            _ => None,
        };

        // Spawn here, not in the returned future, so the request is running and registered in
        // `running`, where a cancel can find it, before anything polls for its answer.
        let token = Token::new();
        let (shared, task_id) = (Arc::clone(&self), id.clone());
        // Hold the lock from before the task exists until it is in `running`. A request answered at
        // once (method not found) can finish on another worker before this thread inserts it. Its
        // `finished` would then run first, and the entry and abort handle would stay for the rest
        // of the session.
        let mut running = self
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let task = tokio::spawn(token.clone().scope(async move {
            let answered = match Panic::watching(answering).await {
                Ok(answered) => answered,
                Err(panicked) => {
                    let doing = describe(&method, Some(&task_id));
                    panicked.report(&shared.client, &doing).await;
                    Err(panicked.into_error(&doing))
                }
            };
            if let Some(next) = lifecycle {
                match (&answered, next) {
                    (Ok(_), next) => shared.state.set(next),
                    (Err(_), Stage::Initialized) => shared.state.set(Stage::Uninitialized),
                    (Err(_), _) => {}
                }
            }
            shared.finished(&task_id);
            answered
        }));
        running.insert(
            id.clone(),
            Running {
                token,
                abort: task.abort_handle(),
            },
        );
        drop(running);

        Box::pin(async move {
            let answered = match task.await {
                Ok(answered) => answered,
                // Aborted by `$/cancelRequest`, which the protocol answers with `RequestCancelled`.
                Err(stopped) if stopped.is_cancelled() => {
                    self.finished(&id);
                    // An `initialize` aborted before it answered never ran the reset above. Reset
                    // here, so a later `initialize` is accepted instead of refused for good.
                    if lifecycle == Some(Stage::Initialized) {
                        self.state.set(Stage::Uninitialized);
                    }
                    Err(Error::request_cancelled())
                }
                Err(_) => Err(Error::internal_error()),
            };
            Some(match answered {
                Ok(result) => Response::from_raw(id, result),
                Err(error) => Response::from_error(id, error),
            })
        })
    }

    fn notification(self: Arc<Self>, request: Request) -> BoxFuture<'static, Option<Response>> {
        let (method, _, params) = request.into_parts();
        match (self.state.get(), method.as_ref()) {
            (_, "exit") => {
                self.state.set(Stage::Exited);
                return Box::pin(async { None });
            }
            (_, "$/cancelRequest") => {
                if let Ok(cancelled) = methods::parse::<CancelParams>(params) {
                    self.cancel(&cancelled.id.into());
                }
                return Box::pin(async { None });
            }
            // Before `initialize`, and after `shutdown`, a notification other than `exit` is
            // dropped, as the protocol says.
            (Stage::Uninitialized | Stage::ShutDown | Stage::Exited, _) => {
                return Box::pin(async { None });
            }
            _ => {}
        }
        let handling = match methods::notification(Arc::clone(&self.server), &method, params) {
            Some(Ok(handling)) => handling,
            Some(Err(unread)) => {
                let client = self.client.clone();
                return Box::pin(async move {
                    client
                        .log_message(
                            ls_types::MessageType::ERROR,
                            format!("ignored {method}: {unread}"),
                        )
                        .await;
                    None
                });
            }
            // A server may ignore `$/` notifications, and unknown ones too.
            None => return Box::pin(async { None }),
        };
        Box::pin(async move {
            if let Err(panicked) = Panic::watching(handling).await {
                panicked
                    .report(&self.client, &describe(&method, None))
                    .await;
            }
            None
        })
    }

    fn cancel(&self, id: &Id) {
        let running = self
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id);
        if let Some(running) = running {
            running.token.cancel();
            running.abort.abort();
        }
    }

    fn finished(&self, id: &Id) {
        self.running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id);
    }
}

/// See [`LspService::build`].
pub struct LspServiceBuilder<S> {
    server: Arc<S>,
    client: Client,
    state: Arc<State>,
    outgoing: tokio::sync::mpsc::Receiver<Request>,
    custom: HashMap<&'static str, Custom<S>>,
}

impl<S> fmt::Debug for LspServiceBuilder<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LspServiceBuilder").finish_non_exhaustive()
    }
}

impl<S: LanguageServer> LspServiceBuilder<S> {
    /// Handles `name`, a request outside the protocol, with `callback`: `async fn(&self)` or
    /// `async fn(&self, params)`.
    #[must_use]
    pub fn custom_method<P, R, F>(mut self, name: &'static str, callback: F) -> LspServiceBuilder<S>
    where
        P: FromParams,
        R: Serialize + Send + 'static,
        F: for<'a> Method<&'a S, P, R> + Clone + Send + Sync + 'static,
    {
        let custom: Custom<S> = Arc::new(move |server: Arc<S>, params: Option<Params>| {
            let callback = callback.clone();
            answer(async move {
                let params = P::from_params(params)?;
                callback.invoke(&server, params).await
            })
        });
        self.custom.insert(name, custom);
        self
    }

    pub fn finish(self) -> (LspService<S>, ClientSocket) {
        let shared = Arc::new(Shared {
            server: self.server,
            client: self.client.clone(),
            state: self.state,
            custom: self.custom,
            running: Mutex::default(),
        });
        (
            LspService { shared },
            ClientSocket::new(self.outgoing, self.client.waiting()),
        )
    }
}

/// A method outside the protocol: `async fn(&self) -> Result<R>` or
/// `async fn(&self, params) -> Result<R>`.
pub trait Method<S, P, R> {
    type Future: Future<Output = Result<R>> + Send;

    fn invoke(&self, server: S, params: P) -> Self::Future;
}

impl<F, S, R, Fut> Method<S, (), R> for F
where
    F: Fn(S) -> Fut,
    Fut: Future<Output = Result<R>> + Send,
{
    type Future = Fut;

    fn invoke(&self, server: S, (): ()) -> Fut {
        self(server)
    }
}

impl<F, S, P, R, Fut> Method<S, (P,), R> for F
where
    F: Fn(S, P) -> Fut,
    Fut: Future<Output = Result<R>> + Send,
{
    type Future = Fut;

    fn invoke(&self, server: S, (params,): (P,)) -> Fut {
        self(server, params)
    }
}

/// Parses a custom method's parameters from their JSON: none, or one value.
pub trait FromParams: Sized + Send + 'static {
    fn from_params(params: Option<Params>) -> Result<Self>;
}

impl FromParams for () {
    fn from_params(_: Option<Params>) -> Result<()> {
        Ok(())
    }
}

impl<P: serde::de::DeserializeOwned + Send + 'static> FromParams for (P,) {
    fn from_params(params: Option<Params>) -> Result<(P,)> {
        methods::parse(params).map(|params| (params,))
    }
}
