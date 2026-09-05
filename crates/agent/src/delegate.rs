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
use bravebot_core::policy::Policy;
use bravebot_core::value::Labelled;
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
front of you is yours to do or to report back on.";

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

/// Run one delegate to completion.
///
/// The parent's policy is borrowed for the whole of it, for two reasons that happen to coincide.
/// It owns the audit trail, and one trail has to record both runs. And it is the only thing that
/// can say what the parent already holds, which is what a delegate's own grants are narrowed
/// against and what its trust map starts from.
#[allow(clippy::too_many_arguments)]
pub fn run<S: Sink, R: Reporter>(
    policy: &mut Policy<'_, S>,
    config: &bravebot_config::Config,
    egress: &bravebot_net::Egress,
    workspace: &crate::workspace::Workspace,
    home: Option<&std::path::Path>,
    model: Option<&str>,
    cancel: &bravebot_core::cancel::Cancel,
    confirmer: &mut dyn Confirmer,
    reporter: &mut R,
    spec: &DelegateSpec,
) -> Result<Delegated, TurnError> {
    // Cloned before the sink is lent out, because that borrow lasts as long as the delegate's own
    // policy does. Both are the person's standing decisions and a delegate inherits them: rules
    // written in advance about what to ask about do not stop applying because the asking moved.
    let trust = policy.trust().clone();
    let programs = policy.programs().clone();
    let permissions = policy.permissions().clone();

    let task = Task::delegated(spec.clone())
        .with_home(home.map(std::path::Path::to_path_buf))
        .with_model(model.map(str::to_string))
        .with_permissions(permissions);

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
        policy.sink(),
        trust,
        programs,
        cancel,
    )?;
    // Taken back before anything else, so a person who vouched for the build inside a delegate is
    // not asked again by the next one.
    policy.adopt_from_delegate(outcome.trust, outcome.programs);

    Ok(Delegated {
        report: outcome.answer,
        kind: spec.kind(),
        rounds: outcome.steps,
        usage: Usage {
            // What the rounds cost, split the way the turn counted it: everything it spent, less
            // what the model wrote, is what the requests carried.
            prompt_tokens: outcome.tokens.saturating_sub(outcome.output_tokens),
            completion_tokens: outcome.output_tokens,
        },
        inference: Duration::from_millis(outcome.timing.inference_ms),
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
}
