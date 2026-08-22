//! Progress for a run with no interface to draw on.
//!
//! A one-shot run has no spinner and no redraw, so without this it prints nothing at all until
//! the reply arrives. Turns take as many rounds as the work needs, and one that reads a dozen
//! files and edits three of them is a long silence to sit through.
//!
//! Everything here goes to stderr. The reply is the command's output and belongs on stdout,
//! where it can be piped into something else; a progress log that shared that stream would
//! corrupt it.

use bua_agent::report::{Activity, Reporter};
use bua_core::todo::Row;
use std::io::Write;

/// Marks a call, matching the interactive transcript so the two read alike.
const CALL_MARKER: &str = "\u{23fa}";
/// Marks the detail belonging to the line above it.
const DETAIL_MARKER: &str = "\u{23bf}";

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
}

#[cfg(test)]
mod tests {
    use super::*;

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
