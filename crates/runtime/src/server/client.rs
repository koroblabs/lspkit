//! Messages from the server to the client: notifications, and requests that wait for an answer.

use std::{
    collections::HashMap,
    fmt::{self, Display},
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
};

use ls_types::{
    ApplyWorkspaceEditParams, ApplyWorkspaceEditResponse, ConfigurationItem, ConfigurationParams,
    Diagnostic, LogMessageParams, MessageType, ProgressToken, PublishDiagnosticsParams,
    Registration, RegistrationParams, ShowMessageParams, Unregistration, UnregistrationParams, Uri,
    WorkDoneProgressCreateParams, WorkspaceEdit,
    notification::{self, Notification},
    request,
};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use super::state::State;
use crate::jsonrpc::{Error, Id, Params, Request, Response, Result};

#[cfg(test)]
mod tests;

/// The server's handle to its client. Clones are cheap and all refer to the same client.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    /// Bounded, so a client that stops reading blocks the senders instead of growing this without
    /// limit. The transport's byte queue behind it is bounded too.
    outgoing: mpsc::Sender<Request>,
    waiting: Arc<Waiting>,
    next_id: AtomicI64,
    state: Arc<State>,
}

/// The server's requests awaiting the client's answer, by id. The [`Client`] that sends a request
/// registers it, and the socket's sink settles it when the answer arrives.
///
/// It is separate from [`Client`] so the sink can hold only this. A sink holding a whole client
/// kept the sender of the channel the socket's stream reads, so the stream never ended after the
/// server and its clients were dropped. In phrost's flow tests that leaked 6.5 KB per server a
/// test built (channel, client and recorded traffic).
#[derive(Default)]
pub(crate) struct Waiting(Mutex<Pending>);

#[derive(Default)]
struct Pending {
    by_id: HashMap<Id, oneshot::Sender<Response>>,
    /// Nothing more will be read from the client, so nothing will be answered.
    closed: bool,
}

impl Waiting {
    fn pending(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Registers `answer` for `id`'s response. If nothing more will be read, `answer` is dropped at
    /// once, so the request fails instead of waiting forever.
    fn wait(&self, id: Id, answer: oneshot::Sender<Response>) {
        let mut pending = self.pending();
        if !pending.closed {
            pending.by_id.insert(id, answer);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.pending().by_id.len()
    }

    /// Stops waiting for `id`: the request gave up, or was never sent.
    fn forget(&self, id: &Id) {
        self.pending().by_id.remove(id);
    }

    /// Marks that nothing more will be read from the client. Every waiting request fails, and so
    /// does every later one. Without this, a notification handler awaiting a raw `send_request`
    /// after the client left held the loop, which reads nothing while a notification runs, and the
    /// process never exited.
    pub(crate) fn close(&self) {
        let mut pending = self.pending();
        pending.closed = true;
        pending.by_id.clear();
    }

    /// Delivers the client's answer to the waiting request. An answer nobody waits for (a
    /// duplicate, or one to a request that gave up) is dropped.
    pub(crate) fn answered(&self, response: Response) {
        let waiting = self.pending().by_id.remove(response.id());
        if let Some(waiting) = waiting {
            let _ = waiting.send(response);
        }
    }
}

/// Stops waiting for `id` when dropped.
struct Forget<'a> {
    waiting: &'a Waiting,
    id: Id,
}

impl Drop for Forget<'_> {
    fn drop(&mut self) {
        self.waiting.forget(&self.id);
    }
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client").finish_non_exhaustive()
    }
}

impl Client {
    pub(crate) fn new(state: Arc<State>) -> (Client, mpsc::Receiver<Request>) {
        let (outgoing, received) = mpsc::channel(64);
        let client = Client {
            inner: Arc::new(Inner {
                outgoing,
                waiting: Arc::default(),
                next_id: AtomicI64::new(0),
                state,
            }),
        };
        (client, received)
    }

    /// The table that settles answers to this client's requests, held by the socket's sink.
    pub(crate) fn waiting(&self) -> Arc<Waiting> {
        Arc::clone(&self.inner.waiting)
    }

    /// Sends a notification. Until `initialize` is answered this sends nothing, as the protocol
    /// requires; only `log_message` and `show_message` go out before then.
    pub async fn send_notification<N: Notification>(&self, params: N::Params) {
        if !self.inner.state.speaks() {
            return;
        }
        self.notify::<N>(params).await;
    }

    /// Sends a request and waits for the answer. Bound the wait (`client::ask`): a client may
    /// ignore a request, and an unbounded wait can stall the server.
    pub async fn send_request<R: request::Request>(&self, params: R::Params) -> Result<R::Result> {
        if !self.inner.state.speaks() {
            return Err(Error::not_initialized());
        }
        let params =
            serde_json::value::to_raw_value(&params).map_err(|_| Error::internal_error())?;
        let id = Id::Number(self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let (answer, answered) = oneshot::channel();
        self.inner.waiting.wait(id.clone(), answer);
        // Remove the id however this ends: answered, refused, or dropped by `client::ask`'s timeout
        // or a cancelled request. Otherwise every unanswered request kept its entry and channel for
        // the rest of the session.
        let _waiting = Forget {
            waiting: &self.inner.waiting,
            id: id.clone(),
        };
        let request = Request::from_parts(R::METHOD.into(), Some(id), Some(Params::own(params)));
        if self.inner.outgoing.send(request).await.is_err() {
            return Err(Error::internal_error());
        }
        let response = answered.await.map_err(|_| Error::internal_error())?;
        let result = response.into_raw()?;
        serde_json::from_str(result.get()).map_err(|err| Error::invalid_params(err.to_string()))
    }

    /// Serializes the typed parameters once, straight to the JSON that is written. Going through a
    /// `Value` built every string of every diagnostic twice, on the most frequent message a server
    /// sends.
    async fn notify<N: Notification>(&self, params: N::Params) {
        let Ok(params) = serde_json::value::to_raw_value(&params) else {
            return;
        };
        let request = Request::from_parts(N::METHOD.into(), None, Some(Params::own(params)));
        // A closed channel means the session has ended, so there is nobody to tell.
        let _ = self.inner.outgoing.send(request).await;
    }

    pub async fn log_message(&self, typ: MessageType, message: impl Display) {
        let message = message.to_string();
        self.notify::<notification::LogMessage>(LogMessageParams { typ, message })
            .await;
    }

    pub async fn show_message(&self, typ: MessageType, message: impl Display) {
        let message = message.to_string();
        self.notify::<notification::ShowMessage>(ShowMessageParams { typ, message })
            .await;
    }

    pub async fn publish_diagnostics(
        &self,
        uri: Uri,
        diagnostics: Vec<Diagnostic>,
        version: Option<i32>,
    ) {
        self.send_notification::<notification::PublishDiagnostics>(PublishDiagnosticsParams {
            uri,
            diagnostics,
            version,
        })
        .await;
    }

    pub async fn register_capability(&self, registrations: Vec<Registration>) -> Result<()> {
        self.send_request::<request::RegisterCapability>(RegistrationParams { registrations })
            .await
    }

    pub async fn unregister_capability(&self, unregisterations: Vec<Unregistration>) -> Result<()> {
        self.send_request::<request::UnregisterCapability>(UnregistrationParams {
            unregisterations,
        })
        .await
    }

    pub async fn create_work_done_progress(&self, token: ProgressToken) -> Result<()> {
        self.send_request::<request::WorkDoneProgressCreate>(WorkDoneProgressCreateParams { token })
            .await
    }

    pub async fn code_lens_refresh(&self) -> Result<()> {
        self.send_request::<request::CodeLensRefresh>(()).await
    }

    pub async fn semantic_tokens_refresh(&self) -> Result<()> {
        self.send_request::<request::SemanticTokensRefresh>(())
            .await
    }

    pub async fn inlay_hint_refresh(&self) -> Result<()> {
        self.send_request::<request::InlayHintRefreshRequest>(())
            .await
    }

    pub async fn workspace_diagnostic_refresh(&self) -> Result<()> {
        self.send_request::<request::WorkspaceDiagnosticRefresh>(())
            .await
    }

    /// Asks the client to apply `edit`.
    ///
    /// The answer is delivered as soon as it is read, and may overtake notifications the client
    /// sent before it. The `didChange` that the edit causes may not be handled yet when this
    /// returns, so do not assume the document is updated. Keeping the order would deadlock a
    /// notification handler that awaits the client, since the answer would queue behind it.
    pub async fn apply_edit(&self, edit: WorkspaceEdit) -> Result<ApplyWorkspaceEditResponse> {
        self.send_request::<request::ApplyWorkspaceEdit>(ApplyWorkspaceEditParams {
            edit,
            label: None,
        })
        .await
    }

    pub async fn configuration(&self, items: Vec<ConfigurationItem>) -> Result<Vec<Value>> {
        self.send_request::<request::WorkspaceConfiguration>(ConfigurationParams { items })
            .await
    }
}
