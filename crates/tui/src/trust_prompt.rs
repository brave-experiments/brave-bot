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

use bravebot_core::trust::TrustStore;
use bravebot_i18n::t;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use std::path::Path;

use crate::theme;

/// What the user decided about the working directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Trust,
    Decline,
    /// Leave without starting a session at all.
    Leave,
}

/// Ask about `directory`, returning the trust map the session should start with.
///
/// Trusting records the workspace root, which covers everything beneath it. Declining records
/// nothing, leaving an empty map in which no path is trusted.
///
/// Asked afresh every time a session begins. The answer is standing permission for as long as
/// that session lasts and no longer: it is not written down anywhere a later launch will read it,
/// so nothing this grants can be inherited by a session whose user was never asked. A resumed
/// session is the one exception, and it inherits the answer its own user gave rather than
/// skipping the question, which is why this is not called at all in that case.
///
/// `None` is the third answer: the user pressed Ctrl-C, which is neither trusting nor declining
/// but a request to leave, so no session begins at all.
pub fn ask<B: Backend>(terminal: &mut Terminal<B>, directory: &Path) -> Option<TrustStore> {
    let answer = match terminal.draw(|frame| draw(frame, directory)) {
        Ok(_) => read_answer(),
        // A terminal that cannot be drawn to cannot carry the question.
        Err(_) => Answer::Decline,
    };

    trust_for(answer)
}

/// The map an answer starts the session with, or `None` for leaving.
fn trust_for(answer: Answer) -> Option<TrustStore> {
    match answer {
        Answer::Leave => None,
        Answer::Trust => {
            let mut trust = TrustStore::new();
            trust.trust(".");
            Some(trust)
        }
        Answer::Decline => Some(TrustStore::new()),
    }
}

/// Block until the user answers.
fn read_answer() -> Answer {
    loop {
        match event::read() {
            // Presses only: the interface asks for disambiguated keys, so a release arrives too,
            // and answering a question twice grants standing permission on one keystroke.
            Ok(TermEvent::Key(key)) if key.kind != event::KeyEventKind::Press => continue,
            Ok(TermEvent::Key(key)) => match answer_for(key) {
                Some(answer) => return answer,
                None => continue,
            },
            Ok(_) => continue,
            Err(_) => return Answer::Decline,
        }
    }
}

/// Interpret one key press, or `None` for a key that answers nothing.
///
/// Separated from the loop so it can be tested without a terminal.
fn answer_for(key: KeyEvent) -> Option<Answer> {
    // Raw mode delivers Ctrl-C as a key rather than as a signal, so a prompt that ignored it
    // would be a screen with no way out: the interrupt everyone reaches for would do nothing.
    // It is not an answer to the question, so it starts nothing rather than declining.
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') => Some(Answer::Leave),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Char('y' | 'Y') => Some(Answer::Trust),
        KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(Answer::Decline),
        // Enter is deliberately not a yes: it is the key most likely to be pressed
        // out of habit, and this question grants standing permission.
        _ => None,
    }
}

/// Draw the question.
fn draw(frame: &mut ratatui::Frame, directory: &Path) {
    let area = centred(frame.area());
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{} ", t!(trust_directory_question)),
                Style::default()
                    .fg(theme::brand_primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                directory.display().to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("?"),
        ]),
        Line::raw(""),
        // One line each, wrapped by the paragraph rather than broken here: a translation does
        // not break where the English did, and a sentence split into two spans cannot be rewrapped.
        Line::from(Span::raw(t!(trust_directory_explained))),
        Line::raw(""),
        Line::from(Span::styled(
            t!(trust_directory_regardless),
            Style::default().fg(theme::muted()),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                "  y",
                Style::default()
                    .fg(theme::ok())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" {}    ", t!(trust_directory_yes))),
            Span::styled(
                "n",
                Style::default()
                    .fg(theme::fail())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" {}    ", t!(trust_directory_no))),
            Span::styled(
                "ctrl-c",
                Style::default()
                    .fg(theme::muted())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}", t!(quit)),
                Style::default().fg(theme::muted()),
            ),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme::brand_primary()))
                    .title(format!(" {} ", t!(trust_directory_title)))
                    // The background as well as the border, because `Clear` empties the cells
                    // under the panel without colouring them.
                    .style(Style::default().bg(theme::background()).fg(theme::text())),
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
    use ratatui::style::Color;

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

    /// `Clear` empties the cells under the panel without colouring them, so a prompt that styled
    /// only its border is a hole in the palette: themed border, terminal-default everything else.
    /// This is the first screen of a session and it grants standing permission over a whole tree,
    /// so its chrome is the first thing that says the question is the system's own.
    #[test]
    fn the_prompt_paints_the_themes_background_inside_its_border() {
        let _held = theme::exclusive();
        let theme = theme::find("nord").expect("nord is built in");
        theme::apply(&theme);
        let painted = theme::background();

        let mut terminal = Terminal::new(TestBackend::new(72, 20)).expect("terminal");
        terminal
            .draw(|frame| draw(frame, Path::new("/home/me/project")))
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        theme::apply_brave();

        let inside = centred(*buffer.area());
        // Every cell the border encloses, including the rows the prose did not reach: an unpainted
        // row below the keys is the same hole as an unpainted one beside them.
        for y in 1..inside.height - 1 {
            for x in 1..inside.width - 1 {
                let cell = &buffer[(inside.x + x, inside.y + y)];
                assert_eq!(
                    cell.bg, painted,
                    "the cell at {x},{y} kept the terminal's own background"
                );
                assert_ne!(
                    cell.fg,
                    Color::Reset,
                    "the cell at {x},{y} kept the terminal's own text colour"
                );
            }
        }
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

    /// Ctrl-C is the interrupt everyone reaches for, and raw mode turns it into an ordinary key
    /// press. A prompt that ignored it would be a screen with no way out.
    #[test]
    fn ctrl_c_leaves_rather_than_answering_the_question() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(answer_for(key), Some(Answer::Leave));
    }

    /// Leaving is not a quiet decline: a session that started anyway would be one the user
    /// never agreed to have.
    #[test]
    fn leaving_starts_no_session() {
        assert!(trust_for(Answer::Leave).is_none());
        assert!(trust_for(Answer::Decline).is_some());
    }

    /// A plain `c` is not an interrupt, and neither is any other control chord.
    #[test]
    fn only_ctrl_c_leaves() {
        assert_eq!(
            answer_for(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            answer_for(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)),
            None
        );
    }

    /// Declining leaves nothing trusted, so every write is shown. Asked of the answer rather than
    /// of a store built here, which is what the clause is about: a `trust_for` that trusted `.`
    /// on a decline would have satisfied a test that only built its own empty store.
    #[test]
    fn declining_trusts_nothing() {
        let trust = trust_for(Answer::Decline).expect("declining still starts a session");
        assert!(trust.is_empty());
        assert!(!trust.is_trusted("src/main.rs"));
        assert!(!trust.is_trusted("."));
    }
}
