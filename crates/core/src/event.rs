//! Typed audit events.
//!
//! The kernel never prints. Everything observable leaves through [`Event`], so the
//! audit trail is machine-readable and a stray `println!` cannot interleave with it
//! or leak content into a log.
//!
//! Events carry labels and decisions, never slot contents.

use crate::capability::Capability;
use crate::label::Label;
use crate::slot::SlotId;
use std::fmt;

/// Which principle a refusal upholds. Useful for explaining a block to a user
/// without re-deriving why it happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Principle {
    /// Untrusted data attempted to influence where an action goes.
    IntegrityGate,
    /// Private data attempted to leave without declassification.
    Confinement,
    /// An operation was attempted without the capability it requires.
    Capability,
    /// A confinement boundary could not be established, so the operation was refused
    /// rather than run unconfined.
    ConfinementUnavailable,
    /// An effect's target could not be located uniquely within the content it was to be
    /// applied to, so it was refused rather than applied to a guess.
    AmbiguousEffect,
}

/// The role a field plays in an action.
///
/// The asymmetry between these two is the anti-injection mechanism: routing decides
/// *where* an effect lands and must be trusted, while content is merely carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Decides where an action goes: a path, a URL, a command name. Must be `(T,pub)`.
    Routing,
    /// The payload. May be untrusted; must not be private at release time.
    Content,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Routing => f.write_str("routing"),
            Self::Content => f.write_str("content"),
        }
    }
}

/// One thing that happened, or was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A gate allowed an operation.
    GatePassed { gate: &'static str, detail: String },
    /// A gate refused an operation.
    GateBlocked {
        gate: &'static str,
        detail: String,
        reason: String,
    },
    /// A slot was written.
    SlotWritten { slot: SlotId, label: Label },
    /// A capability produced data at a label.
    Observed {
        capability: Capability,
        label: Label,
    },
    /// Untrusted content was authorised for release.
    Declassified {
        slot: SlotId,
        from: Label,
        to: Label,
        reason: &'static str,
    },
    /// A field was checked immediately before an effect fired.
    ActionField {
        tool: String,
        field: String,
        role: Role,
        label: Label,
        allowed: bool,
    },
}

/// Somewhere for events to go. Implemented outside the kernel: a terminal renderer, a
/// JSONL file, or both.
pub trait Sink {
    fn emit(&mut self, event: Event);
}

/// Discards everything. For tests that do not assert on the trail.
#[derive(Debug, Default)]
pub struct NullSink;

impl Sink for NullSink {
    fn emit(&mut self, _event: Event) {}
}

/// Retains events in order. For tests, and for replaying a run's trail.
#[derive(Debug, Default)]
pub struct RecordingSink {
    events: Vec<Event>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Every refusal, in order.
    pub fn blocked(&self) -> impl Iterator<Item = &Event> {
        self.events
            .iter()
            .filter(|e| matches!(e, Event::GateBlocked { .. }))
    }

    /// Whether the run completed without a single refusal.
    pub fn clean(&self) -> bool {
        self.blocked().next().is_none()
            && !self
                .events
                .iter()
                .any(|e| matches!(e, Event::ActionField { allowed: false, .. }))
    }
}

impl Sink for RecordingSink {
    fn emit(&mut self, event: Event) {
        self.events.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_sink_keeps_order() {
        let mut sink = RecordingSink::new();
        sink.emit(Event::GatePassed {
            gate: "first",
            detail: String::new(),
        });
        sink.emit(Event::GatePassed {
            gate: "second",
            detail: String::new(),
        });
        assert_eq!(sink.events().len(), 2);
        assert!(matches!(
            sink.events()[0],
            Event::GatePassed { gate: "first", .. }
        ));
    }

    #[test]
    fn a_run_with_no_refusals_is_clean() {
        let mut sink = RecordingSink::new();
        sink.emit(Event::SlotWritten {
            slot: SlotId::new("s"),
            label: Label::untrusted_public(),
        });
        assert!(sink.clean());
    }

    #[test]
    fn a_blocked_gate_makes_a_run_unclean() {
        let mut sink = RecordingSink::new();
        sink.emit(Event::GateBlocked {
            gate: "action",
            detail: "field=path".into(),
            reason: "untrusted routing".into(),
        });
        assert!(!sink.clean());
        assert_eq!(sink.blocked().count(), 1);
    }

    /// A refused action field counts as unclean even without a GateBlocked event, so a
    /// caller cannot report success by only emitting the field-level record.
    #[test]
    fn a_refused_action_field_makes_a_run_unclean() {
        let mut sink = RecordingSink::new();
        sink.emit(Event::ActionField {
            tool: "write_file".into(),
            field: "path".into(),
            role: Role::Routing,
            label: Label::untrusted_public(),
            allowed: false,
        });
        assert!(!sink.clean());
    }

    #[test]
    fn null_sink_discards() {
        let mut sink = NullSink;
        sink.emit(Event::GatePassed {
            gate: "x",
            detail: String::new(),
        });
    }
}
