//! Editing the prompt in the user's own editor.
//!
//! A prompt worth thinking about outgrows the box it is typed in. The caret reaches any word of it
//! now, but the box is still ten rows at most and has nothing to search, fold or reflow with.
//! Ctrl-G hands the line to whatever the user edits text with, waits for them to finish, and takes
//! back what they saved.
//!
//! Nothing labelled passes through here. The line is the user's own words on their way from the
//! keyboard to their editor and back, and nothing read out of the workspace joins them.
//!
//! `$VISUAL` and `$EDITOR` hold a command line by convention, and one is split into argv here
//! rather than handed to a shell. That is the rule the rest of the system runs under, and it
//! costs nothing to keep: a shell between the user's configuration and what starts would be a
//! second interpreter with its own opinions about quoting.

use bravebot_i18n::t;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// What to open when nothing is configured, in the order they are tried.
///
/// A guess, and kept to editors that open in the terminal the key was pressed in, so finishing
/// puts the user back where they were. The full editors come first: someone with `vim` or `emacs`
/// installed almost certainly reaches for it, and `nano` is the last resort rather than the first.
///
/// Only reached when the user has said nothing. A configured editor that will not start is an
/// answer, not a reason to run something they did not choose.
#[cfg(not(windows))]
const FALLBACKS: &[&str] = &["vim", "vi", "emacs", "nano"];

#[cfg(windows)]
const FALLBACKS: &[&str] = &["notepad"];

/// Editors that return before the file has been edited, and the flag that makes them wait.
///
/// A window opens, the process exits at once, the line comes back exactly as it went, and
/// nothing anywhere says why. That is the most confusing outcome available: neither a failure
/// nor an edit. The flag is only added where the command is a bare program, since a user who
/// wrote arguments of their own has already said how they want it run.
const WAITING: &[(&str, &str)] = &[
    ("code", "--wait"),
    ("codium", "--wait"),
    ("cursor", "--wait"),
    ("windsurf", "--wait"),
    ("zed", "--wait"),
    ("subl", "--wait"),
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
            Failure::NoEditor => write!(f, "{}", t!(editor_none_configured)),
            Failure::Editor(what) => write!(f, "{what}"),
            Failure::Scratch(error) => {
                write!(f, "{}", t!(editor_scratch_unusable, problem = error))
            }
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

/// Put the transcript in front of the user in their editor, and read nothing back.
///
/// The one difference from [`edit`], and the whole of it. A prompt is something they are still
/// writing, so what they save comes back; a transcript is the record of what happened, and a
/// record that can be edited into the session is not a record. The file is theirs to do anything
/// with, and this process never looks at it again.
pub fn show(text: &str) -> Result<(), Failure> {
    show_through(text, open_in_an_editor)
}

/// The file half of [`show`], with opening it left to the caller so it can be tested.
fn show_through(
    text: &str,
    open: impl FnOnce(&Path) -> Result<(), Failure>,
) -> Result<(), Failure> {
    let path = scratch("transcript");
    if let Err(failure) = write_scratch(&path, text) {
        let _ = std::fs::remove_file(&path);
        return Err(failure);
    }
    let opened = open(&path);
    // Before the result is examined, so a transcript does not outlive the look at it down any
    // path. Whatever the person wanted to keep, they kept from inside their own editor.
    let _ = std::fs::remove_file(&path);
    opened
}

/// The round trip through a file, with opening it left to the caller.
///
/// Separated so the file half can be tested without an editor, which is the half with the
/// property worth pinning: what comes back when nothing was written.
fn round_trip(
    line: &str,
    open: impl FnOnce(&Path) -> Result<(), Failure>,
) -> Result<String, Failure> {
    let path = scratch("prompt");
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
/// The system's temporary directory rather than `~/.bravebot`, which is the user's configuration
/// surface and is read as trusted: scratch files do not belong in it.
fn scratch(kind: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("bravebot-{kind}-{}-{stamp}.md", std::process::id()))
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
    let (program, arguments) = which_editor(configured())?;
    start(&program, &arguments, path)
}

/// The editor to run, given whatever the user configured.
fn which_editor(command: Option<String>) -> Result<(PathBuf, Vec<String>), Failure> {
    let Some(command) = command else {
        return FALLBACKS
            .iter()
            .find_map(|fallback| command_line(fallback))
            .ok_or(Failure::NoEditor);
    };

    command_line(&command)
        .ok_or_else(|| Failure::Editor(t!(editor_named_but_missing, command = command)))
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

    split(command, |name| lookup(name, &working))
}

/// Split a command into a program and its arguments, with finding the program left to the caller.
///
/// Separated so the splitting and the flag can be tested against an installation laid out like a
/// real one, without a machine that has that editor on its `$PATH`.
fn split(command: &str, find: impl Fn(&str) -> Option<PathBuf>) -> Option<(PathBuf, Vec<String>)> {
    let (program, mut arguments) = match find(command) {
        Some(program) => (program, Vec::new()),
        None => {
            let mut words = command.split_whitespace();
            let program = find(words.next()?)?;
            (program, words.map(str::to_string).collect())
        }
    };

    if arguments.is_empty() {
        arguments.extend(waiting_flag(&program));
    }
    Some((program, arguments))
}

/// Find the editor `command` names, keeping the name it was found under.
///
/// The shared lookup canonicalises, because an approval recorded against a program has to name the
/// file that actually ran and not a symlink that may be repointed. An editor is started rather than
/// approved, and here the name is part of what was asked for: MacVim installs `vim`, `vi` and
/// `gvim` as links to one shim that reads its own `argv[0]` and opens a detached window for the
/// `m*` and `g*` spellings. Resolved through the link it becomes `mvim`, so asking for `vim` got a
/// GUI window and a prompt that came straight back. Started the way the shell would start it, under
/// the name on the `$PATH`, it stays in the terminal.
///
/// So the file is located with the shared lookup, which decides whether the name is a program at
/// all, and then the path that was not canonicalised is what runs.
///
/// The name has to be looked for where the shell would look for it, not in the directory the
/// canonicalised file turned out to live in. MacVim's bundle holds a `vim` link but no `vi` one:
/// `vi` is a link in the Homebrew directory on the `$PATH`, and joining the name onto the bundle
/// found nothing.
fn lookup(command: &str, working: &Path) -> Option<PathBuf> {
    let resolved = bravebot_agent::programs::resolve(command, working)?;

    let named = if Path::new(command).is_absolute() {
        PathBuf::from(command)
    } else if command.contains('/') || (cfg!(windows) && command.contains('\\')) {
        working.join(command)
    } else {
        let path = std::env::var_os("PATH")?;
        by_name(command, &resolved, std::env::split_paths(&path))?
    };

    // Only where it is still the same program by another name. A link pointing somewhere else, or
    // a name that no longer resolves, is not something to run on a guess.
    if same_file(&named, &resolved) {
        Some(named)
    } else {
        Some(resolved)
    }
}

/// Where on the `$PATH` `command` names the same file the lookup settled on.
///
/// The first entry holding it, exactly as the shell would find it. An empty entry means the current
/// directory and is skipped for the reason the shared lookup skips it: a file in the workspace has
/// no business shadowing a program.
fn by_name(
    command: &str,
    resolved: &Path,
    directories: impl Iterator<Item = PathBuf>,
) -> Option<PathBuf> {
    directories
        .filter(|directory| !directory.as_os_str().is_empty())
        .map(|directory| directory.join(command))
        .find(|candidate| same_file(candidate, resolved))
}

/// Whether two paths reach one file, links and all.
fn same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
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
            Some(code) => t!(editor_exited_badly, editor = name, code = code),
            None => t!(editor_was_stopped, editor = name),
        })),
        Err(error) => Err(Failure::Editor(t!(
            editor_would_not_start,
            editor = name,
            problem = error
        ))),
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

    /// A scratch directory, removed with the test.
    struct Scratch {
        path: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("bravebot-editor-{name}"));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create");
            Self { path }
        }

        #[cfg(unix)]
        fn program(&self, name: &str) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let path = self.path.join(name);
            std::fs::write(&path, "#!/bin/sh\n").expect("write");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// MacVim installs `vim`, `vi` and `gvim` as links to one shim that reads its own `argv[0]` and
    /// opens a detached GUI window for the `m*` and `g*` spellings. Resolved through the link, a
    /// request for `vim` became `mvim`: a window opened, the process forked, and the prompt came
    /// straight back unedited. The name is part of what was asked for, so it survives the lookup.
    #[cfg(unix)]
    #[test]
    fn a_program_reached_through_a_link_keeps_the_name_it_was_asked_for() {
        let scratch = Scratch::new("named");
        scratch.program("mvim");
        let asked = scratch.path.join("vim");
        std::os::unix::fs::symlink("mvim", &asked).expect("link");

        let found = lookup(asked.to_str().unwrap(), &scratch.path).expect("the editor is found");

        assert_eq!(
            found.file_name().unwrap(),
            "vim",
            "the editor was started under the name the link points at, not the one asked for"
        );
    }

    /// The real path from a configured editor to what runs, which is where the fix has to be wired
    /// in. An absolute path needs no `$PATH`, so the whole of `command_line` can be checked here: a
    /// link to the shim is started as the link, not as what it points at.
    #[cfg(unix)]
    #[test]
    fn a_configured_link_to_a_gui_shim_runs_as_the_link() {
        let scratch = Scratch::new("configured");
        scratch.program("mvim");
        let asked = scratch.path.join("vim");
        std::os::unix::fs::symlink("mvim", &asked).expect("link");

        let (program, arguments) =
            command_line(asked.to_str().unwrap()).expect("the editor is found");

        assert_eq!(
            program, asked,
            "the configured editor ran under the shim's own name, which forks a GUI window"
        );
        assert!(arguments.is_empty());
    }

    /// The whole choice, against an installation laid out the way MacVim's is: `vim` and `vi` on
    /// the `$PATH` are links into a bundle holding one shim, which reads its own `argv[0]` and
    /// forks a GUI window for the `m*` spellings. What runs has to be the name that was asked for,
    /// and no waiting flag belongs on a terminal editor.
    #[cfg(unix)]
    #[test]
    fn a_terminal_editor_behind_a_gui_shim_is_started_under_its_own_name() {
        let scratch = Scratch::new("macvim");
        let bundle = scratch.path.join("MacVim.app/Contents/bin");
        std::fs::create_dir_all(&bundle).expect("create");
        let shim = bundle.join("mvim");
        std::fs::write(&shim, "#!/bin/sh\n").expect("write");
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        std::os::unix::fs::symlink("mvim", bundle.join("vim")).expect("link");
        for name in ["vim", "vi"] {
            std::os::unix::fs::symlink(bundle.join("vim"), scratch.path.join(name)).expect("link");
        }

        for name in ["vim", "vi"] {
            let (program, arguments) = split(name, |command| {
                by_name(command, &shim, [scratch.path.clone()].into_iter())
            })
            .expect("the editor is found");

            assert_eq!(
                program,
                scratch.path.join(name),
                "{name} was started under the shim's own name, which forks a GUI window"
            );
            assert!(
                arguments.is_empty(),
                "a terminal editor was given {arguments:?}"
            );
        }
    }

    /// The name is looked for where the shell would look for it. MacVim's bundle holds a `vim` link
    /// but no `vi` one: `vi` lives in the directory that is actually on the `$PATH`, so searching
    /// the directory the canonicalised file turned out to be in found nothing and `vi` went on
    /// opening a GUI window after `vim` had been fixed.
    #[cfg(unix)]
    #[test]
    fn the_name_is_looked_for_on_the_path_not_beside_the_resolved_file() {
        let scratch = Scratch::new("bundle");
        let bundle = scratch.path.join("bundle");
        std::fs::create_dir_all(&bundle).expect("create");
        let shim = bundle.join("mvim");
        std::fs::write(&shim, "#!/bin/sh\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        // On the `$PATH`, and the only place this spelling exists.
        let on_path = scratch.path.join("vi");
        std::os::unix::fs::symlink(&shim, &on_path).expect("link");

        let found = by_name("vi", &shim, [scratch.path.clone()].into_iter())
            .expect("the name is found on the path");

        assert_eq!(found, on_path);
    }

    /// An empty `$PATH` entry means the current directory, and a file in the workspace has no
    /// business shadowing the editor.
    #[cfg(unix)]
    #[test]
    fn an_empty_path_entry_is_not_searched() {
        let scratch = Scratch::new("empty-entry");
        let real = scratch.program("vim");

        let found = by_name("vim", &real, [PathBuf::new()].into_iter());

        assert!(found.is_none());
    }

    /// The name only survives while it is the same program. A link repointed at something else is
    /// not an editor to start on a guess.
    #[cfg(unix)]
    #[test]
    fn a_name_that_is_no_longer_the_same_program_falls_back_to_the_resolved_path() {
        let scratch = Scratch::new("elsewhere");
        let real = scratch.program("emacs");

        let found = lookup(real.to_str().unwrap(), &scratch.path).expect("the editor is found");

        assert_eq!(found.canonicalize().unwrap(), real.canonicalize().unwrap());
    }

    /// Someone with `vim` or `emacs` installed chose to install it, and opening `nano` at them
    /// instead is a guess overriding an answer they already gave. `nano` stays on the list for the
    /// person who has neither, and stays last.
    #[test]
    fn a_full_editor_is_preferred_to_the_last_resort() {
        let order = |name: &str| FALLBACKS.iter().position(|entry| *entry == name);

        assert!(order("vim") < order("nano"));
        assert!(order("vi") < order("nano"));
        assert!(order("emacs") < order("nano"));
        assert_eq!(order("nano"), Some(FALLBACKS.len() - 1));
    }

    /// A name the user exported is an answer. Trying the list after it would open an editor they
    /// did not choose and leave them thinking their configuration had worked.
    #[test]
    fn a_configured_editor_that_will_not_start_ends_the_search() {
        let outcome = which_editor(Some("definitely-not-an-editor-on-this-machine".to_string()));

        assert!(
            matches!(outcome, Err(Failure::Editor(_))),
            "the fallback list was tried behind a configured editor"
        );
    }

    /// The one difference between showing a transcript and editing a prompt, and the whole of it.
    /// A record that can be edited into the session is not a record, so whatever the editor saved
    /// stays in the editor.
    #[test]
    fn a_transcript_opened_in_the_editor_is_never_read_back() {
        let mut opened = None;
        show_through("what happened", |path| {
            assert_eq!(
                std::fs::read_to_string(path).expect("the transcript is there"),
                "what happened"
            );
            std::fs::write(path, "what did not happen").unwrap();
            opened = Some(path.to_path_buf());
            Ok(())
        })
        .expect("the editor opens");

        // Nothing came back to compare, which is the property: the call returns no text at all,
        // so there is no path by which an edited record could reach the session.
        assert!(opened.is_some(), "the editor was never handed a file");
    }

    /// A transcript holds content nobody vouched for. It goes where scratch files go, and not
    /// into the workspace, where the next turn would find it as a file somebody had written.
    #[test]
    fn a_transcript_opened_in_the_editor_is_written_outside_the_workspace() {
        let mut written = None;
        show_through("what happened", |path| {
            written = Some(path.to_path_buf());
            Ok(())
        })
        .expect("the editor opens");

        let written = written.expect("the editor was handed a file");
        assert!(
            written.starts_with(std::env::temp_dir()),
            "the transcript was written to {}",
            written.display()
        );
        assert_ne!(
            written.parent(),
            std::env::current_dir().ok().as_deref(),
            "the transcript was written into the working directory"
        );
    }

    /// It does not outlive the look at it, down either path. A transcript left in a shared
    /// temporary directory is the session's contents sitting where anyone can read them.
    #[test]
    fn the_file_goes_when_the_editor_exits() {
        let mut left = None;
        show_through("what happened", |path| {
            left = Some(path.to_path_buf());
            Ok(())
        })
        .expect("the editor opens");
        assert!(
            !left.expect("a file").exists(),
            "the transcript was left behind"
        );

        let mut left = None;
        let failed = show_through("what happened", |path| {
            left = Some(path.to_path_buf());
            Err(Failure::Editor("no".to_string()))
        });
        assert!(failed.is_err());
        assert!(
            !left.expect("a file").exists(),
            "an editor that failed left the transcript behind"
        );
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

    /// A terminal editor already waits, and an argument it did not ask for is one it may refuse
    /// to start over.
    #[test]
    fn a_terminal_editor_is_given_no_extra_flag() {
        assert_eq!(waiting_flag(Path::new("/usr/bin/vim")), None);
        assert_eq!(waiting_flag(Path::new("nano")), None);
    }
}
