//! How much a session asks before it acts.
//!
//! A different axis from [`crate::Mode`], which decides *when* control flow is settled. This decides
//! what happens when a gate has already established that an effect needs a person's consent: whether
//! the question is put to them, answered yes on their behalf, or answered no without asking.
//!
//! Held by the session rather than by a task, because it is a standing answer the person changes
//! while they work: they watch a turn edit files it should not be editing and want the next one to
//! stop and ask, or they have read enough diffs for one afternoon and want the next ten to go
//! through. One key cycles it, and the mode in force is written under the box for as long as it is.
//!
//! # Why plan mode refuses rather than asks
//!
//! [`PermissionMode::Plan`] is not a stricter [`PermissionMode::Ask`]. Its point is that the session
//! is for deciding what to do, so a write is refused outright and the planner is told why: a mode
//! whose only difference was that the person keeps saying no is one where a planner keeps proposing
//! writes and a person keeps declining them, which is the interaction it exists to avoid.
//!
//! Commands are still asked about, and deliberately. Research is most of what planning is, `git log`
//! and a test run are how it is done, and a mode that could not read the tree would write plans from
//! less than the person could see themselves.

use crate::confirm::{
    Confirmer, Decision, OutputRequest, RunDecision, RunRequest, VouchRequest, WriteRequest,
};

/// How much this session asks before it acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PermissionMode {
    /// Every effect is put to the person. What a session has always done, and the default.
    #[default]
    Ask,
    /// Writes go through unasked. Commands are still asked about.
    ///
    /// The two are not the same risk: a write lands in a tree the person can read afterwards, and
    /// `git diff` will show them all of it. A command runs with everything their own shell has and
    /// leaves no diff, so it keeps its prompt.
    AcceptEdits,
    /// Nothing is written. The session is for deciding what to do, not doing it.
    Plan,
    /// Nothing is asked at all, including vouching for files nobody vouched for.
    ///
    /// What `--dangerously-skip-permissions` selects, reachable only where it was given. See
    /// [`Confining::confirm_vouch`] for what the last of those costs.
    Bypass,
}

impl PermissionMode {
    /// The modes one key cycles through, in order, ending back at the first.
    ///
    /// `Bypass` is not among them: it is reachable only where the flag was given, so the ladder
    /// depends on that and is built by [`PermissionMode::cycle`] rather than listed here.
    const LADDER: [Self; 3] = [Self::Ask, Self::AcceptEdits, Self::Plan];

    /// The next mode round the ladder.
    ///
    /// `bypass_available` is whether `--dangerously-skip-permissions` was given. Without it the
    /// fourth rung does not exist: a session that could cycle into bypassing every check would make
    /// the flag decorative, and the flag is the record that somebody accepted what it costs.
    pub fn cycle(self, bypass_available: bool) -> Self {
        // From bypass the ladder is rejoined at the start, so the key remains a cycle rather than a
        // one-way door out of the mode the flag asked for.
        let Some(rung) = Self::LADDER.iter().position(|mode| *mode == self) else {
            return Self::Ask;
        };
        match rung + 1 == Self::LADDER.len() {
            true if bypass_available => Self::Bypass,
            true => Self::Ask,
            false => Self::LADDER[rung + 1],
        }
    }

    /// Whether a write happens without anybody being asked.
    pub fn writes_unasked(self) -> bool {
        matches!(self, Self::AcceptEdits | Self::Bypass)
    }

    /// Whether writing is refused however the person would have answered.
    pub fn refuses_writes(self) -> bool {
        self == Self::Plan
    }

    /// What the planner is told about the mode, or `None` where there is nothing to say.
    ///
    /// Only plan mode says anything. The others change who answers a question, which is not a fact
    /// about the work and not one the planner is better for knowing: told that its writes are going
    /// through unasked, a model has been handed a reason to be bolder than the person expected.
    ///
    /// Plan mode is different because the refusal is unconditional. Without this the planner spends
    /// the session proposing writes and reading back a refusal it cannot account for, and a planner
    /// that cannot tell a policy from a mistake retries.
    pub fn instruction(self) -> Option<&'static str> {
        match self {
            Self::Plan => Some(
                "\n\nPlan mode. The user is deciding what to do, so writing is refused for this \
                 turn however they would have answered: do not call write or edit, and do not \
                 offer to. Read the code, run what you need to understand it, and answer with what \
                 you would do and why. When the plan is settled they will leave this mode and ask \
                 you to carry it out.",
            ),
            Self::Ask | Self::AcceptEdits | Self::Bypass => None,
        }
    }
}

/// Another confirmer, with the questions this mode does not put to a person already answered.
///
/// A wrapper over whatever the caller already had rather than an implementation of its own, because
/// two of the six questions are not permission questions and must still reach the person: a question
/// the planner posed asks for information rather than consent, and an interjection is the person
/// speaking unprompted. Answering either here would put words in the mouth of somebody sitting in
/// front of the session.
///
/// Wired unconditionally, with the mode deciding what it does, so there is no second branch of an
/// otherwise identical turn for a permission bug to live in.
pub struct Confining<'a, C: Confirmer> {
    inner: &'a mut C,
    mode: PermissionMode,
}

impl<'a, C: Confirmer> Confining<'a, C> {
    pub fn new(inner: &'a mut C, mode: PermissionMode) -> Self {
        Self { inner, mode }
    }
}

impl<C: Confirmer> Confirmer for Confining<'_, C> {
    /// The one question the mode may answer either way.
    ///
    /// A refusal is not the same as a person having said no, but the tool reports it the same way,
    /// and that is the right thing to tell the planner: what it needs to know is that the write did
    /// not happen and retrying will not help. Plan mode also says so in the system prompt, which is
    /// where the reason belongs.
    fn confirm_write(&mut self, request: &WriteRequest) -> Decision {
        match self.mode {
            PermissionMode::AcceptEdits | PermissionMode::Bypass => Decision::Approve,
            PermissionMode::Plan => Decision::Reject,
            PermissionMode::Ask => self.inner.confirm_write(request),
        }
    }

    /// Approves without vouching for the programs, where it approves at all. Remembering writes a
    /// standing permission into the session record, which outlives the mode: that record can be
    /// resumed in another, and the vouched list would then claim a person approved programs nobody
    /// ever showed them.
    ///
    /// Accepting edits does not accept runs. A write lands in a tree that `git diff` will show in
    /// full afterwards; a command runs with everything the user's shell has and leaves no diff.
    fn confirm_run(&mut self, request: &RunRequest) -> RunDecision {
        match self.mode {
            PermissionMode::Bypass => RunDecision::approve(),
            PermissionMode::Ask | PermissionMode::AcceptEdits | PermissionMode::Plan => {
                self.inner.confirm_run(request)
            }
        }
    }

    fn confirm_read_output(&mut self, request: &OutputRequest) -> Decision {
        match self.mode {
            PermissionMode::Bypass => Decision::Approve,
            PermissionMode::Ask | PermissionMode::AcceptEdits | PermissionMode::Plan => {
                self.inner.confirm_read_output(request)
            }
        }
    }

    /// Asked in every mode but bypass, including plan mode.
    ///
    /// Accepting edits does not accept fetches: the two have nothing to do with each other, and a
    /// person who said yes to writing files in this tree has said nothing about which hosts may be
    /// talked to. Plan mode asks rather than refusing, because reading a page is how a plan gets
    /// written and a fetch changes nothing here; what it does do is leave the machine, which is
    /// the person's to agree to.
    fn confirm_fetch(&mut self, request: &crate::confirm::FetchRequest) -> Decision {
        match self.mode {
            PermissionMode::Bypass => Decision::Approve,
            PermissionMode::Ask | PermissionMode::AcceptEdits | PermissionMode::Plan => {
                self.inner.confirm_fetch(request)
            }
        }
    }

    /// Vouches for the file only where every check is being bypassed, which is the part of that mode
    /// that costs the most: the label on those bytes is what keeps a file's contents from being read
    /// as instructions, and this hands it over for every quarantined file the planner asks for.
    fn confirm_vouch(&mut self, request: &VouchRequest) -> Decision {
        match self.mode {
            PermissionMode::Bypass => Decision::Approve,
            PermissionMode::Ask | PermissionMode::AcceptEdits | PermissionMode::Plan => {
                self.inner.confirm_vouch(request)
            }
        }
    }

    /// Always the inner confirmer's. A question the planner posed is not a permission, and an answer
    /// invented here would be reported to the model as the user's own words.
    fn ask_user(&mut self, asking: &bravebot_core::ask::Asking) -> Vec<bravebot_core::ask::Answer> {
        self.inner.ask_user(asking)
    }

    /// Always the inner confirmer's, for the same reason: this is the person talking, not being
    /// asked.
    fn interjection(&mut self) -> Option<String> {
        self.inner.interjection()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confirm::{ApproveRuns, ChoosesFirst, Intent, Unattended};

    fn a_write() -> WriteRequest {
        WriteRequest {
            path: "src/main.rs".to_string(),
            contents: "fn main() {}\n".to_string(),
            existing: None,
            intent: Intent::Create,
            untrusted: false,
        }
    }

    fn a_series() -> bravebot_core::ask::Asking {
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

    fn a_run() -> RunRequest {
        RunRequest::from_pipeline(
            &bravebot_core::Pipeline::new(vec![bravebot_core::Stage::new(
                "git",
                vec!["log".into()],
            )]),
            &["/usr/bin/git".into()],
            "/tmp/project",
        )
    }

    /// Asking is what a session has always done, and what it must still do unless somebody changed
    /// it: the mode nobody chose cannot be one that stops putting writes to a person.
    #[test]
    fn the_default_mode_asks_about_everything() {
        assert_eq!(PermissionMode::default(), PermissionMode::Ask);

        let mut refusing = Unattended;
        let mut confining = Confining::new(&mut refusing, PermissionMode::Ask);
        // The inner confirmer's answer, whatever it is, rather than one this decided.
        assert_eq!(confining.confirm_write(&a_write()), Decision::Reject);
        assert!(!confining.confirm_run(&a_run()).approved());
    }

    /// The whole of what accepting edits buys, and the whole of what it does not: a write goes
    /// through and a command still asks. A write lands where `git diff` will show it; a command runs
    /// with everything the user's shell has.
    #[test]
    fn accepting_edits_lets_writes_through_but_not_commands() {
        let mut refusing = Unattended;
        let mut confining = Confining::new(&mut refusing, PermissionMode::AcceptEdits);
        assert_eq!(confining.confirm_write(&a_write()), Decision::Approve);
        assert!(
            !confining.confirm_run(&a_run()).approved(),
            "accepting edits must not accept running programs"
        );
    }

    /// Plan mode refuses rather than asks, so a write does not happen even where the wrapped
    /// confirmer would have approved one. Asking is what `Ask` is for.
    #[test]
    fn plan_mode_refuses_a_write_the_person_would_have_approved() {
        let mut approving = ApproveRuns;
        let mut confining = Confining::new(&mut approving, PermissionMode::Plan);
        assert_eq!(confining.confirm_write(&a_write()), Decision::Reject);
    }

    /// Research is most of what planning is, and `git log` is how it is done. The prompt stays,
    /// so it is the person's answer rather than the mode's.
    #[test]
    fn plan_mode_still_lets_a_command_be_asked_about() {
        let mut approving = ApproveRuns;
        let mut confining = Confining::new(&mut approving, PermissionMode::Plan);
        assert!(confining.confirm_run(&a_run()).approved());
    }

    /// Every consent question answered, which is what the flag selects.
    #[test]
    fn bypassing_answers_every_permission_question() {
        let mut refusing = Unattended;
        let mut confining = Confining::new(&mut refusing, PermissionMode::Bypass);
        assert_eq!(confining.confirm_write(&a_write()), Decision::Approve);
        let run = confining.confirm_run(&a_run());
        assert!(run.approved());
        assert!(!run.remember, "a standing permission outlived the mode");
    }

    /// Only plan mode tells the planner anything. Told that its writes are going through unasked, a
    /// model has been handed a reason to be bolder than the person expected.
    #[test]
    fn only_plan_mode_says_anything_to_the_planner() {
        assert!(PermissionMode::Plan.instruction().is_some());
        for quiet in [
            PermissionMode::Ask,
            PermissionMode::AcceptEdits,
            PermissionMode::Bypass,
        ] {
            assert!(quiet.instruction().is_none(), "{quiet:?} spoke up");
        }
    }

    /// The ladder one key walks. Three rungs without the flag, back to the start from the last: a
    /// key that stopped cycling would be a mode somebody could not leave.
    #[test]
    fn the_key_cycles_three_modes_without_the_flag() {
        let mut mode = PermissionMode::default();
        let mut seen = Vec::new();
        for _ in 0..4 {
            mode = mode.cycle(false);
            seen.push(mode);
        }
        assert_eq!(
            seen,
            vec![
                PermissionMode::AcceptEdits,
                PermissionMode::Plan,
                PermissionMode::Ask,
                PermissionMode::AcceptEdits,
            ]
        );
    }

    /// The fourth rung exists only where the flag was given. Without that, a session could cycle
    /// into bypassing every check and the flag would be decorative.
    #[test]
    fn bypass_is_only_reachable_where_the_flag_was_given() {
        let mut mode = PermissionMode::default();
        let mut seen = Vec::new();
        for _ in 0..4 {
            mode = mode.cycle(true);
            seen.push(mode);
        }
        assert_eq!(
            seen,
            vec![
                PermissionMode::AcceptEdits,
                PermissionMode::Plan,
                PermissionMode::Bypass,
                PermissionMode::Ask,
            ],
            "the ladder must come back round rather than stop at bypass"
        );

        // The rung is unreachable without it, however many times the key is pressed.
        let mut mode = PermissionMode::default();
        for _ in 0..12 {
            mode = mode.cycle(false);
            assert_ne!(mode, PermissionMode::Bypass);
        }
    }

    /// A question the planner posed asks for information rather than consent, and it must reach the
    /// person in every mode: an answer invented here would be reported to the model as theirs.
    #[test]
    fn no_mode_answers_a_question_that_is_not_a_permission() {
        for mode in [
            PermissionMode::Ask,
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
            PermissionMode::Bypass,
        ] {
            let mut choosing = ChoosesFirst;
            let asked = Confining::new(&mut choosing, mode).ask_user(&a_series());
            assert_eq!(
                asked,
                vec![
                    bravebot_core::ask::Answer::Chosen(vec![0]),
                    bravebot_core::ask::Answer::Declined
                ],
                "{mode:?} answered a question that was not a permission"
            );
        }
    }

    /// A session started in bypass by the flag can still be cycled out of and back into it, so the
    /// key means the same thing wherever the session began.
    #[test]
    fn a_session_started_in_bypass_can_cycle_out_of_it() {
        assert_eq!(PermissionMode::Bypass.cycle(true), PermissionMode::Ask);
        // And back round to it, so nothing is one-way.
        let mut mode = PermissionMode::Bypass;
        for _ in 0..4 {
            mode = mode.cycle(true);
        }
        assert_eq!(mode, PermissionMode::Bypass);
    }
}
