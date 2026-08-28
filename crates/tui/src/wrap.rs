//! Wrapping the input so typing past the edge stays visible.
//!
//! A single-line input silently swallows anything past the right edge, cursor included, which
//! looks like the program has stopped accepting keys. So the box wraps and grows downward as
//! text is typed, the way an editor would.
//!
//! Growth is capped. Past the cap the view scrolls to keep the cursor visible, wherever in the
//! text it is, because an input that expanded without limit would eventually push the transcript
//! off screen entirely.
//!
//! Width is measured in display columns rather than characters, so a line of CJK or emoji wraps
//! where it actually reaches the edge rather than twice as late.

use unicode_width::UnicodeWidthChar;

/// Rows the input may occupy before it scrolls instead of growing.
///
/// Enough for a substantial paragraph while leaving the transcript the majority of a standard
/// terminal.
pub const MAX_ROWS: usize = 10;

/// Text broken into display rows, with the cursor located.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrapped {
    /// The rows, in order. Always at least one, so an empty input still shows a cursor.
    pub rows: Vec<String>,
    /// Which row the cursor sits on.
    pub cursor_row: usize,
    /// The cursor's column within that row, in display columns.
    pub cursor_col: usize,
    /// The cursor's byte offset within that row.
    ///
    /// Both this and the column, because drawing the cursor needs to split the row where it is and
    /// measuring where it lands on screen needs the width of what precedes it.
    pub cursor_index: usize,
}

impl Wrapped {
    /// Rows to show, and the first row of the window.
    ///
    /// Returns the whole thing when it fits, and otherwise a window holding the cursor's row, so
    /// the row being edited stays on screen wherever in the text it is.
    pub fn window(&self, visible: usize) -> (usize, &[String]) {
        let visible = visible.max(1);
        if self.rows.len() <= visible {
            return (0, &self.rows);
        }
        // At the bottom of the window where it can be, since that is where the text continues.
        let first = self
            .cursor_row
            .saturating_sub(visible - 1)
            .min(self.rows.len() - visible);
        (first, &self.rows[first..first + visible])
    }
}

/// Break `text` into rows no wider than `width` display columns, with the cursor at `caret`.
///
/// Breaks at spaces where possible so words are not split mid-way, and hard-breaks a word longer
/// than the whole width, since there is nowhere better to put it. Newlines in the text start a
/// new row.
///
/// `caret` is a byte offset into `text`, and is clamped rather than trusted, because a slice taken
/// past the end or inside a character would panic and the box would rather draw the cursor a
/// character out than take the interface down.
pub fn wrap(text: &str, width: usize, caret: usize) -> Wrapped {
    let width = width.max(1);
    let mut rows: Vec<String> = Vec::new();
    // Where each row starts in `text`, which is what locates the caret once the rows are built.
    // Recorded as the rows are pushed because a space break moves a partial word between them.
    let mut starts: Vec<usize> = Vec::new();
    let mut row = String::new();
    let mut row_start = 0usize;
    let mut row_width = 0usize;
    // Where the current row could be split, and how wide it was at that point.
    let mut last_space: Option<(usize, usize)> = None;

    for (index, c) in text.char_indices() {
        if c == '\n' {
            rows.push(std::mem::take(&mut row));
            starts.push(row_start);
            row_start = index + c.len_utf8();
            row_width = 0;
            last_space = None;
            continue;
        }

        // Control characters have no width and would corrupt the column count.
        let c_width = c.width().unwrap_or(0);

        if row_width + c_width > width {
            match last_space {
                // Break after the last space, carrying the partial word to the next row.
                Some((byte_index, _)) if byte_index < row.len() => {
                    let carried: String = row[byte_index..].to_string();
                    let carried_start = row_start + byte_index;
                    row.truncate(byte_index);
                    rows.push(std::mem::take(&mut row));
                    starts.push(row_start);
                    row = carried.trim_start().to_string();
                    row_start = carried_start + (carried.len() - row.len());
                    row_width = display_width(&row);
                }
                // A word wider than the row, or no space at all: hard-break.
                _ => {
                    rows.push(std::mem::take(&mut row));
                    starts.push(row_start);
                    row_start = index;
                    row_width = 0;
                }
            }
            last_space = None;
        }

        if c == ' ' {
            // Recorded after pushing, so the space stays on the row it ends.
            row.push(c);
            row_width += c_width;
            last_space = Some((row.len(), row_width));
            continue;
        }

        row.push(c);
        row_width += c_width;
    }

    rows.push(row);
    starts.push(row_start);

    let caret = boundary_at_or_before(text, caret);
    // The last row that begins at or before the caret. A caret inside the spaces a break trimmed
    // away belongs to the row that ended there, which is where it was typed.
    let cursor_row = starts.partition_point(|start| *start <= caret).max(1) - 1;
    let within = caret
        .saturating_sub(starts[cursor_row])
        .min(rows[cursor_row].len());

    Wrapped {
        cursor_col: display_width(&rows[cursor_row][..within]),
        cursor_index: within,
        cursor_row,
        rows,
    }
}

/// The largest character boundary of `text` no greater than `offset`.
fn boundary_at_or_before(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Display columns a string occupies.
pub fn display_width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrapped with the caret where typing leaves it, which is what most of these are about.
    fn at_end(text: &str, width: usize) -> Wrapped {
        wrap(text, width, text.len())
    }

    #[test]
    fn short_text_stays_on_one_row() {
        let w = at_end("hello", 20);
        assert_eq!(w.rows, vec!["hello"]);
        assert_eq!(w.cursor_row, 0);
        assert_eq!(w.cursor_col, 5);
    }

    /// An empty input still needs a row, or there is nowhere to draw the cursor.
    #[test]
    fn empty_text_has_one_empty_row() {
        let w = at_end("", 20);
        assert_eq!(w.rows, vec![""]);
        assert_eq!(w.cursor_row, 0);
        assert_eq!(w.cursor_col, 0);
    }

    /// The bug this exists to fix: text past the edge must appear on the next row rather than
    /// vanishing.
    #[test]
    fn text_past_the_edge_continues_on_the_next_row() {
        let w = at_end("aaa bbb ccc ddd", 8);
        assert!(w.rows.len() > 1, "did not wrap: {:?}", w.rows);
        // Every character survives somewhere.
        let rejoined: String = w.rows.join("");
        assert!(rejoined.contains("ddd"), "the tail was lost: {:?}", w.rows);
    }

    /// No row may exceed the width, or the terminal clips it and the wrap was pointless.
    #[test]
    fn no_row_exceeds_the_width() {
        let text = "the quick brown fox jumps over the lazy dog and keeps on running";
        for width in [4, 7, 10, 13, 20, 40] {
            for row in at_end(text, width).rows {
                assert!(
                    display_width(&row) <= width,
                    "row {row:?} is wider than {width}"
                );
            }
        }
    }

    /// Breaking mid-word makes text hard to read, so a space is preferred.
    #[test]
    fn wrapping_prefers_a_space() {
        let w = at_end("hello world", 8);
        assert_eq!(w.rows[0].trim_end(), "hello");
        assert_eq!(w.rows[1], "world");
    }

    /// A word with nowhere to break must still be broken, or it would exceed the width.
    #[test]
    fn an_overlong_word_is_hard_broken() {
        let w = at_end("abcdefghijkl", 5);
        assert!(w.rows.len() >= 3, "{:?}", w.rows);
        for row in &w.rows {
            assert!(display_width(row) <= 5);
        }
        assert_eq!(w.rows.concat(), "abcdefghijkl", "characters were lost");
    }

    #[test]
    fn newlines_start_a_new_row() {
        let w = at_end("one\ntwo", 20);
        assert_eq!(w.rows, vec!["one", "two"]);
        assert_eq!(w.cursor_row, 1);
        assert_eq!(w.cursor_col, 3);
    }

    /// The cursor must follow the text, or it is drawn somewhere the next keystroke will not
    /// appear.
    #[test]
    fn the_cursor_lands_after_the_last_character() {
        let w = at_end("aaa bbb", 4);
        assert_eq!(w.cursor_row, w.rows.len() - 1);
        assert_eq!(w.cursor_col, display_width(w.rows.last().unwrap()));
    }

    /// The cursor is drawn where the caret is, not at the end: a caret moved back into the text
    /// is the whole point of being able to move it.
    #[test]
    fn the_cursor_follows_a_caret_inside_the_text() {
        let w = wrap("hello", 20, 2);
        assert_eq!(w.cursor_row, 0);
        assert_eq!(w.cursor_col, 2);
    }

    /// A caret on a later row must be reported on that row, or moving up would appear to do
    /// nothing while the keystrokes landed somewhere else.
    #[test]
    fn a_caret_on_a_wrapped_row_is_reported_there() {
        // "aaa " ends the first row, so byte 5 is one character into the second.
        let w = wrap("aaa bbb", 4, 5);
        assert_eq!(w.rows.len(), 2);
        assert_eq!(w.cursor_row, 1);
        assert_eq!(w.cursor_col, 1);
    }

    /// The caret after a newline belongs to the row the newline started, not the one it ended.
    #[test]
    fn a_caret_after_a_newline_is_on_the_new_row() {
        let w = wrap("one\ntwo", 20, 4);
        assert_eq!(w.cursor_row, 1);
        assert_eq!(w.cursor_col, 0);
    }

    /// Columns are display columns, so a caret past a wide character is drawn where that
    /// character actually ends.
    #[test]
    fn a_caret_past_a_wide_character_is_measured_in_columns() {
        let w = wrap("日本", 20, "日".len());
        assert_eq!(w.cursor_col, 2);
    }

    /// Slicing past the end or inside a character would panic, and drawing the cursor a character
    /// out is better than taking the interface down.
    #[test]
    fn a_caret_off_the_end_or_inside_a_character_does_not_panic() {
        assert_eq!(wrap("abc", 20, 99).cursor_col, 3);
        assert_eq!(wrap("日本", 20, 1).cursor_col, 0);
    }

    /// Wide characters occupy two columns, so measuring in characters would wrap late and clip.
    #[test]
    fn wide_characters_are_measured_in_columns() {
        assert_eq!(display_width("日本語"), 6);
        for row in at_end("日本語テキスト", 4).rows {
            assert!(display_width(&row) <= 4, "row {row:?} is too wide");
        }
    }

    #[test]
    fn a_zero_width_does_not_hang_or_panic() {
        let w = at_end("abc", 0);
        assert!(!w.rows.is_empty());
    }

    /// While it fits, the whole input is shown from the top.
    #[test]
    fn a_short_input_shows_every_row() {
        let w = at_end("one two three", 6);
        let (first, rows) = w.window(MAX_ROWS);
        assert_eq!(first, 0);
        assert_eq!(rows.len(), w.rows.len());
    }

    /// Past the cap the view follows the cursor, since typing happens at the end.
    #[test]
    fn a_long_input_scrolls_to_keep_the_cursor_visible() {
        let text = "word ".repeat(200);
        let w = at_end(&text, 10);
        assert!(w.rows.len() > 3);

        let (first, rows) = w.window(3);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            first + rows.len() - 1,
            w.cursor_row,
            "the cursor's row is outside the window"
        );
    }

    /// The window follows the caret upwards too, or a user moving back through a long prompt
    /// would edit a row they cannot see.
    #[test]
    fn the_window_follows_a_caret_back_up_the_text() {
        let text = "word ".repeat(200);
        let w = wrap(&text, 10, 0);
        let (first, rows) = w.window(3);
        assert_eq!(first, 0, "the window stayed at the end: {rows:?}");
    }

    /// A window of zero would show nothing at all, so it is treated as one row.
    #[test]
    fn a_zero_window_still_shows_a_row() {
        let w = at_end("hello", 20);
        let (_, rows) = w.window(0);
        assert_eq!(rows.len(), 1);
    }
}
