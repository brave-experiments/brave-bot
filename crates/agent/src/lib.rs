//! Task execution.
//!
//! Holds the label-aware tools and the turn loop. Tools take their routing arguments
//! from precommitted routing, never from model output, so a turn cannot be redirected
//! by the content it processes.

pub mod backend;
pub mod compact;
pub mod confirm;
pub mod conversation;
pub mod delegate;
pub mod diff;
pub mod exec;
pub mod glob;
pub mod home;
pub mod manifest;
pub mod mode;
pub mod permissions;
pub mod preamble;
pub mod processor;
pub mod programs;
pub mod replace;
pub mod report;
pub mod scrub;
pub mod shell;
pub mod skills;
pub mod subscription;
pub mod timing;
pub mod tools;
pub mod turn;
pub mod workspace;

pub use confirm::{Confirmer, Decision, Intent, RunDecision, RunRequest, Unattended, WriteRequest};
pub use conversation::Conversation;
pub use delegate::Delegated;
pub use mode::Mode;
pub use processor::ProcessorError;
pub use report::{Activity, IgnoreReports, Reporter};
pub use subscription::{Discovery, ImportedSubscription};
pub use turn::{Outcome, Task, TurnError};
pub use workspace::{Workspace, WorkspaceError};
