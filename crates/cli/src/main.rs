//! Command-line entry point.

mod progress;

use bua_agent::Workspace;
use bua_agent::turn::{self, Task};
use bua_config::Config;
use bua_core::cancel::Cancel;
use bua_core::event::{Event, RecordingSink, Role};
use bua_core::trust::TrustStore;
use std::io::{IsTerminal, Read};
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("bua {VERSION}");
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") => {
            print_help();
            ExitCode::SUCCESS
        }
        // With no arguments the interactive session is the natural default.
        None => interactive(bua_tui::app::Start::Fresh),
        // Picking up where a session left off, chosen from a list or named outright.
        Some("--resume" | "-r") => match args.get(1) {
            Some(id) => resume_named(id),
            None => interactive(bua_tui::app::Start::Choose),
        },
        // The flag may lead, as it does for every other agent: `bua -p "task"`. Without this arm
        // it would be caught below as an unknown option.
        Some("-p" | "--print") => run_task(&args),
        Some("doctor") => doctor(),
        Some("import-leo-creds") => import_leo_creds(&args[1..]),
        Some(flag) if flag.starts_with('-') => {
            eprintln!("unknown option: {flag}");
            print_help();
            ExitCode::FAILURE
        }
        // Anything else is treated as the task prompt.
        Some(_) => run_task(&args),
    }
}

fn print_help() {
    println!("bua {VERSION}: a coding agent resistant to prompt injection");
    println!();
    println!("Usage:");
    println!("  bua                               Start an interactive session");
    println!("  bua \"<task>\" [--file <path>]...   Run a single task");
    println!("  cat file | bua -p \"<task>\"        ...with piped input, never trusted");
    println!("  bua --resume [id]                 Pick up a session in this directory");
    println!("  bua doctor                        Check configuration and confinement");
    println!("  bua import-leo-creds [channel]    Import a Leo Premium subscription");
    println!();
    println!("Interactive keys:");
    println!("  Enter                 Send");
    println!("  Ctrl-T                Toggle the audit trail");
    println!("  Up/Down               Walk back through sent prompts");
    println!("  Wheel, PageUp/Down    Scroll the transcript");
    println!("  Home/End              Jump to the start or the latest");
    println!("  Esc                   Cancel a running turn, clear the input, or leave");
    println!("  Ctrl-C                Leave");
    println!();
    println!("Options:");
    println!("  --file <path>    Include a workspace file as context (repeatable)");
    println!("  -p, --print      Non-interactive. Reads piped stdin as quarantined context");
    println!("  --trace          Print the audit trail");
    println!("  -h, --help       Show this message");
    println!("  -V, --version    Show the version");
}

/// Parse `<prompt> [--file path]... [--trace] [-p]`.
fn run_task(args: &[String]) -> ExitCode {
    let mut prompt = String::new();
    let mut files = Vec::new();
    let mut trace = false;
    let mut print = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--file" => match args.get(index + 1) {
                Some(path) => {
                    files.push(path.clone());
                    index += 2;
                }
                None => {
                    eprintln!("--file requires a path");
                    return ExitCode::FAILURE;
                }
            },
            "--trace" => {
                trace = true;
                index += 1;
            }
            "-p" | "--print" => {
                print = true;
                index += 1;
            }
            other if prompt.is_empty() => {
                prompt = other.to_string();
                index += 1;
            }
            other => {
                eprintln!("unexpected argument: {other}");
                return ExitCode::FAILURE;
            }
        }
    }

    // Read before the emptiness check below, since `cat notes.md | bua -p` is a complete
    // invocation: the pipe is the input and the prompt may be left off.
    let piped = if print {
        let stdin = std::io::stdin();
        let is_tty = stdin.is_terminal();
        match piped_input(stdin.lock(), is_tty) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };

    if prompt.is_empty() && piped.is_none() {
        eprintln!("a task is required");
        return ExitCode::FAILURE;
    }

    let config = match Config::from_env() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("configuration error: {err}");
            return ExitCode::FAILURE;
        }
    };

    let workspace = match current_workspace() {
        Ok(w) => w,
        Err(err) => {
            eprintln!("workspace error: {err}");
            return ExitCode::FAILURE;
        }
    };

    let egress = bua_net::Egress::new();
    let mut sink = RecordingSink::new();

    let mut task = Task::new(prompt);
    for file in files {
        task = task.with_file(file);
    }
    if let Some(text) = piped {
        task = task.with_piped_input(text);
    }

    // A one-shot run has nobody to ask about a write, so writes are refused rather than
    // silently applied.
    let mut confirmer = bua_agent::RefuseWrites;

    // Progress goes to stderr so stdout stays the reply and nothing else, which is what makes
    // the command pipeable. Without it a long turn prints nothing until it is over.
    let mut reporter = progress::Progress::new(std::io::stderr());

    match turn::run_cancellable(
        &config,
        &egress,
        &workspace,
        &task,
        &mut confirmer,
        &mut reporter,
        &mut sink,
        TrustStore::new(),
        &Cancel::new(),
    ) {
        Ok(outcome) => {
            // The reply is untrusted model output. Printing it is safe, since the
            // terminal is not a decision, so it is released explicitly for display.
            println!("{}", outcome.reply_for_display());
            if trace {
                println!();
                print_trace(&sink);
                println!("model: {}", outcome.model);
            }
            if outcome.clean {
                ExitCode::SUCCESS
            } else {
                eprintln!();
                eprintln!("note: a policy gate refused something during this turn");
                ExitCode::FAILURE
            }
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

/// What a pipe may carry before it is refused.
///
/// Matches the cap other agents document, and exists because the alternative is a `bua -p` that a
/// stray `cat` of a disk image holds open while it fills memory.
const PIPE_CAP: usize = 10 * 1024 * 1024;

/// Read input piped into the process, if any.
///
/// Generic over the source, as [`progress::Progress`] is over its sink, so a test can pass bytes
/// and a terminal answer without a real pipe.
///
/// A terminal means nothing was piped, and a read error means nobody is feeding us: neither is a
/// reason to stop, so the run continues on the argument prompt. Exceeding the cap is different,
/// since silently truncating would hand the model a fragment of what the user piped and say
/// nothing about it.
fn piped_input(source: impl Read, is_tty: bool) -> Result<Option<String>, String> {
    if is_tty {
        return Ok(None);
    }

    // One byte past the cap, so the buffer that proves the input was too large is not itself the
    // problem the cap exists to avoid.
    let mut buffer = Vec::new();
    if let Err(err) = source.take(PIPE_CAP as u64 + 1).read_to_end(&mut buffer) {
        eprintln!("warning: could not read piped input: {err}");
        return Ok(None);
    }

    if buffer.len() > PIPE_CAP {
        return Err(format!(
            "piped input is larger than {} MiB. Write it to a file and name that instead",
            PIPE_CAP / (1024 * 1024)
        ));
    }

    if buffer.is_empty() {
        return Ok(None);
    }

    // Lossy because the bytes are never decided from: they go into a slot and the planner is shown
    // a reference, so a replacement character changes nothing that matters.
    Ok(Some(String::from_utf8_lossy(&buffer).into_owned()))
}

/// Print the audit trail: what was checked, allowed, and refused.
fn print_trace(sink: &RecordingSink) {
    println!("audit trail");
    for event in sink.events() {
        match event {
            Event::GatePassed { gate, detail } => println!("  ok      {gate}: {detail}"),
            Event::GateBlocked { gate, reason, .. } => println!("  BLOCK   {gate}: {reason}"),
            Event::Observed { capability, label } => {
                println!("  observe {capability} produced {label}")
            }
            Event::SlotWritten { slot, label } => println!("  slot    {slot} at {label}"),
            Event::SlotDeferred {
                slot,
                label,
                origin,
            } => println!("  defer   {slot} holds {origin}, unread, at {label}"),
            Event::Declassified { slot, from, to, .. } => {
                println!("  release {slot} {from} -> {to}")
            }
            Event::ActionField {
                tool,
                field,
                role,
                label,
                allowed,
            } => {
                let mark = if *allowed { "ok     " } else { "BLOCK  " };
                let role = match role {
                    Role::Routing => "routing",
                    Role::Content => "content",
                };
                println!("  {mark} {tool}.{field} [{role}] {label}");
            }
        }
    }
}

/// Start an interactive session.
/// Resume a session by the id the picker shows, without showing the picker.
fn resume_named(id: &str) -> ExitCode {
    let Ok(directory) = std::env::current_dir() else {
        eprintln!("cannot tell which directory this is");
        return ExitCode::FAILURE;
    };
    match bua_tui::sessions::load(&directory, id) {
        Some(record) => interactive(bua_tui::app::Start::Resuming(Box::new(record))),
        None => {
            eprintln!("no session {id} in this directory");
            ExitCode::FAILURE
        }
    }
}

fn interactive(start: bua_tui::app::Start) -> ExitCode {
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("configuration error: {err}");
            return ExitCode::FAILURE;
        }
    };

    let workspace = match current_workspace() {
        Ok(w) => w,
        Err(err) => {
            eprintln!("workspace error: {err}");
            return ExitCode::FAILURE;
        }
    };

    // Reported in the status bar so the guarantee in force is visible for the whole
    // session rather than assumed.
    let confinement = match bua_sandbox::for_current_platform() {
        Ok(sandbox) => sandbox.capabilities().level.to_string(),
        Err(_) => "none".to_string(),
    };

    match bua_tui::app::run(&config, &workspace, confinement, start) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("interface error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The workspace is the current directory: file arguments resolve relative to it, and
/// confinement keeps reads inside it.
fn current_workspace() -> Result<Workspace, String> {
    std::env::current_dir()
        .map_err(|e| e.to_string())
        .and_then(|dir| Workspace::new(dir).map_err(|e| e.to_string()))
}

/// Import a Leo Premium subscription from a local Brave install.
///
/// This registers as an *additional* device rather than taking the browser's credentials, so the
/// browser keeps its own and nothing it holds is spent. Only the order id is read from the
/// profile; the credentials themselves are minted here and signed by Brave's service.
fn import_leo_creds(args: &[String]) -> ExitCode {
    let mut channel = None;
    let mut forget = false;

    for arg in args {
        match arg.as_str() {
            "--forget" => forget = true,
            other if other.starts_with('-') => {
                eprintln!("unknown option: {other}");
                return ExitCode::FAILURE;
            }
            other => match bua_skus::Channel::parse(other) {
                Some(parsed) => channel = Some(parsed),
                None => {
                    eprintln!("unknown channel: {other}");
                    eprintln!("expected one of: stable, beta, nightly, development");
                    return ExitCode::FAILURE;
                }
            },
        }
    }

    // Stable is what someone importing without saying which install means.
    let channel = channel.unwrap_or(bua_skus::Channel::Stable);

    if forget {
        return match bua_skus::store::clear(channel) {
            Ok(()) => {
                println!("forgot the {} subscription", channel.as_str());
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("{err}");
                ExitCode::FAILURE
            }
        };
    }

    // Warned about early: the import would otherwise succeed and then never be used, since a
    // credential is only ever sent to the premium host.
    match Config::from_env() {
        Ok(config) if config.premium_endpoint.is_none() => {
            eprintln!(
                "warning: this build has no premium endpoint, so imported credentials will not be used"
            );
            eprintln!(
                "         set {} and rebuild",
                bua_config::env_var::PREMIUM_ENDPOINT
            );
        }
        _ => {}
    }

    println!(
        "looking for a Leo subscription in Brave {}",
        channel.as_str()
    );

    let order = match bua_skus::find_leo_order(channel) {
        Ok(order) => order,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "found a {} subscription: {}",
        order.environment.as_str(),
        order.order_id
    );
    println!("registering this install as a new device");

    // A fresh request id is what makes this a new device rather than a claim on an existing
    // device's batch.
    let request_id = bua_skus::new_request_id();

    let registration =
        match bua_skus::device::register(order.environment, &order.order_id, &request_id) {
            Ok(registration) => registration,
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::FAILURE;
            }
        };

    let credentials: bua_skus::StoredCredentials = registration.into();
    let count = credentials.credentials.len();
    let last = credentials
        .credentials
        .iter()
        .map(|c| c.valid_to.as_str())
        .max()
        .unwrap_or("unknown")
        .to_string();

    if let Err(err) = bua_skus::store::save(channel, &credentials) {
        eprintln!("{err}");
        return ExitCode::FAILURE;
    }

    println!("stored {count} credentials in the system keychain, valid through {last}");
    println!("premium requests will now use them; the browser's own credentials were untouched");
    ExitCode::SUCCESS
}

/// Report whether configuration is usable, without revealing the signing key.
fn doctor() -> ExitCode {
    let mut ok = true;

    match Config::from_env() {
        Ok(config) => {
            println!("configuration OK");
            println!("  endpoint  {}", config.chat_completions_url());
            match config.premium_chat_completions_url() {
                Some(url) => println!("  premium   {url}"),
                None => println!("  premium   not configured"),
            }
            println!("  key id    {}", config.key_id);
            println!("  model     {}", config.model);
            println!("  key       {} (never transmitted)", config.signing_key);
            report_subscription();
        }
        Err(err) => {
            eprintln!("configuration error: {err}");
            ok = false;
        }
    }

    println!();
    report_confinement(&mut ok);

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Report which channels have an imported subscription, and how much of it is left.
///
/// Counts only: a credential is a bearer secret, so none of it is printed.
fn report_subscription() {
    for channel in bua_skus::Channel::ALL {
        if let Ok(stored) = bua_skus::store::load(channel) {
            println!(
                "  leo       {} subscription imported, {} of {} credentials unspent",
                channel.as_str(),
                stored.remaining(),
                stored.credentials.len()
            );
        }
    }
}

/// Report the confinement actually achieved here.
///
/// Printed rather than assumed: the guarantee differs by platform and kernel, and a
/// user is entitled to know which one they have before trusting the sandbox.
fn report_confinement(ok: &mut bool) {
    match bua_sandbox::for_current_platform() {
        Ok(sandbox) => {
            let caps = sandbox.capabilities();
            println!("confinement {}", caps.level);
            println!("  mechanisms       {}", caps.mechanisms.join(", "));
            println!(
                "  network denial   {}",
                if caps.network_denial_enforced {
                    "kernel-enforced"
                } else {
                    "NOT enforced"
                }
            );
        }
        Err(err) => {
            // Not a warning: without confinement, untrusted work will be refused
            // rather than run, so this is a hard problem for the user to solve.
            eprintln!("confinement unavailable");
            eprintln!("  {err}");
            *ok = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An interactive `bua -p "task"` must not block waiting for a pipe that is not coming.
    #[test]
    fn a_terminal_stdin_is_not_read() {
        let source: &[u8] = b"this would be read from a pipe";
        assert_eq!(piped_input(source, true), Ok(None));
    }

    #[test]
    fn piped_bytes_are_read_when_stdin_is_not_a_terminal() {
        let source: &[u8] = b"a build log\n";
        assert_eq!(piped_input(source, false), Ok(Some("a build log\n".into())));
    }

    /// Truncating would hand the planner a fragment of what the user piped without saying so, and
    /// a quarantined fragment is one nobody can notice is short.
    #[test]
    fn input_over_the_cap_is_refused() {
        let oversized = vec![b'x'; PIPE_CAP + 1];
        assert!(
            piped_input(oversized.as_slice(), false).is_err(),
            "an oversized pipe must be refused, not truncated"
        );

        let at_the_cap = vec![b'x'; PIPE_CAP];
        assert!(
            piped_input(at_the_cap.as_slice(), false).is_ok(),
            "the cap itself is allowed"
        );
    }
}
