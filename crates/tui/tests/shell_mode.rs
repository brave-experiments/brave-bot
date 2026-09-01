//! What shell mode looks like on a real frame.
//!
//! The marker and the hint are the whole of how a user knows the line is going to a shell rather
//! than to the model, so they are asserted against the drawn buffer rather than against the state
//! that feeds it.

use bravebot_tui::render;
use bravebot_tui::state::{Entry, Session};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// The drawn screen, as one string per row, so a marker at the start of a line can be found.
fn rows(session: &Session, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|f| {
            render::draw(f, session);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

fn screen(session: &Session) -> String {
    rows(session, 80, 24).join("\n")
}

/// The mode has to be discoverable, or a user who never learns it never learns it is there. The
/// hint line no longer lists any binding, so the shortcuts are where this is answered, and the `!`
/// has to be among them.
#[test]
fn the_shortcuts_offer_shell_mode() {
    let mut session = Session::new("kernel-enforced");
    session.type_char('?');
    let drawn = rows(&session, 100, 40).join("\n");
    assert!(
        drawn.contains("run a shell command"),
        "a user has no way to discover the mode:\n{drawn}"
    );
}

/// The prompt marker is how a user knows where the line is going. Drawn as `!` rather than `>`, and
/// coloured, because pressing Enter on the wrong one runs something.
#[test]
fn the_prompt_marker_changes_in_shell_mode() {
    let mut session = Session::new("kernel-enforced");
    session.shell = true;
    session.type_char('l');
    session.type_char('s');

    let drawn = rows(&session, 80, 24);
    assert!(
        drawn.iter().any(|row| row.contains("! ls")),
        "the command was not drawn behind the marker:\n{}",
        drawn.join("\n")
    );
    assert!(
        !drawn.iter().any(|row| row.contains("> ls")),
        "it was drawn as an ordinary prompt:\n{}",
        drawn.join("\n")
    );
}

/// In shell mode the usual bindings are beside the point, and which shell is about to run the line
/// is the thing a user cannot otherwise find out.
#[test]
fn the_hint_names_the_shell_while_in_shell_mode() {
    let mut session = Session::new("kernel-enforced");
    session.shell = true;

    let drawn = screen(&session);
    assert!(
        drawn.contains("esc to cancel"),
        "no way out was offered:\n{drawn}"
    );
    assert!(
        drawn.contains("/") && drawn.contains("sh"),
        "the shell was not named:\n{drawn}"
    );
}

/// A command and its output read as one thing in the scrollback, the command behind the marker it
/// was typed behind.
#[test]
fn a_command_and_its_output_are_shown_together() {
    let mut session = Session::new("kernel-enforced");
    session.transcript.push(Entry::shell("echo hello"));
    session.transcript.push(Entry::output("hello"));

    let drawn = rows(&session, 80, 24);
    let command = drawn
        .iter()
        .position(|row| row.contains("! echo hello"))
        .expect("the command was not shown");
    let output = drawn
        .iter()
        .position(|row| row.trim() == "hello")
        .expect("the output was not shown");
    assert!(
        output > command,
        "the output was not beneath the command:\n{}",
        drawn.join("\n")
    );
}

/// Output is trusted, because the user typed the command that produced it. Drawing it inside the
/// quarantine margin would tell them the opposite of what the kernel decided.
#[test]
fn output_is_not_drawn_as_quarantined() {
    let mut session = Session::new("kernel-enforced");
    session.transcript.push(Entry::shell("cat notes"));
    session.transcript.push(Entry::output("some text"));

    let drawn = screen(&session);
    assert!(
        !drawn.contains("untrusted"),
        "output the user vouched for was marked untrusted:\n{drawn}"
    );
    assert!(
        !drawn.contains('\u{2503}'),
        "the quarantine bar was drawn down the user's own output:\n{drawn}"
    );
}

/// The interface draws its own structure, so text must not be able to draw any of it. An escape
/// sequence would let a file's contents recolour the screen or reposition the cursor, and a forged
/// margin is worse than none: drawing one is only worth anything if content cannot.
#[test]
fn output_cannot_draw_its_own_escapes() {
    let mut session = Session::new("kernel-enforced");
    session.transcript.push(Entry::shell("cat hostile"));
    session
        .transcript
        .push(Entry::output("\u{1b}[31mred\u{1b}[0m and \u{1b}[2Jcleared"));

    let drawn = screen(&session);
    assert!(
        !drawn.contains('\u{1b}'),
        "an escape reached the terminal:\n{drawn}"
    );
    // Shown rather than dropped, so a user can tell the bytes were there.
    assert!(
        drawn.contains('\u{241b}'),
        "the escape was silently swallowed:\n{drawn}"
    );
}

/// A command that printed nothing has to say so. A blank gap would look like a command that never
/// ran, which is the one thing the user cannot tell by looking.
#[test]
fn a_command_that_printed_nothing_says_so() {
    let mut session = Session::new("kernel-enforced");
    session.printed("");

    assert!(
        screen(&session).contains("no output"),
        "silence was indistinguishable from a command that did not run"
    );
}

/// While a command runs the box holds a spinner and the way to stop it, rather than an idle prompt
/// that looks ready for input it would not accept.
#[test]
fn a_running_command_shows_that_it_is_running() {
    let mut session = Session::new("kernel-enforced");
    session.begin_command();

    let drawn = screen(&session);
    assert!(
        drawn.contains("running"),
        "no sign it was working:\n{drawn}"
    );
    assert!(drawn.contains("esc to stop"), "no way to stop it:\n{drawn}");
}
