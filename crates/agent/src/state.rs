//! The bounded turn loop: state instead of history.
//!
//! A turn re-sends the whole conversation every round, so the request grows with the length of the
//! run and [`crate::compact`] exists to cut it back down. This is the other answer to the same
//! problem. The history is never sent at all. Each step carries three things:
//!
//! - the **task**, and the standing instructions and skills that came with it, unchanged all run
//! - the **state**, a structure the model maintains itself, which is the only thing that survives
//!   from one step to the next
//! - the **newest observation**, which is what the last action produced
//!
//! The model answers with a patch to the state and, ordinarily, a tool call. The patch is merged,
//! the words that produced it are dropped, and the next step sees the result. Nothing accumulates,
//! so the request has a size rather than a growth rate.
//!
//! # What this reuses, and why that is the whole design
//!
//! Every gate. Actions are ordinary tool calls dispatched through [`crate::tools::dispatch`], so a
//! write is approved by the same person, a read is quarantined by the same rule, a path is checked
//! against the same trust map, and the audit trail records the same lines. This mode changes what
//! the model is *shown* in order to choose an action. It changes nothing about what an action is
//! allowed to do.
//!
//! That is what keeps it honest. A run whose context is bounded is a run that has forgotten
//! things, and a design that also relaxed a gate on the strength of the state having been checked
//! would be trusting a summary the model wrote of its own reasoning. Nothing here does.
//!
//! # What it costs
//!
//! Everything the model failed to write down. The paper is explicit that the state has to be a
//! sufficient statistic for the rest of the run, and names the cases where it cannot be: a schema
//! nobody knows in advance, an observation whose relevance was not apparent when it arrived, and a
//! task whose object *is* the history. The third is worth spelling out here because it is the
//! ordinary case for a coding agent: "what did you just change, and why" is a question about the
//! trajectory, and this mode is the one that threw the trajectory away.
//!
//! So the transcript keeps everything. What is bounded is the request, exactly as with compaction:
//! the person watching sees every step, and the session record holds every one. The model is the
//! only party working from the state alone.

use bravebot_aichat::AichatClient;
use bravebot_aichat::protocol::{ChatRequest, Message, Tool, ToolCall};
use bravebot_config::Config;
use bravebot_core::cancel::Cancel;
use bravebot_core::capability::{Capability, CapabilitySet};
use bravebot_core::event::Sink;
use bravebot_core::policy::{Policy, ReleasePlan, Routing};
use bravebot_core::programs::TrustedPrograms;
use bravebot_core::reference::Presentation;
use bravebot_core::state::{Change, Patch, State, Value as StateValue};
use bravebot_core::trust::TrustStore;
use bravebot_core::value::Labelled;
use bravebot_net::Egress;
use serde_json::{Value, json};
use std::time::Instant;

use crate::confirm::Confirmer;
use crate::conversation::Conversation;
use crate::report::{Phase, Reporter};
use crate::timing::Elapsed;
use crate::tools;
use crate::turn::{Outcome, Task, TurnError};

/// What the model is told about the way this mode works.
///
/// Appended to the ordinary system prompt rather than replacing it, because everything the turn
/// loop's prompt says about quarantine, references, processors and writes is still true: the tools
/// are the same tools and the gates are the same gates. What is added is the one thing that
/// differs, which is that the history is not coming back.
///
/// Written plainly, and it says the cost out loud. A model that does not know its own reasoning is
/// about to be discarded will keep notes in prose that nobody will ever read back to it.
const STATE_PROMPT: &str = "\
\n\nHow this run works, which is different from the usual way:
\n\nYou are not shown the conversation. Each step you receive the task, your own execution state, \
and the newest observation, and nothing else. What you are reading now is everything you get. Your \
reasoning this step is discarded as soon as your state update is recorded, and the observations and \
actions of earlier steps are already gone.
\n\nSo the execution state is your memory, and it is the only one you have. Anything you will need \
later has to be in it before the step ends. Anything you leave out is gone, and you will not be \
able to tell that it is missing.
\n\n**Every step, call update_state, in the same reply as whatever else you do.** A step that calls \
a tool and does not call update_state throws away everything it worked out, including the sentence \
it just wrote: saying \"I have read a.txt and it contains alpha\" records nothing, because nobody \
reads your words back to you. Put it in the patch or lose it.
\n\nPass a patch holding just the keys that changed:
\n\n- a key you set is set, and a key you do not mention keeps the value it had
- a key set to null is deleted
- an object merges into the object already there, key by key, so touching one field of a group \
leaves the rest of the group alone
\n\nNever restate the whole state to keep it. Omitting a key is how you leave it alone, not how you \
remove it, so the shortest correct patch is the right one.
\n\nWhat to keep in it: what you have established, what you have ruled out and why, where you are \
in the work, and what remains. Record a failed attempt as well as a successful one, or you will try \
it again several steps from now with no idea that you already have.
\n\nReferences such as ref:0 keep working across steps, so a reference is worth keeping in the \
state, with a note of what it was about. You still cannot read a quarantined one.
\n\nThe state has a size limit. If a patch is refused for making it too large, drop what the rest \
of the work does not need rather than trimming what it does.
\n\nWhen the work is done, write the answer to the task in words, in that same reply, and call no \
tool but update_state. A reply with a state update and no words in it ends the run having said \
nothing to the person who asked, so the words are the part that matters and the patch goes beside \
them. Do not end on a description of what you are about to do next: either do it, or say plainly \
that you have not.";

/// The name of the one tool this mode adds.
const UPDATE_STATE: &str = "update_state";

/// The tool the model carries its patch in.
///
/// A tool call rather than a fenced block in the reply, which is how the paper does it, and the
/// difference is worth stating. A block in prose has to be found before it can be read, and
/// finding a document inside a paragraph is the thing [`crate::manifest`] refuses to do because a
/// document recovered from prose is a document nobody wrote. A tool call arrives in a field of its
/// own, already delimited by the transport, so there is nothing to search and no way to pick the
/// wrong one. It also means a malformed patch comes back as one refused call rather than as an
/// unreadable turn.
fn update_state_tool() -> Tool {
    Tool::function(
        UPDATE_STATE,
        "Record what this step established, in your execution state. Call this exactly once every \
         step. Pass only the keys that changed: a key you omit keeps its value, a key set to null \
         is deleted, and an object merges into the object already there rather than replacing it.",
        json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "object",
                    "description": "The keys that changed, and only those. Values may be strings, \
                                    whole numbers, booleans, arrays, or nested objects. Set a key \
                                    to null to delete it. An object merges key by key, so passing \
                                    {\"inventory\":{\"shelf_2\":\"item_z\"}} changes shelf_2 and \
                                    leaves every other shelf alone."
                }
            },
            "required": ["patch"]
        }),
    )
}

/// The tools one step is offered: everything a turn has, plus the state patch.
fn offered() -> Vec<Tool> {
    let mut tools = tools::available();
    tools.push(update_state_tool());
    tools
}

/// Why a patch could not be read out of a call.
///
/// Separate from [`bravebot_core::state::StateError`], which is about a patch that was read and
/// would not merge. Both come back to the model as an observation, because both are things it can
/// do differently next step.
#[derive(Debug)]
enum PatchError {
    /// The arguments were not JSON at all.
    NotJson(String),
    /// There was no `patch` object in them.
    NoPatch,
    /// A value used a JSON shape the state has no case for.
    Unsupported { key: String },
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotJson(detail) => write!(f, "the arguments were not valid JSON: {detail}"),
            Self::NoPatch => write!(
                f,
                "there was no 'patch' object in the arguments; pass the keys that changed under \
                 'patch'"
            ),
            Self::Unsupported { key } => write!(
                f,
                "'{key}' held something the state cannot store; use a string, a whole number, a \
                 boolean, an array, or an object"
            ),
        }
    }
}

/// Read a patch out of what the model passed to `update_state`.
///
/// Decoding only. Whether the patch may be adopted is
/// [`bravebot_core::policy::Policy::adopt_state_patch`], and whether the state it produces is
/// within bounds is [`bravebot_core::state::State::merged`]: this turns JSON into the kernel's
/// shape and decides nothing else.
fn read_patch(arguments: &Value) -> Result<Patch, PatchError> {
    let object = arguments
        .get("patch")
        .and_then(Value::as_object)
        .ok_or(PatchError::NoPatch)?;

    let mut patch = Patch::new();
    for (key, value) in object {
        patch = patch.with(key.clone(), read_change(key, value)?);
    }
    Ok(patch)
}

/// One JSON value as one change to one key.
fn read_change(key: &str, value: &Value) -> Result<Change, PatchError> {
    match value {
        // The paper's null-deletion, which is why the patch type has a variant for it rather than
        // treating an absent key and a null one alike.
        Value::Null => Ok(Change::Delete),
        // An object merges. This is the case that makes a patch about one shelf leave the other
        // four hundred and ninety-nine alone.
        Value::Object(fields) => {
            let mut nested = Patch::new();
            for (name, field) in fields {
                nested = nested.with(name.clone(), read_change(name, field)?);
            }
            Ok(Change::Merge(nested))
        }
        other => read_value(key, other).map(Change::Set),
    }
}

/// One JSON value as one state value.
///
/// A closed set, as the kernel's is. A shape with no case here is refused and said so, rather than
/// coerced into the nearest thing that fits: the paper measured type coercion as a fifth of all
/// failures on smaller models, and a runtime that quietly turned a number into a string would be
/// making that worse while appearing to work.
fn read_value(key: &str, value: &Value) -> Result<StateValue, PatchError> {
    match value {
        Value::String(text) => Ok(StateValue::Text(text.clone())),
        Value::Bool(flag) => Ok(StateValue::Bool(*flag)),
        Value::Number(number) => number
            .as_i64()
            .map(StateValue::Number)
            // A float. Refused rather than truncated, because a state that silently rounded would
            // be a state that disagrees with what the model believes it recorded.
            .ok_or_else(|| PatchError::Unsupported {
                key: key.to_string(),
            }),
        Value::Array(entries) => entries
            .iter()
            .map(|entry| read_value(key, entry))
            .collect::<Result<Vec<_>, _>>()
            .map(StateValue::List),
        Value::Object(fields) => fields
            .iter()
            .map(|(name, field)| read_value(name, field).map(|v| (name.clone(), v)))
            .collect::<Result<std::collections::BTreeMap<_, _>, _>>()
            .map(StateValue::Map),
        Value::Null => Err(PatchError::Unsupported {
            key: key.to_string(),
        }),
    }
}

/// How an observation is introduced to the model.
///
/// A fixed prefix in the driver's own words, so the model can tell the observation from the state
/// beside it. Not localized: this goes to the model, and [LOCALE-1] keeps the catalog for what a
/// person reads.
const OBSERVATION: &str = "Latest observation";

/// What one step's request carries, built fresh each step and thrown away after it.
///
/// This is the whole of the bound. The request is the system prompt, the task, the state, and the
/// newest observation, and it is assembled from those four every time rather than appended to, so
/// there is no path by which a step's history can survive into the next one. A function that built
/// this by pushing onto a growing vector would be the turn loop again with extra steps.
fn request_for(
    system: &str,
    task: &str,
    state: &State,
    observation: Option<&str>,
    model: &str,
    may_act: bool,
) -> ChatRequest {
    let mut messages = vec![Message::system(system), Message::user(task)];

    // The state is sent as one user message rather than folded into the system prompt, because the
    // system prompt is the build's and this changes every step. Rendered by the kernel, which owns
    // the escaping: see `bravebot_core::state`.
    messages.push(Message::user(format!(
        "Your execution state:\n\n```json\n{}\n```",
        state.render()
    )));

    if let Some(observation) = observation {
        messages.push(Message::user(format!("{OBSERVATION}:\n\n{observation}")));
    }

    let request = ChatRequest::new(model, messages);
    // On the last step the tools are withdrawn, so the reply has nothing to do but answer. A
    // request offering none is not one a call can be answering, which is what makes the withdrawal
    // the end of the run rather than a suggestion.
    if may_act {
        request.with_tools(offered())
    } else {
        request
    }
}

/// How many steps a bounded run may take, whether or not anybody is watching.
///
/// **Unlike a turn, this applies even with a person in front of it**, and the difference is the
/// point of the mode. A turn that goes round in circles gets slower and more expensive every round,
/// because each one re-sends the whole history, so it becomes obvious and eventually hits a context
/// budget. A bounded run's request never grows: a loop here costs the same at step five thousand as
/// at step five, looks the same on screen, and has nothing at all to stop it. The one thing that
/// made an unbounded interactive turn safe is exactly the thing this mode removed.
///
/// So the caller's `rounds`, which the interface leaves unset, is a ceiling this floor sits under:
/// see [`crate::turn::Task::rounds`]. A person can still stop a run at any point, and the bound is
/// high enough that reaching it means something has gone wrong rather than that the work was long.
pub const MAX_STEPS: usize = 200;

/// Run a task in the bounded loop, continuing a session.
///
/// The signature [`crate::turn::resume`] has, deliberately: this is a turn loop, an interactive
/// session may be in this mode, and a caller that can run one must be able to run the other
/// without knowing which it has.
///
/// The conversation is borrowed and written to, exactly as a turn writes to it, and for the same
/// reason: it is the person's transcript and the session's record. What differs is that nothing
/// reads it back. A step's request is built from the state, so what accumulates here is the record
/// of the run and never its input.
#[allow(clippy::too_many_arguments)]
pub fn resume<S: Sink, C: Confirmer, R: Reporter>(
    config: &Config,
    egress: &Egress,
    workspace: &crate::workspace::Workspace,
    task: &Task,
    conversation: &mut Conversation,
    confirmer: &mut C,
    reporter: &mut R,
    sink: &mut S,
    trust: TrustStore,
    programs: TrustedPrograms,
    cancel: &Cancel,
) -> Result<Outcome, TurnError> {
    // First thing in the run, as in a turn, so the wall figure covers the reads that happen before
    // the first request: skill discovery, the preamble, and this mode's context files.
    let began = Instant::now();
    let mut spent = Elapsed::default();

    let mut routing = Routing::new();
    routing.insert_trusted("task", task.prompt.clone());
    for (index, file) in task.files.iter().enumerate() {
        routing.insert_trusted(format!("file_{index}"), file.clone());
    }

    let capabilities = CapabilitySet::from_iter([
        Capability::WebFetch,
        Capability::FileRead,
        Capability::FileWrite,
        Capability::ShellExec,
    ]);

    let mut policy = Policy::begin(routing, ReleasePlan::new(), capabilities, sink)
        .map_err(|d| TurnError::Precommit(d.to_string()))?
        .with_trust(trust)
        .with_programs(programs)
        .resuming(conversation.context());

    let (catalogue, mut notices) =
        crate::skills::discover(&mut policy, workspace, task.home.as_deref());
    let preamble =
        crate::preamble::compose(&mut policy, workspace, task.home.as_deref(), &catalogue);
    notices.extend(preamble.notices.iter().cloned());
    for notice in &notices {
        reporter.notice(notice.message.clone());
    }

    // The task, the standing instructions, and this mode's own rules. Fixed for the whole run,
    // which is what the paper calls the immutable specification: it is the one part of the request
    // that is the same at step two hundred as at step one.
    let system = format!(
        "{}{}{STATE_PROMPT}",
        crate::turn::system_prompt(),
        preamble.text
    );

    // Context files go into the transcript the same way a turn's do, so the person sees what the
    // run was given. They reach the model as the first observation rather than as history: there is
    // no history to put them in.
    let mut observation = String::new();
    for index in 0..task.files.len() {
        let path = policy
            .routing()
            .get(&format!("file_{index}"))
            .expect("routing was precommitted with this key")
            .to_string();
        policy.vouch_for_named_path(&path);
        let contents = workspace.read(&mut policy, &Labelled::trusted(path.clone()))?;
        conversation.observed(policy.context_integrity());

        let slot = conversation.next_reference();
        let presented = policy
            .present("chat", slot, &path, &contents, conversation.quarantine())
            .map_err(|d| TurnError::Precommit(d.to_string()))?;

        let line = match &presented {
            Presentation::Visible(body) => format!("Contents of {path}:\n\n{body}"),
            Presentation::Quarantined(reference) => format!(
                "{path} could not be shown to you.\n\n{}",
                reference.describe()
            ),
        };
        conversation.push(Message::user(line.clone()));
        observation.push_str(&line);
        observation.push_str("\n\n");
    }

    conversation.push(Message::user(task.prompt.clone()));

    let mut subscription = crate::turn::discover_subscription(config, reporter);

    let mut state = State::new();
    let mut steps = 0usize;
    let mut tokens = 0u64;
    let mut output_tokens = 0u64;
    // What one step's request came to, not a running total. The claim this mode makes is about
    // exactly this figure: that it is the same at step two hundred as at step one.
    let mut context_tokens;
    // What the last step produced, which is the only thing carried forward besides the state.
    let mut latest = (!observation.trim().is_empty()).then(|| observation.trim().to_string());
    // Whether the model may still act. Cleared once, when the step budget runs out, so the last
    // request goes out with no tools and the run ends on an answer rather than on an apology.
    let mut may_act = true;
    // A bound applies here even with a person watching, unlike a turn. The caller's figure is a
    // ceiling rather than the whole rule: a request that never grows means a loop costs the same at
    // step five thousand as at step five and looks the same on screen, so the thing that makes an
    // unbounded interactive turn safe is missing. See [`MAX_STEPS`].
    let limit = task.rounds.unwrap_or(MAX_STEPS).min(MAX_STEPS);

    let completion = loop {
        if cancel.is_cancelled() {
            return Err(TurnError::Cancelled);
        }

        reporter.phase(Phase::of_round(steps));

        let model = task.model.as_deref().unwrap_or(&config.default_model);
        let request = request_for(
            &system,
            &task.prompt,
            &state,
            latest.as_deref(),
            model,
            may_act,
        );

        let written_before = output_tokens;
        let as_written = policy.authorise_display_release("the reply as the model writes it");
        let asked_at = Instant::now();
        let completion = {
            let mut client = AichatClient::new(config, egress).with_cancel(cancel.clone());
            if let Some(subscription) = subscription.as_mut() {
                client = client.with_subscription(subscription);
            }
            client.complete_streaming(&mut policy, &request, |progress| {
                reporter.output_tokens(written_before + progress.output_tokens);
                reporter.streaming(progress.written.declassify(&as_written).to_string());
            })?
        };
        spent.inference += asked_at.elapsed();
        tokens += completion.usage.total();
        output_tokens += completion.usage.completion_tokens;
        // What one step's request came to. Recorded rather than added up, because the point of the
        // mode is that this figure does not grow, and a total would hide that.
        context_tokens = completion.usage.prompt_tokens;

        // What the model said on the way to its calls, released to a screen exactly as a turn
        // releases it. This is the reasoning the paper discards, and discarding it means not
        // sending it again: the person watching still sees it, and the transcript still holds it.
        let proof = policy.authorise_display_release("what the model said this step");
        reporter.narration(completion.content.clone().declassify(&proof));

        // The budget is spent, so this reply is the answer whatever it holds. A model that asked for
        // a tool anyway does not get one: the request offered none, so nothing here is answering a
        // call, and running them would put the run back in the loop the budget exists to end.
        if !may_act {
            if !completion.calls.is_empty() {
                reporter.narration(
                    "the step budget was spent, so the last calls were not run".to_string(),
                );
            }
            break completion;
        }

        // The state comes first, whatever order the model asked for things in. A step that read a
        // file and recorded what it found should record it even if the read fails, and a patch
        // applied after an action would be a patch describing a world that had already moved on.
        let (patch_calls, action_calls): (Vec<_>, Vec<_>) = completion
            .calls
            .iter()
            .partition(|call| tools::strip_namespace(&call.function.name) == UPDATE_STATE);

        let mut observations: Vec<String> = Vec::new();

        for call in &patch_calls {
            match apply_patch(&mut policy, &state, call, reporter) {
                Ok(next) => state = next,
                // The state is unchanged, and the model is told why in the next observation. A
                // refused patch does not end the run: it is the commonest thing a model gets wrong
                // in this mode, and the answer is to say so and let it try again.
                Err(said) => observations.push(said),
            }
        }
        // Nothing left to do and nothing said about the state: the model has answered. Checked
        // after the patch so a final step may record what it concluded.
        if action_calls.is_empty() && observations.is_empty() {
            // Unless it answered with nothing. A reply that is a state update and no words ends the
            // run having said nothing to the person who asked, and the state is not an answer: they
            // cannot see it, and it is written in note form for the model's own use.
            //
            // Observed in a real run, twice. The model finished the work, recorded it, and stopped,
            // leaving a session whose last line was a note about what it had learned and no reply.
            // Asking for the answer costs one request and is the difference between a finished task
            // and a silent one.
            let said_nothing = {
                let proof = policy.authorise_display_release("whether the reply was empty");
                completion
                    .content
                    .clone()
                    .declassify(&proof)
                    .trim()
                    .is_empty()
            };
            if !said_nothing {
                break completion;
            }
            observations.push(
                "(from the system, not the user) That reply had no words in it, only a state \
                 update, so nothing has been said to the user yet. They cannot see your state. \
                 Answer the task now, in words, from what your state holds."
                    .to_string(),
            );
        }

        // A step that acted and recorded nothing is about to forget what it just did, because the
        // words it said about the action go no further than this round. Silence here is not
        // neutral: it is the whole failure the mode is exposed to, arriving quietly.
        //
        // Observed in a real run. Asked to read three files one at a time, the model read the
        // first, said what it held, recorded nothing, and on the next step had neither the file nor
        // its own sentence about it. It then reported the second file's contents under the first
        // file's name and asked the user for the other two. Nothing was wrong with the state; there
        // simply was not one.
        //
        // So the reminder is an observation like any other, and it names what was lost rather than
        // scolding: what the model just said is the thing it is about to lose, and it is still on
        // screen for it to copy.
        if !action_calls.is_empty() && patch_calls.is_empty() {
            reporter.narration(
                "that step recorded nothing, so what it just learned would have been lost"
                    .to_string(),
            );
            observations.push(
                "(from the system, not the user) You did not call update_state on that step, so \
                 nothing you worked out was recorded. What you said has already been discarded and \
                 this observation is all that is left of it. Record what you learned from the \
                 result below, together with anything from the step before that is still needed. \
                 Then carry on with the task: recording is something you do alongside the work, \
                 never instead of it, and the user is still waiting for an answer in words."
                    .to_string(),
            );
        }

        steps += 1;

        // The step's own account of itself goes into the transcript, so a person reading back sees
        // a run of steps rather than a run of results. Labelled from the context, as everywhere.
        //
        // **Recorded here rather than above**, after the decision to carry on, because the last
        // step's words are the answer and the answer is written down once, after the loop. Pushed
        // before that decision they are written twice: the person's transcript shows the reply
        // doubled, the session file stores it doubled, and since a bounded session resumes as an
        // ordinary turn, the next turn sends the duplicate back to the model. The turn loop puts it
        // in the same place for the same reason.
        let spoken = {
            let (text, _) = completion.content.clone().into_parts_for_decoding();
            policy.label_model_output("chat", text)
        };
        let slot = conversation.next_reference();
        let presented = policy
            .present(
                "assistant",
                slot,
                "what the model said this step",
                &spoken,
                conversation.quarantine(),
            )
            .map_err(|d| TurnError::Precommit(d.to_string()))?;
        let replayed: Option<Vec<_>> = match &presented {
            Presentation::Visible(_) => completion.calls.iter().map(ToolCall::as_request).collect(),
            Presentation::Quarantined(_) => None,
        };
        conversation.push(match (&presented, &replayed) {
            (Presentation::Visible(text), Some(calls)) => {
                Message::assistant_calling(text.clone(), calls.clone())
            }
            (Presentation::Visible(text), None) => Message::assistant(text.clone()),
            (Presentation::Quarantined(reference), _) => {
                Message::assistant(format!("(a step ran. {})", reference.describe()))
            }
        });

        for call in &action_calls {
            if cancel.is_cancelled() {
                return Err(TurnError::Cancelled);
            }

            // Wrapped per call rather than once for the run, because the borrow has to be given
            // back for the next call. What it counted is taken off the tool figure below.
            let mut asking = crate::confirm::Timed::new(confirmer);
            let ran_at = Instant::now();
            let output = tools::dispatch(
                &mut policy,
                &mut tools::Tools {
                    workspace,
                    skills: &catalogue,
                    slots: conversation.quarantine(),
                    chat: crate::processor::Chat {
                        config,
                        egress,
                        subscription: subscription
                            .as_mut()
                            .map(|s| s as &mut dyn bravebot_aichat::Subscription),
                        model: task.model.as_deref(),
                        cancel: Some(cancel),
                    },
                    cancel,
                },
                &mut asking,
                reporter,
                call,
            );
            let took = ran_at.elapsed();
            let stalled = asking.waited();
            spent.stalled += stalled;
            spent.inference += output.inference;
            // Both taken off, so the four figures partition the run rather than double-count what
            // nests inside a call. Saturating: these are separate clocks.
            spent.tools += took
                .saturating_sub(stalled)
                .saturating_sub(output.inference);
            tokens += output.usage.total();
            output_tokens += output.usage.completion_tokens;
            conversation.observed(policy.context_integrity());

            let body = crate::turn::record_output(
                &mut policy,
                conversation,
                reporter,
                &output,
                call,
                replayed.is_some(),
            )?;
            observations.push(body);
        }

        // The budget is spent, so the tools are taken away rather than the run: the next request
        // offers none and the model answers with what its state holds. Ending here instead would
        // throw the work away and tell the user only that something went round in circles.
        //
        // Withdrawing tools and going round again, rather than making one special call here, is the
        // turn loop's own shape and is what keeps the last request on the ordinary path: it gets the
        // cancellation check at the top, the live token count, and the narration, none of which a
        // second call site would have had.
        if steps >= limit && may_act {
            may_act = false;
            reporter.narration(format!(
                "that is {limit} steps without an answer, so this run has to finish with what it \
                 has"
            ));
            observations.push(format!(
                "(from the system, not the user) You have taken {limit} steps and have no more. \
                 Answer now with what you know, from your state. If the work is not finished, say \
                 what you established, what stopped you, and what would let you finish."
            ));
        }

        // The one thing that survives besides the state. Everything else this step produced has
        // been recorded for the person and dropped from the input.
        latest = Some(observations.join("\n\n"));
    };

    let proof = policy.authorise_display_release("assistant reply");
    let display = completion.content.clone().declassify(&proof);

    let answer = {
        let (spoken, _) = completion.content.clone().into_parts_for_decoding();
        policy.label_model_output("chat", spoken)
    };
    let slot = conversation.next_reference();
    let presented = policy
        .present(
            "reply",
            slot,
            "your previous answer",
            &answer,
            conversation.quarantine(),
        )
        .map_err(|d| TurnError::Precommit(d.to_string()))?;
    conversation.push(Message::assistant(match &presented {
        Presentation::Visible(text) => text.clone(),
        Presentation::Quarantined(reference) => {
            format!("(you answered. {})", reference.describe())
        }
    }));
    conversation.observed(policy.context_integrity());

    let trust = policy.trust().clone();
    let programs = policy.programs().clone();

    spent.wall = began.elapsed();

    Ok(Outcome {
        reply: completion.content,
        model: completion.model,
        steps,
        trust,
        programs,
        tokens,
        output_tokens,
        context_tokens,
        premium: subscription.is_some(),
        clean: policy.finish(),
        display,
        notices: notices.into_iter().map(|n| n.message).collect(),
        attempt: None,
        timing: spent.finish(),
    })
}

/// Apply one `update_state` call, or say why it did not apply.
///
/// The error is a sentence for the model, because every way this fails is something the model can
/// do differently next step. Nothing here ends the run.
fn apply_patch<S: Sink, R: Reporter>(
    policy: &mut Policy<'_, S>,
    state: &State,
    call: &ToolCall,
    reporter: &mut R,
) -> Result<State, String> {
    let verb = crate::report::verb_for(UPDATE_STATE);
    reporter.tool_started(crate::report::Activity::running(verb, ""));

    let arguments = match call.arguments() {
        Ok(value) => value,
        Err(e) => {
            let said = PatchError::NotJson(e.to_string()).to_string();
            reporter.tool_finished(crate::report::Activity::running(verb, "").failed(said.clone()));
            return Err(said);
        }
    };

    let patch = match read_patch(&arguments) {
        Ok(patch) => patch,
        Err(e) => {
            let said = e.to_string();
            reporter.tool_finished(crate::report::Activity::running(verb, "").failed(said.clone()));
            return Err(said);
        }
    };

    // The gate. A patch is model output going back into the model's own context, and it is checked
    // the way a summary of the conversation is: refused once the context has gone untrusted.
    let labelled = policy.label_model_output(UPDATE_STATE, patch);
    let adopted = match policy.adopt_state_patch(&labelled) {
        Ok(patch) => patch,
        Err(denial) => {
            let said = denial.to_string();
            reporter.tool_finished(crate::report::Activity::running(verb, "").failed(said.clone()));
            return Err(said);
        }
    };

    let described = adopted.describe();
    match state.merged(&adopted) {
        Ok(next) => {
            reporter.tool_finished(crate::report::Activity::running(verb, "").done(described));
            Ok(next)
        }
        // The bound the mode exists for, reached. The state is untouched, and what comes back is
        // the kernel's own sentence, which says what would fix it.
        Err(e) => {
            let said = format!("the state was not updated: {e}");
            reporter.tool_finished(crate::report::Activity::running(verb, "").failed(said.clone()));
            Err(said)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(json: &str) -> Value {
        serde_json::from_str(json).expect("test JSON")
    }

    #[test]
    fn a_patch_of_plain_values_is_read() {
        let patch = read_patch(&arguments(
            r#"{"patch":{"dir":"/tmp","count":3,"done":true}}"#,
        ))
        .expect("a readable patch");

        let state = State::new().merged(&patch).expect("it merges");
        assert_eq!(
            state.get("dir"),
            Some(&StateValue::Text("/tmp".to_string()))
        );
        assert_eq!(state.get("count"), Some(&StateValue::Number(3)));
        assert_eq!(state.get("done"), Some(&StateValue::Bool(true)));
    }

    /// The paper's null-deletion, spelled the way a model spells it.
    #[test]
    fn a_null_deletes_the_key() {
        let state = State::new()
            .merged(&read_patch(&arguments(r#"{"patch":{"scratch":"x"}}"#)).expect("set"))
            .expect("merges");
        let state = state
            .merged(&read_patch(&arguments(r#"{"patch":{"scratch":null}}"#)).expect("delete"))
            .expect("merges");

        assert_eq!(state.get("scratch"), None);
    }

    /// The paper's worked example, end to end through the decoder: a nested null empties one shelf
    /// and leaves its neighbour alone.
    #[test]
    fn a_nested_null_deletes_only_that_key() {
        let state = State::new()
            .merged(
                &read_patch(&arguments(
                    r#"{"patch":{"inventory":{"shelf_41":"item_11","shelf_42":"item_12"}}}"#,
                ))
                .expect("a first patch"),
            )
            .expect("merges");

        let state = state
            .merged(
                &read_patch(&arguments(r#"{"patch":{"inventory":{"shelf_42":null}}}"#))
                    .expect("a nested delete"),
            )
            .expect("merges");

        let Some(StateValue::Map(inventory)) = state.get("inventory") else {
            panic!("the group is gone");
        };
        assert_eq!(inventory.get("shelf_42"), None);
        assert_eq!(
            inventory.get("shelf_41"),
            Some(&StateValue::Text("item_11".to_string())),
            "a neighbouring shelf was emptied too"
        );
    }

    /// An object merges rather than replacing, which is the property that keeps a patch about one
    /// field from dropping every sibling of it.
    #[test]
    fn an_object_in_a_patch_merges_rather_than_replacing() {
        let state = State::new()
            .merged(
                &read_patch(&arguments(
                    r#"{"patch":{"progress":{"read":true,"written":false}}}"#,
                ))
                .expect("a first patch"),
            )
            .expect("merges");

        let state = state
            .merged(
                &read_patch(&arguments(r#"{"patch":{"progress":{"written":true}}}"#))
                    .expect("a second patch"),
            )
            .expect("merges");

        let Some(StateValue::Map(progress)) = state.get("progress") else {
            panic!("the group is gone");
        };
        assert_eq!(progress.get("read"), Some(&StateValue::Bool(true)));
        assert_eq!(progress.get("written"), Some(&StateValue::Bool(true)));
    }

    #[test]
    fn a_list_is_read_as_a_list() {
        let patch = read_patch(&arguments(r#"{"patch":{"tried":["a","b"]}}"#)).expect("readable");
        let state = State::new().merged(&patch).expect("merges");
        assert_eq!(
            state.get("tried"),
            Some(&StateValue::List(vec![
                StateValue::Text("a".to_string()),
                StateValue::Text("b".to_string())
            ]))
        );
    }

    #[test]
    fn arguments_with_no_patch_are_refused_with_a_sentence_the_model_can_act_on() {
        let refused = read_patch(&arguments(r#"{"state":{"a":1}}"#)).expect_err("no patch");
        assert!(refused.to_string().contains("'patch'"), "{refused}");
    }

    /// A float is refused rather than truncated. A state that silently rounded would disagree with
    /// what the model believes it recorded.
    #[test]
    fn a_fractional_number_is_refused_rather_than_rounded() {
        let refused = read_patch(&arguments(r#"{"patch":{"ratio":0.5}}"#)).expect_err("a float");
        assert!(refused.to_string().contains("ratio"), "{refused}");
    }

    /// The bound, stated as a test over the thing that actually goes on the wire. A run of two
    /// hundred steps must send a request the same size as a run of one, because that is the only
    /// claim this mode makes.
    #[test]
    fn the_request_does_not_grow_with_the_number_of_steps() {
        let state = State::new()
            .merged(
                &read_patch(&arguments(r#"{"patch":{"where":"src/main.rs","found":2}}"#))
                    .expect("a patch"),
            )
            .expect("merges");

        let early = request_for(
            "system",
            "the task",
            &state,
            Some("an observation"),
            "m",
            true,
        );
        let late = request_for(
            "system",
            "the task",
            &state,
            Some("an observation"),
            "m",
            true,
        );

        // Same four messages whatever step it is: the system prompt, the task, the state, and one
        // observation. Nothing here counts steps, and nothing accumulates.
        assert_eq!(early.messages.len(), 4);
        assert_eq!(late.messages.len(), early.messages.len());
    }

    /// The first step has nothing to observe, so it sends three messages rather than four. A blank
    /// observation would be a line inviting the model to account for something that never happened.
    #[test]
    fn a_first_step_with_nothing_observed_sends_no_observation() {
        let request = request_for("system", "the task", &State::new(), None, "m", true);
        assert_eq!(request.messages.len(), 3);
        assert!(
            !request
                .messages
                .iter()
                .any(|m| m.content.text().contains(OBSERVATION))
        );
    }

    /// The state reaches the model through the kernel's renderer, so a value holding a quote mark
    /// cannot close the block it is in and add structure of its own.
    #[test]
    fn the_state_reaches_the_model_through_the_kernels_renderer() {
        let state = State::new()
            .merged(
                &read_patch(&arguments(
                    r#"{"patch":{"note":"a \" quote and a \n newline"}}"#,
                ))
                .expect("a patch"),
            )
            .expect("merges");

        let request = request_for("system", "the task", &state, None, "m", true);
        let sent = request.messages[2].content.text();
        assert!(sent.contains(r#"\""#), "{sent}");
        assert!(!sent.contains("a \" quote"), "{sent}");
    }

    /// A bound applies even where the caller sets none, which is what the interface passes. Unlike a
    /// turn: a turn that goes round in circles gets slower and dearer every round and eventually
    /// meets a context budget, while a bounded run's request never grows, so a loop here costs the
    /// same at step five thousand as at step five and nothing else would ever end it.
    #[test]
    fn an_unbounded_caller_still_gets_the_step_floor() {
        let unbounded = Task::new("go").with_rounds(None);
        assert_eq!(
            unbounded.rounds.unwrap_or(MAX_STEPS).min(MAX_STEPS),
            MAX_STEPS
        );

        // A caller asking for fewer keeps its own figure.
        let tighter = Task::new("go").with_rounds(Some(5));
        assert_eq!(tighter.rounds.unwrap_or(MAX_STEPS).min(MAX_STEPS), 5);

        // And one asking for more does not get it: the floor is a ceiling here.
        let looser = Task::new("go").with_rounds(Some(MAX_STEPS * 10));
        assert_eq!(looser.rounds.unwrap_or(MAX_STEPS).min(MAX_STEPS), MAX_STEPS);
    }

    /// The model is offered the ordinary tools and the state patch, so an action in this mode is
    /// the same action with the same gates. A mode that had to invent its own action space would be
    /// a second set of tools to keep in step with the first.
    #[test]
    fn a_step_is_offered_the_ordinary_tools_and_the_state_patch() {
        let offered = offered();
        let names: Vec<&str> = offered
            .iter()
            .map(|tool| tool.function.name.as_str())
            .collect();
        assert!(names.contains(&UPDATE_STATE), "{names:?}");
        assert!(names.contains(&"read_file"), "{names:?}");
        assert!(names.contains(&"write_file"), "{names:?}");
        assert_eq!(names.len(), tools::available().len() + 1);
    }
}
