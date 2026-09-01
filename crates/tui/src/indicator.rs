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
use bravebot_i18n::t;
use std::borrow::Cow;
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
    /// What to call what is happening.
    ///
    /// Usually the turn's own word. A turn keeping a task list lends its active task instead,
    /// which is why this is not a `&'static str`: the text came from the model.
    pub verb: Cow<'static, str>,
    /// Elapsed time, already formatted.
    pub elapsed: String,
    /// Tokens spent, already formatted. `None` when the server reported none.
    pub tokens: Option<String>,
    /// Tokens the model has written, already formatted. `None` before it writes any.
    ///
    /// Shown separately from the total because it is the part that moves while someone waits: the
    /// total is mostly the history each round re-sends, and it only changes between rounds.
    pub written: Option<String>,
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
            verb: Cow::Borrowed(verbs::for_turn(turn)),
            elapsed: format_elapsed(elapsed),
            tokens: (tokens > 0).then(|| format_tokens(tokens)),
            written: None,
        }
    }

    /// Report how much the model has written so far.
    ///
    /// Zero is left absent rather than shown, so the counter appears when there is something to
    /// count instead of sitting at `0` through the whole wait before the first chunk lands.
    pub fn writing(mut self, written: u64) -> Self {
        self.written = (written > 0).then(|| format_tokens(written));
        self
    }

    /// Replace the word with what the model says it is doing.
    ///
    /// Naming the work is better than naming the turn: "Adding prompt history…" answers the
    /// question a waiting user has, and the generic word does not. Only the label changes, so a
    /// list that says nothing useful costs nothing but a worse word.
    pub fn labelled(mut self, what: String) -> Self {
        self.verb = Cow::Owned(what);
        self
    }

    /// The counters, parenthesised, for rendering in their own style.
    pub fn detail(&self) -> String {
        let mut detail = self.elapsed.clone();
        if let Some(tokens) = &self.tokens {
            // The arrow marks these as tokens consumed, matching how usage is reported.
            detail.push_str(&format!(
                " · {}",
                t!(indicator_tokens_read, tokens = tokens)
            ));
        }
        if let Some(written) = &self.written {
            // The opposite arrow, since these went the other way. No unit: it sits beside a figure
            // already labelled tokens, and repeating the word crowds the line.
            detail.push_str(&format!(
                " · {}",
                t!(indicator_tokens_written, tokens = written)
            ));
        }
        format!("({detail})")
    }

    /// The whole line, as shown.
    pub fn line(&self) -> String {
        format!("{} {}… {}", self.glyph, self.verb, self.detail())
    }
}

/// The spinner glyph for a moment, for a wait that is not a turn.
///
/// Shared so a running command turns the same shape at the same rate as a running turn: two
/// spinners that differed would look like two kinds of busy.
pub fn glyph_at(elapsed: Duration) -> &'static str {
    FRAMES[(elapsed.as_millis() / FRAME_MILLIS) as usize % FRAMES.len()]
}

/// Format a duration the way someone waiting reads it.
///
/// Seconds alone up to a minute, then minutes and seconds. No hours: a turn that ran that long
/// has gone wrong, and `73m 04s` says so more plainly than `1h 13m`.
pub fn format_elapsed(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    if total < 60 {
        return t!(elapsed_seconds, seconds = total);
    }
    // Zero-padded here rather than in the catalog, since the padding is what keeps the line from
    // twitching as the seconds roll over and is not something a translation gets a say in.
    t!(
        elapsed_minutes,
        minutes = total / 60,
        seconds = format!("{:02}", total % 60)
    )
}

/// Format a token count compactly.
///
/// Thousands are abbreviated with one decimal, because the exact figure is noise at that scale
/// and a five-digit number in a status line is harder to read at a glance than `38.3k`.
pub fn format_tokens(tokens: u64) -> String {
    if tokens < 1_000 {
        return tokens.to_string();
    }
    if tokens < 1_000_000 {
        return t!(
            tokens_thousands,
            thousands = rounded(tokens as f64 / 1_000.0)
        );
    }
    t!(
        tokens_millions,
        millions = rounded(tokens as f64 / 1_000_000.0)
    )
}

/// One decimal place, written the way the reader's language writes a fraction.
fn rounded(value: f64) -> String {
    format!("{value:.1}").replace('.', t!(number_decimal_separator))
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

    /// A named task replaces the word, so the line says what is happening rather than which turn
    /// this is. Everything else about the line is untouched.
    #[test]
    fn a_named_task_replaces_the_word_and_nothing_else() {
        let generic = Indicator::new(0, Duration::from_secs(751), 38_300);
        let named = generic
            .clone()
            .labelled("Adding prompt history".to_string());

        assert_eq!(
            named.line(),
            "✳ Adding prompt history… (12m 31s · ↓ 38.3k tokens)"
        );
        assert_eq!(named.glyph, generic.glyph);
        assert_eq!(named.detail(), generic.detail());
    }

    /// Output tokens sit beside the total, marked with the opposite arrow: one is what was sent,
    /// the other what came back.
    #[test]
    fn written_tokens_appear_alongside_the_total() {
        let indicator = Indicator::new(0, Duration::from_secs(12), 9_200).writing(512);
        assert_eq!(
            indicator.detail(),
            "(12s \u{b7} \u{2193} 9.2k tokens \u{b7} \u{2191} 512)"
        );
    }

    /// Nothing written yet means no counter, rather than a `0` sitting there through the wait
    /// before the first chunk lands.
    #[test]
    fn nothing_written_shows_no_counter() {
        let indicator = Indicator::new(0, Duration::from_secs(3), 0).writing(0);
        assert!(indicator.written.is_none());
        assert_eq!(indicator.detail(), "(3s)");
    }

    /// The figure has to be able to climb, since that is the whole point of streaming it.
    #[test]
    fn the_written_count_climbs() {
        let first = Indicator::new(0, Duration::from_secs(1), 0).writing(10);
        let later = Indicator::new(0, Duration::from_secs(1), 0).writing(120);
        assert_ne!(first.written, later.written);
        assert_eq!(later.written.as_deref(), Some("120"));
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
