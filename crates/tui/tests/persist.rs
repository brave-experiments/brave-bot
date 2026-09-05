//! History must survive a restart, which is the whole point of storing it.
//!
//! Uses a temporary HOME so the developer's own history is never read or written.

use bravebot_tui::history::Entry;
use bravebot_tui::store;
use std::sync::Mutex;

/// The prompts of what was read back, which is what these tests are about.
///
/// When each was sent and where from are stored beside them and checked where that is the point;
/// everywhere else they are noise around the question of whether the prompt survived.
fn prompts(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.prompt.as_str()).collect()
}

/// A prompt sent now from nowhere in particular.
fn sent(prompt: &str) -> Entry {
    Entry::sent(prompt, None)
}

/// One lock for the whole file, not one per test.
///
/// `HOME` is process-wide, so every test here contends for the same thing. A mutex declared
/// inside each function would be a different mutex, and two tests would then be free to run at
/// once and see each other's HOME.
static HOME_LOCK: Mutex<()> = Mutex::new(());

/// Point HOME at a scratch directory for the duration of the closure.
fn with_temp_home<T>(name: &str, body: impl FnOnce() -> T) -> T {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let dir = std::env::temp_dir().join(format!("bravebot-home-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch home");

    let previous = std::env::var_os("HOME");
    // SAFETY: single-threaded within the lock, and restored before returning.
    unsafe { std::env::set_var("HOME", &dir) };

    let result = body();

    match previous {
        Some(value) => unsafe { std::env::set_var("HOME", value) },
        None => unsafe { std::env::remove_var("HOME") },
    }
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn an_appended_prompt_is_read_back_next_session() {
    with_temp_home("append", || {
        assert!(store::load_history().is_empty(), "started with history");

        store::append_history(&sent("what does this do?"));
        store::append_history(&sent("and this?"));

        // A fresh load is what the next session does.
        assert_eq!(
            prompts(&store::load_history()),
            ["what does this do?", "and this?"]
        );
    });
}

/// The directory is created on demand, so a first run works with no setup.
#[test]
fn the_directory_is_created_on_first_write() {
    with_temp_home("mkdir", || {
        let dir = store::directory().expect("a home");
        assert!(!dir.exists(), "the directory existed already");

        store::append_history(&sent("first ever prompt"));
        assert!(dir.exists(), "the directory was not created");
        assert_eq!(prompts(&store::load_history()), ["first ever prompt"]);
    });
}

/// A multi-line prompt is the case a line-based file gets wrong, so it is checked through the
/// filesystem rather than only through the encoder.
#[test]
fn a_multiline_prompt_survives_a_round_trip_on_disk() {
    with_temp_home("multiline", || {
        let prompt = "explain this:\n\nfn main() {\n    println!(\"hi\");\n}";
        store::append_history(&sent(prompt));
        assert_eq!(prompts(&store::load_history()), [prompt]);
    });
}

/// Saving replaces the file, which is how a cancelled prompt is dropped.
#[test]
fn saving_replaces_what_was_stored() {
    with_temp_home("save", || {
        store::append_history(&sent("one"));
        store::append_history(&sent("two"));
        store::append_history(&sent("three"));

        store::save_history(&[sent("one"), sent("two")]);
        assert_eq!(prompts(&store::load_history()), ["one", "two"]);
    });
}

/// The file cannot grow without bound, and the newest entries are the ones kept.
#[test]
fn the_stored_history_is_capped() {
    with_temp_home("cap", || {
        let entries: Vec<Entry> = (0..1_500)
            .map(|n| Entry::sent(format!("prompt {n}"), None))
            .collect();
        store::save_history(&entries);

        let loaded = store::load_history();
        assert_eq!(loaded.len(), 1_000);
        assert_eq!(loaded.last().unwrap().prompt, "prompt 1499");
        assert_eq!(loaded.first().unwrap().prompt, "prompt 500");
    });
}

/// A hand-edited or corrupt file must not stop a session starting.
#[test]
fn a_corrupt_file_reads_as_no_history() {
    with_temp_home("corrupt", || {
        let dir = store::directory().expect("a home");
        std::fs::create_dir_all(&dir).expect("dir");
        // Invalid UTF-8, which `read_to_string` refuses.
        std::fs::write(dir.join("history"), [0xff, 0xfe, 0x00]).expect("write");

        assert!(
            store::load_history().is_empty(),
            "a corrupt file was parsed"
        );
    });
}

/// With nowhere to store anything, every operation is a no-op rather than a failure.
#[test]
fn no_home_directory_is_not_an_error() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let previous = std::env::var_os("HOME");
    unsafe { std::env::remove_var("HOME") };

    assert!(store::directory().is_none());
    assert!(store::load_history().is_empty());
    // Must not panic.
    store::append_history(&sent("nowhere to go"));
    store::save_history(&[sent("nor here")]);

    if let Some(value) = previous {
        unsafe { std::env::set_var("HOME", value) };
    }
}

/// The session reads what an earlier one wrote, which is the feature end to end.
#[test]
fn a_session_recalls_a_prompt_stored_by_an_earlier_session() {
    with_temp_home("session", || {
        // An earlier session left this behind.
        store::append_history(&sent("a question from before"));

        let mut session = bravebot_tui::state::Session::new("test").with_stored_history();
        session.recall_older();

        assert_eq!(session.input(), "a question from before");
        assert_eq!(session.history.position(), Some((1, 1)));
    });
}

/// And what this session sends is there for the next one.
#[test]
fn a_prompt_sent_now_is_stored_for_next_time() {
    with_temp_home("session-write", || {
        let mut session = bravebot_tui::state::Session::new("test").with_stored_history();
        for c in "asked now".chars() {
            session.type_char(c);
        }
        session.submit().expect("submitted");

        assert_eq!(prompts(&store::load_history()), ["asked now"]);
    });
}

/// The search shows an age beside every prompt and narrows to one workspace, and neither survives
/// a restart unless the file holds them.
#[test]
fn when_and_where_a_prompt_was_sent_outlive_the_session() {
    with_temp_home("session-recorded", || {
        let mut session = bravebot_tui::state::Session::new("test")
            .in_workspace("/work/here")
            .with_stored_history();
        for c in "asked here".chars() {
            session.type_char(c);
        }
        session.submit().expect("submitted");

        let stored = store::load_history();
        assert_eq!(prompts(&stored), ["asked here"]);
        assert_eq!(stored[0].project.as_deref(), Some("/work/here"));
        assert!(stored[0].at.is_some(), "no time was stored");
    });
}

/// A history written before either was kept is still somebody's history, and it is the one file
/// here whose loss they would notice.
#[test]
fn a_history_from_an_older_version_is_still_read() {
    with_temp_home("session-older", || {
        let dir = store::directory().expect("a home");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(
            dir.join("history"),
            "a question from before
",
        )
        .expect("write");

        let mut session = bravebot_tui::state::Session::new("test").with_stored_history();
        session.recall_older();
        assert_eq!(session.input(), "a question from before");
    });
}

/// A cancelled prompt must not be left on disk: it went back into the input box.
#[test]
fn a_cancelled_prompt_is_removed_from_the_stored_history() {
    with_temp_home("session-cancel", || {
        let mut session = bravebot_tui::state::Session::new("test").with_stored_history();
        for c in "abandoned".chars() {
            session.type_char(c);
        }
        let prompt = session.submit().expect("submitted");
        assert_eq!(prompts(&store::load_history()), ["abandoned"]);

        session.restore(prompt);
        assert!(
            store::load_history().is_empty(),
            "the cancelled prompt stayed on disk"
        );
    });
}

/// History, session records, and the user's skills all live in one directory. Two definitions of
/// where that is would drift, and the interface would write its state somewhere the agent never
/// looks.
#[test]
fn the_interface_and_the_agent_agree_on_where_home_is() {
    with_temp_home("agreement", || {
        assert_eq!(store::directory(), bravebot_agent::home::directory());
        assert!(store::directory().is_some(), "no home was found at all");
    });
}

/// The theme choice outlives the session that made it, the same way the model choice does.
#[test]
fn a_chosen_theme_is_read_back_next_session() {
    with_temp_home("theme", || {
        assert_eq!(store::load_theme(), None, "started with a theme");
        store::save_theme("nord");
        assert_eq!(store::load_theme().as_deref(), Some("nord"));
    });
}
