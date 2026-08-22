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
use bua_tui::sessions::{self, Handle};
use std::path::PathBuf;

/// A scratch home, so a test never touches the real one.
///
/// `HOME` is process-wide, so these tests must not run beside each other. They are in one file
/// and one test function for that reason: separate `#[test]`s would run on separate threads and
/// fight over it.
struct Scratch {
    home: PathBuf,
    project: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("bua-sessions-test-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::create_dir_all(&project).expect("create project");
        // SAFETY: single-threaded test, and no other test in this binary reads HOME.
        unsafe { std::env::set_var("HOME", &home) };
        Self { home, project }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.home.parent().expect("a root"));
    }
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
    handle.save(&conversation.snapshot(), 1, "make a space invaders game");
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
    handle.save(&conversation.snapshot(), 2, "make a space invaders game");
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
    other.save(&Conversation::new().snapshot(), 1, "something else");
    assert_eq!(sessions::list(&elsewhere).len(), 1);
    assert_eq!(sessions::list(&scratch.project).len(), 1);

    // Resuming continues the same session rather than starting a new one beside it.
    let record = sessions::load(&scratch.project, &listed[0].id).expect("the session loads");
    let mut resumed = Handle::resuming(&scratch.project, &record);
    resumed.save(&conversation.snapshot(), 3, "");
    let listed = sessions::list(&scratch.project);
    assert_eq!(listed.len(), 1, "resuming forked the session");
    assert_eq!(listed[0].title, "make a space invaders game");

    // A record from a newer build, or one truncated by a full disk, costs its own line in the
    // list and nothing more: it is not a reason to be unable to show the rest.
    let directory = sessions::project_directory(&scratch.project).expect("a directory");
    std::fs::write(directory.join("nonsense.json"), "{ this is not json").expect("write");
    let listed = sessions::list(&scratch.project);
    assert_eq!(listed.len(), 1, "an unreadable record hid the readable one");
    assert_eq!(listed[0].title, "make a space invaders game");
}
