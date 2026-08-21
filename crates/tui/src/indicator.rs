//! The working indicator: a spinning glyph, a word, elapsed time, and tokens spent.
//!
//! ```text
//! ✻ Grooving… (12m 31s · ↓ 38.3k tokens)
//! ```
//!
//! Turns run synchronously, so the interface is unresponsive while one is in flight. Without
//! something visibly moving that is indistinguishable from a hang, and the two counters answer
//! the questions a waiting user actually has: how long has this been going, and what is it
//! costing me.
//!
//! No progress bar, because there is nothing honest to measure against. A turn takes as many
//! rounds as the model asks for, and a bar that guessed would be a lie that looks like data.
//!
//! Rendering is separated from drawing so the whole thing is testable without a terminal.

use crate::verbs;
use std::time::Duration;

/// Glyphs the spinner cycles through.
///
/// Deliberately a shape that reads as rotation rather than a set of unrelated pictures: the
/// eye should see one thing turning, not a slideshow. All are single-width in a terminal, so
/// the text after them does not jitter as the frame changes.
const FRAMES: &[&str] = &["✳", "✽", "✻", "✺", "✹", "✸"];

/// How long each glyph is shown. Slow enough not to strobe, quick enough to read as motion.
const FRAME_MILLIS: u128 = 120;

/// What to show while a turn is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Indicator {
    /// The spinner glyph for this moment.
    pub glyph: &'static str,
    /// The word for this turn.
    pub verb: &'static str,
    /// Elapsed time, already formatted.
    pub elapsed: String,
    /// Tokens spent, already formatted. `None` when the server reported none.
    pub tokens: Option<String>,
}

impl Indicator {
    /// Build the indicator for a turn at a point in time.
    ///
    /// `turn` selects the word, `elapsed` drives both the clock and which glyph shows, and
    /// `tokens` is the running total. Taking elapsed time as an argument rather than reading a
    /// clock keeps this a pure function, so a test can assert on any instant it likes.
    pub fn new(turn: usize, elapsed: Duration, tokens: u64) -> Self {
        let frame = (elapsed.as_millis() / FRAME_MILLIS) as usize % FRAMES.len();
        Self {
            glyph: FRAMES[frame],
            verb: verbs::for_turn(turn),
            elapsed: format_elapsed(elapsed),
            tokens: (tokens > 0).then(|| format_tokens(tokens)),
        }
    }

    /// The counters, parenthesised, for rendering in their own style.
    pub fn detail(&self) -> String {
        let mut detail = self.elapsed.clone();
        if let Some(tokens) = &self.tokens {
            // The arrow marks these as tokens consumed, matching how usage is reported.
            detail.push_str(&format!(" · ↓ {tokens} tokens"));
        }
        format!("({detail})")
    }

    /// The whole line, as shown.
    pub fn line(&self) -> String {
        format!("{} {}… {}", self.glyph, self.verb, self.detail())
    }
}

/// Format a duration the way someone waiting reads it.
///
/// Seconds alone up to a minute, then minutes and seconds. No hours: a turn that ran that long
/// has gone wrong, and `73m 04s` says so more plainly than `1h 13m`.
fn format_elapsed(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    if total < 60 {
        format!("{total}s")
    } else {
        format!("{}m {:02}s", total / 60, total % 60)
    }
}

/// Format a token count compactly.
///
/// Thousands are abbreviated with one decimal, because the exact figure is noise at that scale
/// and a five-digit number in a status line is harder to read at a glance than `38.3k`.
fn format_tokens(tokens: u64) -> String {
    if tokens < 1_000 {
        tokens.to_string()
    } else if tokens < 1_000_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64, tokens: u64) -> Indicator {
        Indicator::new(0, Duration::from_secs(secs), tokens)
    }

    /// The shape asked for, end to end.
    #[test]
    fn the_line_has_the_expected_shape() {
        let indicator = Indicator::new(0, Duration::from_secs(751), 38_300);
        assert_eq!(indicator.line(), "✳ Grooving… (12m 31s · ↓ 38.3k tokens)");
    }

    /// Something must move, or the interface is indistinguishable from a hang.
    #[test]
    fn the_glyph_advances_with_time() {
        let first = Indicator::new(0, Duration::from_millis(0), 0).glyph;
        let second = Indicator::new(0, Duration::from_millis(FRAME_MILLIS as u64), 0).glyph;
        assert_ne!(first, second, "the spinner did not advance");
    }

    /// And it must keep moving indefinitely rather than stopping at the end of the list.
    #[test]
    fn the_glyph_cycles_forever() {
        let full_cycle = FRAME_MILLIS as u64 * FRAMES.len() as u64;
        assert_eq!(
            Indicator::new(0, Duration::from_millis(0), 0).glyph,
            Indicator::new(0, Duration::from_millis(full_cycle), 0).glyph,
        );
        // Far in the future it is still producing a valid frame, not panicking on an index.
        let late = Indicator::new(0, Duration::from_secs(86_400), 0);
        assert!(FRAMES.contains(&late.glyph));
    }

    /// Every glyph gets used, or the ones at the end are decoration that never shows.
    #[test]
    fn every_frame_is_reachable() {
        let seen: Vec<&str> = (0..FRAMES.len())
            .map(|n| {
                Indicator::new(0, Duration::from_millis(n as u64 * FRAME_MILLIS as u64), 0).glyph
            })
            .collect();
        for frame in FRAMES {
            assert!(seen.contains(frame), "{frame} is never shown");
        }
    }

    /// A different word each turn is the point; the same word every time would not signal a
    /// new turn had started.
    #[test]
    fn the_word_changes_between_turns() {
        let first = Indicator::new(0, Duration::from_secs(1), 0).verb;
        let second = Indicator::new(1, Duration::from_secs(1), 0).verb;
        assert_ne!(first, second);
    }

    #[test]
    fn seconds_show_alone_under_a_minute() {
        assert_eq!(format_elapsed(Duration::from_secs(0)), "0s");
        assert_eq!(format_elapsed(Duration::from_secs(9)), "9s");
        assert_eq!(format_elapsed(Duration::from_secs(59)), "59s");
    }

    /// Seconds are zero-padded past a minute so the line does not change width as it ticks.
    #[test]
    fn minutes_and_seconds_are_padded() {
        assert_eq!(format_elapsed(Duration::from_secs(60)), "1m 00s");
        assert_eq!(format_elapsed(Duration::from_secs(61)), "1m 01s");
        assert_eq!(format_elapsed(Duration::from_secs(751)), "12m 31s");
    }

    /// No hours: a turn running that long has gone wrong, and the minute count says so.
    #[test]
    fn long_runs_keep_counting_in_minutes() {
        assert_eq!(format_elapsed(Duration::from_secs(4_384)), "73m 04s");
    }

    #[test]
    fn small_token_counts_are_exact() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(1), "1");
        assert_eq!(format_tokens(999), "999");
    }

    #[test]
    fn thousands_are_abbreviated() {
        assert_eq!(format_tokens(1_000), "1.0k");
        assert_eq!(format_tokens(38_300), "38.3k");
        assert_eq!(format_tokens(999_400), "999.4k");
    }

    #[test]
    fn millions_are_abbreviated() {
        assert_eq!(format_tokens(1_000_000), "1.0M");
        assert_eq!(format_tokens(2_500_000), "2.5M");
    }

    /// A server that reports no usage must not produce "↓ 0 tokens", which would look like a
    /// bug rather than an absent figure.
    #[test]
    fn no_tokens_means_no_token_section() {
        let indicator = at(5, 0);
        assert!(indicator.tokens.is_none());
        let line = indicator.line();
        assert!(
            !line.contains("tokens"),
            "showed a token count of zero: {line}"
        );
        assert!(line.contains("5s"), "lost the elapsed time: {line}");
    }

    /// The counters exist to answer "how long" and "how much", so both must appear once there
    /// is something to report.
    #[test]
    fn both_counters_appear_when_available() {
        let line = at(90, 1_500).line();
        assert!(line.contains("1m 30s"));
        assert!(line.contains("1.5k tokens"));
    }
}
