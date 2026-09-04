//! Asking the user to approve an effect, and putting the planner's questions to them.
//!
//! Four questions travel this way, and they differ in what is at stake. A write and a run ask for
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
use bravebot_core::Pipeline;
use bravebot_core::ask::{Answer, Asking};
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
/// Every run asks. See [`bravebot_core::policy::Policy::run_needs_approval`] for why that has no
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

    /// The commands this would add to the trusted list, without repeats and in stage order.
    ///
    /// Named for the prompt, which has to say what vouching would cover. A pipeline of two stages
    /// vouches for both, since a run that still had to ask about one of them would not have
    /// stopped asking, and its output would still be untrusted.
    ///
    /// Each entry is a program **and its exact arguments**. Vouching for `git log` says nothing
    /// about `git push`.
    pub fn would_vouch_for(&self) -> Vec<bravebot_core::programs::Command> {
        let mut named: Vec<bravebot_core::programs::Command> = Vec::new();
        for (stage, path) in self.pipeline.stages.iter().zip(&self.resolved) {
            let command = bravebot_core::programs::Command::new(path.clone(), stage.args.clone());
            if !named.contains(&command) {
                named.push(command);
            }
        }
        named
    }
}

/// A command's output the planner has asked to read.
///
/// The bytes are here in full, released for display, because that is the entire point: a person
/// deciding whether the model may read something must be reading it themselves. Unlike every other
/// question in this file, the answer rests on what is in front of them rather than on a prediction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputRequest {
    /// The command that produced it, as it was approved.
    pub command: String,
    /// What it printed, in full.
    pub output: String,
    /// The reference the planner named, for the account given afterwards.
    pub reference: String,
}

impl OutputRequest {
    pub fn lines(&self) -> usize {
        self.output.lines().count()
    }

    /// A short description for a prompt line.
    pub fn summary(&self) -> String {
        format!(
            "let the model read {} of output from {}",
            tally(self.lines(), "line", "lines"),
            self.command
        )
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

    /// Ask whether the planner may read a command's output. Implementations must default to
    /// refusal when they cannot ask.
    ///
    /// The one question in this trait whose answer rests on bytes rather than on a prediction, so
    /// an implementation that cannot show them must refuse: approving unseen is the one thing this
    /// question cannot mean.
    fn confirm_read_output(&mut self, request: &OutputRequest) -> Decision;

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

    fn confirm_read_output(&mut self, _request: &OutputRequest) -> Decision {
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

    /// Refuses. The name says writes, and a test that wanted a program to run should have to say
    /// so: approving execution as a side effect of approving writes is how a test ends up running
    /// something nobody meant it to.
    fn confirm_run(&mut self, _request: &RunRequest) -> RunDecision {
        RunDecision::reject()
    }

    fn confirm_read_output(&mut self, _request: &OutputRequest) -> Decision {
        Decision::Reject
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

    fn confirm_read_output(&mut self, _request: &OutputRequest) -> Decision {
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

    /// Refuses. A test that wants output read says so with [`ReadsOutput`], so no test picks up
    /// quarantined bytes it never asked for.
    fn confirm_read_output(&mut self, _request: &OutputRequest) -> Decision {
        Decision::Reject
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

    fn confirm_read_output(&mut self, _request: &OutputRequest) -> Decision {
        Decision::Reject
    }

    fn ask_user(&mut self, _asking: &Asking) -> Vec<Answer> {
        Vec::new()
    }
}

/// Approves a run once and lets its output be read. Test-only.
#[derive(Debug, Default)]
pub struct ReadsOutput;

impl Confirmer for ReadsOutput {
    fn confirm_write(&mut self, _request: &WriteRequest) -> Decision {
        Decision::Reject
    }

    fn confirm_run(&mut self, _request: &RunRequest) -> RunDecision {
        RunDecision::approve()
    }

    fn confirm_read_output(&mut self, _request: &OutputRequest) -> Decision {
        Decision::Approve
    }

    fn ask_user(&mut self, _asking: &Asking) -> Vec<Answer> {
        Vec::new()
    }
}

/// Another confirmer, with a stopwatch on how long its answers took to arrive.
///
/// Wrapped here rather than measured at each prompt because this is the one place every question
/// passes through. The terminal draws four different prompts from three different call sites, and a
/// timer added to each would be four chances to add it to three of them; a turn that asked a
/// question the fourth way would then look as though nobody was ever waiting.
///
/// What is measured is the wait, not the decision. A refusal takes as long to arrive as an approval,
/// and the person was equally away from their desk either way. Nothing here reads a request or an
/// answer: it starts a clock, hands the question straight through, and stops it.
pub struct Timed<'a, C: Confirmer> {
    inner: &'a mut C,
    waited: std::time::Duration,
}

impl<'a, C: Confirmer> Timed<'a, C> {
    pub fn new(inner: &'a mut C) -> Self {
        Self {
            inner,
            waited: std::time::Duration::ZERO,
        }
    }

    /// How long this confirmer has kept the turn waiting, over every question it has been asked.
    pub fn waited(&self) -> std::time::Duration {
        self.waited
    }

    /// Time one question, whatever kind it is.
    fn timing<T>(&mut self, ask: impl FnOnce(&mut C) -> T) -> T {
        let started = std::time::Instant::now();
        let answer = ask(self.inner);
        self.waited += started.elapsed();
        answer
    }
}

impl<C: Confirmer> Confirmer for Timed<'_, C> {
    fn confirm_write(&mut self, request: &WriteRequest) -> Decision {
        self.timing(|inner| inner.confirm_write(request))
    }

    fn confirm_run(&mut self, request: &RunRequest) -> RunDecision {
        self.timing(|inner| inner.confirm_run(request))
    }

    fn confirm_read_output(&mut self, request: &OutputRequest) -> Decision {
        self.timing(|inner| inner.confirm_read_output(request))
    }

    fn ask_user(&mut self, asking: &Asking) -> Vec<Answer> {
        self.timing(|inner| inner.ask_user(asking))
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
        bravebot_core::ask::asking(&bravebot_core::ask::Series::new(vec![
            bravebot_core::ask::Question::new(
                "Cache",
                "Which cache layer?",
                vec![
                    bravebot_core::ask::Choice::new("HTTP", None),
                    bravebot_core::ask::Choice::new("Query", None),
                ],
                false,
            ),
            bravebot_core::ask::Question::new("Branch", "Which branch?", Vec::new(), false),
        ]))
    }

    fn a_write() -> WriteRequest {
        WriteRequest {
            path: "src/main.rs".to_string(),
            contents: "fn main() {}\n".to_string(),
            existing: None,
            intent: Intent::Overwrite,
            untrusted: false,
        }
    }

    fn an_output() -> OutputRequest {
        OutputRequest {
            command: "git log".to_string(),
            output: "one line\n".to_string(),
            reference: "output_1".to_string(),
        }
    }

    /// A confirmer that takes its time answering, so a test can assert the wait was noticed rather
    /// than assert on a real clock.
    struct Slow(std::time::Duration);

    impl Confirmer for Slow {
        fn confirm_write(&mut self, _request: &WriteRequest) -> Decision {
            std::thread::sleep(self.0);
            Decision::Approve
        }

        /// Refuses, and takes just as long about it. That is the point of the test below: a wait is
        /// a wait whichever way it is answered.
        fn confirm_run(&mut self, _request: &RunRequest) -> RunDecision {
            std::thread::sleep(self.0);
            RunDecision::reject()
        }

        fn confirm_read_output(&mut self, _request: &OutputRequest) -> Decision {
            std::thread::sleep(self.0);
            Decision::Reject
        }

        fn ask_user(&mut self, _asking: &Asking) -> Vec<Answer> {
            std::thread::sleep(self.0);
            Vec::new()
        }
    }

    /// The figure this whole thing exists for. Without it, time a person spent reading a diff is
    /// indistinguishable from time the model spent thinking, and only one of the two is worth
    /// trying to reduce.
    #[test]
    fn the_time_a_person_takes_to_answer_is_counted() {
        let mut slow = Slow(std::time::Duration::from_millis(30));
        let mut timed = Timed::new(&mut slow);

        assert_eq!(timed.confirm_write(&a_write()), Decision::Approve);
        assert!(
            timed.waited() >= std::time::Duration::from_millis(30),
            "the wait was not counted: {:?}",
            timed.waited()
        );
    }

    /// Every question, not only the one that happened to be instrumented first. A turn that asked
    /// the fourth way would otherwise report that nobody was ever waiting.
    #[test]
    fn every_kind_of_question_is_timed() {
        let each = std::time::Duration::from_millis(10);
        let mut slow = Slow(each);
        let mut timed = Timed::new(&mut slow);

        timed.confirm_write(&a_write());
        timed.confirm_run(&a_run());
        timed.confirm_read_output(&an_output());
        timed.ask_user(&a_series());

        assert!(
            timed.waited() >= each * 4,
            "some question was not timed: {:?}",
            timed.waited()
        );
    }

    /// A refusal took as long to arrive as an approval would have, and the person was equally away
    /// from their desk. Counting only approvals would understate exactly the sessions where somebody
    /// sat there saying no.
    #[test]
    fn a_refusal_is_a_wait_like_any_other() {
        let mut slow = Slow(std::time::Duration::from_millis(30));
        let mut timed = Timed::new(&mut slow);

        assert!(!timed.confirm_run(&a_run()).approved());
        assert!(
            timed.waited() >= std::time::Duration::from_millis(30),
            "a refusal was not counted as a wait: {:?}",
            timed.waited()
        );
    }

    /// Nothing asked is no time waited, so a turn that never stopped reports none rather than
    /// something small and unexplained.
    #[test]
    fn a_turn_that_asked_nothing_waited_for_nothing() {
        let mut slow = Slow(std::time::Duration::from_millis(30));
        let timed = Timed::new(&mut slow);
        assert_eq!(timed.waited(), std::time::Duration::ZERO);
    }

    /// The answer has to be the inner confirmer's, unchanged. A wrapper that measured correctly and
    /// altered a decision would be a permission bug wearing a stopwatch.
    #[test]
    fn the_answer_passes_through_untouched() {
        let mut approving = ApproveWrites;
        let mut timed = Timed::new(&mut approving);
        assert_eq!(timed.confirm_write(&a_write()), Decision::Approve);
        assert!(!timed.confirm_run(&a_run()).approved());

        let mut refusing = Unattended;
        let mut timed = Timed::new(&mut refusing);
        assert_eq!(timed.confirm_write(&a_write()), Decision::Reject);
        assert!(timed.ask_user(&a_series()).is_empty());
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
            pipeline: Pipeline::new(vec![bravebot_core::Stage::new("git", vec!["log".into()])]),
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
                bravebot_core::Stage::new("git", vec!["log".into()]),
                bravebot_core::Stage::new("sed", vec!["-n".into()]),
            ]),
            resolved: vec!["/usr/bin/git".into(), "/usr/bin/sed".into()],
            directory: "/tmp".into(),
        };
        assert_eq!(
            request
                .would_vouch_for()
                .iter()
                .map(bravebot_core::programs::Command::display)
                .collect::<Vec<_>>(),
            vec![
                "/usr/bin/git log".to_string(),
                "/usr/bin/sed -n".to_string()
            ],
            "vouching must name the arguments, since they are part of what is vouched for"
        );
    }

    /// The same program with different arguments is two entries, because vouching is for a
    /// command and not for a program: `sed -n` and `sed -e` do different things.
    #[test]
    fn the_same_program_with_different_arguments_is_two_entries() {
        let request = RunRequest {
            pipeline: Pipeline::new(vec![
                bravebot_core::Stage::new("sed", vec!["-n".into()]),
                bravebot_core::Stage::new("sed", vec!["-e".into()]),
            ]),
            resolved: vec!["/usr/bin/sed".into(), "/usr/bin/sed".into()],
            directory: "/tmp".into(),
        };
        assert_eq!(request.would_vouch_for().len(), 2);
    }

    /// The identical command twice is one entry, so the prompt does not offer to vouch for it
    /// twice.
    #[test]
    fn the_identical_command_twice_is_named_once() {
        let request = RunRequest {
            pipeline: Pipeline::new(vec![
                bravebot_core::Stage::new("sed", vec!["-n".into()]),
                bravebot_core::Stage::new("sed", vec!["-n".into()]),
            ]),
            resolved: vec!["/usr/bin/sed".into(), "/usr/bin/sed".into()],
            directory: "/tmp".into(),
        };
        assert_eq!(request.would_vouch_for().len(), 1);
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
