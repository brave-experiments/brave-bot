//! Command-line entry point.

#![forbid(unsafe_code)]

mod progress;

use bravebot_agent::turn::{self, Task};
use bravebot_agent::{Mode, Workspace};
use bravebot_config::Config;
use bravebot_core::cancel::Cancel;
use bravebot_core::event::{Event, RecordingSink, Role};
use bravebot_core::trust::TrustStore;
use bravebot_i18n::t;
use bravebot_sandbox::SandboxError;
use bravebot_sandbox::policy::Capabilities;
use bravebot_tui::sessions::Resumable;
use std::io::{IsTerminal, Read, Write};
use std::path::Path;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    // Before anything is printed, and exactly once: every later lookup reads what this settled
    // on. Nothing else in the tree consults the environment about a language.
    bravebot_i18n::init_from_environment();

    // The process's own argv, parsed below into the documented flags. There is no other
    // way for a command line tool to learn what it was asked to do.
    // nosemgrep: rust.lang.security.args.args
    let mut args: Vec<String> = std::env::args().skip(1).collect();

    // Engaged here rather than deeper in because it must be true before the first thing that could
    // write is reached, and this is the last moment that is certain to be before all of them.
    if take_incognito(&mut args) {
        bravebot_core::incognito::engage();
    }

    // Taken out before anything dispatches on the first argument, because this one belongs to every
    // way of starting: a session, a resumed session, and a one-shot run all put the same four
    // questions to the same trait. Reading it per subcommand would be four chances to read it in
    // three of them, and the flag would then be silently ignored wherever it was forgotten.
    let skip_permissions = take_skip_permissions(&mut args);

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
        None => interactive(bravebot_tui::app::Start::Fresh, skip_permissions),
        // Picking up where a session left off, chosen from a list or named outright.
        Some("--resume" | "-r") => match args.get(1) {
            Some(id) => resume_named(id, skip_permissions),
            None => interactive(bravebot_tui::app::Start::Choose, skip_permissions),
        },
        // The same, for the session somebody was in a moment ago, which is the one they mean
        // often enough that asking them to find its id is asking for nothing.
        Some("--continue" | "-c") => continue_here(skip_permissions),
        // Fork a session, creating a new session record that starts with the same transcript.
        Some("--fork" | "-f") => match args.get(1) {
            Some(id) => fork_named(id, skip_permissions),
            None => {
                eprintln!("{}", t!(cli_fork_needs_a_name));
                ExitCode::FAILURE
            }
        },
        // The task flags may lead: `bravebot -p "task"` and `bravebot --mode manifest "task"`
        // would otherwise be caught below as unknown options.
        Some("-p" | "--print" | "--mode" | "--model" | "--file" | "--add-dir" | "--trace") => {
            run_task(&args, skip_permissions)
        }
        Some("doctor") => doctor(),
        Some("import-leo-creds") => import_leo_creds(&args[1..]),
        Some(flag) if flag.starts_with('-') => {
            eprintln!("{}", t!(cli_unknown_option, flag = flag));
            print_help();
            ExitCode::FAILURE
        }
        // Anything else is treated as the task prompt.
        Some(_) => run_task(&args, skip_permissions),
    }
}

/// Take `--dangerously-skip-permissions` out of the arguments, reporting whether it was there.
///
/// Removed before dispatch for the reason `--incognito` is, and it composes with it: the flag belongs
/// to every way of starting rather than to a task, since a session, a resumed session and a one-shot
/// run all put the same questions to the same trait. Read per subcommand it would be three chances to
/// read it in two of them.
///
/// See [`bravebot_agent::PermissionMode`] for what giving it up costs. Deny rules from the settings
/// file are not part of it: they refuse before there is anything to prompt about, so the flag means
/// "stop asking me" rather than "forget what I wrote down".
///
/// Repeats are one flag rather than an error, as with `--incognito`.
fn take_skip_permissions(args: &mut Vec<String>) -> bool {
    let asked = args.len();
    args.retain(|arg| arg != "--dangerously-skip-permissions");
    args.len() != asked
}

fn print_help() {
    /// Wide enough for the longest invocation below, so a translated description starts in the
    /// same column as every other one rather than wherever hand-counted spaces left it.
    const FORM: usize = 39;
    /// The same, for the key column and the option column.
    const KEY: usize = 22;
    /// Wide enough for `--dangerously-skip-permissions` and a gap. A narrower column would leave
    /// that one flag touching its own description, since the padding below cannot go negative.
    const OPTION: usize = 32;

    println!("{}", t!(cli_tagline, version = VERSION));
    println!();
    println!("{}", t!(cli_usage_heading));
    for (form, description) in [
        ("bravebot", t!(cli_usage_interactive)),
        ("bravebot \"<task>\" [--file <path>]...", t!(cli_usage_task)),
        ("cat file | bravebot -p \"<task>\"", t!(cli_usage_piped)),
        ("bravebot --resume [id]", t!(cli_usage_resume)),
        ("bravebot --continue", t!(cli_usage_continue)),
        ("bravebot --fork <id>", t!(cli_usage_fork)),
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
        ("--add-dir <path>", t!(cli_option_add_dir)),
        ("--mode <mode>", t!(cli_option_mode)),
        ("--model <name>", t!(cli_option_model)),
        ("-p, --print", t!(cli_option_print)),
        ("--trace", t!(cli_option_trace)),
        ("--incognito", t!(cli_option_incognito)),
        (
            "--dangerously-skip-permissions",
            t!(cli_option_dangerously_skip_permissions),
        ),
        ("-h, --help", t!(cli_option_help)),
        ("-V, --version", t!(cli_option_version)),
    ] {
        println!("  {flags:<OPTION$}{description}");
    }
}

/// Take `--incognito` out of the arguments, reporting whether it was there.
///
/// Removed before anything dispatches on what leads the list, so the mode composes with every
/// other way of starting rather than being a fourth flag that may lead: `--incognito -p "task"`,
/// `--incognito --resume` and `--incognito` on its own all mean what they look like, and the
/// dispatch below goes on matching against a list the mode is no longer in.
///
/// Repeats are one flag rather than an error. `--incognito --incognito` asks for a mode that is
/// already on, and refusing it would be a rule about typing rather than about privacy.
fn take_incognito(args: &mut Vec<String>) -> bool {
    let asked = args.len();
    args.retain(|arg| arg != "--incognito");
    args.len() != asked
}

/// What a one-shot invocation asked for, before anything runs.
#[derive(Debug)]
struct Invocation {
    prompt: String,
    files: Vec<String>,
    mode: Mode,
    /// The model the command line named. `None` leaves the configured one in force rather than
    /// standing for a model of its own.
    model: Option<String>,
    /// Directories outside the working one that this run may reach into.
    directories: Vec<String>,
    trace: bool,
    print: bool,
}

/// Parse `<prompt> [--file path]... [--add-dir path]... [--mode name] [--model name]
/// [--trace] [-p]`.
fn parse_invocation(args: &[String]) -> Result<Invocation, String> {
    let mut prompt = String::new();
    let mut files = Vec::new();
    let mut mode = Mode::default();
    let mut model = None;
    let mut directories = Vec::new();
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
            // A blank name is refused rather than read as no choice. A stored one is allowed to
            // be blank, where it means the file holds nothing and the configured model answers,
            // but a script that computed an empty variable asked for a model and would otherwise
            // be given whatever was configured without being told.
            "--model" => match args.get(index + 1).map(|name| name.trim()) {
                Some(name) if !name.is_empty() => {
                    model = Some(name.to_string());
                    index += 2;
                }
                _ => return Err(t!(cli_model_needs_a_name).to_string()),
            },
            "--file" => match args.get(index + 1) {
                Some(path) => {
                    files.push(path.clone());
                    index += 2;
                }
                None => return Err(t!(cli_file_needs_a_path).to_string()),
            },
            // Repeatable, since reaching one sibling checkout is no more natural than reaching
            // two, and a flag that could only be given once would be a rule about typing.
            "--add-dir" => match args.get(index + 1).map(|path| path.trim()) {
                Some(path) if !path.is_empty() => {
                    directories.push(path.to_string());
                    index += 2;
                }
                _ => return Err(t!(cli_add_dir_needs_a_path).to_string()),
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
        model,
        directories,
        trace,
        print,
    })
}

fn run_task(args: &[String], skip_permissions: bool) -> ExitCode {
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
        model,
        directories,
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

    let mut workspace = match current_workspace() {
        Ok(w) => w,
        Err(err) => {
            eprintln!("{}", t!(cli_workspace_problem, problem = err));
            return ExitCode::FAILURE;
        }
    };

    // Fatal rather than said and carried on with. A session leaves the person to retype it; a
    // script that asked to reach a directory and did not gets a turn that fails somewhere further
    // in, over a file it was told it could open.
    if let Err(problem) = open_directories(&mut workspace, &directories) {
        eprintln!("{problem}");
        return ExitCode::FAILURE;
    }

    let egress = bravebot_net::Egress::new();
    let mut sink = RecordingSink::new();

    // The rules the settings file carried. Anything unreadable is named on stderr, beside the rest
    // of what this run has to say about itself.
    let settings = bravebot_config::Settings::load();
    let (permissions, rejected) = rules_for_a_one_shot_run(
        &settings,
        bravebot_agent::home::directory().as_deref(),
        skip_permissions,
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

    // Bypassing where the flag was given, and asking otherwise. There is no mode key here: a
    // one-shot run has no session to hold a mode and nobody to press anything, so the command line
    // is the whole of what can say.
    let permission_mode = match skip_permissions {
        true => bravebot_agent::PermissionMode::Bypass,
        false => bravebot_agent::PermissionMode::Ask,
    };
    // Resolved against the configuration rather than at parse, since a tier word names a model
    // only the configuration knows: the AWS account's ARN for that tier where it named one, and
    // Brave's name for it otherwise. The settings key this flag outranks accepts those words, so a
    // flag that did not would refuse a spelling the file it overrides takes.
    let named = model.map(|name| config.model_named(&name));
    // Held onto because it is what separates a substitution worth reporting from one worth failing
    // the run over: below the flag the model is whatever was recorded or configured, and a script
    // that never named one did not ask for what it did not get.
    let named_on_the_command_line = named.is_some();
    let mut task = Task::new(prompt)
        .with_home(bravebot_agent::home::directory())
        .with_model(model_asked_for(named, bravebot_tui::store::load_model()))
        .with_effort(bravebot_tui::store::load_effort())
        .with_permissions(permissions)
        .with_permission_mode(permission_mode);
    for file in files {
        task = task.with_file(file);
    }
    if let Some(text) = piped {
        task = task.with_piped_input(text);
    }

    // A one-shot run has nobody to ask about a write, so writes are refused rather than
    // silently applied. Manifest is the same: unattended, empty map, no y/n.
    //
    // Unless the flag was given, which is the one way a run nobody is watching may write: the
    // person accepted that when they typed it, and this is the only path where the refusal above
    // is what stands between the flag and an effect.
    let mut unattended = bravebot_agent::Unattended;
    let mut confirmer = bravebot_agent::Confining::new(&mut unattended, permission_mode);
    // On stderr, beside the progress lines, so a pipe of the reply is unaffected. Said even here,
    // where nobody may be reading: a run that wrote to the tree without asking should leave a record
    // of having been told not to ask.
    if skip_permissions {
        eprintln!(
            "{}",
            t!(cli_notice, notice = t!(session_permissions_skipped))
        );
    }

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
            // What was asked for against what answered. A model this run cannot be served is
            // substituted rather than refused: one that needs a subscription comes back as
            // whatever the free tier serves, with a 200 and an ordinary reply, so the name the
            // server reports is the only trace of it. Which name the service was actually asked
            // for, and whether it reports that name at all, are the backend's questions and are
            // put to it here, where the configuration is in hand.
            //
            // Against the model in force rather than the flag alone. A model pinned in the
            // settings file is the one a repository commits beside its scripts, so a substitution
            // of that is the case the reporting is for, and a session is told about it whatever
            // named the model.
            let asked = bravebot_agent::backend::Backend::name_as_asked(&config, &model);
            let comparable = bravebot_agent::backend::Backend::reports_the_model_it_was_asked_for(
                &config, &model,
            );
            let not_served = model_not_served(&asked, comparable, &outcome.model);

            let finished = Finished {
                reply: outcome.reply_for_display(),
                notices: &outcome.notices,
                attempt: attempt.as_deref(),
                trail: trace.then_some((&sink, outcome.model.as_str())),
                clean: outcome.clean,
                not_served: not_served.as_deref(),
                named_on_the_command_line,
            };
            report(
                &mut std::io::stdout().lock(),
                &mut std::io::stderr().lock(),
                &finished,
            );
            match finished.succeeded() {
                true => ExitCode::SUCCESS,
                false => ExitCode::FAILURE,
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

/// The rules a one-shot run works under.
///
/// Nobody is watching one, so the allow list is left out: an allow rule answers a prompt in
/// advance, and where there is nobody to ask it would instead be a line in a settings file letting
/// the run write, execute or fetch unwatched, beside the flag that is meant to be the only way
/// that happens. The deny and ask lists carry over, and both still decide something here.
///
/// With the flag, the person answered every one of those prompts themselves when they typed it, so
/// their file is read whole.
fn rules_for_a_one_shot_run(
    settings: &bravebot_config::Settings,
    home: Option<&Path>,
    skip_permissions: bool,
) -> (
    bravebot_core::permissions::Permissions,
    Vec<bravebot_core::permissions::Rejected>,
) {
    match skip_permissions {
        true => bravebot_agent::permissions::from_settings(settings, home),
        false => bravebot_agent::permissions::for_an_unattended_run(settings, home),
    }
}

/// Open every directory the command line named, or say which one could not be opened.
///
/// Reachability and nothing else. `/add-dir` grants a second thing, recording that the person
/// vouched for the directory, and this deliberately does not: a run nobody is watching holds an
/// empty trust map, the directory it was started in included, so a rule trusting a sibling
/// checkout would leave the tree the run was pointed at more trusted than the one it works in.
/// Reads there are on the same footing as reads of the project's own files.
///
/// `~` is left to the shell, which expands it before this ever sees the path. A path that is not
/// absolute, does not exist, is not a directory, or lies inside the working one is refused by the
/// workspace, and the refusal names which.
fn open_directories(workspace: &mut Workspace, directories: &[String]) -> Result<(), String> {
    for directory in directories {
        workspace.add_directory(directory).map_err(|problem| {
            t!(
                session_directory_not_added,
                directory = directory,
                problem = problem.to_string()
            )
        })?;
    }
    Ok(())
}

/// The model a run asks for: the one the command line named, else the one a session would read.
///
/// A run started from a script resolves a model the way a session opening in the same directory
/// does, so a script reaches the model somebody already chose without an interactive step, and
/// neither surface has a model the other cannot ask for. Below both is the configured model,
/// which is what an absent record leaves in force.
///
/// The command line outranks the record because it names a model for one run and nothing else,
/// which is the only way a script can pin one against a choice made elsewhere.
fn model_asked_for(named: Option<String>, stored: Option<String>) -> Option<String> {
    named.or(stored)
}

/// The complaint a run has when one model was asked for and another answered, if it has one.
///
/// The endpoint substitutes rather than refusing, so the reported name is the only trace there is.
/// About the model in force whatever named it: the flag, a remembered choice and the settings key
/// all pin a model somebody expects to be answered by.
///
/// Nothing where the name asks for whichever model the server picks rather than for a particular
/// one, because resolving to a model is what that name is for. Nothing where the backend does not
/// report the name it was asked for either, where a reply that says something else is the
/// indirection working rather than a different model answering.
fn model_not_served(asked: &str, comparable: bool, served: &str) -> Option<String> {
    if !comparable || asked == bravebot_config::DEFAULT_MODEL || asked == served {
        return None;
    }
    Some(t!(
        session_model_substituted,
        asked = asked,
        served = served
    ))
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
    /// What to say where one model was asked for and another one answered.
    not_served: Option<&'a str>,
    /// Whether the command line named the model, which is what makes a substitution a failed run
    /// rather than a thing to report.
    named_on_the_command_line: bool,
}

impl Finished<'_> {
    /// Whether the run did what it was asked, which is what the process exits on.
    ///
    /// A turn that was refused something did not, and neither did one answered by a model other
    /// than the one the command line named. Both are invisible to a script reading stdout, which
    /// is what makes the status the only thing that can carry them.
    ///
    /// A substitution of a model the command line did not name is reported and not failed. Below
    /// the flag the model is whatever was recorded or configured, so failing there would have a
    /// script that names no model start exiting non-zero over a choice made in a terminal, and the
    /// flag is what a script that cannot tolerate a substitution has.
    fn succeeded(&self) -> bool {
        self.clean && !(self.named_on_the_command_line && self.not_served.is_some())
    }
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
    if let Some(complaint) = run.not_served {
        let _ = writeln!(beside, "{complaint}");
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
            // None, and there can be none: an aside is a question a person types beside a
            // conversation, and a manifest run has neither.
            asides: &[],
            trust: &trust,
            programs: &programs,
            directories: &[],
            manifest: Some(&stored),
        },
    );
}
fn resume_named(id: &str, skip_permissions: bool) -> ExitCode {
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
        Some(record) => interactive(
            bravebot_tui::app::Start::Resuming(Box::new(record)),
            skip_permissions,
        ),
        None => {
            eprintln!("{}", t!(cli_no_such_session, id = id));
            ExitCode::FAILURE
        }
    }
}

fn fork_named(id: &str, skip_permissions: bool) -> ExitCode {
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
        Some(_) => match bravebot_tui::sessions::fork(&directory, id) {
            Some(record) => interactive(
                bravebot_tui::app::Start::Resuming(Box::new(record)),
                skip_permissions,
            ),
            None => {
                eprintln!("{}", t!(cli_no_such_session, id = id));
                ExitCode::FAILURE
            }
        },
        None => {
            eprintln!("{}", t!(cli_no_such_session, id = id));
            ExitCode::FAILURE
        }
    }
}

/// Pick up the most recent session under this directory, without anybody naming one.
///
/// Where there is none, this says so and fails. Starting a fresh session instead would answer a
/// different question than the one asked, and it would answer it by throwing away the request:
/// somebody who meant to carry on and got an empty transcript has lost the thing they asked for.
fn continue_here(skip_permissions: bool) -> ExitCode {
    let Ok(directory) = std::env::current_dir() else {
        eprintln!("{}", t!(cli_directory_unknown));
        return ExitCode::FAILURE;
    };
    match bravebot_tui::sessions::most_recent(&directory) {
        // By the id, so this arrives at the interface the way a named resume does, down to a
        // record that went away between the list and the read.
        Some(session) => resume_named(&session.id, skip_permissions),
        None => {
            eprintln!("{}", t!(cli_nothing_to_continue));
            ExitCode::FAILURE
        }
    }
}

fn interactive(start: bravebot_tui::app::Start, skip_permissions: bool) -> ExitCode {
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

    match bravebot_tui::app::run(
        &mut config,
        &workspace,
        confinement,
        start,
        skip_permissions,
    ) {
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

    // Refused rather than silently skipped, and refused before the device is registered so a
    // batch is not minted that nothing will ever be able to spend. An import is a write by
    // definition: a credential that did not outlive the session would not be an import. It is the
    // one command an incognito session cannot carry out rather than merely decline to record.
    // Forgetting above is allowed: removing a stored secret leaves less behind, not more.
    if bravebot_core::incognito::engaged() {
        eprintln!("{}", t!(leo_not_while_incognito));
        return ExitCode::FAILURE;
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

            // Which files are in force, weakest first, and then only the names where that order
            // decided something. A person reading a value they did not expect has three places it
            // could have come from, and the paths are the whole of what tells them which.
            for layer in settings.layers() {
                fact(t!(doctor_settings_layer), layer.display().to_string());
            }
            for (name, path) in settings.overridden() {
                fact(
                    t!(doctor_settings_override),
                    t!(
                        doctor_settings_overridden,
                        name = name,
                        path = path.display().to_string()
                    ),
                );
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
    // Not a warning: without confinement, untrusted work will be refused rather than run, so
    // this is a hard problem for the user to solve.
    if !report_confinement(
        bravebot_sandbox::for_current_platform().map(|sandbox| sandbox.capabilities()),
    ) {
        ok = false;
    }

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
                .map(bravebot_config::bedrock::Entry::display_name)
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

/// What `doctor` says about confinement: the lines, and whether there was any.
struct Confinement {
    lines: Vec<String>,
    established: bool,
}

/// Report the confinement actually achieved here, and say whether there was any.
///
/// Printed rather than assumed: the guarantee differs by platform and kernel, and a
/// user is entitled to know which one they have before trusting the sandbox.
///
/// Takes what the lookup found rather than calling it: a machine has only its own backend to
/// look up, so what is said about the other two levels is otherwise unreachable.
fn report_confinement(found: Result<Capabilities, SandboxError>) -> bool {
    let report = confinement(found);
    for line in &report.lines {
        // A refusal is not a finding among the others: it goes where a problem goes.
        match report.established {
            true => println!("{line}"),
            false => eprintln!("{line}"),
        }
    }
    report.established
}

/// The confinement section of `doctor`: the level in force, and what enforces it.
///
/// Built rather than printed, so what the section says about a level is a value a test can hold.
fn confinement(found: Result<Capabilities, SandboxError>) -> Confinement {
    match found {
        Ok(caps) => Confinement {
            lines: vec![
                t!(doctor_confinement, level = named(caps.level)).to_string(),
                aligned(t!(doctor_mechanisms), caps.mechanisms.join(", "), DETAIL),
                aligned(
                    t!(doctor_network_denial),
                    if caps.network_denial_enforced {
                        t!(doctor_kernel_enforced)
                    } else {
                        t!(doctor_not_enforced)
                    },
                    DETAIL,
                ),
            ],
            established: true,
        },
        Err(err) => Confinement {
            lines: vec![
                t!(doctor_confinement_unavailable).to_string(),
                format!("  {err}"),
            ],
            established: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bravebot_core::capability::Capability;
    use bravebot_core::event::Sink;
    use bravebot_core::label::Label;
    use bravebot_core::slot::SlotId;
    use bravebot_sandbox::policy::ConfinementLevel;
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

    /// The lines `doctor` prints for a set of capabilities, for the two tests below.
    fn confinement_report(level: ConfinementLevel, network_denial_enforced: bool) -> Vec<String> {
        confinement(Ok(Capabilities {
            level,
            mechanisms: vec!["a mechanism"],
            network_denial_enforced,
        }))
        .lines
    }

    /// The guarantee genuinely differs by platform and kernel, so which of the three is in force
    /// is what somebody runs `doctor` to learn before trusting the sandbox with untrusted work. A
    /// level it does not name is one they have to assume.
    #[test]
    fn doctor_names_the_confinement_level_in_force() {
        // The opening line rather than the report, because "kernel-enforced" is also what the
        // network denial line says, and a level reported into the wrong field is not reported.
        let opening = |level| {
            confinement_report(level, false)
                .first()
                .expect("the report opens with the level")
                .clone()
        };

        for level in [
            ConfinementLevel::Kernel,
            ConfinementLevel::Partial,
            ConfinementLevel::None,
        ] {
            let line = opening(level);
            assert!(
                line.contains(&named(level)),
                "the level in force is not in the line that reports it: {line}"
            );
        }

        // Three openings that read the same would satisfy the loop above while telling a reader
        // nothing about which of the three they have.
        assert_ne!(
            opening(ConfinementLevel::Kernel),
            opening(ConfinementLevel::Partial)
        );
        assert_ne!(
            opening(ConfinementLevel::Partial),
            opening(ConfinementLevel::None)
        );
        assert_ne!(
            opening(ConfinementLevel::Kernel),
            opening(ConfinementLevel::None)
        );
    }

    /// An overstated capability is the failure this report exists to prevent. A backend that
    /// leaves network denial to convention rather than to the kernel is the case somebody most
    /// needs told, since it is the one where the guarantee they assume is not the one they have.
    #[test]
    fn doctor_says_whether_the_kernel_enforces_network_denial() {
        // One level, so the difference can only be the answer to that question.
        assert_ne!(
            confinement_report(ConfinementLevel::Kernel, true).last(),
            confinement_report(ConfinementLevel::Kernel, false).last(),
            "the report reads the same whether or not the kernel enforces network denial"
        );
    }

    /// Without a backend, untrusted work is refused rather than run unconfined, so this is a thing
    /// the user has to go and fix: a `doctor` that reported it and still exited successfully would
    /// present it as one finding among the others, and one that did not say what was missing would
    /// leave them nothing to act on.
    #[test]
    fn confinement_that_could_not_be_established_fails_the_run() {
        let missing = || SandboxError::Unavailable {
            platform: "test",
            detail: "no backend is implemented here".into(),
        };

        assert!(
            !report_confinement(Err(missing())),
            "doctor reported no confinement and still passed"
        );

        let lines = confinement(Err(missing())).lines;
        assert!(
            lines
                .iter()
                .any(|line| line.contains(&missing().to_string())),
            "the refusal does not say what was missing: {lines:?}"
        );
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
            not_served: None,
            named_on_the_command_line: false,
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
            not_served: None,
            named_on_the_command_line: false,
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

    /// A scratch directory that removes itself, so tests do not leave state behind.
    ///
    /// Under this crate's own build directory rather than the system temporary one, which is
    /// shared between users and where a name this predictable is somebody else's to create first.
    struct Scratch {
        path: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/test-scratch")
                .join(name);
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create scratch");
            Self {
                path: path.canonicalize().expect("canonical scratch"),
            }
        }

        /// A directory inside the scratch, made ready to be used.
        fn directory(&self, name: &str) -> PathBuf {
            let path = self.path.join(name);
            std::fs::create_dir_all(&path).expect("create directory");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// The mode composes rather than leads: what is left after taking it out is the invocation the
    /// person would have typed without it, so every dispatch below sees what it always saw.
    #[test]
    fn the_incognito_flag_is_taken_out_wherever_it_appears() {
        for typed in [
            &["--incognito", "-p", "do a thing"][..],
            &["-p", "--incognito", "do a thing"][..],
            &["-p", "do a thing", "--incognito"][..],
        ] {
            let mut arguments = args(typed);
            assert!(
                take_incognito(&mut arguments),
                "{typed:?} did not engage it"
            );
            assert_eq!(
                arguments,
                args(&["-p", "do a thing"]),
                "left over: {typed:?}"
            );
        }
    }

    /// Asking twice for a mode that is already on is not an error to report.
    #[test]
    fn asking_for_incognito_twice_is_asking_once() {
        let mut arguments = args(&["--incognito", "--incognito", "do a thing"]);
        assert!(take_incognito(&mut arguments));
        assert_eq!(arguments, args(&["do a thing"]));
    }

    /// The flag is the whole invocation often enough to be worth pinning: what is left is nothing,
    /// which the dispatch reads as the interactive session, and that is the intent.
    #[test]
    fn incognito_alone_leaves_the_interactive_session() {
        let mut arguments = args(&["--incognito"]);
        assert!(take_incognito(&mut arguments));
        assert!(arguments.is_empty());
    }

    /// An ordinary invocation is untouched, and stays not incognito. A mode that turned itself on
    /// for something that merely mentioned it would be worse than one that never worked.
    #[test]
    fn an_invocation_without_the_flag_is_left_alone() {
        let mut arguments = args(&["-p", "write about incognito mode"]);
        assert!(!take_incognito(&mut arguments));
        assert_eq!(arguments, args(&["-p", "write about incognito mode"]));
    }

    /// Absent is the state every run is in unless somebody typed the flag, and it is the only state
    /// in which a write is put to a person first.
    #[test]
    fn permissions_are_enforced_unless_the_flag_is_given() {
        let mut arguments = args(&["do a thing"]);
        assert!(!take_skip_permissions(&mut arguments));
        assert_eq!(arguments, args(&["do a thing"]));
    }

    /// Taken out wherever it appears, so the parser downstream sees only what it already understood.
    /// Left in, it would come back as "unexpected argument" from whichever parser met it.
    #[test]
    fn the_skip_permissions_flag_is_taken_out_wherever_it_appears() {
        for typed in [
            &["--dangerously-skip-permissions", "-p", "do a thing"][..],
            &["-p", "--dangerously-skip-permissions", "do a thing"][..],
            &["-p", "do a thing", "--dangerously-skip-permissions"][..],
        ] {
            let mut arguments = args(typed);
            assert!(
                take_skip_permissions(&mut arguments),
                "{typed:?} did not engage it"
            );
            assert_eq!(
                arguments,
                args(&["-p", "do a thing"]),
                "left over: {typed:?}"
            );
        }
    }

    /// It belongs to every way of starting, not only to a one-shot run, so stripping it must leave a
    /// resume or a bare interactive invocation still recognisable to the dispatch below.
    #[test]
    fn the_flag_leaves_every_other_way_of_starting_intact() {
        let mut arguments = args(&[
            "--resume",
            "1787860306-65099",
            "--dangerously-skip-permissions",
        ]);
        assert!(take_skip_permissions(&mut arguments));
        assert_eq!(arguments, args(&["--resume", "1787860306-65099"]));

        // Nothing but the flag is an interactive session, not an unknown option.
        let mut alone = args(&["--dangerously-skip-permissions"]);
        assert!(take_skip_permissions(&mut alone));
        assert!(alone.is_empty());
    }

    /// It composes with the other flag that is taken out before dispatch, in either order: both are
    /// about the whole run rather than about a task, and somebody may well want both.
    #[test]
    fn it_composes_with_incognito() {
        for typed in [
            &["--incognito", "--dangerously-skip-permissions", "-p", "x"][..],
            &["--dangerously-skip-permissions", "--incognito", "-p", "x"][..],
        ] {
            let mut arguments = args(typed);
            assert!(take_incognito(&mut arguments), "{typed:?}");
            assert!(take_skip_permissions(&mut arguments), "{typed:?}");
            assert_eq!(arguments, args(&["-p", "x"]), "left over: {typed:?}");
        }
    }

    #[test]
    fn a_model_flag_names_the_model_a_run_asks_for() {
        let invocation =
            parse_invocation(&args(&["--model", "some-model", "do a thing"])).expect("parses");
        assert_eq!(invocation.model.as_deref(), Some("some-model"));
        assert_eq!(invocation.prompt, "do a thing");
    }

    /// A run that named no model leaves the configured one in force, which is the whole of how
    /// `--model` ranks above configuration without standing in for it.
    #[test]
    fn a_run_that_named_no_model_names_nothing() {
        let invocation = parse_invocation(&args(&["do a thing"])).expect("parses");
        assert_eq!(invocation.model, None);
    }

    /// The name is carried as typed, since what a tier word and an older spelling of the routing
    /// entry name is a question for the configuration and there is none at parse.
    #[test]
    fn a_model_name_is_carried_as_it_was_typed() {
        let invocation =
            parse_invocation(&args(&["--model", "automatic", "do a thing"])).expect("parses");
        assert_eq!(invocation.model.as_deref(), Some("automatic"));
    }

    /// A tier word in the settings key this flag outranks resolves to a model that exists, so a
    /// flag that sent the word as written would refuse a spelling the file it overrides takes and
    /// be answered by whatever the service substitutes for a name it has never heard of.
    #[test]
    fn a_tier_word_on_the_command_line_names_the_model_the_settings_key_would() {
        let config = bravebot_config::Config::from_lookup(|key| match key {
            "BRAVE_AI_CHAT_ENDPOINT" => Some("https://example.invalid".into()),
            "BRAVE_SERVICES_KEY_ID" => Some("test-key-id".into()),
            "SERVICES_KEY_AICHAT" => Some("test-signing-key".into()),
            _ => None,
        })
        .expect("a configuration");

        let named = parse_invocation(&args(&["--model", "opus", "do a thing"]))
            .expect("parses")
            .model
            .map(|name| config.model_named(&name));

        assert_eq!(
            named.as_deref(),
            Some(bravebot_config::bedrock::Tier::Opus.brave_model())
        );
    }

    #[test]
    fn a_model_flag_with_no_name_is_refused() {
        let err = parse_invocation(&args(&["--model"])).expect_err("must refuse");
        assert!(err.contains("--model"), "{err}");
    }

    /// A script that computed an empty variable asked for a model. Reading the blank as no choice
    /// would answer it with whatever was configured and say nothing, which is the substitution
    /// this flag exists to make impossible.
    #[test]
    fn a_blank_model_is_refused_rather_than_read_as_no_choice() {
        for typed in [
            args(&["--model", "", "do a thing"]),
            args(&["--model", "   ", "do a thing"]),
        ] {
            let err = parse_invocation(&typed).expect_err("must refuse");
            assert!(err.contains("--model"), "{typed:?}: {err}");
        }
    }

    /// The flag names a model for one run, which is the only thing a script can pin a model
    /// against a choice recorded elsewhere with.
    #[test]
    fn the_command_line_outranks_the_record_a_session_would_read() {
        assert_eq!(
            model_asked_for(
                Some("named-on-the-command-line".into()),
                Some("chosen".into())
            )
            .as_deref(),
            Some("named-on-the-command-line")
        );
    }

    /// A run that named no model asks for what a session opening in the same directory would, so
    /// reaching a model somebody already chose needs no interactive step and no flag.
    #[test]
    fn a_run_that_named_no_model_reads_the_record_a_session_would() {
        assert_eq!(
            model_asked_for(None, Some("chosen".into())).as_deref(),
            Some("chosen")
        );
    }

    /// Nothing recorded and nothing named leaves the configured model in force, which is what a
    /// person who has never picked one gets in either surface.
    #[test]
    fn a_run_with_nothing_to_go_on_leaves_the_configured_model_in_force() {
        assert_eq!(model_asked_for(None, None), None);
    }

    /// A run asked for a model and was answered by another. Nothing on stdout says so, and a
    /// model is pinned for a reason whichever route pinned it.
    #[test]
    fn a_model_asked_for_and_not_served_is_reported() {
        let complaint = model_not_served("a-premium-model", true, "a-free-one")
            .expect("a complaint about the substitution");
        assert!(complaint.contains("a-premium-model"), "{complaint}");
        assert!(complaint.contains("a-free-one"), "{complaint}");
    }

    /// The routing entry asks for whichever model the server picks, so a concrete name coming back
    /// is that name working rather than a model standing in for another.
    #[test]
    fn a_routing_entry_answered_by_a_model_is_not_a_substitution() {
        assert_eq!(
            model_not_served(bravebot_config::DEFAULT_MODEL, true, "the-model-picked"),
            None
        );
    }

    /// A backend asked by an opaque handle answers with a name that never matched what went in, so
    /// comparing them would fail every run ever made against one.
    #[test]
    fn a_backend_that_does_not_report_what_it_was_asked_is_not_compared() {
        assert_eq!(
            model_not_served("an-opaque-handle", false, "some-model"),
            None
        );
    }

    #[test]
    fn a_model_that_answered_as_asked_is_no_complaint() {
        assert_eq!(model_not_served("same-model", true, "same-model"), None);
    }

    /// The complaint is about the run rather than part of what the run produced, so a pipe of
    /// stdout carries the reply and nothing else whichever way the run went.
    #[test]
    fn a_substituted_model_is_reported_beside_the_reply_never_in_it() {
        let (reply, beside) = written(&Finished {
            reply: "ok",
            notices: &[],
            attempt: None,
            trail: None,
            clean: true,
            not_served: Some("a-premium-model was not served"),
            named_on_the_command_line: true,
        });

        assert_eq!(reply, "ok\n");
        assert!(
            beside.contains("a-premium-model was not served"),
            "{beside}"
        );
    }

    /// The status is the only part of a finished run a script is certain to read, so a model the
    /// command line named and did not get has to reach it. A turn nothing refused is not enough on
    /// its own.
    #[test]
    fn a_run_answered_by_a_model_other_than_the_one_it_named_does_not_succeed() {
        let substituted = Finished {
            reply: "ok",
            notices: &[],
            attempt: None,
            trail: None,
            clean: true,
            not_served: Some("a-premium-model was not served"),
            named_on_the_command_line: true,
        };
        let served = Finished {
            not_served: None,
            ..substituted
        };

        assert!(!substituted.succeeded());
        assert!(served.succeeded());
    }

    /// Below the flag the model is whatever was recorded or configured, so failing here would have
    /// a script that names no model exit non-zero over a choice made in a terminal. It is still
    /// reported, which is the whole of what a person needs to see it.
    #[test]
    fn a_substitution_the_command_line_did_not_ask_for_is_reported_and_not_failed() {
        let substituted = Finished {
            reply: "ok",
            notices: &[],
            attempt: None,
            trail: None,
            clean: true,
            not_served: Some("a-premium-model was not served"),
            named_on_the_command_line: false,
        };

        let (reply, beside) = written(&substituted);
        assert_eq!(reply, "ok\n");
        assert!(beside.contains("was not served"), "{beside}");
        assert!(substituted.succeeded());
    }

    #[test]
    fn a_directory_flag_names_a_directory_the_run_may_reach() {
        let invocation = parse_invocation(&args(&["--add-dir", "/somewhere/else", "do a thing"]))
            .expect("parses");
        assert_eq!(invocation.directories, vec!["/somewhere/else".to_string()]);
        assert_eq!(invocation.prompt, "do a thing");
    }

    /// Reaching one sibling checkout is no more natural than reaching two, and a flag that could
    /// only be given once would be a rule about typing.
    #[test]
    fn the_directory_flag_is_repeatable() {
        let invocation = parse_invocation(&args(&[
            "--add-dir",
            "/one",
            "--add-dir",
            "/two",
            "do a thing",
        ]))
        .expect("parses");
        assert_eq!(
            invocation.directories,
            vec!["/one".to_string(), "/two".to_string()]
        );
    }

    #[test]
    fn a_directory_flag_with_no_path_is_refused() {
        for typed in [
            args(&["--add-dir"]),
            args(&["--add-dir", "  ", "do a thing"]),
        ] {
            let err = parse_invocation(&typed).expect_err("must refuse");
            assert!(err.contains("--add-dir"), "{typed:?}: {err}");
        }
    }

    /// The point of the flag: a file outside the working directory is unreachable until one is
    /// opened, and reachable afterwards.
    #[test]
    fn a_directory_the_command_line_named_is_reachable() {
        let scratch = Scratch::new("add-dir-reachable");
        let project = scratch.directory("project");
        let beside = scratch.directory("beside");
        let file = beside.join("notes.md");
        std::fs::write(&file, "notes").expect("write");

        let mut workspace = Workspace::new(project).expect("a workspace");
        assert!(
            workspace.confines(&file).is_err(),
            "reachable before it was opened"
        );
        open_directories(&mut workspace, &[beside.display().to_string()]).expect("opens");
        assert!(workspace.confines(&file).is_ok(), "not reachable after");
    }

    /// A script that asked to reach a directory and did not would otherwise fail somewhere further
    /// in, over a file it was told it could open.
    #[test]
    fn a_directory_that_cannot_be_opened_stops_the_run() {
        let scratch = Scratch::new("add-dir-missing");
        let mut workspace = Workspace::new(scratch.directory("project")).expect("a workspace");
        let absent = scratch.path.join("not-here");

        let err = open_directories(&mut workspace, &[absent.display().to_string()])
            .expect_err("must refuse");
        assert!(err.contains("not-here"), "{err}");
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
            not_served: None,
            named_on_the_command_line: false,
        });
        assert_eq!(reply, "ok\n");
        assert!(beside.contains("not usable"), "got: {beside}");
        assert!(!reply.contains("not usable"));
    }

    /// Nobody is watching a one-shot run, so the list that answers a prompt in advance answers
    /// nothing: there is no prompt for it to reach. Without the flag an allow rule decides no
    /// more than a line nobody wrote, while the two lists that refuse and that force an ask carry
    /// over, because those say something a run with nobody at it can still act on.
    #[test]
    fn an_allow_rule_decides_nothing_for_a_run_nobody_is_watching() {
        use bravebot_core::permissions::{Decision, Ruling, Subject};

        let settings = bravebot_config::Settings::parse(
            r#"{
              "permissions": {
                "allow": ["Edit(**)"],
                "ask": ["Bash(git push *)"],
                "deny": ["Read(./.env)"]
              }
            }"#,
        );

        let (permissions, rejected) = rules_for_a_one_shot_run(&settings, None, false);
        assert!(rejected.is_empty());
        assert_eq!(
            permissions.for_path(Subject::Edit, "notes.md"),
            Decision::Unmatched,
            "an allow rule answered a write prompt in a run with nobody to prompt"
        );
        assert_eq!(
            permissions.for_path(Subject::Read, ".env"),
            Decision::Ruled(Ruling::Deny)
        );
        assert_eq!(
            permissions.for_command("git push origin main"),
            Decision::Ruled(Ruling::Ask)
        );
    }

    /// With the flag the person answered every prompt themselves, so their file is read whole: the
    /// list is theirs, and the run may act because they said so on the command line.
    #[test]
    fn the_flag_is_what_lets_an_allow_rule_decide_again() {
        use bravebot_core::permissions::{Decision, Ruling, Subject};

        let settings =
            bravebot_config::Settings::parse(r#"{"permissions": {"allow": ["Edit(**)"]}}"#);

        let (permissions, _) = rules_for_a_one_shot_run(&settings, None, true);
        assert_eq!(
            permissions.for_path(Subject::Edit, "notes.md"),
            Decision::Ruled(Ruling::Allow)
        );
    }
}
