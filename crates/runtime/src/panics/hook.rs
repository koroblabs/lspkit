//! Records where a panic was raised, which `catch_unwind` does not return.
//!
//! `catch_unwind` returns only the payload. The file and line are passed only to the panic hook,
//! while the panic is raised. Without them, "attempt to subtract with overflow" does not say which
//! of a few hundred subtractions failed.

use std::{
    cell::Cell,
    sync::{Once, OnceLock},
};

/// The server's name, used in every message this crate writes.
static NAME: OnceLock<&'static str> = OnceLock::new();

thread_local! {
    /// Where the last panic on this thread was raised.
    ///
    /// Thread-local because the hook runs on the panicking thread, and the catch that reads the
    /// slot is lower on the same thread's stack. Two threads panicking at once cannot see each
    /// other's location, as they could with a shared slot.
    static LOCATION: Cell<Option<String>> = const { Cell::new(None) };
}

/// Installs a panic hook that records where each panic happened and then calls the previous hook.
///
/// Only the first call has an effect, so a test that needs the location can call it whether or not
/// the server has started. Because the previous hook still runs, stderr keeps the full default
/// report (thread, message, location, `RUST_BACKTRACE`).
///
/// It also sets the server's name for every message this crate writes. The first name wins: one
/// process is one server.
pub fn install(name: &'static str) {
    static INSTALLED: Once = Once::new();

    let _ = NAME.set(name);

    INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            LOCATION.with(|slot| slot.set(info.location().map(ToString::to_string)));
            previous(info);
        }));
    });
}

/// Takes the location of the panic being caught, if the hook was installed to record it.
///
/// Taking clears the slot, so a caught panic never gets the location of an earlier uncaught one.
pub(crate) fn take_location() -> Option<String> {
    LOCATION.with(Cell::take)
}

/// The server's name, as [`install`] was given it.
pub(crate) fn name() -> &'static str {
    NAME.get().copied().unwrap_or("the language server")
}
