//! Plan then execute.
//!
//! The other mode in this crate is a loop: the model sees a result, decides what to do next,
//! sees that result, and so on. This one decides everything first. One model call turns the
//! task string into a step list, the kernel freezes it, and a driver walks it with no model in
//! the control path at all. Nothing a step reads can add a step, remove one, or change which
//! slot feeds which, because by the time the first byte is read the program is already fixed.
//!
//! This follows the plan-then-execute architecture in the SafeHouse specification, whose three
//! planning phases survive here as they are written there.
//!
//! # The phases
//!
//! 1. **Abstract planning**, in two calls, neither of which has tools.
//!    1. **Shape.** What has to happen, in plain words, as if a person were doing it. No JSON,
//!       no slots, no capability names. This is where the goal gets stated.
//!    2. **Fit.** The same work, expressed for a machine that cannot look at anything before
//!       deciding. This is the hard half, and separating it is what makes it hard *visibly*:
//!       a plan that assumed it would see a result has to become a transform here, or be
//!       declared impossible.
//!
//!    Two calls rather than one, and the reason it is still sound is not the count. It is that
//!    the planner's context holds the task string and the driver's own words and has never been
//!    shown anything else, which is as true of the second call as of the first. That is not a
//!    property of this mode: the planner is never shown untrusted content in either mode, since
//!    `Policy::present` quarantines it and hands over a reference.
//!    `Policy::before_planning` is where that is written down so a change has to get past it.
//!
//!    No step has read anything at this point either, which is a separate fact and worth
//!    keeping separate: the planning policy holds no read capability at all, so it is not that
//!    nothing happened to read, it is that nothing could have.
//!
//!    What is still refused is a *re-plan*: a plan that fails validation fails the run, and
//!    nothing replans once a step has read something.
//!
//!    Splitting also separates two failure modes that used to look identical from outside. A
//!    run that goes wrong now says whether the model misunderstood the goal, which the shape
//!    shows, or fitted it badly to the tool set, which the manifest shows, or was refused by a
//!    gate, which the audit trail shows. Both artefacts are kept and reported.
//!
//! 2. **Concrete mapping.** Deterministic, no model. Capability names become tool names through
//!    a table the model has never seen.
//! 3. **Structural validation.** Deterministic, no model, in the kernel.
//!    [`bua_core::policy::Policy::adopt_manifest`] refuses a plan from a context that met
//!    anything untrusted, then [`bua_core::manifest::validate`] refuses one that is not well
//!    formed. Any violation fails the run; a manifest is never half adopted.
//!
//! Phase 2 is thinner here than in the specification, and the reason is worth stating rather
//! than papering over. There the mapping injects provider URLs, MCP tool names, and API bases
//! from an operator registry, so that a malicious domain cannot reach the planner. A coding
//! agent working in one directory has no providers to inject: the whole of its world is the
//! workspace the user opened. What remains is the part that still buys something, which is that
//! the planner names capabilities and never tool names.
//!
//! The specification's `CAPABILITY_LABEL` has no counterpart here either, and its absence is an
//! improvement rather than a gap. A static table mapping capability to label cannot express a
//! trust map keyed by path, so the label a read produces stays what it already was: the kernel's
//! decision at the moment of observation, from the path and the user's own trust rules.
//!
//! # What this mode cannot do
//!
//! `edit_file` is unavailable, because locating a passage to replace means having read the file
//! and the planner has read nothing. `todo_write` is unavailable because the manifest is a
//! better task list than a narrated one, having been fixed in advance. And there is no
//! conversation to resume: the planner is never shown a result, so there is nothing for a second
//! turn to continue. A manifest run is one run, start to finish.

use bua_aichat::AichatClient;
use bua_aichat::protocol::{ChatRequest, Message};
use bua_config::Config;
use bua_core::cancel::Cancel;
use bua_core::capability::{Capability, CapabilitySet};
use bua_core::event::Sink;
use bua_core::label::{Confidentiality, Label};
use bua_core::manifest::{self, Arg, Draft, DraftStep, Manifest, Step, Tier};
use bua_core::policy::{Policy, ReleasePlan, Routing};
use bua_core::slot::{SlotId, SlotStore};
use bua_core::trust::TrustStore;
use bua_core::value::Labelled;
use bua_net::Egress;
use serde_json::Value;

use crate::confirm::{Confirmer, Decision, Intent, WriteRequest};
use crate::conversation::Conversation;
use crate::processor::Chat;
use crate::report::{Activity, Phase, Reporter};
use crate::turn::{Outcome, Task, TurnError};
use crate::workspace::Workspace;

/// What the first planning call is told.
///
/// Deliberately knows nothing about slots, capabilities or JSON. Asking for the goal in plain
/// words first is not politeness to the model: it is the artefact a person reads when a run
/// does the wrong thing, and it is the difference between "it misunderstood me" and "it
/// understood me and could not express it".
///
/// It must nonetheless say that **an agent will carry this out and can read the workspace**. An
/// earlier version left that out, on the grounds that the first call should not think about
/// mechanism at all, and the model drew the only other conclusion available: that it was being
/// asked the question itself, could not see the files, and should ask the user to paste them.
/// The fit call then correctly refused to express "wait for the user" as a static manifest, and
/// a run failed at phase one for want of a sentence. Never remove it again.
const SHAPE_PROMPT: &str = "\
You are planning work in a code workspace. Say what has to happen to finish the task, as a short
numbered list, in plain words. Describe the goal and the steps someone would take.

An agent will carry out what you describe. It can read files in the workspace, search them, and
write them. So plan the reading: say which files should be read and what should be worked out
from them. You are not being asked to do the work and you are not expected to have seen any code.

Never ask for anything to be pasted or provided. Nothing will answer you: this is the only thing
you will be asked, and a plan that waits for a reply cannot be carried out. If you need to see a
file, that is a step, not a question.

Do not write JSON and do not name tools. How each step is carried out is somebody else's problem.

Be specific about which files are involved where the task names them, and say plainly where it \
does not name them and something will have to be searched for. If the task cannot be finished by \
reading and writing files in this workspace, say so and say why.

Keep it under ten lines.";

/// What the second planning call is told, on top of the capability catalogue.
///
/// Its whole job is the translation the first call was allowed to ignore: turning "look at this
/// and then decide" into something a machine with no ability to decide can run.
const FIT_PROMPT: &str = "\
You are given a plan written as if the work could be done by someone who looks at things and \
then decides. Rewrite it as a static manifest for a machine that cannot do that.

There is no second turn. You will never be shown what a step produced, you cannot branch on it, \
and nothing can be added or changed once you have answered. Every step is fixed now.

The translation that matters: anywhere the plan says to look at something and then decide, the \
deciding has to happen inside a TRANSFORM, which is an isolated model that does see the text. \
Read into a slot, transform the slot, and act on what the transform produced. If some part of \
the plan cannot be expressed that way, say so rather than approximating it.";

/// The shape of the answer the second call must give.
///
/// It names capabilities, never tools. Kept apart from [`FIT_PROMPT`] because one says what the
/// job is and the other says what the output must look like, and the second is the part that
/// has to agree with the schema in the kernel.
const MANIFEST_PROMPT: &str = "\
Output ONLY valid JSON, with no prose, no explanation and no code fences. The shape is:

  {\"steps\": [{\"capability\": \"NAME\", \"args\": {...}}, ...]}

If the task cannot be done as a fixed list of these steps, output ONLY:

  {\"error\": \"<brief reason>\"}

Slots carry data between steps. A step that reads something names an out_slot to put it in, and \
a later step names that slot to use it. You choose the slot names; use short lowercase words. \
Every slot must be written by an earlier step before anything reads it, and no slot is written \
twice.

Rules that the manifest is checked against, so a plan breaking one is refused outright:

- Every read and every transform comes before the first write or answer. Nothing reads the \
  workspace after the plan has changed it.
- Paths are workspace-relative. No leading slash, no '..' anywhere in them.
- A write takes either contents or from_slot, never both and never neither.
- At most one ANSWER step, and it says which slot holds the answer.

You cannot read a file yourself, so you cannot know what one contains. To say something about a \
file's contents, read it into a slot and TRANSFORM that slot: the transform is an isolated model \
that does see the text. To answer the user, ANSWER a slot a transform produced. Only use \
FILE_WRITE with contents for a file whose whole body you can write from the task alone.";

/// How the planner is told about one capability.
///
/// The tool name is deliberately absent. What the planner picks from is a set of names and
/// argument lists, and the mapping from those to the code that runs is the driver's.
struct Advertised {
    capability: &'static str,
    tool: &'static str,
    tier: Tier,
    /// The kernel capability the step needs, where it needs one. A transform needs none: it is
    /// gated by `Policy::before_processor`, which decides what it may read rather than whether
    /// it may run.
    gate: Option<Capability>,
    summary: &'static str,
    args: &'static str,
}

/// The registry. Operator-controlled, never model-authored, and the only route from a
/// capability name to a tool name.
const REGISTRY: &[Advertised] = &[
    Advertised {
        capability: "FILE_READ",
        tool: "read_file",
        tier: Tier::Fetch,
        gate: Some(Capability::FileRead),
        summary: "Read one workspace file into a slot",
        args: "path*, out_slot*, offset, limit",
    },
    Advertised {
        capability: "FILE_LIST",
        tool: "list_files",
        tier: Tier::Fetch,
        gate: Some(Capability::FileRead),
        summary: "List workspace files under a directory into a slot",
        args: "directory*, pattern, out_slot*",
    },
    Advertised {
        capability: "FILE_SEARCH",
        tool: "search",
        tier: Tier::Fetch,
        gate: Some(Capability::FileRead),
        summary: "Find a literal substring in workspace files, into a slot",
        args: "pattern*, directory, include, out_slot*",
    },
    Advertised {
        capability: "TRANSFORM",
        tool: "process",
        tier: Tier::Process,
        gate: None,
        summary: "Summarise or rewrite slots into a new slot, through an isolated model \
                  with no tools",
        args: "reads* (list of slot names), instruction*, out_slot*",
    },
    Advertised {
        capability: "FILE_WRITE",
        tool: "write_file",
        tier: Tier::Act,
        gate: Some(Capability::FileWrite),
        summary: "Write a workspace file, from a slot or from a body you give here",
        args: "path*, and exactly one of contents or from_slot",
    },
    Advertised {
        capability: "ANSWER",
        tool: manifest::ANSWER,
        tier: Tier::Act,
        gate: None,
        summary: "Show the user what a slot holds. This is the only way a plan replies",
        args: "from_slot*",
    },
];

fn advertised_for(capability: &str) -> Option<&'static Advertised> {
    REGISTRY.iter().find(|entry| entry.capability == capability)
}

fn registered_tool(tool: &str) -> Option<&'static Advertised> {
    REGISTRY.iter().find(|entry| entry.tool == tool)
}

/// The catalogue the planner is shown: names, one-line summaries, and argument lists.
fn catalogue() -> String {
    let mut lines = String::from("Capabilities. Arguments marked * are required.\n");
    let mut tier = None;
    for entry in REGISTRY {
        if tier != Some(entry.tier) {
            tier = Some(entry.tier);
            lines.push_str(&format!("\n{}:\n", tier_heading(entry.tier)));
        }
        lines.push_str(&format!(
            "  {} - {}. args: {}\n",
            entry.capability, entry.summary, entry.args
        ));
    }
    lines
}

fn tier_heading(tier: Tier) -> &'static str {
    match tier {
        Tier::Fetch => "Tier 1, reads (no model involved)",
        Tier::Process => "Tier 2, transforms (isolated model, no tools)",
        Tier::Act => "Tier 3, actions (these come last)",
    }
}

/// Why a run could not start.
///
/// Distinct from a step failing: nothing has happened yet, and nothing will.
#[derive(Debug)]
pub enum PlanError {
    /// The planner said it could not do the task, in its own words.
    Refused(String),
    /// What came back was not the JSON shape asked for.
    Malformed(String),
    /// The planner named something the registry does not have.
    UnknownCapability(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(reason) => write!(f, "no plan was made: {reason}"),
            Self::Malformed(detail) => write!(f, "the plan was not usable: {detail}"),
            Self::UnknownCapability(name) => {
                write!(f, "the plan names '{name}', which is not a capability")
            }
        }
    }
}

/// Phase 1's output, read into the shape phase 2 works on.
///
/// Not validation. Anything this rejects is a document that is not a plan at all, which is a
/// different question from whether a plan is well formed. That question belongs to the kernel.
fn parse(source: &str) -> Result<Draft, PlanError> {
    let document: Value = serde_json::from_str(source.trim())
        .map_err(|e| PlanError::Malformed(format!("not JSON: {e}")))?;

    // The planner's own way of saying it cannot do this. Honoured rather than treated as a
    // malformed plan, because "I cannot plan this" is a useful answer and a person should get
    // it in the planner's words.
    if let Some(reason) = document.get("error").and_then(Value::as_str) {
        return Err(PlanError::Refused(reason.to_string()));
    }

    let steps = document
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| PlanError::Malformed("no 'steps' array".to_string()))?;

    let mut draft = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate() {
        let capability = step
            .get("capability")
            .and_then(Value::as_str)
            .ok_or_else(|| PlanError::Malformed(format!("step {index} names no capability")))?;

        let mut parsed = DraftStep::new(capability);
        if let Some(args) = step.get("args").and_then(Value::as_object) {
            for (key, value) in args {
                let arg = read_arg(value).ok_or_else(|| {
                    PlanError::Malformed(format!(
                        "step {index}: '{key}' is not text, a whole number, or a list of names"
                    ))
                })?;
                parsed = parsed.with(key.clone(), arg);
            }
        }
        draft.push(parsed);
    }

    Ok(Draft::new(draft))
}

/// One JSON value as one argument, or nothing.
///
/// Deliberately narrow. A manifest is a program the driver executes step by step, and a value
/// shape it has no case for would have to be interpreted at run time by something.
fn read_arg(value: &Value) -> Option<Arg> {
    match value {
        Value::String(text) => Some(Arg::Text(text.clone())),
        Value::Number(number) => number.as_u64().map(Arg::Count),
        Value::Array(entries) => entries
            .iter()
            .map(|entry| entry.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .map(Arg::List),
        _ => None,
    }
}

/// Phase 2. Capability names become tool names, deterministically and with no model.
///
/// The rewrite is the whole of it: a step keeps its arguments and gains the name of the code
/// that will run it. A capability the registry does not have fails here rather than at the
/// schema, so the message says what went wrong in the planner's own vocabulary.
fn map_to_concrete(draft: Draft) -> Result<Draft, PlanError> {
    let mut mapped = Vec::with_capacity(draft.steps.len());
    for step in draft.steps {
        let entry = advertised_for(&step.tool)
            .ok_or_else(|| PlanError::UnknownCapability(step.tool.clone()))?;
        mapped.push(DraftStep {
            tool: entry.tool.to_string(),
            args: step.args,
        });
    }
    Ok(Draft::new(mapped))
}

/// The summary shown before anything runs.
///
/// The specification calls this the task accomplishment template, and it is the point at which
/// a person can still say no to the whole thing. Every destination in it was fixed while the
/// only input in existence was the task string, so what is on the screen is what will happen.
fn template(plan: &Manifest) -> String {
    let mut lines = String::from("Plan, fixed before anything runs:\n");
    for (index, step) in plan.steps().iter().enumerate() {
        lines.push_str(&format!(
            "  {}. [{}] {}\n",
            index + 1,
            step.tier(),
            step.describe()
        ));
    }
    lines
}

/// Run one task as a manifest.
///
/// Two policies, and the split is the mode rather than an artefact. The first holds the
/// planning call and has no capability but the network, so a bug that tried to read a file
/// during planning is refused by the capability gate rather than by anyone remembering not to.
/// It is finished before the second begins, and the second's routing comes from the plan the
/// first produced, which is what "the routing lock happens before anything is fetched" means
/// when written as code.
#[allow(clippy::too_many_arguments)]
pub fn run<S: Sink, C: Confirmer, R: Reporter>(
    config: &Config,
    egress: &Egress,
    workspace: &Workspace,
    task: &Task,
    confirmer: &mut C,
    reporter: &mut R,
    sink: &mut S,
    trust: TrustStore,
    cancel: &Cancel,
) -> Result<Outcome, TurnError> {
    let mut subscription = config
        .premium_endpoint
        .as_deref()
        .and_then(crate::ImportedSubscription::discover);

    // Owned here rather than inside either half, so a failure in either one still comes back
    // with everything that got as far as existing.
    let mut attempt = Attempt::default();

    let planned = match plan(
        config,
        egress,
        task,
        reporter,
        sink,
        subscription.as_mut(),
        cancel,
        &mut attempt,
    ) {
        Ok(planned) => planned,
        Err(error) => return Err(stopped(attempt, error)),
    };

    match execute(
        config,
        egress,
        workspace,
        planned,
        confirmer,
        reporter,
        sink,
        trust,
        subscription.as_mut(),
        cancel,
        &mut attempt,
    ) {
        Ok(outcome) => Ok(outcome),
        Err(error) => Err(stopped(attempt, error)),
    }
}

/// Wrap a failure up with everything the run managed to produce.
///
/// A cancellation is left alone: the user stopped it, so there is nothing to inspect and a
/// report of a half-run they interrupted on purpose is noise.
fn stopped(attempt: Attempt, error: TurnError) -> TurnError {
    match error {
        TurnError::Cancelled => TurnError::Cancelled,
        other => TurnError::Manifest {
            attempt: Box::new(attempt),
            detail: other.to_string(),
        },
    }
}

/// What planning produced, ready for a driver.
struct Planned {
    plan: Manifest,
    /// The label the plan carries, which every field lifted out of it carries too.
    label: Label,
    model: String,
    tokens: u64,
    output_tokens: u64,
    /// Whether the planning policy refused anything.
    clean: bool,
}

/// Everything one run produced, whether or not it finished.
///
/// Every field is filled the moment it exists rather than at the end, because the run you most
/// need to look at is the one that stopped. A plan that would not parse, a manifest that failed
/// the schema, a step that errored halfway: each of those throws away the run, and each of them
/// leaves this behind.
///
/// Between them the fields separate failure modes that otherwise look identical from outside.
/// A wrong result is a model that misunderstood the goal, which [`Attempt::shape`] shows; a
/// model that understood it and could not fit it to a tool set that cannot look before deciding,
/// which [`Attempt::proposed`] shows verbatim; a plan that was well formed and did the wrong
/// thing, which [`Attempt::plan`] and [`Attempt::steps`] show; or a gate that refused, which the
/// audit trail shows.
///
/// All of it is released text. All of it came from a context holding the task string and the
/// driver's own words, or from the driver's own account of what it did, so none of it is a byte
/// anybody was not allowed to read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attempt {
    /// The goal and the steps, in plain words. Present once the first call has answered.
    pub shape: Option<String>,
    /// What the second call said, **verbatim**, whether or not it was a usable manifest.
    ///
    /// Kept raw on purpose. A manifest that failed to parse is the one case where the rendered
    /// plan does not exist and the model's actual words are the only thing to look at.
    pub proposed: Option<String>,
    /// The frozen plan, one line per step. Present once it validated.
    pub plan: Option<String>,
    /// What each step did, in order, including the one that failed.
    pub steps: Vec<String>,
}

impl Attempt {
    /// Record what a step did, or did not do.
    fn stepped(&mut self, index: usize, step: &Step, note: &str) {
        self.steps.push(format!(
            "{}. [{}] {}: {note}",
            index + 1,
            step.tier(),
            step.describe()
        ));
    }

    /// How the whole attempt reads to a person, for a failure report.
    pub fn describe(&self) -> String {
        let mut out = String::new();
        if let Some(shape) = &self.shape {
            out.push_str("goal, as understood\n");
            for line in shape.lines() {
                out.push_str(&format!("  {line}\n"));
            }
        }
        match (&self.plan, &self.proposed) {
            (Some(plan), _) => out.push_str(plan),
            // No frozen plan means it never validated, so the model's own words are all there is.
            (None, Some(proposed)) => {
                out.push_str("manifest proposed, which was not usable\n");
                for line in proposed.lines() {
                    out.push_str(&format!("  {line}\n"));
                }
            }
            (None, None) => {}
        }
        if !self.steps.is_empty() {
            out.push_str("steps that ran\n");
            for line in &self.steps {
                out.push_str(&format!("  {line}\n"));
            }
        }
        out
    }
}

/// Phase 1, in two calls, then phases 2 and 3.
#[allow(clippy::too_many_arguments)]
fn plan<S: Sink, R: Reporter>(
    config: &Config,
    egress: &Egress,
    task: &Task,
    reporter: &mut R,
    sink: &mut S,
    mut subscription: Option<&mut crate::ImportedSubscription>,
    cancel: &Cancel,
    attempt: &mut Attempt,
) -> Result<Planned, TurnError> {
    if cancel.is_cancelled() {
        return Err(TurnError::Cancelled);
    }

    let mut routing = Routing::new();
    routing.insert_trusted("task", task.prompt.clone());

    // The network and nothing else. A planner cannot read and cannot write, so the only thing
    // this policy can authorise is the calls that make the plan.
    let mut policy = Policy::begin(
        routing,
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::WebFetch]),
        sink,
    )
    .map_err(|d| TurnError::Precommit(d.to_string()))?;

    reporter.phase(Phase::Planning);

    // Files the user named go in as their paths, not their contents. The planner works from the
    // task string alone, so a file it should look at becomes a step rather than a paste, and
    // what it is told is a name the user typed.
    let mut opening = task.prompt.clone();
    if !task.files.is_empty() {
        opening.push_str("\n\nFiles the user named: ");
        opening.push_str(&task.files.join(", "));
    }

    // A real conversation, so the second call sees the first as a turn that happened rather
    // than as a quotation. It holds two exchanges and ends there: no tool result ever joins it,
    // which is what keeps its integrity where `before_planning` insists it stays.
    let mut history = Conversation::new();
    history.push(Message::user(opening));

    let mut tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut model = String::new();

    // Call one: the goal, in plain words.
    let shape = ask(
        config,
        egress,
        &mut policy,
        reporter,
        subscription.as_deref_mut(),
        cancel,
        "shape",
        SHAPE_PROMPT,
        &history,
        &mut tokens,
        &mut output_tokens,
        &mut model,
    )?;
    // Kept before anything else can go wrong, which is the point of keeping it at all.
    attempt.shape = Some(shape.clone());
    reporter.narration(format!("Goal, as understood:\n{shape}"));
    history.push(Message::assistant(shape.clone()));

    // Call two: the same work, fitted to a machine that cannot look before deciding.
    history.push(Message::user(
        "Now express that as a manifest, following the rules you were given.",
    ));
    let proposal_text = ask(
        config,
        egress,
        &mut policy,
        reporter,
        subscription,
        cancel,
        "fit",
        &format!("{FIT_PROMPT}\n\n{MANIFEST_PROMPT}\n\n{}", catalogue()),
        &history,
        &mut tokens,
        &mut output_tokens,
        &mut model,
    )?;

    // Kept verbatim and kept first. A manifest that will not parse has no rendered form, so
    // the model's own words are the only thing anyone can look at afterwards.
    attempt.proposed = Some(proposal_text.clone());

    let label = Label::new(policy.context_integrity(), Confidentiality::Public);
    let draft = parse(&proposal_text).map_err(|e| TurnError::Precommit(e.to_string()))?;
    let concrete = map_to_concrete(draft).map_err(|e| TurnError::Precommit(e.to_string()))?;

    // The same label the text arrived with, carried onto the value derived from it. Not an
    // assignment: a draft parsed out of trusted bytes is trusted for exactly as long as those
    // bytes were, and `adopt_manifest` checks that again rather than taking this on trust.
    let plan = policy
        .adopt_manifest(&Labelled::new(concrete, label))
        .map_err(|d| TurnError::Precommit(d.to_string()))?;

    attempt.plan = Some(template(&plan));

    Ok(Planned {
        plan,
        label,
        model,
        tokens,
        output_tokens,
        clean: policy.finish(),
    })
}

/// One planning call.
///
/// The gate comes first every time, and it is the same gate for the first call as for the
/// second: a plan may be asked for only from a planner whose context has been shown nothing but
/// trusted input. What comes back is labelled from that context and then read through
/// `read_trusted_content`, which hands over the bytes because they are trusted and refuses
/// outright otherwise. Everything a caller does with the result is a decision taken from this
/// text, which is why it may not be read any other way.
#[allow(clippy::too_many_arguments)]
fn ask<S: Sink, R: Reporter>(
    config: &Config,
    egress: &Egress,
    policy: &mut Policy<'_, S>,
    reporter: &mut R,
    subscription: Option<&mut crate::ImportedSubscription>,
    cancel: &Cancel,
    round: &'static str,
    system: &str,
    history: &Conversation,
    tokens: &mut u64,
    output_tokens: &mut u64,
    model: &mut String,
) -> Result<String, TurnError> {
    if cancel.is_cancelled() {
        return Err(TurnError::Cancelled);
    }

    policy
        .before_planning(round)
        .map_err(|d| TurnError::Precommit(d.to_string()))?;

    // No tools on the request. There is nothing for a planner to call, and no result for a
    // reply to be steered by.
    let request = ChatRequest::new(&config.model, history.with_system(system));

    let written_before = *output_tokens;
    let completion = {
        let mut client = AichatClient::new(config, egress);
        if let Some(subscription) = subscription {
            client = client.with_subscription(subscription);
        }
        client.complete_streaming(policy, &request, |progress| {
            reporter.output_tokens(written_before + progress.output_tokens);
        })?
    };

    *tokens += completion.usage.total();
    *output_tokens += completion.usage.completion_tokens;
    *model = completion.model;

    let (spoken, _) = completion.content.into_parts_for_decoding();
    let labelled = policy.label_model_output(round, spoken);
    policy
        .read_trusted_content(round, &labelled)
        .map_err(|d| TurnError::Precommit(d.to_string()))
}

/// The driver: walk the frozen program.
///
/// No model call decides anything here. The only one made at all is a transform's, and what it
/// produces goes straight into the slot the plan named, at the label taint gave that slot before
/// the call was made.
#[allow(clippy::too_many_arguments)]
fn execute<S: Sink, C: Confirmer, R: Reporter>(
    config: &Config,
    egress: &Egress,
    workspace: &Workspace,
    planned: Planned,
    confirmer: &mut C,
    reporter: &mut R,
    sink: &mut S,
    trust: TrustStore,
    subscription: Option<&mut crate::ImportedSubscription>,
    cancel: &Cancel,
    attempt: &mut Attempt,
) -> Result<Outcome, TurnError> {
    let Planned {
        plan,
        label,
        model,
        mut tokens,
        mut output_tokens,
        clean: planning_was_clean,
    } = planned;

    // The pre-execution routing lock. Every destination the plan will use goes in here, through
    // the gate that refuses anything not (T,pub), before a single byte has been read. A plan
    // that somehow arrived untrusted fails at this line even if it had passed everything above.
    let mut routing = Routing::new();
    routing.insert_trusted("plan", format!("{} steps", plan.len()));
    for (key, value) in plan.routing() {
        routing
            .insert(key, Labelled::new(value, label))
            .map_err(|d| TurnError::Precommit(d.to_string()))?;
    }

    // What the plan may show the user, named before the run rather than during it.
    let mut release = ReleasePlan::new();
    for slot in plan.released() {
        release = release.allow(slot);
    }

    let policy = Policy::begin(
        routing,
        release,
        CapabilitySet::from_iter([
            Capability::FileRead,
            Capability::FileWrite,
            Capability::WebFetch,
        ]),
        sink,
    )
    .map_err(|d| TurnError::Precommit(d.to_string()))?;
    let mut policy = policy.with_trust(trust);

    // Shown before the first step, which is the last moment at which the whole of what is about
    // to happen is still a proposal.
    reporter.narration(
        attempt
            .plan
            .clone()
            .expect("planning fills this before execution begins"),
    );

    let mut slots = SlotStore::new();
    let mut answer: Option<Labelled<String>> = None;
    let mut shown = String::new();
    let mut chat = Chat {
        config,
        egress,
        subscription: subscription.map(|s| s as &mut dyn bua_aichat::Subscription),
    };

    for (index, step) in plan.steps().iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(TurnError::Cancelled);
        }

        let entry = registered_tool(step.tool()).expect("a validated step names a registered tool");
        let activity = Activity::running(entry.capability, step.describe());
        reporter.tool_started(activity.clone());

        let outcome = run_step(
            &mut policy,
            workspace,
            &mut slots,
            &mut chat,
            confirmer,
            index,
            step,
            entry,
        );

        match outcome {
            Ok(done) => {
                tokens += done.tokens;
                output_tokens += done.output_tokens;
                if let Some(answered) = done.answer {
                    shown = answered.shown;
                    answer = Some(answered.value);
                }
                attempt.stepped(index, step, &done.note);
                reporter.tool_finished(activity.with_changes(done.changes).done(done.note));
            }
            Err(failure) => {
                // A step that fails ends the run. There is no next step to try instead: the
                // plan said this one comes first, and everything after it was written on the
                // assumption that it happened.
                attempt.stepped(index, step, &format!("FAILED, {failure}"));
                reporter.tool_finished(activity.failed(failure.clone()));
                let _ = policy.finish();
                return Err(TurnError::Precommit(format!(
                    "step {}: {failure}",
                    index + 1
                )));
            }
        }
    }

    // A plan with no ANSWER step still has to say something, and the driver's own account of
    // what it did is the only thing it can say without anyone reading untrusted bytes to write
    // it. Trusted, because the driver wrote every word of it from a count it kept itself.
    let reply = match answer {
        Some(answered) => answered,
        None => {
            shown = format!(
                "Ran the {} step(s) in the plan. It had no ANSWER step, so there is nothing to \
                 show.",
                plan.len()
            );
            Labelled::trusted(shown.clone())
        }
    };

    let trust = policy.trust().clone();
    Ok(Outcome {
        reply,
        attempt: Some(attempt.clone()),
        model,
        steps: plan.len(),
        clean: planning_was_clean && policy.finish(),
        trust,
        tokens,
        output_tokens,
        display: shown,
    })
}

/// What one step produced, for the driver's bookkeeping.
#[derive(Default)]
struct Done {
    note: String,
    changes: Vec<crate::diff::Change>,
    tokens: u64,
    output_tokens: u64,
    /// What this step answered the user with, where it was the one that answered.
    answer: Option<Answered>,
}

/// The one thing a step hands back in two forms.
///
/// The released text goes on a screen and the labelled value goes into the outcome, because a
/// caller that only had the released string would have lost the fact that it is untrusted.
struct Answered {
    shown: String,
    value: Labelled<String>,
}

/// Run one step. Errors are fatal to the run and say why.
#[allow(clippy::too_many_arguments)]
fn run_step<S: Sink, C: Confirmer>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    slots: &mut SlotStore,
    chat: &mut Chat<'_>,
    confirmer: &mut C,
    index: usize,
    step: &Step,
    entry: &'static Advertised,
) -> Result<Done, String> {
    if let Some(capability) = entry.gate {
        policy
            .before_capability(capability)
            .map_err(|d| d.to_string())?;
    }

    match step.tool() {
        "read_file" => fetch(policy, slots, step, |policy| {
            let path = locked(policy, index, "path")?;
            let offset = step.count("offset").unwrap_or(1).max(1) as usize;
            let limit = step
                .count("limit")
                .unwrap_or(u64::MAX)
                .min(usize::MAX as u64) as usize;
            let page = workspace
                .read_page(policy, &Labelled::trusted(path.clone()), offset, limit)
                .map_err(|e| e.to_string())?;
            let rendered = policy.render_in_place("read_file", &page, |page| page.lines.join("\n"));
            let note = policy.render_in_place("read_file", &page, |page| {
                crate::tools::tally(page.lines.len(), "line", "lines")
            });
            Ok((rendered, note, path))
        }),
        "list_files" => fetch(policy, slots, step, |policy| {
            let directory = locked(policy, index, "directory")?;
            let pattern = step.arg("pattern").and_then(Arg::text).map(str::to_string);
            let listing = workspace
                .list(
                    policy,
                    &Labelled::trusted(directory.clone()),
                    pattern.map(Labelled::trusted).as_ref(),
                )
                .map_err(|e| e.to_string())?;
            let rendered = policy.render_in_place("list_files", &listing, |l| l.files.join("\n"));
            let note = policy.render_in_place("list_files", &listing, |l| {
                crate::tools::tally(l.files.len(), "file", "files")
            });
            Ok((rendered, note, directory))
        }),
        "search" => fetch(policy, slots, step, |policy| {
            let pattern = locked(policy, index, "pattern")?;
            let directory = step
                .arg("directory")
                .and_then(Arg::text)
                .unwrap_or(".")
                .to_string();
            let include = step.arg("include").and_then(Arg::text).map(str::to_string);
            let hits = workspace
                .grep(
                    policy,
                    &Labelled::trusted(pattern.clone()),
                    &Labelled::trusted(directory),
                    include.map(Labelled::trusted).as_ref(),
                )
                .map_err(|e| e.to_string())?;
            let rendered = policy.render_in_place("search", &hits, |hits| {
                hits.matches
                    .iter()
                    .map(|m| format!("{}:{}: {}", m.path, m.line, m.text))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            let note = policy.render_in_place("search", &hits, |hits| {
                crate::tools::tally(hits.matches.len(), "match", "matches")
            });
            Ok((rendered, note, pattern))
        }),
        "process" => {
            let out_slot = step
                .out_slot()
                .expect("a validated transform names an output slot")
                .clone();
            // The instruction is the plan's, fixed before anything was read, so it is trusted
            // by the same provenance every other field of the plan is.
            let instruction = Labelled::trusted(step.text("instruction").to_string());
            let spec = policy
                .before_processor(&format!("step_{index}"), step.reads(), &instruction, slots)
                .map_err(|d| d.to_string())?;
            let processed =
                crate::processor::run(policy, chat, slots, &spec).map_err(|e| e.to_string())?;
            let note = policy.render_in_place("process", &processed.text, |text| {
                crate::tools::tally(text.lines().count(), "line", "lines")
            });
            let note = release_note(policy, note);
            policy
                .quarantine("process", out_slot, "a transform", &processed.text, slots)
                .map_err(|d| d.to_string())?;
            Ok(Done {
                note,
                tokens: processed.usage.total(),
                output_tokens: processed.usage.completion_tokens,
                ..Done::default()
            })
        }
        "write_file" => write(policy, workspace, slots, confirmer, index, step),
        manifest::ANSWER => {
            let slot = step
                .reads()
                .first()
                .expect("a validated answer names a slot")
                .clone();
            let content = policy
                .resolve(manifest::ANSWER, &slot, slots)
                .map_err(|d| d.to_string())?;
            // The release plan named this slot before the run began, so this is the one thing
            // in the mode that content could not have nominated for itself.
            let proof = policy
                .declassify(&slot, content.label())
                .map_err(|d| d.to_string())?;
            let text = content.clone().declassify(&proof);
            Ok(Done {
                note: format!(
                    "{} shown",
                    crate::tools::tally(text.lines().count(), "line", "lines")
                ),
                answer: Some(Answered {
                    shown: text,
                    value: content,
                }),
                ..Done::default()
            })
        }
        other => Err(format!("no handler for '{other}'")),
    }
}

/// Run a Tier 1 step and put what it read into the slot the plan named.
fn fetch<S: Sink>(
    policy: &mut Policy<'_, S>,
    slots: &mut SlotStore,
    step: &Step,
    read: impl FnOnce(
        &mut Policy<'_, S>,
    ) -> Result<(Labelled<String>, Labelled<String>, String), String>,
) -> Result<Done, String> {
    let out_slot = step
        .out_slot()
        .expect("a validated fetch names an output slot")
        .clone();
    let (content, note, origin) = read(policy)?;
    let note = release_note(policy, note);
    policy
        .quarantine(step.tool(), out_slot, &origin, &content, slots)
        .map_err(|d| d.to_string())?;
    Ok(Done {
        note,
        ..Done::default()
    })
}

/// A count, shaped inside the kernel and released to the line a person reads.
fn release_note<S: Sink>(policy: &mut Policy<'_, S>, note: Labelled<String>) -> String {
    let proof = policy.authorise_display_release("what a step produced");
    note.declassify(&proof)
}

/// Read a routing field out of the lock rather than out of the step.
///
/// The values are the same, and taking them from here is the point: the destination in force is
/// the one that was fixed before execution, so nothing between then and now can have moved it.
fn locked<S: Sink>(policy: &Policy<'_, S>, index: usize, field: &str) -> Result<String, String> {
    policy
        .routing()
        .get(&format!("step_{index}_{field}"))
        .map(str::to_string)
        .ok_or_else(|| format!("'{field}' was not locked into routing before the run"))
}

/// A Tier 3 write.
///
/// The same shape a turn's write has, and for the same reasons: the person sees the exact path
/// and the exact body before any endorsement exists, and the endorsement is minted for what they
/// saw. What differs is where the path came from, which here is the routing lock.
fn write<S: Sink, C: Confirmer>(
    policy: &mut Policy<'_, S>,
    workspace: &Workspace,
    slots: &SlotStore,
    confirmer: &mut C,
    index: usize,
    step: &Step,
) -> Result<Done, String> {
    let path = locked(policy, index, "path")?;

    let body = match step.arg("from_slot").and_then(Arg::text) {
        // Quarantined bytes going back into the workspace they came from, without the driver
        // reading one of them.
        Some(name) => {
            let slot = SlotId::new(name);
            let content = policy
                .resolve("write_file", &slot, slots)
                .map_err(|d| d.to_string())?;
            policy.declassify_into_workspace(&slot, &path, content)
        }
        // A body the plan carried. Trusted, because the plan was fixed while the task string
        // was the only input in existence.
        None => Labelled::new(step.text("contents").to_string(), Label::trusted_public()),
    };
    let body_label = body.label();

    let proof = policy.authorise_display_release("proposed write");
    let shown = body.clone().declassify(&proof);
    let existing = workspace.peek_for_review(&path);
    let replaced_age = workspace.age_of(&path);
    let intent = if existing.is_some() {
        Intent::Overwrite
    } else {
        Intent::Create
    };

    if policy.write_needs_approval(&path, body_label) {
        let request = WriteRequest {
            intent,
            existing: existing.clone(),
            path: path.clone(),
            contents: shown.clone(),
        };
        if confirmer.confirm_write(&request) == Decision::Reject {
            return Err(format!("the user did not approve writing {path}"));
        }
    }

    policy.issue_grant("file_write", "path", path.clone());
    workspace
        .write_endorsed(policy, &Labelled::trusted(path.clone()), &body)
        .map_err(|e| e.to_string())?;
    policy.reconcile_after_write(&path, body_label);

    let (note, changes) =
        crate::tools::change_report(intent, existing.as_deref(), &shown, replaced_age);
    Ok(Done {
        note,
        changes,
        ..Done::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry advertises what the schema validates. A capability offered here but unknown
    /// there would be refused after the planner had already used it, and a tool the schema knows
    /// with nothing advertising it is a handler nothing can reach.
    #[test]
    fn the_registry_and_the_kernel_schema_describe_the_same_tools() {
        let mut advertised: Vec<&str> = REGISTRY.iter().map(|entry| entry.tool).collect();
        let mut validated: Vec<&str> = manifest::tools().iter().map(|(tool, _)| *tool).collect();
        advertised.sort_unstable();
        validated.sort_unstable();
        assert_eq!(advertised, validated);
    }

    /// The tier a capability is advertised under is the tier its plan will be checked against,
    /// or the catalogue's "these come last" heading is a lie the planner acts on.
    #[test]
    fn every_capability_is_advertised_at_the_tier_it_is_validated_at() {
        for (tool, tier) in manifest::tools() {
            let entry = registered_tool(tool).expect("every validated tool is registered");
            assert_eq!(entry.tier, tier, "{tool} is advertised at the wrong tier");
        }
    }

    #[test]
    fn a_plan_naming_an_unregistered_capability_is_refused() {
        let draft = Draft::new(vec![DraftStep::new("SEND_EMAIL").with_text("to", "a@b.c")]);
        let failure = map_to_concrete(draft).expect_err("must refuse");
        assert!(matches!(failure, PlanError::UnknownCapability(_)));
    }

    /// The planner's own way of saying no. Treating it as a malformed plan would tell a person
    /// their agent broke when what happened is that it declined, and said why.
    #[test]
    fn a_declining_planner_is_reported_in_its_own_words() {
        let failure = parse(r#"{"error": "the task needs a browser"}"#).expect_err("must refuse");
        assert!(failure.to_string().contains("the task needs a browser"));
    }

    /// Prose around JSON is the most common thing a model does when told to emit only JSON, and
    /// it must fail rather than be salvaged: a plan half recovered from a paragraph is a plan
    /// nobody wrote.
    #[test]
    fn a_plan_wrapped_in_prose_is_refused() {
        for text in [
            "Sure! Here is the plan: {\"steps\": []}",
            "```json\n{\"steps\": []}\n```",
            "",
        ] {
            assert!(parse(text).is_err(), "'{text}' was accepted");
        }
    }

    /// Surrounding whitespace is formatting, not a different document.
    #[test]
    fn whitespace_around_a_plan_is_ignored() {
        let draft = parse("  \n{\"steps\": []}\n ").expect("whitespace is not content");
        assert!(draft.steps.is_empty());
    }

    #[test]
    fn argument_shapes_outside_the_grammar_are_refused() {
        let plan = r#"{"steps":[{"capability":"FILE_READ","args":{"path":{"nested":true}}}]}"#;
        assert!(matches!(parse(plan), Err(PlanError::Malformed(_))));
    }

    #[test]
    fn text_numbers_and_name_lists_are_read_as_arguments() {
        let plan = r#"{"steps":[{"capability":"FILE_READ","args":
            {"path":"a.md","limit":40,"out_slot":"doc"}}]}"#;
        let draft = parse(plan).expect("these are the three shapes a step takes");
        let args = &draft.steps[0].args;
        assert_eq!(args.get("path"), Some(&Arg::Text("a.md".to_string())));
        assert_eq!(args.get("limit"), Some(&Arg::Count(40)));
    }

    /// Phase 2 is the only route from a capability name to a tool name, and it must not carry
    /// the planner's arguments anywhere but through.
    #[test]
    fn concrete_mapping_renames_the_tool_and_changes_nothing_else() {
        let draft = parse(
            r#"{"steps":[{"capability":"FILE_READ","args":{"path":"a.md","out_slot":"doc"}}]}"#,
        )
        .unwrap();
        let mapped = map_to_concrete(draft.clone()).unwrap();
        assert_eq!(mapped.steps[0].tool, "read_file");
        assert_eq!(mapped.steps[0].args, draft.steps[0].args);
    }

    /// The catalogue is what the planner picks from. A tool name in it would be a name the
    /// planner could invent variations of, and the mapping to code is the driver's to make.
    #[test]
    fn the_catalogue_names_capabilities_and_never_tools() {
        let shown = catalogue();
        for entry in REGISTRY {
            assert!(
                shown.contains(entry.capability),
                "{} is missing",
                entry.capability
            );
            assert!(
                !shown.contains(entry.tool),
                "{} leaked into the catalogue",
                entry.tool
            );
        }
    }
}
