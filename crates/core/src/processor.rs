//! Isolated processors: the one thing that reads quarantined content.
//!
//! The planner cannot read an untrusted file, which is the whole point, and that leaves a gap:
//! an agent that may not look at a file also cannot change it. `edit_file` refuses, because
//! locating a passage is a comparison, and `write_file` needs a body the planner would have to
//! have written blind.
//!
//! A processor closes that gap without weakening anything. It is a second model instance that
//! holds no capabilities at all: no tools, no conversation, no memory of the session, no
//! workspace, no way to spawn anything. It reads the slots the driver hands it and produces
//! text. That text is not returned to the planner either; it goes straight into a new slot at
//! the label its inputs taint it to, and the planner gets a reference.
//!
//! So injected text in a processor's input can do exactly one thing: change the bytes in a slot
//! nobody has read. It cannot redirect an effect, because it never reaches a routing field. It
//! cannot widen its own access, because a [`ProcessorSpec`] is built by the driver, frozen
//! before the run, and holds no way to add a slot. It cannot persist, because the processor is
//! gone when the call returns.
//!
//! What a processor is **not** is a sandbox in the operating-system sense. There is no untrusted
//! code here to confine: the code making the call is the driver's own, and `bua-sandbox` exists
//! for processes that run someone else's. The confinement is the capability set, which is empty,
//! and the label on the output, which no part of the processor chooses.

use crate::label::Label;
use crate::slot::SlotId;

/// What the driver fixed about one processor before it ran.
///
/// Only [`crate::policy::Policy::before_processor`] constructs one, and nothing here can widen
/// it afterwards: the input slots, the instruction, and the label the output will carry are all
/// decided before the processor exists. The processor itself never sees this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorSpec {
    id: String,
    reads: Vec<SlotId>,
    instruction: String,
    out_label: Label,
    unchanged: Option<SlotId>,
}

impl ProcessorSpec {
    pub(crate) fn new(
        id: impl Into<String>,
        reads: Vec<SlotId>,
        instruction: impl Into<String>,
        out_label: Label,
        unchanged: Option<SlotId>,
    ) -> Self {
        Self {
            id: id.into(),
            reads,
            instruction: instruction.into(),
            out_label,
            unchanged,
        }
    }

    /// Which input the answer falls back to when the processor says nothing should change.
    ///
    /// Chosen by the planner out of the slots it named, before the processor exists. A processor
    /// asked to leave a document alone otherwise has to reproduce it byte for byte, and one that
    /// explains itself instead destroys the file: the words become the file, and nobody
    /// downstream is allowed to read them and notice.
    pub fn unchanged(&self) -> Option<&SlotId> {
        self.unchanged.as_ref()
    }

    /// What a processor says when the document should be left as it is.
    ///
    /// Safe by construction: a document whose entire content is this word is replaced by itself.
    pub const UNCHANGED: &'static str = "UNCHANGED";

    /// The processor's name in the audit trail. Driver-chosen, never derived from content.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The slots it may read, and the only ones it will be given.
    pub fn reads(&self) -> &[SlotId] {
        &self.reads
    }

    /// What it was asked to do.
    ///
    /// The planner's own words, checked public before the spec was built. Readable because it
    /// is not workspace content: it is the instruction the driver is about to send, and a
    /// driver that could not hold it could not send it.
    pub fn instruction(&self) -> &str {
        &self.instruction
    }

    /// The label the output will carry, computed by taint over the inputs.
    ///
    /// Fixed here rather than after the run, so nothing the processor produces has any say in
    /// how its output is labelled.
    pub fn out_label(&self) -> Label {
        self.out_label
    }

    /// The processor as the audit trail describes it: what it reads and what that makes its
    /// output. Never the content, and never the instruction, which can be long.
    pub fn describe(&self) -> String {
        let reads: Vec<&str> = self.reads.iter().map(SlotId::as_str).collect();
        format!(
            "{} reads {} and writes {}",
            self.id,
            reads.join(", "),
            self.out_label
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_description_names_the_slots_and_the_label_but_no_content() {
        let spec = ProcessorSpec::new(
            "processor:1",
            vec![SlotId::new("ref:0"), SlotId::new("ref:1")],
            "rewrite the function and output the whole file",
            Label::untrusted_private(),
            None,
        );

        let described = spec.describe();
        assert!(described.contains("ref:0"));
        assert!(described.contains("ref:1"));
        assert!(described.contains("(U,priv)"));
        assert!(!described.contains("rewrite the function"));
    }
}
