//! Which way a task is run.
//!
//! Two shapes, and the difference is *when* control flow is decided rather than how strictly
//! anything is enforced. Both modes run under the same kernel, the same gates, and the same
//! guarantee: untrusted content can be carried and written, and can never decide what happens.
//!
//! What changes is the scope of the commitment. A [`Mode::Turn`] run precommits routing and the
//! release plan per turn, then lets the planner choose the next step from what it has seen. A
//! [`Mode::Manifest`] run precommits both for the whole run, from a plan fixed before anything
//! was observed. The second buys a program nothing at run time can reshape, and pays for it in
//! everything a plan cannot know in advance.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Observe, decide, act, repeat. The default, and the only mode a session can hold, since a
    /// session is several turns over one conversation.
    #[default]
    Turn,
    /// Plan the whole run first, then execute it with no model in the control path.
    Manifest,
}

impl Mode {
    /// The names accepted on a command line, for help text that cannot drift from the parser.
    pub const NAMES: [&'static str; 2] = ["turn", "manifest"];
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Turn => f.write_str("turn"),
            Self::Manifest => f.write_str("manifest"),
        }
    }
}

impl FromStr for Mode {
    type Err = String;

    /// Exact names only. A near miss is refused rather than guessed at, because guessing wrong
    /// here would silently run the mode the user did not ask for.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "turn" => Ok(Self::Turn),
            "manifest" => Ok(Self::Manifest),
            other => Err(format!(
                "'{other}' is not a mode; use {}",
                Self::NAMES.join(" or ")
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Turn mode is what a session is and what an unqualified run gets, so the default has to
    /// stay the one that behaves the way it always has.
    #[test]
    fn the_default_is_the_turn_loop() {
        assert_eq!(Mode::default(), Mode::Turn);
    }

    #[test]
    fn every_advertised_name_parses_back() {
        for name in Mode::NAMES {
            let mode: Mode = name.parse().expect("advertised names must parse");
            assert_eq!(mode.to_string(), name);
        }
    }

    /// A misspelling must fail rather than fall back, or someone asking for a frozen plan gets
    /// an adaptive loop and no indication that they did.
    #[test]
    fn an_unknown_mode_is_refused() {
        for text in ["", "Manifest", "manifests", "strict", "safehouse"] {
            assert!(text.parse::<Mode>().is_err(), "'{text}' was accepted");
        }
    }
}
