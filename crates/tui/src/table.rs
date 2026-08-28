//! GitHub-flavoured tables for the transcript.
//!
//! A model asked anything comparative answers with a table, and a table shown as its source is
//! the one markdown form that reads worse than plain prose: pipes and dashes in columns that do
//! not line up, because the cells are not as wide as their headings.
//!
//! Unlike inline styling this makes a layout decision, which is why it is not in
//! [`crate::markdown`]. The decision is never to widen the transcript: columns are sized to the
//! room left after the transcript's own lead, cells too long for their column are folded onto
//! further rows, and a table that cannot be given a legible column at all is refused, so the
//! source lines are drawn exactly as they were before this module existed. Refusal is always
//! available and always the safe answer: the text arrived through a turn, and the worst outcome
//! for input built to defeat the parser is prose that still says what it said.
//!
//! Nothing here decides anything in the sense the kernel means. The input is assistant output
//! already released for the screen, and the only output is styled spans.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthChar;

use crate::markdown;
use crate::wrap::display_width;

/// Columns a table may declare before it is drawn as its source instead.
///
/// Not a limit on what markdown can express, but the point past which a table stops being
/// readable in a terminal: twenty-four columns leaves under three characters each on a standard
/// width. The cap also bounds the layout, which would otherwise size itself from a header row
/// built to have a hundred thousand cells.
pub const MAX_COLUMNS: usize = 24;

/// Columns a cell needs before its column is worth drawing.
///
/// Below this a column is too narrow to say that a word began, and the table would be a rule
/// with rubble under it: a worse answer than the pipes and dashes the model wrote.
const MIN_CELL_WIDTH: usize = 4;

/// Columns between one cell and the next.
const GAP: usize = 2;

/// The rule under the header, dim for the reason the transcript has no frame: the table is what
/// is being read, and the rule is only there to say where the columns are.
const RULE: char = '\u{2500}';

/// Which way a column's cells sit in their column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Centre,
    Right,
}

/// A table read out of the source, before any decision about how wide it may be.
#[derive(Debug, PartialEq, Eq)]
pub struct Parsed {
    pub header: Vec<String>,
    pub aligns: Vec<Align>,
    pub body: Vec<Vec<String>>,
    /// Source lines the table occupies, so the caller knows where prose resumes.
    pub consumed: usize,
}

/// Split one line into cells, or `None` if it is not a row at all.
///
/// A row must contain a pipe that is neither escaped nor inside inline code, which is what keeps
/// a sentence containing a vertical bar out of the parser. Leading and trailing pipes are
/// optional, `\|` is a pipe in the cell, and a pipe inside backticks belongs to the code.
pub fn cells(line: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut cell = String::new();
    // Length of the backtick run that opened the current code span, zero outside one.
    let mut fence = 0usize;
    let mut escaped = false;
    let mut split = false;

    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if escaped {
            // Only a pipe or a backslash is escapable, so a Windows path keeps its backslashes.
            if c != '|' && c != '\\' {
                cell.push('\\');
            }
            cell.push(c);
            escaped = false;
            continue;
        }

        match c {
            '\\' => escaped = true,
            '`' => {
                let mut run = 1;
                while chars.peek() == Some(&'`') {
                    chars.next();
                    run += 1;
                }
                if fence == 0 {
                    fence = run;
                } else if fence == run {
                    fence = 0;
                }
                for _ in 0..run {
                    cell.push('`');
                }
            }
            '|' if fence == 0 => {
                split = true;
                out.push(cell.trim().to_string());
                cell = String::new();
            }
            _ => cell.push(c),
        }
    }

    if escaped {
        cell.push('\\');
    }
    out.push(cell.trim().to_string());

    if !split {
        return None;
    }

    // An edge pipe writes an empty cell that was never meant to be one.
    if out.first().is_some_and(String::is_empty) {
        out.remove(0);
    }
    if out.last().is_some_and(String::is_empty) {
        out.pop();
    }

    (!out.is_empty()).then_some(out)
}

/// The alignment one delimiter cell declares, if that is what it is.
fn alignment(cell: &str) -> Option<Align> {
    let left = cell.starts_with(':');
    let rest = if left { &cell[1..] } else { cell };
    let right = rest.ends_with(':');
    let dashes = if right { &rest[..rest.len() - 1] } else { rest };

    if dashes.is_empty() || !dashes.chars().all(|c| c == '-') {
        return None;
    }

    Some(match (left, right) {
        (true, true) => Align::Centre,
        (false, true) => Align::Right,
        _ => Align::Left,
    })
}

/// Read a table starting at `lines[0]`, if one starts there.
pub fn parse(lines: &[&str]) -> Option<Parsed> {
    let header = cells(lines.first()?)?;
    if header.len() > MAX_COLUMNS {
        return None;
    }

    let delimiter = cells(lines.get(1)?)?;
    if delimiter.len() != header.len() {
        return None;
    }
    let aligns: Vec<Align> = delimiter
        .iter()
        .map(|cell| alignment(cell))
        .collect::<Option<_>>()?;

    let mut body = Vec::new();
    let mut at = 2;
    while let Some(line) = lines.get(at) {
        let Some(mut row) = cells(line) else { break };
        // What GFM does with a ragged row. A model that forgets a pipe on row nine should not
        // cost the reader rows one to eight.
        row.resize(header.len(), String::new());
        body.push(row);
        at += 1;
    }

    Some(Parsed {
        header,
        aligns,
        body,
        consumed: at,
    })
}

/// A laid-out table: the rows to draw, and how many source lines they stand in for.
#[derive(Debug)]
pub struct Laid {
    pub rows: Vec<Vec<Span<'static>>>,
    pub consumed: usize,
}

/// One cell, already styled, with the width it will occupy on screen.
struct Cell {
    spans: Vec<Span<'static>>,
    width: usize,
}

/// Style a cell once, so the width the layout budgets for is the width that gets drawn.
///
/// Measuring the source instead would be wrong by the length of every marker: `**x**` is six
/// columns in the file and one on screen, and a column budgeted from the file leaves the rule
/// five columns adrift on that row alone.
fn cell(text: &str, style: Style) -> Cell {
    let spans = markdown::spans(text, style);
    let width = spans.iter().map(|span| display_width(&span.content)).sum();
    Cell { spans, width }
}

/// Fold styled text into rows no wider than `width`, keeping each character's style.
///
/// Restyling the folded rows would be the easy way and is wrong: a cell reading `` `**x**` `` is
/// six columns of code, and a second pass over the row's plain text would read the stars as
/// emphasis and draw one column where six were reserved.
fn fold(spans: &[Span<'static>], width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);

    let mut rows: Vec<Vec<(char, Style)>> = Vec::new();
    let mut row: Vec<(char, Style)> = Vec::new();
    let mut row_width = 0usize;
    // Where the row could be split: the index just after the last space on it.
    let mut last_space: Option<usize> = None;

    for (c, style) in spans
        .iter()
        .flat_map(|span| span.content.chars().map(move |c| (c, span.style)))
    {
        // Control characters have no width and would corrupt the column count.
        let c_width = c.width().unwrap_or(0);

        if row_width + c_width > width {
            match last_space {
                // Break after the last space, carrying the partial word to the next row.
                Some(index) if index < row.len() => {
                    let carried = row.split_off(index);
                    rows.push(std::mem::take(&mut row));
                    row = carried;
                    row_width = row.iter().map(|(c, _)| c.width().unwrap_or(0)).sum();
                }
                // A word wider than the column, or no space at all: hard-break.
                _ => {
                    rows.push(std::mem::take(&mut row));
                    row_width = 0;
                }
            }
            last_space = None;
        }

        row.push((c, style));
        row_width += c_width;
        if c == ' ' {
            last_space = Some(row.len());
        }
    }
    rows.push(row);

    rows.into_iter().map(coalesce).collect()
}

/// Gather styled characters back into spans, dropping the trailing spaces a fold leaves behind.
///
/// Trailing spaces would count towards the row's width, so a right-aligned cell would sit that
/// many columns short of its edge.
fn coalesce(chars: Vec<(char, Style)>) -> Vec<Span<'static>> {
    let mut chars = chars;
    while chars.last().is_some_and(|(c, _)| *c == ' ') {
        chars.pop();
    }

    let mut out: Vec<Span<'static>> = Vec::new();
    for (c, style) in chars {
        match out.last_mut() {
            Some(span) if span.style == style => span.content.to_mut().push(c),
            _ => out.push(Span::styled(c.to_string(), style)),
        }
    }
    out
}

/// Column widths for cells of `natural` width given `room`, or `None` if no split is legible.
///
/// A table that already fits is left at its own size rather than stretched across the terminal,
/// which is what a reader expects of a two-column table on a wide screen.
fn widths(natural: &[usize], room: usize) -> Option<Vec<usize>> {
    let columns = natural.len();
    let budget = room.checked_sub(GAP * columns.saturating_sub(1))?;
    if budget < columns * MIN_CELL_WIDTH {
        return None;
    }

    if natural.iter().sum::<usize>() <= budget {
        return Some(natural.to_vec());
    }

    // The widest column a budget of this size can afford to give everyone. Narrow columns keep
    // their natural width and the wide ones share what is left of the room.
    let mut low = MIN_CELL_WIDTH;
    let mut high = *natural.iter().max()?;
    while low < high {
        let cap = low + (high - low).div_ceil(2);
        let total: usize = natural.iter().map(|width| (*width).min(cap)).sum();
        if total <= budget {
            low = cap;
        } else {
            high = cap - 1;
        }
    }

    let mut out: Vec<usize> = natural.iter().map(|width| (*width).min(low)).collect();

    // Whatever the cap left over, handed to the columns it cut, so the rule reaches the edge.
    let mut spare = budget - out.iter().sum::<usize>();
    while spare > 0 {
        let mut given = false;
        for (width, wanted) in out.iter_mut().zip(natural) {
            if spare > 0 && *width < *wanted {
                *width += 1;
                spare -= 1;
                given = true;
            }
        }
        if !given {
            break;
        }
    }

    Some(out)
}

/// One folded row of a cell, padded to `width` and sat according to `align`.
fn sat(spans: Vec<Span<'static>>, width: usize, align: Align) -> Vec<Span<'static>> {
    let drawn: usize = spans.iter().map(|span| display_width(&span.content)).sum();
    let pad = width.saturating_sub(drawn);
    let (before, after) = match align {
        Align::Left => (0, pad),
        Align::Right => (pad, 0),
        Align::Centre => (pad / 2, pad - pad / 2),
    };

    let mut out = Vec::new();
    if before > 0 {
        out.push(Span::raw(" ".repeat(before)));
    }
    out.extend(spans);
    if after > 0 {
        out.push(Span::raw(" ".repeat(after)));
    }
    out
}

/// Draw one table row, which is as many screen rows as its tallest cell folded to.
fn row(cells: &[Cell], widths: &[usize], aligns: &[Align]) -> Vec<Vec<Span<'static>>> {
    let folded: Vec<Vec<Vec<Span<'static>>>> = cells
        .iter()
        .zip(widths)
        .map(|(cell, width)| fold(&cell.spans, *width))
        .collect();
    let height = folded.iter().map(Vec::len).max().unwrap_or(0);

    (0..height)
        .map(|line| {
            let mut out = Vec::new();
            for (index, column) in folded.iter().enumerate() {
                if index > 0 {
                    out.push(Span::raw(" ".repeat(GAP)));
                }
                let spans = column.get(line).cloned().unwrap_or_default();
                out.extend(sat(spans, widths[index], aligns[index]));
            }
            out
        })
        .collect()
}

/// Lay out the table starting at `lines[0]` in `room` display columns, if one starts there and
/// fits.
///
/// `None` means the caller must draw the source lines the way it always has: either the text is
/// not a table, or there is not enough of the terminal left to draw one legibly.
pub fn table(lines: &[&str], room: usize, base: Style) -> Option<Laid> {
    let parsed = parse(lines)?;

    let header: Vec<Cell> = parsed
        .header
        .iter()
        .map(|text| cell(text, base.add_modifier(Modifier::BOLD)))
        .collect();
    let body: Vec<Vec<Cell>> = parsed
        .body
        .iter()
        .map(|cells| cells.iter().map(|text| cell(text, base)).collect())
        .collect();

    let natural: Vec<usize> = (0..header.len())
        .map(|column| {
            body.iter()
                .map(|cells| cells[column].width)
                .chain(std::iter::once(header[column].width))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let widths = widths(&natural, room)?;

    let mut rows = row(&header, &widths, &parsed.aligns);
    rows.push(
        widths
            .iter()
            .enumerate()
            .flat_map(|(index, width)| {
                let mut out = Vec::new();
                if index > 0 {
                    out.push(Span::raw(" ".repeat(GAP)));
                }
                out.push(Span::styled(
                    RULE.to_string().repeat(*width),
                    base.fg(Color::DarkGray),
                ));
                out
            })
            .collect(),
    );
    for cells in &body {
        rows.extend(row(cells, &widths, &parsed.aligns));
    }

    Some(Laid {
        rows,
        consumed: parsed.consumed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Option<Parsed> {
        let lines: Vec<&str> = text.lines().collect();
        parse(&lines)
    }

    #[test]
    fn a_header_a_delimiter_and_rows_make_a_table() {
        let table = parsed("| a | b |\n| --- | --- |\n| 1 | 2 |").expect("a table");
        assert_eq!(table.header, ["a", "b"]);
        assert_eq!(table.body, [["1", "2"]]);
        assert_eq!(table.consumed, 3);
    }

    /// Models write both forms, and a table is a table either way.
    #[test]
    fn edge_pipes_are_optional() {
        let with = parsed("| a | b |\n| --- | --- |\n| 1 | 2 |").expect("a table");
        let without = parsed("a | b\n--- | ---\n1 | 2").expect("a table");
        assert_eq!(with.header, without.header);
        assert_eq!(with.body, without.body);
    }

    /// A candidate that fails has to leave the text alone. Prose containing a vertical bar is
    /// common, and rearranging it into columns because of one character would lose what the
    /// model actually said.
    #[test]
    fn a_block_without_a_delimiter_row_is_not_a_table() {
        assert!(parsed("| a | b |\n| 1 | 2 |").is_none());
        assert!(parsed("choose one | or the other\nand then continue").is_none());
    }

    #[test]
    fn a_delimiter_row_of_the_wrong_width_is_not_a_table() {
        assert!(parsed("| a | b |\n| --- |\n| 1 | 2 |").is_none());
        assert!(parsed("| a | b |\n| --- | not a delimiter |\n| 1 | 2 |").is_none());
    }

    #[test]
    fn colons_in_the_delimiter_row_set_the_alignment() {
        let table = parsed("a | b | c\n:--- | :---: | ---:\n1 | 2 | 3").expect("a table");
        assert_eq!(table.aligns, [Align::Left, Align::Centre, Align::Right]);
    }

    /// The pipe a model is most likely to write is in a code span, describing a shell pipeline.
    /// Splitting on it invents a column the table does not have.
    #[test]
    fn a_pipe_inside_inline_code_stays_in_its_cell() {
        assert_eq!(cells("| `a | b` | c |").expect("a row"), ["`a | b`", "c"]);
    }

    #[test]
    fn an_escaped_pipe_is_a_pipe() {
        assert_eq!(cells(r"| a \| b | c |").expect("a row"), ["a | b", "c"]);
        // A backslash before anything else is a backslash, so a path survives.
        assert_eq!(cells(r"| C:\dir | c |").expect("a row"), [r"C:\dir", "c"]);
    }

    #[test]
    fn a_short_row_is_padded_and_a_long_row_is_cut() {
        let table = parsed("a | b\n--- | ---\n| 1 |\n1 | 2 | 3").expect("a table");
        assert_eq!(table.body, [["1", ""], ["1", "2"]]);
    }

    /// The caller draws the lines after a table as prose, so it has to know where the table
    /// stopped. Counting wrong swallows a paragraph or repeats a row.
    #[test]
    fn the_table_ends_at_the_first_line_that_is_not_a_row() {
        let table = parsed("a | b\n--- | ---\n1 | 2\n\nand then prose").expect("a table");
        assert_eq!(table.consumed, 3);
        assert_eq!(table.body.len(), 1);
    }

    #[test]
    fn a_table_of_ten_thousand_columns_is_refused() {
        let row = "|".repeat(10_000);
        assert!(parsed(&format!("{row}\n{row}")).is_none());
    }

    /// The shape that would take longest, so the one worth checking terminates and invents
    /// nothing.
    #[test]
    fn a_line_of_nothing_but_pipes_terminates() {
        let line = "|".repeat(100_000);
        let split = cells(&line).expect("a row");
        assert!(
            split.iter().all(String::is_empty),
            "text appeared from nowhere"
        );
    }

    /// Every drawn row the same width is the whole reason this module exists. One row a column
    /// wider than another looks broken in a way raw pipes do not.
    fn laid(text: &str, room: usize) -> Laid {
        let lines: Vec<&str> = text.lines().collect();
        table(&lines, room, Style::default()).expect("a table")
    }

    fn drawn(text: &str, room: usize) -> Vec<String> {
        laid(text, room)
            .rows
            .iter()
            .map(|row| row.iter().map(|span| span.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn a_table_is_drawn_as_columns_without_its_pipes() {
        let rows = drawn("| gate | answer |\n| --- | --- |\n| edit | refuses |", 40);
        assert_eq!(rows[0], "gate  answer ");
        assert_eq!(rows[1], "────  ───────");
        assert_eq!(rows[2], "edit  refuses");
    }

    #[test]
    fn every_drawn_row_is_the_same_width() {
        let rows = drawn(
            "| a | b | c |\n| --- | :---: | ---: |\n| 日本語 | **x** | `y|z` |\n| ☃ | longer | q |",
            40,
        );
        let widths: Vec<usize> = rows.iter().map(|row| display_width(row)).collect();
        assert!(
            widths.windows(2).all(|pair| pair[0] == pair[1]),
            "ragged rows: {widths:?}"
        );
    }

    /// The width has to be of what is drawn. Measuring `**x**` as six columns and drawing one
    /// leaves the rule five columns adrift on that row alone.
    #[test]
    fn markup_in_a_cell_does_not_widen_its_column() {
        let plain = drawn("| a |\n| --- |\n| xx |", 40);
        let marked = drawn("| a |\n| --- |\n| **xx** |", 40);
        assert_eq!(
            display_width(&plain[1]),
            display_width(&marked[1]),
            "the markers were counted as content"
        );
    }

    #[test]
    fn alignment_puts_the_text_where_the_delimiter_says() {
        let rows = drawn(
            "| left | centre | right |\n| :--- | :---: | ---: |\n| x | x | x |",
            40,
        );
        // Columns of four, six, and five, so a one-character cell shows where it sits.
        assert_eq!(rows[2], "x       x         x");
    }

    /// Nothing may be dropped from the screen, so a cell too long for its column folds onto
    /// further rows rather than being cut.
    #[test]
    fn a_cell_wider_than_its_column_folds() {
        let rows = drawn(
            "| gate | answer |\n| --- | --- |\n| edit | it refuses on an untrusted file |",
            28,
        );
        assert!(rows.len() > 3, "the long cell did not fold: {rows:?}");
        let shown: String = rows.concat();
        assert!(shown.contains("untrusted"), "content was lost: {rows:?}");
        for row in &rows {
            assert!(display_width(row) <= 28, "row {row:?} overflowed");
        }
    }

    #[test]
    fn a_narrow_table_is_not_stretched_to_the_terminal() {
        let rows = drawn("| a | b |\n| --- | --- |\n| x | y |", 200);
        assert_eq!(display_width(&rows[0]), display_width(&rows[2]));
        assert!(display_width(&rows[0]) < 20, "stretched: {:?}", rows[0]);
    }

    /// Refusing is a supported outcome, not a failure. The reader gets the model's own
    /// characters, which is what they got before this module existed.
    #[test]
    fn a_table_that_cannot_be_given_legible_columns_is_refused() {
        let source = "| a | b | c | d | e | f |\n| --- | --- | --- | --- | --- | --- |\n| 1 | 2 | 3 | 4 | 5 | 6 |";
        let lines: Vec<&str> = source.lines().collect();
        for room in [0, 1, 20] {
            assert!(
                table(&lines, room, Style::default()).is_none(),
                "drew a table in {room} columns"
            );
        }
    }

    #[test]
    fn the_header_is_bold_and_code_in_a_cell_keeps_its_colour() {
        let rows = laid("| a |\n| --- |\n| `x` |", 40).rows;
        assert!(
            rows[0]
                .iter()
                .any(|span| span.style.add_modifier.contains(Modifier::BOLD)),
            "the header was not bold"
        );
        assert!(
            rows[2]
                .iter()
                .any(|span| span.style.fg == Some(Color::Cyan)),
            "inline code lost its colour"
        );
    }

    #[test]
    fn a_table_with_no_body_is_still_a_table() {
        let rows = drawn("| a | b |\n| --- | --- |", 40);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn odd_input_does_not_panic() {
        assert!(parsed("").is_none());
        assert!(cells("no pipe here").is_none());
        assert!(cells("|").is_none());
        assert!(parsed("日本 | 語\n--- | ---\n☃ | x").is_some());
        assert!(cells("| unclosed `code | here |").is_some());
        assert!(table(&[], 40, Style::default()).is_none());
        // A column of nothing is still a column, and must not divide by its own emptiness.
        assert!(drawn("|  |\n| --- |\n|  |", 40).len() == 3);
    }
}
