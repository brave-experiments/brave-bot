//! The planner's task list.
//!
//! A turn of any length is opaque from outside: the user sees a spinner and has no idea whether
//! the model is on the first of five steps or the last. A list the model maintains as it works
//! answers that, and answers it in the model's own terms rather than the driver's guess.
//!
//! The list is model output, so it carries the integrity of the context the model was working
//! from. That makes it content, never routing: nothing here decides anything. It lands nowhere,
//! has no destination to endorse, and is read by exactly two things, the planner that wrote it
//! and the human watching. Both are gated already, by [`crate::policy::Policy::present`] and
//! [`crate::policy::Policy::render_in_place`] respectively.
//!
//! Rendering lives here rather than in the interface because shaping the list means looking at
//! the statuses in it, and the driver may not hold untrusted bytes to look at them. The kernel
//! can, inside a render closure, because choosing a glyph decides nothing an attacker could
//! steer: every status renders as *something*, and no item is dropped or reordered on the
//! strength of its text.

use std::fmt;

/// How far along one task is.
///
/// A closed set rather than free text, so an unrecognised status cannot mean "not done" in the
/// interface and "done" in the model's head. Anything the model writes that is not one of these
/// parses as [`Status::Pending`]: an item nobody can classify is work outstanding, which is the
/// reading that cannot quietly hide something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Not started.
    Pending,
    /// Being worked on now.
    Active,
    /// Finished.
    Done,
}

impl Status {
    /// Read a status the model supplied.
    ///
    /// Deliberately total: there is no error case, because a rejected status would be a decision
    /// made from content. Unknown text is outstanding work.
    pub fn parse(text: &str) -> Self {
        match text.trim() {
            "in_progress" | "active" => Self::Active,
            "completed" | "done" => Self::Done,
            _ => Self::Pending,
        }
    }

    /// The values the model is told to use.
    pub const NAMES: [&'static str; 3] = ["pending", "in_progress", "completed"];
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Active => write!(f, "in_progress"),
            Self::Done => write!(f, "completed"),
        }
    }
}

/// One task on the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub content: String,
    pub status: Status,
}

impl Item {
    pub fn new(content: impl Into<String>, status: Status) -> Self {
        Self {
            content: content.into(),
            status,
        }
    }
}

/// The whole list, as the model last wrote it.
///
/// Replaced wholesale on every update, never mutated item by item. Amending one entry would mean
/// the driver finding it first, and the identifier it searched by would be model-authored text:
/// a comparison on untrusted content, which is the one thing the driver may not do. Whole-list
/// replacement compares nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct List {
    pub items: Vec<Item>,
}

impl List {
    pub fn new(items: Vec<Item>) -> Self {
        Self { items }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// How many tasks are finished.
    pub fn done(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.status == Status::Done)
            .count()
    }
}

/// One line of the list, shaped for a screen.
///
/// Carries the styling decision as data rather than as a formatted string, so the interface can
/// draw a strikethrough or a colour without parsing anything back out of the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The task, as the model wrote it.
    pub content: String,
    /// What the interface should draw in the margin.
    pub marker: &'static str,
    pub status: Status,
}

impl Row {
    /// Whether this line should read as struck through.
    pub fn struck(&self) -> bool {
        self.status == Status::Done
    }
}

/// Marker for a finished task.
const DONE_MARKER: &str = "✓";
/// Marker for anything still outstanding, started or not.
const PENDING_MARKER: &str = "■";

/// Shape a list into rows for display.
///
/// Meant to be called inside [`crate::policy::Policy::render_in_place`], which is what lets the
/// content be looked at at all. Every item produces exactly one row: nothing is filtered,
/// reordered, or truncated, so the statuses in the list cannot change what the user is shown the
/// existence of, only how it looks.
pub fn rows(list: &List) -> Vec<Row> {
    list.items
        .iter()
        .map(|item| Row {
            content: item.content.clone(),
            marker: match item.status {
                Status::Done => DONE_MARKER,
                Status::Pending | Status::Active => PENDING_MARKER,
            },
            status: item.status,
        })
        .collect()
}

/// The task in progress, for labelling the working indicator.
///
/// Presentation, like [`rows`]: the word beside a spinner is not a decision, and the fallback
/// when nothing is marked active is the generic word the indicator would have used anyway.
pub fn active_label(list: &List) -> Option<String> {
    list.items
        .iter()
        .find(|i| i.status == Status::Active)
        .map(|i| i.content.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The statuses the model is told about are the ones that parse, or the instruction would be
    /// describing a vocabulary the code does not read.
    #[test]
    fn every_advertised_status_parses_to_itself() {
        for name in Status::NAMES {
            assert_eq!(
                Status::parse(name).to_string(),
                name,
                "'{name}' is advertised but does not round-trip"
            );
        }
    }

    /// An unrecognised status must count as outstanding. Treating it as done would let a typo,
    /// or a model that invented a word, silently mark work finished.
    #[test]
    fn an_unknown_status_is_outstanding_work() {
        for text in ["", "cancelled", "COMPLETED", "nearly", "✓"] {
            assert_eq!(
                Status::parse(text),
                Status::Pending,
                "'{text}' was not treated as outstanding"
            );
        }
    }

    #[test]
    fn familiar_spellings_are_accepted() {
        assert_eq!(Status::parse("done"), Status::Done);
        assert_eq!(Status::parse("active"), Status::Active);
        // Whitespace is the model's formatting, not a different status.
        assert_eq!(Status::parse("  completed "), Status::Done);
    }

    fn list() -> List {
        List::new(vec![
            Item::new("Escape cancels an in-flight request", Status::Done),
            Item::new("Add prompt history", Status::Active),
            Item::new("Persist it across sessions", Status::Pending),
        ])
    }

    /// Nothing is filtered or reordered, so no status can hide an item from the user.
    #[test]
    fn every_item_produces_exactly_one_row_in_order() {
        let list = list();
        let rows = rows(&list);
        assert_eq!(rows.len(), list.len());
        for (row, item) in rows.iter().zip(&list.items) {
            assert_eq!(row.content, item.content);
        }
    }

    #[test]
    fn a_finished_task_is_marked_and_struck() {
        let rows = rows(&list());
        assert_eq!(rows[0].marker, DONE_MARKER);
        assert!(rows[0].struck());
    }

    /// Started and unstarted both read as outstanding: the distinction is carried by the
    /// indicator's word, not by a third glyph nobody would decode.
    #[test]
    fn outstanding_tasks_are_not_struck_whether_started_or_not() {
        let rows = rows(&list());
        for row in &rows[1..] {
            assert_eq!(row.marker, PENDING_MARKER);
            assert!(!row.struck());
        }
    }

    #[test]
    fn an_empty_list_renders_no_rows() {
        assert!(rows(&List::default()).is_empty());
    }

    #[test]
    fn the_active_task_names_the_indicator() {
        assert_eq!(active_label(&list()).as_deref(), Some("Add prompt history"));
    }

    /// With nothing active there is no word to borrow, and the indicator keeps its own.
    #[test]
    fn a_list_with_nothing_active_has_no_label() {
        let list = List::new(vec![Item::new("only task", Status::Done)]);
        assert!(active_label(&list).is_none());
    }

    /// The first active item wins rather than the last, so a model that leaves two marked does
    /// not make the word jump about between redraws.
    #[test]
    fn the_first_active_task_wins_when_the_model_marks_several() {
        let list = List::new(vec![
            Item::new("first", Status::Active),
            Item::new("second", Status::Active),
        ]);
        assert_eq!(active_label(&list).as_deref(), Some("first"));
    }

    #[test]
    fn progress_is_counted() {
        assert_eq!(list().done(), 1);
        assert_eq!(list().len(), 3);
    }
}
