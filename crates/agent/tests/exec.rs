//! Tests for running a pipeline, against real processes in a real directory.
//!
//! Everything here is about the plumbing rather than the gates: whether stages are chained the way
//! a shell would chain them, whether an argument survives intact, and whether a run that goes
//! wrong comes back with what it produced instead of nothing.

use bua_agent::exec::{self, ExecError};
use bua_core::cancel::Cancel;
use bua_core::{Pipeline, Stage};
use std::path::PathBuf;

/// A scratch directory that removes itself, so tests do not leave state behind.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("bua-exec-{name}"));
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

fn run(pipeline: Pipeline, at: &std::path::Path) -> Result<exec::Ran, ExecError> {
    exec::run(&pipeline, at, &Cancel::new())
}

#[test]
fn a_single_stage_returns_what_it_printed() {
    let scratch = Scratch::new("single");
    let ran = run(
        Pipeline::new(vec![Stage::new("echo", vec!["hello".into()])]),
        &scratch.path,
    )
    .expect("echo runs");
    assert_eq!(ran.stdout.trim(), "hello");
    assert!(ran.succeeded());
}

/// The reason `run` takes a pipeline rather than a single program: narrowing output has to be a
/// stage, because a pipe character would be a destination nobody approved.
#[test]
fn stages_are_chained_so_one_feeds_the_next() {
    let scratch = Scratch::new("chain");
    let ran = run(
        Pipeline::new(vec![
            Stage::new("printf", vec!["a\nb\nc\n".into()]),
            Stage::new("wc", vec!["-l".into()]),
        ]),
        &scratch.path,
    )
    .expect("the pipeline runs");
    assert_eq!(ran.stdout.trim(), "3");
    assert!(ran.succeeded());
}

/// The property the whole tool rests on. A metacharacter in an argument is one argument, because
/// nothing ever builds a command line for anything to re-parse.
#[test]
fn a_metacharacter_in_an_argument_stays_one_argument() {
    let scratch = Scratch::new("metachar");
    std::fs::write(scratch.path.join("keep.txt"), "still here").unwrap();

    let ran = run(
        Pipeline::new(vec![Stage::new(
            "echo",
            vec!["; rm -rf .".into(), "&& whoami".into(), "$(id)".into()],
        )]),
        &scratch.path,
    )
    .expect("echo runs");

    assert_eq!(ran.stdout.trim(), "; rm -rf . && whoami $(id)");
    assert!(
        scratch.path.join("keep.txt").exists(),
        "a metacharacter was interpreted rather than carried"
    );
}

/// An argument that looks like a redirection is text, not a destination.
#[test]
fn a_redirection_in_an_argument_writes_no_file() {
    let scratch = Scratch::new("redirect");
    let ran = run(
        Pipeline::new(vec![Stage::new(
            "echo",
            vec![">".into(), "written.txt".into()],
        )]),
        &scratch.path,
    )
    .expect("echo runs");
    assert_eq!(ran.stdout.trim(), "> written.txt");
    assert!(
        !scratch.path.join("written.txt").exists(),
        "an argument was treated as a redirection"
    );
}

#[test]
fn a_stage_runs_in_the_directory_it_was_given() {
    let scratch = Scratch::new("cwd");
    let ran = run(
        Pipeline::new(vec![Stage::new("pwd", Vec::new())]),
        &scratch.path,
    )
    .expect("pwd runs");
    // The scratch path may be reached through a symlink, so the tail is what is compared.
    assert!(
        ran.stdout.trim().ends_with("bua-exec-cwd"),
        "ran somewhere else: {}",
        ran.stdout.trim()
    );
}

/// A failing stage is reported as failing, and its explanation comes back rather than being
/// dropped. A run that produced something must not come back empty.
#[test]
fn a_failing_stage_reports_its_code_and_its_message() {
    let scratch = Scratch::new("failing");
    let ran = run(
        Pipeline::new(vec![Stage::new("ls", vec!["no-such-file".into()])]),
        &scratch.path,
    )
    .expect("ls runs even when it fails");
    assert!(!ran.succeeded());
    assert_eq!(ran.failures().len(), 1);
    assert!(
        !ran.stderr.is_empty(),
        "a stage explained itself and the explanation was dropped"
    );
}

/// A shell reports only the last stage, which hides the case that matters: an early stage failing
/// while a later one cheerfully processes the nothing it was handed.
#[test]
fn an_early_stage_failing_makes_the_whole_pipeline_fail() {
    let scratch = Scratch::new("early");
    let ran = run(
        Pipeline::new(vec![
            Stage::new("ls", vec!["no-such-file".into()]),
            Stage::new("wc", vec!["-l".into()]),
        ]),
        &scratch.path,
    )
    .expect("the pipeline runs");
    assert!(
        !ran.succeeded(),
        "an early failure was hidden by a later success"
    );
    assert_eq!(ran.failures().first().map(|(at, _)| *at), Some(1));
}

/// A program that is not installed is a refusal to report, not a panic. The name is safe to say
/// back: argv was endorsed by a person, so it is not something an attacker chose.
#[test]
fn a_program_that_does_not_exist_is_reported_by_name() {
    let scratch = Scratch::new("missing");
    let error = run(
        Pipeline::new(vec![Stage::new("bua-no-such-program-anywhere", Vec::new())]),
        &scratch.path,
    )
    .expect_err("a missing program cannot run");
    match error {
        ExecError::NotStarted { program, .. } => {
            assert_eq!(program, "bua-no-such-program-anywhere");
        }
        other => panic!("expected NotStarted, got {other:?}"),
    }
}

/// Nothing is typed at a program bua started, so a stage that reads stdin gets an empty one.
/// Inheriting the terminal's would hang the turn on input nobody is going to provide.
#[test]
fn a_stage_that_reads_stdin_is_given_nothing_rather_than_the_terminal() {
    let scratch = Scratch::new("stdin");
    let ran = run(
        Pipeline::new(vec![Stage::new("cat", Vec::new())]),
        &scratch.path,
    )
    .expect("cat runs and ends");
    assert_eq!(ran.stdout, "", "something was fed in that nobody approved");
    assert!(ran.succeeded());
}

/// A user who changes their mind does not wait out a slow program, and the program does not
/// survive the decision.
#[test]
fn cancelling_stops_a_running_pipeline() {
    let scratch = Scratch::new("cancel");
    let cancel = Cancel::new();
    let flag = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        flag.cancel();
    });

    let pipeline = Pipeline::new(vec![Stage::new("sleep", vec!["30".into()])]);
    let started = std::time::Instant::now();
    let error = exec::run(&pipeline, &scratch.path, &cancel).expect_err("cancelled");
    assert!(matches!(error, ExecError::Cancelled));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "cancelling did not stop the pipeline promptly"
    );
}

/// More output than a pipe buffer holds must not deadlock. The stages are chained by descriptor
/// and stderr is drained on its own thread precisely so this case works.
#[test]
fn a_large_result_does_not_deadlock() {
    let scratch = Scratch::new("large");
    let ran = run(
        Pipeline::new(vec![
            Stage::new("yes", vec!["padding-line".into()]),
            Stage::new("head", vec!["-n".into(), "200000".into()]),
            Stage::new("wc", vec!["-l".into()]),
        ]),
        &scratch.path,
    )
    .expect("the pipeline runs");
    assert_eq!(ran.stdout.trim(), "200000");
}
