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

use bua_core::event::{Event, Role};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};

use crate::state::{Session, Speaker, Status};
use crate::wrap;

/// Marks a turn boundary in the transcript.
const TURN_MARKER: &str = "⏺";
/// Marks a detail belonging to the entry above it.
const DETAIL_MARKER: &str = "⎿";

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
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
}

/// Build the transcript as lines, so height is known before rendering.
fn transcript_lines(session: &Session) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();

    if session.transcript.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!("{TURN_MARKER} "), Style::default().fg(Color::Cyan)),
            Span::styled("bua", Style::default().add_modifier(Modifier::BOLD)),
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
            Speaker::Assistant => {
                for (index, text) in entry.text.lines().enumerate() {
                    if index == 0 {
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("{TURN_MARKER} "),
                                Style::default().fg(Color::Green),
                            ),
                            Span::raw(text.to_string()),
                        ]));
                    } else {
                        lines.push(Line::from(format!("  {text}")));
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
        }

        if session.show_trail && !entry.trail.is_empty() {
            for event in &entry.trail {
                lines.push(trail_line(event));
            }
        }
        lines.push(Line::raw(""));
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

/// Render one audit event.
///
/// Refusals are coloured differently from passes: a blocked gate is the most important
/// thing on the screen when it happens.
fn trail_line(event: &Event) -> Line<'static> {
    let bad = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);

    match event {
        Event::GatePassed { gate, detail } => Line::from(Span::styled(
            format!("  {DETAIL_MARKER} {gate}: {detail}"),
            dim(),
        )),
        Event::GateBlocked { gate, reason, .. } => Line::from(Span::styled(
            format!("  {DETAIL_MARKER} {gate}: {reason}"),
            bad,
        )),
        Event::Observed { capability, label } => Line::from(Span::styled(
            format!("  {DETAIL_MARKER} {capability} produced {label}"),
            dim(),
        )),
        Event::SlotWritten { slot, label } => Line::from(Span::styled(
            format!("  {DETAIL_MARKER} slot {slot} at {label}"),
            dim(),
        )),
        Event::Declassified { slot, from, to, .. } => Line::from(Span::styled(
            format!("  {DETAIL_MARKER} released {slot} {from} → {to}"),
            dim(),
        )),
        Event::ActionField {
            tool,
            field,
            role,
            label,
            allowed,
        } => {
            let role = match role {
                Role::Routing => "routing",
                Role::Content => "content",
            };
            Line::from(Span::styled(
                format!("  {DETAIL_MARKER} {tool}.{field} [{role}] {label}"),
                if *allowed { dim() } else { bad },
            ))
        }
    }
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
    // While a turn runs the box holds the one-line indicator instead of the input.
    if session.status == Status::Working {
        return 3;
    }

    let rows = wrap::wrap(&session.input, input_text_width(width))
        .rows
        .len()
        .min(wrap::MAX_ROWS);

    // Leave at least one line of transcript and the hint line, whatever the text does.
    let ceiling = (height as usize).saturating_sub(2).max(3);
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
        vec![body]
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
                "  {trail}  ·  scroll to look back  ·  ctrl-c exit  ·  confinement {}",
                session.confinement
            ),
            dim(),
        ))),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use bua_core::capability::Capability;
    use bua_core::label::Label;
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

    #[test]
    fn an_empty_session_shows_a_greeting_and_hint() {
        let session = Session::new("kernel-enforced");
        let output = rendered(&session);
        assert!(output.contains("Ask a question"), "no hint shown");
        assert!(output.contains("bua"));
    }

    /// Confinement belongs on screen at all times, not only in doctor.
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
            output.contains(indicator.verb),
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
