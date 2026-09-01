//! Command-line entry point.

mod progress;

use bravebot_agent::Workspace;
use bravebot_agent::turn::{self, Task};
use bravebot_config::Config;
use bravebot_core::cancel::Cancel;
use bravebot_core::event::{Event, RecordingSink, Role};
use bravebot_core::trust::TrustStore;
use bravebot_i18n::t;
use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    // Before anything is printed, and exactly once: every later lookup reads what this settled
    // on. Nothing else in the tree consults the environment about a language.
    bravebot_i18n::init_from_environment();

    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            // The same words a session record writes down, so the two can be compared without
            // anyone having to work out what "the current build" means.
            println!("bravebot {}", bravebot_tui::BUILD);
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") => {
            print_help();
            ExitCode::SUCCESS
        }
        // With no arguments the interactive session is the natural default.
        None => interactive(bravebot_tui::app::Start::Fresh),
        // Picking up where a session left off, chosen from a list or named outright.
        Some("--resume" | "-r") => match args.get(1) {
            Some(id) => resume_named(id),
            None => interactive(bravebot_tui::app::Start::Choose),
        },
        // The flag may lead, as it does for every other agent: `bravebot -p "task"`. Without this arm
        // it would be caught below as an unknown option.
        Some("-p" | "--print") => run_task(&args),
        Some("doctor") => doctor(),
        Some("import-leo-creds") => import_leo_creds(&args[1..]),
        Some(flag) if flag.starts_with('-') => {
            eprintln!("{}", t!(cli_unknown_option, flag = flag));
            print_help();
            ExitCode::FAILURE
        }
        // Anything else is treated as the task prompt.
        Some(_) => run_task(&args),
    }
}

fn print_help() {
    /// Wide enough for the longest invocation below, so a translated description starts in the
    /// same column as every other one rather than wherever hand-counted spaces left it.
    const FORM: usize = 39;
    /// The same, for the key column and the option column, which are narrower.
    const KEY: usize = 22;
    const OPTION: usize = 17;

    println!("{}", t!(cli_tagline, version = VERSION));
    println!();
    println!("{}", t!(cli_usage_heading));
    for (form, description) in [
        ("bravebot", t!(cli_usage_interactive)),
        ("bravebot \"<task>\" [--file <path>]...", t!(cli_usage_task)),
        ("cat file | bravebot -p \"<task>\"", t!(cli_usage_piped)),
        ("bravebot --resume [id]", t!(cli_usage_resume)),
        ("bravebot doctor", t!(cli_usage_doctor)),
        ("bravebot import-leo-creds [channel]", t!(cli_usage_import)),
    ] {
        println!("  {form:<FORM$}{description}");
    }
    println!();

    println!("{}", t!(cli_keys_heading));
    for (keys, description) in [
        ("Enter", t!(cli_key_send)),
        ("Ctrl-T", t!(cli_key_audit)),
        ("Up/Down", t!(cli_key_history)),
        ("Wheel, PageUp/Down", t!(cli_key_scroll)),
        ("Home/End", t!(cli_key_jump)),
        ("Esc", t!(cli_key_cancel)),
        ("Ctrl-C", t!(cli_key_leave)),
    ] {
        println!("  {keys:<KEY$}{description}");
    }
    println!();

    // Listed from the commands themselves, so one renamed or added cannot leave this advertising a
    // word that no longer works. The interface offers the same list when a slash is typed.
    println!("{}", t!(cli_commands_heading));
    for command in bravebot_tui::app::commands() {
        let word = if command.argument.is_empty() {
            command.name.to_string()
        } else {
            format!("{} {}", command.name, command.argument)
        };
        println!("  {word:<20}  {}", command.description);
    }
    // Not a command, but typed in the same place and worth finding here.
    println!("  {:<20}  {}", "@<path>", t!(cli_name_a_file));
    println!();

    println!("{}", t!(cli_options_heading));
    for (flags, description) in [
        ("--file <path>", t!(cli_option_file)),
        ("-p, --print", t!(cli_option_print)),
        ("--trace", t!(cli_option_trace)),
        ("-h, --help", t!(cli_option_help)),
        ("-V, --version", t!(cli_option_version)),
    ] {
        println!("  {flags:<OPTION$}{description}");
    }
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
                    eprintln!("{}", t!(cli_file_needs_a_path));
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
                eprintln!("{}", t!(cli_unexpected_argument, argument = other));
                return ExitCode::FAILURE;
            }
        }
    }

    // Read before the emptiness check below, since `cat notes.md | bravebot -p` is a complete
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
        eprintln!("{}", t!(cli_task_required));
        return ExitCode::FAILURE;
    }

    let config = match Config::from_env() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("{}", t!(cli_configuration_problem, problem = err));
            return ExitCode::FAILURE;
        }
    };

    let workspace = match current_workspace() {
        Ok(w) => w,
        Err(err) => {
            eprintln!("{}", t!(cli_workspace_problem, problem = err));
            return ExitCode::FAILURE;
        }
    };

    let egress = bravebot_net::Egress::new();
    let mut sink = RecordingSink::new();

    // The same choice the interface records. A preference about which model to think with is the
    // user's, not the interface's, so a one-shot run honours it rather than reverting to the
    // default the environment happens to name.
    let mut task = Task::new(prompt)
        .with_home(bravebot_agent::home::directory())
        .with_model(bravebot_tui::store::load_model());
    for file in files {
        task = task.with_file(file);
    }
    if let Some(text) = piped {
        task = task.with_piped_input(text);
    }

    // A one-shot run has nobody to ask about a write, so writes are refused rather than
    // silently applied.
    let mut confirmer = bravebot_agent::Unattended;

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
            report(
                &mut std::io::stdout().lock(),
                &mut std::io::stderr().lock(),
                &Finished {
                    reply: outcome.reply_for_display(),
                    notices: &outcome.notices,
                    trail: trace.then_some((&sink, outcome.model.as_str())),
                    clean: outcome.clean,
                },
            );
            if outcome.clean {
                ExitCode::SUCCESS
            } else {
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
/// Matches the cap other agents document, and exists because the alternative is a `bravebot -p` that a
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
        eprintln!("{}", t!(cli_piped_input_unreadable, problem = err));
        return Ok(None);
    }

    if buffer.len() > PIPE_CAP {
        return Err(t!(
            cli_piped_input_too_large,
            limit = PIPE_CAP / (1024 * 1024)
        ));
    }

    if buffer.is_empty() {
        return Ok(None);
    }

    // Lossy because the bytes are never decided from: they go into a slot and the planner is shown
    // a reference, so a replacement character changes nothing that matters.
    Ok(Some(String::from_utf8_lossy(&buffer).into_owned()))
}

/// What a finished turn has to say, before anything decides where it goes.
struct Finished<'a> {
    reply: &'a str,
    /// The driver's own words about what loaded and what did not, never anything read out of a
    /// file.
    notices: &'a [String],
    /// The trail and the model behind it, when `--trace` asked for them.
    trail: Option<(&'a RecordingSink, &'a str)>,
    /// Whether no gate refused anything during the turn.
    clean: bool,
}

/// Write a finished turn: the reply to `reply`, every other word to `beside`.
///
/// Which stream each part lands on is the whole of what this decides, so it takes both rather
/// than reaching for stdout and stderr itself: a run's output is then something a test can read
/// back. A notice or an audit trail sharing the reply's stream would corrupt whatever the reply
/// was piped into.
fn report(reply: &mut impl Write, beside: &mut impl Write, run: &Finished<'_>) {
    for notice in run.notices {
        let _ = writeln!(beside, "{}", t!(cli_notice, notice = notice));
    }
    let _ = writeln!(reply, "{}", run.reply);
    if let Some((sink, model)) = run.trail {
        let _ = writeln!(beside);
        print_trace(beside, sink);
        let _ = writeln!(beside, "{}", t!(cli_model_used, model = model));
    }
    if !run.clean {
        let _ = writeln!(beside);
        let _ = writeln!(beside, "{}", t!(cli_something_was_refused));
    }
}

/// Print the audit trail: what was checked, allowed, and refused.
///
/// A failed write is dropped, as it is for progress. The trail describes a turn that has already
/// happened, so a closed stderr means nobody is reading it, not that the run should die holding a
/// reply it has already produced.
fn print_trace(output: &mut impl Write, sink: &RecordingSink) {
    macro_rules! trace {
        ($($arg:tt)*) => {
            { let _ = writeln!(output, $($arg)*); }
        };
    }

    trace!("audit trail");
    for event in sink.events() {
        match event {
            Event::GatePassed { gate, detail } => trace!("  ok      {gate}: {detail}"),
            Event::GateBlocked { gate, reason, .. } => trace!("  BLOCK   {gate}: {reason}"),
            Event::Observed { capability, label } => {
                trace!("  observe {capability} produced {label}")
            }
            Event::SlotWritten { slot, label } => trace!("  slot    {slot} at {label}"),
            Event::SlotDeferred {
                slot,
                label,
                origin,
            } => trace!("  defer   {slot} holds {origin}, unread, at {label}"),
            Event::Declassified { slot, from, to, .. } => trace!("  release {slot} {from} -> {to}"),
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
                trace!("  {mark} {tool}.{field} [{role}] {label}");
            }
        }
    }
}

/// Start an interactive session.
/// Resume a session by the id the picker shows, without showing the picker.
fn resume_named(id: &str) -> ExitCode {
    let Ok(directory) = std::env::current_dir() else {
        eprintln!("{}", t!(cli_directory_unknown));
        return ExitCode::FAILURE;
    };
    match bravebot_tui::sessions::load(&directory, id) {
        Some(record) => interactive(bravebot_tui::app::Start::Resuming(Box::new(record))),
        None => {
            eprintln!("{}", t!(cli_no_such_session, id = id));
            ExitCode::FAILURE
        }
    }
}

fn interactive(start: bravebot_tui::app::Start) -> ExitCode {
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("{}", t!(cli_configuration_problem, problem = err));
            return ExitCode::FAILURE;
        }
    };

    let workspace = match current_workspace() {
        Ok(w) => w,
        Err(err) => {
            eprintln!("{}", t!(cli_workspace_problem, problem = err));
            return ExitCode::FAILURE;
        }
    };

    // Reported in the status bar so the guarantee in force is visible for the whole
    // session rather than assumed.
    let confinement = match bravebot_sandbox::for_current_platform() {
        Ok(sandbox) => sandbox.capabilities().level.to_string(),
        Err(_) => "none".to_string(),
    };

    match bravebot_tui::app::run(&config, &workspace, confinement, start) {
        // Printed after the terminal is handed back, so it survives on the screen the person is
        // left looking at rather than going onto the alternate screen with everything else. A
        // session is worth resuming far more often than anybody thinks to write its name down
        // beforehand, and the picker is no help to someone who has already closed the window.
        Ok(Some(id)) => {
            println!("{}", t!(cli_resume_heading));
            println!("bravebot --resume {id}");
            ExitCode::SUCCESS
        }
        Ok(None) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{}", t!(cli_interface_problem, problem = err));
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
                eprintln!("{}", t!(cli_unknown_option, flag = other));
                return ExitCode::FAILURE;
            }
            other => match bravebot_skus::Channel::parse(other) {
                Some(parsed) => channel = Some(parsed),
                None => {
                    eprintln!("{}", t!(leo_unknown_channel, channel = other));
                    eprintln!("{}", t!(leo_expected_channel));
                    return ExitCode::FAILURE;
                }
            },
        }
    }

    // Stable is what someone importing without saying which install means.
    let channel = channel.unwrap_or(bravebot_skus::Channel::Stable);

    if forget {
        return match bravebot_skus::store::clear(channel) {
            Ok(()) => {
                println!("{}", t!(leo_forgotten, channel = channel.as_str()));
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
            eprintln!("{}", t!(leo_no_premium_endpoint));
            eprintln!(
                "         {}",
                t!(
                    leo_set_and_rebuild,
                    variable = bravebot_config::env_var::PREMIUM_ENDPOINT
                )
            );
        }
        _ => {}
    }

    println!("{}", t!(leo_looking, channel = channel.as_str()));

    let order = match bravebot_skus::find_leo_order(channel) {
        Ok(order) => order,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "{}",
        t!(
            leo_found,
            environment = order.environment.as_str(),
            order = &order.order_id
        )
    );
    println!("{}", t!(leo_registering));

    // A fresh request id is what makes this a new device rather than a claim on an existing
    // device's batch.
    let request_id = bravebot_skus::new_request_id();

    let registration =
        match bravebot_skus::device::register(order.environment, &order.order_id, &request_id) {
            Ok(registration) => registration,
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::FAILURE;
            }
        };

    let credentials: bravebot_skus::StoredCredentials = registration.into();
    let count = credentials.credentials.len();
    let last = credentials
        .credentials
        .iter()
        .map(|c| c.valid_to.as_str())
        .max()
        .unwrap_or("unknown")
        .to_string();

    if let Err(err) = bravebot_skus::store::save(channel, &credentials) {
        eprintln!("{err}");
        return ExitCode::FAILURE;
    }

    println!("{}", t!(leo_stored, count = count, expiry = last));
    println!("{}", t!(leo_browser_untouched));
    ExitCode::SUCCESS
}

/// Report whether configuration is usable, without revealing the signing key.
fn doctor() -> ExitCode {
    let mut ok = true;

    match Config::from_env() {
        Ok(config) => {
            println!("{}", t!(doctor_configuration_ok));
            fact(t!(doctor_endpoint), config.chat_completions_url());
            match config.premium_chat_completions_url() {
                Some(url) => fact(t!(doctor_premium), &url),
                None => fact(t!(doctor_premium), t!(doctor_premium_absent)),
            }
            fact(t!(doctor_key_id), &config.key_id);
            // What a run would actually request, since a choice made with `/model` overrides the
            // configured default and reporting only the default would explain the wrong thing.
            match bravebot_tui::store::load_model() {
                Some(chosen) => fact(t!(doctor_model), t!(doctor_model_chosen, model = chosen)),
                None => fact(
                    t!(doctor_model),
                    t!(doctor_model_default, model = config.default_model),
                ),
            }
            fact(
                t!(doctor_key_name),
                t!(doctor_key, key = config.signing_key),
            );
            report_subscription();
        }
        Err(err) => {
            eprintln!("{}", t!(cli_configuration_problem, problem = err));
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

/// Where the values line up in what `doctor` reports, and in its confinement section.
const FACT: usize = 10;
const DETAIL: usize = 17;

/// One line of what `doctor` found: a name, then what it is, in two columns.
///
/// The gap is computed rather than typed into each line, since a translated name is not the
/// length the English one was and a column of hand-counted spaces stops being a column. Counted
/// in characters, which is not the width a terminal draws for every script, but is much closer
/// to it than a count of bytes and needs nothing to work it out.
fn aligned(name: impl AsRef<str>, value: impl AsRef<str>, column: usize) -> String {
    let name = name.as_ref();
    let gap = column.saturating_sub(name.chars().count()).max(1);
    format!("  {name}{}{}", " ".repeat(gap), value.as_ref())
}

fn fact(name: impl AsRef<str>, value: impl AsRef<str>) {
    println!("{}", aligned(name, value, FACT));
}

fn detail(name: impl AsRef<str>, value: impl AsRef<str>) {
    println!("{}", aligned(name, value, DETAIL));
}

/// Report which channels have an imported subscription, and how much of it is left.
///
/// Counts only: a credential is a bearer secret, so none of it is printed.
fn report_subscription() {
    for channel in bravebot_skus::Channel::ALL {
        if let Ok(stored) = bravebot_skus::store::load(channel) {
            fact(
                t!(doctor_leo),
                t!(
                    doctor_subscription,
                    channel = channel.as_str(),
                    unspent = stored.remaining(),
                    total = stored.credentials.len()
                ),
            );
        }
    }
}

/// Report the confinement actually achieved here.
///
/// Printed rather than assumed: the guarantee differs by platform and kernel, and a
/// user is entitled to know which one they have before trusting the sandbox.
fn report_confinement(ok: &mut bool) {
    match bravebot_sandbox::for_current_platform() {
        Ok(sandbox) => {
            let caps = sandbox.capabilities();
            println!("{}", t!(doctor_confinement, level = caps.level));
            detail(t!(doctor_mechanisms), caps.mechanisms.join(", "));
            detail(
                t!(doctor_network_denial),
                if caps.network_denial_enforced {
                    t!(doctor_kernel_enforced)
                } else {
                    t!(doctor_not_enforced)
                },
            );
        }
        Err(err) => {
            // Not a warning: without confinement, untrusted work will be refused
            // rather than run, so this is a hard problem for the user to solve.
            eprintln!("{}", t!(doctor_confinement_unavailable));
            eprintln!("  {err}");
            *ok = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bravebot_core::capability::Capability;
    use bravebot_core::event::Sink;
    use bravebot_core::label::Label;
    use bravebot_core::slot::SlotId;

    /// The column the English names were hand-spaced into, so extracting them changed nothing
    /// about what `doctor` prints.
    #[test]
    fn a_name_shorter_than_its_column_is_padded_out_to_it() {
        assert_eq!(
            aligned("endpoint", "https://example", FACT),
            "  endpoint  https://example"
        );
        assert_eq!(aligned("key id", "abc", FACT), "  key id    abc");
        assert_eq!(
            aligned("network denial", "kernel-enforced", DETAIL),
            "  network denial   kernel-enforced"
        );
    }

    /// A translation is not the length the English was, and a name that fills the column has to
    /// stay a name rather than running into what it is naming.
    #[test]
    fn a_name_longer_than_its_column_still_leaves_a_gap() {
        let line = aligned("point de terminaison", "https://example", FACT);
        assert_eq!(line, "  point de terminaison https://example");
    }

    /// An interactive `bravebot -p "task"` must not block waiting for a pipe that is not coming.
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

    fn trail_of(events: Vec<Event>) -> RecordingSink {
        let mut sink = RecordingSink::new();
        for event in events {
            sink.emit(event);
        }
        sink
    }

    /// Reads back what a run would have written, as the two streams it writes to.
    fn written(run: &Finished<'_>) -> (String, String) {
        let mut reply = Vec::new();
        let mut beside = Vec::new();
        report(&mut reply, &mut beside, run);
        (
            String::from_utf8(reply).expect("utf-8"),
            String::from_utf8(beside).expect("utf-8"),
        )
    }

    /// The reply is what gets piped onward, so anything else sharing its stream corrupts the
    /// file at the other end. This is the whole of what makes a one-shot run pipeable.
    #[test]
    fn stdout_carries_the_reply_and_nothing_else() {
        let sink = trail_of(vec![Event::GatePassed {
            gate: "display",
            detail: "assistant reply shown to the user".into(),
        }]);
        let (reply, beside) = written(&Finished {
            reply: "ok",
            notices: &["a skill was loaded".to_string()],
            trail: Some((&sink, "qwen-3-235b")),
            clean: false,
        });

        assert_eq!(reply, "ok\n");
        assert!(beside.contains("audit trail"), "got: {beside}");
        assert!(beside.contains("note: a skill was loaded"), "got: {beside}");
        assert!(beside.contains("model: qwen-3-235b"), "got: {beside}");
        assert!(beside.contains("a policy gate refused"), "got: {beside}");
    }

    /// Without `--trace` the trail is not written at all, rather than written somewhere quieter.
    #[test]
    fn an_untraced_run_writes_no_trail() {
        let (reply, beside) = written(&Finished {
            reply: "ok",
            notices: &[],
            trail: None,
            clean: true,
        });

        assert_eq!(reply, "ok\n");
        assert!(beside.is_empty(), "got: {beside}");
    }

    /// Every arm of the trail is a line somebody reads to see what the turn was allowed to do,
    /// so a refusal has to be as legible as a pass.
    #[test]
    fn the_trail_renders_a_line_for_every_event() {
        let sink = trail_of(vec![
            Event::GatePassed {
                gate: "capability",
                detail: "file_read granted".into(),
            },
            Event::GateBlocked {
                gate: "network",
                detail: "egress".into(),
                reason: "host not allowed".into(),
            },
            Event::Observed {
                capability: Capability::FileRead,
                label: Label::untrusted_private(),
            },
            Event::SlotWritten {
                slot: SlotId::new("file:a.rs"),
                label: Label::untrusted_private(),
            },
            Event::SlotDeferred {
                slot: SlotId::new("file:b.rs"),
                label: Label::untrusted_private(),
                origin: "b.rs".into(),
            },
            Event::Declassified {
                slot: SlotId::new("reply"),
                from: Label::untrusted_public(),
                to: Label::trusted_public(),
                reason: "present",
            },
            Event::ActionField {
                tool: "write".into(),
                field: "path".into(),
                role: Role::Routing,
                label: Label::untrusted_public(),
                allowed: false,
            },
        ]);

        let mut output = Vec::new();
        print_trace(&mut output, &sink);
        let trail = String::from_utf8(output).expect("utf-8");
        let lines: Vec<&str> = trail.lines().collect();

        assert_eq!(lines[0], "audit trail");
        assert_eq!(lines.len(), 8, "a line each, plus the heading: {trail}");
        assert!(lines[1].contains("ok      capability: file_read granted"));
        assert!(lines[2].contains("BLOCK   network: host not allowed"));
        assert!(lines[3].starts_with("  observe "));
        assert!(lines[4].starts_with("  slot    file:a.rs"));
        assert!(lines[5].contains("holds b.rs, unread"));
        assert!(lines[6].starts_with("  release reply "));
        assert!(lines[7].contains("BLOCK") && lines[7].contains("write.path [routing]"));
    }

    /// The trail is written after the reply is already on stdout, so a stderr nobody is reading
    /// must not take the run down with it: the exit code still has a turn to report on.
    #[test]
    fn a_closed_stream_does_not_stop_the_trail() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            }
        }

        let mut sink = RecordingSink::new();
        sink.emit(Event::GatePassed {
            gate: "display",
            detail: "assistant reply shown to the user".into(),
        });
        print_trace(&mut Closed, &sink);
    }
}
