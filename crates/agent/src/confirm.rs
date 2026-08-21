//! Asking the user to approve a write.
//!
//! A model-proposed write path cannot be promoted the way a read path can: a read that
//! goes to the wrong file wastes a step, while a write to the wrong file destroys work.
//! So the trust for a write comes from a person.
//!
//! The approval is what mints the endorsement. That endorsement is single-use and bound to
//! the exact path shown, so an approval cannot be replayed against a second write or
//! redirected to a different file after the fact.
//!
//! What is shown is a diff, not a body. An approval the reviewer cannot actually read is
//! decorative, and a whole-file body asks them to spot the difference themselves.

use crate::diff::Diff;
use std::fmt;

/// How a proposed write came about.
///
/// A reviewer needs this distinction: an edit replaces a passage the model located, while
/// an overwrite discards whatever the file held. The resulting diff may look similar, so
/// the intent is carried explicitly rather than inferred from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// A file that does not exist yet.
    Create,
    /// A whole-file replacement.
    Overwrite,
    /// A targeted replacement of matched text.
    Edit,
}

/// A write the model has asked to perform.
///
/// `path` and `contents` are untrusted strings at this point — they are shown to a person
/// precisely because nothing else can vouch for them. `contents` is always the complete
/// resulting file, including for an edit, so a reviewer sees the outcome rather than
/// having to apply a patch mentally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRequest {
    /// Workspace-relative path, as the model proposed it.
    pub path: String,
    /// The complete body that would end up on disk.
    pub contents: String,
    /// The current contents, when the file already exists, so a reviewer can see what
    /// would be lost.
    pub existing: Option<String>,
    pub intent: Intent,
}

impl WriteRequest {
    /// Whether this would replace an existing file rather than create a new one.
    pub fn is_overwrite(&self) -> bool {
        self.existing.is_some()
    }

    /// The change this would make, for display.
    pub fn diff(&self) -> Diff {
        Diff::compute(self.existing.as_deref().unwrap_or(""), &self.contents)
    }

    /// A short description for a prompt line.
    pub fn summary(&self) -> String {
        let verb = match self.intent {
            Intent::Create => "create",
            Intent::Overwrite => "overwrite",
            Intent::Edit => "edit",
        };
        match self.intent {
            Intent::Create => {
                let lines = self.contents.lines().count();
                format!("create {} ({lines} lines)", self.path)
            }
            _ => {
                let diff = self.diff();
                format!(
                    "{verb} {} (+{} -{})",
                    self.path,
                    diff.added(),
                    diff.removed()
                )
            }
        }
    }
}

/// What the user decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Approve,
    Reject,
}

/// Something that can ask a person about a write.
///
/// A trait so the kernel and the agent never depend on a terminal: the interactive session
/// prompts, a one-shot run refuses, and tests decide without either.
pub trait Confirmer {
    /// Ask about a write. Implementations must default to refusal when they cannot ask.
    fn confirm_write(&mut self, request: &WriteRequest) -> Decision;
}

/// Refuses every write.
///
/// The right behaviour where no one can be asked — a one-shot command, a pipeline, a cron
/// job. Silently approving in a non-interactive context would make the confirmation
/// decorative exactly where it matters most.
#[derive(Debug, Default)]
pub struct RefuseWrites;

impl Confirmer for RefuseWrites {
    fn confirm_write(&mut self, _request: &WriteRequest) -> Decision {
        Decision::Reject
    }
}

/// Approves every write. Test-only, and named so its use is conspicuous.
#[derive(Debug, Default)]
pub struct ApproveWrites;

impl Confirmer for ApproveWrites {
    fn confirm_write(&mut self, _request: &WriteRequest) -> Decision {
        Decision::Approve
    }
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Approve => f.write_str("approved"),
            Self::Reject => f.write_str("rejected"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> WriteRequest {
        WriteRequest {
            path: "notes.md".into(),
            contents: "one\ntwo\n".into(),
            existing: None,
            intent: Intent::Create,
        }
    }

    #[test]
    fn a_new_file_is_described_as_a_creation() {
        let r = request();
        assert!(!r.is_overwrite());
        assert_eq!(r.summary(), "create notes.md (2 lines)");
    }

    /// Overwriting is the dangerous case, so the summary must say so plainly.
    #[test]
    fn an_existing_file_is_described_as_an_overwrite() {
        let r = WriteRequest {
            existing: Some("old".into()),
            intent: Intent::Overwrite,
            ..request()
        };
        assert!(r.is_overwrite());
        assert!(r.summary().starts_with("overwrite"));
    }

    /// An overwrite summary counts the lines lost, not just those written — that is the
    /// number a reviewer is deciding about.
    #[test]
    fn an_overwrite_summary_counts_both_sides() {
        let r = WriteRequest {
            contents: "one\ntwo\n".into(),
            existing: Some("a\nb\nc\n".into()),
            intent: Intent::Overwrite,
            ..request()
        };
        assert_eq!(r.summary(), "overwrite notes.md (+2 -3)");
    }

    /// An edit must not be described as an overwrite: the reviewer's question is
    /// different even when the diff is not.
    #[test]
    fn an_edit_is_described_as_an_edit() {
        let r = WriteRequest {
            contents: "one\nTWO\n".into(),
            existing: Some("one\ntwo\n".into()),
            intent: Intent::Edit,
            ..request()
        };
        assert_eq!(r.summary(), "edit notes.md (+1 -1)");
    }

    /// The diff is against what is on disk, so an unchanged region is not reported as a
    /// change.
    #[test]
    fn the_diff_compares_against_the_existing_file() {
        let r = WriteRequest {
            contents: "keep\nnew\n".into(),
            existing: Some("keep\nold\n".into()),
            intent: Intent::Edit,
            ..request()
        };
        let diff = r.diff();
        assert_eq!((diff.added(), diff.removed()), (1, 1));
    }

    /// A creation has nothing to compare against, so every line is an addition.
    #[test]
    fn a_creation_diffs_against_nothing() {
        let diff = request().diff();
        assert_eq!(diff.added(), 2);
        assert_eq!(diff.removed(), 0);
    }

    /// The default where nobody can be asked must be refusal.
    #[test]
    fn the_non_interactive_confirmer_refuses() {
        assert_eq!(
            RefuseWrites.confirm_write(&request()),
            Decision::Reject,
            "a non-interactive run must not approve writes"
        );
    }

    #[test]
    fn the_test_confirmer_approves() {
        assert_eq!(ApproveWrites.confirm_write(&request()), Decision::Approve);
    }
}
