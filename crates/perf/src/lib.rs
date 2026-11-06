//! Samples, percentiles, resident memory and budgets for a server's `perf` command.
//!
//! A server's `perf` command measures its own requests on a real project and exits non-zero when
//! one is over its budget. The server decides what to measure and what each budget is. This crate
//! does the arithmetic and gives the verdict, so every server computes a p95 the same way.
//!
//! Percentiles use nearest rank. phrost used nearest rank and orthra rounded an interpolated
//! index, so the same timings gave two different p95s and the servers' budgets could not be
//! compared.
//!
//! - [`Samples`]: timings for one kind of request, how many of them answered, and where the worst
//!   one was.
//! - [`rss`]: the process's resident memory, as the operating system counts it.
//! - [`Budget`] and [`Verdict`]: a ceiling, and whether a measurement is under it.

mod budget;
pub mod rss;
mod samples;

#[cfg(test)]
mod tests;

pub use self::{
    budget::{Budget, Judged, OverBudget, Verdict},
    samples::{Samples, Worst, ms},
};

// Compiles and runs the README's examples as doctests, so they stay in step with the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
