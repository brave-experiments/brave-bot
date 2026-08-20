//! Information-flow kernel.
//!
//! This crate performs no I/O. It defines the label lattice, quarantined storage,
//! capabilities, and the policy gates every consequential action passes through.
//! Nothing here prints: diagnostics leave through typed events so the audit trail
//! stays machine-readable.

pub mod capability;
pub mod label;
pub mod slot;
pub mod value;

pub use capability::{Capability, CapabilitySet, CapabilityToken};
pub use label::{Confidentiality, Integrity, Label, taint_all};
pub use slot::{SlotError, SlotId, SlotReader, SlotStore, SlotWriter};
pub use value::{Declassification, Labelled};
