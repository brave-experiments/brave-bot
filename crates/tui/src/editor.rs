//! Editing the prompt in the user's own editor.
//!
//! A prompt worth thinking about outgrows the box it is typed in. There is no way back to a word
//! once it has scrolled off, and the keys that would move there belong to the history and the
//! transcript, so a long prompt is written blind. Ctrl-G hands the line to whatever the user
//! edits text with, waits for them to finish, and takes back what they saved.
//!
//! Nothing labelled passes through here. The line is the user's own words on their way from the
//! keyboard to their editor and back, and nothing read out of the workspace joins them.
//!
//! `$VISUAL` and `$EDITOR` hold a command line by convention, and one is split into argv here
//! rather than handed to a shell. That is the rule the rest of the system runs under, and it
//! costs nothing to keep: a shell between the user's configuration and what starts would be a
//! second interpreter with its own opinions about quoting.

use std::ffi::OsString;
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// What to open when nothing is configured, in the order they are tried.
///
/// A guess, and kept to editors that open in the terminal the key was pressed in, so finishing
/// puts the user back where they were.
///
/// Only reached when the user has said nothing. A configured editor that will not start is an
/// answer, not a reason to run something they did not choose.
///
/// On macOS the system's own `vim` is named outright, ahead of whatever `$PATH` reaches first.
/// That is the one place here where a name is not left to `$PATH` to resolve, and the reason is
/// that Homebrew's `vim` is MacVim's build: it opens a window, detaches, and hands the line back
/// before anyone has typed into it, so the edit is silently lost. Naming the system binary is
/// only defensible because this is the list of guesses, which is reached exactly when the user
/// has expressed no preference. `$VISUAL` and `$EDITOR` still decide when they are set, and a
/// name in either is still resolved through `$PATH` like any other.
#[cfg(target_os = "macos")]
const FALLBACKS: &[&str] = &["/usr/bin/vim", "vim", "vi", "nano"];

#[cfg(not(any(windows, target_os = "macos")))]
const FALLBACKS: &[&str] = &["vim", "vi", "nano"];

#[cfg(windows)]
const FALLBACKS: &[&str] = &["notepad"];

/// Editors that return before the file has been edited, and the flag that makes them stay.
///
/// A window opens, the process exits at once, the line comes back exactly as it went, and
/// nothing anywhere says why. That is the most confusing outcome available: neither a failure
/// nor an edit. The flag is only added where the command is a bare program, since a user who
/// wrote arguments of their own has already said how they want it run.
///
/// The vim family is here because a vim built with a GUI forks into it, and which build a name
/// reaches is not something the name says: `vim` is the system's on one machine and MacVim's on
/// the next. `-f` costs a terminal vim nothing, since not forking is all it was ever going to
/// do, so it can be asked of every one of them rather than of the builds that turn out to need
/// it. `nvim` is absent deliberately: it never forks and does not take the flag.
const WAITING: &[(&str, &str)] = &[
    ("code", "--wait"),
    ("codium", "--wait"),
    ("cursor", "--wait"),
    ("windsurf", "--wait"),
    ("zed", "--wait"),
    ("subl", "--wait"),
    ("vim", "-f"),
    ("gvim", "-f"),
    ("mvim", "-f"),
    ("view", "-f"),
];

/// Why a line did not come back from an editor.
#[derive(Debug)]
pub enum Failure {
    /// Nothing is configured, and nothing on the fallback list is installed.
    NoEditor,
    /// An editor was found, and it did not edit the line.
    Editor(String),
    /// The file the editor was to open could not be written, or could not be read back.
    Scratch(io::Error),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::NoEditor => write!(
                f,
                "no editor found: set $VISUAL or $EDITOR to the one you want"
            ),
            Failure::Editor(what) => write!(f, "{what}"),
            Failure::Scratch(error) => write!(f, "the file to edit could not be used: {error}"),
        }
    }
}

/// Put `line` in front of the user in their editor, and return what they saved.
///
/// Quitting without saving returns `line` unchanged, because the file already holds it: not
/// saving loses the edits rather than the prompt.
pub fn edit(line: &str) -> Result<String, Failure> {
    round_trip(line, open_in_an_editor)
}

/// The round trip through a file, with opening it left to the caller.
///
/// Separated so the file half can be tested without an editor, which is the half with the
/// property worth pinning: what comes back when nothing was written.
fn round_trip(
    line: &str,
    open: impl FnOnce(&Path) -> Result<(), Failure>,
) -> Result<String, Failure> {
    let path = scratch();
    if let Err(failure) = write_scratch(&path, line) {
        // A write that failed partway leaves a file behind, and one holding a prompt is not
        // something to leave in a shared directory.
        let _ = std::fs::remove_file(&path);
        return Err(failure);
    }

    let opened = open(&path);
    let saved = std::fs::read_to_string(&path);
    // Before either result is examined, so the prompt does not outlive the edit down any path.
    let _ = std::fs::remove_file(&path);

    opened?;
    let mut text = saved.map_err(Failure::Scratch)?;
    tidy(&mut text);
    Ok(text)
}

/// Where the line is put for the editor to open.
///
/// The system's temporary directory rather than `~/.bua`, which is the user's configuration
/// surface and is read as trusted: scratch files do not belong in it.
fn scratch() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("bua-prompt-{}-{stamp}.md", std::process::id()))
}

/// Write the line where the editor will find it.
///
/// Created rather than opened: on a shared temporary directory an existing name may be a symlink
/// someone else left pointing at a file of theirs, and refusing to reuse one is what keeps this
/// from writing the prompt through it. Readable by nobody else for the same reason.
fn write_scratch(path: &Path, line: &str) -> Result<(), Failure> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(Failure::Scratch)?;
    file.write_all(line.as_bytes()).map_err(Failure::Scratch)?;
    file.flush().map_err(Failure::Scratch)
}

/// Make what an editor saved fit the box it is going back into.
fn tidy(text: &mut String) {
    if text.contains('\r') {
        *text = text.replace("\r\n", "\n").replace('\r', "\n");
    }
    // Editors end a file with a newline, and the box would draw it as an empty line below the
    // prompt. One only: a blank line the user left deliberately is theirs to have left.
    if text.ends_with('\n') {
        text.pop();
    }
}

/// Run an editor on `path` and wait for it.
fn open_in_an_editor(path: &Path) -> Result<(), Failure> {
    let Some(command) = configured() else {
        let Some((program, arguments)) = first_installed(FALLBACKS) else {
            return Err(Failure::NoEditor);
        };
        return start(&program, &arguments, path);
    };

    match command_line(&command) {
        Some((program, arguments)) => start(&program, &arguments, path),
        None => Err(Failure::Editor(format!(
            "'{command}' was not found, and $VISUAL or $EDITOR names it, so nothing else was tried"
        ))),
    }
}

/// The first of `candidates` that is installed, and the arguments to run it with.
///
/// Order is the whole of what this list says, since every name on it is an editor: the first one
/// present is the guess, and one that is not installed is the next name's turn rather than a
/// failure.
fn first_installed(candidates: &[&str]) -> Option<(PathBuf, Vec<String>)> {
    candidates
        .iter()
        .find_map(|candidate| command_line(candidate))
}

/// The editor the user configured, if they configured one.
fn configured() -> Option<String> {
    chosen(std::env::var_os("VISUAL"), std::env::var_os("EDITOR"))
}

/// Which of the two variables answers, given what they hold.
///
/// `$VISUAL` first, which is the convention: `$EDITOR` is the one that has to work on a teletype,
/// and `$VISUAL` is what the user wants when the terminal can do more than print. An empty value
/// is not an answer, since exporting a variable to nothing is how a profile takes one back.
fn chosen(visual: Option<OsString>, editor: Option<OsString>) -> Option<String> {
    [visual, editor]
        .into_iter()
        .flatten()
        .map(|value| value.to_string_lossy().into_owned())
        .find(|value| !value.trim().is_empty())
}

/// Split a configured editor into the program to run and the arguments to run it with.
///
/// `None` when there is no such program, which is the fallback list's turn or, for a configured
/// editor, the whole answer.
///
/// Whitespace usually separates a command from its flags, but it is also just a character in a
/// path, and on macOS most of `/Applications` has one. So the lookup decides and the shape of the
/// string never does: a name that resolves whole is a path and keeps its spaces, and only one
/// that does not is read as a command line. That leaves an editor whose path has a space in it
/// and takes arguments unreachable, which needs quoting rules to fix and has never been asked for.
fn command_line(command: &str) -> Option<(PathBuf, Vec<String>)> {
    // Only consulted for a name with a separator in it, and a relative one at that. The current
    // directory is what such a name means to whoever exported it.
    let working = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let (program, mut arguments) = match bua_agent::programs::resolve(command, &working) {
        Some(program) => (program, Vec::new()),
        None => {
            let mut words = command.split_whitespace();
            let program = bua_agent::programs::resolve(words.next()?, &working)?;
            (program, words.map(str::to_string).collect())
        }
    };

    if arguments.is_empty() {
        arguments.extend(waiting_flag(&program));
    }
    Some((program, arguments))
}

/// The flag that makes `program` wait, if it is one of the ones that needs telling.
fn waiting_flag(program: &Path) -> Option<String> {
    let stem = program.file_stem()?.to_string_lossy().into_owned();
    WAITING
        .iter()
        .find(|(name, _)| *name == stem)
        .map(|(_, flag)| (*flag).to_string())
}

/// Start the editor and wait for it to finish.
///
/// A non-zero exit is taken at its word and the line is left alone. `vi` spells "discard this"
/// as `:cq`, which exits 1, and honouring that is the difference between an editor that can be
/// abandoned and one that cannot.
fn start(program: &Path, arguments: &[String], path: &Path) -> Result<(), Failure> {
    let name = program
        .file_name()
        .unwrap_or(program.as_os_str())
        .to_string_lossy()
        .into_owned();

    match Command::new(program).args(arguments).arg(path).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(Failure::Editor(match status.code() {
            Some(code) => format!("{name} exited with status {code}, so the line is unchanged"),
            None => format!("{name} was stopped before it finished, so the line is unchanged"),
        })),
        Err(error) => Err(Failure::Editor(format!("{name} would not start: {error}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The convention both variables are part of: `$EDITOR` is the one that must work anywhere,
    /// and `$VISUAL` is what to use when the terminal can do more than print a line at a time.
    #[test]
    fn visual_answers_before_editor() {
        assert_eq!(
            chosen(Some("vim".into()), Some("ed".into())),
            Some("vim".to_string())
        );
        assert_eq!(chosen(None, Some("ed".into())), Some("ed".to_string()));
        assert_eq!(chosen(None, None), None);
    }

    /// Exporting a variable to nothing is how a shell profile takes one back. Reading that as a
    /// configured editor would try to run the empty string and report that it was not found,
    /// when what the user has is no editor configured at all.
    #[test]
    fn an_empty_variable_is_not_a_configured_editor() {
        assert_eq!(
            chosen(Some("".into()), Some("vi".into())),
            Some("vi".into())
        );
        assert_eq!(chosen(Some("   ".into()), None), None);
    }

    /// The whole point of the round trip: what was saved is what comes back.
    #[test]
    fn what_the_editor_saved_becomes_the_line() {
        let line = round_trip("first thoughts", |path| {
            std::fs::write(path, "second thoughts").unwrap();
            Ok(())
        })
        .expect("the round trip completes");

        assert_eq!(line, "second thoughts");
    }

    /// Quitting without saving must not blank the prompt. The file already holds the line, so
    /// what comes back is what went in, and not saving costs the edits rather than the prompt.
    #[test]
    fn quitting_without_saving_leaves_the_line_as_it_was() {
        let line = round_trip("first thoughts", |_| Ok(())).expect("the round trip completes");

        assert_eq!(line, "first thoughts");
    }

    /// The editor opens on what has been typed so far, not on an empty file: pressing the key
    /// halfway through a prompt continues it rather than starting again.
    #[test]
    fn the_editor_opens_on_what_was_already_typed() {
        round_trip("half a thought", |path| {
            assert_eq!(std::fs::read_to_string(path).unwrap(), "half a thought");
            Ok(())
        })
        .expect("the round trip completes");
    }

    /// A prompt is the user's own words and has no business staying in a shared directory after
    /// the edit, whether the edit worked or not.
    #[test]
    fn the_file_does_not_outlive_the_edit() {
        let mut opened = PathBuf::new();
        round_trip("something private", |path| {
            opened = path.to_path_buf();
            Ok(())
        })
        .expect("the round trip completes");
        assert!(!opened.exists(), "the file was left behind after an edit");

        let mut failed = PathBuf::new();
        let outcome = round_trip("something private", |path| {
            failed = path.to_path_buf();
            Err(Failure::Editor("no".to_string()))
        });
        assert!(outcome.is_err());
        assert!(!failed.exists(), "the file was left behind after a failure");
    }

    /// An editor that failed says nothing about what the user wanted, so the line stays as it
    /// was rather than being replaced by whatever happened to be in the file.
    #[test]
    fn an_editor_that_failed_does_not_produce_a_line() {
        let outcome = round_trip("first thoughts", |path| {
            std::fs::write(path, "half an edit").unwrap();
            Err(Failure::Editor("stopped".to_string()))
        });

        assert!(matches!(outcome, Err(Failure::Editor(_))));
    }

    /// Editors end a file with a newline. Kept, it would draw an empty line under the prompt on
    /// every trip through the editor, and they would accumulate.
    #[test]
    fn the_newline_an_editor_leaves_at_the_end_is_dropped() {
        let mut text = "a prompt\n".to_string();
        tidy(&mut text);
        assert_eq!(text, "a prompt");
    }

    /// One newline, not every one: a blank line at the end of a paragraph was typed on purpose.
    #[test]
    fn only_the_last_newline_goes() {
        let mut text = "a prompt\n\n".to_string();
        tidy(&mut text);
        assert_eq!(text, "a prompt\n");
    }

    /// A file saved by an editor on Windows, or edited over one, comes back with the line endings
    /// the box draws, which are the ones a paste is normalised to.
    #[test]
    fn line_endings_come_back_the_way_a_paste_does() {
        let mut text = "one\r\ntwo\rthree\n".to_string();
        tidy(&mut text);
        assert_eq!(text, "one\ntwo\nthree");
    }

    /// The guesses are in preference order, and a name that is not installed is the next one's
    /// turn rather than the end of the list.
    #[test]
    #[cfg(unix)]
    fn the_first_installed_guess_is_the_one_taken() {
        let (program, _) = first_installed(&["no-such-editor-a", "sh", "no-such-editor-b"])
            .expect("sh is installed");
        assert_eq!(program.file_name().unwrap(), "sh");
    }

    /// Nothing installed is the one case with no editor to run, and it has to be told apart from
    /// an editor that ran and failed: the answer is to set one, not to try again.
    #[test]
    fn nothing_installed_is_no_editor_at_all() {
        assert!(first_installed(&["no-such-editor-a", "no-such-editor-b"]).is_none());
        assert!(first_installed(&[]).is_none());
    }

    /// A bare GUI editor exits the moment its window opens, and the line would come back
    /// unchanged with nothing to say why.
    #[test]
    fn a_gui_editor_is_told_to_wait() {
        assert_eq!(
            waiting_flag(Path::new("/usr/local/bin/code")),
            Some("--wait".to_string())
        );
    }

    /// The flag belongs to the program, not to the name it was reached by, so a resolved path
    /// and an extension are both still that program.
    #[test]
    fn the_flag_follows_the_program_through_a_path() {
        assert_eq!(
            waiting_flag(Path::new("code.cmd")),
            Some("--wait".to_string())
        );
    }

    /// A terminal editor that cannot fork already waits, and an argument it did not ask for is
    /// one it may refuse to start over.
    #[test]
    fn a_terminal_editor_is_given_no_extra_flag() {
        assert_eq!(waiting_flag(Path::new("/usr/bin/nano")), None);
        assert_eq!(waiting_flag(Path::new("nvim")), None);
    }

    /// A vim built with a GUI forks into it and hands the line back before anyone has typed, and
    /// the name does not say which build it reached: `vim` is the system's on one machine and
    /// MacVim's on the next. Asking every vim to stay costs a terminal one nothing.
    #[test]
    fn every_vim_is_asked_to_stay_in_the_foreground() {
        for name in ["/usr/bin/vim", "/opt/homebrew/bin/vim", "gvim", "mvim"] {
            assert_eq!(
                waiting_flag(Path::new(name)),
                Some("-f".to_string()),
                "{name} was left free to fork"
            );
        }
    }

    /// The guess must not depend on what a package manager put ahead of the system's own editor.
    /// Homebrew's `vim` here is MacVim's build, and picking it up loses the edit.
    #[test]
    #[cfg(target_os = "macos")]
    fn the_guess_on_macos_is_the_system_vim_in_the_foreground() {
        let (program, arguments) = first_installed(FALLBACKS).expect("macOS ships /usr/bin/vim");

        assert_eq!(program, Path::new("/usr/bin/vim"));
        assert_eq!(arguments, ["-f"]);
    }
}
