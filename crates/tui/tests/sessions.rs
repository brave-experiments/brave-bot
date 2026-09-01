//! Sessions written to disk and read back.
//!
//! These run against a real `~/.bravebot`, redirected by `HOME`, because the point of the feature is
//! what is on the filesystem afterwards: a record that a later process can find, and an audit a
//! person can read.

use bravebot_agent::Conversation;
use bravebot_agent::Workspace;
use bravebot_aichat::protocol::Message;
use bravebot_core::capability::Capability;
use bravebot_core::event::Event;
use bravebot_core::label::Label;
use bravebot_core::programs::TrustedPrograms;
use bravebot_core::todo::{Item, List, Row, Status, rows};
use bravebot_core::trust::TrustStore;
use bravebot_tui::sessions::{self, Handle, Standing, StoredManifest};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

/// Serialises the tests in this binary.
///
/// `HOME` is process-wide, and each test points it at a directory of its own, so two running at
/// once would send one test's writes into the other's home. Held for the lifetime of the
/// [`Scratch`], which is exactly the span over which `HOME` belongs to one test.
///
/// This was a comment saying there was only one test function here for that reason. That is not
/// a property anyone can maintain, and the next test added broke the first.
static HOME: Mutex<()> = Mutex::new(());

/// A scratch home, so a test never touches the real one.
struct Scratch {
    home: PathBuf,
    project: PathBuf,
    /// Dropped last, releasing `HOME` only once this test's directory is gone.
    _lock: MutexGuard<'static, ()>,
}

impl Scratch {
    fn new(name: &str) -> Self {
        // A test that panicked while holding this poisoned nothing worth protecting: the guard
        // covers an environment variable, and the next test overwrites it anyway.
        let lock = HOME.lock().unwrap_or_else(|held| held.into_inner());
        let root = std::env::temp_dir().join(format!("bravebot-sessions-test-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::create_dir_all(&project).expect("create project");
        // SAFETY: `HOME` is held for as long as this value lives, so no other test in this
        // binary is reading or writing the variable while this one owns it.
        unsafe { std::env::set_var("HOME", &home) };
        Self {
            home,
            project,
            _lock: lock,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.home.parent().expect("a root"));
    }
}

/// A task list for turn one, so the record has something to carry.
fn a_plan() -> BTreeMap<usize, Vec<Row>> {
    BTreeMap::from([(
        1,
        rows(&List::new(vec![
            Item::new("read the file", Status::Done),
            Item::new("change it", Status::Active),
        ])),
    )])
}

/// A map with both polarities, so the round trip is tested on the case that matters: a path a
/// write marked untrusted inside a tree the user vouched for.
fn a_trust_map() -> TrustStore {
    let mut trust = TrustStore::new();
    trust.trust(".");
    trust.distrust("src/fetched.json");
    trust
}

/// Two programs the user vouched for, by resolved path.
fn a_program_list() -> TrustedPrograms {
    TrustedPrograms::from_iter([
        bravebot_core::programs::Command::new("/usr/bin/git", vec!["log".to_string()]),
        bravebot_core::programs::Command::new("/usr/bin/make", vec!["check".to_string()]),
    ])
}

fn a_conversation() -> Conversation {
    let mut conversation = Conversation::new();
    conversation.push(Message::user("make a space invaders game"));
    conversation.push(Message::assistant("here it is"));
    conversation
}

/// Events as the trail records them, with a time on each. The times themselves do not matter to
/// these tests; what matters is that the writer takes the event's own rather than its own.
fn stamped(events: Vec<Event>) -> Vec<bravebot_tui::audit::Stamped> {
    events
        .into_iter()
        .enumerate()
        .map(|(n, event)| bravebot_tui::audit::Stamped {
            at: 1_700_000_000 + n as u64,
            event,
        })
        .collect()
}

/// The name to hand somebody who wants this session back. A session opened and left has no record
/// behind it, so naming it would be offering a command that answers "no session by that name".
#[test]
fn a_session_is_named_once_there_is_a_record_to_name() {
    let scratch = Scratch::new("resumable");

    let mut handle = Handle::begin(&scratch.project);
    assert_eq!(
        handle.resumable(),
        None,
        "a session that was never written offered itself for resuming"
    );

    let conversation = a_conversation();
    handle.save(
        "make a space invaders game",
        Standing {
            conversation: &conversation.snapshot(),
            turns: 1,
            tokens: 1_200,
            todos: &a_plan(),
            trust: &a_trust_map(),
            programs: &a_program_list(),
            directories: &[],
            manifest: None,
        },
    );

    let named = handle.resumable().expect("a written session has a name");
    assert_eq!(named, handle.id());

    // And the name is the one that fetches it, which is the whole of what it is for.
    let record = sessions::load(&scratch.project, named).expect("the named session loads");
    assert_eq!(record.title, "make a space invaders game");

    // A resumed session writes back to the record it came from, so it can be named from the start.
    let resumed = Handle::resuming(&scratch.project, &record);
    assert_eq!(resumed.resumable(), Some(named));
}

#[test]
fn sessions_are_written_read_back_and_kept_per_directory() {
    let scratch = Scratch::new("round-trip");

    // Nothing has happened yet, so there is nothing to resume.
    assert!(sessions::list(&scratch.project).is_empty());

    let conversation = a_conversation();
    let mut handle = Handle::begin(&scratch.project);
    handle.save(
        "make a space invaders game",
        Standing {
            conversation: &conversation.snapshot(),
            turns: 1,
            tokens: 1_200,
            todos: &a_plan(),
            trust: &a_trust_map(),
            programs: &a_program_list(),
            directories: &[],
            manifest: None,
        },
    );
    handle.append_audit(
        1,
        &stamped(vec![
            Event::Observed {
                capability: Capability::FileRead,
                label: Label::untrusted_private(),
            },
            Event::GateBlocked {
                gate: "trusted-read",
                detail: "edit_file".to_string(),
                reason: "content is untrusted".to_string(),
            },
        ]),
    );

    // It is in the list, described the way the picker shows it.
    let listed = sessions::list(&scratch.project);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].title, "make a space invaders game");
    assert!(listed[0].bytes > 0, "the size was not measured");

    // And it comes back as the conversation it was.
    let record = sessions::load(&scratch.project, &listed[0].id).expect("the session loads");
    let restored = Conversation::restored(record.conversation);
    assert_eq!(restored.len(), 2);
    assert_eq!(
        restored.messages()[0].content.text(),
        "make a space invaders game"
    );

    // The audit is beside it, one event per line, with both axes in words.
    let audit = sessions::project_directory(&scratch.project)
        .map(|dir| dir.join(format!("{}.audit.jsonl", listed[0].id)))
        .expect("an audit path");
    let written = std::fs::read_to_string(&audit).expect("the audit was written");
    let lines: Vec<&str> = written.lines().collect();
    assert_eq!(lines.len(), 2, "one line per event: {written}");

    let first: serde_json::Value = serde_json::from_str(lines[0]).expect("a json line");
    assert_eq!(first["event"]["label"]["integrity"], "untrusted");
    assert_eq!(first["event"]["label"]["confidentiality"], "private");
    assert_eq!(first["turn"], 1);

    let second: serde_json::Value = serde_json::from_str(lines[1]).expect("a json line");
    assert_eq!(second["event"]["kind"], "gate_blocked");
    assert_eq!(second["event"]["reason"], "content is untrusted");

    // A second turn appends rather than starting the file again: the audit is the whole session.
    handle.append_audit(
        2,
        &stamped(vec![Event::GatePassed {
            gate: "capability",
            detail: "file_read granted".to_string(),
        }]),
    );
    let written = std::fs::read_to_string(&audit).expect("the audit is still there");
    assert_eq!(written.lines().count(), 3);

    // Saving again updates the record rather than adding a second one.
    handle.save(
        "make a space invaders game",
        Standing {
            conversation: &conversation.snapshot(),
            turns: 2,
            tokens: 3_400,
            todos: &a_plan(),
            trust: &a_trust_map(),
            // Carried forward the way a live session carries it, so the assertion below is about
            // the list surviving a re-save and a resume rather than about one write.
            programs: &a_program_list(),
            directories: &[],
            manifest: None,
        },
    );
    assert_eq!(sessions::list(&scratch.project).len(), 1);

    // Another directory has its own list, which is the point of keying by directory.
    let elsewhere = scratch
        .project
        .parent()
        .expect("a parent")
        .join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("create");
    assert!(
        sessions::list(&elsewhere).is_empty(),
        "sessions leaked between directories"
    );

    let mut other = Handle::begin(&elsewhere);
    other.save(
        "something else",
        Standing {
            conversation: &Conversation::new().snapshot(),
            turns: 1,
            tokens: 0,
            todos: &BTreeMap::new(),
            trust: &TrustStore::new(),
            programs: &TrustedPrograms::new(),
            directories: &[],
            manifest: None,
        },
    );
    assert_eq!(sessions::list(&elsewhere).len(), 1);
    assert_eq!(sessions::list(&scratch.project).len(), 1);

    // Resuming continues the same session rather than starting a new one beside it.
    let record = sessions::load(&scratch.project, &listed[0].id).expect("the session loads");
    let mut resumed = Handle::resuming(&scratch.project, &record);
    resumed.save(
        "",
        Standing {
            conversation: &conversation.snapshot(),
            turns: 3,
            tokens: 5_600,
            todos: &a_plan(),
            trust: &a_trust_map(),
            // Carried forward the way a live session carries it, so the assertion below is about
            // the list surviving a re-save and a resume rather than about one write.
            programs: &a_program_list(),
            directories: &[],
            manifest: None,
        },
    );
    let listed = sessions::list(&scratch.project);
    assert_eq!(listed.len(), 1, "resuming forked the session");
    assert_eq!(listed[0].title, "make a space invaders game");

    // The audit comes back grouped by the turn that left it, which is what puts a trail under
    // the right entry when the transcript is replayed. Reading the record alone left every turn
    // from before the resume with nothing beneath it, though the events were on disk all along.
    let trails = sessions::audit_of(&scratch.project, &listed[0].id);
    assert_eq!(trails.keys().copied().collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(trails[&1].len(), 2);
    assert!(
        trails[&1].iter().any(|line| line.blocked),
        "the refusal came back as an ordinary line"
    );
    assert!(trails[&1][0].text.contains("(U,priv)"), "{:?}", trails[&1]);
    assert_eq!(trails[&2].len(), 1);
    assert!(!trails[&2][0].blocked);

    // A session that never ran a turn has no audit, which is not a failure to report.
    assert!(sessions::audit_of(&elsewhere, "no-such-session").is_empty());

    // The plan each turn worked to comes back with it, under the turn that kept it, and shaped
    // by the same code that draws a live one rather than by a glyph stored in the file.
    let record = sessions::load(&scratch.project, &listed[0].id).expect("the session loads");
    let recalled = sessions::recall(&scratch.project, &record);
    assert_eq!(recalled.todos, a_plan());
    assert_eq!(recalled.todos[&1][0].marker, "✓");
    assert_eq!(recalled.todos[&1][1].status, Status::Active);

    // The trust map goes with the session, so picking it up carries the answer its own user gave
    // and the rules its writes recorded. Both polarities, with the deeper one still winning.
    let restored = record.trust_map().expect("the session recorded a map");
    assert!(restored.is_trusted("src/main.rs"));
    assert!(
        !restored.is_trusted("src/fetched.json"),
        "a path a write had distrusted came back trusted"
    );

    // The programs go with the session too, and for the same reason: the person resuming is the
    // person who vouched for them, so they are not asked about the same program again.
    let vouched = record.trusted_programs();
    assert!(vouched.contains("/usr/bin/git", &["log".to_string()]));
    assert!(vouched.contains("/usr/bin/make", &["check".to_string()]));
    assert!(
        !vouched.contains("/usr/bin/git", &["push".to_string()]),
        "a resumed session vouched for a command it was never given"
    );
    assert_eq!(vouched.len(), 2, "the list came back with something extra");

    // A session that declined recorded that it declined, which is not the same as a record that
    // predates the map. Both trust nothing; only the second is asked about again.
    let declined = sessions::load(&elsewhere, &sessions::list(&elsewhere)[0].id).expect("loads");
    let declined_map = declined.trust_map().expect("declining is still an answer");
    assert!(declined_map.is_empty());

    // A record from a build that never wrote a plan is not a broken record.
    let plainer = sessions::load(&elsewhere, &sessions::list(&elsewhere)[0].id).expect("loads");
    assert!(plainer.todo_rows().is_empty());
    assert_eq!(plainer.tokens, 0);

    // What the session has spent comes back with it. The figure answers "what has this cost me",
    // and starting it again at zero understated a session by everything it had already spent.
    assert_eq!(record.tokens, 5_600, "the last save's total was not kept");

    // A record from a newer build, or one truncated by a full disk, costs its own line in the
    // list and nothing more: it is not a reason to be unable to show the rest.
    let directory = sessions::project_directory(&scratch.project).expect("a directory");
    std::fs::write(directory.join("nonsense.json"), "{ this is not json").expect("write");
    let listed = sessions::list(&scratch.project);
    assert_eq!(listed.len(), 1, "an unreadable record hid the readable one");
    assert_eq!(listed[0].title, "make a space invaders game");

    // A half-written last line is what a killed session leaves. It costs itself and no more: the
    // turns before it still have their trail.
    let audit_path = directory.join(format!("{}.audit.jsonl", listed[0].id));
    let mut contents = std::fs::read_to_string(&audit_path).expect("the audit");
    contents.push_str("{\"at\":1,\"turn\":3,\"eve");
    std::fs::write(&audit_path, contents).expect("write");

    let trails = sessions::audit_of(&scratch.project, &listed[0].id);
    assert_eq!(
        trails.keys().copied().collect::<Vec<_>>(),
        vec![1, 2],
        "a truncated line took the readable ones with it"
    );
}

/// A trail whose events all share one timestamp cannot say which came first, how long a step
/// took, or when a turn ended. It used to: the file was stamped as it was written, which happens
/// once per turn.
#[test]
fn the_audit_keeps_the_time_each_event_happened() {
    let scratch = Scratch::new("audit-times");
    let mut handle = sessions::Handle::begin(&scratch.project);
    handle.save(
        "a task",
        Standing {
            conversation: &a_conversation().snapshot(),
            turns: 1,
            tokens: 0,
            todos: &a_plan(),
            trust: &a_trust_map(),
            programs: &TrustedPrograms::new(),
            directories: &[],
            manifest: None,
        },
    );

    handle.append_audit(
        1,
        &[
            bravebot_tui::audit::Stamped {
                at: 1_700_000_000,
                event: Event::GatePassed {
                    gate: "capability",
                    detail: "file_read granted".to_string(),
                },
            },
            bravebot_tui::audit::Stamped {
                at: 1_700_000_042,
                event: Event::GatePassed {
                    gate: "capability",
                    detail: "file_write granted".to_string(),
                },
            },
        ],
    );

    let audit = sessions::project_directory(&scratch.project)
        .map(|dir| dir.join(format!("{}.audit.jsonl", handle.id())))
        .expect("an audit path");
    let written = std::fs::read_to_string(&audit).expect("the audit was written");
    let times: Vec<u64> = written
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("json"))
        .map(|line| line["at"].as_u64().expect("a time"))
        .collect();

    assert_eq!(
        times,
        vec![1_700_000_000, 1_700_000_042],
        "the events were stamped when they were written down rather than when they happened"
    );
}

/// A renamed session must be findable under its new name at once, without waiting for another
/// turn: a user who renames and then walks away should not lose the name.
#[test]
fn renaming_a_session_rewrites_the_record_immediately() {
    let scratch = Scratch::new("rename");
    let conversation = a_conversation();

    let mut handle = Handle::begin(&scratch.project);
    handle.save(
        "make a space invaders game",
        Standing {
            conversation: &conversation.snapshot(),
            turns: 1,
            tokens: 1_200,
            todos: &a_plan(),
            trust: &a_trust_map(),
            programs: &TrustedPrograms::new(),
            directories: &[],
            manifest: None,
        },
    );
    let derived = sessions::list(&scratch.project)[0].title.clone();
    assert_eq!(derived, "make a space invaders game");

    assert!(handle.rename("the parser bug"));

    let listed = sessions::list(&scratch.project);
    assert_eq!(listed.len(), 1, "renaming made a second session");
    assert_eq!(listed[0].title, "the parser bug");

    // The rest of the record has to survive being amended, since a rename knows none of it.
    let record = sessions::load(&scratch.project, handle.id()).expect("the record is still there");
    assert_eq!(record.turns, 1);
    assert_eq!(record.tokens, 1_200);
    assert!(
        !record.conversation.messages.is_empty(),
        "the conversation was lost"
    );
}

/// The chosen name has to outlast the turn that follows it, or the derived title would take it
/// back the moment the user said anything else.
#[test]
fn a_chosen_name_survives_the_next_turn() {
    let scratch = Scratch::new("rename-survives");
    let conversation = a_conversation();

    let mut handle = Handle::begin(&scratch.project);
    assert!(handle.rename("the parser bug"));
    handle.save(
        "some later question entirely",
        Standing {
            conversation: &conversation.snapshot(),
            turns: 1,
            tokens: 10,
            todos: &a_plan(),
            trust: &a_trust_map(),
            programs: &TrustedPrograms::new(),
            directories: &[],
            manifest: None,
        },
    );

    assert_eq!(sessions::list(&scratch.project)[0].title, "the parser bug");
}

/// Renaming before the first turn has no record to rewrite, so the name has to wait on the handle
/// and be written by the first save rather than being dropped.
#[test]
fn a_session_can_be_named_before_it_has_a_record() {
    let scratch = Scratch::new("rename-early");
    let mut handle = Handle::begin(&scratch.project);

    assert!(handle.rename("named up front"));
    assert!(
        sessions::list(&scratch.project).is_empty(),
        "renaming created a record for a session with no turns"
    );
    assert_eq!(handle.title(), "named up front");
}

/// An empty name is refused rather than silently keeping the old one, which would look like the
/// rename worked.
#[test]
fn an_empty_name_is_refused() {
    let scratch = Scratch::new("rename-empty");
    let mut handle = Handle::begin(&scratch.project);
    handle.rename("a real name");

    for empty in ["", "   ", "\t"] {
        assert!(!handle.rename(empty), "{empty:?} was accepted");
    }
    assert_eq!(handle.title(), "a real name");
}

/// `/add-dir` grants two things at once and only one of them is a trust rule. Carrying the rule
/// alone across a resume left an absolute rule about a tree nothing could open: every path under
/// it refused for escaping the workspace, with nothing on screen to say why.
#[test]
fn a_resumed_session_can_still_open_the_directory_it_added() {
    let scratch = Scratch::new("added-directories");
    let notes = scratch.project.parent().expect("a root").join("notes");
    std::fs::create_dir_all(&notes).expect("create the directory to add");
    std::fs::write(notes.join("todo.md"), "buy milk").expect("write");

    let mut workspace = Workspace::new(&scratch.project).expect("workspace");
    let added = workspace
        .add_directory(notes.to_str().expect("utf-8 path"))
        .expect("the directory is added");
    let todo = added.join("todo.md").display().to_string();

    let mut trust = TrustStore::new();
    trust.trust(&added.display().to_string());

    let conversation = a_conversation();
    let mut handle = Handle::begin(&scratch.project);
    handle.save(
        "read my notes",
        Standing {
            conversation: &conversation.snapshot(),
            turns: 1,
            tokens: 10,
            todos: &BTreeMap::new(),
            trust: &trust,
            programs: &TrustedPrograms::new(),
            directories: workspace.added_directories(),
            manifest: None,
        },
    );

    let record = sessions::load(&scratch.project, handle.id()).expect("the session loads");
    assert_eq!(
        record.directories,
        vec![added.display().to_string()],
        "the record kept the rule but not the directory it was about"
    );

    // Where a resume starts from: a workspace built for the working directory, nothing open.
    let mut resumed = Workspace::new(&scratch.project).expect("workspace");
    assert!(
        resumed.survey(&todo).is_err(),
        "the file was reachable before anything reopened its directory"
    );

    assert!(
        record.reopen_added_directories(&mut resumed).is_empty(),
        "a directory that is still there is not worth a line"
    );
    assert!(
        record
            .trust_map()
            .expect("the session recorded a map")
            .is_trusted(&todo),
        "the rule half of what /add-dir granted"
    );
    assert_eq!(
        resumed.survey(&todo).expect("the file is readable again"),
        "buy milk".len(),
        "the reachable half of what /add-dir granted"
    );
}

/// The rule comes back whatever became of the tree, so a directory that has gone since has to be
/// said out loud: passing over it silently leaves precisely the rule about files nothing can open
/// that restoring the directory exists to prevent.
#[test]
fn a_directory_that_has_gone_since_is_reported_on_resume() {
    let scratch = Scratch::new("added-directory-gone");
    let notes = scratch.project.parent().expect("a root").join("notes");
    std::fs::create_dir_all(&notes).expect("create the directory to add");

    let mut workspace = Workspace::new(&scratch.project).expect("workspace");
    let added = workspace
        .add_directory(notes.to_str().expect("utf-8 path"))
        .expect("the directory is added");

    let conversation = a_conversation();
    let mut handle = Handle::begin(&scratch.project);
    handle.save(
        "read my notes",
        Standing {
            conversation: &conversation.snapshot(),
            turns: 1,
            tokens: 10,
            todos: &BTreeMap::new(),
            trust: &TrustStore::new(),
            programs: &TrustedPrograms::new(),
            directories: workspace.added_directories(),
            manifest: None,
        },
    );
    std::fs::remove_dir_all(&notes).expect("the directory goes away between sessions");

    let record = sessions::load(&scratch.project, handle.id()).expect("the session loads");
    let mut resumed = Workspace::new(&scratch.project).expect("workspace");
    let notes_said = record.reopen_added_directories(&mut resumed);

    assert_eq!(notes_said.len(), 1, "{notes_said:?}");
    assert!(
        notes_said[0].contains(&added.display().to_string()),
        "the line does not say which directory: {notes_said:?}"
    );
    assert!(
        resumed.added_directories().is_empty(),
        "a directory that could not be opened was counted as open"
    );
}

/// A manifest run is written down so it can be read, and marked so it cannot be continued. The
/// conversation is empty on purpose: filling it would make the picker offer a session that has
/// nothing to resume.
#[test]
fn a_manifest_run_is_recorded_and_cannot_be_resumed() {
    let scratch = Scratch::new("manifest-record");
    let conversation = Conversation::new();
    let stored = StoredManifest::of(
        &bravebot_agent::manifest::Attempt {
            shape: Some("1. Read it.".into()),
            proposed: Some("{\"steps\":[]}".into()),
            plan: None,
            steps: Vec::new(),
        },
        Some("the plan is not well formed".into()),
    );

    let mut handle = Handle::begin(&scratch.project);
    handle.save(
        "summarise the docs",
        Standing {
            conversation: &conversation.snapshot(),
            turns: 1,
            tokens: 0,
            todos: &BTreeMap::new(),
            trust: &TrustStore::new(),
            programs: &TrustedPrograms::new(),
            directories: &[],
            manifest: Some(&stored),
        },
    );

    let listed = sessions::list(&scratch.project);
    assert_eq!(listed.len(), 1);
    assert!(listed[0].manifest, "the list did not mark it");

    let record = sessions::load(&scratch.project, handle.id()).expect("the record loads");
    let kept = record.manifest.expect("the attempt was dropped");
    assert_eq!(kept.shape.as_deref(), Some("1. Read it."));
    assert_eq!(kept.failure.as_deref(), Some("the plan is not well formed"));
    let report = kept.describe();
    assert!(report.contains("not usable"), "{report}");
    assert!(report.contains("the plan is not well formed"), "{report}");
}
