//! Moving text and pictures between the terminal and the user's clipboard.
//!
//! Copying goes out two ways, tried in order, because neither works everywhere. The platform's own
//! tool is what works locally and is always there on macOS and Windows; the escape sequence is what
//! works over ssh, where the clipboard that matters belongs to the terminal at the other end and no
//! local tool can reach it.
//!
//! Pasting comes back in one way, and only because the terminal cannot do it. Command-V on macOS
//! never reaches this process at all: the byte stream over a pty has no encoding for that modifier,
//! and the terminal claims the chord for itself in any case. What the terminal does instead is
//! write the clipboard's *text* into the pty, and an image has no text, so the picture a user is
//! looking at is the one thing the ordinary paste cannot carry. Reading the clipboard here goes
//! around the pty entirely, which is why Control-V can move what Command-V cannot.
//!
//! Nothing labelled passes through here. What is copied was read off the screen, and everything on
//! the screen was released for display before it was drawn. What is pasted is the user's own input,
//! on the footing of the prompt it lands in, which
//! [`bravebot_core::policy::Policy::admit_pasted_image`] states in full.

use std::io::Write;
use std::process::{Command, Stdio};

/// Put `text` on the clipboard, reporting whether anything took it.
pub fn copy(text: &str) -> bool {
    for (program, arguments) in COPY_TOOLS {
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
const COPY_TOOLS: &[(&str, &[&str])] = &[("pbcopy", &[])];

#[cfg(target_os = "windows")]
const COPY_TOOLS: &[(&str, &[&str])] = &[("clip", &[])];

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const COPY_TOOLS: &[(&str, &[&str])] = &[
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

/// The largest image a paste will carry.
///
/// Not a policy rule: nothing about a big picture is unsafe, it is that encoding one into a request
/// costs a third again in base64 and a screenshot of a large display already runs to several
/// megabytes. The refusal happens here, where the user is still looking at the paste that caused
/// it, rather than at an endpoint that would answer with a number.
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// What the clipboard had when it was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pasted {
    Text(String),
    Image(Image),
    /// A picture too big to send, with the size it would have been.
    TooLarge(usize),
    /// Nothing this can use: an empty clipboard, or one holding something that is neither.
    Nothing,
}

/// A picture off the clipboard, in a form the API takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Always one of this module's own constants, never a string read from anywhere.
    ///
    /// It ends up in the data URL, where it is routing. Taking it from a filename or from what a
    /// tool printed would be letting whoever wrote that choose it.
    pub media_type: &'static str,
    pub bytes: Vec<u8>,
}

/// Read the clipboard.
///
/// A picture wins over text when the clipboard holds both, which it often does: copying an image in
/// a browser leaves the page's URL as the text flavour, and copying a spreadsheet range leaves a
/// rendering of it. The tie is broken that way because text has another route in and a picture has
/// none. Command-V still pastes the text flavour, works everywhere, and is what the fingers already
/// know, so preferring it here would leave one of the two flavours reachable by nothing at all.
pub fn paste() -> Pasted {
    match image_on_clipboard() {
        Some((_, bytes)) if bytes.len() > MAX_IMAGE_BYTES => return Pasted::TooLarge(bytes.len()),
        Some((media_type, bytes)) => return Pasted::Image(Image { media_type, bytes }),
        None => {}
    }

    match text_on_clipboard() {
        Some(text) if !text.is_empty() => Pasted::Text(text),
        _ => Pasted::Nothing,
    }
}

/// Whether the clipboard has a picture on it, without reading the picture.
///
/// Asked so the interface can say that Control-V would do something, which is the whole of how
/// anyone finds out: a chord nothing mentions is a chord nobody presses. It asks the platform for
/// the list of flavours rather than for the bytes, since a screenshot runs to megabytes and
/// fetching one to answer a yes-or-no question would be felt.
///
/// The comparison is against a literal, and what it yields is a constant. Whoever filled the
/// clipboard therefore chooses whether a hint appears and nothing else: not the media type, which
/// this module owns, and not where anything lands.
pub fn holds_an_image() -> bool {
    image_flavour_on_clipboard()
}

#[cfg(target_os = "macos")]
mod platform {
    use super::run_text;

    /// The pasteboard offers a coercion to PNG for anything it holds as a picture, so one flavour
    /// covers screenshots, browsers, and image editors alike. AppleScript hands it back as hex
    /// inside a wrapper, which is the price of there being no `pbpaste` for pictures.
    pub fn image() -> Option<(&'static str, Vec<u8>)> {
        let script = "the clipboard as «class PNGf»";
        let wrapped = run_text("osascript", &["-e", script])?;
        let hex = wrapped
            .trim()
            .strip_prefix("«data PNGf")?
            .strip_suffix('»')?;
        Some(("image/png", from_hex(hex)?))
    }

    /// Asked about the one flavour that matters rather than for the whole list, because sizing a
    /// flavour materialises it: a clipboard holding a screenshot offers the same picture eight ways,
    /// and asking what is on it costs the better part of a second. Asking about PNG alone is under a
    /// tenth of that, which is what makes this affordable on a focus change.
    pub fn has_image() -> bool {
        run_text("osascript", &["-e", "clipboard info for «class PNGf»"])
            .is_some_and(|info| info.contains("«class PNGf»"))
    }

    pub const TEXT_TOOLS: &[(&str, &[&str])] = &[("pbpaste", &[])];

    /// Bytes back out of the hex AppleScript writes them as.
    ///
    /// An odd length or a stray character means the wrapper was not what this expected, and half a
    /// picture is worse than none, so the whole thing is dropped rather than truncated.
    fn from_hex(hex: &str) -> Option<Vec<u8>> {
        if !hex.len().is_multiple_of(2) {
            return None;
        }
        hex.as_bytes()
            .chunks(2)
            .map(|pair| {
                let digits = std::str::from_utf8(pair).ok()?;
                u8::from_str_radix(digits, 16).ok()
            })
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The wrapper is the only thing standing between AppleScript's hex and a PNG, so the
        /// first bytes of one have to come back exactly.
        #[test]
        fn the_hex_applescript_writes_comes_back_as_bytes() {
            assert_eq!(
                from_hex("89504E470D0A1A0A"),
                Some(vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
            );
        }

        /// Half a picture is worse than none: it would be sent, rejected by the endpoint, and
        /// reported as a fault of the request rather than of the read that truncated it.
        #[test]
        fn a_wrapper_that_is_not_what_was_expected_yields_no_picture() {
            assert_eq!(
                from_hex("89504E470D0A1A0"),
                None,
                "an odd length was accepted"
            );
            assert_eq!(
                from_hex("89504E47zzzz1A0A"),
                None,
                "a stray character was accepted"
            );
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::run_text;

    /// PowerShell renders whatever picture the clipboard holds as PNG and prints it base64, since
    /// binary down a pipe from PowerShell is mangled by the console encoding.
    const SCRIPT: &str = "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; \
                          $image = [Windows.Forms.Clipboard]::GetImage(); \
                          if ($image) { $stream = New-Object IO.MemoryStream; \
                          $image.Save($stream, [Drawing.Imaging.ImageFormat]::Png); \
                          [Convert]::ToBase64String($stream.ToArray()) }";

    pub fn image() -> Option<(&'static str, Vec<u8>)> {
        use base64::Engine;
        let encoded = run_text("powershell", &["-NoProfile", "-Command", SCRIPT])?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .ok()?;
        (!bytes.is_empty()).then_some(("image/png", bytes))
    }

    /// No cheaper answer than the picture itself here, so this reads it and throws it away. Only
    /// asked when the terminal regains focus, which is rare enough to afford it.
    pub fn has_image() -> bool {
        image().is_some()
    }

    pub const TEXT_TOOLS: &[(&str, &[&str])] =
        &[("powershell", &["-NoProfile", "-Command", "Get-Clipboard"])];
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::{run_bytes, run_text};

    /// Both display servers, since which one is running is not something this can assume and a tool
    /// for the other simply fails to spawn.
    const IMAGE_TOOLS: &[(&str, &[&str])] = &[
        ("wl-paste", &["--no-newline", "--type", "image/png"]),
        (
            "xclip",
            &["-selection", "clipboard", "-t", "image/png", "-o"],
        ),
    ];

    const FLAVOUR_TOOLS: &[(&str, &[&str])] = &[
        ("wl-paste", &["--list-types"]),
        ("xclip", &["-selection", "clipboard", "-t", "TARGETS", "-o"]),
    ];

    pub fn image() -> Option<(&'static str, Vec<u8>)> {
        for (program, arguments) in IMAGE_TOOLS {
            match run_bytes(program, arguments) {
                Some(bytes) if !bytes.is_empty() => return Some(("image/png", bytes)),
                _ => {}
            }
        }
        None
    }

    pub fn has_image() -> bool {
        FLAVOUR_TOOLS.iter().any(|(program, arguments)| {
            run_text(program, arguments).is_some_and(|types| types.contains("image/png"))
        })
    }

    pub const TEXT_TOOLS: &[(&str, &[&str])] = &[
        ("wl-paste", &["--no-newline"]),
        ("xclip", &["-selection", "clipboard", "-o"]),
        ("xsel", &["--clipboard", "--output"]),
    ];
}

fn image_on_clipboard() -> Option<(&'static str, Vec<u8>)> {
    platform::image()
}

fn image_flavour_on_clipboard() -> bool {
    platform::has_image()
}

/// The clipboard's text, from the first tool that has any.
///
/// A tool that is not installed fails to spawn, which is the next one's turn, exactly as copying
/// works. A tool that runs and finds nothing has answered, and the answer is that there is nothing.
fn text_on_clipboard() -> Option<String> {
    for (program, arguments) in platform::TEXT_TOOLS {
        if let Some(text) = run_text(program, arguments) {
            return Some(text);
        }
    }
    None
}

/// Run a tool and take what it printed, or nothing if it could not run or refused.
///
/// A non-zero exit is a refusal: `osascript` returns one when the clipboard holds no picture, which
/// is the ordinary case and not a fault worth reporting.
fn run_bytes(program: &str, arguments: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn run_text(program: &str, arguments: &[&str]) -> Option<String> {
    String::from_utf8(run_bytes(program, arguments)?).ok()
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

    /// A machine with none of these tools installed is the ordinary case on a bare server, and it
    /// has to read as an empty clipboard rather than as anything a caller must handle.
    #[test]
    fn a_missing_tool_reads_as_nothing_on_the_clipboard() {
        assert_eq!(run_text("a-clipboard-tool-that-does-not-exist", &[]), None);
    }
}
