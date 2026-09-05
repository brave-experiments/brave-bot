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

use bravebot_agent::Workspace;
use bravebot_agent::conversation::Conversation;
use bravebot_agent::turn::{self, PastedImage, Task};
use bravebot_config::Config;
use bravebot_core::cancel::Cancel;
use bravebot_core::permissions::Permissions;
use bravebot_core::programs::TrustedPrograms;
use bravebot_core::trust::TrustStore;
use bravebot_i18n::t;
use bravebot_net::Egress;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, KeyboardEnhancementFlags, MouseButton, MouseEvent, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use std::io::{self, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::audit::{Stamped, Trail};
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

/// The line that opens the model picker instead of starting a turn.
const MODEL_COMMAND: &str = "/model";

/// The line that opens the theme picker, or applies a theme named after the word.
const THEME_COMMAND: &str = "/theme";

/// The line that opens another directory, taking the path to open as its argument.
const ADD_DIR_COMMAND: &str = "/add-dir";

/// The line that reports what this session is and what it may touch.
const STATUS_COMMAND: &str = "/status";

/// The line that summarises the conversation so far, in place of sending all of it.
const COMPACT_COMMAND: &str = "/compact";

/// The line that starts a new session in place of this one.
const CLEAR_COMMAND: &str = "/clear";

/// The line that renames this session, taking the new name as its argument.
const RENAME_COMMAND: &str = "/rename";

/// The line that repeats a prompt, taking the prompt and any interval as its argument.
const LOOP_COMMAND: &str = "/loop";

/// The one line that ends the session instead of starting a turn.
const EXIT_COMMAND: &str = "/exit";

/// One command, and what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// The word typed, including the leading slash.
    pub name: &'static str,
    /// What it takes after the word, or empty where it takes nothing.
    pub argument: &'static str,
    /// One line, for the list shown while a command is being typed.
    pub description: &'static str,
}

/// Every command, in the order they are offered.
///
/// The one place they are written down. The hint line, the completion list and the key handler all
/// read from here, so a command that is renamed or added cannot leave any of them advertising
/// something that no longer works.
pub fn commands() -> [Command; 9] {
    [
        Command {
            name: STATUS_COMMAND,
            argument: "",
            description: t!(command_status),
        },
        Command {
            name: MODEL_COMMAND,
            argument: "",
            description: t!(command_model),
        },
        Command {
            name: THEME_COMMAND,
            argument: "[name]",
            description: t!(command_theme),
        },
        Command {
            name: ADD_DIR_COMMAND,
            argument: "<path>",
            description: t!(command_add_dir),
        },
        Command {
            name: RENAME_COMMAND,
            argument: "<name>",
            description: t!(command_rename),
        },
        Command {
            name: COMPACT_COMMAND,
            argument: "",
            description: t!(command_compact),
        },
        Command {
            name: CLEAR_COMMAND,
            argument: "",
            description: t!(command_clear),
        },
        Command {
            name: LOOP_COMMAND,
            argument: "[interval] <prompt>",
            description: t!(command_loop),
        },
        Command {
            name: EXIT_COMMAND,
            argument: "",
            description: t!(command_exit),
        },
    ]
}

/// The commands a half-typed line could still become, in the order they are offered.
///
/// Empty unless the line is a lone word starting with a slash: a command takes its argument after a
/// space, so once there is one the command is settled and there is nothing left to complete. A
/// line that is not a command at all completes to nothing, which is what closes the list.
pub fn completions(line: &str) -> Vec<Command> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('/') || trimmed.contains(char::is_whitespace) {
        return Vec::new();
    }
    commands()
        .iter()
        .filter(|command| command.name.starts_with(trimmed))
        .copied()
        .collect()
}

/// The argument given to `command`, if that is what the line is.
///
/// `None` for anything else, so a prompt that merely mentions the word is still a prompt, and so is
/// a longer word that happens to start with it. The bare command with no argument is `Some("")`,
/// which the caller answers by saying what it needs rather than silently doing nothing.
fn argument_to<'a>(line: &'a str, command: &str) -> Option<&'a str> {
    let rest = line.trim().strip_prefix(command)?;
    if rest.is_empty() {
        return Some("");
    }
    // A following character that is not a space means a longer word, not this command.
    rest.strip_prefix(' ').map(str::trim)
}

/// What a key press asked for.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Redraw,
    Submit(String),
    /// Bring what is on the clipboard into the prompt. Runs the platform's clipboard tools, so
    /// the loop does it rather than the key handler.
    Paste,
    /// Stop the turn in flight.
    Cancel,
    /// Take what the selection covers, which needs the screen as it was last drawn.
    Copy,
    /// Write the prompt somewhere with room to think. Needs the terminal, which the loop hands
    /// over to the editor and takes back afterwards.
    Edit,
    /// Ask which model to use. Needs the network and the terminal, so the loop runs it.
    ChooseModel,
    /// Ask which theme to paint in. Needs the terminal, so the loop runs it.
    ChooseTheme,
    /// Apply a theme by name without opening the picker.
    SetTheme(String),
    /// Open another directory. Needs the workspace and the trust map, which the loop owns.
    AddDirectory(String),
    /// Summarise the conversation so far. Needs the conversation and the network, which the loop
    /// owns.
    Compact,
    /// Start a new session here. Needs the conversation and the session record, which the loop owns.
    Clear,
    /// Call this session something else. Needs the session record, which the loop owns.
    Rename(String),
    /// Report what this session is. Needs the workspace and the trust map, which the loop owns.
    Status,
    /// Run a command the user typed in shell mode. Needs the workspace and the conversation.
    Run(String),
    /// Put the transcript in front of the user in their editor. Needs the terminal, which the
    /// loop owns, and gives the session nothing back.
    Show,
    Quit,
}

/// Move the caret or delete around it, and say whether the key was one that does.
///
/// Shared by the idle and mid-turn handlers, because what has been typed can be edited in both:
/// the box holds the same line either way, and only sending it is refused while a turn runs.
///
/// Both Ctrl and Alt are read as the word modifier, since terminals disagree about which they send
/// for Ctrl-Left.
fn edit_line(session: &mut Session, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let word = ctrl || alt;

    match key.code {
        KeyCode::Left if word => session.move_word_left(),
        KeyCode::Right if word => session.move_word_right(),
        KeyCode::Left => session.move_left(),
        KeyCode::Right => session.move_right(),
        // Bare, because the ends of the line being typed are what these mean in every other text
        // field. The transcript keeps them under Ctrl.
        KeyCode::Home if !ctrl => session.move_to_line_start(),
        KeyCode::End if !ctrl => session.move_to_line_end(),
        KeyCode::Delete => session.delete_forward(),
        // With a modifier only: a bare Backspace is also how shell mode is left, so it stays
        // where that is decided.
        KeyCode::Backspace if word => session.delete_word_before(),
        // The readline bindings as well as the named keys, because a terminal or an ssh session
        // with a keymap of its own may deliver none of the above, and then the middle of a line
        // could not be reached at all.
        KeyCode::Char('a') if ctrl => session.move_to_line_start(),
        KeyCode::Char('e') if ctrl => session.move_to_line_end(),
        KeyCode::Char('b') if word => session.move_word_left(),
        KeyCode::Char('f') if word => session.move_word_right(),
        KeyCode::Char('w') if ctrl => session.delete_word_before(),
        KeyCode::Char('u') if ctrl => session.delete_to_line_start(),
        KeyCode::Char('k') if ctrl => session.delete_to_line_end(),
        _ => return false,
    }
    true
}

/// Whether `key` asks for a new line in the prompt rather than for the prompt to be sent.
///
/// Two spellings, because a terminal has two ways of saying it and which one arrives is not
/// something the user chose:
///
/// - Shift-Enter, which needs the terminal to report the modifier. Most do not: the byte for Enter
///   is the same however it was pressed, which is why [`take_over_terminal`] asks for disambiguated
///   keys where that is understood.
/// - Ctrl-J, which is the byte `\n`. That is what a terminal configured to send a newline for
///   Shift-Enter delivers, and it is the arrangement iTerm and Terminal.app need, since neither
///   reports the modifier. It is also typeable directly on a terminal that does neither.
///
/// Ctrl-J rather than Ctrl-M: `\r` is Enter itself and binding it would break sending. Nothing else
/// may claim Ctrl-J, whatever readline does with it, since on those terminals it is the only way to
/// write a paragraph.
fn starts_a_line(key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Enter => key.modifiers.contains(KeyModifiers::SHIFT),
        KeyCode::Char('j') => key.modifiers.contains(KeyModifiers::CONTROL),
        _ => false,
    }
}

/// Whether a key press reaches the turn in flight.
///
/// The scroller answers these keys before the turn does, because it is the nearest thing there is
/// to stop. Somebody who opened it to read what the turn had already done is not asking for the
/// turn to end when they close it again, and the press that reaches the turn is the next one.
///
/// Named rather than written out at each loop, because there are three of them and a condition
/// copied three times is a condition that ends up meaning three things.
fn stops_the_turn(session: &Session, key: KeyEvent) -> bool {
    !session.scrolling() && (is_ctrl_c(key) || wants_cancel(key))
}

/// Interpret a key press while the scroller is open.
///
/// Every key the scroller answers is answered here, and a key it does not name does nothing at
/// all: nothing falls through to the box. A mode that leaks its keystrokes into a box the person
/// cannot see is the worse half of both, since `j` would scroll and also type a `j`, and the
/// prompt they had half written would quietly be a different prompt by the time they came back
/// to it. The line is untouched throughout and comes back exactly as it was.
///
/// Nothing here sends anything, so there is nothing in it for a running turn to refuse, and the
/// same list answers whether or not one is in flight.
fn scroller_key(session: &mut Session, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // The key list is read instead of the transcript rather than alongside it, so anything at all
    // puts it away, and that press is spent doing so. The list says as much, because a key that
    // silently did two things would be worse than one that does the obvious one.
    if session.scroller().is_some_and(|scroller| scroller.help) {
        session.toggle_scroller_help();
        return Action::Redraw;
    }

    // A search being typed takes the letters back, because typing is what they mean. Only the
    // three keys that finish one are read as anything else.
    if session.typing_a_search() {
        return match key.code {
            KeyCode::Enter => {
                session.run_search();
                land_on_a_match(session);
                Action::Redraw
            }
            KeyCode::Esc => {
                session.abandon_search();
                Action::Redraw
            }
            // Backspacing past the start abandons the search, which is what the key means once
            // there is nothing left of the thing it deletes.
            KeyCode::Backspace => {
                if !session.backspace_search() {
                    session.abandon_search();
                }
                Action::Redraw
            }
            KeyCode::Char(c) if !ctrl => {
                session.type_into_search(c);
                Action::Redraw
            }
            _ => Action::None,
        };
    }

    match key.code {
        // Four keys close it. Ctrl-C is one of them and does nothing else here: the scroller is
        // the nearest thing there is to stop, so a turn in flight goes on running and the press
        // that reaches it is the next one.
        KeyCode::Char('q') if !ctrl => {
            session.close_scroller();
            Action::Redraw
        }
        // The same ladder every other stop key here walks: the nearest thing there is to stop.
        // A standing search is nearer than the mode holding it, so the highlights come off first
        // and the press after that is the one that closes the scroller.
        KeyCode::Esc => {
            if !session.clear_search() {
                session.close_scroller();
            }
            Action::Redraw
        }
        KeyCode::Char('o') | KeyCode::Char('c') if ctrl => {
            session.close_scroller();
            Action::Redraw
        }

        // A line at a time.
        KeyCode::Up | KeyCode::Char('k') => {
            session.scroller_back(1);
            Action::Redraw
        }
        KeyCode::Down | KeyCode::Char('j') => {
            session.scroller_on(1);
            Action::Redraw
        }

        // Half a screen, which is the one movement that keeps context on both sides of itself.
        KeyCode::Char('u') if ctrl => {
            session.scroller_back(session.half_screen());
            Action::Redraw
        }
        KeyCode::Char('d') if ctrl => {
            session.scroller_on(session.half_screen());
            Action::Redraw
        }

        // A whole screen, in both dialects. `b` is the same key with or without Ctrl, because
        // somebody who knows one spelling should not find the other typing a letter.
        KeyCode::Char(' ') | KeyCode::PageDown => {
            session.scroller_on(session.whole_screen());
            Action::Redraw
        }
        KeyCode::Char('f') if ctrl => {
            session.scroller_on(session.whole_screen());
            Action::Redraw
        }
        KeyCode::Char('b') | KeyCode::PageUp => {
            session.scroller_back(session.whole_screen());
            Action::Redraw
        }

        // The ends.
        KeyCode::Char('g') if !ctrl => {
            session.scroller_to_first_row();
            Action::Redraw
        }
        KeyCode::Char('G') => {
            session.scroller_to_last_row();
            Action::Redraw
        }
        KeyCode::Home => {
            session.scroller_to_first_row();
            Action::Redraw
        }
        KeyCode::End => {
            session.scroller_to_last_row();
            Action::Redraw
        }

        // Turn by turn. Where these land is settled by what the person typed, since a prompt is
        // the one thing in a transcript they wrote themselves.
        KeyCode::Char('{') => {
            session.to_previous_prompt();
            Action::Redraw
        }
        KeyCode::Char('}') => {
            session.to_next_prompt();
            Action::Redraw
        }

        KeyCode::Char('/') => {
            session.begin_search();
            Action::Redraw
        }
        KeyCode::Char('n') => {
            walk_the_matches(session, true);
            Action::Redraw
        }
        KeyCode::Char('N') => {
            walk_the_matches(session, false);
            Action::Redraw
        }

        KeyCode::Char('?') => {
            session.toggle_scroller_help();
            Action::Redraw
        }
        KeyCode::Char('v') if !ctrl && session.status != Status::Working => Action::Show,

        _ => Action::None,
    }
}

/// Move to the first match at or after the top of the view.
///
/// The transcript is laid out again here, because the needle was set by the key press being
/// answered: the last frame was drawn looking for something else, and what it found is no answer
/// to the question just asked.
fn land_on_a_match(session: &mut Session) {
    let laid = render::as_last_drawn(session);
    session.land_on_a_match(&laid.matches);
}

/// Walk to the next match, or the previous one.
///
/// The rows are the ones the last frame found, which is the frame the person is looking at while
/// they press the key. Nothing has changed about what is being looked for since it was drawn, so
/// there is nothing to lay out again.
fn walk_the_matches(session: &mut Session, forwards: bool) {
    let found = session.laid.matches.clone();
    session.to_a_match(&found, forwards);
}

/// Interpret a key press against the session.
///
/// Separated from the loop so it can be tested without a terminal.
pub fn handle_key(session: &mut Session, key: KeyEvent) -> Action {
    // Before everything, including the keys that edit the line: while the scroller is open there
    // is no line being edited, and the keys belong to it.
    if session.scrolling() {
        return scroller_key(session, key);
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // The hint offering the way out lives for one press, and this is it. Cleared before the arms
    // rather than after, so the Ctrl-C that puts it up survives its own press.
    session.cleared_by_interrupt = false;

    // Before the match, since a key that moves the caret cannot also be one of the keys below:
    // the ones this answers are exactly the ones nothing else claims.
    if edit_line(session, key) {
        return Action::Redraw;
    }

    match key.code {
        // Ctrl-C is read against what there is to stop, nearest first: the turn in flight, then
        // the line in the box, then the loop that keeps sending prompts, then the session. A
        // person who wants the answer to stop gets that from the first press, and nothing they
        // were part way through writing is taken with it.
        KeyCode::Char('c') if ctrl && session.status == Status::Working => Action::Cancel,
        KeyCode::Char('c') if ctrl && !session.input().is_empty() => {
            session.clear_input();
            // Said only here. A press that leaves is not one to explain, and the hint is the
            // answer to what a person has just done rather than standing advice.
            session.cleared_by_interrupt = true;
            Action::Redraw
        }
        // Before leaving, because a loop is a thing still happening and leaving is what a person
        // reaches for when nothing else will stop it. Reversed, the key that ends a loop would be
        // the key that ends the session, and there would be no way to keep one without the other.
        KeyCode::Char('c') if ctrl && session.looping().is_some() => {
            session.stop_loop();
            Action::Redraw
        }
        KeyCode::Char('c') if ctrl => {
            session.quit();
            Action::Quit
        }
        KeyCode::Char('d') if ctrl && session.input().is_empty() => {
            session.quit();
            Action::Quit
        }
        // Reading back through what happened, rather than typing at it. The transcript already
        // scrolls; what needs a mode is everything a person does once they are reading, since the
        // keys for it are letters and the box takes letters.
        KeyCode::Char('o') if ctrl => {
            session.open_scroller();
            Action::Redraw
        }
        // The paste that can carry a picture. Command-V is the terminal's own, goes through the
        // pty as text, and therefore drops everything that is not text; this one reads the
        // clipboard directly. Readline's quoted-insert is what the chord costs, and nobody has
        // ever wanted it here.
        KeyCode::Char('v') if ctrl => Action::Paste,
        // The box can be moved around in now, but it is still capped at ten rows and has none of
        // what someone reaches for on a long prompt. A paragraph worth thinking about goes
        // somewhere with room instead.
        KeyCode::Char('g') if ctrl => Action::Edit,
        // Escape means "stop what is happening" before it means anything else, so a turn in
        // flight is cancelled first. The prompt comes back for editing rather than being lost.
        KeyCode::Esc if session.status == Status::Working => Action::Cancel,
        // Then it discards a half-typed prompt, and an armed shell mode is something to abandon
        // even with no line behind it: the marker is on screen, and Backspace at that same caret
        // already backs out of it.
        //
        // On an empty line it does nothing at all. It used to leave, which made every press a
        // question of what was in the box: the key for abandoning a thought was the key for
        // ending the session as soon as the thought was short enough. Ctrl-C is the way out.
        KeyCode::Esc => {
            session.clear_input();
            Action::Redraw
        }
        // Before every arm that sends, because this is the one Enter that does not: a paragraph is
        // written in the box rather than only pasted into it, and the box grows to hold it. Shell
        // mode too, where a multi-line command is a `for` loop somebody typed.
        _ if starts_a_line(key) => {
            session.type_newline();
            Action::Redraw
        }
        // Before every command arm, because in shell mode the line is a command and nothing else.
        // `/status` is a path to a program somebody might have, and `!` is how they said so.
        KeyCode::Enter if session.shell => match session.submit_command() {
            Some(line) => Action::Run(line),
            None => Action::None,
        },
        // Typed before submitting, so the word never reaches the planner as a prompt.
        KeyCode::Enter if session.input().trim() == EXIT_COMMAND => {
            session.clear_input();
            session.quit();
            Action::Quit
        }
        KeyCode::Enter if session.input().trim() == MODEL_COMMAND => {
            session.clear_input();
            Action::ChooseModel
        }
        KeyCode::Enter if argument_to(session.input(), THEME_COMMAND).is_some() => {
            let name = argument_to(session.input(), THEME_COMMAND)
                .expect("the guard just matched")
                .to_string();
            session.clear_input();
            if name.is_empty() {
                Action::ChooseTheme
            } else {
                Action::SetTheme(name)
            }
        }
        KeyCode::Enter if session.input().trim() == STATUS_COMMAND => {
            session.clear_input();
            Action::Status
        }
        KeyCode::Enter if session.input().trim() == COMPACT_COMMAND => {
            session.clear_input();
            Action::Compact
        }
        KeyCode::Enter if session.input().trim() == CLEAR_COMMAND => {
            session.clear_input();
            Action::Clear
        }
        KeyCode::Enter if argument_to(session.input(), ADD_DIR_COMMAND).is_some() => {
            let directory = argument_to(session.input(), ADD_DIR_COMMAND)
                .expect("the guard just matched")
                .to_string();
            session.clear_input();
            Action::AddDirectory(directory)
        }
        KeyCode::Enter if argument_to(session.input(), RENAME_COMMAND).is_some() => {
            let name = argument_to(session.input(), RENAME_COMMAND)
                .expect("the guard just matched")
                .to_string();
            session.clear_input();
            Action::Rename(name)
        }
        // The command that sends a prompt rather than the line it was typed on. `/loop 5m check
        // the deploy` arms the loop and hands back "check the deploy", which is what every tick
        // sends from here on.
        KeyCode::Enter if argument_to(session.input(), LOOP_COMMAND).is_some() => {
            let argument = argument_to(session.input(), LOOP_COMMAND)
                .expect("the guard just matched")
                .to_string();
            session.clear_input();
            match crate::loops::parse(&argument) {
                Some(request) => match session.start_loop(request) {
                    Some(prompt) => Action::Submit(prompt),
                    None => Action::Redraw,
                },
                None => {
                    session.note(t!(loop_needs_a_prompt));
                    Action::Redraw
                }
            }
        }
        // A half-typed command, after every arm that recognises a whole one. Enter takes the
        // highlighted row rather than sending "/mod" to the planner, which is never what was meant.
        // A half-typed command, after every arm that recognises a whole one. Enter takes the
        // highlighted row rather than sending "/mod" to the planner, which is never what was meant.
        //
        // A reference only completes while it is still unfinished. A prompt ending in one that
        // already names a file is a finished sentence, and Enter has to send it: completing there
        // would leave a user pressing Enter twice to say something perfectly well formed.
        KeyCode::Enter if session.completion_would_change_the_line() => {
            session.accept_completion();
            Action::Redraw
        }
        KeyCode::Enter => match session.submit() {
            Some(prompt) => Action::Submit(prompt),
            None => Action::None,
        },
        _ => navigate(session, key),
    }
}

/// The keys that mean the same thing whether or not a turn is running.
///
/// Everything a person does to the line they are composing and to their view of what has already
/// happened: editing it, completing it, walking back through what they have said before, and
/// scrolling. None of it sends anything, and sending is the whole of what a running turn refuses,
/// so none of it has any reason to ask whether one is running.
///
/// One ladder rather than one per caller, because there were two and they drifted. Mid-turn, Up
/// and Down reached no arm at all and fell through to nothing, so a person who could see their
/// last prompt in the transcript could not recall it into the box, and the keys that scroll did
/// not scroll either. `handle_paste_while_working` was already named for this reason; this is the
/// same lesson in the same file.
fn navigate(session: &mut Session, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Backspace => {
            session.backspace();
            Action::Redraw
        }
        // A thought to come back to, set aside without being sent and without being lost. One key
        // both ways, because what it does is read off the line: there is nothing for a person to
        // remember about which press was which.
        //
        // In the shared ladder, so it works while a turn runs like everything else that writes the
        // box: it sends nothing, and sending is the whole of what a running turn refuses. Putting a
        // half-written thought away is most wanted exactly when a turn is in flight and a better
        // one has just occurred to somebody.
        //
        // It reaches this process at all because raw mode turns off the terminal's flow control,
        // where this chord is the byte that freezes the screen.
        KeyCode::Char('s') if ctrl => {
            session.stash();
            Action::Redraw
        }
        // What a turn did, drawn per entry as the turn fills in. In the shared ladder because it
        // sets a render flag and sends nothing, and because the trail is *for* watching a turn:
        // refused mid-turn, a person who wanted to see which tools a turn was calling had to wait
        // for it to finish before they were allowed to ask.
        KeyCode::Char('t') if ctrl => {
            session.toggle_trail();
            Action::Redraw
        }
        // Tab completes, which is what it does everywhere else. Only while a command is being
        // typed: with nothing offered it inserts nothing, rather than a stray character.
        KeyCode::Tab if session.is_completing() => {
            session.accept_completion();
            Action::Redraw
        }
        // While the list is open the arrows walk it. History and scrolling get them back the moment
        // it closes, which is as soon as the line stops being a lone half-typed command.
        KeyCode::Up if session.is_completing() => {
            session.previous_completion();
            Action::Redraw
        }
        KeyCode::Down if session.is_completing() => {
            session.next_completion();
            Action::Redraw
        }
        // A pasted paragraph has rows of its own, and moving between them is what these keys mean
        // inside it. Only while there is a row to move to: at the top of the line they go back to
        // the history below, the way they do in a shell.
        KeyCode::Up if session.is_multiline() && session.move_up_a_line() => Action::Redraw,
        KeyCode::Down if session.is_multiline() && session.move_down_a_line() => Action::Redraw,
        // What is waiting comes back before anything older does. The queue holds the most recent
        // thing the person said, and the history holds a copy of every line in it, so Up handed
        // back a copy of a prompt that was still going to be sent: the person edited the copy,
        // and the original went anyway. Taking the queue back is about sending, which is the one
        // thing a running turn is allowed to differ about.
        KeyCode::Up if !session.queued.is_empty() => {
            session.unqueue();
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
        // The page keys walk the prompt first: the start of this line, then the line before, and
        // the same downwards. Only while there is somewhere to go, so at the ends of the prompt they
        // fall through to the transcript below, which is what they did before there was a caret.
        KeyCode::PageUp if session.page_up() => Action::Redraw,
        KeyCode::PageDown if session.page_down() => Action::Redraw,
        KeyCode::PageUp => {
            session.scroll_up(10);
            Action::Redraw
        }
        KeyCode::PageDown => {
            session.scroll_down(10);
            Action::Redraw
        }
        // Jump to either end of the transcript. Under Ctrl, because bare Home and End belong to
        // the line being typed: they were the transcript's before there was a caret to move.
        KeyCode::Home if ctrl => {
            session.scroll_up(u16::MAX);
            Action::Redraw
        }
        KeyCode::End if ctrl => {
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

/// Take a paste while a turn is running.
///
/// The same folding as the idle path, for the same reason: a stack trace pasted mid-turn would
/// otherwise push the reply being read off the screen, and mid-turn is exactly when there is a
/// reply worth reading. Named rather than bound inline at each working loop, because there are two
/// of them and writing the same arm twice is how they came to disagree.
///
/// A drop is recognised here too, since a drop reaches the terminal as a paste of the path and a
/// running turn is when people drop a file: they are watching a reply and want to hand over the
/// screenshot it is about. Without this the path was written out as prose, so the line said
/// nothing about a file and the attachment was never staged.
///
/// Nothing comes back, since both loops redraw every frame regardless of what the event was.
pub fn handle_paste_while_working(session: &mut Session, text: &str) {
    if !session.drop_files(text) {
        session.paste_text(text);
    }
}

/// Interpret a key press while a turn is running.
///
/// Only the ones that cannot start anything. What the user types goes into the box and stays
/// there, and the keys that look back through the transcript still work. Enter is not among them:
/// a second turn must not begin while the first is in flight, so the line waits, and sending it
/// is the first thing available when the turn ends.
///
/// Everything used to be dropped here, cancel and mouse aside. A user typing during a slow turn
/// therefore watched their words go nowhere, with nothing on the screen to say why, which is
/// indistinguishable from an interface that has stopped responding.
pub fn handle_key_while_working(session: &mut Session, key: KeyEvent) -> Action {
    // The same list as at rest, for the reason the ladder below is the same list: nothing the
    // scroller does sends anything, and sending is the whole of what a running turn refuses.
    if session.scrolling() {
        return scroller_key(session, key);
    }

    // Before the modifier guard, since the readline bindings are how the caret moves on a terminal
    // that sends nothing for the named keys, and a line that can be typed mid-turn has to be
    // editable mid-turn: the alternative is a box that takes words and will not let them be fixed.
    if edit_line(session, key) {
        return Action::Redraw;
    }

    // Before the modifier guard, since one of its two spellings is a Ctrl chord. A paragraph can be
    // written while a turn runs, like everything else typed here; plain Enter is still refused.
    if starts_a_line(key) {
        session.type_newline();
        return Action::Redraw;
    }

    // Before the modifier guard, since a line that can be typed mid-turn can be pasted into
    // mid-turn: what is refused while a turn runs is sending, never writing.
    if key.code == KeyCode::Char('v') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Action::Paste;
    }

    // Before the modifier guard for the same reason, and mid-turn is when it is wanted most: a
    // person reading back through a turn that is going wrong is reading because it is going wrong.
    if key.code == KeyCode::Char('o') && key.modifiers.contains(KeyModifiers::CONTROL) {
        session.open_scroller();
        return Action::Redraw;
    }

    // After the arm that starts a line, so Shift-Enter still writes a paragraph, and before the
    // ladder, where Enter means nothing. A second turn still must not begin while the first is in
    // flight; what changes is that the line no longer waits in the box for the person to notice
    // the turn has ended and press Enter again.
    if key.code == KeyCode::Enter && session.queue() {
        return Action::Redraw;
    }

    // Refused here rather than left to fall through the ladder's catch-all for control chords. The
    // editor takes the terminal for a child process, which is the screen the turn is drawing on, and
    // the line it hands back would be waiting for a box that has moved on. That is a decision about
    // this key, and a key that does nothing by accident reads the same as one that does nothing on
    // purpose.
    if key.code == KeyCode::Char('g') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Action::None;
    }

    // The same ladder the idle path uses, rather than a shorter copy of it. Nothing in it sends,
    // so there is nothing here for a running turn to refuse.
    navigate(session, key)
}

/// Interpret a paste.
///
/// A paste is one act rather than a run of keys, which is the whole point of asking the
/// terminal for it separately: the text lands in the box and the user decides when to send it.
/// It never submits, whatever it ends with.
/// Answer a paste the terminal delivered, which is what Command-V and the middle mouse button
/// become by the time they reach this process.
///
/// A paste that arrives carrying nothing is the interesting one. It means the terminal wrote the
/// bracketed-paste markers with nothing between them, which is what happens when the clipboard
/// holds something with no text in it at all: a picture. The terminal has no way to send that down
/// a pty and no way to say so either, so the empty paste is the whole of the signal, and the answer
/// is to go and read the clipboard directly.
///
/// Said once per session and then not again, because a user who has been told which key carries a
/// picture does not need telling every time they use the other one.
pub fn handle_paste(session: &mut Session, text: &str) -> Action {
    // A picture copied to the clipboard reaches a terminal as a paste of nothing at all, since
    // the terminal hands over text and there is none. Said before anything else looks at the
    // text, because an empty paste is no more a drop than it is a prompt.
    if text.is_empty() {
        session.note_once(t!(paste_arrived_empty));
        return Action::Paste;
    }
    // A drop reaches the terminal as a paste of the path, so this is where one is recognised.
    // Anything that is not a drop is text the user pasted, which lands in the box whole or behind
    // a marker depending on how much of the screen it was about to take.
    if !session.drop_files(text) {
        session.paste_text(text);
    }
    Action::Redraw
}

/// Bring what is on the clipboard into the prompt.
///
/// Separated from the loop so it can be tested without a terminal, and taking the read as an
/// argument so a test can say what the clipboard held.
fn take_from_clipboard(session: &mut Session, pasted: crate::clipboard::Pasted) {
    use crate::clipboard::{MAX_IMAGE_BYTES, Pasted};

    // Whatever the answer was, it is the current one, so the hint has served its purpose. It comes
    // back at the next focus change if the picture is still there and still wanted.
    session.image_on_clipboard = false;

    match pasted {
        // A command line is not a sentence, so a marker in one names nothing and would be passed to
        // the shell as literal text. Saying so beats writing it and letting the shell complain.
        Pasted::Image(_) if session.shell => session.note(t!(paste_not_a_command)),
        Pasted::Image(image) => session.attach(image),
        Pasted::Text(text) => session.paste_text(&text),
        Pasted::TooLarge(bytes) => session.note(t!(
            paste_too_large,
            size = in_megabytes(bytes),
            limit = in_megabytes(MAX_IMAGE_BYTES)
        )),
        Pasted::Nothing => session.note(t!(paste_nothing_on_clipboard)),
    }
}

/// A byte count as a person would say it, since nobody reads seven digits off a screen.
fn in_megabytes(bytes: usize) -> String {
    let size = format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))
        .replace('.', t!(number_decimal_separator));
    t!(megabytes, size = size)
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
        // While the scroller is open the wheel is one of its keys, so it stops at the first row
        // and the last the way every other movement in that mode does.
        MouseEventKind::ScrollUp if session.scrolling() => {
            session.scroller_back(3);
            Action::Redraw
        }
        MouseEventKind::ScrollDown if session.scrolling() => {
            session.scroller_on(3);
            Action::Redraw
        }
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
/// Draw a frame, and take back what it laid the transcript out to.
///
/// Every draw a person looks at goes through here, so nothing can redraw without the session
/// learning the shape of what was drawn. The scroller's keys are answered against those numbers,
/// and a stale set is a jump to the wrong row.
fn redraw(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    session: &mut Session,
) -> io::Result<()> {
    let mut laid = crate::state::Laid::default();
    terminal
        .draw(|frame| laid = render::draw(frame, session))
        .map_err(io::Error::other)?;
    session.note_layout(laid);
    Ok(())
}

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
        // Drawn to be read back rather than to be looked at, and it draws what the last frame
        // drew, so it has nothing new to tell the session about the layout.
        let completed = terminal
            .draw(|frame| {
                render::draw(frame, session);
            })
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
/// Returns the id of the session left behind, where there is one to pick up again.
pub fn run(
    config: &mut Config,
    workspace: &Workspace,
    confinement: String,
    start: Start,
) -> io::Result<Option<String>> {
    let mut stdout = io::stdout();
    take_over_terminal(&mut stdout)?;

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
        // The picker refuses Enter on one of these, and `--resume` refuses it too. If a record
        // still arrives here, loading its empty conversation as a turn would continue a run
        // that cannot be continued.
        Some(Start::Resuming(record)) if record.manifest.is_some() => Ok(None),
        Some(start) => event_loop(&mut terminal, config, workspace, confinement, start),
        // Leaving at the picker resumed nothing and started nothing, so there is nothing to say
        // about picking anything up.
        None => Ok(None),
    };

    // Restore the terminal even if the loop failed: leaving a user in raw mode on an
    // alternate screen is worse than the original error.
    hand_back_terminal(terminal.backend_mut())?;
    terminal.show_cursor()?;

    result
}

/// Put the terminal into the state the interface draws in.
///
/// Mouse capture is what makes the wheel scroll the transcript. It costs the terminal's own text
/// selection, so it is given back on the way out.
///
/// Bracketed paste is what keeps a pasted prompt from sending itself. Without it the terminal
/// delivers a paste as ordinary keystrokes, and the newline most clipboards carry at the end
/// arrives as Enter.
///
/// Disambiguated keys are what makes Shift-Enter reach the interface at all. A terminal sends the
/// same byte for Enter however it was pressed, so without this the modifier is not merely ignored,
/// it never arrives, and a newline in the prompt would be unreachable.
///
/// Asked for only where the terminal says it understands the request. Sending it blind to one that
/// does not leaves the escape sequence on the screen.
///
/// One definition rather than one per caller, because the interface gives the terminal away and
/// takes it back again whenever the prompt is edited elsewhere, and a difference between the two
/// setups would show as a mode that only survives until the first edit.
fn take_over_terminal<W: Write>(out: &mut W) -> io::Result<()> {
    enable_raw_mode()?;
    // After raw mode, so the reply arrives as bytes rather than a line, and before the
    // alternate screen, so the query is not painted into the session.
    crate::theme::sense(out);
    crate::theme::restore_saved();
    execute!(
        out,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste,
        // Asked for so the clipboard can be looked at when the user comes back from copying
        // something, which is the moment a picture appears on it and the only cheap moment to
        // notice. Polling instead would spawn a clipboard tool every few frames forever.
        EnableFocusChange
    )?;

    if enhanced_keys() {
        execute!(
            out,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }

    // Mouse capture asks for motion reported whether or not a button is down, which is a stream
    // of events for a pointer merely crossing the window and a redraw for each one. Only the
    // drag matters here, so all-motion reporting goes back off: what stays on reports the
    // buttons, the wheel, and motion while a button is held, which is the gesture being read.
    write!(out, "{TRACK_MOTION_ONLY_WHILE_DRAGGING}")?;
    out.flush()
}

/// Put the terminal back the way it was found.
fn hand_back_terminal<W: Write>(out: &mut W) -> io::Result<()> {
    disable_raw_mode()?;
    // Popped before the modes below, so the stack is unwound in the order it was built.
    if enhanced_keys() {
        execute!(out, PopKeyboardEnhancementFlags)?;
    }
    execute!(
        out,
        DisableBracketedPaste,
        DisableFocusChange,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    out.flush()
}

/// Whether the terminal understands the request for disambiguated keys.
///
/// Asked once and remembered, because answering means writing a query and waiting for a reply, and
/// doing that on every handover to an editor would cost a round trip each time.
fn enhanced_keys() -> bool {
    use std::sync::OnceLock;
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| supports_keyboard_enhancement().unwrap_or(false))
}

/// Hand the terminal to the user's editor, and take back whatever they saved.
///
/// The editor gets the terminal properly rather than sharing it: raw mode off, the alternate
/// screen left, the mouse and paste modes given back. A full-screen editor drawing over an
/// interface that still believes it owns the screen is the alternative, and neither of them
/// would be legible.
///
/// The screen is cleared on the way back because the alternate screen came back empty while the
/// interface still holds the frame it drew before leaving. Without it the first redraw sends only
/// what changed, over a screen that has nothing under it.
fn edit_prompt(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    session: &mut Session,
) -> io::Result<()> {
    hand_back_terminal(terminal.backend_mut())?;
    terminal.show_cursor()?;

    let edited = crate::editor::edit(session.input());

    take_over_terminal(terminal.backend_mut())?;
    terminal.clear()?;

    match edited {
        Ok(line) => session.take_edited(line),
        // Said rather than swallowed: an editor that would not start looks exactly like a key
        // that does nothing, and the user has no other way to tell the two apart.
        Err(failure) => session.note(failure.to_string()),
    }
    Ok(())
}

/// Sign in to the backend the next request will use, where it needs one and has none.
///
/// The URL and the code go into the transcript, where the person is already looking. The interface
/// keeps the screen throughout: handing it over instead put the one thing somebody has to read and
/// type underneath a display that was about to be redrawn, and left them in a terminal that no longer
/// looked like the program they were using.
///
/// Off-thread for the reason a turn is: the sign-in waits for a browser to be visited, which is as
/// long as the person takes, and run here it would freeze the interface for the whole of it.
///
/// Nothing happens in the common case. A good session is not a sign-in, a build with no AWS
/// configuration cannot want one, and a model served by Brave never needs one whatever else is
/// configured, so the question is asked of the model about to answer rather than of what exists.
fn sign_in_if_needed(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    session: &mut Session,
    config: &Config,
) -> io::Result<()> {
    let model = session.model().unwrap_or(&config.default_model).to_string();
    if !bravebot_agent::backend::Backend::needs_sign_in(config, &model) {
        return Ok(());
    }

    session.note(t!(session_signing_in));

    let (lines, arriving) = mpsc::channel::<String>();
    let worker_config = config.clone();
    let worker = thread::spawn(move || {
        bravebot_agent::backend::Backend::sign_in_if_needed(&worker_config, &model, |line| {
            // A closed channel is an interface that has stopped listening, and there is nothing to
            // be done about it from here: the sign-in is already running.
            let _ = lines.send(line);
        })
        .map_err(|failure| failure.to_string())
    });

    // Drawn as they arrive rather than collected, because a code is only useful while the command
    // that printed it is still waiting.
    loop {
        redraw(terminal, session)?;

        match arriving.recv_timeout(FRAME) {
            Ok(line) => session.note(line),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Said rather than swallowed, and not fatal: the turn goes ahead and fails with the backend's
    // own account of what is wrong, which is more use than this function's guess at it.
    if let Ok(Err(failure)) = worker.join() {
        session.note(failure);
    }
    redraw(terminal, session)?;
    Ok(())
}

/// Hand the transcript to the user's editor, and take nothing back from it.
///
/// The terminal is given up and taken back the way it is for a prompt, since an editor needs the
/// screen. Nothing is read afterwards: the file was a look at the record, not a draft of it.
fn show_transcript(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    session: &mut Session,
) -> io::Result<()> {
    let text = render::as_text(session);

    hand_back_terminal(terminal.backend_mut())?;
    terminal.show_cursor()?;

    let shown = crate::editor::show(&text);

    take_over_terminal(terminal.backend_mut())?;
    terminal.clear()?;

    // Said rather than swallowed, for the reason the prompt's editor says it: an editor that
    // would not start looks exactly like a key that does nothing.
    if let Err(failure) = shown {
        session.note(failure.to_string());
    }
    Ok(())
}

/// Concrete in the backend rather than generic: the loop is only ever driven by a real
/// terminal, and a generic backend's error type carries no bounds to convert from.
/// The session's own name, for telling somebody how to pick it up again.
///
/// Nothing for a session that was opened and left without sending anything: there is no record to
/// resume, and naming one would be offering a command that answers "no session by that name".
fn left_behind(stored: &crate::sessions::Handle) -> Option<String> {
    stored.resumable().map(str::to_string)
}

/// Returns the id of the session left behind, where there is one to pick up again.
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config: &mut Config,
    workspace: &Workspace,
    confinement: String,
    start: Start,
) -> io::Result<Option<String>> {
    // Owned rather than borrowed, because `/add-dir` opens another directory partway through and
    // the turns after it must see one. The primary root never changes, so nothing keyed on it
    // (the session record, where AGENTS.md is looked for) moves underneath.
    let mut workspace = workspace.clone();

    // The one place persistence is turned on: history in ~/.bravebot outlives the session.
    let mut session = Session::new(confinement)
        .with_stored_history()
        .in_workspace(workspace.root())
        .on_tier(crate::status::configured_tier(config));

    // The model outlived the session that chose it, so the window that came with it has to be asked
    // for again: it is reported by the listing and nowhere else, and nothing on disk remembers it.
    adopt_budget_for_current_model(&mut session, config);

    // Outlives every turn, which is the point: a turn begins with the exchange so far rather
    // than with nothing, so the user can say "try that again" and be understood. A resumed
    // session begins with an exchange that outlived the process it happened in.
    let (mut conversation, mut stored, inherited_trust, mut programs) = match start {
        // Already answered before the loop was entered: the picker runs once, in `run`.
        Start::Fresh | Start::Choose => (
            Conversation::new(),
            crate::sessions::Handle::begin(workspace.root()),
            None,
            // A session that was never asked vouches for nothing, exactly as with the map.
            TrustedPrograms::new(),
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
            session.restore_spend(record.tokens, record.spend.clone());
            session.restore_timing(record.timing.clone());
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
            if let Some(note) = crate::sessions::build_note(record.build.as_deref(), crate::BUILD) {
                session.note(note);
            }
            // The trust map goes with the session, so picking one up carries the answer its own
            // user gave. `None` for a record from before this was kept, which is asked about.
            let inherited = record.trust_map();
            // The programs go the same way and for the same reason: the person resuming is the
            // person who vouched for them. Unlike the map there is nothing to ask about an
            // absent list, since an empty one simply means every run asks.
            let vouched = record.trusted_programs();
            // The other half of `/add-dir`, which the map cannot carry: a directory has to be
            // open for an absolute path in it to resolve at all. Restored here so the rule and
            // the reach come back together, rather than the rule alone.
            for note in record.reopen_added_directories(&mut workspace) {
                session.note(note);
            }
            (conversation, handle, inherited, vouched)
        }
    };

    // Settled once, before any turn. Nothing means the user left at the question, and a session
    // they never agreed to have must not begin behind it.
    let Some(mut trust) = opening_trust(terminal, &mut session, workspace.root(), inherited_trust)
    else {
        return Ok(left_behind(&stored));
    };

    // The rules the user wrote in advance, read once for the session: a person editing the file
    // mid-session is describing the next one, and rules that changed halfway through a turn would
    // be the harder thing to explain. Every turn below is given these.
    let settings = bravebot_config::Settings::load();
    let (permissions, rejected) = bravebot_agent::permissions::from_settings(
        &settings,
        bravebot_agent::home::directory().as_deref(),
    );
    // Said out loud, because a rule that parses as nothing is a rule somebody believes is in
    // force. A misspelled deny rule reads as protection that is not there.
    for problem in &rejected {
        session.note(t!(
            session_permission_rule_ignored,
            problem = problem.to_string()
        ));
    }
    // Named after the startup question, so a directory a file asked for is opened on the same
    // terms as one typed at `/add-dir`, and after the person has agreed to the workspace at all.
    for directory in bravebot_agent::permissions::additional_directories(&settings) {
        let named = against_workspace(workspace.root(), directory);
        add_directory(&mut session, &mut workspace, &mut trust, &named);
    }

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
            redraw(terminal, &mut session)?;
            needs_draw = false;
            drawn_at = Instant::now();
        }

        if session.is_quitting() {
            return Ok(left_behind(&stored));
        }

        // A loop's next tick, looked at before the interface settles down to wait for a key.
        // Nobody is going to press anything to make it happen, so a tick taken only after input
        // arrives would sit there until somebody typed something unrelated. It becomes an
        // ordinary submission, because that is what it is: the line the person typed at `/loop`,
        // sent again.
        let action = match session.loop_tick() {
            Some(prompt) => Action::Submit(prompt),
            None => {
                if !event::poll(POLL)? {
                    continue;
                }
                match event::read()? {
                    // Presses only. Asking for disambiguated keys asks for releases as well, and a
                    // release handled as a press types every character twice.
                    TermEvent::Key(key) if key.kind == KeyEventKind::Release => Action::None,
                    TermEvent::Key(key) => handle_key(&mut session, key),
                    TermEvent::Mouse(mouse) => handle_mouse(&mut session, mouse),
                    TermEvent::Paste(text) => handle_paste(&mut session, &text),
                    // Coming back from copying something is the moment a picture appears on the
                    // clipboard, and the cheapest moment to notice: once per switch away and back,
                    // rather than a clipboard tool spawned on a timer for the whole life of the
                    // session.
                    TermEvent::FocusGained => {
                        session.image_on_clipboard = crate::clipboard::holds_an_image();
                        Action::Redraw
                    }
                    _ => Action::None,
                }
            }
        };

        needs_draw |= !matches!(action, Action::None);

        match action {
            Action::Quit => return Ok(left_behind(&stored)),
            Action::Copy => copy_selection(terminal, &mut session)?,
            Action::Paste => take_from_clipboard(&mut session, crate::clipboard::paste()),
            Action::Edit => {
                edit_prompt(terminal, &mut session)?;
                needs_draw = true;
            }
            Action::Show => {
                show_transcript(terminal, &mut session)?;
                needs_draw = true;
            }
            Action::ChooseModel => {
                choose_model(terminal, &mut session, config);
                needs_draw = true;
            }
            Action::ChooseTheme => {
                choose_theme(terminal, &mut session);
                needs_draw = true;
            }
            Action::SetTheme(name) => {
                set_theme(&mut session, &name);
                needs_draw = true;
            }
            Action::AddDirectory(directory) => {
                add_directory(&mut session, &mut workspace, &mut trust, &directory);
            }
            Action::Rename(name) => {
                if name.is_empty() {
                    session.note(t!(session_rename_needs_a_name));
                } else if stored.rename(&name) {
                    session.note(t!(session_renamed, title = stored.title()));
                } else {
                    session.note(t!(session_rename_needs_something));
                }
            }
            Action::Status => {
                let theme = crate::theme::name();
                let report = crate::status::report(&crate::status::Facts {
                    session_name: stored.title(),
                    session_id: stored.id(),
                    directory: workspace.root(),
                    added_directories: workspace.added_directories(),
                    model: session.model(),
                    served_model: session.served_model(),
                    premium: session.premium(),
                    theme: &theme,
                    config,
                    confinement: &session.confinement,
                    turns: session.turns,
                    tokens: session.tokens,
                    timing: session.timing_total(),
                    trust: &trust,
                    programs: &programs,
                    looping: session.looping(),
                });
                session.report(report);
                needs_draw = true;
            }
            Action::Compact => {
                let events;
                (conversation, events) =
                    compact_animated(terminal, &mut session, config, conversation, &trust)?;

                // Written now rather than at the end of the next turn: the shortening is the
                // change, and a session that compacted and then slept should resume compacted.
                let title = stored.title().to_string();
                stored.save(
                    &title,
                    crate::sessions::Standing {
                        conversation: &conversation.snapshot(),
                        turns: session.turns,
                        tokens: session.tokens,
                        spend: session.spend_by_turn(),
                        timing: session.timing_by_turn(),
                        model: session.served_model(),
                        todos: &session.todos_by_turn(),
                        trust: &trust,
                        programs: &programs,
                        directories: workspace.added_directories(),
                        manifest: None,
                    },
                );
                stored.append_audit(session.turns, &events);
                needs_draw = true;
            }
            Action::Clear => {
                // A new handle means a new id, so the session so far keeps its own files and stays
                // resumable. Nothing is deleted: what the user asked for is a clean context, and
                // throwing away the record would be answering a question they did not ask.
                session.clear();
                conversation = Conversation::new();
                stored = crate::sessions::Handle::begin(workspace.root());
                session.note(t!(session_cleared));

                // A new session, so it is asked what a new session is asked. The map goes with the
                // context and the directories opened under it go too, since opening one is a grant
                // and leaving it reachable with nothing vouching for it would outlive its answer.
                workspace.close_added_directories();
                let Some(fresh) = opening_trust(terminal, &mut session, workspace.root(), None)
                else {
                    return Ok(left_behind(&stored));
                };
                trust = fresh;
                // A new session vouches for no program, on the same reasoning as the map: the
                // list is a standing permission, and this begins a session that was never asked.
                programs = TrustedPrograms::new();
                needs_draw = true;
            }
            Action::Submit(prompt) => {
                // A prompt sent while this one was running goes when it ends, in the order it was
                // typed, and so does anything typed during that one. Looping here rather than
                // going back round the outer loop keeps a queued prompt from waiting on a key
                // press that nobody is there to make.
                let mut sending = Some(prompt);
                while let Some(prompt) = sending {
                    // Both are threaded through: a turn that writes untrusted data into a trusted
                    // path records that, and the next turn must honour it, and a turn that has been
                    // had is a turn the next one can be asked about.
                    let events;
                    (conversation, trust, programs, events) = run_turn_animated(
                        terminal,
                        &mut session,
                        config,
                        &workspace,
                        &prompt,
                        conversation,
                        trust,
                        programs,
                        &permissions,
                    )?;

                    // Written after each turn rather than at the end, because the end may never
                    // come: the session worth resuming is the one whose machine slept and never
                    // woke. Best-effort, like everything else under ~/.bravebot.
                    stored.save(
                        &prompt,
                        crate::sessions::Standing {
                            conversation: &conversation.snapshot(),
                            turns: session.turns,
                            tokens: session.tokens,
                            spend: session.spend_by_turn(),
                            timing: session.timing_by_turn(),
                            model: session.served_model(),
                            todos: &session.todos_by_turn(),
                            trust: &trust,
                            programs: &programs,
                            directories: workspace.added_directories(),
                            manifest: None,
                        },
                    );
                    stored.append_audit(session.turns, &events);

                    // Nothing waiting goes out after somebody has asked to leave.
                    sending = if session.is_quitting() {
                        None
                    } else {
                        session.send_queued()
                    };
                }
            }
            Action::Run(line) => {
                let events =
                    run_command(terminal, &mut session, &workspace, &line, &mut conversation)?;
                // Saved like a turn, and for the same reason: the command is in the conversation
                // now, so a session resumed without it would have the planner referring to output
                // it can no longer see.
                stored.save(
                    &line,
                    crate::sessions::Standing {
                        conversation: &conversation.snapshot(),
                        turns: session.turns,
                        tokens: session.tokens,
                        spend: session.spend_by_turn(),
                        timing: session.timing_by_turn(),
                        model: session.served_model(),
                        todos: &session.todos_by_turn(),
                        trust: &trust,
                        programs: &programs,
                        directories: workspace.added_directories(),
                        manifest: None,
                    },
                );
                stored.append_audit(session.turns, &events);
                needs_draw = true;
            }
            // Cancel is only reachable while a turn runs, which `run_turn_animated` handles.
            Action::Cancel | Action::None | Action::Redraw => {}
        }
    }
}

/// Open another directory and vouch for it, for the rest of this session.
///
/// Two things happen together, and both are needed. The workspace makes the directory reachable at
/// all, since an absolute path is refused otherwise. The trust map records that the user vouched
/// for it, which is what the write gates consult. Doing only the first would leave every write
/// there asking; doing only the second would leave a rule about files nothing can open.
///
/// The path recorded is the canonical one, not the name typed: `~/notes/../notes` and a symlink
/// both name a directory whose rules should be about where it actually is.
///
/// Session-scoped on purpose. `docs/specs/trust-map.md` is explicit that trust is not sticky per directory,
/// so a later session starts without this and is asked again. It does survive `--resume`, since
/// that restores the map its own user gave.
fn add_directory(
    session: &mut Session,
    workspace: &mut Workspace,
    trust: &mut TrustStore,
    directory: &str,
) {
    if directory.is_empty() {
        session.note(t!(session_add_dir_needs_a_path));
        return;
    }

    // Expanded here rather than in the workspace, because `~` is a shell convention and a library
    // resolving it would be guessing at a home the caller never named.
    let expanded = expand_home(directory);

    match workspace.add_directory(&expanded) {
        Ok(added) => {
            let shown = added.display().to_string();
            trust.trust(&shown);
            session.note(t!(session_directory_added, directory = shown));
        }
        Err(error) => session.note(t!(
            session_directory_not_added,
            directory = directory,
            problem = error
        )),
    }
}

/// The name a settings file gave a directory, as an absolute path.
///
/// A relative name means a path under the workspace, which is what `../shared` in a settings file
/// says: the file is about a project, and the directory it wants is next to that project rather
/// than next to wherever the agent happened to be started from.
fn against_workspace(root: &std::path::Path, directory: &str) -> String {
    match std::path::Path::new(directory).is_absolute() {
        true => directory.to_string(),
        false => root.join(directory).display().to_string(),
    }
}

/// Replace a leading `~` with the user's home directory.
///
/// Only a leading one, and only when it is the whole first segment, so a directory genuinely called
/// `~notes` is left alone. Without a home to expand to, the path is passed through and the
/// workspace refuses it for not being absolute, which says the same thing.
fn expand_home(directory: &str) -> String {
    let Some(rest) = directory.strip_prefix('~') else {
        return directory.to_string();
    };
    if !(rest.is_empty() || rest.starts_with('/')) {
        return directory.to_string();
    }
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => {
            format!("{}{rest}", std::path::Path::new(&home).display())
        }
        _ => directory.to_string(),
    }
}

/// Ask the endpoint what it offers, let the user pick, and remember what they picked.
///
/// A refusal or an unreachable endpoint leaves the model as it was and says so. That is the right
/// outcome for a list nobody could fetch: guessing a set of names would offer choices the backend
/// may not have, and a picker showing only "automatic" would look like a server with one model.
///
/// The list is content and the choice is routing. Nothing here is quarantined, because there is no
/// planner context to keep it out of: the names are drawn for a person, and their pick is the
/// endorsement for the request field it lands in.
/// Ask the endpoint what it offers.
///
/// Shared by the picker and by the budget lookup a session does when it starts, because both want
/// the same listing and a second copy of the policy setup would be a second place for the gate to
/// be got wrong.
fn list_models(
    config: &Config,
    chosen: Option<&str>,
) -> Result<Vec<bravebot_aichat::models::Model>, String> {
    // Bedrock is additive. Its tiers come from configuration rather than from a listing, since it has
    // no listing endpoint and an ARN does not say which model it resolves to, but the Brave roster is
    // what everyone has and a settings block adds to it rather than replacing it.
    //
    // The configured tiers come first: someone who named them went out of their way to, and the
    // alternative buries them under a roster they did not ask about.
    let mut configured = config
        .bedrock
        .as_ref()
        .map(bedrock_models)
        .unwrap_or_default();

    // Additive on the same terms. A block that named its models is taken at its word and costs no
    // round trip, which is what keeps a configured gateway working offline. One that named none is
    // asked, because the alternative is a gateway configured exactly as the tool this block's shape
    // came from configures it, offering nothing.
    for provider in &config.providers {
        match provider.models.is_empty() {
            false => configured.extend(provider_models(provider)),
            true => configured.extend(in_reading_order(
                fetch_gateway_models(provider).unwrap_or_default(),
                chosen.unwrap_or(&config.default_model),
            )),
        }
    }

    // A build from source pointed only at Bedrock has blank Brave credentials, and asking with them
    // would list a roster whose every request then fails unsigned.
    if !config.serves_aichat() {
        return Ok(configured);
    }

    combined(configured, fetch_models(config))
}

/// The tiers and the listing as one roster.
///
/// Split from the request so the joining is testable without a server. A listing nobody could fetch
/// is not worth losing the tiers over: they need no network to know, and a picker that refused
/// everything because one half was unreachable would leave the working half unpickable.
fn combined(
    configured: Vec<bravebot_aichat::models::Model>,
    listed: Result<Vec<bravebot_aichat::models::Model>, String>,
) -> Result<Vec<bravebot_aichat::models::Model>, String> {
    match listed {
        Ok(listed) => Ok(configured.into_iter().chain(listed).collect()),
        Err(_) if !configured.is_empty() => Ok(configured),
        Err(problem) => Err(problem),
    }
}

/// Ask the Brave endpoint what it offers.
fn fetch_models(config: &Config) -> Result<Vec<bravebot_aichat::models::Model>, String> {
    let mut sink = Trail::new();
    let egress = Egress::new();

    // A policy exists because `bravebot-net` is the only way out to the network and its gate takes one.
    // Routing is the listing itself: this is not a turn, nothing is read from the workspace, and no
    // model is involved, so there is no prompt to anchor it to.
    let mut routing = bravebot_core::policy::Routing::new();
    routing.insert_trusted("models", config.models_url());

    bravebot_core::policy::Policy::begin(
        routing,
        bravebot_core::policy::ReleasePlan::new(),
        bravebot_core::capability::CapabilitySet::from_iter([
            bravebot_core::capability::Capability::WebFetch,
        ]),
        &mut sink,
    )
    .map_err(|denial| denial.to_string())
    .and_then(|mut policy| {
        bravebot_aichat::models::list(&mut policy, config, &egress)
            .map_err(|error| error.to_string())
    })
}

/// Ask one gateway what it offers, for a block that named no models.
///
/// What comes back names the gateway the same way a configured model from it does, so one service
/// does not appear twice under two names in the same list.
///
/// A gateway nothing holds a credential for is not asked. The listing would come back refused, and
/// the useful thing to say about that gateway is what `doctor` already says: no credential found.
fn fetch_gateway_models(
    provider: &bravebot_config::provider::Provider,
) -> Result<Vec<bravebot_aichat::models::Model>, String> {
    let token = provider
        .token(|name| std::env::var(name).ok())
        .ok_or_else(|| format!("no credential for {}", provider.display_name()))?;

    let mut sink = Trail::new();
    let egress = Egress::new();

    // Both destinations are the gateway's own endpoint, which came from configuration. Nothing
    // fetched decides either, which is what makes asking a service for a list of names safe at all.
    let mut routing = bravebot_core::policy::Routing::new();
    routing.insert_trusted("models", provider.models_url());
    routing.insert_trusted("account-models", provider.account_models_url());

    bravebot_core::policy::Policy::begin(
        routing,
        bravebot_core::policy::ReleasePlan::new(),
        bravebot_core::capability::CapabilitySet::from_iter([
            bravebot_core::capability::Capability::WebFetch,
        ]),
        &mut sink,
    )
    .map_err(|denial| denial.to_string())
    .and_then(|mut policy| {
        bravebot_aichat::models::list_from_gateway(&mut policy, provider, &token, &egress)
            .map_err(|error| error.to_string())
    })
}

/// A fetched gateway roster in the order a person should meet it: what a session would use, then
/// everything else by name.
///
/// The gateway's own order is roughly newest-first, which puts a model somebody has never heard of at
/// the top and buries the one they work with. A configured roster needs none of this: the file is
/// already the order they chose.
///
/// Alphabetical by the key, so the provider id groups the rows and a name somebody half-remembers is
/// where they would look for it. Sorted rather than capped, because a picker filters as it is typed
/// and dropping rows would decide somebody may not choose a model their gateway serves.
fn in_reading_order(
    mut models: Vec<bravebot_aichat::models::Model>,
    chosen: &str,
) -> Vec<bravebot_aichat::models::Model> {
    models.sort_by(|left, right| {
        let ranked = |model: &bravebot_aichat::models::Model| model.key != chosen;
        ranked(left)
            .cmp(&ranked(right))
            .then_with(|| left.key.cmp(&right.key))
    });
    models
}

/// The models a Bedrock configuration offers, strongest tier first.
///
/// One entry per tier that names a model, and nothing else. A tier whose variable is unset is one
/// this configuration cannot reach: an ARN cannot be derived from a model name, so an entry invented
/// for it would be a choice that fails at the far end for a reason nothing here could explain.
///
/// No `automatic` among them. There it means "let the server choose", which Bedrock does not offer: a
/// request names one model and gets it or an error. The entry still reaches the picker, from the Brave
/// half of the roster, where it is a choice that backend can honour.
///
/// Every entry is marked free. Premium here means a Leo subscription, and reaching a model through
/// somebody's own AWS account does not involve one.
///
/// Every entry says whose account answers. Both rosters are offered together and a bare tier name
/// would sit beside a Brave entry for the same model, where the two are reached and billed
/// differently and nothing else would say which was about to answer.
///
/// Not "Bedrock" alone: the Brave roster already says that of models it serves through its own AWS
/// account, so the word distinguishes nothing. The profile is the useful thing, being what decides
/// which credentials sign the request, and the account is all that can be said without one.
fn bedrock_models(
    bedrock: &bravebot_config::bedrock::Bedrock,
) -> Vec<bravebot_aichat::models::Model> {
    bedrock
        .models()
        .iter()
        .map(|(tier, name)| bravebot_aichat::models::Model {
            key: name.clone(),
            // The tier alone, because the account it is reached through is the same for all of them
            // and is said once, over the section these rows sit in.
            display_name: tier.display_name().to_string(),
            premium: false,
            provider: Some(match bedrock.profile.as_deref() {
                Some(profile) => t!(picker_service_bedrock_profile, profile = profile),
                None => t!(picker_service_bedrock).to_string(),
            }),
            // The same figure for every tier, because it is a property of what an opaque profile
            // ARN gets rather than of a particular model.
            conversation_tokens: Some(bravebot_config::bedrock::CONTEXT_WINDOW),
        })
        .collect()
}

/// The models a gateway was configured to offer, in the order the file listed them.
///
/// One entry per named model, and nothing else. A gateway's own roster runs to hundreds of models
/// across upstreams most people will never use, so asking it would produce a listing nobody could
/// pick from, and asking it at all would cost a round trip on a path that must still work offline.
///
/// Every entry is marked free. Premium means a Leo subscription, and a gateway reached with somebody's
/// own bearer token does not involve one. What it costs them is between them and the gateway.
///
/// Every entry says which service answers it, because the same slug may be reachable more than one
/// way: `anthropic/claude-sonnet-4.5` through a gateway and Brave's own Sonnet are different bills
/// and different credentials, and nothing else about the model would say which was about to answer.
fn provider_models(
    provider: &bravebot_config::provider::Provider,
) -> Vec<bravebot_aichat::models::Model> {
    provider
        .models
        .iter()
        .map(|model| bravebot_aichat::models::Model {
            // Qualified by the provider's own id, because the key is what a choice is remembered as
            // and what later selects a backend. The same slug may be reachable through more than one
            // service, and the bare name says nothing about which was picked.
            key: format!("{}/{}", provider.id, model.id),
            // The slug whole, since it is what a request names, and unqualified, since the gateway
            // is said over the section rather than on every row under it.
            display_name: model.id.clone(),
            premium: false,
            provider: Some(provider.display_name().to_string()),
            // Stated or assumed, never absent: a window is what the budget is taken from, and
            // reporting nothing would leave the session on a default chosen for a different service.
            conversation_tokens: Some(model.window()),
        })
        .collect()
}

/// Take the budget for the model already in force, without asking anyone to choose it again.
///
/// A model chosen in an earlier session is read back off disk, and until this ran the window that
/// came with it was not: the budget stayed at the default and a session with room for a hundred
/// thousand tokens compacted at twenty-four, having said nothing about why.
///
/// A listing that cannot be fetched is not worth a word. The budget falls back to the default, which
/// is what it was before this existed, and a session that is merely offline should not open with a
/// complaint about a request nobody asked for.
fn adopt_budget_for_current_model(session: &mut Session, config: &mut Config) {
    let Ok(models) = list_models(config, session.model()) else {
        return;
    };
    if config.adopt_window(advertised_window(&models, session.model())) {
        session.note(t!(session_context_budget, budget = config.context_budget));
    }
}

/// The window advertised for `chosen`, or `None` where the listing does not describe it.
///
/// Split from the fetch so the matching is testable without a server. Nothing chosen means
/// `automatic`, whose model is resolved per request, so no entry's window is the one in force.
fn advertised_window(
    models: &[bravebot_aichat::models::Model],
    chosen: Option<&str>,
) -> Option<u64> {
    let name = chosen?;
    models
        .iter()
        .find(|model| model.key == name)?
        .conversation_tokens
}

fn choose_model(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    session: &mut Session,
    config: &mut Config,
) {
    match list_models(config, session.model()) {
        Ok(models) => {
            let chosen = crate::model_prompt::choose(terminal, models, session.model(), |frame| {
                render::draw(frame, session);
            });
            if let Some(chosen) = chosen {
                // The listing is the only place a window is ever reported, so the budget is taken
                // here, while the entry that named it is in hand. Said once when it changes, since
                // a budget belongs to the model rather than to a turn.
                if config.adopt_window(chosen.conversation_tokens) {
                    session.note(t!(session_context_budget, budget = config.context_budget));
                }
                // Which service answers is said here as well as in the picker: the row that
                // carried it is gone by the time the note is read, and the same slug reached
                // through two services is two bills.
                session.note(match chosen.provider.as_deref() {
                    Some(service) => t!(
                        session_using_model_from,
                        model = &chosen.display_name,
                        service = service
                    ),
                    None => t!(session_using_model, model = &chosen.display_name),
                });
                session.choose_model(chosen.key);
            }
        }
        Err(detail) => session.note(t!(session_models_unavailable, problem = detail)),
    }
}

/// Open the theme picker and persist what the person chose.
fn choose_theme(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, session: &mut Session) {
    let themes = crate::theme::offered();
    let current = crate::theme::name();
    if let Some(chosen) = crate::theme_prompt::choose(terminal, themes, &current, |frame| {
        render::draw(frame, session);
    }) {
        crate::store::save_theme(&chosen.name);
        session.note(t!(session_theme_set, theme = &chosen.name));
    }
}

/// Apply a theme by name without opening the picker.
fn set_theme(session: &mut Session, name: &str) {
    match crate::theme::find(name) {
        Some(theme) => {
            crate::theme::apply(&theme);
            crate::store::save_theme(&theme.name);
            session.note(t!(session_theme_set, theme = &theme.name));
        }
        None => session.note(t!(session_no_such_theme, theme = name)),
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
    let (trust, inherited) = match inherited {
        Some(trust) => (trust, true),
        None => (crate::trust_prompt::ask(terminal, root)?, false),
    };

    if !trust.is_trusted(".") {
        session.note(t!(session_not_trusting));
        return Some(trust);
    }
    let where_it_is = root.display();
    session.note(if inherited {
        t!(session_trusting_as_left, directory = where_it_is)
    } else {
        t!(session_trusting, directory = where_it_is)
    });
    Some(trust)
}

/// Run a command the user typed in shell mode, redrawing while it runs.
///
/// On a worker thread for the reason a turn is: a command can take as long as it likes, and running
/// it on the thread that owns the terminal would freeze the interface for the duration and make a
/// slow build indistinguishable from a hang. Escape cancels, which kills it.
///
/// No approval is asked for. The prompt a run normally goes through exists so a person endorses argv
/// the *planner* chose, and here the person typed it themselves: asking would be asking them to
/// confirm their own keystroke. What it printed goes into the conversation, labelled from that same
/// provenance by the kernel. See [`bravebot_agent::shell::record`].
fn run_command(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    session: &mut Session,
    workspace: &Workspace,
    line: &str,
    conversation: &mut Conversation,
) -> io::Result<Vec<Stamped>> {
    let cancel = Cancel::new();
    let worker_cancel = cancel.clone();
    let worker_line = line.to_string();
    // Commands run in the primary root, which is the directory the prompt says it is in. An added
    // directory is reachable by path from here, so nothing is out of reach.
    let directory = workspace.root().to_path_buf();

    // Only the process goes to the worker. The conversation stays here, because handing it over
    // means taking it back, and a panicked thread hands back nothing: substituting a fresh one would
    // reset context integrity to trusted, which is an upgrade and never allowed.
    let worker =
        thread::spawn(move || bravebot_agent::shell::run(&worker_line, &directory, &worker_cancel));

    // Drawn while it runs so the command appears in the transcript immediately and Escape has
    // somewhere to be pressed. The spinner is the turn indicator's, without the token counters,
    // which measure something no command spends.
    session.begin_command();
    while !worker.is_finished() {
        redraw(terminal, session)?;

        while event::poll(Duration::ZERO)? {
            match event::read()? {
                TermEvent::Key(key) if key.kind == KeyEventKind::Release => {}
                // A running command is something to stop, so Ctrl-C stops it and stays, for the
                // reason it stops a turn: the way out is the press after that, at the box.
                TermEvent::Key(key) if stops_the_turn(session, key) => {
                    cancel.cancel();
                }
                TermEvent::Mouse(mouse) => {
                    let action = handle_mouse(session, mouse);
                    if action == Action::Copy {
                        copy_selection(terminal, session)?;
                    }
                }
                // Everything else waits. A command is brief and the keys that matter during one are
                // the two above; taking a prompt here would leave it half-typed when the output
                // lands on top of it.
                _ => {}
            }
        }

        std::thread::sleep(FRAME.min(Duration::from_millis(50)));
    }

    let ran = worker.join().unwrap_or_else(|_| {
        Err(bravebot_agent::shell::ShellError::Io(
            t!(command_thread_stopped).to_string(),
        ))
    });
    session.finish_command();

    // The labelling happens here, on the thread that owns the conversation, and its trail is
    // returned so the decision reaches the audit file and Ctrl-T. Trusting a command's output is the
    // most consequential thing this feature does, so it must not be the one thing left unrecorded.
    let mut sink = Trail::new();
    match ran {
        Ok(ran) => match bravebot_agent::shell::record(line, &ran, conversation, &mut sink) {
            Ok(recorded) => {
                session.printed(&recorded.text);
                if !recorded.succeeded {
                    session.note(t!(command_reported_a_failure));
                }
            }
            Err(error) => session.note(format!("{error}")),
        },
        Err(error) => session.note(format!("{error}")),
    }

    Ok(sink.events().to_vec())
}

/// Summarise the conversation, showing the spinner while it happens.
///
/// A smaller relative of [`run_turn_animated`], and smaller because there is less to do: a
/// summariser has no tools, so nothing asks about a write, nothing asks the user a question, and
/// nothing lands in the workspace. What is left is a model call that takes as long as a round, and
/// a loop that keeps the screen alive while it does.
///
/// Not cancellable, deliberately. The call is one round with nothing to interrupt part way, and a
/// cancel would leave the same conversation it started with, which is what happens anyway if it
/// fails.
fn compact_animated(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    session: &mut Session,
    config: &Config,
    conversation: Conversation,
    trust: &TrustStore,
) -> io::Result<(Conversation, Vec<Stamped>)> {
    // For the reason a turn does it: the summary is one request to the same backend, and a sign-in
    // is not something a worker thread can ask for.
    sign_in_if_needed(terminal, session, config)?;

    let (to_main, from_worker) = mpsc::channel::<crate::remote_confirm::ToMain>();

    let worker_config = config.clone();
    let worker_trust = trust.clone();
    let model = session.model().map(str::to_string);

    session.begin_aside();

    let worker = thread::spawn(move || {
        let mut sink = Trail::new();
        let mut reporter = crate::remote_confirm::RemoteReporter::new(to_main);
        let egress = Egress::new();
        let mut conversation = conversation;
        // Reduced to what a person can be told before it crosses back, since the error types are
        // the kernel's and this thread is the only place they mean anything.
        let done = turn::compact(
            &worker_config,
            &egress,
            &mut conversation,
            model.as_deref(),
            &mut reporter,
            &mut sink,
            worker_trust,
        )
        .map_err(|e| e.to_string());
        (done, conversation, sink)
    });

    loop {
        redraw(terminal, session)?;

        // Input is still read, so a long summary does not leave the interface deaf, and the
        // frame's waiting is done here for the reason the turn loop does it here: a key press
        // has to wake the loop rather than queue behind it.
        if event::poll(FRAME)? {
            while event::poll(Duration::ZERO)? {
                match event::read()? {
                    // A summary is one request, so there is no round for a cancel to land between and
                    // nothing here can stop it. Said once, because the alternative is a key that does
                    // nothing and says nothing, which reads as the interface having hung at the one
                    // moment it is working hardest.
                    TermEvent::Key(key) if is_ctrl_c(key) && !session.scrolling() => {
                        // The one place Ctrl-C still leaves with something in flight: a summary is
                        // one request with no round for a stop to land between, so there is
                        // nothing here for the press to stop and leaving is all it can mean.
                        session.quit();
                    }
                    TermEvent::Key(key) if wants_cancel(key) && !session.scrolling() => {
                        session.note_once(t!(compact_uninterruptible));
                    }
                    TermEvent::Key(key) => {
                        handle_key_while_working(session, key);
                    }
                    TermEvent::Paste(text) => handle_paste_while_working(session, &text),
                    TermEvent::Mouse(mouse) => {
                        let action = handle_mouse(session, mouse);
                        if action == Action::Copy {
                            copy_selection(terminal, session)?;
                        }
                    }
                    _ => {}
                }
            }
        }

        // Drained for the same reason the turn's own loop drains, and waiting on the terminal
        // for the same reason it does: a person may be typing their next prompt while this runs.
        let carrying_on = drain_worker(&from_worker, Duration::ZERO, |message| match message {
            crate::remote_confirm::ToMain::Phase(phase) => session.set_phase(phase),
            crate::remote_confirm::ToMain::Narration(text) => session.narrate(text),
            crate::remote_confirm::ToMain::Streaming(text) => session.streaming(&text),
            _ => {}
        });

        if !carrying_on {
            break;
        }
    }

    let (done, conversation, sink) = worker.join().unwrap_or_else(|_| {
        (
            Err(t!(compact_ended_unexpectedly).to_string()),
            Conversation::new(),
            Trail::new(),
        )
    });

    match &done {
        Ok(Some(summary)) => {
            session.end_aside(summary.usage.total());
            session.measured(conversation.last_request_tokens(), config.context_budget);
            session.note(t!(
                compact_done,
                summarised = summary.summarised,
                kept = summary.kept
            ));
        }
        Ok(None) => {
            session.end_aside(0);
            session.note(t!(compact_nothing_to_do));
        }
        Err(message) => {
            session.end_aside(0);
            session.note(t!(compact_failed, problem = message));
        }
    }

    Ok((conversation, sink.events().to_vec()))
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
    programs: TrustedPrograms,
    permissions: &Permissions,
) -> io::Result<(Conversation, TrustStore, TrustedPrograms, Vec<Stamped>)> {
    // The prompt is in the transcript by now, and drawn before anything that might take a moment:
    // a check that has to run the AWS CLI holds the frame for as long as the process takes, and
    // until this the line somebody typed is nowhere on their screen.
    redraw(terminal, session)?;

    // Before the worker starts, because a sign-in needs the terminal and this is the thread that
    // has it. Left to the worker, the URL and code the AWS CLI prints would land in a frame this
    // loop redraws over.
    sign_in_if_needed(terminal, session, config)?;

    // One channel for everything the worker sends, because the main thread waits on exactly one
    // thing and `mpsc` cannot select across two. Only a write expects a reply.
    let (to_main, from_worker) = mpsc::channel::<crate::remote_confirm::ToMain>();
    let (answer_tx, answer_rx) = mpsc::channel::<crate::remote_confirm::Reply>();
    // Prompts typed while this turn runs, going the other way. Shared rather than sent, so that
    // taking one back off the queue takes it out of the turn's reach too.
    let typed = session.interjections();

    // A fresh token per turn: reusing one could cancel a turn before it started.
    let cancel = Cancel::new();
    let worker_cancel = cancel.clone();

    // Cloned rather than borrowed so the worker owns everything it needs. Config and Workspace
    // are cheap handles; Egress builds its own connection pool.
    let worker_config = config.clone();
    let worker_workspace = workspace.clone();
    // Every file named with `@` becomes context, which a turn treats as trusted: the user typed the
    // path and their keystroke is what vouches for it, exactly as `--file` does on the command
    // line. Read back out of the prompt rather than tracked while it is typed, so the line that was
    // sent and the files that came with it cannot disagree.
    // A long paste sits in the line behind a marker, and this is where the marker is put back to
    // the words it stands for. Folding one is a way of drawing a line, not a way of sending one:
    // the planner is given what pasting into the box has always given it, and everything the user
    // looks at keeps the short form.
    let sent = session.unfolded(prompt);
    // No round limit: the person reading this screen is the bound, and a stop reaches a turn
    // mid-round. A number here would only interrupt work that was going fine.
    // Which tick of a loop this is, where it is one at all. A prompt the person typed in the
    // middle of a loop is not a tick of it and carries nothing.
    let tick = session.looping().and_then(|running| running.tick());
    let mut task = Task::new(&sent)
        .with_rounds(None)
        .with_home(bravebot_agent::home::directory())
        .with_model(session.model().map(str::to_string))
        .with_permissions(permissions.clone())
        .ticking(tick);
    for file in crate::entries::referenced(&sent) {
        task = task.with_file(file);
    }
    // Dropped files, read back out of the line the same way and for the same reason: a marker the
    // user deleted is an attachment they took off.
    for attached in session.sent_attachments().to_vec() {
        task = match attached.kind {
            crate::dropped::Kind::Attachment(media) => task.with_attachment(attached.name, media),
            // A text file is context, which is what `@` and `--file` already do with one. It
            // goes in as a drop rather than as a named file because a drop comes from wherever
            // the user dragged it from, and that is rarely inside the workspace.
            crate::dropped::Kind::Text => task.with_dropped_text(attached.name),
        };
    }
    // Pasted pictures, in the order the markers in the prompt number them. A model reading
    // "[Image #2]" has to be able to count to the picture that answers it.
    for image in session.sent_pasted() {
        task = task.with_image(PastedImage {
            media_type: image.media_type.to_string(),
            bytes: image.bytes.clone(),
        });
    }
    let task = task;
    // Kept so a failed turn does not lose the user's decisions. Both of them: a run approved
    // "always" in a turn that then failed is still an answer the user gave.
    let fallback = trust.clone();
    let fallback_programs = programs.clone();

    let worker = thread::spawn(move || {
        let mut sink = Trail::new();
        // Two handles over one channel back to the thread that owns the terminal: one asks about
        // writes and waits, the other reports progress and moves on.
        let mut reporter = crate::remote_confirm::RemoteReporter::new(to_main.clone());
        let mut confirmer = crate::remote_confirm::RemoteConfirmer::new(to_main, answer_rx, typed);
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
            programs,
            &worker_cancel,
        );
        (outcome, conversation, sink)
    });

    // Redraw until the turn finishes, answering approvals and watching for a cancel on the way.
    loop {
        redraw(terminal, session)?;

        // Input is polled here rather than in the outer loop, which is blocked for the duration
        // of the turn. Without this the interface would take none at all while working, and a
        // long turn is exactly when someone wants to copy what has appeared so far.
        //
        // This is also where the frame's waiting is done, and that is the point of it. Blocking on
        // the worker instead left a key press sitting for up to a frame before anything looked at
        // it, so typing during a turn lagged while typing between turns did not, for no reason a
        // person could see. A keystroke wakes this the instant it arrives; the worker's messages
        // are picked up on the way round and drawn at the frame rate, which is all they need.
        //
        // Everything waiting, not one event per pass: a drag read one event at a time would take
        // seconds to catch up with the pointer.
        if event::poll(FRAME)? {
            while event::poll(Duration::ZERO)? {
                match event::read()? {
                    // Presses only, for the reason the outer loop ignores releases: a release taken
                    // for a press would type every character twice, and cancel the turn on the way up
                    // from the Escape that already cancelled it.
                    TermEvent::Key(key) if key.kind == KeyEventKind::Release => {}
                    // Both keys stop the turn and neither leaves. Ctrl-C is the way out of the
                    // program, but there is a turn to stop first, and a person watching an answer
                    // go wrong is asking for the answer to stop rather than for the session to
                    // end. The next press, at the box, is the one that leaves.
                    //
                    // Nothing is said about stopping. The stop is the prompt coming back to the
                    // box a moment later, which is both the answer and what the person wanted;
                    // a line saying "cancelling…" is a progress report on a key press.
                    TermEvent::Key(key) if stops_the_turn(session, key) => {
                        cancel.cancel();
                    }
                    TermEvent::Key(key) => {
                        handle_key_while_working(session, key);
                    }
                    TermEvent::Paste(text) => handle_paste_while_working(session, &text),
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
        }

        let carrying_on = drain_worker(&from_worker, Duration::ZERO, |message| match message {
            crate::remote_confirm::ToMain::Write(request) => {
                let answer = crate::confirm::ask(terminal, &request);
                // Ctrl-C at the prompt is the same request it is anywhere else in a turn: stop.
                // Set before the answer goes back, so the worker sees it as soon as it wakes.
                if answer == crate::confirm::Answer::Interrupt {
                    cancel.cancel();
                }
                // A closed channel means the worker is already gone, so there is nothing to
                // answer and the loop below will collect its result.
                let _ = answer_tx.send(crate::remote_confirm::Reply::Write(answer.decision()));
            }
            crate::remote_confirm::ToMain::Run(request) => {
                let answer = crate::confirm::ask_run(terminal, &request);
                // Ctrl-C at the prompt is the same request it is anywhere else in a turn: stop.
                // Set before the answer goes back, so the worker sees it as soon as it wakes.
                if answer == crate::confirm::RunAnswer::Interrupt {
                    cancel.cancel();
                }
                // What was vouched for travels back with the turn's outcome, exactly as the
                // trust map does: the tool records it on the policy, and the policy carries it
                // out. Recording it here as well would give the session a second copy to
                // disagree with.
                let _ = answer_tx.send(crate::remote_confirm::Reply::Run(answer.decision()));
            }
            crate::remote_confirm::ToMain::ReadOutput(request) => {
                let answer = crate::confirm::ask_output(terminal, &request);
                if answer == crate::confirm::Answer::Interrupt {
                    cancel.cancel();
                }
                let _ = answer_tx.send(crate::remote_confirm::Reply::ReadOutput(answer.decision()));
            }
            crate::remote_confirm::ToMain::Vouch(request) => {
                let answer = crate::confirm::ask_vouch(terminal, &request);
                if answer == crate::confirm::Answer::Interrupt {
                    cancel.cancel();
                }
                if answer == crate::confirm::Answer::Approve {
                    // Said on the transcript because it is a standing decision the user will not
                    // otherwise see recorded anywhere until they ask for /status.
                    session.note(t!(session_vouched_for, path = &request.path));
                }
                let _ = answer_tx.send(crate::remote_confirm::Reply::Vouch(answer.decision()));
            }
            crate::remote_confirm::ToMain::Ask(asking) => {
                // A planner that loops back over the same decision should not make the user
                // restate it. The note is what keeps that from being invisible: an answer given
                // once and reused silently would look like a question that was never asked.
                let known: Vec<Option<bravebot_core::ask::Answer>> = asking
                    .prompts
                    .iter()
                    .map(|prompt| session.recall_answer(&prompt.key))
                    .collect();
                for (prompt, earlier) in asking.prompts.iter().zip(&known) {
                    if earlier.is_some() {
                        session.note(t!(session_answered_already, question = &prompt.question));
                    }
                }

                // Only what is still outstanding is drawn, so the count in the title is the
                // number of questions the person actually has to answer.
                let outstanding = bravebot_core::ask::Asking {
                    prompts: asking
                        .prompts
                        .iter()
                        .zip(&known)
                        .filter(|(_, earlier)| earlier.is_none())
                        .map(|(prompt, _)| prompt.clone())
                        .collect(),
                };
                let fresh = crate::ask::ask(terminal, &outstanding);

                let answers = crate::ask::in_order(known, fresh);
                for (prompt, answer) in asking.prompts.iter().zip(&answers) {
                    session.remember_answer(prompt.key.clone(), answer.clone());
                }
                let _ = answer_tx.send(crate::remote_confirm::Reply::Ask(answers));
            }
            // No reply: each of these is recorded and the next redraw, one iteration away,
            // shows it. That is what makes a long turn legible while it runs.
            crate::remote_confirm::ToMain::Todos(rows) => session.set_todos(rows),
            crate::remote_confirm::ToMain::Written(written) => session.set_written(written),
            crate::remote_confirm::ToMain::Phase(phase) => session.set_phase(phase),
            crate::remote_confirm::ToMain::Narration(text) => session.narrate(text),
            crate::remote_confirm::ToMain::Notice(text) => session.note_once(text),
            crate::remote_confirm::ToMain::Streaming(text) => session.streaming(&text),
            crate::remote_confirm::ToMain::Started(activity) => session.start_activity(activity),
            crate::remote_confirm::ToMain::Finished(activity) => session.finish_activity(activity),
            crate::remote_confirm::ToMain::Quarantined(shown) => session.show(shown),
            crate::remote_confirm::ToMain::Landed(landing) => session.landed(landing),
            // The turn has taken the oldest waiting prompt, so it stops being something waiting
            // above the box and becomes something said. Which prompt is not named: the turn takes
            // them in the order they were sent and this end hands them over in that order, so the
            // oldest is the one that has gone.
            crate::remote_confirm::ToMain::Interjected(_) => session.interjected(),
        });

        // The worker dropped its senders, so the turn is over.
        if !carrying_on {
            break;
        }
    }

    let (outcome, conversation, sink) = worker.join().unwrap_or_else(|_| {
        // A panicked turn is reported rather than propagated: the session survives. The
        // conversation does not, since the thread that held it is gone.
        (
            Err(turn::TurnError::Precommit(
                t!(turn_ended_unexpectedly).to_string(),
            )),
            Conversation::new(),
            Trail::new(),
        )
    });

    // A cancelled turn returns the prompt for editing instead of recording a failure: the user
    // stopped it deliberately, so there is nothing to report.
    let events = sink.events().to_vec();

    if matches!(outcome, Err(turn::TurnError::Cancelled)) {
        // Not when the cancel was somebody leaving. Restoring returns the session to idle, which
        // would put it back in the loop it was on its way out of, and hand back a prompt to a box
        // nobody is going to see.
        if session.is_quitting() {
            return Ok((conversation, fallback, fallback_programs, events));
        }
        session.restore(prompt);
        // What was lined up behind it stays lined up, and the loop sends the next one as it does
        // after any turn. A stop is aimed at the turn in flight: the prompts behind it are ones
        // the person typed and has not taken back, and throwing them away made stopping a turn
        // that had gone wrong cost every prompt they had queued while it did.
        return Ok((conversation, fallback, fallback_programs, events));
    }

    // Whether a difference between the model asked for and the one reported means anything is the
    // backend's question, and so is which name the service was actually asked for: a gateway is
    // asked for the part of a qualified name that it knows the model by. Answered here, where the
    // configuration is in hand.
    let chosen = session.model().unwrap_or(&config.default_model);
    let asked = Asked {
        name: bravebot_agent::backend::Backend::name_as_asked(config, chosen),
        comparable: bravebot_agent::backend::Backend::reports_the_model_it_was_asked_for(
            config, chosen,
        ),
    };
    let (trust, programs) = fold_outcome(
        session,
        outcome,
        sink,
        fallback,
        fallback_programs,
        config.context_budget,
        asked,
    );
    Ok((conversation, trust, programs, events))
}

/// Hand the worker's messages to `handle`: the one this frame waited for, and then everything
/// already queued behind it.
///
/// `wait` is how long to block for the first, and a caller that is already blocking on something
/// else passes nothing. Which one a loop waits on decides how quickly it answers a key press, so
/// it belongs to the caller rather than here.
///
/// `false` when the worker has dropped its senders and the turn is over.
///
/// Drained rather than taken one per pass, for the same reason terminal events are. A reply
/// arrives as hundreds of small messages, a draw rebuilds the whole transcript from its markdown,
/// and a frame for each spent longer laying out finished turns than the reply took to arrive: the
/// queue outran the drawing and what was on the screen fell behind what had been said.
fn drain_worker(
    from_worker: &mpsc::Receiver<crate::remote_confirm::ToMain>,
    wait: Duration,
    mut handle: impl FnMut(crate::remote_confirm::ToMain),
) -> bool {
    let mut received = from_worker.recv_timeout(wait);
    loop {
        match received {
            Ok(message) => handle(message),
            Err(mpsc::RecvTimeoutError::Timeout) => return true,
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
        }

        // Whatever else is waiting, without waiting for it. Empty means the burst is over and the
        // next frame shows all of it at once.
        received = from_worker.try_recv().map_err(|gone| match gone {
            mpsc::TryRecvError::Empty => mpsc::RecvTimeoutError::Timeout,
            mpsc::TryRecvError::Disconnected => mpsc::RecvTimeoutError::Disconnected,
        });
    }
}

/// Whether a key press asks for whatever is in flight to stop, and nothing more.
///
/// Escape, and only Escape. Ctrl-C asks for it too, but Ctrl-C also leaves, so the loops take it
/// separately: which of the two it means depends on whether there is anything to stop.
fn wants_cancel(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Esc)
}

/// Whether a key press is Ctrl-C.
///
/// What it asks for depends on what is happening. With a turn in flight, or a command running, it
/// stops that and stays; with nothing to stop it leaves. So this says which key was pressed and
/// the loops say what it meant, since only they know which of the two they are.
///
/// It once always left, because stopping the turn and staying had left no way out at all: Ctrl-C
/// did nothing whatever at the prompt. It leaves from the prompt now, so the press that stops a
/// turn is followed by a press that leaves, and both requests have a key again.
fn is_ctrl_c(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c'))
}

/// What the last turn asked its backend for, for comparing against what answered.
///
/// The two travel together because either alone is misleading: a name means nothing without knowing
/// whether it is comparable to a reply at all.
struct Asked {
    /// The name the service was actually sent, which for a gateway is not the name a session holds.
    name: String,
    /// Whether that name and the one a reply reports are drawn from one roster.
    comparable: bool,
}

/// Fold a finished turn into the session.
fn fold_outcome(
    session: &mut Session,
    outcome: Result<turn::Outcome, turn::TurnError>,
    sink: Trail,
    fallback: TrustStore,
    fallback_programs: TrustedPrograms,
    budget: u64,
    asked: Asked,
) -> (TrustStore, TrustedPrograms) {
    match outcome {
        Ok(outcome) => {
            let trail = sink.bare();
            session.complete(
                outcome.reply_for_display().to_string(),
                trail,
                outcome.tokens,
            );
            if !outcome.clean {
                session.note(t!(session_something_was_refused));
            }
            // Where the wall clock went, beside what the turn cost. The wall figure is the
            // session's own, taken from the moment Enter was pressed; this fills in the parts, which
            // only the worker saw.
            session.spent_time(outcome.timing);

            // What the turn's last request came to, against what it would be compacted at. Not
            // the same figure as the cost above: that adds every round together, this says how
            // full the context is now.
            session.measured(outcome.context_tokens, budget);

            // What was asked for against what answered. The endpoint substitutes rather than
            // refusing: a premium model requested without a credential comes back as whatever the
            // free tier serves, with a 200 and a perfectly ordinary reply. So the only trace is
            // this field, and a session that never compares them cannot tell a model it chose from
            // one chosen for it.
            //
            // Said only when they differ, and only when the difference is new, since it would
            // otherwise be a line on every turn for the rest of the session.
            let already = session.substituted_model().is_some();
            session.served(
                session.model().map(|_| asked.name),
                outcome.model.clone(),
                outcome.premium,
                asked.comparable,
            );
            if !already && let Some(asked) = session.substituted_model() {
                session.note(t!(
                    session_model_substituted,
                    asked = asked,
                    served = &outcome.model
                ));
            }
            // Where the turn was a tick, this is what arms the next one: an interval from the
            // driver's own clock, or the wait the turn asked for. Measured from here rather than
            // from when the tick went out, so the gap is between runs and a turn that outlasts
            // its own interval is not immediately due again.
            session.loop_turn_ended(outcome.wakeup);

            // Carries forward any rule the turn recorded, so a path that received untrusted
            // data cannot be read back as trusted by the next turn, and any program the user
            // vouched for during it, so they are not asked about it again.
            (outcome.trust, outcome.programs)
        }
        Err(error) => {
            // Stopping a turn stops the loop it was part of, and stops one the person was not
            // part way through: the key means "stop what is happening", and a schedule that
            // survived it would send the next prompt as though nothing had been said.
            if matches!(error, turn::TurnError::Cancelled) {
                session.stop_loop();
            } else {
                // Any other failure is a tick that ended, and a self-paced loop that was told
                // nothing falls back to the driver's own wait. A loop must not end because one
                // request failed, and must not run forever on failures either.
                session.loop_turn_ended(None);
            }

            // The trail is kept on failure too: a refusal is exactly when a user wants
            // to see what happened.
            let trail = sink
                .events()
                .iter()
                .map(|stamped| crate::audit::as_line(&stamped.event))
                .collect();
            session.fail(t!(session_error, problem = error));
            if let Some(last) = session.transcript.last_mut() {
                last.trail = trail;
            }
            (fallback, fallback_programs)
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

    fn listed(key: &str, window: Option<u64>) -> bravebot_aichat::models::Model {
        bravebot_aichat::models::Model {
            key: key.to_string(),
            display_name: key.to_string(),
            premium: false,
            provider: None,
            conversation_tokens: window,
        }
    }

    fn bedrock_configured(pairs: &[(&str, &str)]) -> bravebot_config::bedrock::Bedrock {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        bravebot_config::bedrock::Bedrock::from_lookup(|name| {
            owned
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        })
        .expect("configured")
    }

    /// The picker offers what was configured, under the tier names a person recognises rather than
    /// the ARNs they were given as. An ARN is unreadable and identical-looking between tiers.
    #[test]
    fn the_bedrock_picker_offers_the_configured_tiers_by_name() {
        use bravebot_config::env_var;

        let models = bedrock_models(&bedrock_configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
            (env_var::BEDROCK_OPUS_MODEL, "opus-arn"),
            (env_var::BEDROCK_SONNET_MODEL, "sonnet-arn"),
            (env_var::BEDROCK_HAIKU_MODEL, "haiku-arn"),
        ]));

        let shown: Vec<&str> = models.iter().map(|m| m.display_name.as_str()).collect();
        assert_eq!(shown, ["Opus", "Sonnet", "Haiku"]);
        // The key is what lands in the request's model field, and it has to be the ARN.
        assert_eq!(models[0].key, "opus-arn");
    }

    /// Naming the service does not distinguish the row: Brave serves some of its own roster through
    /// Bedrock and says so in the display name it sends, so "(Bedrock)" appeared on both halves of the
    /// picker and told a person nothing about which account was about to be billed.
    #[test]
    fn a_configured_tier_is_not_confusable_with_a_brave_model_served_through_bedrock() {
        use bravebot_config::env_var;

        let mut brave = listed("gpt-5.5", Some(102_400));
        brave.display_name = "GPT-5.5 (Bedrock)".to_string();

        let roster = combined(
            bedrock_models(&bedrock_configured(&[
                (env_var::USE_BEDROCK, "1"),
                (env_var::AWS_REGION, "us-west-2"),
                (env_var::AWS_PROFILE, "some-profile"),
                (env_var::BEDROCK_SONNET_MODEL, "sonnet-arn"),
            ])),
            Ok(vec![brave]),
        )
        .expect("a roster");

        assert_eq!(roster.len(), 2);
        // The profile is what decides which credentials sign, and no name off the wire carries it.
        let configured = roster[0].provider.as_deref().expect("a service");
        assert!(configured.contains("some-profile"), "{configured}");
        assert_eq!(
            roster[1].provider, None,
            "the Brave roster names no service"
        );
    }

    /// A profile is optional, and a row still has to say the tier is reached through the person's own
    /// account rather than through Brave's.
    #[test]
    fn a_tier_with_no_profile_configured_still_names_the_account() {
        use bravebot_config::env_var;

        let models = bedrock_models(&bedrock_configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
            (env_var::BEDROCK_OPUS_MODEL, "opus-arn"),
        ]));
        assert_eq!(models[0].display_name, "Opus");
        assert_eq!(
            models[0].provider.as_deref(),
            Some(t!(picker_service_bedrock))
        );
    }

    /// The same condition Claude Code applies: a tier appears only when its variable names a model.
    /// An entry invented for an unset tier is a choice that fails at the far end.
    #[test]
    fn a_tier_with_no_model_configured_is_not_offered() {
        use bravebot_config::env_var;

        let models = bedrock_models(&bedrock_configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
            (env_var::BEDROCK_SONNET_MODEL, "sonnet-arn"),
        ]));

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].display_name, "Sonnet");
    }

    /// A settings block adds to the roster rather than replacing it. Replacing it left a person who
    /// configured one tier with a picker offering exactly one model, and no way back to the Brave
    /// models every build has.
    #[test]
    fn configured_tiers_are_offered_alongside_the_brave_roster() {
        use bravebot_config::env_var;

        let configured = bedrock_models(&bedrock_configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
            (env_var::BEDROCK_OPUS_MODEL, "opus-arn"),
        ]));
        let roster = combined(
            configured,
            Ok(vec![
                bravebot_aichat::models::Model::automatic(),
                listed("claude-sonnet", Some(102_400)),
            ]),
        )
        .expect("a roster");

        let keys: Vec<&str> = roster.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            ["opus-arn", bravebot_config::DEFAULT_MODEL, "claude-sonnet"]
        );
    }

    /// The tiers need no network to know. Losing them because the other half was unreachable would
    /// leave the only models this configuration can definitely reach unpickable.
    #[test]
    fn an_unreachable_listing_still_offers_the_configured_tiers() {
        use bravebot_config::env_var;

        let configured = bedrock_models(&bedrock_configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
            (env_var::BEDROCK_OPUS_MODEL, "opus-arn"),
        ]));
        let roster = combined(configured, Err("the endpoint is unreachable".into()))
            .expect("the tiers survive");
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].key, "opus-arn");
    }

    /// With nothing configured there is nothing to fall back to, and a picker showing an empty list
    /// would read as a backend with no models rather than as a listing that failed.
    #[test]
    fn an_unreachable_listing_with_no_tiers_configured_is_still_a_failure() {
        assert!(combined(vec![], Err("the endpoint is unreachable".into())).is_err());
    }

    /// `automatic` means "let the server choose", which Bedrock does not offer: a request names one
    /// model and gets it or an error. It reaches the picker from the Brave half instead.
    #[test]
    fn the_bedrock_picker_does_not_offer_automatic() {
        use bravebot_config::env_var;

        let models = bedrock_models(&bedrock_configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
            (env_var::BEDROCK_OPUS_MODEL, "opus-arn"),
        ]));
        assert!(!models.iter().any(|model| model.is_automatic()));
    }

    /// Premium means a Leo subscription. Reaching a model through somebody's own AWS account does
    /// not involve one, and marking it premium would ask them to import a subscription to use what
    /// they already pay for.
    #[test]
    fn bedrock_models_are_not_marked_premium() {
        use bravebot_config::env_var;

        let models = bedrock_models(&bedrock_configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
            (env_var::BEDROCK_OPUS_MODEL, "opus-arn"),
        ]));
        assert!(!models[0].premium);
    }

    /// The budget lookup shares this listing, so a Bedrock entry has to carry a window or a session
    /// would fall back to the default and compact five times sooner than it had to.
    #[test]
    fn a_bedrock_entry_carries_the_window_the_budget_is_taken_from() {
        use bravebot_config::env_var;

        let models = bedrock_models(&bedrock_configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
            (env_var::BEDROCK_OPUS_MODEL, "opus-arn"),
        ]));
        assert_eq!(
            advertised_window(&models, Some("opus-arn")),
            Some(bravebot_config::bedrock::CONTEXT_WINDOW)
        );
    }

    /// A gateway configured by a `provider` block, for the picker tests below.
    fn gateway(models: &str) -> bravebot_config::provider::Provider {
        let text = format!(
            r#"{{"provider": {{"openrouter": {{
                "options": {{"baseURL": "https://openrouter.example.invalid/api/v1"}},
                "models": {models}
            }}}}}}"#
        );
        let serde_json::Value::Object(root) = serde_json::from_str(&text).expect("json") else {
            panic!("not an object");
        };
        bravebot_config::provider::Provider::all(&root)
            .pop()
            .expect("one provider")
    }

    /// A gateway's roster is what the file named and nothing else. Asking the gateway would list
    /// hundreds of models across upstreams nobody configured, and would cost a round trip on a path
    /// that has to work offline.
    ///
    /// Each key is qualified by the provider's own id, which is what makes a remembered choice say
    /// which service it was for.
    #[test]
    fn only_the_gateway_models_the_file_named_are_offered() {
        let models = provider_models(&gateway(r#"{"z-ai/glm-4.6": {}, "moonshot/kimi-k2": {}}"#));
        let keys: Vec<&str> = models.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            ["openrouter/moonshot/kimi-k2", "openrouter/z-ai/glm-4.6"]
        );
    }

    /// The same model may be reachable more than one way, billed and credentialled differently, and
    /// nothing else on the row would say which was about to answer.
    #[test]
    fn a_gateway_row_says_which_service_answers_it() {
        let models = provider_models(&gateway(r#"{"anthropic/claude-sonnet-4.5": {}}"#));
        assert_eq!(models[0].display_name, "anthropic/claude-sonnet-4.5");
        assert_eq!(models[0].provider.as_deref(), Some("openrouter"));
    }

    /// Premium means a Leo subscription. A gateway reached with somebody's own bearer token does not
    /// involve one, and marking it premium would ask them to import a subscription to use what they
    /// already pay for.
    #[test]
    fn gateway_models_are_not_marked_premium() {
        let models = provider_models(&gateway(r#"{"z-ai/glm-4.6": {}}"#));
        assert!(!models[0].premium);
    }

    /// The budget lookup shares this listing, so an entry has to carry a window. A model that stated
    /// one gets it, and the figure has to survive reaching the picker or the session compacts against
    /// a window belonging to a different service.
    #[test]
    fn a_gateway_entry_carries_the_window_the_budget_is_taken_from() {
        let models = provider_models(&gateway(
            r#"{"anthropic/claude-sonnet-4.5": {"limit": {"context": 1000000, "output": 64000}}}"#,
        ));
        assert_eq!(
            advertised_window(&models, Some("openrouter/anthropic/claude-sonnet-4.5")),
            Some(1_000_000)
        );
    }

    /// A window nobody stated is the conservative default rather than nothing at all. Reporting
    /// nothing would leave the budget on a figure chosen for a different service, and a budget above
    /// the real window does not delay compaction but removes it.
    #[test]
    fn a_gateway_model_with_no_stated_window_still_carries_one() {
        let models = provider_models(&gateway(r#"{"z-ai/glm-4.6": {}}"#));
        assert_eq!(
            advertised_window(&models, Some("openrouter/z-ai/glm-4.6")),
            Some(bravebot_config::provider::CONTEXT_WINDOW)
        );
    }

    /// A gateway is additive on the same terms Bedrock is: its models are offered beside the others
    /// rather than in place of them, so nothing a person could reach before stops being reachable.
    #[test]
    fn gateway_models_are_offered_alongside_the_other_rosters() {
        use bravebot_config::env_var;

        let mut configured = bedrock_models(&bedrock_configured(&[
            (env_var::USE_BEDROCK, "1"),
            (env_var::AWS_REGION, "us-west-2"),
            (env_var::BEDROCK_OPUS_MODEL, "opus-arn"),
        ]));
        configured.extend(provider_models(&gateway(r#"{"z-ai/glm-4.6": {}}"#)));

        let roster = combined(
            configured,
            Ok(vec![bravebot_aichat::models::Model::automatic()]),
        )
        .expect("a roster");
        let keys: Vec<&str> = roster.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "opus-arn",
                "openrouter/z-ai/glm-4.6",
                bravebot_config::DEFAULT_MODEL
            ]
        );
    }

    /// A fetched roster arrives roughly newest-first, which puts a model nobody has heard of at the
    /// top and buries the one they work with. The model in force leads, and the rest sort by name so
    /// a half-remembered one is where somebody would look for it.
    #[test]
    fn a_fetched_roster_leads_with_the_model_in_force() {
        // Last in the gateway's order and last alphabetically, so leading is the only way it gets
        // to the top: a test whose input already led with it would pass against no sorting at all.
        let roster = vec![
            listed("openrouter/anthropic/claude-sonnet-4.5", None),
            listed("openrouter/moonshot/kimi-k2", None),
            listed("openrouter/z-ai/glm-4.6", None),
        ];
        let ordered = in_reading_order(roster, "openrouter/z-ai/glm-4.6");
        let keys: Vec<&str> = ordered.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "openrouter/z-ai/glm-4.6",
                "openrouter/anthropic/claude-sonnet-4.5",
                "openrouter/moonshot/kimi-k2"
            ]
        );
    }

    /// Nothing in force is the ordinary first run. Every row still sorts by name rather than keeping
    /// an order that means nothing to the person reading it.
    #[test]
    fn a_fetched_roster_nobody_has_chosen_from_is_still_sorted() {
        let roster = vec![
            listed("openrouter/z-ai/glm-4.6", None),
            listed("openrouter/anthropic/claude-sonnet-4.5", None),
        ];
        let ordered = in_reading_order(roster, bravebot_config::DEFAULT_MODEL);
        let keys: Vec<&str> = ordered.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "openrouter/anthropic/claude-sonnet-4.5",
                "openrouter/z-ai/glm-4.6"
            ]
        );
    }

    /// A gateway's models need no network to know, so a Brave listing that could not be fetched must
    /// not withdraw them: that is the position somebody offline is most likely to be in.
    #[test]
    fn an_unreachable_listing_still_offers_the_gateway_models() {
        let configured = provider_models(&gateway(r#"{"z-ai/glm-4.6": {}}"#));
        let roster = combined(configured, Err("the endpoint is unreachable".into()))
            .expect("the gateway models survive");
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].key, "openrouter/z-ai/glm-4.6");
    }

    /// A model chosen in an earlier session is read back off disk, and the window that came with it
    /// is not: it is reported by the listing and nowhere else. Until this was looked up, a session
    /// with room for a hundred thousand tokens compacted at twenty-four thousand.
    #[test]
    fn the_window_of_a_model_chosen_earlier_is_found_in_the_listing() {
        let models = [
            listed("claude-opus", Some(102_400)),
            listed("some-other-model", Some(8_000)),
        ];
        assert_eq!(
            advertised_window(&models, Some("claude-opus")),
            Some(102_400)
        );
    }

    /// Nothing chosen is `automatic`, whose model is resolved per request, so no entry's window is
    /// the one in force.
    #[test]
    fn nothing_chosen_has_no_advertised_window() {
        let models = [listed("claude-opus", Some(102_400))];
        assert_eq!(advertised_window(&models, None), None);
    }

    /// A model that has been withdrawn since it was chosen. The default stands rather than the
    /// window of whichever entry happened to be first.
    #[test]
    fn a_model_the_listing_no_longer_offers_has_no_window() {
        let models = [listed("claude-opus", Some(102_400))];
        assert_eq!(advertised_window(&models, Some("withdrawn-model")), None);
    }

    /// An entry that reports no window of its own leaves the budget alone.
    #[test]
    fn a_model_that_advertises_nothing_has_no_window() {
        let models = [listed("quiet-model", None)];
        assert_eq!(advertised_window(&models, Some("quiet-model")), None);
    }

    /// A reply arrives as hundreds of messages and a draw rebuilds the whole transcript, so a
    /// frame per message put the drawing behind the talking. Everything queued is taken before
    /// the caller draws again, which is what keeps one frame's worth of reply to one frame.
    #[test]
    fn everything_the_worker_has_already_said_is_taken_in_one_pass() {
        let (outbound, inbound) = std::sync::mpsc::channel();
        for piece in ["a", "b", "c", "d"] {
            outbound
                .send(crate::remote_confirm::ToMain::Streaming(piece.to_string()))
                .expect("queued");
        }

        let mut taken = Vec::new();
        let carrying_on = drain_worker(&inbound, FRAME, |message| {
            if let crate::remote_confirm::ToMain::Streaming(text) = message {
                taken.push(text);
            }
        });

        assert!(carrying_on, "the worker is still there");
        assert_eq!(
            taken,
            vec!["a", "b", "c", "d"],
            "the burst took several passes"
        );
    }

    /// The turn is over when the worker lets go of its senders, and the loop has to notice that
    /// rather than waiting a frame at a time forever.
    #[test]
    fn a_worker_that_has_gone_ends_the_wait() {
        let (outbound, inbound) = std::sync::mpsc::channel();
        outbound
            .send(crate::remote_confirm::ToMain::Streaming(
                "last words".into(),
            ))
            .expect("queued");
        drop(outbound);

        let mut taken = Vec::new();
        let carrying_on = drain_worker(&inbound, FRAME, |message| taken.push(message));

        assert_eq!(
            taken.len(),
            1,
            "what was already said was dropped on the floor"
        );
        assert!(!carrying_on, "the loop would have gone on waiting");
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    mod scroller {
        use super::*;
        use crate::state::Laid;

        /// A session with a transcript already laid out, so the keys have rows to move over.
        /// The numbers are the ones a frame would have written back after drawing.
        fn reading() -> Session {
            let mut session = Session::new("kernel-enforced");
            session.note_layout(Laid {
                width: 80,
                height: 10,
                rows: 100,
                prompts: vec![0, 30, 60],
                matches: Vec::new(),
            });
            session
        }

        fn opened() -> Session {
            let mut session = reading();
            session.open_scroller();
            session
        }

        #[test]
        fn ctrl_o_opens_the_scroller() {
            let mut session = reading();
            assert!(!session.scrolling());

            handle_key(&mut session, ctrl('o'));

            assert!(session.scrolling(), "the key did not open it");
        }

        /// A viewer that jumps somewhere on the way in has lost the thing the person opened it to
        /// look at. The scroller reads the offset the wheel writes, so there is nothing to jump.
        #[test]
        fn the_scroller_opens_on_the_view_that_was_already_there() {
            let mut session = reading();
            session.scroll_up(7);

            handle_key(&mut session, ctrl('o'));

            assert_eq!(session.scroll, 7, "opening it moved the view");
        }

        #[test]
        fn q_escape_and_ctrl_o_each_close_the_scroller() {
            for closing in [key(KeyCode::Char('q')), key(KeyCode::Esc), ctrl('o')] {
                let mut session = opened();
                handle_key(&mut session, closing);
                assert!(!session.scrolling(), "{closing:?} did not close it");
            }
        }

        #[test]
        fn closing_the_scroller_leaves_the_view_where_it_was() {
            let mut session = opened();
            handle_key(&mut session, key(KeyCode::Char('k')));
            handle_key(&mut session, key(KeyCode::Char('k')));
            let looking_at = session.scroll;

            handle_key(&mut session, key(KeyCode::Char('q')));

            assert_eq!(session.scroll, looking_at, "closing it moved the view");
        }

        /// The scroller is the nearest thing there is to stop, so the press that stops it is not
        /// also the press that stops the turn. Each rung of that ladder is visible, and this one
        /// is the innermost.
        #[test]
        fn ctrl_c_closes_the_scroller_before_it_reaches_anything_else() {
            let mut session = opened();
            session.status = Status::Working;

            assert_eq!(handle_key(&mut session, ctrl('c')), Action::Redraw);
            assert!(!session.scrolling(), "the scroller stayed open");
            assert_eq!(
                session.status,
                Status::Working,
                "the turn was stopped by the press that closed the scroller"
            );

            assert_eq!(
                handle_key(&mut session, ctrl('c')),
                Action::Cancel,
                "the next press did not reach the turn"
            );
        }

        /// Reading back through a turn that is going wrong is reading precisely because it is
        /// going wrong. Nothing in the mode sends anything, so there is nothing here for a
        /// running turn to refuse.
        #[test]
        fn a_turn_goes_on_running_while_the_scroller_is_open() {
            let mut session = reading();
            session.status = Status::Working;

            handle_key_while_working(&mut session, ctrl('o'));
            assert!(session.scrolling(), "the key did not open it mid-turn");

            for pressed in [key(KeyCode::Char('k')), key(KeyCode::Char('g'))] {
                assert_eq!(
                    handle_key_while_working(&mut session, pressed),
                    Action::Redraw
                );
            }

            assert_eq!(session.status, Status::Working, "the turn was stopped");
            assert!(session.scrolling(), "the scroller closed on its own");
        }

        /// A mode that leaks its keystrokes into a box nobody can see is the worse half of both:
        /// `j` would scroll and also type a `j`, and the only way to find out would be to close
        /// the scroller and look.
        /// The loops answer the stop keys before anything else does, so the guard that lets the
        /// scroller have them first is the whole of what keeps the key that closes it from
        /// ending the turn the person opened it to read.
        #[test]
        fn the_scroller_answers_the_stop_keys_before_the_turn_does() {
            let mut session = reading();
            session.status = Status::Working;

            for stopping in [ctrl('c'), key(KeyCode::Esc)] {
                assert!(
                    stops_the_turn(&session, stopping),
                    "{stopping:?} did not reach the turn with nothing in the way"
                );
            }

            session.open_scroller();
            for stopping in [ctrl('c'), key(KeyCode::Esc)] {
                assert!(
                    !stops_the_turn(&session, stopping),
                    "{stopping:?} stopped the turn from inside the scroller"
                );
            }
        }

        #[test]
        fn a_typed_character_does_not_reach_the_box_while_the_scroller_is_open() {
            let mut session = opened();
            for c in "jkgbnv".chars() {
                handle_key(&mut session, key(KeyCode::Char(c)));
            }

            assert!(
                session.input().is_empty(),
                "the keys reached the box: {:?}",
                session.input()
            );
        }

        #[test]
        fn the_line_comes_back_untouched_when_the_scroller_closes() {
            let mut session = reading();
            for c in "half a thought".chars() {
                handle_key(&mut session, key(KeyCode::Char(c)));
            }
            handle_key(&mut session, key(KeyCode::Left));

            handle_key(&mut session, ctrl('o'));
            for c in "jkG{}".chars() {
                handle_key(&mut session, key(KeyCode::Char(c)));
            }
            handle_key(&mut session, key(KeyCode::Char('q')));

            assert_eq!(session.input(), "half a thought");

            // The caret is where it was left, which is the half of the line's state that is not
            // in the text: typing here has to land where it would have landed.
            handle_key(&mut session, key(KeyCode::Char('!')));
            assert_eq!(session.input(), "half a though!t");
        }

        #[test]
        fn enter_sends_nothing_from_inside_the_scroller() {
            let mut session = opened();
            for c in "a prompt".chars() {
                session.type_char(c);
            }

            assert_eq!(handle_key(&mut session, key(KeyCode::Enter)), Action::None);
            assert_eq!(session.input(), "a prompt", "the line was taken");
        }

        #[test]
        fn a_key_the_scroller_does_not_take_does_nothing() {
            let mut session = opened();
            let before = session.scroll;

            for pressed in [key(KeyCode::Char('z')), key(KeyCode::Tab), ctrl('w')] {
                assert_eq!(
                    handle_key(&mut session, pressed),
                    Action::None,
                    "{pressed:?} did something"
                );
            }

            assert_eq!(session.scroll, before);
            assert!(session.input().is_empty());
        }

        #[test]
        fn the_line_keys_move_the_view_by_a_line() {
            let mut session = opened();

            handle_key(&mut session, key(KeyCode::Up));
            assert_eq!(session.scroll, 1);
            handle_key(&mut session, key(KeyCode::Char('k')));
            assert_eq!(session.scroll, 2);
            handle_key(&mut session, key(KeyCode::Char('j')));
            assert_eq!(session.scroll, 1);
            handle_key(&mut session, key(KeyCode::Down));
            assert_eq!(session.scroll, 0);
        }

        #[test]
        fn the_half_page_keys_move_the_view_by_half_a_screen() {
            let mut session = opened();

            handle_key(&mut session, ctrl('u'));
            assert_eq!(session.scroll, 5);
            handle_key(&mut session, ctrl('d'));
            assert_eq!(session.scroll, 0);
        }

        /// Both dialects, because somebody who knows one spelling should not find the other
        /// typing a letter at them.
        #[test]
        fn the_page_keys_move_the_view_by_a_whole_screen() {
            for (back, on) in [
                (key(KeyCode::Char('b')), key(KeyCode::Char(' '))),
                (ctrl('b'), ctrl('f')),
                (key(KeyCode::PageUp), key(KeyCode::PageDown)),
            ] {
                let mut session = opened();
                handle_key(&mut session, back);
                assert_eq!(session.scroll, 10, "{back:?} did not move a screen");
                handle_key(&mut session, on);
                assert_eq!(session.scroll, 0, "{on:?} did not move a screen");
            }
        }

        #[test]
        fn g_and_shift_g_reach_the_first_row_and_the_last() {
            for (first, last) in [
                (key(KeyCode::Char('g')), key(KeyCode::Char('G'))),
                (ctrl_key(KeyCode::Home), ctrl_key(KeyCode::End)),
            ] {
                let mut session = opened();
                handle_key(&mut session, first);
                assert_eq!(
                    session.top_row(),
                    0,
                    "{first:?} did not reach the first row"
                );
                handle_key(&mut session, last);
                assert_eq!(session.top_row(), 90, "{last:?} did not reach the last");
            }
        }

        /// A prompt is the one thing in a transcript the person wrote themselves, so where these
        /// land is settled by what they typed and by nothing read out of the workspace.
        #[test]
        fn the_prompt_keys_land_on_the_turn_before_and_the_turn_after() {
            let mut session = opened();

            handle_key(&mut session, key(KeyCode::Char('{')));
            assert_eq!(session.top_row(), 60);
            handle_key(&mut session, key(KeyCode::Char('{')));
            assert_eq!(session.top_row(), 30);
            handle_key(&mut session, key(KeyCode::Char('}')));
            assert_eq!(session.top_row(), 60);

            // Past the last prompt there is nowhere further to go but the end of the transcript,
            // which is where somebody pressing the key again is asking to be.
            handle_key(&mut session, key(KeyCode::Char('}')));
            assert_eq!(session.top_row(), 90);
        }

        /// A held key has to come to rest somewhere the next press can move away from. Counting
        /// past the end and back again is a key that does nothing for as long as it was held.
        #[test]
        fn the_view_stops_at_the_first_row_rather_than_scrolling_past_it() {
            let mut session = opened();
            for _ in 0..40 {
                handle_key(&mut session, key(KeyCode::Char('b')));
            }

            assert_eq!(session.top_row(), 0);
            handle_key(&mut session, key(KeyCode::Char('j')));
            assert_eq!(
                session.top_row(),
                1,
                "the view had counted past the first row"
            );
        }

        #[test]
        fn the_view_stops_at_the_last_row_rather_than_scrolling_past_it() {
            let mut session = opened();
            for _ in 0..40 {
                handle_key(&mut session, key(KeyCode::Char(' ')));
            }

            assert_eq!(session.scroll, 0);
            handle_key(&mut session, key(KeyCode::Char('k')));
            assert_eq!(session.scroll, 1, "the view had counted past the last row");
        }

        #[test]
        fn the_wheel_scrolls_the_scroller_as_it_scrolls_the_transcript() {
            let mut session = opened();

            handle_mouse(&mut session, drag(MouseEventKind::ScrollUp, 0, 0));
            assert_eq!(session.scroll, 3);
            handle_mouse(&mut session, drag(MouseEventKind::ScrollDown, 0, 0));
            assert_eq!(session.scroll, 0);

            for _ in 0..40 {
                handle_mouse(&mut session, drag(MouseEventKind::ScrollUp, 0, 0));
            }
            assert_eq!(session.top_row(), 0, "the wheel counted past the first row");
        }

        /// The keys go into the needle while one is being typed, because that is what typing
        /// means. Abandoning it leaves the view where it was rather than where a half-typed
        /// search had reached.
        #[test]
        fn escape_abandons_a_half_typed_search() {
            let mut session = opened();
            session.scroller_back(20);
            let looking_at = session.scroll;

            handle_key(&mut session, key(KeyCode::Char('/')));
            for c in "gjkq".chars() {
                handle_key(&mut session, key(KeyCode::Char(c)));
            }
            assert!(session.typing_a_search(), "a letter was read as a movement");
            assert_eq!(session.scroll, looking_at, "typing moved the view");

            handle_key(&mut session, key(KeyCode::Esc));

            assert!(!session.typing_a_search());
            assert!(
                session.scrolling(),
                "abandoning the search closed the scroller"
            );
            assert_eq!(session.scroll, looking_at);
        }

        /// The nearest thing there is to stop, which is the ladder every other stop key here
        /// walks. Closing the mode to get the highlights off the screen would mean losing the
        /// place somebody had scrolled to in order to undo a search they had finished with.
        #[test]
        fn escape_clears_a_finished_search_before_it_closes_the_scroller() {
            let mut session = opened();
            session.scroller_back(20);
            let looking_at = session.scroll;

            handle_key(&mut session, key(KeyCode::Char('/')));
            for c in "notes".chars() {
                handle_key(&mut session, key(KeyCode::Char(c)));
            }
            handle_key(&mut session, key(KeyCode::Enter));
            assert_eq!(session.needle(), "notes");

            handle_key(&mut session, key(KeyCode::Esc));
            assert_eq!(session.needle(), "", "the search was not cleared");
            assert!(
                session.scrolling(),
                "clearing the search closed the scroller"
            );
            assert_eq!(
                session.scroll, looking_at,
                "clearing the search moved the view"
            );

            handle_key(&mut session, key(KeyCode::Esc));
            assert!(!session.scrolling(), "the next press did not close it");
        }

        /// Backspacing past the start is what the key means when there is nothing left of the
        /// thing it deletes: the search goes, rather than the press doing nothing at all.
        #[test]
        fn backspacing_past_the_start_abandons_the_search() {
            let mut session = opened();
            handle_key(&mut session, key(KeyCode::Char('/')));
            handle_key(&mut session, key(KeyCode::Char('a')));

            handle_key(&mut session, key(KeyCode::Backspace));
            assert!(
                session.typing_a_search(),
                "one character took the whole search"
            );
            handle_key(&mut session, key(KeyCode::Backspace));
            assert!(!session.typing_a_search());
        }

        /// A mode where the letters do nothing and nothing says why is indistinguishable from an
        /// interface that has stopped responding.
        #[test]
        fn the_help_key_lists_the_keys() {
            let mut session = opened();

            handle_key(&mut session, key(KeyCode::Char('?')));
            assert!(session.scroller().expect("open").help);

            // Read instead of the transcript rather than alongside it, so anything at all puts it
            // away and that press is spent doing so.
            handle_key(&mut session, key(KeyCode::Char('j')));
            assert!(!session.scroller().expect("open").help);
            assert_eq!(
                session.scroll, 0,
                "the press that put the list away also moved"
            );
        }

        #[test]
        fn v_asks_for_the_editor() {
            let mut session = opened();
            assert_eq!(
                handle_key(&mut session, key(KeyCode::Char('v'))),
                Action::Show
            );
        }

        /// The same answer the key that edits a prompt gives, and for the same reason: an editor
        /// needs the screen, and a running turn is drawing it.
        #[test]
        fn the_transcript_editor_key_does_nothing_while_a_turn_runs() {
            let mut session = opened();
            session.status = Status::Working;

            assert_eq!(
                handle_key_while_working(&mut session, key(KeyCode::Char('v'))),
                Action::None
            );
        }
    }

    mod pasting {
        use super::*;
        use crate::clipboard::{Image, MAX_IMAGE_BYTES, Pasted};

        fn picture(bytes: Vec<u8>) -> Pasted {
            Pasted::Image(Image {
                media_type: "image/png",
                bytes,
            })
        }

        /// The chord exists because Command-V cannot reach this process, so it has to be answered
        /// where the keys are read rather than left to the terminal.
        #[test]
        fn ctrl_v_asks_for_the_clipboard_to_be_read() {
            let mut session = Session::new("kernel-enforced");
            assert_eq!(handle_key(&mut session, ctrl('v')), Action::Paste);
        }

        /// A line can be typed while a turn runs, so it can be pasted into while a turn runs. What
        /// is refused mid-turn is sending, never writing.
        #[test]
        fn ctrl_v_reads_the_clipboard_during_a_turn_too() {
            let mut session = Session::new("kernel-enforced");
            session.status = Status::Working;
            assert_eq!(
                handle_key_while_working(&mut session, ctrl('v')),
                Action::Paste
            );
        }

        /// A paste that arrives carrying nothing is a Command-V the terminal could not answer: it
        /// wrote the markers and found no text between them, which is what a clipboard holding
        /// only a picture looks like from in here.
        #[test]
        fn an_empty_paste_goes_and_reads_the_clipboard_instead() {
            let mut session = Session::new("kernel-enforced");
            assert_eq!(handle_paste(&mut session, ""), Action::Paste);
            assert!(
                session.transcript[0].text.contains("ctrl-v"),
                "the note did not name the key that works"
            );
        }

        /// Said once and then not again: a user who has been told which key carries a picture does
        /// not need telling every time they use the other one.
        #[test]
        fn which_key_carries_a_picture_is_said_once_per_session() {
            let mut session = Session::new("kernel-enforced");
            handle_paste(&mut session, "");
            handle_paste(&mut session, "");
            handle_paste(&mut session, "");

            assert_eq!(session.transcript.len(), 1, "the note was repeated");
        }

        /// A paste that did carry text is an ordinary paste and must stay one, or every Command-V
        /// of a paragraph would go and read the clipboard a second time.
        #[test]
        fn a_paste_that_carried_text_is_left_alone() {
            let mut session = Session::new("kernel-enforced");
            assert_eq!(handle_paste(&mut session, "some words"), Action::Redraw);
            assert_eq!(session.input(), "some words");
            assert!(
                session.transcript.is_empty(),
                "an ordinary paste said something"
            );
        }

        /// Shell mode's line is a command, and a marker in one is text the shell would be handed
        /// verbatim. Nothing about a picture belongs there.
        #[test]
        fn a_picture_is_refused_in_shell_mode_rather_than_written_into_the_command() {
            let mut session = Session::new("kernel-enforced");
            session.shell = true;
            take_from_clipboard(&mut session, picture(b"pixels".to_vec()));

            assert!(
                session.input().is_empty(),
                "a marker reached the command line"
            );
            assert!(session.transcript[0].text.contains("not a command"));
        }

        #[test]
        fn a_picture_off_the_clipboard_becomes_a_marker_in_the_line() {
            let mut session = Session::new("kernel-enforced");
            take_from_clipboard(&mut session, picture(b"pixels".to_vec()));
            assert_eq!(session.input(), "[Image #1]");
        }

        /// Refused rather than truncated, and with the size, because half a picture would be sent,
        /// rejected by the endpoint, and reported as a fault of the request.
        #[test]
        fn a_picture_too_large_to_send_says_so_with_its_size() {
            let mut session = Session::new("kernel-enforced");
            take_from_clipboard(&mut session, Pasted::TooLarge(MAX_IMAGE_BYTES * 2));

            assert!(
                session.input().is_empty(),
                "an oversized picture reached the line"
            );
            assert!(
                session.transcript[0].text.contains("20.0 MB"),
                "the size was not reported: {}",
                session.transcript[0].text
            );
        }

        /// Carrying on saying it once the picture is in the prompt is nagging, and the answer is
        /// stale in any case: the hint is about what a key would do, and the key has been pressed.
        #[test]
        fn a_paste_clears_the_hint_that_prompted_it() {
            let mut session = Session::new("kernel-enforced");
            session.image_on_clipboard = true;
            take_from_clipboard(&mut session, picture(b"pixels".to_vec()));

            assert!(!session.image_on_clipboard, "the hint outlived the paste");
        }
    }

    fn shift(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    /// A session with `text` typed into the box, one key at a time.
    fn typed_into(text: &str) -> Session {
        let mut session = Session::new("none");
        for c in text.chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        session
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
        assert_eq!(session.input(), "write me a game\n");
        assert_eq!(session.status, Status::Idle, "the paste started a turn");
        assert!(session.transcript.is_empty(), "the paste sent something");
    }

    /// A working loop is where a paste most needs folding, since a turn in flight is what the
    /// long paste would push off the screen. One of the two loops used to write the paste whole,
    /// so this pins the folding at the seam they now share rather than at either call site.
    #[test]
    fn a_long_paste_folds_while_a_turn_is_running() {
        let mut session = Session::new("none");
        handle_paste_while_working(&mut session, "first\nsecond\nthird\nfourth");

        assert_eq!(session.input(), "[Pasted text #1 +4 lines]");
    }

    /// A drop reaches the terminal as a paste, so a working loop that only pasted wrote the path
    /// out as prose: the file was never staged, and the line said nothing about an attachment.
    /// A turn in flight is when people drop a file, because they are reading a reply and want to
    /// hand over the picture it is about.
    #[test]
    fn a_file_dropped_while_a_turn_is_running_is_attached() {
        let directory = std::env::temp_dir().join("bravebot-app-drop-while-working");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("scratch");
        let file = directory.join("shot.png");
        std::fs::write(&file, [0x89u8, 0x50]).expect("write");

        let mut session = Session::new("none").in_workspace(&directory);
        session.type_char('a');
        session.submit();
        handle_paste_while_working(&mut session, &file.to_string_lossy());

        assert_eq!(session.input(), "[Image #1] ");
        assert_eq!(session.attached().len(), 1, "nothing was staged");
        assert_eq!(session.attached()[0].name, "shot.png");

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A paragraph can be written in the box rather than only pasted into it. Shift-Enter is the
    /// one Enter that does not send.
    #[test]
    fn shift_enter_starts_a_line_instead_of_sending() {
        let mut session = typed_into("first");
        assert_eq!(
            handle_key(&mut session, shift(KeyCode::Enter)),
            Action::Redraw
        );
        for c in "second".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(session.input(), "first\nsecond");
        assert_eq!(session.status, Status::Idle, "the newline started a turn");
        assert!(session.transcript.is_empty(), "the newline sent something");
    }

    /// The bug this exists for: a terminal that cannot report the modifier on Enter is configured
    /// to send `\n` for Shift-Enter instead, which arrives in raw mode as Ctrl-J. Unbound, the
    /// keystroke did nothing at all, which is what iTerm and Terminal.app both showed.
    #[test]
    fn ctrl_j_starts_a_line_too() {
        let mut session = typed_into("first");
        assert_eq!(handle_key(&mut session, ctrl('j')), Action::Redraw);
        for c in "second".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(session.input(), "first\nsecond");
        assert_eq!(session.status, Status::Idle, "the newline started a turn");
    }

    /// Whichever spelling arrives, sending is still Enter's alone.
    #[test]
    fn ctrl_j_is_not_swallowed_while_a_turn_runs() {
        let mut session = typed_into("go");
        handle_key(&mut session, key(KeyCode::Enter));
        handle_key_while_working(&mut session, key(KeyCode::Char('a')));

        assert_eq!(
            handle_key_while_working(&mut session, ctrl('j')),
            Action::Redraw,
            "the control guard swallowed the newline"
        );
        assert_eq!(session.input(), "a\n");
    }

    /// Ctrl-M is Enter itself, so binding it would take sending away.
    #[test]
    fn only_ctrl_j_starts_a_line() {
        assert!(starts_a_line(ctrl('j')));
        assert!(starts_a_line(shift(KeyCode::Enter)));
        assert!(!starts_a_line(ctrl('m')));
        assert!(!starts_a_line(key(KeyCode::Enter)));
        assert!(!starts_a_line(key(KeyCode::Char('j'))));
    }

    /// The newline lands at the caret like any other keystroke, not at the end of the line.
    #[test]
    fn a_newline_lands_at_the_caret() {
        let mut session = typed_into("ab");
        handle_key(&mut session, key(KeyCode::Left));
        handle_key(&mut session, shift(KeyCode::Enter));

        assert_eq!(session.input(), "a\nb");
        assert_eq!(session.caret(), 2);
    }

    /// And plain Enter still sends the paragraph, once: the two are not the same key.
    #[test]
    fn enter_still_sends_a_paragraph_written_with_shift_enter() {
        let mut session = typed_into("first");
        handle_key(&mut session, shift(KeyCode::Enter));
        for c in "second".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("first\nsecond".to_string())
        );
    }

    /// A `!` after a newline is punctuation in a sentence, not a request for a shell: the mode is
    /// only ever armed by the first character of an empty line.
    #[test]
    fn a_newline_does_not_arm_shell_mode() {
        let mut session = typed_into("wait");
        handle_key(&mut session, shift(KeyCode::Enter));
        handle_key(&mut session, key(KeyCode::Char('!')));

        assert!(!session.shell, "a newline left the box looking empty");
        assert_eq!(session.input(), "wait\n!");
    }

    /// A multi-line command is a `for` loop somebody typed, so the mode gets the key too.
    #[test]
    fn shift_enter_works_in_shell_mode() {
        let mut session = Session::new("none");
        type_line(&mut session, "!for f in *; do");
        handle_key(&mut session, shift(KeyCode::Enter));
        type_line(&mut session, "  echo $f");

        assert!(session.shell, "the mode was left by a newline");
        assert_eq!(session.input(), "for f in *; do\n  echo $f");
    }

    /// A turn in flight refuses Enter but not this one: what can be typed mid-turn can be written
    /// as a paragraph mid-turn.
    #[test]
    fn shift_enter_works_while_a_turn_runs() {
        let mut session = typed_into("go");
        handle_key(&mut session, key(KeyCode::Enter));
        handle_key_while_working(&mut session, key(KeyCode::Char('a')));

        assert_eq!(
            handle_key_while_working(&mut session, shift(KeyCode::Enter)),
            Action::Redraw
        );
        assert_eq!(session.input(), "a\n");
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
        assert_eq!(session.input(), "a");
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

    /// Escape on an empty line used to leave, which made every press a question of what was in
    /// the box: the key for abandoning a thought ended the session as soon as the thought was
    /// short enough. It abandons, and never more than that.
    #[test]
    fn escape_on_an_empty_line_does_not_quit() {
        let mut session = Session::new("none");
        assert_eq!(handle_key(&mut session, key(KeyCode::Esc)), Action::Redraw);
        assert!(!session.is_quitting(), "escape ended the session");
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
        assert!(session.input().is_empty(), "the input was not cleared");
        assert!(
            !session.is_quitting(),
            "clearing the input ended the session"
        );
    }

    /// The prompt has to be able to leave the box. Ctrl-G is the one key that says so, and a
    /// control combination that falls through to the catch-all would type a stray 'g' instead.
    #[test]
    fn ctrl_g_asks_for_the_editor() {
        let mut session = Session::new("none");
        for c in "half a thought".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(handle_key(&mut session, ctrl('g')), Action::Edit);
        assert_eq!(
            session.input(),
            "half a thought",
            "the line was disturbed before the editor saw it"
        );
    }

    /// Handing the terminal to an editor mid-turn would take the screen away from the turn that
    /// is drawing on it, and the line the editor returned would be waiting for a box that has
    /// moved on. The keys the user can still use while a turn runs do not include this one.
    #[test]
    fn the_editor_key_does_nothing_while_a_turn_runs() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('x')));
        handle_key(&mut session, key(KeyCode::Enter));
        assert_eq!(session.status, Status::Working);

        assert_eq!(
            handle_key_while_working(&mut session, ctrl('g')),
            Action::None
        );
    }

    /// One key both ways, read against the line rather than remembered: a line to put away is put
    /// away, and an empty box is where the one put away earlier is wanted. A control combination
    /// falling through to the catch-all would type a stray 's' into the prompt instead.
    #[test]
    fn ctrl_s_puts_the_line_away_and_brings_it_back() {
        let mut session = Session::new("none");
        for c in "half a thought".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(handle_key(&mut session, ctrl('s')), Action::Redraw);
        assert_eq!(session.input(), "", "the line stayed in the box");

        assert_eq!(handle_key(&mut session, ctrl('s')), Action::Redraw);
        assert_eq!(session.input(), "half a thought");
    }

    /// The key writes a line and sends nothing, and sending is the whole of what a running turn
    /// refuses. Mid-turn is also when it is most wanted: a person watching a turn go wrong has
    /// somewhere to put the half-written thought a better one just replaced.
    #[test]
    fn the_stash_key_works_while_a_turn_runs() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('x')));
        handle_key(&mut session, key(KeyCode::Enter));
        assert_eq!(session.status, Status::Working);

        for c in "the next thing".chars() {
            handle_key_while_working(&mut session, key(KeyCode::Char(c)));
        }
        assert_eq!(
            handle_key_while_working(&mut session, ctrl('s')),
            Action::Redraw
        );
        assert_eq!(session.input(), "");

        handle_key_while_working(&mut session, ctrl('s'));
        assert_eq!(session.input(), "the next thing");
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

    /// Escape only ever stops. Ctrl-C is the key the loops read against what is happening, so it
    /// is told apart from a plain `c` and from every other press before either meaning is
    /// reached.
    #[test]
    fn escape_only_stops_and_ctrl_c_is_read_against_what_is_happening() {
        assert!(wants_cancel(key(KeyCode::Esc)));
        assert!(
            !wants_cancel(ctrl('c')),
            "ctrl-c is more than a request to stop"
        );

        assert!(is_ctrl_c(ctrl('c')));
        assert!(!is_ctrl_c(key(KeyCode::Esc)));
        assert!(!is_ctrl_c(key(KeyCode::Char('c'))));
        assert!(!is_ctrl_c(key(KeyCode::Enter)));
        assert!(!is_ctrl_c(key(KeyCode::Up)));
    }

    /// And a second press does the same thing again rather than leaving. A key pressed twice in
    /// a row should not mean two different things, least of all when the second is the exit.
    #[test]
    fn escape_twice_clears_and_stays() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('x')));

        assert_eq!(handle_key(&mut session, key(KeyCode::Esc)), Action::Redraw);
        assert_eq!(handle_key(&mut session, key(KeyCode::Esc)), Action::Redraw);
        assert!(!session.is_quitting());
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
        assert!(
            session.input().is_empty(),
            "backspace did not clear the line"
        );
        assert_eq!(handle_key(&mut session, ctrl('d')), Action::Quit);
    }

    /// Type a line, one key at a time, the way a user does.
    fn type_line(session: &mut Session, line: &str) {
        for c in line.chars() {
            handle_key(session, key(KeyCode::Char(c)));
        }
    }

    /// `!` on an empty line is the mode rather than a character, which is what makes the rest of
    /// the line the command exactly as typed.
    #[test]
    fn a_bang_on_an_empty_line_enters_shell_mode_without_being_typed() {
        let mut session = Session::new("none");
        type_line(&mut session, "!");

        assert!(session.shell, "the mode did not turn on");
        assert!(
            session.input().is_empty(),
            "the marker was typed into the line"
        );
    }

    /// The same for `?`: it puts the list of keys up rather than typing a character, so nothing is
    /// left in the box to delete and pressing it again takes the list down.
    #[test]
    fn a_question_mark_on_an_empty_line_toggles_the_list_without_being_typed() {
        let mut session = Session::new("none");

        type_line(&mut session, "?");
        assert!(session.shortcuts, "the list did not come up");
        assert!(session.input().is_empty(), "the marker was typed");

        type_line(&mut session, "?");
        assert!(!session.shortcuts, "the list did not go down again");
        assert!(session.input().is_empty(), "the marker was typed");
    }

    /// Punctuation everywhere but the head of the line. Somebody writing "what is this?" is asking
    /// the model a question, not asking for the keys.
    #[test]
    fn a_question_mark_inside_a_sentence_is_punctuation() {
        let mut session = Session::new("none");
        type_line(&mut session, "what is this?");

        assert!(!session.shortcuts, "the list came up mid-sentence");
        assert_eq!(session.input(), "what is this?");
    }

    /// Typing again means the reading is over, so the list goes rather than sitting under a line it
    /// says nothing about.
    #[test]
    fn typing_takes_the_list_down() {
        let mut session = Session::new("none");
        type_line(&mut session, "?");
        type_line(&mut session, "a");

        assert!(!session.shortcuts, "the list stayed up");
        assert_eq!(session.input(), "a", "the character was not typed");
    }

    /// Escape takes down whatever is up, and with an empty box the list is the only thing there is.
    #[test]
    fn escape_takes_the_list_down() {
        let mut session = Session::new("none");
        type_line(&mut session, "?");

        assert_eq!(handle_key(&mut session, key(KeyCode::Esc)), Action::Redraw);
        assert!(!session.shortcuts, "the list stayed up");
    }

    /// The list is documentation, and a turn in flight refuses sending and nothing else. The key
    /// used to set the flag with the list refused a place to be drawn, so the press did nothing on
    /// screen and the list came up unasked when the turn ended, attached to no press at all.
    #[test]
    fn a_question_mark_lists_the_keys_while_a_turn_runs() {
        let mut session = Session::new("none");
        type_line(&mut session, "anything");
        session.submit().expect("the prompt is sent");

        handle_key_while_working(&mut session, key(KeyCode::Char('?')));
        assert!(session.shortcuts, "the list did not come up");
        assert!(session.input().is_empty(), "the marker was typed");
        assert_eq!(
            session.offered(),
            crate::state::Offered::Shortcuts,
            "the list had nowhere to be drawn"
        );

        handle_key_while_working(&mut session, key(KeyCode::Char('?')));
        assert!(!session.shortcuts, "the list did not go down again");
    }

    /// A line being composed mid-turn is one Enter queues, and finishing it is machinery for
    /// something about to be sent. The list is not, which is why it is the one thing offered there.
    #[test]
    fn nothing_is_offered_for_completion_while_a_turn_runs() {
        let mut session = Session::new("none");
        type_line(&mut session, "anything");
        session.submit().expect("the prompt is sent");

        handle_key_while_working(&mut session, key(KeyCode::Char('/')));
        assert_eq!(session.offered(), crate::state::Offered::Nothing);
    }

    /// In shell mode a `?` is a glob for the shell to expand, so it is typed like any other
    /// character rather than putting the list up.
    #[test]
    fn a_question_mark_in_shell_mode_is_a_glob() {
        let mut session = Session::new("none");
        type_line(&mut session, "!");
        type_line(&mut session, "?");

        assert!(!session.shortcuts, "the list came up over a command");
        assert_eq!(session.input(), "?");
    }

    #[test]
    fn enter_in_shell_mode_runs_the_line_rather_than_prompting() {
        let mut session = Session::new("none");
        type_line(&mut session, "!ls -la");

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Run("ls -la".to_string())
        );
    }

    /// The mode lasts one command. Leaving it on would send the next thing typed to a shell, which
    /// is the sort of surprise that ends up running a sentence.
    #[test]
    fn running_a_command_leaves_shell_mode() {
        let mut session = Session::new("none");
        type_line(&mut session, "!pwd");
        handle_key(&mut session, key(KeyCode::Enter));

        assert!(!session.shell, "the mode stayed on after the command ran");
        type_line(&mut session, "what did that print");
        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("what did that print".to_string())
        );
    }

    /// A slash command is a word this program knows, but in shell mode the line is a command line:
    /// `/status` could be a program somebody has, and `!` is how they said which they meant.
    #[test]
    fn a_slash_command_in_shell_mode_is_a_command_line() {
        let mut session = Session::new("none");
        type_line(&mut session, "!/status");

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Run("/status".to_string())
        );
    }

    /// Nothing is offered in shell mode: `/usr/bin/env` is a path and an argument with an `@` in it
    /// is an argument, so completing either would rewrite the line under someone typing it.
    #[test]
    fn shell_mode_offers_no_completions() {
        let mut session = Session::new("none");
        type_line(&mut session, "!/st");

        assert!(!session.is_completing(), "a completion was offered");
        // Enter must therefore run it rather than accepting a highlighted row.
        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Run("/st".to_string())
        );
    }

    /// The marker looks like a character, so deleting back past it has to leave the mode. Otherwise
    /// the only way out is clearing the whole line, which nobody would guess.
    #[test]
    fn backspacing_past_the_marker_leaves_shell_mode() {
        let mut session = Session::new("none");
        type_line(&mut session, "!ls");

        handle_key(&mut session, key(KeyCode::Backspace));
        handle_key(&mut session, key(KeyCode::Backspace));
        assert!(session.shell, "the mode was left while text remained");

        handle_key(&mut session, key(KeyCode::Backspace));
        assert!(!session.shell, "the mode outlived the marker");
    }

    /// The marker sits before the caret, not before the line, so Backspace with the caret moved to
    /// the start is the press that deletes it. What follows was typed on purpose and stays: the
    /// mode is what was deleted, and the words become an ordinary prompt.
    #[test]
    fn backspacing_at_the_start_leaves_the_mode_and_keeps_the_line() {
        let mut session = Session::new("none");
        type_line(&mut session, "!ls -la");

        handle_key(&mut session, key(KeyCode::Home));
        handle_key(&mut session, key(KeyCode::Backspace));

        assert!(!session.shell, "the mode outlived the marker");
        assert_eq!(session.input(), "ls -la", "the line went with the marker");
    }

    /// Escape abandons the line, and the mode is part of the line: leaving it armed would send the
    /// next thing typed to a shell.
    #[test]
    fn escape_leaves_shell_mode() {
        let mut session = Session::new("none");
        type_line(&mut session, "!rm -rf /");

        handle_key(&mut session, key(KeyCode::Esc));

        assert!(!session.shell, "the mode survived being cancelled");
        assert!(session.input().is_empty());
    }

    /// The mode is part of the line even when nothing was typed behind the marker, so Escape
    /// leaves it rather than ending the session. Backspace at the same caret already does, and
    /// having the two keys disagree is how a press meant to back out of a mode loses the session.
    #[test]
    fn escape_leaves_shell_mode_armed_on_an_empty_line() {
        let mut session = Session::new("none");
        type_line(&mut session, "!");
        assert!(session.shell, "the mode was never armed");

        let action = handle_key(&mut session, key(KeyCode::Esc));

        assert_eq!(action, Action::Redraw, "escape ended the session");
        assert!(!session.shell, "the mode survived being cancelled");
    }

    /// A `!` mid-sentence is punctuation. Treating it as the mode would make "no way!" a command.
    #[test]
    fn a_bang_inside_a_line_is_an_ordinary_character() {
        let mut session = Session::new("none");
        type_line(&mut session, "no way!");

        assert!(!session.shell);
        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("no way!".to_string())
        );
    }

    /// Shell history expansion is the shell's business, and a `!` inside a command belongs to it.
    #[test]
    fn a_bang_inside_a_command_stays_in_the_command() {
        let mut session = Session::new("none");
        type_line(&mut session, "!echo hi!");

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Run("echo hi!".to_string())
        );
    }

    /// A prompt that comes back after a cancelled turn is English, not a command line. The mode was
    /// enterable mid-turn, so `!` then escape then enter ran the user's own sentence: "rm the old
    /// builds" is a reasonable prompt, and a shell reads it as an instruction.
    #[test]
    fn a_prompt_restored_after_a_cancelled_turn_is_never_run_as_a_command() {
        let mut session = Session::new("none");
        type_line(&mut session, "rm the old builds");
        session.submit().expect("the prompt is sent");

        // Mid-turn, which used to be the moment the mode could be armed unseen.
        handle_key_while_working(&mut session, key(KeyCode::Char('!')));
        session.restore("rm the old builds".to_string());
        session.clear_input();

        assert!(
            !session.shell,
            "the mode survived a cancelled turn, so the prompt was armed as a command"
        );
        type_line(&mut session, "rm the old builds");
        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("rm the old builds".to_string()),
            "the user's sentence was about to be run by a shell"
        );
    }

    /// Whatever else a cancelled turn does with the line, it must not come back armed: the text is a
    /// prompt, and the mode changes what Enter does to it.
    #[test]
    fn restoring_a_prompt_leaves_shell_mode() {
        let mut session = Session::new("none");
        type_line(&mut session, "some prompt");
        session.submit().expect("the prompt is sent");
        session.shell = true;

        session.restore("some prompt".to_string());

        assert!(!session.shell);
    }

    /// The mode belongs to the idle prompt. Entering it mid-turn arms a line the user cannot act on
    /// until the turn ends, which is the wrong moment to find out what Enter now does.
    ///
    /// The character is still typed, because words typed during a turn are kept: what is refused is
    /// the mode, not the keystroke.
    #[test]
    fn shell_mode_cannot_be_entered_while_a_turn_runs() {
        let mut session = Session::new("none");
        type_line(&mut session, "anything");
        session.submit().expect("the prompt is sent");

        handle_key_while_working(&mut session, key(KeyCode::Char('!')));

        assert!(!session.shell, "the mode turned on mid-turn");
        assert_eq!(session.input(), "!", "the keystroke was dropped");
    }

    /// Enter on a bare marker would run an empty line, which a shell accepts and which would put a
    /// pointless entry in the transcript.
    #[test]
    fn enter_on_an_empty_shell_line_does_nothing() {
        let mut session = Session::new("none");
        type_line(&mut session, "!");

        assert_eq!(handle_key(&mut session, key(KeyCode::Enter)), Action::None);
        assert!(session.transcript.is_empty());
    }

    /// The word is a command, not a prompt: it must end the session rather than reach the planner.
    #[test]
    fn typing_the_exit_command_quits() {
        let mut session = Session::new("none");
        for c in EXIT_COMMAND.chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(handle_key(&mut session, key(KeyCode::Enter)), Action::Quit);
        assert!(session.is_quitting());
        assert!(session.input().is_empty(), "the command stayed on the line");
        assert!(
            session.transcript.is_empty(),
            "the command was sent as a prompt"
        );
    }

    /// Only the bare word, so a prompt that merely mentions it is still a prompt.
    #[test]
    fn a_prompt_containing_the_exit_command_is_still_a_prompt() {
        let mut session = Session::new("none");
        for c in "what does /exit do".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("what does /exit do".to_string())
        );
        assert!(!session.is_quitting());
    }

    /// A command, not a prompt: asking for the picker must not also ask the planner about models.
    #[test]
    fn typing_the_model_command_opens_the_picker() {
        let mut session = Session::new("none");
        for c in MODEL_COMMAND.chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::ChooseModel
        );
        assert!(session.input().is_empty(), "the command stayed on the line");
        assert!(
            session.transcript.is_empty(),
            "the command was sent as a prompt"
        );
        assert_eq!(session.status, Status::Idle, "a turn began");
    }

    /// Only the bare word. "/model is slow today" is a thing to say to the planner.
    #[test]
    fn a_prompt_containing_the_model_command_is_still_a_prompt() {
        let mut session = Session::new("none");
        for c in "why is /model slow".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("why is /model slow".to_string())
        );
    }

    /// A command, not a prompt: asking for the picker must not also ask the planner about themes.
    #[test]
    fn typing_the_theme_command_opens_the_picker() {
        let mut session = Session::new("none");
        for c in THEME_COMMAND.chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::ChooseTheme
        );
        assert!(session.input().is_empty(), "the command stayed on the line");
        assert!(
            session.transcript.is_empty(),
            "the command was sent as a prompt"
        );
    }

    /// A longer word that only starts with the command is a prompt, not the command.
    #[test]
    fn a_prompt_containing_the_theme_command_is_still_a_prompt() {
        let mut session = Session::new("none");
        for c in "what does /theme do".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("what does /theme do".to_string())
        );
    }

    /// `/themes` is not `/theme`: the whole word must match.
    #[test]
    fn a_longer_word_starting_with_theme_is_a_prompt() {
        let mut session = Session::new("none");
        for c in "/themes".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("/themes".to_string())
        );
    }

    /// Naming a theme on the line applies it without opening the picker.
    #[test]
    fn the_theme_command_carries_its_name() {
        let mut session = Session::new("none");
        for c in "/theme nord".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::SetTheme("nord".to_string())
        );
        assert!(session.input().is_empty(), "the command stayed on the line");
    }

    /// The argument is the point of this one, so it must arrive with the action.
    #[test]
    fn the_add_dir_command_carries_its_directory() {
        let mut session = Session::new("none");
        for c in "/add-dir ~/notes".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::AddDirectory("~/notes".to_string())
        );
        assert!(session.input().is_empty(), "the command stayed on the line");
        assert!(
            session.transcript.is_empty(),
            "the command was sent as a prompt"
        );
    }

    /// With no argument there is nothing to open, and the loop says so rather than doing nothing.
    #[test]
    fn the_bare_add_dir_command_is_still_the_command() {
        let mut session = Session::new("none");
        for c in ADD_DIR_COMMAND.chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::AddDirectory(String::new())
        );
    }

    /// A longer word beginning with the command is not the command, or "/add-dirs are useful"
    /// would open a directory called "s are useful".
    #[test]
    fn a_longer_word_starting_with_the_command_is_a_prompt() {
        let mut session = Session::new("none");
        for c in "/add-dirs are confusing".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("/add-dirs are confusing".to_string())
        );
    }

    /// And a sentence that merely mentions it is a thing to say to the planner.
    #[test]
    fn a_prompt_containing_the_add_dir_command_is_still_a_prompt() {
        let mut session = Session::new("none");
        for c in "what does /add-dir do".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("what does /add-dir do".to_string())
        );
    }

    /// A command, not a prompt: it must not reach the planner as a request to clear something.
    #[test]
    fn typing_the_clear_command_starts_a_new_session() {
        let mut session = Session::new("none");
        for c in CLEAR_COMMAND.chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(handle_key(&mut session, key(KeyCode::Enter)), Action::Clear);
        assert!(session.input().is_empty(), "the command stayed on the line");
        assert!(
            session.transcript.is_empty(),
            "the command was sent as a prompt"
        );
    }

    /// Typed before submitting, so the word never reaches the planner as a prompt: a session
    /// asking to be shortened must not answer by talking about shortening itself.
    #[test]
    fn the_compact_command_asks_for_a_summary_rather_than_being_sent() {
        let mut session = Session::new("none");
        for c in COMPACT_COMMAND.chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Compact
        );
        assert!(session.input().is_empty(), "the command stayed on the line");
        assert!(
            session.transcript.is_empty(),
            "the command was sent as a prompt"
        );
    }

    /// Only the bare word, so asking the planner about compacting something still asks it.
    #[test]
    fn a_prompt_containing_the_compact_command_is_still_a_prompt() {
        let mut session = Session::new("none");
        for c in "how does /compact work".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("how does /compact work".to_string())
        );
    }

    /// A person who has just typed /compact will reach for one of these, and nothing here can
    /// stop the one request a summary takes. Both are answered rather than swallowed: a key that
    /// does nothing and says nothing reads as an interface that has hung.
    #[test]
    fn a_key_that_would_stop_a_turn_is_answered_during_a_summary() {
        assert!(wants_cancel(key(KeyCode::Esc)));
        assert!(is_ctrl_c(ctrl('c')));
    }

    /// `commands()` is the one place the set is written down, so a command missing from it is a
    /// command the hint line, the completion list and Tab all fail to know about.
    #[test]
    fn compacting_is_offered_while_a_command_is_being_typed() {
        assert_eq!(
            completions("/comp")
                .iter()
                .map(|command| command.name)
                .collect::<Vec<_>>(),
            vec![COMPACT_COMMAND]
        );
    }

    /// Only the bare word, so asking the planner about clearing something still asks it.
    #[test]
    fn a_prompt_containing_the_clear_command_is_still_a_prompt() {
        let mut session = Session::new("none");
        for c in "does /clear delete anything".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("does /clear delete anything".to_string())
        );
    }

    /// The name is the point of this one, so it must arrive with the action, spaces and all.
    #[test]
    fn the_rename_command_carries_the_whole_name() {
        let mut session = Session::new("none");
        for c in "/rename the parser bug".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Rename("the parser bug".to_string())
        );
        assert!(session.input().is_empty(), "the command stayed on the line");
        assert!(
            session.transcript.is_empty(),
            "the command was sent as a prompt"
        );
    }

    /// With no name there is nothing to rename to, and the loop says so.
    #[test]
    fn the_bare_rename_command_is_still_the_command() {
        let mut session = Session::new("none");
        for c in RENAME_COMMAND.chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Rename(String::new())
        );
    }

    /// A sentence mentioning it is a thing to say to the planner.
    #[test]
    fn a_prompt_containing_the_rename_command_is_still_a_prompt() {
        let mut session = Session::new("none");
        for c in "what does /rename do".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("what does /rename do".to_string())
        );
    }

    /// The command that does not send the line it was typed on. What goes to the planner is what
    /// was left after the interval was read off, which is what every later tick sends too.
    #[test]
    fn the_loop_command_sends_what_is_left_after_the_interval() {
        let mut session = Session::new("none");
        for c in "/loop 5m check the deploy".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("check the deploy".to_string())
        );
        assert!(session.input().is_empty(), "the command stayed on the line");
        assert_eq!(
            session.looping().map(|running| running.prompt()),
            Some("check the deploy")
        );
    }

    /// With nothing to repeat there is no loop to start, and the interface says what it needs
    /// rather than quietly doing nothing.
    #[test]
    fn the_bare_loop_command_is_still_the_command() {
        for line in [LOOP_COMMAND, "/loop 5m"] {
            let mut session = Session::new("none");
            for c in line.chars() {
                handle_key(&mut session, key(KeyCode::Char(c)));
            }

            assert_eq!(
                handle_key(&mut session, key(KeyCode::Enter)),
                Action::Redraw
            );
            assert!(session.looping().is_none(), "{line} started a loop");
            assert_eq!(session.transcript.len(), 1, "{line} said nothing");
        }
    }

    /// A sentence mentioning it is a thing to say to the planner.
    #[test]
    fn a_prompt_containing_the_loop_command_is_still_a_prompt() {
        let mut session = Session::new("none");
        for c in "what does /loop do".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("what does /loop do".to_string())
        );
        assert!(session.looping().is_none());
    }

    #[test]
    fn a_longer_word_starting_with_loop_is_a_prompt() {
        let mut session = Session::new("none");
        for c in "/looper is a good name".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("/looper is a good name".to_string())
        );
    }

    /// The key that means "stop what is happening" has to reach the thing that keeps happening
    /// before it reaches the thing that would take the session with it.
    #[test]
    fn interrupting_stops_the_loop_before_it_leaves() {
        let mut session = Session::new("none");
        session.start_loop(crate::loops::parse("5m watch").expect("a request"));
        session.complete("done", Vec::new(), 0);

        assert_eq!(handle_key(&mut session, ctrl('c')), Action::Redraw);
        assert!(
            session.looping().is_none(),
            "the loop survived the interrupt"
        );
        assert!(!session.is_quitting(), "the session left as well");

        assert_eq!(handle_key(&mut session, ctrl('c')), Action::Quit);
    }

    /// A half-written line is nearer than the loop, so stopping the loop does not cost somebody
    /// the sentence they were in the middle of.
    #[test]
    fn interrupting_clears_the_line_before_it_stops_the_loop() {
        let mut session = Session::new("none");
        session.start_loop(crate::loops::parse("5m watch").expect("a request"));
        session.complete("done", Vec::new(), 0);
        for c in "half a thought".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        handle_key(&mut session, ctrl('c'));
        assert!(session.input().is_empty());
        assert!(session.looping().is_some(), "the loop went with the line");
    }

    /// One parser serves every command that takes an argument, so its rules are worth pinning
    /// once: the bare word, the word with an argument, and a longer word that merely starts alike.
    #[test]
    fn an_argument_is_taken_only_after_the_whole_command_word() {
        assert_eq!(
            argument_to("/rename a name", RENAME_COMMAND),
            Some("a name")
        );
        assert_eq!(argument_to("/rename", RENAME_COMMAND), Some(""));
        assert_eq!(
            argument_to("  /rename  spaced  ", RENAME_COMMAND),
            Some("spaced")
        );
        assert_eq!(argument_to("/renamed thing", RENAME_COMMAND), None);
        assert_eq!(argument_to("please /rename it", RENAME_COMMAND), None);
    }

    /// A slash offers everything; a letter narrows it; a space settles it, since the argument
    /// comes next and there is nothing left to complete.
    #[test]
    fn what_a_half_typed_line_could_become() {
        assert_eq!(completions("/").len(), commands().len());
        assert_eq!(
            completions("/cl")
                .iter()
                .map(|c| c.name)
                .collect::<Vec<_>>(),
            vec![CLEAR_COMMAND]
        );
        assert!(
            completions("/add-dir ~/notes").is_empty(),
            "a settled command"
        );
        assert!(completions("what does /model do").is_empty(), "a prompt");
        assert!(completions("").is_empty());
        assert!(completions("/zzz").is_empty(), "a word matching nothing");
    }

    /// Tab is what completes everywhere else, and a command taking an argument gets the space its
    /// argument goes after.
    #[test]
    fn tab_takes_the_highlighted_command() {
        let mut session = Session::new("none");
        for c in "/mod".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        assert_eq!(handle_key(&mut session, key(KeyCode::Tab)), Action::Redraw);
        assert_eq!(session.input(), MODEL_COMMAND, "no argument, so no space");

        let mut session = Session::new("none");
        for c in "/add".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        handle_key(&mut session, key(KeyCode::Tab));
        assert_eq!(
            session.input(),
            format!("{ADD_DIR_COMMAND} "),
            "argument follows"
        );
    }

    /// Enter on a half-typed command takes the highlighted row rather than sending "/mod" to the
    /// planner, which is never what was meant.
    #[test]
    fn enter_on_a_half_typed_command_completes_it() {
        let mut session = Session::new("none");
        for c in "/mod".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Redraw
        );
        assert_eq!(session.input(), MODEL_COMMAND);
        assert!(session.transcript.is_empty(), "a fragment was sent");
    }

    /// A command typed out in full still runs on Enter: completing must not get in the way of the
    /// thing it exists to help with.
    #[test]
    fn enter_on_a_whole_command_still_runs_it() {
        let mut session = Session::new("none");
        for c in CLEAR_COMMAND.chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        assert_eq!(handle_key(&mut session, key(KeyCode::Enter)), Action::Clear);
    }

    /// The arrows belong to the list while it is open, and to history and scrolling once it is not.
    #[test]
    fn the_arrows_walk_the_offered_commands_while_one_is_being_typed() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('/')));

        assert_eq!(
            session.highlighted_completion().map(|c| c.name),
            Some(commands()[0].name),
            "the list opens at the top"
        );
        handle_key(&mut session, key(KeyCode::Down));
        assert_eq!(
            session.highlighted_completion().map(|c| c.name),
            Some(commands()[1].name)
        );
        handle_key(&mut session, key(KeyCode::Up));
        assert_eq!(
            session.highlighted_completion().map(|c| c.name),
            Some(commands()[0].name)
        );

        // Up at the top stays, rather than wrapping to the end.
        handle_key(&mut session, key(KeyCode::Up));
        assert_eq!(
            session.highlighted_completion().map(|c| c.name),
            Some(commands()[0].name)
        );
    }

    /// Down past the end stays on the last, so Tab always takes something.
    #[test]
    fn walking_past_the_end_stays_on_the_last_command() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('/')));
        for _ in 0..commands().len() + 3 {
            handle_key(&mut session, key(KeyCode::Down));
        }
        assert_eq!(
            session.highlighted_completion().map(|c| c.name),
            Some(commands()[commands().len() - 1].name)
        );
    }

    /// Typing returns the cursor to the top, so the highlighted row does not drift to a different
    /// command as the list narrows under it.
    #[test]
    fn typing_another_letter_returns_to_the_top_of_the_list() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('/')));
        for _ in 0..4 {
            handle_key(&mut session, key(KeyCode::Down));
        }
        handle_key(&mut session, key(KeyCode::Char('c')));

        assert_eq!(
            session.highlighted_completion().map(|c| c.name),
            Some(COMPACT_COMMAND),
            "the cursor did not return to the top of what now matches"
        );
    }

    /// A paste narrows the list without touching the cursor, so the cursor may be past the end of
    /// what is left. Reading it must still name something, or Tab would complete nothing.
    #[test]
    fn a_cursor_past_the_end_of_a_narrowed_list_still_names_a_command() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('/')));
        // To the last of them, counted rather than named, so adding a command does not make this
        // test assert about the wrong row.
        for _ in 0..commands().len() {
            handle_key(&mut session, key(KeyCode::Down));
        }
        assert_eq!(
            session.highlighted_completion().map(|c| c.name),
            Some(commands()[commands().len() - 1].name)
        );

        // Now one command matches, while the cursor still points at the last.
        handle_paste(&mut session, "cl");
        assert_eq!(
            session.highlighted_completion().map(|c| c.name),
            Some(CLEAR_COMMAND),
            "the cursor pointed past the narrowed list"
        );
        handle_key(&mut session, key(KeyCode::Tab));
        assert_eq!(session.input(), CLEAR_COMMAND);
    }

    /// With nothing being offered, Tab must not insert anything and the arrows go back to what they
    /// do the rest of the time.
    #[test]
    fn tab_does_nothing_when_no_command_is_being_typed() {
        let mut session = Session::new("none");
        for c in "an ordinary prompt".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        assert_eq!(handle_key(&mut session, key(KeyCode::Tab)), Action::None);
        assert_eq!(session.input(), "an ordinary prompt");
    }

    #[test]
    fn typing_the_status_command_reports_rather_than_prompting() {
        let mut session = Session::new("none");
        for c in STATUS_COMMAND.chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Status
        );
        assert!(session.input().is_empty(), "the command stayed on the line");
        assert!(
            session.transcript.is_empty(),
            "the command was sent as a prompt"
        );
    }

    /// Asking the planner about status is a question, not a command.
    #[test]
    fn a_prompt_containing_the_status_command_is_still_a_prompt() {
        let mut session = Session::new("none");
        for c in "what does /status show".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(
            handle_key(&mut session, key(KeyCode::Enter)),
            Action::Submit("what does /status show".to_string())
        );
    }

    /// A directory whose own name begins with a tilde is not a home-relative path.
    #[test]
    fn a_tilde_is_expanded_only_as_a_whole_first_segment() {
        let home = std::env::var("HOME").expect("a home directory");
        assert_eq!(expand_home("~/notes"), format!("{home}/notes"));
        assert_eq!(expand_home("~"), home);
        assert_eq!(expand_home("~notes"), "~notes");
        assert_eq!(expand_home("/tmp/notes"), "/tmp/notes");
        assert_eq!(expand_home("relative/notes"), "relative/notes");
    }

    #[test]
    fn ctrl_t_toggles_the_trail() {
        let mut session = Session::new("none");
        assert_eq!(handle_key(&mut session, ctrl('t')), Action::Redraw);
        assert!(session.show_trail);
        handle_key(&mut session, ctrl('t'));
        assert!(!session.show_trail);
    }

    /// The trail is drawn per entry as a turn fills in, so it is what somebody watching a turn call
    /// tools wants. Refused mid-turn, the key was swallowed by the catch-all for control chords
    /// while the hint line went on advertising it on every frame of that turn.
    #[test]
    fn the_trail_can_be_asked_for_while_a_turn_runs() {
        let mut session = Session::new("none");
        type_line(&mut session, "anything");
        session.submit().expect("the prompt is sent");

        assert_eq!(
            handle_key_while_working(&mut session, ctrl('t')),
            Action::Redraw
        );
        assert!(session.show_trail, "the key was swallowed");
        assert_eq!(session.input(), "", "a stray character was typed");
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
        assert_eq!(session.input(), "first question");
        assert_eq!(session.scroll, 0, "recall scrolled the transcript as well");
    }

    /// The keys that walk the history do it whether or not a turn is running. They reached no
    /// arm at all mid-turn and fell through to nothing, so a person watching a turn go wrong
    /// could see their last prompt in the transcript and had no way to get it back into the box.
    #[test]
    fn up_recalls_a_previous_prompt_while_a_turn_is_running() {
        let mut session = Session::new("none");
        for c in "first question".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        handle_key(&mut session, key(KeyCode::Enter));
        // No `complete`: the turn is still in flight.
        assert_eq!(session.status, Status::Working);

        assert_eq!(
            handle_key_while_working(&mut session, key(KeyCode::Up)),
            Action::Redraw
        );
        assert_eq!(session.input(), "first question");

        assert_eq!(
            handle_key_while_working(&mut session, key(KeyCode::Down)),
            Action::Redraw
        );
        assert!(session.input().is_empty(), "Down did not walk back out");
    }

    /// The press a person makes when an answer is going wrong in front of them is asking for the
    /// answer to stop, not for the session to end. It left instead, taking the transcript and
    /// everything else with it.
    #[test]
    fn ctrl_c_stops_a_turn_rather_than_leaving() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit();
        assert_eq!(session.status, Status::Working);

        assert_eq!(handle_key(&mut session, ctrl('c')), Action::Cancel);
        assert!(
            !session.is_quitting(),
            "stopping the turn ended the session"
        );
    }

    /// The line in the box is the next thing there is to stop, and a person half way through a
    /// sentence is not asking to leave. Leaving over it would take the words with the session.
    #[test]
    fn ctrl_c_clears_the_line_before_it_leaves() {
        let mut session = Session::new("none");
        for c in "half a thought".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }

        assert_eq!(handle_key(&mut session, ctrl('c')), Action::Redraw);
        assert_eq!(session.input(), "");
        assert!(
            !session.is_quitting(),
            "clearing the line ended the session"
        );
        assert!(
            session.cleared_by_interrupt,
            "nothing said what the next press would do"
        );
    }

    /// A press that ends the session is not one to explain, and the hint is the answer to a line
    /// having just gone. On an empty line nothing went, so there is nothing to answer.
    #[test]
    fn the_way_out_is_offered_only_where_a_line_was_taken() {
        let mut session = Session::new("none");
        handle_key(&mut session, ctrl('c'));
        assert!(
            !session.cleared_by_interrupt,
            "offered where nothing was cleared"
        );
    }

    /// It lives for one press. Standing there afterwards, it would go on offering an exit to
    /// somebody who has started writing the next line and is no longer being asked anything.
    #[test]
    fn the_way_out_stops_being_offered_at_the_next_press() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('x')));
        handle_key(&mut session, ctrl('c'));
        assert!(session.cleared_by_interrupt);

        handle_key(&mut session, key(KeyCode::Char('y')));
        assert!(
            !session.cleared_by_interrupt,
            "the hint outstayed its press"
        );
    }

    /// The whole ladder, nearest first: the turn, then the line the stop put back, then the
    /// session. Each press has something of its own to answer, so none of them is a press that
    /// silently did another one's job.
    #[test]
    fn ctrl_c_leaves_once_there_is_nothing_left_to_stop() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit();

        assert_eq!(handle_key(&mut session, ctrl('c')), Action::Cancel);
        session.restore("a");
        assert_eq!(session.input(), "a", "the stopped prompt came back");

        assert_eq!(handle_key(&mut session, ctrl('c')), Action::Redraw);
        assert!(!session.is_quitting());

        assert_eq!(handle_key(&mut session, ctrl('c')), Action::Quit);
        assert!(session.is_quitting());
    }

    /// Escape is still the key that stops a turn, and stopping a turn is not leaving. Losing that
    /// distinction is what made Ctrl-C useless as a way out.
    #[test]
    fn escape_stops_the_turn_without_ending_the_session() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit();

        assert!(wants_cancel(key(KeyCode::Esc)));
        assert!(!session.is_quitting(), "stopping a turn ended the session");
    }

    /// Enter mid-turn used to reach nothing, so the line sat in the box while the person waited
    /// to notice the turn had ended. It goes now, and says that it is waiting.
    #[test]
    fn enter_queues_a_prompt_while_a_turn_is_running() {
        let mut session = Session::new("none");
        for c in "first".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        handle_key(&mut session, key(KeyCode::Enter));
        assert_eq!(session.status, Status::Working);

        for c in "second".chars() {
            handle_key_while_working(&mut session, key(KeyCode::Char(c)));
        }
        assert_eq!(
            handle_key_while_working(&mut session, key(KeyCode::Enter)),
            Action::Redraw
        );
        assert!(session.input().is_empty(), "the line stayed in the box");
        assert_eq!(session.queued.len(), 1);
        assert_eq!(session.queued[0].prompt, "second");
    }

    /// The point of queueing, and what it used to fail at: a prompt typed mid-turn is put where the
    /// running turn can reach it, not left for a turn that may be minutes away. A person who says
    /// "no, not that file" while an agent works is trying to redirect the work in front of them,
    /// and an instruction that waits for the answer arrives after the thing it was meant to change.
    #[test]
    fn a_prompt_queued_mid_turn_is_within_the_running_turns_reach() {
        let mut session = Session::new("none");
        for c in "first".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        handle_key(&mut session, key(KeyCode::Enter));

        // What the worker holds. Taken before the prompt is queued, exactly as a turn takes it.
        let reaching = session.interjections();
        assert!(reaching.take().is_none(), "something was waiting already");

        for c in "actually stop".chars() {
            handle_key_while_working(&mut session, key(KeyCode::Char(c)));
        }
        handle_key_while_working(&mut session, key(KeyCode::Enter));

        assert_eq!(
            reaching.take().as_deref(),
            Some("actually stop"),
            "the turn could not reach a prompt typed while it ran"
        );
    }

    /// Until the turn takes it, a queued prompt has not been said: it is drawn above the box as
    /// waiting, and the transcript is for what has happened. It joins the transcript at the moment
    /// the planner is given it, which is what keeps the two reading in the same order.
    #[test]
    fn a_queued_prompt_joins_the_transcript_when_the_planner_is_given_it() {
        let mut session = Session::new("none");
        for c in "first".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        handle_key(&mut session, key(KeyCode::Enter));
        for c in "second".chars() {
            handle_key_while_working(&mut session, key(KeyCode::Char(c)));
        }
        handle_key_while_working(&mut session, key(KeyCode::Enter));

        let said = session
            .transcript
            .iter()
            .filter(|entry| entry.speaker == crate::state::Speaker::User)
            .count();
        assert_eq!(said, 1, "a prompt that had not gone anywhere was recorded");

        session.interjected();
        let said: Vec<&str> = session
            .transcript
            .iter()
            .filter(|entry| entry.speaker == crate::state::Speaker::User)
            .map(|entry| entry.text.as_str())
            .collect();
        assert_eq!(said, vec!["first", "second"]);
        assert!(session.queued.is_empty(), "it is still drawn as waiting");
    }

    /// A prompt still waiting when the turn ends is a turn of its own, as every queued prompt used
    /// to be. What must not happen is both: the copy left for the turn that has finished has to go,
    /// or the same words reach the planner twice, the second time as an interjection into the very
    /// turn they started.
    #[test]
    fn a_prompt_that_outlived_the_turn_is_sent_once() {
        let mut session = Session::new("none");
        for c in "first".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        handle_key(&mut session, key(KeyCode::Enter));
        for c in "second".chars() {
            handle_key_while_working(&mut session, key(KeyCode::Char(c)));
        }
        handle_key_while_working(&mut session, key(KeyCode::Enter));

        let reaching = session.interjections();
        session.complete("done", Vec::new(), 0);
        assert_eq!(session.send_queued().as_deref(), Some("second"));
        assert!(
            reaching.take().is_none(),
            "the prompt was sent as a turn and left waiting to be interjected as well"
        );
    }

    /// Three prompts queued during one turn, and the rule that governs them afterwards: the running
    /// turn takes what it can, one becomes the next turn, and the rest go on waiting under the same
    /// rule. What must hold throughout is that the queue on the screen and the buffer the turn takes
    /// from never disagree about how many prompts are waiting.
    #[test]
    fn what_is_still_waiting_stays_in_step_with_what_is_drawn() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit();
        let reaching = session.interjections();

        for line in ["one", "two", "three"] {
            for c in line.chars() {
                session.type_char(c);
            }
            assert!(session.queue(), "nothing was queued");
        }

        // The turn takes the first, as it would at a round boundary.
        assert_eq!(reaching.take().as_deref(), Some("one"));
        session.interjected();
        assert_eq!(session.queued.len(), 2);

        // Then it ends with two still waiting. The oldest becomes a turn of its own.
        session.complete("an answer", Vec::new(), 0);
        assert_eq!(session.send_queued().as_deref(), Some("two"));
        assert_eq!(session.queued.len(), 1);

        // And the last is waiting for the turn that just began, not for a second copy of itself.
        assert_eq!(reaching.take().as_deref(), Some("three"));
        assert!(reaching.take().is_none(), "a prompt was waiting twice over");
    }

    /// Up reaches for the last thing the person said, and while prompts are waiting that is the
    /// queue. It walked the history instead, which holds a copy of every queued line: the copy
    /// came back, the person rewrote it, and the prompt they meant to take back went anyway.
    ///
    /// One press for the whole queue, not one press each. A key that gave them back a line at a
    /// time would leave the person pressing it until they guessed there were none left, with the
    /// ones they had already taken back sitting in the box in front of them.
    #[test]
    fn up_takes_back_everything_waiting_rather_than_a_copy_of_it() {
        let mut session = Session::new("none");
        for c in "first".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        handle_key(&mut session, key(KeyCode::Enter));
        for line in ["second", "third"] {
            for c in line.chars() {
                handle_key_while_working(&mut session, key(KeyCode::Char(c)));
            }
            handle_key_while_working(&mut session, key(KeyCode::Enter));
        }
        assert_eq!(session.queued.len(), 2);

        assert_eq!(
            handle_key_while_working(&mut session, key(KeyCode::Up)),
            Action::Redraw
        );
        assert_eq!(session.input(), "second\nthird");
        assert!(session.queued.is_empty(), "a prompt was left waiting");
    }

    /// Taking a prompt back has to take it out of the turn's reach too, or the key reads as having
    /// done nothing: the line comes back to the box, the person rewrites it, and the copy the turn
    /// was still holding arrives at the planner anyway. Which is the bug the old queue could not
    /// have, because nothing could reach a queued prompt until the turn was over.
    #[test]
    fn taking_the_queue_back_takes_it_out_of_the_turns_reach() {
        let mut session = Session::new("none");
        for c in "first".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        handle_key(&mut session, key(KeyCode::Enter));
        let reaching = session.interjections();
        for c in "second".chars() {
            handle_key_while_working(&mut session, key(KeyCode::Char(c)));
        }
        handle_key_while_working(&mut session, key(KeyCode::Enter));

        handle_key_while_working(&mut session, key(KeyCode::Up));
        assert_eq!(session.input(), "second");
        assert!(
            reaching.take().is_none(),
            "a prompt taken back was still on its way to the planner"
        );
    }

    /// The other side of it: a prompt the turn has already been given cannot be taken back, because
    /// the planner has it. Offering it to the box would leave the person editing a line that had
    /// gone, and pressing Enter would say it twice.
    #[test]
    fn a_prompt_the_turn_has_taken_cannot_be_taken_back() {
        let mut session = Session::new("none");
        for c in "first".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        handle_key(&mut session, key(KeyCode::Enter));
        let reaching = session.interjections();
        for c in "second".chars() {
            handle_key_while_working(&mut session, key(KeyCode::Char(c)));
        }
        handle_key_while_working(&mut session, key(KeyCode::Enter));

        // The turn takes it, as it would at its next round boundary.
        assert_eq!(reaching.take().as_deref(), Some("second"));

        handle_key_while_working(&mut session, key(KeyCode::Up));
        assert!(
            !session.input().contains("second"),
            "a prompt the planner already had came back to the box: {}",
            session.input()
        );
    }

    /// With nothing waiting the key means what it has always meant. Taking the queue back is the
    /// exception, and it lasts exactly as long as there is a queue.
    #[test]
    fn up_walks_the_history_again_once_nothing_is_waiting() {
        let mut session = Session::new("none");
        for c in "first".chars() {
            handle_key(&mut session, key(KeyCode::Char(c)));
        }
        handle_key(&mut session, key(KeyCode::Enter));
        for c in "second".chars() {
            handle_key_while_working(&mut session, key(KeyCode::Char(c)));
        }
        handle_key_while_working(&mut session, key(KeyCode::Enter));
        handle_key_while_working(&mut session, key(KeyCode::Up));
        for _ in 0.."second".len() {
            handle_key_while_working(&mut session, key(KeyCode::Backspace));
        }
        assert_eq!(session.input(), "");

        handle_key_while_working(&mut session, key(KeyCode::Up));
        assert_eq!(session.input(), "second", "the history was out of reach");
    }

    /// Shift-Enter writes a paragraph mid-turn as it does at rest, so it must not be caught by
    /// the arm that queues. A queued half-sentence is worse than one that waits to be finished.
    #[test]
    fn starting_a_line_mid_turn_does_not_queue_it() {
        let mut session = Session::new("none");
        session.type_char('a');
        session.submit();

        for c in "half".chars() {
            handle_key_while_working(&mut session, key(KeyCode::Char(c)));
        }
        handle_key_while_working(
            &mut session,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
        );

        assert!(
            session.queued.is_empty(),
            "a paragraph was sent half-written"
        );
        assert!(session.input().contains('\n'), "no line was started");
    }

    /// Why a key is allowed to mean something different while a turn runs, or `None` where it is
    /// not.
    ///
    /// The whole of the difference between the two paths, in one place. Every one of them is about
    /// sending or about leaving, which are the two things the box does not decide.
    fn allowed_to_differ(key: KeyEvent) -> Option<&'static str> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            // Shift-Enter and Ctrl-J start a line and are not this key.
            KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => Some("sends"),
            KeyCode::Esc => Some("stops the turn in flight"),
            KeyCode::Char('c') if ctrl => Some("stops the turn in flight, and then leaves"),
            KeyCode::Char('d') if ctrl => Some("leaves, which is not something the box does"),
            KeyCode::Char('g') if ctrl => {
                Some("hands the screen the turn is drawing on to an editor")
            }
            KeyCode::Char('!') => Some("arms a mode that changes what Enter does"),
            _ => None,
        }
    }

    /// Every key the two paths could see, so a binding added to one and not the other is caught
    /// here rather than by somebody pressing it.
    fn every_key() -> Vec<KeyEvent> {
        let codes = (0x20u8..=0x7e)
            .map(|byte| KeyCode::Char(char::from(byte)))
            .chain([
                KeyCode::Enter,
                KeyCode::Tab,
                KeyCode::BackTab,
                KeyCode::Backspace,
                KeyCode::Delete,
                KeyCode::Insert,
                KeyCode::Home,
                KeyCode::End,
                KeyCode::PageUp,
                KeyCode::PageDown,
                KeyCode::Up,
                KeyCode::Down,
                KeyCode::Left,
                KeyCode::Right,
                KeyCode::Esc,
                KeyCode::F(1),
            ]);
        let modifiers = [
            KeyModifiers::NONE,
            KeyModifiers::CONTROL,
            KeyModifiers::SHIFT,
            KeyModifiers::ALT,
        ];
        codes
            .flat_map(|code| modifiers.iter().map(move |held| KeyEvent::new(code, *held)))
            .collect()
    }

    /// The two paths answer the same set of keys, and the ones they are allowed to disagree about
    /// are named rather than discovered.
    ///
    /// Asserted as a set because a list was the bug. This walked six key codes, and three keys that
    /// send nothing were answered by the idle path alone: none of the three was in the list, so the
    /// test that was supposed to pin the agreement said nothing whatever about any of them.
    #[test]
    fn the_two_paths_answer_the_same_set_of_keys() {
        /// Everything a key press can reach that is not about sending, so a difference in any of it
        /// is a difference the clause forbids.
        fn answered(session: &Session, action: Action) -> String {
            format!(
                "{action:?} input={:?} caret={} scroll={} shell={} shortcuts={} trail={} \
                 stashed={:?} scrolling={} queued={} browsing={}",
                session.input(),
                session.caret(),
                session.scroll,
                session.shell,
                session.shortcuts,
                session.show_trail,
                session.stashed(),
                session.scrolling(),
                session.queued.len(),
                session.history.is_browsing(),
            )
        }

        let sent = |finished: bool, typed: &str| {
            let mut session = Session::new("none");
            type_line(&mut session, "first question");
            handle_key(&mut session, key(KeyCode::Enter));
            // The same transcript either way, so only the running turn differs.
            if finished {
                session.complete("an answer", Vec::new(), 0);
            } else {
                session.narrate("an answer");
            }
            // Several arms are read against whether there is a line, so both cases are swept.
            for c in typed.chars() {
                if finished {
                    handle_key(&mut session, key(KeyCode::Char(c)));
                } else {
                    handle_key_while_working(&mut session, key(KeyCode::Char(c)));
                }
            }
            session
        };

        for line in ["", "half a thought"] {
            for pressed in every_key() {
                if allowed_to_differ(pressed).is_some() {
                    continue;
                }
                let mut idle = sent(true, line);
                let mut working = sent(false, line);
                assert_eq!(working.status, Status::Working);
                assert_eq!(
                    answered(&idle, Action::None),
                    answered(&working, Action::None),
                    "the two sessions differed before {pressed:?} was pressed"
                );

                let at_rest = handle_key(&mut idle, pressed);
                let mid_turn = handle_key_while_working(&mut working, pressed);

                assert_eq!(
                    answered(&idle, at_rest),
                    answered(&working, mid_turn),
                    "{pressed:?} was answered differently while a turn was running, \
                     over a line of {line:?}"
                );
            }
        }
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
        assert_eq!(session.input(), "sent");

        handle_key(&mut session, key(KeyCode::Down));
        assert_eq!(
            session.input(),
            "being typed",
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
        assert!(session.input().is_empty());
    }

    /// With nothing typed the prompt has nowhere to go, so the keys are the transcript's, which is
    /// what they were before there was a caret to move.
    #[test]
    fn page_keys_scroll_further() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::PageUp));
        assert_eq!(session.scroll, 10);
        handle_key(&mut session, key(KeyCode::PageDown));
        assert_eq!(session.scroll, 0);
    }

    /// The first press takes the start of the line and the second the line before it, so someone
    /// who wanted this line's start gets it without losing their place in the paragraph.
    #[test]
    fn page_up_takes_the_start_of_the_line_then_the_line_before() {
        let mut session = Session::new("none");
        handle_paste(&mut session, "first line\nsecond line\nthird line");

        handle_key(&mut session, key(KeyCode::PageUp));
        assert_eq!(session.caret(), "first line\nsecond line\n".len());
        handle_key(&mut session, key(KeyCode::PageUp));
        assert_eq!(session.caret(), "first line\n".len());
        handle_key(&mut session, key(KeyCode::PageUp));
        assert_eq!(session.caret(), 0);
    }

    /// And the same downwards, by line ends.
    #[test]
    fn page_down_takes_the_end_of_the_line_then_the_line_after() {
        let mut session = Session::new("none");
        handle_paste(&mut session, "first line\nsecond line\nthird line");
        // Up to the very start, which is where paging down has somewhere to go from.
        for _ in 0..3 {
            handle_key(&mut session, key(KeyCode::PageUp));
        }
        assert_eq!(session.caret(), 0);

        handle_key(&mut session, key(KeyCode::PageDown));
        assert_eq!(session.caret(), "first line".len());
        handle_key(&mut session, key(KeyCode::PageDown));
        assert_eq!(session.caret(), "first line\nsecond line".len());
    }

    /// Off the ends of the prompt the keys go back to the transcript, so a paragraph does not trap
    /// them and scrolling stays reachable.
    #[test]
    fn the_page_keys_reach_the_transcript_from_the_ends_of_the_prompt() {
        let mut session = Session::new("none");
        handle_paste(&mut session, "one\ntwo");

        // The caret starts at the end, so the start of "two" and then the start of "one".
        handle_key(&mut session, key(KeyCode::PageUp));
        handle_key(&mut session, key(KeyCode::PageUp));
        assert_eq!(session.caret(), 0);

        handle_key(&mut session, key(KeyCode::PageUp));
        assert_eq!(session.scroll, 10, "the transcript was unreachable");

        // And back down: the end of "one", then the end of "two".
        handle_key(&mut session, key(KeyCode::PageDown));
        handle_key(&mut session, key(KeyCode::PageDown));
        assert_eq!(session.caret(), session.input().len());

        handle_key(&mut session, key(KeyCode::PageDown));
        assert_eq!(session.scroll, 0);
    }

    /// A single-line prompt has ends too, so the keys reach them before the transcript.
    #[test]
    fn the_page_keys_work_on_one_line() {
        let mut session = typed_into("a sentence");
        handle_key(&mut session, key(KeyCode::PageUp));
        assert_eq!(session.caret(), 0);
        assert_eq!(session.scroll, 0, "the transcript scrolled instead");
        handle_key(&mut session, key(KeyCode::PageDown));
        assert_eq!(session.caret(), "a sentence".len());
    }

    /// Under Ctrl, because the bare keys belong to the line being typed.
    #[test]
    fn home_and_end_jump_to_the_extremes() {
        let mut session = Session::new("none");
        handle_key(&mut session, ctrl_key(KeyCode::Home));
        assert_eq!(session.scroll, u16::MAX);
        handle_key(&mut session, ctrl_key(KeyCode::End));
        assert_eq!(session.scroll, 0);
    }

    /// The keys that move around a line of text, which is what makes the box editable rather than
    /// only appendable.
    #[test]
    fn the_arrows_move_the_caret_through_the_line() {
        let mut session = typed_into("abc");
        assert_eq!(session.caret(), 3);

        assert_eq!(handle_key(&mut session, key(KeyCode::Left)), Action::Redraw);
        assert_eq!(session.caret(), 2);
        handle_key(&mut session, key(KeyCode::Right));
        assert_eq!(session.caret(), 3);

        // And they stop at the ends rather than wrapping or panicking.
        handle_key(&mut session, key(KeyCode::Right));
        assert_eq!(session.caret(), 3);
        for _ in 0..5 {
            handle_key(&mut session, key(KeyCode::Left));
        }
        assert_eq!(session.caret(), 0);
    }

    /// The bug this exists for: typing used to only ever append, so a correction meant deleting
    /// back to it and retyping the rest.
    #[test]
    fn typing_lands_where_the_caret_is() {
        let mut session = typed_into("ac");
        handle_key(&mut session, key(KeyCode::Left));
        handle_key(&mut session, key(KeyCode::Char('b')));
        assert_eq!(session.input(), "abc");
        assert_eq!(
            session.caret(),
            2,
            "the caret did not follow what was typed"
        );
    }

    /// Home and End reach the ends of the line, which is what they do in every other text field.
    #[test]
    fn home_and_end_reach_the_ends_of_the_line() {
        let mut session = typed_into("a sentence");
        handle_key(&mut session, key(KeyCode::Home));
        assert_eq!(session.caret(), 0);
        assert_eq!(session.scroll, 0, "the transcript scrolled instead");
        handle_key(&mut session, key(KeyCode::End));
        assert_eq!(session.caret(), "a sentence".len());
    }

    /// Backspace deletes before the caret rather than at the end of the line.
    #[test]
    fn backspace_deletes_at_the_caret() {
        let mut session = typed_into("abc");
        handle_key(&mut session, key(KeyCode::Left));
        handle_key(&mut session, key(KeyCode::Backspace));
        assert_eq!(session.input(), "ac");
        assert_eq!(session.caret(), 1);
    }

    /// Delete takes the character after the caret, which is the half Backspace cannot reach.
    #[test]
    fn delete_takes_the_character_after_the_caret() {
        let mut session = typed_into("abc");
        handle_key(&mut session, key(KeyCode::Home));
        handle_key(&mut session, key(KeyCode::Delete));
        assert_eq!(session.input(), "bc");
        assert_eq!(session.caret(), 0);
    }

    /// A word at a time, since a path or a flag is one thing to cross rather than a dozen.
    #[test]
    fn the_word_keys_cross_a_word_at_a_time() {
        let mut session = typed_into("read src/main.rs now");
        handle_key(&mut session, ctrl_key(KeyCode::Left));
        assert_eq!(session.caret(), "read src/main.rs ".len());
        handle_key(&mut session, ctrl_key(KeyCode::Left));
        assert_eq!(session.caret(), "read ".len());
        handle_key(&mut session, ctrl_key(KeyCode::Right));
        assert_eq!(session.caret(), "read src/main.rs".len());
    }

    /// The readline bindings, because a terminal may send nothing at all for the named keys and
    /// then the middle of a line would be unreachable.
    #[test]
    fn the_readline_bindings_move_and_delete_too() {
        let mut session = typed_into("some words here");
        handle_key(&mut session, ctrl('a'));
        assert_eq!(session.caret(), 0);
        handle_key(&mut session, ctrl('e'));
        assert_eq!(session.caret(), "some words here".len());

        handle_key(&mut session, ctrl('w'));
        assert_eq!(session.input(), "some words ");
        handle_key(&mut session, ctrl('u'));
        assert_eq!(session.input(), "");
    }

    /// Ctrl-K takes the rest of the line, which is the other half of Ctrl-U.
    #[test]
    fn ctrl_k_takes_the_rest_of_the_line() {
        let mut session = typed_into("keep this drop that");
        for _ in 0..2 {
            handle_key(&mut session, ctrl_key(KeyCode::Left));
        }
        handle_key(&mut session, ctrl('k'));
        assert_eq!(session.input(), "keep this ");
    }

    /// The keys that used to be typed as characters, and now must not be: Ctrl-A on an empty line
    /// once inserted a literal 'a'.
    #[test]
    fn the_editing_keys_are_not_typed_as_characters() {
        let mut session = Session::new("none");
        for binding in ['a', 'e', 'b', 'f', 'w', 'u', 'k'] {
            handle_key(&mut session, ctrl(binding));
        }
        assert!(
            session.input().is_empty(),
            "a binding was typed: {}",
            session.input()
        );
    }

    /// A pasted paragraph has rows, and Up and Down move between them before they reach for the
    /// history: a line the user can see is the one they meant to edit.
    #[test]
    fn up_and_down_move_within_a_pasted_paragraph() {
        let mut session = Session::new("none");
        handle_paste(&mut session, "first line\nsecond row");
        assert_eq!(handle_key(&mut session, key(KeyCode::Up)), Action::Redraw);
        assert_eq!(session.caret(), "first line".len());
        handle_key(&mut session, key(KeyCode::Down));
        assert_eq!(session.caret(), "first line\nsecond row".len());
    }

    /// The caret keeps its place along the line, and clamps to the end of a shorter one rather
    /// than off it.
    #[test]
    fn moving_between_lines_keeps_the_place_along_them() {
        let mut session = Session::new("none");
        handle_paste(&mut session, "a longer first line\nshort\nlast");
        handle_key(&mut session, key(KeyCode::Home));
        for _ in 0..3 {
            handle_key(&mut session, key(KeyCode::Right));
        }
        handle_key(&mut session, key(KeyCode::Up));
        assert_eq!(session.caret(), "a longer first line\nsho".len());

        // "last" is shorter than where the caret is, so it clamps to the end of it.
        handle_key(&mut session, key(KeyCode::End));
        handle_key(&mut session, key(KeyCode::Down));
        handle_key(&mut session, key(KeyCode::Down));
        assert_eq!(session.caret(), session.input().len());
    }

    /// Off the top of the line, Up is the history's again, so a one-line prompt is unaffected and
    /// a paragraph does not trap the keys.
    #[test]
    fn up_reaches_history_from_the_top_of_the_line() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('a')));
        handle_key(&mut session, key(KeyCode::Enter));
        session.complete("ok", Vec::new(), 0);

        handle_paste(&mut session, "one\ntwo");
        handle_key(&mut session, key(KeyCode::Up));
        handle_key(&mut session, key(KeyCode::Up));
        assert!(session.history.is_browsing(), "history was unreachable");
    }

    /// The line can be typed mid-turn, so it has to be editable mid-turn: a box that takes words
    /// and will not let them be fixed is worse than one that takes none.
    #[test]
    fn the_caret_moves_while_a_turn_runs() {
        let mut session = typed_into("ac");
        handle_key(&mut session, key(KeyCode::Enter));
        for c in "ac".chars() {
            handle_key_while_working(&mut session, key(KeyCode::Char(c)));
        }

        handle_key_while_working(&mut session, key(KeyCode::Left));
        handle_key_while_working(&mut session, key(KeyCode::Char('b')));
        assert_eq!(session.input(), "abc");
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
        assert_eq!(session.input(), "hello", "scrolling altered the input");
    }

    #[test]
    fn backspace_deletes() {
        let mut session = Session::new("none");
        handle_key(&mut session, key(KeyCode::Char('a')));
        handle_key(&mut session, key(KeyCode::Backspace));
        assert!(session.input().is_empty());
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

    /// A directory a settings file named is opened and vouched for by the same route `/add-dir`
    /// takes, so neither has a way in that the other lacks. A relative name means a path under the
    /// workspace, which is what `../shared` in a file about a project says.
    #[test]
    fn a_settings_file_directory_is_opened_and_trusted_like_one_typed() {
        let root = std::env::temp_dir().join("bravebot-settings-dir-test");
        let outside = root.join("shared");
        let project = root.join("project");
        std::fs::create_dir_all(&outside).expect("scratch");
        std::fs::create_dir_all(&project).expect("scratch");

        let mut workspace = Workspace::new(&project).expect("workspace");
        let mut session = Session::new("none");
        let mut trust = TrustStore::new();

        // Named the way a settings file names it, relative to the project.
        let named = against_workspace(workspace.root(), "../shared");
        add_directory(&mut session, &mut workspace, &mut trust, &named);

        // Both halves, since either alone is useless: reach without trust asks about every write
        // there, and trust without reach is a rule about files nothing can open.
        let canonical = outside.canonicalize().expect("canonical");
        assert!(
            workspace.added_directories().contains(&canonical),
            "a directory a settings file named was not opened"
        );
        assert!(
            trust.is_trusted(&canonical.display().to_string()),
            "a directory a settings file named was not vouched for"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// An absolute name is left alone. Joining it onto the workspace would name a path inside the
    /// project that nobody asked for.
    #[test]
    fn an_absolute_directory_in_a_settings_file_is_not_joined_onto_the_workspace() {
        assert_eq!(
            against_workspace(std::path::Path::new("/tmp/project"), "/opt/other"),
            "/opt/other"
        );
    }
}
