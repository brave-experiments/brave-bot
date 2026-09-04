//! Where a turn's wall clock went.
//!
//! A turn takes a length of time, and until now that was the only thing recorded about it. The
//! figure is unactionable on its own: a turn that took four minutes because the model was slow, one
//! that took four minutes because a test suite ran, and one that took four minutes because it put a
//! diff on the screen and waited for somebody to come back from lunch are the same number and want
//! three different fixes.
//!
//! So the clock is split four ways, and the split is a partition rather than a set of overlapping
//! measures: every millisecond of a turn is charged to exactly one of inference, tools, stalled, or
//! the remainder. That is what makes the remainder worth reading. Approval prompts happen inside
//! tool calls, so what is spent waiting for a person is taken out of the tool figure rather than
//! counted in both.
//!
//! Milliseconds, and `u64` rather than `Duration`, because these are written into a session record
//! and read back by whatever is to hand. A `Duration` serialises as a struct of seconds and
//! nanoseconds, which is a poor thing to meet in a JSON file six months later.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// How a turn's wall clock divided up.
///
/// Nothing in here is content, a path, or a label. It is four counts of milliseconds, so it may be
/// written down and shown without any release: see [`crate::turn::Outcome::timing`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timing {
    /// The whole turn, end to end, as the turn loop saw it.
    ///
    /// The one figure that is not a component: the other three are parts of this, and what they do
    /// not account for is [`Timing::overhead_ms`].
    #[serde(default)]
    pub wall_ms: u64,
    /// Waiting on the model, summed over every round and every processor.
    ///
    /// A turn is several requests when the model calls tools, and a processor is a request of its
    /// own, so a figure for one round would understate this by however long the rest took.
    #[serde(default)]
    pub inference_ms: u64,
    /// Running tools, with time spent waiting for a person taken out.
    ///
    /// An approval prompt is drawn from inside a tool call, so the naive measure would charge the
    /// same seconds here and to [`Timing::stalled_ms`]. Subtracted rather than left in both,
    /// because the four figures are meant to add up.
    #[serde(default)]
    pub tools_ms: u64,
    /// Waiting for a person to answer a prompt.
    ///
    /// Every approval and every question, whatever was answered: a refusal took as long to arrive
    /// as an approval, and this measures the wait rather than the decision.
    #[serde(default)]
    pub stalled_ms: u64,
}

impl Timing {
    /// What the turn spent on none of the three, which is the driver's own.
    ///
    /// Reading a session back, this is the figure with nobody to blame for it: not the model, not a
    /// subprocess, not the person. Anything large here is time the harness spent on itself, and
    /// there is no other way to see it.
    ///
    /// Saturating, because the parts are measured separately and a clock that steps back must not
    /// produce a wrapped figure in the billions.
    pub fn overhead_ms(&self) -> u64 {
        self.wall_ms
            .saturating_sub(self.inference_ms)
            .saturating_sub(self.tools_ms)
            .saturating_sub(self.stalled_ms)
    }

    /// Add another turn's figures, for a total over a session.
    pub fn add(&mut self, other: Timing) {
        self.wall_ms += other.wall_ms;
        self.inference_ms += other.inference_ms;
        self.tools_ms += other.tools_ms;
        self.stalled_ms += other.stalled_ms;
    }
}

/// Durations as the turn loop accumulates them, before they become a [`Timing`].
///
/// The loop adds up `Duration`s because that is what `Instant::elapsed` gives it, and rounding each
/// one to milliseconds as it arrived would lose a fraction per round and end up short by the number
/// of rounds. Rounded once, here.
#[derive(Debug, Clone, Copy, Default)]
pub struct Elapsed {
    pub wall: Duration,
    pub inference: Duration,
    /// Already net of `stalled`: see [`Timing::tools_ms`].
    pub tools: Duration,
    pub stalled: Duration,
}

impl Elapsed {
    pub fn finish(self) -> Timing {
        Timing {
            wall_ms: self.wall.as_millis() as u64,
            inference_ms: self.inference.as_millis() as u64,
            tools_ms: self.tools.as_millis() as u64,
            stalled_ms: self.stalled.as_millis() as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The point of the whole type: what is left when the three known costs are taken out.
    #[test]
    fn the_remainder_is_what_nothing_else_accounts_for() {
        let timing = Timing {
            wall_ms: 10_000,
            inference_ms: 6_000,
            tools_ms: 1_500,
            stalled_ms: 2_000,
        };
        assert_eq!(timing.overhead_ms(), 500);
    }

    /// A clock that steps backwards, or three parts measured slightly over the whole, must not turn
    /// a small discrepancy into a figure in the billions.
    #[test]
    fn parts_exceeding_the_whole_do_not_wrap() {
        let timing = Timing {
            wall_ms: 100,
            inference_ms: 200,
            tools_ms: 200,
            stalled_ms: 200,
        };
        assert_eq!(timing.overhead_ms(), 0);
    }

    #[test]
    fn a_session_total_adds_every_turn() {
        let mut total = Timing::default();
        total.add(Timing {
            wall_ms: 100,
            inference_ms: 60,
            tools_ms: 10,
            stalled_ms: 20,
        });
        total.add(Timing {
            wall_ms: 50,
            inference_ms: 30,
            tools_ms: 5,
            stalled_ms: 5,
        });
        assert_eq!(total.wall_ms, 150);
        assert_eq!(total.inference_ms, 90);
        assert_eq!(total.tools_ms, 15);
        assert_eq!(total.stalled_ms, 25);
        assert_eq!(total.overhead_ms(), 20);
    }

    /// Rounded once at the end rather than per round, or a turn of many short rounds comes back
    /// short by a fraction of a millisecond per round.
    #[test]
    fn durations_are_rounded_once_rather_than_per_round() {
        let mut elapsed = Elapsed::default();
        for _ in 0..10 {
            elapsed.inference += Duration::from_micros(1_900);
        }
        assert_eq!(elapsed.finish().inference_ms, 19);
    }

    /// The figures are written into a session record, so they have to survive being one.
    #[test]
    fn timing_survives_a_round_trip_as_json() {
        let timing = Timing {
            wall_ms: 1,
            inference_ms: 2,
            tools_ms: 3,
            stalled_ms: 4,
        };
        let body = serde_json::to_string(&timing).expect("serialises");
        assert_eq!(
            serde_json::from_str::<Timing>(&body).expect("parses"),
            timing
        );
    }

    /// A record written before this was kept reads as a turn that took no time, which is the same
    /// way an absent token breakdown reads: nothing recorded rather than an error.
    #[test]
    fn a_record_without_timing_reads_as_zero() {
        let timing: Timing = serde_json::from_str("{}").expect("parses");
        assert_eq!(timing, Timing::default());
    }
}
