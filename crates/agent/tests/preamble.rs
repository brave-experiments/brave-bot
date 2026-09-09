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
        preamble::compose(
            &mut policy,
            &workspace,
            Some(&home),
            &Catalogue::default(),
            None,
            None,
        )
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
        preamble::compose(
            &mut policy,
            &workspace,
            None,
            &Catalogue::default(),
            None,
            None,
        )
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
        preamble::compose(
            &mut policy,
            &workspace,
            None,
            &Catalogue::default(),
            None,
            None,
        )
    };
    assert!(
        !before.text.contains("LATE-CONVENTION"),
        "the file was found before it existed"
    );

    std::fs::write(project.join("AGENTS.md"), "LATE-CONVENTION").unwrap();

    let after = {
        let mut policy = policy(&mut sink, &["."]);
        preamble::compose(
            &mut policy,
            &workspace,
            None,
            &Catalogue::default(),
            None,
            None,
        )
    };
    assert!(
        after.text.contains("LATE-CONVENTION"),
        "a file written between turns was never picked up: {}",
        after.text
    );
}

/// A project that wrote its conventions down should not have them ignored over the spelling.
#[test]
fn the_project_file_may_be_named_claude_md() {
    for (name, subdirectory) in [("CLAUDE.md", None), ("CLAUDE.md", Some(".claude"))] {
        let scratch = Scratch::new(&format!("claude-md-{}", subdirectory.unwrap_or("root")));
        let project = scratch.directory("project");
        let holder = match subdirectory {
            Some(sub) => {
                let path = project.join(sub);
                std::fs::create_dir_all(&path).unwrap();
                path
            }
            None => project.clone(),
        };
        std::fs::write(holder.join(name), "PROJECT-CONVENTION").unwrap();
        let workspace = Workspace::new(&project).expect("workspace");

        let mut sink = RecordingSink::new();
        let preamble = {
            let mut policy = policy(&mut sink, &["."]);
            preamble::compose(
                &mut policy,
                &workspace,
                None,
                &Catalogue::default(),
                None,
                None,
            )
        };

        assert!(
            preamble.text.contains("PROJECT-CONVENTION"),
            "{name} under {subdirectory:?} was not read: {}",
            preamble.text
        );
    }
}

/// One set of instructions under two names is still one set. Reading both would state
/// everything twice, and the planner pays for the whole system prompt on every request.
#[test]
fn only_the_first_project_file_that_exists_is_read() {
    let scratch = Scratch::new("first-wins");
    let project = scratch.directory("project");
    std::fs::write(project.join("AGENTS.md"), "THE-REAL-ONE").unwrap();
    std::fs::write(project.join("CLAUDE.md"), "THE-OTHER-ONE").unwrap();
    let workspace = Workspace::new(&project).expect("workspace");

    let mut sink = RecordingSink::new();
    let preamble = {
        let mut policy = policy(&mut sink, &["."]);
        preamble::compose(
            &mut policy,
            &workspace,
            None,
            &Catalogue::default(),
            None,
            None,
        )
    };

    assert!(preamble.text.contains("THE-REAL-ONE"));
    assert!(
        !preamble.text.contains("THE-OTHER-ONE"),
        "both files were read: {}",
        preamble.text
    );
}

/// The shape that cost a real turn a whole round trip: `AGENTS.md` holding one sentence naming
/// the document the project actually keeps its conventions in.
#[test]
fn a_project_file_that_only_names_another_is_followed() {
    let scratch = Scratch::new("pointer");
    let project = scratch.directory("project");
    std::fs::create_dir_all(project.join(".claude")).unwrap();
    std::fs::write(
        project.join("AGENTS.md"),
        "Refer to canonical agent instructions in `.claude/CLAUDE.md`.",
    )
    .unwrap();
    std::fs::write(project.join(".claude/CLAUDE.md"), "THE-REAL-CONVENTIONS").unwrap();
    let workspace = Workspace::new(&project).expect("workspace");

    let mut sink = RecordingSink::new();
    let preamble = {
        let mut policy = policy(&mut sink, &["."]);
        preamble::compose(
            &mut policy,
            &workspace,
            None,
            &Catalogue::default(),
            None,
            None,
        )
    };

    assert!(
        preamble.text.contains("THE-REAL-CONVENTIONS"),
        "the pointer was not followed: {}",
        preamble.text
    );
    // The header names where the instructions came from, not where the pointer was.
    assert!(
        preamble.text.contains("From .claude/CLAUDE.md:"),
        "the source was misnamed: {}",
        preamble.text
    );
}

/// The test that keeps the rule above from eating instructions. A real document cites other
/// files all the time, and swapping the conventions for whatever they mentioned first would be
/// far worse than the round trip this saves.
#[test]
fn a_project_file_that_merely_cites_another_is_read_as_itself() {
    let scratch = Scratch::new("not-a-pointer");
    let project = scratch.directory("project");
    let long = format!(
        "THE-REAL-CONVENTIONS\n\nSee also docs/style.md.\n\n{}",
        "Write a test for everything you change. ".repeat(20)
    );
    std::fs::write(project.join("AGENTS.md"), &long).unwrap();
    std::fs::write(project.join("docs-style-decoy.md"), "DECOY").unwrap();
    let workspace = Workspace::new(&project).expect("workspace");

    let mut sink = RecordingSink::new();
    let preamble = {
        let mut policy = policy(&mut sink, &["."]);
        preamble::compose(
            &mut policy,
            &workspace,
            None,
            &Catalogue::default(),
            None,
            None,
        )
    };

    assert!(preamble.text.contains("THE-REAL-CONVENTIONS"));
    assert!(!preamble.text.contains("DECOY"));
}

/// A pointer naming something the workspace does not hold changes nothing: the file that was
/// found is still the instructions, and confinement is what refuses the rest.
#[test]
fn a_pointer_that_names_nothing_readable_leaves_the_file_standing() {
    let scratch = Scratch::new("pointer-dangling");
    let project = scratch.directory("project");
    std::fs::write(
        project.join("AGENTS.md"),
        "See ../../../etc/passwd.md for the conventions.",
    )
    .unwrap();
    let workspace = Workspace::new(&project).expect("workspace");

    let mut sink = RecordingSink::new();
    let preamble = {
        let mut policy = policy(&mut sink, &["."]);
        preamble::compose(
            &mut policy,
            &workspace,
            None,
            &Catalogue::default(),
            None,
            None,
        )
    };

    assert!(
        preamble.text.contains("See ../../../etc/passwd.md"),
        "the file that was actually found did not reach the planner: {}",
        preamble.text
    );
}

/// The whole reason the block exists. Without it a planner has to run `pwd` to learn where it is,
/// which costs a prompt to approve the run and a second one to be shown the answer, because a
/// command's output comes back quarantined.
#[test]
fn the_working_directory_is_stated_so_nothing_has_to_run_pwd() {
    let scratch = Scratch::new("cwd");
    let project = scratch.directory("project");
    let workspace = Workspace::new(&project).expect("workspace");

    let mut sink = RecordingSink::new();
    let preamble = {
        let mut policy = policy(&mut sink, &["."]);
        preamble::compose(
            &mut policy,
            &workspace,
            None,
            &Catalogue::default(),
            None,
            None,
        )
    };

    assert!(
        preamble
            .text
            .contains(&workspace.root().display().to_string()),
        "the working directory is not in the preamble: {}",
        preamble.text
    );
}

/// A project with no AGENTS.md and no skills still gets the block. These are facts about the
/// machine rather than anything read out of the tree, so there is nothing for an empty project to
/// be missing.
#[test]
fn the_environment_is_stated_even_with_no_instructions_to_read() {
    let scratch = Scratch::new("bare");
    let project = scratch.directory("project");
    let workspace = Workspace::new(&project).expect("workspace");

    let mut sink = RecordingSink::new();
    let preamble = {
        let mut policy = policy(&mut sink, &["."]);
        preamble::compose(
            &mut policy,
            &workspace,
            None,
            &Catalogue::default(),
            None,
            None,
        )
    };

    for fact in ["Working directory:", "Platform:", "Today's date:"] {
        assert!(
            preamble.text.contains(fact),
            "`{fact}` is missing from a preamble with no instructions: {}",
            preamble.text
        );
    }
}

/// Said from the tree rather than assumed. A planner told a checkout is not a repository would
/// avoid git in one that is, and the answer has to come from what is on disk.
#[test]
fn whether_the_tree_is_a_git_repository_is_said_either_way() {
    let scratch = Scratch::new("git");
    let plain = scratch.directory("plain");
    let checkout = scratch.directory("checkout");
    std::fs::create_dir_all(checkout.join(".git")).unwrap();

    let mut sink = RecordingSink::new();
    let described = |workspace: &Workspace, sink: &mut RecordingSink| {
        let mut policy = policy(sink, &["."]);
        preamble::compose(
            &mut policy,
            workspace,
            None,
            &Catalogue::default(),
            None,
            None,
        )
        .text
    };

    let in_checkout = described(&Workspace::new(&checkout).expect("workspace"), &mut sink);
    assert!(
        in_checkout.contains("Is a git repository: true"),
        "a checkout was not reported as one: {in_checkout}"
    );

    let in_plain = described(&Workspace::new(&plain).expect("workspace"), &mut sink);
    assert!(
        in_plain.contains("Is a git repository: false"),
        "a directory with no .git was reported as a repository: {in_plain}"
    );
}

/// `/cd` moves the working directory, and the block is composed per turn, so the next turn states
/// where the session went. A value read once at startup would go stale the moment it moved.
#[test]
fn moving_the_working_directory_restates_it() {
    let scratch = Scratch::new("moved");
    let first = scratch.directory("first");
    let second = scratch.directory("second");

    let mut sink = RecordingSink::new();
    let described = |workspace: &Workspace, sink: &mut RecordingSink| {
        let mut policy = policy(sink, &["."]);
        preamble::compose(
            &mut policy,
            workspace,
            None,
            &Catalogue::default(),
            None,
            None,
        )
        .text
    };

    let here = Workspace::new(&first).expect("workspace");
    let there = Workspace::new(&second).expect("workspace");

    let before = described(&here, &mut sink);
    let after = described(&there, &mut sink);

    assert!(before.contains(&here.root().display().to_string()));
    assert!(
        after.contains(&there.root().display().to_string()),
        "the preamble did not follow the working directory: {after}"
    );
    assert!(
        !after.contains(&here.root().display().to_string()),
        "the preamble still names the directory the session left: {after}"
    );
}

/// A condition waiting on something outside the session is the case a goal handles worst if the
/// turn is left to work it out: answering so as to be sent back spends a round of the goal's
/// budget and a judge's reading of the whole conversation, and ten of those give up minutes
/// before the thing being waited for happens. Naming `sleep` matters because a planner reaching
/// for a shell loop gets a refusal from the command-line compiler and reads it as there being no
/// way to wait at all.
#[test]
fn a_turn_under_a_goal_is_told_how_to_wait_for_something_outside_the_session() {
    let scratch = Scratch::new("goal-waiting");
    let project = scratch.directory("project");
    let workspace = Workspace::new(&project).expect("workspace");

    let mut sink = RecordingSink::new();
    let preamble = {
        let mut policy = policy(&mut sink, &["."]);
        preamble::compose(
            &mut policy,
            &workspace,
            None,
            &Catalogue::default(),
            None,
            Some("a.txt exists"),
        )
    };

    assert!(
        preamble.text.contains("sleep"),
        "the turn was not told what to wait with: {}",
        preamble.text
    );
    assert!(
        preamble.text.contains("rather than answering"),
        "the turn was not told to wait here rather than be sent back: {}",
        preamble.text
    );
}
