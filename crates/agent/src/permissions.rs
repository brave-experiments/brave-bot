//! Reading the `permissions` block out of the settings file.
//!
//! The kernel owns the rule language and this owns finding the file, because reading a rule needs
//! to know where `~` and the settings file are, which is I/O the kernel does not do. What crosses
//! the boundary is rule text and two directory names.
//!
//! # Why the anchors are what they are
//!
//! A `~/` rule points at the user's home directory. A `/` rule points at the directory the
//! settings file sits in, which is `~/.bravebot`: Claude Code anchors a single leading slash at
//! the settings source rather than at the filesystem root, so a rule written in a user-level file
//! means something inside that file's own directory. That is a trap worth reproducing rather than
//! improving on, because somebody who has read those docs will write `//` when they mean the root,
//! and quietly meaning something else here would be worse than agreeing.

use bravebot_config::Settings;
use bravebot_core::permissions::{Anchors, Permissions, Rejected};

/// The rules a settings file carried, and any of its lines that were not rules.
///
/// The rejects are returned rather than logged so a caller can report them where a person will
/// read them. A rule nobody can act on is worth saying out loud: a misspelled deny rule reads as
/// protection that is not there.
pub fn from_settings(
    settings: &Settings,
    home: Option<&std::path::Path>,
) -> (Permissions, Vec<Rejected>) {
    let lists = settings.permissions();
    Permissions::parse(&lists.deny, &lists.ask, &lists.allow, &anchors(home))
}

/// The same rules for a run with nobody at it, which is every list but the one that allows.
///
/// An allow rule answers a prompt in advance. Where nobody can be asked there is no prompt for it
/// to answer, so honouring one would not be saving somebody a keystroke: it would be letting a
/// line in a settings file write, run a program or fetch a URL in a run nobody is watching, beside
/// the command-line flag that is meant to be the only way that happens.
///
/// The other two carry over, because both still say something such a run can act on. A deny rule
/// refuses before there is anything to prompt about, and an ask rule turns a write that would have
/// gone through silently into one there is nobody to approve.
pub fn for_an_unattended_run(
    settings: &Settings,
    home: Option<&std::path::Path>,
) -> (Permissions, Vec<Rejected>) {
    let lists = settings.permissions();
    let anchors = anchors(home);
    let (permissions, mut rejected) = Permissions::parse(&lists.deny, &lists.ask, &[], &anchors);
    // Read for its rejects and then dropped, rather than not read at all. A line the person
    // believes is in force and that nothing can act on is worth saying out loud wherever it was
    // written, and an allow rule that is silently unreadable here reads to them as one that holds.
    let (_, unreadable) = Permissions::parse(&[], &[], &lists.allow, &anchors);
    rejected.extend(unreadable);
    (permissions, rejected)
}

/// What a leading `~` and a leading `/` in a rule are resolved against.
fn anchors(home: Option<&std::path::Path>) -> Anchors {
    let home = home.map(|home| home.display().to_string());
    Anchors {
        // The settings file lives in the global state directory, so a `/` rule is anchored there.
        settings_dir: home.as_ref().map(|home| format!("{home}/.bravebot")),
        home,
    }
}

/// The directories a settings file said to open, in the order it named them.
///
/// Names only. Opening one is the caller's to do, through the same path `/add-dir` takes, so that
/// a directory a file named and a directory a person typed are reachable on identical terms and
/// neither has a route the other lacks.
pub fn additional_directories(settings: &Settings) -> &[String] {
    &settings.permissions().additional_directories
}

#[cfg(test)]
mod tests {
    use super::*;
    use bravebot_core::permissions::{Decision, Ruling, Subject};
    use std::path::PathBuf;

    /// The block from Claude Code's own documentation, read out of a settings file and into rules
    /// that decide something. This is the whole point of the module.
    #[test]
    fn a_settings_file_block_becomes_rules_that_decide() {
        let settings = Settings::parse(
            r#"{
              "permissions": {
                "allow": ["Bash(git diff *)"],
                "ask": ["Bash(git push *)"],
                "deny": ["Read(./.env)"]
              }
            }"#,
        );
        let (permissions, rejected) = from_settings(&settings, Some(&PathBuf::from("/home/x")));
        assert!(rejected.is_empty());
        assert_eq!(
            permissions.for_command("git diff --stat"),
            Decision::Ruled(Ruling::Allow)
        );
        assert_eq!(
            permissions.for_command("git push origin main"),
            Decision::Ruled(Ruling::Ask)
        );
        assert_eq!(
            permissions.for_path(Subject::Read, ".env"),
            Decision::Ruled(Ruling::Deny)
        );
    }

    /// No settings file means no rules, which is the state every session was in before this
    /// existed and must stay indistinguishable from it.
    #[test]
    fn no_block_is_no_rules() {
        let (permissions, rejected) = from_settings(&Settings::default(), None);
        assert!(permissions.is_empty());
        assert!(rejected.is_empty());
    }

    /// A line that is not a rule is handed back rather than dropped in silence, so a person can be
    /// told which of their rules is doing nothing.
    #[test]
    fn a_line_that_is_not_a_rule_is_reported() {
        let settings = Settings::parse(r#"{"permissions": {"deny": ["Read(.env)", "Nonsense"]}}"#);
        let (permissions, rejected) = from_settings(&settings, Some(&PathBuf::from("/home/x")));
        assert_eq!(permissions.len(), 1);
        assert_eq!(rejected.len(), 1);
        assert!(rejected[0].to_string().contains("Nonsense"));
    }

    /// A run with nobody at it keeps the two lists it can act on and loses the one that answers a
    /// prompt, because there is no prompt for that one to answer.
    #[test]
    fn a_run_nobody_is_watching_keeps_every_rule_but_the_ones_that_allow() {
        let settings = Settings::parse(
            r#"{
              "permissions": {
                "allow": ["Bash(git diff *)"],
                "ask": ["Bash(git push *)"],
                "deny": ["Read(./.env)"]
              }
            }"#,
        );
        let (permissions, rejected) =
            for_an_unattended_run(&settings, Some(&PathBuf::from("/home/x")));
        assert!(rejected.is_empty());
        assert_eq!(
            permissions.for_command("git diff --stat"),
            Decision::Unmatched
        );
        assert_eq!(
            permissions.for_command("git push origin main"),
            Decision::Ruled(Ruling::Ask)
        );
        assert_eq!(
            permissions.for_path(Subject::Read, ".env"),
            Decision::Ruled(Ruling::Deny)
        );
    }

    /// An allow rule that cannot be read is still named, though nothing would have acted on it
    /// here. A rule reported nowhere reads to the person who wrote it as one that is in force, and
    /// the same file is read by a session where it decides something.
    #[test]
    fn an_unreadable_allow_rule_is_reported_to_a_run_nobody_is_watching() {
        let settings = Settings::parse(r#"{"permissions": {"allow": ["Bash(git diff *"]}}"#);
        let (permissions, rejected) =
            for_an_unattended_run(&settings, Some(&PathBuf::from("/home/x")));
        assert!(permissions.is_empty());
        assert_eq!(rejected.len(), 1);
        assert!(rejected[0].to_string().contains("git diff"));
    }

    /// A single leading slash is anchored at the settings file's own directory, which is the
    /// documented behaviour and the one somebody is most likely to get wrong.
    #[test]
    fn a_single_slash_rule_is_anchored_at_the_settings_directory() {
        let settings = Settings::parse(r#"{"permissions": {"deny": ["Read(/secrets/**)"]}}"#);
        let (permissions, _) = from_settings(&settings, Some(&PathBuf::from("/home/x")));
        assert_eq!(
            permissions.for_path(Subject::Read, "/home/x/.bravebot/secrets/key"),
            Decision::Ruled(Ruling::Deny)
        );
        assert_eq!(
            permissions.for_path(Subject::Read, "/secrets/key"),
            Decision::Unmatched
        );
    }

    /// The directories a file asked for come back in the order it named them, since a caller
    /// opens them one at a time and reports each.
    #[test]
    fn the_directories_a_file_named_come_back_in_order() {
        let settings = Settings::parse(
            r#"{"permissions": {"additionalDirectories": ["../shared", "/opt/other"]}}"#,
        );
        assert_eq!(
            additional_directories(&settings),
            ["../shared", "/opt/other"]
        );
    }
}
