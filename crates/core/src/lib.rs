//! Information-flow kernel.
//!
//! This crate performs no I/O. It defines the label lattice, quarantined storage,
//! capabilities, and the policy gates every consequential action passes through.
//! Nothing here prints: diagnostics leave through typed events so the audit trail
//! stays machine-readable.

pub mod label;

pub use label::{Confidentiality, Integrity, Label, taint_all};
