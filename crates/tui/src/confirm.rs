//! Asking the user about a write, in the terminal.
//!
//! Turns run synchronously, so this can draw a prompt and block on a keypress from inside
//! the turn that requested the write. The alternative — collecting writes and asking
//! afterwards — would mean the model continuing on the assumption a write had happened.
//!
//! Nothing is approved by default. An unreadable terminal, an unexpected key, or a lost
//! event all resolve to refusal.

use bua_agent::confirm::{Confirmer, Decision, WriteRequest};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

/// How many lines of the proposed body to show.
///
/// Enough to judge a small edit; a reviewer who needs more can decline and ask to see the
/// file. Showing an unbounded body would push the question itself off screen.
const PREVIEW_LINES: usize = 16;

/// Prompts in the terminal for each write.
pub struct TerminalConfirmer<'t, B: Backend> {
    terminal: &'t mut Terminal<B>,
}

impl<'t, B: Backend> TerminalConfirmer<'t, B> {
    pub fn new(terminal: &'t mut Terminal<B>) -> Self {
        Self { terminal }
    }
}

impl<B: Backend> Confirmer for TerminalConfirmer<'_, B> {
    fn confirm_write(&mut self, request: &WriteRequest) -> Decision {
        // A terminal that cannot be drawn to cannot carry a question, so refuse rather
        // than proceed unseen.
        if self.terminal.draw(|frame| draw(frame, request)).is_err() {
            return Decision::Reject;
        }

        loop {
            match event::read() {
                Ok(TermEvent::Key(key)) => match key.code {
                    KeyCode::Char('y' | 'Y') => return Decision::Approve,
                    KeyCode::Char('n' | 'N') | KeyCode::Esc => return Decision::Reject,
                    // Enter is deliberately not an approval: it is the key most likely to
                    // be pressed out of habit.
                    _ => continue,
                },
                Ok(_) => continue,
                // Losing the event stream must not approve anything.
                Err(_) => return Decision::Reject,
            }
        }
    }
}

/// Draw the confirmation over the session.
fn draw(frame: &mut ratatui::Frame, request: &WriteRequest) {
    let area = centred(frame.area());
    frame.render_widget(Clear, area);

    let verb = if request.is_overwrite() {
        "Overwrite"
    } else {
        "Create"
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{verb} "),
                Style::default()
                    .fg(if request.is_overwrite() {
                        Color::Yellow
                    } else {
                        Color::Green
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                request.path.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::raw(""),
    ];

    if let Some(existing) = &request.existing {
        lines.push(Line::from(Span::styled(
            format!("  replaces {} existing lines", existing.lines().count()),
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::raw(""));
    }

    // The preview is capped by the space actually available, not just by PREVIEW_LINES:
    // the question and the keys must stay on screen, and a fixed cap taller than the box
    // would push them off.
    let reserved = lines.len() + 3;
    let budget = (area.height as usize)
        .saturating_sub(2) // borders
        .saturating_sub(reserved);
    let shown = PREVIEW_LINES.min(budget);

    for line in request.contents.lines().take(shown) {
        lines.push(Line::from(Span::styled(
            format!("  {line}"),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let total = request.contents.lines().count();
    if total > shown {
        lines.push(Line::from(Span::styled(
            format!("  … {} more lines", total - shown),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(
            "  y",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" write it    "),
        Span::styled(
            "n",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" leave it alone"),
    ]));

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(" approve this write? "),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// A centred box, sized to the terminal but never larger than it.
fn centred(area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(10),
            Constraint::Percentage(80),
            Constraint::Percentage(10),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(5),
            Constraint::Percentage(90),
            Constraint::Percentage(5),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn request(contents: &str, existing: Option<&str>) -> WriteRequest {
        WriteRequest {
            path: "src/main.rs".into(),
            contents: contents.into(),
            existing: existing.map(str::to_string),
        }
    }

    fn rendered(request: &WriteRequest) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal.draw(|frame| draw(frame, request)).expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn a_new_file_prompt_shows_the_path_and_body() {
        let output = rendered(&request("fn main() {}", None));
        assert!(output.contains("Create"));
        assert!(output.contains("src/main.rs"));
        assert!(output.contains("fn main()"));
        assert!(output.contains("write it"));
    }

    /// Overwriting is the dangerous case, so the prompt must say what would be lost.
    #[test]
    fn an_overwrite_prompt_says_what_it_replaces() {
        let output = rendered(&request("new", Some("a\nb\nc")));
        assert!(output.contains("Overwrite"));
        assert!(
            output.contains("replaces 3 existing lines"),
            "the prompt does not say what is lost: {output}"
        );
    }

    /// A large body must not push the question off screen.
    #[test]
    fn a_long_body_is_truncated_and_says_so() {
        let body = (0..200)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let output = rendered(&request(&body, None));
        assert!(output.contains("more lines"), "no truncation notice");
        assert!(output.contains("write it"), "the question was pushed off");
    }

    #[test]
    fn a_tiny_terminal_still_renders_the_prompt() {
        let mut terminal = Terminal::new(TestBackend::new(20, 8)).expect("terminal");
        terminal
            .draw(|frame| draw(frame, &request("x", None)))
            .expect("must not panic on a small area");
    }
}
