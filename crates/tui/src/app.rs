//! The event loop.
//!
//! Runs turns one at a time. Each turn builds its own policy inside `turn::run`, so
//! nothing about the interface can extend a policy's life beyond the turn that created
//! it. What does outlive a turn is the conversation, which is how a follow-up like "try that
//! again" has anything to refer to.
//!
//! Turns run synchronously: the interface shows "working" and stops accepting input until
//! the reply arrives. That is honest about what is happening, and it keeps two turns from
//! ever being in flight together.

use bua_agent::Workspace;
use bua_agent::conversation::Conversation;
use bua_agent::turn::{self, Task};
use bua_config::Config;
use bua_core::cancel::Cancel;
use bua_core::event::RecordingSink;
use bua_core::trust::TrustStore;
use bua_net::Egress;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event as TermEvent, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use std::io::{self, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::render;
use crate::select;
use crate::state::{Session, Status};

/// How long to wait for a key before redrawing. Short enough that a status change appears
/// promptly, long enough not to spin.
const POLL: Duration = Duration::from_millis(100);

/// Asks for motion reported only while a button is held.
///
/// Sent after [`EnableMouseCapture`], which asks for all three tracking modes at once, including
/// the one that reports a pointer merely crossing the window. Turning that one off is the whole
/// intent, but terminals disagree about what the three modes are: some keep a flag per mode and
/// use the highest one set, others keep a single state that the last request wins. Sending the
/// two that are wanted again, after the one that is not, lands both kinds in the same place.
/// Without that, a terminal of the second kind reads it as "no tracking at all" and the wheel
/// goes back to scrolling the window behind the session.
const TRACK_MOTION_ONLY_WHILE_DRAGGING: &str = "\x1b[?1003l\x1b[?1000h\x1b[?1002h";

/// How often to redraw while a turn runs. Matches the spinner's own frame time so the animation
/// advances by one glyph per redraw rather than skipping.
const FRAME: Duration = Duration::from_millis(120);

/// What a key press asked for.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Redraw,
    Submit(String),
    /// Stop the turn in flight.
    Cancel,
    /// Take what the selection covers, which needs the screen as it was last drawn.
    Copy,
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
        // Escape means "stop what is happening" before it means anything else, so a turn in
        // flight is cancelled first. The prompt comes back for editing rather than being lost.
        KeyCode::Esc if session.status == Status::Working => Action::Cancel,
        // Then it discards a half-typed prompt, and only leaves once the line is already empty.
        // Pressing it to abandon a thought should not also end the session.
        KeyCode::Esc if !session.input.is_empty() => {
            session.clear_input();
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
        // Up and Down walk the prompt history, which is what they do in a shell and so what a
        // user expects at a prompt. Scrolling the transcript keeps the wheel and the page keys,
        // and Up still scrolls once there is no history left to walk.
        KeyCode::Up if !session.history.is_empty() => {
            session.recall_older();
            Action::Redraw
        }
        KeyCode::Down if session.history.is_browsing() => {
            session.recall_newer();
            Action::Redraw
        }
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

/// Interpret a paste.
///
/// A paste is one act rather than a run of keys, which is the whole point of asking the
/// terminal for it separately: the text lands in the box and the user decides when to send it.
/// It never submits, whatever it ends with.
pub fn handle_paste(session: &mut Session, text: &str) -> Action {
    session.paste(text);
    Action::Redraw
}

/// Interpret a mouse event.
///
/// The wheel is what most people reach for first, so it scrolls without any modifier.
///
/// Dragging selects. Capturing the mouse for the wheel is what took the terminal's own selection
/// away, so the drag that would have highlighted a line arrives here instead, and answering it
/// is the only way a user gets to copy anything.
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
        MouseEventKind::Down(MouseButton::Left) => {
            session.begin_selection(mouse.row, mouse.column);
            Action::Redraw
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            session.extend_selection(mouse.row, mouse.column);
            Action::Redraw
        }
        MouseEventKind::Up(MouseButton::Left) => Action::Copy,
        _ => Action::None,
    }
}

/// Take what the selection covers and put it on the clipboard.
///
/// What the user swept over is what they saw: wrapped, scrolled and trimmed exactly as it was
/// drawn. So it is read back off a frame rather than out of the transcript, which would have to
/// be laid out a second time to say what any of it looked like.
///
/// Drawn again to get one. A finished draw resets the buffer the next one will be built in, so
/// the frame that drew the screen is the only place the screen can be read from.
///
/// A click that swept over nothing just puts the selection away.
fn copy_selection(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    session: &mut Session,
) -> io::Result<()> {
    let Some(selection) = session.selection else {
        return Ok(());
    };
    if selection.is_empty() {
        session.clear_selection();
        return Ok(());
    }

    let text = {
        let completed = terminal
            .draw(|frame| render::draw(frame, session))
            .map_err(io::Error::other)?;
        select::text(completed.buffer, &selection)
    };

    // Nothing but the padding between widgets, which is not something to put on a clipboard and
    // not something to claim to have copied either.
    if text.is_empty() {
        return Ok(());
    }

    if crate::clipboard::copy(&text) {
        session.note_copied(text.chars().count());
    }
    Ok(())
}

/// What a session begins with.
#[derive(Debug, Default)]
pub enum Start {
    /// A new session, with nothing behind it.
    #[default]
    Fresh,
    /// Ask which of this directory's sessions to pick up, if there are any.
    Choose,
    /// A session read back off disk, continuing where it left off.
    Resuming(Box<crate::sessions::Record>),
}

/// Run the interface until the user leaves.
pub fn run(
    config: &Config,
    workspace: &Workspace,
    confinement: String,
    start: Start,
) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Mouse capture is what makes the wheel scroll the transcript. It costs the
    // terminal's own text selection, so it is disabled again on the way out.
    //
    // Bracketed paste is what keeps a pasted prompt from sending itself. Without it the
    // terminal delivers a paste as ordinary keystrokes, and the newline most clipboards carry
    // at the end arrives as Enter.
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;

    // Mouse capture asks for motion reported whether or not a button is down, which is a stream
    // of events for a pointer merely crossing the window and a redraw for each one. Only the
    // drag matters here, so all-motion reporting goes back off: what stays on reports the
    // buttons, the wheel, and motion while a button is held, which is the gesture being read.
    write!(stdout, "{TRACK_MOTION_ONLY_WHILE_DRAGGING}")?;
    stdout.flush()?;

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    // Asked before the session begins, so what it starts with is settled before anything is
    // drawn for it. Choosing nothing is an ordinary session rather than an error.
    let start = match start {
        Start::Choose => match crate::resume::choose(&mut terminal, workspace.root()) {
            crate::resume::Choice::Resume(record) => Some(Start::Resuming(record)),
            crate::resume::Choice::Fresh => Some(Start::Fresh),
            // Leaving at the picker starts nothing. The terminal is still put back below.
            crate::resume::Choice::Quit => None,
        },
        chosen => Some(chosen),
    };

    let result = match start {
        Some(start) => event_loop(&mut terminal, config, workspace, confinement, start),
        None => Ok(()),
    };

    // Restore the terminal even if the loop failed: leaving a user in raw mode on an
    // alternate screen is worse than the original error.
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
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
    start: Start,
) -> io::Result<()> {
    // The one place persistence is turned on: history in ~/.bua outlives the session.
    let mut session = Session::new(confinement).with_stored_history();

    // Outlives every turn, which is the point: a turn begins with the exchange so far rather
    // than with nothing, so the user can say "try that again" and be understood. A resumed
    // session begins with an exchange that outlived the process it happened in.
    let (mut conversation, mut stored, inherited_trust) = match start {
        // Already answered before the loop was entered: the picker runs once, in `run`.
        Start::Fresh | Start::Choose => (
            Conversation::new(),
            crate::sessions::Handle::begin(workspace.root()),
            None,
        ),
        Start::Resuming(record) => {
            let handle = crate::sessions::Handle::resuming(workspace.root(), &record);
            let conversation = Conversation::restored(record.conversation.clone());
            // Shown before anything else, because a session that silently continues something
            // the user cannot see is one they will contradict without meaning to. The trail comes
            // out of the audit beside the record, so Ctrl-T answers for the whole session rather
            // than only for the turns this process ran.
            let recalled = crate::sessions::recall(workspace.root(), &record);
            session.replay(&conversation, &record.title, &recalled);
            session.restore_spend(record.tokens);
            // Said after the transcript, so it reads as a caveat on what was just shown: the work
            // it describes may not be in the tree the user is now looking at.
            if let Some(note) = crate::sessions::branch_note(
                record.branch.as_deref(),
                crate::sessions::branch_of(workspace.root()).as_deref(),
            ) {
                session.note(note);
            }
            // The same caveat about the other half of what produced that transcript: not the
            // tree it ran against, but the code that ran.
            if let Some(note) =
                crate::sessions::build_note(record.build.as_deref(), crate::BUILD)
            {
                session.note(note);
            }
            // The trust map goes with the session, so picking one up carries the answer its own
            // user gave. `None` for a record from before this was kept, which is asked about.
            let inherited = record.trust_map();
            (conversation, handle, inherited)
        }
    };

    // Settled once, before any turn. Nothing means the user left at the question, and a session
    // they never agreed to have must not begin behind it.
    let Some(mut trust) = opening_trust(terminal, &mut session, workspace.root(), inherited_trust)
    else {
        return Ok(());
    };

    // Drawn when something has changed rather than on every pass. A drag arrives as a stream of
    // positions, and a frame for each costs more than the whole gesture is worth: with a long
    // transcript the queue outruns the drawing and the highlight trails seconds behind the
    // pointer. Coalescing a burst into one frame is what makes it keep up.
    let mut needs_draw = true;
    let mut drawn_at = Instant::now();

    loop {
        // Waiting for the burst to end, but not indefinitely: a drag that never pauses would
        // otherwise show nothing until it stopped.
        let waited_long_enough = drawn_at.elapsed() >= FRAME;
        if needs_draw && (waited_long_enough || !event::poll(Duration::ZERO)?) {
            terminal
                .draw(|frame| render::draw(frame, &session))
                .map_err(io::Error::other)?;
            needs_draw = false;
            drawn_at = Instant::now();
        }

        if session.is_quitting() {
            return Ok(());
        }

        if !event::poll(POLL)? {
            continue;
        }

        let action = match event::read()? {
            TermEvent::Key(key) => handle_key(&mut session, key),
            TermEvent::Mouse(mouse) => handle_mouse(&mut session, mouse),
            TermEvent::Paste(text) => handle_paste(&mut session, &text),
            _ => Action::None,
        };

        needs_draw |= !matches!(action, Action::None);

        match action {
            Action::Quit => return Ok(()),
            Action::Copy => copy_selection(terminal, &mut session)?,
            Action::Submit(prompt) => {
                // Both are threaded through: a turn that writes untrusted data into a trusted
                // path records that, and the next turn must honour it, and a turn that has been
                // had is a turn the next one can be asked about.
                let events;
                (conversation, trust, events) = run_turn_animated(
                    terminal,
                    &mut session,
                    config,
                    workspace,
                    &prompt,
                    conversation,
                    trust,
                )?;

                // Written after each turn rather than at the end, because the end may never
                // come: the session worth resuming is the one whose machine slept and never
                // woke. Best-effort, like everything else under ~/.bua.
                stored.save(
                    &prompt,
                    crate::sessions::Standing {
                        conversation: &conversation.snapshot(),
                        turns: session.turns,
                        tokens: session.tokens,
                        todos: &session.todos_by_turn(),
                        trust: &trust,
                    },
                );
                stored.append_audit(session.turns, &events);
            }
            // Only reachable while a turn runs, which is handled inside `run_turn_animated`.
            Action::Cancel | Action::None | Action::Redraw => {}
        }
    }
}

/// The trust map the session starts with, or nothing if the user asked to leave.
///
/// A fresh session always asks, whatever any session in this directory answered before. The
/// question grants standing permission, and a launch that skipped it because someone said yes
/// last week would be granting that permission on behalf of a user who was never asked, which is
/// trust assumed from silence rather than granted.
///
/// Resuming is the one case that does not ask, and it is not an exception to that: the map comes
/// out of the record of the very session being picked up, so the answer being honoured is the one
/// its own user gave. It carries the rules that session's writes recorded too, which is what stops
/// a resumed turn reading back a file an earlier turn poisoned. A record from before the map was
/// kept has none, and is asked about.
fn opening_trust(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    session: &mut Session,
    root: &std::path::Path,
    inherited: Option<TrustStore>,
) -> Option<TrustStore> {
    // Said only when resuming. On a fresh start the user has just answered the question and does
    // not need telling where the answer came from.
    let (trust, how) = match inherited {
        Some(trust) => (trust, " (as this session left it)"),
        None => (crate::trust_prompt::ask(terminal, root)?, ""),
    };

    if trust.is_trusted(".") {
        session.note(format!("trusting {}{how}", root.display()));
    } else {
        session.note("this directory is not trusted; every write will be shown to you");
    }
    Some(trust)
}

/// Run a turn on a worker thread, redrawing while it works.
///
/// The turn itself blocks on network requests, so running it here would freeze the indicator on
/// its first frame and make a slow model look like a hang. Off-thread, the loop below keeps
/// drawing, and the elapsed time and spinner advance on their own.
///
/// Write approvals come back over a channel because only this thread owns the terminal. The
/// worker blocks until an answer arrives, which is what a write must wait for anyway.
#[allow(clippy::too_many_arguments)]
fn run_turn_animated(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    session: &mut Session,
    config: &Config,
    workspace: &Workspace,
    prompt: &str,
    conversation: Conversation,
    trust: TrustStore,
) -> io::Result<(Conversation, TrustStore, Vec<bua_core::event::Event>)> {
    // One channel for everything the worker sends, because the main thread waits on exactly one
    // thing and `mpsc` cannot select across two. Only a write expects a reply.
    let (to_main, from_worker) = mpsc::channel::<crate::remote_confirm::ToMain>();
    let (answer_tx, answer_rx) = mpsc::channel::<bua_agent::Decision>();

    // A fresh token per turn: reusing one could cancel a turn before it started.
    let cancel = Cancel::new();
    let worker_cancel = cancel.clone();

    // Cloned rather than borrowed so the worker owns everything it needs. Config and Workspace
    // are cheap handles; Egress builds its own connection pool.
    let worker_config = config.clone();
    let worker_workspace = workspace.clone();
    let task = Task::new(prompt);
    // Kept so a failed turn does not lose the user's decisions.
    let fallback = trust.clone();

    let worker = thread::spawn(move || {
        let mut sink = RecordingSink::new();
        // Two handles over one channel back to the thread that owns the terminal: one asks about
        // writes and waits, the other reports progress and moves on.
        let mut reporter = crate::remote_confirm::RemoteReporter::new(to_main.clone());
        let mut confirmer = crate::remote_confirm::RemoteConfirmer::new(to_main, answer_rx);
        let egress = Egress::new();
        // Owned by the worker for the duration and handed back afterwards, whether the turn
        // succeeded or not. A failed turn is still part of the conversation, and the next one
        // is usually about it.
        let mut conversation = conversation;
        let outcome = turn::resume(
            &worker_config,
            &egress,
            &worker_workspace,
            &task,
            &mut conversation,
            &mut confirmer,
            &mut reporter,
            &mut sink,
            trust,
            &worker_cancel,
        );
        (outcome, conversation, sink)
    });

    // Redraw until the turn finishes, answering approvals and watching for a cancel on the way.
    loop {
        terminal
            .draw(|frame| render::draw(frame, session))
            .map_err(io::Error::other)?;

        // Input is polled here rather than in the outer loop, which is blocked for the duration
        // of the turn. Without this the interface would take none at all while working, and a
        // long turn is exactly when someone wants to copy what has appeared so far.
        // Everything waiting, not one event per pass: this loop wakes at the frame rate, so a
        // drag read one event at a time would take seconds to catch up with the pointer.
        while event::poll(Duration::ZERO)? {
            match event::read()? {
                TermEvent::Key(key) if wants_cancel(key) => {
                    cancel.cancel();
                    session.note("cancelling…");
                }
                TermEvent::Mouse(mouse) => {
                    // Bound rather than tested inline, because handling the event scrolls and
                    // moves the selection whatever it returns. A match guard would hide that.
                    let action = handle_mouse(session, mouse);
                    if action == Action::Copy {
                        copy_selection(terminal, session)?;
                    }
                }
                _ => {}
            }
        }

        match from_worker.recv_timeout(FRAME) {
            Ok(crate::remote_confirm::ToMain::Write(request)) => {
                let answer = crate::confirm::ask(terminal, &request);
                // Ctrl-C at the prompt is the same request it is anywhere else in a turn: stop.
                // Set before the answer goes back, so the worker sees it as soon as it wakes.
                if answer == crate::confirm::Answer::Interrupt {
                    cancel.cancel();
                    session.note("cancelling…");
                }
                // A closed channel means the worker is already gone, so there is nothing to
                // answer and the loop below will collect its result.
                let _ = answer_tx.send(answer.decision());
            }
            // No reply: each of these is recorded and the next redraw, one iteration away,
            // shows it. That is what makes a long turn legible while it runs.
            Ok(crate::remote_confirm::ToMain::Todos(rows)) => session.set_todos(rows),
            Ok(crate::remote_confirm::ToMain::Written(written)) => session.set_written(written),
            Ok(crate::remote_confirm::ToMain::Phase(phase)) => session.set_phase(phase),
            Ok(crate::remote_confirm::ToMain::Narration(text)) => session.narrate(text),
            Ok(crate::remote_confirm::ToMain::Started(activity)) => {
                session.start_activity(activity)
            }
            Ok(crate::remote_confirm::ToMain::Finished(activity)) => {
                session.finish_activity(activity)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // The worker dropped its senders, so the turn is over.
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let (outcome, conversation, sink) = worker.join().unwrap_or_else(|_| {
        // A panicked turn is reported rather than propagated: the session survives. The
        // conversation does not, since the thread that held it is gone.
        (
            Err(turn::TurnError::Precommit(
                "the turn ended unexpectedly".to_string(),
            )),
            Conversation::new(),
            RecordingSink::new(),
        )
    });

    // A cancelled turn returns the prompt for editing instead of recording a failure: the user
    // stopped it deliberately, so there is nothing to report.
    let events = sink.events().to_vec();

    if matches!(outcome, Err(turn::TurnError::Cancelled)) {
        session.restore(prompt);
        return Ok((conversation, fallback, events));
    }

    Ok((
        conversation,
        fold_outcome(session, outcome, sink, fallback),
        events,
    ))
}

/// Whether a key press asks for the turn in flight to stop.
///
/// Escape is the obvious key, and Ctrl-C is included because a user reaching for the usual
/// interrupt should not have to discover a different one.
fn wants_cancel(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Esc)
        || (key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')))
}

/// Fold a finished turn into the session.
fn fold_outcome(
    session: &mut Session,
    outcome: Result<turn::Outcome, turn::TurnError>,
    sink: RecordingSink,
    fallback: TrustStore,
) -> TrustStore {
    match outcome {
        Ok(outcome) => {
            let trail = sink.events().to_vec();
            session.complete(
                outcome.reply_for_display().to_string(),
                trail,
                outcome.tokens,
            );
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
            let trail = sink.events().iter().map(crate::audit::as_line).collect();
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

    fn drag(kind: MouseEventKind, row: u16, column: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// Capturing the mouse for the wheel took the terminal's own selection away, so a drag has
    /// to select here or a user cannot copy a line of their own transcript at all.
    #[test]
    fn dragging_selects_and_releasing_asks_for_the_copy() {
        let mut session = Session::new("none");

        assert_eq!(
            handle_mouse(
                &mut session,
                drag(MouseEventKind::Down(MouseButton::Left), 3, 4)
            ),
            Action::Redraw
        );
        assert_eq!(
            handle_mouse(
                &mut session,
                drag(MouseEventKind::Drag(MouseButton::Left), 3, 20)
            ),
            Action::Redraw
        );
        assert_eq!(
            handle_mouse(
                &mut session,
                drag(MouseEventKind::Up(MouseButton::Left), 3, 20)
            ),
            Action::Copy
        );

        let selection = session.selection.expect("nothing was selected");
        assert!(!selection.is_empty());
        assert!(
            selection.covers(3, 10),
            "the sweep did not cover its middle"
        );
    }

    /// The sequence has to leave a terminal that keeps one tracking state in the mode that
    /// reports drags, or the wheel stops reaching the session and scrolls the window behind it
    /// instead. What is asked for last is what such a terminal ends up in.
    #[test]
    fn the_mouse_request_ends_in_the_mode_that_reports_a_drag() {
        assert!(
            TRACK_MOTION_ONLY_WHILE_DRAGGING.ends_with("\x1b[?1002h"),
            "a terminal with one tracking state would be left without drags"
        );
        assert!(
            TRACK_MOTION_ONLY_WHILE_DRAGGING.starts_with("\x1b[?1003l"),
            "all-motion reporting was never turned off"
        );
        assert!(
            TRACK_MOTION_ONLY_WHILE_DRAGGING.contains("\x1b[?1000h"),
            "button reporting, which carries the wheel, was not asked for again"
        );
    }

    /// A pointer crossing the window is not input. Terminals report that motion by default, and
    /// answering each report with a redraw is what made a drag crawl: the queue outran the
    /// drawing and the highlight trailed behind the pointer.
    #[test]
    fn moving_the_pointer_without_a_button_asks_for_nothing() {
        let mut session = Session::new("none");
        session.begin_selection(2, 2);
        session.extend_selection(2, 8);

        let action = handle_mouse(&mut session, drag(MouseEventKind::Moved, 9, 40));

        assert_eq!(action, Action::None, "a bare move asked for work");
        let selection = session.selection.expect("the selection was disturbed");
        assert!(
            selection.covers(2, 4),
            "the selection moved with the pointer"
        );
    }

    /// Selecting has nothing to do with the turn: what is on the screen is already there, and a
    /// long turn is exactly when someone wants to copy part of it.
    #[test]
    fn selecting_works_while_a_turn_is_running() {
        let mut session = Session::new("none");
        session.type_char('x');
        session.submit();
        assert_eq!(session.status, Status::Working);

        handle_mouse(
            &mut session,
            drag(MouseEventKind::Down(MouseButton::Left), 1, 0),
        );
        handle_mouse(
            &mut session,
            drag(MouseEventKind::Drag(MouseButton::Left), 1, 5),
        );
        assert!(session.selection.is_some());
    }

    /// The bug this exists for: a prompt copied from somewhere else usually ends in a newline,
    /// and a paste delivered as keystrokes turns that into Enter. It used to send itself before
    /// its author had read it back.
    #[test]
    fn a_paste_that_ends_in_a_newline_does_not_send_it() {
        let mut session = Session::new("none");
        let action = handle_paste(&mut session, "write me a game\n");

        assert_eq!(action, Action::Redraw);
        assert_eq!(session.input, "write me a game\n");
        assert_eq!(session.status, Status::Idle, "the paste started a turn");
        assert!(session.transcript.is_empty(), "the paste sent something");
    }

    /// And it is still one prompt afterwards: Enter sends what was pasted, once.
    #[test]
    fn a_pasted_prompt_is_sent_when_the_user_says_so() {
        let mut session = Session::new("none");
        handle_paste(&mut session, "write me a game\n");

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("write me a game".to_string())
        );
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
    fn escape_quits_on_an_empty_line() {
        let mut session = Session::new("none");
        assert_eq!(handle_key(&mut session, key(KeyCode::Esc)), Action::Quit);
        assert!(session.is_quitting());
    }

    /// Escape is how a half-typed prompt is abandoned, so it must not also end the session
    /// while there is something to discard.
    #[test]
    fn escape_clears_a_typed_line_without_quitting() {
        let mut session = Session::new("none");
        for c in "half a thought".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(handle_key(&mut session, key(KeyCode::Esc)), Action::Redraw);
        assert!(session.input.is_empty(), "the input was not cleared");
        assert!(
            !session.is_quitting(),
            "clearing the input ended the session"
        );
    }

    /// Escape means "stop this" before it means anything else, so a turn in flight is cancelled
    /// rather than the input being cleared.
    #[test]
    fn escape_cancels_a_turn_in_flight() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('x')));
        handle_key(&mut session, key(KeyCode::Enter));
        assert_eq!(session.status, Status::Working);

        assert_eq!(handle_key(&mut session, key(KeyCode::Esc)), Action::Cancel);
        assert!(!session.is_quitting(), "cancelling ended the session");
    }

    /// The keys that ask a running turn to stop. Ctrl-C is included because it is what a user
    /// reaches for out of habit.
    #[test]
    fn cancel_keys_are_escape_and_ctrl_c() {
        assert!(wants_cancel(key(KeyCode::Esc)));
        assert!(wants_cancel(ctrl('c')));
        assert!(!wants_cancel(key(KeyCode::Char('c'))));
        assert!(!wants_cancel(key(KeyCode::Enter)));
        assert!(!wants_cancel(key(KeyCode::Up)));
    }

    /// And a second press then leaves, so the key still reaches the exit without a detour.
    #[test]
    fn escape_twice_clears_then_quits() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('x')));

        assert_eq!(handle_key(&mut session, key(KeyCode::Esc)), Action::Redraw);
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

    /// With nothing sent yet, the arrows scroll: there is no history to walk.
    #[test]
    fn arrow_keys_scroll_when_there_is_no_history() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Up));
        assert_eq!(session.scroll, 1);
        handle_key(&mut session, key(KeyCode::Up));
        assert_eq!(session.scroll, 2);
        handle_key(&mut session, key(KeyCode::Down));
        assert_eq!(session.scroll, 1);
    }

    /// Once something has been sent, Up recalls it, which is what a shell does and so what a
    /// user expects at a prompt.
    #[test]
    fn up_recalls_a_previous_prompt() {
        let mut session = Session::new("none");
        for c in "first question".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        handle_key(&mut session, key(KeyCode::Enter));
        session.complete("an answer", Vec::new(), 0);

        assert_eq!(handle_key(&mut session, key(KeyCode::Up)), Action::Redraw);
        assert_eq!(session.input, "first question");
        assert_eq!(session.scroll, 0, "recall scrolled the transcript as well");
    }

    /// Down walks back out of history, restoring the line that was being typed.
    #[test]
    fn down_returns_from_history_to_the_typed_line() {
        let mut session = Session::new("none");
        for c in "sent".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        handle_key(&mut session, key(KeyCode::Enter));
        session.complete("ok", Vec::new(), 0);

        for c in "being typed".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        handle_key(&mut session, key(KeyCode::Up));
        assert_eq!(session.input, "sent");

        handle_key(&mut session, key(KeyCode::Down));
        assert_eq!(
            session.input, "being typed",
            "the half-typed line was not restored"
        );
    }

    /// Typing over a recalled prompt makes it the working line, so the position label goes away.
    #[test]
    fn typing_leaves_history_browsing() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('a')));
        handle_key(&mut session, key(KeyCode::Enter));
        session.complete("ok", Vec::new(), 0);

        handle_key(&mut session, key(KeyCode::Up));
        assert!(session.history.is_browsing());
        handle_key(&mut session, key(KeyCode::Char('b')));
        assert!(!session.history.is_browsing());
    }

    /// Escape leaves history along with clearing, so it cannot be left labelled with an empty box.
    #[test]
    fn escape_leaves_history_browsing() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('a')));
        handle_key(&mut session, key(KeyCode::Enter));
        session.complete("ok", Vec::new(), 0);

        handle_key(&mut session, key(KeyCode::Up));
        assert!(session.history.is_browsing());
        handle_key(&mut session, key(KeyCode::Esc));
        assert!(!session.history.is_browsing());
        assert!(session.input.is_empty());
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
