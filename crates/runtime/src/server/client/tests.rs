use std::{sync::Arc, time::Duration};

use super::Client;
use crate::server::state::{Stage, State};

/// A request the client never answers is forgotten once the server gives up on it: here by a
/// timeout, as in `client::ask`, or when a cancelled request's task is dropped.
#[tokio::test(start_paused = true)]
async fn a_request_given_up_on_is_no_longer_waited_for() {
    let state = Arc::new(State::new());
    state.set(Stage::Initialized);
    let (client, mut sent) = Client::new(state);

    let asking = client.clone();
    let asked = tokio::spawn(async move {
        tokio::time::timeout(
            Duration::from_secs(5),
            asking.register_capability(Vec::new()),
        )
        .await
    });
    let request = sent.recv().await.expect("the request went out");
    assert_eq!(request.method(), "client/registerCapability");
    assert_eq!(client.inner.waiting.len(), 1, "and is waited for");

    assert!(
        asked.await.expect("no panic").is_err(),
        "the client never answers, so the wait times out"
    );
    assert_eq!(client.inner.waiting.len(), 0, "and is forgotten");
}
