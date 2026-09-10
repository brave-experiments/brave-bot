//! The same end-to-end claim as `live.rs`, against a second server.
//!
//! One implementation agreeing with this client is weak evidence about a protocol: the shapes an
//! answer can take differ per server, and `locations_in` has to handle all of them. This is the
//! second data point, and it is a Node server rather than a Rust one so the launch path is exercised
//! for a script whose shebang names an interpreter.
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
/// otherwise needs, and a `.ts` file in the tree proper would be indexed by every other test's
/// server too. Under `target/`, which is already ignored.
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

    // The server refuses to start without a TypeScript installation it can find, and it looks in the
    // workspace. Linked from wherever the global one is rather than installed here, since a test that
    // fetched a package would be a test that needs the network.
    let modules = root.join("node_modules");
    std::fs::create_dir_all(&modules).ok()?;
    let linked = modules.join("typescript");
    if !linked.exists() {
        let global = resolve("typescript-language-server")?
            .parent()?
            .parent()?
            .join("lib/node_modules/typescript");
        if global.is_dir() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&global, &linked).ok()?;
        }
    }

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
        // The state directory itself, which is what the agent hands over: a bare home would
        // put the index next to the user's own files rather than under `.bravebot`.
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".bravebot")),
        resolve,
        false,
        Vec::new(),
    );

    // LSP-5 asks before a server starts. A test is not a person, so it says yes explicitly rather
    // than having the answer inferred: what is being exercised is the protocol, not the prompt.
    match servers.ask(&mut policy, question, &mut |_| true) {
        Ok(answer) => Some(answer),
        Err(error) => {
            // This server drives `tsserver.js` out of a TypeScript installation, and TypeScript 7
            // does not ship one, so on a machine with 7 installed the pair cannot work whatever this
            // crate does. Skipped rather than failed, because it says nothing about the client: the
            // point of this file is a second opinion on the protocol, and there is none to be had
            // here. Anything else is a real failure.
            let said = error.to_string();
            if said.contains("tsserver.js") || said.contains("valid TypeScript installation") {
                eprintln!(
                    "skipped: the installed typescript-language-server cannot drive the \
                           installed TypeScript ({said})"
                );
                return None;
            }
            panic!("typescript-language-server is installed, so this must answer: {error}")
        }
    }
}

/// A second server answers a real question correctly.
///
/// `resolve` is called from `useIt` in the same file, so asking for its definition from that call
/// must come back naming `lib.ts` at the line the function is declared on.
#[test]
fn a_second_server_answers_a_real_question() {
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
        "the server must find the definition of resolve"
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

/// `findReferences` against the second server.
#[test]
fn references_are_found_by_a_second_server() {
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
