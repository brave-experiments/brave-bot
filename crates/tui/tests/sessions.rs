//! Sessions written to disk and read back.
//!
//! These run against a real `~/.bua`, redirected by `HOME`, because the point of the feature is
//! what is on the filesystem afterwards: a record that a later process can find, and an audit a
//! person can read.

use bua_agent::Conversation;
use bua_aichat::protocol::Message;
use bua_core::capability::Capability;
use bua_core::event::Event;
use bua_core::label::Label;
use bua_core::todo::{Item, List, Row, Status, rows};
use bua_core::trust::TrustStore;
use bua_tui::sessions::{self, Handle, Standing};
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
        let root = std::env::temp_dir().join(format!("bua-sessions-test-{name}"));
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

/// Events as the trail records them, with a time on each. The times themselves do not matter to
/// these tests; what matters is that the writer takes the event's own rather than its own.
fn stamped(events: Vec<Event>) -> Vec<bua_tui::audit::Stamped> {
    events
        .into_iter()
        .enumerate()
        .map(|(n, event)| bua_tui::audit::Stamped {
            at: 1_700_000_000 + n as u64,
            event,
        })
        .collect()
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
        },
    );

    handle.append_audit(
        1,
        &[
            bua_tui::audit::Stamped {
                at: 1_700_000_000,
                event: Event::GatePassed {
                    gate: "capability",
                    detail: "file_read granted".to_string(),
                },
            },
            bua_tui::audit::Stamped {
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
