//! Command-line entry point.

mod progress;

use bravebot_agent::turn::{self, Task};
use bravebot_agent::{Mode, Workspace};
use bravebot_config::Config;
use bravebot_core::cancel::Cancel;
use bravebot_core::event::{Event, RecordingSink, Role};
use bravebot_core::trust::TrustStore;
use bravebot_i18n::t;
use bravebot_tui::sessions::Resumable;
use std::io::{IsTerminal, Read, Write};
use std::path::Path;
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
        // The task flags may lead: `bravebot -p "task"` and `bravebot --mode manifest "task"`
        // would otherwise be caught below as unknown options.
        Some("-p" | "--print" | "--mode" | "--file" | "--trace") => run_task(&args),
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
        ("Ctrl-R", t!(cli_key_history_search)),
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
        ("--mode <mode>", t!(cli_option_mode)),
        ("-p, --print", t!(cli_option_print)),
        ("--trace", t!(cli_option_trace)),
        ("-h, --help", t!(cli_option_help)),
        ("-V, --version", t!(cli_option_version)),
    ] {
        println!("  {flags:<OPTION$}{description}");
    }
}

/// What a one-shot invocation asked for, before anything runs.
#[derive(Debug)]
struct Invocation {
    prompt: String,
    files: Vec<String>,
    mode: Mode,
    trace: bool,
    print: bool,
}

/// Parse `<prompt> [--file path]... [--mode name] [--trace] [-p]`.
fn parse_invocation(args: &[String]) -> Result<Invocation, String> {
    let mut prompt = String::new();
    let mut files = Vec::new();
    let mut mode = Mode::default();
    let mut trace = false;
    let mut print = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--mode" => match args.get(index + 1).map(|name| name.parse::<Mode>()) {
                Some(Ok(chosen)) => {
                    mode = chosen;
                    index += 2;
                }
                Some(Err(complaint)) => return Err(complaint),
                None => {
                    return Err(t!(cli_mode_needs_a_name, names = Mode::NAMES.join(", ")));
                }
            },
            "--file" => match args.get(index + 1) {
                Some(path) => {
                    files.push(path.clone());
                    index += 2;
                }
                None => return Err(t!(cli_file_needs_a_path).to_string()),
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
            other => return Err(t!(cli_unexpected_argument, argument = other)),
        }
    }

    Ok(Invocation {
        prompt,
        files,
        mode,
        trace,
        print,
    })
}

fn run_task(args: &[String]) -> ExitCode {
    let invocation = match parse_invocation(args) {
        Ok(invocation) => invocation,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let Invocation {
        prompt,
        files,
        mode,
        trace,
        print,
    } = invocation;

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
    // The rules the settings file carried. A one-shot run refuses every write anyway, so what
    // these add here is the deny list: a rule keeping a file from being read holds for a run
    // nobody is watching exactly as it does for a session. Anything unreadable is named on stderr,
    // beside the rest of what this run has to say about itself.
    let settings = bravebot_config::Settings::load();
    let (permissions, rejected) = bravebot_agent::permissions::from_settings(
        &settings,
        bravebot_agent::home::directory().as_deref(),
    );
    for problem in &rejected {
        eprintln!(
            "{}",
            t!(
                session_permission_rule_ignored,
                problem = problem.to_string()
            )
        );
    }

    let mut task = Task::new(prompt)
        .with_home(bravebot_agent::home::directory())
        .with_model(bravebot_tui::store::load_model())
        .with_permissions(permissions);
    for file in files {
        task = task.with_file(file);
    }
    if let Some(text) = piped {
        task = task.with_piped_input(text);
    }

    // A one-shot run has nobody to ask about a write, so writes are refused rather than
    // silently applied. Manifest is the same: unattended, empty map, no y/n.
    let mut confirmer = bravebot_agent::Unattended;

    // Progress goes to stderr so stdout stays the reply and nothing else, which is what makes
    // the command pipeable. Without it a long turn prints nothing until it is over.
    let mut reporter = progress::Progress::new(std::io::stderr());

    // Before the turn, so the sign-in's own output is not interleaved with progress lines and a
    // browser opening is accounted for. Nothing happens where no sign-in is wanted, which includes
    // every run whose model is served by Brave.
    //
    // Not fatal: the turn goes ahead and fails with the backend's own account of what is wrong,
    // which says more than this could guess.
    let model = task
        .model
        .as_deref()
        .unwrap_or(&config.default_model)
        .to_string();
    // The sign-in's own lines go to stderr as they arrive, beside every other progress line, which
    // keeps stdout the reply and nothing else. A URL and a code are no use after the fact, so they
    // are printed while the command that wrote them is still waiting.
    let signed_in = bravebot_agent::backend::Backend::sign_in_if_needed(&config, &model, |line| {
        eprintln!("{line}");
    });
    if let Err(failure) = signed_in {
        eprintln!("{}", t!(cli_notice, notice = failure.to_string()));
    }

    // Both modes take the same arguments and return the same outcome. The whole of the
    // difference is inside: one asks the model what to do next after every result, the other
    // asked once, before there were any.
    let outcome = match mode {
        Mode::Turn => turn::run_cancellable(
            &config,
            &egress,
            &workspace,
            &task,
            &mut confirmer,
            &mut reporter,
            &mut sink,
            TrustStore::new(),
            &Cancel::new(),
        ),
        Mode::Manifest => bravebot_agent::manifest::run(
            &config,
            &egress,
            &workspace,
            &task,
            &mut confirmer,
            &mut reporter,
            &mut sink,
            TrustStore::new(),
            &Cancel::new(),
        ),
    };

    // A manifest run is written down like any other session. It cannot be resumed, and the
    // picker says so, but "cannot be continued" is a different thing from "leaves no trace":
    // the run somebody needs to read is the one that stopped, and until now it left nothing.
    if mode == Mode::Manifest {
        record_manifest_run(&workspace, &task.prompt, &outcome);
    }

    match outcome {
        Ok(outcome) => {
            // The reply is untrusted model output. Printing it is safe, since the
            // terminal is not a decision, so it is released explicitly for display.
            // A traced manifest run puts the plan on stderr with the trail, never on
            // stdout: stdout stays the reply so a pipe is still a pipe.
            let attempt = if trace {
                outcome
                    .attempt
                    .as_ref()
                    .map(bravebot_agent::manifest::Attempt::describe)
            } else {
                None
            };
            report(
                &mut std::io::stdout().lock(),
                &mut std::io::stderr().lock(),
                &Finished {
                    reply: outcome.reply_for_display(),
                    notices: &outcome.notices,
                    attempt: attempt.as_deref(),
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
        // A run that stopped is the one worth looking at, so what it produced is printed
        // whether or not --trace was asked for. Without it a failed plan is a one-line
        // complaint about a document nobody can see.
        Err(bravebot_agent::TurnError::Manifest { attempt, detail }) => {
            eprintln!("{detail}");
            let report = attempt.describe();
            if !report.is_empty() {
                eprintln!();
                eprint!("{report}");
            }
            if trace {
                eprintln!();
                print_trace(&mut std::io::stderr().lock(), &sink);
            }
            ExitCode::FAILURE
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
    /// What a traced manifest run planned, when there was one. On stderr with the trail,
    /// never on stdout: a pipe of the reply must not pick up the plan.
    attempt: Option<&'a str>,
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
    if let Some(attempt) = run.attempt {
        let _ = writeln!(beside);
        let _ = write!(beside, "{attempt}");
    }
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

/// Write a manifest run into the session store, finished or not.
///
/// Best-effort, like everything else under `~/.bravebot`: a run that cannot be written down still
/// ran, and failing the command because the record did not save would be the wrong trade.
fn record_manifest_run(
    workspace: &bravebot_agent::Workspace,
    prompt: &str,
    outcome: &Result<bravebot_agent::Outcome, bravebot_agent::TurnError>,
) {
    use bravebot_core::programs::TrustedPrograms;
    use bravebot_tui::sessions::{Handle, Standing, StoredManifest};

    let (stored, trust) = match outcome {
        Ok(finished) => (
            finished
                .attempt
                .as_ref()
                .map(|attempt| StoredManifest::of(attempt, None)),
            finished.trust.clone(),
        ),
        Err(bravebot_agent::TurnError::Manifest { attempt, detail }) => (
            Some(StoredManifest::of(attempt, Some(detail.clone()))),
            TrustStore::new(),
        ),
        // Cancelled, or a failure with nothing to show. Nothing worth a record.
        Err(_) => (None, TrustStore::new()),
    };

    let Some(stored) = stored else {
        return;
    };

    let conversation = bravebot_agent::Conversation::new();
    let snapshot = conversation.snapshot();
    let todos = std::collections::BTreeMap::new();
    let programs = TrustedPrograms::new();
    let tokens = outcome.as_ref().map(|o| o.tokens).unwrap_or(0);
    // One turn, so the breakdown and the total say the same thing. Written anyway, because a
    // reader comparing runs should not have to special-case where the figure came from.
    let spend = std::collections::BTreeMap::from([(1, tokens)]);
    // Where that one turn's time went, on the same footing. A manifest run is the case where this
    // matters most: it is the mode nobody is watching, so a run that spent its afternoon blocked on
    // an approval nobody was there to give leaves this as the only trace of it.
    let timing = std::collections::BTreeMap::from([(
        1,
        outcome.as_ref().map(|o| o.timing).unwrap_or_default(),
    )]);
    let mut handle = Handle::begin(workspace.root());
    handle.save(
        prompt,
        Standing {
            // Empty, and it has to be: a manifest run has no conversation, which is the same
            // fact that makes it unresumable. Filling this with something conversation-shaped
            // would make the picker offer to continue a run that cannot be continued.
            conversation: &snapshot,
            turns: 1,
            tokens,
            spend: &spend,
            timing: &timing,
            model: outcome.as_ref().ok().map(|o| o.model.as_str()),
            todos: &todos,
            trust: &trust,
            programs: &programs,
            directories: &[],
            manifest: Some(&stored),
        },
    );
}
fn resume_named(id: &str) -> ExitCode {
    let Ok(directory) = std::env::current_dir() else {
        eprintln!("{}", t!(cli_directory_unknown));
        return ExitCode::FAILURE;
    };
    match bravebot_tui::sessions::load(&directory, id) {
        Some(record) if record.manifest.is_some() => {
            eprintln!("{}", bravebot_tui::resume::manifest_note());
            if let Some(stored) = &record.manifest {
                let report = stored.describe();
                if !report.is_empty() {
                    eprintln!();
                    eprint!("{report}");
                }
            }
            ExitCode::FAILURE
        }
        Some(record) => interactive(bravebot_tui::app::Start::Resuming(Box::new(record))),
        None => {
            eprintln!("{}", t!(cli_no_such_session, id = id));
            ExitCode::FAILURE
        }
    }
}

fn interactive(start: bravebot_tui::app::Start) -> ExitCode {
    let mut config = match Config::from_env() {
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
        Ok(sandbox) => named(sandbox.capabilities().level),
        Err(_) => named(bravebot_sandbox::policy::ConfinementLevel::None),
    };

    match bravebot_tui::app::run(&mut config, &workspace, confinement, start) {
        // Printed after the terminal is handed back, so it survives on the screen the person is
        // left looking at rather than going onto the alternate screen with everything else. A
        // session is worth resuming far more often than anybody thinks to write its name down
        // beforehand, and the picker is no help to someone who has already closed the window.
        Ok(Some(left)) => {
            println!("{}", resume_hint(&left, workspace.root()));
            ExitCode::SUCCESS
        }
        Ok(None) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{}", t!(cli_interface_problem, problem = err));
            ExitCode::FAILURE
        }
    }
}

/// What to say on the way out about picking this session up again.
///
/// `--resume` looks an id up under the working directory it is run in, so the id on its own is a
/// whole answer only while the session ended where it started. `/cd` moves the record, and the
/// shell this is printed into did not move with it: a bare id there names a session the shell
/// cannot find, or worse, finds an earlier state of the same one and resumes that. Naming the
/// directory is what keeps the line true.
///
/// Said only when the two differ. Telling somebody the directory they are already standing in
/// reads as though something had happened to it.
fn resume_hint(left: &Resumable, started_in: &Path) -> String {
    let heading = match left.directory == started_in {
        true => t!(cli_resume_heading).to_string(),
        false => t!(
            cli_resume_moved,
            directory = left.directory.display().to_string()
        ),
    };
    format!("{heading}\nbravebot --resume {}", left.id)
}

/// What to call the confinement that was achieved.
///
/// Named here rather than by `bravebot_sandbox`, whose Display is a diagnostic and whose whole
/// point is to hold no words meant for a person: it depends on nothing, and a catalog is a
/// dependency. So it reports which of the three it got and this says it in the reader's language.
fn named(level: bravebot_sandbox::policy::ConfinementLevel) -> String {
    use bravebot_sandbox::policy::ConfinementLevel;
    match level {
        ConfinementLevel::Kernel => t!(confinement_kernel),
        ConfinementLevel::Partial => t!(confinement_partial),
        ConfinementLevel::None => t!(confinement_none),
    }
    .to_string()
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

    // There is one stored batch, so forgetting takes no channel: naming one would suggest
    // `--forget nightly` leaves a stable import in place, and it does not.
    if forget {
        return match bravebot_skus::store::clear() {
            Ok(()) => {
                println!("{}", t!(leo_forgotten));
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

    if let Err(err) = bravebot_skus::store::save(&credentials) {
        eprintln!("{err}");
        return ExitCode::FAILURE;
    }

    // Named so the user knows where the secret went: it is an ordinary file now, and one they may
    // want to inspect, exclude from a backup, or delete by hand.
    let where_stored = match bravebot_skus::store::path() {
        Ok(path) => path.display().to_string(),
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "{}",
        t!(
            leo_stored,
            count = count,
            path = where_stored,
            expiry = last
        )
    );
    println!("{}", t!(leo_browser_untouched));
    ExitCode::SUCCESS
}

/// Report whether configuration is usable, without revealing the signing key.
fn doctor() -> ExitCode {
    let mut ok = true;

    // Read before the configuration so the file can be reported even when it is what made the
    // configuration wrong.
    let settings = bravebot_config::Settings::load();

    match Config::from_env_and_settings(&settings) {
        Ok(config) => {
            println!("{}", t!(doctor_configuration_ok));

            // Names only. On some machines a value here is a credential, and a diagnostic that
            // prints one is a diagnostic people paste into issues.
            //
            // The variables the file names, not everything it configured: a gateway is a block rather
            // than a variable and gets a section of its own below. A file that only configures one
            // therefore names nothing here, which is absence rather than an empty list.
            let named: Vec<&str> = settings.names().collect();
            match (named.is_empty(), settings.is_empty()) {
                (false, _) => fact(
                    t!(doctor_settings),
                    t!(doctor_settings_names, names = named.join(", ")),
                ),
                (true, false) => fact(t!(doctor_settings), t!(doctor_settings_no_variables)),
                (true, true) => fact(t!(doctor_settings), t!(doctor_settings_absent)),
            }

            // Counted rather than listed: a rule is the user's own text and printing it back says
            // nothing they cannot read in the file. What is worth saying is which of them this
            // build could not act on, because those are the ones that look like protection and
            // are not.
            let (permissions, rejected) = bravebot_agent::permissions::from_settings(
                &settings,
                bravebot_agent::home::directory().as_deref(),
            );
            fact(
                t!(doctor_permissions),
                match permissions.is_empty() {
                    true => t!(doctor_permissions_absent).to_string(),
                    false => t!(doctor_permissions_count, count = permissions.len() as i64),
                },
            );
            for problem in &rejected {
                ok = false;
                fact(t!(doctor_permissions_unreadable), problem.to_string());
            }

            // Both, where both are reachable, because both are offered to a person choosing and a
            // report naming one of them explains only the half of the picker they happened to use.
            if config.serves_aichat() {
                report_aichat(&config);
            }
            if let Some(bedrock) = config.bedrock.as_ref() {
                report_bedrock(bedrock);
            }
            for provider in &config.providers {
                report_gateway(provider);
            }

            // What a run would actually request, since a choice made with `/model` overrides the
            // configured default and reporting only the default would explain the wrong thing.
            match bravebot_tui::store::load_model() {
                Some(chosen) => fact(t!(doctor_model), t!(doctor_model_chosen, model = chosen)),
                None => fact(
                    t!(doctor_model),
                    t!(doctor_model_default, model = &config.default_model),
                ),
            }

            // Only where a subscription means something. A Leo credential is what the premium half of
            // the Brave roster needs, and it means nothing to Bedrock.
            if config.serves_aichat() {
                report_subscription();
            }
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

/// What `doctor` says about the Bedrock half of the roster.
///
/// No key and no endpoint: there is no key, and the host is derived from the region rather than
/// configured. The credentials are the AWS CLI's to hold, which is why the profile is the useful
/// thing to report and there is nothing here to redact.
fn report_bedrock(bedrock: &bravebot_config::bedrock::Bedrock) {
    fact(t!(doctor_backend), t!(doctor_backend_bedrock));
    fact(t!(doctor_region), &bedrock.region);
    match bedrock.profile.as_deref() {
        Some(profile) => fact(t!(doctor_profile), profile),
        None => fact(t!(doctor_profile), t!(doctor_profile_absent)),
    }

    // The tiers a person may choose, by name. An ARN is unreadable and looks identical between
    // tiers, so the names are what tells someone whether the block did what they meant.
    match bedrock.models().is_empty() {
        false => fact(
            t!(doctor_tiers),
            bedrock
                .models()
                .iter()
                .map(|(tier, _)| tier.display_name())
                .collect::<Vec<_>>()
                .join(", "),
        ),
        true => fact(t!(doctor_tiers), t!(doctor_tiers_absent)),
    }
}

/// What `doctor` says about one configured gateway.
///
/// Whether a credential can be found rather than what it is, because on this path the value is a
/// bearer token and a diagnostic that printed one is a diagnostic people paste into issues. Which
/// models, because a gateway's own roster is far larger than the block names and the listed slugs are
/// what tells someone whether the block did what they meant.
fn report_gateway(provider: &bravebot_config::provider::Provider) {
    fact(
        t!(doctor_backend),
        t!(doctor_backend_gateway, gateway = provider.display_name()),
    );
    fact(t!(doctor_endpoint), provider.chat_completions_url());
    fact(
        t!(doctor_key_name),
        gateway_credential(provider, |name| std::env::var(name).ok()),
    );
    match provider.models.is_empty() {
        false => fact(
            t!(doctor_tiers),
            provider
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ),
        true => fact(t!(doctor_tiers), t!(doctor_gateway_models_absent)),
    }
}

/// What `doctor` says about a gateway's credential: that one was found, never what it is.
///
/// Separate from the printing so the withholding is testable. The value here is a long-lived bearer
/// token, so a diagnostic that echoed one would put a live credential in every issue somebody pastes
/// this into.
fn gateway_credential(
    provider: &bravebot_config::provider::Provider,
    lookup: impl Fn(&str) -> Option<String>,
) -> &'static str {
    match provider.token(lookup).is_some() {
        true => t!(doctor_gateway_token),
        false => t!(doctor_gateway_token_absent),
    }
}

/// What `doctor` says about a build pointed at the Brave backend.
fn report_aichat(config: &Config) {
    fact(t!(doctor_backend), t!(doctor_backend_aichat));
    fact(t!(doctor_endpoint), config.chat_completions_url());
    match config.premium_chat_completions_url() {
        Some(url) => fact(t!(doctor_premium), &url),
        None => fact(t!(doctor_premium), t!(doctor_premium_absent)),
    }
    fact(t!(doctor_key_id), &config.key_id);
    // Redacting `Display`, so what is reported is the placeholder rather than the key.
    fact(
        t!(doctor_key_name),
        t!(doctor_key, key = &config.signing_key),
    );
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

/// Report the imported subscription, and how much of it is left.
///
/// Counts only: a credential is a bearer secret, so none of it is printed. The environment rather
/// than the channel it came from, because that is what decides whether the batch can be spent
/// against the endpoint this build talks to, and it is what the file records.
fn report_subscription() {
    if let Ok(stored) = bravebot_skus::store::load() {
        fact(
            t!(doctor_leo),
            t!(
                doctor_subscription,
                environment = stored.environment.as_str(),
                unspent = stored.remaining(),
                total = stored.credentials.len()
            ),
        );
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
            println!("{}", t!(doctor_confinement, level = named(caps.level)));
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
    use std::path::PathBuf;

    /// A session that stayed where it started needs no directory: the shell reading this line is
    /// already standing in the one the id will be looked up under.
    #[test]
    fn a_session_that_stayed_put_is_named_by_its_id_alone() {
        let left = Resumable {
            id: "abc".to_string(),
            directory: PathBuf::from("/work"),
        };
        let hint = resume_hint(&left, Path::new("/work"));

        assert!(hint.contains("bravebot --resume abc"), "{hint}");
        assert!(
            !hint.contains("/work"),
            "the directory somebody is already in was named at them: {hint}"
        );
    }

    /// A session that moved with `/cd` left its record where it moved to, and `--resume` looks an
    /// id up under the directory it is run in. Printing the id alone would name a session this
    /// shell cannot find, or find an earlier state of the same one and resume that.
    #[test]
    fn a_session_that_moved_says_where_to_resume_it() {
        let left = Resumable {
            id: "abc".to_string(),
            directory: PathBuf::from("/other"),
        };
        let hint = resume_hint(&left, Path::new("/work"));

        assert!(hint.contains("bravebot --resume abc"), "{hint}");
        assert!(
            hint.contains("/other"),
            "the line does not say where the session went: {hint}"
        );
    }

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

    /// The gateway a settings file configured, for the `doctor` tests below.
    fn configured_gateway(text: &str) -> bravebot_config::provider::Provider {
        bravebot_config::Settings::parse(text)
            .providers()
            .first()
            .expect("one provider")
            .clone()
    }

    /// A gateway's credential is a long-lived bearer token, so a diagnostic that echoed one would put
    /// a live credential in every issue somebody pastes this into.
    #[test]
    fn a_gateway_credential_is_reported_as_found_and_never_printed() {
        let provider = configured_gateway(
            r#"{"provider": {"gw": {
                "env": ["A_TOKEN_VARIABLE"],
                "options": {"baseURL": "https://example.invalid/v1", "apiKey": "in-the-file"}
            }}}"#,
        );

        let from_variable = gateway_credential(&provider, |name| match name {
            "A_TOKEN_VARIABLE" => Some("secret-from-the-environment".to_string()),
            _ => None,
        });
        assert!(!from_variable.contains("secret-from-the-environment"));

        // A token written into the file is found too, and is withheld on the same footing.
        let from_file = gateway_credential(&provider, |_| None);
        assert!(!from_file.contains("in-the-file"));
        assert_eq!(from_variable, from_file);
    }

    /// A gateway nothing holds a credential for says so. Reporting one as found would answer the
    /// question `doctor` exists to answer wrongly, and the request then fails somewhere further away.
    #[test]
    fn a_gateway_with_no_credential_is_reported_as_having_none() {
        let provider = configured_gateway(
            r#"{"provider": {"gw": {
                "env": ["ABSENT_ONE"],
                "options": {"baseURL": "https://example.invalid/v1"}
            }}}"#,
        );

        assert_ne!(
            gateway_credential(&provider, |_| None),
            gateway_credential(&provider, |_| Some("anything".to_string()))
        );
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
            attempt: None,
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
            attempt: None,
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

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_string()).collect()
    }

    /// Turn is what an unqualified run has always been, so an omitted `--mode` has to stay that.
    #[test]
    fn the_default_mode_is_the_turn_loop() {
        let invocation = parse_invocation(&args(&["do a thing"])).expect("parses");
        assert_eq!(invocation.mode, Mode::Turn);
        assert_eq!(invocation.prompt, "do a thing");
    }

    #[test]
    fn a_leading_mode_flag_is_a_task_not_an_unknown_option() {
        let invocation =
            parse_invocation(&args(&["--mode", "manifest", "do a thing"])).expect("parses");
        assert_eq!(invocation.mode, Mode::Manifest);
        assert_eq!(invocation.prompt, "do a thing");
    }

    #[test]
    fn an_unknown_mode_is_refused_rather_than_guessed() {
        let err =
            parse_invocation(&args(&["--mode", "safe", "do a thing"])).expect_err("must refuse");
        assert!(err.contains("safe"), "{err}");
        assert!(err.contains("turn") && err.contains("manifest"), "{err}");
    }

    /// A failed plan is the document nobody would otherwise see. It belongs beside the reply,
    /// never in it: a pipe of stdout would otherwise pick up the model's own words mixed into
    /// whatever the run produced.
    #[test]
    fn a_failed_plan_is_printed_beside_the_reply() {
        let (reply, beside) = written(&Finished {
            reply: "ok",
            notices: &[],
            attempt: Some("manifest proposed, which was not usable\n  not JSON\n"),
            trail: None,
            clean: true,
        });
        assert_eq!(reply, "ok\n");
        assert!(beside.contains("not usable"), "got: {beside}");
        assert!(!reply.contains("not usable"));
    }
}
