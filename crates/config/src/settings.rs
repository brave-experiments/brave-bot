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

    /// Read an `env` block out of settings JSON.
    ///
    /// Only string values are taken. JSON allows a number or a boolean where a variable wants a
    /// string, and coercing one would invent a spelling the writer did not choose: `1` and `true`
    /// are not obviously `"1"` and `"true"` to whoever has to debug it later.
    pub fn parse(text: &str) -> Self {
        let Ok(serde_json::Value::Object(root)) = serde_json::from_str(text) else {
            return Self::default();
        };
        let Some(serde_json::Value::Object(block)) = root.get("env") else {
            return Self::default();
        };
        let env = block
            .iter()
            .filter_map(|(name, value)| match value {
                serde_json::Value::String(value) => Some((name.clone(), value.clone())),
                _ => None,
            })
            .collect();
        Self { env }
    }

    /// What this file says a variable is, if it says anything.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.env.get(name).map(String::as_str)
    }

    /// Whether the file set anything at all.
    pub fn is_empty(&self) -> bool {
        self.env.is_empty()
    }

    /// Every name the file set, for `doctor` to report.
    ///
    /// Names only. The values include credentials on some machines, and a diagnostic that prints
    /// them is a diagnostic people paste into issues.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.env.keys().map(String::as_str)
    }
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
