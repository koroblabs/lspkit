//! Drives the server in process as an editor would. This is where the version guard, the quiet
//! publish and an edit's effect on the next answer can be tested.

mod incremental;

use expect_test::expect;
use lspkit_flow::{Client, Flow};
use lspkit_runtime::quiet::QUIET;
use serde_json::json;

use crate::server::Backend;

const URI: &str = "file:///project/a.txt";

async fn started() -> Flow<Backend> {
    let mut flow = Flow::new(Backend::new, Client::Answering);
    flow.request("initialize", json!({ "capabilities": {} }))
        .await;
    flow.notify("initialized", json!({})).await;
    flow
}

#[tokio::test(start_paused = true)]
async fn an_open_is_published_at_once_and_an_edit_once_typing_stops() {
    let mut flow = started().await;
    flow.notify(
        "textDocument/didOpen",
        json!({ "textDocument": {
            "uri": URI, "languageId": "text", "version": 1, "text": "one  \ntwo\n"
        } }),
    )
    .await;
    // Delete the trailing spaces.
    flow.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": URI, "version": 2 },
            "contentChanges": [{
                "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 5 } },
                "text": ""
            }]
        }),
    )
    .await;
    assert_eq!(
        flow.publications(URI).len(),
        1,
        "an edit waits for the quiet"
    );

    flow.after(QUIET).await;
    expect![[r#"
        [[{"code":"trailing-whitespace","message":"whitespace at the end of a line","range":{"end":{"character":5,"line":0},"start":{"character":3,"line":0}},"severity":2,"source":"kitserver"}],[]]"#]]
    .assert_eq(&json!(flow.publications(URI)).to_string());
}
