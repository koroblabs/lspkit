use super::editor::Editor;
use crate::panics;

/// A panic in a spawned task reaches the client's log, described by what the task was doing.
#[tokio::test]
async fn a_panic_in_a_spawned_task_is_reported_not_lost() {
    let editor = Editor::new(true).await;
    let task = panics::spawn(
        editor.client.clone(),
        || "checking the project".to_owned(),
        async { panic!("boom") },
    );
    task.await.expect("the task itself ends normally");
    editor.settle().await;

    let received = editor.received();
    // The server's name comes from whichever test installed the hook first, so it is not pinned.
    assert!(
        received.contains("failed while checking the project: boom"),
        "{received}"
    );
}
