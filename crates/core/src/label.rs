//! The label lattice `L = I × C`.
//!
//! ```text
//! I ∈ {T, U}          integrity:       T = trusted, U = untrusted
//! C ∈ {pub, priv}     confidentiality: pub ⊑ priv
//!
//! (U,pub) ⊑ (U,priv) ⊑ (T,priv)
//! (U,pub) ⊑ (T,pub)  ⊑ (T,priv)
//! ```
//!
//! `(U,priv)` and `(T,pub)` are **incomparable** — neither flows into the other. That
//! is what makes this a lattice rather than a pair of booleans, and it is why
//! [`Label`] implements [`PartialOrd`] by hand instead of deriving [`Ord`].
//!
//! Taint moves in opposite directions on the two axes: integrity **meets** (one
//! untrusted input taints the result) while confidentiality **joins** (one private
//! input makes the result private). See [`taint_all`].

use std::fmt;

/// Integrity axis. `U` is bottom: untrusted values cannot become trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Integrity {
    /// Trusted — derived only from trusted input.
    Trusted,
    /// Untrusted — derived from content we do not control.
    Untrusted,
}

impl Integrity {
    /// Lattice meet (⊓). `U` is bottom, so `meet(T, U) == U`.
    pub fn meet(self, other: Self) -> Self {
        if self == Self::Untrusted || other == Self::Untrusted {
            Self::Untrusted
        } else {
            Self::Trusted
        }
    }

    /// `self ⊑ other`.
    pub fn flows_to(self, other: Self) -> bool {
        self == Self::Untrusted || other == Self::Trusted
    }
}

/// Confidentiality axis. `priv` is top: private values cannot become public
/// without an explicit, audited declassification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Confidentiality {
    /// Public — safe to release.
    Public,
    /// Private — confidential; must be declassified before crossing a bridge.
    Private,
}

impl Confidentiality {
    /// Lattice join (⊔). `priv` is top, so `join(pub, priv) == priv`.
    pub fn join(self, other: Self) -> Self {
        if self == Self::Private || other == Self::Private {
            Self::Private
        } else {
            Self::Public
        }
    }

    /// `self ⊑ other`.
    pub fn flows_to(self, other: Self) -> bool {
        self == Self::Public || other == Self::Private
    }
}

/// A provenance label: one point in the lattice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Label {
    pub integrity: Integrity,
    pub confidentiality: Confidentiality,
}

impl Label {
    pub const fn new(integrity: Integrity, confidentiality: Confidentiality) -> Self {
        Self {
            integrity,
            confidentiality,
        }
    }

    /// `(T,pub)` — the only label routing fields accept.
    pub const fn trusted_public() -> Self {
        Self::new(Integrity::Trusted, Confidentiality::Public)
    }

    pub const fn trusted_private() -> Self {
        Self::new(Integrity::Trusted, Confidentiality::Private)
    }

    pub const fn untrusted_public() -> Self {
        Self::new(Integrity::Untrusted, Confidentiality::Public)
    }

    pub const fn untrusted_private() -> Self {
        Self::new(Integrity::Untrusted, Confidentiality::Private)
    }

    /// `self ⊑ other` — whether a value labelled `self` may flow into a slot or
    /// agent whose ceiling is `other`.
    pub fn flows_to(self, other: Self) -> bool {
        self.integrity.flows_to(other.integrity)
            && self.confidentiality.flows_to(other.confidentiality)
    }

    /// Whether `self` may be *degraded* into `other`.
    ///
    /// Distinct from [`Label::flows_to`], which is the lattice ordering `⊑` used for
    /// ceiling checks and in which `U ⊑ T`. Degradation is the direction taint
    /// travels: integrity may go trusted → untrusted, confidentiality may go public
    /// → private, never the reverse. Conflating the two would let a relabel launder
    /// untrusted content into trusted.
    pub fn degrades_to(self, other: Self) -> bool {
        other.integrity == self.integrity.meet(other.integrity)
            && other.confidentiality == self.confidentiality.join(other.confidentiality)
    }

    pub fn is_trusted(self) -> bool {
        self.integrity == Integrity::Trusted
    }

    pub fn is_public(self) -> bool {
        self.confidentiality == Confidentiality::Public
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let i = match self.integrity {
            Integrity::Trusted => "T",
            Integrity::Untrusted => "U",
        };
        let c = match self.confidentiality {
            Confidentiality::Public => "pub",
            Confidentiality::Private => "priv",
        };
        write!(f, "({i},{c})")
    }
}

/// Partial order on labels. Deliberately partial: `(U,priv)` and `(T,pub)` return
/// `None` because neither may flow into the other.
impl PartialOrd for Label {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        use std::cmp::Ordering;
        match (self.flows_to(*other), other.flows_to(*self)) {
            (true, true) => Some(Ordering::Equal),
            (true, false) => Some(Ordering::Less),
            (false, true) => Some(Ordering::Greater),
            (false, false) => None,
        }
    }
}

/// Output label for a computation, given its inputs.
///
/// Integrity meets, confidentiality joins. No inputs means no taint, so the result
/// is `(T,pub)` — the identity for both operations.
pub fn taint_all(labels: impl IntoIterator<Item = Label>) -> Label {
    labels.into_iter().fold(Label::trusted_public(), |acc, l| {
        Label::new(
            acc.integrity.meet(l.integrity),
            acc.confidentiality.join(l.confidentiality),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const T_PUB: Label = Label::trusted_public();
    const T_PRIV: Label = Label::trusted_private();
    const U_PUB: Label = Label::untrusted_public();
    const U_PRIV: Label = Label::untrusted_private();

    #[test]
    fn integrity_meet_is_pessimistic() {
        use Integrity::*;
        assert_eq!(Trusted.meet(Trusted), Trusted);
        assert_eq!(Trusted.meet(Untrusted), Untrusted);
        assert_eq!(Untrusted.meet(Trusted), Untrusted);
        assert_eq!(Untrusted.meet(Untrusted), Untrusted);
    }

    #[test]
    fn confidentiality_join_is_pessimistic() {
        use Confidentiality::*;
        assert_eq!(Public.join(Public), Public);
        assert_eq!(Public.join(Private), Private);
        assert_eq!(Private.join(Public), Private);
        assert_eq!(Private.join(Private), Private);
    }

    #[test]
    fn bottom_flows_everywhere_and_top_flows_nowhere() {
        for l in [T_PUB, T_PRIV, U_PUB, U_PRIV] {
            assert!(U_PUB.flows_to(l), "(U,pub) is bottom, must flow into {l}");
            assert!(l.flows_to(T_PRIV), "(T,priv) is top, {l} must flow into it");
        }
    }

    /// The property that makes this a lattice: the middle elements are incomparable.
    #[test]
    fn middle_elements_are_incomparable() {
        assert!(!U_PRIV.flows_to(T_PUB));
        assert!(!T_PUB.flows_to(U_PRIV));
        assert_eq!(U_PRIV.partial_cmp(&T_PUB), None);
        assert_eq!(T_PUB.partial_cmp(&U_PRIV), None);
    }

    #[test]
    fn ordering_is_reflexive_and_antisymmetric() {
        use std::cmp::Ordering;
        for l in [T_PUB, T_PRIV, U_PUB, U_PRIV] {
            assert!(l.flows_to(l));
            assert_eq!(l.partial_cmp(&l), Some(Ordering::Equal));
        }
        assert_eq!(U_PUB.partial_cmp(&T_PRIV), Some(Ordering::Less));
        assert_eq!(T_PRIV.partial_cmp(&U_PUB), Some(Ordering::Greater));
    }

    #[test]
    fn ordering_is_transitive() {
        let all = [T_PUB, T_PRIV, U_PUB, U_PRIV];
        for a in all {
            for b in all {
                for c in all {
                    if a.flows_to(b) && b.flows_to(c) {
                        assert!(a.flows_to(c), "{a} ⊑ {b} ⊑ {c} but not {a} ⊑ {c}");
                    }
                }
            }
        }
    }

    /// Degradation is not the lattice ordering. `(U,pub) ⊑ (T,pub)` holds, but a
    /// value may never be *upgraded* from untrusted to trusted.
    #[test]
    fn degradation_is_not_the_lattice_ordering() {
        assert!(U_PUB.flows_to(T_PUB));
        assert!(!U_PUB.degrades_to(T_PUB));
        assert!(T_PUB.degrades_to(U_PUB));
    }

    #[test]
    fn degradation_is_reflexive() {
        for l in [T_PUB, T_PRIV, U_PUB, U_PRIV] {
            assert!(l.degrades_to(l));
        }
    }

    #[test]
    fn top_of_taint_degrades_from_everything() {
        for l in [T_PUB, T_PRIV, U_PUB, U_PRIV] {
            assert!(l.degrades_to(U_PRIV), "{l} must degrade to (U,priv)");
        }
        assert!(!U_PRIV.degrades_to(T_PUB));
    }

    /// Degradation agrees with taint: combining inputs always yields a label each
    /// input can degrade into.
    #[test]
    fn taint_result_is_a_degradation_of_its_inputs() {
        let all = [T_PUB, T_PRIV, U_PUB, U_PRIV];
        for a in all {
            for b in all {
                let out = taint_all([a, b]);
                assert!(a.degrades_to(out), "{a} must degrade to taint {out}");
                assert!(b.degrades_to(out), "{b} must degrade to taint {out}");
            }
        }
    }

    #[test]
    fn no_inputs_means_no_taint() {
        assert_eq!(taint_all([]), T_PUB);
    }

    #[test]
    fn one_untrusted_input_taints_the_result() {
        assert_eq!(taint_all([T_PUB, T_PUB]), T_PUB);
        assert_eq!(taint_all([T_PUB, U_PUB]), U_PUB);
    }

    #[test]
    fn one_private_input_makes_the_result_private() {
        assert_eq!(taint_all([T_PUB, T_PRIV]), T_PRIV);
    }

    /// A trusted-private input and an untrusted-public input together produce
    /// untrusted-private: each axis degrades independently.
    #[test]
    fn axes_degrade_independently() {
        assert_eq!(taint_all([T_PRIV, U_PUB]), U_PRIV);
    }

    #[test]
    fn taint_is_order_independent() {
        assert_eq!(taint_all([T_PRIV, U_PUB]), taint_all([U_PUB, T_PRIV]));
    }

    /// Taint is not monotone in ⊑ — it descends the integrity axis while ascending
    /// the confidentiality axis — so the guarantee is stated per axis: the result is
    /// never more trusted, and never less private, than any input.
    #[test]
    fn taint_only_degrades_on_each_axis() {
        let all = [T_PUB, T_PRIV, U_PUB, U_PRIV];
        for a in all {
            for b in all {
                let out = taint_all([a, b]);
                for input in [a, b] {
                    assert!(
                        out.integrity.flows_to(input.integrity),
                        "taint of {a} and {b} is {out}, more trusted than input {input}"
                    );
                    assert!(
                        input.confidentiality.flows_to(out.confidentiality),
                        "taint of {a} and {b} is {out}, less private than input {input}"
                    );
                }
            }
        }
    }

    /// A trusted input cannot launder an untrusted one by being combined with it.
    #[test]
    fn trusted_input_cannot_launder_untrusted() {
        for trusted in [T_PUB, T_PRIV] {
            for untrusted in [U_PUB, U_PRIV] {
                assert!(!taint_all([trusted, untrusted]).is_trusted());
            }
        }
    }

    #[test]
    fn display_matches_the_canonical_notation() {
        assert_eq!(T_PUB.to_string(), "(T,pub)");
        assert_eq!(U_PRIV.to_string(), "(U,priv)");
    }
}
