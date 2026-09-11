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
//! code here to confine: the code making the call is the driver's own, and `bravebot-sandbox` exists
//! for processes that run someone else's. The confinement is the capability set, which is empty,
//! and the label on the output, which no part of the processor chooses.

use crate::label::Label;
use crate::slot::SlotId;

/// One piece of a processor's input.
///
/// A picture cannot be concatenated into a body, so an input is a sequence rather than a string:
/// the runs of text and the pictures between them, in the order the slots were named. Assembled by
/// [`crate::policy::Policy::compose_processor_input`], which is the only thing that builds one, and
/// carried wrapped so the driver hands it to a request without reading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// A run of text: the documents, fenced, as they were before pictures existed.
    Text(String),
    /// A picture, as a `data:` URI and the media type to send it under.
    ///
    /// The media type is the driver's, from a closed table of extensions. Nothing read chooses it,
    /// so this cannot be reached by a file containing something that looks like a picture.
    Picture { media: String, data: String },
}

impl Piece {
    /// The text of this piece, or `None` for a picture.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Picture { .. } => None,
        }
    }

    /// Whether this piece is a picture.
    pub fn is_a_picture(&self) -> bool {
        matches!(self, Self::Picture { .. })
    }
}

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
    about: Option<SlotId>,
}

impl ProcessorSpec {
    pub(crate) fn new(
        id: impl Into<String>,
        reads: Vec<SlotId>,
        instruction: impl Into<String>,
        out_label: Label,
        about: Option<SlotId>,
    ) -> Self {
        Self {
            id: id.into(),
            reads,
            instruction: instruction.into(),
            out_label,
            about,
        }
    }

    /// Which document this call is about.
    ///
    /// Chosen by the planner out of the slots it named, before the processor exists, and it
    /// decides two things. The answer replaces that document and may be written to no other
    /// file: a processor produces one document however many it was given, and a planner that
    /// assumed otherwise wrote a game's HTML into a Python script. And where the answer marks no
    /// document, this one stands as it was and there is nothing to write for it, so a processor
    /// with nothing to change leaves the line out rather than reproducing a file it was told to
    /// leave alone.
    pub fn about(&self) -> Option<&SlotId> {
        self.about.as_ref()
    }

    /// The line that separates what a processor wants to say from what it produced.
    ///
    /// A processor has one output and has always wanted two: the document, and a word about what
    /// it did with it. With nowhere to put the second it put it in the first, and the sentences
    /// became the file. Twice.
    ///
    /// Everything before the line is a note for the person watching. Everything after it is the
    /// document, whatever it says. An answer without the line at all names no document, so
    /// nothing is written and the document the call was about stands as it was: that is also how
    /// a processor says it found nothing to change, since a driver that read a word out of the
    /// answer instead would be deciding from untrusted bytes.
    ///
    /// A document that contains this line splits at it, and the part before goes to a screen
    /// instead of into the file. That is a reshaping of untrusted content by untrusted content:
    /// what it can reach is which bytes land in a slot nobody reads, and a person sees both
    /// halves either way.
    pub const NOTE_MARKER: &'static str = "===== the document starts here =====";

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
