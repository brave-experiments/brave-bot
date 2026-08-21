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
//! it into a trusted directory, read it back — now labelled trusted, because the path says
//! so — and use it for routing. Recording the destination as untrusted when untrusted bytes
//! land there closes the loop: what comes out of a file is never more trusted than what
//! went in.
//!
//! This crate performs no I/O; paths here are workspace-relative strings, compared by
//! segment. The caller is responsible for having resolved and confined them first.

use crate::label::Integrity;
use std::collections::BTreeMap;

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
    /// `None` when no rule covers it, which the caller should treat as untrusted — this
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
}

/// Normalise a workspace-relative path for comparison.
///
/// Strips a leading `./` and any surrounding slashes so `./src/`, `src`, and `/src` are one
/// rule rather than three that shadow each other confusingly.
fn normalise(path: &str) -> String {
    let trimmed = path.trim();
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
/// wrongly accept — and that mistake would hand trust to a path the user never named.
fn covers(prefix: &str, path: &str) -> bool {
    // The workspace root covers everything.
    if prefix.is_empty() {
        return true;
    }
    if path == prefix {
        return true;
    }
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// How specific a rule is. Deeper rules win; the root is least specific.
fn specificity(prefix: &str) -> usize {
    if prefix.is_empty() {
        0
    } else {
        prefix.split('/').count()
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
}
