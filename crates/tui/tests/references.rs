//! File references typed with `@`, against a real directory.
//!
//! The property that matters is at the end: a file named with `@` reaches the turn as **trusted**
//! context, because the user typed the path. These check the way there, since a completion that
//! offered the wrong file would be admitting the wrong contents.

use bravebot_tui::Session;
use bravebot_tui::app::{Action, handle_key, handle_paste};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("bravebot-references-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("src")).expect("create");
        std::fs::write(path.join("Cargo.toml"), "[package]").expect("write");
        std::fs::write(path.join("README.md"), "# notes").expect("write");
        std::fs::write(path.join("src/main.rs"), "fn main() {}").expect("write");
        Self { path }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn typing(session: &mut Session, text: &str) {
    for c in text.chars() {
        handle_key(session, key(KeyCode::Char(c)));
    }
}

fn session(scratch: &Scratch) -> Session {
    Session::new("none").in_workspace(&scratch.path)
}

/// An `@` offers what is in the workspace, so a user can see which files they may admit.
#[test]
fn an_at_sign_offers_the_workspace() {
    let scratch = Scratch::new("offer");
    let mut session = session(&scratch);
    typing(&mut session, "look at @");

    assert!(session.is_completing(), "nothing was offered");
    let offered: Vec<String> = match session.offered() {
        bravebot_tui::state::Offered::Files(entries) => {
            entries.into_iter().map(|e| e.path).collect()
        }
        other => panic!("files were not offered: {other:?}"),
    };
    assert_eq!(
        offered,
        vec![
            "src/".to_string(),
            "Cargo.toml".to_string(),
            "README.md".to_string()
        ],
        "directories come first, then files, each alphabetical"
    );
}

/// Tab completes the reference in place, leaving the sentence it was written into alone.
#[test]
fn tab_completes_a_reference_without_disturbing_the_sentence() {
    let scratch = Scratch::new("tab");
    let mut session = session(&scratch);
    typing(&mut session, "please read @READ");

    assert_eq!(handle_key(&mut session, key(KeyCode::Tab)), Action::Redraw);
    assert_eq!(session.input, "please read @README.md ");
}

/// A directory completes without a trailing space, so the path can be typed onwards into it. A file
/// gets one, since the reference is finished.
#[test]
fn a_directory_completes_so_typing_can_continue_into_it() {
    let scratch = Scratch::new("walk");
    let mut session = session(&scratch);
    typing(&mut session, "@sr");
    handle_key(&mut session, key(KeyCode::Tab));
    assert_eq!(session.input, "@src/");

    typing(&mut session, "mai");
    handle_key(&mut session, key(KeyCode::Tab));
    assert_eq!(session.input, "@src/main.rs ");
}

/// The arrows walk the offered files, and Enter takes one rather than sending a half-typed path.
#[test]
fn the_arrows_and_enter_choose_among_the_offered_files() {
    let scratch = Scratch::new("choose");
    let mut session = session(&scratch);
    typing(&mut session, "@");

    handle_key(&mut session, key(KeyCode::Down));
    handle_key(&mut session, key(KeyCode::Down));
    assert_eq!(
        handle_key(&mut session, key(KeyCode::Enter)),
        Action::Redraw
    );
    assert_eq!(session.input, "@README.md ", "the third entry");
    assert!(
        session.transcript.is_empty(),
        "a half-typed reference was sent"
    );
}

/// A reference finished by a space closes the list, so the arrows go back to history and scrolling
/// and the rest of the sentence can be typed.
#[test]
fn a_finished_reference_closes_the_list() {
    let scratch = Scratch::new("closed");
    let mut session = session(&scratch);
    typing(&mut session, "@README.md ");
    assert!(!session.is_completing(), "the list stayed open");

    typing(&mut session, "what does it say");
    assert!(!session.is_completing());
}

/// The completion must not become a way to browse the filesystem.
#[test]
fn a_reference_cannot_climb_out_of_the_workspace() {
    let scratch = Scratch::new("escape");
    let mut session = session(&scratch);
    typing(&mut session, "@../");
    assert!(!session.is_completing(), "a path outside was offered");

    session.clear_input();
    typing(&mut session, "@/etc/");
    assert!(!session.is_completing(), "an absolute path was offered");
}

/// A paste can narrow the list under a cursor that was further down, and reading the highlighted
/// row must still name something rather than pointing past the end.
#[test]
fn a_cursor_past_the_end_of_a_narrowed_list_still_names_a_file() {
    let scratch = Scratch::new("narrow");
    let mut session = session(&scratch);
    typing(&mut session, "@");
    for _ in 0..2 {
        handle_key(&mut session, key(KeyCode::Down));
    }

    handle_paste(&mut session, "Car");
    handle_key(&mut session, key(KeyCode::Tab));
    assert_eq!(session.input, "@Cargo.toml ");
}

/// An ordinary sentence containing an address is not a reference: only a word beginning with `@`
/// is, and only while it is the one being typed.
#[test]
fn an_address_in_a_sentence_is_not_a_reference() {
    let scratch = Scratch::new("address");
    let mut session = session(&scratch);
    typing(&mut session, "mail me@example.com");
    assert!(!session.is_completing());
}

/// A directory is somewhere to type through, not a file to read, so naming one includes nothing.
#[test]
fn a_directory_reference_is_not_included_as_a_file() {
    assert!(bravebot_tui::entries::referenced("look in @src/").is_empty());
}

/// A prompt ending in a finished reference sends on Enter. It reads as still being completed, since
/// the last word is what the list is about, but the sentence is done: completing there left a user
/// pressing Enter twice to say something perfectly well formed.
#[test]
fn enter_sends_a_prompt_that_ends_in_a_finished_reference() {
    let scratch = Scratch::new("finished");
    let mut session = session(&scratch);
    typing(&mut session, "explain @README.md");

    assert!(session.is_completing(), "the list is about the last word");
    assert!(
        !session.completion_would_change_the_line(),
        "taking the offer would change nothing, so Enter must send"
    );
    assert_eq!(
        handle_key(&mut session, key(KeyCode::Enter)),
        Action::Submit("explain @README.md".to_string())
    );
}

/// And one ending in a half-typed reference completes instead, which is the case that made Enter
/// worth overloading at all.
#[test]
fn enter_completes_a_prompt_that_ends_in_a_half_typed_reference() {
    let scratch = Scratch::new("half");
    let mut session = session(&scratch);
    typing(&mut session, "explain @READ");

    assert_eq!(
        handle_key(&mut session, key(KeyCode::Enter)),
        Action::Redraw
    );
    assert_eq!(session.input, "explain @README.md ");
}

/// The reading the event loop does when it builds a turn, kept beside the typing tests so the two
/// cannot drift. That the contents then reach the model as trusted context is checked against a real
/// turn in the agent crate, where the streaming harness lives:
/// `a_turn_includes_referenced_file_contents`.
#[test]
fn the_files_a_submitted_line_would_include() {
    let scratch = Scratch::new("collect");
    let mut session = session(&scratch);
    typing(&mut session, "compare @src/main.rs with @README.md");

    let prompt = match handle_key(&mut session, key(KeyCode::Enter)) {
        Action::Submit(prompt) => prompt,
        other => panic!("the line was not sent: {other:?}"),
    };
    assert_eq!(
        bravebot_tui::entries::referenced(&prompt),
        vec!["src/main.rs".to_string(), "README.md".to_string()]
    );
}
