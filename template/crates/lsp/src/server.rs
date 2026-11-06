//! The handlers: what the server does with each message.

use std::sync::Arc;

use lspkit_runtime::{
    protocol::{Client, LanguageServer, jsonrpc, ls_types::*},
    quiet,
};
use lspkit_text::{Encoding, LineIndex};
use tokio::sync::Mutex;

use crate::documents::{Document, Documents, Refused};

#[derive(Debug)]
struct State {
    documents: Documents,
    /// The unit the client counts columns in, negotiated at `initialize`.
    encoding: Encoding,
}

#[derive(Debug)]
pub(crate) struct Backend {
    client: Client,
    state: Arc<Mutex<State>>,
}

impl Backend {
    pub(crate) fn new(client: Client) -> Backend {
        Backend {
            client,
            state: Arc::new(Mutex::new(State {
                documents: Documents::default(),
                // The protocol's default, until `initialize` negotiates another.
                encoding: Encoding::Utf16,
            })),
        }
    }

    /// The diagnostics for the open document at `uri`, with the version they describe.
    async fn diagnostics(state: &Mutex<State>, uri: &Uri) -> Option<(Vec<Diagnostic>, i32)> {
        let state = state.lock().await;
        let document = state.documents.get(uri)?;
        let index = LineIndex::new(&document.text);
        let found = kitserver_check::check(&document.text)
            .into_iter()
            .filter_map(|finding| {
                Some(Diagnostic {
                    range: lspkit_text::lsp::range(
                        &index,
                        &document.text,
                        finding.range,
                        state.encoding,
                    )?,
                    severity: Some(DiagnosticSeverity::WARNING),
                    code: Some(NumberOrString::String(finding.code.to_owned())),
                    source: Some(crate::NAME.to_owned()),
                    message: finding.message,
                    ..Diagnostic::default()
                })
            })
            .collect();
        Some((found, document.version))
    }

    async fn publish(&self, uri: &Uri) {
        if let Some((found, version)) = Self::diagnostics(&self.state, uri).await {
            self.client
                .publish_diagnostics(uri.clone(), found, Some(version))
                .await;
        }
    }
}

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        let (encoding, kind) = lspkit_text::lsp::negotiate(&params.capabilities);
        self.state.lock().await.encoding = encoding;
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                position_encoding: Some(kind),
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        ..TextDocumentSyncOptions::default()
                    },
                )),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: crate::NAME.to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
            offset_encoding: None,
        })
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }

    /// Publishes at once, since nobody is typing yet and the user is waiting for the first result.
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let document = params.text_document;
        self.state.lock().await.documents.open(
            document.uri.clone(),
            Document {
                text: document.text,
                version: document.version,
            },
        );
        self.publish(&document.uri).await;
    }

    /// Publishes once typing pauses, and only if this is still the latest version.
    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let (uri, version) = (params.text_document.uri, params.text_document.version);
        let refused = {
            let mut state = self.state.lock().await;
            let encoding = state.encoding;
            state
                .documents
                .change(&uri, version, params.content_changes, encoding)
        };
        match refused {
            Ok(()) => {
                let (asked, held, client) = (
                    Arc::clone(&self.state),
                    Arc::clone(&self.state),
                    self.client.clone(),
                );
                let (named, current) = (uri.clone(), uri.clone());
                quiet::when_quiet(
                    self.client.clone(),
                    move || format!("checking {}", named.as_str()),
                    version,
                    move || async move {
                        let state = asked.lock().await;
                        state
                            .documents
                            .get(&current)
                            .map(|document| document.version)
                    },
                    move || async move {
                        if let Some((found, version)) = Self::diagnostics(&held, &uri).await {
                            client.publish_diagnostics(uri, found, Some(version)).await;
                        }
                    },
                );
            }
            Err(Refused::NotOpen) => {}
            Err(why) => {
                // Logged, because every later answer would be about the wrong text.
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("ignored a change to {}: {why:?}", uri.as_str()),
                    )
                    .await;
            }
        }
    }

    /// Clears the closed document's diagnostics, since the client keeps the last ones it was sent.
    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.state.lock().await.documents.close(&uri);
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }
}
