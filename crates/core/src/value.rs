//! Labelled values.
//!
//! A [`Labelled`] pairs a value with its provenance label. The label lives *outside*
//! the value: the model never sees it and cannot address it, so injected text cannot
//! forge one.
//!
//! The important property is that untrusted content is **carryable but not
//! inspectable**. `Labelled` deliberately does not implement `Deref`, `PartialEq`,
//! or `Display`, and exposes no infallible getter. Code can move it, store it, and
//! hand it to a gate, but cannot branch on its contents — so untrusted data cannot
//! reach a decision. Reading the inner value requires [`Labelled::declassify`],
//! which demands a [`Declassification`] witness that only the policy layer can mint.
//!
//! This is the compile-time form of the rule that the driver carries content but
//! never inspects it.

use crate::label::Label;
use std::fmt;

/// Proof that a read of untrusted content has been authorised and recorded.
///
/// Only the policy layer constructs these, via [`Declassification::authorise`],
/// which is `pub(crate)`. Downstream crates cannot fabricate one, so
/// [`Labelled::declassify`] cannot be called without going through a gate.
#[derive(Debug)]
pub struct Declassification {
    reason: &'static str,
}

impl Declassification {
    pub(crate) fn authorise(reason: &'static str) -> Self {
        Self { reason }
    }

    /// Why this read was permitted — recorded in the audit trail.
    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

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
    /// already `(T,pub)` — there is nothing to declassify. Returns the original
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
    fn a_witness_permits_reading() {
        let v = Labelled::new("page body".to_string(), Label::untrusted_public());
        let proof = Declassification::authorise("test");
        assert_eq!(v.declassify(&proof), "page body");
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

    /// Debug output must never contain the value, or private content leaks into logs.
    #[test]
    fn debug_redacts_the_value() {
        let v = Labelled::new("sensitive-token".to_string(), Label::untrusted_private());
        let shown = format!("{v:?}");
        assert!(!shown.contains("sensitive-token"), "leaked: {shown}");
        assert!(shown.contains("(U,priv)"));
    }
}
