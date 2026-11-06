//! The language server: it tracks open documents and publishes their findings.
//!
//! The findings come from `kitserver-check`. The protocol, panic containment, the quiet publish and
//! requests to the client come from `lspkit_runtime`. This crate holds the open documents and
//! converts byte ranges to the client's positions.

mod documents;
mod server;

#[cfg(test)]
mod tests;

use self::server::Backend;

/// The server's name in the messages sent on its behalf.
const NAME: &str = "kitserver";

/// Serves the protocol over stdin and stdout until the client exits, and returns the process exit
/// code: 0 after `shutdown` then `exit`, 1 for an `exit` without `shutdown`.
pub async fn serve() -> std::process::ExitCode {
    lspkit_runtime::serve_stdio(NAME, Backend::new)
        .await
        .exit_code()
}
