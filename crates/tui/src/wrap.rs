//! Wrapping the input so typing past the edge stays visible.
//!
//! A single-line input silently swallows anything past the right edge, cursor included, which
//! looks like the program has stopped accepting keys. So the box wraps and grows downward as
//! text is typed, the way an editor would.
//!
//! Growth is capped. Past the cap the view scrolls to keep the cursor visible, because an input
//! that expanded without limit would eventually push the transcript off screen entirely.
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
}

impl Wrapped {
    /// Rows to show, and the first row of the window.
    ///
    /// Returns the whole thing when it fits, and otherwise the last `visible` rows, so the
    /// cursor stays on screen while typing at the end.
    pub fn window(&self, visible: usize) -> (usize, &[String]) {
        let visible = visible.max(1);
        if self.rows.len() <= visible {
            return (0, &self.rows);
        }
        let first = self.rows.len() - visible;
        (first, &self.rows[first..])
    }
}

/// Break `text` into rows no wider than `width` display columns.
///
/// Breaks at spaces where possible so words are not split mid-way, and hard-breaks a word longer
/// than the whole width, since there is nowhere better to put it. Newlines in the text start a
/// new row.
///
/// The cursor is reported as sitting after the last character, which is where the next keystroke
/// lands: this input is only ever edited at its end.
pub fn wrap(text: &str, width: usize) -> Wrapped {
    let width = width.max(1);
    let mut rows: Vec<String> = Vec::new();
    let mut row = String::new();
    let mut row_width = 0usize;
    // Where the current row could be split, and how wide it was at that point.
    let mut last_space: Option<(usize, usize)> = None;

    for c in text.chars() {
        if c == '\n' {
            rows.push(std::mem::take(&mut row));
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
                    row.truncate(byte_index);
                    rows.push(std::mem::take(&mut row));
                    row = carried.trim_start().to_string();
                    row_width = display_width(&row);
                }
                // A word wider than the row, or no space at all: hard-break.
                _ => {
                    rows.push(std::mem::take(&mut row));
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

    let cursor_col = row_width;
    rows.push(row);

    Wrapped {
        cursor_row: rows.len() - 1,
        cursor_col,
        rows,
    }
}

/// Display columns a string occupies.
pub fn display_width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_stays_on_one_row() {
        let w = wrap("hello", 20);
        assert_eq!(w.rows, vec!["hello"]);
        assert_eq!(w.cursor_row, 0);
        assert_eq!(w.cursor_col, 5);
    }

    /// An empty input still needs a row, or there is nowhere to draw the cursor.
    #[test]
    fn empty_text_has_one_empty_row() {
        let w = wrap("", 20);
        assert_eq!(w.rows, vec![""]);
        assert_eq!(w.cursor_row, 0);
        assert_eq!(w.cursor_col, 0);
    }

    /// The bug this exists to fix: text past the edge must appear on the next row rather than
    /// vanishing.
    #[test]
    fn text_past_the_edge_continues_on_the_next_row() {
        let w = wrap("aaa bbb ccc ddd", 8);
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
            for row in wrap(text, width).rows {
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
        let w = wrap("hello world", 8);
        assert_eq!(w.rows[0].trim_end(), "hello");
        assert_eq!(w.rows[1], "world");
    }

    /// A word with nowhere to break must still be broken, or it would exceed the width.
    #[test]
    fn an_overlong_word_is_hard_broken() {
        let w = wrap("abcdefghijkl", 5);
        assert!(w.rows.len() >= 3, "{:?}", w.rows);
        for row in &w.rows {
            assert!(display_width(row) <= 5);
        }
        assert_eq!(w.rows.concat(), "abcdefghijkl", "characters were lost");
    }

    #[test]
    fn newlines_start_a_new_row() {
        let w = wrap("one\ntwo", 20);
        assert_eq!(w.rows, vec!["one", "two"]);
        assert_eq!(w.cursor_row, 1);
        assert_eq!(w.cursor_col, 3);
    }

    /// The cursor must follow the text, or it is drawn somewhere the next keystroke will not
    /// appear.
    #[test]
    fn the_cursor_lands_after_the_last_character() {
        let w = wrap("aaa bbb", 4);
        assert_eq!(w.cursor_row, w.rows.len() - 1);
        assert_eq!(w.cursor_col, display_width(w.rows.last().unwrap()));
    }

    /// Wide characters occupy two columns, so measuring in characters would wrap late and clip.
    #[test]
    fn wide_characters_are_measured_in_columns() {
        assert_eq!(display_width("日本語"), 6);
        for row in wrap("日本語テキスト", 4).rows {
            assert!(display_width(&row) <= 4, "row {row:?} is too wide");
        }
    }

    #[test]
    fn a_zero_width_does_not_hang_or_panic() {
        let w = wrap("abc", 0);
        assert!(!w.rows.is_empty());
    }

    /// While it fits, the whole input is shown from the top.
    #[test]
    fn a_short_input_shows_every_row() {
        let w = wrap("one two three", 6);
        let (first, rows) = w.window(MAX_ROWS);
        assert_eq!(first, 0);
        assert_eq!(rows.len(), w.rows.len());
    }

    /// Past the cap the view follows the cursor, since typing happens at the end.
    #[test]
    fn a_long_input_scrolls_to_keep_the_cursor_visible() {
        let text = "word ".repeat(200);
        let w = wrap(&text, 10);
        assert!(w.rows.len() > 3);

        let (first, rows) = w.window(3);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            first + rows.len() - 1,
            w.cursor_row,
            "the cursor's row is outside the window"
        );
    }

    /// A window of zero would show nothing at all, so it is treated as one row.
    #[test]
    fn a_zero_window_still_shows_a_row() {
        let w = wrap("hello", 20);
        let (_, rows) = w.window(0);
        assert_eq!(rows.len(), 1);
    }
}
