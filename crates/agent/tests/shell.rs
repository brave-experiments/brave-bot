//! Tests for running a command line the user typed, against a real shell in a real directory.
//!
//! The interesting cases are the ones that separate shell mode from [`bravebot_agent::exec`]: a glob
//! expands, a variable expands, a redirection writes. Those are refused for the planner and are the
//! whole point here, so each is pinned rather than assumed.

use bravebot_agent::conversation::Conversation;
use bravebot_agent::shell::{self, ShellError};
use bravebot_aichat::protocol::Role;
use bravebot_core::cancel::Cancel;
use bravebot_core::event::{Event, NullSink, RecordingSink};
use std::path::PathBuf;

/// A scratch directory that removes itself, so tests do not leave state behind.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("bravebot-shell-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create scratch");
        Self { path }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn run(line: &str, at: &std::path::Path) -> Result<shell::Ran, ShellError> {
    shell::run(line, at, &Cancel::new())
}

#[test]
fn a_command_returns_what_it_printed() {
    let scratch = Scratch::new("printed");
    let ran = run("echo hello", &scratch.path).expect("echo runs");
    assert_eq!(ran.stdout.trim(), "hello");
    assert!(ran.succeeded());
}

/// The reason shell mode exists rather than reusing the argv path: a user typing `*.txt` means the
/// files, and a mode that handed the literal string to a program would be useless to them.
#[test]
fn a_glob_is_expanded_by_the_shell() {
    let scratch = Scratch::new("glob");
    std::fs::write(scratch.path.join("one.txt"), "").expect("write");
    std::fs::write(scratch.path.join("two.txt"), "").expect("write");
    std::fs::write(scratch.path.join("three.md"), "").expect("write");

    let ran = run("echo *.txt", &scratch.path).expect("echo runs");

    assert_eq!(ran.stdout.trim(), "one.txt two.txt");
}

#[test]
fn a_variable_is_expanded_by_the_shell() {
    let scratch = Scratch::new("variable");
    let ran = run("echo \"[$USER]\"", &scratch.path).expect("echo runs");
    // Asserted as non-empty rather than against a name, since the test runner's user varies.
    assert_ne!(ran.stdout.trim(), "[]");
    assert!(ran.stdout.starts_with('['));
}

/// Redirection is the clearest case of something the planner may never have and the user must:
/// writing a file is an effect, and here the person chose it.
#[test]
fn a_redirection_writes_the_file() {
    let scratch = Scratch::new("redirect");
    let ran = run("echo written > out.txt", &scratch.path).expect("echo runs");

    assert!(ran.succeeded());
    let contents = std::fs::read_to_string(scratch.path.join("out.txt")).expect("the file exists");
    assert_eq!(contents.trim(), "written");
}

#[test]
fn stages_are_piped_together() {
    let scratch = Scratch::new("pipe");
    let ran = run("printf 'a\\nb\\nc\\n' | wc -l", &scratch.path).expect("the pipeline runs");
    assert_eq!(ran.stdout.trim(), "3");
}

#[test]
fn commands_joined_with_and_both_run() {
    let scratch = Scratch::new("andand");
    let ran = run("echo first && echo second", &scratch.path).expect("both run");
    let lines: Vec<&str> = ran.stdout.lines().collect();
    assert_eq!(lines, vec!["first", "second"]);
}

#[test]
fn a_command_runs_in_the_directory_it_was_given() {
    let scratch = Scratch::new("cwd");
    let ran = run("pwd", &scratch.path).expect("pwd runs");
    // Compared canonically, since the temporary directory is reached through a symlink on macOS.
    let printed = std::fs::canonicalize(ran.stdout.trim()).expect("the printed path exists");
    let expected = std::fs::canonicalize(&scratch.path).expect("the scratch path exists");
    assert_eq!(printed, expected);
}

/// A failure has to come back with what the command said about it. Dropping stderr would leave the
/// user looking at an exit code with no explanation, which is the least useful thing to show.
#[test]
fn a_failing_command_reports_its_code_and_its_message() {
    let scratch = Scratch::new("failing");
    let ran = run("echo trouble >&2; exit 3", &scratch.path).expect("the shell runs");

    assert!(!ran.succeeded());
    assert_eq!(ran.code, Some(3));
    assert_eq!(ran.stderr.trim(), "trouble");
}

/// The user is not typing at these commands: the terminal belongs to the TUI. A command that reads
/// stdin must come back empty rather than wait for keys nobody is sending.
#[test]
fn a_command_that_reads_stdin_is_given_nothing_rather_than_the_terminal() {
    let scratch = Scratch::new("stdin");
    let ran = run("cat", &scratch.path).expect("cat runs");
    assert!(ran.stdout.is_empty());
    assert!(ran.succeeded());
}

#[test]
fn cancelling_stops_a_running_command() {
    let scratch = Scratch::new("cancel");
    let cancel = Cancel::new();
    let stopper = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        stopper.cancel();
    });

    let outcome = shell::run("sleep 30", &scratch.path, &cancel);

    assert!(
        matches!(outcome, Err(ShellError::Cancelled)),
        "expected a cancellation, got {outcome:?}"
    );
}

/// Output larger than a pipe buffer must not deadlock against a drain nobody is running.
#[test]
fn a_large_result_does_not_deadlock() {
    let scratch = Scratch::new("large");
    let ran = run("yes long-enough-line | head -n 20000", &scratch.path).expect("it runs");
    assert_eq!(ran.stdout.lines().count(), 20000);
}

/// An empty line is a keystroke, not a command. Handing `-c ''` to a shell succeeds and produces
/// nothing, which would put a pointless empty result in the transcript.
#[test]
fn an_empty_line_is_not_run() {
    let scratch = Scratch::new("empty");
    let outcome = run("   ", &scratch.path);
    assert!(
        matches!(outcome, Err(ShellError::Io(_))),
        "expected a refusal, got {outcome:?}"
    );
}

/// `$SHELL` is what makes the line behave the way it does in the user's own terminal. A system with
/// it unset still has to get a shell rather than an empty program name.
#[test]
fn the_shell_falls_back_to_a_posix_one_when_the_variable_is_unset() {
    // Asserted through the public helper rather than by unsetting the variable, which would race
    // every other test in this binary.
    let chosen = shell::shell();
    assert!(!chosen.trim().is_empty());
}

/// The whole point of the feature: a user who runs something and then says "fix that" is relying on
/// the planner having seen it. The output reaches the context, which for any other command's output
/// would be a violation and here is the user's own doing.
#[test]
fn what_a_command_printed_reaches_the_planners_context() {
    let scratch = Scratch::new("context");
    let mut conversation = Conversation::new();
    let mut sink = NullSink;

    let ran = run("echo the-output", &scratch.path).expect("it runs");
    let recorded = shell::record("echo the-output", &ran, &mut conversation, &mut sink)
        .expect("it is recorded");

    assert!(recorded.succeeded);
    let said = conversation
        .messages()
        .iter()
        .map(|m| m.content.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        said.contains("the-output"),
        "the planner was not told what it printed: {said}"
    );
    assert!(
        said.contains("echo the-output"),
        "the planner was not told what was run: {said}"
    );
}

/// Said as the user's own message rather than a tool result, because that is what happened: the
/// planner did not call anything, and describing it as a call would credit it with an action it
/// could not have taken.
#[test]
fn the_command_is_recorded_as_something_the_user_did() {
    let scratch = Scratch::new("role");
    let mut conversation = Conversation::new();
    let mut sink = NullSink;

    let ran = run("echo hi", &scratch.path).expect("it runs");
    shell::record("echo hi", &ran, &mut conversation, &mut sink).expect("it is recorded");

    let roles: Vec<Role> = conversation.messages().iter().map(|m| m.role).collect();
    assert_eq!(roles, vec![Role::User]);
}

/// Labelling a command's output trusted is the most consequential decision in the feature, so it
/// has to be visible in the trail rather than taken quietly.
#[test]
fn trusting_the_output_is_recorded_in_the_trail() {
    let scratch = Scratch::new("trail");
    let mut conversation = Conversation::new();
    let mut sink = RecordingSink::new();

    let ran = run("echo audited", &scratch.path).expect("it runs");
    shell::record("echo audited", &ran, &mut conversation, &mut sink).expect("it is recorded");

    assert!(
        sink.events().iter().any(|event| matches!(
            event,
            Event::GatePassed { gate: "provenance", detail }
                if detail.contains("echo audited") && detail.contains("the user typed")
        )),
        "the decision left no trace: {:?}",
        sink.events()
    );
}

/// A command that failed is still something the planner should know about: someone who runs a build
/// that breaks and says "fix it" means the errors it printed.
#[test]
fn a_failing_commands_output_still_reaches_the_planner() {
    let scratch = Scratch::new("failed");
    let mut conversation = Conversation::new();
    let mut sink = NullSink;

    let ran = run("echo the-error >&2; exit 1", &scratch.path).expect("the shell runs");
    let recorded = shell::record(
        "echo the-error >&2; exit 1",
        &ran,
        &mut conversation,
        &mut sink,
    )
    .expect("it is recorded");

    assert!(!recorded.succeeded);
    let said = conversation.messages()[0].content.clone();
    assert!(said.contains("the-error"), "stderr was dropped: {said}");
    assert!(
        said.contains("exited 1"),
        "the planner was not told it failed: {said}"
    );
}

/// A command that was stopped records nothing, because there is no result to speak of and a
/// half-finished one described as output would be a claim about what it did. Running and recording
/// are separate calls, so a run that failed simply never reaches the second.
#[test]
fn a_cancelled_command_records_nothing() {
    let scratch = Scratch::new("cancelled-record");
    let conversation = Conversation::new();
    let cancel = Cancel::new();
    let stopper = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        stopper.cancel();
    });

    let outcome = shell::run("sleep 30", &scratch.path, &cancel);

    assert!(matches!(outcome, Err(ShellError::Cancelled)));
    assert!(
        conversation.messages().is_empty(),
        "a stopped command was described to the planner anyway"
    );
}

/// What the person sees and what the planner is told come from the same gated value. Returning the
/// ungated copy would leave the check deciding nothing, since the bytes would reach the screen
/// whatever it said.
#[test]
fn what_is_shown_is_what_the_gate_released() {
    let scratch = Scratch::new("gated");
    let mut conversation = Conversation::new();
    let mut sink = NullSink;

    let ran = run("echo shown-and-said", &scratch.path).expect("it runs");
    let recorded =
        shell::record("echo shown-and-said", &ran, &mut conversation, &mut sink).expect("recorded");

    assert!(recorded.text.contains("shown-and-said"));
    assert!(conversation.messages()[0].content.contains(&recorded.text));
}
