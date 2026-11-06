//! Architecture checks for a Cargo workspace, run as tests.
//!
//! A consumer states its rules as data in a test of its own, and this crate checks the workspace
//! against them. Rules kept only in documentation get broken: phrost's predecessor documented
//! that its analysis crate must not contain language semantics, and that crate grew to 28 520
//! lines that reached into parser internals, because nothing failed when someone added a
//! convenient `use`.
//!
//! - [`edges`]: which crates may depend on which, and which crate may name each foreign engine.
//! - [`reads`]: every filesystem read on a request's path, each with a written reason. In a
//!   language server, a fallback is usually a read.
//! - [`baseline`]: settings every server keeps (unwinding panics, forbidden `unsafe`, a committed
//!   lockfile). Each is a one-line change that no other test would catch.
//!
//! The rules belong to the consumer. Reading, matching and reporting live here, so a fix to how a
//! manifest is read reaches every consumer at once.

pub mod baseline;
pub mod edges;
pub mod reads;
pub mod workspace;

#[cfg(test)]
mod tests;

// Compiles and runs the README's examples as doctests, so they stay in step with the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
