//! The mark an empty session opens on.
//!
//! Drawn in the transcript's own area rather than as a splash, so the first reply lands where
//! the mark was and nothing has to be dismissed. It is left aligned on a margin of its own, and
//! floated down from the top edge, because a block pinned into the corner reads as an error
//! message rather than as a title.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme;
use crate::theme::Rgb;

/// The wordmark, one string per row, padded to a rectangle.
///
/// Padded because the rows are otherwise three different lengths, and a row that ends early is a
/// row nothing can be checked against: the test that the mark is a rectangle is what would catch
/// a stroke lost while editing it.
const LOGO: &[&str] = &[
    "██                                 ██             ██   ",
    "██▀▀█▄ ██▀█ ▄█▀▀██ ██  ██ ▄█▀▀██   ██▀▀█▄ ▄█▀▀█▄ ▀██▀▀ ",
    "██░░██░██░░░██░░██░▀█▄▄█▀░██▀▀▀▀░  ██░░██░██░░██░ ██░░░",
    "██▄▄█▀░██░  ▀█▄▄██░ ▀██▀░░▀█▄▄▄▀░  ██▄▄█▀░▀█▄▄█▀░  ▀█▄▄",
    " ░░░░░░ ░░   ░░░░░░  ░░░░  ░░░░░░   ░░░░░░ ░░░░░░   ░░░",
];

/// The character the mark's drop shadow is drawn with.
///
/// Coloured apart from the letterform, since a shadow in the same ink as the letters is not a
/// shadow: it reads as a second, blurrier copy of the word.
const SHADOW: char = '░';

/// The column the name divides at.
///
/// Only "brave" carries the brand, and "bot" is drawn in whatever ink the terminal is set to, so
/// the mark reads as the name of a thing built on Brave rather than as a five letter logo with
/// three more letters stuck to it. Columns 33 and 34 are blank down every row, which is the gap
/// between the halves, and the seam falls after it so "bot" begins on its own first stroke.
const SEAM: usize = 35;

/// The two oranges the branded half fades between, left to right, as the Brave mark does.
///
/// Taken from [`crate::theme`] rather than written again here, because the same orange opens the
/// mark and draws every note the session makes in its own voice: two literals of it are how the
/// two would come to disagree.
const GRADIENT: (Rgb, Rgb) = (theme::BRAND, theme::BRAND_DEEP);

/// The left margin, wider than the transcript's lead so the mark sits clear of the edge.
const INDENT: &str = "   ";

/// Rows kept above the mark at most, however tall the terminal is.
///
/// A tall terminal would otherwise push the mark into the middle of the screen, a long way from
/// the box the user is about to type in.
const MAX_TOP: u16 = 5;

/// Columns the mark itself needs.
fn mark_width() -> usize {
    LOGO.first().map_or(0, |row| row.chars().count())
}

/// Whether the mark fits beside its margin.
///
/// A terminal narrower than this wraps the mark rather than clipping it, which folds the second
/// half of the word under the first and reads as a rendering fault. Dropping it is the better
/// failure: the name is written out below in any case.
fn fits(width: u16) -> bool {
    width as usize >= INDENT.len() + mark_width()
}

/// Rows the block occupies, so the padding can be measured against what is left.
fn height(with_mark: bool) -> u16 {
    // The mark when there is room for it, a blank, the name, and room for the invitation and the
    // line or two the session reports about starting up.
    let mark = if with_mark { LOGO.len() as u16 } else { 0 };
    mark + 5
}

/// Blank rows above the mark, given the height the transcript has.
///
/// A third of what is spare rather than half: the mark belongs above the middle of its area,
/// since the input box sits below it and the eye should travel downwards to reach it.
fn top_padding(width: u16, available: u16) -> u16 {
    (available.saturating_sub(height(fits(width))) / 3).min(MAX_TOP)
}

/// The brand's colour at one column of the branded half.
///
/// Mixed in whole channel steps rather than by ratio, since the gradient spans thirty five columns
/// and twenty two points of green: floating point buys no shade the terminal could show.
fn brand_at(column: usize) -> Color {
    let (start, end) = GRADIENT;
    let span = (SEAM - 1) as i32;
    let at = column.min(SEAM - 1) as i32;
    let mix = |from: u8, to: u8| (from as i32 + (to as i32 - from as i32) * at / span) as u8;

    Color::Rgb(
        mix(start.0, end.0),
        mix(start.1, end.1),
        mix(start.2, end.2),
    )
}

/// The ink a character takes, given the column it sits in.
///
/// Past the seam the letterform is left unstyled rather than given a colour of its own, so it
/// takes whatever the terminal is set to and stays legible on a light theme and a dark one alike.
/// The shadow keeps out of the gradient: it is depth rather than part of the mark's colour.
fn ink(column: usize, character: char) -> Style {
    if character == SHADOW {
        Style::default().fg(theme::muted())
    } else if column < SEAM {
        Style::default().fg(brand_at(column))
    } else {
        Style::default()
    }
}

/// One row of the mark, with the shadow inked apart from the letterform and "brave" apart
/// from "bot".
fn mark_row(row: &str) -> Line<'static> {
    let mut spans = vec![Span::raw(INDENT)];
    let mut run = String::new();
    let mut current: Option<Style> = None;

    // Batched by the ink itself rather than by which half the column is in, so the columns the
    // gradient rounds to the same shade stay one span instead of one span each.
    for (column, character) in row.chars().enumerate() {
        let next = ink(column, character);
        if let Some(had) = current.filter(|had| *had != next) {
            spans.push(Span::styled(std::mem::take(&mut run), had));
        }
        current = Some(next);
        run.push(character);
    }
    if let Some(had) = current {
        spans.push(Span::styled(run, had));
    }

    Line::from(spans)
}

/// The head of the opening screen: the mark, and the name beneath it.
///
/// `width` and `available` are the transcript area's. The first decides whether the mark is drawn
/// at all and the second how far down it floats. A terminal too short for the padding gets none.
///
/// Stops at the name because whatever the session has to report about starting up goes next, and
/// [`invitation`] closes the screen underneath that.
pub fn lines(confinement: &str, width: u16, available: u16) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    for _ in 0..top_padding(width, available) {
        lines.push(Line::raw(""));
    }

    if fits(width) {
        lines.extend(LOGO.iter().map(|row| mark_row(row)));
        lines.push(Line::raw(""));
    }

    lines.push(Line::from(vec![
        Span::raw(INDENT),
        Span::styled("bravebot", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("  ·  confinement {confinement}"),
            Style::default().fg(theme::muted()),
        ),
    ]));

    lines
}

/// The line that closes the opening screen, drawn under whatever the session reported.
///
/// Last rather than tucked under the name, so it sits nearest the box it is asking the user to
/// type in and reads as the next thing to do rather than as part of the title.
pub fn invitation() -> Line<'static> {
    Line::from(vec![
        Span::raw(INDENT),
        Span::styled(
            "Ask a question about this workspace.",
            Style::default().fg(theme::muted()),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wide enough for the mark, which most of these are not about.
    const WIDE: u16 = 90;

    fn rows(width: u16, available: u16) -> Vec<String> {
        lines("kernel-enforced", width, available)
            .iter()
            .chain(std::iter::once(&invitation()))
            .map(|line| line.to_string())
            .collect()
    }

    /// The mark is the point of the screen, so a short terminal loses the padding and keeps it.
    #[test]
    fn a_short_terminal_drops_the_padding_rather_than_the_mark() {
        let rows = rows(WIDE, 3);
        assert_eq!(top_padding(WIDE, 3), 0);
        assert!(rows[0].contains('█'), "the mark was pushed off: {rows:?}");
    }

    /// Glued to the top is what this exists to avoid.
    #[test]
    fn a_tall_terminal_floats_the_mark_down_from_the_top() {
        let rows = rows(WIDE, 40);
        assert!(
            rows.first().is_some_and(|row| row.trim().is_empty()),
            "the mark is against the top edge: {rows:?}"
        );
    }

    /// And a very tall one does not float it into the middle, a long way from the input box.
    #[test]
    fn the_padding_is_bounded_however_tall_the_terminal_is() {
        assert_eq!(top_padding(WIDE, 200), MAX_TOP);
    }

    /// Left aligned on a margin of its own: against the side edge is the other half of what
    /// this exists to avoid.
    #[test]
    fn every_row_of_the_mark_is_indented() {
        for row in rows(WIDE, 24).iter().filter(|row| !row.trim().is_empty()) {
            assert!(row.starts_with(INDENT), "glued to the edge: {row}");
        }
    }

    /// The mark says nothing a screen reader or a narrow pane can use, so the name is written
    /// out too, next to the confinement the session is running under.
    #[test]
    fn the_mark_names_the_agent_and_its_confinement() {
        let all = rows(WIDE, 24).join("\n");
        assert!(all.contains("bravebot"), "{all}");
        assert!(all.contains("confinement kernel-enforced"), "{all}");
        assert!(all.contains("Ask a question"), "{all}");
    }

    /// Wrapping the word under itself reads as a rendering fault, so a pane too narrow for the
    /// mark gets the name instead and the greeting still says what the session is.
    #[test]
    fn a_narrow_pane_drops_the_mark_rather_than_wrapping_it() {
        let narrow = (INDENT.len() + mark_width() - 1) as u16;
        let all = rows(narrow, 24).join("\n");
        assert!(!all.contains('█'), "the mark will wrap: {all}");
        assert!(all.contains("bravebot"), "nothing was left to read: {all}");
    }

    /// Every row the same width, or a stroke was lost while editing the mark.
    #[test]
    fn the_marks_rows_are_all_the_same_width() {
        let widths: Vec<usize> = LOGO.iter().map(|row| row.chars().count()).collect();
        assert!(
            widths.windows(2).all(|pair| pair[0] == pair[1]),
            "ragged: {widths:?}"
        );
    }

    /// The ink each column of a row is drawn in, the margin aside.
    fn inks(row: &str) -> Vec<Option<Color>> {
        mark_row(row)
            .spans
            .iter()
            .skip(1)
            .flat_map(|span| std::iter::repeat_n(span.style.fg, span.content.chars().count()))
            .collect()
    }

    /// The shadow is what gives the letterform its depth, so it is inked apart from it rather
    /// than drawn in the same orange.
    #[test]
    fn the_shadow_is_not_drawn_in_the_letterforms_ink() {
        let inks = inks(LOGO[2]);
        assert_eq!(inks[0], Some(brand_at(0)), "no letterform: {inks:?}");
        assert!(inks.contains(&Some(theme::muted())), "no shadow: {inks:?}");
    }

    /// Only the half of the name that is a brand is drawn in the brand's colours.
    #[test]
    fn the_orange_stops_at_the_end_of_brave() {
        let inks = inks(LOGO[1]);
        assert_eq!(inks[0], Some(brand_at(0)), "brave lost its ink");
        let muted = theme::muted();
        assert!(
            inks[SEAM..]
                .iter()
                .all(|ink| ink.is_none() || *ink == Some(muted)),
            "bot was branded too: {:?}",
            &inks[SEAM..]
        );
    }

    /// The gradient is the point: it has to actually travel between the two oranges rather than
    /// round to one of them across the whole word.
    #[test]
    fn the_gradient_runs_from_one_orange_to_the_other() {
        let (start, end) = GRADIENT;
        assert_eq!(brand_at(0), Color::Rgb(start.0, start.1, start.2));
        assert_eq!(brand_at(SEAM - 1), Color::Rgb(end.0, end.1, end.2));

        let shades: Vec<Color> = (0..SEAM).map(brand_at).collect();
        let steps = shades.windows(2).filter(|pair| pair[0] != pair[1]).count();
        assert!(steps > 4, "the fade is too coarse to read as one: {steps}");
    }

    /// A shade is never skipped backwards: a gradient that reversed anywhere would read as a
    /// banding fault rather than as a fade.
    #[test]
    fn the_gradient_only_ever_travels_one_way() {
        let green = |column: usize| match brand_at(column) {
            Color::Rgb(_, green, _) => green,
            other => panic!("not a mixed colour: {other:?}"),
        };
        assert!(
            (1..SEAM).all(|column| green(column) <= green(column - 1)),
            "the fade doubles back"
        );
    }

    /// Past the seam the letterform takes the terminal's own ink, so the mark reads on a light
    /// theme as well as a dark one.
    #[test]
    fn bot_is_left_in_the_terminals_own_ink() {
        let inks = inks(LOGO[1]);
        assert!(
            inks[SEAM..].contains(&None),
            "bot was given a colour: {inks:?}"
        );
    }

    /// The seam is a column count into art that may be edited, so it is pinned to the gap it is
    /// meant to fall in: a stroke either side of it would be cut in half by the colour change.
    #[test]
    fn the_seam_falls_in_the_gap_between_the_two_halves() {
        for row in LOGO {
            let columns: Vec<char> = row.chars().collect();
            assert_eq!(columns[SEAM - 2], ' ', "the seam cuts a stroke: {row}");
            assert_eq!(columns[SEAM - 1], ' ', "the seam cuts a stroke: {row}");
        }
    }
}
