//! Running a delegated agent.
//!
//! The kernel decides what a delegate is and what it holds; this runs it. A delegate is a turn,
//! deliberately and not by convenience: the same loop, the same gates, the same presentation of
//! every result. Four things differ, and each is fixed before it starts.
//!
//! - **Its capabilities**, which its kind asked for and the parent's own set narrowed.
//! - **Its tools**, derived from those capabilities, minus the four a delegate never gets.
//! - **Its prompt**, which is its kind's and which the planner cannot write a word of.
//! - **Its bound**, which is its kind's, because nobody is watching a delegate the way a person
//!   watches a turn: the person is watching the turn, and the turn is blocked.
//!
//! What crosses back is the report and nothing else. Its exchange, its tool results, its
//! narration and its quarantine all die with it, which is the whole point: a planner that ran the
//! build itself reads the log, and a planner that asked a delegate to run it is told what failed.
//!
//! The report is model output like any other, labelled by the integrity of the context that
//! produced it and presented through the same gate. A delegate is a planner, so its context holds
//! nothing untrusted and its report is ordinarily shown; where that context had met something
//! untrusted the report is quarantined and the parent is handed a reference, exactly as it would
//! be for a file. Nothing is relabelled and nothing is asserted trusted on a delegate's word.

use bravebot_aichat::protocol::Usage;
use bravebot_core::delegate::{DelegateSpec, Kind};
use bravebot_core::event::Sink;
use bravebot_core::policy::{Policy, Vouched};
use bravebot_core::value::Labelled;
use std::fmt;
use std::time::Duration;

use crate::confirm::Confirmer;
use crate::conversation::Conversation;
use crate::report::Reporter;
use crate::turn::{self, Task, TurnError};

/// How a delegate is introduced to itself.
///
/// Everything here is about the one thing that makes a delegate different from a turn: what it
/// says at the end is all anybody gets. Nothing rests on it, in the sense that nothing in the
/// paragraph is a gate, and it is here because a model that knows its answer is the whole of its
/// output writes a better one.
const DELEGATED: &str = "\
You are a delegated agent. Another agent gave you one task and is waiting for you to finish it, \
so you are not talking to a person and there is nobody to ask: no question you write will reach \
anyone, and the agent waiting for you cannot answer one either.

One thing crosses back when you stop, and it is your final answer. Nothing else does. What you \
read, what you searched, what the tests printed, what you said between tool calls: none of it \
reaches the agent that asked, and none of it can be looked up afterwards. It saw nothing you \
saw.

So write the answer for somebody who watched none of this. Be specific in the way that is only \
possible for whoever actually looked: name the file, the line, the command you ran and what it \
printed, the reference you processed. An answer saying you investigated the problem and it is \
now resolved is worth nothing to the only reader you have, because they cannot go and check, and \
they have to decide what to do next from your sentence alone.

Say what you did not settle. Where the task was ambiguous, take the most useful reading of it, do \
that, and say in the answer which reading you took and what the other one was. Where you could \
not finish, say how far you got and what stopped you. Both are more use than a confident answer \
about something you did not do, and neither costs you anything: you are not being marked, you \
are being read by somebody who has to act on this.

You cannot delegate. There is no tool for it and asking for one achieves nothing, so the work in \
front of you is yours to do or to report back on.

You cannot fetch a URL either. Anything you need from the network has to be in a file here \
already, so where a task turns on something only a fetch would settle, say so in the answer and \
leave it to whoever asked.";

/// What a kind is told it may not do.
///
/// Said although the tools are simply absent, and for the reason [`crate::processor`] gives for
/// telling a processor what it is: a model that knows the shape of its situation does better work
/// than one that discovers it by being refused. The absence is what makes it true; this only
/// makes it legible.
fn limits(kind: Kind) -> &'static str {
    match kind {
        Kind::Reader => {
            "\n\nYou can read, list, search and hand quarantined files to processors. You cannot \
             write a file and you cannot run a program, so do not plan around either: what you \
             produce is the answer, and a change somebody else has to make belongs in it as a \
             description precise enough to act on."
        }
        Kind::Checker => {
            "\n\nYou can read, list, search, hand quarantined files to processors, and run \
             programs. You cannot write a file. So you can find out whether this project builds \
             and what its tests say, and you cannot fix what you find: report the failure with \
             the command that produced it and enough of what it printed to act on, and leave the \
             fixing to whoever asked."
        }
        Kind::Worker => {
            "\n\nYou can read, list, search, hand quarantined files to processors, run programs \
             and write files. Every write is still shown to a person for approval before it \
             happens, exactly as it would be for the agent that asked you, so say what you \
             intend to change before you change it and do not retry a write that was refused."
        }
    }
}

/// The whole of what a delegate of this kind is told.
///
/// Its own introduction, then the guidance every planner here gets, then what its kind cannot do.
/// The middle is shared with the turn a person is watching rather than copied: reading a
/// workspace, changing a file it may not see, and reporting only what it actually knows are the
/// same problems whoever is waiting for the answer.
pub fn prompt_for(kind: Kind) -> String {
    format!("{DELEGATED}{}{}", turn::PLANNING, limits(kind))
}

/// What one delegate produced.
pub struct Delegated {
    /// Its answer, as the kernel labelled it from the context that produced it.
    ///
    /// Never read on the way past. The parent presents it, and the label decides whether the
    /// parent's planner is shown the words or a reference to them.
    pub report: Labelled<String>,
    /// What it was, for the line the person watching reads.
    pub kind: Kind,
    /// How many rounds of tool calls it took.
    pub rounds: usize,
    /// What it cost, so the turn can report the whole of what it spent.
    pub usage: Usage,
    /// How long it spent waiting on the model.
    ///
    /// Reported apart from the wall clock so the seconds land in the turn's inference figure
    /// rather than in its tool figure. A delegate is a run of requests, and charging them to tool
    /// execution would make a turn that delegated its work read as one that ran a very slow
    /// subprocess.
    pub inference: Duration,
}

/// Everything about a delegate that was settled before it existed.
///
/// Taken from the parent's policy on the turn's own thread, because that is the only thing that
/// can say what the parent already holds. Once this exists the delegate needs nothing further
/// from the run that spawned it, which is what lets it run alongside that run rather than inside
/// it.
pub struct Seeded {
    /// What the kernel built: the kind, the narrowed capabilities, the bound and the task.
    pub spec: DelegateSpec,
    /// The standing decisions it starts from, kept so what comes back can be compared against it.
    ///
    /// Only the answers a person gave inside the delegate are taken back, and this is what
    /// "inside" is measured from.
    pub vouched: Vouched,
    /// Rules written in advance about what to ask about, which do not stop applying because the
    /// asking moved.
    pub permissions: bravebot_core::permissions::Permissions,
}

/// Names what it holds and never the task.
///
/// The same rule the audit trail follows: a task is a paragraph a planner wrote, and a type that
/// printed it would put it in every debugging line that mentioned one. The trust map is left out
/// on the same reasoning, being a list of a person's paths.
impl fmt::Debug for Seeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Seeded")
            .field("kind", &self.spec.kind())
            .field("rounds", &self.spec.rounds())
            .finish_non_exhaustive()
    }
}

/// What one delegate left behind.
pub struct Finished {
    /// Its report and what it cost.
    pub delegated: Delegated,
    /// The standing decisions as they stood when it stopped, for the parent to take back.
    pub vouched: Vouched,
}

/// Settle everything a delegate needs from the run that spawned it.
///
/// Separated from running it because these three come off the parent's policy and the run does
/// not: a delegate holds no reference to its parent once it starts, so nothing it does has to
/// wait for the parent and nothing the parent does has to wait for it.
pub fn seed<S: Sink>(policy: &Policy<'_, S>, spec: DelegateSpec) -> Seeded {
    Seeded {
        spec,
        vouched: policy.vouched(),
        permissions: policy.permissions().clone(),
    }
}

/// Run one delegate to completion.
///
/// Takes nothing belonging to the run that spawned it. The trail, the reporter and the confirmer
/// are lent rather than owned, because there is one of each however many runs are going: one
/// trail records them all, one screen shows them all, and one person answers for them all.
///
/// Takes the three by trait object rather than by type parameter. A delegate is a turn, and a
/// turn lends these three to the delegates it starts, so a type parameter here would describe a
/// tower of lenders one level deeper for every level of nesting: a type the compiler builds for
/// ever and a program that cannot be compiled. A delegate cannot delegate, so the tower is one
/// level tall whatever the types say, and saying so here is what makes that true of the types.
#[allow(clippy::too_many_arguments)]
pub fn run(
    seeded: &Seeded,
    config: &bravebot_config::Config,
    egress: &bravebot_net::Egress,
    workspace: &crate::workspace::Workspace,
    home: Option<&std::path::Path>,
    model: Option<&str>,
    // The spawning turn's, since a delegate is that turn's work done elsewhere.
    permission_mode: crate::PermissionMode,
    cancel: &bravebot_core::cancel::Cancel,
    confirmer: &mut (dyn Confirmer + Send),
    reporter: &mut (dyn Reporter + Send),
    sink: &mut (dyn Sink + Send),
) -> Result<Finished, TurnError> {
    // The mode is the spawning turn's, and inherited rather than chosen: a delegate is that turn's
    // own work done elsewhere, so a session that is planning must not have writes happening inside
    // one. Enforcement already comes down this way, the confirmer being the person's own; this is
    // what tells the delegate's planner why a write would be refused.
    let task = Task::delegated(seeded.spec.clone())
        .with_home(home.map(std::path::Path::to_path_buf))
        .with_model(model.map(str::to_string))
        .with_permissions(seeded.permissions.clone())
        .with_permission_mode(permission_mode);

    // Its own, and it dies here. A reference minted inside a delegate names nothing once it has
    // gone, which is what makes "nothing but the report crosses back" a fact about the data rather
    // than a promise about the prose.
    let mut conversation = Conversation::new();

    let outcome = turn::delegated(
        config,
        egress,
        workspace,
        &task,
        &mut conversation,
        confirmer,
        reporter,
        sink,
        seeded.vouched.trust.clone(),
        seeded.vouched.programs.clone(),
        cancel,
    )?;

    Ok(Finished {
        delegated: Delegated {
            report: outcome.answer,
            kind: seeded.spec.kind(),
            rounds: outcome.steps,
            usage: Usage {
                // What the rounds cost, split the way the turn counted it: everything it spent,
                // less what the model wrote, is what the requests carried.
                prompt_tokens: outcome.tokens.saturating_sub(outcome.output_tokens),
                completion_tokens: outcome.output_tokens,
            },
            inference: Duration::from_millis(outcome.timing.inference_ms),
        },
        vouched: Vouched {
            trust: outcome.trust,
            programs: outcome.programs,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The middle of a delegate's prompt is the planner's own, so guidance improved for one is
    /// improved for the other. A copy would drift, and what it would drift away from is the
    /// hard-won part: how to change a file nobody may read.
    #[test]
    fn every_kind_is_told_the_guidance_the_planner_is_told() {
        for name in Kind::NAMES {
            let kind = Kind::from_name(name).expect("enumerated");
            let prompt = prompt_for(kind);
            assert!(
                prompt.contains(turn::PLANNING),
                "a {name} was told something other than what the planner is told"
            );
        }
    }

    /// Said although the tools are absent, because a model that knows the shape of its situation
    /// plans around it instead of discovering it by being refused.
    #[test]
    fn each_kind_is_told_what_it_cannot_do() {
        let reader = prompt_for(Kind::Reader);
        assert!(reader.contains("You cannot write a file"));
        assert!(reader.contains("you cannot run a program"));

        let checker = prompt_for(Kind::Checker);
        assert!(checker.contains("You cannot write a file"));
        assert!(checker.contains("and run programs"));

        let worker = prompt_for(Kind::Worker);
        assert!(worker.contains("write files"));
        assert!(!worker.contains("You cannot write a file"));
    }

    /// The two things no delegate has, and the two the prompt has to be honest about: a model
    /// told to ask when it is stuck, with nothing to ask, ends a run on a question nobody reads.
    #[test]
    fn no_kind_is_told_it_may_ask_a_person_or_delegate() {
        for name in Kind::NAMES {
            let kind = Kind::from_name(name).expect("enumerated");
            let prompt = prompt_for(kind);
            assert!(
                prompt.contains("there is nobody to ask"),
                "a {name} was not told it has nobody to ask"
            );
            assert!(
                prompt.contains("You cannot delegate."),
                "a {name} was not told it cannot delegate"
            );
            assert!(
                !prompt.contains("use ask_user"),
                "a {name} was told to use a tool it does not have"
            );
            assert!(
                !prompt.contains("call todo_write"),
                "a {name} was told to use a tool it does not have"
            );
        }
    }

    /// The absence is what makes it true, and saying it is what stops a delegate planning around
    /// a fetch and spending a round finding out it cannot make one. Said once for all three
    /// kinds rather than kind by kind, because no kind has it.
    #[test]
    fn no_kind_is_told_it_may_reach_the_network() {
        for name in Kind::NAMES {
            let kind = Kind::from_name(name).expect("enumerated");
            let prompt = prompt_for(kind);
            assert!(
                prompt.contains("You cannot fetch a URL"),
                "a {name} was not told it cannot reach the network"
            );
            assert!(
                !prompt.contains("call fetch_url"),
                "a {name} was told to use a tool it does not have"
            );
        }
    }
}
