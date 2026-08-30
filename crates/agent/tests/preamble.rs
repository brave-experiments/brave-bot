//! Where standing instructions are read from, and in what order.
//!
//! No test here touches `HOME`. The home root is an argument, which is what keeps the rest of the
//! suite from depending on whatever the developer happens to have installed.

use bravebot_agent::preamble;
use bravebot_agent::skills::Catalogue;
use bravebot_agent::workspace::Workspace;
use bravebot_core::capability::{Capability, CapabilitySet};
use bravebot_core::event::RecordingSink;
use bravebot_core::policy::{Policy, ReleasePlan, Routing};
use bravebot_core::trust::TrustStore;
use std::path::PathBuf;

/// A scratch directory that removes itself, so tests do not leave state behind.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("bravebot-preamble-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create scratch");
        Self { path }
    }

    fn directory(&self, name: &str) -> PathBuf {
        let dir = self.path.join(name);
        std::fs::create_dir_all(&dir).expect("create directory");
        dir
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn routing() -> Routing {
    let mut r = Routing::new();
    r.insert_trusted("task", "do the work");
    r
}

fn policy<'s>(sink: &'s mut RecordingSink, trusted: &[&str]) -> Policy<'s, RecordingSink> {
    let mut store = TrustStore::new();
    for path in trusted {
        store.trust(path);
    }
    Policy::begin(
        routing(),
        ReleasePlan::new(),
        CapabilitySet::from_iter([Capability::FileRead, Capability::FileWrite]),
        sink,
    )
    .expect("policy")
    .with_trust(store)
}

/// Both files are read, so the question is which one the planner reads last. The project is the
/// more specific of the two, and a convention stated there is the one that has to win when it
/// disagrees with a habit the user carries between projects.
#[test]
fn the_home_agents_file_is_read_before_the_project_one() {
    let scratch = Scratch::new("ordering");
    let home = scratch.directory("home");
    let project = scratch.directory("project");
    std::fs::write(home.join("AGENTS.md"), "GLOBAL-CONVENTION").unwrap();
    std::fs::write(project.join("AGENTS.md"), "PROJECT-CONVENTION").unwrap();
    let workspace = Workspace::new(&project).expect("workspace");

    let mut sink = RecordingSink::new();
    let preamble = {
        let mut policy = policy(&mut sink, &["."]);
        preamble::compose(&mut policy, &workspace, Some(&home), &Catalogue::default())
    };

    let global = preamble
        .text
        .find("GLOBAL-CONVENTION")
        .expect("the home file was not read at all");
    let project = preamble
        .text
        .find("PROJECT-CONVENTION")
        .expect("the project file was not read at all");
    assert!(
        global < project,
        "the project file did not have the last word: {}",
        preamble.text
    );
}

/// A directory opened with `/add-dir` is somewhere to read files from, not a second project. Its
/// conventions are not the ones this work is being done under, and treating them as standing
/// instructions would let opening a directory silently change how every later turn behaves.
#[test]
fn an_added_directory_contributes_no_standing_instructions() {
    let scratch = Scratch::new("added");
    let project = scratch.directory("project");
    let other = scratch.directory("other");
    std::fs::write(other.join("AGENTS.md"), "ADDED-CONVENTION").unwrap();
    let mut workspace = Workspace::new(&project).expect("workspace");
    workspace
        .add_directory(other.to_str().expect("path is utf-8"))
        .expect("add the directory");

    let mut sink = RecordingSink::new();
    let preamble = {
        let mut policy = policy(&mut sink, &["."]);
        preamble::compose(&mut policy, &workspace, None, &Catalogue::default())
    };

    assert!(
        !preamble.text.contains("ADDED-CONVENTION"),
        "an added directory's AGENTS.md became a standing instruction: {}",
        preamble.text
    );
}

/// Writing an AGENTS.md mid-session works, and so does having the agent write one. Reading the
/// sources once at startup would mean the file that was just written is the one instruction the
/// planner cannot see.
#[test]
fn a_file_written_after_one_turn_is_read_by_the_next() {
    let scratch = Scratch::new("afresh");
    let project = scratch.directory("project");
    let workspace = Workspace::new(&project).expect("workspace");

    let mut sink = RecordingSink::new();
    let before = {
        let mut policy = policy(&mut sink, &["."]);
        preamble::compose(&mut policy, &workspace, None, &Catalogue::default())
    };
    assert!(
        !before.text.contains("LATE-CONVENTION"),
        "the file was found before it existed"
    );

    std::fs::write(project.join("AGENTS.md"), "LATE-CONVENTION").unwrap();

    let after = {
        let mut policy = policy(&mut sink, &["."]);
        preamble::compose(&mut policy, &workspace, None, &Catalogue::default())
    };
    assert!(
        after.text.contains("LATE-CONVENTION"),
        "a file written between turns was never picked up: {}",
        after.text
    );
}
