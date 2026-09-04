//! The `env` block in `~/.bravebot/settings.json`.
//!
//! A file rather than only the process environment, because the values that select a backend are
//! long-lived: which AWS profile to assume and which model each tier names are properties of a
//! person's account, not of the shell a session happened to start in. Exporting them from a shell
//! profile works and keeps working; this exists so it is not the only way.
//!
//! Deliberately the same shape as Claude Code's `~/.claude/settings.json`, down to the variable
//! names, so a block that configures one configures the other unedited. A different spelling for
//! the same six values would be a second thing to learn for no gain.
//!
//! # What this file is trusted for
//!
//! Every name in the block is read, not a chosen subset. The file is the user's own configuration
//! surface, on the footing [`crate`]'s callers already treat `~/.bravebot` as: a value here is
//! something the person running the agent typed, and it is trusted exactly as far as a variable
//! they exported would be. Nothing a turn produces can write it, and no model output reaches it.
//!
//! It does not become the process environment. Values are consulted where a variable would be
//! consulted, and handed to a subprocess only where that subprocess is the thing they configure.
//! Installing them globally would put every name in the block in front of every command `run`
//! ever starts, which is a much larger claim than "this is how I reach the backend".

use std::collections::BTreeMap;
use std::path::PathBuf;

/// The file, inside the global state directory.
const SETTINGS_FILE: &str = "settings.json";

/// The most of it worth reading.
///
/// A settings file is a handful of short strings. Bounded so a file that grew by accident, or was
/// replaced by something else entirely, is refused rather than parsed.
const MAX_BYTES: u64 = 64 * 1024;

/// The `env` block, or empty when there is no file or it cannot be read.
///
/// Every failure is the same as absence. A missing home directory, no file, a syntax error, a
/// value that is not a string: none of them is worth refusing to start over, because the process
/// environment and the built-in values still describe a working backend. A file nobody can parse
/// is reported by `doctor` rather than at startup, where the person who mistyped it is not
/// necessarily the person watching.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    env: BTreeMap<String, String>,
    scrub: Vec<String>,
    permissions: PermissionLists,
}

/// The `permissions` block, as text, exactly as the file spelled it.
///
/// Rule text rather than parsed rules, because reading a rule needs to know where the settings
/// file sits and where home is, and this crate is where the file was found rather than where a
/// rule is matched. The kernel owns the rule language; this hands it the lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionLists {
    pub deny: Vec<String>,
    pub ask: Vec<String>,
    pub allow: Vec<String>,
    /// Directories a file says to make reachable, alongside the working directory.
    pub additional_directories: Vec<String>,
}

impl PermissionLists {
    /// Whether the block said anything.
    pub fn is_empty(&self) -> bool {
        self.deny.is_empty()
            && self.ask.is_empty()
            && self.allow.is_empty()
            && self.additional_directories.is_empty()
    }
}

impl Settings {
    /// Read the settings file for this user.
    pub fn load() -> Self {
        Self::from_home(home())
    }

    /// As [`Settings::load`], for a named home directory, so a test needs no ambient one.
    pub fn from_home(home: Option<PathBuf>) -> Self {
        let Some(path) = home.map(|home| home.join(SETTINGS_FILE)) else {
            return Self::default();
        };
        match std::fs::metadata(&path) {
            Ok(found) if found.len() > MAX_BYTES => return Self::default(),
            Ok(_) => {}
            Err(_) => return Self::default(),
        }
        std::fs::read_to_string(&path)
            .ok()
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// Read the `env` block and the scrub list out of settings JSON.
    ///
    /// Only string values are taken. JSON allows a number or a boolean where a variable wants a
    /// string, and coercing one would invent a spelling the writer did not choose: `1` and `true`
    /// are not obviously `"1"` and `"true"` to whoever has to debug it later.
    ///
    /// The two blocks are independent: a file naming variables to keep from a subprocess is read
    /// whether or not it also configures a backend.
    pub fn parse(text: &str) -> Self {
        let Ok(serde_json::Value::Object(root)) = serde_json::from_str(text) else {
            return Self::default();
        };
        let env = match root.get("env") {
            Some(serde_json::Value::Object(block)) => block
                .iter()
                .filter_map(|(name, value)| match value {
                    serde_json::Value::String(value) => Some((name.clone(), value.clone())),
                    _ => None,
                })
                .collect(),
            _ => BTreeMap::new(),
        };
        Self {
            env,
            scrub: scrub_list(&root),
            permissions: permission_lists(&root),
        }
    }

    /// What this file says a variable is, if it says anything.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.env.get(name).map(String::as_str)
    }

    /// Whether the file set anything at all.
    pub fn is_empty(&self) -> bool {
        self.env.is_empty() && self.scrub.is_empty() && self.permissions.is_empty()
    }

    /// The rule text and added directories the `permissions` block carried.
    pub fn permissions(&self) -> &PermissionLists {
        &self.permissions
    }

    /// Variables this file says to keep from a program the agent runs, beyond the built-in set.
    ///
    /// Names only, which is the whole reason this may live in a file at all: naming a variable
    /// takes something away from a subprocess and can grant nothing. A value here could put a
    /// credential in front of every command instead, which is what the `env` block declines to do.
    pub fn scrubbed(&self) -> impl Iterator<Item = &str> {
        self.scrub.iter().map(String::as_str)
    }

    /// Every name the file set, for `doctor` to report.
    ///
    /// Names only. The values include credentials on some machines, and a diagnostic that prints
    /// them is a diagnostic people paste into issues.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.env.keys().map(String::as_str)
    }
}

/// The `run.scrubEnv` array: names a file says to keep from a program the agent runs.
///
/// Strings only, and empty where the block is absent or shaped differently. A malformed entry is
/// dropped rather than refused, on the same footing as everything else here: a half-typed file must
/// not stop a session, and the built-in set still holds whatever this says.
fn scrub_list(root: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    let Some(serde_json::Value::Object(run)) = root.get("run") else {
        return Vec::new();
    };
    let Some(serde_json::Value::Array(names)) = run.get("scrubEnv") else {
        return Vec::new();
    };
    names
        .iter()
        .filter_map(|name| match name {
            serde_json::Value::String(name) if !name.trim().is_empty() => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// The `permissions` block: three lists of rule text, and the directories to open.
///
/// Strings only, and a malformed entry is dropped rather than refused, on the same footing as
/// everything else here. A rule that is not a string cannot be matched against anything, and
/// refusing the file over one would take away the rules that were readable.
///
/// `defaultMode` is read by nothing yet. A file setting it is not an error and not a warning here:
/// [`Settings::parse`] reads what the file says and reports it, and which modes exist is a
/// question for whoever consults them.
fn permission_lists(root: &serde_json::Map<String, serde_json::Value>) -> PermissionLists {
    let Some(serde_json::Value::Object(block)) = root.get("permissions") else {
        return PermissionLists::default();
    };
    PermissionLists {
        deny: strings(block, "deny"),
        ask: strings(block, "ask"),
        allow: strings(block, "allow"),
        additional_directories: strings(block, "additionalDirectories"),
    }
}

/// One array of non-empty strings out of a block, or empty for every other shape.
fn strings(block: &serde_json::Map<String, serde_json::Value>, name: &str) -> Vec<String> {
    let Some(serde_json::Value::Array(entries)) = block.get(name) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| match entry {
            serde_json::Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// The global state directory, or `None` when there is no home to look in.
///
/// No fallback: a relative `.bravebot` would be a different directory per working directory, which
/// is the opposite of what this file is for.
fn home() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".bravebot"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The point of the file: a block copied from `~/.claude/settings.json` configures this agent
    /// without being rewritten first.
    #[test]
    fn an_env_block_is_read() {
        let settings = Settings::parse(
            r#"{"env": {"AWS_REGION": "us-west-2", "AWS_PROFILE": "some-profile"}}"#,
        );
        assert_eq!(settings.get("AWS_REGION"), Some("us-west-2"));
        assert_eq!(settings.get("AWS_PROFILE"), Some("some-profile"));
    }

    /// Every name is read rather than a chosen subset. The file is the user's own configuration
    /// surface, and a settings file that silently drops what it was told is worse than one that
    /// reads a name nothing happens to consult.
    #[test]
    fn a_name_this_crate_does_not_know_is_still_read() {
        let settings = Settings::parse(r#"{"env": {"SOMETHING_ELSE": "value"}}"#);
        assert_eq!(settings.get("SOMETHING_ELSE"), Some("value"));
    }

    /// Settings files carry other blocks. One this crate does not read must not stop it finding
    /// the one it does.
    #[test]
    fn other_blocks_are_ignored() {
        let settings = Settings::parse(r#"{"model": "opus", "env": {"AWS_REGION": "us-west-2"}}"#);
        assert_eq!(settings.get("AWS_REGION"), Some("us-west-2"));
    }

    /// A variable is a string. Coercing a number or a boolean would invent a spelling the writer
    /// did not choose, and `1` is not obviously `"1"` to whoever debugs it later.
    #[test]
    fn a_value_that_is_not_a_string_is_left_out() {
        let settings = Settings::parse(r#"{"env": {"A": 1, "B": true, "C": null, "D": "yes"}}"#);
        assert_eq!(settings.get("A"), None);
        assert_eq!(settings.get("B"), None);
        assert_eq!(settings.get("C"), None);
        assert_eq!(settings.get("D"), Some("yes"));
    }

    /// Every failure reads as absence, because the process environment and the built-in values
    /// still describe a working backend. A half-typed settings file must not stop a session.
    #[test]
    fn anything_unparseable_reads_as_no_settings_at_all() {
        for text in [
            "",
            "   ",
            "not json",
            "{",
            "[]",
            "null",
            r#"{"env": "not a block"}"#,
            r#"{"env": []}"#,
            r#"{"no_env_here": {"AWS_REGION": "us-west-2"}}"#,
        ] {
            assert!(
                Settings::parse(text).is_empty(),
                "{text:?} was read as settings"
            );
        }
    }

    /// A machine with no home directory has no settings file, and that is not an error.
    #[test]
    fn no_home_directory_is_not_an_error() {
        assert!(Settings::from_home(None).is_empty());
    }

    /// Nothing is there to read on a fresh machine, which is the common case and must be quiet.
    #[test]
    fn a_missing_file_is_not_an_error() {
        let missing = std::env::temp_dir().join("bravebot-settings-absent");
        assert!(Settings::from_home(Some(missing)).is_empty());
    }

    /// The escape hatch for a setup that needs a variable the built-in set removes: a person names
    /// their own, and those are kept from a program the agent runs too.
    #[test]
    fn a_file_may_name_variables_to_keep_from_a_program() {
        let settings = Settings::parse(r#"{"run": {"scrubEnv": ["MY_TOKEN", "OTHER_SECRET"]}}"#);
        let named: Vec<&str> = settings.scrubbed().collect();
        assert_eq!(named, ["MY_TOKEN", "OTHER_SECRET"]);
    }

    /// The two blocks are independent. A file that only says what to withhold from a subprocess
    /// configures no backend, and reading it must not depend on an `env` block being present.
    #[test]
    fn a_scrub_list_is_read_without_an_env_block() {
        let settings = Settings::parse(r#"{"run": {"scrubEnv": ["MY_TOKEN"]}}"#);
        assert_eq!(settings.get("AWS_REGION"), None);
        assert_eq!(settings.scrubbed().collect::<Vec<_>>(), ["MY_TOKEN"]);
        assert!(!settings.is_empty());
    }

    /// A name is a string, and an entry that is not one is dropped rather than refusing the file:
    /// the built-in set still holds whatever else the block says.
    #[test]
    fn a_scrub_entry_that_is_not_a_name_is_left_out() {
        let settings =
            Settings::parse(r#"{"run": {"scrubEnv": ["KEEP", 1, true, null, "", "  ", "ALSO"]}}"#);
        assert_eq!(settings.scrubbed().collect::<Vec<_>>(), ["KEEP", "ALSO"]);
    }

    /// Every shape that is not a list of names reads as an empty list, on the same footing as the
    /// rest of this file: a half-typed settings file must not stop a session.
    #[test]
    fn a_malformed_scrub_block_names_nothing() {
        for text in [
            r#"{"run": {}}"#,
            r#"{"run": {"scrubEnv": {}}}"#,
            r#"{"run": {"scrubEnv": "MY_TOKEN"}}"#,
            r#"{"run": "not a block"}"#,
            r#"{"run": []}"#,
            r#"{"scrubEnv": ["MY_TOKEN"]}"#,
        ] {
            assert_eq!(
                Settings::parse(text).scrubbed().count(),
                0,
                "{text:?} named something"
            );
        }
    }

    /// The block a person copies out of `~/.claude/settings.json`, read without being rewritten
    /// first, which is the whole reason this file has the shape it has.
    #[test]
    fn a_permissions_block_is_read() {
        let settings = Settings::parse(
            r#"{
              "permissions": {
                "defaultMode": "acceptEdits",
                "allow": ["Bash(git diff *)", "Bash(npm test *)"],
                "ask": ["Bash(git push *)"],
                "deny": ["Read(./.env)", "Read(./.env.*)"],
                "additionalDirectories": ["../shared"]
              }
            }"#,
        );
        let permissions = settings.permissions();
        assert_eq!(permissions.allow, ["Bash(git diff *)", "Bash(npm test *)"]);
        assert_eq!(permissions.ask, ["Bash(git push *)"]);
        assert_eq!(permissions.deny, ["Read(./.env)", "Read(./.env.*)"]);
        assert_eq!(permissions.additional_directories, ["../shared"]);
        assert!(!settings.is_empty());
    }

    /// Each list stands alone. A file that only refuses things configures no backend and grants
    /// nothing, and reading it must not depend on the other lists being there.
    #[test]
    fn one_list_is_read_without_the_others() {
        let settings = Settings::parse(r#"{"permissions": {"deny": ["Read(./.env)"]}}"#);
        let permissions = settings.permissions();
        assert_eq!(permissions.deny, ["Read(./.env)"]);
        assert!(permissions.allow.is_empty());
        assert!(permissions.ask.is_empty());
        assert!(!settings.is_empty());
    }

    /// A rule is a string. An entry that is not one cannot be matched against anything, and
    /// dropping it keeps the rules that were readable rather than losing the file over one.
    #[test]
    fn an_entry_that_is_not_a_rule_is_left_out() {
        let settings = Settings::parse(
            r#"{"permissions": {"deny": ["Read(./.env)", 1, true, null, "", "  ", []]}}"#,
        );
        assert_eq!(settings.permissions().deny, ["Read(./.env)"]);
    }

    /// Every shape that is not a block of lists reads as no rules at all, on the same footing as
    /// the rest of this file: a half-typed settings file must not stop a session.
    #[test]
    fn a_malformed_permissions_block_carries_no_rules() {
        for text in [
            r#"{"permissions": {}}"#,
            r#"{"permissions": []}"#,
            r#"{"permissions": "deny everything"}"#,
            r#"{"permissions": {"deny": "Read(./.env)"}}"#,
            r#"{"permissions": {"deny": {}}}"#,
            r#"{"permissions": {"unknown": ["Read(./.env)"]}}"#,
            r#"{"deny": ["Read(./.env)"]}"#,
        ] {
            assert!(
                Settings::parse(text).permissions().is_empty(),
                "{text:?} carried a rule"
            );
        }
    }

    /// The blocks are independent of each other. A file that configures a backend and says nothing
    /// about permissions has no rules, and the reverse.
    #[test]
    fn the_permissions_block_and_the_env_block_do_not_need_each_other() {
        let settings = Settings::parse(r#"{"permissions": {"deny": ["Bash"]}}"#);
        assert_eq!(settings.get("AWS_REGION"), None);
        assert_eq!(settings.permissions().deny, ["Bash"]);

        let settings = Settings::parse(r#"{"env": {"AWS_REGION": "us-west-2"}}"#);
        assert!(settings.permissions().is_empty());
        assert_eq!(settings.get("AWS_REGION"), Some("us-west-2"));
    }

    /// `doctor` reports which names a file set. It must not report what they were: on some
    /// machines a value here is a credential, and a diagnostic that prints one is a diagnostic
    /// people paste into issues.
    #[test]
    fn the_names_are_reportable_and_the_values_are_not() {
        let settings = Settings::parse(r#"{"env": {"AWS_PROFILE": "a-secret-looking-value"}}"#);
        let reported: Vec<&str> = settings.names().collect();
        assert_eq!(reported, ["AWS_PROFILE"]);
        assert!(!format!("{reported:?}").contains("a-secret-looking-value"));
    }
}
