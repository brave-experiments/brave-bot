//! Drawing the session.
//!
//! Untrusted content, whether model replies or tool output, is shown, but the interface makes its
//! provenance visible rather than presenting everything as equally authoritative. That is
//! the point of showing labels in the trail: a user can see which text the system trusted
//! and which it merely carried.
//!
//! The layout is deliberately quiet: the transcript has no frame so replies read as text
//! rather than as boxed output, and only the input keeps a border, since that is the one
//! place the cursor needs locating.

use bravebot_agent::diff::Change;
use bravebot_agent::report::{Activity, Landing, Shown};
use bravebot_i18n::t;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};
use unicode_width::UnicodeWidthChar;

use crate::audit::TrailLine;
use crate::logo;
use crate::markdown;
use crate::state::{Laid, Session, Speaker, Status};
use crate::table;
use crate::theme;
use crate::wrap;

/// Marks a turn boundary in the transcript.
const TURN_MARKER: &str = "⏺";

/// Columns the transcript's own lead occupies before a reply's text.
///
/// A table is budgeted against what is left after it. Given the full width it would be two
/// columns too wide on every row, and the paragraph would soft-wrap the columns it just aligned.
const LEAD: usize = 2;
/// Marks a detail belonging to the entry above it.
const DETAIL_MARKER: &str = "⎿";
/// Joins the first task to the line above it, so the list reads as belonging to that turn.
const BRANCH_MARKER: &str = "└";

fn dim() -> Style {
    Style::default().fg(theme::muted())
}

/// Draw a task list beneath whatever it belongs to.
///
/// The first row carries a branch so the block attaches to the line above rather than floating.
/// Finished tasks are struck through and dimmed, which is what makes progress legible at a
/// glance: the eye finds the unstruck lines.
fn todo_lines(todos: &[bravebot_core::todo::Row]) -> Vec<Line<'static>> {
    todos
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let lead = if index == 0 {
                format!("  {BRANCH_MARKER} ")
            } else {
                "    ".to_string()
            };
            let (marker, text) = if row.struck() {
                (
                    Style::default().fg(theme::ok()),
                    dim().add_modifier(Modifier::CROSSED_OUT),
                )
            } else {
                (
                    Style::default().fg(theme::running()),
                    Style::default().add_modifier(Modifier::BOLD),
                )
            };
            Line::from(vec![
                Span::styled(lead, dim()),
                Span::styled(format!("{} ", row.marker), marker),
                Span::styled(row.content.clone(), text),
            ])
        })
        .collect()
}

/// How many diff lines a finished write shows before the rest is summarised away.
///
/// Enough to see what happened, not so much that a large edit pushes everything before it off
/// the screen. The approval prompt showed the whole thing; this is the record afterwards.
const MAX_DIFF_LINES: usize = 12;

/// Drawn down the margin of everything the model was not allowed to read.
///
/// One glyph, on every line of the block, so the mark cannot be ended by anything written inside
/// it. A caption could be imitated; a margin cannot.
const QUARANTINE_BAR: &str = "\u{2503}";

/// What the box says when there is nothing in it.
///
/// Drawn rather than typed, so it is never part of a prompt and never has to be deleted: the
/// first character typed takes its place, because the line it was standing in for now exists.
fn placeholder() -> &'static str {
    t!(input_placeholder)
}

/// Replace control characters, so drawn text cannot move the cursor or recolour the screen.
///
/// The interface's structure is drawn by this module: the margin down a quarantine block, the `!`
/// before a command, the colour that says which is which. An escape sequence in the text would let
/// the text draw those instead, and a forged margin is worse than no margin, since the whole point
/// of drawing one is that content cannot.
///
/// Everything the terminal would act on becomes a visible glyph, so what is on the screen stays a
/// faithful record of the bytes without being able to act. Tabs and newlines are handled before this
/// (lines are already split, and a tab is only ever width), so both are safe to keep.
fn printable(text: &str) -> String {
    if !text.chars().any(|c| c.is_control() && c != '\t') {
        return text.to_string();
    }
    text.chars()
        .map(|c| {
            if !c.is_control() || c == '\t' {
                c
            } else {
                // The Unicode pictures for C0, so an escape reads as ␛ rather than vanishing: a
                // character silently dropped is one a user cannot tell was ever there.
                char::from_u32(0x2400 + c as u32).unwrap_or('\u{fffd}')
            }
        })
        .collect()
}

/// Lay styled text out behind a margin, one drawn row at a time.
///
/// The bar goes at the head of every row the terminal draws, rather than at the head of every
/// logical line. Everything built here is handed to a wrapping paragraph, and ratatui prefixes
/// nothing to the rows its own wrapping produces: a line wider than the screen used to continue at
/// column 0 with no margin at all, which is the one thing the marking exists to make impossible,
/// and with the right padding the content could paint a bar of its own in the margin column.
/// Breaking the text here means no row can begin with content.
///
/// Control characters are replaced on the way in, for the same reason they are anywhere else the
/// margin means something: a row the renderer laid out is still a row the content could otherwise
/// move the cursor off.
pub(crate) fn marked_rows(
    margin: &Span<'static>,
    spans: &[Span<'static>],
    width: usize,
) -> Vec<Line<'static>> {
    // One column at least. A terminal too narrow for the margin still gets the margin, and the
    // content is broken a character at a time rather than drawn outside it.
    let room = width.saturating_sub(margin.width()).max(1);
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut row: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;

    for span in spans {
        for piece in pieces(&printable(&span.content)) {
            let mut piece = piece;
            loop {
                let reached = wrap::display_width(&piece);
                if used + reached <= room {
                    used += reached;
                    row.push(Span::styled(piece, span.style));
                    break;
                }
                if used > 0 {
                    rows.push(behind(margin, std::mem::take(&mut row)));
                    used = 0;
                    // The break falls between words, so the spaces that ended a row stay on it.
                    piece = piece.trim_start().to_string();
                    continue;
                }
                // A word wider than the row itself. There is nowhere better to break it, and
                // dropping the tail would lose bytes the person is being shown on purpose.
                let (head, tail) = split_at_width(&piece, room);
                row.push(Span::styled(head, span.style));
                rows.push(behind(margin, std::mem::take(&mut row)));
                piece = tail;
            }
        }
    }

    // Always a row, so an empty line is a marked one rather than a gap in the block.
    rows.push(behind(margin, row));
    rows
}

/// One row, with the margin in front of it.
fn behind(margin: &Span<'static>, mut spans: Vec<Span<'static>>) -> Line<'static> {
    spans.insert(0, margin.clone());
    Line::from(spans)
}

/// Text split into words carrying the spaces that follow them, which is where a row may break.
fn pieces(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut spacing = false;
    for c in text.chars() {
        if c == ' ' {
            spacing = true;
        } else if spacing {
            out.push(std::mem::take(&mut current));
            spacing = false;
        }
        current.push(c);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// `text` cut where it reaches `width` display columns, taking at least one character.
///
/// At least one, or a wide character in a one-column row would cut nothing and the caller would
/// break the same word forever.
fn split_at_width(text: &str, width: usize) -> (String, String) {
    let mut used = 0usize;
    for (index, c) in text.char_indices() {
        let reached = c.width().unwrap_or(0);
        if index > 0 && used + reached > width {
            return (text[..index].to_string(), text[index..].to_string());
        }
        used += reached;
    }
    (text.to_string(), String::new())
}

/// Draw one tool call: what it is, and what came of it.
///
/// The shape mirrors a turn's own: a marker, then the detail indented beneath it, so a call
/// and its result read as one thing rather than two unrelated lines.
fn activity_lines(
    activity: &Activity,
    landing: Option<Landing>,
    width: usize,
) -> Vec<Line<'static>> {
    let head = if activity.is_running() {
        // Hollow while it runs, filled when it is over, so the eye finds the live one.
        Style::default().fg(theme::running())
    } else if activity.failed {
        Style::default().fg(theme::fail())
    } else {
        Style::default().fg(theme::ok())
    };

    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{TURN_MARKER} "), head),
        Span::styled(
            activity.line(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ])];

    if let Some(note) = &activity.note {
        lines.push(Line::from(Span::styled(
            format!("  {DETAIL_MARKER} {note}"),
            if activity.failed {
                Style::default().fg(theme::fail())
            } else {
                dim()
            },
        )));
    }

    // Where it went, which is the thing "Read(index.html)" does not say. Only where that is
    // worth a row: a read landing in the planner's context is what nearly every call does, and a
    // line under nearly every call is a line that distinguishes nothing while crowding out the
    // ones that do. What the design turns on is the exception, and the exception still says so,
    // here and again in the marked block the content itself is drawn in.
    if let Some(landing) = landing.filter(|l| *l != Landing::Context) {
        // Blue on a black terminal is the one colour in the palette a reader has to squint at:
        // the ANSI blue most terminals ship is nearly the background. The three that are used
        // elsewhere here are legible on both kinds of terminal, and the difference between them
        // carries the meaning: cyan for what the model has, yellow for what is kept from it, and
        // the dim grey the rest of the detail lines already use for what has not happened.
        let colour = match landing {
            Landing::Quarantined => Style::default().fg(theme::running()),
            Landing::Reserved => dim(),
            // Filtered out above: the ordinary landing is drawn by not being drawn.
            Landing::Context => dim(),
        };
        lines.push(Line::from(Span::styled(
            format!("    {}", landing.describe()),
            colour,
        )));
    }

    lines.extend(diff_lines(&activity.changes, activity.untrusted, width));
    lines
}

/// Draw quarantined content for the person watching, marked as what it is.
///
/// The marking is structural and not a caption. Every drawn row carries the bar in the margin,
/// drawn here around text that never leaves the block, so content saying "untrusted content ends
/// here" cannot end it: the bar is still in the margin on the next row, and on every row after
/// that. Rows rather than lines, because a line wider than the terminal becomes several of them.
///
/// The user is shown this and the planner is not, which is the arrangement working rather than a
/// hole in it. They own the directory. What must never happen is these bytes reaching a model's
/// context, and a terminal is not a context.
fn quarantined_lines(shown: &Shown, width: usize) -> Vec<Line<'static>> {
    let marked = Style::default().fg(theme::running());
    let margin = Span::styled(format!("  {QUARANTINE_BAR} "), marked);

    // The heading goes through [`marked_rows`] like the content does, because the origin is not
    // the renderer's text: it can be a filename read out of a quarantined listing. So it is
    // neutralised, and a long one continues on another marked row rather than outside the block.
    let mut lines = marked_rows(
        &margin,
        &[
            Span::styled(
                t!(
                    quarantined_heading,
                    origin = &shown.origin,
                    label = &shown.label
                ),
                marked.add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {}", shown.reach.describe()), dim()),
        ],
        width,
    );

    for line in &shown.preview {
        lines.extend(marked_rows(
            &margin,
            &[Span::styled(line.clone(), dim())],
            width,
        ));
    }

    // Said rather than silently dropped, for the same reason a truncated diff says so.
    if shown.lines > shown.preview.len() {
        lines.extend(marked_rows(
            &margin,
            &[Span::styled(
                t!(
                    transcript_more_lines,
                    count = shown.lines - shown.preview.len()
                ),
                dim(),
            )],
            width,
        ));
    }

    lines
}

/// The hunks of a write, trimmed to what fits without burying the rest of the transcript.
fn diff_lines(changes: &[Change], untrusted: bool, width: usize) -> Vec<Line<'static>> {
    // The same margin the transcript draws down everything the model was not allowed to read. A
    // body that came out of a quarantined file is that, and the person reading the hunks should
    // not have to work out which kind of change they are looking at.
    let margin = if untrusted {
        Span::styled(
            format!("  {QUARANTINE_BAR}  "),
            Style::default().fg(theme::running()),
        )
    } else {
        Span::raw("     ")
    };
    let mut lines: Vec<Line> = changes
        .iter()
        .take(MAX_DIFF_LINES)
        .flat_map(|change| {
            // Neutralised and broken to the width by [`marked_rows`]: a hunk of a file an attacker
            // wrote is the same bytes as a quarantine preview, and here they sit beside a margin
            // that means something.
            let body = match change {
                Change::Added(text) => {
                    Span::styled(format!("+ {text}"), Style::default().fg(theme::ok()))
                }
                Change::Removed(text) => {
                    Span::styled(format!("- {text}"), Style::default().fg(theme::fail()))
                }
                Change::Kept(text) => Span::styled(format!("  {text}"), dim()),
                Change::Elided(count) => {
                    Span::styled(t!(transcript_unchanged, count = *count), dim())
                }
            };
            marked_rows(&margin, &[body], width)
        })
        .collect();

    // Said rather than silently dropped: a change that stops without saying so reads as the
    // whole change, which is how a reviewer misses half of it. Worded for both kinds of write,
    // since a new file's lines were never a diff of anything.
    if changes.len() > MAX_DIFF_LINES {
        lines.extend(marked_rows(
            &margin,
            &[Span::styled(
                t!(
                    transcript_more_lines,
                    count = changes.len() - MAX_DIFF_LINES
                ),
                dim(),
            )],
            width,
        ));
    }

    lines
}

/// Draw the whole interface, and say what the transcript came to.
///
/// The measurements go back to the session, because a key pressed next is answered against them
/// and none of them exist before a frame: how tall the transcript is, and where in it the rows
/// worth jumping to are, are both answers about a paragraph wrapped at a particular width.
pub fn draw(frame: &mut Frame, session: &Session) -> Laid {
    // A named theme paints the frame so chrome and text share one background. `brave` leaves the
    // terminal's own colours alone. It is painted before the mode is chosen, since the scroller
    // covers the same frame.
    if theme::paints_background() {
        frame.render_widget(
            Block::default().style(Style::default().bg(theme::background()).fg(theme::text())),
            frame.area(),
        );
    }

    // A different screen rather than the same one with a different footer: nothing below the
    // transcript is reachable while the scroller is open, so nothing below it is drawn.
    if session.scrolling() {
        return draw_scroller(frame, session);
    }

    // What is running sits above the box rather than in place of it, so the two are measured
    // together: whatever the indicator takes is height the input no longer has.
    let status_height = status_height(session, frame.area().height);

    // The input's height depends on how far the text wraps, so it is measured before the layout
    // rather than fixed: a fixed height is what made typing past the edge disappear.
    let input_height = input_height(
        session,
        frame.area().width,
        frame.area().height.saturating_sub(status_height),
    );

    // Beneath the box while something is being typed towards, and gone otherwise. Built rather
    // than counted, so a session offering nothing gives the whole height to the transcript and the
    // rows reserved are the rows drawn: the shortcut list folds into as many columns as the width
    // holds, so its height is not something a count of the entries can know. Bounded, so neither a
    // directory of many files nor a narrow terminal can push the transcript off the screen.
    let offered = session.offered();
    let room = frame
        .area()
        .height
        .saturating_sub(status_height)
        .saturating_sub(input_height + 1)
        .saturating_sub(1);
    let beneath = lines_beneath_the_box(session, frame.area().width, &offered);
    let offered_height = (beneath.len() as u16).min(room);

    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),                 // transcript
            Constraint::Length(status_height),  // what is running
            Constraint::Length(input_height),   // input
            Constraint::Length(offered_height), // what is being offered
            Constraint::Length(1),              // hint line
        ])
        .split(frame.area());

    let laid = draw_transcript(frame, areas[0], session);
    draw_status(frame, areas[1], session);
    draw_input(frame, areas[2], session);
    frame.render_widget(Paragraph::new(beneath), areas[3]);
    draw_hint(frame, areas[4], session);

    // Last, over everything: the selection is of the screen rather than of any one widget, and
    // the user swept it over whatever happened to be there.
    if let Some(selection) = &session.selection {
        crate::select::highlight(frame.buffer_mut(), selection);
    }

    laid
}

/// Draw the scroller: the transcript, and one row saying what the keys are.
///
/// The box goes, and so does the indicator above it and anything being offered beneath it. Every
/// one of them is a thing the person is being invited to type at, and while the scroller is open
/// no key reaches any of them: a box drawn under a mode that cannot reach it is three rows of the
/// screen spent inviting a keystroke that would do nothing. What they cost is given to the
/// transcript, which is the whole of what somebody opened a pager to look at.
///
/// The one row kept is the footer, because a mode where the letters do nothing with nothing on
/// the screen to say why is indistinguishable from an interface that has stopped responding.
fn draw_scroller(frame: &mut Frame, session: &Session) -> Laid {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // transcript
            Constraint::Length(1), // footer
        ])
        .split(frame.area());

    let laid = draw_transcript(frame, areas[0], session);
    draw_scroller_hint(frame, areas[1], session, laid.matches.len());

    // Over the transcript rather than beside it, because a key list is read instead of the
    // transcript and never at the same time.
    if session.scroller().is_some_and(|scroller| scroller.help) {
        draw_scroller_help(frame, frame.area());
    }

    if let Some(selection) = &session.selection {
        crate::select::highlight(frame.buffer_mut(), selection);
    }

    laid
}

/// The keys the scroller answers, in the order somebody reaches for them.
///
/// The way out is last and is never the row that did not fit: a list that scrolled its own exit
/// off the screen would be a mode nobody could leave.
fn scroller_keys() -> [(&'static str, &'static str); 8] {
    [
        ("up/down, j/k", t!(scroller_key_line)),
        ("ctrl-u / ctrl-d", t!(scroller_key_half_page)),
        ("space / b", t!(scroller_key_full_page)),
        ("g / G", t!(scroller_key_ends)),
        ("{ / }", t!(scroller_key_prompts)),
        ("/ then n/N", t!(scroller_key_search)),
        ("v", t!(scroller_key_editor)),
        ("?", t!(scroller_key_this_list)),
    ]
}

/// What closes the scroller, which is the one row of the key list that is never dropped.
fn scroller_exit() -> (&'static str, &'static str) {
    ("q / esc / ctrl-o", t!(scroller_key_close))
}

/// Draw the key list over the transcript.
///
/// Short terminals lose rows from the middle of the list rather than the end of it. Every row here
/// is a convenience except the last, and the last is the only one somebody is stuck without.
fn draw_scroller_help(frame: &mut Frame, area: Rect) {
    let row = |(key, what): (&str, &str)| {
        Line::from(vec![
            Span::styled(format!(" {key:<18}"), Style::default().fg(Color::Cyan)),
            Span::styled(what.to_string(), dim()),
        ])
    };

    // A border costs two rows, and on a terminal with three there is no version of this worth
    // having a frame around: the way out is what the list is for, and it goes on the screen with
    // or without one.
    let bordered = area.height >= 4;
    let room = if bordered {
        (area.height as usize).saturating_sub(2)
    } else {
        area.height as usize
    }
    .max(1);

    let mut rows: Vec<Line> = scroller_keys()
        .into_iter()
        .take(room.saturating_sub(1))
        .map(row)
        .collect();
    rows.push(row(scroller_exit()));

    let width = area.width.min(58);
    let height = (rows.len() as u16 + if bordered { 2 } else { 0 }).min(area.height);
    let box_area = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };

    frame.render_widget(ratatui::widgets::Clear, box_area);
    let list = Paragraph::new(rows);
    let list = if bordered {
        list.block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(format!(" {} ", t!(scroller_title))),
        )
    } else {
        list
    };
    frame.render_widget(list, box_area);
}

/// The one row under the transcript while the scroller is open.
///
/// What it says depends on what the scroller is doing, and in every case the way out comes early
/// enough to survive a narrow terminal cutting the end off.
fn draw_scroller_hint(frame: &mut Frame, area: Rect, session: &Session, found: usize) {
    let scroller = match session.scroller() {
        Some(scroller) => scroller,
        None => return,
    };

    // A search being typed owns the line: what somebody is typing is the thing they are looking
    // at, and a caret says the keys are going here rather than into the box.
    if let Some(typing) = &scroller.typing {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("  /{typing}"), Style::default().fg(Color::Cyan)),
                Span::styled(" ", Style::default().add_modifier(Modifier::REVERSED)),
                Span::styled(format!("  ·  {}", t!(scroller_searching)), dim()),
            ])),
            area,
        );
        return;
    }

    if !scroller.needle.is_empty() {
        // Never the matched text. The footer is the one row of the screen the interface speaks in
        // its own voice, and a quotation there is untrusted content drawn outside a marked block.
        let standing = if found == 0 {
            t!(scroller_no_matches).to_string()
        } else {
            t!(
                scroller_match_of,
                at = scroller.at.min(found - 1) + 1,
                total = found
            )
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("  /{}", scroller.needle),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(format!("  ·  {standing}"), dim()),
                Span::styled(format!("  ·  {}", t!(scroller_search_keys)), dim()),
            ])),
            area,
        );
        return;
    }

    let below = session.rows_below();
    let arrived = if below == 0 {
        String::new()
    } else {
        format!("  ·  {}", t!(scroller_rows_below, count = below))
    };

    // The indicator's row went with the box, and a turn that has not written anything yet leaves
    // nothing else on the screen moving. Somebody reading back through a turn in flight has to be
    // able to tell it is still in flight, so the footer says so in the indicator's own word.
    let running = session.indicator().map(|indicator| {
        Span::styled(
            format!("  ·  {}…", indicator.verb),
            Style::default().fg(Color::Green),
        )
    });

    // Four things and no more, because every key is behind `?` and a row long enough to be cut in
    // half advertises whatever happened to be at the near end of it. `/` is here because it is
    // the one key nobody guesses, and everything else on the row is a way to find out the rest.
    let mut spans = vec![
        Span::styled(
            format!("  {}", t!(scroller_footer)),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(format!("  ·  {}", t!(scroller_footer_keys)), dim()),
    ];
    spans.extend(running);
    spans.push(Span::styled(
        format!("{arrived}  ·  {}", t!(scroller_footer_search)),
        dim(),
    ));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Build the transcript as lines, so height is known before rendering.
///
/// `width` is the terminal's, and a table is laid out against what is left of it after the lead.
/// `height` is the transcript area's, which only the opening mark uses: it floats down from the
/// top edge by a share of whatever room there is.
/// Draw what the model said, styled rather than shown with its markdown markers.
///
/// Used for a finished turn in the transcript and for the reply still arriving beneath it, which
/// is what makes the handover invisible: the same words in the same shape in the same place, so
/// the moment a round ends nothing on the screen moves.
fn assistant_lines(text: &str, width: u16) -> Vec<Line<'static>> {
    let source: Vec<&str> = text.lines().collect();
    let room = (width as usize).saturating_sub(LEAD);
    let lead = |at: usize| {
        if at == 0 {
            Span::styled(format!("{TURN_MARKER} "), Style::default().fg(theme::ok()))
        } else {
            Span::raw("  ")
        }
    };

    let mut lines = Vec::new();
    let mut at = 0;
    while at < source.len() {
        // A table is several source lines drawn as one block, so it is tried first and the lines
        // it consumed are skipped. Anything that is not one falls through to the styling every
        // other line gets.
        match table::table(&source[at..], room, Style::default()) {
            Some(laid) => {
                for (index, row) in laid.rows.into_iter().enumerate() {
                    let mut spans = vec![lead(at + index)];
                    spans.extend(row);
                    lines.push(Line::from(spans));
                }
                at += laid.consumed;
            }
            None => {
                let mut spans = vec![lead(at)];
                spans.extend(markdown::spans(source[at], Style::default()));
                lines.push(Line::from(spans));
                at += 1;
            }
        }
    }
    lines
}

#[cfg(test)]
fn transcript_lines(session: &Session, width: u16, height: u16) -> Vec<Line<'static>> {
    with_prompts(session, width, height).0
}

/// The transcript, and the index of the line each prompt the person typed begins at.
///
/// Two answers from one pass, because working the second out afterwards would mean deciding which
/// drawn lines were prompts by looking at them, and the thing that knows is the pass that drew
/// them.
fn with_prompts(session: &Session, width: u16, height: u16) -> (Vec<Line<'static>>, Vec<usize>) {
    let mut prompts: Vec<usize> = Vec::new();
    let mut lines: Vec<Line> = Vec::new();

    // Nothing has been said yet, so the screen opens on the mark, laid out against the height
    // rather than stacked into the top corner. A note is not a conversation: the transcript
    // already holds what starting up reported, and that belongs under the mark rather than in
    // place of it, which is why this is not a test for an empty transcript. It was one, and the
    // mark was never drawn at all, since the trust answer is noted before the first frame.
    let opening = session
        .transcript
        .iter()
        .all(|entry| entry.speaker == Speaker::System);
    if opening {
        lines.extend(logo::lines(&session.confinement, width, height));
        lines.push(Line::raw(""));
    }

    for entry in &session.transcript {
        match entry.speaker {
            // The user's own words, echoed the way they were typed.
            Speaker::User => {
                prompts.push(lines.len());
                for (index, text) in entry.text.lines().enumerate() {
                    let prefix = if index == 0 { "> " } else { "  " };
                    lines.push(Line::from(Span::styled(
                        format!("{prefix}{text}"),
                        Style::default().fg(theme::brand_primary()),
                    )));
                }
            }
            // The model writes markdown whether or not it is asked to, so the reply is styled
            // rather than shown with its markers.
            Speaker::Assistant => lines.extend(assistant_lines(&entry.text, width)),
            Speaker::System => {
                for text in entry.text.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("{DETAIL_MARKER} {text}"),
                        Style::default().fg(theme::note()),
                    )));
                }
            }
            // Echoed with the marker the user typed it behind, so the scrollback reads back the way
            // the session happened.
            Speaker::Shell => {
                for (index, text) in entry.text.lines().enumerate() {
                    let prefix = if index == 0 { "! " } else { "  " };
                    lines.push(Line::from(Span::styled(
                        format!("{prefix}{}", printable(text)),
                        Style::default().fg(theme::accent()),
                    )));
                }
            }
            // Plainly, and indented to sit under the command. No marker down the margin and no
            // markdown: this is a terminal's output, and the user is reading it as one.
            //
            // Not a quarantine block either, which is the point of the whole feature: the user
            // typed the command, so the kernel labelled what it printed trusted, and dressing it as
            // untrusted would say the opposite of what is true.
            Speaker::Output => {
                for text in entry.text.lines() {
                    lines.push(Line::from(Span::raw(format!("  {}", printable(text)))));
                }
            }
            // What the turn did, kept in the scrollback next to what it said about it.
            Speaker::Tool => match &entry.activity {
                Some(activity) => {
                    lines.extend(activity_lines(activity, entry.landing, width as usize))
                }
                // A call read back out of a stored session, which records that it happened and
                // not what came of it. Drawn without the coloured marker a live call earns,
                // since green would claim an outcome the record does not have.
                None => lines.push(Line::from(vec![
                    Span::styled(format!("{TURN_MARKER} "), dim()),
                    Span::styled(entry.text.clone(), dim()),
                ])),
            },
        }

        // What the model was not allowed to read, for the person who is.
        if let Some(shown) = &entry.shown {
            lines.extend(quarantined_lines(shown, width as usize));
        }

        // The plan the turn worked to, kept next to what it produced.
        lines.extend(todo_lines(&entry.todos));

        if session.show_trail && !entry.trail.is_empty() {
            for recorded in &entry.trail {
                lines.push(trail_line(recorded));
            }
        }

        // Tool calls come in runs and read as one block, so they are not spaced apart. A turn
        // that read six files would otherwise take twelve lines of blank.
        if entry.speaker != Speaker::Tool {
            lines.push(Line::raw(""));
        }
    }

    // The reply being written right now, under everything that has already happened. Drawn from
    // the session rather than from the transcript because it is not an entry yet: when the round
    // ends the same words arrive as one, in this same place and this same shape, and the tail is
    // dropped in the same breath. Nothing on the screen moves at the handover.
    if !session.streaming.is_empty() {
        lines.extend(assistant_lines(&session.streaming, width));
        lines.push(Line::raw(""));
    }

    // One blank above it and no more. Every entry already leaves a trailing blank behind it, so
    // an invitation that carried its own would sit two rows below whatever startup reported.
    if opening {
        if lines.last().is_none_or(|line| line.width() != 0) {
            lines.push(Line::raw(""));
        }
        lines.push(logo::invitation());
    }

    (lines, prompts)
}

/// Drawn over every character a search matched.
///
/// Reversed rather than coloured, so it reads as a highlight against whatever the row was already
/// wearing: a match in a quarantined preview is dim grey, and a colour of its own would be one
/// more thing on the screen the content had chosen.
fn marked() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// Highlight every occurrence of `needle`, and say which lines held one.
///
/// The spans are split where a match begins and ends and the pieces restyled. Nothing on the
/// screen moves and nothing leaves the block it was drawn in: a match inside a quarantined
/// preview is highlighted between that block's margin and the end of its row, exactly where the
/// characters already were.
fn highlight(lines: &mut [Line<'static>], needle: &str) -> Vec<usize> {
    let mut held = Vec::new();
    if needle.is_empty() {
        return held;
    }

    for (index, line) in lines.iter_mut().enumerate() {
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        let found = crate::state::matched(&text, needle);
        if found.is_empty() {
            continue;
        }
        held.push(index);

        let mut rebuilt: Vec<Span<'static>> = Vec::new();
        let mut at = 0;
        for span in line.spans.drain(..) {
            let characters: Vec<char> = span.content.chars().collect();
            let mut cut = 0;
            while cut < characters.len() {
                let here = at + cut;
                // Either this character is inside a match, and the piece runs to the end of that
                // match, or it is not, and the piece runs to the start of the next one. Both
                // bounds are past `cut`, so the walk always advances.
                let (until, style) =
                    match found.iter().find(|(from, to)| here >= *from && here < *to) {
                        Some((_, to)) => (to - at, span.style.patch(marked())),
                        None => (
                            found
                                .iter()
                                .map(|(from, _)| *from)
                                .find(|from| *from > here)
                                .map_or(characters.len(), |from| from - at),
                            span.style,
                        ),
                    };
                let until = until.min(characters.len());
                let piece: String = characters[cut..until].iter().collect();
                rebuilt.push(Span::styled(piece, style));
                cut = until;
            }
            at += characters.len();
        }
        line.spans = rebuilt;
    }
    held
}

/// How many rows one line takes at this width, once it has been wrapped.
///
/// Through the same wrapping that will draw it, rather than through a second implementation that
/// agrees with the first until one day it does not: a count that drifts puts a jump a row or two
/// off, which is the kind of wrong nobody can see and everybody blames on something else.
///
/// This runs over every line of the transcript on every frame the scroller is open, so it does
/// not copy the text to count it. A line that fits is one row and is answered from its width
/// alone, which is nearly all of them.
fn rows_of(line: &Line<'_>, width: u16) -> u16 {
    let width = width.max(1);
    if line.width() <= width as usize {
        return 1;
    }
    let borrowed: Vec<Span<'_>> = line
        .spans
        .iter()
        .map(|span| Span::styled(span.content.as_ref(), span.style))
        .collect();
    Paragraph::new(Line::from(borrowed))
        .wrap(Wrap { trim: false })
        .line_count(width) as u16
}

/// The transcript laid out at a width: the lines to draw, and where the rows worth reaching are.
///
/// Where each line begins is only worked out while the scroller is open. It costs a wrap of every
/// line, and at rest nothing asks the question: the wheel moves by rows and never has to know
/// which row is which.
fn lay_out(session: &Session, width: u16, height: u16) -> (Vec<Line<'static>>, Laid) {
    let (mut lines, prompts) = with_prompts(session, width, height);
    let held = highlight(&mut lines, session.needle());

    let mut laid = Laid {
        width,
        height,
        ..Laid::default()
    };
    if !session.scrolling() {
        return (lines, laid);
    }

    let mut prompts = prompts.into_iter().peekable();
    let mut held = held.into_iter().peekable();
    let mut at = 0u16;
    for (index, line) in lines.iter().enumerate() {
        if prompts.peek() == Some(&index) {
            laid.prompts.push(at);
            prompts.next();
        }
        if held.peek() == Some(&index) {
            laid.matches.push(at);
            held.next();
        }
        at = at.saturating_add(rows_of(line, width));
    }
    laid.rows = at;
    (lines, laid)
}

/// The transcript as plain text, laid out exactly as it is drawn.
///
/// The rows as they are on the screen, margins included, so untrusted content is marked in the
/// file the same way it is marked on the screen and control characters are already glyphs. What
/// goes to an editor is what the person was looking at, not a second rendering of it that agrees
/// about the words and about nothing else.
pub fn as_text(session: &Session) -> String {
    let (lines, _) = lay_out(session, session.laid.width, session.laid.height);
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// Lay the transcript out again at the size the last frame used.
///
/// What a key needs in order to jump: the rows a search matched, at the width they were matched
/// at. The size comes from the last frame because that is the frame the person is looking at when
/// they press the key.
pub fn as_last_drawn(session: &Session) -> Laid {
    lay_out(session, session.laid.width, session.laid.height).1
}

fn draw_transcript(frame: &mut Frame, area: Rect, session: &Session) -> Laid {
    let (lines, mut laid) = lay_out(session, area.width, area.height);
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });

    // Scroll counts up from the bottom, so new output stays in view by default. The count has to
    // be of *drawn* rows rather than of lines: the paragraph wraps, so one line of a reply can
    // occupy three rows, and counting the lines put the bottom of the transcript exactly that
    // many rows below the screen. The end of a wrapped reply was therefore never shown, and
    // appeared only once the next message pushed it up.
    let total = paragraph.line_count(area.width) as u16;
    let max_offset = total.saturating_sub(area.height);

    // While the scroller is open the view is drawn from the row it is holding, counted from the
    // top, and not from the offset the last frame left behind. The end of the transcript moves
    // with every token a turn writes, so a frame drawn by counting back from it puts the view
    // wherever the rows that arrived since the last frame have pushed it: the anchor is correct
    // and the arithmetic reaching it is a frame out of date. Read from the top, nothing a turn
    // appends below can move what is above it.
    let offset = if session.scrolling() {
        session.top_row().min(max_offset)
    } else {
        max_offset.saturating_sub(session.scroll.min(max_offset))
    };

    frame.render_widget(paragraph.scroll((offset, 0)), area);

    // The paragraph's own count rather than the sum of the rows measured line by line, so what a
    // key is answered against is the number the view was actually drawn with.
    laid.rows = total;
    laid
}

/// Render one line of the audit trail.
///
/// Refusals are coloured differently from passes: a blocked gate is the most important
/// thing on the screen when it happens. The wording is settled in [`crate::audit`], so a line
/// that happened in this session and one read back off disk are drawn the same way.
fn trail_line(recorded: &TrailLine) -> Line<'static> {
    let style = if recorded.blocked {
        Style::default()
            .fg(theme::fail())
            .add_modifier(Modifier::BOLD)
    } else {
        dim()
    };
    Line::from(Span::styled(
        format!("  {DETAIL_MARKER} {}", recorded.text),
        style,
    ))
}

/// Columns available for input text inside the box.
///
/// Two for the borders, two for the `> ` prompt, and one for the caret, so a wrap computed against
/// this matches what the terminal will actually show. The caret takes a column only past the end of
/// the text, where it has no character to sit on and is drawn as a highlighted space, but the row
/// it ends up on is not known before wrapping, so the column is kept on every row.
fn input_text_width(total: u16) -> usize {
    (total as usize).saturating_sub(5).max(1)
}

/// Rows the indicator above the box needs, and the task list under it.
///
/// Zero when nothing is running. Bounded so a long list cannot take the transcript and the box
/// with it: the point of showing what is happening is lost if the box it is happening above has
/// been squeezed off the screen.
fn status_height(session: &Session, height: u16) -> u16 {
    // A row of transcript, three of box, and the hint line are what has to survive this.
    let ceiling = (height as usize).saturating_sub(5).max(1);

    match session.status {
        // The indicator, and the task list beneath it if the turn kept one.
        Status::Working => (1 + session.todos.len()).min(ceiling) as u16,
        // A command spends no tokens and keeps no task list, so one line says everything.
        Status::Running => 1,
        Status::Idle | Status::Quitting => 0,
    }
}

/// Rows the input box needs, borders included.
///
/// Grows with the text up to [`wrap::MAX_ROWS`], and never takes so much of a short terminal that
/// the transcript disappears. The box is measured the same way whatever the session is doing: a
/// turn in flight does not take it away, because a person typing their next prompt has to be able
/// to see what they are typing.
fn input_height(session: &Session, width: u16, height: u16) -> u16 {
    // Leave at least one line of transcript and the hint line, whatever is in the box.
    let ceiling = (height as usize).saturating_sub(2).max(3);

    let rows = wrap::wrap(session.input(), input_text_width(width), session.caret())
        .rows
        .len()
        .min(wrap::MAX_ROWS);

    (rows + 2).min(ceiling) as u16
}

/// A row with `at..through` of it drawn as a block, which is where the caret is.
///
/// A block rather than a glyph inserted between two characters, because inserting one moves
/// everything after it by a column: the text shifted left and right under the caret as it moved,
/// which is far more distracting than the caret itself. Nothing moves now, since the caret occupies
/// cells that were already there.
///
/// An empty range is the caret past the end of the line, where there is no cell to occupy, so a
/// highlighted space is added. That is the one place the caret still takes a column, and
/// [`input_text_width`] reserves it.
fn caret_spans(row: &str, at: usize, through: usize, colour: Color) -> Vec<Span<'static>> {
    // Reversed rather than a chosen pair of colours, so the cells invert whatever the terminal's
    // own foreground and background happen to be and stay legible on either kind of theme.
    let block = Style::default().fg(colour).add_modifier(Modifier::REVERSED);

    let mut spans = vec![Span::raw(row[..at].to_string())];
    if through > at {
        spans.push(Span::styled(row[at..through].to_string(), block));
        spans.push(Span::raw(row[through..].to_string()));
    } else {
        spans.push(Span::styled(" ", block));
    }
    spans
}

/// What opens a row of the box: the prompt character, or the indent a continuation lines up under.
///
/// One place, because the invitation is drawn behind the same opening as the line it stands in
/// for, and two copies of it are how the two came to sit in different columns.
fn lead_for(index: usize, shell: bool) -> &'static str {
    // Only the first row carries the prompt; continuations are indented to line up beneath it.
    if index != 0 {
        "  "
    } else if shell {
        "! "
    } else {
        "> "
    }
}

/// The invitation, with the caret sitting on its first character.
///
/// The caret keeps its place rather than being drawn past the words, because the box is empty and
/// that is where the first character will land. Dimmed so it reads as an absence of text rather
/// than as text somebody has to clear.
fn placeholder_spans(colour: Color) -> Vec<Span<'static>> {
    let block = Style::default().fg(colour).add_modifier(Modifier::REVERSED);
    let mut characters = placeholder().chars();
    let first: String = characters.by_ref().take(1).collect();
    vec![
        Span::styled(first, block),
        Span::styled(characters.as_str().to_string(), dim()),
    ]
}

/// How much of the row starting at `start` a span of the line covers, in the row's own offsets.
///
/// `None` when the two do not meet, which is every row a covered marker does not reach.
fn covered(row: &str, start: usize, span: (usize, usize)) -> Option<(usize, usize)> {
    let (from, to) = span;
    let at = from.saturating_sub(start).min(row.len());
    let through = to.saturating_sub(start).min(row.len());
    (at < through).then_some((at, through))
}

/// Draw what is running, above the box.
///
/// Above rather than inside, because the box is still the user's: a turn takes as long as it takes
/// and the next prompt is usually thought of during it, so the line being typed has to stay on
/// screen. It was drawn over, and someone typing through a slow turn watched their words go
/// nowhere, which is indistinguishable from an interface that has stopped responding.
fn draw_status(frame: &mut Frame, area: Rect, session: &Session) {
    let working = session.status == Status::Working;
    let running = session.status == Status::Running;

    let indicator = if running {
        // Elapsed time and nothing else, because a command spends no tokens and reports no phase.
        // The clock is what a waiting user wants, and Escape is the other half of the answer.
        Line::from(vec![
            Span::styled(
                format!("  {} ", session.spinner()),
                Style::default().fg(theme::accent()),
            ),
            Span::styled("running… ", Style::default().fg(theme::accent())),
            Span::styled(format!("({})  esc to stop", session.elapsed_words()), dim()),
        ])
    } else if working {
        match session.indicator() {
            Some(indicator) => Line::from(vec![
                Span::styled(
                    format!("  {} ", indicator.glyph),
                    Style::default().fg(theme::accent()),
                ),
                Span::styled(
                    format!("{}… ", indicator.verb),
                    Style::default().fg(theme::ok()),
                ),
                // Dim: the counters answer a question without competing for attention.
                Span::styled(indicator.detail(), dim()),
            ]),
            None => Line::from(Span::styled("  waiting for the model…", dim())),
        }
    } else {
        return;
    };

    // The list sits under the indicator, so what is being worked on and what remains are read
    // together. Trimmed to what this area was given rather than overflowing into the box below.
    let room = (area.height as usize).saturating_sub(1);
    let mut lines = vec![indicator];
    if working {
        lines.extend(todo_lines(&session.todos).into_iter().take(room));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_input(frame: &mut Frame, area: Rect, session: &Session) {
    let wrapped = wrap::wrap(
        session.input(),
        input_text_width(area.width),
        session.caret(),
    );
    let visible = (area.height as usize).saturating_sub(2).max(1);
    let (first, rows) = wrapped.window(visible);
    let marker = session.marker_at_caret();

    // Shell mode is coloured throughout rather than only in the marker, because the whole line
    // means something different: it goes to a shell instead of the model, and that is worth more
    // than one character of distinction at the moment somebody presses Enter.
    let colour = if session.shell {
        theme::accent()
    } else {
        theme::brand_primary()
    };

    // Wrapping is computed above rather than left to `Paragraph`, because the cursor has to be
    // placed after the last character and only an explicit wrap knows where that is.
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(offset, row)| {
            let index = first + offset;
            let mut spans = vec![Span::styled(
                lead_for(index, session.shell),
                Style::default().fg(colour),
            )];
            // A marker is one thing to the caret, so the caret covers the whole of it rather than
            // the one character it happens to start with. The span is located in the line, not in
            // the row, because the wrap is free to put a long marker across two of them.
            match marker.and_then(|span| covered(row, wrapped.starts[index], span)) {
                Some((at, through)) => spans.extend(caret_spans(row, at, through, colour)),
                None if index == wrapped.cursor_row => {
                    let at = wrapped.cursor_index;
                    let on = row[at..].chars().next().map_or(at, |c| at + c.len_utf8());
                    spans.extend(caret_spans(row, at, on, colour));
                }
                None => spans.push(Span::raw(row.clone())),
            }
            Line::from(spans)
        })
        .collect();

    // An empty box says what it is for. Shell mode says something different with its own colour
    // and its own prompt character, so it is left to say it: what a person needs there is which
    // shell and how to get back out, not an invitation to ask a question.
    //
    // Behind the same prompt character the typed line gets, so the words stand exactly where the
    // first one typed will land and nothing on the row moves when it does.
    let lines = if session.input().is_empty() && !session.shell {
        let mut spans = vec![Span::styled(
            lead_for(0, session.shell),
            Style::default().fg(colour),
        )];
        spans.extend(placeholder_spans(colour));
        vec![Line::from(spans)]
    } else {
        lines
    };

    // The position sits in the border while browsing, labelling the box without taking a row
    // away from the text.
    //
    // Dimmed while something is in flight. The line can be typed and edited then, but it cannot
    // be sent, and the border is what says which of the two the box is currently good for.
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if session.status != Status::Idle {
            dim()
        } else if session.shell {
            Style::default().fg(theme::accent())
        } else {
            Style::default().fg(theme::brand_primary())
        });

    if let Some((index, total)) = session.history.position() {
        block = block.title_top(Line::from(Span::styled(
            format!(" History {index}/{total} "),
            dim(),
        )));
    }

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// How a command is written in the list: the word, and what follows it.
fn command_word(command: &crate::app::Command) -> String {
    if command.argument.is_empty() {
        command.name.to_string()
    } else {
        format!("{} {}", command.name, command.argument)
    }
}

/// What is attached to the line being typed, one per row.
///
/// The marker as it appears in the line, then the path it stands for. The path is shown because
/// the marker is deliberately short and a user about to send a file should be able to see which
/// file it is: `[Image #1]` alone is not enough to tell a screenshot from a private key.
///
/// A filename is content, and this is content reaching a screen rather than a model. It is the
/// user's own drop being read back to them, so nothing here is a decision and nothing is labelled.
fn attached_lines(session: &Session, width: u16) -> Vec<Line<'static>> {
    session
        .attached_to_the_line()
        .map(|attached| {
            let lead = format!("  {} ", attached.marker);
            let room = (width as usize).saturating_sub(lead.chars().count());
            Line::from(vec![
                Span::styled(lead, Style::default().fg(theme::brand_primary())),
                Span::styled(printable(&tail_of(&attached.shown, room)), dim()),
            ])
        })
        .collect()
}

/// The line put away, on one row, with the key that brings it back.
///
/// Drawn because the key took a line off the screen. Without a row saying where it went, a press
/// that emptied the box is indistinguishable from one that threw the words away, and the only way
/// to find out which it was is to press the key again and hope.
///
/// One row however long the line was: this is a reminder that something is waiting, not the line
/// itself, and a paragraph put away would push the transcript off the screen to say so.
///
/// The words come before the key where the two cannot both fit. Which line is waiting is the part
/// only this row can say; the key is also in the list `?` puts up, and cut to `ctrl-s to bring i` it
/// is worse than absent. Below the width that holds a legible few words of the line, the reminder
/// goes and the line keeps the row.
///
/// No row may run past the edge, for the reason the shortcut list may not: a row that wraps takes a
/// row the layout did not reserve and pushes the hint line off the screen.
///
/// The text is the user's own words on their way back to them, so nothing here is a decision and
/// nothing is labelled. `printable` all the same, since a line can be pasted and a paste can carry
/// anything.
fn stashed_lines(session: &Session, width: u16) -> Vec<Line<'static>> {
    /// Enough of the line to recognise it by. Below this the reminder is dropped to make room.
    const LEGIBLE: usize = 12;

    let Some(stashed) = session.stashed() else {
        return Vec::new();
    };

    let lead = "  stashed ";
    let trail = "  ctrl-s to bring it back";
    let after_lead = (width as usize).saturating_sub(lead.chars().count());
    // Nothing of the line itself would fit, so the row would say only that something is stashed
    // without saying what. A terminal this narrow has no room to spare for that.
    if after_lead == 0 {
        return Vec::new();
    }

    let with_trail = after_lead.saturating_sub(trail.chars().count());
    let (room, trail) = if with_trail >= LEGIBLE {
        (with_trail, trail)
    } else {
        (after_lead, "")
    };

    vec![Line::from(vec![
        Span::styled(lead, Style::default().fg(theme::brand_primary())),
        Span::styled(printable(&head_of(stashed, room)), dim()),
        Span::styled(trail, dim()),
    ])]
}

/// Prompts that have been sent and are waiting for the turn in flight to end.
///
/// The line as it was typed, and under it the word saying what has happened to it. Marked rather
/// than merely indented, because a person who pressed Enter has to be able to tell at a glance
/// that the words went somewhere: a line sitting quietly under the box is what this replaced, and
/// it read as a key press that had been ignored.
///
/// The prompt is the user's own text on its way back to them, so nothing here is a decision and
/// nothing is labelled. Control characters go through `printable` all the same, since a prompt can
/// be pasted and a paste can carry anything.
fn queued_lines(session: &Session, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for waiting in &session.queued {
        let room = (width as usize).saturating_sub(4);
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default().fg(theme::brand_primary())),
            Span::styled(
                printable(&head_of(&waiting.prompt, room)),
                Style::default().fg(theme::brand_primary()),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                " QUEUED ",
                Style::default()
                    .fg(Color::Black)
                    .bg(theme::brand_primary())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    lines
}

/// The opening of a line, with an ellipsis where it was cut.
///
/// The opposite end from [`tail_of`], and for the opposite reason: a path is identified by its
/// end and a sentence by its beginning. A queued prompt runs to one row, and the words that say
/// which prompt it is are the first ones.
fn head_of(line: &str, room: usize) -> String {
    // A prompt may be a paragraph; it is one row here, so the rest is not shown at all.
    let first = line.lines().next().unwrap_or("");
    let characters: Vec<char> = first.chars().collect();
    if characters.len() <= room && first.len() == line.len() {
        return first.to_string();
    }
    let kept: String = characters
        .iter()
        .take(room.saturating_sub(1))
        .collect::<String>();
    format!("{kept}…")
}

/// A path shortened from the left, keeping as much of the end as will fit.
///
/// From the left because the end is the part that identifies the file. Cutting the other way is
/// what a plain truncation does, and it leaves every attachment in a deep directory reading as the
/// same long prefix with the filename gone, which is the one thing the row exists to show.
fn tail_of(path: &str, room: usize) -> String {
    let characters: Vec<char> = path.chars().collect();
    if characters.len() <= room || room == 0 {
        return path.to_string();
    }

    // One column for the ellipsis that says something was cut.
    let kept = room.saturating_sub(1);
    let tail: String = characters[characters.len() - kept..].iter().collect();
    format!("…{tail}")
}

/// Everything drawn between the box and the hint line, one per row.
///
/// Returned rather than rendered, because the height the layout reserves has to be the height this
/// comes to: the shortcut list folds into as many columns as the width holds, so counting the
/// entries would not answer it.
///
/// What is offered is commands or files, never a mixture, because the line can only be being typed
/// towards one of them. Nothing labelled is involved either way: the commands are this program's own
/// words, and the filenames are read out of the directory to show a person which files are in it,
/// never to decide anything and never reaching a model from here.
fn lines_beneath_the_box(
    session: &Session,
    width: u16,
    offered: &crate::state::Offered,
) -> Vec<Line<'static>> {
    // Attachments first, nearest the box, because they belong to the line still in it: they are
    // what the next Enter will carry, and a file staged mid-turn appeared below the queue, which
    // reads as belonging to a prompt already gone.
    let mut lines = attached_lines(session, width);
    // Then the line put away, which is about the box and nothing else: it is what the next press of
    // one key puts back into it. Above the queue because those prompts have gone and this one has
    // not, and below the attachments because they belong to the line in the box now.
    lines.extend(stashed_lines(session, width));
    // Then waiting prompts, which describe something already done. Below the line they are no
    // part of, and still above what is offered, since a line the person believes they have sent
    // has to be visible without hunting for it.
    lines.extend(queued_lines(session, width));
    lines.extend(match offered {
        crate::state::Offered::Nothing => Vec::new(),
        crate::state::Offered::Commands(commands) => command_lines(session, commands),
        crate::state::Offered::Files(entries) => entry_lines(session, entries),
        crate::state::Offered::Shortcuts => shortcut_lines(width),
    });
    lines
}

/// One row per command, with what it does.
///
/// The description column is measured from every command rather than from the ones on screen, so it
/// sits in the same place however far the list has narrowed. Measuring the visible rows instead
/// would slide the descriptions sideways with each letter typed.
fn command_lines(session: &Session, offered: &[crate::app::Command]) -> Vec<Line<'static>> {
    let column = crate::app::commands()
        .iter()
        .map(|command| command_word(command).chars().count())
        .max()
        .unwrap_or(0);

    let highlighted = session.highlighted_completion();
    offered
        .iter()
        .map(|command| {
            let chosen = Some(*command) == highlighted;
            let name = if chosen {
                Style::default()
                    .fg(theme::brand_primary())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::brand_primary())
            };

            let word = command_word(command);
            let padding = column.saturating_sub(word.chars().count()) + 2;

            Line::from(vec![
                Span::styled(if chosen { "  ❯ " } else { "    " }, name),
                Span::styled(word, name),
                Span::raw(" ".repeat(padding)),
                Span::styled(command.description, dim()),
            ])
        })
        .collect()
}

/// How the hint line says where the rest of the bindings went.
const SHORTCUTS_HINT: &str = "? for shortcuts";

/// Every key and marker, and what it does, in the order they are listed.
///
/// The one place they are written down, so a binding that changes cannot leave the list advertising
/// something that no longer works. Only what a person reaches for: the readline chords that move
/// the caret are there for the terminals that send nothing else, not because anybody looks them up.
///
/// The meanings are kept short deliberately. The longest of them sets the column, so a word saved
/// here is what lets two columns fit a terminal eighty wide, and that halves the rows the list takes
/// out of the transcript.
const SHORTCUTS: [(&str, &str); 17] = [
    ("!", "run a shell command"),
    ("/", "commands"),
    ("@", "name a file"),
    ("?", "this list"),
    ("enter", "send"),
    ("shift-enter", "new line, or ctrl-j"),
    ("tab", "take what is offered"),
    ("esc", "clear the line"),
    ("up / down", "earlier prompts"),
    ("pgup / pgdn", "scroll the transcript"),
    ("ctrl-c", "stop, clear, then exit"),
    ("ctrl-d", "exit"),
    ("ctrl-g", "write prompt in $EDITOR"),
    ("ctrl-s", "stash, or bring it back"),
    ("ctrl-t", "show what a turn did"),
    ("ctrl-v", "paste, pictures too"),
    ("drag", "select, copy on release"),
];

/// The shortcuts in as many columns as the width will hold.
///
/// Filled down each column rather than across each row, so the markers stay together at the top of
/// the first one: read across and `!`, `/` and `@` would be split up by whatever the width happened
/// to be. One column when nothing else fits, and the meanings are cut to the width there rather
/// than drawn past the edge, where the terminal would wrap them under the keys and put the list one
/// row over the height the layout reserved for it.
fn shortcut_lines(width: u16) -> Vec<Line<'static>> {
    /// Blank columns between one column of the list and the next.
    const GUTTER: usize = 3;
    /// Where the list starts, matching the other rows drawn beneath the box.
    const INDENT: usize = 4;

    let room = (width as usize).saturating_sub(INDENT);
    // Nothing legible fits, so nothing is drawn. Every column below is measured against this, and a
    // width of nothing would leave them all at zero and every entry cut down to its ellipsis.
    if room == 0 {
        return Vec::new();
    }

    let key_column = SHORTCUTS
        .iter()
        .map(|(key, _)| key.chars().count())
        .max()
        .unwrap_or(0)
        .min(room);
    let widest = SHORTCUTS
        .iter()
        .map(|(_, meaning)| meaning.chars().count())
        .max()
        .unwrap_or(0);

    let columns = ((room + GUTTER) / (key_column + 2 + widest + GUTTER)).max(1);
    let rows = SHORTCUTS.len().div_ceil(columns);
    // What a meaning has to itself once the keys, the padding and the gutters are taken out. As wide
    // as the longest wherever there is room for it, which is every width but the narrowest; zero
    // where the keys alone fill the row, and then they are listed without their meanings rather than
    // with the meanings running past the edge.
    let meaning_column = widest.min(
        (room.saturating_sub(GUTTER * (columns - 1)) / columns).saturating_sub(key_column + 2),
    );

    (0..rows)
        .map(|row| {
            let mut spans = vec![Span::raw(" ".repeat(INDENT))];
            for column in 0..columns {
                let Some((key, meaning)) = SHORTCUTS.get(row + column * rows) else {
                    break;
                };
                // Only between columns, so no row carries trailing blanks a selection would pick up.
                if column > 0 {
                    spans.push(Span::raw(" ".repeat(GUTTER)));
                }
                let key = head_of(key, key_column);
                let gap = key_column - key.chars().count();
                spans.push(Span::styled(
                    key,
                    Style::default().fg(theme::brand_primary()),
                ));
                if meaning_column == 0 {
                    continue;
                }
                spans.push(Span::raw(" ".repeat(gap + 2)));
                let shown = head_of(meaning, meaning_column);
                let padding = meaning_column - shown.chars().count();
                spans.push(Span::styled(shown, dim()));
                // Padded only where another column follows, for the same reason as the gutter.
                if row + (column + 1) * rows < SHORTCUTS.len() {
                    spans.push(Span::raw(" ".repeat(padding)));
                }
            }
            Line::from(spans)
        })
        .collect()
}

/// One row per workspace entry a reference could name.
///
/// A directory is dimmer than a file and keeps its trailing slash, because the two are chosen for
/// different reasons: a file is what a reference ends at, and a directory is somewhere to keep
/// typing. Nothing here says what a file contains, only that it exists.
fn entry_lines(session: &Session, offered: &[crate::entries::Entry]) -> Vec<Line<'static>> {
    let highlighted = session.highlighted_entry();
    offered
        .iter()
        .map(|entry| {
            let chosen = Some(entry) == highlighted.as_ref();
            let colour = if entry.is_directory {
                Style::default().fg(theme::accent())
            } else {
                Style::default().fg(theme::brand_primary())
            };
            let style = if chosen {
                colour.add_modifier(Modifier::BOLD)
            } else {
                colour
            };

            Line::from(vec![
                Span::styled(if chosen { "  ❯ " } else { "    " }, style),
                Span::styled(entry.path.clone(), style),
            ])
        })
        .collect()
}

/// The shortcut line. Keeps the bindings discoverable without a help command.
fn draw_hint(frame: &mut Frame, area: Rect, session: &Session) {
    let trail = if session.show_trail {
        "ctrl-t hide trail"
    } else {
        "ctrl-t show trail"
    };

    // In shell mode the usual bindings are beside the point: the line goes to a shell, so what a
    // user needs to know is which shell and how to get back out again.
    if session.shell {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("  ! {}", bravebot_agent::shell::shell()),
                    Style::default().fg(theme::accent()),
                ),
                Span::styled("  ·  esc to cancel  ·  output goes to the model", dim()),
            ])),
            area,
        );
        return;
    }

    // Only once a request has been measured. A gauge reading zero before anything has been sent
    // would be a claim about a context nobody has counted.
    let context = match session.fullness() {
        Some(percent) => format!("  ·  context {percent}%"),
        None => String::new(),
    };

    // Not a list of bindings any more. Every one of them, with what it does, is a `?` away, which
    // is both more than this line could hold and the moment a person wants to know; what stays here
    // is what the session is doing, which is the part they cannot ask for.
    //
    // The state comes first because the line is cut by a terminal narrower than it, and the end is
    // what goes. A binding cut off is one somebody learns once; a figure cut off is the only thing
    // here they have no other way to see.
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(
                "  {trail}  ·  confinement {}{context}  ·  {SHORTCUTS_HINT}",
                session.confinement
            ),
            dim(),
        ))),
        area,
    );

    // The line the person was writing has just gone, so the press that took it is the one thing
    // worth explaining: without this, a key they pressed to stop something emptied the box and
    // said nothing, and the next press of it ends the session.
    if session.cleared_by_interrupt {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "ctrl-c again to exit  ",
                Style::default().fg(theme::brand_primary()),
            )))
            .alignment(Alignment::Right),
            area,
        );
        return;
    }

    // A copy is silent otherwise, and a clipboard that may or may not have taken something is
    // worse than no clipboard: the user pastes to find out. Right-aligned, out of the way of the
    // hints, where the answer to "did that work" belongs.
    if let Some(characters) = session.copied {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("{} to clipboard  ", tally(characters, "char", "chars")),
                Style::default().fg(theme::brand_primary()),
            )))
            .alignment(Alignment::Right),
            area,
        );
        return;
    }

    // Command-V cannot carry a picture and cannot say so: the chord never reaches this process, and
    // what the terminal does with it is write the clipboard's text into the pty, of which a picture
    // has none. So the only way anyone finds out which key does work is being told before they try
    // the one that does not.
    if session.image_on_clipboard {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "image on clipboard  ·  ctrl-v to paste  ",
                Style::default().fg(theme::brand_primary()),
            )))
            .alignment(Alignment::Right),
            area,
        );
    }
}

/// A count with the right noun, so a line does not read "1 chars".
fn tally(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bravebot_core::capability::Capability;
    use bravebot_core::event::Event;
    use bravebot_core::label::Label;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Render into a test backend and return the visible text.
    fn rendered(session: &Session) -> String {
        let mut terminal = Terminal::new(TestBackend::new(90, 24)).expect("terminal");
        terminal
            .draw(|frame| {
                draw(frame, session);
            })
            .expect("draw succeeds");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    /// Foreground inks of the non-space cells on every row that contains `needle`.
    fn inks_on_row_containing(session: &Session, needle: &str) -> Vec<Color> {
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| {
                draw(frame, session);
            })
            .expect("draw succeeds");
        let buffer = terminal.backend().buffer();
        let area = buffer.area();

        let mut inks = Vec::new();
        for row in 0..area.height {
            let text: String = (0..area.width)
                .map(|column| buffer.cell((column, row)).expect("cell").symbol())
                .collect();
            if !text.contains(needle) {
                continue;
            }
            inks.extend((0..area.width).filter_map(|column| {
                let cell = buffer.cell((column, row)).expect("cell");
                (cell.symbol() != " ").then_some(cell.fg)
            }));
        }
        assert!(!inks.is_empty(), "nothing containing {needle:?} was drawn");
        inks
    }

    mod scroller {
        use super::*;
        use crate::state::{Entry, Laid};
        use bravebot_agent::report::Activity;

        /// A session reading back over one prompt, one reply, and a file it was not allowed to
        /// read, with the scroller open over it.
        fn reading() -> Session {
            reading_over(24)
        }

        /// The same, over a transcript already this many rows tall when the scroller opens.
        ///
        /// The layout is noted before the mode opens, because noting it afterwards is rows
        /// arriving underneath an open scroller, which is a different thing and moves the view.
        fn reading_over(rows: u16) -> Session {
            let shown = Shown {
                origin: "notes.md".to_string(),
                reach: bravebot_agent::report::Reach::NotThePlanner,
                label: "(U,priv)".to_string(),
                preview: vec![
                    "the haystack holds a needle".to_string(),
                    // What the terminal would act on, on its way to a glyph.
                    "a needle behind \u{1b}[31m an escape".to_string(),
                ],
                lines: 40,
            };

            let mut session = Session::new("kernel-enforced");
            session.transcript.push(Entry::user("look at the notes"));
            session.transcript.push(Entry::assistant(
                "there is a needle in there".to_string(),
                Vec::new(),
            ));
            let mut read = Entry::tool(Activity::running("read", "notes.md").done("40 lines"));
            read.shown = Some(shown);
            session.transcript.push(read);
            session.note_layout(Laid {
                width: 90,
                height: 24,
                rows,
                ..Laid::default()
            });
            session.open_scroller();
            session
        }

        /// Render, and give back both what is on the screen and which cells are wearing the
        /// highlight, since a match is a thing you can see rather than a thing you are told about.
        fn screen(session: &Session) -> (String, String) {
            let mut terminal = Terminal::new(TestBackend::new(90, 24)).expect("terminal");
            terminal
                .draw(|frame| {
                    draw(frame, session);
                })
                .expect("draw succeeds");
            let buffer = terminal.backend().buffer().clone();
            let drawn: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

            // Only above the box. The caret in it is drawn reversed as well, and what this is
            // asking about is the transcript.
            let box_top = (0..buffer.area.height)
                .find(|row| buffer[(0, *row)].symbol() == "╭")
                .unwrap_or(buffer.area.height);
            let marked: String = (0..box_top)
                .flat_map(|row| (0..buffer.area.width).map(move |column| (column, row)))
                .map(|at| &buffer[at])
                .filter(|cell| cell.modifier.contains(Modifier::REVERSED))
                .map(|cell| cell.symbol())
                .collect();
            (drawn, marked)
        }

        /// Search a session for `needle`, the way the keys do it.
        fn searching(needle: &str) -> Session {
            let mut session = reading();
            session.begin_search();
            for c in needle.chars() {
                session.type_into_search(c);
            }
            session.run_search();
            session
        }

        /// Long transcript, for measuring what laying one out costs.
        fn a_long_session() -> Session {
            let mut session = Session::new("kernel-enforced");
            for n in 0..500 {
                session.transcript.push(Entry::user(format!("prompt {n}")));
                session.transcript.push(Entry::assistant(
                    format!(
                        "a reply to {n} that runs on for a while so that it wraps at eighty or \
                     ninety columns and takes more than one row of the screen to draw"
                    ),
                    Vec::new(),
                ));
            }
            session
        }

        /// Finding where the rows are is an extra pass over the transcript, on every frame the
        /// scroller is open. It has to stay in proportion to the drawing the interface already does,
        /// since the alternative is a mode that gets slower the more there is to read back through.
        ///
        /// A ratio rather than a wall clock, because what matters is that the pass is of the same
        /// order as the one beside it and not that either takes a particular number of milliseconds.
        #[test]
        fn measuring_where_the_rows_are_stays_in_proportion_to_drawing_them() {
            let at_rest = a_long_session();
            let started = std::time::Instant::now();
            let (lines, plain) = lay_out(&at_rest, 90, 24);
            // The wrap a frame at rest already pays for, and the one this is in proportion to.
            // Laying the lines out is the cheaper half of drawing them, and measuring against it
            // alone compares the new pass with something no frame has ever consisted of.
            let rows = Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .line_count(90);
            let baseline = started.elapsed();
            assert!(rows > 2000, "the transcript is not long: {rows}");

            let mut scrolling = a_long_session();
            scrolling.open_scroller();
            let started = std::time::Instant::now();
            let (_, laid) = lay_out(&scrolling, 90, 24);
            let took = started.elapsed();

            assert!(
                laid.rows > 2000,
                "the transcript is not long: {}",
                laid.rows
            );
            assert_eq!(plain.rows, 0, "the rows were counted with nobody asking");
            assert!(
                took < baseline * 4,
                "measuring took {took:?} against {baseline:?} to draw, which is out of proportion"
            );
        }

        /// Nothing has been drawn yet, so the width is zero. Laying out against it must not panic.
        #[test]
        fn measuring_before_the_first_frame_does_not_panic() {
            let mut session = Session::new("kernel-enforced");
            session.transcript.push(Entry::user("anything"));
            session.open_scroller();

            let _ = as_last_drawn(&session);
            let _ = as_text(&session);
        }

        #[test]
        fn the_scroller_says_it_is_open_and_which_key_closes_it() {
            let (drawn, _) = screen(&reading());

            assert!(drawn.contains("scroller"), "the mode does not say it is on");
            assert!(
                drawn.contains("q closes"),
                "the way out is not on the screen: {drawn}"
            );
        }

        /// The transcript at rest keeps its own line, since the bindings the scroller advertises
        /// are the ones it has taken.
        #[test]
        fn the_usual_hint_comes_back_when_the_scroller_closes() {
            let mut session = reading();
            session.close_scroller();
            let (drawn, _) = screen(&session);

            assert!(!drawn.contains("q closes"));
            assert!(
                drawn.contains(SHORTCUTS_HINT),
                "the usual line did not come back"
            );
        }

        /// Draw, and give back what the frame laid the transcript out to.
        fn drawn_over(session: &Session) -> Laid {
            let mut terminal = Terminal::new(TestBackend::new(90, 24)).expect("terminal");
            let mut laid = Laid::default();
            terminal
                .draw(|frame| laid = draw(frame, session))
                .expect("draw succeeds");
            laid
        }

        /// The box, the indicator above it and anything offered beneath it are all things to type
        /// at, and no key reaches any of them from in here. Drawing them would be rows of the
        /// screen spent inviting a keystroke that would do nothing, and the transcript is the
        /// whole of what somebody opened a pager to look at.
        #[test]
        fn the_scroller_takes_the_whole_screen_but_its_own_footer() {
            let mut session = reading();
            session.status = Status::Working;

            let (drawn, _) = screen(&session);
            assert!(
                !drawn.contains('╭'),
                "the box was drawn under the scroller: {drawn}"
            );
            assert!(
                !drawn.contains(placeholder()),
                "the box was still inviting a prompt: {drawn}"
            );

            let open = drawn_over(&session);
            assert_eq!(
                open.height, 23,
                "the transcript did not have every row but the footer"
            );

            session.close_scroller();
            let (at_rest, _) = screen(&session);
            assert!(at_rest.contains('╭'), "the box did not come back");
            assert!(
                drawn_over(&session).height < open.height,
                "the rows the box gave up did not go to the transcript"
            );
        }

        /// The indicator went with the box, so the footer is the only thing left that can say a
        /// turn is still in flight. A person reading back through one that is going wrong is
        /// reading precisely because it is going wrong, and a screen with nothing moving on it
        /// says the opposite of what is happening.
        #[test]
        fn the_scroller_says_a_turn_is_still_running() {
            let session = reading();
            let (idle, _) = screen(&session);

            let mut working = reading();
            working.status = Status::Working;
            let verb = working
                .indicator()
                .expect("a turn in flight has an indicator")
                .verb
                .to_string();
            let (running, _) = screen(&working);

            assert!(
                running.contains(&format!("{verb}…")),
                "the footer did not say the turn was still running: {running}"
            );
            assert!(
                !idle.contains(&format!("{verb}…")),
                "the footer said a turn was running with nothing in flight: {idle}"
            );
        }

        #[test]
        fn every_match_on_the_screen_is_highlighted() {
            let (_, marked) = screen(&searching("needle"));

            assert!(
                marked.contains("needle"),
                "nothing on the screen was highlighted"
            );
            assert_eq!(
                marked.matches("needle").count(),
                3,
                "not every occurrence was marked: {marked:?}"
            );
        }

        #[test]
        fn how_many_matches_there_are_is_drawn() {
            let (drawn, _) = screen(&searching("needle"));

            assert!(
                drawn.contains("1 of 3"),
                "the footer does not say where in the matches the view is: {drawn}"
            );
        }

        /// The footer is the one row of the screen the interface speaks in its own voice. A
        /// quotation there is untrusted content drawn outside a marked block, and a line that can
        /// be quoted is a line that can be written to read like the interface.
        #[test]
        fn the_search_footer_never_quotes_what_it_matched() {
            let (drawn, _) = screen(&searching("needle"));

            let footer = drawn
                .as_str()
                .split("/needle")
                .nth(1)
                .expect("the footer names what is being looked for");
            assert!(
                !footer.contains("haystack"),
                "the footer quoted the line it matched: {footer}"
            );
        }

        /// An interface that shows content and then refuses to let a person find it has protected
        /// nobody and made the audit worse.
        #[test]
        fn a_search_matches_quarantined_content_too() {
            let (_, marked) = screen(&searching("haystack"));

            assert!(
                marked.contains("haystack"),
                "a match in a quarantined block was not highlighted"
            );
        }

        /// Highlighting splits the spans a row is already made of, so nothing moves and nothing
        /// leaves the block it was drawn in: the margin is still in front of the row, and the
        /// match is still behind it.
        #[test]
        fn a_match_inside_a_quarantined_block_stays_inside_it() {
            let session = searching("haystack");
            let (lines, _) = lay_out(&session, 90, 24);

            let row = lines
                .iter()
                .find(|line| {
                    let drawn: String = line
                        .spans
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect();
                    drawn.contains("haystack")
                })
                .expect("the matched row is drawn");

            let drawn: String = row.spans.iter().map(|span| span.content.as_ref()).collect();
            assert!(
                drawn.starts_with(&format!("  {QUARANTINE_BAR} ")),
                "the match left the margin behind: {drawn}"
            );
            assert!(
                row.spans
                    .iter()
                    .any(|span| span.style.add_modifier.contains(Modifier::REVERSED)),
                "the match inside the block was not highlighted"
            );
        }

        #[test]
        fn a_quarantined_row_is_marked_in_the_scroller_as_it_is_in_the_transcript() {
            let searched = searching("haystack");
            let mut plain = reading();
            plain.close_scroller();

            let marked_rows = |session: &Session| {
                let (lines, _) = lay_out(session, 90, 24);
                lines
                    .iter()
                    .filter(|line| {
                        let drawn: String = line
                            .spans
                            .iter()
                            .map(|span| span.content.as_ref())
                            .collect();
                        drawn.starts_with(&format!("  {QUARANTINE_BAR} "))
                    })
                    .count()
            };

            assert_eq!(
                marked_rows(&searched),
                marked_rows(&plain),
                "the block lost or gained a margin under a search"
            );
        }

        /// What a needle meets is the glyph the escape was replaced with on its way to the
        /// screen. The bytes behind it are not there to be found, which is what keeps a match
        /// from being a thing nobody can see.
        #[test]
        fn a_search_matches_what_is_drawn_and_not_the_bytes_behind_it() {
            let (_, marked) = screen(&searching("\u{1b}[31m"));
            assert!(
                marked.is_empty(),
                "the raw escape was found in the drawn text: {marked:?}"
            );

            let (_, glyph) = screen(&searching("␛"));
            assert!(
                !glyph.is_empty(),
                "the glyph the escape is drawn as could not be found"
            );
        }

        #[test]
        fn a_search_that_matches_nothing_says_so_and_moves_nothing() {
            let mut session = reading_over(100);
            session.scroller_back(30);
            let looking_at = session.scroll;

            session.begin_search();
            for c in "nothing at all like this".chars() {
                session.type_into_search(c);
            }
            session.run_search();
            let found = as_last_drawn(&session);
            session.land_on_a_match(&found.matches);

            assert_eq!(
                session.scroll, looking_at,
                "a search with no answer moved the view"
            );
            let (drawn, _) = screen(&session);
            assert!(
                drawn.contains("no matches"),
                "the footer did not say there were none: {drawn}"
            );
        }

        /// Draw one frame the way the loop draws it, and give back the top row of the screen and
        /// what the frame laid the transcript out to.
        fn a_frame(session: &mut Session) -> String {
            let mut terminal = Terminal::new(TestBackend::new(90, 24)).expect("terminal");
            let mut laid = Laid::default();
            terminal
                .draw(|frame| laid = draw(frame, session))
                .expect("draw succeeds");
            session.note_layout(laid);

            let buffer = terminal.backend().buffer();
            let top: String = (0..buffer.area.width)
                .map(|column| buffer[(column, 0)].symbol())
                .collect();
            top.trim_end().to_string()
        }

        /// Holding the view has to hold it in the frame that draws it, not in the one after. The
        /// end of the transcript moves with every token, so a frame that counts back from it is
        /// drawn wherever the rows that arrived since the last frame have pushed the count, and
        /// somebody trying to read while a turn writes watches the screen slide under them.
        #[test]
        fn a_turn_writing_underneath_does_not_slide_the_view_between_frames() {
            let mut session = Session::new("kernel-enforced");
            for n in 0..60 {
                session.transcript.push(Entry::user(format!("prompt {n}")));
                session
                    .transcript
                    .push(Entry::assistant(format!("reply {n}"), Vec::new()));
            }
            session.open_scroller();

            a_frame(&mut session);
            session.scroller_back(40);
            let reading = a_frame(&mut session);
            assert!(
                reading.contains("prompt") || reading.contains("reply"),
                "the view is not on the transcript: {reading:?}"
            );

            for n in 0..3 {
                session
                    .transcript
                    .push(Entry::assistant(format!("arriving {n}"), Vec::new()));
                assert_eq!(
                    a_frame(&mut session),
                    reading,
                    "a row arriving below slid the view"
                );
            }
        }

        /// The view is held, so what arrives goes below it. Somebody reading has to be able to
        /// tell that there is something they have not read.
        #[test]
        fn the_scroller_says_more_has_arrived_below() {
            let mut session = reading_over(100);
            session.scroller_back(12);

            let (drawn, _) = screen(&session);
            assert!(
                drawn.contains("12 rows below"),
                "nothing said there was more underneath: {drawn}"
            );
        }

        /// The way out is the one row of a key list that must never be the row that did not fit.
        #[test]
        fn the_help_renders_on_a_tiny_terminal() {
            let mut session = reading();
            session.toggle_scroller_help();

            for (width, height) in [(90, 24), (40, 8), (20, 4), (10, 3), (30, 2), (30, 1)] {
                let mut terminal =
                    Terminal::new(TestBackend::new(width, height)).expect("terminal");
                terminal
                    .draw(|frame| {
                        draw(frame, &session);
                    })
                    .expect("draw succeeds");
                let drawn: String = terminal
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(
                    drawn.contains("q / esc"),
                    "the way out did not fit at {width}x{height}: {drawn}"
                );
            }
        }

        /// What goes to an editor is what the person was looking at, not a second rendering that
        /// agrees about the words and about nothing else. The margin is how untrusted content is
        /// marked, so it goes too.
        #[test]
        fn what_goes_to_the_editor_is_marked_the_way_the_screen_is() {
            let session = reading_over(40);

            let text = as_text(&session);
            let quarantined: Vec<&str> = text
                .lines()
                .filter(|line| line.contains("haystack"))
                .collect();

            assert_eq!(quarantined.len(), 1, "the block is not in the file");
            assert!(
                quarantined[0].starts_with(&format!("  {QUARANTINE_BAR} ")),
                "untrusted content reached the file unmarked: {}",
                quarantined[0]
            );
            assert!(
                !text.contains('\u{1b}'),
                "an escape reached the file as an escape"
            );
        }
    }

    mod progress {
        use super::*;
        use bravebot_agent::report::Activity;

        fn working() -> Session {
            let mut session = Session::new("kernel-enforced");
            session.type_char('a');
            session.submit();
            session
        }

        /// A call has to be legible while it runs, or the transcript is still blank during the
        /// part of a turn that takes the longest.
        #[test]
        fn a_call_in_flight_is_drawn() {
            let mut session = working();
            session.start_activity(Activity::running("Read", "src/main.rs"));
            assert!(rendered(&session).contains("Read(src/main.rs)"));
        }

        /// A person who pressed Enter has to be able to see that the words went somewhere. The
        /// line sitting quietly under the box is what this replaced, and it read as a key press
        /// that had been ignored.
        #[test]
        fn a_waiting_prompt_is_shown_as_waiting() {
            let mut session = working();
            for c in "do some long task".chars() {
                session.type_char(c);
            }
            assert!(session.queue());

            let output = rendered(&session);
            assert!(
                output.contains("do some long task"),
                "the prompt was not shown"
            );
            assert!(output.contains("QUEUED"), "nothing said it was waiting");
        }

        /// Nothing is waiting once the queue has been taken back, so nothing under the box may
        /// go on saying that something is. The rows are the only place a person can see what is
        /// still going to be sent, and rows left behind would say two prompts were on their way
        /// while both of them sat in the box.
        #[test]
        fn the_waiting_rows_go_when_the_queue_is_taken_back() {
            let mut session = working();
            for line in ["do some long task", "and another one"] {
                for c in line.chars() {
                    session.type_char(c);
                }
                assert!(session.queue());
            }
            assert!(rendered(&session).contains("QUEUED"), "nothing was waiting");

            session.unqueue();

            let output = rendered(&session);
            assert!(!output.contains("QUEUED"), "still drawn as waiting");
            assert!(
                output.contains("do some long task") && output.contains("and another one"),
                "the prompts left the screen instead of coming back to the box"
            );
        }

        /// It stops being drawn the moment its own turn starts, because from then on it is in
        /// the transcript like any other prompt and two copies is one too many.
        #[test]
        fn a_prompt_stops_waiting_once_its_turn_begins() {
            let mut session = working();
            for c in "do some long task".chars() {
                session.type_char(c);
            }
            session.queue();
            session.complete("an answer", Vec::new(), 0);
            session.send_queued().expect("it went");

            let output = rendered(&session);
            assert!(!output.contains("QUEUED"), "still drawn as waiting");
            assert!(
                output.contains("do some long task"),
                "it left the screen entirely"
            );
        }

        /// Nearly every call reads into the planner's context, so a line saying so appeared
        /// under nearly every call and distinguished nothing. It crowded out the lines that do.
        #[test]
        fn the_ordinary_landing_is_not_given_a_line_of_its_own() {
            let mut session = working();
            session.finish_activity(Activity::running("Read", "src/main.rs").done("12 lines"));
            session.landed(bravebot_agent::report::Landing::Context);

            let output = rendered(&session);
            assert!(
                output.contains("Read(src/main.rs)"),
                "the call itself is still drawn"
            );
            assert!(
                !output.contains("planner's context"),
                "the ordinary case took a row anyway"
            );
        }

        /// What the design turns on is the exception, and dropping the ordinary line is what
        /// leaves room for it to be noticed.
        #[test]
        fn a_result_the_planner_may_not_read_still_says_so() {
            let mut session = working();
            session.finish_activity(Activity::running("Read", "notes.md").done("3 lines"));
            session.landed(bravebot_agent::report::Landing::Quarantined);
            assert!(rendered(&session).contains("only an isolated processor"));

            let mut named = working();
            named.finish_activity(Activity::running("List", ".").done("4 files"));
            named.landed(bravebot_agent::report::Landing::Reserved);
            assert!(rendered(&named).contains("only its name is known"));
        }

        /// The longest silence in a turn is the one while the model writes, and it is the one
        /// with the most to show. A token counter is not what is being written.
        #[test]
        fn a_reply_is_drawn_while_it_is_still_arriving() {
            let mut session = working();
            session.streaming("Looking at the render code");
            assert!(
                rendered(&session).contains("Looking at the render code"),
                "the reply was invisible until the round was over"
            );
        }

        /// The tail and the entry that replaces it are the same words drawn the same way in the
        /// same place, which is what makes the handover invisible. Drawn differently, a finished
        /// round would make the screen jump for no reason a reader could name.
        #[test]
        fn a_reply_looks_the_same_arriving_as_it_does_arrived() {
            let mut arriving = working();
            arriving.streaming("# Heading\n\nSome **bold** prose.");

            let mut arrived = working();
            arrived.narrate("# Heading\n\nSome **bold** prose.");

            assert_eq!(rendered(&arriving), rendered(&arrived));
        }

        #[test]
        fn a_finished_call_shows_what_came_of_it() {
            let mut session = working();
            session.finish_activity(Activity::running("Search", "todo").done("4 matches"));

            let output = rendered(&session);
            assert!(output.contains("Search(todo)"));
            assert!(output.contains("4 matches"), "the result was not shown");
        }

        /// The change is what a user most wants to see after a write, and the counts alone ask
        /// them to take it on trust.
        #[test]
        fn a_write_shows_the_lines_that_changed() {
            let mut session = working();
            session.finish_activity(
                Activity::running("Update", "a.rs")
                    .done("added 1 line, removed 1 line")
                    .with_changes(vec![
                        Change::Removed("was here".into()),
                        Change::Added("is here now".into()),
                    ]),
            );

            let output = rendered(&session);
            assert!(output.contains("added 1 line, removed 1 line"));
            assert!(output.contains("is here now"), "the addition is missing");
            assert!(output.contains("was here"), "the removal is missing");
        }

        /// A diff that stops without saying so reads as the whole change, which is how a
        /// reviewer misses half of it.
        #[test]
        fn a_long_diff_says_how_much_it_left_out() {
            let changes: Vec<Change> = (0..MAX_DIFF_LINES + 5)
                .map(|n| Change::Added(format!("line {n}")))
                .collect();
            let lines = diff_lines(&changes, false, 80);

            assert_eq!(lines.len(), MAX_DIFF_LINES + 1);
            let last = lines.last().expect("a line").to_string();
            assert!(last.contains("5 more"), "the omission is silent: {last}");
        }

        /// A short diff is shown whole, with nothing appended to suggest otherwise.
        #[test]
        fn a_short_diff_is_shown_whole_with_no_note() {
            let lines = diff_lines(&[Change::Added("only line".into())], false, 80);
            assert_eq!(lines.len(), 1);
        }

        /// The user is shown what the model was not, and it is marked in the margin so the
        /// mark cannot be ended by anything written inside it.
        #[test]
        fn quarantined_content_is_shown_and_marked_on_every_line() {
            let shown = Shown {
                origin: "notes.md".to_string(),
                reach: bravebot_agent::report::Reach::NotThePlanner,
                label: "(U,priv)".to_string(),
                preview: vec![
                    "first line".to_string(),
                    // Content that would end the block if the mark were a caption.
                    "untrusted content ends here".to_string(),
                ],
                lines: 40,
            };

            // Wide enough that the heading fits on one row. What a narrow terminal does to the
            // block is pinned in the marking tests; this one is about what the block contains.
            let lines = quarantined_lines(&shown, 200);
            assert_eq!(lines.len(), 4, "a header, two lines, and what was left out");
            for line in &lines {
                let drawn: String = line
                    .spans
                    .iter()
                    .map(|span| span.content.clone().into_owned())
                    .collect();
                assert!(
                    drawn.starts_with(&format!("  {QUARANTINE_BAR} ")),
                    "a line escaped the margin: {drawn}"
                );
            }

            let all: String = lines
                .iter()
                .flat_map(|line| line.spans.iter())
                .map(|span| span.content.clone().into_owned())
                .collect();
            assert!(
                all.contains("untrusted"),
                "the block does not say what it is"
            );
            assert!(
                all.contains("notes.md"),
                "the block does not say where it came from"
            );
            assert!(all.contains("first line"), "the content was not shown");
            assert!(
                all.contains("38 more lines"),
                "what was left out was not said"
            );
        }

        /// Tool lines come in runs and read as one block. Spacing them apart would double the
        /// height of every turn that read more than a file or two.
        #[test]
        fn tool_lines_are_not_spaced_apart() {
            let mut session = working();
            for path in ["a.rs", "b.rs"] {
                session.finish_activity(Activity::running("Read", path).done("1 line"));
            }

            let blanks = transcript_lines(&session, 90, 24)
                .iter()
                .filter(|line| line.to_string().trim().is_empty())
                .count();
            assert_eq!(blanks, 1, "the prompt's own blank line is the only one");
        }
    }

    /// The end of a path is the part that names the file, so a row too narrow for the whole thing
    /// keeps the end. Cutting the other way leaves every attachment in a deep directory reading as
    /// the same prefix with the filename gone, which is the one thing the row exists to show.
    #[test]
    fn a_path_too_long_for_its_row_keeps_the_end_of_it() {
        let path = "/very/deeply/nested/directory/screenshot.png";
        let shown = tail_of(path, 20);
        assert!(shown.chars().count() <= 20, "{shown}");
        assert!(shown.ends_with("screenshot.png"), "{shown}");
        assert!(shown.starts_with('…'), "nothing said it was cut: {shown}");
    }

    #[test]
    fn a_path_that_fits_is_left_alone() {
        assert_eq!(tail_of("/tmp/a.png", 40), "/tmp/a.png");
    }

    /// A user about to send a file should be able to see which file: `[Image #1]` alone does not
    /// tell a screenshot from a private key.
    #[test]
    fn an_attached_file_is_named_under_the_box() {
        let directory = std::env::temp_dir().join("bravebot-render-attached");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("scratch");
        std::fs::write(directory.join("shot.png"), [0x89u8, 0x50]).expect("write");

        let mut session = Session::new("none").in_workspace(&directory);
        session.drop_files(&directory.join("shot.png").to_string_lossy());

        let output = rendered(&session);
        assert!(output.contains("[Image #1]"), "no marker drawn: {output}");
        assert!(
            output.contains("shot.png"),
            "the file was not named: {output}"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// An empty box says nothing about what it takes, and the one thing a person opening this for
    /// the first time needs to know is that they may simply ask.
    #[test]
    fn an_empty_box_says_what_it_is_for() {
        let session = Session::new("none");
        assert!(
            rendered(&session).contains("Ask Brave Bot to do anything"),
            "the box said nothing"
        );
    }

    /// The prompt character stays put and the invitation stands behind it, in the column the first
    /// character typed lands in. Drawn over the prompt instead, the whole row shifted the moment
    /// somebody typed, which reads as the box jumping under their hands.
    #[test]
    fn the_invitation_stands_where_the_first_character_will() {
        const WIDTH: u16 = 90;

        let empty = Session::new("none");
        let invited = rendered_at(&empty, WIDTH, 24)
            .find("Ask Brave Bot")
            .expect("the invitation was not drawn");

        let mut typed = Session::new("none");
        typed.type_char('f');
        let landed = rendered_at(&typed, WIDTH, 24)
            .find("> f")
            .expect("the typed line was not drawn")
            + "> ".len();

        assert_eq!(
            invited % WIDTH as usize,
            landed % WIDTH as usize,
            "the row moved when the first character was typed"
        );
    }

    /// It stands in for the line rather than being part of it, so the first character typed takes
    /// its place. Left drawn, it would read as text the person now has to delete.
    #[test]
    fn the_invitation_goes_the_moment_anything_is_typed() {
        let mut session = Session::new("none");
        session.type_char('h');

        let output = rendered(&session);
        assert!(
            !output.contains("Ask Brave Bot"),
            "the invitation outstayed the empty line: {output}"
        );
        assert!(output.contains('h'), "the character was not drawn");
    }

    /// And it comes back when the line goes, since the box is empty again and says so again.
    #[test]
    fn the_invitation_comes_back_when_the_line_does_not() {
        let mut session = Session::new("none");
        session.type_char('h');
        session.clear_input();

        assert!(rendered(&session).contains("Ask Brave Bot to do anything"));
    }

    /// Shell mode has its own prompt, its own colour and its own hint line, all saying the line
    /// goes to a shell. An invitation to ask a question would contradict every one of them.
    #[test]
    fn the_invitation_is_not_offered_where_the_line_is_a_command() {
        let mut session = Session::new("none");
        session.type_char('!');
        assert!(session.shell, "shell mode was not armed");

        let output = rendered(&session);
        assert!(
            !output.contains("Ask Brave Bot"),
            "offered in shell mode: {output}"
        );
    }

    /// A key pressed to stop something emptied the box and said nothing, and the same key pressed
    /// again ends the session. The line under the box is where that is said, because it is where a
    /// person is already looking when the words they were writing vanish.
    #[test]
    fn the_way_out_is_offered_where_the_line_went() {
        let mut session = Session::new("none");
        session.type_char('x');
        assert!(
            !rendered(&session).contains("ctrl-c again"),
            "offered before anything was cleared"
        );

        session.clear_input();
        session.cleared_by_interrupt = true;

        let hint = hint_row_at(&session, 90, 24);
        assert!(hint.contains("ctrl-c again to exit"), "{hint}");
    }

    /// Rubbing the marker out is the only way a person has to change their mind about a file, and
    /// the row under the box is the only place they can see whether it worked. Left drawn, it says
    /// a file is going that is not.
    #[test]
    fn deleting_the_marker_takes_the_row_out_from_under_the_box() {
        let directory = std::env::temp_dir().join("bravebot-render-attached-deleted");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("scratch");
        std::fs::write(directory.join("shot.png"), [0x89u8, 0x50]).expect("write");

        let mut session = Session::new("none").in_workspace(&directory);
        session.drop_files(&directory.join("shot.png").to_string_lossy());
        assert!(
            rendered(&session).contains("shot.png"),
            "the file was never named"
        );

        while !session.input().is_empty() {
            session.backspace();
        }
        for c in "what is in the directory?".chars() {
            session.type_char(c);
        }

        let output = rendered(&session);
        assert!(
            !output.contains("shot.png"),
            "the attachment was still drawn: {output}"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// An attachment belongs to the line still in the box, and a queued prompt has already gone.
    /// Drawn the other way round, a file staged during a turn sat under prompts it was no part
    /// of, which reads as though it went with one of them.
    #[test]
    fn what_is_attached_is_drawn_above_what_is_waiting() {
        let directory = std::env::temp_dir().join("bravebot-render-attached-order");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("scratch");
        std::fs::write(directory.join("shot.png"), [0x89u8, 0x50]).expect("write");

        let mut session = Session::new("none").in_workspace(&directory);
        session.type_char('a');
        session.submit();
        for c in "the waiting prompt".chars() {
            session.type_char(c);
        }
        assert!(session.queue(), "nothing was queued");
        session.drop_files(&directory.join("shot.png").to_string_lossy());

        let output = rendered(&session);
        let attached = output.find("shot.png").expect("the file was not named");
        let waiting = output.find("QUEUED").expect("nothing said it was waiting");
        assert!(
            attached < waiting,
            "the attachment was drawn under the queue: {output}"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The key took a line off the screen, so something has to say where it went. Without the row,
    /// a press that emptied the box is indistinguishable from one that threw the words away, and the
    /// only way to find out which it was is to press again and hope.
    #[test]
    fn a_stashed_line_is_named_under_the_box() {
        let mut session = Session::new("none");
        for c in "the thought I put away".chars() {
            session.type_char(c);
        }
        session.stash();

        let output = rendered(&session);
        assert!(
            output.contains("the thought I put away"),
            "the line went nowhere anybody could see: {output}"
        );
        assert!(
            output.contains("ctrl-s"),
            "nothing said which key brings it back: {output}"
        );
    }

    /// The row goes when the line comes back, because the line is in the box and the row would be
    /// saying a second copy is waiting somewhere.
    #[test]
    fn the_row_goes_when_the_stashed_line_comes_back() {
        let mut session = Session::new("none");
        for c in "a thought".chars() {
            session.type_char(c);
        }
        session.stash();
        session.stash();

        let output = rendered(&session);
        assert!(
            !output.contains("stashed"),
            "the row outlived the stash: {output}"
        );
    }

    /// A paragraph put away is one row here. It is a reminder that something is waiting rather than
    /// the line itself, and drawn whole it would push the transcript off the screen to say so.
    #[test]
    fn a_stashed_paragraph_is_one_row() {
        let mut session = Session::new("none");
        session.paste_text("first line\nsecond line\nthird line");
        session.stash();

        assert_eq!(
            stashed_lines(&session, 90).len(),
            1,
            "a paragraph took more than its row"
        );
    }

    /// No row may be wider than the terminal. One that is wraps, which takes a row the layout did not
    /// reserve and pushes the hint line off the screen. Swept across every width down to nothing,
    /// since the arithmetic that drops the reminder to keep the words is where this would go wrong.
    #[test]
    fn no_stashed_row_runs_past_the_edge() {
        let mut session = Session::new("none");
        for c in "rewrite the parser to use the new lexer entirely".chars() {
            session.type_char(c);
        }
        session.stash();

        for width in 0..=200u16 {
            for line in stashed_lines(&session, width) {
                let drawn = line.to_string();
                assert!(
                    drawn.chars().count() <= width as usize,
                    "a row ran past {width}: {drawn}"
                );
            }
        }
    }

    /// Which line is waiting is the part only this row can say, and the key is in the list `?` puts
    /// up as well. So where the two cannot both fit the words stay: cut to `ctrl-s to bring i`, the
    /// reminder is worse than absent.
    #[test]
    fn a_narrow_terminal_keeps_the_words_and_drops_the_reminder() {
        let mut session = Session::new("none");
        for c in "rewrite the parser".chars() {
            session.type_char(c);
        }
        session.stash();

        let narrow = stashed_lines(&session, 30)
            .first()
            .expect("no row was drawn")
            .to_string();
        assert!(
            narrow.contains("rewrite"),
            "the line was crushed out of its own row: {narrow}"
        );
        assert!(
            !narrow.contains("ctrl-s to bring i"),
            "the reminder was drawn cut off: {narrow}"
        );
    }

    /// The row belongs to the box: it is what the next press of one key puts back into it. Above the
    /// queue, whose prompts have gone, and below the attachments, which belong to the line in the
    /// box now.
    #[test]
    fn what_is_stashed_is_drawn_between_the_attachments_and_the_queue() {
        let directory = std::env::temp_dir().join("bravebot-render-stash-order");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("scratch");
        std::fs::write(directory.join("shot.png"), [0x89u8, 0x50]).expect("write");

        let mut session = Session::new("none").in_workspace(&directory);
        session.type_char('a');
        session.submit();
        for c in "the waiting prompt".chars() {
            session.type_char(c);
        }
        assert!(session.queue(), "nothing was queued");
        for c in "the stashed thought".chars() {
            session.type_char(c);
        }
        session.stash();
        session.drop_files(&directory.join("shot.png").to_string_lossy());

        let output = rendered(&session);
        let attached = output.find("shot.png").expect("the file was not named");
        let stashed = output
            .find("the stashed thought")
            .expect("the stashed line was not named");
        let waiting = output.find("QUEUED").expect("nothing said it was waiting");
        assert!(
            attached < stashed && stashed < waiting,
            "the rows beneath the box were out of order: {output}"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The whole point of folding a paste is the screen: a stack trace in the box pushes off the
    /// reply it was pasted to ask about, so the box shows the marker and none of the lines.
    #[test]
    fn a_folded_paste_keeps_its_lines_off_the_screen() {
        let mut session = Session::new("none");
        session.paste_text("first\nsecond\nthird\nfourth\n");

        let output = rendered(&session);
        assert!(
            output.contains("[Pasted text #1 +4 lines]"),
            "no marker drawn: {output}"
        );
        assert!(
            !output.contains("second"),
            "the paste took the screen: {output}"
        );
    }

    #[test]
    fn an_empty_session_shows_a_greeting_and_hint() {
        let session = Session::new("kernel-enforced");
        let output = rendered(&session);
        assert!(output.contains("Ask a question"), "no hint shown");
        assert!(output.contains("bravebot"));
    }

    /// The trust answer is noted before the first frame is drawn, so by the time anyone sees the
    /// opening screen the transcript is not empty. Testing for an empty one meant the mark was
    /// never drawn in a real session at all, only in tests that forgot to note anything.
    #[test]
    fn the_mark_survives_the_note_that_starting_up_leaves() {
        let mut session = Session::new("kernel-enforced");
        session.note("trusting /tmp/x");

        let output = rendered(&session);
        assert!(output.contains('█'), "the mark was not drawn: {output}");
        assert!(output.contains("trusting /tmp/x"), "the note was lost");
        assert!(output.contains("Ask a question"), "no hint shown");
    }

    /// And it gives way the moment there is a conversation to read instead.
    #[test]
    fn the_mark_goes_when_the_first_prompt_is_sent() {
        let mut session = Session::new("kernel-enforced");
        session.note("trusting /tmp/x");
        for character in "hello".chars() {
            session.type_char(character);
        }
        session.submit();

        let output = rendered(&session);
        assert!(!output.contains('█'), "the mark outstayed its welcome");
        assert!(
            !output.contains("Ask a question"),
            "the hint outstayed its welcome"
        );
    }

    /// Confinement belongs on screen at all times, not only in doctor.
    /// A copy is otherwise silent, and a clipboard that may or may not have taken something is
    /// worse than none: the user pastes somewhere to find out whether it worked.
    #[test]
    fn a_copy_says_how_much_it_took() {
        let mut session = Session::new("kernel-enforced");
        session.copied = Some(106);
        assert!(
            rendered(&session).contains("106 chars to clipboard"),
            "the copy was not reported"
        );
    }

    /// Nobody presses a chord nothing mentions, and Command-V is the one the fingers already know:
    /// it reaches the terminal, not this process, and quietly drops the picture. Saying so before
    /// they try it is the only moment that helps.
    #[test]
    fn a_picture_on_the_clipboard_says_which_key_carries_it() {
        let mut session = Session::new("kernel-enforced");
        session.image_on_clipboard = true;
        let output = rendered(&session);
        assert!(output.contains("image on clipboard"), "no hint shown");
        assert!(output.contains("ctrl-v"), "the hint did not name the key");
    }

    /// One line, two things that want it. What a copy took is the answer to something the user did
    /// a moment ago, and the hint is standing advice, so the answer wins.
    #[test]
    fn a_copy_is_reported_over_the_clipboard_hint() {
        let mut session = Session::new("kernel-enforced");
        session.image_on_clipboard = true;
        session.copied = Some(12);
        let output = rendered(&session);
        assert!(output.contains("12 chars to clipboard"));
        assert!(
            !output.contains("image on clipboard"),
            "both were drawn at once"
        );
    }

    #[test]
    fn a_copy_of_one_character_reads_naturally() {
        let mut session = Session::new("kernel-enforced");
        session.copied = Some(1);
        assert!(rendered(&session).contains("1 char to clipboard"));
    }

    /// The highlight is painted over the finished frame, so what is drawn under it keeps its own
    /// colours and only the background says what is selected.
    #[test]
    fn a_selection_is_drawn_over_whatever_it_covers() {
        let mut session = Session::new("kernel-enforced");
        session.begin_selection(0, 0);
        session.extend_selection(0, 5);

        let mut terminal = Terminal::new(TestBackend::new(90, 24)).expect("terminal");
        terminal
            .draw(|frame| {
                draw(frame, &session);
            })
            .expect("draw succeeds");

        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer.cell((0, 0)).expect("cell").bg,
            theme::brand_primary()
        );
        assert_ne!(
            buffer.cell((6, 0)).expect("cell").bg,
            theme::brand_primary()
        );
    }

    /// The way to the bindings sits after the confinement, so it is the first thing a narrow
    /// terminal cuts off. Asserted at a width an ordinary terminal actually has, and against that
    /// row alone: the heading names the confinement too, so matching it anywhere on the screen says
    /// nothing about whether the hint line still carries it. Widening this to make it pass would be
    /// hiding the truncation, and so would asserting against the whole screen again.
    #[test]
    fn the_hint_line_says_how_to_find_the_bindings_and_reports_confinement() {
        let hint = hint_row_at(&Session::new("kernel-enforced"), 120, 24);
        assert!(hint.contains(SHORTCUTS_HINT), "{hint}");
        assert!(hint.contains("confinement kernel-enforced"), "{hint}");
    }

    /// The point of moving the bindings off the hint line: it has to fit where it used to be cut,
    /// which is the narrow terminal somebody is actually working in.
    #[test]
    fn the_hint_line_fits_a_narrow_terminal_whole() {
        let hint = hint_row_at(&Session::new("kernel-enforced"), 80, 24);
        assert!(
            hint.contains(SHORTCUTS_HINT),
            "the line is still cut: {hint}"
        );
    }

    /// A `?` answers what the keys are, which is what the hint line stopped listing.
    #[test]
    fn a_question_mark_lists_every_shortcut() {
        let mut session = Session::new("none");
        session.type_char('?');
        let output = rendered_at(&session, 120, 40);

        for (key, meaning) in SHORTCUTS {
            assert!(output.contains(key), "{key} missing");
            assert!(output.contains(meaning), "{key} has no meaning on screen");
        }
    }

    /// The list folds into columns, so it does not push the transcript off a short terminal the way
    /// one row per binding would.
    #[test]
    fn the_shortcuts_use_fewer_rows_where_the_width_allows() {
        let wide = shortcut_lines(200).len();
        let narrow = shortcut_lines(40).len();
        assert!(wide < narrow, "{wide} rows wide, {narrow} narrow");
        assert_eq!(narrow, SHORTCUTS.len(), "a narrow terminal cut a binding");
    }

    /// No row may be wider than the terminal. A row that is wraps, which puts the list a row over
    /// the height the layout reserved for it and pushes the hint line off the screen. Swept across
    /// every width down to nothing, since the arithmetic that fits the columns is where this would
    /// go wrong.
    #[test]
    fn no_shortcut_row_runs_past_the_edge() {
        for width in 0..=200u16 {
            for line in shortcut_lines(width) {
                let drawn = line.to_string();
                assert!(
                    drawn.chars().count() <= width as usize,
                    "a row ran past {width}: {drawn}"
                );
            }
        }
    }

    /// Nothing to choose among them, so the keys that walk a list keep their usual meanings: Tab
    /// completing here would rewrite the line, and Up would stop reaching the history.
    #[test]
    fn the_shortcuts_are_not_something_to_complete() {
        let mut session = Session::new("none");
        session.type_char('?');
        assert!(!session.is_completing());
        assert!(!session.completion_would_change_the_line());
    }

    /// The hint line names the key that opens the list, and the list names it too. They are separate
    /// strings, so this is what keeps them from disagreeing about which key to press.
    #[test]
    fn the_hint_and_the_list_name_the_same_key() {
        let key = SHORTCUTS
            .iter()
            .find(|(_, meaning)| *meaning == "this list")
            .map(|(key, _)| *key)
            .expect("the list lists itself");
        assert!(
            SHORTCUTS_HINT.starts_with(key),
            "the hint says {SHORTCUTS_HINT:?} and the list says {key:?}"
        );

        // And it is the key that actually opens it, rather than one the list merely claims.
        let mut session = Session::new("none");
        for c in key.chars() {
            session.type_char(c);
        }
        assert!(session.shortcuts, "{key:?} did not open the list");
    }

    /// Typing a slash offers every command with what it does, which is the thing the hint line
    /// stopped listing.
    #[test]
    fn a_slash_offers_every_command_and_what_it_does() {
        let mut session = Session::new("none");
        session.type_char('/');
        let output = rendered_at(&session, 120, 24);

        for command in crate::app::commands() {
            assert!(output.contains(command.name), "{} missing", command.name);
            assert!(
                output.contains(command.description),
                "{} has no description on screen",
                command.name
            );
        }
    }

    /// The figure a person needs to decide whether to compact by hand, on the line they already
    /// read for the state of the session.
    #[test]
    fn the_hint_line_says_how_full_the_context_is() {
        let mut session = Session::new("none");
        session.measured(62_000, 100_000);
        let output = rendered_at(&session, 120, 24);

        assert!(output.contains("context 62%"), "{output}");
    }

    /// Before anything has been measured there is no figure, and a gauge at zero would be a claim
    /// about a context nobody counted.
    #[test]
    fn the_hint_line_says_nothing_about_an_unmeasured_context() {
        let output = rendered_at(&Session::new("none"), 120, 24);
        assert!(!output.contains("context"), "{output}");
    }

    /// Narrowing shows only what still matches, so the list answers what the half-typed word could
    /// become rather than repeating the whole set.
    #[test]
    fn a_narrowed_list_offers_only_what_still_matches() {
        let mut session = Session::new("none");
        for c in "/cl".chars() {
            session.type_char(c);
        }
        let output = rendered_at(&session, 120, 24);

        assert!(output.contains("/clear"), "{output}");
        assert!(
            !output.contains("/model"),
            "a command that cannot match was offered"
        );
        assert!(
            !output.contains("/rename"),
            "a command that cannot match was offered"
        );
    }

    /// The descriptions line up in one column, and stay there as the list narrows. Measured from
    /// every command rather than the visible ones, or they would slide sideways with each letter.
    #[test]
    fn the_descriptions_share_a_column_however_far_the_list_narrows() {
        // Read off the buffer by cell, since a rendered row is one cell per column and a symbol
        // may be more than one byte.
        let column_of = |line: &str| -> usize {
            let mut session = Session::new("none");
            for c in line.chars() {
                session.type_char(c);
            }
            let mut terminal = Terminal::new(TestBackend::new(96, 14)).expect("terminal");
            terminal
                .draw(|frame| {
                    draw(frame, &session);
                })
                .expect("draw succeeds");
            let buffer = terminal.backend().buffer();
            for row in 0..14u16 {
                let text: String = (0..96u16)
                    .map(|column| buffer.cell((column, row)).expect("cell").symbol())
                    .collect();
                if let Some(at) = text.find("Call this conversation") {
                    // The row is ASCII up to the description apart from the marker, so a character
                    // count to that point is the column it sits in.
                    return text[..at].chars().count();
                }
            }
            panic!("the /rename row was not on screen");
        };

        assert_eq!(
            column_of("/"),
            column_of("/rename"),
            "the description moved as the list narrowed"
        );
    }

    /// With nothing being typed the rows go, giving the height back to the transcript rather than
    /// leaving a gap where the list was.
    #[test]
    fn an_ordinary_prompt_offers_nothing() {
        let mut session = Session::new("none");
        for c in "what does this do".chars() {
            session.type_char(c);
        }
        let output = rendered_at(&session, 120, 24);
        assert!(!output.contains("Choose which model"), "{output}");
    }

    #[test]
    fn a_working_session_shows_the_indicator() {
        let mut session = Session::new("partial");
        session.type_char('a');
        session.submit();
        let output = rendered(&session);

        let indicator = session.indicator().expect("a turn is in flight");
        assert!(
            output.contains(indicator.verb.as_ref()),
            "the indicator's word is missing: {output}"
        );
        assert!(
            output.contains(indicator.glyph),
            "the spinner glyph is missing: {output}"
        );
        // Shown from the start, so a slow first response still reads as alive.
        assert!(output.contains("0s"), "no elapsed time: {output}");
    }

    /// Tokens accumulated earlier in the session appear once there are some to report.
    #[test]
    fn the_indicator_reports_accumulated_tokens() {
        let mut session = Session::new("partial");
        session.type_char('a');
        session.submit();
        session.complete("first reply", Vec::new(), 38_300);
        session.type_char('b');
        session.submit();

        let output = rendered(&session);
        assert!(output.contains("38.3k tokens"), "no token count: {output}");
    }

    /// An idle session shows no indicator at all.
    #[test]
    fn an_idle_session_shows_no_indicator() {
        let session = Session::new("partial");
        assert!(session.indicator().is_none());
    }

    #[test]
    fn the_transcript_distinguishes_speakers() {
        let mut session = Session::new("none");
        for c in "what is this".chars() {
            session.type_char(c);
        }
        session.submit();
        session.complete("it is a project", Vec::new(), 0);

        let output = rendered(&session);
        assert!(output.contains("what is this"), "the prompt is not echoed");
        assert!(output.contains("it is a project"));
        // The user's line is prefixed, the assistant's is marked.
        assert!(output.contains("> what is this"));
        assert!(output.contains(TURN_MARKER));
    }

    /// Markdown markers belong to the model's formatting, not to what it said.
    #[test]
    fn a_reply_is_shown_as_markdown_rather_than_its_markers() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit();
        session.complete("edit **main.rs** now", Vec::new(), 0);

        let output = rendered(&session);
        assert!(output.contains("edit main.rs now"), "not styled: {output}");
        assert!(
            !output.contains("**"),
            "the markers are still shown: {output}"
        );
    }

    /// A table shown as its source is the one markdown form that reads worse than prose, since
    /// the cells are not as wide as their headings and nothing lines up.
    #[test]
    fn a_reply_containing_a_table_is_drawn_as_one() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit();
        session.complete(
            "| gate | answer |\n| --- | --- |\n| edit_file | refuses |",
            Vec::new(),
            0,
        );

        let output = rendered_at(&session, 90, 24);
        assert!(output.contains("refuses"), "not drawn: {output}");
        assert!(
            output.contains('\u{2500}'),
            "no rule under the header: {output}"
        );
        assert!(!output.contains('|'), "the pipes are still shown: {output}");
    }

    /// Refusing is a supported outcome, not a failure. The reader gets the model's own
    /// characters, which is what they got before tables were drawn at all.
    #[test]
    fn a_table_too_wide_for_the_terminal_falls_back_to_its_source() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit();
        session.complete(
            "| a | b | c | d | e | f |\n| --- | --- | --- | --- | --- | --- |\n| 1 | 2 | 3 | 4 | 5 | 6 |",
            Vec::new(),
            0,
        );

        let output = rendered_at(&session, 20, 24);
        assert!(output.contains('|'), "the source was not shown: {output}");
    }

    /// A table is a block in the middle of a reply, not the whole of it, so the prose either
    /// side of one has to survive being skipped over.
    #[test]
    fn prose_around_a_table_is_still_prose() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit();
        session.complete(
            "before the table\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\nafter the table",
            Vec::new(),
            0,
        );

        let output = rendered_at(&session, 90, 24);
        assert!(
            output.contains("before the table"),
            "lost the lead-in: {output}"
        );
        assert!(
            output.contains("after the table"),
            "lost the follow-on: {output}"
        );
        assert!(output.contains(TURN_MARKER));
    }

    /// The common false positive: a vertical bar in a sentence is punctuation, and rearranging
    /// the sentence into columns because of it would lose what the model said.
    #[test]
    fn a_pipe_in_a_sentence_is_still_a_pipe() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit();
        session.complete("choose one | or the other", Vec::new(), 0);

        assert!(rendered_at(&session, 90, 24).contains("choose one | or the other"));
    }

    /// Untrusted content is drawn raw behind the bar. Columns inside a marked block are
    /// structure the content chose for itself, and the margin is the one thing it may not
    /// imitate.
    #[test]
    fn a_quarantined_preview_is_never_drawn_as_a_table() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit();
        session.complete("read it", Vec::new(), 0);
        session.transcript.last_mut().expect("an entry").shown = Some(Shown {
            origin: "notes.md".into(),
            reach: bravebot_agent::report::Reach::NotThePlanner,
            label: "(U,priv)".to_string(),
            preview: vec![
                "| a | b |".into(),
                "| --- | --- |".into(),
                "| 1 | 2 |".into(),
            ],
            lines: 3,
        });

        // Wide enough that the heading fits on one row, since the count below is of the block's
        // lines rather than of the rows a narrow terminal breaks them into.
        let lines = transcript_lines(&session, 200, 24);
        let marked: Vec<String> = lines
            .iter()
            .map(|line| line.to_string())
            .filter(|line| line.contains(QUARANTINE_BAR))
            .collect();
        assert_eq!(marked.len(), 4, "the block lost a line: {marked:?}");
        for line in &marked {
            assert!(
                line.starts_with(&format!("  {QUARANTINE_BAR} ")),
                "unmarked: {line}"
            );
        }
        assert!(
            marked.iter().any(|line| line.contains("---")),
            "the preview was reshaped: {marked:?}"
        );
    }

    /// `LEAD` is the width a table is budgeted against. If the marker were wider than it, the
    /// first row would overflow and the paragraph would wrap the columns it just aligned.
    #[test]
    fn the_lead_is_as_wide_as_the_marker_it_stands_for() {
        assert_eq!(crate::wrap::display_width(TURN_MARKER) + 1, LEAD);
    }

    /// The trail is hidden until asked for, so ordinary use is not noisy.
    #[test]
    fn the_trail_is_hidden_by_default() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit();
        session.complete(
            "reply",
            vec![Event::Observed {
                capability: Capability::FileRead,
                label: Label::untrusted_private(),
            }],
            0,
        );

        assert!(!rendered(&session).contains("(U,priv)"));
        session.toggle_trail();
        assert!(
            rendered(&session).contains("(U,priv)"),
            "the trail did not appear"
        );
    }

    /// A call read back off disk is drawn, where before it was dropped and a resumed transcript
    /// said the model answered without saying it had read anything.
    #[test]
    fn a_recalled_call_is_shown() {
        let mut session = Session::new("none");
        session
            .transcript
            .push(crate::state::Entry::recalled_tool("Read(src/main.rs)"));

        assert!(rendered(&session).contains("Read(src/main.rs)"));
    }

    /// And drawn without the marker a live call earns. Green says the call finished cleanly, and
    /// nothing in the record says that: what it holds is that the call was made.
    #[test]
    fn a_recalled_call_does_not_claim_an_outcome() {
        let mut session = Session::new("none");
        session
            .transcript
            .push(crate::state::Entry::recalled_tool("Read(src/main.rs)"));
        let recalled = marker_style(&session);

        let mut session = Session::new("none");
        session.start_activity(Activity::running("Read", "src/main.rs").done("12 lines"));
        assert_ne!(
            recalled,
            marker_style(&session),
            "a recalled call was drawn as one that finished cleanly"
        );
        assert_eq!(recalled, dim());
    }

    /// The style of the marker on the first drawn line.
    fn marker_style(session: &Session) -> Style {
        transcript_lines(session, 90, 24)
            .first()
            .and_then(|line| line.spans.first())
            .map(|span| span.style)
            .expect("something was drawn")
    }

    #[test]
    fn the_hint_reflects_the_trail_state() {
        let mut session = Session::new("none");
        assert!(rendered(&session).contains("show trail"));
        session.toggle_trail();
        assert!(rendered(&session).contains("hide trail"));
    }

    /// A refusal must be visible, since it is the most important thing on screen.
    #[test]
    fn a_blocked_gate_is_shown_in_the_trail() {
        let mut session = Session::new("none");
        session.toggle_trail();
        session.type_char('a');
        session.submit();
        session.complete(
            "refused",
            vec![Event::GateBlocked {
                gate: "action",
                detail: String::new(),
                reason: "injection blocked".into(),
            }],
            0,
        );

        assert!(rendered(&session).contains("injection blocked"));
    }

    #[test]
    fn a_system_note_is_shown() {
        let mut session = Session::new("none");
        session.note("confinement unavailable");
        assert!(rendered(&session).contains("confinement unavailable"));
    }

    /// Yellow is spoken for: a call still running, and the margin down every block of content the
    /// planner may not read. A note the interface makes about the session is neither, and drawing
    /// it in the same ink said that the trust answer was quarantined content.
    #[test]
    fn a_system_note_is_not_drawn_in_the_ink_that_marks_untrusted_content() {
        let mut session = Session::new("none");
        session.note("trusting /tmp/x");

        let inks = inks_on_row_containing(&session, "trusting");
        assert!(
            inks.iter().all(|ink| *ink == theme::note()),
            "the note was not drawn in one ink of its own: {inks:?}"
        );
        assert!(
            !inks.contains(&theme::running()),
            "the note is still yellow"
        );
    }

    /// Long replies must not panic the renderer.
    #[test]
    fn a_long_reply_renders() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit();
        session.complete("x".repeat(5_000), Vec::new(), 0);
        assert!(rendered(&session).contains('x'));
    }

    /// A narrow terminal is a normal condition, not a crash.
    #[test]
    fn a_tiny_terminal_renders() {
        let session = Session::new("none");
        let mut terminal = Terminal::new(TestBackend::new(10, 5)).expect("terminal");
        terminal
            .draw(|frame| {
                draw(frame, &session);
            })
            .expect("draw must not panic on a small area");
    }

    /// The end of a reply that wraps must be on the screen when it arrives.
    ///
    /// The offset used to be computed from the number of lines rather than the number of rows
    /// they occupy once wrapped, so a reply with long lines ended that many rows below the
    /// bottom of the window. It appeared only when the next message pushed it up, which is
    /// exactly how it was reported: "they only showed up after I said are you done now?".
    #[test]
    fn the_end_of_a_wrapped_reply_is_visible_when_it_arrives() {
        let mut session = Session::new("test");
        // Enough turns to fill the window, each reply long enough to wrap several times: the
        // gap between lines and rows is what used to push the end off the bottom.
        for turn in 0..6 {
            session.type_char('x');
            session.submit();
            session.complete(
                format!("reply {turn}: {}", "wrapping words ".repeat(12)),
                Vec::new(),
                0,
            );
        }
        session.type_char('x');
        session.submit();
        session.complete(
            format!("{}\nTHE LAST LINE", "more wrapping words ".repeat(12)),
            Vec::new(),
            0,
        );

        let drawn = rendered_at(&session, 60, 20);
        assert!(
            drawn.contains("THE LAST LINE"),
            "the end of the reply was below the window: {drawn}"
        );
    }

    /// Scrolling back must actually change what is shown, or the binding is decorative.
    #[test]
    fn scrolling_back_changes_the_view() {
        let mut session = Session::new("none");
        for turn in 0..40 {
            session.type_char('q');
            session.submit();
            session.complete(format!("reply number {turn}"), Vec::new(), 0);
        }

        let bottom = rendered(&session);
        assert!(bottom.contains("reply number 39"), "latest reply not shown");

        // A modest scroll moves into the middle of the history, not to its start: each
        // turn occupies several lines.
        session.scroll_up(60);
        let scrolled = rendered(&session);
        assert_ne!(bottom, scrolled, "scrolling had no effect");
        assert!(
            !scrolled.contains("reply number 39"),
            "the latest reply is still shown after scrolling back"
        );

        // Scrolling far past the top clamps to the first entry rather than blanking.
        session.scroll_up(u16::MAX);
        let top = rendered(&session);
        assert!(
            top.contains("reply number 0"),
            "scrolling to the top did not reach the first reply"
        );

        // And coming back down returns to the latest output.
        session.scroll_down(u16::MAX);
        assert!(
            rendered(&session).contains("reply number 39"),
            "scrolling back down did not return to the latest reply"
        );
    }
    /// Render at a chosen size, since wrapping depends on width.
    fn rendered_at(session: &Session, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| {
                draw(frame, session);
            })
            .expect("draw succeeds");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    /// The hint line alone, which is the last row of the screen.
    ///
    /// Needed because the heading carries the confinement too: searching the whole screen for it
    /// cannot tell a hint line that still fits from one that has been cut off.
    fn hint_row_at(session: &Session, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| {
                draw(frame, session);
            })
            .expect("draw succeeds");
        let buffer = terminal.backend().buffer().clone();
        (0..width)
            .map(|column| buffer[(column, height - 1)].symbol())
            .collect()
    }

    fn typed(text: &str) -> Session {
        let mut session = Session::new("test");
        for c in text.chars() {
            session.type_char(c);
        }
        session
    }

    /// Where the caret is on screen, and what it is drawn over.
    ///
    /// Found by the reversed cell rather than by a glyph, because the caret is a style over a
    /// character the user typed: there is nothing in the text to search for.
    fn caret_cell(session: &Session, width: u16, height: u16) -> Option<(u16, u16, String)> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| {
                draw(frame, session);
            })
            .expect("draw succeeds");
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .flat_map(|row| (0..width).map(move |column| (column, row)))
            .find(|at| buffer[*at].modifier.contains(Modifier::REVERSED))
            .map(|(column, row)| (column, row, buffer[(column, row)].symbol().to_string()))
    }

    /// Every cell the caret covers, in reading order.
    ///
    /// Read off the screen the same way [`caret_cell`] reads one, because what the caret covers is
    /// a style over text the user can see and there is nothing else to search for.
    fn caret_cells(session: &Session, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| {
                draw(frame, session);
            })
            .expect("draw succeeds");
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .flat_map(|row| (0..width).map(move |column| (column, row)))
            .filter(|at| buffer[*at].modifier.contains(Modifier::REVERSED))
            .map(|at| buffer[at].symbol().to_string())
            .collect()
    }

    fn picture() -> crate::clipboard::Image {
        crate::clipboard::Image {
            media_type: "image/png",
            bytes: b"pixels".to_vec(),
        }
    }

    /// The caret crosses a marker in one press, so it has to be drawn over the whole of one:
    /// a block on the opening bracket alone says the next press takes a bracket.
    #[test]
    fn the_caret_covers_a_whole_marker() {
        let mut session = typed("look at ");
        session.attach(picture());
        session.move_left();

        assert_eq!(caret_cells(&session, 40, 12), "[Image #1]");
    }

    /// A marker is a span of the line rather than of the row it landed on, and a narrow box puts
    /// a long one across two rows. Covering only the row the caret is on would show half of it
    /// highlighted and leave the rest looking like ordinary words.
    #[test]
    fn a_marker_the_wrap_split_is_covered_on_both_rows() {
        let mut session = Session::new("test");
        session.paste_text("one\ntwo\nthree\nfour");
        session.move_left();

        assert_eq!(caret_cells(&session, 22, 12), "[Pasted text #1 +4 lines]");
    }

    /// The bug: text past the right edge used to be clipped, cursor included, which looked like
    /// the program had stopped taking keys.
    #[test]
    fn typing_past_the_edge_stays_visible() {
        let text = format!("{}TAIL", "abcdefghij ".repeat(9));
        let output = rendered_at(&typed(&text), 50, 14);

        assert!(
            output.contains("TAIL"),
            "the end of the input was clipped: {output}"
        );
        assert!(
            caret_cell(&typed(&text), 50, 14).is_some(),
            "the cursor was clipped: {output}"
        );
    }

    /// The caret sits on the character the next keystroke will land before, which need not be the
    /// end of the line: a caret that stayed at the end would point at the wrong place after every
    /// Left press.
    #[test]
    fn the_caret_is_drawn_where_it_sits() {
        let mut session = typed("abcd");
        session.move_left();
        session.move_left();

        let (_, _, on) = caret_cell(&session, 40, 12).expect("the caret was not drawn");
        assert_eq!(on, "c", "the caret was not on the character it precedes");
    }

    /// The bug this replaced a glyph to fix: a caret inserted between two characters pushed
    /// everything after it along, so the text shifted left and right as the caret moved.
    #[test]
    fn the_caret_does_not_move_the_text_it_passes_over() {
        let mut session = typed("abcd");
        let settled = rendered_at(&session, 40, 12);

        for _ in 0..4 {
            session.move_left();
            assert_eq!(
                rendered_at(&session, 40, 12),
                settled,
                "the text moved under the caret"
            );
        }
    }

    /// Past the end of the line the caret has no character to sit on, so it is drawn as a
    /// highlighted space rather than vanishing at the one moment typing is about to happen.
    #[test]
    fn the_caret_past_the_end_of_the_line_is_a_block() {
        let (_, _, on) = caret_cell(&typed("abc"), 40, 12).expect("the caret was not drawn");
        assert_eq!(on, " ");
    }

    /// The row is measured with a column spare for the caret past the end of it, so a full row
    /// keeps every character it was given.
    #[test]
    fn a_full_row_is_not_clipped() {
        let width = 24u16;
        let session = typed(&"x".repeat(input_text_width(width)));
        let output = rendered_at(&session, width, 12);

        assert_eq!(
            output.matches('x').count(),
            input_text_width(width),
            "a character was pushed off the edge: {output}"
        );
    }

    /// The box grows with the text rather than staying one line.
    #[test]
    fn the_input_box_grows_with_the_text() {
        let short = input_height(&typed("hi"), 50, 24);
        let long = input_height(&typed(&"word ".repeat(30)), 50, 24);
        assert!(
            long > short,
            "the box did not grow: {short} then {long} rows"
        );
    }

    /// But not without limit, or a long paste would push the transcript off screen.
    #[test]
    fn the_input_box_stops_growing_at_the_cap() {
        let huge = input_height(&typed(&"word ".repeat(500)), 50, 40);
        assert_eq!(huge as usize, wrap::MAX_ROWS + 2, "the cap was not applied");
    }

    /// Past the cap the view follows the cursor, so typing at the end stays visible.
    #[test]
    fn a_very_long_input_scrolls_to_the_cursor() {
        let text = format!("{}TAIL", "word ".repeat(90));
        let output = rendered_at(&typed(&text), 40, 20);
        assert!(
            output.contains("TAIL"),
            "the cursor's row scrolled away: {output}"
        );
        assert!(caret_cell(&typed(&text), 40, 20).is_some());
    }

    /// A short terminal must still show some transcript and the hint line.
    #[test]
    fn the_input_leaves_room_on_a_short_terminal() {
        let session = typed(&"word ".repeat(200));
        let height = 8;
        let used = input_height(&session, 40, height);
        assert!(
            used < height - 1,
            "the input took {used} of {height} rows, leaving nothing for the transcript"
        );
    }

    /// Only the first row carries the prompt; continuations align beneath it.
    #[test]
    fn continuation_rows_are_indented_not_prompted() {
        let output = rendered_at(&typed(&"abcdefghij ".repeat(6)), 40, 14);
        assert_eq!(
            output.matches("> ").count(),
            1,
            "more than one prompt marker was drawn: {output}"
        );
    }

    /// The bug: the indicator was drawn in place of the box, so a prompt typed during a turn went
    /// somewhere the person typing it could not see. The turn is slow and the next prompt is
    /// thought of while it runs, so the box has to stay.
    #[test]
    fn a_prompt_typed_during_a_turn_is_visible() {
        let mut session = typed("first");
        session.submit().expect("submitted");
        for c in "SECOND".chars() {
            session.type_char(c);
        }

        let output = rendered_at(&session, 60, 16);
        assert!(
            output.contains("SECOND"),
            "the line typed mid-turn was not drawn: {output}"
        );
        assert!(
            output.contains("…"),
            "the indicator went missing with the box: {output}"
        );
        assert!(
            caret_cell(&session, 60, 16).is_some(),
            "there is no caret to type at"
        );
    }

    /// And it grows with the text like any other, since a paragraph can be written mid-turn.
    #[test]
    fn the_box_grows_mid_turn_too() {
        let mut session = typed("first");
        session.submit().expect("submitted");
        let bare = input_height(&session, 50, 24);
        for c in "word ".repeat(30).chars() {
            session.type_char(c);
        }

        assert!(
            input_height(&session, 50, 24) > bare,
            "the box did not grow while a turn ran"
        );
    }

    /// Nothing is running, so nothing is said about it and the whole height goes to the rest.
    #[test]
    fn an_idle_session_shows_no_indicator_row() {
        assert_eq!(status_height(&typed("hi"), 24), 0);
    }
    /// The position belongs in the border, where it labels the box without costing a row.
    #[test]
    fn browsing_history_shows_the_position_in_the_border() {
        let mut session = Session::new("none");
        for n in 1..=83 {
            for c in format!("prompt {n}").chars() {
                session.type_char(c);
            }
            session.submit().expect("submitted");
            session.complete("ok", Vec::new(), 0);
        }
        for _ in 0..6 {
            session.recall_older();
        }

        let output = rendered_at(&session, 60, 10);
        assert!(
            output.contains("History 78/83"),
            "the position is not shown: {output}"
        );
        assert!(
            output.contains("prompt 78"),
            "the recalled prompt is not shown: {output}"
        );
    }

    mod todos {
        use super::*;
        use bravebot_core::todo::{Item, List, Status, rows};

        fn list(entries: &[(&str, Status)]) -> Vec<bravebot_core::todo::Row> {
            rows(&List::new(
                entries
                    .iter()
                    .map(|(content, status)| Item::new(*content, *status))
                    .collect(),
            ))
        }

        fn three() -> Vec<bravebot_core::todo::Row> {
            list(&[
                ("Escape cancels a turn", Status::Done),
                ("Add prompt history", Status::Active),
                ("Persist it across sessions", Status::Pending),
            ])
        }

        fn working_with(todos: Vec<bravebot_core::todo::Row>) -> Session {
            let mut session = Session::new("test");
            session.type_char('a');
            session.submit();
            session.set_todos(todos);
            session
        }

        /// The whole list is on screen while the turn runs, which is the point of the feature:
        /// what is done, what is being worked on, and what is left.
        #[test]
        fn a_running_turn_shows_the_whole_list() {
            let output = rendered_at(&working_with(three()), 60, 16);
            for task in [
                "Escape cancels a turn",
                "Add prompt history",
                "Persist it across sessions",
            ] {
                assert!(output.contains(task), "'{task}' is missing: {output}");
            }
        }

        /// The indicator's area has to grow, or the list would be drawn over the box or clipped
        /// away.
        #[test]
        fn the_indicator_area_grows_to_hold_the_list() {
            let bare = status_height(&working_with(Vec::new()), 24);
            let with_list = status_height(&working_with(three()), 24);
            assert_eq!(
                with_list as usize,
                bare as usize + 3,
                "the area did not grow by one row per task"
            );
        }

        /// A long list on a short terminal must not squeeze the transcript and the box out
        /// entirely: a list nobody can type underneath is worse than a shorter list.
        #[test]
        fn a_long_list_leaves_room_for_the_box_and_the_transcript() {
            let many: Vec<_> = (0..40)
                .map(|n| (format!("task {n}"), Status::Pending))
                .collect();
            let borrowed: Vec<_> = many.iter().map(|(t, s)| (t.as_str(), *s)).collect();
            let session = working_with(list(&borrowed));
            let height = 10;
            let status = status_height(&session, height);
            let input = input_height(&session, 60, height - status);
            assert!(
                status + input < height - 1,
                "the list and the box took {status} and {input} of {height} rows, leaving nothing for the transcript"
            );
        }

        /// The box is still there under the list, and still takes what is typed into it.
        #[test]
        fn the_box_stays_beneath_the_list() {
            let mut session = working_with(three());
            for c in "MID".chars() {
                session.type_char(c);
            }

            let output = rendered_at(&session, 60, 16);
            assert!(
                output.contains("MID"),
                "the box was drawn over by the list: {output}"
            );
        }

        /// And it must not panic when the box is smaller than the list.
        #[test]
        fn a_list_taller_than_the_terminal_renders() {
            let many: Vec<_> = (0..60)
                .map(|n| (format!("task {n}"), Status::Active))
                .collect();
            let borrowed: Vec<_> = many.iter().map(|(t, s)| (t.as_str(), *s)).collect();
            rendered_at(&working_with(list(&borrowed)), 30, 8);
        }

        /// Finished work is struck through: that is what makes the list readable at a glance,
        /// and a marker alone would not show it.
        #[test]
        fn a_finished_task_is_drawn_struck_through() {
            let session = working_with(three());
            let mut terminal = Terminal::new(TestBackend::new(60, 16)).expect("terminal");
            terminal
                .draw(|frame| {
                    draw(frame, &session);
                })
                .expect("draw succeeds");

            // Found by content rather than position, so the assertion survives a layout change.
            let buffer = terminal.backend().buffer().clone();
            let struck = buffer
                .content()
                .iter()
                .any(|cell| cell.modifier.contains(Modifier::CROSSED_OUT));
            assert!(struck, "no cell was drawn struck through");
        }

        /// Outstanding work must not be struck through, or every task would look finished.
        #[test]
        fn outstanding_tasks_are_not_struck_through() {
            let session = working_with(list(&[("still to do", Status::Pending)]));
            let mut terminal = Terminal::new(TestBackend::new(60, 16)).expect("terminal");
            terminal
                .draw(|frame| {
                    draw(frame, &session);
                })
                .expect("draw succeeds");

            let buffer = terminal.backend().buffer().clone();
            let struck = buffer
                .content()
                .iter()
                .any(|cell| cell.modifier.contains(Modifier::CROSSED_OUT));
            assert!(!struck, "an unfinished task was drawn struck through");
        }

        /// The list stays visible after the turn, attached to the reply it belongs to.
        #[test]
        fn a_finished_turn_still_shows_its_list() {
            let mut session = working_with(three());
            session.complete("all done", Vec::new(), 0);

            let output = rendered_at(&session, 60, 20);
            assert!(output.contains("all done"));
            assert!(
                output.contains("Add prompt history"),
                "the list vanished when the turn ended: {output}"
            );
        }

        /// A turn that never calls the tool must look exactly as it did before.
        #[test]
        fn a_turn_without_a_list_is_unchanged() {
            let mut with = Session::new("test");
            with.type_char('a');
            with.submit();
            let before = rendered_at(&with, 60, 16);

            with.set_todos(Vec::new());
            assert_eq!(before, rendered_at(&with, 60, 16));
        }
    }

    /// And it is absent when not browsing, so the border is not permanently labelled.
    #[test]
    fn an_unbrowsed_input_has_no_position_in_the_border() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit().expect("submitted");
        session.complete("ok", Vec::new(), 0);

        let output = rendered_at(&session, 60, 10);
        assert!(
            !output.contains("History"),
            "the border was labelled: {output}"
        );
    }
}
