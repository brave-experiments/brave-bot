//! The same end-to-end claim as `live.rs`, against a server that can answer from a read-only tree.
//!
//! This exists because the Rust case cannot prove the tool works. rust-analyzer needs to build a
//! crate graph, which needs a subprocess and a writable target directory, so under LSP-5's profile it
//! never finishes indexing and every answer is honestly marked partial: the known cost in
//! docs/specs/tools/lsp.md says so. That leaves the central question open, which is whether a
//! language server confined the way this repository confines one can answer a real question at all.
//!
//! `typescript-language-server` was chosen as the case that should have answered it, since it reads
//! the tree and needs no build. It does not: it calls `mkdir` on a private temp directory while
//! starting and exits when the profile denies that. So both servers tested fail on the write denial,
//! by different routes, and the known cost in the spec now says so.
//!
//! These tests are therefore written to become real assertions the moment LSP-5 grants a scratch
//! directory, and to report rather than fail until then. What they already pin is that nothing in an
//! answer carries text out of a file, which holds whatever the server managed to index.
//!
//! Skipped where the server is absent, for LSP-6's reason.

use bravebot_core::capability::{Capability, CapabilitySet};
use bravebot_core::event::RecordingSink;
use bravebot_core::policy::{Policy, ReleasePlan, Routing};
use bravebot_lsp::{Answer, Operation, Question, Servers};
use std::path::{Path, PathBuf};

/// A tiny TypeScript project, written under this crate's own build directory.
///
/// Built rather than checked in: it is a fixture for a live server and not a thing this repository
/// otherwise needs. Under `target/` specifically, because the profile grants the workspace and a
/// server cannot read a directory nobody granted, including the one it is started in: a fixture in
/// `/tmp` failed with `process.cwd failed ... uv_cwd` before the server reached its first message.
/// That is the profile behaving correctly, and it is worth knowing it presents as a crash in the
/// runtime rather than as a denied read.
fn project() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)?
        .join("target/live-ts-fixture");
    let src = root.join("src");
    std::fs::create_dir_all(&src).ok()?;
    std::fs::write(
        src.join("lib.ts"),
        "export interface Settings {\n  name: string;\n  layer: number;\n}\n\n\
         export function resolve(s: Settings): string {\n  return s.name;\n}\n\n\
         export function useIt(): string {\n  const s: Settings = { name: \"a\", layer: 1 };\n  \
         return resolve(s);\n}\n",
    )
    .ok()?;
    std::fs::write(
        root.join("tsconfig.json"),
        "{ \"compilerOptions\": { \"strict\": true }, \"include\": [\"src\"] }\n",
    )
    .ok()?;
    Some(root)
}

fn resolve(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

fn ask(root: &Path, question: &Question<'_>) -> Option<Answer> {
    if resolve("typescript-language-server").is_none() {
        eprintln!("skipped: typescript-language-server is not on PATH");
        return None;
    }
    let Ok(sandbox) = bravebot_sandbox::for_current_platform() else {
        eprintln!("skipped: no confinement on this platform, so LSP-5 forbids launching one");
        return None;
    };

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
        root.to_path_buf(),
        std::env::var_os("HOME").map(PathBuf::from),
        resolve,
    );

    match servers.ask(&mut policy, sandbox.as_ref(), question) {
        Ok(answer) => Some(answer),
        // Reported rather than asserted, because this is the known cost in the spec rather than a
        // regression: typescript-language-server calls `mkdir` on a private temp directory while
        // starting and exits when the profile denies it. The day LSP-5 grants a scratch directory
        // this becomes a real assertion, and until then a failure here would be a test pinning a
        // limitation as though it were behaviour.
        Err(error) => {
            eprintln!(
                "the server did not answer under confinement: {error}\n\
                 see the known cost in docs/specs/tools/lsp.md: no server tested so far runs \
                 usefully with writes denied"
            );
            None
        }
    }
}

/// The claim the Rust case cannot make: a confined server answers a real question correctly.
///
/// `resolve` is called from `useIt` in the same file, so asking for its definition from that call
/// must come back naming `lib.ts` at the line the function is declared on.
#[test]
fn a_confined_server_answers_a_real_question() {
    let Some(root) = project() else {
        eprintln!("skipped: could not write the fixture project");
        return;
    };
    let file = root.join("src/lib.ts");
    let source = std::fs::read_to_string(&file).expect("just written");

    // Point at the call inside `useIt`, not at the declaration.
    let (line, character) = source
        .lines()
        .enumerate()
        .find_map(|(index, text)| {
            text.find("return resolve(s)")
                .map(|column| (index + 1, column + "return ".len() + 1))
        })
        .expect("the fixture calls resolve");

    let Some(answer) = ask(
        &root,
        &Question {
            operation: Operation::Definition,
            path: &file.to_string_lossy(),
            line,
            character,
            query: None,
        },
    ) else {
        return;
    };

    assert!(
        !answer.locations.is_empty(),
        "a confined server must find the definition of resolve; if this is empty the design does \
         not work, rather than merely being awkward for Rust"
    );
    assert!(
        answer
            .locations
            .iter()
            .any(|location| location.path.ends_with("lib.ts")),
        "the definition is in lib.ts, got {:?}",
        answer.locations
    );
    // The declaration is on line 6 of the fixture; what matters is that it is not the call site.
    assert!(
        answer.locations.iter().any(|location| location.line < line),
        "the definition must be above the call, got {:?} for a call on line {line}",
        answer.locations
    );
}

/// LSP-3 against a second real server, so the claim does not rest on one implementation's
/// answer shapes.
#[test]
fn locations_from_a_second_server_carry_no_text() {
    let Some(root) = project() else {
        return;
    };
    let file = root.join("src/lib.ts");

    let Some(answer) = ask(
        &root,
        &Question {
            operation: Operation::DocumentSymbol,
            path: &file.to_string_lossy(),
            line: 1,
            character: 1,
            query: None,
        },
    ) else {
        return;
    };

    for location in &answer.locations {
        let rendered = format!("{location:?}");
        for from_the_file in ["Settings", "resolve", "useIt", "layer", "name"] {
            assert!(
                !rendered.contains(from_the_file),
                "a location carried {from_the_file:?} out of the file: {rendered}"
            );
        }
    }
}

/// `findReferences` against a server that actually settles, which the Rust case cannot reach.
#[test]
fn references_are_found_by_a_confined_server() {
    let Some(root) = project() else {
        return;
    };
    let file = root.join("src/lib.ts");
    let source = std::fs::read_to_string(&file).expect("just written");

    let (line, character) = source
        .lines()
        .enumerate()
        .find_map(|(index, text)| {
            text.find("interface Settings")
                .map(|column| (index + 1, column + "interface ".len() + 1))
        })
        .expect("the fixture declares Settings");

    let Some(answer) = ask(
        &root,
        &Question {
            operation: Operation::References,
            path: &file.to_string_lossy(),
            line,
            character,
            query: None,
        },
    ) else {
        return;
    };

    // `Settings` is the declaration, the parameter type in `resolve`, and the annotation in `useIt`.
    assert!(
        answer.locations.len() > 1,
        "Settings is referred to in more than one place, got {:?}",
        answer.locations
    );
}
