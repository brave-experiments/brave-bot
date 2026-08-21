//! Task execution.
//!
//! Holds the label-aware tools and the turn loop. Tools take their routing arguments
//! from precommitted routing, never from model output, so a turn cannot be redirected
//! by the content it processes.

pub mod turn;
pub mod workspace;

pub use turn::{Outcome, Task, TurnError};
pub use workspace::{Workspace, WorkspaceError};
