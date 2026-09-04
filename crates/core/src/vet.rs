//! Vetting: asking whether quarantined content is what it was supposed to be.
//!
//! A vetting call is a processor by another name. It reads one quarantined slot, holds no
//! capabilities at all, and produces one thing. What differs is the shape of that one thing: a
//! processor writes a document into a slot nobody reads, and a vetting call answers a question
//! the planner asked, with one of two words the driver wrote.
//!
//! That word is the whole of the difference and the whole of the cost. Everywhere else in this
//! system, what untrusted bytes reach is a slot; here they reach a bit in the planner's context,
//! because a verdict nobody is told is not a verdict. The bit is one of two literals fixed in
//! this file, the reason behind it never leaves the screen, and an answer that is neither literal
//! is a refusal. What an attacker who owns the content can buy with it is written down in
//! [`docs/specs/vetting.md`](../../../docs/specs/vetting.md) rather than left to be discovered.
//!
//! A verdict decides nothing on its own. It changes no label, opens no slot and moves no file: it
//! is reported, and what to do about it is the planner's problem.

use crate::label::Label;
use crate::slot::SlotId;

/// What one vetting call answered.
///
/// Two states and no third. A checker that answers with something else is [`Verdict::Unsafe`],
/// because the direction to fail in is the one that tells the planner less than it hoped for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The content reads as the kind of thing it was said to be, and asks the reader for nothing.
    Safe,
    /// The content carries instructions, or is not what it was expected to be, or the checker did
    /// not answer in a way that could be read.
    Unsafe,
}

impl Verdict {
    /// The word a checker writes to say the content asked nothing of it.
    ///
    /// A literal in the driver's own source, so what the planner is eventually told is a word
    /// this file chose rather than a word the content wrote.
    pub const SAFE: &'static str = "VERDICT: SAFE";

    /// The word a checker writes to say the content tried to direct it.
    pub const UNSAFE: &'static str = "VERDICT: UNSAFE";

    /// How the audit trail and the planner refer to it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Unsafe => "unsafe",
        }
    }
}

/// What the driver fixed about one vetting call before it ran.
///
/// Built in one place, frozen before the content is read, and holding no way to widen itself
/// afterwards. The checker never sees this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VetSpec {
    id: String,
    reads: SlotId,
    expected: Option<String>,
    input_label: Label,
}

impl VetSpec {
    pub(crate) fn new(
        id: impl Into<String>,
        reads: SlotId,
        expected: Option<String>,
        input_label: Label,
    ) -> Self {
        Self {
            id: id.into(),
            reads,
            expected,
            input_label,
        }
    }

    /// The call's name in the audit trail. Driver-chosen, never derived from content.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The one slot it may read, and the only thing it will be given.
    ///
    /// One rather than several because a verdict is about a thing, and a call given two documents
    /// answering with one word has said something about neither of them.
    pub fn reads(&self) -> &SlotId {
        &self.reads
    }

    /// What the planner said the content was supposed to be, where it knew.
    ///
    /// The planner's own words, checked public before the spec was built. Content that is a
    /// plausible answer to a question nobody asked is the case this exists for: a checker told
    /// what it is looking at can say that a shell script arriving where a changelog was expected
    /// is wrong, and one told nothing can only say that it is a shell script.
    pub fn expected(&self) -> Option<&str> {
        self.expected.as_deref()
    }

    /// The label of what it reads, and of the answer it gives back.
    ///
    /// Fixed here rather than after the run, so nothing the checker produces has any say in how
    /// what it produces is labelled.
    pub fn input_label(&self) -> Label {
        self.input_label
    }

    /// The call as the audit trail describes it: what it reads, and whether it was told what to
    /// expect. Never the content, and never the expectation, which is the planner's prose.
    pub fn describe(&self) -> String {
        let told = match self.expected {
            Some(_) => "told what to expect",
            None => "told nothing about what to expect",
        };
        format!(
            "{} vets {} ({}), {told}",
            self.id, self.reads, self.input_label
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The trail says which slot was vetted and at what label. Somebody reading it back is
    /// entitled to know what was looked at without the record having quoted it at them.
    #[test]
    fn a_description_names_the_slot_and_the_label_but_no_content() {
        let spec = VetSpec::new(
            "vet:1",
            SlotId::new("ref:0"),
            Some("the release notes for version 2".to_string()),
            Label::untrusted_private(),
        );

        let described = spec.describe();
        assert!(described.contains("ref:0"), "{described}");
        assert!(described.contains("(U,priv)"), "{described}");
        assert!(described.contains("told what to expect"), "{described}");
        assert!(!described.contains("release notes"), "{described}");
    }

    /// A call the planner could not describe still says so, rather than reading as one that was
    /// given an expectation and had it dropped.
    #[test]
    fn a_description_says_when_it_was_told_nothing_to_expect() {
        let spec = VetSpec::new(
            "vet:1",
            SlotId::new("ref:0"),
            None,
            Label::untrusted_public(),
        );

        assert!(
            spec.describe()
                .contains("told nothing about what to expect"),
            "{}",
            spec.describe()
        );
    }

    /// The two words are distinct and neither contains the other, so finding one in an answer is
    /// not also finding the other.
    #[test]
    fn the_two_verdict_words_cannot_be_mistaken_for_each_other() {
        assert!(!Verdict::SAFE.contains(Verdict::UNSAFE));
        assert!(!Verdict::UNSAFE.contains(Verdict::SAFE));
    }
}
