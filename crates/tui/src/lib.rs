//! Interactive terminal interface.
//!
//! A session is N sequential turns, each with its own policy and routing precommit. The
//! interface holds transcript and input state only: no policy outlives a turn, so
//! conversation history can never become routing for a later one.

pub mod app;
pub mod confirm;
pub mod indicator;
pub mod remote_confirm;
pub mod render;
pub mod state;
pub mod trust_prompt;
pub mod verbs;

pub use state::{Entry, Session, Speaker, Status};
