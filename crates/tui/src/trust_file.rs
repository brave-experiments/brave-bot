//! The trust map, kept between sessions.
//!
//! A trust map is not only the answer to the startup question. `Policy::reconcile_after_write`
//! adds to it as a session runs: untrusted bytes landing in a trusted tree record that exact path
//! as untrusted, so reading it back cannot launder them. The event loop carries that forward from
//! one turn to the next. Without a file it stops at the edge of the process, and the next session
//! answers the startup question again, trusts the root again, and reads the poisoned file back as
//! trusted. That is integrity going untrusted → trusted, which is the one direction a label may
//! never move.
//!
//! So the map is written down, one file per working directory, under `~/.bua/trust`. Not beside
//! the session records: trust belongs to the directory rather than to any one session, and a
//! second session in the same checkout inherits what the first one learned.
//!
//! # Reading it back cannot upgrade anything
//!
//! The file is editable, which would be fatal if a word in it were a label. It is not. A rule
//! saying `trusted` is a record of the user's own vouching, restored, and the user may edit their
//! own decisions with an editor exactly as they may with the prompt. What must never happen is a
//! *rule being lost*, since a lost rule is a distrust that silently becomes trust. Hence the
//! strictness here:
//!
//! - An unrecognised integrity reads as untrusted, the safe direction, as `Snapshot::context`
//!   already does for the conversation.
//! - A file that will not parse, or one rule of which names no path, makes the whole file
//!   [`Stored::Unreadable`]. Nothing is trusted for that session, and the user is told. Salvaging
//!   the rules that did parse is the tempting mistake: the one that did not may be the distrust
//!   that matters.
//! - A version this build does not know is unreadable for the same reason.
//!
//! Everything else degrades to doing nothing, like the rest of `~/.bua`: a map that cannot be
//! written still leaves a working session, one turn at a time.

use bua_core::label::Integrity;
use bua_core::trust::TrustStore;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Where trust maps live inside the state directory.
const TRUST: &str = "trust";

/// The format written by this build.
const VERSION: u64 = 1;

/// The word for a trusted rule. Anything else reads as untrusted.
const TRUSTED: &str = "trusted";
const UNTRUSTED: &str = "untrusted";

/// What was on disk for a working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stored {
    /// No file. Nothing has ever been recorded for this directory.
    Nothing,
    /// The rules that were recorded, which may be empty.
    Rules(TrustStore),
    /// A file that could not be read, so what it recorded is unknown.
    ///
    /// Distinct from [`Stored::Nothing`] because the two must be answered differently: nothing
    /// recorded means ask, and an unreadable file means do not, since asking is how a forgotten
    /// distrust turns back into trust.
    Unreadable,
}

/// What a session should start with, given what was recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opening {
    /// Use these rules as they are. The user has vouched already; asking again would only offer
    /// them the chance to say something they have said.
    Remembered(TrustStore),
    /// Ask the startup question, and apply the answer on top of these rules.
    ///
    /// The rules are carried into the answer rather than replaced by it, so a recorded distrust
    /// survives a fresh yes: it is the more specific rule, and the longest match wins.
    Ask(TrustStore),
    /// Trust nothing and say why. The recorded map could not be read, so there is no way to know
    /// which paths it had ruled out.
    Refuse,
}

/// Decide how a session opens.
///
/// Pure, so the decision this rests on can be tested without a terminal or a home directory.
pub fn opening(stored: Stored) -> Opening {
    match stored {
        Stored::Unreadable => Opening::Refuse,
        Stored::Nothing => Opening::Ask(TrustStore::new()),
        // A map holding only distrust is a map nobody has answered the startup question for:
        // reconcile can write those rules during a session that trusted nothing at all. Asking
        // keeps that session's user able to change their mind without losing what was learned.
        Stored::Rules(rules) if !grants_trust(&rules) => Opening::Ask(rules),
        Stored::Rules(rules) => Opening::Remembered(rules),
    }
}

/// Whether any rule vouches for anything.
fn grants_trust(rules: &TrustStore) -> bool {
    rules
        .rules()
        .any(|(_, integrity)| integrity == Integrity::Trusted)
}

/// Read the map recorded for `project`.
pub fn load(project: &Path) -> Stored {
    let Some(path) = path(project) else {
        return Stored::Nothing;
    };
    match std::fs::read_to_string(&path) {
        Ok(contents) => parse(&contents),
        // Any other error is a file that exists and cannot be read, which is not the same as one
        // that was never written.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Stored::Nothing,
        Err(_) => Stored::Unreadable,
    }
}

/// Decode a map from what the file holds.
///
/// Separate from the I/O so the rules above are testable as rules rather than as filesystem
/// states.
pub fn parse(contents: &str) -> Stored {
    let Ok(document) = serde_json::from_str::<Value>(contents) else {
        return Stored::Unreadable;
    };
    if document["version"].as_u64() != Some(VERSION) {
        return Stored::Unreadable;
    }
    let Some(entries) = document["rules"].as_array() else {
        return Stored::Unreadable;
    };

    let mut rules = TrustStore::new();
    for entry in entries {
        // A rule naming no path cannot be honoured, and honouring the rest would quietly drop
        // whatever it said.
        let Some(path) = entry["path"].as_str() else {
            return Stored::Unreadable;
        };
        match entry["integrity"].as_str() {
            Some(TRUSTED) => rules.trust(path),
            _ => rules.distrust(path),
        }
    }
    Stored::Rules(rules)
}

/// Write the map for `project`, replacing what was there.
///
/// Best-effort, like everything under `~/.bua`: a map that cannot be written costs the next
/// session its memory, which is worse than nothing but not worth ending this one over.
pub fn save(project: &Path, trust: &TrustStore) {
    let Some(path) = path(project) else {
        return;
    };
    let Some(directory) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(directory).is_err() {
        return;
    }

    // Written beside and renamed, so a session killed mid-write leaves the last good map rather
    // than half of a new one. Half a map is a map with rules missing, which is the failure this
    // whole module exists to prevent.
    let temporary = path.with_extension("tmp");
    if std::fs::write(&temporary, encode(trust)).is_ok() {
        let _ = std::fs::rename(&temporary, &path);
    }
}

/// The map as it is written down.
pub fn encode(trust: &TrustStore) -> String {
    let rules: Vec<Value> = trust
        .rules()
        .map(|(path, integrity)| {
            json!({
                "path": path,
                "integrity": match integrity {
                    Integrity::Trusted => TRUSTED,
                    Integrity::Untrusted => UNTRUSTED,
                },
            })
        })
        .collect();

    let document = json!({ "version": VERSION, "rules": rules });
    format!("{document:#}\n")
}

/// Where the map for a working directory lives.
///
/// Keyed the way sessions are, so one mangling serves both and a person looking at `~/.bua` can
/// see which directory a file belongs to.
pub fn path(project: &Path) -> Option<PathBuf> {
    Some(
        crate::store::directory()?
            .join(TRUST)
            .join(format!("{}.json", crate::sessions::key_for(project))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules_of(stored: Stored) -> TrustStore {
        match stored {
            Stored::Rules(rules) => rules,
            other => panic!("expected rules, got {other:?}"),
        }
    }

    /// The property the module exists for: a path a write marked untrusted inside a trusted tree
    /// must still be untrusted after a round trip, or reading it back launders it.
    #[test]
    fn a_distrusted_path_inside_a_trusted_tree_survives_the_round_trip() {
        let mut trust = TrustStore::new();
        trust.trust(".");
        trust.distrust("src/fetched.json");

        let back = rules_of(parse(&encode(&trust)));
        assert!(back.is_trusted("src/main.rs"), "the root lost its trust");
        assert!(
            !back.is_trusted("src/fetched.json"),
            "a path a write had distrusted came back trusted"
        );
    }

    /// Both polarities at any depth, since the map is not a flag and a trusted island inside an
    /// untrusted subtree is a case the trust store supports.
    #[test]
    fn rules_of_both_polarities_survive_at_any_depth() {
        let mut trust = TrustStore::new();
        trust.trust(".");
        trust.distrust("vendor");
        trust.trust("vendor/ours");
        trust.distrust("vendor/ours/generated.js");

        let back = rules_of(parse(&encode(&trust)));
        assert!(back.is_trusted("README.md"));
        assert!(!back.is_trusted("vendor/lib.js"));
        assert!(back.is_trusted("vendor/ours/code.js"));
        assert!(!back.is_trusted("vendor/ours/generated.js"));
    }

    /// An empty map is a real answer: the user was asked and declined. It must not be confused
    /// with never having been asked, which is what an absent file says.
    #[test]
    fn an_empty_map_round_trips_as_an_empty_map() {
        let back = rules_of(parse(&encode(&TrustStore::new())));
        assert!(back.is_empty());
    }

    /// A word this build does not know must not become trust. Everything unrecognised degrades in
    /// the safe direction, which is the only direction it may degrade in.
    #[test]
    fn an_unrecognised_integrity_reads_as_untrusted() {
        for word in ["\"Trusted\"", "\"trusted-pending\"", "\"\"", "true", "null"] {
            let contents =
                format!(r#"{{"version":1,"rules":[{{"path":"src","integrity":{word}}}]}}"#);
            let back = rules_of(parse(&contents));
            assert!(
                !back.is_trusted("src/main.rs"),
                "{word} was read as trusted"
            );
        }
    }

    /// A rule naming no path cannot be applied, and applying the others would drop whatever it
    /// said: the dropped one may be the distrust that matters.
    #[test]
    fn a_rule_that_names_no_path_makes_the_whole_file_unreadable() {
        let contents = r#"{"version":1,"rules":[
            {"path":".","integrity":"trusted"},
            {"integrity":"untrusted"}
        ]}"#;
        assert_eq!(parse(contents), Stored::Unreadable);
    }

    #[test]
    fn a_file_that_is_not_json_is_unreadable() {
        assert_eq!(parse("not json at all"), Stored::Unreadable);
        assert_eq!(parse(""), Stored::Unreadable);
    }

    /// A newer build's format may record rules this one cannot see, so guessing at it would drop
    /// them. Refusing to read it is what keeps that from being silent.
    #[test]
    fn a_version_this_build_does_not_know_is_unreadable() {
        assert_eq!(
            parse(r#"{"version":2,"rules":[{"path":".","integrity":"trusted"}]}"#),
            Stored::Unreadable
        );
        assert_eq!(parse(r#"{"rules":[]}"#), Stored::Unreadable);
    }

    #[test]
    fn rules_that_are_not_a_list_are_unreadable() {
        assert_eq!(
            parse(r#"{"version":1,"rules":"everything"}"#),
            Stored::Unreadable
        );
        assert_eq!(parse(r#"{"version":1}"#), Stored::Unreadable);
    }

    /// Nothing recorded is the only case that asks.
    #[test]
    fn a_directory_nobody_has_answered_for_is_asked_about() {
        assert_eq!(opening(Stored::Nothing), Opening::Ask(TrustStore::new()));
    }

    /// An unreadable map must not reach the question. Asking is how a forgotten distrust becomes
    /// trust again, which is exactly what an unreadable map cannot rule out.
    #[test]
    fn an_unreadable_map_is_not_asked_about_and_trusts_nothing() {
        assert_eq!(opening(Stored::Unreadable), Opening::Refuse);
    }

    /// Having vouched once is standing permission, which is what the question grants. Asking each
    /// time would train the user to press y without reading it.
    #[test]
    fn a_map_that_already_vouches_for_something_is_not_asked_about_again() {
        let mut trust = TrustStore::new();
        trust.trust(".");
        assert_eq!(
            opening(Stored::Rules(trust.clone())),
            Opening::Remembered(trust)
        );
    }

    /// A session that trusted nothing can still have recorded distrust. The user may change their
    /// mind about the root, and doing so must not discard what the last session learned.
    #[test]
    fn a_map_of_only_distrust_is_asked_about_without_losing_it() {
        let mut recorded = TrustStore::new();
        recorded.distrust("notes/fetched.md");

        let Opening::Ask(mut trust) = opening(Stored::Rules(recorded)) else {
            panic!("a map with nothing trusted should still be asked about");
        };
        // What the user answers on top of it.
        trust.trust(".");

        assert!(trust.is_trusted("src/main.rs"));
        assert!(
            !trust.is_trusted("notes/fetched.md"),
            "a fresh yes overwrote a recorded distrust"
        );
    }
}
