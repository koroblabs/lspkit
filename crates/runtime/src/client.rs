//! Server-to-client requests that cannot stall the server, and readers for the client's
//! capabilities.
//!
//! A server awaits each request it sends, and a client may ignore one. In phrost, three startup
//! requests (create a progress token, watch files, offer the type hierarchy) were awaited before
//! indexing. A client that ignored them left the process idle at 18 MB, answering "still indexing"
//! for four minutes, with nothing timed out or logged. A request awaited in a notification handler
//! blocks the whole server, because notifications are handled in order.

use std::{future::Future, path::PathBuf, time::Duration};

use ls_types::{
    ClientCapabilities, DidChangeWatchedFilesRegistrationOptions, FileSystemWatcher,
    InitializeParams, MessageType, Registration,
};

use crate::{jsonrpc, server::Client};

/// How long a server-to-client request waits for an answer before the server gives up on it.
///
/// Everything a server asks of a client is optional (a progress bar, a file watch, a dynamic
/// capability), so the server can go on without it. Five seconds is enough for a busy client.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// Sends `request` and waits at most [`REPLY_TIMEOUT`] for the answer.
///
/// Returns `true` if the client accepted. A refusal and a timeout are logged the same way, since
/// both mean the feature named by `what` ("show an indexing bar") is unavailable this session.
pub async fn ask(
    client: &Client,
    what: &str,
    request: impl Future<Output = jsonrpc::Result<()>>,
) -> bool {
    let reason = match tokio::time::timeout(REPLY_TIMEOUT, request).await {
        Ok(Ok(())) => return true,
        Ok(Err(error)) => format!("the client refused: {error}"),
        Err(_) => format!(
            "the client did not answer within {} seconds",
            REPLY_TIMEOUT.as_secs()
        ),
    };
    client
        .log_message(
            MessageType::WARNING,
            format!("could not {what} -- {reason}"),
        )
        .await;
    false
}

/// Asks the client, through [`ask`], to report changes on disk to files matching `watchers`. The
/// registration uses `id`, so making it twice replaces the first.
pub async fn watch_files(
    client: &Client,
    id: &str,
    what: &str,
    watchers: Vec<FileSystemWatcher>,
) -> bool {
    let Ok(options) = serde_json::to_value(DidChangeWatchedFilesRegistrationOptions { watchers })
    else {
        return false;
    };
    let registration = Registration {
        id: id.to_owned(),
        method: "workspace/didChangeWatchedFiles".to_owned(),
        register_options: Some(options),
    };
    ask(client, what, client.register_capability(vec![registration])).await
}

/// Whether the client supports dynamic file watching ([`watch_files`]). Do not ask one that does
/// not: it would refuse, and log a warning nobody can act on.
pub fn can_watch_files(capabilities: &ClientCapabilities) -> bool {
    capabilities
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.did_change_watched_files.as_ref())
        .and_then(|watched| watched.dynamic_registration)
        .unwrap_or(false)
}

/// The workspace folders, or the single root an older client sends instead. Empty when the client
/// opened a single file.
#[allow(deprecated)]
pub fn roots(params: &InitializeParams) -> Vec<PathBuf> {
    let folders: Vec<PathBuf> = params
        .workspace_folders
        .iter()
        .flatten()
        .filter_map(|folder| folder.uri.to_file_path().map(|path| path.to_path_buf()))
        .collect();
    if !folders.is_empty() {
        return folders;
    }
    params
        .root_uri
        .as_ref()
        .and_then(|uri| uri.to_file_path())
        .map(|path| vec![path.to_path_buf()])
        .unwrap_or_default()
}
