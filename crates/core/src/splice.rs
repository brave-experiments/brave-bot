//! Locating a passage inside untrusted content, without letting the driver see it.
//!
//! An edit names a passage to replace. Deciding whether that passage is present, and whether
//! it is present exactly once, is a decision derived from untrusted bytes — so by the rule in
//! CLAUDE.md it cannot happen in the driver. If it did, untrusted file content would be
//! choosing whether an effect occurs.
//!
//! So the search happens here, in the kernel, and the driver receives a [`Splice`]: a verdict
//! plus, on success, the resulting text still wrapped in its label. The driver can hand that
//! to a write and show it to a person. It never learns *where* the match was, how many there
//! were, or what the file contains.
//!
//! The verdict itself is metadata about the content, not the content — the same category as a
//! label or a byte count, which the audit trail has always carried. What matters is that the
//! bytes never reach a branch in the driver.

use crate::label::Label;
use crate::value::{Declassification, Labelled};
use std::fmt;

/// Why a passage could not be replaced.
///
/// Deliberately does not carry the passage, the file, or any surrounding text: an error is a
/// thing the driver formats and returns, so anything in here would escape the labelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpliceRefusal {
    /// The passage is not present.
    NotFound,
    /// The passage occurs more than once, so which was meant is unknowable.
    ///
    /// The count is a property of the content rather than the content itself, and telling the
    /// model how many matches exist is what lets it supply a longer, unique passage.
    Ambiguous { occurrences: usize },
    /// The passage and its replacement are identical, so nothing would change.
    Unchanged,
    /// The passage was empty, which identifies no location.
    Empty,
}

impl fmt::Display for SpliceRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(
                f,
                "the text to replace is not present; it must match exactly, including \
                 whitespace and indentation. Read the file again to see its current contents"
            ),
            Self::Ambiguous { occurrences } => write!(
                f,
                "the text to replace occurs {occurrences} times, so it is unclear which was \
                 meant. Include more surrounding context to make it unique, or ask to replace \
                 every occurrence"
            ),
            Self::Unchanged => write!(
                f,
                "the old and new text are identical, so this edit would change nothing"
            ),
            Self::Empty => write!(
                f,
                "the text to replace must not be empty; write the whole file instead"
            ),
        }
    }
}

/// A successful splice: the resulting text, still labelled, and how many places changed.
#[derive(Debug)]
pub struct Splice {
    /// The full resulting text. Carries the label of the content it was derived from, so it
    /// cannot be inspected without a witness.
    pub contents: Labelled<String>,
    /// How many occurrences were replaced.
    pub occurrences: usize,
}

/// Replace `old` with `new` inside `source`, deciding presence and uniqueness here.
///
/// `old` and `new` come from the model and are untrusted; `source` is file content and is
/// untrusted. None of them is returned to the caller in readable form. The result carries the
/// meet of every input's label, because the output is derived from all three — a replacement
/// built from untrusted text is untrusted however trusted the file was.
///
/// Takes a [`Declassification`] because it reads the bytes to do the search. That witness is
/// the audited record that the read happened; the point of routing it through the kernel is
/// that the *decision* stays here rather than in the driver.
pub fn splice(
    source: &Labelled<String>,
    old: &Labelled<String>,
    new: &Labelled<String>,
    all: bool,
    proof: &Declassification,
) -> Result<Splice, SpliceRefusal> {
    let label = crate::label::taint_all([source.label(), old.label(), new.label()]);

    let source_text = source.clone().declassify(proof);
    let old_text = old.clone().declassify(proof);
    let new_text = new.clone().declassify(proof);

    if old_text.is_empty() {
        return Err(SpliceRefusal::Empty);
    }
    if old_text == new_text {
        return Err(SpliceRefusal::Unchanged);
    }

    let occurrences = source_text.matches(&old_text).count();
    let replaced = match occurrences {
        0 => return Err(SpliceRefusal::NotFound),
        1 => source_text.replacen(&old_text, &new_text, 1),
        _ if all => source_text.replace(&old_text, &new_text),
        many => return Err(SpliceRefusal::Ambiguous { occurrences: many }),
    };

    Ok(Splice {
        contents: Labelled::new(replaced, label),
        occurrences: if all { occurrences } else { 1 },
    })
}

/// Whether `candidate` is byte-identical to `expected`, without revealing either.
///
/// Used for the staleness check before an endorsed edit: the driver needs to know whether the
/// file changed since it was read, which is a comparison of two untrusted values. Returning a
/// bool rather than letting the driver compare keeps that decision in the kernel.
pub fn contents_match(
    candidate: &Labelled<String>,
    expected: &Labelled<String>,
    proof: &Declassification,
) -> bool {
    candidate.clone().declassify(proof) == expected.clone().declassify(proof)
}

/// The label a splice result would carry, without performing one.
pub fn result_label(source: Label, old: Label, new: Label) -> Label {
    crate::label::taint_all([source, old, new])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::label::{Integrity, Label};

    /// Only the policy layer mints these, but tests live inside the crate so they may.
    fn proof() -> Declassification {
        Declassification::authorise("test")
    }

    fn untrusted(text: &str) -> Labelled<String> {
        Labelled::new(text.to_string(), Label::untrusted_private())
    }

    fn read(value: Labelled<String>) -> String {
        value.declassify(&proof())
    }

    #[test]
    fn a_unique_passage_is_replaced() {
        let result = splice(
            &untrusted("a\nb\nc\n"),
            &untrusted("b"),
            &untrusted("B"),
            false,
            &proof(),
        )
        .expect("unique");
        assert_eq!(result.occurrences, 1);
        assert_eq!(read(result.contents), "a\nB\nc\n");
    }

    /// The decision this module exists for: an ambiguous passage is refused, and the refusal
    /// is the kernel's rather than the driver's.
    #[test]
    fn an_ambiguous_passage_is_refused() {
        let error = splice(
            &untrusted("x\nx\n"),
            &untrusted("x"),
            &untrusted("y"),
            false,
            &proof(),
        )
        .expect_err("ambiguous");
        assert_eq!(error, SpliceRefusal::Ambiguous { occurrences: 2 });
    }

    #[test]
    fn every_occurrence_can_be_replaced_when_asked() {
        let result = splice(
            &untrusted("x\nx\nx\n"),
            &untrusted("x"),
            &untrusted("y"),
            true,
            &proof(),
        )
        .expect("all");
        assert_eq!(result.occurrences, 3);
        assert_eq!(read(result.contents), "y\ny\ny\n");
    }

    #[test]
    fn an_absent_passage_is_refused() {
        let error = splice(
            &untrusted("a\n"),
            &untrusted("zzz"),
            &untrusted("y"),
            false,
            &proof(),
        )
        .expect_err("absent");
        assert_eq!(error, SpliceRefusal::NotFound);
    }

    #[test]
    fn an_empty_passage_is_refused() {
        assert_eq!(
            splice(
                &untrusted("a\n"),
                &untrusted(""),
                &untrusted("y"),
                false,
                &proof()
            )
            .expect_err("empty"),
            SpliceRefusal::Empty
        );
    }

    #[test]
    fn an_identical_replacement_is_refused() {
        assert_eq!(
            splice(
                &untrusted("a\n"),
                &untrusted("a"),
                &untrusted("a"),
                false,
                &proof()
            )
            .expect_err("identical"),
            SpliceRefusal::Unchanged
        );
    }

    /// The result is derived from every input, so one untrusted input taints it. Otherwise an
    /// edit would be a way to launder model-supplied text into a trusted value.
    #[test]
    fn the_result_is_tainted_by_every_input() {
        let trusted_file = Labelled::new("a\nb\n".to_string(), Label::trusted_public());
        let untrusted_new = Labelled::new("B".to_string(), Label::untrusted_public());

        let result = splice(
            &trusted_file,
            &Labelled::new("b".to_string(), Label::trusted_public()),
            &untrusted_new,
            false,
            &proof(),
        )
        .expect("splices");

        assert_eq!(
            result.contents.label().integrity,
            Integrity::Untrusted,
            "untrusted replacement text produced a trusted result"
        );
    }

    /// A wholly trusted edit stays trusted, or a vouched-for workspace could never be
    /// edited without prompting.
    #[test]
    fn a_wholly_trusted_splice_stays_trusted() {
        let t = |s: &str| Labelled::new(s.to_string(), Label::trusted_public());
        let result = splice(&t("a\nb\n"), &t("b"), &t("B"), false, &proof()).expect("splices");
        assert_eq!(result.contents.label(), Label::trusted_public());
    }

    /// Whitespace is part of the passage, so an edit cannot land on a differently indented
    /// line than the one it named.
    #[test]
    fn matching_is_whitespace_exact() {
        let source = untrusted("if (x) {\n    y();\n}\n");
        assert_eq!(
            splice(
                &source,
                &untrusted("if (x) {\n  y();\n}"),
                &untrusted("z"),
                false,
                &proof()
            )
            .expect_err("indentation differs"),
            SpliceRefusal::NotFound
        );
        let result = splice(
            &source,
            &untrusted("if (x) {\n    y();\n}"),
            &untrusted("z"),
            false,
            &proof(),
        )
        .expect("exact");
        assert_eq!(read(result.contents), "z\n");
    }

    #[test]
    fn a_refusal_carries_no_content() {
        // The refusal is formatted by the driver, so it must not embed the file or passage.
        let error = splice(
            &untrusted("secret data here"),
            &untrusted("absent-passage"),
            &untrusted("x"),
            false,
            &proof(),
        )
        .expect_err("absent");
        let shown = error.to_string();
        assert!(!shown.contains("secret data"), "content leaked: {shown}");
        assert!(
            !shown.contains("absent-passage"),
            "the passage leaked: {shown}"
        );
    }

    #[test]
    fn identical_contents_compare_equal() {
        assert!(contents_match(
            &untrusted("same"),
            &untrusted("same"),
            &proof()
        ));
        assert!(!contents_match(
            &untrusted("one"),
            &untrusted("two"),
            &proof()
        ));
    }

    #[test]
    fn the_result_label_is_predictable_without_splicing() {
        let label = result_label(
            Label::trusted_public(),
            Label::trusted_public(),
            Label::untrusted_public(),
        );
        assert_eq!(label.integrity, Integrity::Untrusted);
    }
}
