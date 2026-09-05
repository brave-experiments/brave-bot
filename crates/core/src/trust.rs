//! Which paths a user vouches for.
//!
//! The kernel decides *whether* a write needs a person's approval by asking two questions:
//! is the destination somewhere the user vouched for, and is the data trusted? A write of
//! trusted data to a trusted path is ordinary work and needs no interruption. Anything else
//! is shown to a person, because that is where a mistake cannot be undone by a later step.
//!
//! # Most specific rule wins
//!
//! Trust is recorded per path prefix, and the *longest* matching prefix decides. Both
//! polarities are expressible, so a trusted project may contain an untrusted subtree, and
//! that subtree may in turn contain a trusted path again:
//!
//! ```text
//!   .            (unset)
//!   src          trusted     → src/main.rs is trusted
//!   src/vendor   untrusted   → src/vendor/lib.js is untrusted
//!   src/vendor/ours.js trusted → trusted again, being more specific
//! ```
//!
//! # Why writing untrusted data marks the destination untrusted
//!
//! Without that rule the trust store would launder data. A turn could fetch a page, write
//! it into a trusted directory, read it back (now labelled trusted, because the path says
//! so) and use it for routing. Recording the destination as untrusted when untrusted bytes
//! land there closes the loop: what comes out of a file is never more trusted than what
//! went in.
//!
//! This crate performs no I/O; paths here are workspace-relative strings, compared by
//! segment. The caller is responsible for having resolved and confined them first.

use crate::label::Integrity;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A user's decisions about which paths are trustworthy.
///
/// Empty by default, which means nothing is trusted and every write is shown to a person.
/// That is the right default: trust has to be granted, never assumed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustStore {
    /// Normalised path prefix → whether that subtree is trusted.
    ///
    /// A `BTreeMap` rather than a hash map so iteration order is deterministic, which keeps
    /// the audit trail reproducible.
    rules: BTreeMap<String, Integrity>,
}

impl TrustStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `path` and everything beneath it is trusted.
    pub fn trust(&mut self, path: &str) {
        self.rules.insert(normalise(path), Integrity::Trusted);
    }

    /// Record that `path` and everything beneath it is untrusted.
    ///
    /// Used both for an explicit user decision and for the automatic marking that happens
    /// when untrusted bytes are written somewhere.
    pub fn distrust(&mut self, path: &str) {
        self.rules.insert(normalise(path), Integrity::Untrusted);
    }

    /// The integrity of `path`, by the longest matching rule.
    ///
    /// `None` when no rule covers it, which the caller should treat as untrusted. This
    /// returns an option rather than defaulting so a caller cannot silently confuse "the
    /// user vouched for this" with "nobody has said".
    pub fn integrity_of(&self, path: &str) -> Option<Integrity> {
        let path = normalise(path);
        self.rules
            .iter()
            .filter(|(prefix, _)| covers(prefix, &path))
            .max_by_key(|(prefix, _)| specificity(prefix))
            .map(|(_, integrity)| *integrity)
    }

    /// Whether `path` is trusted. Anything not covered by a rule is not.
    pub fn is_trusted(&self, path: &str) -> bool {
        self.integrity_of(path) == Some(Integrity::Trusted)
    }

    /// Whether any path has been vouched for at all.
    ///
    /// Distinguishes "the user declined" from "the user trusted nothing yet", which callers
    /// report differently.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The rules, most general first, for display and for the audit trail.
    pub fn rules(&self) -> impl Iterator<Item = (&str, Integrity)> {
        self.rules
            .iter()
            .map(|(prefix, integrity)| (prefix.as_str(), *integrity))
    }

    /// The same decisions, said about the same files, once the working directory has moved.
    ///
    /// A relative rule means a path under the working directory, so a working directory that moves
    /// takes every relative rule with it and leaves each one pointing at a file nobody decided
    /// anything about. Carrying the map across unchanged would be the worst of both: the yes given
    /// for one project would vouch for another, and every no given inside the old one would be
    /// forgotten.
    ///
    /// So each rule is resolved to the file it was always about, and then written down again the
    /// way the new working directory names it: relatively where it is inside `to`, absolutely
    /// where it is not. Nothing is granted and nothing is withdrawn; the same files come out with
    /// the same answers, which is what makes this a re-spelling rather than a decision.
    ///
    /// `from` and `to` are absolute paths, and the caller is responsible for having canonicalised
    /// them: two spellings of one directory would rebase into two rules about one tree.
    pub fn rebased(&self, from: &Path, to: &Path) -> Self {
        let mut moved = Self::new();
        for (key, integrity) in self.rules() {
            let absolute = match is_absolute(key) {
                true => PathBuf::from(key),
                false => from.join(key),
            };
            // Component-wise, so `/work/srcfoo` is not taken to be inside `/work/src`, which is the
            // same whole-segment rule `covers` applies and for the same reason.
            let rekeyed = match absolute.strip_prefix(to) {
                Ok(inside) => inside.to_string_lossy().into_owned(),
                Err(_) => absolute.to_string_lossy().into_owned(),
            };
            match integrity {
                Integrity::Trusted => moved.trust(&rekeyed),
                Integrity::Untrusted => moved.distrust(&rekeyed),
            }
        }
        moved
    }
}

/// Whether a normalised key names an absolute path rather than a workspace-relative one.
///
/// The two are separate namespaces, and nothing is a member of both. A relative key is a path
/// under the primary root; an absolute key is a path in a directory the user added by name.
fn is_absolute(key: &str) -> bool {
    key.starts_with('/')
}

/// Normalise a path for comparison.
///
/// Relative paths lose a leading `./` and any surrounding slashes, so `./src/`, `src` and `src/`
/// are one rule rather than three that shadow each other confusingly.
///
/// An absolute path **keeps** its leading slash, which is what makes it a different rule from the
/// relative path spelled the same way. Collapsing the two would be a security bug rather than an
/// inconvenience: stripping the slash turns `/` into the empty key, which is the primary root's own
/// rule, so trusting one added directory would silently trust the entire workspace.
fn normalise(path: &str) -> String {
    let trimmed = path.trim();
    if let Some(rest) = trimmed.strip_prefix('/') {
        let rest = rest.trim_end_matches('/');
        return format!("/{rest}");
    }
    let trimmed = trimmed.strip_prefix("./").unwrap_or(trimmed);
    let trimmed = trimmed.trim_matches('/');
    if trimmed == "." || trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_string()
    }
}

/// Whether `prefix` covers `path`, matching whole segments only.
///
/// Segment-wise so `src` does not cover `srcfoo`, which a plain string prefix test would
/// wrongly accept, and that mistake would hand trust to a path the user never named.
fn covers(prefix: &str, path: &str) -> bool {
    // Neither namespace says anything about the other: a rule about the workspace cannot decide a
    // path in an added directory, and the reverse.
    if is_absolute(prefix) != is_absolute(path) {
        return false;
    }
    // The primary root covers every relative path; `/` covers every absolute one.
    if prefix.is_empty() || prefix == "/" {
        return true;
    }
    if path == prefix {
        return true;
    }
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// How specific a rule is. Deeper rules win; a namespace's own root is least specific.
fn specificity(prefix: &str) -> usize {
    if prefix.is_empty() || prefix == "/" {
        0
    } else {
        prefix.trim_start_matches('/').split('/').count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_store_trusts_nothing() {
        let store = TrustStore::new();
        assert!(store.is_empty());
        assert_eq!(store.integrity_of("src/main.rs"), None);
        assert!(!store.is_trusted("src/main.rs"));
    }

    #[test]
    fn trusting_a_directory_covers_what_is_beneath_it() {
        let mut store = TrustStore::new();
        store.trust("src");
        assert!(store.is_trusted("src"));
        assert!(store.is_trusted("src/main.rs"));
        assert!(store.is_trusted("src/deep/nested.rs"));
        assert!(!store.is_trusted("other/main.rs"));
    }

    /// Trusting the workspace root is the startup case, so it must cover everything.
    #[test]
    fn trusting_the_root_covers_the_whole_tree() {
        let mut store = TrustStore::new();
        store.trust(".");
        assert!(store.is_trusted("anything/at/all.rs"));
        assert!(store.is_trusted("top.rs"));
    }

    /// The rule the design turns on: a more specific decision overrides a broader one.
    #[test]
    fn an_untrusted_subpath_overrides_a_trusted_parent() {
        let mut store = TrustStore::new();
        store.trust(".");
        store.distrust("vendor");

        assert!(store.is_trusted("src/main.rs"));
        assert!(!store.is_trusted("vendor/lib.js"));
        assert!(!store.is_trusted("vendor"));
    }

    /// And the reverse, which the user asked for explicitly: a trusted path inside an
    /// untrusted directory.
    #[test]
    fn a_trusted_subpath_overrides_an_untrusted_parent() {
        let mut store = TrustStore::new();
        store.distrust("vendor");
        store.trust("vendor/ours");

        assert!(!store.is_trusted("vendor/lib.js"));
        assert!(store.is_trusted("vendor/ours/code.js"));
        assert!(store.is_trusted("vendor/ours"));
    }

    /// Specificity must keep working to arbitrary depth, alternating polarity.
    #[test]
    fn the_deepest_rule_wins_at_any_depth() {
        let mut store = TrustStore::new();
        store.trust(".");
        store.distrust("a");
        store.trust("a/b");
        store.distrust("a/b/c");
        store.trust("a/b/c/d");

        assert!(store.is_trusted("z.rs"));
        assert!(!store.is_trusted("a/x.rs"));
        assert!(store.is_trusted("a/b/x.rs"));
        assert!(!store.is_trusted("a/b/c/x.rs"));
        assert!(store.is_trusted("a/b/c/d/x.rs"));
    }

    /// A prefix must match whole segments: treating `src` as covering `srcfoo` would grant
    /// trust to a path the user never named.
    #[test]
    fn a_rule_matches_whole_segments_only() {
        let mut store = TrustStore::new();
        store.trust("src");
        assert!(!store.is_trusted("srcfoo/main.rs"));
        assert!(!store.is_trusted("srcfoo"));
        assert!(store.is_trusted("src/foo"));
    }

    /// Equivalent spellings of a path must be one rule, or a user could trust `./src` and
    /// be surprised that `src` is still untrusted.
    #[test]
    fn equivalent_path_spellings_are_the_same_rule() {
        let mut store = TrustStore::new();
        store.trust("./src/");
        assert!(store.is_trusted("src/main.rs"));

        store.distrust("src");
        assert!(
            !store.is_trusted("src/main.rs"),
            "a differently spelled path became a second rule"
        );
    }

    /// Re-deciding must replace the earlier decision rather than accumulating rules whose
    /// resolution depends on insertion order.
    #[test]
    fn a_later_decision_replaces_an_earlier_one() {
        let mut store = TrustStore::new();
        store.trust("src");
        assert!(store.is_trusted("src/main.rs"));
        store.distrust("src");
        assert!(!store.is_trusted("src/main.rs"));
        store.trust("src");
        assert!(store.is_trusted("src/main.rs"));
    }

    #[test]
    fn integrity_of_reports_which_decision_applied() {
        let mut store = TrustStore::new();
        store.trust(".");
        store.distrust("vendor");

        assert_eq!(store.integrity_of("src/a.rs"), Some(Integrity::Trusted));
        assert_eq!(
            store.integrity_of("vendor/a.js"),
            Some(Integrity::Untrusted)
        );
    }

    /// With no root rule, an unmentioned path stays unknown rather than inheriting from an
    /// unrelated sibling rule.
    #[test]
    fn an_unrelated_path_is_unaffected_by_other_rules() {
        let mut store = TrustStore::new();
        store.trust("src");
        assert_eq!(store.integrity_of("docs/readme.md"), None);
    }

    /// An added directory is trusted by its absolute path, and that says nothing about the
    /// workspace. Two files may be called `src/main.rs`, so a rule has to name which one it means.
    #[test]
    fn an_absolute_rule_does_not_decide_a_relative_path() {
        let mut store = TrustStore::new();
        store.trust("/Users/me/notes");

        assert!(store.is_trusted("/Users/me/notes/todo.md"));
        assert_eq!(
            store.integrity_of("Users/me/notes/todo.md"),
            None,
            "an absolute rule leaked into the workspace"
        );
        assert_eq!(store.integrity_of("src/main.rs"), None);
    }

    /// And the reverse: vouching for the workspace must not vouch for a directory outside it.
    #[test]
    fn trusting_the_workspace_says_nothing_about_an_added_directory() {
        let mut store = TrustStore::new();
        store.trust(".");

        assert!(store.is_trusted("src/main.rs"));
        assert_eq!(
            store.integrity_of("/Users/me/notes/todo.md"),
            None,
            "the workspace rule reached outside it"
        );
    }

    /// The bug this separation exists to prevent. Stripping the slash would make `/` the empty key,
    /// which is the workspace's own rule, so trusting one added directory would hand trust to every
    /// file in the project.
    #[test]
    fn trusting_the_filesystem_root_does_not_trust_the_workspace() {
        let mut store = TrustStore::new();
        store.trust("/");

        assert!(store.is_trusted("/etc/hosts"));
        assert_eq!(
            store.integrity_of("src/main.rs"),
            None,
            "an absolute rule became the workspace root rule"
        );
    }

    /// Most specific still wins inside the absolute namespace, so an added directory can hold an
    /// untrusted subtree exactly as the workspace can.
    #[test]
    fn the_deepest_absolute_rule_wins() {
        let mut store = TrustStore::new();
        store.trust("/Users/me/notes");
        store.distrust("/Users/me/notes/clipped");

        assert!(store.is_trusted("/Users/me/notes/todo.md"));
        assert!(!store.is_trusted("/Users/me/notes/clipped/from-the-web.md"));
    }

    /// Two added directories are separate rules, so one does not answer for the other.
    #[test]
    fn one_added_directory_does_not_cover_a_sibling() {
        let mut store = TrustStore::new();
        store.trust("/Users/me/notes");

        assert!(store.is_trusted("/Users/me/notes/todo.md"));
        assert_eq!(store.integrity_of("/Users/me/other/todo.md"), None);
        // Whole segments only, here too: `notes` must not cover `notes-backup`.
        assert_eq!(store.integrity_of("/Users/me/notes-backup/todo.md"), None);
    }

    /// A trailing slash is the same rule, as it is for a relative path.
    #[test]
    fn equivalent_absolute_spellings_are_the_same_rule() {
        let mut store = TrustStore::new();
        store.trust("/Users/me/notes/");
        assert!(store.is_trusted("/Users/me/notes/todo.md"));

        store.distrust("/Users/me/notes");
        assert!(
            !store.is_trusted("/Users/me/notes/todo.md"),
            "a differently spelled path became a second rule"
        );
    }

    /// Moving the working directory must not carry the yes given for one project into another. The
    /// rule is still about the project it was given for, which is now somewhere else, so that is
    /// where it is written down.
    #[test]
    fn a_yes_for_one_project_does_not_follow_a_move_to_another() {
        let mut store = TrustStore::new();
        store.trust(".");

        let moved = store.rebased(Path::new("/work"), Path::new("/other"));
        assert_eq!(
            moved.integrity_of("src/main.rs"),
            None,
            "the answer given for /work vouched for /other"
        );
        assert!(moved.is_trusted("/work/src/main.rs"));
    }

    /// And a no given inside the project is not forgotten by moving into it. `vendor` was
    /// untrusted before the move and names the same files after it, so it stays untrusted.
    #[test]
    fn a_no_inside_the_new_directory_survives_the_move() {
        let mut store = TrustStore::new();
        store.trust(".");
        store.distrust("src/vendor");

        let moved = store.rebased(Path::new("/work"), Path::new("/work/src"));
        assert_eq!(
            moved.integrity_of("vendor/lib.js"),
            Some(Integrity::Untrusted),
            "an untrusted subtree became trusted by moving into its parent"
        );
    }

    /// The other direction: a rule about a directory the user added by name becomes an ordinary
    /// relative rule once that directory is the working directory.
    #[test]
    fn a_rule_in_an_added_directory_becomes_relative_when_it_is_moved_into() {
        let mut store = TrustStore::new();
        store.trust("/Users/me/notes");
        store.distrust("/Users/me/notes/private");

        let moved = store.rebased(Path::new("/work"), Path::new("/Users/me/notes"));
        assert!(moved.is_trusted("todo.md"));
        assert_eq!(
            moved.integrity_of("private/diary.md"),
            Some(Integrity::Untrusted)
        );
    }

    /// Whole segments decide what is inside the new directory, exactly as they decide what a rule
    /// covers. A prefix test on the characters would move `srcfoo` in as though it were `src`.
    #[test]
    fn a_sibling_with_a_longer_name_is_not_taken_to_be_inside() {
        let mut store = TrustStore::new();
        store.distrust("srcfoo");

        let moved = store.rebased(Path::new("/work"), Path::new("/work/src"));
        assert_eq!(
            moved.integrity_of("foo"),
            None,
            "a sibling directory was rebased as though it were inside the new one"
        );
        assert_eq!(
            moved.integrity_of("/work/srcfoo"),
            Some(Integrity::Untrusted)
        );
    }

    /// Rebasing decides nothing. Every rule that went in comes out, saying the same thing about
    /// the same files, which is what makes it safe to do without asking anybody.
    #[test]
    fn rebasing_neither_grants_nor_withdraws_anything() {
        let mut store = TrustStore::new();
        store.trust(".");
        store.distrust("vendor");
        store.trust("/Users/me/notes");

        let moved = store.rebased(Path::new("/work"), Path::new("/other"));
        assert_eq!(moved.rules().count(), 3);
        assert!(moved.is_trusted("/work/src/main.rs"));
        assert_eq!(
            moved.integrity_of("/work/vendor/lib.js"),
            Some(Integrity::Untrusted)
        );
        assert!(moved.is_trusted("/Users/me/notes/todo.md"));
    }
}
