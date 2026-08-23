//! The frozen program a manifest run executes.
//!
//! A turn decides what to do next after each thing it reads. A manifest run decides everything
//! before it reads anything: the planner emits a step list, this module refuses it or freezes
//! it, and the driver then walks it with no model in the control path. Untrusted content can
//! still change the bytes a step carries. It cannot add a step, remove one, or change which
//! slot feeds which, because by the time any content exists the program is already fixed.
//!
//! This follows the plan-then-execute architecture in the SafeHouse specification, including
//! its tool schema: a static contract per tool, checked by a pure function, with any violation
//! failing the run rather than being repaired.
//!
//! # What validation is for
//!
//! Not defence against a hostile planner. A [`Draft`] only ever reaches [`validate`] after
//! [`crate::policy::Policy::adopt_manifest`] has established that it came from a context
//! holding nothing untrusted, so it is trusted content and examining it decides nothing an
//! attacker steers. Validation is what makes the frozen program *well formed*: every slot read
//! was written by an earlier step, no slot is written twice, no path leaves the workspace, and
//! nothing reads anything after the first action. Those are the properties the driver relies
//! on to run without asking questions, and a manifest that fails one of them is refused whole.
//!
//! # Tiers
//!
//! The specification's three tiers survive intact, because they are what makes a plan
//! reviewable before it runs:
//!
//! - [`Tier::Fetch`] reads. No model is involved at all: the driver calls the same workspace
//!   code an ordinary turn calls, and the result goes straight into a slot.
//! - [`Tier::Process`] transforms. An isolated model with no tools reads the slots its step
//!   names and writes one more.
//! - [`Tier::Act`] changes something or shows something. These are the only steps with a
//!   consequence outside the slot store, and they are the ones a person endorses.

use crate::slot::SlotId;
use std::collections::BTreeMap;
use std::fmt;

/// Which of the three tiers a step belongs to.
///
/// Ordered so the validator can say "no read after an action" without naming the tiers
/// individually: an action is the highest tier, and the sequence may not go back down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Reads something into a slot. Deterministic, no model.
    Fetch,
    /// Transforms slots into another slot, through an isolated model.
    Process,
    /// Writes a file or shows an answer. The only tier with an effect.
    Act,
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fetch => f.write_str("fetch"),
            Self::Process => f.write_str("process"),
            Self::Act => f.write_str("act"),
        }
    }
}

/// A value one of a step's arguments holds.
///
/// Three shapes, which is every shape a step in this tool set needs. Kept deliberately small:
/// a manifest is a program the driver executes, and an argument grammar wide enough to express
/// arbitrary structure would be an interpreter nobody asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    Text(String),
    Count(u64),
    List(Vec<String>),
}

impl Arg {
    /// The text, where this is text. `None` rather than a rendering, so a contract asking for a
    /// path cannot be satisfied by a number that happens to print like one.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            _ => None,
        }
    }

    pub fn count(&self) -> Option<u64> {
        match self {
            Self::Count(value) => Some(*value),
            _ => None,
        }
    }

    pub fn list(&self) -> Option<&[String]> {
        match self {
            Self::List(values) => Some(values),
            _ => None,
        }
    }

    /// Whether this holds nothing.
    ///
    /// The specification's `required` check catches absent, `""`, `[]` and `{}` alike, on the
    /// grounds that a field present and empty is a field the planner did not fill in. A count
    /// is never empty: zero is a value someone may have meant.
    fn is_empty(&self) -> bool {
        match self {
            Self::Text(value) => value.trim().is_empty(),
            Self::List(values) => values.is_empty(),
            Self::Count(_) => false,
        }
    }
}

/// One step as the planner proposed it, before any of it has been checked.
///
/// Named a draft rather than a step because it is not one yet. Nothing constructs a [`Step`]
/// except [`validate`], so a value of that type has been through the schema by the only route
/// there is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftStep {
    pub tool: String,
    pub args: BTreeMap<String, Arg>,
}

impl DraftStep {
    pub fn new(tool: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            args: BTreeMap::new(),
        }
    }

    pub fn with(mut self, key: impl Into<String>, value: Arg) -> Self {
        self.args.insert(key.into(), value);
        self
    }

    pub fn with_text(self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.with(key, Arg::Text(value.into()))
    }
}

/// A whole plan as proposed, before validation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    pub steps: Vec<DraftStep>,
}

impl Draft {
    pub fn new(steps: Vec<DraftStep>) -> Self {
        Self { steps }
    }
}

/// A step that passed the schema.
///
/// The fields the driver routes on are lifted out of the argument map during validation, so the
/// driver never has to go looking for them and cannot mistake one tool's convention for
/// another's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    tool: &'static str,
    tier: Tier,
    args: BTreeMap<String, Arg>,
    out_slot: Option<SlotId>,
    reads: Vec<SlotId>,
}

impl Step {
    /// The tool's name, from the static schema rather than from the plan. A validated step
    /// names a tool that exists, because a name that did not match one never got this far.
    pub fn tool(&self) -> &'static str {
        self.tool
    }

    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// The slot this step fills, where it fills one. An action fills none.
    pub fn out_slot(&self) -> Option<&SlotId> {
        self.out_slot.as_ref()
    }

    /// The slots this step reads, in the order the plan named them.
    pub fn reads(&self) -> &[SlotId] {
        &self.reads
    }

    pub fn arg(&self, key: &str) -> Option<&Arg> {
        self.args.get(key)
    }

    /// A text argument, or the empty string.
    ///
    /// Total because the schema already decided what must be present: a required text field is
    /// there by the time anything asks, and an optional one absent means the default the caller
    /// would have chosen anyway.
    pub fn text(&self, key: &str) -> &str {
        self.args.get(key).and_then(Arg::text).unwrap_or_default()
    }

    pub fn count(&self, key: &str) -> Option<u64> {
        self.args.get(key).and_then(Arg::count)
    }

    /// How this step reads to a person, for the summary shown before the run starts.
    pub fn describe(&self) -> String {
        let contract = contract_for(self.tool).expect("a validated step names a known tool");
        let subject = contract
            .headline
            .iter()
            .find_map(|field| self.args.get(*field).and_then(Arg::text))
            .unwrap_or("");

        let reads = match self.reads.as_slice() {
            [] => String::new(),
            slots => format!(
                " from {}",
                slots
                    .iter()
                    .map(SlotId::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let writes = match &self.out_slot {
            Some(slot) => format!(" into {slot}"),
            None => String::new(),
        };

        format!("{} {subject}{reads}{writes}", contract.verb)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A validated plan, frozen.
///
/// There is no way to add a step, and no way to change one. Construction is [`validate`] and
/// nothing else, which is what "fixed before any execution begins" means in a type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    steps: Vec<Step>,
}

impl Manifest {
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Every value the driver must lock into routing before the first step runs.
    ///
    /// The specification calls this the pre-execution routing lock, and it is the reason a
    /// manifest run can write a file without asking the model where: the destination was fixed
    /// while the only input in existence was the user's own task string. Returned as key/value
    /// pairs so the caller inserts them through [`crate::policy::Routing::insert`], which
    /// refuses anything not `(T,pub)` and is the check that actually holds.
    pub fn routing(&self) -> Vec<(String, String)> {
        let mut fields = Vec::new();
        for (index, step) in self.steps.iter().enumerate() {
            let contract = contract_for(step.tool).expect("a validated step names a known tool");
            for field in contract.routing {
                if let Some(value) = step.args.get(*field).and_then(Arg::text) {
                    fields.push((format!("step_{index}_{field}"), value.to_string()));
                }
            }
        }
        fields
    }

    /// The slots this plan will release to the user's screen.
    ///
    /// Named here so they can go into the release plan before the policy exists, which is what
    /// makes [`crate::policy::Policy::declassify`] able to refuse everything else: a slot
    /// nominated after content was read is a slot content nominated.
    pub fn released(&self) -> Vec<SlotId> {
        self.steps
            .iter()
            .filter(|step| step.tool == ANSWER)
            .flat_map(|step| step.reads.iter().cloned())
            .collect()
    }
}

/// Why a plan was refused.
///
/// Every variant names the step it failed on, because a plan is a program and "step 3 reads a
/// slot nothing wrote" is the only form of this message that helps anyone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    Empty,
    UnknownTool {
        step: usize,
        tool: String,
    },
    Missing {
        step: usize,
        tool: &'static str,
        field: &'static str,
    },
    WrongType {
        step: usize,
        tool: &'static str,
        field: &'static str,
        wanted: &'static str,
    },
    Unknown {
        step: usize,
        tool: &'static str,
        field: String,
    },
    /// Exactly one of a set of alternatives was required and that is not what arrived.
    Alternatives {
        step: usize,
        tool: &'static str,
        fields: &'static [&'static str],
    },
    /// Two steps write the same slot. A slot is written once, so one of them would lose.
    SlotWrittenTwice {
        step: usize,
        slot: String,
    },
    /// A step reads a slot no earlier step wrote. This is the forward-validity check.
    SlotNotYetWritten {
        step: usize,
        slot: String,
    },
    EscapingPath {
        step: usize,
        field: &'static str,
        path: String,
    },
    /// An action wrote a slot, which would let a later step consume what an effect produced.
    ActionFillsSlot {
        step: usize,
        tool: &'static str,
    },
    /// Something read the workspace after an action had already changed it.
    ReadAfterAction {
        step: usize,
        tool: &'static str,
    },
    /// More than one step answers the user.
    AnswerRepeated {
        step: usize,
    },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "the plan has no steps"),
            Self::UnknownTool { step, tool } => {
                write!(f, "step {step}: '{tool}' is not a capability in this plan")
            }
            Self::Missing { step, tool, field } => {
                write!(f, "step {step} ({tool}): '{field}' is required and empty")
            }
            Self::WrongType {
                step,
                tool,
                field,
                wanted,
            } => write!(f, "step {step} ({tool}): '{field}' must be {wanted}"),
            Self::Unknown { step, tool, field } => {
                write!(
                    f,
                    "step {step} ({tool}): '{field}' is not an argument it takes"
                )
            }
            Self::Alternatives { step, tool, fields } => write!(
                f,
                "step {step} ({tool}): give exactly one of {}",
                fields.join(" or ")
            ),
            Self::SlotWrittenTwice { step, slot } => write!(
                f,
                "step {step}: '{slot}' is already written by an earlier step; \
                 every slot is written once"
            ),
            Self::SlotNotYetWritten { step, slot } => {
                write!(f, "step {step}: '{slot}' is read before anything writes it")
            }
            Self::EscapingPath { step, field, path } => write!(
                f,
                "step {step}: '{field}' is '{path}', which is not inside the workspace"
            ),
            Self::ActionFillsSlot { step, tool } => write!(
                f,
                "step {step} ({tool}): an action writes no slot, so nothing can depend on \
                 what it did"
            ),
            Self::ReadAfterAction { step, tool } => write!(
                f,
                "step {step} ({tool}): every read and every processor comes before the first \
                 action"
            ),
            Self::AnswerRepeated { step } => {
                write!(f, "step {step}: the plan already answers the user")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

/// The tool a plan uses to show the user something.
///
/// Named as a constant because two things depend on it and a string typed twice is a string
/// that will disagree with itself: the release plan, and the driver's choice of what to print.
pub const ANSWER: &str = "answer";

/// The static contract for one tool.
///
/// This is the specification's `TOOL_SCHEMA`, and it plays the same role: the single place that
/// says what a step of this kind must look like, consulted by a pure function, with no
/// heuristics and nothing learned at runtime.
struct Contract {
    tool: &'static str,
    tier: Tier,
    /// Must be present and must not be empty.
    required: &'static [&'static str],
    /// Every argument this tool accepts. Anything else is a typo or an invention.
    accepted: &'static [&'static str],
    /// The one argument naming the slot this step fills.
    slot_output: Option<&'static str>,
    /// Arguments holding a list of slot names, each of which must already be written.
    slot_inputs: &'static [&'static str],
    /// Arguments holding a single slot name, likewise.
    slot_refs: &'static [&'static str],
    /// Arguments that must be a workspace-relative path.
    paths: &'static [&'static str],
    /// Arguments that must be a whole number where present.
    counts: &'static [&'static str],
    /// Exactly one of these must be present. Empty where the tool has no such choice.
    alternatives: &'static [&'static str],
    /// Arguments that decide where an effect lands, to be locked before execution.
    routing: &'static [&'static str],
    /// Which argument to name first when describing the step to a person.
    headline: &'static [&'static str],
    /// The driver's word for what this step does.
    verb: &'static str,
}

/// Every tool a manifest may name.
///
/// Shorter than the turn loop's set, and deliberately. `edit_file` is absent because locating a
/// passage means having read the file, and in this mode nothing has: the planner works from the
/// task string alone. `todo_write` is absent because the manifest already is the task list, and
/// a better one, since it was fixed in advance rather than narrated as work went.
const CONTRACTS: &[Contract] = &[
    Contract {
        tool: "read_file",
        tier: Tier::Fetch,
        required: &["path", "out_slot"],
        accepted: &["path", "out_slot", "offset", "limit"],
        slot_output: Some("out_slot"),
        slot_inputs: &[],
        slot_refs: &[],
        paths: &["path"],
        counts: &["offset", "limit"],
        alternatives: &[],
        routing: &["path"],
        headline: &["path"],
        verb: "read",
    },
    Contract {
        tool: "list_files",
        tier: Tier::Fetch,
        required: &["directory", "out_slot"],
        accepted: &["directory", "pattern", "out_slot"],
        slot_output: Some("out_slot"),
        slot_inputs: &[],
        slot_refs: &[],
        paths: &["directory"],
        counts: &[],
        alternatives: &[],
        routing: &["directory", "pattern"],
        headline: &["directory"],
        verb: "list",
    },
    Contract {
        tool: "search",
        tier: Tier::Fetch,
        required: &["pattern", "out_slot"],
        accepted: &["pattern", "directory", "include", "out_slot"],
        slot_output: Some("out_slot"),
        slot_inputs: &[],
        slot_refs: &[],
        paths: &["directory"],
        counts: &[],
        alternatives: &[],
        routing: &["pattern", "directory", "include"],
        headline: &["pattern"],
        verb: "search for",
    },
    Contract {
        tool: "process",
        tier: Tier::Process,
        required: &["reads", "instruction", "out_slot"],
        accepted: &["reads", "instruction", "out_slot"],
        slot_output: Some("out_slot"),
        slot_inputs: &["reads"],
        slot_refs: &[],
        paths: &[],
        counts: &[],
        alternatives: &[],
        routing: &[],
        headline: &[],
        verb: "process",
    },
    Contract {
        tool: "write_file",
        tier: Tier::Act,
        required: &["path"],
        accepted: &["path", "contents", "from_slot"],
        slot_output: None,
        slot_inputs: &[],
        slot_refs: &["from_slot"],
        paths: &["path"],
        counts: &[],
        alternatives: &["contents", "from_slot"],
        routing: &["path"],
        headline: &["path"],
        verb: "write",
    },
    Contract {
        tool: ANSWER,
        tier: Tier::Act,
        required: &["from_slot"],
        accepted: &["from_slot"],
        slot_output: None,
        slot_inputs: &[],
        slot_refs: &["from_slot"],
        paths: &[],
        counts: &[],
        alternatives: &[],
        routing: &[],
        headline: &[],
        verb: "answer",
    },
];

fn contract_for(tool: &str) -> Option<&'static Contract> {
    CONTRACTS.iter().find(|contract| contract.tool == tool)
}

/// Every tool the schema knows, with its tier.
///
/// Exists so the layer that advertises capabilities to a planner can be checked against the
/// layer that validates what comes back. Two tables that must agree and have no way to say so
/// are two tables that will disagree.
pub fn tools() -> Vec<(&'static str, Tier)> {
    CONTRACTS.iter().map(|c| (c.tool, c.tier)).collect()
}

/// Turn a proposal into a frozen program, or refuse it.
///
/// Pure: no model, no network, no filesystem, and no state carried between calls. The same
/// draft validates the same way every time, which is what lets a rejected plan be reported to
/// a person as a fact about the plan rather than as something that went wrong.
///
/// The order of the checks is not arbitrary. Slot bookkeeping happens as the walk proceeds, so
/// "read before written" is decided by position rather than by a second pass: a plan whose
/// steps are in the wrong order fails for that reason, not for a missing slot.
pub fn validate(draft: &Draft) -> Result<Manifest, ManifestError> {
    if draft.steps.is_empty() {
        return Err(ManifestError::Empty);
    }

    let mut steps = Vec::with_capacity(draft.steps.len());
    let mut written: Vec<SlotId> = Vec::new();
    let mut acted = false;
    let mut answered = false;

    for (index, draft_step) in draft.steps.iter().enumerate() {
        let Some(contract) = contract_for(&draft_step.tool) else {
            return Err(ManifestError::UnknownTool {
                step: index,
                tool: draft_step.tool.clone(),
            });
        };

        for key in draft_step.args.keys() {
            if !contract.accepted.contains(&key.as_str()) {
                return Err(ManifestError::Unknown {
                    step: index,
                    tool: contract.tool,
                    field: key.clone(),
                });
            }
        }

        for field in contract.required {
            match draft_step.args.get(*field) {
                Some(value) if !value.is_empty() => {}
                _ => {
                    return Err(ManifestError::Missing {
                        step: index,
                        tool: contract.tool,
                        field,
                    });
                }
            }
        }

        if !contract.alternatives.is_empty() {
            let given = contract
                .alternatives
                .iter()
                .filter(|field| {
                    draft_step
                        .args
                        .get(**field)
                        .is_some_and(|value| !value.is_empty())
                })
                .count();
            if given != 1 {
                return Err(ManifestError::Alternatives {
                    step: index,
                    tool: contract.tool,
                    fields: contract.alternatives,
                });
            }
        }

        for field in contract.counts {
            if let Some(value) = draft_step.args.get(*field)
                && value.count().is_none()
            {
                return Err(ManifestError::WrongType {
                    step: index,
                    tool: contract.tool,
                    field,
                    wanted: "a whole number",
                });
            }
        }

        for field in contract.paths {
            if let Some(value) = draft_step.args.get(*field) {
                let Some(path) = value.text() else {
                    return Err(ManifestError::WrongType {
                        step: index,
                        tool: contract.tool,
                        field,
                        wanted: "a path",
                    });
                };
                if !is_inside_workspace(path) {
                    return Err(ManifestError::EscapingPath {
                        step: index,
                        field,
                        path: path.to_string(),
                    });
                }
            }
        }

        let mut reads = Vec::new();
        for field in contract.slot_inputs {
            if let Some(value) = draft_step.args.get(*field) {
                let Some(names) = value.list() else {
                    return Err(ManifestError::WrongType {
                        step: index,
                        tool: contract.tool,
                        field,
                        wanted: "a list of slot names",
                    });
                };
                for name in names {
                    reads.push(SlotId::new(name.clone()));
                }
            }
        }
        for field in contract.slot_refs {
            if let Some(value) = draft_step.args.get(*field) {
                let Some(name) = value.text() else {
                    return Err(ManifestError::WrongType {
                        step: index,
                        tool: contract.tool,
                        field,
                        wanted: "a slot name",
                    });
                };
                reads.push(SlotId::new(name));
            }
        }

        // Forward validity. A slot must already be written by the time a step names it, which
        // is what stops a plan describing a cycle or depending on a step that never runs.
        for slot in &reads {
            if !written.contains(slot) {
                return Err(ManifestError::SlotNotYetWritten {
                    step: index,
                    slot: slot.as_str().to_string(),
                });
            }
        }

        let out_slot = match contract.slot_output {
            Some(field) => {
                let Some(name) = draft_step.args.get(field).and_then(Arg::text) else {
                    return Err(ManifestError::WrongType {
                        step: index,
                        tool: contract.tool,
                        field,
                        wanted: "a slot name",
                    });
                };
                let slot = SlotId::new(name);
                if written.contains(&slot) {
                    return Err(ManifestError::SlotWrittenTwice {
                        step: index,
                        slot: name.to_string(),
                    });
                }
                written.push(slot.clone());
                Some(slot)
            }
            None => None,
        };

        // An action producing a slot would mean a later step consuming what an effect returned,
        // and the plan would then be reasoning about the world it had just changed. Nothing in
        // this tool set does that, and the schema says so rather than leaving it to be true by
        // accident.
        if contract.tier == Tier::Act && out_slot.is_some() {
            return Err(ManifestError::ActionFillsSlot {
                step: index,
                tool: contract.tool,
            });
        }

        if acted && contract.tier < Tier::Act {
            return Err(ManifestError::ReadAfterAction {
                step: index,
                tool: contract.tool,
            });
        }
        if contract.tier == Tier::Act {
            acted = true;
        }

        if contract.tool == ANSWER {
            if answered {
                return Err(ManifestError::AnswerRepeated { step: index });
            }
            answered = true;
        }

        steps.push(Step {
            tool: contract.tool,
            tier: contract.tier,
            args: draft_step.args.clone(),
            out_slot,
            reads,
        });
    }

    Ok(Manifest { steps })
}

/// Whether a path stays inside the workspace.
///
/// The workspace refuses an escaping path anyway, so this decides nothing the filesystem layer
/// would not. It is here because a plan is shown to a person before it runs, and `../../.ssh`
/// in a step someone is being asked to approve is a thing they should never have had to spot.
/// Refusing it at validation means the plan is rejected whole rather than half executed.
fn is_inside_workspace(path: &str) -> bool {
    if path.starts_with('/') || path.starts_with('~') {
        return false;
    }
    // Windows-style roots and drive letters are not paths this workspace takes, and a backslash
    // is a legal filename byte on the platforms it does, so neither is special-cased: what is
    // checked is the component grammar the workspace itself resolves.
    !path.split('/').any(|component| component == "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &str, slot: &str) -> DraftStep {
        DraftStep::new("read_file")
            .with_text("path", path)
            .with_text("out_slot", slot)
    }

    fn process(reads: &[&str], out: &str) -> DraftStep {
        DraftStep::new("process")
            .with(
                "reads",
                Arg::List(reads.iter().map(|s| s.to_string()).collect()),
            )
            .with_text("instruction", "summarise")
            .with_text("out_slot", out)
    }

    fn answer(slot: &str) -> DraftStep {
        DraftStep::new(ANSWER).with_text("from_slot", slot)
    }

    #[test]
    fn a_gather_process_answer_plan_validates() {
        let plan = validate(&Draft::new(vec![
            read("README.md", "readme"),
            process(&["readme"], "summary"),
            answer("summary"),
        ]))
        .expect("this is the shape the mode exists for");

        assert_eq!(plan.len(), 3);
        assert_eq!(plan.steps()[0].tier(), Tier::Fetch);
        assert_eq!(plan.steps()[1].tier(), Tier::Process);
        assert_eq!(plan.steps()[2].tier(), Tier::Act);
    }

    /// The whole mode rests on the program being fixed in advance, and a program with no steps
    /// is a run that did nothing while reporting that it had followed a plan.
    #[test]
    fn an_empty_plan_is_refused() {
        assert_eq!(validate(&Draft::default()), Err(ManifestError::Empty));
    }

    /// A tool the schema does not know is not a tool the driver has a handler for. Accepting it
    /// would mean discovering that at execution time, with earlier steps already run.
    #[test]
    fn a_tool_outside_the_schema_is_refused() {
        let err = validate(&Draft::new(vec![
            DraftStep::new("edit_file")
                .with_text("path", "a.rs")
                .with_text("old_text", "x")
                .with_text("new_text", "y"),
        ]))
        .unwrap_err();
        assert!(matches!(err, ManifestError::UnknownTool { step: 0, .. }));
    }

    /// Forward validity is the property that makes the driver able to run without checking
    /// anything: every slot a step reads was filled by a step that already ran.
    #[test]
    fn reading_a_slot_before_anything_writes_it_is_refused() {
        let err = validate(&Draft::new(vec![
            process(&["readme"], "summary"),
            read("README.md", "readme"),
        ]))
        .unwrap_err();
        assert_eq!(
            err,
            ManifestError::SlotNotYetWritten {
                step: 0,
                slot: "readme".to_string()
            }
        );
    }

    /// A slot is written once. Two steps writing one would leave the second failing at run time
    /// against a store that refuses a second write, halfway through a plan a person approved.
    #[test]
    fn two_steps_writing_one_slot_are_refused() {
        let err = validate(&Draft::new(vec![
            read("a.md", "doc"),
            read("b.md", "doc"),
            answer("doc"),
        ]))
        .unwrap_err();
        assert_eq!(
            err,
            ManifestError::SlotWrittenTwice {
                step: 1,
                slot: "doc".to_string()
            }
        );
    }

    /// A required field that is present but blank is a field the planner did not fill in, and
    /// reading a file called "" is not what anyone meant by it.
    #[test]
    fn a_blank_required_field_counts_as_missing() {
        for blank in ["", "   "] {
            let err = validate(&Draft::new(vec![read(blank, "doc"), answer("doc")])).unwrap_err();
            assert_eq!(
                err,
                ManifestError::Missing {
                    step: 0,
                    tool: "read_file",
                    field: "path"
                }
            );
        }
    }

    #[test]
    fn an_empty_list_of_reads_counts_as_missing() {
        let err = validate(&Draft::new(vec![
            read("a.md", "doc"),
            process(&[], "summary"),
        ]))
        .unwrap_err();
        assert_eq!(
            err,
            ManifestError::Missing {
                step: 1,
                tool: "process",
                field: "reads"
            }
        );
    }

    /// A path leaving the workspace is refused here rather than at the filesystem, so a plan
    /// nobody should approve is never put in front of anybody to approve.
    #[test]
    fn a_path_outside_the_workspace_is_refused() {
        for path in [
            "/etc/passwd",
            "../secrets",
            "~/.ssh/id_rsa",
            "a/../../b",
            // Refused even though it resolves back inside. Deciding otherwise would mean the
            // kernel modelling a filesystem it has no access to, and a plan nobody can read at
            // a glance is a plan nobody should be asked to approve.
            "a/b/../c.txt",
        ] {
            let err = validate(&Draft::new(vec![read(path, "doc"), answer("doc")])).unwrap_err();
            assert!(
                matches!(err, ManifestError::EscapingPath { step: 0, .. }),
                "{path} was allowed"
            );
        }
    }

    #[test]
    fn ordinary_relative_paths_are_accepted() {
        for path in ["README.md", "src/main.rs", "..hidden", "a..b/c"] {
            let plan = validate(&Draft::new(vec![read(path, "doc"), answer("doc")]));
            assert!(plan.is_ok(), "{path} was refused");
        }
    }

    /// Nothing reads the workspace after the plan has changed it. A read placed after a write
    /// would be a plan reasoning about consequences it caused, which is the shape a turn has
    /// and a manifest deliberately does not.
    #[test]
    fn a_read_after_an_action_is_refused() {
        let err = validate(&Draft::new(vec![
            read("a.md", "doc"),
            DraftStep::new("write_file")
                .with_text("path", "b.md")
                .with_text("from_slot", "doc"),
            read("b.md", "again"),
        ]))
        .unwrap_err();
        assert_eq!(
            err,
            ManifestError::ReadAfterAction {
                step: 2,
                tool: "read_file"
            }
        );
    }

    /// Several actions in a row are fine: each one's destination was locked before the run, so
    /// position buys nothing extra. It is only going back to reading that is refused.
    #[test]
    fn several_actions_may_follow_one_another() {
        let plan = validate(&Draft::new(vec![
            read("a.md", "doc"),
            process(&["doc"], "out"),
            DraftStep::new("write_file")
                .with_text("path", "b.md")
                .with_text("from_slot", "out"),
            answer("out"),
        ]));
        assert!(plan.is_ok(), "{plan:?}");
    }

    #[test]
    fn a_write_must_give_exactly_one_source() {
        let both = DraftStep::new("write_file")
            .with_text("path", "b.md")
            .with_text("contents", "hello")
            .with_text("from_slot", "doc");
        let err = validate(&Draft::new(vec![read("a.md", "doc"), both])).unwrap_err();
        assert!(matches!(err, ManifestError::Alternatives { step: 1, .. }));

        let neither = DraftStep::new("write_file").with_text("path", "b.md");
        let err = validate(&Draft::new(vec![neither])).unwrap_err();
        assert!(matches!(err, ManifestError::Alternatives { step: 0, .. }));
    }

    /// A body the plan wrote is fixed before anything is read, so it is as trustworthy as
    /// the task string it came from. This is the one way a manifest run creates a file from
    /// nothing, and refusing it would leave the mode unable to write a file that is not a
    /// transformation of one it read.
    #[test]
    fn a_write_may_carry_a_body_the_plan_fixed_in_advance() {
        let plan = validate(&Draft::new(vec![
            DraftStep::new("write_file")
                .with_text("path", "notes.md")
                .with_text("contents", "# Notes\n"),
        ]));
        assert!(plan.is_ok(), "{plan:?}");
    }

    /// An argument the tool does not take is a plan that meant something the driver will not
    /// do. Silently dropping it would run a step nobody described.
    #[test]
    fn an_argument_outside_the_contract_is_refused() {
        let err = validate(&Draft::new(vec![
            read("a.md", "doc").with_text("encoding", "utf-16"),
        ]))
        .unwrap_err();
        assert!(matches!(err, ManifestError::Unknown { step: 0, .. }));
    }

    #[test]
    fn a_count_field_must_hold_a_number() {
        let err = validate(&Draft::new(vec![
            read("a.md", "doc").with_text("limit", "lots"),
        ]))
        .unwrap_err();
        assert_eq!(
            err,
            ManifestError::WrongType {
                step: 0,
                tool: "read_file",
                field: "limit",
                wanted: "a whole number"
            }
        );
    }

    #[test]
    fn only_one_step_may_answer() {
        let err = validate(&Draft::new(vec![
            read("a.md", "doc"),
            answer("doc"),
            answer("doc"),
        ]))
        .unwrap_err();
        assert_eq!(err, ManifestError::AnswerRepeated { step: 2 });
    }

    /// The release plan is built from the manifest before the policy exists. If a slot could
    /// reach it any other way, content would be nominating itself for release.
    #[test]
    fn only_the_answered_slot_is_named_for_release() {
        let plan = validate(&Draft::new(vec![
            read("a.md", "doc"),
            process(&["doc"], "summary"),
            answer("summary"),
        ]))
        .unwrap();
        assert_eq!(plan.released(), vec![SlotId::new("summary")]);
    }

    #[test]
    fn a_plan_that_answers_nothing_releases_nothing() {
        let plan = validate(&Draft::new(vec![
            read("a.md", "doc"),
            DraftStep::new("write_file")
                .with_text("path", "b.md")
                .with_text("from_slot", "doc"),
        ]))
        .unwrap();
        assert!(plan.released().is_empty());
    }

    /// Every destination an effect could land on is lockable before the first step runs. A
    /// routing field the manifest did not surface would be one the driver took from somewhere
    /// else, and by then something has been read.
    #[test]
    fn every_effect_destination_is_named_for_the_routing_lock() {
        let plan = validate(&Draft::new(vec![
            read("src/a.rs", "doc"),
            DraftStep::new("write_file")
                .with_text("path", "src/b.rs")
                .with_text("from_slot", "doc"),
        ]))
        .unwrap();
        let routing = plan.routing();
        assert!(routing.contains(&("step_0_path".to_string(), "src/a.rs".to_string())));
        assert!(routing.contains(&("step_1_path".to_string(), "src/b.rs".to_string())));
    }

    /// Validation is a function of the draft and nothing else, so the same plan is refused for
    /// the same reason every time and a person can be told what to change.
    #[test]
    fn validation_is_deterministic() {
        let draft = Draft::new(vec![process(&["missing"], "out")]);
        assert_eq!(validate(&draft), validate(&draft));
    }

    #[test]
    fn a_step_describes_itself_for_the_summary() {
        let plan = validate(&Draft::new(vec![
            read("README.md", "readme"),
            process(&["readme"], "summary"),
            answer("summary"),
        ]))
        .unwrap();
        let lines: Vec<String> = plan.steps().iter().map(Step::describe).collect();
        assert_eq!(lines[0], "read README.md into readme");
        assert_eq!(lines[1], "process from readme into summary");
        assert_eq!(lines[2], "answer from summary");
    }

    #[test]
    fn every_listed_tool_has_a_contract() {
        for (tool, _) in tools() {
            assert!(contract_for(tool).is_some(), "{tool} is listed only");
        }
    }
}
