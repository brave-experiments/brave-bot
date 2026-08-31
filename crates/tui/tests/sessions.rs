//! Sessions written to disk and read back.
//!
//! These run against a real `~/.bravebot`, redirected by `HOME`, because the point of the feature is
//! what is on the filesystem afterwards: a record that a later process can find, and an audit a
//! person can read.

use bravebot_agent::Conversation;
use bravebot_aichat::protocol::Message;
use bravebot_core::capability::Capability;
use bravebot_core::event::Event;
use bravebot_core::label::Label;
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

fn a_conversation() -> Conversation {
    let mut conversation = Conversation::new();
    conversation.push(Message::user("make a space invaders game"));
    conversation.push(Message::assistant("here it is"));
    conversation
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
            manifest: None,
        },
    );
    handle.append_audit(
        1,
        &[
            Event::Observed {
                capability: Capability::FileRead,
                label: Label::untrusted_private(),
            },
            Event::GateBlocked {
                gate: "trusted-read",
                detail: "edit_file".to_string(),
                reason: "content is untrusted".to_string(),
            },
        ],
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
    assert_eq!(restored.messages()[0].content, "make a space invaders game");

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
        &[Event::GatePassed {
            gate: "capability",
            detail: "file_read granted".to_string(),
        }],
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

/// A manifest run leaves a record like any other session. Until it did, a run that failed left
/// nothing at all and the only way to find out what happened was to run it again.
#[test]
fn a_manifest_run_is_written_down_and_marked() {
    let scratch = Scratch::new("manifest-record");
    let mut handle = Handle::begin(&scratch.project);

    let attempt = bravebot_agent::manifest::Attempt {
        shape: Some("1. Read the readme. 2. Summarise it.".to_string()),
        proposed: Some("Sure, first I will read it".to_string()),
        plan: None,
        steps: Vec::new(),
    };
    let stored = StoredManifest::of(&attempt, Some("the plan was not usable".to_string()));

    handle.save(
        "summarise the readme",
        Standing {
            conversation: &bravebot_agent::Conversation::new().snapshot(),
            turns: 1,
            tokens: 0,
            todos: &BTreeMap::new(),
            trust: &TrustStore::new(),
            manifest: Some(&stored),
        },
    );

    let record = sessions::load(&scratch.project, handle.id()).expect("it was written");
    let manifest = record.manifest.expect("this was a manifest run");
    assert_eq!(
        manifest.shape.as_deref(),
        Some("1. Read the readme. 2. Summarise it.")
    );
    // The raw proposal survives, which is the whole point: a plan that would not parse has no
    // rendered form and the model's own words are the only thing left to read.
    assert_eq!(
        manifest.proposed.as_deref(),
        Some("Sure, first I will read it")
    );
    assert_eq!(manifest.failure.as_deref(), Some("the plan was not usable"));

    // And the list marks it, so the picker can refuse it before anyone selects it.
    let listed = sessions::list(&scratch.project);
    assert!(listed.iter().any(|s| s.id == record.id && s.manifest));
}

/// A turn session must not be marked, or the picker would refuse to resume everything.
#[test]
fn a_turn_session_is_not_marked_as_a_manifest_run() {
    let scratch = Scratch::new("turn-not-marked");
    let mut handle = Handle::begin(&scratch.project);
    let mut conversation = Conversation::new();
    conversation.push(Message::user("hello"));

    handle.save(
        "hello",
        Standing {
            conversation: &conversation.snapshot(),
            turns: 1,
            tokens: 10,
            todos: &BTreeMap::new(),
            trust: &TrustStore::new(),
            manifest: None,
        },
    );

    let record = sessions::load(&scratch.project, handle.id()).expect("it was written");
    assert!(record.manifest.is_none());
    assert!(sessions::list(&scratch.project).iter().all(|s| !s.manifest));
}
