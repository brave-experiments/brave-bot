//! Selecting text off the screen with the mouse.
//!
//! Capturing the mouse is what makes the wheel scroll the transcript, and it costs the terminal
//! its own selection: a drag arrives here instead of highlighting anything, so a user who tries
//! to copy a line gets nothing at all. Since the drag arrives here, it is answered here.
//!
//! The selection is over screen cells rather than over the text behind them. That is not a
//! shortcut: what the user swept over is what they saw, wrapped and scrolled exactly as it was
//! drawn, and the cells hold precisely that. Reading it back out of the buffer needs no second
//! copy of the layout to be kept in step with the first.
//!
//! Nothing labelled is involved. Everything on the screen was released for display before it was
//! drawn, and this reads the screen.

use ratatui::buffer::Buffer;
use ratatui::style::Color;

/// A sweep across the screen, from where the button went down to where the pointer is now.
///
/// Held as row and column, in that order, so comparing two positions gives reading order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    anchor: (u16, u16),
    head: (u16, u16),
}

impl Selection {
    /// Begin a selection where the button went down.
    pub fn started_at(row: u16, column: u16) -> Self {
        Self {
            anchor: (row, column),
            head: (row, column),
        }
    }

    /// Move the loose end to follow the pointer.
    pub fn extend_to(&mut self, row: u16, column: u16) {
        self.head = (row, column);
    }

    /// Nothing was swept over, which is what a plain click is.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// The two ends in reading order, so a selection dragged upwards reads like one dragged
    /// down.
    fn ends(&self) -> ((u16, u16), (u16, u16)) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// Whether this cell is inside the selection.
    ///
    /// The far end is excluded, the way a text cursor sits between two characters rather than on
    /// one: otherwise a click with no drag at all would select something.
    pub fn covers(&self, row: u16, column: u16) -> bool {
        let (start, end) = self.ends();
        (row, column) >= start && (row, column) < end
    }
}

/// The text the selection covers, read off the screen as it was last drawn.
///
/// Rows are joined with newlines and their trailing blanks dropped, because those blanks are a
/// widget padding its area rather than anything a person would want pasted.
pub fn text(buffer: &Buffer, selection: &Selection) -> String {
    let area = buffer.area;
    let (start, end) = selection.ends();

    let mut out = String::new();
    for row in start.0..=end.0.min(area.bottom().saturating_sub(1)) {
        let from = if row == start.0 { start.1 } else { area.left() };
        let to = if row == end.0 { end.1 } else { area.right() };

        let mut line = String::new();
        for column in from..to.min(area.right()) {
            if let Some(cell) = buffer.cell((column, row)) {
                line.push_str(cell.symbol());
            }
        }

        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line.trim_end());
    }
    out
}

/// Paint the selection over what has already been drawn.
///
/// Only the background changes, so the text keeps whatever colour it was given and a highlighted
/// line still reads as the line it was.
pub fn highlight(buffer: &mut Buffer, selection: &Selection) {
    let area = buffer.area;
    for row in area.top()..area.bottom() {
        for column in area.left()..area.right() {
            if selection.covers(row, column)
                && let Some(cell) = buffer.cell_mut((column, row))
            {
                cell.set_bg(Color::Blue);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::text::Line;
    use ratatui::widgets::{Paragraph, Widget};

    fn screen(lines: &[&str]) -> Buffer {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, lines.len() as u16));
        let rows: Vec<Line> = lines.iter().map(|l| Line::raw(l.to_string())).collect();
        Paragraph::new(rows).render(buffer.area, &mut buffer);
        buffer
    }

    #[test]
    fn a_click_with_no_drag_selects_nothing() {
        let selection = Selection::started_at(1, 4);
        assert!(selection.is_empty());
        assert!(!selection.covers(1, 4));
    }

    #[test]
    fn a_sweep_across_one_row_takes_what_it_covered() {
        let buffer = screen(&["hello there", "second line"]);
        let mut selection = Selection::started_at(0, 6);
        selection.extend_to(0, 11);
        assert_eq!(text(&buffer, &selection), "there");
    }

    /// Dragging back to the start is the same selection as dragging forward to the end, because
    /// a user who overshoots and comes back should not get a different answer.
    #[test]
    fn a_sweep_reads_the_same_in_either_direction() {
        let buffer = screen(&["hello there"]);
        let mut forward = Selection::started_at(0, 0);
        forward.extend_to(0, 5);
        let mut backward = Selection::started_at(0, 5);
        backward.extend_to(0, 0);
        assert_eq!(text(&buffer, &forward), text(&buffer, &backward));
    }

    /// Across rows the ends are partial and everything between them is whole, which is what a
    /// text selection means and not what a rectangle would give.
    #[test]
    fn a_sweep_across_rows_takes_whole_rows_in_between() {
        let buffer = screen(&["first line", "middle row", "last line"]);
        let mut selection = Selection::started_at(0, 6);
        selection.extend_to(2, 4);
        assert_eq!(text(&buffer, &selection), "line\nmiddle row\nlast");
    }

    /// The blanks a widget pads its area with are not text anyone wrote, and pasting them turns
    /// a copied paragraph into a block of trailing whitespace.
    #[test]
    fn the_padding_a_widget_drew_is_not_copied() {
        let buffer = screen(&["short", "also short"]);
        let mut selection = Selection::started_at(0, 0);
        selection.extend_to(1, 20);
        assert_eq!(text(&buffer, &selection), "short\nalso short");
    }

    #[test]
    fn a_highlight_covers_the_selection_and_nothing_else() {
        let mut buffer = screen(&["hello there"]);
        let mut selection = Selection::started_at(0, 6);
        selection.extend_to(0, 11);
        highlight(&mut buffer, &selection);

        assert_eq!(buffer.cell((6, 0)).expect("cell").bg, Color::Blue);
        assert_ne!(buffer.cell((5, 0)).expect("cell").bg, Color::Blue);
        assert_ne!(buffer.cell((11, 0)).expect("cell").bg, Color::Blue);
    }
}
