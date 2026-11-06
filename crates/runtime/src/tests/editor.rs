use std::sync::{Arc, Mutex};

use crate::protocol::{
    Client, LanguageServer, LspService,
    jsonrpc::{Request, Response, Result},
    ls_types::{InitializeParams, InitializeResult},
};
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::Value;

/// A server with nothing in it but its client.
struct Bare {
    client: Client,
}

impl LanguageServer for Bare {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult::default())
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

/// An initialized server's client, and what reached the editor through it.
pub(super) struct Editor {
    _service: LspService<Bare>,
    pub(super) client: Client,
    seen: Arc<Mutex<Vec<Request>>>,
}

impl Editor {
    /// An editor that answers every request with `null`, or, if `answering` is false, ignores
    /// them all, as a client may.
    pub(super) async fn new(answering: bool) -> Editor {
        let (service, socket) = LspService::new(|client| Bare { client });
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        tokio::spawn(async move {
            let (mut requests, mut responses) = socket.split();
            while let Some(request) = requests.next().await {
                let id = request.id().cloned();
                record.lock().expect("uncontended").push(request);
                if answering
                    && let Some(id) = id
                    && responses
                        .send(Response::from_ok(id, Value::Null))
                        .await
                        .is_err()
                {
                    break;
                }
            }
        });
        // A client sends nothing until the server has answered `initialize`.
        let initialize = Request::build("initialize")
            .id(1)
            .params(serde_json::json!({ "capabilities": {} }))
            .finish();
        service.call(initialize).await.expect("the service answers");
        let client = service.inner().client.clone();
        Editor {
            _service: service,
            client,
            seen,
        }
    }

    /// Lets the editor's task read what was sent.
    pub(super) async fn settle(&self) {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }

    /// Every message the editor received, one line each: the method, then the parameters.
    pub(super) fn received(&self) -> String {
        self.seen
            .lock()
            .expect("uncontended")
            .iter()
            .map(|request| {
                let params = request
                    .params()
                    .map(|params| params.to_string())
                    .unwrap_or_default();
                format!("{} {params}\n", request.method())
            })
            .collect()
    }
}
