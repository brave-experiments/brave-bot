//! Asking a turn to stop.
//!
//! A turn can spend a long time waiting on a model, and a user who has changed their mind should
//! not have to wait it out. So a turn carries a token it checks between steps, and setting the
//! token asks it to stop at the next check.
//!
//! Cooperative rather than pre-emptive, and deliberately so. Killing a turn mid-effect could
//! leave a file half written, so cancellation happens only at points where nothing is in
//! progress: between rounds, and before each tool call. A request already on the wire runs to
//! completion, and its result is discarded.
//!
//! The flag only ever goes from unset to set. Reusing a token for a second turn would risk
//! cancelling it before it began, so each turn gets a fresh one.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A one-way flag asking a turn to stop.
///
/// Cheap to clone: every clone refers to the same flag, which is how the interface signals a turn
/// running on another thread.
#[derive(Debug, Clone, Default)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
}

impl Cancel {
    /// A token that has not been cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the turn to stop at its next check.
    ///
    /// Safe to call more than once, and from any thread. `Release` pairs with the `Acquire` in
    /// [`Cancel::is_cancelled`] so a turn that sees the flag also sees everything written before
    /// it was set.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }

    /// Whether a stop has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn a_fresh_token_is_not_cancelled() {
        assert!(!Cancel::new().is_cancelled());
    }

    #[test]
    fn cancelling_is_observable() {
        let cancel = Cancel::new();
        cancel.cancel();
        assert!(cancel.is_cancelled());
    }

    /// The point of the type: one side sets the flag and the other sees it.
    #[test]
    fn a_clone_shares_the_flag() {
        let cancel = Cancel::new();
        let remote = cancel.clone();
        assert!(!remote.is_cancelled());
        cancel.cancel();
        assert!(remote.is_cancelled(), "the clone did not see the request");
    }

    /// Cancellation crosses a thread boundary, which is the only way it is ever used.
    #[test]
    fn cancelling_crosses_threads() {
        let cancel = Cancel::new();
        let worker = cancel.clone();

        let handle = thread::spawn(move || {
            while !worker.is_cancelled() {
                std::hint::spin_loop();
            }
            "stopped"
        });

        cancel.cancel();
        assert_eq!(handle.join().expect("worker finished"), "stopped");
    }

    /// Asking twice is not an error: a user may press the key more than once.
    #[test]
    fn cancelling_twice_is_harmless() {
        let cancel = Cancel::new();
        cancel.cancel();
        cancel.cancel();
        assert!(cancel.is_cancelled());
    }

    /// The flag never clears, so a turn cannot be told to stop and then quietly continue.
    #[test]
    fn cancellation_does_not_reset() {
        let cancel = Cancel::new();
        cancel.cancel();
        for _ in 0..10 {
            assert!(cancel.is_cancelled());
        }
    }
}
