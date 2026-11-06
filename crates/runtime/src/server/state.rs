//! Where a session is in the protocol's lifecycle.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    /// Only `initialize` and `exit` are handled.
    Uninitialized,
    Initializing,
    Initialized,
    /// `shutdown` was answered; only `exit` is handled now.
    ShutDown,
    Exited,
}

#[derive(Debug)]
pub(crate) struct State(AtomicU8);

impl State {
    pub(crate) fn new() -> State {
        State(AtomicU8::new(Stage::Uninitialized as u8))
    }

    pub(crate) fn get(&self) -> Stage {
        match self.0.load(Ordering::Acquire) {
            0 => Stage::Uninitialized,
            1 => Stage::Initializing,
            2 => Stage::Initialized,
            3 => Stage::ShutDown,
            _ => Stage::Exited,
        }
    }

    pub(crate) fn set(&self, stage: Stage) {
        self.0.store(stage as u8, Ordering::Release);
    }

    /// Whether the server may send the client anything but a log or a shown message.
    pub(crate) fn speaks(&self) -> bool {
        matches!(self.get(), Stage::Initialized | Stage::ShutDown)
    }
}
