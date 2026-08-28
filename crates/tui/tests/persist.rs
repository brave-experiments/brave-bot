//! History must survive a restart, which is the whole point of storing it.
//!
//! Uses a temporary HOME so the developer's own history is never read or written.

use bravebot_tui::store;
use std::sync::Mutex;

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

        store::append_history("what does this do?");
        store::append_history("and this?");

        // A fresh load is what the next session does.
        assert_eq!(
            store::load_history(),
            vec!["what does this do?".to_string(), "and this?".to_string()]
        );
    });
}

/// The directory is created on demand, so a first run works with no setup.
#[test]
fn the_directory_is_created_on_first_write() {
    with_temp_home("mkdir", || {
        let dir = store::directory().expect("a home");
        assert!(!dir.exists(), "the directory existed already");

        store::append_history("first ever prompt");
        assert!(dir.exists(), "the directory was not created");
        assert_eq!(store::load_history(), vec!["first ever prompt".to_string()]);
    });
}

/// A multi-line prompt is the case a line-based file gets wrong, so it is checked through the
/// filesystem rather than only through the encoder.
#[test]
fn a_multiline_prompt_survives_a_round_trip_on_disk() {
    with_temp_home("multiline", || {
        let prompt = "explain this:\n\nfn main() {\n    println!(\"hi\");\n}";
        store::append_history(prompt);
        assert_eq!(store::load_history(), vec![prompt.to_string()]);
    });
}

/// Saving replaces the file, which is how a cancelled prompt is dropped.
#[test]
fn saving_replaces_what_was_stored() {
    with_temp_home("save", || {
        store::append_history("one");
        store::append_history("two");
        store::append_history("three");

        store::save_history(&["one".to_string(), "two".to_string()]);
        assert_eq!(
            store::load_history(),
            vec!["one".to_string(), "two".to_string()]
        );
    });
}

/// The file cannot grow without bound, and the newest entries are the ones kept.
#[test]
fn the_stored_history_is_capped() {
    with_temp_home("cap", || {
        let entries: Vec<String> = (0..1_500).map(|n| format!("prompt {n}")).collect();
        store::save_history(&entries);

        let loaded = store::load_history();
        assert_eq!(loaded.len(), 1_000);
        assert_eq!(loaded.last().unwrap(), "prompt 1499");
        assert_eq!(loaded.first().unwrap(), "prompt 500");
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
    store::append_history("nowhere to go");
    store::save_history(&["nor here".to_string()]);

    if let Some(value) = previous {
        unsafe { std::env::set_var("HOME", value) };
    }
}

/// The session reads what an earlier one wrote, which is the feature end to end.
#[test]
fn a_session_recalls_a_prompt_stored_by_an_earlier_session() {
    with_temp_home("session", || {
        // An earlier session left this behind.
        store::append_history("a question from before");

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

        assert_eq!(store::load_history(), vec!["asked now".to_string()]);
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
        assert_eq!(store::load_history(), vec!["abandoned".to_string()]);

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
