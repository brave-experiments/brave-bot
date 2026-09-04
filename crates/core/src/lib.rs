//! Information-flow kernel.
//!
//! This crate performs no I/O. It defines the label lattice, quarantined storage,
//! capabilities, and the policy gates every consequential action passes through.
//! Nothing here prints: diagnostics leave through typed events so the audit trail
//! stays machine-readable.
//!
//! # Credit
//!
//! Ali Shahin Shamsabadi and Brian R. Bondy developed the idea this crate implements: the
//! `L = I × C` lattice, write-once quarantine, and the routing/content asymmetry that
//! makes injected text unable to redirect an action. Ali built the first prototype of it
//! in SafeHouse; this applies the same idea to a general-purpose agent.

pub mod ask;
pub mod cancel;
pub mod capability;
pub mod command;
pub mod event;
pub mod fence;
pub mod label;
pub mod manifest;
pub mod policy;
pub mod processor;
pub mod programs;
pub mod pure;
pub mod reference;
pub mod slot;
pub mod todo;
pub mod trust;
pub mod value;
pub mod vet;

pub use ask::{Answer, Choice, Prompt, Question};
pub use cancel::Cancel;
pub use capability::{Capability, CapabilitySet, CapabilityToken};
pub use command::{Pipeline, Stage};
pub use event::{Event, NullSink, Principle, RecordingSink, Role, Sink};
pub use label::{Confidentiality, Integrity, Label, taint_all};
pub use manifest::{Draft, DraftStep, Manifest, ManifestError, Step as ManifestStep, Tier};
pub use policy::{Denial, Policy, ReleasePlan, Routing};
pub use processor::ProcessorSpec;
pub use programs::TrustedPrograms;
pub use pure::{Filter, is_pure_filter};
pub use reference::{Presentation, Reference};
pub use slot::{Measured, SlotError, SlotId, SlotReader, SlotStore, SlotWriter};
pub use todo::{Item as TodoItem, List as TodoList, Status as TodoStatus};
pub use trust::{Provenance, TrustStore};
pub use value::{Declassification, Labelled};
pub use vet::{Verdict, VetSpec};
