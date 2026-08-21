//! The event loop.
//!
//! Runs turns one at a time. Each turn builds its own policy inside `turn::run`, so
//! nothing about the interface can extend a policy's life beyond the turn that created
//! it — the session only accumulates text for display.
//!
//! Turns run synchronously: the interface shows "working" and stops accepting input until
//! the reply arrives. That is honest about what is happening, and it keeps two turns from
//! ever being in flight together.

use bua_agent::Workspace;
use bua_agent::turn::{self, Task};
use bua_config::Config;
use bua_core::event::RecordingSink;
use bua_net::Egress;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use std::io;
use std::time::Duration;

use crate::render;
use crate::state::Session;

/// How long to wait for a key before redrawing. Short enough that a status change appears
/// promptly, long enough not to spin.
const POLL: Duration = Duration::from_millis(100);

/// What a key press asked for.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Redraw,
    Submit(String),
    Quit,
}

/// Interpret a key press against the session.
///
/// Separated from the loop so it can be tested without a terminal.
pub fn handle_key(session: &mut Session, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
        KeyCode::Char('c') if ctrl => {
            session.quit();
            Action::Quit
        }
        KeyCode::Char('d') if ctrl && session.input.is_empty() => {
            session.quit();
            Action::Quit
        }
        KeyCode::Char('t') if ctrl => {
            session.toggle_trail();
            Action::Redraw
        }
        KeyCode::Esc => {
            session.quit();
            Action::Quit
        }
        KeyCode::Enter => match session.submit() {
            Some(prompt) => Action::Submit(prompt),
            None => Action::None,
        },
        KeyCode::Backspace => {
            session.backspace();
            Action::Redraw
        }
        KeyCode::PageUp => {
            session.scroll_up(5);
            Action::Redraw
        }
        KeyCode::PageDown => {
            session.scroll_down(5);
            Action::Redraw
        }
        // Any other control combination is ignored rather than typed. Without this,
        // Ctrl-D on a non-empty line falls through and inserts a literal 'd'.
        KeyCode::Char(_) if ctrl => Action::None,
        KeyCode::Char(c) => {
            session.type_char(c);
            Action::Redraw
        }
        _ => Action::None,
    }
}

/// Run the interface until the user leaves.
pub fn run(config: &Config, workspace: &Workspace, confinement: String) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = event_loop(&mut terminal, config, workspace, confinement);

    // Restore the terminal even if the loop failed: leaving a user in raw mode on an
    // alternate screen is worse than the original error.
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Concrete in the backend rather than generic: the loop is only ever driven by a real
/// terminal, and a generic backend's error type carries no bounds to convert from.
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config: &Config,
    workspace: &Workspace,
    confinement: String,
) -> io::Result<()> {
    let egress = Egress::new();
    let mut session = Session::new(confinement);

    loop {
        terminal
            .draw(|frame| render::draw(frame, &session))
            .map_err(io::Error::other)?;

        if session.is_quitting() {
            return Ok(());
        }

        if !event::poll(POLL)? {
            continue;
        }

        let TermEvent::Key(key) = event::read()? else {
            continue;
        };

        match handle_key(&mut session, key) {
            Action::Quit => return Ok(()),
            Action::Submit(prompt) => {
                // Show "working" before the request goes out, so the interface is not
                // frozen without explanation.
                terminal
                    .draw(|frame| render::draw(frame, &session))
                    .map_err(io::Error::other)?;
                run_turn(&mut session, config, &egress, workspace, &prompt);
            }
            Action::None | Action::Redraw => {}
        }
    }
}

/// Run one turn and fold the result into the session.
fn run_turn(
    session: &mut Session,
    config: &Config,
    egress: &Egress,
    workspace: &Workspace,
    prompt: &str,
) {
    let mut sink = RecordingSink::new();
    let task = Task::new(prompt);

    match turn::run(config, egress, workspace, &task, &mut sink) {
        Ok(outcome) => {
            let trail = sink.events().to_vec();
            session.complete(outcome.reply_for_display().to_string(), trail);
            if !outcome.clean {
                session.note("a policy gate refused something during that turn");
            }
        }
        Err(error) => {
            // The trail is kept on failure too: a refusal is exactly when a user wants
            // to see what happened.
            let trail = sink.events().to_vec();
            session.fail(format!("error: {error}"));
            if let Some(last) = session.transcript.last_mut() {
                last.trail = trail;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Status;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn typing_a_character_asks_for_a_redraw() {
        let mut session = Session::new("none");
        assert_eq!(
            handle_key(&mut session, key(KeyCode::Char('a'))),
            Action::Redraw
        );
        assert_eq!(session.input, "a");
    }

    #[test]
    fn enter_submits_the_prompt() {
        let mut session = Session::new("none");
        for c in "hello".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("hello".to_string())
        );
    }

    #[test]
    fn enter_on_empty_input_does_nothing() {
        let mut session = Session::new("none");
        assert_eq!(handle_key(&mut session, key(KeyCode::Enter)), Action::None);
        assert_eq!(session.status, Status::Idle);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut session = Session::new("none");
        assert_eq!(handle_key(&mut session, ctrl('c')), Action::Quit);
        assert!(session.is_quitting());
    }

    #[test]
    fn escape_quits() {
        let mut session = Session::new("none");
        assert_eq!(handle_key(&mut session, key(KeyCode::Esc)), Action::Quit);
    }

    /// Ctrl-D only quits on an empty line, matching shell behaviour, so it cannot discard
    /// a half-typed prompt.
    #[test]
    fn ctrl_d_quits_only_when_the_line_is_empty() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('x')));
        assert_ne!(handle_key(&mut session, ctrl('d')), Action::Quit);
        assert!(!session.is_quitting());

        handle_key(&mut session, key(KeyCode::Backspace));
        assert!(session.input.is_empty(), "backspace did not clear the line");
        assert_eq!(handle_key(&mut session, ctrl('d')), Action::Quit);
    }

    #[test]
    fn ctrl_t_toggles_the_trail() {
        let mut session = Session::new("none");
        assert_eq!(handle_key(&mut session, ctrl('t')), Action::Redraw);
        assert!(session.show_trail);
        handle_key(&mut session, ctrl('t'));
        assert!(!session.show_trail);
    }

    #[test]
    fn page_keys_scroll() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::PageUp));
        assert_eq!(session.scroll, 5);
        handle_key(&mut session, key(KeyCode::PageDown));
        assert_eq!(session.scroll, 0);
    }

    #[test]
    fn backspace_deletes() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('a')));
        handle_key(&mut session, key(KeyCode::Backspace));
        assert!(session.input.is_empty());
    }

    /// Keys that mean nothing here must be ignored rather than mishandled.
    #[test]
    fn unknown_keys_are_ignored() {
        let mut session = Session::new("none");
        assert_eq!(handle_key(&mut session, key(KeyCode::F(5))), Action::None);
        assert_eq!(handle_key(&mut session, key(KeyCode::Insert)), Action::None);
    }

    /// A second submission cannot start while a turn is in flight.
    #[test]
    fn enter_is_inert_while_working() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('a')));
        handle_key(&mut session, key(KeyCode::Enter));
        assert_eq!(session.status, Status::Working);

        handle_key(&mut session, key(KeyCode::Char('b')));
        assert_eq!(handle_key(&mut session, key(KeyCode::Enter)), Action::None);
    }
}
