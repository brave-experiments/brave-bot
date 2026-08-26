//! Asking the user to approve a write, and putting the planner's questions to them.
//!
//! Two questions travel this way, and they differ in what is at stake. A write asks for
//! permission, and the answer decides whether an effect happens. A question the planner posed
//! asks for information, and the answer decides nothing on its own: it is text the model reads.
//! What they share is consent, so both live here rather than in [`crate::report`], which
//! announces and expects no reply.
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
use bua_core::ask::{Answer, Asking};
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
/// `path` and `contents` are untrusted strings at this point. They are shown to a person
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
    /// Whether the body came from somewhere nobody vouched for.
    ///
    /// Shown to the reviewer as untrusted wherever it is drawn. Reading a diff of a file the
    /// model never saw is a different act from reviewing the model's own work, and the screen
    /// should not make the two look alike.
    pub untrusted: bool,
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

/// Something that can put a question to a person.
///
/// A trait so the kernel and the agent never depend on a terminal: the interactive session
/// prompts, a one-shot run refuses, and tests decide without either.
///
/// Neither method has a default body. Failing closed is the behaviour that matters most here, so
/// it is written out at every implementation rather than inherited from a trait an implementor
/// never read.
pub trait Confirmer {
    /// Ask about a write. Implementations must default to refusal when they cannot ask.
    fn confirm_write(&mut self, request: &WriteRequest) -> Decision;

    /// Put a series of questions to the person, one answer per question in the order they were
    /// asked.
    ///
    /// Implementations that cannot ask must return **no answers at all**, rather than a decline
    /// for each question. The kernel reads a missing answer as a decline anyway, and saying
    /// nothing is the one reply that cannot be wrong about how many questions there were.
    ///
    /// The questions arrive already shaped and released by the kernel, so an implementation
    /// draws what it was handed rather than formatting anything itself.
    fn ask_user(&mut self, asking: &Asking) -> Vec<Answer>;
}

/// Nobody to ask: refuses every write and answers no question.
///
/// The right behaviour where no one is there: a one-shot command, a pipeline, a cron job.
/// Silently approving in a non-interactive context would make the confirmation decorative
/// exactly where it matters most, and answering a question on the user's behalf would put words
/// in their mouth that the planner would then treat as theirs.
#[derive(Debug, Default)]
pub struct Unattended;

impl Confirmer for Unattended {
    fn confirm_write(&mut self, _request: &WriteRequest) -> Decision {
        Decision::Reject
    }

    fn ask_user(&mut self, _asking: &Asking) -> Vec<Answer> {
        Vec::new()
    }
}

/// Approves every write. Test-only, and named so its use is conspicuous.
///
/// Answers no question even so: approving a write is a yes to something the test set up, while
/// choosing an option would be inventing an answer no test asked for.
#[derive(Debug, Default)]
pub struct ApproveWrites;

impl Confirmer for ApproveWrites {
    fn confirm_write(&mut self, _request: &WriteRequest) -> Decision {
        Decision::Approve
    }

    fn ask_user(&mut self, _asking: &Asking) -> Vec<Answer> {
        Vec::new()
    }
}

/// Takes the first option of every question, and refuses writes. Test-only.
#[derive(Debug, Default)]
pub struct ChoosesFirst;

impl Confirmer for ChoosesFirst {
    fn confirm_write(&mut self, _request: &WriteRequest) -> Decision {
        Decision::Reject
    }

    fn ask_user(&mut self, asking: &Asking) -> Vec<Answer> {
        asking
            .prompts
            .iter()
            .map(|prompt| match prompt.rows.first() {
                Some(row) => Answer::Chosen(vec![row.index]),
                // Nothing to choose. Inventing text here would test the wrong thing.
                None => Answer::Declined,
            })
            .collect()
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

    fn a_series() -> Asking {
        bua_core::ask::asking(&bua_core::ask::Series::new(vec![
            bua_core::ask::Question::new(
                "Cache",
                "Which cache layer?",
                vec![
                    bua_core::ask::Choice::new("HTTP", None),
                    bua_core::ask::Choice::new("Query", None),
                ],
                false,
            ),
            bua_core::ask::Question::new("Branch", "Which branch?", Vec::new(), false),
        ]))
    }

    /// Nobody is there, so nothing is answered. Saying nothing rather than a decline per
    /// question is the reply that cannot be wrong about how many questions there were.
    #[test]
    fn an_unattended_run_answers_no_question() {
        assert!(Unattended.ask_user(&a_series()).is_empty());
    }

    /// Approving a write is a yes to something the test set up. Choosing an option would be
    /// inventing an answer no test asked for.
    #[test]
    fn approving_writes_does_not_imply_answering_questions() {
        assert!(ApproveWrites.ask_user(&a_series()).is_empty());
    }

    /// One answer per question, in the order they were asked, so a test double cannot quietly
    /// shift an answer onto the wrong question.
    #[test]
    fn a_chooser_answers_every_question_in_the_series() {
        assert_eq!(
            ChoosesFirst.ask_user(&a_series()),
            vec![Answer::Chosen(vec![0]), Answer::Declined],
            "a question with no options was answered with an option"
        );
    }

    fn request() -> WriteRequest {
        WriteRequest {
            path: "notes.md".into(),
            contents: "one\ntwo\n".into(),
            existing: None,
            intent: Intent::Create,
            untrusted: false,
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

    /// An overwrite summary counts the lines lost, not just those written, since that is the
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
            Unattended.confirm_write(&request()),
            Decision::Reject,
            "a non-interactive run must not approve writes"
        );
    }

    #[test]
    fn the_test_confirmer_approves() {
        assert_eq!(ApproveWrites.confirm_write(&request()), Decision::Approve);
    }
}
