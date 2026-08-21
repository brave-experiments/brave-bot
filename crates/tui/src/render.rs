//! Drawing the session.
//!
//! Untrusted content — model replies, tool output — is shown, but the interface makes its
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

/// Marks a turn boundary in the transcript.
const TURN_MARKER: &str = "⏺";
/// Marks a detail belonging to the entry above it.
const DETAIL_MARKER: &str = "⎿";

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// Draw the whole interface.
pub fn draw(frame: &mut Frame, session: &Session) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // transcript
            Constraint::Length(3), // input
            Constraint::Length(1), // hint line
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

    if session.status == Status::Working {
        lines.push(Line::from(Span::styled(
            format!("{TURN_MARKER} working…"),
            Style::default().fg(Color::Green),
        )));
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

fn draw_input(frame: &mut Frame, area: Rect, session: &Session) {
    let working = session.status == Status::Working;

    let body = if working {
        Line::from(Span::styled("  waiting for the model…", dim()))
    } else {
        Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan)),
            Span::raw(session.input.clone()),
            Span::styled("▌", Style::default().fg(Color::Cyan)),
        ])
    };

    frame.render_widget(
        Paragraph::new(body)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(if working {
                        dim()
                    } else {
                        Style::default().fg(Color::Cyan)
                    }),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
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
    fn a_working_session_says_so() {
        let mut session = Session::new("partial");
        session.type_char('a');
        session.submit();
        let output = rendered(&session);
        assert!(output.contains("working"));
        assert!(output.contains("waiting for the model"));
    }

    #[test]
    fn the_transcript_distinguishes_speakers() {
        let mut session = Session::new("none");
        for c in "what is this".chars() {
            session.type_char(c);
        }
        session.submit();
        session.complete("it is a project", Vec::new());

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
        session.complete("x".repeat(5_000), Vec::new());
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
            session.complete(format!("reply number {turn}"), Vec::new());
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
}
