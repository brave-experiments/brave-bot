//! Progress for a run with no interface to draw on.
//!
//! A one-shot run has no spinner and no redraw, so without this it prints nothing at all until
//! the reply arrives. Turns take as many rounds as the work needs, and one that reads a dozen
//! files and edits three of them is a long silence to sit through.
//!
//! Everything here goes to stderr. The reply is the command's output and belongs on stdout,
//! where it can be piped into something else; a progress log that shared that stream would
//! corrupt it.

use bravebot_agent::report::{Activity, Reporter, Shown};
use bravebot_core::todo::Row;
use bravebot_tui::wrap::display_width;
use std::io::Write;

/// Marks a call, matching the interactive transcript so the two read alike.
const CALL_MARKER: &str = "\u{23fa}";
/// Marks the detail belonging to the line above it.
const DETAIL_MARKER: &str = "\u{23bf}";

/// Drawn down the margin of everything the planner was not allowed to read.
///
/// The same glyph the interactive transcript uses, on every row of the block, so the mark cannot
/// be ended by anything written inside it. A caption could be imitated; a margin cannot.
const QUARANTINE_BAR: &str = "\u{2503}";

/// The width a shown block is broken at.
///
/// stderr is as often a pipe or a log file as a terminal, so there is no width to ask for, and a
/// previewed line arrives trimmed to 160 characters: wider than most terminals, which would wrap
/// it themselves. A row the terminal wrapped starts at column 0 with no margin in front of it,
/// which is the one thing the marking exists to make impossible, so the rows are the reporter's
/// to make. Eighty is the width assumed of a terminal that has not said otherwise.
const BLOCK_WIDTH: usize = 80;

/// Replace control characters, so shown text cannot move the cursor or recolour the screen.
///
/// The margin in front of every row is written by this module. An escape sequence in the content
/// would let the content write one instead, and a forged margin is worse than no margin, since
/// drawing one is the whole claim being made about the block.
fn printable(text: &str) -> String {
    text.chars()
        .map(|c| {
            if !c.is_control() || c == '\t' {
                c
            } else {
                // The Unicode pictures for C0, so an escape reads as ␛ rather than vanishing: a
                // character silently dropped is one nobody can tell was ever in the file.
                char::from_u32(0x2400 + c as u32).unwrap_or('\u{fffd}')
            }
        })
        .collect()
}

/// One line of shown content as rows, each beginning with the margin.
fn marked(margin: &str, line: &str) -> Vec<String> {
    // One column at least. A margin wider than the block still gets its margin, and the content
    // is broken a character at a time rather than written outside it.
    let room = BLOCK_WIDTH.saturating_sub(display_width(margin)).max(1);
    let mut rows = Vec::new();
    let mut row = String::new();
    let mut used = 0usize;

    for c in printable(line).chars() {
        let mut buffer = [0u8; 4];
        let reached = display_width(c.encode_utf8(&mut buffer));
        if used > 0 && used + reached > room {
            rows.push(format!("{margin}{row}"));
            row.clear();
            used = 0;
        }
        row.push(c);
        used += reached;
    }

    // Always a row, so an empty line is a marked one rather than a gap in the block.
    rows.push(format!("{margin}{row}"));
    rows
}

/// Writes progress as it arrives.
///
/// Generic over the sink so a test can read back exactly what a run would have printed.
pub struct Progress<W: Write> {
    out: W,
}

impl<W: Write> Progress<W> {
    pub fn new(out: W) -> Self {
        Self { out }
    }

    /// A failed write is dropped. Progress announces and has nothing to refuse with, so a
    /// closed stderr means nobody is reading, not that the turn should stop.
    fn say(&mut self, line: &str) {
        let _ = writeln!(self.out, "{line}");
        let _ = self.out.flush();
    }
}

impl<W: Write> Reporter for Progress<W> {
    /// Left to the per-call lines. A task list redrawn in place is legible; the same list
    /// reprinted in full after every step is a wall of repetition.
    fn todos(&mut self, _rows: Vec<Row>) {}

    fn narration(&mut self, text: String) {
        // Dropped here rather than by the turn, which cannot look at the text to decide. This
        // side may: it has been released, and a blank line is a presentation question.
        if text.trim().is_empty() {
            return;
        }
        self.say(&format!("\n{text}\n"));
    }

    /// Printed as the call begins, which is the whole point: a slow call should be visible
    /// while it is slow rather than only once it is over.
    fn tool_started(&mut self, activity: Activity) {
        self.say(&format!("{CALL_MARKER} {}", activity.line()));
    }

    fn tool_finished(&mut self, activity: Activity) {
        if let Some(note) = &activity.note {
            self.say(&format!("  {DETAIL_MARKER} {note}"));
        }
    }

    /// Written where the interactive transcript would draw the block, and for the same reason:
    /// the person watching owns the workspace and is the one who can tell whether the agent is
    /// working on the right file. Silence here is indistinguishable from having nothing to show.
    ///
    /// On stderr with the rest of the progress, since stdout is the reply.
    fn quarantined(&mut self, shown: Shown) {
        let margin = format!("  {QUARANTINE_BAR} ");

        // The heading is marked like the content, because the origin is not this module's text:
        // it can be a filename read out of a quarantined listing. So it is neutralised too, and a
        // long one continues on another marked row rather than outside the block.
        let mut block = vec![
            format!("untrusted \u{b7} {} \u{b7} {}", shown.origin, shown.label),
            format!("  {}", shown.reach.describe()),
        ];
        block.extend(shown.preview.iter().cloned());

        // Said rather than silently dropped: a preview that stops without saying so reads as the
        // whole of what was there.
        if shown.lines > shown.preview.len() {
            block.push(format!(
                "\u{2026} {} more lines",
                shown.lines - shown.preview.len()
            ));
        }

        self.say("");
        for line in &block {
            for row in marked(&margin, line) {
                self.say(&row);
            }
        }
        self.say("");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bravebot_agent::report::Reach;

    fn log(run: impl FnOnce(&mut Progress<&mut Vec<u8>>)) -> String {
        let mut buffer = Vec::new();
        let mut progress = Progress::new(&mut buffer);
        run(&mut progress);
        String::from_utf8(buffer).expect("utf-8")
    }

    /// The line has to appear before the call finishes, or a one-shot run is silent for
    /// exactly as long as the slow part takes.
    #[test]
    fn a_call_is_printed_when_it_begins() {
        let written = log(|p| p.tool_started(Activity::running("Read", "src/main.rs")));
        assert!(written.contains("Read(src/main.rs)"), "got: {written}");
    }

    #[test]
    fn a_finished_call_prints_what_came_of_it() {
        let written = log(|p| {
            p.tool_finished(Activity::running("Search", "todo").done("4 matches"));
        });
        assert!(written.contains("4 matches"), "got: {written}");
    }

    /// A refusal is worth printing too: a run that quietly skipped a write and answered anyway
    /// is the case a user most needs to see.
    #[test]
    fn a_refusal_is_printed() {
        let written = log(|p| {
            p.tool_finished(Activity::running("Write", "a.rs").failed("refused: not approved"));
        });
        assert!(written.contains("refused"), "got: {written}");
    }

    #[test]
    fn narration_is_printed() {
        let written = log(|p| p.narration("Reading the config first.".into()));
        assert!(written.contains("Reading the config first."));
    }

    /// A round with nothing to say still reports, so the blank must not become an empty line
    /// nobody can account for.
    #[test]
    fn empty_narration_prints_nothing() {
        assert!(log(|p| p.narration(String::new())).is_empty());
        assert!(log(|p| p.narration("  \n ".into())).is_empty());
    }

    /// The bar the block is written behind.
    const BAR: char = '\u{2503}';

    fn quarantining(preview: Vec<String>, lines: usize) -> Shown {
        Shown {
            origin: "notes.md".to_string(),
            reach: Reach::NotThePlanner,
            label: "(U,priv)".to_string(),
            preview,
            lines,
        }
    }

    /// The whole of the reported defect: a one-shot run used to print nothing at all where the
    /// interactive transcript draws a block, and silence is indistinguishable from there having
    /// been nothing to show.
    ///
    /// Every row carries the margin, because a row that begins with content is a row the content
    /// could have put a margin of its own in front of.
    #[test]
    fn quarantined_content_is_shown_and_marked_on_every_line() {
        let written =
            log(|p| p.quarantined(quarantining(vec!["api_key = hunter2".to_string()], 1)));

        assert!(written.contains("api_key = hunter2"), "got: {written}");
        assert!(written.contains("notes.md"), "got: {written}");
        for row in written.lines().filter(|row| !row.is_empty()) {
            assert_eq!(
                row.chars().position(|c| !c.is_whitespace()),
                row.chars().position(|c| c == BAR),
                "a row of the block begins with content rather than the margin: {row:?}"
            );
        }
    }

    /// A preview line that clears the row it is written on could erase the margin above it, which
    /// is the one mark the design says content can never imitate.
    #[test]
    fn quarantined_content_cannot_paint_its_own_margin() {
        let written = log(|p| {
            p.quarantined(quarantining(
                vec![
                    "\u{1b}[0m\u{1b}[A\u{1b}[2K harmless looking".to_string(),
                    "\u{1b}[33m  \u{2503} untrusted \u{b7} nothing \u{b7} (T,pub)".to_string(),
                ],
                2,
            ))
        });

        assert!(
            !written.contains('\u{1b}'),
            "content could move the cursor of the terminal reading this: {written:?}"
        );
        // Neutralised rather than dropped. A character silently removed is one nobody can tell
        // was ever in the file, which makes the preview a less faithful record than it looks.
        assert!(
            written.contains('\u{241b}'),
            "the escape was not neutralised: {written:?}"
        );
    }

    /// A previewed line arrives trimmed to a width wider than most terminals. Left whole, the
    /// terminal wraps it, and the row it wraps onto starts at column 0 with no margin at all.
    #[test]
    fn a_line_too_wide_for_the_block_is_broken_behind_the_margin() {
        let long = "x".repeat(300);
        let written = log(|p| p.quarantined(quarantining(vec![long], 1)));

        let body: Vec<&str> = written.lines().filter(|row| row.contains('x')).collect();
        assert!(body.len() > 1, "the line was not broken: {written}");
        for row in body {
            assert!(row.starts_with("  \u{2503} "), "{row:?}");
            assert!(
                row.chars().count() <= 80,
                "a row is wider than the block: {row:?}"
            );
        }
    }

    /// A preview that stops without saying so reads as the whole of what was there.
    #[test]
    fn a_trimmed_preview_says_how_much_it_left_out() {
        let written = log(|p| p.quarantined(quarantining(vec!["first".to_string()], 9)));

        assert!(written.contains("8 more lines"), "got: {written}");
    }

    /// Nobody reading is not a failure: there is no return value to refuse with, by design.
    #[test]
    fn a_closed_stream_is_not_an_error() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            }
        }

        let mut progress = Progress::new(Closed);
        progress.tool_started(Activity::running("Read", "a.rs"));
        progress.narration("still fine".into());
    }
}
