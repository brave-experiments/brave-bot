//! Dropping a file on the box, end to end through the session.
//!
//! Real files in a real directory, because the whole question a drop asks is whether a path names
//! something, and a fake filesystem would answer it for free.

use bravebot_tui::dropped::{Kind, Reach};
use bravebot_tui::state::Session;
use std::path::PathBuf;

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("bravebot-drop-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create scratch");
        Self { path }
    }

    fn file(&self, name: &str) -> String {
        let at = self.path.join(name);
        std::fs::write(&at, [0x89u8, 0x50]).expect("write");
        at.to_string_lossy().to_string()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn session_in(scratch: &Scratch) -> Session {
    Session::new("none")
        .in_workspace(&scratch.path)
        .reaching(Reach::of(&scratch.path, &[]))
}

/// The behaviour asked for: a supported type inside the workspace becomes a marker.
#[test]
fn dropping_an_image_puts_a_marker_in_the_line() {
    let scratch = Scratch::new("image");
    let path = scratch.file("shot.png");
    let mut session = session_in(&scratch);

    assert!(session.drop_files(&path), "not recognised as a drop");
    assert_eq!(session.input(), "[Image #1] ");
    assert_eq!(session.attached().len(), 1);
    assert_eq!(session.attached()[0].name, "shot.png");
    assert_eq!(session.attached()[0].kind, Kind::Attachment("image/png"));
}

/// And the other half of it: an unsupported type writes out its path, as dropping one always did.
#[test]
fn dropping_an_unsupported_type_writes_out_the_path() {
    let scratch = Scratch::new("dmg");
    let path = scratch.file("installer.dmg");
    let mut session = session_in(&scratch);

    assert!(session.drop_files(&path));
    assert_eq!(session.input(), format!("{path} "));
    assert!(session.attached().is_empty(), "a .dmg was attached");
}

/// The name given to the task is workspace-relative, because that is the only form the workspace
/// resolves against its root. An absolute path there is refused.
#[test]
fn the_name_handed_to_the_task_is_relative_to_the_workspace() {
    let scratch = Scratch::new("relative");
    std::fs::create_dir_all(scratch.path.join("shots")).unwrap();
    let path = scratch.file("shots/a.png");
    let mut session = session_in(&scratch);

    session.drop_files(&path);
    assert_eq!(session.attached()[0].name, "shots/a.png");
}

/// A drop from outside the workspace names a file the turn would refuse to open, so the path is
/// written out instead of a marker that would fail later.
#[test]
fn a_drop_from_outside_the_workspace_writes_out_its_path() {
    let outside = Scratch::new("outside");
    let path = outside.file("shot.png");

    let workspace = Scratch::new("inside");
    let mut session = session_in(&workspace);

    assert!(session.drop_files(&path));
    assert_eq!(session.input(), format!("{path} "));
    assert!(
        session.attached().is_empty(),
        "a file the turn cannot open was attached"
    );
}

/// And `/add-dir` is how that is fixed, which is the whole reason someone opens one.
#[test]
fn opening_a_directory_makes_a_drop_from_it_attachable() {
    let outside = Scratch::new("added");
    let path = outside.file("shot.png");
    let workspace = Scratch::new("added-workspace");

    let mut session = session_in(&workspace);
    session.now_reaching(Reach::of(
        &workspace.path,
        &[outside.path.canonicalize().unwrap()],
    ));

    session.drop_files(&path);
    assert_eq!(
        session.attached().len(),
        1,
        "the added directory was ignored"
    );
    // Absolute, which is the only form the workspace resolves inside an added directory.
    assert!(
        session.attached()[0].name.starts_with('/'),
        "{}",
        session.attached()[0].name
    );
}

/// Deleting the marker is the only way a user has to take an attachment off, since the marker is
/// the only part of it they can see.
#[test]
fn deleting_the_marker_takes_the_attachment_off() {
    let scratch = Scratch::new("deleted");
    let path = scratch.file("shot.png");
    let mut session = session_in(&scratch);

    session.drop_files(&path);
    for c in " look".chars() {
        session.type_char(c);
    }

    // Sent as typed: the attachment goes.
    let mut kept = session_in(&scratch);
    kept.drop_files(&path);
    kept.submit();
    assert_eq!(kept.sent_attachments().len(), 1);

    // The marker rubbed out, the way a user rubs it out: it does not.
    while !session.input().is_empty() {
        session.backspace();
    }
    for c in "look".chars() {
        session.type_char(c);
    }
    session.submit();
    assert!(
        session.sent_attachments().is_empty(),
        "a deleted marker still sent its file"
    );
}

/// Markers are never reused, or deleting one would renumber the marker sitting in the line the
/// user is looking at.
#[test]
fn a_second_drop_gets_its_own_number() {
    let scratch = Scratch::new("numbering");
    let first = scratch.file("a.png");
    let second = scratch.file("b.png");
    let mut session = session_in(&scratch);

    session.drop_files(&first);
    session.drop_files(&second);
    assert_eq!(session.input(), "[Image #1] [Image #2] ");
}

/// The load-bearing one, again at this level: a paste that merely mentions a real file is a paste.
#[test]
fn pasting_prose_about_a_real_file_is_still_prose() {
    let scratch = Scratch::new("prose");
    let path = scratch.file("shot.png");
    let mut session = session_in(&scratch);

    let prose = format!("have a look at {path} please");
    assert!(!session.drop_files(&prose), "prose was taken as a drop");
    assert!(session.attached().is_empty());
}

/// A dropped text file is context, which is what @ and --file already make of one.
#[test]
fn a_dropped_text_file_is_context_rather_than_an_attachment() {
    let scratch = Scratch::new("text");
    let path = scratch.file("notes.md");
    let mut session = session_in(&scratch);

    session.drop_files(&path);
    assert_eq!(session.attached()[0].kind, Kind::Text);
    assert_eq!(session.input(), "[File #1] ");
}

/// Several files at once, which is what dropping a selection does.
#[test]
fn several_files_dropped_together_each_get_a_marker() {
    let scratch = Scratch::new("several");
    let a = scratch.file("a.png");
    let b = scratch.file("b.pdf");
    let mut session = session_in(&scratch);

    session.drop_files(&format!("{a} {b}"));
    assert_eq!(session.input(), "[Image #1] [PDF #2] ");
    assert_eq!(session.attached().len(), 2);
}

/// A supported file beside one nothing takes: the marker and the path sit side by side, in the
/// order they were dropped.
#[test]
fn a_mixed_drop_keeps_each_in_its_place() {
    let scratch = Scratch::new("mixed");
    let a = scratch.file("a.png");
    let b = scratch.file("b.dmg");
    let mut session = session_in(&scratch);

    session.drop_files(&format!("{a} {b}"));
    assert_eq!(session.input(), format!("[Image #1] {b} "));
}

/// Sending clears them, or the next line would carry the last line's files.
#[test]
fn sending_a_line_clears_what_was_attached_to_it() {
    let scratch = Scratch::new("cleared");
    let path = scratch.file("shot.png");
    let mut session = session_in(&scratch);

    session.drop_files(&path);
    session.submit();
    assert!(session.attached().is_empty(), "the next line inherits them");
    assert_eq!(session.sent_attachments().len(), 1);
}

/// A drop leaves a trailing space, which is what a terminal does when a file is dropped into a
/// shell: whatever is typed next, or dropped next, does not run into the marker.
#[test]
fn a_drop_leaves_room_after_itself() {
    let scratch = Scratch::new("spacing");
    let path = scratch.file("a.png");
    let mut session = session_in(&scratch);

    session.drop_files(&path);
    for c in "what is this".chars() {
        session.type_char(c);
    }
    assert_eq!(session.input(), "[Image #1] what is this");
}
