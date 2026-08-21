//! Telling the interface what a turn is doing, while it does it.
//!
//! Distinct from [`crate::confirm`], and the difference is consent. A write asks, blocks, and
//! must refuse if nobody can answer. Progress announces: there is no question, no reply, and no
//! answer that could change what happens. So this returns nothing, and a listener that has gone
//! away is not an error.
//!
//! That asymmetry decides the failure behaviour. A closed channel means a write must not happen,
//! but a task list nobody is drawing is merely unseen, and failing the turn over it would let the
//! display outrank the work.

use bua_core::todo::Row;

/// Something that can be told about progress.
///
/// A trait so a turn does not depend on a terminal: the interactive session draws, a one-shot run
/// ignores, and tests record.
pub trait Reporter {
    /// The task list changed. Rows are already shaped for display and released.
    fn todos(&mut self, rows: Vec<Row>);
}

/// Discards every report.
///
/// The right behaviour where there is no live display: a one-shot command, a pipeline. Unlike
/// refusing a write, discarding a progress report costs nothing, since it was never going to
/// change what the turn did.
#[derive(Debug, Default)]
pub struct IgnoreReports;

impl Reporter for IgnoreReports {
    fn todos(&mut self, _rows: Vec<Row>) {}
}

/// Keeps what it was told, for tests.
#[derive(Debug, Default)]
pub struct RecordingReporter {
    /// Every update in order, so a test can assert on the sequence rather than the end state.
    pub updates: Vec<Vec<Row>>,
}

impl Reporter for RecordingReporter {
    fn todos(&mut self, rows: Vec<Row>) {
        self.updates.push(rows);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bua_core::todo::{Item, List, Status, rows};

    #[test]
    fn a_recording_reporter_keeps_each_update_in_order() {
        let mut reporter = RecordingReporter::default();
        reporter.todos(rows(&List::new(vec![Item::new("one", Status::Pending)])));
        reporter.todos(rows(&List::new(vec![Item::new("one", Status::Done)])));

        assert_eq!(reporter.updates.len(), 2);
        assert!(!reporter.updates[0][0].struck());
        assert!(reporter.updates[1][0].struck());
    }

    /// Nothing to draw is not a failure. A reporter has no way to refuse, by design: there is no
    /// return value it could refuse with.
    #[test]
    fn ignoring_reports_is_infallible() {
        IgnoreReports.todos(rows(&List::new(vec![Item::new("x", Status::Active)])));
    }
}
