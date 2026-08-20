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
    match Config::from_env() {
        Ok(config) => {
            println!("configuration OK");
            println!("  endpoint  {}", config.chat_completions_url());
            println!("  key id    {}", config.key_id);
            println!("  model     {}", config.model);
            println!("  key       {} (never transmitted)", config.signing_key);
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("configuration error: {err}");
            ExitCode::FAILURE
        }
    }
}
