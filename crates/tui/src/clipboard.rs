//! Putting selected text on the user's clipboard.
//!
//! Two ways, tried in order, because neither works everywhere. The platform's own tool is what
//! works locally and is always there on macOS and Windows; the escape sequence is what works
//! over ssh, where the clipboard that matters belongs to the terminal at the other end and no
//! local tool can reach it.
//!
//! Nothing labelled passes through here. What is copied was read off the screen, and everything
//! on the screen was released for display before it was drawn.

use std::io::Write;
use std::process::{Command, Stdio};

/// Put `text` on the clipboard, reporting whether anything took it.
pub fn copy(text: &str) -> bool {
    for (program, arguments) in TOOLS {
        if pipe_into(program, arguments, text) {
            return true;
        }
    }
    write_escape_sequence(text)
}

/// The clipboard tools worth trying, in the order they are worth trying.
///
/// One per platform that has a certain one, and on the rest the three that a desktop session
/// might have. A tool that is not installed fails to spawn, which is the next one's turn.
#[cfg(target_os = "macos")]
const TOOLS: &[(&str, &[&str])] = &[("pbcopy", &[])];

#[cfg(target_os = "windows")]
const TOOLS: &[(&str, &[&str])] = &[("clip", &[])];

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const TOOLS: &[(&str, &[&str])] = &[
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
];

/// Run a tool and write the text to it, reporting whether it took it.
fn pipe_into(program: &str, arguments: &[&str], text: &str) -> bool {
    let Ok(mut child) = Command::new(program)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };

    // Taken rather than borrowed so the pipe closes here: a tool that reads to end of input
    // would otherwise wait for a handle this process is still holding.
    if let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(text.as_bytes()).is_err()
    {
        let _ = child.wait();
        return false;
    }

    matches!(child.wait(), Ok(status) if status.success())
}

/// Ask the terminal itself to take the text, with OSC 52.
///
/// The fallback for a machine with no clipboard tool, and the only thing that works over ssh,
/// since it is the terminal at the near end that holds the clipboard the user will paste from.
/// Terminals that do not implement it ignore the sequence, and there is no reply to tell the two
/// apart, so this reports what it managed to write and not what the terminal did with it.
fn write_escape_sequence(text: &str) -> bool {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);

    let mut out = std::io::stdout();
    write!(out, "\x1b]52;c;{encoded}\x07").is_ok() && out.flush().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sequence has to be exactly what a terminal recognises, and a test is the only place
    /// that can say so, since a terminal that does not recognise it says nothing at all.
    #[test]
    fn the_escape_sequence_is_the_one_terminals_read() {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode("hello");
        assert_eq!(encoded, "aGVsbG8=");
        assert_eq!(format!("\x1b]52;c;{encoded}\x07"), "\x1b]52;c;aGVsbG8=\x07");
    }

    /// A tool that is not installed is not a failure to copy, it is the next tool's turn.
    #[test]
    fn a_missing_tool_is_not_taken_for_a_successful_copy() {
        assert!(!pipe_into(
            "a-clipboard-tool-that-does-not-exist",
            &[],
            "hello"
        ));
    }
}
