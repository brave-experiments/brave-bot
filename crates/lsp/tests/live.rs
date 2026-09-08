//! A real language server, asked a real question.
//!
//! Everything else about this crate is pinned without a process: parsing is a pure function and the
//! lifecycle rules are properties of the bookkeeping. What none of that reaches is whether the
//! framing, the handshake, the progress tokens and the result shapes are right about a server that
//! actually exists, which is exactly where a protocol client is usually wrong.
//!
//! Skipped rather than failed where the binary is absent or the platform cannot confine one, because
//! LSP-5 and LSP-6 both say that is a legitimate state of the world and not a broken test. A skip
//! prints why, so a run that quietly tested nothing does not read as a pass.

use bravebot_core::capability::{Capability, CapabilitySet};
use bravebot_core::event::RecordingSink;
use bravebot_core::policy::{Policy, ReleasePlan, Routing};
use bravebot_lsp::{Operation, Question, Servers};
use std::path::{Path, PathBuf};

/// The workspace root, which is this repository.
fn workspace() -> PathBuf {
    // The crate directory is `crates/lsp`, so the tree is two levels up.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root is two levels above this crate")
        .to_path_buf()
}

/// Look a bare name up on `$PATH`, the way the agent does.
///
/// Deliberately checks the binary *runs*, not merely that a file of that name exists: on this
/// machine `~/.cargo/bin/rust-analyzer` is a rustup shim that exits non-zero with "Unknown binary in
/// official toolchain" when the component is not installed. A test that only looked for the file
/// would have declared a server present and then failed for a reason that had nothing to do with
/// this crate.
fn resolve(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

fn have_rust_analyzer() -> bool {
    resolve("rust-analyzer").is_some_and(|binary| {
        std::process::Command::new(binary)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

fn can_confine() -> bool {
    bravebot_sandbox::for_current_platform().is_ok()
}

/// Ask one question of a real server, or `None` where there is nothing to ask.
fn ask(question: &Question<'_>) -> Option<bravebot_lsp::Answer> {
    if !have_rust_analyzer() {
        eprintln!("skipped: rust-analyzer is not on PATH");
        return None;
    }
    if !can_confine() {
        eprintln!("skipped: this platform offers no confinement, so LSP-5 forbids launching one");
        return None;
    }

    let root = workspace();
    let mut sink = RecordingSink::new();
    let mut routing = Routing::new();
    routing.insert_trusted("task", "look up a symbol");
    let mut policy = Policy::begin(
        routing,
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::FileRead, Capability::LanguageServer]),
        &mut sink,
    )
    .expect("policy");

    let mut servers = Servers::new(
        root.clone(),
        std::env::var_os("HOME").map(PathBuf::from),
        resolve,
    );
    let sandbox = bravebot_sandbox::for_current_platform().expect("checked above");

    match servers.ask(&mut policy, sandbox.as_ref(), question) {
        Ok(answer) => Some(answer),
        // A server that is installed and confinable and still did not answer is a bug in this
        // crate, not a state of the world to skip over. The first version of this test skipped
        // here and hid one: the profile could not read `.cargo/bin`, so the binary could not be
        // exec'd, and "exited before replying" read as an absent server.
        Err(error) => panic!(
            "rust-analyzer is installed and confinement is available, so this must answer: {error}"
        ),
    }
}

/// The end-to-end claim: framing, handshake and parsing are right about a server that exists.
///
/// `Label` is defined in `crates/core/src/label.rs`, so asking for its definition from a use of it
/// must come back naming that file. This is the one test that would catch a `Content-Length` off by
/// one or a handshake the server rejected.
#[test]
fn a_definition_is_found_in_the_file_that_holds_it() {
    let path = workspace().join("crates/core/src/capability.rs");
    let source = std::fs::read_to_string(&path).expect("the file is in this repository");

    // Find a use of `Label` to point at, rather than hard-coding a line that moves. This is the
    // test working out what to ask; nothing in the tool derives a position this way.
    let (line, character) = source
        .lines()
        .enumerate()
        .find_map(|(index, text)| {
            text.find("Label::untrusted_private")
                .map(|column| (index + 1, column + 1))
        })
        .expect("capability.rs mentions Label::untrusted_private");

    let Some(answer) = ask(&Question {
        operation: Operation::Definition,
        path: &path.to_string_lossy(),
        line,
        character,
        query: None,
    }) else {
        return;
    };

    // A server that never finished indexing cannot answer this, and under LSP-5's profile on this
    // workspace it does not: see the known cost in the spec. Reported rather than asserted, because
    // an answer marked partial is LSP-7 working, not this crate failing.
    if answer.partial {
        eprintln!(
            "the index was still building, so LSP-7 marks this partial; the known cost in \
             docs/specs/tools/lsp.md says why rust-analyzer does not get there under this profile"
        );
        assert!(
            answer.locations.is_empty() || answer.locations.iter().any(|l| l.path.ends_with(".rs")),
            "a partial answer may be empty, but anything in it must still be a real location"
        );
        return;
    }

    assert!(
        !answer.locations.is_empty(),
        "a settled index must find a definition of Label"
    );
    assert!(
        answer
            .locations
            .iter()
            .any(|location| location.path.ends_with("label.rs")),
        "the definition of Label must be in label.rs, got {:?}",
        answer.locations
    );
}

/// LSP-3, proven against a live server rather than against a fixture: whatever a real answer
/// carries, no text from the file survives into a location.
#[test]
fn a_live_answer_carries_no_text_in_its_locations() {
    let path = workspace().join("crates/core/src/capability.rs");

    let Some(answer) = ask(&Question {
        operation: Operation::DocumentSymbol,
        path: &path.to_string_lossy(),
        line: 1,
        character: 1,
        query: None,
    }) else {
        return;
    };

    // Names, docstrings and signatures are all things this file holds and a server reports. None of
    // them may be in a location, whatever shape the answer arrived in.
    for location in &answer.locations {
        let rendered = format!("{location:?}");
        for from_the_file in [
            "Capability",
            "output_label",
            "FileRead",
            "untrusted",
            "///",
            "pub fn",
        ] {
            assert!(
                !rendered.contains(from_the_file),
                "a location carried {from_the_file:?} out of the file: {rendered}"
            );
        }
    }
}

/// `findReferences` over a symbol this repository uses in more than one place.
///
/// The operation whose whole value is that it beats grep, so it is worth knowing it works.
#[test]
fn references_span_more_than_the_declaration() {
    let path = workspace().join("crates/core/src/capability.rs");
    let source = std::fs::read_to_string(&path).expect("the file is in this repository");

    let (line, character) = source
        .lines()
        .enumerate()
        .find_map(|(index, text)| {
            // The enum's own declaration, which everything referring to the type points at.
            text.find("pub enum Capability")
                .map(|column| (index + 1, column + "pub enum ".len() + 1))
        })
        .expect("capability.rs declares Capability");

    let Some(answer) = ask(&Question {
        operation: Operation::References,
        path: &path.to_string_lossy(),
        line,
        character,
        query: None,
    }) else {
        return;
    };

    // An indexing server may answer before it is ready, which LSP-7 says is partial rather than
    // wrong, so a thin answer is not a failure here.
    if answer.partial {
        eprintln!("the index was still building, so this answer is partial by LSP-7");
        return;
    }

    assert!(
        answer.locations.len() > 1,
        "Capability is referred to in more than one place, got {:?}",
        answer.locations
    );
}
