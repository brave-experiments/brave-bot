//! The programs a session's user has vouched for.
//!
//! Every run is put to a person the first time. This is what stops the second, third and fortieth
//! time being put to them as well: a person who has said yes to `cargo` once can say "and again",
//! and runs of that program stop interrupting them for the rest of the session.
//!
//! # What this is not
//!
//! **Not an allowlist.** It does not decide what may run. A program nobody has vouched for still
//! runs, after a prompt, exactly as it did before; the set is empty at the start of every session
//! and nothing is refused for being absent from it. So the property the tool rests on is unchanged
//! and is still the label on the output, never a belief about the binary. A list that decided what
//! could run would be a belief about the binary, and would be worth less than it looked: the
//! interesting programs are the ones a user actually needs, and those are the ones that would be
//! on it.
//!
//! **Not a substitute for reading the argv.** Remembering a program is coarser than remembering
//! the argument vector: someone who vouches for `git` after reading `git log` has also, in this
//! session, stopped being asked about `git push`. That is a real widening and the prompt says so
//! in as many words, because a person agreeing to it should be agreeing to the thing it does.
//!
//! # Keyed by what the name resolved to
//!
//! A name is not a program. `$PATH` and shell aliases both decide what `grep` means, and on the
//! machine this was developed against it means `ugrep`. Remembering the string `grep` would let a
//! later change to `$PATH` inherit an approval given for a different binary, so what is recorded
//! is the resolved path and what is matched is the resolved path. Resolution happens outside this
//! crate, which performs no I/O; see `bua_agent::programs`.
//!
//! # The session, not the directory
//!
//! Kept in the session record and restored on resume, on the same reasoning as the trust map: the
//! person resuming is the person who gave it. A fresh session in the same directory starts empty
//! and asks, because a list kept per directory would answer on behalf of a user who was never
//! asked.

use std::collections::BTreeSet;

/// The set of programs a session has stopped asking about.
///
/// Empty means every run asks, which is the state every session starts in. Membership is granted,
/// never assumed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedPrograms {
    /// Resolved absolute paths, ordered so a record written twice is written the same way.
    resolved: BTreeSet<String>,
}

impl TrustedPrograms {
    /// An empty set: nothing is vouched for, so every run asks.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that the user vouched for the program at this resolved path.
    pub fn trust(&mut self, resolved: impl Into<String>) {
        self.resolved.insert(resolved.into());
    }

    /// Forget one, so it is asked about again.
    pub fn forget(&mut self, resolved: &str) -> bool {
        self.resolved.remove(resolved)
    }

    pub fn contains(&self, resolved: &str) -> bool {
        self.resolved.contains(resolved)
    }

    /// Every path vouched for, in a stable order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.resolved.iter().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.resolved.len()
    }

    pub fn is_empty(&self) -> bool {
        self.resolved.is_empty()
    }
}

impl<S: Into<String>> FromIterator<S> for TrustedPrograms {
    fn from_iter<I: IntoIterator<Item = S>>(paths: I) -> Self {
        Self {
            resolved: paths.into_iter().map(Into::into).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every session starts asking about everything. Membership is granted, never assumed from
    /// silence, which is the same rule the trust map lives by.
    #[test]
    fn nothing_is_vouched_for_to_begin_with() {
        let programs = TrustedPrograms::new();
        assert!(programs.is_empty());
        assert!(!programs.contains("/usr/bin/git"));
    }

    #[test]
    fn a_program_that_was_vouched_for_is_recognised() {
        let mut programs = TrustedPrograms::new();
        programs.trust("/usr/bin/git");
        assert!(programs.contains("/usr/bin/git"));
    }

    /// Matched on the resolved path, so the same name reaching a different binary is a different
    /// program. This is the case `$PATH` and shell aliases create, and inheriting an approval
    /// across it would be inheriting it for something the person never saw.
    #[test]
    fn the_same_name_at_a_different_path_is_a_different_program() {
        let mut programs = TrustedPrograms::new();
        programs.trust("/usr/bin/grep");
        assert!(
            !programs.contains("/opt/homebrew/bin/grep"),
            "an approval followed a name onto a different binary"
        );
    }

    #[test]
    fn vouching_twice_records_one_program() {
        let mut programs = TrustedPrograms::new();
        programs.trust("/bin/ls");
        programs.trust("/bin/ls");
        assert_eq!(programs.len(), 1);
    }

    #[test]
    fn a_program_can_be_forgotten() {
        let mut programs = TrustedPrograms::new();
        programs.trust("/bin/ls");
        assert!(programs.forget("/bin/ls"));
        assert!(!programs.contains("/bin/ls"));
        assert!(
            !programs.forget("/bin/ls"),
            "forgetting twice found nothing"
        );
    }

    /// Written down and read back the same way, so a resumed session vouches for what the record
    /// says and nothing more.
    #[test]
    fn a_set_survives_being_written_down_and_read_back() {
        let programs = TrustedPrograms::from_iter(["/usr/bin/git", "/bin/ls"]);
        let written: Vec<&str> = programs.iter().collect();
        let read = TrustedPrograms::from_iter(written.clone());
        assert_eq!(read, programs);
        assert_eq!(written, vec!["/bin/ls", "/usr/bin/git"], "order is stable");
    }
}
