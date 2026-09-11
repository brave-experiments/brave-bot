//! Labelled values.
//!
//! A [`Labelled`] pairs a value with its provenance label. The label lives *outside*
//! the value: the model never sees it and cannot address it, so injected text cannot
//! forge one.
//!
//! The important property is that untrusted content is **carryable but not
//! inspectable**. `Labelled` deliberately does not implement `Deref`, `PartialEq`,
//! or `Display`, and exposes no infallible getter. Code can move it, store it, and
//! hand it to a gate, but cannot branch on its contents, so untrusted data cannot
//! reach a decision. The only two ways to the inner value are
//! [`Labelled::declassify`], which demands a [`Declassification`] witness that only
//! the policy layer can mint, and [`Labelled::into_trusted`], which fails on anything
//! that is not already `(T,pub)` and so has nothing to declassify.
//!
//! This is the compile-time form of the rule that the driver carries content but
//! never inspects it.

use crate::label::Label;
use crate::policy::Declassification;
use std::fmt;

/// A value carrying a provenance label.
///
/// Intentionally missing: `Deref`, `PartialEq`, `Display`, and any infallible
/// accessor. Adding one would let untrusted content influence control flow, which
/// is the exact failure this type exists to prevent.
#[derive(Clone)]
pub struct Labelled<T> {
    value: T,
    label: Label,
}

impl<T> Labelled<T> {
    pub fn new(value: T, label: Label) -> Self {
        Self { value, label }
    }

    /// A value derived only from trusted input, safe for routing.
    pub fn trusted(value: T) -> Self {
        Self::new(value, Label::trusted_public())
    }

    pub fn label(&self) -> Label {
        self.label
    }

    /// Read the inner value. Requires a policy-minted witness.
    pub fn declassify(self, _proof: &Declassification) -> T {
        self.value
    }

    /// Read the inner value without a witness, permitted only when the label is
    /// already `(T,pub)`, so there is nothing to declassify. Returns the original
    /// value back on mismatch so a caller cannot smuggle content through by
    /// discarding the error.
    pub fn into_trusted(self) -> Result<T, Self> {
        if self.label == Label::trusted_public() {
            Ok(self.value)
        } else {
            Err(self)
        }
    }

    /// Re-label a value, which may only degrade it. Returns `None` if the requested
    /// label is not reachable by degradation from the current one.
    pub fn relabel(self, to: Label) -> Option<Self> {
        if self.label.degrades_to(to) {
            Some(Self {
                value: self.value,
                label: to,
            })
        } else {
            None
        }
    }
}

/// How much content there is, which is the one thing anyone outside may learn about
/// content they may not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shape {
    pub lines: usize,
    pub bytes: usize,
}

impl Labelled<String> {
    /// Count the lines and bytes without releasing the content.
    ///
    /// Not a read, and so not a hole in the rule above: what comes back is derived from how
    /// many bytes there are and never from what they say, and those two numbers are exactly
    /// what LABEL-3 already puts in front of the planner for content it is not shown. The
    /// value itself never leaves, so there is nothing to declassify and no witness to mint.
    ///
    /// `pub(crate)` all the same, so that the one thing outside this crate can learn about
    /// quarantined content stays the [`crate::slot::Measured`] a slot hands back. A driver that
    /// could ask any labelled value how long it is could branch on the answer, and repeating
    /// the question is a side channel on bytes the quarantine exists to withhold.
    pub(crate) fn shape(&self) -> Shape {
        Shape {
            lines: self.value.lines().count(),
            bytes: self.value.len(),
        }
    }
}

/// Shows the label but never the value, so a stray log line cannot leak private
/// content. Private values are redacted even in `Debug`.
impl<T> fmt::Debug for Labelled<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Labelled(<{}>, {})", type_name_of::<T>(), self.label)
    }
}

fn type_name_of<T>() -> &'static str {
    std::any::type_name::<T>()
        .rsplit("::")
        .next()
        .unwrap_or("?")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_public_values_need_no_witness() {
        let v = Labelled::trusted("main.rs".to_string());
        assert_eq!(v.into_trusted().unwrap(), "main.rs");
    }

    #[test]
    fn untrusted_values_cannot_be_read_without_a_witness() {
        let v = Labelled::new("injected".to_string(), Label::untrusted_public());
        let returned = v.into_trusted().expect_err("untrusted must not unwrap");
        assert_eq!(returned.label(), Label::untrusted_public());
    }

    /// A private-but-trusted value is not routing-safe either: `into_trusted`
    /// requires exactly `(T,pub)`, not merely trusted integrity.
    #[test]
    fn trusted_private_values_are_not_routing_safe() {
        let v = Labelled::new("secret".to_string(), Label::trusted_private());
        assert!(v.into_trusted().is_err());
    }

    #[test]
    fn relabel_may_degrade() {
        let v = Labelled::trusted(1u8);
        let degraded = v.relabel(Label::untrusted_private()).expect("may degrade");
        assert_eq!(degraded.label(), Label::untrusted_private());
    }

    #[test]
    fn relabel_may_not_upgrade() {
        let v = Labelled::new(1u8, Label::untrusted_public());
        assert!(v.relabel(Label::trusted_public()).is_none());
    }

    /// Incomparable labels are not reachable from one another.
    #[test]
    fn relabel_refuses_incomparable_labels() {
        let v = Labelled::new(1u8, Label::untrusted_private());
        assert!(v.relabel(Label::trusted_public()).is_none());
    }

    /// How much content there is, is not what the content says. The quarantine hands the
    /// planner a line count and a byte count for content it will never be shown, so counting
    /// them cannot itself require a witness; what must never come back is the text.
    #[test]
    fn content_can_be_measured_without_being_read() {
        let v = Labelled::new("one\ntwo\nthree".to_string(), Label::untrusted_private());
        let shape = v.shape();

        assert_eq!(shape.lines, 3);
        assert_eq!(shape.bytes, 13);
        assert!(v.into_trusted().is_err(), "measuring released the value");
    }

    /// Debug output must never contain the value, or private content leaks into logs.
    #[test]
    fn debug_redacts_the_value() {
        let v = Labelled::new("sensitive-token".to_string(), Label::untrusted_private());
        let shown = format!("{v:?}");
        assert!(!shown.contains("sensitive-token"), "leaked: {shown}");
        assert!(shown.contains("(U,priv)"));
    }
}
