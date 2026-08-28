//! Stamping the build into the binary, so a session record can say which code wrote it.
//!
//! A transcript is read afterwards, often days later and usually because something in it went
//! wrong. Without this, matching one to the code that produced it means inferring the build from
//! its own symptoms, which is guesswork exactly when guesswork is least affordable.

use std::process::Command;

fn main() {
    // Every crate's sources, not only this one's: the stamp says whether the tree was modified,
    // and a change anywhere in it makes that claim false. Cargo would otherwise leave this
    // script alone while another crate changed underneath it, and the record would say clean.
    println!("cargo:rerun-if-changed=..");
    println!("cargo:rustc-env=BRAVEBOT_BUILD={}", describe());
}

/// What this build is, in as much detail as the tree will give.
fn describe() -> String {
    let version = env!("CARGO_PKG_VERSION");
    // A build from a tarball or a vendored source tree has no git to ask, and says so rather
    // than claiming a commit it cannot name.
    let Some(commit) = git(&["rev-parse", "--short", "HEAD"]) else {
        return format!("{version} (no git)");
    };

    match git(&["status", "--porcelain"]) {
        Some(changes) if !changes.is_empty() => format!("{version} ({commit}, modified)"),
        Some(_) => format!("{version} ({commit})"),
        // Git answered one question and not the other, so the commit is known and the state of
        // the tree is not. Saying nothing about it beats saying it was clean.
        None => format!("{version} ({commit}, tree unknown)"),
    }
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
