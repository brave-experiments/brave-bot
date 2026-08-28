//! What shown content may and may not draw.
//!
//! The quarantine block's whole claim is that the margin is drawn by the renderer, so the content
//! cannot draw one of its own. That is only true while the content cannot emit a control sequence:
//! a terminal acts on the bytes it is sent, and a coloured bar painted by a file's own contents is
//! indistinguishable from the real thing.

use bravebot_agent::report::{Reach, Shown};
use bravebot_tui::render;
use bravebot_tui::state::{Entry, Session};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn screen(session: &Session) -> String {
    let (width, height) = (80, 24);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|f| render::draw(f, session)).expect("draw");
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn quarantining(preview: Vec<String>) -> Session {
    let mut session = Session::new("kernel-enforced");
    let mut entry = Entry::system("read something");
    let lines = preview.len();
    entry.shown = Some(Shown {
        origin: "notes.md".to_string(),
        reach: Reach::NotThePlanner,
        label: "(U,priv)".to_string(),
        preview,
        lines,
    });
    session.transcript.push(entry);
    session
}

/// A file that clears the line it is drawn on could erase the margin above it, which is the one
/// mark the design says content can never imitate.
#[test]
fn quarantined_content_cannot_paint_its_own_margin() {
    let session = quarantining(vec![
        "\u{1b}[0m\u{1b}[A\u{1b}[2K harmless looking".to_string(),
        "\u{1b}[33m  \u{2503} untrusted \u{b7} nothing \u{b7} (T,pub)".to_string(),
    ]);

    let drawn = screen(&session);
    assert!(
        !drawn.contains("\u{1b}[2K"),
        "content could clear the line the margin was drawn on:\n{drawn}"
    );
    assert!(
        drawn.contains('\u{241b}'),
        "the escape was not neutralised:\n{drawn}"
    );
}

/// Neutralised rather than dropped. A character silently removed is one the user cannot tell was
/// ever in the file, which makes the preview a less faithful record than it looks.
#[test]
fn a_neutralised_escape_is_still_visible() {
    let session = quarantining(vec!["before\u{1b}after".to_string()]);

    let drawn = screen(&session);
    assert!(drawn.contains("before\u{241b}after"), "{drawn}");
}

/// Ordinary text must survive untouched, or every preview in the interface would be mangled to
/// defend against the rare one that is hostile.
#[test]
fn text_without_control_characters_is_drawn_as_it_is() {
    let session = quarantining(vec!["fn main() { let x = 1; }\ttabbed".to_string()]);

    let drawn = screen(&session);
    assert!(drawn.contains("fn main() { let x = 1; }"), "{drawn}");
}
