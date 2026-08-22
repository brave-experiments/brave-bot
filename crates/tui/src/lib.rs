//! Interactive terminal interface.
//!
//! A session is N sequential turns, each with its own policy and routing precommit. The
//! interface holds transcript and input state only: no policy outlives a turn, so
//! conversation history can never become routing for a later one.

pub mod app;
pub mod clipboard;
pub mod confirm;
pub mod history;
pub mod indicator;
pub mod markdown;
pub mod remote_confirm;
pub mod render;
pub mod select;
pub mod state;
pub mod store;
pub mod trust_prompt;
pub mod verbs;
pub mod wrap;

pub use state::{Entry, Session, Speaker, Status};
