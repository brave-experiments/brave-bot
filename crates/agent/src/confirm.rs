//! Asking the user to approve a write.
//!
//! A model-proposed write path cannot be promoted the way a read path can: a read that
//! goes to the wrong file wastes a step, while a write to the wrong file destroys work.
//! So the trust for a write comes from a person.
//!
//! The approval is what mints the endorsement. That endorsement is single-use and bound to
//! the exact path and body shown, so an approval cannot be replayed against a second write
//! or redirected to a different file after the fact.

use std::fmt;

/// A write the model has asked to perform.
///
/// Both fields are untrusted strings at this point — they are shown to a person precisely
/// because nothing else can vouch for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRequest {
    /// Workspace-relative path, as the model proposed it.
    pub path: String,
    /// The body the model wants written.
    pub contents: String,
    /// The current contents, when the file already exists, so a reviewer can see what
    /// would be lost.
    pub existing: Option<String>,
}

impl WriteRequest {
    /// Whether this would replace an existing file rather than create a new one.
    pub fn is_overwrite(&self) -> bool {
        self.existing.is_some()
    }

    /// A short description for a prompt line.
    pub fn summary(&self) -> String {
        let verb = if self.is_overwrite() {
            "overwrite"
        } else {
            "create"
        };
        let lines = self.contents.lines().count();
        format!("{verb} {} ({lines} lines)", self.path)
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
            ..request()
        };
        assert!(r.is_overwrite());
        assert!(r.summary().starts_with("overwrite"));
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
