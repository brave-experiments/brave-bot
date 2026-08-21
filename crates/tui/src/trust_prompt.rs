//! Asking, at startup, whether to trust the working directory.
//!
//! The answer decides how the session behaves. Trusting the directory means work inside it
//! proceeds without a prompt for every write, because reads from it return trusted data.
//! Declining means everything is untrusted, so every write is shown, which is the correct
//! behaviour for a directory whose contents came from somewhere else.
//!
//! Nothing is trusted by default. An unreadable terminal, an unexpected key, or a lost event
//! stream all resolve to declining, because the failure mode of guessing wrong here is that a
//! session silently writes to files nobody vouched for.

use bua_core::trust::TrustStore;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use std::path::Path;

/// What the user decided about the working directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Trust,
    Decline,
}

/// Ask about `directory`, returning the trust map the session should start with.
///
/// Trusting records the workspace root, which covers everything beneath it. Declining records
/// nothing, leaving an empty map in which no path is trusted.
pub fn ask<B: Backend>(terminal: &mut Terminal<B>, directory: &Path) -> TrustStore {
    let answer = match terminal.draw(|frame| draw(frame, directory)) {
        Ok(_) => read_answer(),
        // A terminal that cannot be drawn to cannot carry the question.
        Err(_) => Answer::Decline,
    };

    let mut trust = TrustStore::new();
    if answer == Answer::Trust {
        trust.trust(".");
    }
    trust
}

/// Block until the user answers.
fn read_answer() -> Answer {
    loop {
        match event::read() {
            Ok(TermEvent::Key(key)) => match key.code {
                KeyCode::Char('y' | 'Y') => return Answer::Trust,
                KeyCode::Char('n' | 'N') | KeyCode::Esc => return Answer::Decline,
                // Enter is deliberately not a yes: it is the key most likely to be pressed
                // out of habit, and this question grants standing permission.
                _ => continue,
            },
            Ok(_) => continue,
            Err(_) => return Answer::Decline,
        }
    }
}

/// Draw the question.
fn draw(frame: &mut ratatui::Frame, directory: &Path) {
    let area = centred(frame.area());
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from(vec![
            Span::styled(
                "Trust ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                directory.display().to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("?"),
        ]),
        Line::raw(""),
        Line::from(Span::raw(
            "Files here will be read as trusted, and edits to them will not be",
        )),
        Line::from(Span::raw(
            "shown to you one by one. Say no if you did not write this code.",
        )),
        Line::raw(""),
        Line::from(Span::styled(
            "Either way, anything derived from the web or from an untrusted",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "file is still shown before it is written.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                "  y",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" trust it    "),
            Span::styled(
                "n",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ask me about every write"),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(" trust this directory? "),
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
            Constraint::Percentage(15),
            Constraint::Percentage(70),
            Constraint::Percentage(15),
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

    fn rendered(directory: &str) -> String {
        let mut terminal = Terminal::new(TestBackend::new(72, 20)).expect("terminal");
        terminal
            .draw(|frame| draw(frame, Path::new(directory)))
            .expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn the_prompt_names_the_directory_and_both_answers() {
        let output = rendered("/home/me/project");
        assert!(output.contains("/home/me/project"));
        assert!(output.contains("trust it"));
        assert!(output.contains("every write"));
    }

    /// The question must say what saying yes actually does, since it grants standing
    /// permission rather than approving one action.
    #[test]
    fn the_prompt_explains_the_consequence() {
        let output = rendered("/tmp/x");
        assert!(output.contains("trusted"), "no mention of trust: {output}");
        // Wrapping can split a phrase across lines, so assert on a short fragment.
        assert!(
            output.contains("Say no if you"),
            "no guidance on when to decline: {output}"
        );
    }

    #[test]
    fn a_tiny_terminal_still_renders() {
        let mut terminal = Terminal::new(TestBackend::new(24, 8)).expect("terminal");
        terminal
            .draw(|frame| draw(frame, Path::new("/tmp/x")))
            .expect("must not panic on a small area");
    }

    /// Trusting records the root, which covers the whole tree.
    #[test]
    fn trusting_covers_the_whole_workspace() {
        let mut trust = TrustStore::new();
        trust.trust(".");
        assert!(trust.is_trusted("src/main.rs"));
        assert!(trust.is_trusted("deep/nested/file.txt"));
    }

    /// Declining leaves nothing trusted, so every write is shown.
    #[test]
    fn declining_trusts_nothing() {
        let trust = TrustStore::new();
        assert!(trust.is_empty());
        assert!(!trust.is_trusted("src/main.rs"));
        assert!(!trust.is_trusted("."));
    }
}
