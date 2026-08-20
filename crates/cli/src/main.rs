//! Command-line entry point.

use bua_config::Config;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("bua {VERSION}");
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") | None => {
            print_help();
            ExitCode::SUCCESS
        }
        Some("doctor") => doctor(),
        Some(unknown) => {
            eprintln!("unknown argument: {unknown}");
            print_help();
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!("bua {VERSION} — a coding agent resistant to prompt injection");
    println!();
    println!("Usage: bua [command]");
    println!();
    println!("Commands:");
    println!("  doctor       Check that configuration is usable");
    println!();
    println!("Options:");
    println!("  -h, --help       Show this message");
    println!("  -V, --version    Show the version");
}

/// Report whether configuration is usable, without revealing the signing key.
fn doctor() -> ExitCode {
    let mut ok = true;

    match Config::from_env() {
        Ok(config) => {
            println!("configuration OK");
            println!("  endpoint  {}", config.chat_completions_url());
            println!("  key id    {}", config.key_id);
            println!("  model     {}", config.model);
            println!("  key       {} (never transmitted)", config.signing_key);
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
