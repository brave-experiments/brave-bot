//! Interactive terminal interface.
//!
//! A session is N sequential turns, each with its own policy and routing precommit. The
//! interface holds transcript and input state only: no policy outlives a turn, so
//! conversation history can never become routing for a later one.

pub mod app;
pub mod ask;
pub mod audit;
pub mod clipboard;
pub mod confirm;
pub mod dropped;
pub mod editor;
pub mod entries;
pub mod history;
pub mod indicator;
pub mod logo;
pub mod markdown;
pub mod model_prompt;
pub mod remote_confirm;
pub mod render;
pub mod resume;
pub mod select;
pub mod sessions;
pub mod state;
pub mod status;
pub mod store;
pub mod table;
pub mod theme;
pub mod trust_prompt;

/// What this build is: the version, the commit it was built from, and whether the tree had
/// uncommitted changes at the time.
///
/// Written into every session record, so a transcript read later can be matched to the code that
/// produced it rather than inferred from its own symptoms.
pub const BUILD: &str = env!("BRAVEBOT_BUILD");
pub mod verbs;
pub mod wrap;

pub use state::{Entry, Session, Speaker, Status};
