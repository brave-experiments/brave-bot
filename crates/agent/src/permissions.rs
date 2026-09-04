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
    let home = home.map(|home| home.display().to_string());
    let anchors = Anchors {
        // The settings file lives in the global state directory, so a `/` rule is anchored there.
        settings_dir: home.as_ref().map(|home| format!("{home}/.bravebot")),
        home,
    };
    Permissions::parse(&lists.deny, &lists.ask, &lists.allow, &anchors)
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
