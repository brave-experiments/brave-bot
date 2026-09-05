//! Delegated agents: a second planner, narrower than the first.
//!
//! A processor holds no capabilities at all, and the reason is written down in
//! [`crate::processor`]: a processor with one tool would be a second planner with untrusted
//! content in its context, which is the thing this design refuses. A delegate is the other half
//! of that sentence. It holds capabilities and it holds no untrusted content: it quarantines what
//! it may not read exactly as the first planner does, and gets a reference back. So the objection
//! to a processor with tools is not an objection to this.
//!
//! What it buys is a context nothing has to be told twice. A planner that runs the build reads
//! the whole log; one that asks a delegate to run the build is told what failed. The work happens
//! either way, and only one of them spends the conversation on it.
//!
//! Three things are fixed before a delegate exists, and none of them by the model:
//!
//! - **Its capabilities**, which are its kind's narrowed by the parent's. Delegation
//!   redistributes authority and never creates it, so a delegate can hold nothing the turn that
//!   spawned it did not already hold.
//! - **Its prompt**, which is a constant per kind. The planner names a kind and cannot describe
//!   one, so there is no sentence it can write that changes what a delegate is.
//! - **Its bound**, which is its kind's. Nobody is watching a delegate the way a person watches
//!   a turn, and the thing being bounded is futility rather than danger.
//!
//! It still asks. Every write and every run a delegate performs passes the same gates with the
//! same single-use endorsements, so a person sees the path and the diff whoever proposed them.
//! What a delegate saves is context, never approval.

use crate::capability::{Capability, CapabilitySet};

/// A kind of delegate: what it may hold, and how long it may go on.
///
/// Enumerated here rather than configured, so the planner selects from a list the driver wrote
/// and cannot describe a delegate of its own. That is the smaller half of the same decision the
/// prompt makes: a name that matches nothing in this enum resolves to nothing and the call is
/// refused, so there is no spelling of `kind` that reaches a capability set nobody chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Reads, lists, searches, and hands quarantined files to processors. Writes nothing and
    /// runs nothing.
    ///
    /// The shape whose value is mostly independence: a question about a tree, answered without
    /// the tree arriving in the asker's context.
    Reader,
    /// A reader that may also run programs, so it can build and test.
    ///
    /// Writes nothing, which is what makes it worth having separately: reporting that the tests
    /// fail does not need permission to change them, and a build log is the single most
    /// context-expensive thing a turn reads.
    Checker,
    /// A checker that may also write files.
    ///
    /// A whole sub-task, done somewhere else. Every write it makes is still shown to a person as
    /// a diff, and the endorsement is minted for the path they saw.
    Worker,
}

impl Kind {
    /// Every kind, in the order the planner is told about them: narrowest first.
    pub const NAMES: [&'static str; 3] = ["reader", "checker", "worker"];

    /// The kind this name selects, or nothing.
    ///
    /// A selection, not a lookup of anything the model wrote into a path or a table key. The set
    /// is this array, so a name naming a traversal, a capability, or anything else at all
    /// matches nothing.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "reader" => Some(Self::Reader),
            "checker" => Some(Self::Checker),
            "worker" => Some(Self::Worker),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reader => "reader",
            Self::Checker => "checker",
            Self::Worker => "worker",
        }
    }

    /// What this kind asks to hold, before the parent's own set narrows it.
    ///
    /// Every kind holds the one for reaching the network, because a planner *is* a model call and
    /// the request out is egress like any other. It is not a tool a delegate can point anywhere:
    /// nothing in any kind's tool set reaches it, so what it buys is the driver's ability to ask
    /// the endpoint on this delegate's behalf. A kind without it is a kind that cannot think.
    pub fn capabilities(self) -> CapabilitySet {
        match self {
            Self::Reader => CapabilitySet::from_iter([Capability::WebFetch, Capability::FileRead]),
            Self::Checker => CapabilitySet::from_iter([
                Capability::WebFetch,
                Capability::FileRead,
                Capability::ShellExec,
            ]),
            Self::Worker => CapabilitySet::from_iter([
                Capability::WebFetch,
                Capability::FileRead,
                Capability::FileWrite,
                Capability::ShellExec,
            ]),
        }
    }

    /// How many rounds of tool calls this kind may make before it has to answer.
    ///
    /// Bounded for every kind, and the bound rises with what the kind can do rather than with
    /// how much anybody trusts it: a delegate that may not write has less to be part-way
    /// through. Not a safety property, exactly as the turn's own bound is not: a gate refuses on
    /// the last round what it refuses on the first.
    pub fn rounds(self) -> usize {
        match self {
            Self::Reader => 60,
            Self::Checker => 80,
            Self::Worker => 120,
        }
    }

    /// What to tell the planner this kind is for, in the tool's own schema.
    pub fn purpose(self) -> &'static str {
        match self {
            Self::Reader => {
                "reads, lists, searches and runs processors; writes nothing and runs nothing"
            }
            Self::Checker => {
                "a reader that may also run programs, so it can build, test and lint; writes \
                 nothing"
            }
            Self::Worker => "a checker that may also write files, so it can finish a sub-task",
        }
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the driver fixed about one delegate before it ran.
///
/// Only [`crate::policy::Policy::before_delegate`] constructs one, and nothing here can widen it
/// afterwards. The delegate never sees this value: what reaches it is a prompt chosen by its
/// kind, a task, and a tool list derived from the capabilities recorded here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegateSpec {
    id: String,
    kind: Kind,
    task: String,
    capabilities: CapabilitySet,
    rounds: usize,
}

impl DelegateSpec {
    pub(crate) fn new(
        id: impl Into<String>,
        kind: Kind,
        task: impl Into<String>,
        capabilities: CapabilitySet,
        rounds: usize,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            task: task.into(),
            capabilities,
            rounds,
        }
    }

    /// The delegate's name in the audit trail. Driver-chosen, never derived from content.
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// What it was asked to do.
    ///
    /// The planner's own words, checked `(T,pub)` before the spec was built. Readable because a
    /// driver that could not hold it could not send it, and trusted because it is about to enter
    /// a planner's context.
    pub fn task(&self) -> &str {
        &self.task
    }

    /// What it holds: its kind's set, already narrowed by the parent's.
    ///
    /// Fixed here rather than looked up later, so nothing between this call and the run can
    /// widen it.
    pub fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// How many rounds it may make before it has to answer.
    pub fn rounds(&self) -> usize {
        self.rounds
    }

    /// The delegate as the audit trail describes it: what it is and what it holds, never the
    /// task, which can be long.
    pub fn describe(&self) -> String {
        let held: Vec<&str> = self.capabilities.iter().map(Capability::as_str).collect();
        let held = if held.is_empty() {
            "nothing".to_string()
        } else {
            held.join(", ")
        };
        format!(
            "{} is a {} delegate holding {} for at most {} rounds",
            self.id, self.kind, held, self.rounds
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The planner selects from the driver's list. Anything else has to resolve to nothing, or
    /// `kind` would be a field the model could write a capability set into.
    #[test]
    fn a_kind_is_selected_from_the_enumerated_set_and_nothing_else() {
        for name in Kind::NAMES {
            assert!(Kind::from_name(name).is_some(), "{name} must be a kind");
        }
        for name in [
            "",
            "Reader",
            "worker ",
            "../worker",
            "planner",
            "file_write",
        ] {
            assert!(Kind::from_name(name).is_none(), "{name} must not be a kind");
        }
    }

    /// The names the planner is shown are the names that resolve, so a list in a tool schema
    /// cannot drift from the set behind it.
    #[test]
    fn every_advertised_name_resolves_to_the_kind_it_names() {
        for name in Kind::NAMES {
            let kind = Kind::from_name(name).expect("advertised");
            assert_eq!(kind.as_str(), name);
        }
    }

    /// A wider kind holds everything a narrower one does. A delegate chosen for being able to
    /// build must not lose the ability to read in exchange.
    #[test]
    fn the_kinds_are_ordered_by_what_they_hold() {
        let reader = Kind::Reader.capabilities();
        let checker = Kind::Checker.capabilities();
        let worker = Kind::Worker.capabilities();

        for capability in reader.iter() {
            assert!(checker.contains(capability), "checker lost {capability}");
        }
        for capability in checker.iter() {
            assert!(worker.contains(capability), "worker lost {capability}");
        }
        assert!(!reader.contains(Capability::FileWrite));
        assert!(!reader.contains(Capability::ShellExec));
        assert!(!checker.contains(Capability::FileWrite));
    }

    /// A planner is a model call, so every kind can reach the endpoint and no kind can reach
    /// anything else off this machine. Without the first a delegate cannot think; with more than
    /// the first, a grant would exist that nothing a person approved asked for.
    #[test]
    fn every_kind_can_reach_the_endpoint_and_nothing_else_remote() {
        for name in Kind::NAMES {
            let kind = Kind::from_name(name).expect("advertised");
            let held = kind.capabilities();
            assert!(
                held.contains(Capability::WebFetch),
                "a {name} could not have made its own requests"
            );
            assert!(!held.contains(Capability::McpCall), "{name}");
            assert!(!held.contains(Capability::GitWrite), "{name}");
        }
    }

    /// Every kind is bounded. An unbounded delegate has nothing watching it: the person is
    /// watching the turn, and the turn is blocked.
    #[test]
    fn every_kind_carries_a_bound() {
        for name in Kind::NAMES {
            let kind = Kind::from_name(name).expect("advertised");
            assert!(kind.rounds() > 0, "{name} must be bounded");
        }
    }

    #[test]
    fn a_description_names_what_it_holds_but_never_the_task() {
        let spec = DelegateSpec::new(
            "delegate:1",
            Kind::Checker,
            "find out whether the tests pass",
            Kind::Checker.capabilities(),
            80,
        );

        let described = spec.describe();
        assert!(described.contains("checker"));
        assert!(described.contains("file_read"));
        assert!(described.contains("shell_exec"));
        assert!(described.contains("80"));
        assert!(!described.contains("whether the tests pass"));
    }
}
