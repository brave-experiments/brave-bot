//! Information-flow kernel.
//!
//! This crate performs no I/O. It defines the label lattice, quarantined storage,
//! capabilities, and the policy gates every consequential action passes through.
//! Nothing here prints: diagnostics leave through typed events so the audit trail
//! stays machine-readable.
//!
//! # Credit
//!
//! The enforcement model implemented here — the `L = I × C` lattice, write-once
//! quarantine, and the routing/content asymmetry that makes injected text unable to
//! redirect an action — was developed jointly by Ali Shahin Shamsabadi, Senior Privacy
//! Researcher at Brave, and Brian R. Bondy, and first prototyped by Ali in the SafeHouse
//! research project. This applies that model to a coding agent.

pub mod capability;
pub mod event;
pub mod label;
pub mod policy;
pub mod reference;
pub mod slot;
pub mod trust;
pub mod value;

pub use capability::{Capability, CapabilitySet, CapabilityToken};
pub use event::{Event, NullSink, Principle, RecordingSink, Role, Sink};
pub use label::{Confidentiality, Integrity, Label, taint_all};
pub use policy::{Denial, Policy, ReleasePlan, Routing};
pub use reference::{Presentation, Reference};
pub use slot::{Measured, SlotError, SlotId, SlotReader, SlotStore, SlotWriter};
pub use trust::TrustStore;
pub use value::{Declassification, Labelled};
