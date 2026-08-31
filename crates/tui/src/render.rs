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
use bravebot_agent::report::Activity;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};

use crate::audit::TrailLine;
use crate::markdown;
use crate::state::{Session, Speaker, Status};
use crate::wrap;

/// Marks a turn boundary in the transcript.
const TURN_MARKER: &str = "⏺";
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

/// Draw one tool call: what it is, and what came of it.
///
/// The shape mirrors a turn's own: a marker, then the detail indented beneath it, so a call
/// and its result read as one thing rather than two unrelated lines.
fn activity_lines(activity: &Activity) -> Vec<Line<'static>> {
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

    lines.extend(diff_lines(&activity.changes));
    lines
}

/// The hunks of a write, trimmed to what fits without burying the rest of the transcript.
fn diff_lines(changes: &[Change]) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = changes
        .iter()
        .take(MAX_DIFF_LINES)
        .map(|change| match change {
            Change::Added(text) => Line::from(Span::styled(
                format!("     + {text}"),
                Style::default().fg(Color::Green),
            )),
            Change::Removed(text) => Line::from(Span::styled(
                format!("     - {text}"),
                Style::default().fg(Color::Red),
            )),
            Change::Kept(text) => Line::from(Span::styled(format!("       {text}"), dim())),
            Change::Elided(count) => Line::from(Span::styled(
                format!("     … {count} unchanged lines"),
                dim(),
            )),
        })
        .collect();

    // Said rather than silently dropped: a change that stops without saying so reads as the
    // whole change, which is how a reviewer misses half of it. Worded for both kinds of write,
    // since a new file's lines were never a diff of anything.
    if changes.len() > MAX_DIFF_LINES {
        lines.push(Line::from(Span::styled(
            format!("     … {} more lines", changes.len() - MAX_DIFF_LINES),
            dim(),
        )));
    }

    lines
}

/// Draw the whole interface.
pub fn draw(frame: &mut Frame, session: &Session) {
    // The input's height depends on how far the text wraps, so it is measured before the layout
    // rather than fixed: a fixed height is what made typing past the edge disappear.
    let input_height = input_height(session, frame.area().width, frame.area().height);

    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),               // transcript
            Constraint::Length(input_height), // input
            Constraint::Length(1),            // hint line
        ])
        .split(frame.area());

    draw_transcript(frame, areas[0], session);
    draw_input(frame, areas[1], session);
    draw_hint(frame, areas[2], session);

    // Last, over everything: the selection is of the screen rather than of any one widget, and
    // the user swept it over whatever happened to be there.
    if let Some(selection) = &session.selection {
        crate::select::highlight(frame.buffer_mut(), selection);
    }
}

/// Build the transcript as lines, so height is known before rendering.
fn transcript_lines(session: &Session) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();

    if session.transcript.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!("{TURN_MARKER} "), Style::default().fg(Color::Cyan)),
            Span::styled("bravebot", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("  ·  confinement {}", session.confinement), dim()),
        ]));
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "  Ask a question about this workspace.".to_string(),
            dim(),
        )));
        return lines;
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
                for (index, text) in entry.text.lines().enumerate() {
                    let lead = if index == 0 {
                        Span::styled(format!("{TURN_MARKER} "), Style::default().fg(Color::Green))
                    } else {
                        Span::raw("  ")
                    };
                    let mut spans = vec![lead];
                    spans.extend(markdown::spans(text, Style::default()));
                    lines.push(Line::from(spans));
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
            // What the turn did, kept in the scrollback next to what it said about it.
            Speaker::Tool => match &entry.activity {
                Some(activity) => lines.extend(activity_lines(activity)),
                // A call read back out of a stored session, which records that it happened and
                // not what came of it. Drawn without the coloured marker a live call earns,
                // since green would claim an outcome the record does not have.
                None => lines.push(Line::from(vec![
                    Span::styled(format!("{TURN_MARKER} "), dim()),
                    Span::styled(entry.text.clone(), dim()),
                ])),
            },
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

    lines
}

fn draw_transcript(frame: &mut Frame, area: Rect, session: &Session) {
    let lines = transcript_lines(session);

    // Scroll counts up from the bottom, so new output stays in view by default.
    let total = lines.len() as u16;
    let max_offset = total.saturating_sub(area.height);
    let offset = max_offset.saturating_sub(session.scroll.min(max_offset));

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((offset, 0)),
        area,
    );
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
/// Two for the borders and two for the `> ` prompt, so a wrap computed against this matches what
/// the terminal will actually show.
fn input_text_width(total: u16) -> usize {
    (total as usize).saturating_sub(4).max(1)
}

/// Rows the input box needs, borders included.
///
/// Grows with the text up to [`wrap::MAX_ROWS`], and never takes so much of a short terminal that
/// the transcript disappears.
fn input_height(session: &Session, width: u16, height: u16) -> u16 {
    // Leave at least one line of transcript and the hint line, whatever is in the box.
    let ceiling = (height as usize).saturating_sub(2).max(3);

    // While a turn runs the box holds the indicator, and the task list beneath it if there is
    // one, instead of the input.
    if session.status == Status::Working {
        return (3 + session.todos.len()).min(ceiling) as u16;
    }

    let rows = wrap::wrap(&session.input, input_text_width(width))
        .rows
        .len()
        .min(wrap::MAX_ROWS);

    (rows + 2).min(ceiling) as u16
}

fn draw_input(frame: &mut Frame, area: Rect, session: &Session) {
    let working = session.status == Status::Working;

    let body = if working {
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
        Line::from(Span::raw(""))
    };

    // Wrapping is computed here rather than left to `Paragraph`, because the cursor has to be
    // placed after the last character and only an explicit wrap knows where that is.
    let lines: Vec<Line> = if working {
        // The list sits under the indicator, so what is being worked on and what remains are read
        // together. Trimmed to what the box was given rather than overflowing it.
        let room = (area.height as usize).saturating_sub(3);
        let mut lines = vec![body];
        lines.extend(todo_lines(&session.todos).into_iter().take(room));
        lines
    } else {
        let wrapped = wrap::wrap(&session.input, input_text_width(area.width));
        let visible = (area.height as usize).saturating_sub(2).max(1);
        let (first, rows) = wrapped.window(visible);

        rows.iter()
            .enumerate()
            .map(|(offset, row)| {
                let index = first + offset;
                // Only the first row carries the prompt; continuations are indented to line up
                // beneath it.
                let lead = if index == 0 { "> " } else { "  " };
                let mut spans = vec![
                    Span::styled(lead, Style::default().fg(Color::Cyan)),
                    Span::raw(row.clone()),
                ];
                if index == wrapped.cursor_row {
                    spans.push(Span::styled("▌", Style::default().fg(Color::Cyan)));
                }
                Line::from(spans)
            })
            .collect()
    };

    // The position sits in the border while browsing, labelling the box without taking a row
    // away from the text.
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if working {
            dim()
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

/// The shortcut line. Keeps the bindings discoverable without a help command.
fn draw_hint(frame: &mut Frame, area: Rect, session: &Session) {
    let trail = if session.show_trail {
        "ctrl-t hide trail"
    } else {
        "ctrl-t show trail"
    };

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(
                "  {trail}  ·  drag to copy  ·  pgup/pgdn or scroll to look back  ·  \
                 ctrl-c exit  ·  confinement {}",
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
            let lines = diff_lines(&changes);

            assert_eq!(lines.len(), MAX_DIFF_LINES + 1);
            let last = lines.last().expect("a line").to_string();
            assert!(last.contains("5 more"), "the omission is silent: {last}");
        }

        /// A short diff is shown whole, with nothing appended to suggest otherwise.
        #[test]
        fn a_short_diff_is_shown_whole_with_no_note() {
            let lines = diff_lines(&[Change::Added("only line".into())]);
            assert_eq!(lines.len(), 1);
        }

        /// Tool lines come in runs and read as one block. Spacing them apart would double the
        /// height of every turn that read more than a file or two.
        #[test]
        fn tool_lines_are_not_spaced_apart() {
            let mut session = working();
            for path in ["a.rs", "b.rs"] {
                session.finish_activity(Activity::running("Read", path).done("1 line"));
            }

            let blanks = transcript_lines(&session)
                .iter()
                .filter(|line| line.to_string().trim().is_empty())
                .count();
            assert_eq!(blanks, 1, "the prompt's own blank line is the only one");
        }
    }

    #[test]
    fn an_empty_session_shows_a_greeting_and_hint() {
        let session = Session::new("kernel-enforced");
        let output = rendered(&session);
        assert!(output.contains("Ask a question"), "no hint shown");
        assert!(output.contains("bravebot"));
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

    #[test]
    fn the_hint_line_reports_confinement() {
        let session = Session::new("kernel-enforced");
        let output = rendered(&session);
        assert!(output.contains("kernel-enforced"));
        assert!(output.contains("ctrl-c exit"));
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
        transcript_lines(session)
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

    fn typed(text: &str) -> Session {
        let mut session = Session::new("test");
        for c in text.chars() {
            session.type_char(c);
        }
        session
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
        assert!(output.contains('▌'), "the cursor was clipped: {output}");
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
        assert!(output.contains('▌'));
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

    /// While a turn runs the box holds the indicator, so it stays one line regardless of what
    /// was typed before.
    #[test]
    fn a_working_box_does_not_grow() {
        let mut session = typed(&"word ".repeat(50));
        session.submit();
        assert_eq!(input_height(&session, 50, 24), 3);
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

        /// The box has to grow, or the list would be drawn outside it or clipped away.
        #[test]
        fn the_box_grows_to_hold_the_list() {
            let bare = input_height(&working_with(Vec::new()), 60, 24);
            let with_list = input_height(&working_with(three()), 60, 24);
            assert_eq!(
                with_list as usize,
                bare as usize + 3,
                "the box did not grow by one row per task"
            );
        }

        /// A long list on a short terminal must not squeeze the transcript out entirely.
        #[test]
        fn a_long_list_leaves_room_for_the_transcript() {
            let many: Vec<_> = (0..40)
                .map(|n| (format!("task {n}"), Status::Pending))
                .collect();
            let borrowed: Vec<_> = many.iter().map(|(t, s)| (t.as_str(), *s)).collect();
            let height = 10;
            let used = input_height(&working_with(list(&borrowed)), 60, height);
            assert!(
                used < height - 1,
                "the list took {used} of {height} rows, leaving nothing for the transcript"
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
