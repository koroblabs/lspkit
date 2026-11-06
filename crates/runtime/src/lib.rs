//! The language-independent parts of a language server.
//!
//! - [`protocol`]: the protocol types, the server trait and the message loop. The loop runs a task
//!   per request, stops one on `$/cancelRequest` ([`cancel`]), handles notifications in order, and
//!   answers a panic with an error.
//! - [`serve_stdio`]: runs a server over stdin and stdout, with [`panics`] and [`transport`] in
//!   place.
//! - [`panics`]: a panicking handler answers its request with an error and the session goes on. A
//!   write that unwinds poisons what it was writing, and a panic in a spawned task is reported.
//! - [`transport`]: queues output, so a slow client grows the queue and does not block the server.
//! - [`client`]: server-to-client requests that give up on a silent client, and readers for the
//!   client's capabilities.
//! - [`progress`]: progress bars, created the way the protocol and Zed require.
//! - [`quiet`]: publishes a buffer's findings once edits stop.
//! - [`reported`]: tracks which closed files a project-wide pass reported, so the next pass clears
//!   only those.
//!
//! The module docs keep the measurements from phrost and orthra behind each choice. The protocol
//! types are `ls-types`, pinned with `=` here. Servers take them through [`protocol`], so a
//! dependency graph has one copy.

pub mod cancel;
pub mod client;
pub mod jsonrpc;
pub mod panics;
pub mod progress;
pub mod protocol;
pub mod quiet;
pub mod reported;
pub mod transport;

mod codec;
mod server;
mod stdio;

pub use self::stdio::serve_stdio;

#[cfg(test)]
mod tests;

// Runs the README's examples as doctests, so they stay in step with the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

// The workspace README's example too. The path leaves this crate, which works because lspkit is
// used from git as a whole and never packaged per crate.
#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
struct WorkspaceReadmeDoctests;
