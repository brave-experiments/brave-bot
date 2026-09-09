//! A condition the session works towards.
//!
//! `/goal` holds one sentence a person typed and puts it to a judge when a turn ends. Where the
//! judge says the condition does not hold yet, the work goes back for another turn; where it says
//! anything else, the goal is over. The condition is settled the moment it is typed and never
//! changes afterwards, so nothing a turn reads, writes or says can decide when this session is
//! allowed to stop.
//!
//! A goal starts no work of its own. It has a condition and no prompt, so an armed goal sits there
//! until the person sends something, and what it then keeps going is their own request.
//!
//! Nothing here is written to disk. A goal lives as long as the session that set it.

/// The word that takes a goal off, rather than setting one called `clear`.
const CLEAR: &str = "clear";

/// How many times a goal may send the work back before it gives up.
///
/// Every one of these is a whole turn with the conversation re-sent, so this is what stands
/// between a condition nobody can satisfy and a session that spends until somebody notices. It is
/// generous enough for the multi-turn work a goal is for and small enough that the bill for
/// getting a condition wrong is ten turns rather than an afternoon.
const MAX_ROUNDS: usize = 10;

/// What the argument to `/goal` asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// The bare word: say what the goal is, or that there is none.
    Report,
    /// Take the goal off.
    Clear,
    /// Work towards this condition.
    Set(String),
}

/// Read the argument to `/goal`.
///
/// One literal word is reserved and the rest of the language is a condition, which is as narrow as
/// the reservation can be made: `clear the build directory first` sets a goal, because the word is
/// not the whole argument.
pub fn parse(argument: &str) -> Asked {
    let argument = argument.trim();
    if argument.is_empty() {
        return Asked::Report;
    }
    if argument == CLEAR {
        return Asked::Clear;
    }
    Asked::Set(argument.to_string())
}

/// The goal a session is working towards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    /// The condition, exactly as it was typed.
    condition: String,
    /// How many times the work has been sent back.
    rounds: usize,
    /// What the judge said last time it was asked.
    last: Option<String>,
}

impl Running {
    /// Arm a goal. Nothing is sent: a goal has a condition and no prompt.
    pub fn begin(condition: String) -> Self {
        Self {
            condition,
            rounds: 0,
            last: None,
        }
    }

    pub fn condition(&self) -> &str {
        &self.condition
    }

    pub fn rounds(&self) -> usize {
        self.rounds
    }

    /// What the judge said the last time it was asked, or `None` before it has been.
    pub fn last_reason(&self) -> Option<&str> {
        self.last.as_deref()
    }

    /// How many more times the work may be sent back.
    pub fn left(&self) -> usize {
        MAX_ROUNDS.saturating_sub(self.rounds)
    }

    /// Record a verdict of not-yet, and say whether there is budget to send the work back.
    ///
    /// The reason is kept either way, because a goal that has just run out of rounds is exactly
    /// when a person wants to read what the judge kept saying.
    pub fn not_met(&mut self, reason: String) -> bool {
        self.last = Some(reason);
        if self.left() == 0 {
            return false;
        }
        self.rounds += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bare_command_asks_what_the_goal_is() {
        assert_eq!(parse(""), Asked::Report);
        assert_eq!(parse("   "), Asked::Report);
    }

    #[test]
    fn the_reserved_word_takes_the_goal_off() {
        assert_eq!(parse("clear"), Asked::Clear);
        assert_eq!(parse("  clear  "), Asked::Clear);
    }

    /// The reservation is one word and not a prefix, for the reason a command name is the whole
    /// word: a person asking for the build directory to be cleaned must not have their goal
    /// silently taken off instead.
    #[test]
    fn a_condition_that_begins_with_the_reserved_word_is_still_a_condition() {
        assert_eq!(
            parse("clear the build directory and cargo test exits 0"),
            Asked::Set("clear the build directory and cargo test exits 0".to_string())
        );
    }

    #[test]
    fn anything_else_is_the_condition_verbatim() {
        assert_eq!(
            parse("  cargo test -p bravebot-tui exits 0  "),
            Asked::Set("cargo test -p bravebot-tui exits 0".to_string())
        );
    }

    #[test]
    fn a_fresh_goal_has_sent_nothing_back_and_has_heard_nothing() {
        let goal = Running::begin("the tests pass".to_string());
        assert_eq!(goal.rounds(), 0);
        assert_eq!(goal.last_reason(), None);
        assert_eq!(goal.left(), MAX_ROUNDS);
    }

    #[test]
    fn a_verdict_of_not_yet_is_kept_and_counted() {
        let mut goal = Running::begin("the tests pass".to_string());
        assert!(goal.not_met("nothing ran them".to_string()));
        assert_eq!(goal.rounds(), 1);
        assert_eq!(goal.last_reason(), Some("nothing ran them"));
    }

    /// A condition nobody can satisfy would otherwise spend a session's whole budget re-sending a
    /// conversation that grows every round.
    #[test]
    fn a_goal_stops_sending_the_work_back_once_its_rounds_are_spent() {
        let mut goal = Running::begin("the tests pass".to_string());
        for round in 1..=MAX_ROUNDS {
            assert!(
                goal.not_met("still nothing".to_string()),
                "the goal gave up on round {round} of {MAX_ROUNDS}"
            );
        }
        assert_eq!(goal.left(), 0);
        assert!(!goal.not_met("still nothing".to_string()));
    }

    /// The round that runs out of budget is the one whose reason a person most wants to read, so
    /// giving up must not also throw away what the judge said.
    #[test]
    fn the_reason_from_the_round_that_gave_up_is_still_kept() {
        let mut goal = Running::begin("the tests pass".to_string());
        for _ in 0..MAX_ROUNDS {
            goal.not_met("still nothing".to_string());
        }
        goal.not_met("the linker is missing".to_string());
        assert_eq!(goal.last_reason(), Some("the linker is missing"));
    }
}
