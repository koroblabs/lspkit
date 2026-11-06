use expect_test::expect;
use lspkit_runtime::protocol::{
    Client as LspClient, LanguageServer, jsonrpc,
    ls_types::{
        Diagnostic, DidOpenTextDocumentParams, InitializeParams, InitializeResult,
        InitializedParams, MessageType, Range, Registration,
    },
};
use serde_json::json;

use super::{Client, Flow};

/// A server that exercises everything a flow asks about: it logs, publishes, and asks the client
/// for a registration without waiting on it.
struct Echo {
    client: LspClient,
}

impl LanguageServer for Echo {
    async fn initialize(&self, _: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        Ok(InitializeResult::default())
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client.log_message(MessageType::INFO, "ready").await;
        let client = self.client.clone();
        tokio::spawn(async move {
            let registration = Registration {
                id: "watch".to_owned(),
                method: "workspace/didChangeWatchedFiles".to_owned(),
                register_options: None,
            };
            let answered = client.register_capability(vec![registration]).await;
            client
                .log_message(
                    MessageType::WARNING,
                    format!("registered: {}", answered.is_ok()),
                )
                .await;
        });
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let diagnostic = Diagnostic {
            range: Range::default(),
            message: params.text_document.text,
            ..Diagnostic::default()
        };
        self.client
            .publish_diagnostics(params.text_document.uri, vec![diagnostic], None)
            .await;
    }
}

async fn started(client: Client) -> Flow<Echo> {
    let mut flow = Flow::new(|client| Echo { client }, client).settling_for(8);
    let answer = flow
        .request("initialize", json!({ "capabilities": {} }))
        .await;
    assert!(answer.is_ok(), "{answer:?}");
    flow.notify("initialized", json!({})).await;
    flow
}

#[tokio::test]
async fn an_answering_client_records_and_answers() {
    let mut flow = started(Client::Answering).await;
    flow.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": "file:///a.txt", "languageId": "", "version": 1, "text": "a finding" } }),
    )
    .await;
    let published = flow.publications("file:///a.txt");
    let publications: Vec<Vec<&str>> = published
        .iter()
        .map(|found| found.iter().filter_map(|d| d["message"].as_str()).collect())
        .collect();
    expect![[r#"
        ["window/logMessage", "client/registerCapability", "window/logMessage", "textDocument/publishDiagnostics"]
        ["ready"]
        ["registered: true"]
        [["a finding"]]
    "#]].assert_eq(&format!(
        "{:?}\n{:?}\n{:?}\n{:?}\n",
        flow.methods(),
        flow.logged(3),
        flow.logged(2),
        publications
    ));
}

#[tokio::test]
async fn a_silent_client_records_and_answers_nothing() {
    let flow = started(Client::Silent).await;
    assert_eq!(
        flow.methods(),
        ["window/logMessage", "client/registerCapability"]
    );
    assert_eq!(
        flow.logged(2),
        Vec::<String>::new(),
        "still waiting on the client"
    );
}

#[tokio::test]
async fn forgetting_drops_what_was_recorded_and_records_what_follows() {
    let mut flow = started(Client::Answering).await;
    flow.forget();
    assert_eq!(flow.methods(), Vec::<String>::new());
    flow.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": "file:///a.txt", "languageId": "", "version": 1, "text": "after" } }),
    )
    .await;
    assert_eq!(flow.methods(), ["textDocument/publishDiagnostics"]);
}

#[tokio::test]
async fn a_started_request_can_be_held_in_flight() {
    let mut flow = Flow::new(|client| Echo { client }, Client::Answering);
    let (id, pending) = flow
        .start("initialize", json!({ "capabilities": {} }))
        .await;
    assert_eq!(id, 1);
    let response = pending.await;
    let (_, result) = response.into_parts();
    assert_eq!(result.ok(), Some(json!({ "capabilities": {} })));
}
