//! Where the user's own files live.
//!
//! `~/.bravebot` holds what should outlive a session: prompt history, session records, and now
//! standing instructions and skills. One definition of where that is, so the interface and the
//! agent cannot drift apart about it.
//!
//! There is deliberately no fallback. A missing `HOME` yields `None` and every caller does
//! without, because inventing a directory would mean reading files from somewhere the user never
//! chose, and this is the one place whose contents are trusted for being the user's own.

use std::path::{Path, PathBuf};

/// The name of the directory inside the user's home.
const DIRECTORY: &str = ".bravebot";

/// The user's own directory, or `None` when there is no home to put it in.
///
/// This is the reading answer. Anything about to write wants [`writable`] instead.
pub fn directory() -> Option<PathBuf> {
    // Read directly rather than taking a dependency for one variable. Absent in some daemon and
    // container environments, which is a case that has to be handled anyway.
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(Path::new(&home).join(DIRECTORY))
}

/// The user's own directory when something may be written into it, or `None` when nothing may be.
///
/// `None` for two different reasons that callers should treat the same way: there is no home to
/// write to, or the session is [incognito] and is not going to write to it. Both mean "do not
/// persist this", and every caller already handled the first, which is what makes the second cost
/// a line rather than a redesign.
///
/// # Why a second function rather than a flag inside the first
///
/// Reads must keep working in an incognito session: the chosen model, the theme, the standing
/// instructions and the credentials all come out of this directory, and a session that could not
/// read them would not be private but crippled. Making [`directory`] itself answer `None` would
/// have taken those away too. Splitting the question in two puts the mode exactly where it belongs,
/// on the writes, and makes each call site say which it is doing.
///
/// [incognito]: bravebot_core::incognito
pub fn writable() -> Option<PathBuf> {
    if bravebot_core::incognito::engaged() {
        return None;
    }
    directory()
}
