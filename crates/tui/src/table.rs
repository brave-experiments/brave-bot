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

/// Columns a table may declare before it is drawn as its source instead.
///
/// Not a limit on what markdown can express, but the point past which a table stops being
/// readable in a terminal: twenty-four columns leaves under three characters each on a standard
/// width. The cap also bounds the layout, which would otherwise size itself from a header row
/// built to have a hundred thousand cells.
pub const MAX_COLUMNS: usize = 24;

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

    #[test]
    fn odd_input_does_not_panic() {
        assert!(parsed("").is_none());
        assert!(cells("no pipe here").is_none());
        assert!(cells("|").is_none());
        assert!(parsed("日本 | 語\n--- | ---\n☃ | x").is_some());
        assert!(cells("| unclosed `code | here |").is_some());
    }
}
