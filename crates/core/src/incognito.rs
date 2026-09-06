//! A session that leaves nothing behind.
//!
//! Ordinarily a session is worth keeping: the prompts a person typed, the record of what happened,
//! and the model and theme they chose all outlive the run that produced them, and
//! [`crate::policy`] treats that directory as the user's own configuration surface. Sometimes it is
//! not worth keeping. Work on someone else's machine, a prompt naming something private, a
//! demonstration in front of a room: in each case the value of the record is negative, and the
//! person wants the session and nothing else.
//!
//! Engaging this says so once, at startup, and every place that would write to the user's own
//! directory declines instead.
//!
//! # Reading is not writing
//!
//! An incognito session still reads. It loads the settings, the chosen model, the theme, and the
//! credentials that let it reach a backend at all, because a mode that could not authenticate
//! would not be a private session but a broken one. What it does not do is add to any of that.
//! This is the same shape as a browser's private window, and for the same reason: the promise is
//! about what survives the session, not about what the session may know.
//!
//! # One way
//!
//! There is no way to turn it off. A flag that could be cleared would mean every write site had to
//! reason about when it was cleared and by what, and a single missed ordering would write the
//! thing the mode exists to not write. Engaging is therefore a one-way door for the life of the
//! process, and [`engaged`] answers the same way from any thread that asks.
//!
//! # What this is not
//!
//! It is not confinement. Nothing here stops a subprocess the user asked for from writing wherever
//! their own shell could, and the `write_file` and `edit_file` tools go on editing the workspace,
//! which is the work rather than a trace of it. The guarantee is about this process and the
//! directory this process owns. [`bravebot-sandbox`] is the crate that confines somebody else's
//! code, and it confines subprocesses rather than this one.
//!
//! [`bravebot-sandbox`]: https://github.com/brave-experiments/brave-bot

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether this process was asked to leave nothing behind.
///
/// Process-wide because the property is: a session is incognito or it is not, and a per-caller
/// answer would let one half of the program record what the other half declined to.
static ENGAGED: AtomicBool = AtomicBool::new(false);

/// Ask that nothing be written to the user's own directory for the rest of this process.
///
/// Called once, from the entry point, before anything has had a chance to persist. Calling it
/// twice is not an error and calling it late is not either; it simply cannot unsay what was
/// already written, which is why the entry point is the only sensible place for it.
pub fn engage() {
    // Release, paired with the acquire in `engaged`: a thread that observes the flag must also
    // observe everything the engaging thread did beforehand. Relaxed would admit a reader seeing
    // the flag set while still working from state prepared before it was.
    ENGAGED.store(true, Ordering::Release);
}

/// Whether this session leaves anything behind.
///
/// Answered by every write site rather than by one gatekeeper, because the write sites are what
/// the promise is about and a gatekeeper is one more thing that can be bypassed.
pub fn engaged() -> bool {
    ENGAGED.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default is the ordinary session. A mode that had to be turned off would make every
    /// existing install private by accident, which is a different product.
    ///
    /// Runs in the same binary as the test below, so it cannot assert `!engaged()` after that one
    /// has run. What it pins is that the constant this starts from is `false`.
    #[test]
    fn nothing_is_incognito_until_it_is_asked_for() {
        assert!(!AtomicBool::new(false).load(Ordering::Acquire));
    }

    /// Engaging twice is the same as engaging once. The entry point should not have to know
    /// whether something earlier already did it.
    #[test]
    fn engaging_is_idempotent() {
        engage();
        assert!(engaged());
        engage();
        assert!(engaged());
    }
}
