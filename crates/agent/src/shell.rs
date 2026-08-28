//! Running a command line the user typed themselves.
//!
//! This is the one place in the repository that hands a string to a shell, and it exists for
//! exactly one caller: shell mode, where the person at the keyboard typed the line and pressed
//! Enter. Nothing the planner produces may reach here.
//!
//! # Why a shell is admissible here and nowhere else
//!
//! [`crate::exec`] takes an argument vector and never builds a command line, because the argv it
//! runs was chosen by the planner and a planner can be steered into choosing it. The exclusion of
//! shell strings is about that: a shell string fuses the destination with the payload, so there is
//! no separable routing field for a person to endorse, and endorsement is what makes planner-chosen
//! argv safe to run.
//!
//! A line the user typed has no such problem, because there is nothing left to endorse. The person
//! who would have been asked to approve the routing is the person who wrote it. Globs, `$VAR`,
//! redirection and `&&` are the reason someone opens a shell at all, and refusing them would make
//! shell mode a worse terminal than the one the user already has in the next window.
//!
//! So the exclusion still holds where it was always aimed: the planner has no shell, and
//! [`crate::exec`] must stay argv-only. Do not add a caller here that passes along anything a model
//! wrote, anything read from a file, or anything a processor produced. The provenance of the string
//! is the whole justification, and it cannot be checked from the bytes.
//!
//! # Not confined, like every other program
//!
//! Commands run with the access the user's own shell would give them, for the reason
//! [`crate::exec`] documents: `git push` needs `~/.ssh`, and the set of programs someone might ask
//! for cannot be enumerated. Shell mode changes nothing about that, since a user who wants to run
//! `rm -rf` in their own workspace is entitled to.

use crate::conversation::Conversation;
use bravebot_aichat::protocol::Message;
use bravebot_core::cancel::Cancel;
use bravebot_core::capability::{Capability, CapabilitySet};
use bravebot_core::event::Sink;
use bravebot_core::policy::{Policy, ReleasePlan, Routing};
use std::fmt;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long a command may run before it is given up on.
///
/// Longer than [`crate::exec::LIMIT`], and deliberately: a user watching their own command run can
/// see that it is slow and stop it themselves, where a turn's pipeline runs behind a spinner with
/// nobody watching. A build someone kicked off by hand is a normal thing to wait ten minutes for.
pub const LIMIT: Duration = Duration::from_secs(600);

/// How often the wait loop looks up to see whether it should stop.
const TICK: Duration = Duration::from_millis(50);

/// What running a command produced.
///
/// Plain strings, because this module labels nothing. The caller wraps them through
/// [`bravebot_core::policy::Policy::label_user_command_output`], which is where the decision that
/// they are the user's own belongs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// What the command printed to standard output.
    pub stdout: String,
    /// What it printed to standard error.
    ///
    /// Kept apart from stdout so the person reading the result can tell an explanation from an
    /// answer, and so a caller can show a failure differently.
    pub stderr: String,
    /// The exit status, or `None` where the shell was killed by a signal.
    pub code: Option<i32>,
}

impl Ran {
    /// Whether the command reported success.
    pub fn succeeded(&self) -> bool {
        self.code == Some(0)
    }
}

#[derive(Debug)]
pub enum ShellError {
    /// The shell itself could not be started, which means the system has no usable one.
    NotStarted { shell: String, detail: String },
    /// Still running when the limit ran out, and killed.
    TookTooLong { after: Duration },
    /// The user asked it to stop, and it was killed.
    Cancelled,
    /// The plumbing failed: a pipe that could not be created or read.
    Io(String),
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotStarted { shell, detail } => {
                write!(f, "'{shell}' could not be started: {detail}")
            }
            Self::TookTooLong { after } => write!(
                f,
                "still running after {} seconds, so it was stopped",
                after.as_secs()
            ),
            Self::Cancelled => f.write_str("stopped"),
            Self::Io(detail) => write!(f, "it could not be run: {detail}"),
        }
    }
}

impl std::error::Error for ShellError {}

/// Which shell to run the line through.
///
/// `$SHELL` so the line behaves the way it would in the user's own terminal, since that is the
/// expectation shell mode sets: their aliases are not loaded, but their syntax is theirs. `/bin/sh`
/// where the variable is unset or empty, which is the one shell a POSIX system is required to have.
pub fn shell() -> String {
    match std::env::var("SHELL") {
        Ok(shell) if !shell.trim().is_empty() => shell,
        _ => "/bin/sh".to_string(),
    }
}

/// Run `line` through the user's shell in `directory`, and collect what it printed.
///
/// `cancel` is checked while waiting, so a slow command can be stopped. Like
/// [`crate::exec::run`] this kills rather than detaching: a command still running is an effect in
/// progress, and asking it to stop is asking for it to end.
///
/// The line is passed as a single argument after `-c`, so the shell parses it and nothing here
/// does. There is no quoting to get wrong because nothing is interpolated: the string the user
/// typed is the string the shell receives.
pub fn run(line: &str, directory: &std::path::Path, cancel: &Cancel) -> Result<Ran, ShellError> {
    if line.trim().is_empty() {
        return Err(ShellError::Io("there was no command to run".to_string()));
    }

    let shell = shell();
    let mut child = match Command::new(&shell)
        .arg("-c")
        .arg(line)
        .current_dir(directory)
        // Nothing is typed at a command bravebot started: the terminal belongs to the TUI, which is
        // still drawing. A program that reads stdin gets nothing rather than fighting for the keys.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return Err(ShellError::NotStarted {
                shell,
                detail: e.to_string(),
            });
        }
    };

    // Drained on threads of their own, started before the wait. A command writing more than a pipe
    // holds would otherwise block forever against a buffer nobody is reading.
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());

    let started = Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            // Not a reason to keep waiting on something we can no longer ask about.
            Err(_) => break None,
            Ok(None) => {}
        }

        if cancel.is_cancelled() {
            stop(&mut child);
            return Err(ShellError::Cancelled);
        }
        let waited = started.elapsed();
        if waited >= LIMIT {
            stop(&mut child);
            return Err(ShellError::TookTooLong { after: waited });
        }

        std::thread::sleep(TICK);
    };

    // Collected after it exited, so every byte it wrote has been sent. A drain thread that vanished
    // contributes nothing rather than failing the run: the exit code already says how it went.
    Ok(Ran {
        stdout: out.and_then(|rx| rx.recv().ok()).unwrap_or_default(),
        stderr: err.and_then(|rx| rx.recv().ok()).unwrap_or_default(),
        code,
    })
}

/// What a command the user ran left behind.
pub struct Recorded {
    /// What it printed, stdout and stderr together, for the screen.
    pub text: String,
    /// Whether it succeeded, for saying so when it did not.
    pub succeeded: bool,
}

/// Put a command the user ran, and what it printed, into the conversation.
///
/// The planner reads both on its next turn, which is the point: someone who runs `git status` in
/// shell mode and then says "commit that" is relying on the agent having seen it.
///
/// Separate from [`run`] so the process runs on a worker thread while this stays with whoever owns
/// the conversation. Handing a `Conversation` to a thread means getting it back, and a thread that
/// panicked returns nothing, so the caller would have to substitute a fresh one: that would reset
/// context integrity to trusted, which is a label upgrade and is never allowed.
///
/// # Why this is not a hole
///
/// The bytes are labelled `(T,priv)` by [`Policy::label_user_command_output`], from provenance this
/// function is the sole witness to: a human typed the command. That is the same assertion the
/// trusted-programs list records when a user answers a run prompt with "remember this", and it is
/// justified the same way, by the user's say-so rather than by anything inspecting the output.
///
/// So this must only ever be called with a line a person typed at the prompt. It takes `line` as a
/// plain `&str` because there is no label that could carry that fact: provenance here is which
/// keystrokes produced the string, which is a property of the call site. Passing anything a model
/// wrote, anything read from a file, or anything a processor produced would launder it.
///
/// The command is recorded as a [`Message::user`] rather than a tool result, because that is what it
/// was: the user did something, and describing it as a call the planner made would credit the
/// planner with an action it did not take and could not have taken.
pub fn record<S: Sink>(
    line: &str,
    ran: &Ran,
    conversation: &mut Conversation,
    sink: &mut S,
) -> Result<Recorded, ShellError> {
    let mut text = ran.stdout.clone();
    if !ran.stderr.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&ran.stderr);
    }

    // A policy for the labelling alone. This is not a turn: nothing is read from the workspace, no
    // model is called, and the routing is the command the user typed, which anchors it to the one
    // trusted input there is.
    let mut routing = Routing::new();
    routing.insert_trusted("command", line.to_string());
    let mut policy = Policy::begin(
        routing,
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::ShellExec]),
        sink,
    )
    .map_err(|denial| ShellError::Io(denial.to_string()))?;

    let labelled = policy.label_user_command_output(line, text);

    // Everything downstream uses what the gate returned, never the string that went in, so the gate
    // decides whether these bytes go anywhere at all. Were a later change to label a command's
    // output untrusted, this refuses and neither the planner nor the screen sees it. Reading the
    // label and then using a copy taken beforehand would make the check decorative.
    let released = policy
        .read_trusted_content("shell", &labelled)
        .map_err(|denial| ShellError::Io(denial.to_string()))?;

    // Said in the driver's own words, from the exit code, which is structure rather than content.
    let outcome = if ran.succeeded() {
        String::new()
    } else {
        match ran.code {
            Some(code) => format!(" (exited {code})"),
            None => " (killed)".to_string(),
        }
    };

    let said = if released.trim().is_empty() {
        format!("I ran `{line}` in the shell myself{outcome}. It printed nothing.")
    } else {
        format!("I ran `{line}` in the shell myself{outcome}. It printed:\n\n{released}")
    };
    conversation.push(Message::user(said));

    Ok(Recorded {
        text: released,
        succeeded: ran.succeeded(),
    })
}

/// Read a pipe on a thread, lossily, so output that is not UTF-8 does not fail the run.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Option<mpsc::Receiver<String>> {
    let mut pipe = pipe?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut raw = Vec::new();
        let text = match pipe.read_to_end(&mut raw) {
            Ok(_) => String::from_utf8_lossy(&raw).into_owned(),
            Err(_) => String::new(),
        };
        let _ = tx.send(text);
    });
    Some(rx)
}

/// Kill the shell and reap it, so nothing is left behind.
///
/// Only the shell itself, which is the honest limit of what this can promise: a command that
/// forked, or a pipeline the shell built, may leave children this never sees. A user who stops
/// something and finds a straggler is in the same position they would be in with a Ctrl-C, and
/// pretending otherwise would need a process group this does not manage.
fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}
