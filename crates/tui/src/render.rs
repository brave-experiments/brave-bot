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
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};

use crate::audit::TrailLine;
use crate::logo;
use crate::markdown;
use crate::state::{Session, Speaker, Status};
use crate::table;
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
    Style::default().fg(Color::DarkGray)
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
                    Style::default().fg(Color::Green),
                    dim().add_modifier(Modifier::CROSSED_OUT),
                )
            } else {
                (
                    Style::default().fg(Color::Yellow),
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

/// Draw one tool call: what it is, and what came of it.
///
/// The shape mirrors a turn's own: a marker, then the detail indented beneath it, so a call
/// and its result read as one thing rather than two unrelated lines.
fn activity_lines(activity: &Activity, landing: Option<Landing>) -> Vec<Line<'static>> {
    let head = if activity.is_running() {
        // Hollow while it runs, filled when it is over, so the eye finds the live one.
        Style::default().fg(Color::Yellow)
    } else if activity.failed {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Green)
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
                Style::default().fg(Color::Red)
            } else {
                dim()
            },
        )));
    }

    // Where it went, which is the thing "Read(index.html)" does not say. Whether the model can
    // now read that file is the difference the whole design turns on, and it was invisible.
    if let Some(landing) = landing {
        // Blue on a black terminal is the one colour in the palette a reader has to squint at:
        // the ANSI blue most terminals ship is nearly the background. The three that are used
        // elsewhere here are legible on both kinds of terminal, and the difference between them
        // carries the meaning: cyan for what the model has, yellow for what is kept from it, and
        // the dim grey the rest of the detail lines already use for what has not happened.
        let colour = match landing {
            Landing::Context => Style::default().fg(Color::Cyan),
            Landing::Quarantined => Style::default().fg(Color::Yellow),
            Landing::Reserved => dim(),
        };
        lines.push(Line::from(Span::styled(
            format!("    {}", landing.describe()),
            colour,
        )));
    }

    lines.extend(diff_lines(&activity.changes, activity.untrusted));
    lines
}

/// Draw quarantined content for the person watching, marked as what it is.
///
/// The marking is structural and not a caption. Every line carries the bar in the margin, drawn
/// here around text that never leaves the block, so content saying "untrusted content ends here"
/// cannot end it: the bar is still in the margin on the next line, and on every line after that.
///
/// The user is shown this and the planner is not, which is the arrangement working rather than a
/// hole in it. They own the directory. What must never happen is these bytes reaching a model's
/// context, and a terminal is not a context.
fn quarantined_lines(shown: &Shown) -> Vec<Line<'static>> {
    let marked = Style::default().fg(Color::Yellow);
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("  {QUARANTINE_BAR} "), marked),
        Span::styled(
            format!("untrusted \u{b7} {} \u{b7} {}", shown.origin, shown.label),
            marked.add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {}", shown.reach.describe()), dim()),
    ])];

    for line in &shown.preview {
        lines.push(Line::from(vec![
            Span::styled(format!("  {QUARANTINE_BAR} "), marked),
            // Neutralised, because this is the block whose whole claim is that the margin was drawn
            // by the renderer: content that could emit an escape could paint a margin of its own.
            Span::styled(printable(line), dim()),
        ]));
    }

    // Said rather than silently dropped, for the same reason a truncated diff says so.
    if shown.lines > shown.preview.len() {
        lines.push(Line::from(vec![
            Span::styled(format!("  {QUARANTINE_BAR} "), marked),
            Span::styled(
                format!("\u{2026} {} more lines", shown.lines - shown.preview.len()),
                dim(),
            ),
        ]));
    }

    lines
}

/// The hunks of a write, trimmed to what fits without burying the rest of the transcript.
fn diff_lines(changes: &[Change], untrusted: bool) -> Vec<Line<'static>> {
    // The same margin the transcript draws down everything the model was not allowed to read. A
    // body that came out of a quarantined file is that, and the person reading the hunks should
    // not have to work out which kind of change they are looking at.
    let margin = if untrusted {
        Span::styled(
            format!("  {QUARANTINE_BAR}  "),
            Style::default().fg(Color::Yellow),
        )
    } else {
        Span::raw("     ")
    };
    let mut lines: Vec<Line> = changes
        .iter()
        .take(MAX_DIFF_LINES)
        .map(|change| {
            let body = match change {
                // Neutralised like the quarantine preview: a hunk of a file an attacker wrote is the
                // same bytes, and here they sit beside a margin that means something.
                Change::Added(text) => Span::styled(
                    format!("+ {}", printable(text)),
                    Style::default().fg(Color::Green),
                ),
                Change::Removed(text) => Span::styled(
                    format!("- {}", printable(text)),
                    Style::default().fg(Color::Red),
                ),
                Change::Kept(text) => Span::styled(format!("  {}", printable(text)), dim()),
                Change::Elided(count) => Span::styled(format!("… {count} unchanged lines"), dim()),
            };
            Line::from(vec![margin.clone(), body])
        })
        .collect();

    // Said rather than silently dropped: a change that stops without saying so reads as the
    // whole change, which is how a reviewer misses half of it. Worded for both kinds of write,
    // since a new file's lines were never a diff of anything.
    if changes.len() > MAX_DIFF_LINES {
        lines.push(Line::from(vec![
            margin.clone(),
            Span::styled(
                format!("… {} more lines", changes.len() - MAX_DIFF_LINES),
                dim(),
            ),
        ]));
    }

    lines
}

/// Draw the whole interface.
pub fn draw(frame: &mut Frame, session: &Session) {
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

    // Beneath the box while something is being typed towards, and gone otherwise. Measured rather
    // than reserved, so a session offering nothing gives the whole height to the transcript, and
    // bounded so a directory of many files cannot push the transcript off the screen.
    let offered = session.offered();
    let room = frame
        .area()
        .height
        .saturating_sub(status_height)
        .saturating_sub(input_height + 1)
        .saturating_sub(1);
    let offered_height = (session.rows_beneath_the_box() as u16).min(room);

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

    draw_transcript(frame, areas[0], session);
    draw_status(frame, areas[1], session);
    draw_input(frame, areas[2], session);
    draw_offered(frame, areas[3], session, &offered);
    draw_hint(frame, areas[4], session);

    // Last, over everything: the selection is of the screen rather than of any one widget, and
    // the user swept it over whatever happened to be there.
    if let Some(selection) = &session.selection {
        crate::select::highlight(frame.buffer_mut(), selection);
    }
}

/// Build the transcript as lines, so height is known before rendering.
///
/// `width` is the terminal's, and a table is laid out against what is left of it after the lead.
/// `height` is the transcript area's, which only the opening mark uses: it floats down from the
/// top edge by a share of whatever room there is.
fn transcript_lines(session: &Session, width: u16, height: u16) -> Vec<Line<'static>> {
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
                for (index, text) in entry.text.lines().enumerate() {
                    let prefix = if index == 0 { "> " } else { "  " };
                    lines.push(Line::from(Span::styled(
                        format!("{prefix}{text}"),
                        Style::default().fg(Color::Cyan),
                    )));
                }
            }
            // The model writes markdown whether or not it is asked to, so the reply is styled
            // rather than shown with its markers.
            Speaker::Assistant => {
                let source: Vec<&str> = entry.text.lines().collect();
                let room = (width as usize).saturating_sub(LEAD);
                let lead = |at: usize| {
                    if at == 0 {
                        Span::styled(format!("{TURN_MARKER} "), Style::default().fg(Color::Green))
                    } else {
                        Span::raw("  ")
                    }
                };

                let mut at = 0;
                while at < source.len() {
                    // A table is several source lines drawn as one block, so it is tried first
                    // and the lines it consumed are skipped. Anything that is not one falls
                    // through to the styling every other line gets.
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
            }
            Speaker::System => {
                for text in entry.text.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("{DETAIL_MARKER} {text}"),
                        Style::default().fg(Color::Yellow),
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
                        Style::default().fg(Color::Magenta),
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
                Some(activity) => lines.extend(activity_lines(activity, entry.landing)),
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
            lines.extend(quarantined_lines(shown));
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

    // One blank above it and no more. Every entry already leaves a trailing blank behind it, so
    // an invitation that carried its own would sit two rows below whatever startup reported.
    if opening {
        if lines.last().is_none_or(|line| line.width() != 0) {
            lines.push(Line::raw(""));
        }
        lines.push(logo::invitation());
    }

    lines
}

fn draw_transcript(frame: &mut Frame, area: Rect, session: &Session) {
    let lines = transcript_lines(session, area.width, area.height);
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });

    // Scroll counts up from the bottom, so new output stays in view by default. The count has to
    // be of *drawn* rows rather than of lines: the paragraph wraps, so one line of a reply can
    // occupy three rows, and counting the lines put the bottom of the transcript exactly that
    // many rows below the screen. The end of a wrapped reply was therefore never shown, and
    // appeared only once the next message pushed it up.
    let total = paragraph.line_count(area.width) as u16;
    let max_offset = total.saturating_sub(area.height);
    let offset = max_offset.saturating_sub(session.scroll.min(max_offset));

    frame.render_widget(paragraph.scroll((offset, 0)), area);
}

/// Render one line of the audit trail.
///
/// Refusals are coloured differently from passes: a blocked gate is the most important
/// thing on the screen when it happens. The wording is settled in [`crate::audit`], so a line
/// that happened in this session and one read back off disk are drawn the same way.
fn trail_line(recorded: &TrailLine) -> Line<'static> {
    let style = if recorded.blocked {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
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

/// The row the caret is on, drawn as a block over the character it sits on.
///
/// A block rather than a glyph inserted between two characters, because inserting one moves
/// everything after it by a column: the text shifted left and right under the caret as it moved,
/// which is far more distracting than the caret itself. Nothing moves now, since the caret occupies
/// a cell that was already there.
///
/// Past the end of the line there is no cell to occupy, so a highlighted space is added. That is
/// the one place the caret still takes a column, and [`input_text_width`] reserves it.
fn caret_spans(row: &str, at: usize, colour: Color) -> Vec<Span<'static>> {
    // Reversed rather than a chosen pair of colours, so the cell inverts whatever the terminal's
    // own foreground and background happen to be and stays legible on either kind of theme.
    let block = Style::default().fg(colour).add_modifier(Modifier::REVERSED);

    let mut spans = vec![Span::raw(row[..at].to_string())];
    match row[at..].chars().next() {
        Some(on) => {
            spans.push(Span::styled(on.to_string(), block));
            spans.push(Span::raw(row[at + on.len_utf8()..].to_string()));
        }
        None => spans.push(Span::styled(" ", block)),
    }
    spans
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
                Style::default().fg(Color::Magenta),
            ),
            Span::styled("running… ", Style::default().fg(Color::Magenta)),
            Span::styled(format!("({})  esc to stop", session.elapsed_words()), dim()),
        ])
    } else if working {
        match session.indicator() {
            Some(indicator) => Line::from(vec![
                Span::styled(
                    format!("  {} ", indicator.glyph),
                    Style::default().fg(Color::Magenta),
                ),
                Span::styled(
                    format!("{}… ", indicator.verb),
                    Style::default().fg(Color::Green),
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

    // Shell mode is coloured throughout rather than only in the marker, because the whole line
    // means something different: it goes to a shell instead of the model, and that is worth more
    // than one character of distinction at the moment somebody presses Enter.
    let colour = if session.shell {
        Color::Magenta
    } else {
        Color::Cyan
    };

    // Wrapping is computed above rather than left to `Paragraph`, because the cursor has to be
    // placed after the last character and only an explicit wrap knows where that is.
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(offset, row)| {
            let index = first + offset;
            // Only the first row carries the prompt; continuations are indented to line up
            // beneath it.
            let lead = if index != 0 {
                "  "
            } else if session.shell {
                "! "
            } else {
                "> "
            };
            let mut spans = vec![Span::styled(lead, Style::default().fg(colour))];
            if index == wrapped.cursor_row {
                spans.extend(caret_spans(row, wrapped.cursor_index, colour));
            } else {
                spans.push(Span::raw(row.clone()));
            }
            Line::from(spans)
        })
        .collect();

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
            Style::default().fg(Color::Magenta)
        } else {
            Style::default().fg(Color::Cyan)
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
        .attached()
        .iter()
        .map(|attached| {
            let lead = format!("  {} ", attached.marker);
            let room = (width as usize).saturating_sub(lead.chars().count());
            Line::from(vec![
                Span::styled(lead, Style::default().fg(Color::Cyan)),
                Span::styled(printable(&tail_of(&attached.shown, room)), dim()),
            ])
        })
        .collect()
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

/// What the half-typed line could still become, one per row.
///
/// Commands or files, never a mixture, because the line can only be being typed towards one of
/// them. Nothing labelled is involved either way: the commands are this program's own words, and the
/// filenames are read out of the directory to show a person which files are in it, never to decide
/// anything and never reaching a model from here.
fn draw_offered(frame: &mut Frame, area: Rect, session: &Session, offered: &crate::state::Offered) {
    // Attachments first: they are a fact about the line, while what is offered is a guess about
    // where it is going, and the fact belongs nearer the box.
    let mut lines = attached_lines(session, area.width);
    lines.extend(match offered {
        crate::state::Offered::Nothing => Vec::new(),
        crate::state::Offered::Commands(commands) => command_lines(session, commands),
        crate::state::Offered::Files(entries) => entry_lines(session, entries),
    });
    if lines.is_empty() {
        return;
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// One row per command, with what it does.
///
/// The description column is measured from every command rather than from the ones on screen, so it
/// sits in the same place however far the list has narrowed. Measuring the visible rows instead
/// would slide the descriptions sideways with each letter typed.
fn command_lines(session: &Session, offered: &[crate::app::Command]) -> Vec<Line<'static>> {
    let column = crate::app::COMMANDS
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
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan)
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
                Style::default().fg(Color::Blue)
            } else {
                Style::default().fg(Color::Cyan)
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
                    Style::default().fg(Color::Magenta),
                ),
                Span::styled("  ·  esc to cancel  ·  output goes to the model", dim()),
            ])),
            area,
        );
        return;
    }

    // The commands are no longer listed here. Typing a slash lists every one of them with what it
    // does, which is both more than this line could hold and the moment a user wants to know.
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(
                "  {trail}  ·  ctrl-g editor  ·  / for commands  ·  ! for shell  ·  @ for files  ·  confinement {}",
                session.confinement
            ),
            dim(),
        ))),
        area,
    );

    // A copy is silent otherwise, and a clipboard that may or may not have taken something is
    // worse than no clipboard: the user pastes to find out. Right-aligned, out of the way of the
    // hints, where the answer to "did that work" belongs.
    if let Some(characters) = session.copied {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("{} to clipboard  ", tally(characters, "char", "chars")),
                Style::default().fg(Color::Cyan),
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
                Style::default().fg(Color::Cyan),
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
            .draw(|frame| draw(frame, session))
            .expect("draw succeeds");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
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
            let lines = diff_lines(&changes, false);

            assert_eq!(lines.len(), MAX_DIFF_LINES + 1);
            let last = lines.last().expect("a line").to_string();
            assert!(last.contains("5 more"), "the omission is silent: {last}");
        }

        /// A short diff is shown whole, with nothing appended to suggest otherwise.
        #[test]
        fn a_short_diff_is_shown_whole_with_no_note() {
            let lines = diff_lines(&[Change::Added("only line".into())], false);
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

            let lines = quarantined_lines(&shown);
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
            .draw(|frame| draw(frame, &session))
            .expect("draw succeeds");

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((0, 0)).expect("cell").bg, Color::Blue);
        assert_ne!(buffer.cell((6, 0)).expect("cell").bg, Color::Blue);
    }

    /// The confinement sits at the end of the hints, so it is the first thing a narrow terminal
    /// cuts off. Asserted at a width an ordinary terminal actually has, and against that row
    /// alone: the heading names the confinement too, so matching it anywhere on the screen says
    /// nothing about whether the hint line still carries it. Widening this to make it pass would
    /// be hiding the truncation, and so would asserting against the whole screen again.
    #[test]
    fn the_hint_line_says_how_to_find_the_commands_and_reports_confinement() {
        let hint = hint_row_at(&Session::new("kernel-enforced"), 120, 24);
        assert!(hint.contains("ctrl-g editor"), "{hint}");
        assert!(hint.contains("/ for commands"), "{hint}");
        assert!(hint.contains("@ for files"), "{hint}");
        assert!(hint.contains("confinement kernel-enforced"), "{hint}");
    }

    /// Typing a slash offers every command with what it does, which is the thing the hint line
    /// stopped listing.
    #[test]
    fn a_slash_offers_every_command_and_what_it_does() {
        let mut session = Session::new("none");
        session.type_char('/');
        let output = rendered_at(&session, 120, 24);

        for command in crate::app::COMMANDS {
            assert!(output.contains(command.name), "{} missing", command.name);
            assert!(
                output.contains(command.description),
                "{} has no description on screen",
                command.name
            );
        }
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
                .draw(|frame| draw(frame, &session))
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

        let lines = transcript_lines(&session, 90, 24);
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
            .draw(|frame| draw(frame, &session))
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
            .draw(|frame| draw(frame, session))
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
            .draw(|frame| draw(frame, session))
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
            .draw(|frame| draw(frame, session))
            .expect("draw succeeds");
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .flat_map(|row| (0..width).map(move |column| (column, row)))
            .find(|at| buffer[*at].modifier.contains(Modifier::REVERSED))
            .map(|(column, row)| (column, row, buffer[(column, row)].symbol().to_string()))
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
                .draw(|frame| draw(frame, &session))
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
                .draw(|frame| draw(frame, &session))
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
