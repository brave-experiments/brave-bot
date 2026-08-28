//! The commands a session's user has vouched for.
//!
//! An entry is a **program and its exact arguments**, and vouching for one is a statement by the
//! user about two things at once:
//!
//! 1. It may run without being asked again.
//! 2. What it prints is **trusted**.
//!
//! Both halves are the user's assertion, not an inference. Nothing here establishes that a command
//! is side-effect-free or that its output is free of influence, and nothing tries: `git log`
//! prints commit messages that whoever contributed to the repository wrote. The user saying "I
//! trust this command and its output" is what makes the output trusted, exactly as
//! [`crate::trust::TrustStore`] makes a directory's contents trusted because the user said so and
//! not because anything inspected them.
//!
//! That is the whole justification, so the prompt has to ask for it in those terms. A person
//! agreeing to this is agreeing that the command's side effects and its output are both theirs to
//! answer for. See `docs/tools.md`.
//!
//! # Distinct from `crate::pure`
//!
//! [`crate::pure`] answers a different question and answers it by audit: whether a program,
//! given a particular argv, can read anything the label does not account for. Its table is
//! hand-checked against each program's full option surface, and nothing a user says extends it.
//! This module is the human-assertion route to the same label, and the two must not be confused:
//! one is a proof about a program, the other is a person taking responsibility for one.
//!
//! # Keyed by program and arguments, both exact
//!
//! Not by program alone. Vouching for `git log` says nothing about `git push`, and it must not:
//! the two do different things and produce different output, and an entry that covered both would
//! be granting far more than the person read.
//!
//! The program is the **resolved path**. `$PATH` and shell aliases decide what a name means, so
//! recording the string would let a later change inherit an assertion made about a different
//! binary. Resolution happens outside this crate, which performs no I/O; see `bua_agent::programs`.
//!
//! # The session, not the directory
//!
//! Kept in the session record and restored on resume, on the same reasoning as the trust map: the
//! person resuming is the person who gave it. A fresh session in the same directory starts empty
//! and asks, because a list kept per directory would grant this on behalf of a user who was never
//! asked.

use std::collections::BTreeSet;

/// One command a user vouched for: a resolved program and the exact arguments it runs with.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Command {
    /// The absolute path the program name resolved to.
    pub program: String,
    /// The arguments, in order. Empty is a real value and different from any non-empty list.
    pub args: Vec<String>,
}

impl Command {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }

    /// The command as a person should read it, with argument boundaries visible.
    pub fn display(&self) -> String {
        crate::command::Stage::new(self.program.clone(), self.args.clone()).display()
    }
}

/// The set of commands a session has vouched for.
///
/// Empty means every run asks and every run's output is untrusted, which is the state every
/// session starts in. Membership is granted, never assumed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedPrograms {
    /// Ordered so a record written twice is written the same way.
    commands: BTreeSet<Command>,
}

impl TrustedPrograms {
    /// An empty set: nothing is vouched for.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that the user vouched for this exact command, side effects and output alike.
    pub fn trust(&mut self, command: Command) {
        self.commands.insert(command);
    }

    /// Forget one, so it is asked about again and its output is untrusted again.
    pub fn forget(&mut self, command: &Command) -> bool {
        self.commands.remove(command)
    }

    /// Whether this exact program and argument list was vouched for.
    pub fn contains(&self, program: &str, args: &[String]) -> bool {
        self.commands
            .iter()
            .any(|c| c.program == program && c.args == args)
    }

    /// Every command vouched for, in a stable order.
    pub fn iter(&self) -> impl Iterator<Item = &Command> {
        self.commands.iter()
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

impl FromIterator<Command> for TrustedPrograms {
    fn from_iter<I: IntoIterator<Item = Command>>(commands: I) -> Self {
        Self {
            commands: commands.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_log() -> Command {
        Command::new("/usr/bin/git", vec!["log".into()])
    }

    /// Every session starts asking about everything, and trusting no output. Membership is
    /// granted, never assumed from silence, which is the rule the trust map lives by.
    #[test]
    fn nothing_is_vouched_for_to_begin_with() {
        let programs = TrustedPrograms::new();
        assert!(programs.is_empty());
        assert!(!programs.contains("/usr/bin/git", &["log".to_string()]));
    }

    #[test]
    fn a_command_that_was_vouched_for_is_recognised() {
        let mut programs = TrustedPrograms::new();
        programs.trust(git_log());
        assert!(programs.contains("/usr/bin/git", &["log".to_string()]));
    }

    /// The reason entries are not keyed by program alone. Vouching for `git log` says nothing
    /// about `git push`: they do different things and print different things, and one entry
    /// covering both would grant far more than the person read.
    #[test]
    fn vouching_for_one_command_says_nothing_about_another_of_the_same_program() {
        let mut programs = TrustedPrograms::new();
        programs.trust(git_log());
        assert!(
            !programs.contains("/usr/bin/git", &["push".to_string()]),
            "an assertion about one command covered a different one"
        );
    }

    /// Arguments are matched exactly and in order, since a different argument list is a different
    /// command with different output.
    #[test]
    fn the_arguments_must_match_exactly() {
        let mut programs = TrustedPrograms::new();
        programs.trust(Command::new(
            "/usr/bin/git",
            vec!["log".into(), "-n".into(), "5".into()],
        ));
        for other in [
            vec!["log".to_string()],
            vec!["log".to_string(), "-n".to_string()],
            vec!["log".to_string(), "-n".to_string(), "50".to_string()],
            vec!["-n".to_string(), "5".to_string(), "log".to_string()],
        ] {
            assert!(
                !programs.contains("/usr/bin/git", &other),
                "{other:?} matched an entry it is not"
            );
        }
    }

    /// A command with no arguments is a real entry, distinct from any command with some.
    #[test]
    fn no_arguments_is_its_own_entry() {
        let mut programs = TrustedPrograms::new();
        programs.trust(Command::new("/bin/pwd", Vec::new()));
        assert!(programs.contains("/bin/pwd", &[]));
        assert!(!programs.contains("/bin/pwd", &["-L".to_string()]));
    }

    /// Matched on the resolved path, so an assertion does not follow a name onto a different
    /// binary when `$PATH` or an alias changes what the name means.
    #[test]
    fn the_same_name_at_a_different_path_is_a_different_program() {
        let mut programs = TrustedPrograms::new();
        programs.trust(Command::new("/usr/bin/grep", vec!["x".into()]));
        assert!(
            !programs.contains("/opt/homebrew/bin/grep", &["x".to_string()]),
            "an assertion followed a name onto a different binary"
        );
    }

    #[test]
    fn vouching_twice_records_one_command() {
        let mut programs = TrustedPrograms::new();
        programs.trust(git_log());
        programs.trust(git_log());
        assert_eq!(programs.len(), 1);
    }

    #[test]
    fn a_command_can_be_forgotten() {
        let mut programs = TrustedPrograms::new();
        programs.trust(git_log());
        assert!(programs.forget(&git_log()));
        assert!(!programs.contains("/usr/bin/git", &["log".to_string()]));
        assert!(
            !programs.forget(&git_log()),
            "forgetting twice found nothing"
        );
    }

    /// The prompt and the status report both show entries, so the rendering has to keep argument
    /// boundaries visible the way the approval prompt does.
    #[test]
    fn a_command_reads_back_with_its_argument_boundaries() {
        let command = Command::new(
            "/usr/bin/git",
            vec!["commit".into(), "-m".into(), "two words".into()],
        );
        assert_eq!(command.display(), "/usr/bin/git commit -m 'two words'");
    }

    /// Written down and read back the same way, so a resumed session vouches for what the record
    /// says and nothing more.
    #[test]
    fn a_set_survives_being_written_down_and_read_back() {
        let programs =
            TrustedPrograms::from_iter([git_log(), Command::new("/bin/ls", vec!["-la".into()])]);
        let written: Vec<Command> = programs.iter().cloned().collect();
        assert_eq!(TrustedPrograms::from_iter(written), programs);
        assert_eq!(
            programs.iter().next().map(Command::display),
            Some("/bin/ls -la".to_string()),
            "order is stable"
        );
    }
}
