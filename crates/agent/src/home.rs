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
pub fn directory() -> Option<PathBuf> {
    // Read directly rather than taking a dependency for one variable. Absent in some daemon and
    // container environments, which is a case that has to be handled anyway.
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(Path::new(&home).join(DIRECTORY))
}
