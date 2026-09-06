//! Lending one confirmer, one reporter and one audit trail to several runs at once.
//!
//! A turn and the delegates it spawned run at the same time and each of the three is single: one
//! person answers the questions, one screen shows the lines, one trail records the decisions.
//! What they need is not a copy each but a turn each, which is what this is.
//!
//! # Why a lock and not a copy
//!
//! Copying is wrong for all three. Two trails leave a hole in the record exactly over the part of
//! the turn nobody watched. Two reporters interleave half-written lines. Two confirmers put two
//! questions on one screen, and a person cannot answer either without reading both.
//!
//! Holding the lock for the whole of one question is deliberate rather than a cost. A person is
//! asked one thing at a time, and a delegate that wants an answer while somebody is reading
//! another delegate's diff waits for them to finish reading it.
//!
//! # What this does not do
//!
//! It carries no content and reads none. Every method here takes what it was handed, takes the
//! lock, and passes it straight through. Nothing is compared, matched or routed, and a value's
//! label is the label it arrived with.

use crate::confirm::{
    Confirmer, Decision, OutputRequest, RunDecision, RunRequest, VouchRequest, WriteRequest,
};
use crate::report::{Activity, DelegateId, Delegation, Landing, Phase, Reporter, Shown};
use bravebot_core::ask::{Answer, Asking};
use bravebot_core::event::{Event, Sink};
use bravebot_core::todo::Row;
use std::sync::{Mutex, MutexGuard};

/// One thing several runs take turns with.
///
/// Holds the borrow for as long as the turn does, and hands out [`Borrowed`] handles that take
/// the lock for one call each.
pub struct Lent<'a, T: ?Sized> {
    inner: Mutex<&'a mut T>,
}

impl<'a, T: ?Sized> Lent<'a, T> {
    pub fn new(inner: &'a mut T) -> Self {
        Self {
            inner: Mutex::new(inner),
        }
    }

    /// A handle for the turn's own work.
    pub fn turn(&self) -> Borrowed<'_, 'a, T> {
        Borrowed {
            lent: self,
            from: None,
        }
    }

    /// A handle for one delegate's work, which says whose every report through it is.
    pub fn delegate(&self, id: DelegateId) -> Borrowed<'_, 'a, T> {
        Borrowed {
            lent: self,
            from: Some(id),
        }
    }

    /// Take the lock.
    ///
    /// A delegate that panicked leaves it poisoned, and the turn is still running and still owns
    /// the screen. What is behind the lock is a confirmer, a reporter or a trail, and a panic
    /// leaves none of the three half written: each method here is one call that either happened
    /// or did not. So the turn carries on with what it was lent rather than dying of somebody
    /// else's failure.
    fn hold(&self) -> MutexGuard<'_, &'a mut T> {
        self.inner.lock().unwrap_or_else(|held| held.into_inner())
    }

    /// Take the lock if nothing else holds it.
    ///
    /// For the one question nobody is waiting on the answer to. See [`Borrowed::interjection`].
    fn try_hold(&self) -> Option<MutexGuard<'_, &'a mut T>> {
        match self.inner.try_lock() {
            Ok(held) => Some(held),
            Err(std::sync::TryLockError::Poisoned(held)) => Some(held.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }
}

/// One run's handle on something lent.
///
/// Copyable, because a delegate is handed one of these and the turn keeps its own.
pub struct Borrowed<'m, 'a, T: ?Sized> {
    lent: &'m Lent<'a, T>,
    /// Whose work goes through this handle, where it is a delegate's.
    from: Option<DelegateId>,
}

impl<T: ?Sized> Clone for Borrowed<'_, '_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> Copy for Borrowed<'_, '_, T> {}

impl<T: Sink + ?Sized> Sink for Borrowed<'_, '_, T> {
    fn emit(&mut self, event: Event) {
        self.lent.hold().emit(event);
    }
}

/// Forward one report, saying whose it is first.
///
/// Both under one lock, so a line and the run it belongs to cannot be separated by another run
/// reporting in between.
macro_rules! reports {
    ($( fn $name:ident(&mut self $(, $arg:ident: $ty:ty)* $(,)?); )*) => {
        $(
            fn $name(&mut self $(, $arg: $ty)*) {
                let mut held = self.lent.hold();
                held.reporting_for(self.from);
                held.$name($($arg),*);
            }
        )*
    };
}

impl<T: Reporter + ?Sized> Reporter for Borrowed<'_, '_, T> {
    reports! {
        fn todos(&mut self, rows: Vec<Row>);
        fn output_tokens(&mut self, written: u64);
        fn phase(&mut self, phase: Phase);
        fn narration(&mut self, text: String);
        fn streaming(&mut self, text: String);
        fn notice(&mut self, text: String);
        fn quarantined(&mut self, shown: Shown);
        fn landed(&mut self, landing: Landing);
        fn tool_started(&mut self, activity: Activity);
        fn tool_finished(&mut self, activity: Activity);
        fn interjected(&mut self, said: String);
        fn delegate_started(&mut self, delegation: Delegation);
    }

    /// Not through the macro: whose report this is was settled when the handle was made, and a
    /// delegate finishing is the turn's news rather than the delegate's own.
    fn delegate_finished(&mut self, delegate: DelegateId, note: String, failed: bool) {
        let mut held = self.lent.hold();
        held.reporting_for(None);
        held.delegate_finished(delegate, note, failed);
    }

    /// Passed on rather than remembered, so a handle for a delegate cannot be talked into
    /// reporting as the turn.
    fn reporting_for(&mut self, _delegate: Option<DelegateId>) {}
}

impl<T: Confirmer + ?Sized> Confirmer for Borrowed<'_, '_, T> {
    fn confirm_write(&mut self, request: &WriteRequest) -> Decision {
        self.lent.hold().confirm_write(request)
    }

    fn confirm_run(&mut self, request: &RunRequest) -> RunDecision {
        self.lent.hold().confirm_run(request)
    }

    fn confirm_read_output(&mut self, request: &OutputRequest) -> Decision {
        self.lent.hold().confirm_read_output(request)
    }

    fn confirm_vouch(&mut self, request: &VouchRequest) -> Decision {
        self.lent.hold().confirm_vouch(request)
    }

    fn ask_user(&mut self, asking: &Asking) -> Vec<Answer> {
        self.lent.hold().ask_user(asking)
    }

    /// The one method here nobody is waiting on, so it never waits.
    ///
    /// Everything else is a question, and a question is asked because a run has stopped until it
    /// is answered. This is a poll: the turn asks between rounds whether anything was typed. With
    /// somebody part way through answering a delegate's question the lock is held for as long as
    /// they take to read it, and a turn that blocked here would stop for that long over a
    /// question it did not ask. What was typed keeps until the next poll.
    fn interjection(&mut self) -> Option<String> {
        self.lent.try_hold()?.interjection()
    }
}
