//! Scratch directories for tests.

use std::path::{Path, PathBuf};

/// An absolute path under the workspace `target/test-scratch/`.
///
/// Tests built these under [`std::env::temp_dir`] before. That directory is shared
/// between users and between processes with different privileges, so a fixed name
/// under it collides whenever two checkouts run the tests at once, and it is the
/// insecure-temporary-file pattern the security scan flags. `target/` is
/// per-checkout and already ignored by git.
///
/// Nothing is created here: callers make and remove the directory as they already did.
pub(crate) fn scratch_dir(name: &str) -> PathBuf {
    // CARGO_MANIFEST_DIR is `<workspace>/crates/<crate>`, so two pops reach the root.
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path.push("target");
    path.push("test-scratch");
    path.push(name);
    path
}

/// A scratch directory that removes itself.
///
/// Removing at the end of the test body leaves one behind whenever an assertion fails, and what it
/// holds is a directory tree with modes on it: the next run of the same test then measures what the
/// failing run left rather than what it created.
pub(crate) struct Scratch {
    path: PathBuf,
}

impl Scratch {
    pub(crate) fn new(name: &str) -> Self {
        let path = scratch_dir(name);
        let _ = std::fs::remove_dir_all(&path);
        Self { path }
    }
}

/// So a test reads as though it held the path, which is all it wants from this.
impl std::ops::Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
