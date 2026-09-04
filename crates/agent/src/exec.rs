//! Running a pipeline of argv stages.
//!
//! Process plumbing and nothing else. Every decision about whether a pipeline may run at all is
//! taken before anything here is called: the capability, the person's approval, and the label the
//! output will carry all belong to [`bravebot_core::policy::Policy`]. This module is what happens after
//! those said yes, so it takes a [`Pipeline`] of plain strings and reports what came back.
//!
//! # No shell, at any point
//!
//! Each stage is spawned with its program and its argument vector passed directly to the operating
//! system. Nothing is concatenated into a string and nothing re-parses one, so an argument
//! containing `; rm -rf /` is one argument and arrives as one argument. That is the property the
//! whole tool rests on, and it lives here: a future change that built a command line out of these
//! parts would break it without any gate noticing.
//!
//! Stages are chained the way a shell chains them, by handing one child's stdout to the next
//! child's stdin as a file descriptor. The operating system moves the bytes, so a large
//! intermediate result cannot deadlock against a buffer we forgot to drain.
//!
//! # What runs is what was resolved
//!
//! Each stage is spawned by the resolved path its caller worked out, not by the name in the
//! [`Pipeline`]. The name was resolved once, before the person was asked; spawning by name here
//! would resolve it a second time, leaving a window in which `$PATH` changed and something other
//! than what they approved ran. See [`crate::programs`].
//!
//! # Not confined
//!
//! Deliberately. `bravebot-sandbox` confines processes running code we did not write; these run with
//! whatever access the user's own shell would give them, because `git push` needs `~/.ssh` and the
//! set of programs someone might ask for cannot be enumerated in advance. What holds is the label
//! on the output, not any belief about the binary. Whether to confine children is issue #4.
//!
//! # Nothing on stdin unless it was approved
//!
//! The first stage is given an empty stdin rather than the terminal's. A program that reads stdin
//! would otherwise block forever on input nobody is typing, and the turn would hang with no
//! indication of why.
//!
//! # This agent's own credentials are not handed on
//!
//! The environment is inherited, less the names in [`scrub`]. A person approving a run reads the
//! binary, the argv and the directory, so a credential travelling alongside those is something they
//! were never shown and could not have endorsed. The signing key has no use in a subprocess anyway:
//! a program the planner chose is doing what somebody approved, not authenticating as this agent.
//!
//! The rest of the environment stays. `run aws s3 ls` and `run gh pr list` are things people ask
//! for, and a filter matching names cannot tell one of those from an exfiltration, so it is not
//! attempted: what holds is narrow and exact rather than broad and approximate.

use bravebot_core::Pipeline;
use bravebot_core::cancel::Cancel;
use std::fmt;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a pipeline may run before it is given up on.
///
/// A program that never terminates would otherwise hold the turn open with nothing to show for
/// it. Generous, because a build or a test run is a reasonable thing to ask for and a limit that
/// cuts those off is worse than no limit for the user who was waiting.
///
/// Reaching it is not an error. What the stages printed before they were killed comes back the
/// same way it does from a pipeline that ended by itself, marked with [`Ran::stopped`].
pub const LIMIT: Duration = Duration::from_secs(300);

/// How often the wait loop looks up to see whether it should stop.
const TICK: Duration = Duration::from_millis(50);

/// How long the drain threads are given to reach the end of their pipe, once the stages are done.
///
/// They are waited on rather than joined, because a stage can leave a child holding the write end
/// of the pipe. `sh -c 'echo hi; sleep 30'` is enough: killing the shell leaves `sleep` holding
/// the same descriptor, so the pipe never reaches end-of-file and a joining thread waits on a
/// process nobody is waiting for. Whatever was read by the time the grace runs out is kept, which
/// is what makes the wait safe to abandon.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// What running a pipeline produced.
///
/// The text fields are what the programs printed. They are returned as plain `String`s because
/// this module has no business labelling anything: the caller wraps them at the label
/// [`bravebot_core::policy::Policy::before_run`] already decided, which is `(U,priv)` whatever is in
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// The last stage's standard output.
    pub stdout: String,
    /// Standard error, from every stage, in stage order.
    ///
    /// Kept apart from stdout because a stage that failed usually explains itself here, and a
    /// person reading the result should be able to tell the explanation from the output.
    pub stderr: String,
    /// The exit code of each stage, in order. `None` where a stage was killed by a signal.
    pub codes: Vec<Option<i32>>,
    /// How long the pipeline had run when the limit ran out and it was killed, or `None` if every
    /// stage exited on its own.
    ///
    /// A pipeline that outstays the limit is stopped, but it is not a pipeline that did not run:
    /// a server asked to serve a page serves it for five minutes and prints as it goes. Reporting
    /// only the failure threw that away and left the caller unable to tell a program that hung
    /// from one that was working exactly as asked. So the output is collected either way and the
    /// distinction is carried here, where a caller can say which happened without reading a byte
    /// of what was printed.
    pub stopped: Option<Duration>,
}

impl Ran {
    /// Whether every stage reported success.
    ///
    /// A pipeline is as good as its worst stage. A shell would report only the last one, which
    /// hides the case that matters most here: an early stage failing and a later one cheerfully
    /// processing the nothing it was handed.
    pub fn succeeded(&self) -> bool {
        // A pipeline that had to be killed did not succeed, whatever its stages had managed to
        // report by then. Stages still running have no code at all, so this would be false in
        // most cases anyway; it is stated rather than relied on.
        self.stopped.is_none() && self.codes.iter().all(|code| *code == Some(0))
    }

    /// The stages that did not succeed, as `1-indexed position, code`, for a short report.
    pub fn failures(&self) -> Vec<(usize, Option<i32>)> {
        self.codes
            .iter()
            .enumerate()
            .filter(|(_, code)| **code != Some(0))
            .map(|(index, code)| (index + 1, *code))
            .collect()
    }
}

#[derive(Debug)]
pub enum ExecError {
    /// The program could not be started: usually not installed, or not executable.
    ///
    /// Carries the program name, which is safe to report: argv was endorsed by a person, so it is
    /// not content an attacker chose.
    NotStarted { program: String, detail: String },
    /// The user asked the turn to stop while this was running, and it has been killed.
    Cancelled,
    /// The plumbing itself failed: a pipe that could not be created or read.
    Io(String),
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotStarted { program, detail } => {
                write!(f, "'{program}' could not be started: {detail}")
            }
            Self::Cancelled => f.write_str("stopped because the turn was cancelled"),
            Self::Io(detail) => write!(f, "the pipeline could not be run: {detail}"),
        }
    }
}

impl std::error::Error for ExecError {}

/// Run a pipeline in `directory`, and collect what it printed.
///
/// `cancel` is checked while waiting, so a user who changes their mind does not have to wait out a
/// slow program. Unlike cancellation between rounds, this one kills: a child already running is an
/// effect in progress, and the request to stop is precisely a request to end it.
pub fn run(
    pipeline: &Pipeline,
    resolved: &[std::path::PathBuf],
    directory: &std::path::Path,
    cancel: &Cancel,
) -> Result<Ran, ExecError> {
    run_within(pipeline, resolved, directory, cancel, LIMIT)
}

/// [`run`], with the limit named rather than taken from [`LIMIT`].
///
/// Exists so the stopping behaviour can be tested: a test that had to wait out the real limit to
/// see what a killed pipeline returns would take five minutes, and one nobody runs proves nothing.
/// Callers in the product use [`run`], so there is one limit in force and it is the documented one.
pub fn run_within(
    pipeline: &Pipeline,
    resolved: &[std::path::PathBuf],
    directory: &std::path::Path,
    cancel: &Cancel,
    limit: Duration,
) -> Result<Ran, ExecError> {
    if pipeline.is_empty() {
        return Err(ExecError::Io("no stages to run".to_string()));
    }
    if resolved.len() != pipeline.len() {
        return Err(ExecError::Io(
            "every stage must have been resolved to a program before it runs".to_string(),
        ));
    }

    let mut children: Vec<Child> = Vec::with_capacity(pipeline.len());
    // Nothing is typed at a program bravebot started, so the first stage reads an empty stdin rather
    // than the terminal's. Inheriting it would let a program that reads stdin hang the turn.
    let mut upstream = Stdio::null();
    // The last stage's stdout is the pipeline's result, so it is kept here rather than handed
    // onwards. Taking it into the chain like the others left nothing to read and the whole run
    // came back empty.
    let mut tail_out = None;
    let last = pipeline.len() - 1;

    for (index, stage) in pipeline.stages.iter().enumerate() {
        // The resolved path, never the name. The name was resolved once, before the person was
        // asked, and running it again by name would leave a window in which `$PATH` changed and
        // something other than what they approved executed.
        let mut command = Command::new(&resolved[index]);
        // The vector, never a string. Nothing here builds a command line, so nothing has to
        // unbuild one.
        command
            .args(&stage.args)
            .current_dir(directory)
            // Taken rather than moved, so the compiler can see every iteration starts with a
            // stdin of its own. The last stage keeps its output instead of passing it on, which
            // leaves this holding a null it never uses.
            .stdin(std::mem::replace(&mut upstream, Stdio::null()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Every stage, not only the first. A credential is as reachable from the middle of a
        // pipeline as from the front, and one stage spared would be the whole of the hole.
        crate::scrub::apply(&mut command);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                // Whatever started already is killed rather than left running behind a pipeline
                // that will never complete.
                for mut started in children {
                    let _ = started.kill();
                    let _ = started.wait();
                }
                return Err(ExecError::NotStarted {
                    program: stage.program.clone(),
                    detail: e.to_string(),
                });
            }
        };

        // Handed to the next stage as a file descriptor, so the operating system moves the bytes
        // between them and no buffer of ours can fill up and deadlock. The last stage has no next
        // stage, so its output is kept to be read.
        match child.stdout.take() {
            Some(out) if index == last => tail_out = Some(out),
            Some(out) => upstream = Stdio::from(out),
            None => upstream = Stdio::null(),
        }
        children.push(child);
    }

    // Drained on threads of their own, for the same reason the stages are chained by descriptor: a
    // stage that writes more to stderr than a pipe holds would block forever if nobody were
    // reading while we waited for it to exit.
    let mut draining: Vec<Drain> = Vec::with_capacity(children.len());
    for child in &mut children {
        if let Some(err) = child.stderr.take() {
            draining.push(Drain::reading(err));
        }
    }

    // Read on a thread of its own, started before the wait, so the last stage's output is being
    // collected while it is still being written. Waiting first and reading afterwards would
    // deadlock as soon as a program produced more than a pipe buffer holds.
    let tail = tail_out.map(Drain::reading);

    let started = Instant::now();
    let mut codes = vec![None; children.len()];
    let mut finished = vec![false; children.len()];
    let mut stopped = None;

    loop {
        for (index, child) in children.iter_mut().enumerate() {
            if finished[index] {
                continue;
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    codes[index] = status.code();
                    finished[index] = true;
                }
                Ok(None) => {}
                // A child we cannot ask about is not one to keep waiting for.
                Err(_) => finished[index] = true,
            }
        }

        if finished.iter().all(|done| *done) {
            break;
        }

        // Both of these kill. A pipeline still running is an effect in progress, and neither a
        // cancellation nor the limit is served by leaving it to finish unwatched.
        if cancel.is_cancelled() {
            stop(&mut children);
            return Err(ExecError::Cancelled);
        }
        let waited = started.elapsed();
        if waited >= limit {
            // Killed, then collected like any other end: the stages are over either way, and the
            // bytes they wrote before they were stopped are the only account of what happened.
            stop(&mut children);
            stopped = Some(waited);
            break;
        }

        std::thread::sleep(TICK);
    }

    // Collected once the stages are over, whether they ended on their own or were killed. The
    // usual case is that every pipe reached its end the moment the stage writing to it did, and
    // the wait returns at once; the grace is there for the pipe something else is still holding,
    // and what had been read by then is taken rather than lost.
    let waiting = Instant::now();
    while waiting.elapsed() < DRAIN_GRACE
        && !(tail.as_ref().is_none_or(Drain::finished) && draining.iter().all(Drain::finished))
    {
        std::thread::sleep(TICK);
    }

    let stdout = tail.as_ref().map(Drain::text).unwrap_or_default();
    let stderr = draining
        .iter()
        .map(Drain::text)
        .collect::<Vec<_>>()
        .concat();

    Ok(Ran {
        stdout,
        stderr,
        codes,
        stopped,
    })
}

/// A pipe being read on a thread of its own, and what has been read so far.
///
/// The bytes are shared rather than sent at the end, because the end may not come: the reader can
/// be left on a pipe a killed stage's own child is still holding open. Anything that handed over
/// its result only on completion would have nothing to hand over in exactly the case where what
/// was printed matters most, which is the pipeline that had to be stopped.
struct Drain {
    read: Arc<Mutex<Vec<u8>>>,
    done: Arc<AtomicBool>,
}

impl Drain {
    /// Start reading `source` into a buffer the caller can look at at any time.
    fn reading<R: Read + Send + 'static>(mut source: R) -> Self {
        let read = Arc::new(Mutex::new(Vec::new()));
        let done = Arc::new(AtomicBool::new(false));
        let into = Arc::clone(&read);
        let ended = Arc::clone(&done);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            loop {
                match source.read(&mut chunk) {
                    // End of the pipe, or a pipe that can no longer be read. Either way there is
                    // no more to come, and a read error is not a failure of the run.
                    Ok(0) | Err(_) => break,
                    Ok(n) => into
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .extend_from_slice(&chunk[..n]),
                }
            }
            ended.store(true, Ordering::Release);
        });
        Self { read, done }
    }

    /// Whether the pipe has reached its end, so there is nothing further to wait for.
    fn finished(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    /// What has been read so far.
    ///
    /// A stage printing bytes that are not UTF-8 is not a failure of the run, so the text is taken
    /// lossily rather than discarded.
    fn text(&self) -> String {
        let read = self.read.lock().unwrap_or_else(|e| e.into_inner());
        String::from_utf8_lossy(&read).into_owned()
    }
}

/// Kill every stage and reap it, so nothing is left behind.
fn stop(children: &mut [Child]) {
    for child in children.iter_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
