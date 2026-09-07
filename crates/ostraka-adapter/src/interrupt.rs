//! A request to stop, from outside the run.
//!
//! An operator pressing Ctrl-C wants the agent to stop, not the terminal to be
//! abandoned with a subprocess still writing into a worktree. The signal
//! arrives on another thread, so what it can do is set a flag; the loops that
//! wait on a vendor check it and kill the child the same way they would at a
//! deadline.
//!
//! Global on purpose. A signal is global — there is one process and one Ctrl-C,
//! and threading a handle down to whichever subprocess happens to be running
//! would be ceremony around a fact that is already true.

use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

/// Asks every waiting adapter to stop. Safe to call from a signal handler.
pub fn request() {
    REQUESTED.store(true, Ordering::SeqCst);
}

pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

/// Clears the request. For tests, which share one process.
pub fn clear() {
    REQUESTED.store(false, Ordering::SeqCst);
}
