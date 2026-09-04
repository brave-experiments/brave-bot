//! Which of this crate's words are prose for a person and which are interface to the planner.
//!
//! The distinction is not visible in a type: both are `&str`, both end up in a `format!`, and the
//! natural reflex when a repository becomes localizable is to reach for every string literal in
//! it. Doing that to a tool's description would change what the model does, in a language the
//! person who changed it does not read, and no test of behaviour would catch it because the
//! behaviour would still be self-consistent.
//!
//! So the boundary is drawn where it can be checked: the modules below talk to the planner and
//! to nobody else, and nothing in them is looked up in a catalog. A string in this crate that
//! reaches the screen belongs somewhere else, which is why the word a transcript line begins
//! with lives with the rest of the reporting surface rather than beside the tool table.

use std::path::Path;

/// The modules whose every string is read by the model rather than by a person.
const PLANNER_FACING: [&str; 6] = [
    "src/tools.rs",
    "src/preamble.rs",
    "src/compact.rs",
    "src/processor.rs",
    "src/vet.rs",
    "src/programs.rs",
];

/// A guard is worth nothing if it is silently covering a file that no longer exists.
#[test]
fn every_planner_facing_module_named_here_is_a_module_that_exists() {
    for name in PLANNER_FACING {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
        assert!(
            path.exists(),
            "{name} is listed as planner-facing and is not here. Either it moved, in which case \
             follow it, or it went away, in which case take it off the list"
        );
    }
}

/// The rule this file exists for: a translation must not be able to reach the model.
///
/// Written as a search of the source rather than as a comparison of two rendered requests
/// because the failure it guards against is somebody adding one lookup, and a request built
/// under the only catalog that has shipped so far would read identically either way.
#[test]
fn what_the_planner_reads_is_not_taken_from_a_catalog() {
    for name in PLANNER_FACING {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
        let source = std::fs::read_to_string(&path).expect("read a module of this crate");
        for (number, line) in source.lines().enumerate() {
            assert!(
                !looks_up_a_message(line),
                "{name}:{} looks up a message in a catalog, and everything in this module is \
                 read by the planner. Translating it would change what the model does. If the \
                 string is shown to a person, it belongs in a module that talks to people.\n\
                 {line}",
                number + 1
            );
        }
    }
}

/// Whether the line calls the catalog lookup, as opposed to merely ending a longer macro name
/// with the same three characters, which `format!(` and `assert!(` both do.
fn looks_up_a_message(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut from = 0;
    while let Some(offset) = line[from..].find("t!(") {
        let at = from + offset;
        match at.checked_sub(1).map(|before| bytes[before]) {
            Some(c) if c.is_ascii_alphanumeric() || c == b'_' || c == b':' => {}
            _ => return true,
        }
        from = at + 3;
    }
    false
}
