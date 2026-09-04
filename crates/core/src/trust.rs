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
use std::fmt;

/// How a path came by its standing.
///
/// The integrity is what a rule means; this is why it says so, and the integrity is derived from
/// it rather than stored beside it. A record that kept both could disagree with itself, and the
/// question a person asks days later is never "was this trusted" on its own: it is "why is the
/// agent allowed to read this", and only one of these two answers that.
///
/// The web's first-party and third-party distinction is the same shape. What decides is where
/// something came from, not what it looks like once it has arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// A person named the path themselves: `@path`, a file dropped on the window.
    Vouched,
    /// The user's own standing instructions for this project, recorded when the session opened.
    Standing,
    /// An isolated checker read the whole file and found nothing addressed to whoever reads it.
    Vetted,
    /// This agent wrote the file, out of data the turn already held and nothing else.
    Written,
    /// Bytes from somewhere nobody vouched for landed here: a fetch, a program's output, a pipe.
    Fetched,
    /// Somebody said no, or something withdrew it.
    Withheld,
}

impl Provenance {
    /// What this origin makes the content worth.
    ///
    /// The whole of the mapping, in one place, so no caller decides it twice. Everything a person
    /// or a checker stood behind is readable; everything that arrived from outside is not.
    pub fn integrity(self) -> Integrity {
        match self {
            Self::Vouched | Self::Standing | Self::Vetted | Self::Written => Integrity::Trusted,
            Self::Fetched | Self::Withheld => Integrity::Untrusted,
        }
    }

    /// The word this is written down as, in a session record and on a status line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vouched => "vouched",
            Self::Standing => "standing",
            Self::Vetted => "vetted",
            Self::Written => "written",
            Self::Fetched => "fetched",
            Self::Withheld => "withheld",
        }
    }

    /// Read a word back. An unrecognised one is [`Provenance::Withheld`], which is the reading
    /// that grants nothing: a record written by a build that knew a word this one does not must
    /// not be read as permission.
    ///
    /// Deliberately not `FromStr`, which is fallible by signature. There is no failing case here:
    /// every word maps to something, and the something an unknown word maps to is the whole point.
    pub fn of_word(word: &str) -> Self {
        match word {
            "vouched" => Self::Vouched,
            "standing" => Self::Standing,
            "vetted" => Self::Vetted,
            "written" => Self::Written,
            "fetched" => Self::Fetched,
            _ => Self::Withheld,
        }
    }
}

impl fmt::Display for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A user's decisions about which paths are trustworthy.
///
/// Empty by default, which means nothing is trusted and every write is shown to a person.
/// That is the right default: trust has to be granted, never assumed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustStore {
    /// Normalised path prefix → where that subtree's standing came from.
    ///
    /// Provenance rather than integrity, because integrity is a function of it and a record
    /// holding both could disagree with itself.
    ///
    /// A `BTreeMap` rather than a hash map so iteration order is deterministic, which keeps
    /// the audit trail reproducible.
    rules: BTreeMap<String, Provenance>,
}

impl TrustStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record where `path` and everything beneath it came by its standing.
    ///
    /// The one way a rule enters the map. What the rule then means is
    /// [`Provenance::integrity`] and is not stored.
    pub fn record(&mut self, path: &str, provenance: Provenance) {
        self.rules.insert(normalise(path), provenance);
    }

    /// Record that a person named `path` themselves.
    pub fn trust(&mut self, path: &str) {
        self.record(path, Provenance::Vouched);
    }

    /// Record that `path` and everything beneath it is not to be read.
    pub fn distrust(&mut self, path: &str) {
        self.record(path, Provenance::Withheld);
    }

    /// Where the longest matching rule says this path's standing came from.
    pub fn provenance_of(&self, path: &str) -> Option<Provenance> {
        let path = normalise(path);
        self.rules
            .iter()
            .filter(|(prefix, _)| covers(prefix, &path))
            .max_by_key(|(prefix, _)| specificity(prefix))
            .map(|(_, provenance)| *provenance)
    }

    /// The integrity of `path`, by the longest matching rule.
    ///
    /// `None` when no rule covers it, which the caller should treat as untrusted. This
    /// returns an option rather than defaulting so a caller cannot silently confuse "the
    /// user vouched for this" with "nobody has said".
    pub fn integrity_of(&self, path: &str) -> Option<Integrity> {
        self.provenance_of(path).map(Provenance::integrity)
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
            .map(|(prefix, provenance)| (prefix.as_str(), provenance.integrity()))
    }

    /// The rules with where each came from, for a record and for a status line.
    pub fn origins(&self) -> impl Iterator<Item = (&str, Provenance)> {
        self.rules
            .iter()
            .map(|(prefix, provenance)| (prefix.as_str(), *provenance))
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
}

#[cfg(test)]
mod provenance_tests {
    use super::*;

    /// The map records an origin and the integrity follows from it, so no caller decides twice
    /// and no record can disagree with itself.
    #[test]
    fn what_a_rule_means_is_derived_from_where_it_came_from() {
        let mut store = TrustStore::new();
        store.record("AGENTS.md", Provenance::Standing);
        store.record("notes.md", Provenance::Vetted);
        store.record("out.json", Provenance::Written);
        store.record("page.html", Provenance::Fetched);
        store.record("vendor", Provenance::Withheld);

        assert!(store.is_trusted("AGENTS.md"));
        assert!(store.is_trusted("notes.md"));
        assert!(store.is_trusted("out.json"));
        assert!(!store.is_trusted("page.html"));
        assert!(!store.is_trusted("vendor/lib.js"));

        assert_eq!(store.provenance_of("notes.md"), Some(Provenance::Vetted));
        assert_eq!(store.provenance_of("page.html"), Some(Provenance::Fetched));
    }

    /// A word from a newer build must not be read as permission by an older one. Every unknown
    /// origin lands on the reading that grants nothing.
    #[test]
    fn an_origin_this_build_does_not_know_grants_nothing() {
        for word in ["", "TRUSTED", "vouched-ish", "some-future-origin"] {
            assert_eq!(
                Provenance::of_word(word),
                Provenance::Withheld,
                "{word:?} was read as something"
            );
            assert_eq!(Provenance::of_word(word).integrity(), Integrity::Untrusted);
        }
    }

    /// The words round-trip, or a record written by this build is read back differently by it.
    #[test]
    fn every_origin_is_written_down_and_read_back_as_itself() {
        for provenance in [
            Provenance::Vouched,
            Provenance::Standing,
            Provenance::Vetted,
            Provenance::Written,
            Provenance::Fetched,
            Provenance::Withheld,
        ] {
            assert_eq!(Provenance::of_word(provenance.as_str()), provenance);
        }
    }
}
