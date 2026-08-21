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
use bua_core::trust::TrustStore;
use bua_net::Egress;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as TermEvent, KeyCode, KeyEvent,
    KeyModifiers, MouseEvent, MouseEventKind,
};
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
        // Arrows scroll the transcript. There is no cursor movement to conflict with —
        // the input is a single line edited at its end — so the obvious keys are free to
        // do the obvious thing rather than requiring PageUp.
        KeyCode::Up => {
            session.scroll_up(1);
            Action::Redraw
        }
        KeyCode::Down => {
            session.scroll_down(1);
            Action::Redraw
        }
        KeyCode::PageUp => {
            session.scroll_up(10);
            Action::Redraw
        }
        KeyCode::PageDown => {
            session.scroll_down(10);
            Action::Redraw
        }
        // Jump to either end of the transcript.
        KeyCode::Home => {
            session.scroll_up(u16::MAX);
            Action::Redraw
        }
        KeyCode::End => {
            session.scroll_down(u16::MAX);
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

/// Interpret a mouse event.
///
/// The wheel is what most people reach for first, so it scrolls without any modifier.
pub fn handle_mouse(session: &mut Session, mouse: MouseEvent) -> Action {
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            session.scroll_up(3);
            Action::Redraw
        }
        MouseEventKind::ScrollDown => {
            session.scroll_down(3);
            Action::Redraw
        }
        _ => Action::None,
    }
}

/// Run the interface until the user leaves.
pub fn run(config: &Config, workspace: &Workspace, confinement: String) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Mouse capture is what makes the wheel scroll the transcript. It costs the
    // terminal's own text selection, so it is disabled again on the way out.
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = event_loop(&mut terminal, config, workspace, confinement);

    // Restore the terminal even if the loop failed: leaving a user in raw mode on an
    // alternate screen is worse than the original error.
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
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

    // Asked once, before any turn: the answer decides whether ordinary work in this directory
    // is interrupted for every write. Nothing is trusted unless the user says so.
    let mut trust = crate::trust_prompt::ask(terminal, workspace.root());
    if trust.is_empty() {
        session.note("this directory is not trusted; every write will be shown to you");
    } else {
        session.note(format!("trusting {}", workspace.root().display()));
    }

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

        let action = match event::read()? {
            TermEvent::Key(key) => handle_key(&mut session, key),
            TermEvent::Mouse(mouse) => handle_mouse(&mut session, mouse),
            _ => Action::None,
        };

        match action {
            Action::Quit => return Ok(()),
            Action::Submit(prompt) => {
                // Show "working" before the request goes out, so the interface is not
                // frozen without explanation.
                terminal
                    .draw(|frame| render::draw(frame, &session))
                    .map_err(io::Error::other)?;
                // The confirmer borrows the terminal to draw its prompt, so it is created
                // per turn rather than held for the loop's lifetime.
                let mut confirmer = crate::confirm::TerminalConfirmer::new(terminal);
                // The map is threaded through: a turn that writes untrusted data into a
                // trusted path records that, and the next turn must honour it.
                trust = run_turn(
                    &mut session,
                    config,
                    &egress,
                    workspace,
                    &mut confirmer,
                    &prompt,
                    trust,
                );
            }
            Action::None | Action::Redraw => {}
        }
    }
}

/// Run one turn and fold the result into the session.
#[allow(clippy::too_many_arguments)]
fn run_turn<C: bua_agent::Confirmer>(
    session: &mut Session,
    config: &Config,
    egress: &Egress,
    workspace: &Workspace,
    confirmer: &mut C,
    prompt: &str,
    trust: TrustStore,
) -> TrustStore {
    let mut sink = RecordingSink::new();
    let task = Task::new(prompt);

    // Kept so a failed turn does not lose the user's decisions. A turn that fails partway may
    // still have written something, and the rules recorded for it are inside the outcome we
    // will not receive — so this is the floor, never an upgrade.
    let fallback = trust.clone();

    match turn::run_with_trust(
        config, egress, workspace, &task, confirmer, &mut sink, trust,
    ) {
        Ok(outcome) => {
            let trail = sink.events().to_vec();
            session.complete(outcome.reply_for_display().to_string(), trail);
            if !outcome.clean {
                session.note("a policy gate refused something during that turn");
            }
            // Carries forward any rule the turn recorded, so a path that received untrusted
            // data cannot be read back as trusted by the next turn.
            outcome.trust
        }
        Err(error) => {
            // The trail is kept on failure too: a refusal is exactly when a user wants
            // to see what happened.
            let trail = sink.events().to_vec();
            session.fail(format!("error: {error}"));
            if let Some(last) = session.transcript.last_mut() {
                last.trail = trail;
            }
            fallback
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

    /// Arrows are the obvious keys for looking back, so they must work without a
    /// modifier and without needing PageUp.
    #[test]
    fn arrow_keys_scroll() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Up));
        assert_eq!(session.scroll, 1);
        handle_key(&mut session, key(KeyCode::Up));
        assert_eq!(session.scroll, 2);
        handle_key(&mut session, key(KeyCode::Down));
        assert_eq!(session.scroll, 1);
    }

    #[test]
    fn page_keys_scroll_further() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::PageUp));
        assert_eq!(session.scroll, 10);
        handle_key(&mut session, key(KeyCode::PageDown));
        assert_eq!(session.scroll, 0);
    }

    #[test]
    fn home_and_end_jump_to_the_extremes() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Home));
        assert_eq!(session.scroll, u16::MAX);
        handle_key(&mut session, key(KeyCode::End));
        assert_eq!(session.scroll, 0);
    }

    /// The wheel is what most people reach for, so it scrolls with no modifier.
    #[test]
    fn the_mouse_wheel_scrolls() {
        let mut session = Session::new("none");
        let wheel = |kind| MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };

        assert_eq!(
            handle_mouse(&mut session, wheel(MouseEventKind::ScrollUp)),
            Action::Redraw
        );
        assert_eq!(session.scroll, 3);
        handle_mouse(&mut session, wheel(MouseEventKind::ScrollDown));
        assert_eq!(session.scroll, 0);
    }

    /// Clicks and drags are not bound to anything, so they must be ignored rather than
    /// misinterpreted as scrolling.
    #[test]
    fn other_mouse_events_are_ignored() {
        let mut session = Session::new("none");
        let moved = MouseEvent {
            kind: MouseEventKind::Moved,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(handle_mouse(&mut session, moved), Action::None);
        assert_eq!(session.scroll, 0);
    }

    /// Typing must not be captured by the scroll bindings.
    #[test]
    fn scroll_keys_do_not_disturb_the_input() {
        let mut session = Session::new("none");
        for c in "hello".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        handle_key(&mut session, key(KeyCode::Up));
        handle_key(&mut session, key(KeyCode::Down));
        assert_eq!(session.input, "hello", "scrolling altered the input");
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
