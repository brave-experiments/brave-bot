//! Asking the user to approve an effect, and putting the planner's questions to them.
//!
//! Three questions travel this way, and they differ in what is at stake. A write and a run ask for
//! permission, and the answer decides whether an effect happens. A question the planner posed asks
//! for information, and the answer decides nothing on its own: it is text the model reads. What
//! they share is consent, so all three live here rather than in [`crate::report`], which announces
//! and expects no reply.
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
use bua_core::Pipeline;
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

/// A pipeline the model has asked to run.
///
/// Carries the [`Pipeline`] itself rather than a rendering of it, because the whole point of an
/// argv vector is that the boundaries between arguments are real: a reviewer is shown each
/// argument as its own thing, and nothing has to trust a rendering to have got the boundaries
/// right.
///
/// There is no `needs_approval` field, and there is no variant of this that skips the prompt.
/// Every run asks. See [`bua_core::policy::Policy::run_needs_approval`] for why that has no
/// exceptions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRequest {
    /// The stages, in order, exactly as they will be executed.
    pub pipeline: Pipeline,
    /// What each stage's program name resolved to, in stage order.
    ///
    /// Shown alongside the name, and it is this that the trusted list records. A name is not a
    /// program: `$PATH` decides what `grep` means, so a person vouching for one should be looking
    /// at the binary they are vouching for.
    pub resolved: Vec<String>,
    /// The directory the stages will run in, for the person to read.
    ///
    /// Shown because a program's effect depends on where it runs at least as much as on its
    /// arguments, and `git clean -fd` is a different proposition in two different trees.
    pub directory: String,
}

impl RunRequest {
    /// Whether approving this would hand the user's own data to a program.
    ///
    /// A second and independent reason to be careful, on confidentiality rather than integrity:
    /// bytes going into a program are released somewhere this policy stops governing.
    pub fn releases_private(&self) -> bool {
        self.pipeline.releases_private()
    }

    /// A short description for a prompt line.
    pub fn summary(&self) -> String {
        format!(
            "run {} in {}",
            tally(self.pipeline.len(), "stage", "stages"),
            self.directory
        )
    }

    /// The programs this would add to the trusted list, without repeats and in stage order.
    ///
    /// Named for the prompt, which has to say what vouching would cover. A pipeline of two stages
    /// vouches for both, since a run that still had to ask about one of them would not have
    /// stopped asking.
    pub fn would_vouch_for(&self) -> Vec<String> {
        let mut named: Vec<String> = Vec::new();
        for path in &self.resolved {
            if !named.iter().any(|seen| seen == path) {
                named.push(path.clone());
            }
        }
        named
    }
}

/// What the user decided about a run.
///
/// Two answers rather than one, because "yes" and "yes, and stop asking" are different things and
/// the second is the one that changes what happens next time. A refusal never remembers: nothing
/// about saying no is a reason to vouch for the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunDecision {
    pub decision: Decision,
    /// Whether the person asked for these programs to stop being asked about this session.
    pub remember: bool,
}

impl RunDecision {
    /// Run it this once.
    pub fn approve() -> Self {
        Self {
            decision: Decision::Approve,
            remember: false,
        }
    }

    /// Run it, and stop asking about these programs for the rest of the session.
    pub fn approve_always() -> Self {
        Self {
            decision: Decision::Approve,
            remember: true,
        }
    }

    /// Do not run it. Never remembers: a refusal is not a reason to vouch for anything.
    pub fn reject() -> Self {
        Self {
            decision: Decision::Reject,
            remember: false,
        }
    }

    pub fn approved(self) -> bool {
        self.decision == Decision::Approve
    }
}

/// `1 stage`, `2 stages`. Local rather than shared, since this crate's other copy is private to
/// the tools module.
fn tally(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("{count} {one}")
    } else {
        format!("{count} {many}")
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
/// No method has a default body. Failing closed is the behaviour that matters most here, so it is
/// written out at every implementation rather than inherited from a trait an implementor never
/// read.
pub trait Confirmer {
    /// Ask about a write. Implementations must default to refusal when they cannot ask.
    fn confirm_write(&mut self, request: &WriteRequest) -> Decision;

    /// Ask about running a pipeline. Implementations must default to refusal when they cannot ask.
    ///
    /// Separate from [`Confirmer::confirm_write`] because the two are not the same question and a
    /// reviewer needs them not to look alike: a write shows a diff of a file, and a run shows argv
    /// that is about to execute with the access the user's own shell has.
    ///
    /// The answer carries whether to remember the programs as well as whether to run them. An
    /// implementation that cannot ask must refuse **and** not remember: inferring a standing
    /// permission from a question nobody answered is worse than inferring a single one.
    fn confirm_run(&mut self, request: &RunRequest) -> RunDecision;

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

    fn confirm_run(&mut self, _request: &RunRequest) -> RunDecision {
        RunDecision::reject()
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

    /// Refuses. The name says writes, and a test that wanted a program to run should have to say
    /// so: approving execution as a side effect of approving writes is how a test ends up running
    /// something nobody meant it to.
    fn confirm_run(&mut self, _request: &RunRequest) -> RunDecision {
        RunDecision::reject()
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

    fn confirm_run(&mut self, _request: &RunRequest) -> RunDecision {
        RunDecision::reject()
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

/// Approves every run and every write. Test-only, and named so its use is conspicuous.
///
/// Exists because a test of the `run` tool needs the approval to succeed, and [`ApproveWrites`]
/// deliberately refuses runs.
#[derive(Debug, Default)]
pub struct ApproveRuns;

impl Confirmer for ApproveRuns {
    fn confirm_write(&mut self, _request: &WriteRequest) -> Decision {
        Decision::Approve
    }

    /// Approves this run without vouching for anything. A test that wants the trusted list
    /// exercised says so with [`RemembersRuns`], so no test picks up a standing permission it
    /// never asked for.
    fn confirm_run(&mut self, _request: &RunRequest) -> RunDecision {
        RunDecision::approve()
    }

    fn ask_user(&mut self, _asking: &Asking) -> Vec<Answer> {
        Vec::new()
    }
}

/// Approves every run and vouches for its programs. Test-only.
#[derive(Debug, Default)]
pub struct RemembersRuns;

impl Confirmer for RemembersRuns {
    fn confirm_write(&mut self, _request: &WriteRequest) -> Decision {
        Decision::Reject
    }

    fn confirm_run(&mut self, _request: &RunRequest) -> RunDecision {
        RunDecision::approve_always()
    }

    fn ask_user(&mut self, _asking: &Asking) -> Vec<Answer> {
        Vec::new()
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

    fn a_run() -> RunRequest {
        RunRequest {
            pipeline: Pipeline::new(vec![bua_core::Stage::new("git", vec!["log".into()])]),
            resolved: vec!["/usr/bin/git".into()],
            directory: "/tmp/project".into(),
        }
    }

    /// Nobody is there, so nothing runs and nothing is vouched for. Picking up a standing
    /// permission from a question nobody answered is worse than picking up a single one.
    #[test]
    fn an_unattended_run_refuses_and_vouches_for_nothing() {
        let answer = Unattended.confirm_run(&a_run());
        assert!(!answer.approved());
        assert!(!answer.remember, "a standing permission was inferred");
    }

    /// A refusal never remembers. Saying no to a run is not a reason to vouch for the program.
    #[test]
    fn a_refusal_never_vouches_for_anything() {
        assert!(!RunDecision::reject().remember);
    }

    /// Approving once is not approving always: the two answers are different and the difference is
    /// the whole point of offering both.
    #[test]
    fn approving_once_does_not_vouch_for_the_program() {
        let once = RunDecision::approve();
        assert!(once.approved());
        assert!(!once.remember);

        let always = RunDecision::approve_always();
        assert!(always.approved());
        assert!(always.remember);
    }

    /// The prompt has to say what vouching would cover, and a pipeline vouches for every program
    /// in it: one that still had to ask about a stage would not have stopped asking.
    #[test]
    fn vouching_covers_every_program_in_the_pipeline() {
        let request = RunRequest {
            pipeline: Pipeline::new(vec![
                bua_core::Stage::new("git", vec!["log".into()]),
                bua_core::Stage::new("sed", vec!["-n".into()]),
            ]),
            resolved: vec!["/usr/bin/git".into(), "/usr/bin/sed".into()],
            directory: "/tmp".into(),
        };
        assert_eq!(
            request.would_vouch_for(),
            vec!["/usr/bin/git".to_string(), "/usr/bin/sed".to_string()]
        );
    }

    /// The same program twice is one entry, so the prompt does not offer to vouch for it twice.
    #[test]
    fn a_program_used_twice_is_named_once() {
        let request = RunRequest {
            pipeline: Pipeline::new(vec![
                bua_core::Stage::new("sed", vec!["-n".into()]),
                bua_core::Stage::new("sed", vec!["-e".into()]),
            ]),
            resolved: vec!["/usr/bin/sed".into(), "/usr/bin/sed".into()],
            directory: "/tmp".into(),
        };
        assert_eq!(request.would_vouch_for(), vec!["/usr/bin/sed".to_string()]);
    }

    /// Approving writes must not approve running programs. A test that wanted a program to run
    /// says so, or a test ends up executing something nobody meant it to.
    #[test]
    fn approving_writes_does_not_approve_a_run() {
        assert!(!ApproveWrites.confirm_run(&a_run()).approved());
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
