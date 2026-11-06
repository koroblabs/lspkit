use crate::protocol::ls_types::{ClientCapabilities, WindowClientCapabilities};
use expect_test::expect;

use super::editor::Editor;
use crate::progress::Progress;

fn drawing_bars() -> ClientCapabilities {
    ClientCapabilities {
        window: Some(WindowClientCapabilities {
            work_done_progress: Some(true),
            ..WindowClientCapabilities::default()
        }),
        ..ClientCapabilities::default()
    }
}

#[tokio::test]
async fn the_token_is_created_before_anything_is_reported_and_only_moves_are_sent() {
    let editor = Editor::new(true).await;
    let progress = Progress::new();
    progress.negotiate(&drawing_bars());

    let mut bar = progress
        .start(&editor.client, "kit/index", "Indexing", 3, "files")
        .await;
    assert!(bar.is_shown());
    bar.reached(1).await;
    bar.reached(1).await;
    bar.reached(3).await;
    bar.finish("3 files indexed").await;
    editor.settle().await;

    expect![[r#"
        window/workDoneProgress/create {"token":"kit/index"}
        $/progress {"token":"kit/index","value":{"kind":"begin","title":"Indexing","cancellable":false,"message":"3 files","percentage":0}}
        $/progress {"token":"kit/index","value":{"kind":"report","message":"1 of 3 files","percentage":33}}
        $/progress {"token":"kit/index","value":{"kind":"report","message":"3 of 3 files","percentage":100}}
        $/progress {"token":"kit/index","value":{"kind":"end","message":"3 files indexed"}}
    "#]]
    .assert_eq(&editor.received());
}

#[tokio::test(start_paused = true)]
async fn a_client_that_ignores_the_request_is_asked_once_a_session() {
    let editor = Editor::new(false).await;
    let progress = Progress::new();
    progress.negotiate(&drawing_bars());

    let mut first = progress
        .start(&editor.client, "kit/index", "Indexing", 3, "files")
        .await;
    assert!(!first.is_shown());
    first.reached(3).await;
    first.finish("done").await;

    let second = progress
        .start(&editor.client, "kit/check", "Checking", 3, "files")
        .await;
    assert!(!second.is_shown());
    editor.settle().await;

    expect![[r#"
        window/workDoneProgress/create {"token":"kit/index"}
        window/logMessage {"type":2,"message":"could not show a progress bar for Indexing -- the client did not answer within 5 seconds"}
    "#]]
    .assert_eq(&editor.received());
}

#[tokio::test]
async fn a_client_that_draws_no_bars_is_not_asked_for_one() {
    let editor = Editor::new(true).await;
    let progress = Progress::new();
    progress.negotiate(&ClientCapabilities::default());

    let bar = progress
        .start(&editor.client, "kit/index", "Indexing", 3, "files")
        .await;
    assert!(!bar.is_shown());
    editor.settle().await;
    assert_eq!(editor.received(), "");
}
