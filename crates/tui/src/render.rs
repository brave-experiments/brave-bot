//! Drawing the session.
//!
//! Untrusted content — model replies, tool output — is shown, but the interface makes its
//! provenance visible rather than presenting everything as equally authoritative. That is
//! the point of showing labels in the trail: a user can see which text the system trusted
//! and which it merely carried.

use bua_core::event::{Event, Role};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::state::{Session, Speaker, Status};

/// Draw the whole interface.
pub fn draw(frame: &mut Frame, session: &Session) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // status bar
            Constraint::Min(3),    // transcript
            Constraint::Length(3), // input
        ])
        .split(frame.area());

    draw_status(frame, areas[0], session);
    draw_transcript(frame, areas[1], session);
    draw_input(frame, areas[2], session);
}

fn draw_status(frame: &mut Frame, area: Rect, session: &Session) {
    let activity = match session.status {
        Status::Idle => "ready",
        Status::Working => "working…",
        Status::Quitting => "exiting",
    };

    let line = Line::from(vec![
        Span::styled(
            " bua ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" {activity}")),
        Span::styled(
            format!("  confinement: {}", session.confinement),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            if session.show_trail {
                "  [trail on]"
            } else {
                ""
            },
            Style::default().fg(Color::Yellow),
        ),
    ]);

    frame.render_widget(Paragraph::new(line), area);
}

fn draw_transcript(frame: &mut Frame, area: Rect, session: &Session) {
    let mut lines: Vec<Line> = Vec::new();

    for entry in &session.transcript {
        let (marker, style) = match entry.speaker {
            Speaker::User => (
                "you",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Speaker::Assistant => ("bua", Style::default().fg(Color::Green)),
            Speaker::System => ("---", Style::default().fg(Color::Yellow)),
        };

        lines.push(Line::from(Span::styled(format!("{marker}:"), style)));
        for text_line in entry.text.lines() {
            lines.push(Line::from(format!("  {text_line}")));
        }

        if session.show_trail && !entry.trail.is_empty() {
            for event in &entry.trail {
                lines.push(trail_line(event));
            }
        }
        lines.push(Line::raw(""));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Ask a question about this workspace. Ctrl-T shows the audit trail.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    // Scroll is measured from the bottom, so new output stays visible by default.
    let height = area.height.saturating_sub(2);
    let total = lines.len() as u16;
    let max_offset = total.saturating_sub(height);
    let offset = max_offset.saturating_sub(session.scroll.min(max_offset));

    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" session "))
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
    let dim = Style::default().fg(Color::DarkGray);
    let bad = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);

    match event {
        Event::GatePassed { gate, detail } => {
            Line::from(Span::styled(format!("    · {gate}: {detail}"), dim))
        }
        Event::GateBlocked { gate, reason, .. } => {
            Line::from(Span::styled(format!("    ✗ {gate}: {reason}"), bad))
        }
        Event::Observed { capability, label } => Line::from(Span::styled(
            format!("    · {capability} produced {label}"),
            dim,
        )),
        Event::SlotWritten { slot, label } => {
            Line::from(Span::styled(format!("    · slot {slot} at {label}"), dim))
        }
        Event::Declassified { slot, from, to, .. } => Line::from(Span::styled(
            format!("    · released {slot} {from} → {to}"),
            dim,
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
            let text = format!(
                "    {} {tool}.{field} [{role}] {label}",
                if *allowed { "·" } else { "✗" }
            );
            Line::from(Span::styled(text, if *allowed { dim } else { bad }))
        }
    }
}

fn draw_input(frame: &mut Frame, area: Rect, session: &Session) {
    let (title, body, style) = match session.status {
        Status::Working => (
            " waiting ",
            "…".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        _ => (" ask ", format!("{}▌", session.input), Style::default()),
    };

    frame.render_widget(
        Paragraph::new(Span::styled(body, style))
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
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
        let buffer = terminal.backend().buffer().clone();
        buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn an_empty_session_shows_a_hint() {
        let session = Session::new("kernel-enforced");
        let output = rendered(&session);
        assert!(output.contains("Ask a question"), "no hint shown");
    }

    #[test]
    fn the_status_bar_reports_confinement() {
        let session = Session::new("kernel-enforced");
        let output = rendered(&session);
        assert!(output.contains("kernel-enforced"));
        assert!(output.contains("ready"));
    }

    #[test]
    fn a_working_session_reports_it() {
        let mut session = Session::new("partial");
        session.type_char('a');
        session.submit();
        let output = rendered(&session);
        assert!(output.contains("working"));
    }

    #[test]
    fn the_transcript_shows_both_speakers() {
        let mut session = Session::new("none");
        for c in "what is this".chars() {
            session.type_char(c);
        }
        session.submit();
        session.complete("it is a project", Vec::new());

        let output = rendered(&session);
        assert!(output.contains("what is this"));
        assert!(output.contains("it is a project"));
        assert!(output.contains("you:"));
        assert!(output.contains("bua:"));
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

        let hidden = rendered(&session);
        assert!(!hidden.contains("(U,priv)"));

        session.toggle_trail();
        let shown = rendered(&session);
        assert!(shown.contains("(U,priv)"), "the trail did not appear");
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

        let output = rendered(&session);
        assert!(output.contains("injection blocked"));
    }

    #[test]
    fn a_system_note_is_shown() {
        let mut session = Session::new("none");
        session.note("confinement unavailable");
        let output = rendered(&session);
        assert!(output.contains("confinement unavailable"));
    }

    /// Long replies must not panic the renderer.
    #[test]
    fn a_long_reply_renders() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit();
        session.complete("x".repeat(5_000), Vec::new());
        let output = rendered(&session);
        assert!(output.contains('x'));
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
}
