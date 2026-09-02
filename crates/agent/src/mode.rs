//! Which way a task is run.
//!
//! Three shapes, and the difference is *when* control flow is decided and *what the model is
//! shown in order to decide it*, rather than how strictly anything is enforced. Every mode runs
//! under the same kernel, the same gates, and the same guarantee: untrusted content can be
//! carried and written, and can never decide what happens.
//!
//! What changes is the scope of the commitment. A [`Mode::Turn`] run precommits routing and the
//! release plan per turn, then lets the planner choose the next step from what it has seen. A
//! [`Mode::Manifest`] run precommits both for the whole run, from a plan fixed before anything
//! was observed. The second buys a program nothing at run time can reshape, and pays for it in
//! everything a plan cannot know in advance.
//!
//! [`Mode::SkillState`] moves on the other axis. It decides the next step from what it has seen,
//! exactly as a turn does, but it is not shown the history it saw it in: each step carries the
//! task, one structured state the model itself maintains, and the newest observation. What that
//! buys is a request whose size stops growing with the length of the run. What it pays is
//! everything the model failed to write into the state before the history went out of view.

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
    /// Observe, decide, act, repeat, with the history replaced by a state the model maintains.
    ///
    /// A session may hold this one, unlike [`Mode::Manifest`], because it is still a turn loop:
    /// what changes is what a round is shown, not who decides the next step.
    SkillState,
}

impl Mode {
    /// The names accepted on a command line, for help text that cannot drift from the parser.
    pub const NAMES: [&'static str; 3] = ["turn", "manifest", "skill-state"];

    /// Whether this mode is a loop a person can be part of, and so a mode a session may hold.
    ///
    /// A session is several turns over one conversation. Both turn loops qualify: the user types,
    /// the model decides what to do next, and the user types again. A manifest run fixes every
    /// step before the first one runs, so a second prompt has nothing to join, which is why an
    /// interactive session cannot be in that mode.
    pub fn is_a_turn_loop(&self) -> bool {
        match self {
            Self::Turn | Self::SkillState => true,
            Self::Manifest => false,
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Turn => f.write_str("turn"),
            Self::Manifest => f.write_str("manifest"),
            Self::SkillState => f.write_str("skill-state"),
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
            "skill-state" => Ok(Self::SkillState),
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
        for text in [
            "",
            "Manifest",
            "manifests",
            "strict",
            "safehouse",
            // Near misses on the newest name, which is the one with a separator in it and so the
            // one most likely to be typed another way. Guessing would run a mode nobody asked for.
            "skill_state",
            "skillstate",
            "skill state",
            "state",
            "SKILL.state",
        ] {
            assert!(text.parse::<Mode>().is_err(), "'{text}' was accepted");
        }
    }

    /// The two loops a session may hold, and the one it may not.
    ///
    /// A session is several turns over one conversation, and both turn modes are turn loops: a
    /// person types, the model decides, and they type again. A manifest run fixes every step
    /// before the first one, so there is nothing for a second prompt to join.
    #[test]
    fn a_session_may_hold_either_turn_loop_and_not_a_frozen_plan() {
        assert!(Mode::Turn.is_a_turn_loop());
        assert!(Mode::SkillState.is_a_turn_loop());
        assert!(!Mode::Manifest.is_a_turn_loop());
    }
}
