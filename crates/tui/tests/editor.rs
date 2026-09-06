//! Taking the prompt through a real editor.
//!
//! `$VISUAL` and `$EDITOR` are process-wide, so these tests are the only ones in the crate that
//! touch them and they are kept in a file of their own, in the same arrangement `HOME` gets.
//!
//! The editors here are ordinary programs standing in for one: `cp` saves over the file it was
//! given, `true` leaves without saving, `false` refuses the way `vi`'s `:cq` does. What is being
//! tested is the whole path a user takes, spawning included, which the unit tests cannot reach.

#![cfg(unix)]

use std::path::PathBuf;
use std::sync::Mutex;

/// One lock for the whole file, not one per test.
///
/// Both variables are process-wide, so every test here contends for the same thing. A mutex
/// declared inside each function would be a different mutex, and two tests would then be free to
/// run at once and see each other's editor.
static EDITOR_LOCK: Mutex<()> = Mutex::new(());

/// Run `body` with `$EDITOR` set to `command` and `$VISUAL` out of the way.
fn with_editor<T>(command: &str, body: impl FnOnce() -> T) -> T {
    with_both(None, Some(command), body)
}

/// Run `body` with both variables set to whatever is given.
fn with_both<T>(visual: Option<&str>, editor: Option<&str>, body: impl FnOnce() -> T) -> T {
    let _guard = EDITOR_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let previous = [
        ("VISUAL", std::env::var_os("VISUAL")),
        ("EDITOR", std::env::var_os("EDITOR")),
    ];
    // SAFETY: single-threaded within the lock, and restored before returning.
    unsafe {
        set("VISUAL", visual);
        set("EDITOR", editor);
    }

    let result = body();

    for (name, value) in previous {
        match value {
            Some(value) => unsafe { std::env::set_var(name, value) },
            None => unsafe { std::env::remove_var(name) },
        }
    }
    result
}

/// SAFETY: callers hold the lock and restore what was there.
unsafe fn set(name: &str, value: Option<&str>) {
    match value {
        Some(value) => unsafe { std::env::set_var(name, value) },
        None => unsafe { std::env::remove_var(name) },
    }
}

/// A program that saves `contents` over the file it is given, which is what an editor does.
fn saves(contents: &str, name: &str) -> PathBuf {
    let source = std::env::temp_dir().join(format!("bravebot-editor-{name}"));
    std::fs::write(&source, contents).expect("scratch source");
    source
}

/// The whole point of the round trip: the line goes out to an editor and comes back changed.
#[test]
fn what_the_editor_saved_becomes_the_line() {
    let source = saves("the considered version\n", "saved");
    let edited = with_editor(&format!("cp {}", source.display()), || {
        bravebot_tui::editor::edit("the first thing that came to mind")
    })
    .expect("the editor ran");

    assert_eq!(edited, "the considered version");
    std::fs::remove_file(&source).ok();
}

/// Quitting without saving must not blank the prompt. The file already holds the line, so what
/// comes back is what went in: not saving costs the edits, never the prompt.
#[test]
fn an_editor_that_saved_nothing_gives_the_line_back() {
    let edited = with_editor("true", || bravebot_tui::editor::edit("half a thought"))
        .expect("the editor ran");

    assert_eq!(edited, "half a thought");
}

/// `vi` spells "discard this" as `:cq`, which exits 1. Honouring it is the difference between an
/// editor that can be abandoned and one whose window is a commitment.
#[test]
fn an_editor_that_refuses_leaves_the_line_alone() {
    let failure = with_editor("false", || bravebot_tui::editor::edit("half a thought"))
        .expect_err("a non-zero exit is not an edit");

    assert!(
        failure.to_string().contains("unchanged"),
        "said {failure}, which does not say the line survived"
    );
}

/// A configured editor is the user's answer to which one to use. Quietly running a different one
/// because theirs is not installed would edit their prompt with a program they did not choose,
/// and the message has to say why nothing happened rather than leave them guessing.
#[test]
fn a_configured_editor_that_is_missing_is_not_replaced_by_a_guess() {
    let failure = with_editor("no-such-editor-anywhere", || {
        bravebot_tui::editor::edit("a line")
    })
    .expect_err("a missing editor is not an edit");

    let said = failure.to_string();
    assert!(said.contains("was not found"), "said {said}");
    assert!(said.contains("nothing else was tried"), "said {said}");
}

/// A path with a space in it is a path. Most of `/Applications` has one, and reading the first
/// space as the end of the program would send a user's editor a file it never opened.
#[test]
fn an_editor_path_with_a_space_in_it_is_one_program() {
    let editor = std::env::temp_dir().join("bravebot editor with spaces.sh");
    std::fs::write(&editor, "#!/bin/sh\nprintf 'edited\\n' > \"$1\"\n").expect("scratch editor");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).expect("mode");

    let edited = with_editor(&editor.display().to_string(), || {
        bravebot_tui::editor::edit("a line")
    })
    .expect("the editor ran");

    assert_eq!(edited, "edited");
    std::fs::remove_file(&editor).ok();
}

/// `$VISUAL` is what the user wants when the terminal can do more than print a line at a time,
/// which is every terminal this runs in, so it answers before `$EDITOR` does.
#[test]
fn visual_is_the_one_that_runs() {
    let wanted = saves("from visual\n", "visual");
    let unwanted = saves("from editor\n", "editor");

    let edited = with_both(
        Some(&format!("cp {}", wanted.display())),
        Some(&format!("cp {}", unwanted.display())),
        || bravebot_tui::editor::edit("a line"),
    )
    .expect("the editor ran");

    assert_eq!(edited, "from visual");
    std::fs::remove_file(&wanted).ok();
    std::fs::remove_file(&unwanted).ok();
}

/// Every scratch file this makes, sorted, so two listings can be compared.
///
/// The name is generated inside the module under test, so a caller cannot know it in advance and
/// watches the directory instead.
fn scratch_files() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("bravebot-prompt-") || name.starts_with("bravebot-transcript-")
                })
        })
        .collect();
    found.sort();
    found
}

/// The prompt does not outlive the edit, down any of the three ways out of one.
///
/// Snapshotted inside the closure because that is where the editor lock is held: another test in
/// this binary part-way through its own edit would otherwise look like a file left behind.
///
/// Worth pinning on its own account, and named by INCOG-8, which states this hand-off as the one
/// thing an incognito session still writes and leans on it being gone by the time the editor
/// closes.
#[test]
fn the_scratch_file_does_not_outlive_the_edit() {
    with_editor("true", || {
        let before = scratch_files();
        let _ = bravebot_tui::editor::edit("something private");
        assert_eq!(
            scratch_files(),
            before,
            "a prompt the editor did not save was left in the temporary directory"
        );
    });

    with_editor("false", || {
        let before = scratch_files();
        let _ = bravebot_tui::editor::edit("something private");
        assert_eq!(
            scratch_files(),
            before,
            "a prompt was left behind when the editor refused"
        );
    });

    with_editor("true", || {
        let before = scratch_files();
        let _ = bravebot_tui::editor::show("a transcript worth not keeping");
        assert_eq!(
            scratch_files(),
            before,
            "a transcript was left in the temporary directory"
        );
    });
}
