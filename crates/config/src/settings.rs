//! The settings files, from `~/.bravebot` and from `.bravebot` beside the work.
//!
//! A file rather than only the process environment, because the values that select a backend are
//! long-lived: which AWS profile to assume and which model each tier names are properties of a
//! person's account, not of the shell a session happened to start in. Exporting them from a shell
//! profile works and keeps working; this exists so it is not the only way.
//!
//! Three files, because an account is not the only scope a value belongs to. A profile is a
//! property of the person, a gateway a particular checkout talks to is a property of that checkout,
//! and something one machine needs is neither. The order is Claude Code's, and so are the merge
//! rules: `~/.bravebot/settings.json`, then `.bravebot/settings.json`, then
//! `.bravebot/settings.local.json`, each overriding the one before it a name at a time rather than
//! wholesale. Copying that resolution rather than inventing one means somebody who knows where to
//! put a value for one of these tools knows it for the other.
//!
//! Blocks borrowing the shape of whichever tool already reads them, so that one copied from
//! elsewhere works unedited rather than being rewritten first. A different spelling for the same
//! values would be a second thing to learn for no gain:
//!
//! - `env`, a flat map of strings, spelled as Claude Code's `~/.claude/settings.json` spells it,
//!   down to the variable names. The switch is this program's own name, since it selects a backend
//!   for this program, but the model names are shared because those name someone's Bedrock
//!   deployment.
//! - `model`, for the same reason: it is where both Claude Code and opencode put that choice, and a
//!   key those tools honour that this one silently dropped is worse than one nobody writes.
//! - `permissions`, whose rules Claude Code spells the same way.
//! - `provider`, in opencode's shape, read by [`crate::provider`].
//!
//! They are independent. A file configuring one has nothing to say about the others, and reading any
//! of them does not depend on another being present.
//!
//! # What these files are trusted for
//!
//! Every name in a block is read, not a chosen subset. These are the user's own configuration
//! surface, on the footing [`crate`]'s callers already treat `~/.bravebot` as: a value is something
//! the person running the agent typed, and it is trusted exactly as far as a variable they exported
//! would be. Nothing a turn produces can write one, and no model output reaches one.
//!
//! A project file is a file in a checkout, which is a weaker claim than a file in a home directory:
//! whoever wrote the checkout wrote it. Nothing here distinguishes them, because the resolution this
//! copies does not. What that costs is written down under Known costs in
//! `docs/specs/backends.md` rather than mitigated here.
//!
//! They do not become the process environment. Values are consulted where a variable would be
//! consulted, and handed to a subprocess only where that subprocess is the thing they configure.
//! Installing them globally would put every name in the block in front of every command `run`
//! ever starts, which is a much larger claim than "this is how I reach the backend".

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The file each layer is named by, inside its own directory.
const SETTINGS_FILE: &str = "settings.json";

/// The machine-local layer, beside the one a checkout can carry.
///
/// A separate name rather than a flag inside the other, so that keeping it out of a checkout is one
/// line in an ignore file and needs no editing of a file somebody else also edits.
const LOCAL_SETTINGS_FILE: &str = "settings.local.json";

/// The directory a checkout keeps its own settings in.
const PROJECT_DIR: &str = ".bravebot";

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
    /// What the top-level `model` key named, if it named anything.
    ///
    /// Separate from `env` because it is not a variable: nothing exports `model`, and folding it
    /// into that map would make it collide with a name someone's shell already uses.
    model: Option<String>,
    /// What the top-level `editorMode` key named, if it named anything.
    ///
    /// The word as the file spelled it, not a mode. Which words name an editing style is a question
    /// for the interface that does the editing, and this crate configures a backend: a name it does
    /// not recognise has to reach the interface to be reported there rather than be dropped here as
    /// though the file had said nothing.
    editor_mode: Option<String>,
    providers: Vec<crate::provider::Provider>,
    layers: Vec<PathBuf>,
    contested: BTreeMap<String, PathBuf>,
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
    /// Directories a file asks to have opened, alongside the working directory.
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
    /// Read every settings layer in force for this user, in this directory.
    pub fn load() -> Self {
        Self::layered(home(), std::env::current_dir().ok().as_deref())
    }

    /// As [`Settings::load`], for a named home and working directory, so a test needs no ambient
    /// ones.
    ///
    /// The working directory is where the process started and not an ancestor of it. A session begun
    /// in a subdirectory therefore reads no project settings, which is the same rule Claude Code
    /// applies and is the reason this walks nothing: a search upward would make what configures a
    /// session depend on which directory somebody happened to `cd` into, and the file it eventually
    /// found could sit above the thing being worked on.
    pub fn layered(home: Option<PathBuf>, cwd: Option<&Path>) -> Self {
        let project = cwd.map(|cwd| cwd.join(PROJECT_DIR));
        let paths = [
            home.map(|home| home.join(SETTINGS_FILE)),
            project.as_ref().map(|dir| dir.join(SETTINGS_FILE)),
            project.as_ref().map(|dir| dir.join(LOCAL_SETTINGS_FILE)),
        ];

        let mut merged = serde_json::Map::new();
        let mut found = Vec::new();
        let mut winner = BTreeMap::new();
        let mut contested = BTreeMap::new();
        for path in paths.into_iter().flatten() {
            let Some(root) = read(&path) else { continue };
            for name in env_names(&root) {
                // Whoever set it before lost it here, which is the only thing worth telling somebody:
                // a name one file sets needs no explanation of where it came from.
                if winner.insert(name.clone(), path.clone()).is_some() {
                    contested.insert(name, path.clone());
                }
            }
            found.push(path);
            merge(&mut merged, root);
        }

        let mut settings = Self::from_map(&merged);
        settings.layers = found;
        settings.contested = contested;
        settings
    }

    /// Read one layer, for a home directory and nothing beside it.
    pub fn from_home(home: Option<PathBuf>) -> Self {
        Self::layered(home, None)
    }

    /// Read the `env` block, the `model` key and the scrub list out of settings JSON.
    ///
    /// Only string values are taken. JSON allows a number or a boolean where a variable wants a
    /// string, and coercing one would invent a spelling the writer did not choose: `1` and `true`
    /// are not obviously `"1"` and `"true"` to whoever has to debug it later.
    ///
    /// Each is read independently, so a file with one and not the others still supplies what it
    /// has: a `model` key is the whole of some people's settings, and requiring an `env` block
    /// beside it would discard it for being alone.
    pub fn parse(text: &str) -> Self {
        let Ok(serde_json::Value::Object(root)) = serde_json::from_str(text) else {
            return Self::default();
        };
        Self::from_map(&root)
    }

    /// The blocks in one settings root, which is a file or several merged into one.
    ///
    /// Merging happens before this, on the JSON, so that every block is read exactly once from a
    /// root that already holds what won. Reading each layer separately and combining the results
    /// afterwards would need this logic twice, once per block, and the second copy is where the two
    /// would drift.
    fn from_map(root: &serde_json::Map<String, serde_json::Value>) -> Self {
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
            scrub: scrub_list(root),
            permissions: permission_lists(root),
            model: word(root, "model"),
            editor_mode: word(root, "editorMode"),
            providers: crate::provider::Provider::all(root),
            layers: Vec::new(),
            contested: BTreeMap::new(),
        }
    }

    /// What the settings in force say a variable is, if they say anything.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.env.get(name).map(String::as_str)
    }

    /// The model the settings in force asked for, if they asked for one.
    ///
    /// A default rather than the model: `/model` records a choice that outlives the session making
    /// it, and that choice wins. This is what answers for somebody who has never made one.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// The editing style the settings in force asked for, if they asked for one.
    ///
    /// The word the file spelled, unrecognised words and all. A default rather than the style in
    /// force: the interface records a choice that outlives the session making it, and that choice
    /// wins. This is what answers for somebody who has never made one.
    pub fn editor_mode(&self) -> Option<&str> {
        self.editor_mode.as_deref()
    }

    /// Whether anything was set at all.
    pub fn is_empty(&self) -> bool {
        self.env.is_empty()
            && self.scrub.is_empty()
            && self.permissions.is_empty()
            && self.model.is_none()
            && self.editor_mode.is_none()
            && self.providers.is_empty()
    }

    /// The rule text and added directories the `permissions` block carried.
    pub fn permissions(&self) -> &PermissionLists {
        &self.permissions
    }

    /// The gateways these settings configured, in the order they were listed.
    pub fn providers(&self) -> &[crate::provider::Provider] {
        &self.providers
    }

    /// The files that were read, weakest first, for `doctor` to report.
    ///
    /// Only the ones that existed and parsed. A layer nobody wrote is absence rather than an entry,
    /// because a diagnostic listing every place a file could have been is a diagnostic where the two
    /// that exist are the hard part to find.
    pub fn layers(&self) -> impl Iterator<Item = &Path> {
        self.layers.iter().map(PathBuf::as_path)
    }

    /// Variables more than one layer set, with the file that won, for `doctor` to report.
    ///
    /// Only the contested ones. A name a single file sets needs no explanation of where it came from,
    /// and listing every name against a path would bury the two that are surprising.
    pub fn overridden(&self) -> impl Iterator<Item = (&str, &Path)> {
        self.contested
            .iter()
            .map(|(name, path)| (name.as_str(), path.as_path()))
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
    /// them is a diagnostic people paste into issues. The keys that are not variables are among them
    /// so a file that sets only one of those is not reported as setting nothing.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.model
            .is_some()
            .then_some("model")
            .into_iter()
            .chain(self.editor_mode.is_some().then_some("editorMode"))
            .chain(self.env.keys().map(String::as_str))
    }
}

/// One layer's JSON, or `None` when there is nothing there worth reading.
///
/// Every failure is the same as absence, per layer rather than for the set: a missing file, an
/// oversized one, a syntax error, or a root that is not an object. A half-typed project file leaves
/// the layers under it in force, because the alternative is a mistake in a checkout deciding that a
/// person's own profile no longer applies.
fn read(path: &Path) -> Option<serde_json::Map<String, serde_json::Value>> {
    match std::fs::metadata(path) {
        Ok(found) if found.len() > MAX_BYTES => return None,
        Ok(_) => {}
        Err(_) => return None,
    }
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(serde_json::Value::Object(root)) => Some(root),
        _ => None,
    }
}

/// A top-level key holding one word, or `None` where the file said nothing usable.
///
/// Strings only, on the footing everything else here reads them: a number or a boolean where a word
/// belongs would have to be given a spelling nobody chose. Blank is absence rather than a choice of
/// nothing, since a key set to `""` is how somebody comments one out without deleting the line.
fn word(root: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    match root.get(key) {
        Some(serde_json::Value::String(word)) => Some(word.trim())
            .filter(|word| !word.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

/// The variables one layer's `env` block sets, for working out which layer won a name.
fn env_names(root: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    match root.get("env") {
        Some(serde_json::Value::Object(block)) => block
            .iter()
            .filter(|(_, value)| value.is_string())
            .map(|(name, _)| name.clone())
            .collect(),
        _ => Vec::new(),
    }
}

/// Lay one settings root over another, a name at a time.
///
/// One level deep, which is Claude Code's rule rather than a general merge: `env` and `provider`
/// combine per name, and a value inside one of those names is replaced whole. So a project file may
/// add a gateway or restate one, and cannot reach inside an inherited gateway to change the host it
/// points at while keeping the rest. A deeper merge would make a request's destination the product of
/// two files, and no single place to read would say where it goes.
///
/// `run.scrubEnv` unions instead, since a name there only ever takes a variable away from a
/// subprocess. Overriding would let a layer hand back something a weaker one withheld, which is a
/// direction this list is not for.
fn merge(
    base: &mut serde_json::Map<String, serde_json::Value>,
    over: serde_json::Map<String, serde_json::Value>,
) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            // `env` and `provider`: per-name, one level down.
            (Some(serde_json::Value::Object(under)), serde_json::Value::Object(above))
                if key == "env" || key == "provider" =>
            {
                under.extend(above);
            }
            // `run` holds one list that unions and nothing else that does, so a sibling added later
            // gets whatever this arm does by default, which is to override.
            (Some(serde_json::Value::Object(under)), serde_json::Value::Object(above))
                if key == "run" =>
            {
                merge_run(under, above);
            }
            // Every rule from every layer, which is the rule the tool this borrows from applies. A
            // layer that replaced the block could drop a `deny` a weaker one set, and a permission
            // taken away by a file somebody did not open is the one outcome worth ruling out.
            (Some(serde_json::Value::Object(under)), serde_json::Value::Object(above))
                if key == "permissions" =>
            {
                merge_permissions(under, above);
            }
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// The `permissions` block, where every list unions and any other name overrides.
///
/// A rule is added by a layer and never removed by one, so `deny` still holds whatever the weakest
/// file said. `additionalDirectories` unions for the same reason it exists: a layer asks for
/// somewhere to work, and the strongest file asking for one place should not un-ask another.
fn merge_permissions(
    under: &mut serde_json::Map<String, serde_json::Value>,
    above: serde_json::Map<String, serde_json::Value>,
) {
    for (key, value) in above {
        match (under.get_mut(&key), value) {
            (Some(serde_json::Value::Array(kept)), serde_json::Value::Array(added)) => {
                kept.extend(added);
            }
            (_, value) => {
                under.insert(key, value);
            }
        }
    }
}

/// The `run` block, where `scrubEnv` unions and every other name overrides.
fn merge_run(
    under: &mut serde_json::Map<String, serde_json::Value>,
    above: serde_json::Map<String, serde_json::Value>,
) {
    for (key, value) in above {
        match (under.get_mut(&key), value) {
            (Some(serde_json::Value::Array(kept)), serde_json::Value::Array(added))
                if key == "scrubEnv" =>
            {
                kept.extend(added);
            }
            (_, value) => {
                under.insert(key, value);
            }
        }
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
/// No fallback to a relative `.bravebot`, which is the project layer and reached deliberately rather
/// than by a home directory going missing. Resolving the weakest layer to the strongest one's
/// location would silently read a checkout's file as though a person had put it in their own
/// directory.
fn home() -> Option<PathBuf> {
    home_named(std::env::var_os("HOME"))
}

/// The same answer, from the value rather than from the variable.
///
/// Split from the read so the rule is testable without a process-wide variable. A test that set
/// `HOME` would have to take a lock against every other test in this binary, restore what was
/// there, and step outside safe Rust to do it, all to check a rule that is a function of one
/// string.
fn home_named(home: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let home = home?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".bravebot"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A home as the environment hands one over.
    fn named(home: &str) -> Option<std::ffi::OsString> {
        Some(std::ffi::OsString::from(home))
    }

    /// STATE-2: this crate resolves the state directory itself, since it sits below the one that
    /// answers where it is. What has to hold across the resolvers is the name, so a layer reading
    /// a settings file finds the same directory a layer writing history does.
    #[test]
    fn the_state_directory_is_the_home_the_environment_names() {
        assert_eq!(
            home_named(named("/somebody/else")),
            Some(PathBuf::from("/somebody/else/.bravebot"))
        );
    }

    /// STATE-2: the other half of the same rule. A fallback here would read settings out of a
    /// directory nobody chose, and settings are what decide which model answers and which
    /// commands run without being asked about.
    #[test]
    fn an_absent_or_empty_home_yields_no_directory_rather_than_a_guess() {
        assert_eq!(home_named(None), None);
        assert_eq!(
            home_named(named("")),
            None,
            "an empty home was joined onto anyway"
        );
    }

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
    /// the ones it does.
    #[test]
    fn other_blocks_are_ignored() {
        let settings = Settings::parse(
            r#"{"permissions": {"allow": []}, "env": {"AWS_REGION": "us-west-2"}}"#,
        );
        assert_eq!(settings.get("AWS_REGION"), Some("us-west-2"));
    }

    /// The key Claude Code and opencode both use for this. Read because a file written for either
    /// of them is the file people have, and a key those tools honour that this one dropped looks
    /// like the setting doing nothing.
    #[test]
    fn a_top_level_model_key_is_read() {
        let settings = Settings::parse(r#"{"model": "opus"}"#);
        assert_eq!(settings.model(), Some("opus"));
    }

    /// The two are independent. A `model` key is the whole of some people's settings, and
    /// requiring an `env` block beside it would discard it for being alone.
    #[test]
    fn a_model_and_an_env_block_are_read_from_the_same_file() {
        let settings = Settings::parse(r#"{"model": "opus", "env": {"AWS_REGION": "us-west-2"}}"#);
        assert_eq!(settings.model(), Some("opus"));
        assert_eq!(settings.get("AWS_REGION"), Some("us-west-2"));
    }

    /// A blank is absence. Sending `""` as a model would be reset by the server anyway, and here it
    /// would shadow the value a release has built in for no stated reason.
    #[test]
    fn a_model_that_is_blank_or_not_a_string_names_nothing() {
        for text in [
            r#"{"model": ""}"#,
            r#"{"model": "   "}"#,
            r#"{"model": 1}"#,
            r#"{"model": true}"#,
            r#"{"model": null}"#,
            r#"{"model": ["opus"]}"#,
            r#"{"model": {"name": "opus"}}"#,
        ] {
            assert_eq!(Settings::parse(text).model(), None, "{text} named a model");
        }
    }

    /// Surrounding space is the shape a hand-edited file has, and a model name with a newline in it
    /// is not one either backend serves.
    #[test]
    fn a_model_is_trimmed() {
        assert_eq!(
            Settings::parse("{\"model\": \" opus\\n\"}").model(),
            Some("opus")
        );
    }

    /// A file that sets only a model has set something, so `doctor` must not report it as absent.
    #[test]
    fn a_file_with_only_a_model_is_not_empty() {
        let settings = Settings::parse(r#"{"model": "opus"}"#);
        assert!(!settings.is_empty());
        assert_eq!(settings.names().collect::<Vec<_>>(), ["model"]);
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
        let missing = crate::testutil::scratch_dir("bravebot-settings-absent");
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

    /// The reason the block is opencode's shape rather than one invented here: a `provider` entry
    /// copied out of `opencode.json` configures this agent without being rewritten first.
    #[test]
    fn a_provider_block_is_read_beside_the_env_block() {
        let settings = Settings::parse(
            r#"{
                "env": {"AWS_REGION": "us-west-2"},
                "provider": {"openrouter": {
                    "options": {"baseURL": "https://openrouter.ai/api/v1"},
                    "models": {"z-ai/glm-4.6": {}}
                }}
            }"#,
        );
        assert_eq!(settings.get("AWS_REGION"), Some("us-west-2"));
        assert_eq!(settings.providers().len(), 1);
        assert!(settings.providers()[0].offers("z-ai/glm-4.6"));
    }

    /// The two blocks are independent. A file that only configures a gateway names no variables, and
    /// reading it must not depend on an `env` block being present.
    #[test]
    fn a_provider_block_is_read_without_an_env_block() {
        let settings = Settings::parse(
            r#"{"provider": {"gw": {"options": {"baseURL": "https://example.invalid/v1"}}}}"#,
        );
        assert_eq!(settings.get("AWS_REGION"), None);
        assert_eq!(settings.providers().len(), 1);
        assert!(!settings.is_empty());
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

    /// The layers on disk, for the tests below.
    ///
    /// Named directories rather than an ambient home, so nothing here reads or writes the settings of
    /// whoever is running the tests, and two of these can run at once.
    struct Layers {
        home: PathBuf,
        cwd: PathBuf,
    }

    impl Layers {
        /// A scratch home and working directory, empty of every layer.
        fn new(name: &str) -> Self {
            let root = crate::testutil::scratch_dir(&format!("bravebot-layers-{name}"));
            let _ = std::fs::remove_dir_all(&root);
            let home = root.join("home");
            let project = root.join("cwd").join(PROJECT_DIR);
            std::fs::create_dir_all(&home).expect("scratch home");
            std::fs::create_dir_all(&project).expect("scratch project");
            Self {
                home,
                cwd: root.join("cwd"),
            }
        }

        fn global(self, text: &str) -> Self {
            std::fs::write(self.home.join(SETTINGS_FILE), text).expect("global layer");
            self
        }

        fn project(self, text: &str) -> Self {
            std::fs::write(self.cwd.join(PROJECT_DIR).join(SETTINGS_FILE), text)
                .expect("project layer");
            self
        }

        fn local(self, text: &str) -> Self {
            std::fs::write(self.cwd.join(PROJECT_DIR).join(LOCAL_SETTINGS_FILE), text)
                .expect("local layer");
            self
        }

        fn read(&self) -> Settings {
            Settings::layered(Some(self.home.clone()), Some(&self.cwd))
        }
    }

    /// The point of a project layer: a checkout says which gateway or profile the work in it uses,
    /// and that beats what the person set for everything else they do.
    #[test]
    fn a_project_layer_overrides_a_name_the_global_one_set() {
        let settings = Layers::new("project-wins")
            .global(r#"{"env": {"AWS_PROFILE": "personal"}}"#)
            .project(r#"{"env": {"AWS_PROFILE": "this-checkout"}}"#)
            .read();
        assert_eq!(settings.get("AWS_PROFILE"), Some("this-checkout"));
    }

    /// A name at a time, not a file at a time. A project file saying one thing must not discard the
    /// rest of somebody's configuration, which is what makes putting one value in a checkout
    /// worthwhile at all.
    #[test]
    fn a_name_only_the_global_layer_set_survives_a_project_layer() {
        let settings = Layers::new("global-survives")
            .global(r#"{"env": {"AWS_REGION": "us-west-2", "AWS_PROFILE": "personal"}}"#)
            .project(r#"{"env": {"AWS_PROFILE": "this-checkout"}}"#)
            .read();
        assert_eq!(settings.get("AWS_REGION"), Some("us-west-2"));
        assert_eq!(settings.get("AWS_PROFILE"), Some("this-checkout"));
    }

    /// The reason the local layer exists: something true of this machine only, which would be wrong
    /// for anybody else who checked the project out.
    #[test]
    fn the_local_layer_beats_the_one_a_checkout_carries() {
        let settings = Layers::new("local-wins")
            .global(r#"{"env": {"AWS_PROFILE": "personal"}}"#)
            .project(r#"{"env": {"AWS_PROFILE": "shared"}}"#)
            .local(r#"{"env": {"AWS_PROFILE": "just-this-machine"}}"#)
            .read();
        assert_eq!(settings.get("AWS_PROFILE"), Some("just-this-machine"));
    }

    /// Naming a variable here only ever takes it away from a subprocess, so the layers add up. An
    /// override would let a project file hand back a secret the person's own file withheld.
    #[test]
    fn every_layer_adds_to_the_names_kept_from_a_program() {
        let settings = Layers::new("scrub-union")
            .global(r#"{"run": {"scrubEnv": ["PERSONAL_TOKEN"]}}"#)
            .project(r#"{"run": {"scrubEnv": ["PROJECT_TOKEN"]}}"#)
            .local(r#"{"run": {"scrubEnv": ["MACHINE_TOKEN"]}}"#)
            .read();
        let mut named: Vec<&str> = settings.scrubbed().collect();
        named.sort_unstable();
        assert_eq!(named, ["MACHINE_TOKEN", "PERSONAL_TOKEN", "PROJECT_TOKEN"]);
    }

    /// A gateway is replaced by name, and the others stay. Merging deeper would make one request's
    /// destination the product of two files, with no single place to read that says where it goes.
    #[test]
    fn a_project_layer_replaces_one_gateway_and_leaves_the_others() {
        let settings = Layers::new("provider-by-id")
            .global(
                r#"{"provider": {
                    "personal": {"options": {"baseURL": "https://personal.invalid/v1"}},
                    "shared": {"options": {"baseURL": "https://shared.invalid/v1"}}
                }}"#,
            )
            .project(
                r#"{"provider": {
                    "shared": {"options": {"baseURL": "https://this-checkout.invalid/v1"}}
                }}"#,
            )
            .read();

        let mut hosts: Vec<(&str, &str)> = settings
            .providers()
            .iter()
            .map(|provider| (provider.id.as_str(), provider.base_url.as_str()))
            .collect();
        hosts.sort_unstable();
        assert_eq!(
            hosts,
            [
                ("personal", "https://personal.invalid/v1"),
                ("shared", "https://this-checkout.invalid/v1"),
            ]
        );
    }

    /// A gateway entry is replaced whole, so a project file naming one has to name the host too. A
    /// deeper merge would leave an entry no single file describes.
    #[test]
    fn a_project_gateway_naming_no_host_replaces_one_that_did() {
        let settings = Layers::new("provider-whole")
            .global(
                r#"{"provider": {"gw": {"options": {"baseURL": "https://personal.invalid/v1"}}}}"#,
            )
            .project(r#"{"provider": {"gw": {"models": {"some-model": {}}}}}"#)
            .read();
        assert!(settings.providers().is_empty());
    }

    /// A mistake in a checkout must not decide that somebody's own profile no longer applies, which
    /// is what refusing the whole stack over one bad layer would do.
    #[test]
    fn an_unparseable_project_layer_leaves_the_global_one_in_force() {
        let settings = Layers::new("bad-project")
            .global(r#"{"env": {"AWS_PROFILE": "personal"}}"#)
            .project("{ not json at all")
            .read();
        assert_eq!(settings.get("AWS_PROFILE"), Some("personal"));
    }

    /// The bound is per layer, on the same footing as a syntax error: a file that grew by accident
    /// in a checkout is refused, and nothing else is.
    #[test]
    fn an_oversized_project_layer_leaves_the_global_one_in_force() {
        let padding = " ".repeat(MAX_BYTES as usize + 1);
        let settings = Layers::new("big-project")
            .global(r#"{"env": {"AWS_PROFILE": "personal"}}"#)
            .project(&format!(
                r#"{{"env": {{"AWS_PROFILE": "too-big"}}}}{padding}"#
            ))
            .read();
        assert_eq!(settings.get("AWS_PROFILE"), Some("personal"));
    }

    /// Somebody working in a directory that carries no settings gets exactly what they had before
    /// any of this existed.
    #[test]
    fn a_directory_with_no_project_layer_reads_the_global_one_alone() {
        let settings = Layers::new("no-project")
            .global(r#"{"env": {"AWS_PROFILE": "personal"}}"#)
            .read();
        assert_eq!(settings.get("AWS_PROFILE"), Some("personal"));
        assert_eq!(settings.layers().count(), 1);
    }

    /// `doctor` says which files are in force, weakest first, so that somebody looking at a value
    /// they did not expect knows which of three files to open.
    #[test]
    fn the_layers_that_were_read_are_reported_weakest_first() {
        let layers = Layers::new("report-order")
            .global(r#"{"env": {"A": "1"}}"#)
            .project(r#"{"env": {"B": "2"}}"#)
            .local(r#"{"env": {"C": "3"}}"#);
        let settings = layers.read();

        let reported: Vec<PathBuf> = settings.layers().map(Path::to_path_buf).collect();
        assert_eq!(
            reported,
            [
                layers.home.join(SETTINGS_FILE),
                layers.cwd.join(PROJECT_DIR).join(SETTINGS_FILE),
                layers.cwd.join(PROJECT_DIR).join(LOCAL_SETTINGS_FILE),
            ]
        );
    }

    /// A layer nobody wrote is absence rather than an entry, so the list names files that exist and
    /// somebody reading it is not hunting through places one could have been.
    #[test]
    fn a_layer_that_is_not_there_is_not_reported() {
        let settings = Layers::new("report-absent")
            .global(r#"{"env": {"A": "1"}}"#)
            .local(r#"{"env": {"C": "3"}}"#)
            .read();
        assert_eq!(settings.layers().count(), 2);
    }

    /// A permission rule is added by a layer and never removed by one. A file that replaced the block
    /// could drop a `deny` a weaker one set, and a permission taken away by a file somebody did not
    /// open is the one outcome worth ruling out.
    #[test]
    fn every_layer_adds_to_the_permission_rules() {
        let settings = Layers::new("permissions-union")
            .global(r#"{"permissions": {"deny": ["run(rm)"], "allow": ["run(ls)"]}}"#)
            .project(r#"{"permissions": {"deny": ["run(curl)"]}}"#)
            .local(r#"{"permissions": {"ask": ["run(git push)"]}}"#)
            .read();

        let rules = settings.permissions();
        let mut denied = rules.deny.clone();
        denied.sort();
        assert_eq!(denied, ["run(curl)", "run(rm)"]);
        assert_eq!(rules.allow, ["run(ls)"]);
        assert_eq!(rules.ask, ["run(git push)"]);
    }

    /// A directory one layer made reachable stays reachable when a stronger layer names another, since
    /// naming somewhere to reach is not a statement about anywhere else.
    #[test]
    fn every_layer_adds_to_the_directories_a_file_makes_reachable() {
        let settings = Layers::new("directories-union")
            .global(r#"{"permissions": {"additionalDirectories": ["/one"]}}"#)
            .project(r#"{"permissions": {"additionalDirectories": ["/two"]}}"#)
            .read();
        let mut named = settings.permissions().additional_directories.clone();
        named.sort();
        assert_eq!(named, ["/one", "/two"]);
    }

    /// A style of editing is one choice, so it resolves the way every other single value does. The
    /// word is handed on as the file spelled it: which words name a style is a question for the
    /// interface that does the editing, and one this crate did not recognise has to reach it to be
    /// reported rather than be dropped here as though the file had said nothing.
    #[test]
    fn a_style_of_editing_resolves_like_any_other_single_value() {
        let settings = Layers::new("editing-override")
            .global(r#"{"editorMode": "emacs"}"#)
            .project(r#"{"editorMode": "vim"}"#)
            .read();
        assert_eq!(settings.editor_mode(), Some("vim"));

        let settings = Layers::new("editing-survives")
            .global(r#"{"editorMode": "vim"}"#)
            .project(r#"{"model": "this-checkout"}"#)
            .read();
        assert_eq!(settings.editor_mode(), Some("vim"));

        let settings = Layers::new("editing-unnamed")
            .global(r#"{"model": "personal-choice"}"#)
            .read();
        assert_eq!(settings.editor_mode(), None);
    }

    /// Setting a key to nothing is how somebody comments one out without deleting the line, so a blank
    /// is absence rather than a choice of nothing. Read as a choice, it would name no style anyway, but
    /// `doctor` would report the file as having set something.
    #[test]
    fn a_blank_value_is_not_a_choice() {
        assert_eq!(Settings::parse(r#"{"editorMode": ""}"#).editor_mode(), None);
        assert_eq!(
            Settings::parse(r#"{"editorMode": "   "}"#).editor_mode(),
            None
        );
        assert!(Settings::parse(r#"{"editorMode": ""}"#).is_empty());
    }

    /// A file that sets only this is not a file that set nothing: `doctor` reports which names a layer
    /// carried, and a person debugging why their box edits the way it does has to see it there.
    #[test]
    fn a_style_of_editing_is_among_the_names_reported() {
        let settings = Settings::parse(r#"{"editorMode": "vim"}"#);
        let reported: Vec<&str> = settings.names().collect();
        assert_eq!(reported, ["editorMode"]);
        assert!(!settings.is_empty());
    }

    /// A model is one choice rather than a list, so the closest layer that names one wins: a checkout
    /// saying which model its work wants is the whole point of naming it there.
    #[test]
    fn the_closest_layer_that_named_a_model_wins() {
        let settings = Layers::new("model-override")
            .global(r#"{"model": "personal-choice"}"#)
            .project(r#"{"model": "this-checkout"}"#)
            .read();
        assert_eq!(settings.model(), Some("this-checkout"));
    }

    /// A layer that says nothing about the model leaves the one a weaker layer named, on the same
    /// footing as every other name.
    #[test]
    fn a_layer_naming_no_model_leaves_the_one_below_it() {
        let settings = Layers::new("model-survives")
            .global(r#"{"model": "personal-choice"}"#)
            .project(r#"{"env": {"AWS_PROFILE": "this-checkout"}}"#)
            .read();
        assert_eq!(settings.model(), Some("personal-choice"));
    }

    /// The reason to report an override at all: somebody seeing a value they did not set has three
    /// files it could be in, and only the winning path narrows it to one.
    #[test]
    fn a_name_more_than_one_layer_set_reports_the_file_that_won() {
        let layers = Layers::new("report-override")
            .global(r#"{"env": {"AWS_PROFILE": "personal", "AWS_REGION": "us-west-2"}}"#)
            .project(r#"{"env": {"AWS_PROFILE": "this-checkout"}}"#);
        let settings = layers.read();

        let reported: Vec<(&str, PathBuf)> = settings
            .overridden()
            .map(|(name, path)| (name, path.to_path_buf()))
            .collect();
        assert_eq!(
            reported,
            [(
                "AWS_PROFILE",
                layers.cwd.join(PROJECT_DIR).join(SETTINGS_FILE)
            )]
        );
    }

    /// A name only one layer set needs no explanation of where it came from, and listing every name
    /// against a path would bury the one that is surprising.
    #[test]
    fn a_name_a_single_layer_set_is_not_reported_as_overridden() {
        let settings = Layers::new("report-uncontested")
            .global(r#"{"env": {"AWS_REGION": "us-west-2"}}"#)
            .project(r#"{"env": {"AWS_PROFILE": "this-checkout"}}"#)
            .read();
        assert_eq!(settings.overridden().count(), 0);
    }

    /// `doctor` prints these, so an override must say which file won and never what it said: on some
    /// machines the value is a credential.
    #[test]
    fn an_override_reports_the_name_and_the_file_and_never_the_value() {
        let settings = Layers::new("report-override-values")
            .global(r#"{"env": {"AWS_PROFILE": "the-old-value"}}"#)
            .project(r#"{"env": {"AWS_PROFILE": "a-secret-looking-value"}}"#)
            .read();

        let reported = format!("{:?}", settings.overridden().collect::<Vec<_>>());
        assert!(reported.contains("AWS_PROFILE"));
        assert!(!reported.contains("a-secret-looking-value"));
        assert!(!reported.contains("the-old-value"));
    }
}
